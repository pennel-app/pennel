//! Thin desktop adapters for the core-owned local conversation service.
use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::state::Shared;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy)]
enum Operation {
    Snapshot,
    Update,
    Begin,
    Cancel,
    Source,
}

fn message(source: impl AsRef<str>) -> String {
    crate::commands::localize_error_message(source)
}

fn checked_scope(shared: &Shared, request: &Value) -> Result<()> {
    if let Some(uid) = request
        .get("scope")
        .and_then(|scope| scope.get("meeting_uid"))
        .and_then(Value::as_str)
        && let Some(current) = shared.engine.meeting_id_for_uid(uid)
    {
        crate::commands::checked_meeting_dir(&shared.engine, &current)?;
        return Ok(());
    }
    if let Some(id) = request
        .get("scope")
        .and_then(|scope| scope.get("meeting_id"))
        .and_then(Value::as_str)
    {
        crate::commands::checked_meeting_dir(&shared.engine, id)?;
    }
    Ok(())
}

fn admit_question(shared: &Shared, request: &mut Value) -> Result<()> {
    checked_scope(shared, request)?;
    let token = request
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let scope = request.get("scope");
    let uid = scope
        .and_then(|scope| scope.get("meeting_uid"))
        .and_then(Value::as_str);
    let active = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .map(|context| (context.uid.clone(), context.token.clone()));
    if let Some((active_uid, active_token)) = active {
        let for_active_meeting = uid == Some(active_uid.as_str());
        let claims_active_token = token.as_deref() == Some(active_token.as_str());
        if for_active_meeting && !claims_active_token || claims_active_token && !for_active_meeting
        {
            return Err(message("This question belongs to another meeting session."));
        }
        if for_active_meeting {
            let revision = request
                .get("expected_notes_revision")
                .and_then(Value::as_str)
                .filter(|revision| !revision.is_empty())
                .ok_or_else(|| message("Save your meeting notes before asking this question."))?;
            crate::meeting_runtime::with_session_snapshot(shared, &active_token, |session| {
                let notes = shared
                    .engine
                    .meeting_notes_checked(&session.id())
                    .map_err(|error| message(error.to_string()))?;
                if pennel_core::evidence_revision(&notes) != revision {
                    return Err(message(
                        "The saved notes changed. Your question was not sent; save your notes and try again.",
                    ));
                }
                Ok(())
            })?;
        }
    }
    // The desktop lifecycle token is not the core session's preview UUID. Core captures its
    // own live-session identity; carrying the UI token across that boundary would confuse them.
    if let Some(object) = request.as_object_mut() {
        object.remove("session_id");
    }
    Ok(())
}

fn dispatch(shared: &Shared, operation: Operation, mut request: Value) -> Result<Value> {
    if matches!(operation, Operation::Begin) {
        admit_question(shared, &mut request)?;
    } else {
        checked_scope(shared, &request)?;
    }
    let request = request.to_string();
    let result = match operation {
        Operation::Snapshot => shared.engine.conversation_snapshot_json(request),
        Operation::Update => shared.engine.update_conversation_json(request),
        Operation::Begin => shared
            .engine
            .clone()
            .begin_conversation_question_json(request, shared.keys()),
        Operation::Cancel => shared.engine.cancel_conversation_question_json(request),
        Operation::Source => shared.engine.conversation_source_json(request),
    }
    .map_err(|error| message(error.to_string()))?;
    serde_json::from_str(&result)
        .map_err(|_| message("The conversation response could not be read."))
}

async fn response(shared: Arc<Shared>, operation: Operation, request: Value) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || dispatch(&shared, operation, request))
        .await
        .map_err(|_| message("The conversation worker stopped unexpectedly."))?
}

#[tauri::command]
pub async fn conversation_snapshot(
    request: Value,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    response(shared.inner().clone(), Operation::Snapshot, request).await
}

#[tauri::command]
pub async fn update_conversation(request: Value, shared: State<'_, Arc<Shared>>) -> Result<Value> {
    response(shared.inner().clone(), Operation::Update, request).await
}

#[tauri::command]
pub async fn begin_conversation_question(
    request: Value,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    response(shared.inner().clone(), Operation::Begin, request).await
}

#[tauri::command]
pub async fn cancel_conversation_question(
    request: Value,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    response(shared.inner().clone(), Operation::Cancel, request).await
}

#[tauri::command]
pub async fn conversation_source(request: Value, shared: State<'_, Arc<Shared>>) -> Result<Value> {
    response(shared.inner().clone(), Operation::Source, request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_folder_hints_cannot_escape_the_configured_library() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let request =
            json!({"scope":{"kind":"meeting","meeting_id":"../outside","meeting_uid":"old"}});
        assert!(checked_scope(&shared, &request).is_err());
        assert!(!root.path().join("outside").exists());
    }

    #[test]
    fn desktop_session_tokens_do_not_cross_the_core_identity_boundary() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let mut request = json!({"scope":{"kind":"library"},"session_id":"expired-ui-token","question":"Fixture"});
        admit_question(&shared, &mut request).unwrap();
        assert!(request.get("session_id").is_none());
        assert_eq!(request["question"], "Fixture");
    }

    #[test]
    fn a_stable_scope_resolves_after_its_initial_folder_hint_was_renamed() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Before rename".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        let old_id = session.id();
        let uid = shared.engine.meeting_uid(old_id.clone()).unwrap();
        session.stop().unwrap();
        let new_id = shared
            .engine
            .rename_meeting(old_id.clone(), "After rename".into())
            .unwrap();
        assert_ne!(old_id, new_id);
        checked_scope(
            &shared,
            &json!({"scope":{"kind":"meeting","meeting_uid":uid,"meeting_id":old_id}}),
        )
        .unwrap();
    }

    #[test]
    fn live_admission_requires_the_original_session_and_acknowledged_notes() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Conversation admission".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        session
            .set_notes("Acknowledged synthetic notes".into())
            .unwrap();
        let uid = shared.engine.meeting_uid(session.id()).unwrap();
        let token = shared.recording.lock().unwrap().begin(None).unwrap();
        shared.recording.lock().unwrap().recording(&token).unwrap();
        *shared.meeting_context.lock().unwrap() = Some(crate::meeting_runtime::MeetingContext {
            token: token.clone(),
            uid: uid.clone(),
            session: session.clone(),
            notes_gate: Arc::new(std::sync::Mutex::new(true)),
            title: "Conversation admission".into(),
            started: std::time::Instant::now(),
            duration_secs: None,
            pending_recording_finish: None,
            system_audio: false,
            heard_system_audio: false,
            audio_warning: None,
            consent_copied: false,
            speaker_evidence: None,
        });
        let mut request = json!({"scope":{"kind":"meeting","meeting_uid":uid},
            "session_id":"replaced-session","expected_notes_revision":pennel_core::evidence_revision("Acknowledged synthetic notes")});
        assert!(admit_question(&shared, &mut request).is_err());
        request["session_id"] = json!(token);
        request["expected_notes_revision"] = json!(pennel_core::evidence_revision("Unsaved edit"));
        assert!(admit_question(&shared, &mut request).is_err());
        request["expected_notes_revision"] = json!(pennel_core::evidence_revision(
            "Acknowledged synthetic notes"
        ));
        admit_question(&shared, &mut request).unwrap();
        assert!(request.get("session_id").is_none());
        assert_eq!(request["scope"]["meeting_uid"], uid);
        session.stop().unwrap();
    }
}
