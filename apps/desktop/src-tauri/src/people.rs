use std::sync::Arc;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::state::Shared;

type Result<T> = std::result::Result<T, String>;

fn message(source: &str) -> String {
    crate::commands::localize_error_message(source)
}

fn decode(text: String) -> Result<Value> {
    serde_json::from_str(&text).map_err(|_| message("The participant response could not be read."))
}

fn snapshot(shared: &Shared, id: &str) -> Result<Value> {
    snapshot_with_sources(shared, id, calendar_source_available)
}

fn calendar_source_available(calendar_id: &str, source_id: Option<&str>) -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::calendar_source_available(calendar_id, source_id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (calendar_id, source_id);
        false
    }
}

fn snapshot_with_sources(
    shared: &Shared,
    id: &str,
    available: impl FnOnce(&str, Option<&str>) -> bool,
) -> Result<Value> {
    crate::commands::checked_meeting_dir(&shared.engine, id)?;
    let value = decode(
        shared
            .engine
            .meeting_people_snapshot_json(id.to_owned())
            .map_err(|error| crate::commands::localize_error_message(error.to_string()))?,
    )?;
    revalidate_calendar(shared, id, value, available)
}

fn revalidate_calendar(
    shared: &Shared,
    id: &str,
    value: Value,
    available: impl FnOnce(&str, Option<&str>) -> bool,
) -> Result<Value> {
    let Some(calendar) = value.get("calendar").filter(|value| value.is_object()) else {
        return Ok(value);
    };
    let Some(calendar_id) = calendar.get("calendar_id").and_then(Value::as_str) else {
        return Ok(value);
    };
    let source_id = calendar.get("source_id").and_then(Value::as_str);
    if available(calendar_id, source_id) {
        return Ok(value);
    }
    let Some(source_key) = source_id.or_else(|| calendar.get("source").and_then(Value::as_str))
    else {
        return Ok(value);
    };
    let result = decode(
        shared
            .engine
            .update_meeting_people_json(
                id.to_owned(),
                json!({"action":"invalidate_source","expected_revision":value["revision"],
                    "source_id":source_key,"account_id":calendar.get("account_id")})
                .to_string(),
            )
            .map_err(|error| crate::commands::localize_error_message(error.to_string()))?,
    )?;
    Ok(result["snapshot"].clone())
}

fn update(shared: &Shared, id: &str, request: Value, session_id: Option<&str>) -> Result<Value> {
    update_with_sources(shared, id, request, session_id, calendar_source_available)
}

fn update_with_sources(
    shared: &Shared,
    id: &str,
    request: Value,
    session_id: Option<&str>,
    available: impl Fn(&str, Option<&str>) -> bool,
) -> Result<Value> {
    crate::commands::checked_meeting_dir(&shared.engine, id)?;
    if session_id.is_none()
        && shared
            .meeting_context
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|context| context.session.id() == id)
    {
        return Err(message(
            "Return to the recording to change its participants.",
        ));
    }
    let apply = || {
        // A source can disappear while its picker is open. Invalidation advances the revision,
        // so the pending action must refresh rather than apply an old identity.
        snapshot_with_sources(shared, id, &available)?;
        decode(
            shared
                .engine
                .update_meeting_people_json(id.to_owned(), request.to_string())
                .map_err(|error| crate::commands::localize_error_message(error.to_string()))?,
        )
    };
    if let Some(session_id) = session_id {
        let apply_to_session = |session: &Arc<pennel_core::MeetingSession>| {
            if session.id() != id {
                return Err(message(
                    "This participant action belongs to another meeting.",
                ));
            }
            apply()
        };
        if request.get("action").and_then(Value::as_str) == Some("insert_mention") {
            crate::meeting_runtime::with_session_notes(shared, session_id, apply_to_session)
        } else {
            crate::meeting_runtime::with_session_snapshot(shared, session_id, apply_to_session)
        }
    } else {
        apply()
    }
}

#[tauri::command]
pub async fn meeting_people_snapshot(id: String, shared: State<'_, Arc<Shared>>) -> Result<Value> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || snapshot(&shared, &id))
        .await
        .map_err(|_| message("The participant worker stopped unexpectedly."))?
}

#[tauri::command]
pub async fn update_meeting_people(
    id: String,
    request: Value,
    session_id: Option<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    let shared = shared.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        update(&shared, &id, request, session_id.as_deref())
    })
    .await
    .map_err(|_| message("The participant worker stopped unexpectedly."))??;
    if let Some(snapshot) = result.get("snapshot") {
        let _ = app.emit_to(
            "main",
            "meeting-people-changed",
            json!({"meeting_uid":snapshot.get("meeting_uid"),"revision":snapshot.get("revision")}),
        );
    }
    if result.get("notes").is_some() {
        let _ = app.emit("library-changed", ());
        crate::cloud::poke(&app);
    }
    Ok(result)
}

#[tauri::command]
pub async fn pick_meeting_contact(window: tauri::WebviewWindow) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            crate::macos::pick_meeting_contact(&window)
                .map_err(|error| message(&error))
                .map(|contact| match contact {
                    Some(contact) => json!({"status":"selected","contact":contact}),
                    None => json!({"status":"cancelled"}),
                })
        })
        .await
        .map_err(|_| message("The contact picker was interrupted."))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Ok(
            json!({"status":"unavailable","reason":message("System contact selection is unavailable on this platform.")}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_routes_reject_paths_outside_the_meeting_library() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        assert!(snapshot(&shared, "../private").is_err());
        assert!(update(&shared, "../private", json!({}), None).is_err());
        assert!(!root.path().join("private").exists());
    }

    #[test]
    fn a_stale_recording_cannot_create_a_person_in_the_selected_meeting() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "People fixture".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        let id = session.id();
        let before = snapshot(&shared, &id).unwrap();
        assert!(update(
            &shared,
            &id,
            json!({"expected_revision":before["revision"],"action":"create_person","display_name":"Synthetic guest"}),
            Some("replaced-recording"),
        ).is_err());
        let after = snapshot(&shared, &id).unwrap();
        assert_eq!(after["revision"], before["revision"]);
        assert!(after["people"].as_array().unwrap().is_empty());
        session.stop().unwrap();
    }

    #[test]
    fn lost_calendar_access_removes_private_enrichment_and_keeps_authored_text() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Calendar fixture".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        let id = session.id();
        session
            .set_notes("Authored @Guest remains.".into())
            .unwrap();
        let before = decode(shared.engine.set_meeting_calendar_context_json(id.clone(), json!({
            "source":"eventkit","source_id":"synthetic-account","calendar_id":"synthetic-calendar",
            "event_id":"synthetic-event","instance_id":"synthetic-instance","source_revision":"r1",
            "participants":[{"source_id":"mailto:guest@example.test","name":"Guest","email":"guest@example.test"}]
        }).to_string()).unwrap()).unwrap();
        let person_id = before["people"][0]["id"].clone();
        let after = revalidate_calendar(&shared, &id, before, |_, _| false).unwrap();
        assert_eq!(after["people"][0]["id"], person_id);
        assert_eq!(after["people"][0]["source_status"], "unavailable");
        assert!(after["people"][0]["email"].is_null());
        assert!(after["calendar"].is_null());
        assert_eq!(shared.engine.meeting_notes(id), "Authored @Guest remains.");
        session.stop().unwrap();
    }

    #[test]
    fn calendar_source_loss_after_loading_rejects_the_stale_update() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Late calendar fixture".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        let id = session.id();
        let before = decode(shared.engine.set_meeting_calendar_context_json(id.clone(), json!({
            "source":"eventkit","source_id":"deleted-synthetic-account","calendar_id":"deleted-synthetic-calendar",
            "event_id":"synthetic-event","source_revision":"r1",
            "participants":[{"source_id":"mailto:guest@example.test","name":"Guest","email":"guest@example.test"}]
        }).to_string()).unwrap()).unwrap();
        assert!(update_with_sources(&shared, &id, json!({"expected_revision":before["revision"],
            "action":"rename_person","person_id":before["people"][0]["id"],"display_name":"Stale update"}), None, |_, _| false).is_err());
        let after = decode(shared.engine.meeting_people_snapshot_json(id).unwrap()).unwrap();
        assert_eq!(after["people"][0]["display_name"], "Guest");
        assert_eq!(after["people"][0]["source_status"], "unavailable");
        assert!(after["people"][0]["email"].is_null());
        session.stop().unwrap();
    }

    #[test]
    fn missing_source_identity_still_checks_calendar_authorization() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Unknown calendar source".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        let id = session.id();
        let before = decode(shared.engine.set_meeting_calendar_context_json(id.clone(), json!({
            "source":"eventkit","source_id":null,"calendar_id":"synthetic-calendar",
            "source_revision":"r1","participants":[{"source_id":"mailto:guest@example.test","name":"Guest","email":"guest@example.test"}]
        }).to_string()).unwrap()).unwrap();
        let after = revalidate_calendar(&shared, &id, before, |calendar, source| {
            assert_eq!(calendar, "synthetic-calendar");
            assert_eq!(source, None);
            false
        })
        .unwrap();
        assert!(after["calendar"].is_null());
        assert_eq!(after["people"][0]["source_status"], "unavailable");
        assert!(after["people"][0]["source"].is_null());
        session.stop().unwrap();
    }
}
