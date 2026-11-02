//! A live question owns its evidence independently of recording and window lifetime.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pennel_core::{
    ChatTurn, InferenceCancellation, MeetingAnswer, MeetingEvidence, MeetingSourceAnchor,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::meeting_runtime::{LibraryJob, with_session_snapshot};
use crate::state::Shared;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuestionIdentity {
    request_id: String,
    meeting_uid: String,
    session_id: String,
}

impl QuestionIdentity {
    fn conversation(&self) -> (String, String) {
        (self.meeting_uid.clone(), self.session_id.clone())
    }

    fn validate(&self) -> Result<()> {
        if [&self.request_id, &self.meeting_uid, &self.session_id]
            .iter()
            .any(|value| value.is_empty() || value.len() > 256)
        {
            return Err("This question has no valid meeting session.".into());
        }
        Ok(())
    }
}

struct ActiveQuestion {
    identity: QuestionIdentity,
    cancellation: Arc<InferenceCancellation>,
}

#[derive(Default)]
pub struct LiveQuestions {
    requests: HashMap<String, ActiveQuestion>,
    conversations: HashMap<(String, String), String>,
    used_ids: HashSet<String>,
}

impl LiveQuestions {
    fn reserve(&mut self, identity: QuestionIdentity) -> Result<Arc<InferenceCancellation>> {
        identity.validate()?;
        if self.used_ids.contains(&identity.request_id) {
            return Err("This question request is no longer available. Try again.".into());
        }
        if self.conversations.contains_key(&identity.conversation()) {
            return Err(
                "A question is already running for this meeting. Cancel it or wait for its answer."
                    .into(),
            );
        }
        let cancellation = InferenceCancellation::new();
        self.used_ids.insert(identity.request_id.clone());
        self.conversations
            .insert(identity.conversation(), identity.request_id.clone());
        self.requests.insert(
            identity.request_id.clone(),
            ActiveQuestion {
                identity,
                cancellation: cancellation.clone(),
            },
        );
        Ok(cancellation)
    }

    fn cancel(&mut self, identity: &QuestionIdentity) -> bool {
        if let Some(request) = self.requests.get(&identity.request_id) {
            if request.identity != *identity {
                return false;
            }
            request.cancellation.cancel();
            self.requests.remove(&identity.request_id);
            if self.conversations.get(&identity.conversation()) == Some(&identity.request_id) {
                self.conversations.remove(&identity.conversation());
            }
            return true;
        }
        // Cancellation can race begin's IPC admission. Never reuse that request ID.
        self.used_ids.insert(identity.request_id.clone());
        false
    }

    fn finish(&mut self, identity: &QuestionIdentity) -> bool {
        let Some(request) = self.requests.get(&identity.request_id) else {
            return false;
        };
        if request.identity != *identity {
            return false;
        }
        let accepted = !request.cancellation.is_cancelled();
        self.requests.remove(&identity.request_id);
        if self.conversations.get(&identity.conversation()) == Some(&identity.request_id) {
            self.conversations.remove(&identity.conversation());
        }
        accepted
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveQuestionSnapshot {
    pub request_id: String,
    pub meeting_uid: String,
    pub session_id: String,
    pub transcript_revision: u64,
    pub notes_revision: String,
    pub included_through_ms: u64,
    pub pending_transcription: u32,
    pub anchors: Vec<MeetingSourceAnchor>,
}

#[derive(Clone, Serialize)]
struct LiveQuestionResult {
    snapshot: LiveQuestionSnapshot,
    answer: Option<MeetingAnswer>,
    error: Option<String>,
}

fn verify_notes(shared: &Shared, id: &str, expected_notes: &str) -> Result<String> {
    let notes = shared
        .engine
        .meeting_notes_checked(id)
        .map_err(|error| error.to_string())?;
    if notes != expected_notes {
        return Err(
            "The saved notes changed. Your question was not sent; save your notes and try again."
                .into(),
        );
    }
    Ok(notes)
}

fn capture_evidence(
    shared: &Shared,
    identity: &QuestionIdentity,
    expected_notes: &str,
) -> Result<(MeetingEvidence, u64, u32)> {
    let active = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .filter(|context| context.uid == identity.meeting_uid)
        .map(|context| context.token.clone());
    if let Some(token) = active {
        if token != identity.session_id {
            return Err("This question belongs to another meeting session.".into());
        }
        let live = with_session_snapshot(shared, &identity.session_id, |session| {
            let id = session.id();
            if shared.engine.meeting_uid(id.clone()).as_deref()
                != Some(identity.meeting_uid.as_str())
            {
                return Err("The meeting source is no longer available.".into());
            }
            let notes = verify_notes(shared, &id, expected_notes)?;
            let (revision, transcript) = session.transcript_snapshot();
            let info = shared
                .engine
                .meeting_info(id.clone())
                .ok_or("The meeting source is no longer available.")?;
            let names = shared.engine.meeting_speakers(id.clone());
            let evidence = MeetingEvidence::new_with_speakers(
                &identity.meeting_uid,
                &info.title,
                &info.start_iso,
                &notes,
                &transcript,
                &names,
            );
            verify_notes(shared, &id, expected_notes)?;
            Ok((evidence, revision, session.pending()))
        });
        if live.is_ok()
            || shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|context| {
                    context.uid == identity.meeting_uid && context.token == identity.session_id
                })
        {
            return live;
        }
        // Finalization may have moved the directory while admission waited for its read gate.
    }
    let id = shared
        .engine
        .meeting_id_for_uid(&identity.meeting_uid)
        .ok_or("The meeting source is no longer available.")?;
    let notes = verify_notes(shared, &id, expected_notes)?;
    let transcript = shared.engine.meeting_transcript(id.clone());
    let info = shared
        .engine
        .meeting_info(id.clone())
        .ok_or("The meeting source is no longer available.")?;
    if shared.engine.meeting_uid(id.clone()).as_deref() != Some(identity.meeting_uid.as_str()) {
        return Err("The meeting source changed. Your question was not sent.".into());
    }
    verify_notes(shared, &id, expected_notes)?;
    let revision = pennel_core::evidence_revision(
        &serde_json::to_string(&transcript).map_err(|_| "The meeting source is unavailable.")?,
    );
    // Twelve hex digits retain exact integer representation in the JavaScript bridge.
    let revision = u64::from_str_radix(&revision[..12], 16).unwrap();
    Ok((
        MeetingEvidence::new_with_speakers(
            &identity.meeting_uid,
            &info.title,
            &info.start_iso,
            &notes,
            &transcript,
            &shared.engine.meeting_speakers(id),
        ),
        revision,
        0,
    ))
}

#[tauri::command]
// Tauri injects app/state; the caller supplies the six identity, evidence and question fields.
#[allow(clippy::too_many_arguments)]
pub async fn begin_live_question(
    request_id: String,
    meeting_uid: String,
    session_id: String,
    expected_notes_text: String,
    question: String,
    history: Vec<ChatTurn>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<LiveQuestionSnapshot> {
    let shared = shared.inner().clone();
    let identity = QuestionIdentity {
        request_id,
        meeting_uid,
        session_id,
    };
    let cancellation = shared
        .live_questions
        .lock()
        .unwrap()
        .reserve(identity.clone())?;
    let retained = shared.clone();
    let retained_identity = identity.clone();
    let response = tauri::async_runtime::spawn_blocking(move || {
        let library_job = LibraryJob::new(&shared)?;
        if cancellation.is_cancelled() {
            return Err("Question cancelled.".into());
        }
        let (evidence, revision, pending) =
            capture_evidence(&shared, &identity, &expected_notes_text)?;
        let prepared = shared
            .engine
            .prepare_meeting_evidence_question(question, history, &evidence, shared.keys())
            .map_err(|error| crate::commands::localize_error_message(error.to_string()))?;
        if cancellation.is_cancelled() {
            return Err("Question cancelled.".into());
        }
        let anchors = evidence
            .anchors_for_sources(prepared.sources())
            .map_err(|error| crate::commands::localize_error_message(error.to_string()))?;
        let snapshot = LiveQuestionSnapshot {
            request_id: identity.request_id.clone(),
            meeting_uid: identity.meeting_uid.clone(),
            session_id: identity.session_id.clone(),
            transcript_revision: revision,
            notes_revision: evidence.notes_revision,
            included_through_ms: evidence.included_through_ms,
            pending_transcription: pending,
            anchors,
        };
        let output_snapshot = snapshot.clone();
        std::thread::Builder::new()
            .name("pennel-meeting-question".into())
            .spawn(move || {
                let result = prepared.complete(cancellation, library_job);
                if !shared.live_questions.lock().unwrap().finish(&identity) {
                    return;
                }
                let event = match result {
                    Ok(answer) => LiveQuestionResult {
                        snapshot,
                        answer: Some(answer),
                        error: None,
                    },
                    Err(error) => LiveQuestionResult {
                        snapshot,
                        answer: None,
                        error: Some(crate::commands::localize_error_message(error.to_string())),
                    },
                };
                let _ = app.emit_to("main", "live-question-result", event);
            })
            .map_err(|_| "The question could not start. Try again.".to_owned())?;
        Ok(output_snapshot)
    })
    .await
    .unwrap_or_else(|_| Err("The question could not start. Try again.".to_owned()));
    if response.is_err() {
        retained
            .live_questions
            .lock()
            .unwrap()
            .finish(&retained_identity);
    }
    response
}

#[tauri::command]
pub fn cancel_live_question(
    request_id: String,
    meeting_uid: String,
    session_id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let identity = QuestionIdentity {
        request_id,
        meeting_uid,
        session_id,
    };
    identity.validate()?;
    shared.live_questions.lock().unwrap().cancel(&identity);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(request: &str, uid: &str, session: &str) -> QuestionIdentity {
        QuestionIdentity {
            request_id: request.into(),
            meeting_uid: uid.into(),
            session_id: session.into(),
        }
    }

    #[test]
    fn cancellation_and_late_completion_are_bound_to_the_original_request() {
        let mut registry = LiveQuestions::default();
        let first = identity("one", "meeting-one", "session-one");
        let cancellation = registry.reserve(first.clone()).unwrap();
        assert!(
            registry
                .reserve(identity("other", "meeting-one", "session-one"))
                .is_err()
        );
        assert!(!registry.cancel(&identity("one", "meeting-two", "session-two")));
        assert!(!cancellation.is_cancelled());
        assert!(registry.cancel(&first));
        assert!(cancellation.is_cancelled());
        registry
            .reserve(identity("two", "meeting-one", "session-one"))
            .unwrap();
        assert!(!registry.finish(&first));
        assert!(registry.finish(&identity("two", "meeting-one", "session-one")));
        assert!(registry.reserve(first).is_err());
    }

    #[test]
    fn cancelling_before_admission_prevents_provider_dispatch() {
        let mut registry = LiveQuestions::default();
        let request = identity("early", "meeting-one", "session-one");
        registry.cancel(&request);
        assert!(registry.reserve(request).is_err());
    }

    #[test]
    fn next_meeting_does_not_steal_an_older_completion() {
        let mut registry = LiveQuestions::default();
        let old = identity("one", "meeting-one", "session-one");
        registry.reserve(old.clone()).unwrap();
        let new = identity("two", "meeting-two", "session-two");
        registry.reserve(new.clone()).unwrap();
        assert!(registry.finish(&old));
        assert!(registry.finish(&new));
    }

    fn saved_meeting(shared: &Arc<Shared>, title: &str, notes: &str) -> String {
        let session = shared
            .engine
            .clone()
            .start_meeting(
                title.into(),
                pennel_core::ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        session.set_notes(notes.into()).unwrap();
        session.stop().unwrap();
        session.id()
    }

    #[test]
    fn saved_snapshot_survives_later_writes_and_never_reads_other_meetings() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let id = saved_meeting(&shared, "First", "First notes");
        let uid = shared.engine.meeting_uid(id.clone()).unwrap();
        let second = saved_meeting(&shared, "Second", "Other meeting secret");
        std::fs::write(
            std::path::PathBuf::from(shared.engine.meeting_dir(id.clone())).join("transcript.md"),
            "[00:01] **Ik:** Frozen speech\n",
        )
        .unwrap();
        let request = identity("one", &uid, "original-session");
        let (snapshot, _, _) = capture_evidence(&shared, &request, "First notes").unwrap();
        shared
            .engine
            .set_meeting_notes(id.clone(), "Later notes".into())
            .unwrap();
        std::fs::write(
            std::path::PathBuf::from(shared.engine.meeting_dir(id)).join("transcript.md"),
            "[00:01] **Ik:** Frozen speech\n[00:20] **Ik:** Later speech\n",
        )
        .unwrap();
        let text = snapshot
            .sources
            .iter()
            .map(|source| source.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("First notes"));
        assert!(text.contains("Frozen speech"));
        assert!(!text.contains("Other meeting secret"));
        assert!(!text.contains("Later"));
        assert!(
            snapshot
                .sources
                .iter()
                .all(|source| source.meeting_id == uid)
        );
        assert!(shared.engine.meeting_info(second).is_some());
    }

    #[test]
    fn changed_or_unreadable_notes_prevent_snapshot_admission() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let id = saved_meeting(&shared, "First", "Saved notes");
        let uid = shared.engine.meeting_uid(id.clone()).unwrap();
        let request = identity("one", &uid, "original-session");
        assert!(capture_evidence(&shared, &request, "Unsaved draft").is_err());
        let notes = std::path::PathBuf::from(shared.engine.meeting_dir(id)).join("notes.md");
        std::fs::remove_file(&notes).unwrap();
        std::fs::create_dir(&notes).unwrap();
        assert!(capture_evidence(&shared, &request, "").is_err());
    }

    #[test]
    fn active_snapshot_uses_session_revision_and_review_keeps_the_uid_after_rename() {
        use std::sync::Mutex;
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Before".into(),
                pennel_core::ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        session.set_notes("Saved active notes".into()).unwrap();
        let uid = shared.engine.meeting_uid(session.id()).unwrap();
        let token = shared.recording.lock().unwrap().begin(None).unwrap();
        shared.recording.lock().unwrap().recording(&token).unwrap();
        let gate = Arc::new(Mutex::new(true));
        *shared.meeting_context.lock().unwrap() = Some(crate::meeting_runtime::MeetingContext {
            token: token.clone(),
            uid: uid.clone(),
            session: session.clone(),
            notes_gate: gate.clone(),
            title: "Before".into(),
            started: std::time::Instant::now(),
            duration_secs: None,
            pending_recording_finish: None,
            system_audio: false,
            heard_system_audio: false,
            audio_warning: None,
            consent_copied: false,
        });
        let request = identity("one", &uid, &token);
        let (active, revision, _) =
            capture_evidence(&shared, &request, "Saved active notes").unwrap();
        assert_eq!(revision, session.transcript_revision());
        assert_eq!(
            active.notes_revision,
            pennel_core::evidence_revision("Saved active notes")
        );
        assert!(
            capture_evidence(
                &shared,
                &identity("wrong", &uid, "another-session"),
                "Saved active notes"
            )
            .is_err()
        );
        *gate.lock().unwrap() = false;
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        assert!(crate::meeting_runtime::with_session_notes(&shared, &token, |_| Ok(())).is_err());
        let (finalizing, _, _) = capture_evidence(&shared, &request, "Saved active notes").unwrap();
        assert_eq!(finalizing.notes_revision, active.notes_revision);
        assert!(capture_evidence(&shared, &request, "Unsaved during finalization").is_err());
        session.stop().unwrap();
        shared.recording.lock().unwrap().complete(&token).unwrap();
        *shared.meeting_context.lock().unwrap() = None;
        let renamed = shared
            .engine
            .rename_meeting(session.id(), "After".into())
            .unwrap();
        assert_ne!(renamed, session.id());
        let (review, _, _) = capture_evidence(&shared, &request, "Saved active notes").unwrap();
        assert_eq!(review.sources[0].meeting_id, uid);
        assert_eq!(review.sources[0].title, "After");
        assert_eq!(review.notes_revision, active.notes_revision);
    }
}
