//! Recording ownership outlives windows and remains busy until audio and transcript writes finish.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pennel_core::{MeetingSession, RecordingPhase};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::diagnostics::Log;
use crate::meeting::MeetingRecorder;
use crate::state::Shared;

const LOG: Log = Log::new("meeting");

pub struct MeetingContext {
    pub token: String,
    pub uid: String,
    pub session: Arc<MeetingSession>,
    pub notes_gate: Arc<Mutex<bool>>,
    pub title: String,
    pub started: Instant,
    pub duration_secs: Option<u64>,
    pub pending_recording_finish: Option<crate::meeting::RecordingFinish>,
    pub system_audio: bool,
    pub heard_system_audio: bool,
    pub audio_warning: Option<String>,
    pub consent_copied: bool,
    pub speaker_evidence: Option<crate::meeting_speaker_evidence::EvidenceSampler>,
}

/// Serialize session writes without retaining capture's lifecycle lock during disk I/O.
pub fn with_session_notes<T>(
    shared: &Shared,
    session_id: &str,
    work: impl FnOnce(&Arc<MeetingSession>) -> Result<T, String>,
) -> Result<T, String> {
    with_session_access(shared, session_id, true, work)
}

/// Read acknowledged evidence during finalization while excluding the final folder rename.
pub fn with_session_snapshot<T>(
    shared: &Shared,
    session_id: &str,
    work: impl FnOnce(&Arc<MeetingSession>) -> Result<T, String>,
) -> Result<T, String> {
    with_session_access(shared, session_id, false, work)
}

fn with_session_access<T>(
    shared: &Shared,
    session_id: &str,
    writing: bool,
    work: impl FnOnce(&Arc<MeetingSession>) -> Result<T, String>,
) -> Result<T, String> {
    let (session, gate) = {
        let context = shared.meeting_context.lock().unwrap();
        let context = context
            .as_ref()
            .filter(|context| context.token == session_id)
            .ok_or("This recording session is no longer available.")?;
        (context.session.clone(), context.notes_gate.clone())
    };
    let accepting = gate.lock().unwrap();
    if writing && !*accepting {
        return Err("This recording session is already being finalized.".into());
    }
    // A reader may have waited through rename. Never read the old session folder after its
    // context was cleared; the caller can resolve the completed meeting by stable UID instead.
    {
        let lifecycle = shared.recording.lock().unwrap();
        let state = lifecycle.snapshot();
        if state.session_id.as_deref() != Some(session_id)
            || !matches!(
                state.phase,
                RecordingPhase::Recording
                    | RecordingPhase::Stopping
                    | RecordingPhase::Finalizing
                    | RecordingPhase::Error
            )
            || (writing && state.phase == RecordingPhase::Finalizing)
            || shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|context| context.token != session_id)
        {
            return Err("This recording session is no longer available for editing notes.".into());
        }
    }
    work(&session)
}

fn close_session_notes(shared: &Shared, token: &str) {
    let gate = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .filter(|context| context.token == token)
        .map(|context| context.notes_gate.clone());
    if let Some(gate) = gate {
        *gate.lock().unwrap() = false;
    }
}

pub struct LibraryJob(Arc<Shared>);

impl LibraryJob {
    pub fn new(shared: &Arc<Shared>) -> Result<Self, String> {
        let _recording = shared.recording.lock().unwrap();
        if shared.quitting.load(Ordering::SeqCst) {
            return Err("Pennel is shutting down. Open it again to continue.".into());
        }
        if shared.updating.load(Ordering::SeqCst) {
            return Err("Pennel is updating. Wait until the app has restarted.".into());
        }
        shared.library_jobs.fetch_add(1, Ordering::SeqCst);
        Ok(Self(shared.clone()))
    }
}

impl Drop for LibraryJob {
    fn drop(&mut self) {
        self.0.library_jobs.fetch_sub(1, Ordering::SeqCst);
    }
}

fn fail(shared: &Shared, token: &str, error: &str) {
    let _ = shared
        .recording
        .lock()
        .unwrap()
        .fail(token, error.to_owned());
}

pub fn set_title(shared: &Shared, title: String, session_id: &str) -> Result<(), String> {
    let lifecycle = shared.recording.lock().unwrap();
    let state = lifecycle.snapshot();
    match state.phase {
        RecordingPhase::Recording | RecordingPhase::Stopping | RecordingPhase::Error => {}
        RecordingPhase::Starting => {
            return Err("Wait until recording starts before changing its title.".into());
        }
        RecordingPhase::Finalizing => {
            return Err(
                "The recording is already being finalized. Rename the meeting in the library afterwards."
                    .into(),
            );
        }
        RecordingPhase::Idle => {
            return Err("There is no active recording whose title can be changed.".into());
        }
    }
    if state.session_id.as_deref() != Some(session_id) {
        return Err("This recording session is no longer active.".into());
    }
    let mut context = shared.meeting_context.lock().unwrap();
    let context = context
        .as_mut()
        .filter(|context| context.token == session_id)
        .ok_or("This recording session is no longer active.")?;
    if !title.trim().is_empty() {
        shared
            .engine
            .mark_meeting_title_manual(context.session.id())
            .map_err(|error| error.to_string())?;
    }
    context.title = title;
    #[cfg(target_os = "macos")]
    crate::macos_tray::changed();
    Ok(())
}

pub fn set_live_speaker_attribution(
    shared: &Shared,
    session_id: &str,
    enabled: bool,
) -> Result<String, String> {
    let session = {
        let lifecycle = shared.recording.lock().unwrap();
        let state = lifecycle.snapshot();
        if state.session_id.as_deref() != Some(session_id)
            || (enabled && state.phase != RecordingPhase::Recording)
        {
            return Err("This recording session is no longer active.".into());
        }
        shared
            .meeting_context
            .lock()
            .unwrap()
            .as_ref()
            .filter(|context| context.token == session_id)
            .map(|context| context.session.clone())
            .ok_or("This recording session is no longer active.")?
    };
    // Activation can prepare local models; notes and capture retain their independent lifetimes.
    session
        .set_live_speaker_attribution(enabled)
        .map_err(|error| error.to_string())
}

fn rename_after_recording(
    engine: &pennel_core::PennelEngine,
    id: String,
    original_title: &str,
    desired_title: &str,
) -> Result<String, String> {
    let wanted = desired_title.trim();
    if wanted.is_empty() || wanted == original_title {
        return Ok(id);
    }
    engine
        .rename_meeting(id, wanted.to_owned())
        .map_err(|error| format!("The recording title could not be saved: {error}"))
}

fn copy_consent(
    enabled: bool,
    language: pennel_core::InterfaceLanguage,
    copy: impl FnOnce(&str) -> Result<(), String>,
) -> Result<bool, String> {
    if !enabled {
        return Ok(false);
    }
    copy(&pennel_core::consent_sentence_for_language(language))?;
    Ok(true)
}

fn begin_recording(shared: &Shared, detected_app: Option<String>) -> Result<String, String> {
    let finalized_error = {
        let lifecycle = shared.recording.lock().unwrap();
        if shared.quitting.load(Ordering::SeqCst) {
            return Err("Pennel is shutting down. Open it again to continue.".into());
        }
        if shared.updating.load(Ordering::SeqCst) {
            return Err("Pennel is updating. Wait until the app has restarted.".into());
        }
        (lifecycle.snapshot().phase == RecordingPhase::Error)
            .then(|| {
                shared
                    .meeting_context
                    .lock()
                    .unwrap()
                    .as_ref()
                    .filter(|context| context.session.is_finalized())
                    .map(|context| context.token.clone())
            })
            .flatten()
    };
    if let Some(token) = finalized_error {
        retry_saved_session(shared, &token)?;
    }
    let mut lifecycle = shared.recording.lock().unwrap();
    if shared.quitting.load(Ordering::SeqCst) {
        return Err("Pennel is shutting down. Open it again to continue.".into());
    }
    if shared.updating.load(Ordering::SeqCst) {
        return Err("Pennel is updating. Wait until the app has restarted.".into());
    }
    let mut context = shared.meeting_context.lock().unwrap();
    if context
        .as_ref()
        .is_some_and(|context| !context.session.is_finalized())
    {
        return Err(
            "The previous recording has not been fully saved. Open it and try saving again.".into(),
        );
    }
    let token = lifecycle
        .begin(detected_app)
        .map_err(|error| error.to_string())?;
    shared.recording_cancel.store(false, Ordering::SeqCst);
    context.take();
    Ok(token)
}

/// Runs on a worker: native capture initialization may need the application's main queue.
pub fn start(
    app: Option<&AppHandle>,
    shared: &Arc<Shared>,
    title: String,
    detected_app: Option<String>,
    attendees: Vec<String>,
) -> Result<String, String> {
    // Manual capture can include room or media audio. Naming follows the recorder's explicit call
    // association rather than narrowing capture because an unrelated call happens to be active.
    // A manual start still records who was invited, so voices can be named after them later.
    let (attendees, calendar_context) =
        if attendees.is_empty() && shared.settings().calendar_enabled {
            current_invitees(&crate::call_detection::calendar())
        } else {
            (attendees, None)
        };
    start_with_calendar(
        app,
        shared,
        title,
        detected_app,
        attendees,
        calendar_context,
    )
}

fn current_invitees(calendar: &serde_json::Value) -> (Vec<String>, Option<serde_json::Value>) {
    let Some(event) = calendar
        .get("current_event")
        .filter(|event| event.is_object())
    else {
        return (Vec::new(), None);
    };
    let attendees = event
        .get("attendees")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let context = event
        .get("calendar_context")
        .filter(|value| value.is_object())
        .cloned();
    (attendees, context)
}

pub fn start_with_calendar(
    app: Option<&AppHandle>,
    shared: &Arc<Shared>,
    title: String,
    detected_app: Option<String>,
    attendees: Vec<String>,
    calendar_context: Option<serde_json::Value>,
) -> Result<String, String> {
    start_with_calendar_impl(
        app,
        shared,
        title,
        detected_app,
        attendees,
        calendar_context,
        None,
    )
}

pub(crate) fn start_detected_with_calendar(
    app: &AppHandle,
    shared: &Arc<Shared>,
    title: String,
    detected_call: crate::call_detection::DetectedCall,
    attendees: Vec<String>,
    calendar_context: Option<serde_json::Value>,
) -> Result<String, String> {
    start_with_calendar_impl(
        Some(app),
        shared,
        title,
        Some(detected_call.app_id.clone()),
        attendees,
        calendar_context,
        Some((app.clone(), detected_call)),
    )
}

fn start_with_calendar_impl(
    app: Option<&AppHandle>,
    shared: &Arc<Shared>,
    title: String,
    detected_app: Option<String>,
    attendees: Vec<String>,
    calendar_context: Option<serde_json::Value>,
    clean_end_context: Option<(AppHandle, crate::call_detection::DetectedCall)>,
) -> Result<String, String> {
    let token = begin_recording(shared, detected_app.clone())?;
    #[cfg(target_os = "macos")]
    let _tray_update = crate::macos_tray::UpdateOnDrop;
    #[cfg(target_os = "macos")]
    crate::macos_tray::changed();
    let title = title.trim().to_owned();
    let settings = shared.settings();
    let keys = shared.keys();
    if let Err(error) = shared.engine.validate_meeting_setup(keys.clone()) {
        let error = error.to_string();
        fail(shared, &token, &error);
        return Err(error);
    }
    #[cfg(target_os = "macos")]
    if settings
        .resolve(&settings.meeting_stt)
        .is_some_and(|(provider, _)| provider.kind == pennel_core::ProviderKind::AppleSpeech)
        && !crate::macos::permissions()
            .get("apple_speech")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        let error = "Apple dictation requires macOS 26 or later. Choose another speech provider.";
        fail(shared, &token, error);
        return Err(error.into());
    }
    let session = match shared
        .engine
        .clone()
        .start_meeting_with(title.clone(), attendees, keys)
    {
        Ok(session) => session,
        Err(error) => {
            let error = error.to_string();
            fail(shared, &token, &error);
            return Err(error);
        }
    };
    let id = session.id();
    let title = shared
        .engine
        .meeting_info(id.clone())
        .map_or(title, |info| info.title);
    if let Some(context) = calendar_context
        && shared
            .engine
            .set_meeting_calendar_context_json(id.clone(), context.to_string())
            .is_err()
    {
        LOG.error("Calendar participant details could not be saved. Recording will continue.");
    }
    *shared.meeting_context.lock().unwrap() = Some(MeetingContext {
        token: token.clone(),
        uid: shared
            .engine
            .meeting_uid(id.clone())
            .unwrap_or_else(|| id.clone()),
        session: session.clone(),
        notes_gate: Arc::new(Mutex::new(true)),
        title: title.clone(),
        started: Instant::now(),
        duration_secs: None,
        pending_recording_finish: None,
        system_audio: false,
        heard_system_audio: false,
        audio_warning: None,
        consent_copied: false,
        speaker_evidence: None,
    });
    // The native callback may be inside capture admission while Stop owns the lifecycle lock.
    // Never wait for that lock here: quiescence drains this callback before freeing its context.
    let owner = Arc::downgrade(shared);
    let level_app = app.cloned();
    let level_session = token.clone();
    let level_started = Instant::now();
    let throttle = Mutex::new(crate::meeting_level::LevelThrottle::new(token.clone()));
    let on_level: crate::meeting_level::LevelCallback = Arc::new(move |source, rms| {
        let Some(shared) = owner.upgrade() else {
            return;
        };
        let Ok(lifecycle) = shared.recording.try_lock() else {
            return;
        };
        let state = lifecycle.snapshot();
        let Ok(mut throttle) = throttle.try_lock() else {
            return;
        };
        if let Some(level) = throttle.sample(
            state.session_id.as_deref(),
            state.phase == RecordingPhase::Recording,
            source,
            rms * 8.0,
            level_started.elapsed(),
        ) && let Some(app) = &level_app
        {
            let _ = app.emit(
                "meeting-level",
                serde_json::json!({ "session_id": level_session, "level": level }),
            );
        }
    });
    let recorder = match MeetingRecorder::start(
        session.clone(),
        title,
        detected_app.clone(),
        settings.echo_cancellation,
        shared.engine.clone(),
        &shared.version,
        on_level,
    ) {
        Ok(recorder) => recorder,
        Err(error) => {
            if session.stop().is_ok() {
                let _ = shared.engine.delete_meeting(id);
                shared.meeting_context.lock().unwrap().take();
            }
            fail(shared, &token, &error);
            return Err(error);
        }
    };
    #[cfg(any(windows, target_os = "macos"))]
    LOG.info(format!(
        "echo cancellation active={}",
        recorder.echo_cancellation_active()
    ));
    let consent_copied = match copy_consent(
        settings.consent_enabled,
        shared.interface_language(),
        |text| crate::platform_clipboard::copy(text, None),
    ) {
        Ok(copied) => copied,
        Err(_) => {
            LOG.error("The recording notice could not be copied. Recording will continue.");
            false
        }
    };
    let system_audio = recorder.has_system_audio();
    let recording_metadata = shared
        .engine
        .meeting_recording_metadata(id.clone())
        .ok()
        .flatten();
    let clean_end = clean_end_context.map(|(app, detected_call)| {
        let owner = Arc::downgrade(shared);
        let session_token = token.clone();
        Arc::new(move || {
            if let Some(shared) = owner.upgrade()
                && stop_matching_with_episode(
                    app.clone(),
                    shared,
                    Some(detected_call.app_id.as_str()),
                    Some(&session_token),
                    Some(&detected_call),
                )
                .is_err()
            {
                LOG.error("A verified call end could not stop its recording.");
            }
        }) as crate::meeting_speaker_evidence::CleanEndCallback
    });
    let evidence_clock = recorder.evidence_clock();
    let capture_source = recorder.app_speaker_capture_source();
    if let Some(context) = shared.meeting_context.lock().unwrap().as_mut() {
        context.started = recorder.started_at();
        context.system_audio = system_audio;
        context.audio_warning = recorder.audio_warning();
        context.consent_copied = consent_copied;
    }
    *shared.meeting.lock().unwrap() = Some(recorder);
    shared
        .recording
        .lock()
        .unwrap()
        .recording(&token)
        .map_err(|error| error.to_string())?;
    // Publish capture ownership before an end notification can stop it. Registration uses the
    // coordinator's lock order so an observer cannot be attached after hardware quiescence.
    {
        let lifecycle = shared.recording.lock().unwrap();
        let state = lifecycle.snapshot();
        if state.phase == RecordingPhase::Recording
            && state.session_id.as_deref() == Some(&token)
            && let Some(context) = shared
                .meeting_context
                .lock()
                .unwrap()
                .as_mut()
                .filter(|context| context.token == token)
        {
            context.speaker_evidence = if settings.meeting_app_speakers {
                recording_metadata.as_ref().and_then(|recording| {
                    crate::meeting_speaker_evidence::EvidenceSampler::start(
                        evidence_clock,
                        detected_app.as_deref(),
                        &recording.meeting_uid,
                        &recording.started_at,
                        {
                            let owner = Arc::downgrade(shared);
                            Arc::new(move || {
                                owner
                                    .upgrade()
                                    .is_some_and(|shared| shared.settings().meeting_app_speakers)
                            })
                        },
                        capture_source,
                        clean_end,
                    )
                })
            } else {
                None
            };
        }
    }
    monitor_health(shared.clone(), token);
    if shared.recording_cancel.load(Ordering::SeqCst) {
        stop_and_wait(shared)?;
    }
    Ok(id)
}

fn take_recorder(
    shared: &Shared,
    expected_call: Option<&str>,
) -> Result<Option<(String, MeetingRecorder)>, String> {
    take_recorder_matching(shared, expected_call, None)
}

fn take_recorder_matching(
    shared: &Shared,
    expected_call: Option<&str>,
    expected_session: Option<&str>,
) -> Result<Option<(String, MeetingRecorder)>, String> {
    let mut lifecycle = shared.recording.lock().unwrap();
    if expected_call.is_some_and(|app| !lifecycle.owns_call(app)) {
        return Ok(None);
    }
    let state = lifecycle.snapshot();
    if expected_session.is_some_and(|id| state.session_id.as_deref() != Some(id)) {
        return Ok(None);
    }
    match state.phase {
        RecordingPhase::Starting => {
            shared.recording_cancel.store(true, Ordering::SeqCst);
            return Ok(None);
        }
        RecordingPhase::Idle
        | RecordingPhase::Error
        | RecordingPhase::Stopping
        | RecordingPhase::Finalizing => return Ok(None),
        RecordingPhase::Recording => {}
    }
    let token = state
        .session_id
        .ok_or("The recording session is missing.")?;
    lifecycle
        .stopping(&token)
        .map_err(|error| error.to_string())?;
    let recorder = match shared.meeting.lock().unwrap().take() {
        Some(recorder) => recorder,
        None => {
            let error = "The audio recording is unavailable.";
            let _ = lifecycle.fail(&token, error.into());
            return Err(error.into());
        }
    };
    Ok(Some((token, recorder)))
}

fn monitor_health(shared: Arc<Shared>, token: String) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let state = shared.recording.lock().unwrap().snapshot();
            if state.session_id.as_deref() != Some(&token)
                || state.phase != RecordingPhase::Recording
            {
                return;
            }
            let terminal = shared
                .meeting
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|recorder| {
                    recorder
                        .session
                        .health()
                        .terminal_error
                        .or_else(|| recorder.terminal_error())
                });
            if terminal.is_some() {
                if let Ok(Some((token, recorder))) =
                    take_recorder_matching(&shared, None, Some(&token))
                {
                    LOG.error("Recording is stopping after an audio or storage error.");
                    let _ = finish(&shared, &token, recorder);
                }
                return;
            }
        }
    });
}

fn quiesce(shared: &Shared, token: &str, recorder: &mut MeetingRecorder) {
    let duration_ms = recorder.quiesce();
    if let Some(context) = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_mut()
        .filter(|context| context.token == token)
    {
        context.duration_secs = Some(duration_ms / 1_000);
    }
}

fn finish_with_flush<T>(
    shared: &Shared,
    token: &str,
    mut recorder: MeetingRecorder,
    flush: impl FnOnce() -> T,
) -> (Result<String, String>, T) {
    quiesce(shared, token, &mut recorder);
    let flushed = flush();
    (finish(shared, token, recorder), flushed)
}

fn finish(shared: &Shared, token: &str, mut recorder: MeetingRecorder) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    let _tray_update = crate::macos_tray::UpdateOnDrop;
    quiesce(shared, token, &mut recorder);
    close_session_notes(shared, token);
    let original_title = recorder.title.clone();
    let system_audio = recorder.has_system_audio();
    let heard_system_audio = recorder.heard_system_audio();
    let audio_warning = recorder.audio_warning();
    let terminal_error = recorder
        .terminal_error()
        .or_else(|| recorder.session.health().terminal_error);
    shared
        .recording
        .lock()
        .unwrap()
        .finalizing(token)
        .map_err(|error| error.to_string())?;
    let mut desired_title = original_title.clone();
    if let Some(context) = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_mut()
        .filter(|context| context.token == token)
    {
        context.system_audio = system_audio;
        context.heard_system_audio = heard_system_audio;
        context.audio_warning = audio_warning.or_else(|| terminal_error.clone());
        desired_title = context.title.clone();
    }
    #[cfg(target_os = "macos")]
    crate::macos_tray::changed();
    let speaker_sampler = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_mut()
        .filter(|context| context.token == token)
        .and_then(|context| context.speaker_evidence.take());
    if let Some(sampler) = &speaker_sampler {
        sampler.request_stop();
    }
    let (captured, completion) = recorder.stop();
    let speaker_evidence = speaker_sampler.map(|sampler| sampler.finish());
    let captured = captured.inspect(|id| {
        if let Some(evidence) = speaker_evidence
            && shared
                .engine
                .set_meeting_app_speaker_evidence(id, evidence)
                .is_err()
        {
            LOG.error("Meeting app speaker evidence could not be saved. The recording is kept.");
        }
    });
    if let Some(context) = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_mut()
        .filter(|context| context.token == token)
    {
        context.pending_recording_finish = Some(completion);
    }
    finish_after_capture(
        shared,
        token,
        captured,
        terminal_error,
        &original_title,
        &desired_title,
    )
}

fn finish_after_capture(
    shared: &Shared,
    token: &str,
    captured: Result<String, String>,
    terminal_error: Option<String>,
    original_title: &str,
    desired_title: &str,
) -> Result<String, String> {
    complete_session(shared, token, || {
        captured.and_then(|id| {
            if let Some(error) = terminal_error {
                return Err(error);
            }
            rename_after_recording(&shared.engine, id, original_title, desired_title)
        })
    })
}

fn complete_session(
    shared: &Shared,
    token: &str,
    work: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let gate = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .filter(|context| context.token == token)
        .map(|context| context.notes_gate.clone())
        .ok_or("This recording session is no longer available.")?;
    let mut accepting = gate.lock().unwrap();
    let result = finish_context(shared, token, work());
    if result.is_err() {
        *accepting = true;
    }
    result
}

fn finish_context(
    shared: &Shared,
    token: &str,
    result: Result<String, String>,
) -> Result<String, String> {
    match result {
        Ok(id) => {
            let mut lifecycle = shared.recording.lock().unwrap();
            lifecycle
                .complete(token)
                .map_err(|error| error.to_string())?;
            shared.meeting_context.lock().unwrap().take();
            Ok(id)
        }
        Err(error) => {
            fail(shared, token, &error);
            Err(error)
        }
    }
}

fn retry_saved_session(shared: &Shared, session_id: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    let _tray_update = crate::macos_tray::UpdateOnDrop;
    let (session, desired_title, pending_recording_finish) = {
        let mut lifecycle = shared.recording.lock().unwrap();
        let state = lifecycle.snapshot();
        if state.phase != RecordingPhase::Error || state.session_id.as_deref() != Some(session_id) {
            return Err("This recording is no longer waiting for recovery.".into());
        }
        if shared.meeting.lock().unwrap().is_some() {
            return Err("Finish the audio recording before saving again.".into());
        }
        let context = shared.meeting_context.lock().unwrap();
        let context = context
            .as_ref()
            .filter(|context| context.token == session_id)
            .ok_or("This recording session is no longer available.")?;
        lifecycle
            .retry_finalizing(session_id)
            .map_err(|error| error.to_string())?;
        (
            context.session.clone(),
            context.title.clone(),
            context.pending_recording_finish,
        )
    };
    close_session_notes(shared, session_id);
    let id = session.id();
    let original_title = shared
        .engine
        .meeting_info(id.clone())
        .map_or_else(|| desired_title.clone(), |info| info.title);
    let saved = if session.is_finalized() {
        Ok(())
    } else {
        session.retry_persistence()
    };
    complete_session(shared, session_id, || {
        saved
            .map_err(|error| error.to_string())
            .and_then(|()| {
                pending_recording_finish.map_or(Ok(()), |finish| {
                    shared
                        .engine
                        .finish_meeting_recording(id.clone(), finish.duration_ms, finish.complete)
                        .map_err(|error| error.to_string())
                })
            })
            .and_then(|()| {
                rename_after_recording(&shared.engine, id, &original_title, &desired_title)
            })
    })
}

/// Saves the retained failed session only; successful speech recognition is never repeated here.
pub fn retry_persistence(
    app: &AppHandle,
    shared: &Arc<Shared>,
    session_id: &str,
) -> Result<(), String> {
    let _job = LibraryJob::new(shared)?;
    let uid = session_uid(shared, session_id);
    let result = retry_saved_session(shared, session_id);
    let _ = app.emit(
        "meeting-finished",
        MeetingFinished {
            id: result.clone().ok(),
            meeting_uid: uid,
            session_id: session_id.to_owned(),
            error: result
                .as_ref()
                .err()
                .map(crate::commands::localize_error_message),
            flush_warning: None,
        },
    );
    if let Ok(id) = &result {
        let _ = app.emit("meeting-stopped", Some(id));
        let _ = app.emit("meeting-updated", id);
        crate::cloud::poke(app);
        spawn_automatic_title(app.clone(), shared.clone(), id.clone());
    }
    result.map(|_| ())
}

#[derive(Clone, Serialize)]
struct MeetingFinished {
    id: Option<String>,
    meeting_uid: Option<String>,
    session_id: String,
    error: Option<String>,
    flush_warning: Option<String>,
}

fn session_uid(shared: &Shared, token: &str) -> Option<String> {
    shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .filter(|context| context.token == token)
        .map(|context| context.uid.clone())
}

/// `expected_call` keeps an ended call from stopping a manual or different recording.
pub fn stop(
    app: AppHandle,
    shared: Arc<Shared>,
    expected_call: Option<&str>,
) -> Result<(), String> {
    stop_matching(app, shared, expected_call, None)
}

pub fn stop_matching(
    app: AppHandle,
    shared: Arc<Shared>,
    expected_call: Option<&str>,
    expected_session: Option<&str>,
) -> Result<(), String> {
    stop_matching_with_episode(app, shared, expected_call, expected_session, None)
}

fn stop_matching_with_episode(
    app: AppHandle,
    shared: Arc<Shared>,
    expected_call: Option<&str>,
    expected_session: Option<&str>,
    clean_end_episode: Option<&crate::call_detection::DetectedCall>,
) -> Result<(), String> {
    let Some((token, mut recorder)) =
        take_recorder_matching(&shared, expected_call, expected_session)?
    else {
        // An end during native startup uses the coordinator's existing cancellation path.
        // Do not reset a detector episode if this callback belongs to a replacement session.
        if let Some(episode) = clean_end_episode {
            let cancelled_same_session = {
                let lifecycle = shared.recording.lock().unwrap();
                let state = lifecycle.snapshot();
                shared.recording_cancel.load(Ordering::SeqCst)
                    && expected_session == state.session_id.as_deref()
                    && lifecycle.owns_call(&episode.app_id)
                    && matches!(
                        state.phase,
                        RecordingPhase::Starting | RecordingPhase::Recording
                    )
            };
            if cancelled_same_session {
                crate::call_detection::recorded_call_ended(&app, episode);
            }
        }
        return Ok(());
    };
    // This command runs off-main. Its IPC receipt acknowledges actual capture shutdown,
    // while the owned worker below performs all persistence and transcript finalization.
    quiesce(&shared, &token, &mut recorder);
    if let Some(episode) = clean_end_episode {
        crate::call_detection::recorded_call_ended(&app, episode);
    }
    #[cfg(target_os = "macos")]
    crate::macos_tray::changed();
    std::thread::spawn(move || {
        let library_job = LibraryJob::new(&shared).ok();
        let uid = session_uid(&shared, &token);
        let (result, flush) = finish_with_flush(&shared, &token, recorder, || {
            crate::flush_frontends(&app, "prepare-recording-stop")
        });
        let flush_warning = flush.result.err().map(|_| {
            "Some draft changes could not be saved. Recording has finished; check your saved draft.".to_owned()
        });
        if flush_warning.is_some() {
            LOG.error("Some draft changes could not be saved before stopping.");
        }
        if result.is_err() {
            LOG.error("Meeting finalization failed; the recording remains visible in Pennel.");
        }
        let finished = MeetingFinished {
            id: result.clone().ok(),
            meeting_uid: uid,
            session_id: token,
            error: result
                .as_ref()
                .err()
                .map(crate::commands::localize_error_message),
            flush_warning: flush_warning.map(crate::commands::localize_error_message),
        };
        let _ = app.emit("meeting-finished", finished);
        crate::cloud::poke(&app);
        let _ = app.emit("meeting-stopped", result.clone().ok());
        let _ = app.emit("recording-flush-complete", flush.request_id);
        if let Ok(id) = result
            && library_job.is_some()
            && !shared.quitting.load(Ordering::SeqCst)
        {
            postprocess(app, shared, id);
        }
    });
    Ok(())
}

fn postprocess(app: AppHandle, shared: Arc<Shared>, id: String) {
    if shared.quitting.load(Ordering::SeqCst) {
        return;
    }
    let _progress = crate::meeting_postprocess::PostprocessProgress::start(&app, &shared, &id);
    let settings = shared.settings();
    if settings.diarization && shared.engine.speaker_recognition_ready() {
        let dir = std::path::PathBuf::from(shared.engine.meeting_dir(id.clone()));
        match crate::speakers::recognize(&shared, &id, &dir) {
            Ok(_) => {}
            Err(_) => {
                LOG.error("Automatic speaker recognition failed; try again from the meeting.")
            }
        }
        let _ = app.emit("meeting-updated", &id);
    } else {
        let (phase, reason) = if settings.diarization {
            ("unavailable", "not_ready")
        } else {
            ("skipped", "disabled")
        };
        if shared
            .engine
            .set_meeting_speaker_processing_status(
                id.clone(),
                phase.into(),
                Some(reason.into()),
                settings.speaker_recognition,
                0,
            )
            .is_err()
        {
            LOG.error("Speaker processing availability could not be saved.");
        }
        let _ = app.emit("meeting-updated", &id);
    }
    if shared.quitting.load(Ordering::SeqCst) {
        return;
    }
    // The event describes preparation; admission and policy epochs remain authoritative in core.
    if shared.settings().summary_automation == pennel_core::SummaryAutomation::Automatic {
        let progress = crate::meeting_postprocess::SummaryProgress::start(&app, &shared, &id);
        let result = shared.engine.summarize_meeting_automatically(
            id.clone(),
            shared.settings().default_template,
            shared.keys(),
        );
        if let Some(progress) = progress {
            match &result {
                Ok(None) => progress.finish(crate::meeting_postprocess::ProgressState::Skipped),
                _ => progress.complete(&result),
            }
        }
        if result.is_err() {
            LOG.error("Automatic summarization failed; transcript and notes remain saved.");
        }
    }
    crate::automatic_suggestions::run_after_summary(app.clone(), shared.clone(), id.clone());
    let _ = app.emit("meeting-updated", &id);
    crate::cloud::poke(&app);
    drop(_progress);
    // Naming follows existing processing so its optional request cannot delay summary or Review.
    automatically_name_meeting(&app, &shared, id);
}

pub(crate) fn spawn_automatic_title(app: AppHandle, shared: Arc<Shared>, id: String) {
    std::thread::spawn(move || automatically_name_meeting(&app, &shared, id));
}

fn automatically_name_meeting(app: &AppHandle, shared: &Arc<Shared>, id: String) {
    if shared.quitting.load(Ordering::SeqCst) {
        return;
    }
    let Ok(_job) = LibraryJob::new(shared) else {
        return;
    };
    match shared
        .engine
        .clone()
        .automatically_name_meeting(id.clone(), shared.keys())
    {
        Ok(true) => {
            let _ = app.emit("meeting-updated", &id);
            crate::cloud::poke(app);
        }
        Ok(false) => {}
        Err(_) => LOG.error("Automatic meeting naming failed; the existing name is kept."),
    }
}

/// Worker-only shutdown path. The host keeps its event loop alive while native audio is released.
pub struct QuiescedRecording {
    token: String,
    recorder: MeetingRecorder,
}

pub fn prepare_shutdown(shared: &Arc<Shared>) -> Result<Option<QuiescedRecording>, String> {
    loop {
        if shared.recording.lock().unwrap().snapshot().phase == RecordingPhase::Starting {
            shared.recording_cancel.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let Some((token, mut recorder)) = take_recorder(shared, None)? else {
            return Ok(None);
        };
        quiesce(shared, &token, &mut recorder);
        return Ok(Some(QuiescedRecording { token, recorder }));
    }
}

pub fn finish_shutdown(
    app: &AppHandle,
    shared: &Arc<Shared>,
    recording: Option<QuiescedRecording>,
) -> Result<Option<String>, String> {
    let Some(QuiescedRecording { token, recorder }) = recording else {
        return stop_and_wait(shared);
    };
    let uid = session_uid(shared, &token);
    let result = finish(shared, &token, recorder);
    let _ = app.emit(
        "meeting-finished",
        MeetingFinished {
            id: result.clone().ok(),
            meeting_uid: uid,
            session_id: token,
            error: result
                .as_ref()
                .err()
                .map(crate::commands::localize_error_message),
            flush_warning: None,
        },
    );
    result.map(Some)
}

pub fn stop_and_wait(shared: &Arc<Shared>) -> Result<Option<String>, String> {
    loop {
        let phase = shared.recording.lock().unwrap().snapshot().phase;
        match phase {
            RecordingPhase::Starting => {
                shared.recording_cancel.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
            }
            RecordingPhase::Stopping | RecordingPhase::Finalizing => {
                std::thread::sleep(Duration::from_millis(20))
            }
            RecordingPhase::Recording => {
                if let Some((token, recorder)) = take_recorder(shared, None)? {
                    return finish(shared, &token, recorder).map(Some);
                }
            }
            RecordingPhase::Error => {
                if shared.meeting.lock().unwrap().is_none() {
                    let token = shared
                        .meeting_context
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|context| context.token.clone());
                    if let Some(token) = token {
                        return retry_saved_session(shared, &token).map(Some);
                    }
                    return Ok(None);
                }
                let message = shared
                    .recording
                    .lock()
                    .unwrap()
                    .snapshot()
                    .error
                    .unwrap_or_else(|| "The recording has not been finalized.".into());
                return Err(message);
            }
            RecordingPhase::Idle => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    #[test]
    fn manual_start_takes_invitees_and_context_from_the_current_event() {
        let calendar = serde_json::json!({
            "current_event": {
                "title": "Weekly",
                "attendees": ["Ann", "Bert"],
                "calendar_context": { "source": "eventkit", "participants": [] }
            }
        });

        let (attendees, context) = super::current_invitees(&calendar);

        assert_eq!(attendees, ["Ann", "Bert"]);
        assert!(context.is_some());
        assert_eq!(
            super::current_invitees(&serde_json::json!({ "current_event": null })),
            (Vec::new(), None)
        );
    }

    #[test]
    fn live_speaker_controls_reject_stale_and_stopping_sessions_without_changing_settings() {
        let (_root, shared, session, token) = recording();
        let before = shared.settings();
        assert!(set_live_speaker_attribution(&shared, "previous-recording", true).is_err());
        assert_eq!(shared.settings(), before);
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        assert!(set_live_speaker_attribution(&shared, &token, true).is_err());
        let disabled: serde_json::Value =
            serde_json::from_str(&set_live_speaker_attribution(&shared, &token, false).unwrap())
                .unwrap();
        assert_eq!(disabled["enabled"], false);
        assert_eq!(shared.settings(), before);
        assert_eq!(
            shared.engine.meeting_notes(session.id()),
            "Een bewaarde afspraak."
        );
        session.stop().unwrap();
    }

    #[test]
    fn unavailable_live_speaker_setup_never_waits_for_the_notes_gate_or_substitutes_a_route() {
        let (_root, shared, session, token) = recording();
        let mut settings = shared.settings();
        settings.speaker_recognition = pennel_core::SpeakerRecognitionMode::Cloud;
        shared.engine.update_settings(settings.clone()).unwrap();
        let gate = shared
            .meeting_context
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .notes_gate
            .clone();
        let writing = gate.lock().unwrap();
        let request_shared = shared.clone();
        let (finished, completion) = std::sync::mpsc::channel();
        let request = std::thread::spawn(move || {
            finished
                .send(set_live_speaker_attribution(&request_shared, &token, true))
                .unwrap()
        });
        let result = completion
            .recv_timeout(Duration::from_secs(2))
            .expect("optional setup must not wait for note persistence")
            .unwrap();
        let status: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(status["enabled"], false);
        assert_eq!(status["available"], false);
        assert_eq!(status["reason_code"], "local_required");
        assert_eq!(shared.settings(), settings);
        drop(writing);
        request.join().unwrap();
        session.stop().unwrap();
    }

    #[test]
    fn capture_quiescence_closes_live_speaker_publication_before_transcript_finalization() {
        let (_root, shared, session, _token) = recording();
        let quiescent = Arc::new(AtomicBool::new(false));
        let mut recorder =
            MeetingRecorder::testing(session.clone(), shared.engine.clone(), quiescent.clone());
        recorder.quiesce();
        assert!(quiescent.load(Ordering::SeqCst));
        assert!(!session.is_finalized());
        let status: serde_json::Value =
            serde_json::from_str(&session.live_speaker_status_json().unwrap()).unwrap();
        assert_eq!(status["state"], "stopped");
        assert_eq!(status["enabled"], false);
        assert_eq!(
            session.clone().set_live_speaker_attribution(true),
            Err(pennel_core::PennelError::Cancelled)
        );
        session.stop().unwrap();
    }

    #[test]
    fn finalizing_snapshot_reads_closed_notes_and_holds_off_folder_rename() {
        let (_root, shared, session, token) = recording();
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        close_session_notes(&shared, &token);
        session.stop().unwrap();
        assert!(with_session_notes(&shared, &token, |_| Ok(())).is_err());
        let (reading, started) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let read_shared = shared.clone();
        let read_token = token.clone();
        let reader = std::thread::spawn(move || {
            with_session_snapshot(&read_shared, &read_token, |session| {
                assert_eq!(
                    read_shared.engine.meeting_notes(session.id()),
                    "Een bewaarde afspraak."
                );
                reading.send(()).unwrap();
                waiting.recv().unwrap();
                Ok(())
            })
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let finish_shared = shared.clone();
        let finish_token = token.clone();
        let original_id = session.id();
        let finishing = std::thread::spawn(move || {
            finish_after_capture(
                &finish_shared,
                &finish_token,
                Ok(original_id),
                None,
                "Originele titel",
                "After snapshot",
            )
        });
        let retained_original = std::path::PathBuf::from(session.dir()).exists();
        release.send(()).unwrap();
        reader.join().unwrap().unwrap();
        let renamed = finishing.join().unwrap().unwrap();
        assert!(retained_original);
        assert_ne!(renamed, session.id());
        assert!(with_session_snapshot(&shared, &token, |_| Ok(())).is_err());
    }

    #[test]
    fn capture_stops_before_a_blocked_notes_write_and_rename_waits_for_it() {
        let (_root, shared, session, token) = recording();
        let (writing, started) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let writer_shared = shared.clone();
        let writer_token = token.clone();
        let writer = std::thread::spawn(move || {
            with_session_notes(&writer_shared, &writer_token, |session| {
                writing.send(()).unwrap();
                waiting.recv().unwrap();
                session
                    .set_notes("The final acknowledged draft".into())
                    .map_err(|error| error.to_string())
            })
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        set_title(&shared, "Renamed after capture".into(), &token).unwrap();
        let quiescent = Arc::new(AtomicBool::new(false));
        let recorder =
            MeetingRecorder::testing(session.clone(), shared.engine.clone(), quiescent.clone());
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        let finishing_shared = shared.clone();
        let finishing_token = token.clone();
        let finishing =
            std::thread::spawn(move || finish(&finishing_shared, &finishing_token, recorder));
        let until = Instant::now() + Duration::from_secs(2);
        while !quiescent.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        let stopped_before_save = quiescent.load(Ordering::SeqCst);
        let retained_original = std::path::PathBuf::from(session.dir()).exists();
        let uid = shared.engine.meeting_uid(session.id()).unwrap();
        release.send(()).unwrap();
        writer.join().unwrap().unwrap();
        let final_id = finishing.join().unwrap().unwrap();
        assert!(
            stopped_before_save,
            "capture must stop while durable notes are still blocked"
        );
        assert!(
            retained_original,
            "rename must wait for the admitted notes write"
        );
        assert_eq!(
            shared.engine.meeting_notes(final_id.clone()),
            "The final acknowledged draft"
        );
        assert_eq!(shared.engine.meeting_uid(final_id.clone()), Some(uid));
        assert_ne!(final_id, session.id());
        assert!(with_session_notes(&shared, &token, |_| Ok(())).is_err());
    }

    #[test]
    fn failed_frontend_flush_never_keeps_capture_running_or_skips_finalization() {
        let (_root, shared, session, token) = recording();
        let quiescent = Arc::new(AtomicBool::new(false));
        let recorder =
            MeetingRecorder::testing(session.clone(), shared.engine.clone(), quiescent.clone());
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        let (result, flush) = finish_with_flush(&shared, &token, recorder, || {
            assert!(quiescent.load(Ordering::SeqCst));
            Err::<(), _>("notes storage failed")
        });
        assert!(flush.is_err());
        assert!(result.is_ok());
        assert!(session.is_finalized());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Idle
        );
    }

    #[test]
    fn a_failed_notes_write_during_stop_cannot_keep_capture_running() {
        let (_root, shared, session, token) = recording();
        let notes = std::path::PathBuf::from(session.dir()).join("notes.md");
        std::fs::remove_file(&notes).unwrap();
        std::fs::create_dir(&notes).unwrap();
        let quiescent = Arc::new(AtomicBool::new(false));
        let recorder =
            MeetingRecorder::testing(session.clone(), shared.engine.clone(), quiescent.clone());
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        let (result, saved) = finish_with_flush(&shared, &token, recorder, || {
            assert!(quiescent.load(Ordering::SeqCst));
            with_session_notes(&shared, &token, |session| {
                session
                    .set_notes("Retained unsaved draft".into())
                    .map_err(|error| error.to_string())
            })
        });
        assert!(saved.is_err());
        assert!(result.is_ok());
        assert!(session.is_finalized());
        assert!(quiescent.load(Ordering::SeqCst));
    }

    #[test]
    fn an_unsaved_failed_meeting_blocks_a_new_recording_and_keeps_its_context() {
        let (_root, shared, session, token) = failed_persistence();
        assert!(start(None, &shared, "Nieuwe meeting".into(), None, Vec::new()).is_err());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().session_id,
            Some(token.clone())
        );
        assert_eq!(
            shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .session
                .id(),
            session.id()
        );
        assert!(!session.is_finalized());
    }

    #[test]
    fn quitting_retries_storage_but_cannot_drop_an_unsaved_failed_meeting() {
        let (_root, shared, session, token) = failed_persistence();
        assert!(stop_and_wait(&shared).is_err());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Error
        );
        assert_eq!(
            shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .token,
            token
        );
        assert!(!session.is_finalized());
        std::fs::remove_dir(std::path::PathBuf::from(session.dir()).join("transcript.md")).unwrap();
        assert_eq!(stop_and_wait(&shared).unwrap(), Some(session.id()));
        assert!(session.is_finalized());
        assert!(shared.meeting_context.lock().unwrap().is_none());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Idle
        );
    }

    #[test]
    fn save_retry_rejects_a_stale_token_before_changing_the_failed_session() {
        let (_root, shared, session, token) = failed_persistence();
        assert!(retry_saved_session(&shared, "vorige-sessie").is_err());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().session_id,
            Some(token)
        );
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Error
        );
        assert!(!session.is_finalized());
    }

    #[test]
    fn save_retry_finishes_the_same_session_and_preserves_its_title_and_notes() {
        let (_root, shared, session, token) = failed_persistence();
        shared
            .meeting_context
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .title = "Herstelde titel".into();
        std::fs::remove_dir(std::path::PathBuf::from(session.dir()).join("transcript.md")).unwrap();
        let id = retry_saved_session(&shared, &token).unwrap();
        assert_ne!(id, session.id());
        assert_eq!(
            shared.engine.meeting_info(id.clone()).unwrap().title,
            "Herstelde titel"
        );
        assert_eq!(shared.engine.meeting_notes(id), "Een bewaarde afspraak.");
        assert!(session.is_finalized());
        assert!(shared.meeting_context.lock().unwrap().is_none());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Idle
        );
    }

    #[test]
    fn save_retry_finalizes_recording_metadata_after_transcript_already_saved() {
        let (_root, shared, session, token) = recording();
        shared
            .engine
            .prepare_meeting_recording(
                session.id(),
                crate::recording_metadata::environment("0.1.12"),
                false,
            )
            .unwrap();
        for track in shared.engine.meeting_audio_tracks(session.id()) {
            std::fs::write(
                std::path::PathBuf::from(session.dir()).join(track.relative_path),
                b"fixture audio",
            )
            .unwrap();
        }
        session.stop().unwrap();
        assert!(session.is_finalized());
        let metadata = std::path::PathBuf::from(session.dir()).join(".pennel/recording.json");
        let original = std::fs::read(&metadata).unwrap();
        std::fs::remove_file(&metadata).unwrap();
        std::fs::create_dir(&metadata).unwrap();
        {
            let mut context = shared.meeting_context.lock().unwrap();
            let context = context.as_mut().unwrap();
            context.title = "Saved title".into();
            context.pending_recording_finish = Some(crate::meeting::RecordingFinish {
                duration_ms: 12_345,
                complete: true,
            });
        }
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        fail(&shared, &token, "Metadata write failed");
        assert!(retry_saved_session(&shared, &token).is_err());
        assert!(shared.engine.meeting_info(session.id()).is_some());
        assert!(shared.meeting_context.lock().unwrap().is_some());
        std::fs::remove_dir(&metadata).unwrap();
        std::fs::write(&metadata, original).unwrap();
        let id = retry_saved_session(&shared, &token).unwrap();
        let metadata = shared
            .engine
            .meeting_recording_metadata(id.clone())
            .unwrap()
            .unwrap();
        assert_eq!(metadata.duration_ms, Some(12_345));
        assert_eq!(metadata.complete, Some(true));
        assert!(metadata.ended_at.is_some());
        assert_eq!(shared.engine.meeting_info(id).unwrap().title, "Saved title");
    }

    #[test]
    fn a_stale_health_watcher_cannot_stop_a_newer_recording() {
        let (_root, shared, session, token) = recording();
        assert!(
            take_recorder_matching(&shared, None, Some("vorige-sessie"))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().session_id,
            Some(token)
        );
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Recording
        );
        session.stop().unwrap();
    }

    #[test]
    fn stale_clean_end_token_cannot_stop_a_same_app_replacement_recording() {
        let (_root, shared, session, _token) = recording();
        let mut replacement = pennel_core::RecordingLifecycle::default();
        let current = replacement
            .begin(Some(crate::meeting_speaker_evidence::TEAMS.into()))
            .unwrap();
        replacement.recording(&current).unwrap();
        *shared.recording.lock().unwrap() = replacement;
        assert!(
            take_recorder_matching(
                &shared,
                Some(crate::meeting_speaker_evidence::TEAMS),
                Some("prior-coordinator-token")
            )
            .unwrap()
            .is_none()
        );
        let state = shared.recording.lock().unwrap().snapshot();
        assert_eq!(state.session_id.as_deref(), Some(current.as_str()));
        assert_eq!(state.phase, RecordingPhase::Recording);
        assert!(!shared.recording_cancel.load(Ordering::SeqCst));
        session.stop().unwrap();
    }

    #[test]
    fn a_durable_capture_failure_can_be_acknowledged_by_starting_again() {
        let (_root, shared, session, token) = recording();
        session.stop().unwrap();
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        assert_eq!(
            finish_after_capture(
                &shared,
                &token,
                Ok(session.id()),
                Some("Microfoon onderbroken".into()),
                "Originele titel",
                "Originele titel"
            ),
            Err("Microfoon onderbroken".into())
        );
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Error
        );
        assert!(shared.meeting_context.lock().unwrap().is_some());
        let next = begin_recording(&shared, None).unwrap();
        assert_ne!(next, token);
        assert!(shared.meeting_context.lock().unwrap().is_none());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Starting
        );
    }

    #[test]
    fn late_title_drafts_after_capture_failure_are_saved_before_quitting() {
        let (_root, shared, session, token) = recording();
        session.stop().unwrap();
        shared
            .recording
            .lock()
            .unwrap()
            .fail(&token, "Microfoon onderbroken".into())
            .unwrap();
        set_title(&shared, "Laatste titel".into(), &token).unwrap();
        let id = stop_and_wait(&shared).unwrap().unwrap();
        assert_eq!(
            shared.engine.meeting_info(id.clone()).unwrap().title,
            "Laatste titel"
        );
        assert_eq!(shared.engine.meeting_notes(id), "Een bewaarde afspraak.");
        assert!(shared.meeting_context.lock().unwrap().is_none());
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Idle
        );
    }

    #[test]
    fn starting_again_saves_the_finalized_failed_context_before_replacing_it() {
        let (_root, shared, session, token) = recording();
        session.stop().unwrap();
        shared
            .recording
            .lock()
            .unwrap()
            .fail(&token, "Microfoon onderbroken".into())
            .unwrap();
        set_title(&shared, "Titel na onderbreking".into(), &token).unwrap();
        let next = begin_recording(&shared, None).unwrap();
        assert_ne!(next, token);
        assert_eq!(shared.engine.meetings()[0].title, "Titel na onderbreking");
        assert!(shared.meeting_context.lock().unwrap().is_none());
    }

    fn failed_persistence() -> (tempfile::TempDir, Arc<Shared>, Arc<MeetingSession>, String) {
        let (root, shared, session, token) = recording();
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        std::fs::create_dir(std::path::PathBuf::from(session.dir()).join("transcript.md")).unwrap();
        let error = session.stop().unwrap_err().to_string();
        fail(&shared, &token, &error);
        (root, shared, session, token)
    }

    #[test]
    fn fresh_local_fast_meeting_finishes_without_a_missing_llm_failure() {
        let (_root, shared, session, _token) = recording();
        assert_eq!(shared.settings().mode, pennel_core::DictationMode::Fast);
        session.stop().unwrap();
        let keys = pennel_core::ApiKeys {
            by_provider: HashMap::new(),
        };
        assert_eq!(
            shared.settings().summary_automation,
            pennel_core::SummaryAutomation::Unanswered
        );
        assert!(
            shared
                .engine
                .summarize_meeting_automatically(
                    session.id(),
                    shared.settings().default_template,
                    keys
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(
            shared.engine.meeting_notes(session.id()),
            "Een bewaarde afspraak."
        );
        assert!(shared.engine.meeting_info(session.id()).is_some());
        assert!(shared.engine.meeting_markdown(session.id()).is_none());
    }

    #[test]
    fn configured_model_never_implies_automatic_summary_consent() {
        let (_root, shared, session, _token) = recording();
        session.stop().unwrap();
        let mut settings = shared.settings();
        let account = pennel_core::new_account(
            pennel_core::ProviderKind::Ollama,
            settings.providers.clone(),
        );
        settings.llm = pennel_core::ModelChoice {
            provider_id: account.id.clone(),
            model: "configured-model".into(),
        };
        settings.providers.push(account);
        let keys = pennel_core::ApiKeys {
            by_provider: HashMap::new(),
        };
        for policy in [
            pennel_core::SummaryAutomation::Unanswered,
            pennel_core::SummaryAutomation::Manual,
        ] {
            settings.summary_automation = policy;
            shared.engine.update_settings(settings.clone()).unwrap();
            assert!(
                shared
                    .engine
                    .summarize_meeting_automatically(
                        session.id(),
                        settings.default_template.clone(),
                        keys.clone()
                    )
                    .unwrap()
                    .is_none()
            );
        }
        assert!(shared.engine.meeting_markdown(session.id()).is_none());
    }

    #[test]
    fn consent_setting_off_never_changes_the_clipboard() {
        let copied = copy_consent(false, pennel_core::InterfaceLanguage::English, |_| {
            panic!("disabled setting must not copy")
        })
        .unwrap();
        assert!(!copied);
    }

    #[test]
    fn enabled_consent_reports_success_only_after_copying_the_canonical_sentence() {
        for language in [
            pennel_core::InterfaceLanguage::English,
            pennel_core::InterfaceLanguage::Dutch,
        ] {
            let mut observed = String::new();
            let copied = copy_consent(true, language, |text| {
                observed = text.to_owned();
                Ok(())
            })
            .unwrap();
            assert!(copied);
            assert_eq!(
                observed,
                pennel_core::consent_sentence_for_language(language)
            );
        }
    }

    #[test]
    fn failed_consent_copy_does_not_report_success() {
        assert!(
            copy_consent(true, pennel_core::InterfaceLanguage::English, |_| Err(
                "Clipboard unavailable.".into()
            ))
            .is_err()
        );
    }

    #[test]
    fn failed_setup_without_capture_does_not_prevent_quitting() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("preview"));
        let token = shared.recording.lock().unwrap().begin(None).unwrap();
        shared
            .recording
            .lock()
            .unwrap()
            .fail(&token, "Geen spraakmodel ingesteld".into())
            .unwrap();
        assert_eq!(stop_and_wait(&shared).unwrap(), None);
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Error
        );
    }

    fn recording() -> (tempfile::TempDir, Arc<Shared>, Arc<MeetingSession>, String) {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("preview");
        let engine =
            pennel_core::PennelEngine::new_preview(data_dir.to_string_lossy().into_owned())
                .unwrap();
        let session = engine
            .clone()
            .start_meeting(
                "Originele titel".into(),
                pennel_core::ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        session.set_notes("Een bewaarde afspraak.".into()).unwrap();
        let mut lifecycle = pennel_core::RecordingLifecycle::default();
        let token = lifecycle.begin(None).unwrap();
        lifecycle.recording(&token).unwrap();
        let (dictation, _) = std::sync::mpsc::channel();
        let uid = engine.meeting_uid(session.id()).unwrap();
        let shared = Arc::new(Shared {
            engine,
            system_interface_language: Default::default(),
            version: "0.2.0-test".into(),
            preview: true,
            migration_warning: None,
            secrets: Arc::new(crate::secrets::Secrets::in_memory()),
            data_dir,
            dictation,
            hotkey: Mutex::new(None),
            hotkey_error: Mutex::new(None),
            meeting: Mutex::new(None),
            recording: Mutex::new(lifecycle),
            recording_cancel: AtomicBool::new(false),
            library_jobs: AtomicUsize::new(0),
            live_questions: Mutex::new(crate::live_questions::LiveQuestions::default()),
            updating: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            dictation_active: AtomicBool::new(false),
            dictation_jobs: AtomicUsize::new(0),
            paste_jobs: AtomicUsize::new(0),
            writing_profile_sharing_epoch: Mutex::new(0),
            writing_profile_source_gate: Mutex::new(()),
            meeting_context: Mutex::new(Some(MeetingContext {
                token: token.clone(),
                uid,
                session: session.clone(),
                notes_gate: Arc::new(Mutex::new(true)),
                title: "Originele titel".into(),
                started: Instant::now(),
                duration_secs: None,
                pending_recording_finish: None,
                system_audio: false,
                heard_system_audio: false,
                audio_warning: None,
                consent_copied: false,
                speaker_evidence: None,
            })),
        });
        (root, shared, session, token)
    }

    #[test]
    fn editing_title_defers_folder_rename_until_session_flush_and_preserves_files() {
        let (_root, shared, session, token) = recording();
        let original_id = session.id();
        let original_dir = std::path::PathBuf::from(session.dir());
        set_title(&shared, "Nieuwe titel".into(), &token).unwrap();
        assert!(original_dir.exists());
        assert_eq!(
            shared
                .engine
                .meeting_info(original_id.clone())
                .unwrap()
                .title,
            "Originele titel"
        );
        assert_eq!(
            shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .title,
            "Nieuwe titel"
        );

        shared.recording.lock().unwrap().stopping(&token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        session.stop().unwrap();
        std::fs::write(
            original_dir.join("transcript.md"),
            b"[00:00] **Ik:** Een afgerond transcript.\n",
        )
        .unwrap();
        std::fs::write(original_dir.join("ik.m4a"), b"fixture me archive").unwrap();
        std::fs::write(original_dir.join("anderen.m4a"), b"fixture others archive").unwrap();
        let id = rename_after_recording(
            &shared.engine,
            original_id.clone(),
            "Originele titel",
            "Nieuwe titel",
        )
        .unwrap();
        let directory = std::path::PathBuf::from(shared.engine.meeting_dir(id.clone()));
        assert_ne!(id, original_id);
        assert!(!original_dir.exists());
        assert_eq!(
            shared.engine.meeting_info(id.clone()).unwrap().title,
            "Nieuwe titel"
        );
        assert_eq!(
            shared.engine.meeting_notes(id.clone()),
            "Een bewaarde afspraak."
        );
        assert_eq!(
            shared.engine.meeting_transcript(id)[0].text,
            "Een afgerond transcript."
        );
        assert_eq!(
            std::fs::read(directory.join("transcript.md")).unwrap(),
            b"[00:00] **Ik:** Een afgerond transcript.\n"
        );
        assert_eq!(
            std::fs::read(directory.join("ik.m4a")).unwrap(),
            b"fixture me archive"
        );
        assert_eq!(
            std::fs::read(directory.join("anderen.m4a")).unwrap(),
            b"fixture others archive"
        );
        assert_eq!(
            shared.recording.lock().unwrap().snapshot().phase,
            RecordingPhase::Finalizing
        );
    }

    #[test]
    fn blank_title_stays_editable_and_does_not_reset_the_stored_title() {
        let (_root, shared, session, token) = recording();
        set_title(&shared, "  ".into(), &token).unwrap();
        assert_eq!(
            shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .title,
            "  "
        );
        session.stop().unwrap();
        let id =
            rename_after_recording(&shared.engine, session.id(), "Originele titel", "  ").unwrap();
        assert_eq!(id, session.id());
        assert_eq!(
            shared.engine.meeting_info(id).unwrap().title,
            "Originele titel"
        );
    }

    #[test]
    fn stale_session_and_finalizing_edits_cannot_change_the_live_title() {
        let (_root, shared, session, token) = recording();
        assert!(set_title(&shared, "Vertraagde wijziging".into(), "vorige-sessie").is_err());
        assert_eq!(
            shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .title,
            "Originele titel"
        );
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        set_title(&shared, "Laatste conceptwijziging".into(), &token).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        assert!(set_title(&shared, "Nog later".into(), &token).is_err());
        assert_eq!(
            shared
                .meeting_context
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .title,
            "Laatste conceptwijziging"
        );
        session.stop().unwrap();
    }
}
