//! The webview's API. Blocking work (network, model loading) runs off the main thread.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use pennel_core::{
    ChatTurn, DetectedAgent, HistoryEntry, KnownVoice, LibrarySearch, LocalModel, MeetingAnswer,
    MeetingExport, MeetingInfo, MeetingTemplate, MeetingVoice, ModelChoice, ModelState,
    ProviderKind, ProviderTask, RemoteModel, Segment, Settings, SpeakerName, SpeakerRecognition,
    VoiceProfile,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::capabilities::{self, Capability, PlatformCapabilities};
use crate::diagnostics::{self, Log};
use crate::dictation::Msg;
use crate::meeting_runtime;
use crate::speakers;
use crate::state::Shared;

const LOG: Log = Log::new("app");

type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    localize_error_message(e.to_string())
}

pub(crate) fn localize_error_message(source: impl AsRef<str>) -> String {
    localize_error_message_for_language(source.as_ref(), crate::ui::language())
}

fn localize_error_message_for_language(
    source: &str,
    language: pennel_core::InterfaceLanguage,
) -> String {
    let mut remaining = source;
    let mut wrappers = Vec::new();
    // These are Pennel's own error envelopes. Captured values in a catalog message remain data.
    for _ in 0..3 {
        let Some((prefix, canonical)) = [
            ("storage error: ", "storage error: "),
            ("network error: ", "network error: "),
            ("invalid response: ", "invalid response: "),
            ("Saving failed: ", "Saving failed: "),
            ("Save failed: ", "Saving failed: "),
            ("Bewaren mislukt: ", "Saving failed: "),
        ]
        .into_iter()
        .find(|(prefix, _)| remaining.starts_with(prefix)) else {
            break;
        };
        wrappers.push(canonical);
        remaining = &remaining[prefix.len()..];
    }
    let mut result = pennel_core::localize_ui_text(language, remaining);
    for prefix in wrappers.into_iter().rev() {
        result = pennel_core::localize_ui_text(language, &format!("{prefix}{result}"));
    }
    result
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(err)?
}

/// From this duration a command the interface waits on is worth a line in the log.
const SLOW: std::time::Duration = std::time::Duration::from_millis(100);

/// Off the main thread, so the webview keeps painting, with its duration logged once noticeable.
/// Synchronous Tauri commands run on the main thread and freeze every window while they work.
async fn off_main<T: Send + 'static>(
    name: &'static str,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    blocking(move || {
        let started = std::time::Instant::now();
        let result = work();
        let elapsed = started.elapsed();
        if elapsed >= SLOW {
            LOG.info(format!("{name} took {} ms", elapsed.as_millis()));
        }
        result
    })
    .await
}

#[derive(Serialize)]
pub struct KindInfo {
    kind: ProviderKind,
    label: String,
    requires_key: bool,
    default_base_url: String,
    stt: bool,
    llm: bool,
    default_stt_model: String,
    default_llm_model: String,
    is_cli_agent: bool,
    availability: Capability,
}

const KINDS: [(ProviderKind, &str); 17] = [
    (ProviderKind::OpenAi, "OpenAI"),
    (ProviderKind::OpenRouter, "OpenRouter"),
    (ProviderKind::Groq, "Groq"),
    (ProviderKind::Mistral, "Mistral"),
    (ProviderKind::Ollama, "Ollama (local)"),
    (ProviderKind::LmStudio, "LM Studio (local)"),
    (ProviderKind::Custom, "Custom server"),
    (ProviderKind::Local, "Local on this computer"),
    (ProviderKind::AppleSpeech, "Apple dictation"),
    (ProviderKind::AppleIntelligence, "Apple Intelligence"),
    (ProviderKind::ClaudeCode, "Claude Code"),
    (ProviderKind::Codex, "Codex"),
    (ProviderKind::OpenCode, "OpenCode"),
    (ProviderKind::Copilot, "GitHub Copilot"),
    (ProviderKind::CursorAgent, "Cursor"),
    (ProviderKind::GeminiCli, "Gemini CLI"),
    (ProviderKind::PennelCloud, "Pennel Cloud"),
];

fn provider_kinds(
    apple_speech: &Capability,
    apple_intelligence: &Capability,
    local_models: &Capability,
) -> Vec<KindInfo> {
    KINDS
        .iter()
        .map(|(kind, label)| KindInfo {
            kind: *kind,
            label: crate::ui::message(*label),
            requires_key: pennel_core::requires_key(*kind),
            default_base_url: pennel_core::default_base_url(*kind),
            stt: pennel_core::supports(*kind, ProviderTask::DictationStt),
            llm: pennel_core::supports(*kind, ProviderTask::Llm),
            default_stt_model: pennel_core::default_model(*kind, ProviderTask::DictationStt),
            default_llm_model: pennel_core::default_model(*kind, ProviderTask::Llm),
            is_cli_agent: kind.is_cli_agent(),
            availability: match *kind {
                ProviderKind::AppleSpeech => apple_speech.clone(),
                ProviderKind::AppleIntelligence => apple_intelligence.clone(),
                ProviderKind::Local => local_models.clone(),
                _ => Capability::available(),
            },
        })
        .collect()
}

#[derive(Serialize)]
pub struct AppInfo {
    interface_language: pennel_core::InterfaceLanguage,
    system_interface_language: pennel_core::InterfaceLanguage,
    settings: Settings,
    keys: HashMap<String, bool>,
    provider_revisions: HashMap<String, u64>,
    cloud_account_epoch: String,
    cloud_signed_in: bool,
    cleanup_verification: pennel_core::CleanupVerificationAvailability,
    active_model_downloads: Vec<String>,
    unsaved_history_count: u32,
    kinds: Vec<KindInfo>,
    platform: &'static str,
    version: String,
    preview: bool,
    migration_warning: Option<String>,
    hotkey_error: Option<String>,
    log_path: Option<String>,
    data_dir: String,
    capabilities: PlatformCapabilities,
    diarizer_model_id: String,
    recording_app_ids: Vec<String>,
}

pub(crate) fn info(shared: &Shared) -> Result<AppInfo> {
    static FIRST_INFO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    let trace = FIRST_INFO.swap(false, std::sync::atomic::Ordering::Relaxed);
    if trace {
        LOG.info("initial app information: reading settings");
    }
    let settings = shared.settings();
    if trace {
        LOG.info("initial app information: checking credential store");
    }
    let keys = settings
        .providers
        .iter()
        .map(|p| {
            shared
                .secrets
                .has(&p.id)
                .map(|present| (p.id.clone(), present))
        })
        .collect::<Result<HashMap<_, _>>>()?;
    if trace {
        LOG.info("initial app information: checking capabilities");
    }
    let hotkey_error = shared.hotkey_error.lock().unwrap().clone();
    let capabilities = capabilities::snapshot(hotkey_error.as_deref()).localized();
    if trace {
        LOG.info("initial app information: assembling state");
    }
    Ok(AppInfo {
        interface_language: settings
            .ui_language
            .resolve(&[shared.system_interface_language.code().to_owned()]),
        system_interface_language: shared.system_interface_language,
        keys,
        provider_revisions: settings
            .providers
            .iter()
            .map(|provider| (provider.id.clone(), shared.secrets.revision()))
            .collect(),
        cloud_account_epoch: crate::cloud::account_epoch(shared),
        cloud_signed_in: shared.secrets.has("pennel-cloud-refresh")?,
        cleanup_verification: shared.engine.cleanup_verification_availability(),
        active_model_downloads: shared.engine.active_model_downloads(),
        unsaved_history_count: shared.engine.unsaved_history_count(),
        settings,
        kinds: provider_kinds(
            &capabilities.apple_speech,
            &capabilities.apple_intelligence,
            &capabilities.local_models,
        ),
        platform: std::env::consts::OS,
        version: shared.version.clone(),
        preview: shared.preview,
        migration_warning: shared.migration_warning.clone().map(crate::ui::message),
        hotkey_error: hotkey_error.map(crate::ui::message),
        log_path: diagnostics::file_path().map(|p| p.to_string_lossy().into_owned()),
        data_dir: shared.data_dir.to_string_lossy().into_owned(),
        capabilities,
        diarizer_model_id: shared.engine.diarizer_model_id(),
        recording_app_ids: pennel_core::known_call_apps()
            .into_iter()
            .chain(pennel_core::browser_apps())
            .collect(),
    })
}

#[tauri::command]
pub async fn app_info(shared: State<'_, Arc<Shared>>) -> Result<AppInfo> {
    let shared = shared.inner().clone();
    off_main("app_info", move || info(&shared)).await
}

#[tauri::command]
pub async fn save_settings(
    settings: Settings,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<AppInfo> {
    let shared = shared.inner().clone();
    off_main("save_settings", move || {
        apply_settings(settings, &app, &shared)
    })
    .await
}

fn apply_settings(mut settings: Settings, app: &AppHandle, shared: &Shared) -> Result<AppInfo> {
    let _inference_mutation = crate::inference::mutation(&shared.engine);
    let old_settings = shared.settings();
    crate::cloud::keep_stored_cloud_sync(&mut settings, &old_settings);
    if shared.preview && old_settings.meetings_dir != settings.meetings_dir {
        return Err(
            "This preview uses its own meeting folder to protect your existing data.".into(),
        );
    }
    let recording = shared.recording.lock().unwrap();
    if old_settings.meetings_dir != settings.meetings_dir
        && (recording.is_busy()
            || shared
                .library_jobs
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0)
    {
        return Err(
            "The meeting folder can only be changed after recording and processing have finished."
                .into(),
        );
    }
    let previous = old_settings.hotkey;
    shared.engine.update_settings(settings).map_err(err)?;
    drop(recording);
    let hotkey = shared.settings().hotkey;
    if hotkey != previous {
        let listener = shared.hotkey.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener.set_trigger(hotkey);
        }
        let _ = shared.dictation.send(Msg::TriggerChanged);
        if let Some(hint) = app.try_state::<crate::TrayHint>() {
            let _ = hint.0.set_text(crate::hotkey_hint(shared));
        }
        LOG.info(format!("shortcut changed to {hotkey:?}"));
    }
    crate::record_shortcut::refresh(app, shared);
    crate::apply_appearance(app);
    crate::ui::apply(app, shared).map_err(err)?;
    if old_settings.dictation_pad_auto_paste && !shared.settings().dictation_pad_auto_paste {
        crate::owned_dictation::invalidate_external(app);
    }
    let _ = app.emit("settings-changed", ());
    drop(_inference_mutation);
    refresh_cleanup_verification(app);
    info(shared)
}

fn refresh_cleanup_verification(app: &AppHandle) {
    crate::pill::refresh_verification(app);
    crate::owned_dictation::refresh_verification(app);
}

#[tauri::command]
pub async fn add_provider(
    kind: ProviderKind,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<AppInfo> {
    let shared = shared.inner().clone();
    blocking(move || {
        let executable = if kind.is_cli_agent() {
            pennel_core::detect_cli_agents()
                .into_iter()
                .find(|agent| agent.kind == kind)
                .map(|agent| agent.path)
        } else {
            None
        };
        let _inference_mutation = crate::inference::mutation(&shared.engine);
        let mut settings = shared.settings();
        let mut account = pennel_core::new_account(kind, settings.providers.clone());
        if let Some(path) = executable {
            account.base_url = path;
        }
        settings.providers.push(account);
        shared.engine.update_settings(settings).map_err(err)?;
        drop(_inference_mutation);
        refresh_cleanup_verification(&app);
        info(&shared)
    })
    .await
}

#[tauri::command]
pub async fn remove_provider(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<AppInfo> {
    let shared = shared.inner().clone();
    off_main("remove_provider", move || {
        let _inference_mutation = crate::inference::mutation(&shared.engine);
        let mut settings = shared.settings();
        settings.providers.retain(|p| p.id != id);
        shared.engine.update_settings(settings).map_err(err)?;
        let _ = shared.secrets.set(&id, "");
        drop(_inference_mutation);
        refresh_cleanup_verification(&app);
        info(&shared)
    })
    .await
}

#[tauri::command]
pub async fn set_key(
    account: String,
    key: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<AppInfo> {
    let shared = shared.inner().clone();
    off_main("set_key", move || {
        let _inference_mutation = crate::inference::mutation(&shared.engine);
        shared.secrets.set(&account, &key)?;
        let _ = app.emit("settings-changed", ());
        drop(_inference_mutation);
        refresh_cleanup_verification(&app);
        info(&shared)
    })
    .await
}

#[tauri::command]
pub async fn provider_models(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Vec<RemoteModel>> {
    let shared = shared.inner().clone();
    blocking(move || {
        let settings = shared.settings();
        let account = settings
            .providers
            .iter()
            .find(|p| p.id == id)
            .ok_or("unknown provider")?;
        if account.kind.is_cli_agent()
            || matches!(
                account.kind,
                ProviderKind::AppleSpeech
                    | ProviderKind::AppleIntelligence
                    | ProviderKind::Local
                    | ProviderKind::PennelCloud
            )
        {
            return Ok(Vec::new());
        }
        pennel_core::provider_models(account.base_url.clone(), shared.secrets.get(&id)).map_err(err)
    })
    .await
}

#[derive(Serialize)]
pub struct LocalModelView {
    #[serde(flatten)]
    model: LocalModel,
    state: ModelState,
}

#[tauri::command]
pub fn local_models(
    language: Option<String>,
    supported_only: bool,
    shared: State<'_, Arc<Shared>>,
) -> Vec<LocalModelView> {
    shared
        .engine
        .local_models(language, supported_only)
        .into_iter()
        .map(|mut model| LocalModelView {
            state: shared.engine.model_state(model.id.clone()),
            model: {
                model.description = crate::ui::message(model.description);
                model
            },
        })
        .collect()
}

#[tauri::command]
pub fn model_state(id: String, shared: State<'_, Arc<Shared>>) -> ModelState {
    if validate_meeting_id(&id).is_err() {
        return ModelState::NotDownloaded;
    }
    shared.engine.model_state(id)
}

struct DownloadEvents {
    app: AppHandle,
    id: String,
    last: std::sync::Mutex<std::time::Instant>,
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    id: String,
    downloaded: u64,
    total: u64,
}

#[derive(Serialize, Clone)]
struct DownloadDone {
    id: String,
    error: Option<String>,
}

impl pennel_core::DownloadListener for DownloadEvents {
    fn on_progress(&self, downloaded: u64, total: u64) {
        let mut last = self.last.lock().unwrap();
        if last.elapsed().as_millis() < 200 && downloaded < total {
            return;
        }
        *last = std::time::Instant::now();
        let _ = self.app.emit(
            "download-progress",
            DownloadProgress {
                id: self.id.clone(),
                downloaded,
                total,
            },
        );
    }
}

#[tauri::command]
pub fn download_model(id: String, app: AppHandle, shared: State<'_, Arc<Shared>>) {
    let shared = shared.inner().clone();
    std::thread::spawn(move || {
        let listener = Arc::new(DownloadEvents {
            app: app.clone(),
            id: id.clone(),
            last: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(1),
            ),
        });
        let result = shared.engine.download_model(id.clone(), listener);
        if let Err(e) = &result {
            LOG.error(format!("download {id}: {e}"));
        } else {
            LOG.info(format!("download {id} complete"));
        }
        let _ = app.emit(
            "download-done",
            DownloadDone {
                id,
                error: result.err().map(err),
            },
        );
    });
}

#[tauri::command]
pub fn cancel_download(id: String, shared: State<'_, Arc<Shared>>) {
    shared.engine.cancel_download(id);
}

#[tauri::command]
pub async fn join_model_download(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<bool> {
    let shared = shared.inner().clone();
    blocking(move || {
        let listener = Arc::new(DownloadEvents {
            app: app.clone(),
            id: id.clone(),
            last: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(1),
            ),
        });
        let result = shared.engine.join_model_download(id.clone(), listener);
        let _ = app.emit(
            "download-done",
            DownloadDone {
                id,
                error: result.as_ref().err().map(err),
            },
        );
        result.map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn delete_model(id: String, shared: State<'_, Arc<Shared>>) -> Result<AppInfo> {
    let shared = shared.inner().clone();
    off_main("delete_model", move || {
        shared.engine.delete_model(id).map_err(err)?;
        info(&shared)
    })
    .await
}

#[tauri::command]
pub async fn history(limit: u32, shared: State<'_, Arc<Shared>>) -> Result<Vec<HistoryEntry>> {
    let shared = shared.inner().clone();
    off_main("history", move || {
        shared
            .engine
            .history(limit)
            .map(|mut entries| {
                for entry in &mut entries {
                    entry.error = entry.error.take().map(localize_error_message);
                }
                entries
            })
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn save_history(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || shared.engine.save_history(id).map_err(err)).await?;
    let _ = app.emit("history-changed", ());
    Ok(())
}

#[tauri::command]
pub fn copy_history(id: String, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let entry = shared.engine.history_entry(id).map_err(err)?;
    if entry.text.is_empty() {
        return Err("This dictation has no text to copy yet.".into());
    }
    crate::platform_clipboard::copy(&entry.text, None)
}

#[tauri::command]
pub async fn apply_task_model_choice(
    slot: String,
    choice: ModelChoice,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<AppInfo> {
    let task = model_task(&slot)?;
    let shared = shared.inner().clone();
    off_main("apply_task_model_choice", move || {
        let mutation = crate::inference::mutation(&shared.engine);
        // Resolve fallible credential metadata before committing the assignment.
        let mut next = info(&shared)?;
        shared
            .engine
            .apply_task_model_choice(task, choice)
            .map_err(err)?;
        next.settings = shared.settings();
        let _ = app.emit("settings-changed", ());
        drop(mutation);
        refresh_cleanup_verification(&app);
        next.cleanup_verification = shared.engine.cleanup_verification_availability();
        Ok(next)
    })
    .await
}

#[tauri::command]
pub async fn test_provider_task(
    slot: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<pennel_core::ProviderTaskTestResult> {
    let task = model_task(&slot)?;
    let shared = shared.inner().clone();
    blocking(move || {
        pennel_core::test_provider_task(shared.engine.clone(), task, shared.keys())
            .map(|mut result| {
                result.message = crate::ui::message(result.message);
                result
            })
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn test_provider_task_choice(
    slot: String,
    choice: ModelChoice,
    shared: State<'_, Arc<Shared>>,
) -> Result<pennel_core::ProviderTaskTestResult> {
    let task = model_task(&slot)?;
    let shared = shared.inner().clone();
    blocking(move || {
        pennel_core::test_provider_task_choice(shared.engine.clone(), task, choice, shared.keys())
            .map(|mut result| {
                result.message = crate::ui::message(result.message);
                result
            })
            .map_err(err)
    })
    .await
}

fn model_task(slot: &str) -> Result<ProviderTask> {
    match slot {
        "dictation_stt" => Ok(ProviderTask::DictationStt),
        "meeting_stt" => Ok(ProviderTask::MeetingStt),
        "llm" => Ok(ProviderTask::Llm),
        _ => Err("Unknown model task.".into()),
    }
}

#[tauri::command]
pub async fn retry_history(id: String, shared: State<'_, Arc<Shared>>) -> Result<String> {
    let shared = shared.inner().clone();
    blocking(move || {
        let keys = shared.keys();
        shared.engine.retry(id, keys).map(|r| r.text).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn delete_history(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("delete_history", move || {
        shared.engine.delete_history(id).map_err(err)
    })
    .await?;
    let _ = app.emit("history-changed", ());
    Ok(())
}

#[tauri::command]
pub fn open_log(app: AppHandle) -> Result<()> {
    open_log_file(&app)
}

pub fn open_log_file(app: &AppHandle) -> Result<()> {
    let path = diagnostics::file_path().ok_or("no log file")?;
    if !path.exists() {
        Log::new("app").info("log file created");
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

#[tauri::command]
pub fn open_path(path: String, app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let path = PathBuf::from(path).canonicalize().map_err(err)?;
    let root = PathBuf::from(shared.engine.meeting_dir(String::new()))
        .canonicalize()
        .map_err(err)?;
    if path != root && path.parent() != Some(root.as_path()) {
        return Err("Only the configured meeting folder can be opened.".into());
    }
    if !path.is_dir() {
        return Err("No meeting folder found.".into());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

fn validate_meeting_id(id: &str) -> Result<()> {
    let mut components = Path::new(id).components();
    if id.is_empty()
        || id
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':' | '\0') || character.is_control())
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("Invalid meeting identifier.".into());
    }
    Ok(())
}

pub(crate) fn checked_meeting_dir(engine: &pennel_core::PennelEngine, id: &str) -> Result<PathBuf> {
    validate_meeting_id(id)?;
    let root = PathBuf::from(engine.meeting_dir(String::new()))
        .canonicalize()
        .map_err(err)?;
    let directory = root.join(id).canonicalize().map_err(err)?;
    if directory.parent() != Some(root.as_path()) || !directory.is_dir() {
        return Err("Meeting not found in the configured meeting folder.".into());
    }
    if engine.meeting_info(id.to_owned()).is_none() {
        return Err("Meeting not found.".into());
    }
    Ok(directory)
}

fn require_inactive_meeting(shared: &Shared, id: &str) -> Result<()> {
    let lifecycle = shared.recording.lock().unwrap();
    let active_id = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .map(|context| context.session.id());
    if lifecycle.is_busy() && active_id.as_deref() == Some(id) {
        return Err("Wait until the recording has finished.".into());
    }
    Ok(())
}

#[derive(Serialize)]
pub struct MeetingRuntimeStatus {
    recognition: Option<pennel_core::MeetingRecognition>,
    recording: bool,
    busy: bool,
    phase: pennel_core::RecordingPhase,
    error: Option<String>,
    session_id: Option<String>,
    id: Option<String>,
    title: Option<String>,
    elapsed_secs: u64,
    system_audio: bool,
    heard_system_audio: bool,
    echo_cancellation_active: bool,
    audio_warning: Option<String>,
    persistence_error: Option<String>,
    transcription_warning: Option<String>,
    terminal_error: Option<String>,
    can_retry_persistence: bool,
    consent_copied: bool,
    pending: u32,
    transcript_revision: u64,
}

#[derive(Serialize)]
pub struct MeetingStatus {
    #[serde(flatten)]
    runtime: MeetingRuntimeStatus,
    transcript: Vec<Segment>,
    notes: String,
}

#[tauri::command]
pub async fn meeting_status(shared: State<'_, Arc<Shared>>) -> Result<MeetingStatus> {
    let shared = shared.inner().clone();
    off_main("meeting_status", move || Ok(status(&shared))).await
}

#[tauri::command]
pub async fn meeting_runtime_status(
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingRuntimeStatus> {
    let shared = shared.inner().clone();
    off_main("meeting_runtime_status", move || {
        Ok(runtime_status(&shared).0)
    })
    .await
}

fn status(shared: &Shared) -> MeetingStatus {
    let (mut runtime, session) = runtime_status(shared);
    let (transcript, notes) = if let Some(session) = session {
        let (revision, transcript) = session.transcript_snapshot();
        runtime.transcript_revision = revision;
        (transcript, shared.engine.meeting_notes(session.id()))
    } else {
        (Vec::new(), String::new())
    };
    MeetingStatus {
        runtime,
        transcript,
        notes,
    }
}

fn runtime_status(
    shared: &Shared,
) -> (
    MeetingRuntimeStatus,
    Option<Arc<pennel_core::MeetingSession>>,
) {
    let (state, details) = {
        let lifecycle = shared.recording.lock().unwrap();
        let state = lifecycle.snapshot();
        let details = shared
            .meeting_context
            .lock()
            .unwrap()
            .as_ref()
            .filter(|context| state.session_id.as_deref() == Some(context.token.as_str()))
            .map(|context| {
                (
                    context.session.clone(),
                    context.title.clone(),
                    context
                        .duration_secs
                        .unwrap_or_else(|| context.started.elapsed().as_secs()),
                    context.system_audio,
                    context.heard_system_audio,
                    context.audio_warning.clone(),
                    context.consent_copied,
                )
            });
        (state, details)
    };
    let recording = state.phase == pennel_core::RecordingPhase::Recording;
    let busy = !matches!(
        state.phase,
        pennel_core::RecordingPhase::Idle | pennel_core::RecordingPhase::Error
    );
    let live_system = shared
        .meeting
        .lock()
        .unwrap()
        .as_ref()
        .filter(|meeting| {
            details
                .as_ref()
                .is_some_and(|(session, ..)| session.id() == meeting.session.id())
        })
        .map(|meeting| {
            #[cfg(any(target_os = "macos", windows))]
            let echo = meeting.echo_cancellation_active();
            #[cfg(not(any(target_os = "macos", windows)))]
            let echo = false;
            (
                meeting.has_system_audio(),
                meeting.heard_system_audio(),
                meeting.audio_warning(),
                meeting.terminal_error(),
                echo,
            )
        });
    let mut result = MeetingRuntimeStatus {
        recognition: None,
        recording,
        busy,
        phase: state.phase,
        error: state.error,
        session_id: state.session_id,
        id: None,
        title: None,
        elapsed_secs: 0,
        system_audio: false,
        heard_system_audio: false,
        echo_cancellation_active: false,
        audio_warning: None,
        persistence_error: None,
        transcription_warning: None,
        terminal_error: None,
        can_retry_persistence: false,
        consent_copied: false,
        pending: 0,
        transcript_revision: 0,
    };
    let mut active_session = None;
    if let Some((
        session,
        title,
        elapsed_secs,
        system_audio,
        heard_system_audio,
        audio_warning,
        consent_copied,
    )) = details
    {
        let id = session.id();
        result.id = Some(id);
        result.title = Some(title);
        result.elapsed_secs = elapsed_secs;
        result.consent_copied = consent_copied;
        result.system_audio = system_audio;
        result.heard_system_audio = heard_system_audio;
        result.audio_warning = audio_warning;
        if let Some((system_audio, heard, warning, terminal, echo)) = live_system {
            result.system_audio = system_audio;
            result.heard_system_audio = heard;
            result.audio_warning = warning.map(crate::ui::message);
            result.terminal_error = terminal;
            result.echo_cancellation_active = echo;
            if recording && !echo && shared.settings().echo_cancellation {
                let explanation =
                    "Echo cancellation is not active. Use headphones to avoid duplicate audio.";
                let explanation = crate::ui::message(explanation);
                result.audio_warning = Some(match result.audio_warning {
                    Some(warning) => format!("{warning} {explanation}"),
                    None => explanation,
                });
            }
        }
        result.pending = session.pending();
        result.recognition = Some(session.recognition_config());
        result.transcript_revision = session.transcript_revision();
        let health = session.health();
        result.persistence_error = health.persistence_error;
        result.transcription_warning = health.transcription_warning;
        result.terminal_error = health.terminal_error.or(result.terminal_error);
        result.can_retry_persistence = health.can_retry_persistence;
        active_session = Some(session);
    }
    for message in [
        &mut result.error,
        &mut result.audio_warning,
        &mut result.persistence_error,
        &mut result.transcription_warning,
        &mut result.terminal_error,
    ] {
        *message = message.take().map(localize_error_message);
    }
    (result, active_session)
}

#[tauri::command]
pub async fn start_meeting(title: String, shared: State<'_, Arc<Shared>>) -> Result<String> {
    let shared = shared.inner().clone();
    blocking(move || meeting_runtime::start(&shared, title, None, Vec::new())).await
}

#[tauri::command]
pub async fn stop_meeting(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("stop_meeting", move || {
        meeting_runtime::stop(app, shared, None)
    })
    .await
}

#[tauri::command]
pub async fn retry_meeting_persistence(
    session_id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    blocking(move || meeting_runtime::retry_persistence(&app, &shared, &session_id)).await
}

#[tauri::command]
pub async fn set_meeting_notes(
    text: String,
    session_id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("set_meeting_notes", move || {
        save_live_notes(&shared, text, &session_id)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

fn save_live_notes(shared: &Shared, text: String, session_id: &str) -> Result<()> {
    // Keep the session guard through the durable write, as before moving it off the UI thread.
    let lifecycle = shared.recording.lock().unwrap();
    let state = lifecycle.snapshot();
    if !matches!(
        state.phase,
        pennel_core::RecordingPhase::Recording
            | pennel_core::RecordingPhase::Stopping
            | pennel_core::RecordingPhase::Error
    ) || state.session_id.as_deref() != Some(session_id)
    {
        return Err("This recording session is no longer available for editing notes.".into());
    }
    let session = shared
        .meeting_context
        .lock()
        .unwrap()
        .as_ref()
        .filter(|context| context.token == session_id)
        .map(|context| context.session.clone())
        .ok_or("no meeting in progress")?;
    session.set_notes(text).map_err(err)
}

#[tauri::command]
pub async fn set_meeting_title(
    title: String,
    session_id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("set_meeting_title", move || {
        meeting_runtime::set_title(&shared, title, &session_id)
    })
    .await
}

#[tauri::command]
pub async fn meetings(query: String, shared: State<'_, Arc<Shared>>) -> Result<Vec<MeetingInfo>> {
    let shared = shared.inner().clone();
    off_main("meetings", move || {
        Ok(if query.trim().is_empty() {
            shared.engine.meetings()
        } else {
            shared.engine.search_meetings(query)
        })
    })
    .await
}

#[derive(Serialize)]
pub struct MeetingDetail {
    info: Option<MeetingInfo>,
    notes: String,
    summary: Option<String>,
    transcript: Vec<Segment>,
    speakers: Vec<SpeakerName>,
    dir: String,
    attendees: Vec<String>,
    audio_tracks: Vec<AudioTrack>,
    summary_outdated: bool,
    retranscription_plan: pennel_core::MeetingRetranscriptionPlan,
    retranscription_status: Option<pennel_core::MeetingRetranscriptionStatus>,
    recording_metadata: Option<pennel_core::MeetingRecordingMetadata>,
}

#[derive(Debug, Serialize)]
pub struct AudioTrack {
    speaker: pennel_core::Speaker,
    available: bool,
}

fn audio_tracks(dir: &Path) -> Vec<AudioTrack> {
    let tracks = pennel_core::resolve_meeting_audio_tracks(dir);
    [pennel_core::Speaker::Me, pennel_core::Speaker::Others]
        .into_iter()
        .map(|speaker| AudioTrack {
            speaker,
            available: tracks
                .iter()
                .any(|track| track.speaker == speaker && track.available),
        })
        .collect()
}

#[tauri::command]
pub async fn meeting_detail(id: String, shared: State<'_, Arc<Shared>>) -> Result<MeetingDetail> {
    let shared = shared.inner().clone();
    off_main("meeting_detail", move || {
        let engine = &shared.engine;
        let directory = checked_meeting_dir(engine, &id)?;
        Ok(MeetingDetail {
            info: engine.meeting_info(id.clone()),
            notes: engine.meeting_notes(id.clone()),
            summary: engine.meeting_markdown(id.clone()),
            transcript: engine.meeting_transcript(id.clone()),
            speakers: engine.meeting_speakers(id.clone()),
            attendees: engine.meeting_attendees(id.clone()),
            dir: directory.to_string_lossy().into_owned(),
            audio_tracks: audio_tracks(&directory),
            summary_outdated: engine.meeting_summary_outdated(&id),
            retranscription_plan: {
                let mut plan = engine.meeting_retranscription_plan(&id);
                plan.reason = plan.reason.map(localize_error_message);
                plan
            },
            retranscription_status: engine
                .meeting_retranscription_status(&id)
                .map(localize_retranscription_status),
            recording_metadata: engine.meeting_recording_metadata(id.clone()).ok().flatten(),
        })
    })
    .await
}

#[tauri::command]
pub async fn meeting_templates(shared: State<'_, Arc<Shared>>) -> Result<Vec<MeetingTemplate>> {
    let shared = shared.inner().clone();
    off_main("meeting_templates", move || {
        Ok(shared.engine.meeting_templates())
    })
    .await
}

#[tauri::command]
pub async fn summarize_meeting(
    id: String,
    template_id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<String> {
    let shared = shared.inner().clone();
    let summary = blocking(move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        let keys = shared.keys();
        shared
            .engine
            .summarize_meeting(id, template_id, keys)
            .map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(summary)
}

#[tauri::command]
pub async fn retranscribe_meeting(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<u32> {
    let shared = shared.inner().clone();
    let count = blocking(move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        let keys = shared.keys();
        shared.engine.retranscribe_failed(id, keys).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(count)
}

fn localize_retranscription_status(
    mut status: pennel_core::MeetingRetranscriptionStatus,
) -> pennel_core::MeetingRetranscriptionStatus {
    status.error = status.error.map(localize_error_message);
    status.warning = status.warning.map(localize_error_message);
    status
}

#[tauri::command]
pub async fn retranscribe_meeting_audio(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<u32> {
    let shared = shared.inner().clone();
    let events = app.clone();
    let count=off_main("retranscribe_meeting_audio",move || {
        let _library_job=meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine,&id)?;
        require_inactive_meeting(&shared,&id)?;
        shared.engine.retranscribe_meeting_audio(&id,|path| {
            crate::platform_audio::decode_audio(path).map_err(|message|pennel_core::PennelError::Storage{message})
        },|status| {
            let _=events.emit("meeting-retranscription-changed",serde_json::json!({"id":id,"status":localize_retranscription_status(status)}));
        }).map_err(err)
    }).await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(count)
}

#[tauri::command]
pub async fn rename_meeting(
    id: String,
    title: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<String> {
    let shared = shared.inner().clone();
    let renamed = off_main("rename_meeting", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        shared.engine.rename_meeting(id, title).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(renamed)
}

#[tauri::command]
pub async fn delete_meeting(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("delete_meeting", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        shared.engine.delete_meeting(id).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

fn is_conflict_copy_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && !name.contains(['/', '\\', ':', '\0'])
        && Path::new(name)
            .file_stem()
            .is_some_and(|stem| stem.to_string_lossy().contains(" (conflict "))
}

fn list_conflict_copies(directory: &Path) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(directory)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_conflict_copy_name(name))
        .collect();
    names.sort();
    Ok(names)
}

fn remove_conflict_copy(directory: &Path, name: &str) -> std::result::Result<(), String> {
    if !is_conflict_copy_name(name) {
        return Err("Only a conflict copy can be deleted here.".into());
    }
    let file = directory.join(name);
    let regular = std::fs::symlink_metadata(&file).is_ok_and(|meta| meta.file_type().is_file());
    if !regular {
        return Err("Conflict copy not found.".into());
    }
    std::fs::remove_file(file).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn meeting_conflict_files(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Vec<String>> {
    let shared = shared.inner().clone();
    off_main("meeting_conflict_files", move || {
        let directory = checked_meeting_dir(&shared.engine, &id)?;
        list_conflict_copies(&directory).map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn delete_conflict_copy(
    id: String,
    name: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("delete_conflict_copy", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        let directory = checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        remove_conflict_copy(&directory, &name)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn search_library(
    query: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<LibrarySearch> {
    let shared = shared.inner().clone();
    blocking(move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        Ok(shared.engine.search_library(query))
    })
    .await
}

#[tauri::command]
pub async fn ask_meetings(
    question: String,
    history: Vec<ChatTurn>,
    meeting_id: Option<String>,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingAnswer> {
    let shared = shared.inner().clone();
    blocking(move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        if question.trim().is_empty() {
            return Err("Type a question first.".into());
        }
        if let Some(id) = &meeting_id {
            checked_meeting_dir(&shared.engine, id)?;
        }
        shared
            .engine
            .ask_meetings(question, history, meeting_id, shared.keys())
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn template_source(id: String, shared: State<'_, Arc<Shared>>) -> Result<Option<String>> {
    let shared = shared.inner().clone();
    off_main("template_source", move || {
        Ok(shared.engine.template_source(id))
    })
    .await
}

#[tauri::command]
pub async fn save_template(
    id: Option<String>,
    name: String,
    body: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<String> {
    if name.trim().is_empty() {
        return Err("Give the template a name.".into());
    }
    let shared = shared.inner().clone();
    let saved = off_main("save_template", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        shared.engine.save_template(id, name, body).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(saved)
}

#[tauri::command]
pub async fn delete_template(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("delete_template", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        shared.engine.delete_template(id).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn rename_speaker(
    id: String,
    voice: u32,
    name: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("rename_speaker", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        shared.engine.rename_speaker(id, voice, name).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn diarize_meeting(
    id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<SpeakerRecognition> {
    let shared = shared.inner().clone();
    let recognition = blocking(move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        let dir = checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        speakers::recognize(&shared, &id, &dir)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(recognition)
}

#[derive(Serialize)]
pub struct SpeakerSetup {
    local_available: bool,
    ready: bool,
    signed_in: bool,
    model_ids: Vec<String>,
    legacy_model: bool,
    profile: Option<VoiceProfile>,
    known: Vec<KnownVoice>,
    enrollment_text: String,
}

#[tauri::command]
pub async fn speaker_setup(shared: State<'_, Arc<Shared>>) -> Result<SpeakerSetup> {
    let shared = shared.inner().clone();
    blocking(move || {
        let engine = &shared.engine;
        Ok(SpeakerSetup {
            local_available: engine.local_speaker_recognition_available(),
            ready: engine.speaker_recognition_ready(),
            signed_in: engine.cloud_signed_in(),
            model_ids: engine.speaker_model_ids(),
            legacy_model: engine.model_state(engine.diarizer_model_id()) == ModelState::Downloaded,
            profile: engine.voice_profile(),
            known: engine.known_voices(),
            enrollment_text: crate::ui::message(engine.voice_enrollment_text()),
        })
    })
    .await
}

#[tauri::command]
pub async fn start_voice_enrollment(app: AppHandle, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("start_voice_enrollment", move || {
        speakers::start_enrollment(&app, &shared)
    })
    .await
}

#[tauri::command]
pub async fn finish_voice_enrollment(shared: State<'_, Arc<Shared>>) -> Result<VoiceProfile> {
    let shared = shared.inner().clone();
    off_main("finish_voice_enrollment", move || {
        speakers::finish_enrollment(&shared)
    })
    .await
}

/// Off the main thread: closing the microphone hops onto it, and the one-minute limit can be
/// closing it at the same moment while holding the enrollment.
#[tauri::command]
pub async fn cancel_voice_enrollment() {
    let _ = tauri::async_runtime::spawn_blocking(speakers::cancel_enrollment).await;
}

#[tauri::command]
pub async fn forget_voice_profile(shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("forget_voice_profile", move || {
        shared
            .engine
            .forget_voice_profile()
            .map_err(speakers::speaker_error)
    })
    .await
}

#[tauri::command]
pub async fn rename_known_voice(
    id: String,
    name: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    if name.trim().is_empty() {
        return Err("Give the voice a name.".into());
    }
    let shared = shared.inner().clone();
    off_main("rename_known_voice", move || {
        shared
            .engine
            .rename_known_voice(id, name.trim().to_owned())
            .map_err(speakers::speaker_error)
    })
    .await
}

#[tauri::command]
pub async fn forget_known_voice(id: String, shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("forget_known_voice", move || {
        shared
            .engine
            .forget_known_voice(id)
            .map_err(speakers::speaker_error)
    })
    .await
}

#[tauri::command]
pub async fn forget_all_voices(shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("forget_all_voices", move || {
        shared
            .engine
            .forget_all_voices()
            .map_err(speakers::speaker_error)
    })
    .await
}

#[tauri::command]
pub async fn meeting_voices(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Vec<MeetingVoice>> {
    let shared = shared.inner().clone();
    off_main("meeting_voices", move || {
        checked_meeting_dir(&shared.engine, &id)?;
        Ok(shared.engine.meeting_voices(id))
    })
    .await
}

#[tauri::command]
pub async fn remember_speaker(
    id: String,
    voice: u32,
    name: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    if name.trim().is_empty() {
        return Err("Give the voice a name to remember it.".into());
    }
    let shared = shared.inner().clone();
    off_main("remember_speaker", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .remember_speaker(id, voice, name.trim().to_owned())
            .map_err(speakers::speaker_error)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn export_meeting(
    id: String,
    include_transcript: bool,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingExport> {
    let shared = shared.inner().clone();
    blocking(move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        shared
            .engine
            .export_meeting_localized(id, include_transcript, shared.interface_language())
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn save_meeting_notes(
    id: String,
    text: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("save_meeting_notes", move || {
        checked_meeting_dir(&shared.engine, &id)?;
        shared.engine.set_meeting_notes(id, text).map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_meeting_attendees(
    id: String,
    attendees: Vec<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    off_main("set_meeting_attendees", move || {
        let _library_job = meeting_runtime::LibraryJob::new(&shared)?;
        checked_meeting_dir(&shared.engine, &id)?;
        shared
            .engine
            .set_meeting_attendees(id, attendees)
            .map_err(err)
    })
    .await?;
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(&app);
    Ok(())
}

#[tauri::command]
pub async fn detect_cli_agents() -> Result<Vec<DetectedAgent>> {
    blocking(|| Ok(pennel_core::detect_cli_agents())).await
}

#[tauri::command]
pub async fn check_cli_agent(
    id: String,
    model: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<String> {
    let shared = shared.inner().clone();
    blocking(move || {
        let account = shared
            .settings()
            .providers
            .into_iter()
            .find(|provider| provider.id == id)
            .ok_or("Unknown provider.")?;
        if !account.kind.is_cli_agent() {
            return Err("This provider is not a CLI agent.".into());
        }
        let path = if account.base_url.is_empty() {
            pennel_core::detect_cli_agents()
                .into_iter()
                .find(|agent| agent.kind == account.kind)
                .map(|agent| agent.path)
                .ok_or("This CLI agent is not installed. Install it and sign in.")?
        } else {
            account.base_url
        };
        pennel_core::check_cli_agent(account.kind, path, model).map_err(err)
    })
    .await
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    crate::show_main(&app, None);
}

#[tauri::command]
pub fn hide_window(window: tauri::Window) {
    #[cfg(target_os = "macos")]
    if matches!(window.label(), "record-prompt" | "pill")
        && let Ok(pointer) = window.ns_window()
    {
        unsafe {
            crate::macos::hide_panel(pointer);
        }
        return;
    }
    crate::workspace::cancel_presentation(window.app_handle(), window.label());
    let _ = window.hide();
    crate::notify_visibility(window.app_handle(), window.label(), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_saving_failures_localize_each_owned_envelope_and_preserve_legacy_aliases() {
        for prefix in ["Saving failed: ", "Save failed: ", "Bewaren mislukt: "] {
            let source = format!("{prefix}storage error: the storage folder is missing");
            assert_eq!(
                localize_error_message_for_language(&source, pennel_core::InterfaceLanguage::Dutch),
                "Bewaren mislukt: Opslagfout: de opslagmap ontbreekt"
            );
            assert_eq!(
                localize_error_message_for_language(
                    &source,
                    pennel_core::InterfaceLanguage::English
                ),
                "Saving failed: Storage error: the storage folder is missing"
            );
        }
        let untrusted = "Mijn eigen notitie: Bewaren mislukt: {error}";
        assert_eq!(
            localize_error_message_for_language(untrusted, pennel_core::InterfaceLanguage::English),
            untrusted
        );
    }

    #[test]
    fn live_note_saves_keep_the_owned_session_and_surface_storage_failures() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().join("data"));
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "Fixture".into(),
                pennel_core::ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        let token = shared.recording.lock().unwrap().begin(None).unwrap();
        shared.recording.lock().unwrap().recording(&token).unwrap();
        *shared.meeting_context.lock().unwrap() = Some(meeting_runtime::MeetingContext {
            token: token.clone(),
            session: session.clone(),
            title: "Fixture".into(),
            started: std::time::Instant::now(),
            duration_secs: None,
            pending_recording_finish: None,
            system_audio: false,
            heard_system_audio: false,
            audio_warning: None,
            consent_copied: false,
        });
        save_live_notes(&shared, "Saved before stop".into(), &token).unwrap();
        assert!(save_live_notes(&shared, "Stale".into(), "other-session").is_err());
        assert_eq!(
            shared.engine.meeting_notes(session.id()),
            "Saved before stop"
        );
        shared.recording.lock().unwrap().stopping(&token).unwrap();
        save_live_notes(&shared, "Flushed during stop".into(), &token).unwrap();
        let notes = PathBuf::from(session.dir()).join("notes.md");
        std::fs::remove_file(&notes).unwrap();
        std::fs::create_dir(&notes).unwrap();
        assert!(save_live_notes(&shared, "Cannot write".into(), &token).is_err());
        std::fs::remove_dir(&notes).unwrap();
        shared.recording.lock().unwrap().finalizing(&token).unwrap();
        assert!(save_live_notes(&shared, "Too late".into(), &token).is_err());
        session.stop().unwrap();
    }

    #[test]
    fn saved_interface_preference_changes_presentation_without_changing_speech_settings() {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let mut settings = shared.settings();
        settings.language = Some("nl".into());
        settings.ui_language = pennel_core::InterfaceLanguagePreference::Dutch;
        shared.engine.update_settings(settings).unwrap();
        let app = info(&shared).unwrap();
        assert_eq!(
            app.interface_language,
            pennel_core::InterfaceLanguage::Dutch
        );
        assert_eq!(
            app.settings.ui_language,
            pennel_core::InterfaceLanguagePreference::Dutch
        );
        assert_eq!(app.settings.language.as_deref(), Some("nl"));
        let local = app
            .kinds
            .iter()
            .find(|kind| kind.kind == ProviderKind::Local)
            .unwrap();
        assert_eq!(local.label, "Local on this computer");
    }

    #[test]
    fn only_conflict_copies_inside_a_meeting_folder_may_be_deleted() {
        assert!(is_conflict_copy_name(
            "notes (conflict Mac 2026-10-25 0930).md"
        ));
        assert!(is_conflict_copy_name(
            "transcript (conflict Laptop 2026-10-25 0930).json"
        ));
        for name in [
            "notes.md",
            "",
            "..",
            "../notes (conflict Mac 1).md",
            "sub/notes (conflict Mac 1).md",
            "sub\\notes (conflict Mac 1).md",
            "C:notes (conflict Mac 1).md",
            "notes(conflict Mac 1).md",
        ] {
            assert!(!is_conflict_copy_name(name), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_named_like_a_conflict_copy_is_neither_listed_nor_deleted() {
        let dir = std::env::temp_dir().join(format!("pennel-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.md"), "mine").unwrap();
        std::os::unix::fs::symlink(dir.join("notes.md"), dir.join("x (conflict a).md")).unwrap();
        std::fs::write(dir.join("y (conflict b).md"), "copy").unwrap();

        assert_eq!(list_conflict_copies(&dir).unwrap(), ["y (conflict b).md"]);
        assert!(remove_conflict_copy(&dir, "x (conflict a).md").is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("notes.md")).unwrap(),
            "mine"
        );
        remove_conflict_copy(&dir, "y (conflict b).md").unwrap();
        assert!(!dir.join("y (conflict b).md").exists());
        assert!(remove_conflict_copy(&dir, "notes.md").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn provider_contract_exposes_every_persisted_kind_with_correct_tasks() {
        let kinds = provider_kinds(
            &Capability::unsupported("macOS vereist"),
            &Capability::unavailable("Schakel Apple Intelligence in."),
            &Capability::unsupported(pennel_core::UNSUPPORTED_PROCESSOR),
        );
        let json = serde_json::to_value(kinds).unwrap();
        let kinds = json.as_array().unwrap();
        let mut actual: Vec<&str> = kinds
            .iter()
            .map(|kind| kind["kind"].as_str().unwrap())
            .collect();
        actual.sort_unstable();
        assert_eq!(
            actual,
            [
                "apple_intelligence",
                "apple_speech",
                "claude_code",
                "codex",
                "copilot",
                "cursor_agent",
                "custom",
                "gemini_cli",
                "groq",
                "lm_studio",
                "local",
                "mistral",
                "ollama",
                "open_ai",
                "open_code",
                "open_router",
                "pennel_cloud"
            ]
        );
        for cli in kinds.iter().filter(|kind| kind["is_cli_agent"] == true) {
            assert_eq!(cli["requires_key"], false);
            assert_eq!(cli["stt"], false);
            assert_eq!(cli["llm"], true);
        }
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| kind["is_cli_agent"] == true)
                .count(),
            6
        );
        let apple = kinds
            .iter()
            .find(|kind| kind["kind"] == "apple_speech")
            .unwrap();
        assert_eq!(apple["stt"], true);
        assert_eq!(apple["llm"], false);
        assert_eq!(apple["availability"]["status"], "unsupported");
        let intelligence = kinds
            .iter()
            .find(|kind| kind["kind"] == "apple_intelligence")
            .unwrap();
        assert_eq!(intelligence["stt"], false);
        assert_eq!(intelligence["llm"], true);
        assert_eq!(intelligence["requires_key"], false);
        assert_eq!(intelligence["default_llm_model"], "system");
        assert_eq!(intelligence["availability"]["status"], "unavailable");
        let local = kinds.iter().find(|kind| kind["kind"] == "local").unwrap();
        assert_eq!(local["availability"]["status"], "unsupported");
        assert_eq!(
            local["availability"]["reason"],
            pennel_core::UNSUPPORTED_PROCESSOR
        );
    }

    #[test]
    fn meeting_identifiers_cannot_address_another_folder_on_either_os() {
        for id in [
            "",
            ".",
            "..",
            "../ander",
            "ander/meeting",
            "ander\\meeting",
            "/tmp/meeting",
            "C:\\meeting",
            "a:b",
            "meeting\nnaam",
            "meeting\0naam",
        ] {
            assert!(validate_meeting_id(id).is_err(), "accepted {id:?}");
        }
        assert!(validate_meeting_id("2026-10-24 0900 Klantgesprek (2)").is_ok());
    }

    #[test]
    fn missing_archives_are_explicit_in_the_meeting_contract() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("ik.m4a"), b"fixture").unwrap();
        let tracks = serde_json::to_value(audio_tracks(directory.path())).unwrap();
        assert_eq!(
            tracks,
            serde_json::json!([
                {"speaker":"me","available":true},
                {"speaker":"others","available":false}
            ])
        );
    }

    #[test]
    fn grouped_archives_are_available_through_the_existing_ui_contract() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("audio")).unwrap();
        std::fs::write(directory.path().join("audio/microphone.m4a"), b"fixture").unwrap();
        std::fs::write(directory.path().join("audio/system.m4a"), b"fixture").unwrap();
        assert!(
            audio_tracks(directory.path())
                .iter()
                .all(|track| track.available)
        );
    }

    #[test]
    fn chat_history_is_data_with_a_stable_wire_contract() {
        let history: Vec<ChatTurn> = serde_json::from_value(serde_json::json!([
            {"question":"Wat is afgesproken?","answer":"Donderdag [1]."}
        ]))
        .unwrap();
        assert_eq!(history[0].question, "Wat is afgesproken?");
        assert_eq!(history[0].answer, "Donderdag [1].");
    }

    #[test]
    fn finalizing_keeps_the_meeting_and_saved_notes_visible_without_a_recorder() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("data");
        let library = root.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        let engine =
            pennel_core::PennelEngine::new(data_dir.to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.meetings_dir = library.to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        let session = engine
            .clone()
            .start_meeting(
                "Klantgesprek".into(),
                pennel_core::ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        session.set_notes("Mijn eigen notities.".into()).unwrap();
        let mut lifecycle = pennel_core::RecordingLifecycle::default();
        let token = lifecycle.begin(None).unwrap();
        lifecycle.recording(&token).unwrap();
        lifecycle.stopping(&token).unwrap();
        lifecycle.finalizing(&token).unwrap();
        let (dictation, _receiver) = std::sync::mpsc::channel();
        let shared = Shared {
            engine,
            system_interface_language: Default::default(),
            version: "0.2.0-test".into(),
            preview: true,
            migration_warning: None,
            data_dir,
            dictation,
            secrets: Arc::new(crate::secrets::Secrets::in_memory()),
            hotkey: std::sync::Mutex::new(None),
            hotkey_error: std::sync::Mutex::new(None),
            meeting: std::sync::Mutex::new(None),
            recording: std::sync::Mutex::new(lifecycle),
            recording_cancel: std::sync::atomic::AtomicBool::new(false),
            library_jobs: std::sync::atomic::AtomicUsize::new(0),
            updating: std::sync::atomic::AtomicBool::new(false),
            quitting: std::sync::atomic::AtomicBool::new(false),
            dictation_active: std::sync::atomic::AtomicBool::new(false),
            dictation_jobs: std::sync::atomic::AtomicUsize::new(0),
            paste_jobs: std::sync::atomic::AtomicUsize::new(0),
            meeting_context: std::sync::Mutex::new(Some(crate::meeting_runtime::MeetingContext {
                token,
                session: session.clone(),
                title: "Klantgesprek".into(),
                started: std::time::Instant::now(),
                duration_secs: Some(42),
                pending_recording_finish: None,
                system_audio: true,
                heard_system_audio: true,
                audio_warning: Some("Systeemgeluid is onderbroken.".into()),
                consent_copied: true,
            })),
        };
        let snapshot = status(&shared);
        assert!(snapshot.runtime.busy);
        assert!(!snapshot.runtime.recording);
        assert_eq!(
            snapshot.runtime.phase,
            pennel_core::RecordingPhase::Finalizing
        );
        assert_eq!(snapshot.runtime.id.as_deref(), Some(session.id().as_str()));
        assert_eq!(snapshot.runtime.title.as_deref(), Some("Klantgesprek"));
        assert_eq!(snapshot.notes, "Mijn eigen notities.");
        assert_eq!(snapshot.runtime.elapsed_secs, 42);
        assert_eq!(
            snapshot.runtime.audio_warning.as_deref(),
            Some("Systeemgeluid is onderbroken.")
        );
        assert!(snapshot.runtime.system_audio);
        assert!(snapshot.runtime.consent_copied);
        let metadata = serde_json::to_value(runtime_status(&shared).0).unwrap();
        assert!(metadata.get("notes").is_none());
        assert!(metadata.get("transcript").is_none());
        assert_eq!(
            metadata["transcript_revision"],
            snapshot.runtime.transcript_revision
        );
        assert_eq!(metadata["id"].as_str(), snapshot.runtime.id.as_deref());
        assert_eq!(metadata["phase"], "finalizing");
        assert_eq!(metadata["elapsed_secs"], 42);
        let mut serialized = serde_json::to_value(snapshot).unwrap();
        let fields = serialized.as_object_mut().unwrap();
        assert!(fields.remove("notes").is_some());
        assert!(fields.remove("transcript").is_some());
        assert_eq!(serialized, metadata);
        assert!(require_inactive_meeting(&shared, &session.id()).is_err());
        session.stop().unwrap();
    }
}
