//! Port of `DictationController.swift`: hotkey gestures drive the recorder, the core transcribes,
//! results are pasted in the order the recordings started.

use std::collections::HashMap;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use pennel_core::DictationPreviewMode;
use pennel_core::{
    CleanupStatus, DictationPreviewObserver, DictationPreviewUpdate, DictationRequest,
    DictationResult, LiveDictationSession, PennelError,
};
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::audio::{self, CaptureError, Recorder};
#[cfg(not(target_os = "macos"))]
use crate::clipboard::{self, Clipboard, LateCopyWatch};
#[cfg(not(target_os = "macos"))]
use crate::clipboard_system::SystemClipboard;
use crate::diagnostics::Log;
use crate::external_paste::{self, ExternalTarget};
use crate::foreground::{self, ForegroundApp};
use crate::gesture::{GestureAction, HotkeyGesture};
use crate::hotkey::{self, HotkeyEvent};
use crate::keys;
#[cfg(not(target_os = "macos"))]
use crate::keys::Shortcut;
use crate::owned_dictation::{self, OwnedCapture, SessionPhase};
use crate::paste_queue::PasteQueue;
use crate::pill::Pill;
use crate::resample::TARGET_RATE;
use crate::state::Shared;
use crate::status::{CommandIntent, DictationStatus, NoSoundWatch, PillState};

const LOG: Log = Log::new("dictation");
const TICK: Duration = Duration::from_secs(1);
const PASTE_KEY: &str = if cfg!(target_os = "macos") {
    "⌘V"
} else {
    "Ctrl+V"
};
const KEPT_IN_HISTORY: &str = "Your words are kept in History";

pub enum Msg {
    PrepareShutdown(Sender<()>),
    Shutdown(Sender<()>),
    Hotkey(HotkeyEvent),
    ManualToggle {
        copy_only: bool,
    },
    OwnedToggle(String),
    OwnedCancel(String),
    OwnedClosed(String),
    OwnedDeliveryError {
        capture: OwnedCapture,
        history_id: String,
    },
    RefreshOwned,
    RefreshSpeechModels,
    SelectSpeechModel {
        choice: pennel_core::ModelChoice,
        reply: Sender<Result<(), String>>,
    },
    Delivered {
        preview_id: String,
        notice: Option<PillState>,
    },
    DeliveryRecovery(RecoveryEvent),
    NoSound {
        session: String,
        silent: bool,
    },
    Preview {
        update: DictationPreviewUpdate,
        owned: Option<OwnedCapture>,
    },
    Finished {
        seq: u64,
        outcome: Box<Result<DictationResult, Failure>>,
        released: Instant,
        is_command: bool,
        destination: ExternalDestination,
        preview_id: String,
        pending: DictationJob,
        owned: Option<OwnedCapture>,
    },
    TriggerChanged,
}

#[derive(Debug)]
pub enum Failure {
    Core(PennelError),
    AppChanged,
}

fn problem(title: &str, detail: Option<&str>) -> PillState {
    PillState::Problem {
        title: title.into(),
        detail: detail.map(str::to_owned),
    }
}

fn inserted_in(app: &str) -> PillState {
    insertion_notice(app, cfg!(target_os = "macos"))
}

fn insertion_notice(app: &str, has_display_name: bool) -> PillState {
    let app = app.trim();
    PillState::Inserted {
        app: (has_display_name && !app.is_empty()).then(|| app.to_owned()),
    }
}

#[cfg(test)]
fn notice_for_cleanup(cleanup: &CleanupStatus) -> Option<PillState> {
    notice_for_cleanup_destination(cleanup, false)
}

fn notice_for_cleanup_destination(cleanup: &CleanupStatus, available: bool) -> Option<PillState> {
    let title = match cleanup {
        CleanupStatus::Failed { message }
            if message.to_lowercase().contains("timed out")
                || message.to_lowercase().contains("timeout") =>
        {
            "Cleanup timed out"
        }
        CleanupStatus::Failed { .. } => "Cleanup failed",
        CleanupStatus::Rejected => "Cleanup could not preserve the wording and meaning",
        _ => return None,
    };
    let detail = match (cleanup, available) {
        (CleanupStatus::Rejected, true) => "Original text available",
        (CleanupStatus::Rejected, false) => "Original text pasted",
        (_, true) => "Original text available (see log)",
        (_, false) => "Original text pasted (see log)",
    };
    Some(problem(title, Some(detail)))
}

pub fn notice_for_error(error: &PennelError) -> Option<PillState> {
    let (title, kept) = match error {
        PennelError::NoSpeech => return None,
        PennelError::MissingApiKey { .. } => ("No API key configured", false),
        PennelError::ModelNotDownloaded => ("Model not downloaded", true),
        PennelError::Http { status, .. } if *status == 401 || *status == 403 => {
            ("API key rejected", true)
        }
        PennelError::Network { .. } => ("No connection", true),
        PennelError::UnsupportedProcessor => ("Processor too old for local models", true),
        PennelError::Http { .. } | PennelError::InvalidResponse { .. } => ("Failed", true),
        _ => ("Failed", false),
    };
    Some(problem(title, kept.then_some(KEPT_IN_HISTORY)))
}

/// Commands are not kept in history when they fail, so these notices don't promise a retry.
pub fn notice_for_command(failure: &Failure) -> Option<PillState> {
    let error = match failure {
        Failure::AppChanged => {
            return Some(problem("Another app is active — command not pasted", None));
        }
        Failure::Core(error) => error,
    };
    let title = match error {
        PennelError::NoSpeech => return None,
        PennelError::MissingApiKey { .. } => "No API key configured",
        PennelError::NotFound => "No text model configured",
        PennelError::Http { status, .. } if *status == 401 || *status == 403 => "API key rejected",
        PennelError::InvalidResponse { message } if message == "selection too long" => {
            "Selection is too long for a command"
        }
        PennelError::Network { .. } => "No connection",
        _ => "Command failed",
    };
    Some(problem(title, None))
}

fn describe(cleanup: &CleanupStatus) -> String {
    match cleanup {
        CleanupStatus::Applied => "applied".into(),
        CleanupStatus::Skipped => "skipped (fast mode)".into(),
        CleanupStatus::Rejected => "rejected".into(),
        CleanupStatus::Failed { .. } => "failed".into(),
    }
}

fn failure_category(failure: &Failure) -> String {
    match failure {
        Failure::AppChanged => "foreground_changed".into(),
        Failure::Core(PennelError::Http { status, .. }) => format!("provider_http_{status}"),
        Failure::Core(PennelError::Network { .. }) => "provider_network".into(),
        Failure::Core(PennelError::InvalidResponse { .. }) => "provider_response".into(),
        Failure::Core(PennelError::Storage { .. }) => "storage".into(),
        Failure::Core(PennelError::MissingApiKey { .. }) => "missing_key".into(),
        Failure::Core(PennelError::ModelNotDownloaded) => "model_not_downloaded".into(),
        Failure::Core(PennelError::NotFound) => "not_found".into(),
        Failure::Core(PennelError::Cancelled) => "Cancelled".into(),
        Failure::Core(PennelError::NoSpeech) => "no_speech".into(),
        Failure::Core(PennelError::NotSignedIn) => "not_signed_in".into(),
        Failure::Core(PennelError::Cloud { .. }) => "cloud".into(),
        Failure::Core(PennelError::VaultLocked) => "vault_locked".into(),
        Failure::Core(PennelError::UnsupportedProcessor) => "unsupported_processor".into(),
    }
}

struct CountedWork<T> {
    owner: T,
    counter: fn(&T) -> &AtomicUsize,
}

impl<T> CountedWork<T> {
    fn new(owner: T, counter: fn(&T) -> &AtomicUsize) -> Self {
        counter(&owner).fetch_add(1, Ordering::SeqCst);
        Self { owner, counter }
    }
}

impl<T> Drop for CountedWork<T> {
    fn drop(&mut self) {
        (self.counter)(&self.owner).fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct DictationJob {
    _pending: CountedWork<Arc<Shared>>,
    preview_priority: Arc<pennel_core::PennelEngine>,
}

impl DictationJob {
    fn new(shared: Arc<Shared>) -> Self {
        shared.engine.begin_meeting_preview_priority();
        Self {
            preview_priority: shared.engine.clone(),
            _pending: CountedWork::new(shared, |shared| &shared.dictation_jobs),
        }
    }
}

impl Drop for DictationJob {
    fn drop(&mut self) {
        self.preview_priority.end_meeting_preview_priority();
    }
}

struct CaptureActivity(Arc<Shared>);

impl CaptureActivity {
    fn begin(shared: Arc<Shared>) -> Option<Self> {
        if shared
            .dictation_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        if shared.updating.load(Ordering::SeqCst)
            || shared.quitting.load(Ordering::SeqCst)
            || crate::shortcut_capture::capturing()
        {
            shared.dictation_active.store(false, Ordering::SeqCst);
            return None;
        }
        shared.engine.begin_meeting_preview_priority();
        Some(Self(shared))
    }
}

impl Drop for CaptureActivity {
    fn drop(&mut self) {
        self.0.dictation_active.store(false, Ordering::SeqCst);
        self.0.engine.end_meeting_preview_priority();
    }
}

#[derive(Debug)]
struct PasteText<T = ExternalTarget> {
    preview_id: String,
    history_id: String,
    text: String,
    released: Instant,
    destination: ExternalDestination<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryEvent {
    pub id: String,
    pub unsaved: bool,
    pub reason: String,
}

#[derive(Debug)]
struct UndeliveredText {
    text: String,
    recovery: RecoveryEvent,
}

fn prepare_delivery<T>(
    result: DictationResult,
    preview_id: &str,
    released: Instant,
    destination: ExternalDestination<T>,
    cancel: impl FnOnce(&ExternalDestination<T>),
) -> Result<PasteText<T>, UndeliveredText> {
    if !result.history_saved {
        cancel(&destination);
        return Err(UndeliveredText {
            text: result.text,
            recovery: RecoveryEvent {
                id: result.history_id,
                unsaved: true,
                reason: "Saving failed. Your text is temporarily available in History.".into(),
            },
        });
    }
    Ok(PasteText {
        preview_id: preview_id.into(),
        history_id: result.history_id,
        text: result.text,
        released,
        destination,
    })
}

struct Delivery {
    destination: ExternalDestination,
    preview_id: String,
    owned: Option<OwnedCapture>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExternalDestination<T = ExternalTarget> {
    CopyOnly,
    Captured(T),
    Unavailable(String),
}

fn capture_destination<T>(
    copy_only: bool,
    capture: impl FnOnce() -> Result<T, String>,
) -> ExternalDestination<T> {
    if copy_only {
        ExternalDestination::CopyOnly
    } else {
        match capture() {
            Ok(target) => ExternalDestination::Captured(target),
            Err(error) => ExternalDestination::Unavailable(error),
        }
    }
}

fn cancel_destination(destination: &ExternalDestination) {
    if let ExternalDestination::Captured(target) = destination {
        target.cancel();
    }
}

#[derive(Default)]
struct PreviewCursor {
    session_id: Option<String>,
    owner_id: Option<String>,
    revision: u64,
}

impl PreviewCursor {
    fn begin(&mut self, session_id: String, owner_id: Option<String>) {
        self.session_id = Some(session_id);
        self.owner_id = owner_id;
        self.revision = 0;
    }

    fn accept(&mut self, update: &DictationPreviewUpdate, owner_valid: bool) -> bool {
        if !owner_valid
            || self.session_id.as_deref() != Some(update.session_id.as_str())
            || update.revision <= self.revision
        {
            return false;
        }
        self.revision = update.revision;
        true
    }

    fn clear_session(&mut self, id: &str) -> bool {
        if self.session_id.as_deref() != Some(id) {
            return false;
        }
        *self = Self::default();
        true
    }

    fn clear_owner(&mut self, id: &str) -> Option<String> {
        if self.owner_id.as_deref() != Some(id) {
            return None;
        }
        let session_id = self.session_id.take();
        *self = Self::default();
        session_id
    }
}

struct PendingOwnedSession {
    owner_id: String,
    session: Arc<LiveDictationSession>,
}

#[derive(Default)]
struct PendingOwnedSessions(HashMap<String, PendingOwnedSession>);

impl PendingOwnedSessions {
    fn track(&mut self, owner_id: &str, session: Arc<LiveDictationSession>) {
        self.0.insert(
            session.id(),
            PendingOwnedSession {
                owner_id: owner_id.into(),
                session,
            },
        );
    }

    fn cancel_owner(&mut self, owner_id: &str) {
        self.0.retain(|_, pending| {
            if pending.owner_id == owner_id {
                pending.session.cancel();
                false
            } else {
                true
            }
        });
    }

    fn complete(&mut self, session_id: &str) {
        self.0.remove(session_id);
    }
}

impl Drop for PendingOwnedSessions {
    fn drop(&mut self) {
        for pending in self.0.values() {
            pending.session.cancel();
        }
    }
}

struct StoppedContext {
    command_pid: Option<u32>,
    app_bundle_id: Option<String>,
}

impl StoppedContext {
    fn new(
        app: Option<&ForegroundApp>,
        owned_app_id: Option<&str>,
        captured_app_id: Option<&str>,
    ) -> Self {
        Self {
            command_pid: if owned_app_id.is_none() {
                app.map(|app| app.pid)
            } else {
                None
            },
            app_bundle_id: owned_app_id.or(captured_app_id).map(str::to_owned),
        }
    }
}

struct PreviewObserver {
    sender: Sender<Msg>,
    owned: Option<OwnedCapture>,
}

impl DictationPreviewObserver for PreviewObserver {
    fn changed(&self, update: DictationPreviewUpdate) {
        let _ = self.sender.send(Msg::Preview {
            update,
            owned: self.owned.clone(),
        });
    }
}

enum CompletedDelivery {
    External(PasteText),
    Owned {
        preview_id: String,
        capture: OwnedCapture,
        history_id: String,
        text: String,
    },
}

fn target_matches(target_pid: Option<u32>, current_pid: Option<u32>) -> bool {
    target_pid.is_none() || target_pid == current_pid
}

fn command_field_matches(
    target_pid: Option<u32>,
    current_pid: Option<u32>,
    captured_field_current: bool,
) -> bool {
    captured_field_current && target_matches(target_pid, current_pid)
}

fn paste_pending(shared: &Arc<Shared>) -> CountedWork<Arc<Shared>> {
    CountedWork::new(shared.clone(), |shared| &shared.paste_jobs)
}

fn copy_only_delivery(requested: bool) -> bool {
    #[cfg(target_os = "linux")]
    return requested || crate::linux_input::is_wayland();
    #[cfg(not(target_os = "linux"))]
    requested
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailure {
    Missing,
    Changed,
    Permission,
    Other,
    Copy,
}

fn recovery_for_failure(history_id: &str, failure: DeliveryFailure) -> RecoveryEvent {
    let notice = match failure {
        DeliveryFailure::Missing => "No active text destination — your text is in History.",
        DeliveryFailure::Changed => {
            "Not pasted — the text destination changed. Your text is in History."
        }
        DeliveryFailure::Permission => "Permission to paste is missing. Your text is in History.",
        DeliveryFailure::Other => "Not pasted. Your text is in History.",
        DeliveryFailure::Copy => "Copying failed. Your text is in History.",
    };
    RecoveryEvent {
        id: history_id.into(),
        unsaved: false,
        reason: notice.into(),
    }
}

fn problem_for_failure(failure: DeliveryFailure) -> PillState {
    let title = match failure {
        DeliveryFailure::Missing => "No active text destination",
        DeliveryFailure::Changed => "Not pasted — the text destination changed",
        DeliveryFailure::Permission => "Permission to paste is missing",
        DeliveryFailure::Other => "Not pasted",
        DeliveryFailure::Copy => "Copying failed",
    };
    problem(title, Some(KEPT_IN_HISTORY))
}

struct DeliveryCompletion {
    notices: Sender<Msg>,
    preview_id: String,
    notice: Option<PillState>,
}

impl DeliveryCompletion {
    fn new(notices: Sender<Msg>, preview_id: String) -> Self {
        Self {
            notices,
            preview_id,
            notice: None,
        }
    }
}

// Only the paste worker creates this guard, after receiving an enqueued job.
impl Drop for DeliveryCompletion {
    fn drop(&mut self) {
        let _ = self.notices.send(Msg::Delivered {
            preview_id: std::mem::take(&mut self.preview_id),
            notice: self.notice.take(),
        });
    }
}

fn enqueue_delivery<T>(jobs: &Sender<T>, job: T, status: &mut DictationStatus) -> Result<(), T> {
    jobs.send(job).map_err(|error| error.0)?;
    status.delivery_started();
    Ok(())
}

fn deliver_owned<T>(
    _delivery: DeliveryCompletion,
    allowed: bool,
    target: Option<&T>,
    paste: impl FnOnce(&T) -> Result<(), String>,
    failed: impl FnOnce(),
) {
    if !allowed {
        return;
    }
    let Some(target) = target else { return };
    if paste(target).is_err() {
        failed();
    }
}

fn unavailable_destination(
    delivery: &mut DeliveryCompletion,
    history_id: &str,
    failure: DeliveryFailure,
) {
    let _ = delivery
        .notices
        .send(Msg::DeliveryRecovery(recovery_for_failure(
            history_id, failure,
        )));
    delivery.notice = Some(problem_for_failure(failure));
}

fn finish_copy(delivery: &mut DeliveryCompletion, history_id: &str, result: Result<(), String>) {
    match result {
        Ok(()) => {
            delivery.notice = Some(PillState::Copied {
                paste_key: PASTE_KEY.into(),
            })
        }
        Err(_) => {
            LOG.error("clipboard write failed");
            unavailable_destination(delivery, history_id, DeliveryFailure::Copy);
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn paste_transaction<C: crate::clipboard::Clipboard>(
    clipboard: &mut C,
    text: &str,
    paste: impl FnOnce() -> Result<(), String>,
    wait: impl FnOnce(),
) -> Result<(), String> {
    let token = crate::clipboard::try_begin(clipboard, text)?;
    let result = paste();
    wait();
    crate::clipboard::end(clipboard, token);
    result
}

#[cfg(not(target_os = "macos"))]
fn copy_persistent(text: &str, clipboard: &mut Option<arboard::Clipboard>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::linux_input::is_wayland() {
        return crate::linux_input::copy_wayland(text);
    }
    if clipboard.is_none() {
        *clipboard = Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
    }
    clipboard
        .as_mut()
        .expect("initialized clipboard")
        .set_text(text)
        .map_err(|error| error.to_string())
}

enum PasteJob {
    Paste {
        result: PasteText,
        pending: CountedWork<Arc<Shared>>,
    },
    ReadSelection {
        reply: Sender<Option<String>>,
        target_pid: Option<u32>,
        target: ExternalTarget,
        pending: CountedWork<Arc<Shared>>,
    },
    Owned {
        preview_id: String,
        capture: OwnedCapture,
        history_id: String,
        text: String,
        pending: CountedWork<Arc<Shared>>,
    },
}

/// Owns the clipboard; pastes and selection reads run strictly one after another so a command
/// never copies Pennel's own text that is still waiting to be restored.
#[cfg(not(target_os = "macos"))]
type LateCopyWork = (
    LateCopyWatch<<SystemClipboard as Clipboard>::Snapshot>,
    CountedWork<Arc<Shared>>,
);

#[cfg(not(target_os = "macos"))]
fn paste_worker(jobs: Receiver<PasteJob>, shared: Arc<Shared>) {
    let mut persistent_clipboard = None;
    let mut clipboard = SystemClipboard::new();
    let mut late: Option<LateCopyWork> = None;
    let mut nonce = 0u64;
    loop {
        let job = if late.is_some() {
            match jobs.recv_timeout(Duration::from_millis(50)) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match jobs.recv() {
                Ok(job) => Some(job),
                Err(_) => return,
            }
        };
        if let Some((watch, _)) = &late
            && (watch.check(&mut clipboard) || watch.expired(Instant::now()))
        {
            late = None;
        }
        match job {
            None => {}
            Some(PasteJob::Paste { result, pending }) => {
                let _pending = pending;
                let mut delivery =
                    DeliveryCompletion::new(shared.dictation.clone(), result.preview_id.clone());
                late = None;
                if matches!(result.destination, ExternalDestination::CopyOnly) {
                    let copied = copy_persistent(&result.text, &mut persistent_clipboard);
                    finish_copy(&mut delivery, &result.history_id, copied);
                    continue;
                }
                let ExternalDestination::Captured(target) = &result.destination else {
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Missing,
                    );
                    continue;
                };
                keys::wait_for_release();
                if !external_paste::availability().0 {
                    target.cancel();
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Permission,
                    );
                    continue;
                }
                if external_paste::prepare(target).is_err()
                    || !external_paste::still_selected(target)
                {
                    target.cancel();
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Changed,
                    );
                    continue;
                }
                let pasted = paste_transaction(
                    &mut clipboard,
                    &result.text,
                    || {
                        if external_paste::still_selected(target) {
                            keys::send(Shortcut::Paste)
                        } else {
                            Err("The text destination changed.".into())
                        }
                    },
                    || std::thread::sleep(clipboard::RESTORE_DELAY),
                )
                .is_ok();
                target.cancel();
                if pasted {
                    LOG.info(format!(
                        "release→paste {} ms",
                        result.released.elapsed().as_millis()
                    ));
                    delivery.notice = Some(inserted_in(&target.name));
                } else {
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Other,
                    );
                }
            }
            Some(PasteJob::ReadSelection {
                reply,
                target_pid,
                target,
                pending,
            }) => {
                #[cfg(target_os = "linux")]
                if crate::linux_input::is_wayland() {
                    let _ = reply.send(None);
                    continue;
                }
                keys::wait_for_release();
                if !command_field_matches(
                    target_pid,
                    foreground::current().map(|app| app.pid),
                    external_paste::is_current(&target),
                ) {
                    let _ = reply.send(None);
                    continue;
                }
                nonce += 1;
                let read = clipboard::read_selection(
                    &mut clipboard,
                    || {
                        if let Err(e) = keys::send(Shortcut::Copy) {
                            LOG.error(format!("copy failed: {e}"));
                        }
                    },
                    nonce,
                );
                late = read.late_watch.map(|watch| (watch, pending));
                let _ = reply.send(read.text);
            }
            Some(PasteJob::Owned {
                preview_id,
                capture,
                history_id,
                text,
                pending,
            }) => {
                let _pending = pending;
                late = None;
                deliver_owned(
                    DeliveryCompletion::new(shared.dictation.clone(), preview_id),
                    owned_dictation::external_allowed(&shared, &capture),
                    capture.external.as_ref(),
                    |target| {
                        crate::external_paste::prepare(target).and_then(|()| {
                            keys::wait_for_release();
                            if !owned_dictation::external_allowed(&shared, &capture)
                                || !crate::external_paste::still_selected(target)
                            {
                                return Err(
                                    "External paste was cancelled. Your text is in Pennel.".into(),
                                );
                            }
                            paste_transaction(
                                &mut clipboard,
                                &text,
                                || {
                                    if owned_dictation::external_allowed(&shared, &capture)
                                        && crate::external_paste::still_selected(target)
                                    {
                                        keys::send(Shortcut::Paste)
                                    } else {
                                        Err("The destination app changed. Your text is in Pennel."
                                            .into())
                                    }
                                },
                                || std::thread::sleep(clipboard::RESTORE_DELAY),
                            )
                        })
                    },
                    || {
                        let _ = shared.dictation.send(Msg::OwnedDeliveryError {
                            capture: capture.clone(),
                            history_id,
                        });
                    },
                );
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn paste_worker(jobs: Receiver<PasteJob>, shared: Arc<Shared>) {
    for job in jobs {
        keys::wait_for_release();
        match job {
            PasteJob::Paste { result, pending } => {
                let _pending = pending;
                let mut delivery =
                    DeliveryCompletion::new(shared.dictation.clone(), result.preview_id.clone());
                if matches!(result.destination, ExternalDestination::CopyOnly) {
                    finish_copy(
                        &mut delivery,
                        &result.history_id,
                        crate::macos::copy_text(&result.text, None),
                    );
                    continue;
                }
                let ExternalDestination::Captured(target) = &result.destination else {
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Missing,
                    );
                    continue;
                };
                if !external_paste::availability().0 {
                    target.cancel();
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Permission,
                    );
                    continue;
                }
                if !external_paste::is_current(target) {
                    target.cancel();
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Changed,
                    );
                    continue;
                }
                let pasted = external_paste::paste(&result.text, target).is_ok();
                target.cancel();
                if pasted {
                    LOG.info(format!(
                        "release→paste {} ms",
                        result.released.elapsed().as_millis()
                    ));
                    delivery.notice = Some(inserted_in(&target.name));
                } else {
                    unavailable_destination(
                        &mut delivery,
                        &result.history_id,
                        DeliveryFailure::Other,
                    );
                }
            }
            PasteJob::ReadSelection {
                reply,
                target_pid,
                target,
                pending,
            } => {
                let _pending = pending;
                if !command_field_matches(
                    target_pid,
                    foreground::current().map(|app| app.pid),
                    external_paste::is_current(&target),
                ) {
                    let _ = reply.send(None);
                    continue;
                }
                let selection = crate::macos::read_selection().ok().flatten();
                let _ = reply.send(
                    external_paste::is_current(&target)
                        .then_some(selection)
                        .flatten(),
                );
            }
            PasteJob::Owned {
                preview_id,
                capture,
                history_id,
                text,
                pending,
            } => {
                let _pending = pending;
                deliver_owned(
                    DeliveryCompletion::new(shared.dictation.clone(), preview_id),
                    owned_dictation::external_allowed(&shared, &capture),
                    capture.external.as_ref(),
                    |target| crate::external_paste::paste(&text, target),
                    || {
                        let _ = shared.dictation.send(Msg::OwnedDeliveryError {
                            capture: capture.clone(),
                            history_id,
                        });
                    },
                );
            }
        }
    }
}

struct Controller {
    app: AppHandle,
    shared: Arc<Shared>,
    pill: Arc<Pill>,
    tx: Sender<Msg>,
    paster: Sender<PasteJob>,
    gesture: HotkeyGesture,
    command: CommandIntent,
    status: DictationStatus,
    queue: PasteQueue<CompletedDelivery>,
    recorder: Option<Recorder>,
    recording_session: Option<Arc<LiveDictationSession>>,
    pending_owned: PendingOwnedSessions,
    preview_cursor: PreviewCursor,
    #[cfg(target_os = "macos")]
    apple_preview: Option<Arc<Mutex<Option<crate::macos::AppleSpeechPreview>>>>,
    capture_activity: Option<CaptureActivity>,
    recording_copy_only: bool,
    requested_owned: Option<String>,
    recording_owned: Option<OwnedCapture>,
    pending_shutdown: Option<Sender<()>>,
}

pub fn spawn(app: AppHandle, shared: Arc<Shared>, rx: Receiver<Msg>, tx: Sender<Msg>) {
    let (paster, jobs) = mpsc::channel();
    let paste_shared = shared.clone();
    std::thread::Builder::new()
        .name("pennel-paste".into())
        .spawn(move || paste_worker(jobs, paste_shared))
        .expect("paste thread");
    let pill = Arc::new(Pill::new(app.clone()));
    let mut controller = Controller {
        app,
        shared,
        pill,
        tx,
        paster,
        gesture: HotkeyGesture::default(),
        command: CommandIntent::default(),
        status: DictationStatus::default(),
        queue: PasteQueue::default(),
        recorder: None,
        recording_session: None,
        pending_owned: PendingOwnedSessions::default(),
        preview_cursor: PreviewCursor::default(),
        #[cfg(target_os = "macos")]
        apple_preview: None,
        capture_activity: None,
        recording_copy_only: false,
        requested_owned: None,
        recording_owned: None,
        pending_shutdown: None,
    };
    let refresh_events: Vec<_> = ["settings-changed", "download-done", "sync-changed"]
        .into_iter()
        .map(|event| {
            let sender = controller.tx.clone();
            controller.app.listen(event, move |_| {
                let _ = sender.send(Msg::RefreshSpeechModels);
            })
        })
        .collect();
    std::thread::Builder::new()
        .name("pennel-dictation".into())
        .spawn(move || {
            controller.refresh_speech_models();
            controller.run(rx);
            for event in refresh_events {
                controller.app.unlisten(event);
            }
        })
        .expect("dictation thread");
}

impl Controller {
    fn run(&mut self, rx: Receiver<Msg>) {
        loop {
            let wait = self
                .next_wake()
                .map(|at| at.saturating_duration_since(Instant::now()));
            let msg = match wait {
                Some(wait) => match rx.recv_timeout(wait) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => return,
                },
            };
            match msg {
                Some(Msg::PrepareShutdown(reply)) => {
                    if self.recorder.is_some() {
                        self.submit();
                    }
                    self.gesture = HotkeyGesture::default();
                    let _ = reply.send(());
                }
                Some(Msg::Shutdown(reply)) => {
                    if self.recorder.is_some() {
                        self.submit();
                    }
                    self.pending_shutdown = Some(reply);
                }
                Some(Msg::Hotkey(event)) => self.hotkey(event),
                Some(Msg::ManualToggle { copy_only }) => {
                    if self.recorder.is_none() {
                        self.recording_copy_only = copy_only_delivery(copy_only);
                        self.requested_owned = owned_dictation::focused_session(&self.app);
                    }
                    let actions = self.gesture.manual_toggle(hotkey::now());
                    self.perform(actions);
                }
                Some(Msg::OwnedToggle(id)) => {
                    if self.recorder.is_some()
                        && self
                            .recording_owned
                            .as_ref()
                            .is_none_or(|owned| owned.id != id)
                    {
                        owned_dictation::failed(
                            &self.app,
                            &id,
                            "Finish your current dictation first".into(),
                        );
                    } else {
                        if self.recorder.is_none() {
                            self.requested_owned = Some(id);
                            self.recording_copy_only = false;
                        }
                        let actions = self.gesture.manual_toggle(hotkey::now());
                        self.perform(actions);
                    }
                }
                Some(Msg::OwnedCancel(id) | Msg::OwnedClosed(id)) => {
                    self.pending_owned.cancel_owner(&id);
                    if self
                        .recording_owned
                        .as_ref()
                        .is_some_and(|owned| owned.id == id)
                    {
                        self.cancel();
                        self.gesture = HotkeyGesture::default();
                    }
                    self.clear_owned_preview(&id);
                }
                Some(Msg::Delivered { preview_id, notice }) => {
                    self.status
                        .delivery_finished(&preview_id, notice, Instant::now());
                    self.render();
                }
                Some(Msg::DeliveryRecovery(recovery)) => {
                    self.emit_recovery(recovery);
                }
                Some(Msg::NoSound { session, silent }) => {
                    if self
                        .recording_session
                        .as_ref()
                        .is_some_and(|recording| recording.id() == session)
                    {
                        self.status.set_no_sound(silent);
                        self.render();
                    }
                }
                Some(Msg::Preview { update, owned }) => {
                    let owner_valid = owned.as_ref().is_none_or(OwnedCapture::valid);
                    if self.preview_cursor.accept(&update, owner_valid) {
                        self.pill.update_preview(update);
                    } else if !owner_valid {
                        self.clear_preview(&update.session_id);
                    }
                }
                Some(Msg::OwnedDeliveryError {
                    capture,
                    history_id,
                }) => self.owned_delivery_failed(&capture, &history_id),
                Some(Msg::RefreshOwned) => {
                    owned_dictation::refresh_idle_pads(&self.app, &self.shared)
                }
                Some(Msg::Finished {
                    seq,
                    outcome,
                    released,
                    is_command,
                    destination,
                    preview_id,
                    pending,
                    owned,
                }) => self.finish(
                    seq,
                    *outcome,
                    released,
                    is_command,
                    Delivery {
                        destination,
                        preview_id,
                        owned,
                    },
                    pending,
                ),
                Some(Msg::SelectSpeechModel { choice, reply }) => {
                    let result = self.select_speech_model(Some(choice));
                    let _ = reply.send(result);
                }
                Some(Msg::RefreshSpeechModels) => self.refresh_speech_models(),
                Some(Msg::TriggerChanged) if !self.gesture.is_manual() => {
                    if self.recorder.is_some() {
                        self.cancel();
                    }
                    self.gesture = HotkeyGesture::default();
                }
                Some(Msg::TriggerChanged) | None => {}
            }
            if self.recorder.is_some() {
                let actions = self.gesture.tick(hotkey::now());
                self.perform(actions);
            }
            if self.status.expire_notice(Instant::now()) {
                self.render();
            }
            if self.pending_shutdown.is_some()
                && self.shared.dictation_jobs.load(Ordering::SeqCst) == 0
                && self.shared.paste_jobs.load(Ordering::SeqCst) == 0
            {
                if let Some(reply) = self.pending_shutdown.take() {
                    let _ = reply.send(());
                }
                return;
            }
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        let tick = if self.pending_shutdown.is_some() {
            Some(Instant::now() + Duration::from_millis(20))
        } else {
            self.recorder.is_some().then(|| Instant::now() + TICK)
        };
        match (tick, self.status.notice_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn hotkey(&mut self, event: HotkeyEvent) {
        if crate::shortcut_capture::capturing() {
            return;
        }
        match event {
            HotkeyEvent::Action(3) if self.recorder.is_some() => {
                let actions = self.gesture.cancel();
                self.perform(actions);
            }
            HotkeyEvent::Action(4)
                if self.recorder.is_some() && self.shared.settings().pill_model_switch_enabled =>
            {
                if self.select_speech_model(None).is_err() {
                    self.refresh_speech_models();
                }
            }
            HotkeyEvent::Action(1) => {
                crate::call_detection::toggle_recording(self.app.clone(), self.shared.clone());
            }
            HotkeyEvent::Action(2) => {
                crate::open_default(&self.app);
            }
            HotkeyEvent::Action(_) => {}
            HotkeyEvent::Edge(edge, time) => {
                let actions = self.gesture.handle(edge, time);
                if actions.contains(&GestureAction::Start) {
                    self.recording_copy_only = copy_only_delivery(false);
                    self.requested_owned = owned_dictation::focused_session(&self.app);
                }
                self.perform(actions);
                if self.recorder.is_some() {
                    crate::record_shortcut::refresh(&self.app, &self.shared);
                }
            }
            HotkeyEvent::Interrupt => {
                let actions = self.gesture.interrupt();
                self.perform(actions);
            }
            HotkeyEvent::Shift(down) => {
                if self.recorder.is_some() {
                    crate::record_shortcut::refresh(&self.app, &self.shared);
                }
                let was = self.command.is_command();
                self.command.shift(
                    down,
                    self.recorder.is_some()
                        && hotkey::trigger_held()
                        && !self.gesture.is_manual()
                        && !self.recording_copy_only
                        && self.recording_owned.is_none(),
                );
                if self.command.is_command() && !was {
                    self.status.mark_command();
                    self.render();
                }
            }
        }
    }

    fn refresh_speech_models(&self) {
        if let Some(session) = &self.recording_session {
            self.pill.refresh_model(session, false);
        }
    }

    fn select_speech_model(
        &mut self,
        choice: Option<pennel_core::ModelChoice>,
    ) -> Result<(), String> {
        let keys = self.shared.keys();
        if let Some(session) = self
            .recording_session
            .as_ref()
            .filter(|_| self.recorder.is_some())
        {
            match choice {
                Some(choice) => session.select_speech_model(choice, keys),
                None => session.next_speech_model(keys),
            }
            .map_err(|_| {
                "This speech model is unavailable. Your current dictation is unchanged.".to_owned()
            })?;
            #[cfg(target_os = "macos")]
            if let Some(preview) = &self.apple_preview {
                let mut service = preview.lock().unwrap();
                if let Some(previous) = service.take() {
                    previous.stop();
                }
                if session.mode() == DictationPreviewMode::System {
                    let generation = session.preview_generation();
                    let session = session.clone();
                    *service = crate::macos::AppleSpeechPreview::start(
                        session.language().as_deref(),
                        move |text| {
                            session.update_system_preview_for_generation(text, generation);
                        },
                    )
                    .ok();
                }
            }
            self.pill.refresh_model(session, true);
        } else {
            match choice {
                Some(choice) => self.shared.engine.select_dictation_model(choice, keys),
                None => self.shared.engine.next_dictation_model(keys),
            }
            .map_err(|_| {
                "This speech model is unavailable. Set up speech recognition first.".to_owned()
            })?;
        }
        let _ = self.app.emit("settings-changed", ());
        self.refresh_speech_models();
        Ok(())
    }

    fn perform(&mut self, actions: Vec<GestureAction>) {
        for action in actions {
            match action {
                GestureAction::Start => self.begin(),
                GestureAction::Cancel => self.cancel(),
                GestureAction::Submit => self.submit(),
            }
        }
    }

    fn begin(&mut self) {
        if self.recorder.is_some()
            || self
                .app
                .state::<crate::ExitState>()
                .requested
                .load(Ordering::SeqCst)
        {
            return;
        }
        let route = owned_dictation::CaptureRoute::resolve(
            owned_dictation::own_app_foreground(&self.app),
            self.requested_owned.take(),
        );
        if let Err(error) = self.shared.engine.ensure_history_capacity() {
            self.gesture = HotkeyGesture::default();
            return self.notify(&error.to_string());
        }
        let allows_command = route.allows_command();
        if route == owned_dictation::CaptureRoute::Unavailable {
            self.gesture = HotkeyGesture::default();
            self.command.reset();
            return self.notify("Select a text field in another app first");
        }
        if let owned_dictation::CaptureRoute::Owned(id) = route {
            match owned_dictation::begin_capture(&self.app, &self.shared, &id) {
                Ok(capture) => self.recording_owned = Some(capture),
                Err(error) => {
                    self.gesture = HotkeyGesture::default();
                    owned_dictation::failed(&self.app, &id, error);
                    return;
                }
            }
        } else {
            let readiness = owned_dictation::readiness(&self.shared, "dictation");
            if !readiness.can_record {
                self.gesture = HotkeyGesture::default();
                return self.notify(
                    readiness
                        .reason
                        .as_deref()
                        .unwrap_or("Set up speech recognition first"),
                );
            }
        }
        let Some(activity) = CaptureActivity::begin(self.shared.clone()) else {
            self.gesture = HotkeyGesture::default();
            if let Some(capture) = self.recording_owned.take() {
                owned_dictation::failed(
                    &self.app,
                    &capture.id,
                    "Pennel is still working. Try again afterwards.".into(),
                );
            }
            return self.notify("Pennel is updating — try again afterwards");
        };
        let session = self.shared.engine.clone().begin_dictation_session(
            Arc::new(PreviewObserver {
                sender: self.tx.clone(),
                owned: self.recording_owned.clone(),
            }),
            self.recording_owned
                .as_ref()
                .is_some_and(|capture| capture.kind.onboarding_practice()),
        );
        let preview_id = session.id();
        self.recording_session = Some(session.clone());
        self.preview_cursor.begin(
            preview_id.clone(),
            self.recording_owned
                .as_ref()
                .map(|capture| capture.id.clone()),
        );
        crate::record_shortcut::set_listening(&self.app, &self.shared, true);
        self.status
            .recording_started(&preview_id, self.gesture.is_hands_free());
        self.command.reset();
        self.command.shift(
            hotkey::shift_held(),
            allows_command
                && !self.gesture.is_manual()
                && !self.recording_copy_only
                && self.recording_owned.is_none(),
        );
        if self.command.is_command() {
            self.status.mark_command();
        }
        self.render();
        self.pill.begin_preview(preview_id.clone(), &session);
        if let Some(capture) = &self.recording_owned {
            owned_dictation::verification(&self.app, capture, &session);
        }

        #[cfg(target_os = "macos")]
        let apple_preview = {
            let service = if session.mode() == DictationPreviewMode::System {
                let generation = session.preview_generation();
                let session = session.clone();
                crate::macos::AppleSpeechPreview::start(
                    session.language().as_deref(),
                    move |text| {
                        session.update_system_preview_for_generation(text, generation);
                    },
                )
                .ok()
            } else {
                None
            };
            let preview = Arc::new(Mutex::new(service));
            self.apple_preview = Some(preview.clone());
            preview
        };
        let pill = self.pill.clone();
        let sound = self.tx.clone();
        let mut watch = NoSoundWatch::default();
        match Recorder::start(
            move |level| {
                pill.level(level);
                if let Some(silent) = watch.level(level, Instant::now()) {
                    let _ = sound.send(Msg::NoSound {
                        session: preview_id.clone(),
                        silent,
                    });
                }
            },
            move |samples| {
                session.feed_audio(samples.to_vec());
                #[cfg(target_os = "macos")]
                if let Some(preview) = apple_preview.lock().unwrap().as_ref() {
                    preview.feed(samples);
                }
            },
        ) {
            Ok(recorder) => self.recorder = Some(recorder),
            Err(CaptureError::NoDevice) => {
                crate::record_shortcut::set_listening(&self.app, &self.shared, false);
                self.discard_recording_preview();
                self.status.recording_cancelled();
                self.gesture = HotkeyGesture::default();
                if let Some(capture) = self.recording_owned.take() {
                    owned_dictation::failed(&self.app, &capture.id, "No microphone found".into());
                }
                return self.notify("No microphone found");
            }
            Err(e) => {
                self.discard_recording_preview();
                self.status.recording_cancelled();
                crate::record_shortcut::set_listening(&self.app, &self.shared, false);
                LOG.error(format!("recording start failed: {e}"));
                self.gesture = HotkeyGesture::default();
                if let Some(capture) = self.recording_owned.take() {
                    owned_dictation::failed(
                        &self.app,
                        &capture.id,
                        "Recording failed. Check microphone access.".into(),
                    );
                }
                return self.notify("Recording failed");
            }
        }
        self.capture_activity = Some(activity);
        self.shared.set_meeting_dictating(true);
        self.refresh_speech_models();
    }

    fn stop_recording(&mut self) -> Vec<f32> {
        crate::record_shortcut::set_listening(&self.app, &self.shared, false);
        let samples = self.recorder.take().map(Recorder::stop).unwrap_or_default();
        #[cfg(target_os = "macos")]
        if let Some(preview) = self.apple_preview.take()
            && let Some(preview) = preview.lock().unwrap().take()
        {
            preview.stop();
        }
        if let Some(session) = &self.recording_session {
            session.stop_preview();
        }
        self.shared.set_meeting_dictating(false);
        self.capture_activity.take();
        samples
    }

    fn cancel(&mut self) {
        self.stop_recording();
        self.discard_recording_preview();
        if let Some(capture) = self.recording_owned.take() {
            owned_dictation::cancelled(&self.app, &capture);
        }
        self.requested_owned = None;
        self.command.reset();
        self.status.recording_cancelled();
        self.render();
    }

    fn discard_recording_preview(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(preview) = self.apple_preview.take()
            && let Some(preview) = preview.lock().unwrap().take()
        {
            preview.stop();
        }
        if let Some(session) = self.recording_session.take() {
            session.cancel();
            self.clear_preview(&session.id());
        }
    }

    fn clear_preview(&mut self, id: &str) {
        if self.preview_cursor.clear_session(id) {
            self.pill.clear_preview(Some(id));
        }
    }

    fn clear_owned_preview(&mut self, owner_id: &str) {
        if let Some(id) = self.preview_cursor.clear_owner(owner_id) {
            self.pill.clear_preview(Some(&id));
        }
    }

    fn submit(&mut self) {
        if self.recorder.is_none() {
            return;
        }
        let released = Instant::now();
        // Transfer ownership before stopping capture, so the updater never sees an idle gap.
        let pending = DictationJob::new(self.shared.clone());
        let owned = self.recording_owned.take();
        let stopping_app = foreground::current();
        let destination = if owned.is_some() {
            ExternalDestination::CopyOnly
        } else {
            capture_destination(self.recording_copy_only, || {
                external_paste::capture_current(&self.app)
            })
        };
        let samples = self.stop_recording();
        if samples.is_empty() {
            cancel_destination(&destination);
            self.discard_recording_preview();
            if let Some(capture) = &owned {
                owned_dictation::failed(
                    &self.app,
                    &capture.id,
                    "No speech recorded. Try again.".into(),
                );
            }
            self.status.recording_cancelled();
            return self.render();
        }
        if audio::is_digital_silence(&samples) {
            cancel_destination(&destination);
            self.discard_recording_preview();
            LOG.error("microphone produced only silence (exact zeros)");
            self.command.reset();
            self.status.recording_cancelled();
            if let Some(capture) = &owned {
                owned_dictation::failed(
                    &self.app,
                    &capture.id,
                    "Your microphone is not producing audio. Check your input device.".into(),
                );
            }
            return self
                .notify("Your microphone is not producing audio — choose another input device");
        }
        let session = self
            .recording_session
            .take()
            .expect("a recording owns its recognition session");
        if let Some(capture) = &owned {
            self.pending_owned.track(&capture.id, session.clone());
            if !capture.valid() {
                self.pending_owned.cancel_owner(&capture.id);
            }
        }
        let preview_id = session.id();
        let context = StoppedContext::new(
            stopping_app.as_ref(),
            owned
                .as_ref()
                .map(|_| self.app.config().identifier.as_str()),
            match &destination {
                ExternalDestination::Captured(target) => Some(target.app_id.as_str()),
                _ => None,
            },
        );
        let command_pid = context.command_pid;
        let seq = self.queue.reserve();
        let request = DictationRequest {
            samples,
            sample_rate: TARGET_RATE,
            app_bundle_id: context.app_bundle_id,
            keys: self.shared.keys(),
        };
        let copy_only = self.recording_copy_only;
        let is_command = self.command.is_command() && !copy_only && owned.is_none();
        if let Some(capture) = &owned {
            owned_dictation::phase(&self.app, capture, SessionPhase::Working);
        }
        self.command.reset();
        self.status.recording_submitted();
        self.render();
        let tx = self.tx.clone();
        if is_command {
            let pid = command_pid;
            let paster = self.paster.clone();
            let shared = self.shared.clone();
            std::thread::spawn(move || {
                let still_there = || {
                    command_field_matches(
                        pid,
                        foreground::current().map(|app| app.pid),
                        matches!(&destination, ExternalDestination::Captured(target) if external_paste::is_current(target)),
                    )
                };
                let outcome = if !still_there() {
                    Err(Failure::AppChanged)
                } else {
                    let ExternalDestination::Captured(target) = &destination else {
                        unreachable!("the current command field has a captured target")
                    };
                    let (reply, selection) = mpsc::channel();
                    let selection = paster
                        .send(PasteJob::ReadSelection {
                            reply,
                            target_pid: pid,
                            target: target.clone(),
                            pending: paste_pending(&shared),
                        })
                        .ok()
                        .and_then(|()| selection.recv().ok())
                        .flatten();
                    if !still_there() {
                        Err(Failure::AppChanged)
                    } else {
                        match session.finish_command(request, selection) {
                            Ok(_) if !still_there() => Err(Failure::AppChanged),
                            other => other.map_err(Failure::Core),
                        }
                    }
                };
                let _ = tx.send(Msg::Finished {
                    seq,
                    outcome: Box::new(outcome),
                    released,
                    is_command: true,
                    destination,
                    preview_id,
                    pending,
                    owned: None,
                });
            });
            return;
        }
        std::thread::spawn(move || {
            let outcome = session.finish(request).map_err(Failure::Core);
            let _ = tx.send(Msg::Finished {
                seq,
                outcome: Box::new(outcome),
                released,
                is_command: false,
                destination,
                preview_id,
                pending,
                owned,
            });
        });
    }

    fn finish(
        &mut self,
        seq: u64,
        outcome: Result<DictationResult, Failure>,
        released: Instant,
        is_command: bool,
        delivery: Delivery,
        pending: DictationJob,
    ) {
        let Delivery {
            destination,
            preview_id,
            owned,
        } = delivery;
        self.pending_owned.complete(&preview_id);
        let copy_only = matches!(destination, ExternalDestination::CopyOnly);
        let _pending = pending;
        let kind = if is_command { "command" } else { "dictation" };
        let elapsed = released.elapsed().as_millis();
        let (text, notice) = match outcome {
            Ok(result) => {
                let line = format!(
                    "{kind} completed in {elapsed} ms: {} → {} characters, cleanup {}",
                    result.raw.chars().count(),
                    result.text.chars().count(),
                    describe(&result.cleanup)
                );
                if !result.history_saved || matches!(result.cleanup, CleanupStatus::Failed { .. }) {
                    LOG.error(line);
                } else {
                    LOG.info(line);
                }
                let notice =
                    notice_for_cleanup_destination(&result.cleanup, copy_only || owned.is_some());
                match prepare_delivery(
                    result,
                    &preview_id,
                    released,
                    destination,
                    cancel_destination,
                ) {
                    Ok(result) => (
                        Some(match &owned {
                            Some(capture) => CompletedDelivery::Owned {
                                preview_id: result.preview_id,
                                capture: capture.clone(),
                                history_id: result.history_id,
                                text: result.text,
                            },
                            None => CompletedDelivery::External(result),
                        }),
                        notice,
                    ),
                    Err(retained) => {
                        let active = owned.as_ref().is_none_or(OwnedCapture::valid);
                        if let Some(capture) = &owned {
                            if let Some(target) = &capture.external {
                                target.cancel();
                            }
                            owned_dictation::complete(
                                &self.app,
                                &self.shared,
                                capture,
                                retained.text,
                            );
                        }
                        if active {
                            let _ = self.app.emit("dictation-recovery", &retained.recovery);
                        }
                        (
                            None,
                            active.then(|| {
                                problem(
                                    "Saving failed",
                                    Some("Your words are temporarily in History"),
                                )
                            }),
                        )
                    }
                }
            }
            Err(failure) => {
                cancel_destination(&destination);
                if let Some(capture) = &owned
                    && capture.valid()
                {
                    let message = match &failure {
                        Failure::Core(PennelError::NoSpeech) => {
                            "No speech recognized. Try again.".into()
                        }
                        Failure::Core(error) => error.to_string(),
                        Failure::AppChanged => "The dictation session changed.".into(),
                    };
                    owned_dictation::failed(&self.app, &capture.id, message);
                }
                match &failure {
                    Failure::Core(PennelError::NoSpeech) => {
                        LOG.info(format!("{kind}: no speech detected ({elapsed} ms)"));
                    }
                    other => LOG.error(format!(
                        "{kind} failed after {elapsed} ms: {}",
                        failure_category(other)
                    )),
                }
                let notice = match &failure {
                    _ if is_command => notice_for_command(&failure),
                    Failure::Core(error) => notice_for_error(error),
                    Failure::AppChanged => notice_for_command(&failure),
                };
                (None, notice)
            }
        };
        self.clear_preview(&preview_id);
        self.status.dictation_finished(notice, Instant::now());
        let _ = self.app.emit("history-changed", ());
        for result in self.queue.fulfill(seq, text) {
            match result {
                CompletedDelivery::External(result) => {
                    if let Err(PasteJob::Paste { result, .. }) = enqueue_delivery(
                        &self.paster,
                        PasteJob::Paste {
                            result,
                            pending: paste_pending(&self.shared),
                        },
                        &mut self.status,
                    ) {
                        cancel_destination(&result.destination);
                        self.emit_recovery(recovery_for_failure(
                            &result.history_id,
                            DeliveryFailure::Other,
                        ));
                        self.status.show_notice(
                            problem_for_failure(DeliveryFailure::Other),
                            Instant::now(),
                        );
                    }
                }
                CompletedDelivery::Owned {
                    preview_id,
                    capture,
                    history_id,
                    text,
                } => {
                    if owned_dictation::complete(&self.app, &self.shared, &capture, text.clone())
                        && owned_dictation::external_allowed(&self.shared, &capture)
                        && let Err(PasteJob::Owned {
                            capture,
                            history_id,
                            ..
                        }) = enqueue_delivery(
                            &self.paster,
                            PasteJob::Owned {
                                preview_id,
                                capture,
                                history_id,
                                text,
                                pending: paste_pending(&self.shared),
                            },
                            &mut self.status,
                        )
                    {
                        self.owned_delivery_failed(&capture, &history_id);
                    }
                }
            }
        }
        self.render();
        let _ = self.tx.send(Msg::RefreshOwned);
    }

    fn emit_recovery(&self, mut recovery: RecoveryEvent) {
        recovery.reason = crate::ui::message(recovery.reason);
        let _ = self.app.emit("dictation-recovery", &recovery);
    }

    fn owned_delivery_failed(&mut self, capture: &OwnedCapture, history_id: &str) {
        if capture.valid() {
            owned_dictation::delivery_error(
                &self.app,
                capture,
                "External paste failed. Your text is in Pennel.".into(),
            );
            self.emit_recovery(recovery_for_failure(history_id, DeliveryFailure::Other));
            self.show(problem_for_failure(DeliveryFailure::Other));
        }
    }

    fn notify(&mut self, title: &str) {
        self.show(problem(title, None));
    }

    fn show(&mut self, notice: PillState) {
        self.status.show_notice(notice, Instant::now());
        self.render();
    }

    fn render(&self) {
        let state = match self.status.pill() {
            PillState::Problem { title, detail } => PillState::Problem {
                title: crate::ui::message(title),
                detail: detail.map(crate::ui::message),
            },
            other => other,
        };
        self.pill.show(state);
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop_recording();
        self.discard_recording_preview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_dictation::SessionKind;

    #[derive(Default)]
    struct PendingObserver(std::sync::Mutex<Vec<DictationPreviewUpdate>>);

    impl DictationPreviewObserver for PendingObserver {
        fn changed(&self, update: DictationPreviewUpdate) {
            self.0.lock().unwrap().push(update);
        }
    }

    struct PendingSpeech {
        entered: Sender<()>,
        release: std::sync::Mutex<Receiver<()>>,
    }

    impl pennel_core::SystemTranscriber for PendingSpeech {
        fn transcribe(
            &self,
            _: Vec<f32>,
            _: u32,
            _: Option<String>,
            _: Vec<String>,
        ) -> Result<String, PennelError> {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok("Een laat resultaat".into())
        }
    }

    fn pending_engine() -> (tempfile::TempDir, Arc<pennel_core::PennelEngine>) {
        let directory = tempfile::tempdir().unwrap();
        let engine =
            pennel_core::PennelEngine::new_preview(directory.path().to_string_lossy().into_owned())
                .unwrap();
        let mut settings = engine.settings();
        settings.providers.push(pennel_core::ProviderAccount {
            id: "apple-pending-test".into(),
            kind: pennel_core::ProviderKind::AppleSpeech,
            name: "Apple".into(),
            base_url: String::new(),
        });
        settings.dictation_stt = pennel_core::ModelChoice {
            provider_id: "apple-pending-test".into(),
            model: "dictation".into(),
        };
        settings.mode = pennel_core::DictationMode::Fast;
        engine.update_settings(settings).unwrap();
        (directory, engine)
    }

    #[test]
    fn home_practice_uses_saved_result_and_history_without_changing_onboarding_or_retention() {
        for retention in [0, 7, 30, 90] {
            for mode in [
                pennel_core::DictationMode::Fast,
                pennel_core::DictationMode::Clean,
            ] {
                let (_directory, engine) = pending_engine();
                let mut settings = engine.settings();
                settings.mode = mode;
                settings.history_retention_days = retention;
                // Missing cleanup setup must retain raw text with the ordinary failure notice,
                // rather than silently forcing onboarding's Fast result or contacting a provider.
                settings.providers.push(pennel_core::ProviderAccount {
                    id: "home-cleanup-missing-key".into(),
                    kind: pennel_core::ProviderKind::Groq,
                    name: "Synthetic cleanup".into(),
                    base_url: "https://example.invalid/v1".into(),
                });
                settings.llm.provider_id = "home-cleanup-missing-key".into();
                engine.update_settings(settings.clone()).unwrap();
                let settings = engine.settings();
                let (entered, _ready) = mpsc::channel();
                let (release, waiting) = mpsc::channel();
                engine.set_system_transcriber(Arc::new(PendingSpeech {
                    entered,
                    release: std::sync::Mutex::new(waiting),
                }));
                release.send(()).unwrap();
                let session = engine.clone().begin_dictation_session(
                    Arc::new(PendingObserver::default()),
                    SessionKind::HomePractice.onboarding_practice(),
                );
                let result = session
                    .finish(DictationRequest {
                        samples: vec![0.1; 16_000],
                        sample_rate: 16_000,
                        app_bundle_id: None,
                        keys: pennel_core::ApiKeys {
                            by_provider: HashMap::new(),
                        },
                    })
                    .unwrap();
                assert_eq!(result.text, "Een laat resultaat");
                if mode == pennel_core::DictationMode::Fast {
                    assert_eq!(result.cleanup, CleanupStatus::Skipped);
                } else {
                    assert!(matches!(result.cleanup, CleanupStatus::Failed { .. }));
                }
                assert!(result.history_saved);
                assert_eq!(engine.history(10).unwrap()[0].text, result.text);
                assert_eq!(engine.settings(), settings);
            }
        }
    }

    #[test]
    fn meeting_previews_remain_paused_until_capture_and_every_dictation_job_exit() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing(directory.path().join("preview"));
        let mut settings = shared.settings();
        settings.providers.push(pennel_core::ProviderAccount {
            id: "apple-preview-priority".into(),
            kind: pennel_core::ProviderKind::AppleSpeech,
            name: "Apple".into(),
            base_url: String::new(),
        });
        settings.meeting_stt = pennel_core::ModelChoice {
            provider_id: "apple-preview-priority".into(),
            model: "system".into(),
        };
        shared.engine.update_settings(settings).unwrap();
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Priority fixture".into(),
                pennel_core::ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        let capture = CaptureActivity::begin(shared.clone()).unwrap();
        let first = DictationJob::new(shared.clone());
        let second = DictationJob::new(shared.clone());
        let busy = || {
            session
                .preview_snapshot()
                .tracks
                .iter()
                .all(|track| track.state == pennel_core::MeetingPreviewState::Busy)
        };
        assert!(busy());
        drop(capture);
        assert!(busy());
        drop(first);
        assert!(busy());
        drop(second);
        assert!(
            session
                .preview_snapshot()
                .tracks
                .iter()
                .all(|track| track.state == pennel_core::MeetingPreviewState::Warming)
        );
        session.stop().unwrap();
    }

    #[test]
    fn closing_a_finalizing_owner_cancels_core_before_any_history_or_result_commits() {
        for practice in [false, true] {
            let (_directory, engine) = pending_engine();
            let (entered, ready) = mpsc::channel();
            let (release, waiting) = mpsc::channel();
            engine.set_system_transcriber(Arc::new(PendingSpeech {
                entered,
                release: std::sync::Mutex::new(waiting),
            }));
            let observer = Arc::new(PendingObserver::default());
            let session = engine
                .clone()
                .begin_dictation_session(observer.clone(), practice);
            let weak = Arc::downgrade(&session);
            let mut pending = PendingOwnedSessions::default();
            pending.track("closed-owner", session.clone());
            let worker = std::thread::spawn(move || {
                session.finish(DictationRequest {
                    samples: vec![0.1; 16_000],
                    sample_rate: 16_000,
                    app_bundle_id: Some("pennel.test.owned".into()),
                    keys: pennel_core::ApiKeys {
                        by_provider: std::collections::HashMap::new(),
                    },
                })
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            pending.cancel_owner("closed-owner");
            assert!(pending.0.is_empty());
            release.send(()).unwrap();
            assert_eq!(worker.join().unwrap(), Err(PennelError::Cancelled));
            assert!(observer.0.lock().unwrap().is_empty());
            assert!(engine.history(10).unwrap().is_empty());
            assert!(
                weak.upgrade().is_none(),
                "the finished worker must release its session"
            );
        }
    }

    #[test]
    fn owner_cancellation_reaches_all_pending_captures_without_touching_another_owner() {
        let (_directory, engine) = pending_engine();
        let observer = Arc::new(PendingObserver::default());
        let old = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let newer = engine
            .clone()
            .begin_dictation_session(observer.clone(), false);
        let other = engine.begin_dictation_session(observer.clone(), false);
        let mut pending = PendingOwnedSessions::default();
        pending.track("pad", old.clone());
        pending.track("pad", newer.clone());
        pending.track("practice", other.clone());
        pending.cancel_owner("pad");
        old.update_system_preview("oud".into());
        newer.update_system_preview("ook oud".into());
        other.update_system_preview("nog actief".into());
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        assert_eq!(observer.0.lock().unwrap()[0].session_id, other.id());
        assert_eq!(pending.0.len(), 1);
        pending.complete(&other.id());
        assert!(pending.0.is_empty());
        other.update_system_preview("bewaarde actieve sessie".into());
        assert_eq!(observer.0.lock().unwrap().len(), 2);
    }

    #[test]
    fn commands_and_style_rules_use_the_app_at_stop_while_owned_modes_keep_pennel() {
        let start = ForegroundApp {
            pid: 24,
            executable: "outlook.exe".into(),
        };
        let stop = ForegroundApp {
            pid: 42,
            executable: "slack.exe".into(),
        };
        let start_context = StoppedContext::new(Some(&start), None, Some("com.microsoft.Outlook"));
        let stop_context =
            StoppedContext::new(Some(&stop), None, Some("com.tinyspeck.slackmacgap"));
        assert_ne!(start_context.command_pid, stop_context.command_pid);
        assert_eq!(stop_context.command_pid, Some(42));
        assert_eq!(
            stop_context.app_bundle_id.as_deref(),
            Some("com.tinyspeck.slackmacgap")
        );
        assert!(command_field_matches(
            stop_context.command_pid,
            Some(42),
            true
        ));
        assert!(!command_field_matches(
            stop_context.command_pid,
            Some(24),
            true
        ));
        assert!(!command_field_matches(
            stop_context.command_pid,
            Some(42),
            false
        ));
        let racing_query =
            StoppedContext::new(Some(&start), None, Some("com.tinyspeck.slackmacgap"));
        assert_eq!(racing_query.app_bundle_id, stop_context.app_bundle_id);
        assert_eq!(
            StoppedContext::new(Some(&stop), None, None).app_bundle_id,
            None
        );
        let owned = StoppedContext::new(
            Some(&stop),
            Some("dev.pennel.preview"),
            Some("com.tinyspeck.slackmacgap"),
        );
        assert_eq!(owned.command_pid, None);
        assert_eq!(owned.app_bundle_id.as_deref(), Some("dev.pennel.preview"));
    }

    #[test]
    fn global_delivery_captures_the_destination_once_and_never_for_copy_only() {
        let calls = std::cell::Cell::new(0);
        let destination = capture_destination(false, || {
            calls.set(calls.get() + 1);
            Ok(42u32)
        });
        assert_eq!(destination, ExternalDestination::Captured(42));
        assert_eq!(calls.get(), 1);
        let copied = capture_destination(true, || {
            calls.set(calls.get() + 1);
            Ok(99u32)
        });
        assert_eq!(copied, ExternalDestination::CopyOnly);
        assert_eq!(calls.get(), 1);
        assert_eq!(
            capture_destination::<u32>(false, || Err("no current field".into())),
            ExternalDestination::Unavailable("no current field".into())
        );
    }

    #[test]
    fn replaced_and_cancelled_recordings_reject_late_preview_updates() {
        let mut cursor = PreviewCursor::default();
        cursor.begin("first".into(), Some("pad".into()));
        assert!(cursor.accept(&preview("first", 1), true));
        assert!(!cursor.accept(&preview("first", 1), true));
        assert!(!cursor.accept(&preview("first", 0), true));
        cursor.begin("second".into(), None);
        assert!(!cursor.accept(&preview("first", 9), true));
        assert!(!cursor.clear_session("first"));
        assert!(cursor.accept(&preview("second", 1), true));
        assert!(cursor.clear_session("second"));
        assert!(!cursor.accept(&preview("second", 2), true));
    }

    #[test]
    fn closed_owners_cannot_publish_preview_text_into_a_different_session() {
        let mut cursor = PreviewCursor::default();
        cursor.begin("practice-capture".into(), Some("practice".into()));
        assert!(!cursor.accept(&preview("practice-capture", 1), false));
        assert_eq!(cursor.clear_owner("other"), None);
        assert_eq!(
            cursor.clear_owner("practice").as_deref(),
            Some("practice-capture")
        );
        assert!(!cursor.accept(&preview("practice-capture", 2), true));
        cursor.begin("pad-capture".into(), Some("pad".into()));
        assert_eq!(cursor.clear_owner("practice"), None);
        assert!(cursor.accept(&preview("pad-capture", 1), true));
    }

    fn preview(id: &str, revision: u64) -> DictationPreviewUpdate {
        DictationPreviewUpdate {
            session_id: id.into(),
            revision,
            text: "Voorlopige tekst".into(),
        }
    }

    #[test]
    fn provider_error_details_never_enter_diagnostic_labels() {
        let private = "sk-test-echoed-private-wording";
        assert_eq!(
            describe(&CleanupStatus::Failed {
                message: private.into()
            }),
            "failed"
        );
        assert_eq!(
            failure_category(&Failure::Core(PennelError::Http {
                status: 400,
                message: private.into()
            })),
            "provider_http_400"
        );
        assert_eq!(
            failure_category(&Failure::Core(PennelError::InvalidResponse {
                message: private.into()
            })),
            "provider_response"
        );
    }

    #[test]
    fn quit_and_update_barriers_release_the_capture_reservation() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("preview"));
        shared.quitting.store(true, Ordering::SeqCst);
        assert!(CaptureActivity::begin(shared.clone()).is_none());
        assert!(!shared.dictation_active.load(Ordering::SeqCst));
        shared.quitting.store(false, Ordering::SeqCst);
        shared.updating.store(true, Ordering::SeqCst);
        assert!(CaptureActivity::begin(shared.clone()).is_none());
        assert!(!shared.dictation_active.load(Ordering::SeqCst));
        shared.updating.store(false, Ordering::SeqCst);
        let active = CaptureActivity::begin(shared.clone()).unwrap();
        assert!(shared.dictation_active.load(Ordering::SeqCst));
        drop(active);
        assert!(!shared.dictation_active.load(Ordering::SeqCst));
    }

    #[test]
    fn owned_notices_have_english_canonical_text() {
        assert_eq!(notice_for_error(&PennelError::NoSpeech), None);
        assert_eq!(
            notice_for_error(&PennelError::Http {
                status: 401,
                message: String::new()
            }),
            Some(problem("API key rejected", Some(KEPT_IN_HISTORY)))
        );
        assert_eq!(
            notice_for_error(&PennelError::UnsupportedProcessor),
            Some(problem(
                "Processor too old for local models",
                Some("Your words are kept in History")
            ))
        );
        assert_eq!(
            notice_for_error(&PennelError::MissingApiKey { task: "stt".into() }),
            Some(problem("No API key configured", None))
        );
        assert_eq!(
            notice_for_cleanup(&CleanupStatus::Failed {
                message: "request Timed Out".into()
            }),
            Some(problem(
                "Cleanup timed out",
                Some("Original text pasted (see log)")
            ))
        );
        assert_eq!(notice_for_cleanup(&CleanupStatus::Applied), None);
        assert_eq!(
            notice_for_command(&Failure::AppChanged),
            Some(problem("Another app is active — command not pasted", None))
        );
        assert_eq!(
            notice_for_command(&Failure::Core(PennelError::InvalidResponse {
                message: "selection too long".into()
            })),
            Some(problem("Selection is too long for a command", None))
        );
    }

    #[test]
    fn every_pill_notice_has_english_and_dutch_text() {
        let mut sources: Vec<String> = [
            KEPT_IN_HISTORY,
            "Your words are temporarily in History",
            "Saving failed",
            "Model not downloaded",
            "Processor too old for local models",
        ]
        .map(String::from)
        .into();
        for failure in [
            DeliveryFailure::Missing,
            DeliveryFailure::Changed,
            DeliveryFailure::Permission,
            DeliveryFailure::Other,
            DeliveryFailure::Copy,
        ] {
            let PillState::Problem { title, .. } = problem_for_failure(failure) else {
                panic!("a failed delivery is a problem");
            };
            sources.push(title);
        }
        for cleanup in [
            CleanupStatus::Failed {
                message: "timed out".into(),
            },
            CleanupStatus::Failed {
                message: "error".into(),
            },
            CleanupStatus::Rejected,
        ] {
            for available in [true, false] {
                let Some(PillState::Problem { title, detail }) =
                    notice_for_cleanup_destination(&cleanup, available)
                else {
                    panic!("a cleanup failure is a problem");
                };
                sources.extend([title, detail.unwrap()]);
            }
        }
        for source in sources {
            let dutch =
                pennel_core::localize_ui_text(pennel_core::InterfaceLanguage::Dutch, &source);
            assert_ne!(dutch, source, "no Dutch text for {source:?}");
        }
    }

    #[test]
    fn cleanup_notices_describe_available_text_without_claiming_it_was_pasted() {
        for (cleanup, title, detail) in [
            (
                CleanupStatus::Failed {
                    message: "request timed out".into(),
                },
                "Cleanup timed out",
                "Original text available (see log)",
            ),
            (
                CleanupStatus::Failed {
                    message: "provider error".into(),
                },
                "Cleanup failed",
                "Original text available (see log)",
            ),
            (
                CleanupStatus::Rejected,
                "Cleanup could not preserve the wording and meaning",
                "Original text available",
            ),
        ] {
            assert_eq!(
                notice_for_cleanup_destination(&cleanup, true),
                Some(problem(title, Some(detail)))
            );
            let Some(PillState::Problem {
                detail: Some(pasted),
                ..
            }) = notice_for_cleanup_destination(&cleanup, false)
            else {
                panic!("a pasted cleanup failure says where the text went");
            };
            assert!(pasted.contains("pasted"));
        }
        assert_eq!(
            notice_for_cleanup_destination(&CleanupStatus::Applied, true),
            None
        );
        assert_eq!(
            notice_for_cleanup_destination(&CleanupStatus::Skipped, true),
            None
        );
    }

    #[test]
    fn copy_confirmation_requires_a_successful_clipboard_write_and_names_the_paste_key() {
        let (notices, received) = mpsc::channel();
        let mut delivery = DeliveryCompletion::new(notices.clone(), "copied-capture".into());
        finish_copy(&mut delivery, "copied-result", Ok(()));
        drop(delivery);
        let Msg::Delivered {
            preview_id,
            notice: Some(copied),
        } = received.try_recv().unwrap()
        else {
            panic!("a successful clipboard write is confirmed");
        };
        assert_eq!(preview_id, "copied-capture");
        let key = if cfg!(target_os = "macos") {
            "⌘V"
        } else {
            "Ctrl+V"
        };
        assert_eq!(
            copied,
            PillState::Copied {
                paste_key: key.into()
            }
        );
        let mut delivery = DeliveryCompletion::new(notices, "failed-capture".into());
        finish_copy(&mut delivery, "copied-result", Err("unavailable".into()));
        drop(delivery);
        assert!(matches!(
            received.try_recv().unwrap(),
            Msg::DeliveryRecovery(_)
        ));
        let Msg::Delivered { preview_id, notice } = received.try_recv().unwrap() else {
            panic!("failure must settle delivery")
        };
        assert_eq!(preview_id, "failed-capture");
        assert_eq!(notice, Some(problem_for_failure(DeliveryFailure::Copy)));
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn a_delivery_without_a_confirmation_still_sends_one_terminal_capture_outcome() {
        let (notices, received) = mpsc::channel();
        drop(DeliveryCompletion::new(notices, "owned-capture".into()));
        let Msg::Delivered { preview_id, notice } = received.try_recv().unwrap() else {
            panic!("an owned or cancelled delivery still settles its pending work");
        };
        assert_eq!(preview_id, "owned-capture");
        assert_eq!(notice, None);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn owned_worker_exits_settle_their_delivery_once_without_showing_a_success_notice() {
        for (exit, allowed, target, paste_error, expected_attempts, expected_failures) in [
            ("success", true, Some(42), false, 1, 0),
            ("invalid owner", false, Some(42), false, 0, 0),
            ("missing target", true, None, false, 0, 0),
            ("cancelled", false, Some(42), false, 0, 0),
            ("preparation failed", true, Some(42), true, 1, 1),
            ("paste failed", true, Some(42), true, 1, 1),
        ] {
            let mut status = DictationStatus::default();
            let (jobs, worker) = mpsc::channel();
            let (notices, received) = mpsc::channel();
            enqueue_delivery(&jobs, exit, &mut status).unwrap();
            assert_eq!(status.pill(), PillState::Working, "{exit}");
            let id = worker.try_recv().unwrap();
            let attempts = std::cell::Cell::new(0);
            let failures = std::cell::Cell::new(0);
            deliver_owned(
                DeliveryCompletion::new(notices, id.into()),
                allowed,
                target.as_ref(),
                |target| {
                    assert_eq!(*target, 42);
                    attempts.set(attempts.get() + 1);
                    if paste_error {
                        Err("synthetic delivery failure".into())
                    } else {
                        Ok(())
                    }
                },
                || failures.set(failures.get() + 1),
            );
            assert_eq!(attempts.get(), expected_attempts, "{exit}");
            assert_eq!(failures.get(), expected_failures, "{exit}");
            let Msg::Delivered { preview_id, notice } = received.try_recv().unwrap() else {
                panic!("{exit} must send a terminal outcome");
            };
            assert_eq!(preview_id, exit);
            assert_eq!(notice, None);
            status.delivery_finished(&preview_id, notice, Instant::now());
            assert_eq!(status.pill(), PillState::Hidden, "{exit}");
            assert!(
                received.try_recv().is_err(),
                "{exit} settled more than once"
            );
        }
    }

    #[test]
    fn a_failed_enqueue_retains_the_job_without_creating_pending_delivery() {
        let (jobs, worker) = mpsc::channel();
        drop(worker);
        let mut status = DictationStatus::default();
        status.delivery_started();
        assert_eq!(
            enqueue_delivery(&jobs, "not-enqueued", &mut status),
            Err("not-enqueued")
        );
        assert_eq!(status.pill(), PillState::Working);
        status.delivery_finished("already-pending", None, Instant::now());
        assert_eq!(status.pill(), PillState::Hidden);
    }

    #[test]
    fn queued_completions_preserve_old_recovery_after_a_new_capture_cancels() {
        let now = Instant::now();
        let (jobs, worker) = mpsc::channel();
        let (notices, received) = mpsc::channel();
        let mut status = DictationStatus::default();
        status.recording_started("first", false);
        status.recording_submitted();
        enqueue_delivery(&jobs, "first", &mut status).unwrap();
        status.dictation_finished(None, now);
        status.recording_started("second", false);
        status.recording_cancelled();
        assert_eq!(status.pill(), PillState::Working);
        let mut delivery = DeliveryCompletion::new(notices, worker.try_recv().unwrap().into());
        unavailable_destination(&mut delivery, "first-history", DeliveryFailure::Changed);
        drop(delivery);
        let Msg::DeliveryRecovery(recovery) = received.try_recv().unwrap() else {
            panic!("recovery must survive supersession")
        };
        assert_eq!(recovery.id, "first-history");
        let Msg::Delivered { preview_id, notice } = received.try_recv().unwrap() else {
            panic!("recovery must settle delivery")
        };
        status.delivery_finished(&preview_id, notice, now);
        assert_eq!(status.pill(), problem_for_failure(DeliveryFailure::Changed));
        status.expire_notice(now + Duration::from_millis(3500));
        assert_eq!(status.pill(), PillState::Hidden);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn insertion_names_the_receiving_app_only_when_it_is_known() {
        assert_eq!(
            insertion_notice(" Mail ", true),
            PillState::Inserted {
                app: Some("Mail".into())
            }
        );
        assert_eq!(inserted_in(""), PillState::Inserted { app: None });
        assert_eq!(
            insertion_notice("slack.exe", false),
            PillState::Inserted { app: None }
        );
        assert_eq!(
            insertion_notice("org.gnome.TextEditor", false),
            PillState::Inserted { app: None }
        );
        assert_eq!(
            problem_for_failure(DeliveryFailure::Changed),
            problem(
                "Not pasted — the text destination changed",
                Some(KEPT_IN_HISTORY)
            )
        );
    }

    #[test]
    fn queued_commands_require_the_original_foreground_process() {
        assert!(target_matches(Some(42), Some(42)));
        assert!(!target_matches(Some(42), Some(9)));
        assert!(!target_matches(Some(42), None));
        assert!(target_matches(None, Some(9)));
        assert!(command_field_matches(Some(42), Some(42), true));
        assert!(!command_field_matches(Some(42), Some(42), false));
        assert!(!command_field_matches(Some(42), Some(9), true));
        assert!(!command_field_matches(None, Some(9), false));
    }

    #[test]
    fn ordered_paste_results_keep_their_own_target_and_release_time() {
        let mut queue: PasteQueue<PasteText<u32>> = PasteQueue::default();
        let first = queue.reserve();
        let second = queue.reserve();
        let released = Instant::now();
        assert!(
            queue
                .fulfill(
                    second,
                    Some(PasteText {
                        preview_id: "second-capture".into(),
                        history_id: "second-history".into(),
                        text: "second".into(),
                        released,
                        destination: ExternalDestination::Captured(42),
                    })
                )
                .is_empty()
        );
        let ready = queue.fulfill(
            first,
            Some(PasteText {
                preview_id: "first-capture".into(),
                history_id: "first-history".into(),
                text: "first".into(),
                released: released - Duration::from_secs(1),
                destination: ExternalDestination::Captured(24),
            }),
        );
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].destination, ExternalDestination::Captured(24));
        assert_eq!(ready[1].destination, ExternalDestination::Captured(42));
        assert_eq!(ready[1].released, released);
        assert_eq!(ready[0].released, released - Duration::from_secs(1));
        assert_eq!(ready[0].preview_id, "first-capture");
        assert_eq!(ready[1].preview_id, "second-capture");
    }

    fn completed_result(id: &str, history_saved: bool) -> DictationResult {
        DictationResult {
            history_id: id.into(),
            history_saved,
            text: "synthetic-private-finished-text".into(),
            raw: "synthetic-private-raw-text".into(),
            cleanup: CleanupStatus::Skipped,
        }
    }

    #[test]
    fn unsaved_results_cancel_the_frozen_target_and_release_the_next_queued_result() {
        let mut queue = PasteQueue::default();
        let first = queue.reserve();
        let second = queue.reserve();
        let released = Instant::now();
        let later = prepare_delivery(
            completed_result("saved-second", true),
            "capture-saved-second",
            released,
            ExternalDestination::Captured(99u32),
            |_| panic!("a saved result must retain its original destination"),
        )
        .unwrap();
        assert!(queue.fulfill(second, Some(later)).is_empty());

        let cancelled = std::cell::Cell::new(false);
        let unsaved = prepare_delivery(
            completed_result("unsaved-first", false),
            "capture-unsaved-first",
            released,
            ExternalDestination::Captured(42u32),
            |destination| {
                assert_eq!(destination, &ExternalDestination::Captured(42));
                cancelled.set(true);
            },
        )
        .unwrap_err();
        assert!(cancelled.get());
        assert_eq!(unsaved.text, "synthetic-private-finished-text");
        assert_eq!(unsaved.recovery.id, "unsaved-first");
        assert!(unsaved.recovery.unsaved);
        let ready = queue.fulfill(first, None);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].history_id, "saved-second");
        assert_eq!(ready[0].destination, ExternalDestination::Captured(99));

        let serialized = serde_json::to_string(&unsaved.recovery).unwrap();
        assert!(!serialized.contains("synthetic-private-finished-text"));
        assert!(!serialized.contains("synthetic-private-raw-text"));
    }

    #[test]
    fn delivery_failures_retain_each_queued_results_own_recovery_identity() {
        let mut queue = PasteQueue::default();
        let first = queue.reserve();
        let second = queue.reserve();
        let released = Instant::now();
        let later = prepare_delivery(
            completed_result("later-result", true),
            "capture-later-result",
            released,
            ExternalDestination::Captured(20u32),
            |_| panic!("saved result was cancelled"),
        )
        .unwrap();
        assert!(queue.fulfill(second, Some(later)).is_empty());
        let earlier = prepare_delivery(
            completed_result("earlier-result", true),
            "capture-earlier-result",
            released,
            ExternalDestination::Captured(10u32),
            |_| panic!("saved result was cancelled"),
        )
        .unwrap();
        let ready = queue.fulfill(first, Some(earlier));
        for (result, expected_id) in ready.iter().zip(["earlier-result", "later-result"]) {
            let recovery = recovery_for_failure(&result.history_id, DeliveryFailure::Changed);
            assert_eq!(recovery.id, expected_id);
            assert!(!recovery.unsaved);
            assert!(recovery.reason.contains("text destination changed"));
        }
    }

    #[test]
    fn copy_failure_recovery_has_a_saved_identity_and_no_dictation_content() {
        let (notices, received) = mpsc::channel();
        let mut delivery = DeliveryCompletion::new(notices, "clipboard-capture".into());
        finish_copy(
            &mut delivery,
            "clipboard-result",
            Err("synthetic-private-finished-text".into()),
        );
        drop(delivery);
        let Msg::DeliveryRecovery(recovery) = received.try_recv().unwrap() else {
            panic!("failed clipboard write must report recoverable text");
        };
        assert_eq!(
            serde_json::to_value(recovery).unwrap(),
            serde_json::json!({
                "id": "clipboard-result",
                "unsaved": false,
                "reason": "Copying failed. Your text is in History."
            })
        );
    }

    struct PasteClipboard {
        state: std::rc::Rc<std::cell::RefCell<(String, u64)>>,
        fail_writes: bool,
    }

    impl crate::clipboard::Clipboard for PasteClipboard {
        type Snapshot = String;

        fn fingerprint(&mut self) -> u64 {
            self.state.borrow().1
        }
        fn snapshot(&mut self) -> String {
            self.state.borrow().0.clone()
        }
        fn restore(&mut self, text: &String) {
            let mut state = self.state.borrow_mut();
            state.0 = text.clone();
            state.1 += 1;
        }
        fn try_set_transient_text(&mut self, text: &str, _: bool) -> Result<(), String> {
            if self.fail_writes {
                return Err("synthetic clipboard error".into());
            }
            let mut state = self.state.borrow_mut();
            state.0 = text.into();
            state.1 += 1;
            Ok(())
        }
        fn text(&mut self) -> Option<String> {
            Some(self.state.borrow().0.clone())
        }
    }

    #[test]
    fn a_failed_clipboard_write_cannot_paste_stale_user_content() {
        let state = std::rc::Rc::new(std::cell::RefCell::new(("earlier user copy".into(), 1)));
        let mut clipboard = PasteClipboard {
            state: state.clone(),
            fail_writes: true,
        };
        let sent = std::cell::Cell::new(false);
        let result = paste_transaction(
            &mut clipboard,
            "synthetic dictation",
            || {
                sent.set(true);
                Ok(())
            },
            || panic!("failed writes must not wait for paste delivery"),
        );
        assert_eq!(result, Err(crate::clipboard::COPY_FAILED.into()));
        assert!(!sent.get());
        assert_eq!(state.borrow().0, "earlier user copy");
    }

    #[test]
    fn paste_errors_restore_the_prior_clipboard_without_replacing_a_new_user_copy() {
        for user_copies in [false, true] {
            let state = std::rc::Rc::new(std::cell::RefCell::new(("earlier user copy".into(), 1)));
            let mut clipboard = PasteClipboard {
                state: state.clone(),
                fail_writes: false,
            };
            let result = paste_transaction(
                &mut clipboard,
                "synthetic dictation",
                || {
                    assert_eq!(state.borrow().0, "synthetic dictation");
                    Err("synthetic paste failure".into())
                },
                || {
                    if user_copies {
                        let mut state = state.borrow_mut();
                        state.0 = "new user copy".into();
                        state.1 += 1;
                    }
                },
            );
            assert!(result.is_err());
            assert_eq!(
                state.borrow().0,
                if user_copies {
                    "new user copy"
                } else {
                    "earlier user copy"
                }
            );
        }
    }

    #[test]
    fn transcription_to_ordered_paste_transfer_has_no_idle_gap() {
        let dictations = Arc::new(AtomicUsize::new(0));
        let pastes = Arc::new(AtomicUsize::new(0));
        let first_job = CountedWork::new(dictations.clone(), |count| count);
        let second_job = CountedWork::new(dictations.clone(), |count| count);
        let mut queue = PasteQueue::default();
        let first = queue.reserve();
        let second = queue.reserve();
        assert!(queue.fulfill(second, Some("second")).is_empty());
        drop(second_job);
        assert_eq!(dictations.load(Ordering::SeqCst), 1);
        let queued: Vec<_> = queue
            .fulfill(first, Some("first"))
            .into_iter()
            .map(|text| (text, CountedWork::new(pastes.clone(), |count| count)))
            .collect();
        assert_eq!(dictations.load(Ordering::SeqCst), 1);
        assert_eq!(pastes.load(Ordering::SeqCst), 2);
        drop(first_job);
        assert_eq!(dictations.load(Ordering::SeqCst), 0);
        assert_eq!(pastes.load(Ordering::SeqCst), 2);
        drop(queued);
        assert_eq!(pastes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn disconnected_queue_releases_pending_operation_count() {
        let count = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let failed = sender.send(CountedWork::new(count.clone(), |count| count));
        assert!(failed.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(failed);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn closing_queue_releases_all_waiting_operations() {
        let count = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = mpsc::channel();
        assert!(
            sender
                .send(CountedWork::new(count.clone(), |count| count))
                .is_ok()
        );
        assert!(
            sender
                .send(CountedWork::new(count.clone(), |count| count))
                .is_ok()
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        drop(receiver);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
