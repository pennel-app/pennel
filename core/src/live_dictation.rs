use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::engine::{DictationRequest, DictationResult, PennelEngine};
use crate::error::PennelError;
use crate::local_engine::CancelToken;
use crate::provider::ProviderKind;
use crate::settings::{DictationMode, Settings};
use crate::speech::{has_speech, is_hallucination};

const SAMPLE_RATE: u32 = 16_000;
const MAX_AUDIO_SAMPLES: usize = SAMPLE_RATE as usize * 6;
const PREVIEW_INTERVAL_SAMPLES: u64 = SAMPLE_RATE as u64;
const MAX_PREVIEW_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum DictationPreviewMode {
    System,
    Local,
    AfterRecording,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
pub struct DictationPreviewUpdate {
    pub session_id: String,
    pub revision: u64,
    pub text: String,
}

/// Observers enqueue display updates; callbacks must not re-enter session mutation methods.
#[uniffi::export(with_foreign)]
pub trait DictationPreviewObserver: Send + Sync {
    fn changed(&self, update: DictationPreviewUpdate);
}

#[derive(Default)]
struct Caption {
    revision: u64,
    text: String,
}

struct PreviewState {
    id: String,
    observer: Arc<dyn DictationPreviewObserver>,
    cancelled: AtomicBool,
    accepting: AtomicBool,
    emission: Mutex<()>,
    caption: Mutex<Caption>,
}

impl PreviewState {
    fn new(observer: Arc<dyn DictationPreviewObserver>) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            observer,
            cancelled: AtomicBool::new(false),
            accepting: AtomicBool::new(true),
            emission: Mutex::new(()),
            caption: Mutex::new(Caption::default()),
        }
    }

    fn ensure_active(&self) -> Result<(), PennelError> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(PennelError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn publish(&self, text: &str, final_raw: bool) -> Result<(), PennelError> {
        let _emission = self.emission.lock().unwrap();
        self.ensure_active()?;
        if !final_raw && !self.accepting.load(Ordering::SeqCst) {
            return Ok(());
        }
        let text: String = text.trim().chars().rev().take(MAX_PREVIEW_CHARS).collect();
        let text: String = text.chars().rev().collect();
        let update = {
            let mut caption = self.caption.lock().unwrap();
            if caption.text == text {
                return Ok(());
            }
            caption.revision += 1;
            caption.text = text.clone();
            DictationPreviewUpdate {
                session_id: self.id.clone(),
                revision: caption.revision,
                text,
            }
        };
        self.observer.changed(update);
        Ok(())
    }

    fn stop(&self, cancel: bool) {
        let _emission = self.emission.lock().unwrap();
        self.accepting.store(false, Ordering::SeqCst);
        if cancel {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }

    fn with_active<T>(
        &self,
        operation: impl FnOnce() -> Result<T, PennelError>,
    ) -> Result<T, PennelError> {
        let _emission = self.emission.lock().unwrap();
        self.ensure_active()?;
        operation()
    }
}

#[derive(Default)]
struct LocalPending {
    ring: VecDeque<f32>,
    received: u64,
    submitted: u64,
    latest: Option<Vec<f32>>,
    stopped: bool,
}

#[derive(Default)]
struct LocalPreview {
    pending: Mutex<LocalPending>,
    ready: Condvar,
    cancel: CancelToken,
}

impl LocalPreview {
    fn feed(&self, samples: &[f32]) {
        let mut pending = self.pending.lock().unwrap();
        if pending.stopped || samples.is_empty() {
            return;
        }
        pending.received = pending.received.saturating_add(samples.len() as u64);
        if samples.len() >= MAX_AUDIO_SAMPLES {
            pending.ring.clear();
        }
        pending.ring.extend(
            samples[samples.len().saturating_sub(MAX_AUDIO_SAMPLES)..]
                .iter()
                .copied()
                .filter(|sample| sample.is_finite()),
        );
        let excess = pending.ring.len().saturating_sub(MAX_AUDIO_SAMPLES);
        pending.ring.drain(..excess);
        if pending.ring.len() >= SAMPLE_RATE as usize
            && pending.received.saturating_sub(pending.submitted) >= PREVIEW_INTERVAL_SAMPLES
        {
            pending.latest = Some(pending.ring.iter().copied().collect());
            pending.submitted = pending.received;
            self.ready.notify_one();
        }
    }

    fn next(&self) -> Option<Vec<f32>> {
        let mut pending = self.pending.lock().unwrap();
        while pending.latest.is_none() && !pending.stopped {
            pending = self.ready.wait(pending).unwrap();
        }
        if pending.stopped {
            None
        } else {
            pending.latest.take()
        }
    }

    fn stop(&self) {
        self.cancel.cancel();
        let mut pending = self.pending.lock().unwrap();
        pending.stopped = true;
        pending.latest = None;
        pending.ring.clear();
        self.ready.notify_all();
    }
}

fn local_worker(
    local: Arc<LocalPreview>,
    state: Arc<PreviewState>,
    mut decode: impl FnMut(&[f32], &CancelToken) -> Option<String>,
) {
    while let Some(samples) = local.next() {
        if local.cancel.is_cancelled() {
            break;
        }
        if !has_speech(&samples, SAMPLE_RATE) {
            continue;
        }
        if let Some(text) = decode(&samples, &local.cancel)
            && !local.cancel.is_cancelled()
            && !is_hallucination(&text)
        {
            let _ = state.publish(&text, false);
        }
    }
}

/// Owns recognition settings and display-only preview lifetime for one recording.
#[derive(uniffi::Object)]
pub struct LiveDictationSession {
    engine: Arc<PennelEngine>,
    settings: Settings,
    cleanup_verification: crate::cleanup_verification::FrozenCleanupVerification,
    practice: bool,
    mode: DictationPreviewMode,
    state: Arc<PreviewState>,
    local: Option<Arc<LocalPreview>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    finishing: Mutex<()>,
    outcome: Mutex<Option<Result<DictationResult, PennelError>>>,
}

impl LiveDictationSession {
    pub(crate) fn new(
        engine: Arc<PennelEngine>,
        observer: Arc<dyn DictationPreviewObserver>,
        mut settings: Settings,
        practice: bool,
    ) -> Arc<Self> {
        if practice {
            settings.mode = DictationMode::Fast;
        }
        let mode = match settings
            .resolve(&settings.dictation_stt)
            .map(|(account, _)| account.kind)
        {
            Some(ProviderKind::AppleSpeech) => DictationPreviewMode::System,
            Some(ProviderKind::Local) => DictationPreviewMode::Local,
            _ => DictationPreviewMode::AfterRecording,
        };
        let state = Arc::new(PreviewState::new(observer));
        let local =
            (mode == DictationPreviewMode::Local).then(|| Arc::new(LocalPreview::default()));
        let worker = local.as_ref().map(|local| {
            let (local, state, engine, settings) = (
                local.clone(),
                state.clone(),
                engine.clone(),
                settings.clone(),
            );
            std::thread::spawn(move || {
                local_worker(local, state, |samples, cancel| {
                    engine
                        .preview_local(&settings, samples, cancel)
                        .ok()
                        .flatten()
                })
            })
        });
        let cleanup_verification =
            crate::cleanup_verification::FrozenCleanupVerification::capture(&engine, &settings);
        Arc::new(Self {
            engine,
            settings,
            cleanup_verification,
            practice,
            mode,
            state,
            local,
            worker: Mutex::new(worker),
            finishing: Mutex::new(()),
            outcome: Mutex::new(None),
        })
    }

    pub(crate) fn cleanup_verification(
        &self,
    ) -> &crate::cleanup_verification::FrozenCleanupVerification {
        &self.cleanup_verification
    }

    pub(crate) fn ensure_active(&self) -> Result<(), PennelError> {
        self.state.ensure_active()
    }

    pub(crate) fn observe_raw(&self, text: &str) -> Result<(), PennelError> {
        self.state.publish(text, true)
    }

    pub(crate) fn with_active<T>(
        &self,
        operation: impl FnOnce() -> Result<T, PennelError>,
    ) -> Result<T, PennelError> {
        self.state.with_active(operation)
    }

    fn finish_with(
        &self,
        request: DictationRequest,
        command: Option<Option<String>>,
    ) -> Result<DictationResult, PennelError> {
        let _finishing = self.finishing.lock().unwrap();
        self.ensure_active()?;
        if let Some(outcome) = self.outcome.lock().unwrap().as_ref() {
            return outcome.clone();
        }
        self.stop_preview();
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
        self.ensure_active()?;
        let outcome = match command {
            None => self.engine.dictate_with_settings_observed(
                request,
                self.settings.clone(),
                Some(self),
            ),
            Some(selection) => self.engine.command_with_settings_observed(
                request,
                selection,
                self.settings.clone(),
                Some(self),
            ),
        };
        *self.outcome.lock().unwrap() = Some(outcome.clone());
        outcome
    }
}

#[uniffi::export]
impl LiveDictationSession {
    pub fn id(&self) -> String {
        self.state.id.clone()
    }
    pub fn mode(&self) -> DictationPreviewMode {
        self.mode
    }
    pub fn cleanup_verification_availability(&self) -> crate::CleanupVerificationAvailability {
        let mut status = self.cleanup_verification.status();
        if self.settings.mode == DictationMode::Fast {
            status.enabled = false;
            status.reason_code = "disabled".into();
        }
        status
    }

    pub fn language(&self) -> Option<String> {
        self.settings.language.clone()
    }

    pub fn vocabulary(&self) -> Vec<String> {
        crate::system_stt::bounded_vocabulary(&self.settings.dictionary)
    }

    /// Mono 16 kHz PCM; at most six seconds and one newest pending decode are retained.
    pub fn feed_audio(&self, samples: Vec<f32>) {
        if self.state.accepting.load(Ordering::SeqCst)
            && let Some(local) = &self.local
        {
            local.feed(&samples);
        }
    }

    pub fn update_system_preview(&self, text: String) {
        if self.mode == DictationPreviewMode::System {
            let _ = self.state.publish(&text, false);
        }
    }

    pub fn stop_preview(&self) {
        self.state.stop(false);
        if let Some(local) = &self.local {
            local.stop();
        }
    }

    pub fn cancel(&self) {
        self.cleanup_verification.cancel();
        self.state.stop(true);
        if let Some(local) = &self.local {
            local.stop();
        }
    }

    pub fn finish(&self, request: DictationRequest) -> Result<DictationResult, PennelError> {
        self.finish_with(request, None)
    }

    pub fn finish_command(
        &self,
        request: DictationRequest,
        selection: Option<String>,
    ) -> Result<DictationResult, PennelError> {
        if self.practice {
            return Err(PennelError::InvalidResponse {
                message: "practice dictation does not support commands".into(),
            });
        }
        self.finish_with(request, Some(selection))
    }
}

impl Drop for LiveDictationSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiKeys, ModelChoice, ProviderAccount, ProviderKind, SystemTranscriber};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Observer(Mutex<Vec<DictationPreviewUpdate>>);

    impl DictationPreviewObserver for Observer {
        fn changed(&self, update: DictationPreviewUpdate) {
            self.0.lock().unwrap().push(update);
        }
    }

    struct Speech(Arc<AtomicUsize>);

    impl SystemTranscriber for Speech {
        fn transcribe(
            &self,
            _: Vec<f32>,
            _: u32,
            language: Option<String>,
            _: Vec<String>,
        ) -> Result<String, PennelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert_eq!(language.as_deref(), Some("nl"));
            Ok("Een echt dictaat.".into())
        }
    }

    fn engine() -> (tempfile::TempDir, Arc<PennelEngine>, Arc<AtomicUsize>) {
        let directory = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(directory.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "apple".into(),
            kind: ProviderKind::AppleSpeech,
            name: "Apple".into(),
            base_url: String::new(),
        });
        settings.dictation_stt = ModelChoice {
            provider_id: "apple".into(),
            model: "dictation".into(),
        };
        settings.language = Some("nl".into());
        settings.mode = DictationMode::Fast;
        engine.update_settings(settings).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        engine.set_system_transcriber(Arc::new(Speech(calls.clone())));
        (directory, engine, calls)
    }

    fn request() -> DictationRequest {
        DictationRequest {
            samples: vec![0.1; 16_000],
            sample_rate: 16_000,
            app_bundle_id: Some("test.editor".into()),
            keys: ApiKeys {
                by_provider: HashMap::new(),
            },
        }
    }

    #[test]
    fn preview_vocabulary_is_bounded_and_frozen_with_the_recording_settings() {
        let (_directory, engine, _) = engine();
        let mut settings = engine.settings();
        settings.dictionary = vec!["  Pennel   Mobile ".into(), "pennel mobile".into()];
        engine.update_settings(settings.clone()).unwrap();
        let session = engine
            .clone()
            .begin_dictation_session(Arc::new(Observer::default()), false);
        settings.dictionary = vec!["Changed later".into()];
        engine.update_settings(settings).unwrap();
        assert_eq!(session.vocabulary(), vec!["Pennel Mobile"]);
        session.cancel();
    }

    #[test]
    fn previews_only_replace_bounded_text_without_transcribing_or_storing_history() {
        let (_directory, engine, calls) = engine();
        let observer = Arc::new(Observer::default());
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        assert_eq!(session.mode(), DictationPreviewMode::System);
        session.update_system_preview("voorlopig".into());
        session.update_system_preview("voorlopig".into());
        session.update_system_preview("é🎙".repeat(200));
        let updates = observer.0.lock().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].session_id, session.id());
        assert_eq!(updates[0].revision, 1);
        assert_eq!(updates[1].revision, 2);
        assert_eq!(updates[1].text.chars().count(), MAX_PREVIEW_CHARS);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(engine.history(10).unwrap().is_empty());
    }

    #[test]
    fn cancellation_rejects_old_updates_and_finishing_without_side_effects() {
        let (_directory, engine, calls) = engine();
        let observer = Arc::new(Observer::default());
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        session.update_system_preview("voor annuleren".into());
        session.cancel();
        session.update_system_preview("oud resultaat".into());
        assert_eq!(session.finish(request()), Err(PennelError::Cancelled));
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(engine.history(10).unwrap().is_empty());
    }

    #[test]
    fn finishing_keeps_the_frozen_provider_language_and_only_inserts_once() {
        let (_directory, engine, calls) = engine();
        let observer = Arc::new(Observer::default());
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let mut changed = engine.settings();
        changed.dictation_stt.provider_id = "groq".into();
        changed.language = Some("en".into());
        engine.update_settings(changed).unwrap();
        session.stop_preview();
        let result = session.finish(request()).unwrap();
        assert_eq!(result.text, "Een echt dictaat.");
        assert_eq!(session.finish(request()).unwrap(), result);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(engine.history(10).unwrap().len(), 1);
        assert_eq!(observer.0.lock().unwrap().last().unwrap().text, result.raw);
    }

    #[test]
    fn practice_stays_fast_and_cannot_run_a_command() {
        let (_directory, engine, _) = engine();
        let mut settings = engine.settings();
        settings.mode = DictationMode::Clean;
        engine.update_settings(settings.clone()).unwrap();
        let session = engine
            .clone()
            .begin_dictation_session(Arc::new(Observer::default()), true);
        assert!(
            session
                .finish_command(request(), Some("selectie".into()))
                .is_err()
        );
        assert_eq!(
            session.finish(request()).unwrap().cleanup,
            crate::CleanupStatus::Skipped
        );
        assert_eq!(engine.settings(), settings);
    }

    #[test]
    fn separate_recordings_have_independent_ids_revisions_and_cancel_gates() {
        let (_directory, engine, _) = engine();
        let observer = Arc::new(Observer::default());
        let first = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let second = engine.begin_dictation_session(observer.clone(), false);
        assert_ne!(first.id(), second.id());
        first.update_system_preview("eerste".into());
        first.cancel();
        second.update_system_preview("tweede".into());
        first.update_system_preview("verouderd".into());
        let updates = observer.0.lock().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].session_id, first.id());
        assert_eq!(updates[1].session_id, second.id());
        assert_eq!(updates[1].revision, 1);
    }

    #[test]
    fn stopping_preview_keeps_the_caption_and_allows_only_final_raw_updates() {
        let (_directory, engine, _) = engine();
        let observer = Arc::new(Observer::default());
        let session = engine.begin_dictation_session(observer.clone(), false);
        session.update_system_preview("voorlopig".into());
        session.stop_preview();
        session.update_system_preview("te laat".into());
        assert_eq!(observer.0.lock().unwrap().last().unwrap().text, "voorlopig");
        session.finish(request()).unwrap();
        session.update_system_preview("ook te laat".into());
        let updates = observer.0.lock().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[1].text, "Een echt dictaat.");
    }

    struct LanguageModel {
        observer: Arc<Observer>,
        entered: Option<std::sync::mpsc::Sender<()>>,
        proceed: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    }

    impl crate::SystemLanguageModel for LanguageModel {
        fn complete(
            &self,
            _: Vec<crate::LanguageModelMessage>,
            _: u64,
        ) -> Result<String, PennelError> {
            assert!(
                !self
                    .observer
                    .0
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .text
                    .is_empty(),
                "raw preview precedes cleanup or command generation"
            );
            if let Some(entered) = &self.entered {
                entered.send(()).unwrap();
            }
            if let Some(proceed) = &self.proceed {
                proceed
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
            }
            Ok("Een echt dictaat.".into())
        }
    }

    fn choose_language_model(engine: &PennelEngine) {
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "apple-llm".into(),
            kind: ProviderKind::AppleIntelligence,
            name: "Apple Intelligence".into(),
            base_url: String::new(),
        });
        settings.llm = ModelChoice {
            provider_id: "apple-llm".into(),
            model: "system".into(),
        };
        settings.mode = DictationMode::Clean;
        engine.update_settings(settings).unwrap();
    }

    #[test]
    fn cloud_preview_makes_no_extra_requests_and_emits_raw_before_cleanup() {
        let (_directory, engine, _) = engine();
        let mut server = mockito::Server::new();
        let transcription = server
            .mock("POST", "/audio/transcriptions")
            .expect(1)
            .with_body(r#"{"text":"Een echt dictaat."}"#)
            .create();
        choose_language_model(&engine);
        let mut settings = engine.settings();
        settings
            .providers
            .iter_mut()
            .find(|account| account.id == "groq")
            .unwrap()
            .base_url = server.url();
        settings.dictation_stt = ModelChoice {
            provider_id: "groq".into(),
            model: "whisper-large-v3-turbo".into(),
        };
        engine.update_settings(settings).unwrap();
        let observer = Arc::new(Observer::default());
        engine.set_system_language_model(Arc::new(LanguageModel {
            observer: observer.clone(),
            entered: None,
            proceed: None,
        }));
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        assert_eq!(session.mode(), DictationPreviewMode::AfterRecording);
        session.feed_audio(vec![0.1; MAX_AUDIO_SAMPLES]);
        session.update_system_preview("mag geen gekozen provider omzeilen".into());
        assert!(observer.0.lock().unwrap().is_empty());
        assert!(engine.history(10).unwrap().is_empty());
        let mut request = request();
        request
            .keys
            .by_provider
            .insert("groq".into(), "synthetic-key".into());
        let result = session.finish(request).unwrap();
        assert_eq!(result.cleanup, crate::CleanupStatus::Applied);
        assert_eq!(observer.0.lock().unwrap().last().unwrap().text, result.raw);
        transcription.assert();
    }

    struct BlockingSpeech {
        entered: std::sync::mpsc::Sender<()>,
        proceed: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl SystemTranscriber for BlockingSpeech {
        fn transcribe(
            &self,
            _: Vec<f32>,
            _: u32,
            _: Option<String>,
            _: Vec<String>,
        ) -> Result<String, PennelError> {
            self.entered.send(()).unwrap();
            self.proceed
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok("Oud resultaat".into())
        }
    }

    #[test]
    fn cancellation_during_recognition_prevents_preview_cleanup_and_history() {
        let (_directory, engine, _) = engine();
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        engine.set_system_transcriber(Arc::new(BlockingSpeech {
            entered,
            proceed: Mutex::new(proceed),
        }));
        let observer = Arc::new(Observer::default());
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let running = session.clone();
        let worker = std::thread::spawn(move || running.finish(request()));
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        session.cancel();
        release.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Err(PennelError::Cancelled));
        assert!(observer.0.lock().unwrap().is_empty());
        assert!(engine.history(10).unwrap().is_empty());
    }

    #[test]
    fn cancellation_during_cleanup_does_not_commit_a_late_result() {
        let (_directory, engine, _) = engine();
        choose_language_model(&engine);
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let observer = Arc::new(Observer::default());
        engine.set_system_language_model(Arc::new(LanguageModel {
            observer: observer.clone(),
            entered: Some(entered),
            proceed: Some(Mutex::new(proceed)),
        }));
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let running = session.clone();
        let worker = std::thread::spawn(move || running.finish(request()));
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        session.cancel();
        release.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Err(PennelError::Cancelled));
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        assert!(engine.history(10).unwrap().is_empty());
    }

    #[test]
    fn command_uses_frozen_provider_and_emits_its_raw_instruction_first() {
        let (_directory, engine, _) = engine();
        choose_language_model(&engine);
        let observer = Arc::new(Observer::default());
        engine.set_system_language_model(Arc::new(LanguageModel {
            observer: observer.clone(),
            entered: None,
            proceed: None,
        }));
        let session = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let mut settings = engine.settings();
        settings.llm.provider_id = "groq".into();
        settings.dictation_stt.provider_id = "groq".into();
        engine.update_settings(settings).unwrap();
        let result = session
            .finish_command(request(), Some("Een selectie".into()))
            .unwrap();
        assert_eq!(result.raw, "Een echt dictaat.");
        assert_eq!(result.cleanup, crate::CleanupStatus::Applied);
        assert_eq!(engine.history(10).unwrap().len(), 1);
    }

    #[test]
    fn local_mailbox_keeps_one_latest_window_with_bounded_finite_audio() {
        let local = LocalPreview::default();
        local.feed(&vec![0.1; MAX_AUDIO_SAMPLES * 4]);
        local.feed(&vec![0.2; MAX_AUDIO_SAMPLES]);
        local.feed(&[f32::NAN, f32::INFINITY]);
        {
            let pending = local.pending.lock().unwrap();
            assert_eq!(pending.ring.len(), MAX_AUDIO_SAMPLES);
            assert_eq!(pending.latest.as_ref().unwrap().len(), MAX_AUDIO_SAMPLES);
            assert!(
                pending
                    .latest
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|sample| *sample == 0.2)
            );
        }
        local.stop();
        local.feed(&vec![0.3; MAX_AUDIO_SAMPLES]);
        let pending = local.pending.lock().unwrap();
        assert!(pending.ring.is_empty());
        assert!(pending.latest.is_none());
        assert!(local.cancel.is_cancelled());
    }

    #[test]
    fn local_worker_serializes_decodes_and_coalesces_pending_windows() {
        let observer = Arc::new(Observer::default());
        let state = Arc::new(PreviewState::new(observer));
        let local = Arc::new(LocalPreview::default());
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, proceed) = std::sync::mpsc::channel();
        let work = local.clone();
        let worker = std::thread::spawn(move || {
            let mut calls = 0;
            local_worker(work, state, |samples, _| {
                calls += 1;
                entered.send(samples.last().copied().unwrap()).unwrap();
                if calls == 1 {
                    proceed
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap();
                }
                Some(format!("venster {calls}"))
            });
            calls
        });
        local.feed(&vec![0.1; SAMPLE_RATE as usize]);
        assert_eq!(
            ready
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            0.1
        );
        for value in [0.2, 0.3, 0.4] {
            local.feed(&vec![value; SAMPLE_RATE as usize]);
        }
        release.send(()).unwrap();
        assert_eq!(
            ready
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            0.4
        );
        local.stop();
        assert_eq!(worker.join().unwrap(), 2);
    }

    #[test]
    fn cancelling_local_work_signals_the_token_and_discards_late_text() {
        let observer = Arc::new(Observer::default());
        let state = Arc::new(PreviewState::new(observer.clone()));
        let local = Arc::new(LocalPreview::default());
        let (entered, ready) = std::sync::mpsc::channel();
        let (work, worker_state) = (local.clone(), state.clone());
        let worker = std::thread::spawn(move || {
            local_worker(work, worker_state, |_, token| {
                entered.send(()).unwrap();
                let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !token.is_cancelled() && std::time::Instant::now() < until {
                    std::thread::yield_now();
                }
                assert!(token.is_cancelled());
                Some("laat modelresultaat".into())
            })
        });
        local.feed(&vec![0.1; SAMPLE_RATE as usize]);
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        state.stop(true);
        local.stop();
        worker.join().unwrap();
        assert!(observer.0.lock().unwrap().is_empty());
    }
}
