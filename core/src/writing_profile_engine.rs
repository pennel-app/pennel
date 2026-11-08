use super::*;
use crate::writing_profile::{SourceDescriptor, SourceKind};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const SAMPLE_BYTES: usize = 3_000;
const REFRESH_SAMPLES: usize = 4;

struct SpeechSample {
    source: SourceDescriptor,
    text: String,
}

fn profile_error(code: &str) -> PennelError {
    PennelError::InvalidResponse {
        message: code.into(),
    }
}

fn digest(bytes: &[u8]) -> String {
    crate::models::hex(&Sha256::digest(bytes))
}

fn chunks(text: &str) -> Vec<&str> {
    let mut remaining = text.trim();
    let mut result = Vec::new();
    while !remaining.is_empty() {
        let mut end = remaining.len().min(SAMPLE_BYTES);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        if end < remaining.len()
            && let Some(boundary) = remaining[..end].rfind(char::is_whitespace)
            && boundary > end / 2
        {
            end = boundary;
        }
        let (part, tail) = remaining.split_at(end);
        result.push(part.trim());
        remaining = tail.trim();
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn append_samples(
    result: &mut Vec<SpeechSample>,
    session_id: &str,
    source_id: &str,
    kind: SourceKind,
    created_at_ms: i64,
    title: &str,
    evidence: &str,
    text: &str,
) {
    for (index, text) in chunks(text).into_iter().enumerate() {
        let word_count = text
            .split_whitespace()
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        if word_count == 0 {
            continue;
        }
        result.push(SpeechSample {
            source: SourceDescriptor {
                id: format!("{source_id}:{index}"),
                session_id: session_id.into(),
                kind,
                created_at_ms,
                title: title.chars().filter(|c| !c.is_control()).take(80).collect(),
                fingerprint: digest(format!("{evidence}\0{text}").as_bytes()),
                word_count,
            },
            text: text.into(),
        });
    }
}

fn eligible_owner_turn(row: &crate::MeetingUtterance) -> bool {
    !row.segment.failed
        && !row.segment.text.trim().is_empty()
        && row.metadata.attribution.kind == crate::AttributionKind::Single
        && row.metadata.attribution.self_voice
        && !row.metadata.attribution.provisional
        && row.metadata.manual.is_none()
        && row.metadata.unresolved_manual.is_empty()
}

impl PennelEngine {
    fn ensure_writing_profile_preview(&self) -> Result<(), PennelError> {
        if self.preview {
            Ok(())
        } else {
            Err(profile_error("preview_only"))
        }
    }

    fn writing_profile_samples(&self) -> Result<Vec<SpeechSample>, PennelError> {
        let mut result = Vec::new();
        let Some(started_at) = self.writing_profile.lock().unwrap().started_at_ms() else {
            return Ok(result);
        };
        for entry in self.history.lock().unwrap().writing_profile_entries()? {
            if entry.created_at_ms < started_at {
                continue;
            }
            let session_id = format!("dictation:{}", entry.id);
            append_samples(
                &mut result,
                &session_id,
                &session_id,
                SourceKind::Dictation,
                entry.created_at_ms,
                "Dictation",
                &digest(entry.raw.as_bytes()),
                &entry.raw,
            );
        }
        // An absent/forgotten enrollment cannot authorize historical self-voice estimates.
        let Some(owner) = self.voice_store().my_voiceprint() else {
            return Ok(result);
        };
        let owner = crate::speakers::voice_profile_fingerprint(&owner);
        let legacy_owner_matches = self
            .writing_profile
            .lock()
            .unwrap()
            .legacy_owner_matches(&owner)?;
        let active: HashSet<_> = self
            .active_sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|session| !session.is_finalized())
            .map(|session| session.id())
            .collect();
        let busy = self.meeting_jobs.lock().unwrap().clone();
        let store = self.meeting_store();
        for meeting in store.list() {
            if active.contains(&meeting.id) || busy.contains(&meeting.id) {
                continue;
            }
            match store.recording_metadata(&meeting.id) {
                Ok(Some(metadata)) if metadata.ended_at.is_none() => continue,
                Err(_) => continue,
                _ => {}
            }
            let Ok(snapshot) = store.read_committed_transcript_snapshot(&meeting.id) else {
                continue;
            };
            if snapshot.metadata_status != crate::TranscriptMetadataStatus::Valid {
                continue;
            }
            let session_id = format!("meeting:{}", snapshot.meeting_uid);
            let created_at_ms = chrono::DateTime::parse_from_rfc3339(&meeting.start_iso)
                .map_or(0, |date| date.timestamp_millis());
            for row in snapshot
                .utterances
                .iter()
                .filter(|row| eligible_owner_turn(row))
            {
                let bound_owner = row.metadata.self_voice_profile_fingerprint.as_deref();
                if bound_owner.map_or(!legacy_owner_matches, |fingerprint| fingerprint != owner) {
                    continue;
                }
                append_samples(
                    &mut result,
                    &session_id,
                    &format!("{session_id}:{}", row.metadata.utterance_id),
                    SourceKind::Meeting,
                    created_at_ms,
                    &meeting.title,
                    &format!(
                        "{owner}:{}:{}",
                        crate::utterance_evidence_revision(row),
                        digest(&serde_json::to_vec(&row.metadata).map_err(PennelError::storage)?)
                    ),
                    &row.segment.text,
                );
            }
        }
        Ok(result)
    }

    fn reconcile_writing_profile(&self) -> Result<Vec<SpeechSample>, PennelError> {
        let samples = self.writing_profile_samples()?;
        let sources: Vec<_> = samples.iter().map(|sample| sample.source.clone()).collect();
        self.writing_profile.lock().unwrap().reconcile(&sources)?;
        Ok(samples)
    }

    fn cancel_writing_profile_analysis(&self) {
        if let Some(cancellation) = self.writing_profile_cancellation.lock().unwrap().as_ref() {
            cancellation.cancel();
        }
    }

    fn run_writing_profile_analysis(
        &self,
        cancellation: Arc<crate::InferenceCancellation>,
        deadline: Instant,
        operation: impl FnOnce(Instant) -> Result<String, PennelError> + Send + 'static,
    ) -> Result<String, PennelError> {
        #[cfg(test)]
        if let Some(pool) = self.writing_profile_test_pool.lock().unwrap().clone() {
            return crate::inference::run_cancelled_in_pool(
                &pool,
                cancellation,
                deadline,
                operation,
            );
        }
        crate::inference::run_cancelled_before(cancellation, deadline, operation)
    }

    pub fn writing_profile_learning_enabled(&self) -> bool {
        self.preview && self.writing_profile.lock().unwrap().learning_enabled()
    }

    pub fn writing_profile_sharing_enabled(&self) -> bool {
        self.preview && self.writing_profile.lock().unwrap().sharing_enabled()
    }

    pub fn writing_profile_status(&self) -> Result<Value, PennelError> {
        if self.preview {
            self.reconcile_writing_profile()?;
        }
        let mut status = serde_json::to_value(self.writing_profile.lock().unwrap().status())
            .map_err(PennelError::storage)?;
        status["available"] = self.preview.into();
        Ok(status)
    }

    pub fn set_writing_profile_learning(&self, enabled: bool) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        {
            let _capture = self.inference_capture_gate();
            self.writing_profile
                .lock()
                .unwrap()
                .set_learning(enabled, now_ms())?;
            self.cancel_writing_profile_analysis();
        }
        self.writing_profile_status()
    }

    pub fn set_writing_profile_sharing(&self, enabled: bool) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        self.writing_profile.lock().unwrap().set_sharing(enabled)?;
        self.writing_profile_status()
    }

    pub fn set_writing_profile_source_excluded(
        &self,
        source_id: &str,
        excluded: bool,
    ) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        {
            let _capture = self.inference_capture_gate();
            self.reconcile_writing_profile()?;
            self.writing_profile
                .lock()
                .unwrap()
                .set_excluded(source_id, excluded)?;
            self.cancel_writing_profile_analysis();
        }
        self.writing_profile_status()
    }

    pub fn set_writing_profile_trait(
        &self,
        language: &str,
        trait_key: &str,
        value: Option<&str>,
    ) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        {
            let _capture = self.inference_capture_gate();
            self.writing_profile
                .lock()
                .unwrap()
                .set_trait(language, trait_key, value)?;
            self.cancel_writing_profile_analysis();
        }
        self.writing_profile_status()
    }

    pub fn delete_writing_profile(&self) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        {
            let _capture = self.inference_capture_gate();
            self.writing_profile.lock().unwrap().reset()?;
            self.cancel_writing_profile_analysis();
        }
        self.writing_profile_status()
    }

    pub fn writing_profile_shared(&self, language: &str) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        self.reconcile_writing_profile()?;
        serde_json::to_value(self.writing_profile.lock().unwrap().shared(language)?)
            .map_err(PennelError::storage)
    }

    /// Processes a bounded batch. The desktop caller debounces wakes; the gate also covers manual refresh.
    pub fn refresh_writing_profile(self: &Arc<Self>) -> Result<Value, PennelError> {
        self.ensure_writing_profile_preview()?;
        let Ok(_refresh) = self.writing_profile_refresh.try_lock() else {
            return self.writing_profile_status();
        };
        let samples = self.reconcile_writing_profile()?;
        if !self.writing_profile_learning_enabled() || self.has_active_session() {
            return self.writing_profile_status();
        }
        let pending = self.writing_profile.lock().unwrap().pending();
        for descriptor in pending.into_iter().take(REFRESH_SAMPLES) {
            let Some(sample) = samples
                .iter()
                .find(|sample| sample.source.id == descriptor.id)
            else {
                continue;
            };
            let Some(ticket) = self.writing_profile.lock().unwrap().ticket(&descriptor.id) else {
                continue;
            };
            let messages = crate::writing_profile::extraction_messages(&sample.text)?;
            let cancellation = crate::InferenceCancellation::for_priority(
                crate::InferencePriority::BackgroundAnalysis,
            );
            let (settings, provider, key, system, epoch) = {
                let _capture = self.inference_capture_gate();
                if self
                    .writing_profile
                    .lock()
                    .unwrap()
                    .ticket(&descriptor.id)
                    .as_ref()
                    != Some(&ticket)
                {
                    break;
                }
                let settings = self.settings();
                let Some((account, model)) = settings.resolve(&settings.llm) else {
                    self.writing_profile
                        .lock()
                        .unwrap()
                        .set_error(Some("provider_unavailable"))?;
                    break;
                };
                if account.kind.is_cli_agent() {
                    self.writing_profile
                        .lock()
                        .unwrap()
                        .set_error(Some("provider_unavailable"))?;
                    break;
                }
                let key = if account.kind == ProviderKind::PennelCloud {
                    self.credential(
                        &account,
                        &ApiKeys {
                            by_provider: HashMap::new(),
                        },
                    )
                    .ok()
                    .flatten()
                } else {
                    self.secret_store()
                        .and_then(|secrets| secrets.get(account.id.clone()))
                };
                if requires_key(account.kind) && key.is_none() {
                    self.writing_profile
                        .lock()
                        .unwrap()
                        .set_error(Some("provider_unavailable"))?;
                    break;
                }
                let provider = self.endpoint(&account, model);
                let system = self.system_llm.read().unwrap().clone();
                let epoch = self.inference_epoch();
                self.register_inference_locked(&cancellation);
                *self.writing_profile_cancellation.lock().unwrap() = Some(cancellation.clone());
                (settings, provider, key, system, epoch)
            };
            let deadline = Instant::now() + Duration::from_secs(45);
            let dispatch_cancellation = cancellation.clone();
            let dispatch_engine = self.clone();
            let dispatch_ticket = ticket.clone();
            let dispatch_settings = settings.clone();
            let response = self.run_writing_profile_analysis(
                cancellation.clone(),
                deadline,
                move |deadline| {
                    #[cfg(test)]
                    if let Some(hook) = dispatch_engine
                        .writing_profile_before_dispatch
                        .lock()
                        .unwrap()
                        .clone()
                    {
                        hook();
                    }
                    {
                        let _capture = dispatch_engine.inference_capture_gate();
                        dispatch_engine.reconcile_writing_profile()?;
                        if dispatch_cancellation.is_cancelled()
                            || dispatch_engine.inference_epoch() != epoch
                            || dispatch_engine.settings().llm != dispatch_settings.llm
                            || dispatch_engine.settings().providers != dispatch_settings.providers
                            || dispatch_engine
                                .writing_profile
                                .lock()
                                .unwrap()
                                .ticket(&dispatch_ticket.source_id)
                                .as_ref()
                                != Some(&dispatch_ticket)
                        {
                            return Err(PennelError::Cancelled);
                        }
                    }
                    if provider.kind == ProviderKind::AppleIntelligence {
                        return crate::system_llm::complete(
                            system.as_deref(),
                            &messages,
                            deadline.saturating_duration_since(Instant::now()).as_secs(),
                        );
                    }
                    crate::llm::chat_complete_before(
                        &provider,
                        key.as_deref(),
                        &messages,
                        deadline,
                        Reasoning::Off,
                    )
                },
            );
            self.finish_inference(&cancellation);
            *self.writing_profile_cancellation.lock().unwrap() = None;
            // Configuration/credentials and consent are checked after execution as well as dispatch.
            if cancellation.is_cancelled()
                || matches!(response, Err(PennelError::Cancelled))
                || self.inference_epoch() != epoch
                || self.settings().llm != settings.llm
                || self.settings().providers != settings.providers
                || !self.writing_profile_learning_enabled()
            {
                break;
            }
            let extraction =
                response.and_then(|text| crate::writing_profile::parse_extraction(&text));
            match extraction {
                Ok(extraction) => {
                    let _capture = self.inference_capture_gate();
                    self.reconcile_writing_profile()?;
                    if self.inference_epoch() != epoch {
                        break;
                    }
                    self.writing_profile
                        .lock()
                        .unwrap()
                        .commit(&ticket, extraction)?;
                    self.writing_profile.lock().unwrap().set_error(None)?;
                }
                Err(_) => {
                    self.writing_profile
                        .lock()
                        .unwrap()
                        .set_error(Some("analysis_failed"))?;
                    break;
                }
            }
        }
        self.writing_profile_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_engine(url: &str) -> (tempfile::TempDir, Arc<PennelEngine>) {
        use crate::cloud::SecretStore;
        let directory = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(directory.path().to_string_lossy().into()).unwrap();
        // Other parallel tests deliberately preempt the global pool; these tests own their pool.
        *engine.writing_profile_test_pool.lock().unwrap() = Some(Arc::new(
            crate::meeting_scheduler::PhysicalInferencePool::default(),
        ));
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        secrets.set(
            "profile-provider".into(),
            Some("synthetic-profile-key".into()),
        );
        engine.set_secret_store(secrets).unwrap();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "profile-provider".into(),
            kind: ProviderKind::OpenAi,
            name: "Profile test".into(),
            base_url: url.into(),
        });
        settings.llm = ModelChoice {
            provider_id: "profile-provider".into(),
            model: "profile-test-model".into(),
        };
        settings.language = Some("nl".into());
        engine.update_settings(settings).unwrap();
        engine.set_writing_profile_learning(true).unwrap();
        (directory, engine)
    }

    fn add_dictation(engine: &PennelEngine, id: &str, text: &str) {
        engine
            .history
            .lock()
            .unwrap()
            .insert_done_with_provenance(
                id,
                None,
                text,
                "This AI cleanup must never become evidence",
                now_ms(),
                HistoryCaptureKind::Dictation,
                true,
            )
            .unwrap();
    }

    fn profile_response() -> String {
        serde_json::json!({"choices":[{"message":{"content": serde_json::json!({
            "language":"en", "traits":{"directness":"direct","detail":"concise","register":"plain"}
        }).to_string()}}]})
        .to_string()
    }

    #[test]
    fn selected_provider_builds_profile_and_deleted_sources_are_removed_while_paused() {
        let mut server = mockito::Server::new();
        let (_directory, engine) = configured_engine(&server.url());
        let request = server
            .mock("POST", "/chat/completions")
            .match_header("authorization", "Bearer synthetic-profile-key")
            .match_body(mockito::Matcher::PartialJson(
                serde_json::json!({"model":"profile-test-model"}),
            ))
            .with_body(profile_response())
            .expect(3)
            .create();
        for id in ["one", "two", "three"] {
            add_dictation(&engine, id, &"keep words clear ".repeat(120));
        }
        let status = engine.refresh_writing_profile().unwrap();
        request.assert();
        assert_eq!(status["languages"][0]["language"], "en");
        assert_eq!(status["languages"][0]["status"], "ready");
        assert_eq!(status["languages"][0]["word_count"], 1080);
        engine.set_writing_profile_sharing(true).unwrap();
        engine.set_writing_profile_learning(false).unwrap();
        engine.delete_history("one".into()).unwrap();
        let shared = engine.writing_profile_shared("en").unwrap();
        assert_eq!(shared["session_count"], 2);
        assert_eq!(shared["status"], "insufficient_evidence");
        assert!(shared["guidance"].as_array().unwrap().is_empty());
        engine
            .history
            .lock()
            .unwrap()
            .prune(1, now_ms() + 2 * 86_400_000)
            .unwrap();
        assert_eq!(
            engine.writing_profile_shared("en").unwrap()["word_count"],
            0
        );
        request.assert();
    }

    #[test]
    fn pause_provider_change_and_source_delete_before_dispatch_send_no_transcript() {
        for mutation in ["pause", "provider", "delete"] {
            let mut server = mockito::Server::new();
            let (_directory, engine) = configured_engine(&server.url());
            let request = server.mock("POST", "/chat/completions").expect(0).create();
            add_dictation(
                &engine,
                "pending",
                "Only this spoken source may be analyzed.",
            );
            let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
            let release = Arc::new(std::sync::Barrier::new(2));
            let worker_release = release.clone();
            *engine.writing_profile_before_dispatch.lock().unwrap() = Some(Arc::new(move || {
                arrived_tx.send(()).unwrap();
                worker_release.wait();
            }));
            let worker_engine = engine.clone();
            let worker = std::thread::spawn(move || worker_engine.refresh_writing_profile());
            arrived_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            match mutation {
                "pause" => {
                    engine.set_writing_profile_learning(false).unwrap();
                }
                "provider" => {
                    let mut settings = engine.settings();
                    settings.llm.model = "replacement-model".into();
                    engine.update_settings(settings).unwrap();
                }
                "delete" => {
                    engine.delete_history("pending".into()).unwrap();
                }
                _ => unreachable!(),
            }
            release.wait();
            worker.join().unwrap().unwrap();
            request.assert();
        }
    }

    #[test]
    fn legacy_owner_binding_is_not_reassigned_and_stamped_recognition_restores_eligibility() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_string_lossy().to_string();
        let engine = PennelEngine::new_preview(path.clone()).unwrap();
        let owner_a = vec![1.0, 0.0];
        let owner_b = vec![0.0, 1.0];
        engine
            .voice_store()
            .set_my_voiceprint(owner_a, 25_000)
            .unwrap();
        engine.set_writing_profile_learning(true).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("A past meeting", chrono::Local::now())
            .unwrap();
        let mut row = crate::MeetingUtterance::new(
            crate::Segment {
                speaker: crate::Speaker::Me,
                voice: None,
                start_ms: 0,
                end_ms: 1_000,
                text: "Own meeting speech with valid attribution.".into(),
                failed: false,
            },
            Some("microphone".into()),
        );
        row.metadata.attribution.kind = crate::AttributionKind::Single;
        row.metadata.attribution.self_voice = true;
        let initial = store
            .write_transcript_snapshot(&id, Some(0), &[row.clone()])
            .unwrap();
        assert_eq!(
            engine.writing_profile_status().unwrap()["sources"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        drop(engine);
        let engine = PennelEngine::new_preview(path).unwrap();
        assert_eq!(
            engine.writing_profile_status().unwrap()["sources"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        engine
            .voice_store()
            .set_my_voiceprint(owner_b.clone(), 25_000)
            .unwrap();
        assert!(
            engine.writing_profile_status().unwrap()["sources"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        row.metadata.self_voice_profile_fingerprint =
            Some(crate::speakers::voice_profile_fingerprint(&owner_b));
        row.metadata.attribution_revision += 1;
        store
            .write_transcript_snapshot(&id, Some(initial.revision), &[row])
            .unwrap();
        assert_eq!(
            engine.writing_profile_status().unwrap()["sources"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn ordinary_capture_admission_survives_retry_without_admitting_legacy_history() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into()).unwrap();
        engine.set_writing_profile_learning(true).unwrap();
        let history = engine.history.lock().unwrap();
        history
            .insert_done_with_id("legacy", None, "legacy words", "AI output", 1)
            .unwrap();
        history
            .insert_done_with_provenance(
                "own",
                None,
                "owner said these words",
                "AI cleanup",
                now_ms(),
                HistoryCaptureKind::Dictation,
                true,
            )
            .unwrap();
        history
            .insert_done_with_provenance(
                "command",
                None,
                "rewrite the selection",
                "other person content",
                3,
                HistoryCaptureKind::Command,
                false,
            )
            .unwrap();
        drop(history);
        let samples = engine.writing_profile_samples().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].text, "owner said these words");
        engine.delete_history("own".into()).unwrap();
        assert!(engine.writing_profile_samples().unwrap().is_empty());
    }

    #[test]
    fn profile_mutation_and_export_are_preview_only() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into()).unwrap();
        assert!(!engine.writing_profile_learning_enabled());
        assert!(engine.set_writing_profile_learning(true).is_err());
        assert!(engine.set_writing_profile_sharing(true).is_err());
        assert!(engine.writing_profile_shared("en").is_err());
    }

    #[test]
    fn large_unicode_sources_are_bounded_without_splitting_codepoints() {
        let input = "héllo 🌻 language words. ".repeat(500);
        let pieces = chunks(&input);
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|part| part.len() <= SAMPLE_BYTES));
        assert_eq!(
            pieces.join(" ").split_whitespace().count(),
            input.split_whitespace().count()
        );
    }

    #[test]
    fn owner_filter_rejects_physical_lane_and_uncertain_attribution() {
        let mut row = crate::MeetingUtterance::new(
            crate::Segment {
                speaker: crate::Speaker::Me,
                voice: None,
                start_ms: 0,
                end_ms: 1000,
                text: "A source only I said".into(),
                failed: false,
            },
            Some("microphone".into()),
        );
        assert!(!eligible_owner_turn(&row));
        row.metadata.attribution.kind = crate::AttributionKind::Single;
        row.metadata.attribution.self_voice = true;
        assert!(eligible_owner_turn(&row));
        row.metadata.attribution.provisional = true;
        assert!(!eligible_owner_turn(&row));
        row.metadata.attribution.provisional = false;
        row.metadata.attribution.kind = crate::AttributionKind::Mixed;
        assert!(!eligible_owner_turn(&row));
    }
}
