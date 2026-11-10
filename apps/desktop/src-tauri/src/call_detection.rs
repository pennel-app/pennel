//! Native observers report activity; shared Rust rules decide whether to offer or start recording.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pennel_core::{AppRule, CallDetector, CallEvent, CallObservation, RecordAction};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::diagnostics::Log;
use crate::state::Shared;

const LOG: Log = Log::new("calls");

#[derive(Clone, Serialize)]
pub struct RecordPrompt {
    pub id: u64,
    pub app_id: String,
    pub app_name: String,
    pub title: Option<String>,
    pub expires_at_ms: u64,
    #[serde(skip)]
    attendees: Vec<String>,
    #[serde(skip)]
    calendar_context: Option<serde_json::Value>,
    #[serde(skip)]
    detected_call: DetectedCall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetectedCall {
    pub(crate) app_id: String,
    revision: u64,
}

#[derive(Default)]
pub struct DetectionState {
    pub prompt: Mutex<Option<RecordPrompt>>,
    active: Mutex<Option<DetectedCall>>,
    next_revision: AtomicU64,
    clean_end_reset: Mutex<Option<DetectedCall>>,
}

impl DetectionState {
    fn begin_episode(&self, app_id: String) -> DetectedCall {
        let call = DetectedCall {
            app_id,
            revision: self.next_revision.fetch_add(1, Ordering::SeqCst) + 1,
        };
        *self.active.lock().unwrap() = Some(call.clone());
        call
    }

    fn matches(&self, call: &DetectedCall) -> bool {
        self.active.lock().unwrap().as_ref() == Some(call)
    }

    fn finish_episode(&self, call: &DetectedCall) -> bool {
        let mut active = self.active.lock().unwrap();
        if active.as_ref() != Some(call) {
            return false;
        }
        active.take();
        *self.clean_end_reset.lock().unwrap() = Some(call.clone());
        true
    }
}

/// Called only after the runtime accepted a clean stop for the original coordinator session.
pub(crate) fn recorded_call_ended(app: &AppHandle, call: &DetectedCall) {
    if let Some(state) = app.try_state::<DetectionState>() {
        // The accepted recording's prompt was already dismissed. A delayed completion must
        // never emit a dismissal that could hide a replacement episode's prompt.
        state.finish_episode(call);
    }
}

fn reset_ended_episode(
    detector: &mut CallDetector,
    active: &mut Option<DetectedCall>,
    ended: &DetectedCall,
    own_app_id: &str,
) -> bool {
    if active.as_ref() != Some(ended) || detector.active_call() != Some(ended.app_id.as_str()) {
        return false;
    }
    *detector = CallDetector::new(own_app_id);
    *active = None;
    true
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn snapshot() -> Result<Vec<CallObservation>, String> {
    #[cfg(target_os = "macos")]
    return crate::macos::call_snapshot(
        &pennel_core::known_call_apps(),
        &pennel_core::browser_apps(),
    )
    .map(|users| {
        users
            .into_iter()
            .map(|user| CallObservation {
                app_id: user.app_id,
                is_call: user.is_call,
                is_playing: user.is_playing,
            })
            .collect()
    });
    #[cfg(windows)]
    return crate::windows_meetings::snapshot();
    #[cfg(target_os = "linux")]
    return crate::linux_meetings::snapshot();
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    Ok(Vec::new())
}

pub fn calendar() -> serde_json::Value {
    #[cfg(target_os = "macos")]
    return crate::macos::calendar_snapshot();
    #[cfg(not(target_os = "macos"))]
    serde_json::json!({ "calendars": [], "current_event": null })
}

fn dismiss(app: &AppHandle) {
    if let Some(state) = app.try_state::<DetectionState>() {
        state.prompt.lock().unwrap().take();
    }
    let _ = app.emit("record-prompt", Option::<RecordPrompt>::None);
    if let Some(window) = app.get_webview_window("record-prompt") {
        crate::pill::hide_without_focus(&window);
    }
}

fn revalidated_calendar_context(
    saved: Option<serde_json::Value>,
    current: &serde_json::Value,
) -> Option<serde_json::Value> {
    let saved = saved?;
    if current.get("has_access").and_then(|value| value.as_bool()) != Some(true)
        || saved
            .get("source_revision")
            .and_then(|value| value.as_str())
            .is_none_or(str::is_empty)
        || current.get("current_event")?.get("calendar_context")? != &saved
    {
        return None;
    }
    Some(saved)
}

fn offer(app: &AppHandle, shared: &Arc<Shared>, detected_call: DetectedCall) {
    if shared.updating.load(Ordering::SeqCst)
        || shared.quitting.load(Ordering::SeqCst)
        || shared.recording.lock().unwrap().is_busy()
    {
        return;
    }
    let settings = shared.settings();
    let app_id = detected_call.app_id.clone();
    let calendar = if settings.calendar_enabled {
        calendar()
    } else {
        serde_json::Value::Null
    };
    let event = calendar.get("current_event");
    let title = event
        .and_then(|event| event.get("title"))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let calendar_id = event
        .and_then(|event| event.get("calendar_id"))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let attendees = event
        .and_then(|event| event.get("attendees"))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let calendar_context = event
        .and_then(|event| event.get("calendar_context"))
        .filter(|value| value.is_object())
        .cloned();
    match pennel_core::decide_recording(settings, app_id.clone(), calendar_id, title.clone()) {
        RecordAction::Never => {}
        RecordAction::Always => {
            if crate::meeting_runtime::start_detected_with_calendar(
                app,
                shared,
                title.unwrap_or_else(|| "Meeting".into()),
                detected_call,
                attendees,
                calendar_context,
            )
            .is_ok()
            {
                let _ = app.emit("meeting-updated", ());
            } else {
                LOG.error("Automatic recording could not start; open Pennel to check the recording status");
            }
        }
        RecordAction::Ask => {
            static NEXT_PROMPT: AtomicU64 = AtomicU64::new(1);
            let prompt = RecordPrompt {
                id: NEXT_PROMPT.fetch_add(1, Ordering::Relaxed),
                app_name: app_name(&app_id),
                app_id,
                title,
                expires_at_ms: now_ms() + 60_000,
                attendees,
                calendar_context,
                detected_call,
            };
            *app.state::<DetectionState>().prompt.lock().unwrap() = Some(prompt.clone());
            crate::show_window(app, "prompt", None);
            let _ = app.emit("record-prompt", Some(prompt));
        }
    }
}

fn app_name(app_id: &str) -> String {
    match app_id {
        "com.microsoft.teams2" => "Microsoft Teams",
        "us.zoom.xos" => "Zoom",
        "com.google.Chrome" => "Google Chrome",
        "com.microsoft.edgemac" => "Microsoft Edge",
        "com.apple.FaceTime" => "FaceTime",
        "com.apple.Safari" => "Safari",
        "org.mozilla.firefox" => "Firefox",
        "com.tinyspeck.slackmacgap" => "Slack",
        other => return other.to_owned(),
    }
    .into()
}

pub fn answer(
    app: &AppHandle,
    shared: &Arc<Shared>,
    answer: &str,
    prompt_id: u64,
) -> Result<(), String> {
    if !matches!(answer, "yes" | "no" | "never") {
        return Err("Unknown answer".into());
    }
    let state = app.state::<DetectionState>();
    let prompt = {
        let mut pending = state.prompt.lock().unwrap();
        if !pending
            .as_ref()
            .is_some_and(|prompt| prompt.id == prompt_id && prompt.expires_at_ms > now_ms())
        {
            return Err("The recording prompt expired".into());
        }
        pending.take().expect("validated prompt")
    };
    dismiss(app);
    if !state.matches(&prompt.detected_call) {
        return Err("The detected meeting has ended".into());
    }
    match answer {
        "yes" => {
            let had_calendar_context = prompt.calendar_context.is_some();
            let calendar_context = if shared.settings().calendar_enabled {
                revalidated_calendar_context(prompt.calendar_context, &calendar())
            } else {
                None
            };
            let attendees = if !had_calendar_context || calendar_context.is_some() {
                prompt.attendees
            } else {
                Vec::new()
            };
            let started = crate::meeting_runtime::start_detected_with_calendar(
                app,
                shared,
                prompt.title.unwrap_or_else(|| "Meeting".into()),
                prompt.detected_call,
                attendees,
                calendar_context,
            );
            if started.is_err() {
                // The prompt is already gone; the recording view shows why it did not start.
                LOG.error("Recording could not start after the recording prompt");
                crate::show_window(app, "live", None);
            }
            started?;
        }
        "never" => {
            let mut settings = shared.settings();
            settings
                .app_rules
                .retain(|rule| rule.bundle_id != prompt.app_id);
            settings.app_rules.push(AppRule {
                bundle_id: prompt.app_id,
                action: RecordAction::Never,
            });
            shared
                .engine
                .update_settings(settings)
                .map_err(|error| error.to_string())?;
        }
        _ => {}
    }
    Ok(())
}

pub fn toggle_recording(app: AppHandle, shared: Arc<Shared>) {
    if shared.recording.lock().unwrap().is_busy() {
        let _ = crate::meeting_runtime::stop(app, shared, None);
    } else {
        crate::show_window(&app, "live", None);
        if crate::meeting_runtime::start(&shared, "Meeting".into(), None, Vec::new()).is_err() {
            LOG.error("Recording could not start from the shortcut");
        }
    }
}

pub fn spawn(app: AppHandle, shared: Arc<Shared>) {
    std::thread::Builder::new()
        .name("pennel-calls".into())
        .spawn(move || {
            let mut detector = CallDetector::new(app.config().identifier.clone());
            let started = Instant::now();
            let mut was_enabled = false;
            let mut active_episode: Option<DetectedCall> = None;
            loop {
                #[cfg(target_os = "macos")]
                crate::refresh_hotkey(&app, &shared);
                let quitting = app
                    .state::<crate::ExitState>()
                    .requested
                    .load(Ordering::SeqCst);
                let enabled = shared.settings().auto_detect && !quitting;
                if enabled {
                    if let Some(ended) = app
                        .state::<DetectionState>()
                        .clean_end_reset
                        .lock()
                        .unwrap()
                        .take()
                    {
                        reset_ended_episode(
                            &mut detector,
                            &mut active_episode,
                            &ended,
                            &app.config().identifier,
                        );
                    }
                    if let Ok(users) = snapshot() {
                        for event in detector.update(&users, started.elapsed().as_secs_f64()) {
                            match event {
                                CallEvent::Started { app_id } => {
                                    let episode =
                                        app.state::<DetectionState>().begin_episode(app_id);
                                    active_episode = Some(episode.clone());
                                    offer(&app, &shared, episode);
                                }
                                CallEvent::Ended { app_id } => {
                                    if let Some(episode) = active_episode.take()
                                        && episode.app_id == app_id
                                        && app.state::<DetectionState>().matches(&episode)
                                    {
                                        app.state::<DetectionState>().active.lock().unwrap().take();
                                        dismiss(&app);
                                    }
                                    let _ = crate::meeting_runtime::stop(
                                        app.clone(),
                                        shared.clone(),
                                        Some(&app_id),
                                    );
                                }
                            }
                        }
                    }
                    let expired = app
                        .state::<DetectionState>()
                        .prompt
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|prompt| prompt.expires_at_ms <= now_ms());
                    if expired {
                        dismiss(&app);
                    }
                } else if was_enabled {
                    detector = CallDetector::new(app.config().identifier.clone());
                    active_episode = None;
                    app.state::<DetectionState>().active.lock().unwrap().take();
                    dismiss(&app);
                }
                was_enabled = enabled;
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .expect("call observer thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_keeps_only_unchanged_authorized_calendar_context() {
        let saved = json!({"source":"eventkit","source_id":"account-a","calendar_id":"calendar","event_id":"event","source_revision":"revision-a","participants":[]});
        let current = json!({"has_access":true,"current_event":{"calendar_context":saved}});
        assert_eq!(
            revalidated_calendar_context(Some(saved.clone()), &current),
            Some(saved.clone())
        );
        let mut changed = current.clone();
        changed["current_event"]["calendar_context"]["source_id"] = json!("account-b");
        assert!(revalidated_calendar_context(Some(saved.clone()), &changed).is_none());
        changed = current.clone();
        changed["current_event"]["calendar_context"]["source_revision"] = json!("revision-b");
        assert!(revalidated_calendar_context(Some(saved.clone()), &changed).is_none());
        changed = current.clone();
        changed["has_access"] = json!(false);
        assert!(revalidated_calendar_context(Some(saved.clone()), &changed).is_none());
        assert!(
            revalidated_calendar_context(
                Some(saved),
                &json!({"has_access":false,"current_event":null})
            )
            .is_none()
        );
    }

    #[test]
    fn prompt_payload_never_exposes_private_calendar_participant_details() {
        let prompt = RecordPrompt {
            id: 1,
            app_id: "call-app".into(),
            app_name: "Call".into(),
            title: Some("Meeting".into()),
            expires_at_ms: 100,
            attendees: vec!["Els".into()],
            calendar_context: Some(json!({"participants":[{"email":"private@example.test"}]})),
            detected_call: DetectedCall {
                app_id: "call-app".into(),
                revision: 1,
            },
        };
        let payload = serde_json::to_string(&prompt).unwrap();
        assert!(!payload.contains("private@example.test"));
        assert!(!payload.contains("attendees"));
        assert!(!payload.contains("calendar_context"));
        assert!(!payload.contains("detected_call"));
        assert!(!payload.contains("revision"));
    }

    #[test]
    fn old_same_app_episode_cannot_clear_or_reset_a_replacement_call() {
        let state = DetectionState::default();
        let first = state.begin_episode("com.microsoft.teams2".into());
        let replacement = state.begin_episode("com.microsoft.teams2".into());
        assert_ne!(first, replacement);
        assert!(!state.finish_episode(&first));
        assert!(state.matches(&replacement));
        assert!(state.clean_end_reset.lock().unwrap().is_none());
        assert!(state.finish_episode(&replacement));
        assert!(!state.finish_episode(&replacement));
        assert_eq!(
            state.clean_end_reset.lock().unwrap().take(),
            Some(replacement)
        );
    }

    #[test]
    fn a_clean_end_reset_allows_a_new_call_but_not_a_prejoin_observation() {
        let state = DetectionState::default();
        let mut detector = CallDetector::new("pennel");
        let playing = |is_call| CallObservation {
            app_id: "com.microsoft.teams2".into(),
            is_call,
            is_playing: true,
        };
        detector.update(&[playing(true)], 0.0);
        assert!(matches!(
            detector.update(&[playing(true)], 5.0).as_slice(),
            [CallEvent::Started { .. }]
        ));
        let episode = state.begin_episode("com.microsoft.teams2".into());
        assert!(state.finish_episode(&episode));
        let reset = state.clean_end_reset.lock().unwrap().take().unwrap();
        assert_eq!(reset, episode);
        assert_eq!(detector.active_call(), Some(reset.app_id.as_str()));
        let mut active = Some(episode);
        assert!(reset_ended_episode(
            &mut detector,
            &mut active,
            &reset,
            "pennel"
        ));
        assert!(detector.update(&[playing(false)], 10.0).is_empty());
        assert!(detector.update(&[playing(false)], 20.0).is_empty());
        assert!(detector.update(&[playing(true)], 21.0).is_empty());
        assert!(matches!(
            detector.update(&[playing(true)], 26.0).as_slice(),
            [CallEvent::Started { .. }]
        ));
    }
}
