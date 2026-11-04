use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::capabilities::{CapabilityStatus, PermissionKind, PermissionStatus};
use crate::external_paste::{self, ExternalTarget};
use crate::state::Shared;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Pad,
    Practice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Listening,
    Working,
    Done,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct DictationSessionSnapshot {
    pub session_id: String,
    pub kind: SessionKind,
    pub phase: SessionPhase,
    pub text: String,
    pub error: Option<String>,
    pub target_name: Option<String>,
    pub external_paste_available: bool,
    pub external_paste_reason: Option<String>,
    pub cleanup_verification: Option<pennel_core::CleanupVerificationAvailability>,
}

impl DictationSessionSnapshot {
    fn localized(mut self) -> Self {
        self.error = self.error.map(crate::commands::localize_error_message);
        self.external_paste_reason = self.external_paste_reason.map(crate::ui::message);
        self
    }
}

struct Session {
    owner: String,
    snapshot: DictationSessionSnapshot,
    metadata_query: u64,
    live: Arc<AtomicBool>,
    capture: Option<Arc<AtomicBool>>,
    external: Vec<ExternalTarget>,
    verification_session: Option<Weak<pennel_core::LiveDictationSession>>,
}

impl Session {
    fn snapshot(&self) -> DictationSessionSnapshot {
        let mut snapshot = self.snapshot.clone();
        if let Some(session) = self.verification_session.as_ref().and_then(Weak::upgrade) {
            snapshot.cleanup_verification = Some(session.cleanup_verification_availability());
        }
        snapshot
    }
}

#[derive(Default)]
pub struct Sessions {
    next: AtomicU64,
    entries: Mutex<HashMap<String, Session>>,
    unsaved_pad: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct OwnedCapture {
    pub id: String,
    pub kind: SessionKind,
    live: Arc<AtomicBool>,
    recording_valid: Arc<AtomicBool>,
    pub external: Option<ExternalTarget>,
    pub external_requested: bool,
}

impl OwnedCapture {
    pub fn valid(&self) -> bool {
        self.live.load(Ordering::SeqCst) && self.recording_valid.load(Ordering::SeqCst)
    }
}

fn emit(app: &AppHandle, owner: &str, snapshot: &DictationSessionSnapshot) {
    let _ = app.emit_to(owner, "dictation-session", snapshot.clone().localized());
}

fn update(app: &AppHandle, id: &str, change: impl FnOnce(&mut DictationSessionSnapshot)) {
    let updated = {
        let sessions = app.state::<Sessions>();
        let mut entries = sessions.entries.lock().unwrap();
        entries.get_mut(id).map(|session| {
            session.metadata_query = session.metadata_query.wrapping_add(1);
            change(&mut session.snapshot);
            (session.owner.clone(), session.snapshot())
        })
    };
    if let Some((owner, snapshot)) = updated {
        emit(app, &owner, &snapshot);
    }
}

pub fn focused_session(app: &AppHandle) -> Option<String> {
    if !own_app_foreground(app) {
        return None;
    }
    let entries: Vec<_> = app
        .state::<Sessions>()
        .entries
        .lock()
        .unwrap()
        .iter()
        .map(|(id, session)| (id.clone(), session.owner.clone()))
        .collect();
    entries.into_iter().find_map(|(id, owner)| {
        app.get_webview_window(&owner)
            .filter(|window| window.is_focused().unwrap_or(false))
            .map(|_| id)
    })
}

pub fn own_app_foreground(app: &AppHandle) -> bool {
    crate::foreground::current().map_or_else(
        || {
            app.webview_windows().iter().any(|(label, window)| {
                crate::workspace::label_spec(label).is_some_and(|spec| spec.mode != "prompt")
                    && window.is_focused().unwrap_or(false)
            })
        },
        |foreground| foreground.pid == std::process::id(),
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum CaptureRoute {
    Global,
    Owned(String),
    Unavailable,
}

impl CaptureRoute {
    pub fn allows_command(&self) -> bool {
        matches!(self, Self::Global)
    }

    pub fn resolve(own_app_active: bool, owned: Option<String>) -> Self {
        match (own_app_active, owned) {
            (true, Some(id)) => Self::Owned(id),
            (false, None) => Self::Global,
            _ => Self::Unavailable,
        }
    }
}

pub fn begin_capture(app: &AppHandle, shared: &Shared, id: &str) -> Result<OwnedCapture, String> {
    let (kind, live, owner) = {
        let sessions = app.state::<Sessions>();
        let entries = sessions.entries.lock().unwrap();
        let session = entries.get(id).ok_or("This dictation session is closed")?;
        if matches!(
            session.snapshot.phase,
            SessionPhase::Listening | SessionPhase::Working
        ) {
            return Err("Finish your current dictation first".into());
        }
        (
            session.snapshot.kind,
            session.live.clone(),
            session.owner.clone(),
        )
    };
    if !own_app_foreground(app)
        || !app
            .get_webview_window(&owner)
            .is_some_and(|window| window.is_focused().unwrap_or(false))
    {
        return Err("Activate the dictation pad or practice field before starting".into());
    }
    let readiness = readiness(shared, "practice");
    if !readiness.can_record {
        return Err(readiness
            .reason
            .unwrap_or("Set up speech recognition first".into()));
    }
    if shared.dictation_jobs.load(Ordering::SeqCst) > 0
        || shared.paste_jobs.load(Ordering::SeqCst) > 0
    {
        return Err("Wait until your previous dictation has finished processing".into());
    }
    if kind == SessionKind::Practice && shared.recording.lock().unwrap().is_busy() {
        return Err("Finish your meeting recording before trying the introduction".into());
    }
    let external_requested = kind == SessionKind::Pad && shared.settings().dictation_pad_auto_paste;
    let external = external_requested.then(external_paste::capture).flatten();
    let valid = Arc::new(AtomicBool::new(true));
    let capture = OwnedCapture {
        id: id.into(),
        kind,
        live,
        recording_valid: valid.clone(),
        external,
        external_requested,
    };
    let previous_targets = {
        let sessions = app.state::<Sessions>();
        let mut entries = sessions.entries.lock().unwrap();
        let session = entries
            .get_mut(id)
            .ok_or("This dictation session is closed")?;
        session.capture = Some(valid);
        // Navigation's idle-only close uses this same lock, including while capture is starting.
        session.snapshot.phase = SessionPhase::Listening;
        session.snapshot.error = None;
        let previous_targets = std::mem::take(&mut session.external);
        if let Some(target) = &capture.external {
            session.external.push(target.clone());
        }
        previous_targets
    };
    for target in previous_targets {
        target.cancel();
    }
    update(app, id, |snapshot| {
        snapshot.phase = SessionPhase::Listening;
        snapshot.error = None;
        snapshot.target_name = capture.external.as_ref().map(|target| target.name.clone());
    });
    Ok(capture)
}

pub fn phase(app: &AppHandle, capture: &OwnedCapture, phase: SessionPhase) {
    if capture.valid() {
        update(app, &capture.id, |snapshot| {
            snapshot.phase = phase;
            snapshot.error = None;
        });
    }
}

pub fn verification(
    app: &AppHandle,
    capture: &OwnedCapture,
    source: &Arc<pennel_core::LiveDictationSession>,
) {
    if capture.valid() {
        let updated = {
            let sessions = app.state::<Sessions>();
            let mut entries = sessions.entries.lock().unwrap();
            entries.get_mut(&capture.id).map(|session| {
                session.verification_session = Some(Arc::downgrade(source));
                session.snapshot.cleanup_verification =
                    Some(source.cleanup_verification_availability());
                (session.owner.clone(), session.snapshot())
            })
        };
        if let Some((owner, snapshot)) = updated {
            emit(app, &owner, &snapshot);
        }
    }
}

pub fn refresh_verification(app: &AppHandle) {
    let snapshots: Vec<_> = app
        .state::<Sessions>()
        .entries
        .lock()
        .unwrap()
        .values()
        .filter(|session| {
            matches!(
                session.snapshot.phase,
                SessionPhase::Listening | SessionPhase::Working
            )
        })
        .map(|session| (session.owner.clone(), session.snapshot()))
        .collect();
    for (owner, snapshot) in snapshots {
        emit(app, &owner, &snapshot);
    }
}

pub fn failed(app: &AppHandle, id: &str, error: String) {
    update(app, id, |snapshot| {
        snapshot.phase = SessionPhase::Error;
        snapshot.error = Some(error);
    });
}

pub fn cancelled(app: &AppHandle, capture: &OwnedCapture) {
    capture.recording_valid.store(false, Ordering::SeqCst);
    if let Some(target) = &capture.external {
        target.cancel();
    }
    update(app, &capture.id, |snapshot| {
        snapshot.phase = SessionPhase::Idle;
        snapshot.error = None;
    });
}

pub fn complete(app: &AppHandle, shared: &Shared, capture: &OwnedCapture, text: String) -> bool {
    if text.trim().is_empty() {
        if capture.valid() {
            failed(app, &capture.id, "No speech recognized. Try again.".into());
        }
        return false;
    }
    let value = match retain_result(shared, capture, text) {
        Ok(Some(value)) => value,
        Ok(None) => return false,
        Err(unsaved) => {
            *app.state::<Sessions>().unsaved_pad.lock().unwrap() = Some(unsaved.text.clone());
            if capture.valid() {
                update(app, &capture.id, |snapshot| {
                    snapshot.phase = SessionPhase::Error;
                    snapshot.text = unsaved.text;
                    snapshot.error = Some(format!(
                        "Saving failed. Your text is here; copy it or try again: {}",
                        unsaved.error
                    ));
                });
            }
            return false;
        }
    };
    if !capture.valid() {
        return false;
    }
    if capture.kind == SessionKind::Practice {
        let mut settings = shared.settings();
        if settings.onboarding.finish_dictation_practice() {
            if let Err(error) = shared.engine.update_settings(settings) {
                failed(app, &capture.id, error.to_string());
                return false;
            }
            let _ = app.emit("settings-changed", ());
        }
    }
    let external_notice = if capture.external_requested && capture.external.is_none() {
        Some("No destination app is available. Your text is in Pennel.".into())
    } else if capture.external_requested && !external_allowed(shared, capture) {
        Some("External paste was cancelled. Your text is in Pennel.".into())
    } else {
        None
    };
    update(app, &capture.id, |snapshot| {
        snapshot.phase = SessionPhase::Done;
        snapshot.text = value;
        snapshot.error = external_notice;
    });
    true
}

#[derive(Debug)]
struct UnsavedResult {
    error: pennel_core::PennelError,
    text: String,
}

fn joined_draft(mut draft: String, text: &str) -> String {
    if !draft.is_empty()
        && !draft.ends_with(char::is_whitespace)
        && !text.starts_with(char::is_whitespace)
    {
        draft.push(' ');
    }
    draft.push_str(text);
    draft
}

fn retain_result(
    shared: &Shared,
    capture: &OwnedCapture,
    text: String,
) -> Result<Option<String>, UnsavedResult> {
    if capture.kind == SessionKind::Pad {
        if !capture.recording_valid.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let fallback = joined_draft(shared.engine.dictation_pad_text(), &text);
        shared
            .engine
            .append_dictation_pad(text)
            .map(Some)
            .map_err(|error| UnsavedResult {
                error,
                text: fallback,
            })
    } else if capture.valid() {
        Ok(Some(text))
    } else {
        Ok(None)
    }
}

pub fn delivery_error(app: &AppHandle, capture: &OwnedCapture, error: String) {
    if capture.valid() {
        update(app, &capture.id, |snapshot| snapshot.error = Some(error));
    }
}

pub fn external_allowed(shared: &Shared, capture: &OwnedCapture) -> bool {
    capture.kind == SessionKind::Pad
        && capture.valid()
        && shared.settings().dictation_pad_auto_paste
        && capture
            .external
            .as_ref()
            .is_some_and(ExternalTarget::is_valid)
}

fn close(app: &AppHandle, owner: &str, id: &str) -> Result<(), String> {
    close_with_policy(app, owner, id, false)
}

fn take_session(
    entries: &mut HashMap<String, Session>,
    owner: &str,
    id: &str,
    only_if_idle: bool,
) -> Result<Option<Session>, String> {
    if let Some(session) = entries.get(id) {
        if session.owner != owner {
            return Err("This dictation session belongs to another window".into());
        }
        if only_if_idle
            && matches!(
                session.snapshot.phase,
                SessionPhase::Listening | SessionPhase::Working
            )
        {
            return Err("Finish your current dictation first".into());
        }
    }
    Ok(entries.remove(id))
}

fn close_with_policy(
    app: &AppHandle,
    owner: &str,
    id: &str,
    only_if_idle: bool,
) -> Result<(), String> {
    let removed = {
        let sessions = app.state::<Sessions>();
        let mut entries = sessions.entries.lock().unwrap();
        take_session(&mut entries, owner, id, only_if_idle)?
    };
    if let Some(session) = removed {
        session.live.store(false, Ordering::SeqCst);
        if session.snapshot.kind == SessionKind::Practice
            && let Some(capture) = session.capture
        {
            capture.store(false, Ordering::SeqCst);
        }
        for target in session.external {
            target.cancel();
        }
        if let Some(shared) = app.try_state::<Arc<Shared>>() {
            let _ = shared
                .dictation
                .send(crate::dictation::Msg::OwnedClosed(id.into()));
        }
    }
    Ok(())
}

fn session_owner_allowed(kind: SessionKind, owner: &str) -> bool {
    matches!(
        (kind, owner),
        (SessionKind::Pad, "main") | (SessionKind::Practice, "onboarding")
    )
}

pub fn close_owner(app: &AppHandle, owner: &str) {
    let ids: Vec<_> = app
        .state::<Sessions>()
        .entries
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, session)| session.owner == owner)
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        let _ = close(app, owner, &id);
    }
}

pub fn focus_lost(app: &AppHandle, owner: &str) {
    let ids: Vec<_> = app
        .state::<Sessions>()
        .entries
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, session)| {
            session.owner == owner && session.snapshot.kind == SessionKind::Practice
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        cancel_recording(app, &id);
        if let Some(shared) = app.try_state::<Arc<Shared>>() {
            let _ = shared
                .dictation
                .send(crate::dictation::Msg::OwnedCancel(id));
        }
    }
}

fn cancel_recording(app: &AppHandle, id: &str) {
    let targets = {
        let sessions = app.state::<Sessions>();
        let entries = sessions.entries.lock().unwrap();
        entries
            .get(id)
            .map(|session| {
                if let Some(capture) = &session.capture {
                    capture.store(false, Ordering::SeqCst);
                }
                session.external.clone()
            })
            .unwrap_or_default()
    };
    for target in targets {
        target.cancel();
    }
    update(app, id, |snapshot| {
        snapshot.phase = SessionPhase::Idle;
        snapshot.error = None;
    });
}

pub fn invalidate_external(app: &AppHandle) {
    let targets: Vec<_> = app
        .state::<Sessions>()
        .entries
        .lock()
        .unwrap()
        .values()
        .flat_map(|session| session.external.iter().cloned())
        .collect();
    for target in targets {
        target.cancel();
    }
}

pub fn refresh_idle_pads(app: &AppHandle, shared: &Shared) {
    if shared.dictation_jobs.load(Ordering::SeqCst) != 0 {
        return;
    }
    let unsaved = app.state::<Sessions>().unsaved_pad.lock().unwrap().clone();
    let text = unsaved
        .clone()
        .unwrap_or_else(|| shared.engine.dictation_pad_text());
    let updates: Vec<_> = {
        let sessions = app.state::<Sessions>();
        let mut entries = sessions.entries.lock().unwrap();
        entries
            .values_mut()
            .filter(|session| {
                session.snapshot.kind == SessionKind::Pad && session.capture.is_none()
            })
            .map(|session| {
                session.metadata_query = session.metadata_query.wrapping_add(1);
                session.snapshot.text = text.clone();
                session.snapshot.phase = if unsaved.is_some() {
                    SessionPhase::Error
                } else {
                    SessionPhase::Idle
                };
                if unsaved.is_some() {
                    session.snapshot.error = Some(
                        "Your dictation pad has not been saved. Copy your text or try again."
                            .into(),
                    );
                }
                (session.owner.clone(), session.snapshot())
            })
            .collect()
    };
    for (owner, snapshot) in updates {
        emit(app, &owner, &snapshot);
    }
}

fn access(capability: &crate::capabilities::Capability) -> pennel_core::SetupAccess {
    match capability.status {
        CapabilityStatus::Available => pennel_core::SetupAccess::Available,
        CapabilityStatus::PermissionRequired => pennel_core::SetupAccess::PermissionRequired,
        CapabilityStatus::Unsupported => pennel_core::SetupAccess::Unsupported,
        CapabilityStatus::Unavailable => pennel_core::SetupAccess::Unavailable,
    }
}

pub fn readiness(shared: &Shared, target: &str) -> pennel_core::SetupReadiness {
    let settings = shared.settings();
    let capabilities =
        crate::capabilities::snapshot(shared.hotkey_error.lock().unwrap().as_deref());
    let microphone = crate::capabilities::permissions()
        .iter()
        .find(|permission| permission.kind == PermissionKind::Microphone)
        .map_or(
            pennel_core::SetupAccess::Unknown,
            |permission| match permission.status {
                PermissionStatus::Granted | PermissionStatus::NotRequired => {
                    pennel_core::SetupAccess::Available
                }
                PermissionStatus::Unknown => pennel_core::SetupAccess::Unknown,
                PermissionStatus::Unsupported => pennel_core::SetupAccess::Unsupported,
                PermissionStatus::Restricted => pennel_core::SetupAccess::Unavailable,
                _ => pennel_core::SetupAccess::PermissionRequired,
            },
        );
    let facts = pennel_core::SetupFacts {
        microphone,
        global_hotkey: access(&capabilities.global_hotkey),
        external_paste: access(&capabilities.selection),
        apple_speech: access(&capabilities.apple_speech),
        keyed_providers: settings
            .providers
            .iter()
            .filter(|provider| shared.secrets.has(&provider.id).unwrap_or(false))
            .map(|provider| provider.id.clone())
            .collect(),
    };
    let downloaded = shared
        .engine
        .local_models(None, false)
        .into_iter()
        .filter(|model| {
            shared.engine.model_state(model.id.clone()) == pennel_core::ModelState::Downloaded
        })
        .map(|model| model.id)
        .collect();
    if target == "meeting" {
        pennel_core::meeting_readiness(settings, facts, downloaded)
    } else {
        pennel_core::dictation_readiness(settings, facts, downloaded)
    }
}

#[tauri::command]
pub fn onboarding_readiness(
    target: String,
    shared: State<'_, Arc<Shared>>,
) -> pennel_core::SetupReadiness {
    let mut result = readiness(&shared, &target);
    result.reason = result.reason.map(crate::ui::message);
    result
}

#[tauri::command]
pub fn pad_text(shared: State<'_, Arc<Shared>>, app: AppHandle) -> String {
    app.state::<Sessions>()
        .unsaved_pad
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| shared.engine.dictation_pad_text())
}

#[tauri::command]
pub fn save_pad_text(
    text: String,
    shared: State<'_, Arc<Shared>>,
    app: AppHandle,
) -> Result<(), String> {
    shared
        .engine
        .save_dictation_pad(text.clone())
        .map_err(|error| error.to_string())?;
    *app.state::<Sessions>().unsaved_pad.lock().unwrap() = None;
    let updates = sync_pad_snapshots(&app.state::<Sessions>(), &text);
    for (owner, snapshot) in updates {
        emit(&app, &owner, &snapshot);
    }
    Ok(())
}

fn sync_pad_snapshots(sessions: &Sessions, text: &str) -> Vec<(String, DictationSessionSnapshot)> {
    sessions
        .entries
        .lock()
        .unwrap()
        .values_mut()
        .filter(|session| session.snapshot.kind == SessionKind::Pad)
        .map(|session| {
            session.snapshot.text = text.into();
            if session.snapshot.phase == SessionPhase::Error {
                session.snapshot.phase = SessionPhase::Idle;
                session.snapshot.error = None;
            }
            (session.owner.clone(), session.snapshot())
        })
        .collect()
}

#[tauri::command]
pub fn open_dictation_session(
    kind: SessionKind,
    window: WebviewWindow,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<DictationSessionSnapshot, String> {
    let owner = window.label();
    if !session_owner_allowed(kind, owner) {
        return Err("This window cannot open a dictation session".into());
    }
    close_owner(&app, owner);
    let id = format!(
        "{}-{}",
        std::process::id(),
        app.state::<Sessions>().next.fetch_add(1, Ordering::SeqCst)
    );
    let (external_paste_available, external_paste_reason) = external_paste::availability();
    let unsaved = kind == SessionKind::Pad
        && app
            .state::<Sessions>()
            .unsaved_pad
            .lock()
            .unwrap()
            .is_some();
    let snapshot = DictationSessionSnapshot {
        cleanup_verification: None,
        session_id: id.clone(),
        kind,
        phase: if kind == SessionKind::Pad && shared.dictation_jobs.load(Ordering::SeqCst) > 0 {
            SessionPhase::Working
        } else if unsaved {
            SessionPhase::Error
        } else {
            SessionPhase::Idle
        },
        text: if kind == SessionKind::Pad {
            pad_text(shared.clone(), app.clone())
        } else {
            String::new()
        },
        error: unsaved
            .then(|| "Your dictation pad has not been saved. Copy your text or try again.".into()),
        target_name: if kind == SessionKind::Pad {
            external_paste::hint()
        } else {
            None
        },
        external_paste_available,
        external_paste_reason,
    };
    app.state::<Sessions>().entries.lock().unwrap().insert(
        id,
        Session {
            owner: owner.into(),
            snapshot: snapshot.clone(),
            metadata_query: 0,
            live: Arc::new(AtomicBool::new(true)),
            capture: None,
            external: Vec::new(),
            verification_session: None,
        },
    );
    Ok(snapshot.localized())
}

#[tauri::command]
pub fn get_dictation_session(
    session_id: String,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<DictationSessionSnapshot, String> {
    let sessions = app.state::<Sessions>();
    let query = {
        let mut entries = sessions.entries.lock().unwrap();
        let session = entries
            .get_mut(&session_id)
            .filter(|session| session.owner == window.label())
            .ok_or("This dictation session is closed")?;
        let Some(query) = begin_target_refresh(session) else {
            return Ok(session.snapshot().localized());
        };
        query
    };
    // Native foreground inspection may dispatch onto the main thread; never hold the session lock here.
    let availability = external_paste::availability();
    let target_name = external_paste::hint();
    let mut entries = sessions.entries.lock().unwrap();
    let session = entries
        .get_mut(&session_id)
        .filter(|session| session.owner == window.label())
        .ok_or("This dictation session is closed")?;
    apply_target_refresh(session, query, target_name, availability);
    Ok(session.snapshot().localized())
}

fn idle_pad(session: &Session) -> bool {
    session.snapshot.kind == SessionKind::Pad
        && !matches!(
            session.snapshot.phase,
            SessionPhase::Listening | SessionPhase::Working
        )
}

fn begin_target_refresh(session: &mut Session) -> Option<u64> {
    if !idle_pad(session) {
        return None;
    }
    session.metadata_query = session.metadata_query.wrapping_add(1);
    Some(session.metadata_query)
}

fn apply_target_refresh(
    session: &mut Session,
    query: u64,
    target_name: Option<String>,
    availability: (bool, Option<String>),
) -> bool {
    if !idle_pad(session) || session.metadata_query != query {
        return false;
    }
    session.snapshot.target_name = target_name;
    session.snapshot.external_paste_available = availability.0;
    session.snapshot.external_paste_reason = availability.1;
    true
}

#[tauri::command]
pub fn toggle_dictation_session(
    session_id: String,
    window: WebviewWindow,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<(), String> {
    get_dictation_session(session_id.clone(), window, app)?;
    shared
        .dictation
        .send(crate::dictation::Msg::OwnedToggle(session_id))
        .map_err(|_| "Dictation is unavailable".into())
}

#[tauri::command]
pub fn cancel_dictation_session(
    session_id: String,
    window: WebviewWindow,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<(), String> {
    get_dictation_session(session_id.clone(), window, app.clone())?;
    cancel_recording(&app, &session_id);
    shared
        .dictation
        .send(crate::dictation::Msg::OwnedCancel(session_id))
        .map_err(|_| "Dictation is unavailable".into())
}

#[tauri::command]
pub fn close_dictation_session(
    session_id: String,
    only_if_idle: Option<bool>,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<(), String> {
    close_with_policy(
        &app,
        window.label(),
        &session_id,
        only_if_idle.unwrap_or(false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_pad_and_owned_onboarding_practice_can_open_sessions() {
        assert!(session_owner_allowed(SessionKind::Pad, "main"));
        assert!(session_owner_allowed(SessionKind::Practice, "onboarding"));
        assert!(!session_owner_allowed(SessionKind::Practice, "main"));
        assert!(!session_owner_allowed(SessionKind::Pad, "history"));
        assert!(!session_owner_allowed(SessionKind::Pad, "onboarding"));
    }

    #[test]
    fn guarded_navigation_cannot_remove_a_listening_or_processing_session() {
        for phase in [SessionPhase::Listening, SessionPhase::Working] {
            let mut session = metadata_session(SessionKind::Pad, phase);
            session.owner = "main".into();
            session.snapshot.text = "Retained local draft".into();
            let live = session.live.clone();
            let mut entries = HashMap::from([("pad".into(), session)]);
            assert!(take_session(&mut entries, "main", "pad", true).is_err());
            assert_eq!(entries["pad"].snapshot.text, "Retained local draft");
            assert!(live.load(Ordering::SeqCst));
            assert!(
                take_session(&mut entries, "main", "pad", false)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn idle_navigation_releases_only_its_own_session() {
        for phase in [SessionPhase::Idle, SessionPhase::Done, SessionPhase::Error] {
            let mut session = metadata_session(SessionKind::Pad, phase);
            session.owner = "main".into();
            let mut entries = HashMap::from([("pad".into(), session)]);
            assert!(take_session(&mut entries, "history", "pad", true).is_err());
            assert!(entries.contains_key("pad"));
            assert!(
                take_session(&mut entries, "main", "pad", true)
                    .unwrap()
                    .is_some()
            );
            assert!(entries.is_empty());
        }
    }

    #[test]
    fn localization_changes_owned_messages_but_never_dictated_text_or_app_names() {
        let content = "Geef Pennel toegang tot de microfoon.";
        let snapshot = DictationSessionSnapshot {
            cleanup_verification: None,
            session_id: "synthetic-owned-session".into(),
            kind: SessionKind::Pad,
            phase: SessionPhase::Error,
            text: content.into(),
            error: Some(content.into()),
            target_name: Some(content.into()),
            external_paste_available: false,
            external_paste_reason: Some(
                "Geen doelapp beschikbaar. Je tekst staat in Pennel.".into(),
            ),
        }
        .localized();
        assert_eq!(snapshot.text, content);
        assert_eq!(snapshot.target_name.as_deref(), Some(content));
        assert_eq!(
            snapshot.error.as_deref(),
            Some("Give Pennel microphone access.")
        );
        assert_eq!(
            snapshot.external_paste_reason.as_deref(),
            Some("No destination app is available. Your text is in Pennel.")
        );
    }

    fn capture(kind: SessionKind) -> OwnedCapture {
        OwnedCapture {
            id: "owned-test".into(),
            kind,
            live: Arc::new(AtomicBool::new(true)),
            recording_valid: Arc::new(AtomicBool::new(true)),
            external: None,
            external_requested: false,
        }
    }

    #[test]
    fn closing_or_cancelling_capture_cannot_revalidate_an_old_result() {
        let old = capture(SessionKind::Practice);
        old.recording_valid.store(false, Ordering::SeqCst);
        let new = capture(SessionKind::Practice);
        assert!(!old.valid());
        assert!(new.valid());
        old.live.store(false, Ordering::SeqCst);
        assert!(!old.valid());
    }

    #[test]
    fn own_window_without_live_owner_never_resolves_to_external_or_command_capture() {
        assert_eq!(CaptureRoute::resolve(true, None), CaptureRoute::Unavailable);
        assert_eq!(
            CaptureRoute::resolve(true, Some("practice".into())),
            CaptureRoute::Owned("practice".into())
        );
        assert_eq!(
            CaptureRoute::resolve(false, Some("closed-owner".into())),
            CaptureRoute::Unavailable
        );
        assert_eq!(CaptureRoute::resolve(false, None), CaptureRoute::Global);
        assert!(!CaptureRoute::resolve(true, None).allows_command());
        assert!(!CaptureRoute::resolve(false, Some("closed-owner".into())).allows_command());
        assert!(CaptureRoute::resolve(false, None).allows_command());
    }

    #[test]
    fn a_closed_pad_retains_text_but_never_authorizes_external_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing(directory.path().into());
        shared
            .engine
            .save_dictation_pad("Mijn concept".into())
            .unwrap();
        let pad = capture(SessionKind::Pad);
        pad.live.store(false, Ordering::SeqCst);
        assert_eq!(
            retain_result(&shared, &pad, "aanvulling".into())
                .unwrap()
                .as_deref(),
            Some("Mijn concept aanvulling")
        );
        assert!(!external_allowed(&shared, &pad));
    }

    #[test]
    fn cancelled_practice_cannot_write_to_the_pad() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing(directory.path().into());
        let practice = capture(SessionKind::Practice);
        practice.recording_valid.store(false, Ordering::SeqCst);
        assert!(
            retain_result(&shared, &practice, "private practice".into())
                .unwrap()
                .is_none()
        );
        assert!(shared.engine.dictation_pad_text().is_empty());
        assert!(!external_allowed(&shared, &practice));
    }

    #[test]
    fn an_explicitly_cancelled_pad_result_is_not_appended_later() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing(directory.path().into());
        shared
            .engine
            .save_dictation_pad("Mijn concept".into())
            .unwrap();
        let pad = capture(SessionKind::Pad);
        pad.recording_valid.store(false, Ordering::SeqCst);
        assert!(
            retain_result(&shared, &pad, "geannuleerd".into())
                .unwrap()
                .is_none()
        );
        assert_eq!(shared.engine.dictation_pad_text(), "Mijn concept");
    }

    #[test]
    fn session_contract_has_no_arbitrary_process_or_credential_fields() {
        let snapshot = DictationSessionSnapshot {
            cleanup_verification: Some(pennel_core::CleanupVerificationAvailability {
                enabled: false,
                available: false,
                reason_code: "unqualified".into(),
                provider_id: Some("synthetic-provider".into()),
                model: Some("synthetic-model".into()),
                endpoint: Some("https://synthetic.example/v1/decisions".into()),
            }),
            session_id: "example".into(),
            kind: SessionKind::Practice,
            phase: SessionPhase::Done,
            text: "Mijn dictaat".into(),
            error: None,
            target_name: None,
            external_paste_available: false,
            external_paste_reason: None,
        };
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value["kind"], "practice");
        assert_eq!(value["phase"], "done");
        assert!(value.get("pid").is_none());
        assert!(value.get("keys").is_none());
        let fields: std::collections::BTreeSet<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            std::collections::BTreeSet::from([
                "session_id",
                "kind",
                "phase",
                "text",
                "error",
                "target_name",
                "external_paste_available",
                "external_paste_reason",
                "cleanup_verification",
            ])
        );
        let verification = value["cleanup_verification"].as_object().unwrap();
        let fields: std::collections::BTreeSet<_> =
            verification.keys().map(String::as_str).collect();
        assert_eq!(
            fields,
            std::collections::BTreeSet::from([
                "enabled",
                "available",
                "reason_code",
                "provider_id",
                "model",
                "endpoint"
            ])
        );
        assert_eq!(
            verification["endpoint"],
            "https://synthetic.example/v1/decisions"
        );
    }

    #[test]
    fn a_failed_pad_append_retains_the_joined_recognition_result_for_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing(directory.path().into());
        shared
            .engine
            .save_dictation_pad("Mijn concept".into())
            .unwrap();
        std::fs::remove_file(directory.path().join("dictation-pad.txt")).unwrap();
        std::fs::create_dir(directory.path().join("dictation-pad.txt")).unwrap();
        let pad = capture(SessionKind::Pad);
        let error = retain_result(&shared, &pad, "herkende tekst".into()).unwrap_err();
        assert_eq!(error.text, "Mijn concept herkende tekst");
        assert_eq!(shared.engine.dictation_pad_text(), "Mijn concept");
        assert!(!external_allowed(&shared, &pad));
    }

    #[test]
    fn saving_manual_edits_updates_the_session_snapshot_used_by_focus_refresh() {
        let sessions = Sessions::default();
        sessions.entries.lock().unwrap().insert(
            "pad".into(),
            Session {
                owner: "dictation".into(),
                metadata_query: 0,
                snapshot: DictationSessionSnapshot {
                    cleanup_verification: None,
                    session_id: "pad".into(),
                    kind: SessionKind::Pad,
                    phase: SessionPhase::Idle,
                    text: "oude tekst".into(),
                    error: None,
                    target_name: None,
                    external_paste_available: false,
                    external_paste_reason: None,
                },
                live: Arc::new(AtomicBool::new(true)),
                capture: None,
                external: Vec::new(),
                verification_session: None,
            },
        );
        let updates = sync_pad_snapshots(&sessions, "handmatige wijziging");
        assert_eq!(updates[0].1.text, "handmatige wijziging");
        assert_eq!(
            sessions.entries.lock().unwrap()["pad"].snapshot.text,
            "handmatige wijziging"
        );
    }

    fn metadata_session(kind: SessionKind, phase: SessionPhase) -> Session {
        Session {
            owner: "dictation".into(),
            snapshot: DictationSessionSnapshot {
                cleanup_verification: None,
                session_id: "pad-metadata".into(),
                kind,
                phase,
                text: "Mijn bewaarde of onbewaarde tekst".into(),
                error: Some("Nog niet bewaard".into()),
                target_name: Some("Mail".into()),
                external_paste_available: true,
                external_paste_reason: None,
            },
            metadata_query: 0,
            live: Arc::new(AtomicBool::new(true)),
            capture: None,
            external: Vec::new(),
            verification_session: None,
        }
    }

    #[test]
    fn idle_pad_target_refresh_keeps_draft_and_error_and_rejects_older_metadata() {
        let mut session = metadata_session(SessionKind::Pad, SessionPhase::Error);
        let old = begin_target_refresh(&mut session).unwrap();
        let current = begin_target_refresh(&mut session).unwrap();
        assert!(apply_target_refresh(
            &mut session,
            current,
            Some("Chrome".into()),
            (true, None)
        ));
        assert!(!apply_target_refresh(
            &mut session,
            old,
            Some("Mail".into()),
            (false, Some("oud".into()))
        ));
        assert_eq!(session.snapshot.target_name.as_deref(), Some("Chrome"));
        assert!(session.snapshot.external_paste_available);
        assert_eq!(session.snapshot.text, "Mijn bewaarde of onbewaarde tekst");
        assert_eq!(session.snapshot.error.as_deref(), Some("Nog niet bewaard"));
        assert_eq!(session.snapshot.phase, SessionPhase::Error);
    }

    #[test]
    fn a_started_capture_keeps_its_captured_target_despite_an_older_refocus_query() {
        let mut session = metadata_session(SessionKind::Pad, SessionPhase::Idle);
        let query = begin_target_refresh(&mut session).unwrap();
        session.snapshot.phase = SessionPhase::Listening;
        session.snapshot.target_name = Some("Vastgelegd doel".into());
        assert!(!apply_target_refresh(
            &mut session,
            query,
            Some("Andere app".into()),
            (false, None)
        ));
        assert!(begin_target_refresh(&mut session).is_none());
        session.snapshot.phase = SessionPhase::Working;
        assert!(begin_target_refresh(&mut session).is_none());
        assert_eq!(
            session.snapshot.target_name.as_deref(),
            Some("Vastgelegd doel")
        );
        assert_eq!(session.snapshot.text, "Mijn bewaarde of onbewaarde tekst");
    }

    #[test]
    fn practice_never_starts_external_target_metadata_inspection() {
        let mut session = metadata_session(SessionKind::Practice, SessionPhase::Idle);
        assert!(begin_target_refresh(&mut session).is_none());
        assert_eq!(session.metadata_query, 0);
    }
}
