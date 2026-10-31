//! Pennel Cloud: account and vault commands for the webview, and the background sync runner.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pennel_core::{
    CloudAccount, CloudDevice, CloudLinkAvailability, CloudLinkRole, CloudLinkView, CloudLoginPoll,
    CloudLoginStart, CloudSyncSetup, MeetingSyncState, PennelError, Settings, SyncItemState,
    SyncReport, SyncStatus,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::diagnostics::Log;
use crate::state::Shared;

const LOG: Log = Log::new("cloud");
const SYNC_INTERVAL: Duration = Duration::from_secs(180);

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct CloudError {
    code: String,
    message: String,
}

impl CloudError {
    pub(crate) fn other(message: impl std::fmt::Display) -> Self {
        Self {
            code: "error".into(),
            message: crate::commands::localize_error_message(message.to_string()),
        }
    }
}

impl From<PennelError> for CloudError {
    fn from(error: PennelError) -> Self {
        let code = match &error {
            PennelError::VaultLocked => "vault_locked".to_owned(),
            PennelError::NotSignedIn => "not_signed_in".to_owned(),
            PennelError::Cloud { code, .. } => code.clone(),
            PennelError::Storage { .. } => "storage".to_owned(),
            PennelError::Network { .. } => "network".to_owned(),
            PennelError::Cancelled => "cancelled".to_owned(),
            PennelError::Http { status, .. } => format!("http_{status}"),
            _ => "error".to_owned(),
        };
        Self {
            code,
            message: crate::commands::localize_error_message(error.to_string()),
        }
    }
}

type Result<T> = std::result::Result<T, CloudError>;

fn err(e: PennelError) -> CloudError {
    e.into()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(CloudError::other)?
}

#[derive(Default)]
struct Signals {
    poked: bool,
    running: bool,
    stop: bool,
}

struct Inner {
    signals: Mutex<Signals>,
    wake: Condvar,
}

/// Runs sync passes on a timer and on demand: one at a time, with at most one rerun queued.
pub struct SyncRunner {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SyncRunner {
    pub fn start(shared: Arc<Shared>, app: AppHandle) -> SyncRunner {
        let recording = {
            let shared = shared.clone();
            move || recording_meeting_ids(&shared)
        };
        let run = move |ids: Vec<String>| {
            if !should_run(
                shared.settings().cloud_sync,
                shared.engine.cloud_signed_in(),
            ) {
                return;
            }
            let report = shared.engine.cloud_sync_run(ids);
            let _ = app.emit("sync-changed", SyncReportView::from(report));
        };
        Self::with_runner(SYNC_INTERVAL, recording, run)
    }

    fn with_runner(
        interval: Duration,
        recording_ids: impl Fn() -> Vec<String> + Send + 'static,
        run: impl Fn(Vec<String>) + Send + 'static,
    ) -> SyncRunner {
        let inner = Arc::new(Inner {
            signals: Mutex::new(Signals {
                poked: true,
                ..Signals::default()
            }),
            wake: Condvar::new(),
        });
        let worker = inner.clone();
        let thread = std::thread::spawn(move || {
            loop {
                {
                    let mut signals = worker.signals.lock().unwrap();
                    while !signals.poked && !signals.stop {
                        let (next, timeout) = worker.wake.wait_timeout(signals, interval).unwrap();
                        signals = next;
                        if timeout.timed_out() {
                            signals.poked = true;
                        }
                    }
                    if signals.stop {
                        return;
                    }
                    signals.poked = false;
                    signals.running = true;
                }
                let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(recording_ids());
                }));
                if pass.is_err() {
                    LOG.error("sync stopped unexpectedly; the next pass will retry");
                }
                worker.signals.lock().unwrap().running = false;
            }
        });
        SyncRunner {
            inner,
            thread: Mutex::new(Some(thread)),
        }
    }

    pub fn poke(&self) {
        self.inner.signals.lock().unwrap().poked = true;
        self.inner.wake.notify_all();
    }

    /// Returns at once; a pass in flight is left to finish on its own because its state is
    /// written per file and a partial pass is safe to resume.
    pub fn stop(&self) {
        let idle = {
            let mut signals = self.inner.signals.lock().unwrap();
            signals.stop = true;
            !signals.running
        };
        self.inner.wake.notify_all();
        if idle && let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}

/// A pass only makes sense while sync is on and an account is signed in; deleting the account
/// or logging out turns every later pass into a no-op.
fn should_run(sync_on: bool, signed_in: bool) -> bool {
    sync_on && signed_in
}

fn recording_meeting_ids(shared: &Shared) -> Vec<String> {
    shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .map(|context| vec![context.session.id()])
        .unwrap_or_default()
}

pub fn poke(app: &AppHandle) {
    if let Some(runner) = app.try_state::<SyncRunner>() {
        runner.poke();
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncStatusView {
    Off,
    SignedOut,
    NeedsSetup,
    NeedsRecoveryCode,
    VaultReset,
    Pending,
    Synced,
    Offline,
    StorageFull { message: String },
    Error { message: String },
}

impl From<SyncStatus> for SyncStatusView {
    fn from(status: SyncStatus) -> Self {
        match status {
            SyncStatus::Off => Self::Off,
            SyncStatus::SignedOut => Self::SignedOut,
            SyncStatus::NeedsSetup => Self::NeedsSetup,
            SyncStatus::NeedsRecoveryCode => Self::NeedsRecoveryCode,
            SyncStatus::VaultReset => Self::VaultReset,
            SyncStatus::Pending => Self::Pending,
            SyncStatus::Synced => Self::Synced,
            SyncStatus::Offline => Self::Offline,
            SyncStatus::StorageFull { message } => Self::StorageFull {
                message: crate::commands::localize_error_message(message),
            },
            SyncStatus::Error { message } => Self::Error {
                message: crate::commands::localize_error_message(message),
            },
        }
    }
}

#[derive(Serialize, Clone)]
pub struct SyncReportView {
    status: SyncStatusView,
    brake: bool,
    uploaded: u32,
    downloaded: u32,
    conflicts: u32,
    last_success_ms: Option<i64>,
}

impl From<SyncReport> for SyncReportView {
    fn from(report: SyncReport) -> Self {
        Self {
            brake: matches!(&report.status, SyncStatus::Error { message } if message == pennel_core::MASS_DELETE),
            status: report.status.into(),
            uploaded: report.uploaded,
            downloaded: report.downloaded,
            conflicts: report.conflicts,
            last_success_ms: report.last_success_ms,
        }
    }
}

#[derive(Serialize)]
pub struct LoginStartView {
    user_code: String,
    verification_uri: String,
    device_code: String,
    interval_secs: u32,
}

impl From<CloudLoginStart> for LoginStartView {
    fn from(start: CloudLoginStart) -> Self {
        Self {
            user_code: start.user_code,
            verification_uri: start.verification_uri,
            device_code: start.device_code,
            interval_secs: start.interval_secs,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginPollView {
    Pending,
    SlowDown,
    Expired,
    Done { email: String },
}

impl From<CloudLoginPoll> for LoginPollView {
    fn from(poll: CloudLoginPoll) -> Self {
        match poll {
            CloudLoginPoll::Pending => Self::Pending,
            CloudLoginPoll::SlowDown => Self::SlowDown,
            CloudLoginPoll::Expired => Self::Expired,
            CloudLoginPoll::Done { email } => Self::Done { email },
        }
    }
}

#[derive(Serialize)]
pub struct AccountView {
    email: String,
    audio_seconds: u64,
    tokens: u64,
    storage_bytes: u64,
    limit_audio_seconds: u64,
    limit_tokens: u64,
    limit_storage_bytes: u64,
}

impl From<CloudAccount> for AccountView {
    fn from(account: CloudAccount) -> Self {
        Self {
            email: account.email,
            audio_seconds: account.audio_seconds,
            tokens: account.tokens,
            storage_bytes: account.storage_bytes,
            limit_audio_seconds: account.limit_audio_seconds,
            limit_tokens: account.limit_tokens,
            limit_storage_bytes: account.limit_storage_bytes,
        }
    }
}

#[derive(Serialize)]
pub struct DeviceView {
    id: String,
    name: String,
    platform: String,
    last_seen_iso: String,
    current: bool,
}

impl From<CloudDevice> for DeviceView {
    fn from(device: CloudDevice) -> Self {
        Self {
            id: device.id,
            name: device.name,
            platform: device.platform,
            last_seen_iso: device.last_seen_iso,
            current: device.current,
        }
    }
}

#[derive(Serialize)]
pub struct MeetingSyncView {
    id: String,
    state: &'static str,
}

impl From<MeetingSyncState> for MeetingSyncView {
    fn from(item: MeetingSyncState) -> Self {
        Self {
            id: item.id,
            state: match item.state {
                SyncItemState::Synced => "synced",
                SyncItemState::Pending => "pending",
                SyncItemState::Conflict => "conflict",
            },
        }
    }
}

/// Only the dedicated cloud commands may switch sync or Claude-toegang on or off.
pub fn keep_stored_cloud_sync(incoming: &mut Settings, stored: &Settings) {
    incoming.cloud_sync = stored.cloud_sync;
    incoming.claude_access = stored.claude_access;
}

fn device_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
        });
    name.unwrap_or_else(|| "Pennel".into())
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

#[tauri::command]
pub async fn cloud_login_start(shared: State<'_, Arc<Shared>>) -> Result<LoginStartView> {
    let shared = shared.inner().clone();
    blocking(move || {
        shared
            .engine
            .cloud_login_start_with_locale(
                device_name(),
                platform().into(),
                Some(shared.interface_language().code().into()),
            )
            .map(Into::into)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn cloud_login_poll(
    device_code: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<LoginPollView> {
    let shared = shared.inner().clone();
    let poll = blocking(move || shared.engine.cloud_login_poll(device_code).map_err(err)).await?;
    if matches!(poll, CloudLoginPoll::Done { .. }) {
        poke(&app);
        crate::claude_access::poke(&app);
    }
    Ok(poll.into())
}

#[tauri::command]
pub async fn cloud_logout(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || {
        shared.engine.cloud_logout().map_err(err)?;
        Ok(())
    })
    .await?;
    let _ = app.emit("sync-changed", SyncReportView::from(current_report(&app)));
    settings_changed(&app);
    crate::claude_access::poke(&app);
    Ok(())
}

fn current_report(app: &AppHandle) -> SyncReport {
    app.state::<Arc<Shared>>().engine.cloud_sync_last()
}

#[tauri::command]
pub async fn cloud_account(shared: State<'_, Arc<Shared>>) -> Result<AccountView> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_account().map(Into::into).map_err(err)).await
}

#[tauri::command]
pub async fn cloud_devices(shared: State<'_, Arc<Shared>>) -> Result<Vec<DeviceView>> {
    let shared = shared.inner().clone();
    blocking(move || {
        shared
            .engine
            .cloud_devices()
            .map(|devices| devices.into_iter().map(Into::into).collect())
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn cloud_rename_device(
    id: String,
    name: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_rename_device(id, name).map_err(err)).await
}

#[tauri::command]
pub async fn cloud_revoke_device(id: String, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_revoke_device(id).map_err(err)).await
}

#[tauri::command]
pub async fn cloud_delete_account(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_delete_account().map_err(err)).await?;
    let _ = app.emit("sync-changed", SyncReportView::from(current_report(&app)));
    settings_changed(&app);
    crate::claude_access::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn cloud_sync_enable(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<String> {
    let shared = shared.inner().clone();
    let code = blocking(move || shared.engine.cloud_sync_enable().map_err(err)).await?;
    poke(&app);
    settings_changed(&app);
    Ok(code)
}

#[tauri::command]
pub async fn cloud_sync_setup(shared: State<'_, Arc<Shared>>) -> Result<CloudSyncSetup> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_setup().map_err(err)).await
}

#[tauri::command]
pub async fn cloud_sync_prepare(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Option<String>> {
    let shared = shared.inner().clone();
    let code = blocking(move || shared.engine.cloud_sync_prepare().map_err(err)).await?;
    settings_changed(&app);
    Ok(code)
}

#[tauri::command]
pub async fn cloud_sync_activate(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_activate().map_err(err)).await?;
    settings_changed(&app);
    poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn cloud_recovery_code_context(
    code: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<String> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_recovery_code_context(code).map_err(err)).await
}

#[tauri::command]
pub async fn cloud_recovery_confirm(
    code: String,
    context: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || {
        shared
            .engine
            .cloud_recovery_confirm(code, context)
            .map_err(err)
    })
    .await?;
    settings_changed(&app);
    poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn cloud_sync_resume(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_resume().map_err(err)).await?;
    settings_changed(&app);
    poke(&app);
    Ok(())
}

fn link_role(role: &str) -> Result<CloudLinkRole> {
    match role {
        "new_device" => Ok(CloudLinkRole::NewDevice),
        "trusted_device" => Ok(CloudLinkRole::TrustedDevice),
        _ => Err(CloudError::other("Unknown device connection role.")),
    }
}

fn link_changed(app: &AppHandle, view: &CloudLinkView) {
    if matches!(&view.phase, pennel_core::CloudLinkPhase::Complete) {
        settings_changed(app);
        poke(app);
    }
}

#[tauri::command]
pub async fn cloud_link_availability(
    shared: State<'_, Arc<Shared>>,
) -> Result<CloudLinkAvailability> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_link_availability().map_err(err)).await
}

#[tauri::command]
pub async fn cloud_link_start(
    role: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<CloudLinkView> {
    let role = link_role(&role)?;
    let shared = shared.inner().clone();
    blocking(move || {
        shared
            .engine
            .cloud_link_start(role, device_name(), platform().into())
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn cloud_link_join(
    code: String,
    role: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<CloudLinkView> {
    let role = link_role(&role)?;
    let shared = shared.inner().clone();
    blocking(move || {
        shared
            .engine
            .cloud_link_join(code, role, device_name(), platform().into())
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn cloud_link_poll(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<CloudLinkView> {
    let shared = shared.inner().clone();
    let view = blocking(move || shared.engine.cloud_link_poll().map_err(err)).await?;
    link_changed(&app, &view);
    Ok(view)
}

#[tauri::command]
pub async fn cloud_link_view(shared: State<'_, Arc<Shared>>) -> Result<CloudLinkView> {
    let shared = shared.inner().clone();
    blocking(move || Ok(shared.engine.cloud_link_view())).await
}

#[tauri::command]
pub async fn cloud_link_approve(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<CloudLinkView> {
    let shared = shared.inner().clone();
    let view = blocking(move || shared.engine.cloud_link_approve().map_err(err)).await?;
    link_changed(&app, &view);
    Ok(view)
}

#[tauri::command]
pub async fn cloud_link_cancel(shared: State<'_, Arc<Shared>>) -> Result<CloudLinkView> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_link_cancel().map_err(err)).await
}

#[tauri::command]
pub async fn cloud_sync_unlock(
    code: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_unlock(code).map_err(err)).await?;
    poke(&app);
    settings_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn cloud_sync_new_recovery_code(shared: State<'_, Arc<Shared>>) -> Result<String> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_new_recovery_code().map_err(err)).await
}

#[tauri::command]
pub async fn cloud_sync_reset(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<String> {
    let shared = shared.inner().clone();
    let code = blocking(move || shared.engine.cloud_sync_reset().map_err(err)).await?;
    poke(&app);
    settings_changed(&app);
    Ok(code)
}

#[tauri::command]
pub async fn cloud_sync_disable(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_disable().map_err(err)).await?;
    let _ = app.emit("sync-changed", SyncReportView::from(current_report(&app)));
    settings_changed(&app);
    crate::claude_access::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn cloud_sync_now(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<SyncReportView> {
    let shared = shared.inner().clone();
    poke(&app);
    blocking(move || Ok(shared.engine.cloud_sync_last().into())).await
}

#[tauri::command]
pub async fn cloud_sync_last(shared: State<'_, Arc<Shared>>) -> Result<SyncReportView> {
    let shared = shared.inner().clone();
    blocking(move || Ok(shared.engine.cloud_sync_last().into())).await
}

#[tauri::command]
pub async fn cloud_sync_confirm_deletes(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.cloud_sync_confirm_deletes().map_err(err)).await?;
    poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn meeting_sync_states(shared: State<'_, Arc<Shared>>) -> Result<Vec<MeetingSyncView>> {
    let shared = shared.inner().clone();
    blocking(move || {
        Ok(states_when_signed_in(shared.engine.cloud_signed_in(), || {
            shared.engine.meeting_sync_states()
        })
        .into_iter()
        .map(Into::into)
        .collect())
    })
    .await
}

/// After logout, account deletion or a revoked device nothing syncs, so no meeting may claim to wait for it.
fn states_when_signed_in<T>(signed_in: bool, states: impl FnOnce() -> Vec<T>) -> Vec<T> {
    if signed_in { states() } else { Vec::new() }
}

fn settings_changed(app: &AppHandle) {
    let _ = app.emit("settings-changed", ());
}

#[tauri::command]
pub async fn cloud_folder_warning(shared: State<'_, Arc<Shared>>) -> Result<Option<String>> {
    let shared = shared.inner().clone();
    blocking(move || Ok(shared.engine.cloud_folder_warning().map(crate::ui::message))).await
}

#[tauri::command]
pub fn open_url(url: String, app: AppHandle) -> Result<()> {
    let parsed = tauri::Url::parse(&url).map_err(CloudError::other)?;
    if parsed.scheme() != "https" {
        return Err(CloudError::other("Only HTTPS links can be opened."));
    }
    LOG.info("link opened in the browser");
    app.opener()
        .open_url(parsed.to_string(), None::<&str>)
        .map_err(CloudError::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn report(status: SyncStatus) -> SyncReport {
        SyncReport {
            status,
            uploaded: 0,
            downloaded: 0,
            conflicts: 0,
            last_success_ms: None,
        }
    }

    #[test]
    fn only_the_mass_delete_error_sets_brake() {
        let brake = SyncReportView::from(report(SyncStatus::Error {
            message: pennel_core::MASS_DELETE.into(),
        }));
        assert!(brake.brake);
        let other = SyncReportView::from(report(SyncStatus::Error {
            message: "iets anders".into(),
        }));
        assert!(!other.brake);
        assert!(!SyncReportView::from(report(SyncStatus::Synced)).brake);
        let json = serde_json::to_value(&brake).unwrap();
        assert_eq!(json["brake"], true);
    }

    #[test]
    fn signed_out_meetings_have_no_sync_state() {
        assert_eq!(states_when_signed_in(true, || vec![1, 2]), vec![1, 2]);
        let mut asked = false;
        let none: Vec<i32> = states_when_signed_in(false, || {
            asked = true;
            vec![1]
        });
        assert!(none.is_empty());
        assert!(!asked);
    }

    #[test]
    fn passes_stop_once_signed_out_or_sync_off() {
        assert!(should_run(true, true));
        assert!(!should_run(true, false));
        assert!(!should_run(false, true));
    }

    #[test]
    fn poke_while_running_queues_one_rerun() {
        let runs = Arc::new(Mutex::new(0u32));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let counter = runs.clone();
        let runner = SyncRunner::with_runner(Duration::from_secs(3600), Vec::new, move |_| {
            *counter.lock().unwrap() += 1;
            let _ = entered_tx.send(());
            let _ = release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(1));
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..5 {
            runner.poke();
        }
        release_tx.send(()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(*runs.lock().unwrap(), 2);
        runner.stop();
    }

    #[test]
    fn stop_ends_thread_promptly() {
        let runner = SyncRunner::with_runner(Duration::from_secs(3600), Vec::new, |_| {});
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        runner.stop();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(runner.thread.lock().unwrap().is_none());
    }

    #[test]
    fn stop_does_not_wait_for_a_running_pass() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let runner = SyncRunner::with_runner(Duration::from_secs(3600), Vec::new, move |_| {
            let _ = entered_tx.send(());
            std::thread::sleep(Duration::from_millis(1500));
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let start = Instant::now();
        runner.stop();
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn active_recording_id_is_passed() {
        let seen = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let sink = seen.clone();
        let runner = SyncRunner::with_runner(
            Duration::from_secs(3600),
            || vec!["2026-10-25-standup".to_string()],
            move |ids| sink.lock().unwrap().push(ids),
        );
        wait_until(|| !seen.lock().unwrap().is_empty());
        assert_eq!(
            seen.lock().unwrap()[0],
            vec!["2026-10-25-standup".to_string()]
        );
        runner.stop();
    }

    #[test]
    fn interval_triggers_a_pass() {
        let runs = Arc::new(Mutex::new(0u32));
        let counter = runs.clone();
        let runner = SyncRunner::with_runner(Duration::from_millis(30), Vec::new, move |_| {
            *counter.lock().unwrap() += 1;
        });
        wait_until(|| *runs.lock().unwrap() >= 3);
        runner.stop();
    }

    #[test]
    fn frontend_settings_never_flip_cloud_sync() {
        let dir = tempfile::tempdir().unwrap();
        let mut stored = crate::state::Shared::testing(dir.path().to_path_buf()).settings();
        stored.cloud_sync = true;
        let mut incoming = stored.clone();
        incoming.cloud_sync = false;
        keep_stored_cloud_sync(&mut incoming, &stored);
        assert!(incoming.cloud_sync);
        stored.cloud_sync = false;
        incoming.cloud_sync = true;
        keep_stored_cloud_sync(&mut incoming, &stored);
        assert!(!incoming.cloud_sync);
        incoming.claude_access = true;
        keep_stored_cloud_sync(&mut incoming, &stored);
        assert!(!incoming.claude_access);
    }

    #[test]
    fn status_views_serialise_every_variant() {
        let all = [
            SyncStatus::Off,
            SyncStatus::SignedOut,
            SyncStatus::NeedsSetup,
            SyncStatus::NeedsRecoveryCode,
            SyncStatus::VaultReset,
            SyncStatus::Pending,
            SyncStatus::Synced,
            SyncStatus::Offline,
            SyncStatus::StorageFull {
                message: "vol".into(),
            },
            SyncStatus::Error {
                message: "mis".into(),
            },
        ];
        let kinds: Vec<String> = all
            .into_iter()
            .map(|status| {
                serde_json::to_value(SyncStatusView::from(status)).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(kinds[3], "needs_recovery_code");
        assert_eq!(kinds[5], "pending");
        assert_eq!(kinds.len(), 10);
    }

    #[test]
    fn a_panicking_pass_does_not_stop_the_runner() {
        let runs = Arc::new(Mutex::new(0u32));
        let counter = runs.clone();
        let runner = SyncRunner::with_runner(Duration::from_secs(3600), Vec::new, move |_| {
            let mut count = counter.lock().unwrap();
            *count += 1;
            if *count == 1 {
                drop(count);
                panic!("boom");
            }
        });
        wait_until(|| *runs.lock().unwrap() >= 1);
        std::thread::sleep(Duration::from_millis(50));
        assert!(!runner.inner.signals.lock().unwrap().running);
        runner.poke();
        wait_until(|| *runs.lock().unwrap() >= 2);
        runner.stop();
    }

    #[test]
    fn errors_map_to_stable_codes() {
        let map = |e: PennelError| CloudError::from(e);
        let locked = map(PennelError::VaultLocked);
        assert_eq!(locked.code, "vault_locked");
        assert_eq!(locked.message, "Enter your recovery code to turn on sync.");
        assert_eq!(map(PennelError::NotSignedIn).code, "not_signed_in");
        let cloud = map(PennelError::Cloud {
            code: "quota".into(),
            message: "Je tegoed is op.".into(),
        });
        assert_eq!(
            (cloud.code.as_str(), cloud.message.as_str()),
            ("quota", "Your credit has run out.")
        );
        assert_eq!(map(PennelError::NoSpeech).code, "error");
        let json = serde_json::to_value(&locked).unwrap();
        assert_eq!(json["code"], "vault_locked");
    }
}
