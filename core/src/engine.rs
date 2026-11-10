use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cleanup::{
    assess_cleanup, cleanup_messages, cleanup_timeout_secs, resolve_style, strip_wrapping,
};
use crate::cloud::account::{CloudAccount, CloudDevice, CloudLoginPoll, CloudLoginStart, Session};
use crate::cloud::crypto::{self, VaultKey};
use crate::cloud::sync::api::{HttpSyncApi, SyncApi};
use crate::cloud::sync::state::SyncState;
use crate::cloud::sync::{self, MeetingSyncState, PassInput, SyncReport, SyncStatus};
use crate::cloud::{SecretStore, VAULT_KEY_ACCOUNT};
use crate::dictation_pad::DictationPad;
use crate::error::PennelError;
use crate::export::MeetingExport;
use crate::history::{HistoryCaptureKind, HistoryEntry, HistoryStatus, HistoryStore};
use crate::live_dictation::{DictationPreviewObserver, LiveDictationSession};
use crate::llm::Reasoning;
use crate::local_engine::LocalEngine;
use crate::meeting_chat::{ChatTurn, MeetingAnswer};
use crate::meeting_index::{MeetingIndex, Passage};
use crate::meetings::SpeakerName;
use crate::meetings::{MeetingInfo, MeetingStore, Segment, Speaker};
use crate::models::{CancelFlag, DownloadListener, LocalModel, ModelState, ModelStore};
use crate::provider::{
    ProviderAccount, ProviderConfig, ProviderKind, ProviderTask, requires_key, supports,
};
use crate::session::MeetingSession;
use crate::settings::ModelChoice;
use crate::settings::{DictationMode, Settings};
use crate::speech::{has_speech, is_hallucination};
use crate::storage::{DirectoryLease, StartupStorage};
use crate::stt::transcribe;
use crate::system_llm::{self, SystemLanguageModel};
use crate::system_stt::SystemTranscriber;
use crate::templates::{MeetingTemplate, TemplateStore};
use crate::wav::{decode_wav, encode_wav};

const MAX_PROMPT_CHARS: usize = 800;

#[path = "dictation_models.rs"]
mod dictation_models;
pub use dictation_models::ConfiguredDictationModel;

#[path = "writing_profile_engine.rs"]
mod writing_profile_engine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingQuestionScope {
    WholeMeeting,
    SelectedExcerpts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeetingQuestionSelection {
    pub scope: MeetingQuestionScope,
    pub available_sources: u32,
    pub included_sources: u32,
    pub available_history_turns: u32,
    pub included_history_turns: u32,
    pub history_reduced: bool,
}

impl MeetingQuestionSelection {
    fn from_messages(
        available_sources: usize,
        included_sources: usize,
        history: &[ChatTurn],
        messages: &[crate::llm::ChatMessage],
    ) -> Self {
        let included_history = messages
            .iter()
            .filter(|message| message.role == "assistant")
            .count();
        let retained = &history[history.len().saturating_sub(included_history)..];
        let history_reduced = included_history != history.len()
            || retained.iter().enumerate().any(|(index, turn)| {
                messages
                    .get(1 + index * 2)
                    .is_none_or(|message| message.content != turn.question)
                    || messages
                        .get(2 + index * 2)
                        .is_none_or(|message| message.content != turn.answer)
            });
        Self {
            scope: if available_sources == included_sources {
                MeetingQuestionScope::WholeMeeting
            } else {
                MeetingQuestionScope::SelectedExcerpts
            },
            available_sources: available_sources.try_into().unwrap_or(u32::MAX),
            included_sources: included_sources.try_into().unwrap_or(u32::MAX),
            available_history_turns: history.len().try_into().unwrap_or(u32::MAX),
            included_history_turns: included_history.try_into().unwrap_or(u32::MAX),
            history_reduced,
        }
    }
}

fn validate_live_question_size(question: &str, kind: ProviderKind) -> Result<(), PennelError> {
    if question.len() > 64 * 1024
        || kind == ProviderKind::AppleIntelligence
            && crate::system_llm::input_bytes(&crate::meeting_chat::chat_messages(
                question,
                &[],
                &[],
            )) > crate::system_llm::APPLE_INPUT_BYTES
    {
        return Err(PennelError::InvalidResponse { message: "This question exceeds the selected setup's input budget. It was not truncated or sent. Shorten the question and try again.".into() });
    }
    Ok(())
}

/// A source-bound request whose provider and evidence cannot change while it is queued.
pub struct PreparedMeetingQuestion {
    provider: ProviderConfig,
    key: Option<String>,
    system: Option<Arc<dyn SystemLanguageModel>>,
    messages: Vec<crate::llm::ChatMessage>,
    sources: Vec<Passage>,
    admission: crate::ForegroundQuestionAdmission,
    _registration: Option<PreparedInferenceRegistration>,
    source_guard: Option<QuestionSourceGuard>,
    conversation_guard: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    cloud_guard: Option<(Arc<Session>, u64)>,
    selection: MeetingQuestionSelection,
}

struct PreparedInferenceRegistration {
    state: Arc<Mutex<crate::inference_engine::InferenceState>>,
    cancellation: Arc<crate::InferenceCancellation>,
}

impl Drop for PreparedInferenceRegistration {
    fn drop(&mut self) {
        let reference = Arc::downgrade(&self.cancellation);
        self.state
            .lock()
            .unwrap()
            .requests
            .retain(|request| request.strong_count() > 0 && !request.ptr_eq(&reference));
    }
}

struct QuestionSourceGuard {
    root: PathBuf,
    uid: String,
    live: Option<Weak<MeetingSession>>,
    anchors: Vec<crate::MeetingSourceAnchor>,
}

struct QuestionPreparation {
    provider: ProviderConfig,
    key: Option<String>,
    system: Option<Arc<dyn SystemLanguageModel>>,
    admission: crate::ForegroundQuestionAdmission,
    registration: Option<PreparedInferenceRegistration>,
    source_guard: Option<QuestionSourceGuard>,
    cloud_guard: Option<(Arc<Session>, u64)>,
}

impl QuestionSourceGuard {
    fn current(&self) -> bool {
        let store = MeetingStore::new(&self.root);
        let Some(id) = store.list().into_iter().find_map(|info| {
            (store.uid_of(&info.id).as_deref() == Some(self.uid.as_str())).then_some(info.id)
        }) else {
            return false;
        };
        let rows = if let Some(session) = self
            .live
            .as_ref()
            .and_then(Weak::upgrade)
            .filter(|session| !session.is_finalized() && session.id() == id)
        {
            session.utterance_snapshot().1
        } else {
            let Ok(snapshot) = store.read_transcript_snapshot(&id) else {
                return false;
            };
            if !matches!(
                snapshot.metadata_status,
                crate::TranscriptMetadataStatus::Valid | crate::TranscriptMetadataStatus::Absent
            ) {
                return false;
            }
            snapshot.utterances
        };
        let notes = store.read_notes(&id);
        self.anchors.iter().all(|anchor| {
            if !anchor.segment_keys.is_empty() {
                anchor.segment_keys.len() == anchor.segment_fingerprints.len()
                    && anchor
                        .segment_keys
                        .iter()
                        .zip(&anchor.segment_fingerprints)
                        .all(|(key, fingerprint)| {
                            key.strip_prefix("utterance:")
                                .and_then(|id| {
                                    rows.iter().find(|row| row.metadata.utterance_id == id)
                                })
                                .is_some_and(|row| {
                                    crate::utterance_evidence_revision(row) == *fingerprint
                                })
                        })
            } else if let (Some(start), Some(end)) = (anchor.notes_start, anchor.notes_end) {
                utf16_source_slice(&notes, start, end) == Some(anchor.source_text.as_str())
            } else {
                false
            }
        })
    }
}

fn utf16_source_slice(text: &str, start: usize, end: usize) -> Option<&str> {
    if start > end {
        return None;
    }
    let mut units = 0;
    let mut first = None;
    let mut last = None;
    for (offset, character) in text.char_indices() {
        if units == start {
            first = Some(offset);
        }
        if units == end {
            last = Some(offset);
            break;
        }
        units += character.len_utf16();
    }
    if units == start && first.is_none() {
        first = Some(text.len());
    }
    if units == end && last.is_none() {
        last = Some(text.len());
    }
    text.get(first?..last?)
}

impl PreparedMeetingQuestion {
    pub(crate) fn with_conversation_guard(
        mut self,
        guard: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        self.source_guard = None;
        self.conversation_guard = Some(guard);
        self
    }
    pub fn sources(&self) -> &[Passage] {
        &self.sources
    }

    pub fn selection(&self) -> MeetingQuestionSelection {
        self.selection
    }

    pub fn complete(
        self,
        cancellation: Arc<crate::InferenceCancellation>,
        physical_work: impl Send + 'static,
    ) -> Result<MeetingAnswer, PennelError> {
        let deadline = self.admission.deadline();
        let cancellation = crate::InferenceCancellation::linked(
            vec![self.admission.cancellation(), cancellation],
            crate::InferencePriority::ForegroundQuestion,
            deadline,
        );
        let worker_cancellation = cancellation.clone();
        crate::inference::run_cancelled_before(cancellation, deadline, move |deadline| {
            let _physical_work = physical_work;
            self.admission.dispatched();
            let valid = || {
                !worker_cancellation.is_cancelled()
                    && self
                        .source_guard
                        .as_ref()
                        .is_none_or(QuestionSourceGuard::current)
                    && self.conversation_guard.as_ref().is_none_or(|guard| guard())
                    && self
                        .cloud_guard
                        .as_ref()
                        .is_none_or(|(session, generation)| {
                            session.generation() == *generation && session.signed_in()
                        })
            };
            if !valid() {
                return Err(PennelError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let seconds = remaining.as_secs();
            let text = if self.provider.kind == ProviderKind::AppleIntelligence {
                if seconds == 0 {
                    return Err(crate::inference::network());
                }
                system_llm::complete(self.system.as_deref(), &self.messages, seconds)
            } else if self.provider.kind.is_cli_agent() {
                if seconds == 0 {
                    return Err(crate::inference::network());
                }
                crate::cli_agent::complete(
                    self.provider.kind,
                    &self.provider.base_url,
                    &self.provider.model,
                    &self.messages,
                    seconds,
                )
            } else {
                crate::llm::chat_complete_before(
                    &self.provider,
                    self.key.as_deref(),
                    &self.messages,
                    deadline,
                    Reasoning::Low,
                )
            }?;
            if !valid() {
                return Err(PennelError::Cancelled);
            }
            Ok(MeetingAnswer {
                citations: crate::meeting_chat::cited(&text, &self.sources),
                text,
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ApiKeys {
    pub by_provider: HashMap<String, String>,
}

impl ApiKeys {
    fn get(&self, account: &ProviderAccount) -> Option<&str> {
        self.by_provider
            .get(&account.id)
            .map(String::as_str)
            .filter(|k| !k.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DictationRequest {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub app_bundle_id: Option<String>,
    pub keys: ApiKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupStatus {
    Applied,
    Skipped,
    Rejected,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DictationResult {
    pub history_id: String,
    pub history_saved: bool,
    pub text: String,
    pub raw: String,
    pub cleanup: CleanupStatus,
}

#[derive(uniffi::Object)]
pub struct PennelEngine {
    settings_path: PathBuf,
    settings: RwLock<Settings>,
    pub(crate) inference_requests: Arc<Mutex<crate::inference_engine::InferenceState>>,
    #[cfg(test)]
    pub(crate) cleanup_qualification: Mutex<Option<crate::cleanup_verification::Qualification>>,
    #[cfg(test)]
    pub(crate) cleanup_reason_codes: Mutex<Vec<String>>,
    #[cfg(test)]
    pub(crate) cleanup_language_evidence: Mutex<Option<String>>,
    history: Mutex<HistoryStore>,
    writing_profile: Mutex<crate::writing_profile::WritingProfileStore>,
    writing_profile_refresh: Mutex<()>,
    writing_profile_cancellation: Mutex<Option<Arc<crate::InferenceCancellation>>>,
    #[cfg(test)]
    writing_profile_before_dispatch: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    writing_profile_test_pool: Mutex<Option<Arc<crate::meeting_scheduler::PhysicalInferencePool>>>,
    pub(crate) conversations: Mutex<crate::conversations::State>,
    unsaved_history: Mutex<RecoveryState>,
    dictation_pad: Mutex<DictationPad>,
    pub(crate) models: ModelStore,
    downloads: Mutex<HashMap<String, Arc<DownloadTask>>>,
    local: LocalEngine,
    index: Mutex<Option<MeetingIndex>>,
    system_stt: RwLock<Option<Arc<dyn SystemTranscriber>>>,
    timed_system_stt: RwLock<Option<Arc<dyn crate::TimedSystemTranscriber>>>,
    system_llm: RwLock<Option<Arc<dyn SystemLanguageModel>>>,
    secrets: RwLock<Option<Arc<dyn SecretStore>>>,
    cloud_session: Mutex<Option<Arc<Session>>>,
    meeting_gate: Mutex<()>,
    active_sessions: Mutex<Vec<Weak<MeetingSession>>>,
    meeting_preview_priority: Mutex<usize>,
    pub(crate) meeting_jobs: Mutex<HashSet<String>>,
    pub(crate) retranscriptions:
        Mutex<HashMap<String, crate::retranscription::MeetingRetranscriptionStatus>>,
    pub(crate) speaker_jobs:
        Mutex<HashMap<String, Arc<crate::speaker_jobs::SpeakerRecognitionJob>>>,
    pub(crate) insight_jobs:
        Mutex<HashMap<String, Arc<crate::meeting_insight_engine::MeetingInsightAnalysisJob>>>,
    pub(crate) insight_library_jobs: Arc<std::sync::atomic::AtomicUsize>,
    library_lease: Mutex<Option<(PathBuf, DirectoryLease)>>,
    /// Held for a whole sync pass and for every vault change, so they never interleave.
    pub(crate) sync_gate: Mutex<()>,
    sync_state: Mutex<Option<SyncState>>,
    sync_last: Mutex<Option<SyncReport>>,
    sync_items: Mutex<Vec<MeetingSyncState>>,
    deleted_meetings: Mutex<Vec<String>>,
    /// Set while a pass held the state; the next pass records them.
    pending_device: Mutex<Option<String>>,
    pending_confirm: Mutex<bool>,
    pub(crate) cloud_link: Mutex<crate::cloud::link::LinkFlow>,
    pub(crate) link_journal_gate: Mutex<()>,
    #[cfg(test)]
    sync_api_override: Mutex<Option<Arc<dyn SyncApi>>>,
    preview: bool,
    _storage: StartupStorage,
}

struct Transcript {
    raw: String,
    text: String,
    cleanup: CleanupStatus,
}

const RECOVERY_ENTRIES: usize = 20;
const RECOVERY_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone)]
struct UnsavedDictation {
    entry: HistoryEntry,
    cleanup: CleanupStatus,
    capture_kind: HistoryCaptureKind,
    profile_allowed: bool,
}

#[derive(Default)]
struct RecoveryState {
    entries: HashMap<String, UnsavedDictation>,
    reserved: usize,
}

impl RecoveryState {
    fn ensure_capacity(&self) -> Result<(), PennelError> {
        let bytes: usize = self
            .entries
            .values()
            .map(|result| {
                result
                    .entry
                    .raw
                    .len()
                    .saturating_add(result.entry.text.len())
            })
            .sum();
        if self.entries.len() + self.reserved >= RECOVERY_ENTRIES || bytes >= RECOVERY_BYTES {
            return Err(PennelError::Storage {
                message: "Save or discard your temporary dictations in History first.".into(),
            });
        }
        Ok(())
    }
}

struct RecoverySlot<'a>(&'a Mutex<RecoveryState>);

impl Drop for RecoverySlot<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap().reserved -= 1;
    }
}

#[derive(Default)]
struct DownloadState {
    listeners: Vec<Arc<DownloadObserver>>,
    progress: Option<(u64, u64)>,
    result: Option<Result<(), PennelError>>,
}

#[derive(Default)]
struct DownloadTask {
    cancel: CancelFlag,
    state: Mutex<DownloadState>,
    finished: Condvar,
}

struct DownloadObserver {
    listener: Arc<dyn DownloadListener>,
    progress: Mutex<Option<(u64, u64)>>,
}

impl DownloadObserver {
    fn publish(&self, downloaded: u64, total: u64) {
        let mut previous = self.progress.lock().unwrap();
        if previous.is_some_and(|(last, _)| downloaded < last) {
            return;
        }
        *previous = Some((downloaded, total));
        self.listener.on_progress(downloaded, total);
    }
}

impl DownloadTask {
    fn subscribe(&self, listener: Arc<dyn DownloadListener>) {
        let observer = Arc::new(DownloadObserver {
            listener,
            progress: Mutex::new(None),
        });
        let progress = {
            let mut state = self.state.lock().unwrap();
            state.listeners.push(observer.clone());
            state.progress
        };
        if let Some((downloaded, total)) = progress {
            observer.publish(downloaded, total);
        }
    }

    fn wait(&self) -> Result<(), PennelError> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(result) = &state.result {
                return result.clone();
            }
            state = self.finished.wait(state).unwrap();
        }
    }

    fn finish(&self, result: Result<(), PennelError>) {
        self.state.lock().unwrap().result = Some(result);
        self.finished.notify_all();
    }
}

impl DownloadListener for DownloadTask {
    fn on_progress(&self, downloaded: u64, total: u64) {
        let listeners = {
            let mut state = self.state.lock().unwrap();
            state.progress = Some((downloaded, total));
            state.listeners.clone()
        };
        for listener in listeners {
            listener.publish(downloaded, total);
        }
    }
}

/// Leading dictionary terms whose comma-joined length fits in `max_chars`.
fn capped_terms(terms: &[String], max_chars: usize) -> Vec<String> {
    let mut used = 0;
    terms
        .iter()
        .take_while(|term| {
            used += term.chars().count() + 2;
            used <= max_chars + 2
        })
        .cloned()
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[uniffi::export]
impl PennelEngine {
    pub(crate) fn recording_recognition(&self, id: &str) -> crate::session::MeetingRecognition {
        self.active_sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|session| session.id() == id)
            .map_or_else(
                || crate::session::MeetingRecognition::from_settings(&self.settings()),
                |session| session.recognition_config(),
            )
    }
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, PennelError> {
        Self::open(data_dir, false)
    }

    #[uniffi::constructor]
    pub fn new_preview(data_dir: String) -> Result<Arc<Self>, PennelError> {
        Self::open(data_dir, true)
    }

    /// Lets the app plug in the OS's on-device recognizer (Apple's dictation model) as a provider.
    pub fn set_system_transcriber(&self, transcriber: Arc<dyn SystemTranscriber>) {
        *self.system_stt.write().unwrap() = Some(transcriber);
        *self.timed_system_stt.write().unwrap() = None;
    }

    pub fn set_timed_system_transcriber(
        &self,
        transcriber: Arc<dyn crate::TimedSystemTranscriber>,
    ) {
        *self.timed_system_stt.write().unwrap() = Some(transcriber);
    }

    pub fn set_secret_store(&self, store: Arc<dyn SecretStore>) -> Result<(), PennelError> {
        let _identity = crate::speaker_jobs::speaker_identity_guard();
        let _gate = self.meeting_gate.lock().unwrap();
        self.ensure_cloud_identity_mutable()?;
        *self.secrets.write().unwrap() = Some(store);
        *self.cloud_session.lock().unwrap() = None;
        Ok(())
    }

    pub fn cloud_login_start(
        &self,
        device_name: String,
        platform: String,
    ) -> Result<CloudLoginStart, PennelError> {
        self.cloud_login_start_with_locale(device_name, platform, None)
    }

    pub fn cloud_login_start_with_locale(
        &self,
        device_name: String,
        platform: String,
        locale: Option<String>,
    ) -> Result<CloudLoginStart, PennelError> {
        if crate::cloud::link::has_pending(self) {
            return Err(PennelError::Cloud {
                code: "pairing_pending".into(),
                message: "Finish or cancel the current device connection before signing in again."
                    .into(),
            });
        }
        let _identity = crate::speaker_jobs::speaker_identity_guard();
        let _gate = self.meeting_gate.lock().unwrap();
        self.ensure_cloud_identity_mutable()?;
        let started = self.cloud_session()?.login_start_with_locale(
            &device_name,
            &platform,
            locale.as_deref(),
        )?;
        self.remember_device_name(&device_name);
        Ok(started)
    }

    pub fn cloud_login_poll(&self, device_code: String) -> Result<CloudLoginPoll, PennelError> {
        let _identity = crate::speaker_jobs::speaker_identity_guard();
        let _gate = self.meeting_gate.lock().unwrap();
        if crate::cloud::link::has_pending(self) {
            return Err(PennelError::Cloud {
                code: "pairing_pending".into(),
                message: "Finish or cancel the current device connection before signing in again."
                    .into(),
            });
        }
        self.ensure_cloud_identity_mutable()?;
        self.cloud_session()?.login_poll(&device_code)
    }

    /// Invalidates pending login work without waiting for a network or recording operation.
    pub fn cloud_login_cancel(&self) {
        let session = self.cloud_session.lock().unwrap().clone();
        if let Some(session) = session {
            session.cancel_login();
        }
    }

    pub fn cloud_signed_in(&self) -> bool {
        self.cloud_session()
            .is_ok_and(|session| session.signed_in())
    }

    /// Meeting files stay; only what this device knew about the cloud is forgotten.
    pub fn cloud_logout(&self) -> Result<(), PennelError> {
        if crate::cloud::link::has_pending(self) {
            self.cloud_link_cancel()?;
            if crate::cloud::link::has_pending(self) {
                return Err(PennelError::Cloud {code:"pairing_pending".into(),message:"The device connection could not be cancelled yet. Try again after unlocking this device.".into()});
            }
        }
        let _identity = crate::speaker_jobs::speaker_identity_guard();
        self.ensure_cloud_identity_mutable()?;
        {
            // Sync settings take these locks in this order too. Do not invert them during logout.
            let _sync = self.sync_gate.lock().unwrap();
            let _meeting = self.meeting_gate.lock().unwrap();
            self.ensure_cloud_identity_mutable()?;
            if let Ok(session) = self.cloud_session() {
                session.logout();
            }
            self.clear_sync_state_locked();
        }
        self.set_claude_access(false)
    }

    pub fn cloud_account(&self) -> Result<CloudAccount, PennelError> {
        self.cloud_session()?.account()
    }

    pub fn cloud_devices(&self) -> Result<Vec<CloudDevice>, PennelError> {
        self.cloud_session()?.devices()
    }

    pub fn cloud_rename_device(&self, id: String, name: String) -> Result<(), PennelError> {
        self.cloud_session()?.rename_device(&id, &name)
    }

    pub fn cloud_revoke_device(&self, id: String) -> Result<(), PennelError> {
        self.cloud_session()?.revoke_device(&id)
    }

    pub fn cloud_delete_account(&self) -> Result<(), PennelError> {
        if crate::cloud::link::has_pending(self) {
            self.cloud_link_cancel()?;
            if crate::cloud::link::has_pending(self) {
                return Err(PennelError::Cloud {code:"pairing_pending".into(),message:"Finish or cancel the current device connection before deleting the account.".into()});
            }
        }
        let _identity = crate::speaker_jobs::speaker_identity_guard();
        self.ensure_cloud_identity_mutable()?;
        {
            let _sync = self.sync_gate.lock().unwrap();
            let _meeting = self.meeting_gate.lock().unwrap();
            self.ensure_cloud_identity_mutable()?;
            self.cloud_session()?.delete_account()?;
            self.clear_sync_state_locked();
        }
        self.set_claude_access(false)
    }

    /// Creates the vault and returns the recovery code to show once. When the account already
    /// has a vault this gives `VaultLocked`: ask for the recovery code instead.
    pub fn cloud_sync_enable(&self) -> Result<String, PennelError> {
        let _gate = self.sync_gate.lock().unwrap();
        let api = self.signed_in_sync_api()?;
        let vault = api.vault()?;
        if vault.wrapped.is_some() {
            return Err(PennelError::VaultLocked);
        }
        let key = VaultKey::generate();
        let code = crypto::new_recovery_code();
        let (wrapped, kdf) = crypto::wrap(&key, &code)?;
        api.put_vault(vault.generation, &wrapped, &kdf, true)?;
        self.adopt_vault(&key, vault.generation)?;
        Ok(code)
    }

    pub fn cloud_sync_unlock(&self, code: String) -> Result<(), PennelError> {
        let _gate = self.sync_gate.lock().unwrap();
        let api = self.signed_in_sync_api()?;
        let vault = api.vault()?;
        let (Some(wrapped), Some(kdf)) = (&vault.wrapped, &vault.kdf) else {
            return Err(PennelError::Cloud {
                code: "vault_missing".into(),
                message: "No vault exists yet.".into(),
            });
        };
        let key = crypto::unwrap(wrapped, kdf, &code)?;
        if kdf.key_check.is_none() {
            let store = self.secret_store().ok_or(PennelError::NotSignedIn)?;
            crate::cloud::store_verified(
                store.as_ref(),
                crate::cloud::VAULT_VALIDATION_ACCOUNT,
                &crate::cloud::setup::validation_marker(&key, &vault),
            )?;
        }
        self.adopt_vault(&key, vault.generation)
    }

    /// Wraps the same vault key under a new code; the old code stops working.
    pub fn cloud_sync_new_recovery_code(&self) -> Result<String, PennelError> {
        let _gate = self.sync_gate.lock().unwrap();
        let api = self.signed_in_sync_api()?;
        let key = self.vault_key().ok_or(PennelError::VaultLocked)?;
        let vault = api.vault()?;
        // Wrapping a key kept from another account would lock that account out of its vault.
        if vault
            .kdf
            .as_ref()
            .and_then(|kdf| kdf.key_check.as_deref())
            .is_some_and(|expected| expected != crypto::vault_check(&key))
        {
            if let Some(secrets) = self.secret_store() {
                secrets.set(VAULT_KEY_ACCOUNT.into(), None);
            }
            return Err(PennelError::VaultLocked);
        }
        if self.with_sync_state(|state| state.generation())? != Some(vault.generation) {
            return Err(PennelError::Cloud {
                code: "generation".into(),
                message: "The vault was reset on another device.".into(),
            });
        }
        let code = crypto::new_recovery_code();
        let (wrapped, kdf) = crypto::wrap(&key, &code)?;
        api.put_vault(vault.generation, &wrapped, &kdf, false)?;
        Ok(code)
    }

    /// Wipes the cloud copy and starts a new vault; local files stay and upload again.
    pub fn cloud_sync_reset(&self) -> Result<String, PennelError> {
        let _gate = self.sync_gate.lock().unwrap();
        let api = self.signed_in_sync_api()?;
        let vault = api.vault()?;
        let generation = api.reset_vault(vault.generation)?;
        let key = VaultKey::generate();
        let code = crypto::new_recovery_code();
        let (wrapped, kdf) = crypto::wrap(&key, &code)?;
        api.put_vault(generation, &wrapped, &kdf, true)?;
        self.adopt_vault(&key, generation)?;
        Ok(code)
    }

    /// Stops syncing; the vault key and sync state stay for when it is turned on again. A pass
    /// already running finishes, but its report no longer counts.
    pub fn cloud_sync_disable(&self) -> Result<(), PennelError> {
        self.set_cloud_sync(false)?;
        *self.sync_last.lock().unwrap() = None;
        Ok(())
    }

    /// One sync pass. Never fails: problems are the report's status.
    pub fn cloud_sync_run(&self, recording_meeting_ids: Vec<String>) -> SyncReport {
        let report = self.sync_pass(&recording_meeting_ids);
        *self.sync_last.lock().unwrap() = Some(report.clone());
        report
    }

    /// Without a pass since start-up, whether a vault exists is unknown: no key reads as
    /// `NeedsSetup` until a pass asks the server.
    pub fn cloud_sync_last(&self) -> SyncReport {
        if crate::cloud::link::has_pending(self) {
            return self.report(SyncStatus::Pending);
        }
        let status = if !self.settings().cloud_sync {
            SyncStatus::Off
        } else if !self.cloud_signed_in() {
            SyncStatus::SignedOut
        } else if let Some(report) = self.sync_last.lock().unwrap().clone() {
            return report;
        } else if self.vault_key().is_none() {
            SyncStatus::NeedsSetup
        } else {
            SyncStatus::Pending
        };
        self.report(status)
    }

    /// The user confirmed that the meetings the brake held back were deleted on purpose.
    /// Never waits for a running pass: the next one picks the confirmation up.
    pub fn cloud_sync_confirm_deletes(&self) -> Result<(), PennelError> {
        match self.try_sync_state(SyncState::confirm_deletes) {
            Some(result) => result,
            None => {
                *self.pending_confirm.lock().unwrap() = true;
                Ok(())
            }
        }
    }

    /// Never waits for a running pass: then the states from before it are returned.
    pub fn meeting_sync_states(&self) -> Vec<MeetingSyncState> {
        let settings = self.settings();
        if !settings.cloud_sync {
            return Vec::new();
        }
        let root = library_root(&settings);
        match self.try_sync_state(|state| sync::item_states(&root, state)) {
            Some(Ok(states)) => {
                *self.sync_items.lock().unwrap() = states.clone();
                states
            }
            _ => self.sync_items.lock().unwrap().clone(),
        }
    }

    pub fn cloud_folder_warning(&self) -> Option<String> {
        sync::folder_warning(&library_root(&self.settings()), &home_dir())
    }

    pub fn set_system_language_model(&self, model: Arc<dyn SystemLanguageModel>) {
        *self.system_llm.write().unwrap() = Some(model);
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    pub fn update_settings(&self, settings: Settings) -> Result<(), PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        self.update_settings_locked(settings)
    }

    pub fn dictate(&self, request: DictationRequest) -> Result<DictationResult, PennelError> {
        self.dictate_with_settings(request, self.settings())
    }

    /// Practice exercises recognition without calling the user's normal cleanup provider.
    pub fn dictate_practice(
        &self,
        request: DictationRequest,
    ) -> Result<DictationResult, PennelError> {
        let mut settings = self.settings();
        settings.mode = DictationMode::Fast;
        self.dictate_with_settings(request, settings)
    }

    pub fn begin_dictation_session(
        self: Arc<Self>,
        observer: Arc<dyn DictationPreviewObserver>,
        practice: bool,
    ) -> Arc<LiveDictationSession> {
        LiveDictationSession::new(self.clone(), observer, self.settings(), practice)
    }

    pub fn dictation_pad_text(&self) -> String {
        self.dictation_pad.lock().unwrap().text()
    }

    pub fn save_dictation_pad(&self, text: String) -> Result<(), PennelError> {
        self.dictation_pad.lock().unwrap().save(text)
    }

    pub fn append_dictation_pad(&self, text: String) -> Result<String, PennelError> {
        self.dictation_pad.lock().unwrap().append(text)
    }

    /// Speaks an instruction that rewrites `selection`, or writes new text when nothing is selected.
    pub fn command(
        &self,
        request: DictationRequest,
        selection: Option<String>,
    ) -> Result<DictationResult, PennelError> {
        self.command_with_settings_observed(request, selection, self.settings(), None)
    }

    pub fn keyboard_writing_configuration(&self) -> String {
        let _capture = self.inference_capture_gate();
        crate::keyboard_writing::configuration(self, &self.settings())
    }

    /// An explicit, one-use preview job; ordinary typing never enters this provider path.
    pub fn prepare_keyboard_writing(
        self: Arc<Self>,
        action: crate::KeyboardRewriteAction,
        selection: String,
        expected_configuration: String,
        keys: ApiKeys,
    ) -> Result<Arc<crate::KeyboardWritingJob>, PennelError> {
        let system_model = self.system_llm.read().unwrap().clone();
        crate::keyboard_writing::KeyboardWritingJob::prepare(
            self,
            action,
            selection,
            expected_configuration,
            keys,
            system_model,
        )
    }

    pub fn retry(&self, history_id: String, keys: ApiKeys) -> Result<DictationResult, PennelError> {
        let retained = self
            .unsaved_history
            .lock()
            .unwrap()
            .entries
            .get(&history_id)
            .cloned();
        if let Some(retained) = retained {
            self.save_history(history_id.clone())?;
            return Ok(DictationResult {
                history_id,
                history_saved: true,
                text: retained.entry.text,
                raw: retained.entry.raw,
                cleanup: retained.cleanup,
            });
        }
        let _slot = self.reserve_history()?;
        let settings = self.settings();
        let (wav, app, created_at_ms, provenance) = {
            let history = self.history.lock().unwrap();
            (
                history.failed_audio(&history_id)?,
                history.app_bundle_id(&history_id)?,
                history.created_at_ms(&history_id)?,
                history.capture_provenance(&history_id)?,
            )
        };
        let transcript = self.process(&settings, &wav, app.as_deref(), &keys)?;
        self.persist_transcript(
            HistoryEntry {
                id: history_id,
                created_at_ms,
                app_bundle_id: app,
                raw: transcript.raw,
                text: transcript.text,
                status: HistoryStatus::Done,
                error: None,
                has_audio: true,
            },
            transcript.cleanup,
            None,
            provenance.0,
            provenance.1,
        )
    }

    pub fn history(&self, limit: u32) -> Result<Vec<HistoryEntry>, PennelError> {
        let stored = self.history.lock().unwrap().list(limit);
        let recovery = self.unsaved_history.lock().unwrap();
        let mut entries = match stored {
            Ok(entries) => entries,
            Err(error) if recovery.entries.is_empty() => return Err(error),
            Err(_) => Vec::new(),
        };
        entries.retain(|entry| !recovery.entries.contains_key(&entry.id));
        let mut temporary: Vec<_> = recovery
            .entries
            .values()
            .map(|result| result.entry.clone())
            .collect();
        temporary.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(b.id.cmp(&a.id)));
        temporary.extend(entries);
        temporary.truncate(limit as usize);
        Ok(temporary)
    }

    pub fn history_entry(&self, id: String) -> Result<HistoryEntry, PennelError> {
        if let Some(result) = self.unsaved_history.lock().unwrap().entries.get(&id) {
            return Ok(result.entry.clone());
        }
        self.history.lock().unwrap().entry(&id)
    }

    pub fn delete_history(&self, id: String) -> Result<(), PennelError> {
        if self
            .unsaved_history
            .lock()
            .unwrap()
            .entries
            .remove(&id)
            .is_some()
        {
            // Explicitly discarding a temporary result must also work while SQLite is unavailable.
            let _ = self.history.lock().unwrap().delete(&id);
            return Ok(());
        }
        self.history.lock().unwrap().delete(&id)
    }

    pub fn ensure_history_capacity(&self) -> Result<(), PennelError> {
        self.unsaved_history.lock().unwrap().ensure_capacity()
    }

    pub fn unsaved_history_count(&self) -> u32 {
        self.unsaved_history.lock().unwrap().entries.len() as u32
    }

    /// Saves retained text without recognition, cleanup, or external delivery.
    pub fn save_history(&self, id: String) -> Result<(), PennelError> {
        let mut recovery = self.unsaved_history.lock().unwrap();
        let Some(result) = recovery.entries.get(&id) else {
            return if self.history.lock().unwrap().is_done(&id)? {
                Ok(())
            } else {
                Err(PennelError::NotFound)
            };
        };
        let entry = &result.entry;
        self.history.lock().unwrap().insert_done_with_provenance(
            &entry.id,
            entry.app_bundle_id.as_deref(),
            &entry.raw,
            &entry.text,
            entry.created_at_ms,
            result.capture_kind,
            result.profile_allowed,
        )?;
        recovery.entries.remove(&id);
        Ok(())
    }

    pub fn retry_unsaved_history(&self) -> Result<(), PennelError> {
        let ids: Vec<String> = self
            .unsaved_history
            .lock()
            .unwrap()
            .entries
            .keys()
            .cloned()
            .collect();
        let mut first_error = None;
        for id in ids {
            if let Err(error) = self.save_history(id) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Called periodically by the app so an idle local model does not stay in memory.
    pub fn unload_idle_models(&self) -> bool {
        let minutes = u64::from(self.settings.read().unwrap().local_unload_minutes);
        self.local.unload_if_idle(
            std::time::Duration::from_secs(minutes * 60),
            std::time::Instant::now(),
        )
    }

    pub fn start_meeting(
        self: Arc<Self>,
        title: String,
        keys: ApiKeys,
    ) -> Result<Arc<MeetingSession>, PennelError> {
        self.start_meeting_with(title, Vec::new(), keys)
    }

    /// Check configuration without contacting providers or creating a meeting folder.
    pub fn validate_meeting_setup(&self, keys: ApiKeys) -> Result<(), PennelError> {
        let settings = self.settings();
        let (account, model) = settings
            .resolve(&settings.meeting_stt)
            .ok_or(PennelError::NotFound)?;
        if model.trim().is_empty() || !supports(account.kind, ProviderTask::MeetingStt) {
            return Err(PennelError::NotFound);
        }
        if requires_key(account.kind) && keys.get(&account).is_none() {
            return Err(PennelError::MissingApiKey {
                task: "meeting_stt".into(),
            });
        }
        if account.kind == ProviderKind::Local {
            if self.models.state(&model) != ModelState::Downloaded {
                return Err(PennelError::ModelNotDownloaded);
            }
            if !self
                .models
                .local_models(None, false)
                .iter()
                .any(|entry| entry.id == model)
            {
                return Err(PennelError::NotFound);
            }
        }
        if account.kind == ProviderKind::AppleSpeech && self.system_stt.read().unwrap().is_none() {
            return Err(PennelError::InvalidResponse {
                message: "Apple dictation is unavailable on this computer".into(),
            });
        }
        Ok(())
    }

    pub fn start_meeting_with(
        self: Arc<Self>,
        title: String,
        attendees: Vec<String>,
        keys: ApiKeys,
    ) -> Result<Arc<MeetingSession>, PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        if self.has_active_session() {
            return Err(PennelError::Storage {
                message: "a meeting is already recording or being finalized".into(),
            });
        }
        self.claim_library()?;
        let store = self.meeting_store();
        let id = store.create(&title, chrono::Local::now())?;
        store.set_attendees(&id, &attendees)?;
        let session = MeetingSession::start(self.clone(), store, id, keys);
        let priority = self.meeting_preview_priority.lock().unwrap();
        session.set_preview_paused(*priority > 0);
        self.active_sessions
            .lock()
            .unwrap()
            .push(Arc::downgrade(&session));
        drop(priority);
        Ok(session)
    }

    pub fn meetings(&self) -> Vec<MeetingInfo> {
        self.meeting_store().list()
    }

    /// Title matches first, then meetings in full-text rank order.
    pub fn search_meetings(&self, query: String) -> Vec<MeetingInfo> {
        self.search_library(query).meetings
    }

    pub fn search_passages(&self, query: String) -> Vec<Passage> {
        self.search_library(query).passages
    }

    /// Matching meetings plus up to two snippets each, from one index sync.
    pub fn search_library(&self, query: String) -> LibrarySearch {
        let store = self.meeting_store();
        if query.trim().is_empty() {
            return LibrarySearch {
                meetings: store.list(),
                passages: Vec::new(),
            };
        }
        let Some(passages) =
            self.with_index(&store, |index| index.search_grouped(&store, &query, 2))
        else {
            return LibrarySearch {
                meetings: store.search(&query),
                passages: Vec::new(),
            };
        };
        let needle = query.trim().to_lowercase();
        let all = store.list();
        let mut meetings: Vec<MeetingInfo> = all
            .iter()
            .filter(|m| m.title.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        for passage in &passages {
            if !meetings.iter().any(|m| m.id == passage.meeting_id)
                && let Some(info) = all.iter().find(|m| m.id == passage.meeting_id)
            {
                meetings.push(info.clone());
            }
        }
        LibrarySearch { meetings, passages }
    }

    pub fn meeting_info(&self, id: String) -> Option<MeetingInfo> {
        self.meeting_store().info(&id)
    }

    pub fn meeting_notes(&self, id: String) -> String {
        self.meeting_store().read_notes(&id)
    }

    pub fn set_meeting_notes(&self, id: String, text: String) -> Result<(), PennelError> {
        self.save_owned_meeting_notes(&id, &text, None, None)
            .map(|_| ())
    }

    pub fn meeting_markdown(&self, id: String) -> Option<String> {
        self.meeting_store().read_meeting(&id)
    }

    pub fn meeting_transcript(&self, id: String) -> Vec<Segment> {
        self.meeting_store().read_transcript(&id)
    }

    pub fn meeting_transcript_snapshot_json(&self, id: String) -> Result<String, PennelError> {
        let snapshot = self.meeting_store().read_transcript_snapshot(&id)?;
        meeting_transcript_projection_json(&snapshot)
    }

    pub fn meeting_utterance_audio_json(
        &self,
        id: String,
        utterance_id: String,
    ) -> Result<String, PennelError> {
        let snapshot = self.meeting_store().read_transcript_snapshot(&id)?;
        if !matches!(
            snapshot.metadata_status,
            crate::TranscriptMetadataStatus::Valid | crate::TranscriptMetadataStatus::Absent
        ) {
            return Err(PennelError::InvalidResponse { message: "The transcript's source metadata is unavailable. Choose an audio track explicitly.".into() });
        }
        let row = snapshot
            .utterances
            .iter()
            .find(|row| row.metadata.utterance_id == utterance_id)
            .ok_or(PennelError::NotFound)?;
        let source =
            match row.metadata.source_id.as_deref() {
                Some("microphone") => Speaker::Me,
                Some("system" | "system_audio" | "room_microphone") => Speaker::Others,
                _ => return Err(PennelError::InvalidResponse {
                    message:
                        "The physical audio source is unknown. Choose an audio track explicitly."
                            .into(),
                }),
            };
        serde_json::to_string(&serde_json::json!({"source":source,"source_id":row.metadata.source_id,"start_ms":row.segment.start_ms}))
            .map_err(PennelError::storage)
    }

    pub fn meeting_dir(&self, id: String) -> String {
        self.meeting_store()
            .dir_of(&id)
            .to_string_lossy()
            .into_owned()
    }

    pub fn rename_meeting(&self, id: String, title: String) -> Result<String, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        self.meeting_store().rename(&id, &title)
    }

    pub fn delete_meeting(&self, id: String) -> Result<(), PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let store = self.meeting_store();
        let uid = store.uid_of(&id);
        let conversation_uid = uid.clone().or_else(|| {
            store
                .read_transcript_snapshot(&id)
                .ok()
                .map(|snapshot| snapshot.meeting_uid)
        });
        if let Some(uid) = uid.as_deref() {
            self.discard_speaker_jobs_for(uid)?;
        }
        store.delete(&id)?;
        if let Some(uid) = conversation_uid.as_deref() {
            self.discard_meeting_conversations(uid);
        }
        let _ = self
            .voice_store()
            .forget_meeting(uid.as_deref().unwrap_or(&id));
        // Deleted in Pennel itself: sync may push this deletion past the mass-delete brake.
        // Kept in memory until the next pass, so deleting never waits for a running one.
        if let Some(uid) = uid
            && self.settings().cloud_sync
            && self.cloud_signed_in()
        {
            self.deleted_meetings.lock().unwrap().push(uid);
        }
        Ok(())
    }

    pub fn prune_meeting_audio(&self) -> u32 {
        if self.claim_library().is_err()
            || self.has_active_session()
            || !self.meeting_jobs.lock().unwrap().is_empty()
            || self.active_speaker_jobs()
        {
            return 0;
        }
        let days = self.settings.read().unwrap().meeting_audio_retention_days;
        self.meeting_store()
            .prune_audio(days, std::time::SystemTime::now())
    }

    /// Retries chunks whose transcription failed during the meeting; returns how many were recovered.
    pub fn retranscribe_failed(&self, id: String, keys: ApiKeys) -> Result<u32, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let store = self.meeting_store();
        let settings = self.settings();
        let snapshot = store.read_transcript_snapshot(&id)?;
        if !matches!(
            snapshot.metadata_status,
            crate::TranscriptMetadataStatus::Valid | crate::TranscriptMetadataStatus::Absent
        ) {
            return Err(PennelError::Storage {
                message: "Restore the transcript's source metadata before retrying recognition."
                    .into(),
            });
        }
        let mut rows = snapshot.utterances;
        let mut recovered = 0;
        let mut retained_partial = false;
        let mut unverified = false;
        // Keep corrupt chunks for review without blocking independent retained audio.
        let mut batch_error = None;
        for row in rows.iter_mut().filter(|row| row.segment.failed) {
            use crate::FailedTranscriptionRecoveryReason as Reason;
            let path = match store.recovery_audio_candidate(&id, row) {
                Ok(path) => path,
                Err(Reason::UnverifiedProvenance | Reason::MetadataUnavailable) => {
                    unverified = true;
                    continue;
                }
                Err(Reason::NoRetainedAudio | Reason::UnreadableAudio) => continue,
                Err(Reason::AudioChanged) => {
                    batch_error = Some(PennelError::storage(
                        "The recovery audio changed before recognition.",
                    ));
                    continue;
                }
            };
            let mut proof = row
                .metadata
                .recovery_audio
                .clone()
                .expect("verified recovery provenance");
            let Ok(wav) = crate::meetings::read_recovery_bytes(&path) else {
                continue;
            };
            let actual_sha = crate::meeting_utterances::digest(&wav);
            if proof.sha256.is_empty() {
                proof.sha256 = actual_sha;
            } else if actual_sha != proof.sha256 {
                batch_error = Some(PennelError::storage(
                    "The recovery audio changed before recognition.",
                ));
                continue;
            }
            let transcript = match self.stt_meeting(&settings, &wav, &keys) {
                Ok(transcript) => transcript,
                Err(error) => {
                    // Stop service work and retain actionable provider/account guidance.
                    batch_error = Some(meeting_retry_error(error));
                    break;
                }
            };
            let actual = crate::meetings::recovery_provenance(&path, proof.disposition);
            if !actual.as_ref().is_some_and(|actual| {
                crate::meetings::recovery_audio_matches(&path, &proof, actual)
            }) {
                batch_error = Some(PennelError::storage(
                    "The recovery audio changed during recognition.",
                ));
                continue;
            }
            proof.end_ms = actual.expect("verified recovery audio").end_ms;
            let segment = &mut row.segment;
            let retained = !is_hallucination(&transcript.text);
            if !transcript.complete && (!retained || transcript.text.len() < segment.text.len()) {
                batch_error = Some(PennelError::InvalidResponse {
                    message: "Recognition stopped before the passage ended. The previous text and original audio remain available for retry.".into(),
                });
                break;
            }
            if let Some((samples, rate)) = decode_wav(&wav) {
                segment.end_ms = segment
                    .start_ms
                    .saturating_add(crate::wav::sample_duration_ms(samples.len() as u64, rate));
            }
            segment.text = if !retained {
                String::new()
            } else {
                transcript.text
            };
            segment.failed = !transcript.complete;
            proof.disposition = if transcript.complete {
                crate::meeting_utterances::RecoveryAudioDisposition::Complete
            } else {
                crate::meeting_utterances::RecoveryAudioDisposition::Pending
            };
            row.metadata.partial_text = (!transcript.complete).then(|| segment.text.clone());
            row.metadata.recovery_audio = Some(proof);
            row.metadata.spans = if retained {
                transcript
                    .spans
                    .into_iter()
                    .map(|span| {
                        crate::TimedTextSpan(
                            span.start_byte,
                            span.end_byte,
                            segment.start_ms.saturating_add(span.start_ms),
                            segment.start_ms.saturating_add(span.end_ms),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            row.metadata.refresh_fingerprints(segment);
            if let Some(manual) = row.metadata.manual.as_mut()
                && manual.source_fingerprint != row.metadata.source_fingerprint
            {
                manual.review_required = true;
            }
            if transcript.complete {
                recovered += 1;
            } else {
                retained_partial = true;
                batch_error = Some(PennelError::InvalidResponse {
                    message: "Recognition stopped before the passage ended. The partial text and original audio remain available for retry.".into(),
                });
                break;
            }
        }
        if recovered == 0
            && !retained_partial
            && let Some(error) = batch_error
        {
            return Err(error);
        }
        // Keep recovered text durable before reporting a failed batch; later retries skip this prefix.
        store.write_transcript_snapshot(&id, Some(snapshot.revision), &rows)?;
        store.restore_recovery_duration(&id)?;
        let _ = store.cleanup_committed_audio(&id);
        if let Some(error) = batch_error {
            return Err(error);
        }
        if recovered == 0 && unverified {
            return Err(PennelError::storage(
                "Some recovery audio cannot be linked to its original transcript. Keep the files and review their sources before retrying recognition.",
            ));
        }
        Ok(recovered)
    }

    /// Writes `meeting.md`: enhanced notes when the user typed any, otherwise a summary.
    pub fn summarize_meeting(
        &self,
        id: String,
        template_id: String,
        keys: ApiKeys,
    ) -> Result<String, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let candidate = self.prepare_owner_summary(&id, &template_id, &keys)?;
        self.commit_generated_summary(&id, &template_id, &candidate)
    }

    /// Answers from one meeting (`meeting_id`) or all meetings, citing the passages it used.
    pub fn ask_meetings(
        &self,
        question: String,
        history: Vec<ChatTurn>,
        meeting_id: Option<String>,
        keys: ApiKeys,
    ) -> Result<MeetingAnswer, PennelError> {
        use crate::meeting_chat::*;

        let settings = self.settings();
        let (account, model) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        let key = self.credential(&account, &keys)?;
        if requires_key(account.kind) && key.is_none() {
            return Err(PennelError::MissingApiKey { task: "llm".into() });
        }
        let store = self.meeting_store();
        let sources = self
            .with_index(&store, |index| {
                select_sources(
                    index,
                    &store,
                    &question,
                    &history,
                    meeting_id.as_deref(),
                    account.kind == ProviderKind::AppleIntelligence,
                )
                .map(|(sources, _)| sources)
            })
            .ok_or(PennelError::Storage {
                message: "search index unavailable".into(),
            })?;
        self.prepare_source_question(
            question,
            history,
            sources,
            QuestionPreparation {
                provider: self.endpoint(&account, model),
                key,
                system: self.system_llm.read().unwrap().clone(),
                admission: crate::ForegroundQuestionAdmission::new(
                    crate::InferenceCancellation::new(),
                    std::time::Instant::now()
                        + std::time::Duration::from_secs(crate::meeting_chat::ASK_TIMEOUT_SECS),
                ),
                registration: None,
                source_guard: None,
                cloud_guard: None,
            },
        )?
        .complete(crate::InferenceCancellation::new(), ())
    }

    pub fn export_meeting(
        &self,
        id: String,
        include_transcript: bool,
    ) -> Result<MeetingExport, PennelError> {
        self.export_meeting_localized(id, include_transcript, crate::InterfaceLanguage::English)
    }

    pub fn export_meeting_localized(
        &self,
        id: String,
        include_transcript: bool,
        language: crate::InterfaceLanguage,
    ) -> Result<MeetingExport, PennelError> {
        let (title, markdown) = crate::export::export_markdown_for_language(
            &self.meeting_store(),
            &id,
            include_transcript,
            language,
        )
        .ok_or(PennelError::NotFound)?;
        Ok(MeetingExport {
            html: crate::export::markdown_to_html(&markdown),
            title,
            markdown,
        })
    }

    pub fn diarizer_model_id(&self) -> String {
        crate::models::DIARIZER_ID.to_owned()
    }

    /// Splits the meeting's "Anderen" segments into voices from the full others-track (16 kHz mono);
    /// returns how many voices were found. Kept for the macOS app; see `recognize_speakers`.
    pub fn diarize_meeting(&self, id: String, samples: Vec<f32>) -> Result<u32, PennelError> {
        self.recognize_speakers(id, None, Some(samples))
            .map(|recognition| recognition.voices)
    }

    pub fn meeting_attendees(&self, id: String) -> Vec<String> {
        self.meeting_store().attendees(&id)
    }

    pub fn set_meeting_attendees(
        &self,
        id: String,
        attendees: Vec<String>,
    ) -> Result<(), PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        store.set_attendees(&id, &attendees)
    }

    pub fn meeting_speakers(&self, id: String) -> Vec<SpeakerName> {
        self.meeting_store().speaker_names(&id)
    }

    pub fn rename_speaker(&self, id: String, voice: u32, name: String) -> Result<(), PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        self.meeting_store().rename_speaker(&id, voice, &name)
    }

    pub fn meeting_templates(&self) -> Vec<MeetingTemplate> {
        self.template_store().list()
    }

    pub fn template_source(&self, id: String) -> Option<String> {
        self.template_store().source(&id)
    }

    pub fn save_template(
        &self,
        id: Option<String>,
        name: String,
        body: String,
    ) -> Result<String, PennelError> {
        self.claim_library()?;
        self.template_store().save(id.as_deref(), &name, &body)
    }

    pub fn delete_template(&self, id: String) -> Result<(), PennelError> {
        self.claim_library()?;
        self.template_store().delete(&id)
    }

    pub fn local_models(&self, language: Option<String>, supported_only: bool) -> Vec<LocalModel> {
        if cfg!(not(feature = "local-models")) {
            return Vec::new();
        }
        self.models
            .local_models(language.as_deref(), supported_only)
    }

    pub fn model_state(&self, id: String) -> ModelState {
        self.models.state(&id)
    }

    pub fn active_model_downloads(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.downloads.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Attaches to an existing worker only; a stale UI snapshot cannot restart a download.
    pub fn join_model_download(
        &self,
        id: String,
        listener: Arc<dyn DownloadListener>,
    ) -> Result<bool, PennelError> {
        let task = self.downloads.lock().unwrap().get(&id).cloned();
        let Some(task) = task else {
            return Ok(false);
        };
        task.subscribe(listener);
        task.wait().map(|()| true)
    }

    /// Blocks until the download finishes, fails or is cancelled via `cancel_download`.
    pub fn download_model(
        &self,
        id: String,
        listener: Arc<dyn DownloadListener>,
    ) -> Result<(), PennelError> {
        if cfg!(not(feature = "local-models")) {
            return Err(PennelError::local_models_unavailable());
        }
        let (task, owner) = {
            let mut downloads = self.downloads.lock().unwrap();
            match downloads.entry(id.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => (entry.get().clone(), false),
                std::collections::hash_map::Entry::Vacant(entry) => (
                    entry.insert(Arc::new(DownloadTask::default())).clone(),
                    true,
                ),
            }
        };
        task.subscribe(listener);
        if !owner {
            return task.wait();
        }
        let result = self.models.download(&id, task.as_ref(), &task.cancel);
        task.finish(result.clone());
        let mut downloads = self.downloads.lock().unwrap();
        if downloads
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, &task))
        {
            downloads.remove(&id);
        }
        result
    }

    pub fn cancel_download(&self, id: String) {
        if let Some(task) = self.downloads.lock().unwrap().get(&id) {
            task.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Also moves dictation off the deleted model when another local model is available.
    pub fn delete_model(&self, id: String) -> Result<(), PennelError> {
        let downloads = self.downloads.lock().unwrap();
        if downloads.contains_key(&id) {
            return Err(PennelError::Storage {
                message: "Wait until the model download has finished or been cancelled.".into(),
            });
        }
        self.models.delete(&id)?;
        drop(downloads);
        let settings = self.settings();
        self.update_settings(settings)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct LibrarySearch {
    pub meetings: Vec<MeetingInfo>,
    pub passages: Vec<Passage>,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn library_root(settings: &Settings) -> PathBuf {
    if settings.meetings_dir.is_empty() {
        home_dir().join("Pennel")
    } else {
        PathBuf::from(&settings.meetings_dir)
    }
}

fn enforce_preview_library(
    data_dir: &std::path::Path,
    settings: &mut Settings,
) -> Result<(), PennelError> {
    let expected = data_dir.join("Meetings");
    if (!settings.meetings_dir.is_empty()
        && std::path::Path::new(&settings.meetings_dir) != expected.as_path())
        || expected
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(PennelError::Storage { message: "the preview uses only its own Meetings folder; external meeting folders are not allowed".into() });
    }
    settings.meetings_dir = expected.to_string_lossy().into_owned();
    Ok(())
}

fn engines() -> &'static Mutex<HashMap<PathBuf, Weak<PennelEngine>>> {
    static ENGINES: OnceLock<Mutex<HashMap<PathBuf, Weak<PennelEngine>>>> = OnceLock::new();
    ENGINES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open_index(path: &std::path::Path) -> Option<MeetingIndex> {
    MeetingIndex::open(path).ok().or_else(|| {
        let _ = std::fs::remove_file(path);
        MeetingIndex::open(path).ok()
    })
}

fn meeting_transcript_projection_json(
    snapshot: &crate::MeetingTranscriptSnapshot,
) -> Result<String, PennelError> {
    serde_json::to_string(&serde_json::json!({
        "meeting_uid": &snapshot.meeting_uid,
        "revision": snapshot.revision,
        "metadata_status": snapshot.metadata_status,
        "transcript": crate::transcript_projection::rows(&snapshot.utterances)?,
    }))
    .map_err(PennelError::storage)
}

impl PennelEngine {
    /// Rust-only detail projection and recovery eligibility from one authoritative snapshot.
    pub fn meeting_transcript_snapshot_with_recovery_plan(
        &self,
        id: &str,
    ) -> Result<(String, crate::FailedTranscriptionRecoveryPlan), PennelError> {
        let store = self.meeting_store();
        let snapshot = store.read_transcript_snapshot(id)?;
        let plan = store.failed_recovery_plan(id, &snapshot);
        Ok((meeting_transcript_projection_json(&snapshot)?, plan))
    }

    pub fn apply_task_model_choice(
        &self,
        task: ProviderTask,
        mut choice: ModelChoice,
    ) -> Result<(), PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        let mut settings = self.settings();
        let (account, _) = settings.resolve(&choice).ok_or(PennelError::NotFound)?;
        if !supports(account.kind, task) {
            return Err(PennelError::InvalidResponse {
                message: "This provider does not support the selected task.".into(),
            });
        }
        choice.model = choice.model.trim().to_owned();
        if choice.model.is_empty() && !account.kind.is_cli_agent() {
            return Err(PennelError::InvalidResponse {
                message: "Choose a model to test first.".into(),
            });
        }
        if account.kind == ProviderKind::Local && !self.models.downloaded().contains(&choice.model)
        {
            return Err(PennelError::ModelNotDownloaded);
        }
        if task == ProviderTask::DictationStt {
            for configured in [settings.dictation_stt.clone(), choice.clone()] {
                if !settings.dictation_models.contains(&configured) {
                    settings.dictation_models.push(configured);
                }
            }
        }
        let selected = match task {
            ProviderTask::DictationStt => &mut settings.dictation_stt,
            ProviderTask::MeetingStt => &mut settings.meeting_stt,
            ProviderTask::Llm => &mut settings.llm,
        };
        if selected == &choice {
            return Ok(());
        }
        *selected = choice;
        // This operation owns one assignment; unrelated incomplete choices are not repaired here.
        self.persist_settings_locked(settings)
    }

    pub(crate) fn secret_store(&self) -> Option<Arc<dyn SecretStore>> {
        self.secrets.read().unwrap().clone()
    }

    pub(crate) fn current_cloud_session(&self) -> Option<Arc<Session>> {
        self.cloud_session.lock().unwrap().clone()
    }

    pub(crate) fn cloud_session(&self) -> Result<Arc<Session>, PennelError> {
        let mut slot = self.cloud_session.lock().unwrap();
        if let Some(session) = slot.as_ref() {
            return Ok(session.clone());
        }
        let secrets = self.secret_store().ok_or(PennelError::NotSignedIn)?;
        let session = Arc::new(Session::new(self.cloud_base_url(), secrets));
        *slot = Some(session.clone());
        Ok(session)
    }

    #[cfg(test)]
    pub(crate) fn use_cloud_session(&self, session: Session) {
        self.invalidate_inference_requests();
        *self.cloud_session.lock().unwrap() = Some(Arc::new(session));
    }

    pub(crate) fn endpoint(&self, account: &ProviderAccount, model: String) -> ProviderConfig {
        let base_url = if account.kind == ProviderKind::PennelCloud {
            let base = self
                .cloud_session()
                .map_or_else(|_| self.cloud_base_url(), |s| s.base_url().to_string());
            format!("{base}/v1")
        } else {
            account.base_url.clone()
        };
        ProviderConfig {
            kind: account.kind,
            base_url,
            model,
        }
    }

    /// The bearer for a request: the user's key, or a fresh Pennel Cloud access token.
    pub(crate) fn credential(
        &self,
        account: &ProviderAccount,
        keys: &ApiKeys,
    ) -> Result<Option<String>, PennelError> {
        if account.kind == ProviderKind::PennelCloud {
            return self.cloud_session()?.access_token().map(Some);
        }
        Ok(keys.get(account).map(str::to_owned))
    }

    pub(crate) fn cloud_base_url(&self) -> String {
        crate::cloud::base_url(self.preview)
    }

    pub(crate) fn data_dir(&self) -> PathBuf {
        self.settings_path
            .parent()
            .expect("data directory")
            .to_path_buf()
    }

    #[cfg(test)]
    pub(crate) fn use_sync_api(&self, api: Arc<dyn SyncApi>) {
        *self.sync_api_override.lock().unwrap() = Some(api);
    }

    pub(crate) fn sync_api(&self) -> Result<Arc<dyn SyncApi>, PennelError> {
        #[cfg(test)]
        if let Some(api) = self.sync_api_override.lock().unwrap().clone() {
            return Ok(api);
        }
        Ok(Arc::new(HttpSyncApi::new(self.cloud_session()?)))
    }

    pub(crate) fn signed_in_sync_api(&self) -> Result<Arc<dyn SyncApi>, PennelError> {
        if !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        self.sync_api()
    }

    pub(crate) fn with_sync_state<T>(
        &self,
        query: impl FnOnce(&SyncState) -> Result<T, PennelError>,
    ) -> Result<T, PennelError> {
        let mut slot = self.sync_state.lock().unwrap();
        if slot.is_none() {
            *slot = Some(SyncState::open(&self.data_dir())?);
        }
        query(slot.as_ref().expect("opened above"))
    }

    /// `None` while a pass holds the state.
    fn try_sync_state<T>(
        &self,
        query: impl FnOnce(&SyncState) -> Result<T, PennelError>,
    ) -> Option<Result<T, PennelError>> {
        let mut slot = match self.sync_state.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::WouldBlock) => return None,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        if slot.is_none() {
            match SyncState::open(&self.data_dir()) {
                Ok(state) => *slot = Some(state),
                Err(error) => return Some(Err(error)),
            }
        }
        Some(query(slot.as_ref().expect("opened above")))
    }

    /// The caller holds `sync_gate` while changing the account and forgetting its sync state.
    fn clear_sync_state_locked(&self) {
        let _ = self.with_sync_state(SyncState::clear);
        self.deleted_meetings.lock().unwrap().clear();
        *self.pending_device.lock().unwrap() = None;
        *self.pending_confirm.lock().unwrap() = false;
        *self.sync_last.lock().unwrap() = None;
    }

    /// Names this device in the conflict copies it causes elsewhere.
    pub(crate) fn remember_device_name(&self, name: &str) {
        let mut pending = self.pending_device.lock().unwrap();
        match self.try_sync_state(|state| state.set_device(name)) {
            Some(_) => *pending = None,
            None => *pending = Some(name.to_string()),
        }
    }

    pub(crate) fn vault_key(&self) -> Option<VaultKey> {
        self.secret_store()?
            .get(VAULT_KEY_ACCOUNT.into())
            .and_then(|encoded| VaultKey::from_b64(&encoded))
    }

    pub(crate) fn adopt_vault(&self, key: &VaultKey, generation: u32) -> Result<(), PennelError> {
        let secrets = self.secret_store().ok_or(PennelError::NotSignedIn)?;
        crate::cloud::store_verified(secrets.as_ref(), VAULT_KEY_ACCOUNT, &key.to_b64())?;
        self.with_sync_state(|state| {
            state.use_vault(&crypto::vault_check(key))?;
            state.set_generation(generation)
        })?;
        *self.sync_last.lock().unwrap() = None;
        self.set_cloud_sync(true)
    }

    pub(crate) fn set_cloud_sync(&self, enabled: bool) -> Result<(), PennelError> {
        self.update_settings_with(|mut settings| {
            settings.cloud_sync = enabled;
            Ok(settings)
        })
    }

    pub(crate) fn report(&self, status: SyncStatus) -> SyncReport {
        SyncReport {
            status,
            uploaded: 0,
            downloaded: 0,
            conflicts: 0,
            last_success_ms: self
                .try_sync_state(|state| state.last_success_ms())
                .and_then(Result::ok)
                .flatten(),
        }
    }

    fn sync_pass(&self, recording: &[String]) -> SyncReport {
        let _gate = self.sync_gate.lock().unwrap();
        if crate::cloud::link::has_pending(self) {
            return self.report(SyncStatus::Pending);
        }
        let mut recording = recording.to_vec();
        recording.extend(
            self.active_sessions
                .lock()
                .unwrap()
                .iter()
                .filter_map(Weak::upgrade)
                .filter(|session| !session.is_finalized())
                .map(|session| session.id()),
        );
        let recording = recording.as_slice();
        let settings = self.settings();
        if !settings.cloud_sync {
            return self.report(SyncStatus::Off);
        }
        if !self.cloud_signed_in() {
            return self.report(SyncStatus::SignedOut);
        }
        if let Err(error) = self.claim_library() {
            return self.report(SyncStatus::Error {
                message: error.to_string(),
            });
        }
        let api = match self.sync_api() {
            Ok(api) => api,
            Err(error) => return self.report(sync::status_for(error)),
        };
        let Some(key) = self.vault_key() else {
            let status = match api.vault() {
                Ok(vault) if vault.wrapped.is_some() => SyncStatus::NeedsRecoveryCode,
                Ok(_) => SyncStatus::NeedsSetup,
                Err(error) => sync::status_for(error),
            };
            return self.report(status);
        };
        let root = library_root(&settings);
        let data_dir = self.data_dir();
        let outcome = self.with_sync_state(|state| {
            let deleted = std::mem::take(&mut *self.deleted_meetings.lock().unwrap());
            if let Err(error) = deleted.iter().try_for_each(|uid| state.mark_deleted(uid)) {
                self.deleted_meetings.lock().unwrap().extend(deleted);
                return Err(error);
            }
            if let Some(name) = self.pending_device.lock().unwrap().take() {
                state.set_device(&name)?;
            }
            if std::mem::take(&mut *self.pending_confirm.lock().unwrap()) {
                state.confirm_deletes()?;
            }
            let device = state.device()?.unwrap_or_else(|| "Pennel".into());
            let mut report = sync::run(PassInput {
                api: api.as_ref(),
                state,
                key: &key,
                root: &root,
                data_dir: &data_dir,
                device: &device,
                recording,
            });
            if report.status == SyncStatus::Synced {
                state.set_last_success_ms(now_ms())?;
            }
            report.last_success_ms = state.last_success_ms()?;
            if let Ok(states) = sync::item_states(&root, state) {
                *self.sync_items.lock().unwrap() = states;
            }
            Ok(report)
        });
        let report = outcome.unwrap_or_else(|error| self.report(sync::status_for(error)));
        // Both mean the stored key does not open the account's current vault.
        if matches!(
            report.status,
            SyncStatus::VaultReset | SyncStatus::NeedsRecoveryCode
        ) && let Some(secrets) = self.secret_store()
        {
            secrets.set(VAULT_KEY_ACCOUNT.into(), None);
        }
        report
    }
}

impl PennelEngine {
    pub fn meeting_uid(&self, id: String) -> Option<String> {
        self.meeting_store().uid_of(&id)
    }

    pub fn meeting_utterance_snapshot(
        &self,
        id: String,
    ) -> Result<crate::MeetingTranscriptSnapshot, PennelError> {
        self.meeting_store().read_transcript_snapshot(&id)
    }

    pub fn meeting_id_for_uid(&self, uid: &str) -> Option<String> {
        let store = self.meeting_store();
        store
            .list()
            .into_iter()
            .find(|meeting| store.uid_of(&meeting.id).as_deref() == Some(uid))
            .map(|meeting| meeting.id)
    }

    pub(crate) fn conversation_live_session(&self, uid: &str) -> Option<Arc<MeetingSession>> {
        self.active_sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|session| {
                !session.is_finalized() && self.meeting_uid(session.id()).as_deref() == Some(uid)
            })
    }

    pub(crate) fn register_conversation_inference(
        &self,
        cancellation: &Arc<crate::InferenceCancellation>,
    ) -> u64 {
        let _gate = self.meeting_gate.lock().unwrap();
        let mut state = self.inference_requests.lock().unwrap();
        if state.mutation_depth > 0 {
            cancellation.cancel();
        }
        state.requests.retain(|request| request.strong_count() > 0);
        state.requests.push(Arc::downgrade(cancellation));
        state.generation
    }

    pub fn meeting_notes_checked(&self, id: &str) -> Result<String, PennelError> {
        match std::fs::read_to_string(self.meeting_store().dir_of(id).join("notes.md")) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(_) => Err(PennelError::Storage {
                message: "The saved meeting notes could not be read. Your question was not sent."
                    .into(),
            }),
        }
    }

    pub fn prepare_meeting_question(
        &self,
        question: String,
        history: Vec<ChatTurn>,
        sources: Vec<Passage>,
        keys: ApiKeys,
    ) -> Result<PreparedMeetingQuestion, PennelError> {
        let admission = crate::ForegroundQuestionAdmission::new(
            crate::InferenceCancellation::new(),
            std::time::Instant::now()
                + std::time::Duration::from_secs(crate::meeting_chat::ASK_TIMEOUT_SECS),
        );
        let settings = self.settings();
        let (account, model) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        let key = self.credential(&account, &keys)?;
        self.prepare_source_question(
            question,
            history,
            sources,
            QuestionPreparation {
                provider: self.endpoint(&account, model),
                key,
                system: self.system_llm.read().unwrap().clone(),
                admission,
                registration: None,
                source_guard: None,
                cloud_guard: None,
            },
        )
    }

    pub fn prepare_meeting_evidence_question(
        &self,
        question: String,
        history: Vec<ChatTurn>,
        evidence: &crate::MeetingEvidence,
        keys: ApiKeys,
    ) -> Result<PreparedMeetingQuestion, PennelError> {
        self.prepare_meeting_evidence_question_admitted(
            question,
            history,
            evidence,
            keys,
            crate::ForegroundQuestionAdmission::new(
                crate::InferenceCancellation::new(),
                std::time::Instant::now()
                    + std::time::Duration::from_secs(crate::meeting_chat::ASK_TIMEOUT_SECS),
            ),
        )
    }

    pub fn prepare_meeting_evidence_question_admitted(
        &self,
        question: String,
        history: Vec<ChatTurn>,
        evidence: &crate::MeetingEvidence,
        keys: ApiKeys,
        admission: crate::ForegroundQuestionAdmission,
    ) -> Result<PreparedMeetingQuestion, PennelError> {
        let cancellation = admission.cancellation();
        let (settings, registration, system) = {
            let _capture = self.inference_capture_gate();
            let state = self.inference_state();
            let mut active = state.lock().unwrap();
            if active.mutation_depth > 0
                || admission
                    .inference_epoch()
                    .is_some_and(|epoch| epoch != active.generation)
            {
                cancellation.cancel();
            }
            active.requests.retain(|request| request.strong_count() > 0);
            active.requests.push(Arc::downgrade(&cancellation));
            drop(active);
            (
                self.settings(),
                PreparedInferenceRegistration {
                    state,
                    cancellation: cancellation.clone(),
                },
                self.system_llm.read().unwrap().clone(),
            )
        };
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        if std::time::Instant::now() >= admission.deadline() {
            return Err(crate::inference::network());
        }
        let (account, model) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        validate_live_question_size(&question, account.kind)?;
        let cloud_guard = if account.kind == ProviderKind::PennelCloud {
            let session = self.cloud_session()?;
            let generation = session.generation();
            Some((session, generation))
        } else {
            None
        };
        let key = if let Some((session, generation)) = &cloud_guard {
            Some(session.inference_token(
                *generation,
                false,
                &cancellation,
                admission.deadline(),
            )?)
        } else {
            self.credential(&account, &keys)?
        };
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        if std::time::Instant::now() >= admission.deadline() {
            return Err(crate::inference::network());
        }
        let sources =
            evidence.selected_sources(&question, account.kind == ProviderKind::AppleIntelligence);
        let uid = admission
            .meeting_uid()
            .map(str::to_owned)
            .or_else(|| {
                sources.first().map(|source| {
                    source
                        .meeting_uid
                        .as_deref()
                        .unwrap_or(&source.meeting_id)
                        .to_owned()
                })
            })
            .ok_or_else(|| PennelError::InvalidResponse {
                message: "The meeting evidence has no valid identity.".into(),
            })?;
        let live = self
            .active_sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|session| self.meeting_uid(session.id()).as_deref() == Some(uid.as_str()))
            .map(|session| Arc::downgrade(&session));
        let provider = if let Some((session, _)) = &cloud_guard {
            ProviderConfig {
                kind: account.kind,
                model,
                base_url: format!("{}/v1", session.base_url().trim_end_matches('/')),
            }
        } else {
            self.endpoint(&account, model)
        };
        let mut prepared = self.prepare_source_question(
            question,
            history,
            sources,
            QuestionPreparation {
                provider,
                key,
                system,
                admission,
                registration: Some(registration),
                source_guard: None,
                cloud_guard,
            },
        )?;
        prepared.selection.available_sources =
            evidence.sources.len().try_into().unwrap_or(u32::MAX);
        prepared.selection.scope =
            if prepared.selection.included_sources == prepared.selection.available_sources {
                MeetingQuestionScope::WholeMeeting
            } else {
                MeetingQuestionScope::SelectedExcerpts
            };
        prepared.source_guard = Some(QuestionSourceGuard {
            root: self.meeting_store().root(),
            uid,
            live,
            anchors: evidence.anchors_for_sources(prepared.sources())?,
        });
        Ok(prepared)
    }

    fn prepare_source_question(
        &self,
        question: String,
        history: Vec<ChatTurn>,
        sources: Vec<Passage>,
        preparation: QuestionPreparation,
    ) -> Result<PreparedMeetingQuestion, PennelError> {
        if question.trim().is_empty() {
            return Err(PennelError::InvalidResponse {
                message: "Type a question first.".into(),
            });
        }
        if !supports(preparation.provider.kind, ProviderTask::Llm) {
            return Err(PennelError::InvalidResponse {
                message: "This provider does not support text processing.".into(),
            });
        }
        if requires_key(preparation.provider.kind) && preparation.key.is_none() {
            return Err(PennelError::MissingApiKey { task: "llm".into() });
        }
        let available_sources = sources.len();
        let (messages, sources) = if preparation.provider.kind == ProviderKind::AppleIntelligence {
            crate::meeting_chat::apple_chat_messages(&question, &history, &sources)?
        } else {
            (
                crate::meeting_chat::chat_messages(&question, &history, &sources),
                sources,
            )
        };
        let selection = MeetingQuestionSelection::from_messages(
            available_sources,
            sources.len(),
            &history,
            &messages,
        );
        if preparation.registration.is_some()
            && crate::llm::request_body(&preparation.provider, &messages, Reasoning::Low)
                .to_string()
                .len()
                > 64 * 1024
        {
            return Err(PennelError::InvalidResponse { message: "The question context exceeds its byte budget. Choose a smaller source selection or shorten the question; it was not truncated or sent.".into() });
        }
        Ok(PreparedMeetingQuestion {
            provider: preparation.provider,
            key: preparation.key,
            system: preparation.system,
            messages,
            sources,
            admission: preparation.admission,
            _registration: preparation.registration,
            source_guard: preparation.source_guard,
            conversation_guard: None,
            cloud_guard: preparation.cloud_guard,
            selection,
        })
    }

    pub(crate) fn inference_capture_gate(&self) -> std::sync::MutexGuard<'_, ()> {
        self.meeting_gate.lock().unwrap()
    }

    pub(crate) fn register_inference(&self, cancellation: &Arc<crate::InferenceCancellation>) {
        let _gate = self.meeting_gate.lock().unwrap();
        self.register_inference_locked(cancellation);
    }

    fn register_inference_locked(&self, cancellation: &Arc<crate::InferenceCancellation>) {
        let mut requests = self.inference_requests.lock().unwrap();
        requests
            .requests
            .retain(|request| request.strong_count() > 0);
        if requests.mutation_depth > 0 {
            cancellation.cancel();
        }
        requests.requests.push(Arc::downgrade(cancellation));
    }

    pub(crate) fn finish_inference(&self, cancellation: &Arc<crate::InferenceCancellation>) {
        let reference = Arc::downgrade(cancellation);
        let mut requests = self.inference_requests.lock().unwrap();
        requests
            .requests
            .retain(|request| request.strong_count() > 0);
        if let Some(index) = requests
            .requests
            .iter()
            .position(|request| request.ptr_eq(&reference))
        {
            requests.requests.swap_remove(index);
        }
    }

    fn invalidate_inference_requests(&self) {
        let mut state = self.inference_requests.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        for request in &state.requests {
            if let Some(cancellation) = request.upgrade() {
                cancellation.cancel();
            }
        }
    }

    pub(crate) fn update_settings_locked(&self, mut settings: Settings) -> Result<(), PennelError> {
        crate::reconcile_legacy_shortcuts(&self.settings(), &mut settings);
        self.persist_settings_locked(settings.normalized_with(&self.models.downloaded()))
    }

    fn persist_settings_locked(&self, mut settings: Settings) -> Result<(), PennelError> {
        if self.preview {
            enforce_preview_library(
                self.settings_path.parent().expect("data directory"),
                &mut settings,
            )?;
        }
        let changing_library = settings.meetings_dir != self.settings.read().unwrap().meetings_dir;
        if changing_library
            && (self.has_active_session()
                || !self.meeting_jobs.lock().unwrap().is_empty()
                || self.active_speaker_jobs()
                || self
                    .insight_library_jobs
                    .load(std::sync::atomic::Ordering::Acquire)
                    > 0)
        {
            return Err(PennelError::Storage {
                message: "the meeting folder can only be changed after the recording is finalized"
                    .into(),
            });
        }
        let next_lease = if changing_library {
            let root = library_root(&settings);
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if self
                .library_lease
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|(held, _)| *held == canonical)
            {
                None
            } else {
                let lease = DirectoryLease::acquire_library(&root)?;
                MeetingStore::new(&root).recover_retranscriptions()?;
                Some((root.canonicalize().map_err(PennelError::storage)?, lease))
            }
        } else {
            None
        };
        let current = self.settings();
        if current.decision != settings.decision
            || current.reranking != settings.reranking
            || current.providers != settings.providers
            || current.llm != settings.llm
            || current.dictionary != settings.dictionary
            || current.cleanup_verification_enabled != settings.cleanup_verification_enabled
            || current.summary_automation != settings.summary_automation
        {
            // Invalidate queued requests before their captured destination can be used.
            self.invalidate_inference_requests();
        }
        settings.save(&self.settings_path)?;
        *self.settings.write().unwrap() = settings;
        if let Some(lease) = next_lease {
            *self.library_lease.lock().unwrap() = Some(lease);
        }
        if changing_library {
            self.recover_speaker_commits()?;
        }
        Ok(())
    }

    pub(crate) fn update_settings_with(
        &self,
        update: impl FnOnce(Settings) -> Result<Settings, PennelError>,
    ) -> Result<(), PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        let current = self.settings();
        let next = update(current.clone())?;
        if next == current {
            return Ok(());
        }
        self.update_settings_locked(next)
    }

    pub(crate) fn ensure_cloud_identity_mutable(&self) -> Result<(), PennelError> {
        if self.has_active_session() {
            return Err(PennelError::Storage {
                message: "Finish recording before changing the Pennel Cloud account.".into(),
            });
        }
        if self.active_speaker_jobs() {
            return Err(PennelError::Storage {
                message: "Finish speaker recognition before changing the Pennel Cloud account."
                    .into(),
            });
        }
        self.invalidate_inference_requests();
        if self.secrets.read().unwrap().is_some() {
            self.writing_profile.lock().unwrap().set_sharing(false)?;
        }
        Ok(())
    }

    fn reserve_history(&self) -> Result<RecoverySlot<'_>, PennelError> {
        let mut recovery = self.unsaved_history.lock().unwrap();
        recovery.ensure_capacity()?;
        recovery.reserved += 1;
        Ok(RecoverySlot(&self.unsaved_history))
    }

    fn persist_transcript(
        &self,
        mut entry: HistoryEntry,
        cleanup: CleanupStatus,
        progress: Option<&LiveDictationSession>,
        capture_kind: HistoryCaptureKind,
        profile_allowed: bool,
    ) -> Result<DictationResult, PennelError> {
        let mut insert = || {
            let saved = self
                .history
                .lock()
                .unwrap()
                .insert_done_with_provenance(
                    &entry.id,
                    entry.app_bundle_id.as_deref(),
                    &entry.raw,
                    &entry.text,
                    entry.created_at_ms,
                    capture_kind,
                    profile_allowed,
                )
                .is_ok();
            if !saved {
                entry.status = HistoryStatus::Unsaved;
                entry.error = Some(
                    "Not saved yet. This text remains temporarily available until Pennel quits."
                        .into(),
                );
                // A provider can return an unexpectedly large result. Retain it intact, then block
                // subsequent capture at the soft byte budget instead of truncating user text.
                self.unsaved_history.lock().unwrap().entries.insert(
                    entry.id.clone(),
                    UnsavedDictation {
                        entry: entry.clone(),
                        cleanup: cleanup.clone(),
                        capture_kind,
                        profile_allowed,
                    },
                );
            }
            Ok(saved)
        };
        let history_saved = if let Some(progress) = progress {
            progress.with_active(insert)?
        } else {
            insert()?
        };
        Ok(DictationResult {
            history_id: entry.id,
            history_saved,
            text: entry.text,
            raw: entry.raw,
            cleanup,
        })
    }

    pub(crate) fn command_with_settings_observed(
        &self,
        request: DictationRequest,
        selection: Option<String>,
        settings: Settings,
        progress: Option<&LiveDictationSession>,
    ) -> Result<DictationResult, PennelError> {
        if let Some(progress) = progress {
            progress.ensure_active()?;
        }
        use crate::command::{COMMAND_TIMEOUT_SECS, MAX_SELECTION_CHARS, command_messages};

        if !has_speech(&request.samples, request.sample_rate) {
            return Err(PennelError::NoSpeech);
        }
        let _slot = self.reserve_history()?;
        if selection
            .as_ref()
            .is_some_and(|s| s.chars().count() > MAX_SELECTION_CHARS)
        {
            return Err(PennelError::InvalidResponse {
                message: "selection too long".into(),
            });
        }
        let (llm_account, llm_model) = settings
            .resolve(&settings.llm)
            .ok_or(PennelError::NotFound)?;
        let llm_key = self.credential(&llm_account, &request.keys)?;
        if requires_key(llm_account.kind) && llm_key.is_none() {
            return Err(PennelError::MissingApiKey { task: "llm".into() });
        }
        let wav = encode_wav(&request.samples, request.sample_rate);
        let instruction = self.stt(&settings, &settings.dictation_stt, &wav, &request.keys)?;
        if instruction.trim().is_empty() || is_hallucination(&instruction) {
            return Err(PennelError::NoSpeech);
        }
        if let Some(progress) = progress {
            progress.observe_raw(&instruction)?;
        }
        let selection = selection.filter(|s| !s.trim().is_empty());
        let messages = command_messages(&instruction, selection.as_deref(), &settings.dictionary);
        let output = self.complete_llm(
            &self.endpoint(&llm_account, llm_model),
            llm_key.as_deref(),
            &messages,
            COMMAND_TIMEOUT_SECS,
            Reasoning::Off,
        )?;
        let text = strip_wrapping(&output);
        if text.is_empty() {
            return Err(PennelError::InvalidResponse {
                message: "empty command result".into(),
            });
        }
        self.persist_transcript(
            HistoryEntry {
                id: uuid::Uuid::now_v7().to_string(),
                created_at_ms: now_ms(),
                app_bundle_id: request.app_bundle_id,
                raw: instruction,
                text,
                status: HistoryStatus::Done,
                error: None,
                has_audio: false,
            },
            CleanupStatus::Applied,
            progress,
            HistoryCaptureKind::Command,
            false,
        )
    }

    fn dictate_with_settings(
        &self,
        request: DictationRequest,
        settings: Settings,
    ) -> Result<DictationResult, PennelError> {
        self.dictate_with_settings_observed(request, settings, None)
    }

    pub(crate) fn dictate_with_settings_observed(
        &self,
        request: DictationRequest,
        settings: Settings,
        progress: Option<&LiveDictationSession>,
    ) -> Result<DictationResult, PennelError> {
        if let Some(progress) = progress {
            progress.ensure_active()?;
        }
        if !has_speech(&request.samples, request.sample_rate) {
            return Err(PennelError::NoSpeech);
        }
        let _slot = self.reserve_history()?;
        let profile_allowed = self.writing_profile_learning_enabled();
        let (account, _) = settings
            .resolve(&settings.dictation_stt)
            .ok_or(PennelError::NotFound)?;
        if requires_key(account.kind) && request.keys.get(&account).is_none() {
            return Err(PennelError::MissingApiKey { task: "stt".into() });
        }
        let wav = encode_wav(&request.samples, request.sample_rate);
        let app = request.app_bundle_id.as_deref();
        let result = self.process_observed(&settings, &wav, app, &request.keys, progress);
        if let Some(progress) = progress {
            progress.ensure_active()?;
        }
        let transcript = match result {
            Err(
                e @ (PennelError::Http { .. }
                | PennelError::Network { .. }
                | PennelError::InvalidResponse { .. }
                | PennelError::Cloud { .. }
                | PennelError::NotSignedIn
                | PennelError::ModelNotDownloaded),
            ) => {
                let insert = || {
                    self.history.lock().unwrap().insert_failed_with_provenance(
                        app,
                        &wav,
                        &e.to_string(),
                        now_ms(),
                        profile_allowed,
                    )
                };
                if let Some(progress) = progress {
                    progress.with_active(insert)?;
                } else {
                    insert()?;
                }
                return Err(e);
            }
            other => other?,
        };
        self.persist_transcript(
            HistoryEntry {
                id: uuid::Uuid::now_v7().to_string(),
                created_at_ms: now_ms(),
                app_bundle_id: request.app_bundle_id,
                raw: transcript.raw,
                text: transcript.text,
                status: HistoryStatus::Done,
                error: None,
                has_audio: false,
            },
            transcript.cleanup,
            progress,
            HistoryCaptureKind::Dictation,
            profile_allowed,
        )
    }

    fn open(data_dir: String, preview: bool) -> Result<Arc<Self>, PennelError> {
        let dir = PathBuf::from(data_dir);
        std::fs::create_dir_all(&dir).map_err(PennelError::storage)?;
        let dir = dir.canonicalize().map_err(PennelError::storage)?;
        let mut registry = engines().lock().unwrap();
        if let Some(engine) = registry.get(&dir).and_then(Weak::upgrade) {
            if engine.preview != preview {
                return Err(PennelError::Storage {
                    message: "this data folder is already open under another Pennel profile".into(),
                });
            }
            return Ok(engine);
        }
        registry.retain(|_, engine| engine.strong_count() > 0);
        let mut storage = StartupStorage::prepare(&dir)?;
        let settings_path = dir.join("settings.json");
        let models = ModelStore::new(&dir);
        let mut settings =
            Settings::load_strict(&settings_path)?.normalized_with(&models.downloaded());
        if preview {
            enforce_preview_library(&dir, &mut settings)?;
        }
        let root = library_root(&settings);
        let library_lease = if (settings_path.exists() && root.exists()) || preview {
            let lease = DirectoryLease::acquire_library(&root)?;
            MeetingStore::new(&root).recover_retranscriptions()?;
            Some((root.canonicalize().map_err(PennelError::storage)?, lease))
        } else {
            None
        };
        let history = HistoryStore::open(&dir)?;
        let writing_profile = crate::writing_profile::WritingProfileStore::open(&dir)?;
        let dictation_pad = DictationPad::open(&dir)?;
        let index = open_index(&dir.join("index.sqlite"));
        storage.finish()?;
        history.prune(settings.history_retention_days, now_ms())?;
        let engine = Arc::new(Self {
            settings_path,
            settings: RwLock::new(settings),
            inference_requests: Arc::new(Mutex::new(
                crate::inference_engine::InferenceState::default(),
            )),
            #[cfg(test)]
            cleanup_qualification: Mutex::new(None),
            #[cfg(test)]
            cleanup_reason_codes: Mutex::new(Vec::new()),
            #[cfg(test)]
            cleanup_language_evidence: Mutex::new(None),
            history: Mutex::new(history),
            writing_profile: Mutex::new(writing_profile),
            writing_profile_refresh: Mutex::new(()),
            writing_profile_cancellation: Mutex::new(None),
            #[cfg(test)]
            writing_profile_before_dispatch: Mutex::new(None),
            #[cfg(test)]
            writing_profile_test_pool: Mutex::new(None),
            conversations: Mutex::new(crate::conversations::State::default()),
            unsaved_history: Mutex::new(RecoveryState::default()),
            dictation_pad: Mutex::new(dictation_pad),
            models,
            downloads: Mutex::new(HashMap::new()),
            local: LocalEngine::default(),
            index: Mutex::new(index),
            system_stt: RwLock::new(None),
            timed_system_stt: RwLock::new(None),
            system_llm: RwLock::new(None),
            secrets: RwLock::new(None),
            cloud_session: Mutex::new(None),
            meeting_gate: Mutex::new(()),
            active_sessions: Mutex::new(Vec::new()),
            meeting_preview_priority: Mutex::new(0),
            meeting_jobs: Mutex::new(HashSet::new()),
            retranscriptions: Mutex::new(HashMap::new()),
            speaker_jobs: Mutex::new(HashMap::new()),
            insight_jobs: Mutex::new(HashMap::new()),
            insight_library_jobs: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            library_lease: Mutex::new(library_lease),
            sync_gate: Mutex::new(()),
            sync_state: Mutex::new(None),
            sync_last: Mutex::new(None),
            sync_items: Mutex::new(Vec::new()),
            deleted_meetings: Mutex::new(Vec::new()),
            pending_device: Mutex::new(None),
            pending_confirm: Mutex::new(false),
            cloud_link: Mutex::new(crate::cloud::link::LinkFlow::default()),
            link_journal_gate: Mutex::new(()),
            #[cfg(test)]
            sync_api_override: Mutex::new(None),
            preview,
            _storage: storage,
        });
        engine.load_speaker_recognition_jobs()?;
        registry.insert(dir, Arc::downgrade(&engine));
        Ok(engine)
    }

    pub fn begin_meeting_preview_priority(&self) {
        crate::meeting_scheduler::physical_pool().speech_started();
        let mut priority = self.meeting_preview_priority.lock().unwrap();
        *priority = priority.saturating_add(1);
        for session in self
            .active_sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            session.set_preview_paused(true);
        }
    }

    pub fn end_meeting_preview_priority(&self) {
        crate::meeting_scheduler::physical_pool().speech_finished();
        let mut priority = self.meeting_preview_priority.lock().unwrap();
        *priority = priority.saturating_sub(1);
        if *priority == 0 {
            for session in self
                .active_sessions
                .lock()
                .unwrap()
                .iter()
                .filter_map(Weak::upgrade)
            {
                session.set_preview_paused(false);
            }
        }
    }

    fn has_active_session(&self) -> bool {
        let mut sessions = self.active_sessions.lock().unwrap();
        sessions.retain(|session| session.strong_count() > 0);
        sessions
            .iter()
            .filter_map(Weak::upgrade)
            .any(|session| !session.is_finalized())
    }

    pub(crate) fn ensure_meeting_inactive(&self, id: &str) -> Result<(), PennelError> {
        if self.meeting_jobs.lock().unwrap().contains(id) {
            return Err(PennelError::storage(
                "This meeting is being processed. Wait until processing has finished.",
            ));
        }
        if self
            .active_sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .any(|session| session.id() == id && !session.is_finalized())
        {
            return Err(PennelError::Storage {
                message: "the meeting is still recording or being finalized".into(),
            });
        }
        Ok(())
    }

    pub(crate) fn edit_meeting(
        &self,
        id: &str,
    ) -> Result<crate::retranscription::MeetingEditGuard<'_>, PennelError> {
        let _gate = self.meeting_gate.lock().unwrap();
        self.ensure_meeting_inactive(id)?;
        self.meeting_jobs.lock().unwrap().insert(id.to_owned());
        Ok(crate::retranscription::MeetingEditGuard {
            jobs: &self.meeting_jobs,
            id: id.to_owned(),
        })
    }

    pub(crate) fn speaker_commit_policy_guard(
        &self,
        expected: (bool, crate::settings::MeetingAppNamePolicy),
    ) -> Result<std::sync::MutexGuard<'_, ()>, PennelError> {
        let guard = self.meeting_gate.lock().unwrap();
        let settings = self.settings();
        if (
            settings.meeting_app_speakers,
            settings.meeting_app_name_policy,
        ) != expected
        {
            return Err(PennelError::Cancelled);
        }
        Ok(guard)
    }

    /// Serialize the final automatic write with consent and inference-configuration changes.
    pub(crate) fn automatic_summary_policy_guard(
        &self,
        cancellation: &crate::InferenceCancellation,
    ) -> Result<std::sync::MutexGuard<'_, ()>, PennelError> {
        let guard = self.meeting_gate.lock().unwrap();
        if cancellation.is_cancelled()
            || self.settings().summary_automation != crate::SummaryAutomation::Automatic
        {
            return Err(PennelError::Cancelled);
        }
        Ok(guard)
    }

    /// Owner-local identity metadata can change during capture, without opening transcript edits.
    pub(crate) fn people_metadata_guard(
        &self,
        id: &str,
    ) -> Result<std::sync::MutexGuard<'_, ()>, PennelError> {
        let gate = self.meeting_gate.lock().unwrap();
        if self.meeting_jobs.lock().unwrap().contains(id) {
            return Err(PennelError::storage(
                "This meeting is being processed. Wait until processing has finished.",
            ));
        }
        Ok(gate)
    }

    pub(crate) fn edit_voice_settings(&self) -> Result<std::sync::MutexGuard<'_, ()>, PennelError> {
        let gate = self.meeting_gate.lock().unwrap();
        if !self.meeting_jobs.lock().unwrap().is_empty() {
            return Err(PennelError::storage(
                "This meeting is being processed. Wait until processing has finished.",
            ));
        }
        Ok(gate)
    }

    pub(crate) fn claim_library(&self) -> Result<(), PennelError> {
        let mut settings = self.settings();
        if self.preview {
            enforce_preview_library(
                self.settings_path.parent().expect("data directory"),
                &mut settings,
            )?;
        }
        let root = library_root(&settings);
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        let mut held = self.library_lease.lock().unwrap();
        if held.as_ref().is_some_and(|(path, _)| *path == canonical) {
            return Ok(());
        }
        let lease = DirectoryLease::acquire_library(&root)?;
        MeetingStore::new(&root).recover_retranscriptions()?;
        *held = Some((root.canonicalize().map_err(PennelError::storage)?, lease));
        Ok(())
    }

    /// Syncs the index with the meeting files first; `None` when the index is unavailable.
    pub(crate) fn with_index<T>(
        &self,
        store: &MeetingStore,
        query: impl FnOnce(&MeetingIndex) -> Result<T, PennelError>,
    ) -> Option<T> {
        let mut guard = self
            .index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let index = guard.as_mut()?;
        index.sync(store).ok()?;
        query(index).ok()
    }

    fn process(
        &self,
        settings: &Settings,
        wav: &[u8],
        app: Option<&str>,
        keys: &ApiKeys,
    ) -> Result<Transcript, PennelError> {
        self.process_observed(settings, wav, app, keys, None)
    }

    fn process_observed(
        &self,
        settings: &Settings,
        wav: &[u8],
        app: Option<&str>,
        keys: &ApiKeys,
        progress: Option<&LiveDictationSession>,
    ) -> Result<Transcript, PennelError> {
        let frozen = progress.map(LiveDictationSession::cleanup_verification);
        let captured = frozen.is_none().then(|| {
            crate::cleanup_verification::FrozenCleanupVerification::capture(self, settings)
        });
        let verification = frozen
            .or(captured.as_ref())
            .expect("captured cleanup context");
        let raw = self.stt(settings, &settings.dictation_stt, wav, keys)?;
        if is_hallucination(&raw) {
            return Err(PennelError::NoSpeech);
        }
        if let Some(progress) = progress {
            progress.observe_raw(&raw)?;
        }
        let (text, cleanup) = match settings.mode {
            DictationMode::Fast => (raw.clone(), CleanupStatus::Skipped),
            DictationMode::Clean => clean(self, settings, &raw, app, keys, verification),
        };
        Ok(Transcript { raw, text, cleanup })
    }

    pub(crate) fn complete_llm(
        &self,
        provider: &ProviderConfig,
        key: Option<&str>,
        messages: &[crate::llm::ChatMessage],
        timeout_secs: u64,
        reasoning: Reasoning,
    ) -> Result<String, PennelError> {
        if !supports(provider.kind, ProviderTask::Llm) {
            return Err(PennelError::InvalidResponse {
                message: "This provider does not support text processing.".into(),
            });
        }
        if provider.kind == ProviderKind::AppleIntelligence {
            let model = self.system_llm.read().unwrap().clone();
            return system_llm::complete(model.as_deref(), messages, timeout_secs);
        }
        crate::llm::chat_complete_within(provider, key, messages, timeout_secs, reasoning)
    }

    fn record_cleanup_reason(
        &self,
        reason: &str,
        elapsed: std::time::Duration,
        raw_chars: usize,
        output_chars: usize,
    ) {
        eprintln!(
            "cleanup reason={reason} elapsed_ms={} raw_chars={raw_chars} output_chars={output_chars}",
            elapsed.as_millis()
        );
        #[cfg(test)]
        self.cleanup_reason_codes
            .lock()
            .unwrap()
            .push(reason.into());
    }

    fn complete_cleanup(
        &self,
        provider: ProviderConfig,
        key: Option<String>,
        messages: Vec<crate::llm::ChatMessage>,
        deadline: std::time::Instant,
        cancellation: Arc<crate::InferenceCancellation>,
    ) -> Result<String, PennelError> {
        if !supports(provider.kind, ProviderTask::Llm) {
            return Err(PennelError::InvalidResponse {
                message: "This provider does not support text processing.".into(),
            });
        }
        let system = self.system_llm.read().unwrap().clone();
        let cancellation = crate::InferenceCancellation::scheduled(
            cancellation,
            crate::InferencePriority::Speech,
            deadline,
        );
        self.register_inference(&cancellation);
        let registration = PreparedInferenceRegistration {
            state: self.inference_state(),
            cancellation: cancellation.clone(),
        };
        let cloud = (provider.kind == ProviderKind::PennelCloud)
            .then(|| self.cloud_session())
            .transpose()?
            .map(|session| {
                let generation = session.generation();
                (session, generation)
            });
        let worker_cancellation = cancellation.clone();
        crate::inference::run_cancelled_before(cancellation, deadline, move |deadline| {
            let _registration = registration;
            let key = match cloud {
                Some((session, generation)) => Some(session.inference_token(
                    generation,
                    false,
                    &worker_cancellation,
                    deadline,
                )?),
                None => key,
            };
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(PennelError::Network {
                    message: "Cleanup deadline exceeded.".into(),
                });
            }
            let seconds = remaining.as_secs();
            if provider.kind == ProviderKind::AppleIntelligence {
                if seconds == 0 {
                    return Err(crate::inference::network());
                }
                return system_llm::complete(system.as_deref(), &messages, seconds);
            }
            if provider.kind.is_cli_agent() {
                if seconds == 0 {
                    return Err(crate::inference::network());
                }
                return crate::cli_agent::complete(
                    provider.kind,
                    &provider.base_url,
                    &provider.model,
                    &messages,
                    seconds,
                );
            }
            crate::llm::chat_complete_before(
                &provider,
                key.as_deref(),
                &messages,
                deadline,
                Reasoning::Off,
            )
        })
    }

    pub(crate) fn stt_meeting(
        &self,
        settings: &Settings,
        wav: &[u8],
        keys: &ApiKeys,
    ) -> Result<crate::SpeechTranscript, PennelError> {
        let _speech = crate::meeting_scheduler::speech_priority();
        let (account, model) = settings
            .resolve(&settings.meeting_stt)
            .ok_or(PennelError::NotFound)?;
        if account.kind == ProviderKind::Local {
            crate::local_engine::ensure_available()?;
            let path = self
                .models
                .path(&model)
                .ok_or(PennelError::ModelNotDownloaded)?;
            let (samples, _) = decode_wav(wav).ok_or(PennelError::InvalidResponse {
                message: "bad audio".into(),
            })?;
            return self.local.transcribe_timed(
                &path,
                &samples,
                settings.language.as_deref(),
                &capped_terms(&settings.dictionary, MAX_PROMPT_CHARS),
            );
        }
        if account.kind == ProviderKind::AppleSpeech {
            let timed = self.timed_system_stt.read().unwrap().clone();
            if let Some(transcriber) = timed {
                let (samples, rate) = decode_wav(wav).ok_or(PennelError::InvalidResponse {
                    message: "bad audio".into(),
                })?;
                let duration = crate::wav::sample_duration_ms(samples.len() as u64, rate);
                let result = transcriber.transcribe_timed(
                    samples,
                    rate,
                    settings.language.clone(),
                    crate::system_stt::bounded_vocabulary(&settings.dictionary),
                )?;
                return Ok(crate::SpeechTranscript::validated(
                    result.text,
                    result.spans,
                    result.timing_kind,
                    duration,
                )
                .trimmed()
                .with_complete(result.complete));
            }
        }
        self.stt(settings, &settings.meeting_stt, wav, keys)
            .map(crate::SpeechTranscript::plain)
    }

    pub(crate) fn transcribe_complete_local_timed(
        &self,
        settings: &Settings,
        model_path: &Path,
        samples: &[f32],
    ) -> Result<crate::SpeechTranscript, PennelError> {
        self.local.transcribe_complete_timed(
            model_path,
            samples,
            settings.language.as_deref(),
            &capped_terms(&settings.dictionary, MAX_PROMPT_CHARS),
        )
    }

    pub(crate) fn preview_local(
        &self,
        settings: &Settings,
        samples: &[f32],
        cancel: &crate::local_engine::CancelToken,
    ) -> Result<Option<String>, PennelError> {
        self.preview_local_choice(settings, &settings.dictation_stt, samples, cancel, false)
    }

    pub(crate) fn preview_meeting_local(
        &self,
        settings: &Settings,
        samples: &[f32],
        cancel: &crate::local_engine::CancelToken,
    ) -> Result<Option<String>, PennelError> {
        self.preview_local_choice(settings, &settings.meeting_stt, samples, cancel, true)
    }

    fn preview_local_choice(
        &self,
        settings: &Settings,
        choice: &ModelChoice,
        samples: &[f32],
        cancel: &crate::local_engine::CancelToken,
        warm_only: bool,
    ) -> Result<Option<String>, PennelError> {
        let Some((account, model)) = settings.resolve(choice) else {
            return Err(PennelError::NotFound);
        };
        if account.kind != ProviderKind::Local {
            return Ok(None);
        }
        crate::local_engine::ensure_available()?;
        let path = self
            .models
            .path(&model)
            .ok_or(PennelError::ModelNotDownloaded)?;
        let vocabulary = capped_terms(&settings.dictionary, MAX_PROMPT_CHARS);
        if warm_only {
            self.local.preview_warm(
                &path,
                samples,
                settings.language.as_deref(),
                &vocabulary,
                cancel,
            )
        } else {
            self.local.preview(
                &path,
                samples,
                settings.language.as_deref(),
                &vocabulary,
                cancel,
            )
        }
    }

    #[allow(
        dead_code,
        reason = "Retain the text-only complete-recognition compatibility projection."
    )]
    pub(crate) fn transcribe_complete_local(
        &self,
        settings: &Settings,
        model_path: &Path,
        samples: &[f32],
    ) -> Result<String, PennelError> {
        self.local.transcribe_complete(
            model_path,
            samples,
            settings.language.as_deref(),
            &capped_terms(&settings.dictionary, MAX_PROMPT_CHARS),
        )
    }

    /// Speech-to-text for one WAV via the provider in `choice`, cloud or local.
    pub(crate) fn stt(
        &self,
        settings: &Settings,
        choice: &ModelChoice,
        wav: &[u8],
        keys: &ApiKeys,
    ) -> Result<String, PennelError> {
        let _speech = crate::meeting_scheduler::speech_priority();
        let prompt: Option<String> = (!settings.dictionary.is_empty()).then(|| {
            settings
                .dictionary
                .join(", ")
                .chars()
                .take(MAX_PROMPT_CHARS)
                .collect()
        });
        let (account, model) = settings.resolve(choice).ok_or(PennelError::NotFound)?;
        if account.kind == ProviderKind::AppleSpeech {
            let transcriber = self
                .system_stt
                .read()
                .unwrap()
                .clone()
                .ok_or(PennelError::NotFound)?;
            let (samples, rate) = decode_wav(wav).ok_or(PennelError::InvalidResponse {
                message: "bad audio".into(),
            })?;
            return transcriber
                .transcribe(
                    samples,
                    rate,
                    settings.language.clone(),
                    crate::system_stt::bounded_vocabulary(&settings.dictionary),
                )
                .map(|text| text.trim().to_owned());
        }
        if account.kind == ProviderKind::Local {
            crate::local_engine::ensure_available()?;
            let path = self
                .models
                .path(&model)
                .ok_or(PennelError::ModelNotDownloaded)?;
            let (samples, _) = decode_wav(wav).ok_or(PennelError::InvalidResponse {
                message: "bad audio".into(),
            })?;
            let vocabulary = capped_terms(&settings.dictionary, MAX_PROMPT_CHARS);
            return self.local.transcribe(
                &path,
                &samples,
                settings.language.as_deref(),
                &vocabulary,
            );
        }
        let key = self.credential(&account, keys)?;
        if requires_key(account.kind) && key.is_none() {
            return Err(PennelError::MissingApiKey { task: "stt".into() });
        }
        transcribe(
            &self.endpoint(&account, model),
            key.as_deref(),
            wav,
            prompt.as_deref(),
            settings.language.as_deref(),
        )
    }

    fn template_store(&self) -> TemplateStore {
        TemplateStore::new(&self.meeting_store().root())
    }

    pub(crate) fn meeting_store(&self) -> MeetingStore {
        let configured = self.settings.read().unwrap().meetings_dir.clone();
        let dir = if configured.is_empty() {
            home_dir().join("Pennel")
        } else {
            PathBuf::from(configured)
        };
        MeetingStore::new(&dir)
    }
}

// Provider errors can include credentials or user content; keep only fixed diagnostic text and known codes.
fn meeting_retry_error(error: PennelError) -> PennelError {
    match error {
        PennelError::Cloud { code, .. } => {
            let (code, message) = match code.as_str() {
                "unauthorized" | "session_revoked" | "not_signed_in" => {
                    return PennelError::NotSignedIn;
                }
                "quota" => (
                    "quota",
                    "You have reached your Pennel Cloud limit for this month. You can choose your own provider in Providers.",
                ),
                "upstream_payment_required" => (
                    "upstream_payment_required",
                    "The provider reports insufficient credit. Check your account.",
                ),
                "overloaded" => (
                    "overloaded",
                    "Pennel Cloud is temporarily overloaded. Try again later.",
                ),
                "upstream_rejected" | "model" | "request" | "multipart" | "file" => (
                    "invalid_request",
                    "The model could not process this request.",
                ),
                "too_large" | "too_long" => ("too_large", "This request is too large."),
                _ => (
                    "upstream",
                    "Pennel Cloud models are temporarily unavailable.",
                ),
            };
            PennelError::Cloud {
                code: code.into(),
                message: message.into(),
            }
        }
        PennelError::Http { status, .. } => crate::http::map_status(status, ""),
        PennelError::Network { .. } => PennelError::Network {
            message: "No connection".into(),
        },
        PennelError::InvalidResponse { .. } => PennelError::InvalidResponse {
            message: "The model could not process this request.".into(),
        },
        PennelError::Storage { .. } => PennelError::Storage {
            message: "The local model could not be read. Check the model and try again.".into(),
        },
        PennelError::MissingApiKey { .. } => PennelError::MissingApiKey {
            task: "meeting_stt".into(),
        },
        other => other,
    }
}

fn clean(
    engine: &PennelEngine,
    settings: &Settings,
    raw: &str,
    app: Option<&str>,
    keys: &ApiKeys,
    verification: &crate::cleanup_verification::FrozenCleanupVerification,
) -> (String, CleanupStatus) {
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(cleanup_timeout_secs(raw));
    clean_before(engine, settings, raw, app, keys, verification, deadline)
}

pub(crate) fn clean_before(
    engine: &PennelEngine,
    settings: &Settings,
    raw: &str,
    app: Option<&str>,
    keys: &ApiKeys,
    verification: &crate::cleanup_verification::FrozenCleanupVerification,
    deadline: std::time::Instant,
) -> (String, CleanupStatus) {
    let started = std::time::Instant::now();
    let style = resolve_style(app, &settings.style_rules);
    let rejected = |reason: &str, output_chars: usize| {
        engine.record_cleanup_reason(reason, started.elapsed(), raw.chars().count(), output_chars);
        (raw.to_owned(), CleanupStatus::Rejected)
    };
    if settings.cleanup_verification_enabled
        && let Err(reason) = verification.ready(settings, style)
    {
        return rejected(reason, 0);
    }
    let Some((account, model)) = settings.resolve(&settings.llm) else {
        return (
            raw.to_owned(),
            CleanupStatus::Failed {
                message: "no llm provider".into(),
            },
        );
    };
    let key = if account.kind == ProviderKind::PennelCloud {
        None
    } else {
        match engine.credential(&account, keys) {
            Ok(key) => key,
            Err(_) => {
                return (
                    raw.to_owned(),
                    CleanupStatus::Failed {
                        message: "Cleanup processing failed. The original dictation was kept."
                            .into(),
                    },
                );
            }
        }
    };
    if requires_key(account.kind) && account.kind != ProviderKind::PennelCloud && key.is_none() {
        return (
            raw.to_owned(),
            CleanupStatus::Failed {
                message: "missing llm key".into(),
            },
        );
    }
    let messages = cleanup_messages(raw, style, &settings.dictionary);
    let result = engine.complete_cleanup(
        engine.endpoint(&account, model),
        key,
        messages,
        deadline,
        verification.cancellation(),
    );
    if std::time::Instant::now() >= deadline {
        return rejected("deadline_exceeded", 0);
    }
    match result {
        Ok(output) => {
            let cleaned = strip_wrapping(&output);
            let assessment = assess_cleanup(raw, &cleaned, style, &settings.dictionary);
            if !assessment.accepted {
                return rejected(assessment.reason_code, cleaned.chars().count());
            }
            if settings.cleanup_verification_enabled
                && let Err(reason) =
                    verification.verify(engine, settings, raw, &cleaned, style, keys, deadline)
            {
                return rejected(reason, cleaned.chars().count());
            }
            if std::time::Instant::now() >= deadline {
                return rejected("deadline_exceeded", cleaned.chars().count());
            }
            if verification.cancellation().is_cancelled() {
                return rejected("cancelled", cleaned.chars().count());
            }
            engine.record_cleanup_reason(
                "accepted",
                started.elapsed(),
                raw.chars().count(),
                cleaned.chars().count(),
            );
            (cleaned, CleanupStatus::Applied)
        }
        Err(crate::PennelError::Cancelled) => rejected("cancelled", 0),
        Err(error) => {
            engine.record_cleanup_reason(
                "generation_failed",
                started.elapsed(),
                raw.chars().count(),
                0,
            );
            let message = if error == crate::system_llm::length_error() {
                crate::system_llm::length_error().to_string()
            } else {
                "Cleanup processing failed. The original dictation was kept.".into()
            };
            (raw.to_owned(), CleanupStatus::Failed { message })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryStatus;
    use mockito::Matcher;

    fn incomplete_model_setup() -> (tempfile::TempDir, Arc<PennelEngine>, ModelChoice) {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.dictation_stt = ModelChoice {
            provider_id: "local".into(),
            model: "whisper-large-v3-turbo".into(),
        };
        settings.meeting_stt = ModelChoice {
            provider_id: "local".into(),
            model: "whisper-large-v3".into(),
        };
        settings.dictionary = vec!["synthetic private vocabulary".into()];
        engine.update_settings(settings).unwrap();
        let selected = ModelChoice {
            provider_id: "local".into(),
            model: "whisper-small-q5_1".into(),
        };
        let model_dir = dir.path().join("models");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(
            model_dir.join("whisper-small-q5_1.bin"),
            b"synthetic model; never inferred",
        )
        .unwrap();
        (dir, engine, selected)
    }

    #[test]
    fn downloaded_draft_model_does_not_reassign_tasks_on_save_or_restart() {
        let (dir, engine, _) = incomplete_model_setup();
        let mut expected = engine.settings();
        expected.language = Some("nl".into());
        engine.update_settings(expected.clone()).unwrap();
        assert_eq!(engine.settings(), expected);
        assert_eq!(
            Settings::load_strict(&dir.path().join("settings.json")).unwrap(),
            expected
        );
        drop(engine);
        let reopened =
            PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(reopened.settings(), expected);
    }

    #[test]
    fn applying_one_model_choice_preserves_other_incomplete_tasks_and_settings() {
        let (_dir, engine, selected) = incomplete_model_setup();
        let mut expected = engine.settings();
        let text = ModelChoice {
            provider_id: "groq".into(),
            model: "synthetic-text-model".into(),
        };
        expected.llm = text.clone();
        engine
            .apply_task_model_choice(ProviderTask::Llm, text)
            .unwrap();
        assert_eq!(engine.settings(), expected);
        expected.dictation_models = vec![expected.dictation_stt.clone(), selected.clone()];
        expected.dictation_stt = selected.clone();
        engine
            .apply_task_model_choice(ProviderTask::DictationStt, selected)
            .unwrap();
        assert_eq!(engine.settings(), expected);
    }

    #[test]
    fn applying_a_model_rejects_invalid_choices_without_mutating_settings() {
        let (_dir, engine, selected) = incomplete_model_setup();
        let before = engine.settings();
        for (task, choice) in [
            (
                ProviderTask::Llm,
                ModelChoice {
                    provider_id: "missing".into(),
                    model: "text".into(),
                },
            ),
            (ProviderTask::Llm, selected),
            (
                ProviderTask::Llm,
                ModelChoice {
                    provider_id: "groq".into(),
                    model: "  ".into(),
                },
            ),
            (ProviderTask::DictationStt, before.dictation_stt.clone()),
            (
                ProviderTask::MeetingStt,
                ModelChoice {
                    provider_id: "local".into(),
                    model: "unknown-local".into(),
                },
            ),
        ] {
            assert!(engine.apply_task_model_choice(task, choice).is_err());
            assert_eq!(engine.settings(), before);
        }
    }

    #[test]
    fn failed_model_choice_persistence_does_not_change_active_settings() {
        let (dir, engine, selected) = incomplete_model_setup();
        let before = engine.settings();
        let path = dir.path().join("settings.json");
        let backup = dir.path().join("settings.before.json");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            engine
                .apply_task_model_choice(ProviderTask::MeetingStt, selected)
                .is_err()
        );
        assert_eq!(engine.settings(), before);
        assert_eq!(Settings::load_strict(&backup).unwrap(), before);
    }

    #[test]
    fn concurrent_model_choices_apply_against_latest_settings() {
        let (_dir, engine, selected) = incomplete_model_setup();
        let mut expected = engine.settings();
        let text = ModelChoice {
            provider_id: "groq".into(),
            model: "synthetic-text-model".into(),
        };
        expected.dictation_models = vec![expected.dictation_stt.clone(), selected.clone()];
        expected.dictation_stt = selected.clone();
        expected.llm = text.clone();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = [
            (ProviderTask::DictationStt, selected),
            (ProviderTask::Llm, text),
        ]
        .into_iter()
        .map(|(task, choice)| {
            let engine = engine.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                engine.apply_task_model_choice(task, choice).unwrap();
            })
        })
        .collect();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(engine.settings(), expected);
    }

    #[test]
    fn cli_model_choice_can_leave_model_selection_to_the_cli() {
        let (_dir, engine, _) = incomplete_model_setup();
        let mut settings = engine.settings();
        let account = crate::provider::new_account(ProviderKind::Codex, settings.providers.clone());
        settings.providers.push(account.clone());
        engine.update_settings(settings).unwrap();
        let mut expected = engine.settings();
        let choice = ModelChoice {
            provider_id: account.id,
            model: String::new(),
        };
        expected.llm = choice.clone();
        engine
            .apply_task_model_choice(ProviderTask::Llm, choice)
            .unwrap();
        assert_eq!(engine.settings(), expected);
    }

    fn speech() -> Vec<f32> {
        (0..16000)
            .map(|i| 0.2 * (i as f32 * 440.0 * std::f32::consts::TAU / 16000.0).sin())
            .collect()
    }

    fn keys() -> ApiKeys {
        ApiKeys {
            by_provider: HashMap::from([("groq".to_owned(), "k".to_owned())]),
        }
    }

    fn request(samples: Vec<f32>, app: Option<&str>) -> DictationRequest {
        DictationRequest {
            samples,
            sample_rate: 16000,
            app_bundle_id: app.map(str::to_owned),
            keys: keys(),
        }
    }

    fn engine(
        dir: &tempfile::TempDir,
        server: &mockito::Server,
        mode: DictationMode,
    ) -> Arc<PennelEngine> {
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers[0].base_url = server.url();
        settings.dictation_stt.provider_id = "groq".into();
        settings.meeting_stt.provider_id = "groq".into();
        settings.mode = mode;
        engine.update_settings(settings).unwrap();
        engine
    }

    fn stt(server: &mut mockito::Server, text: &str) -> mockito::Mock {
        server
            .mock("POST", "/audio/transcriptions")
            .with_body(serde_json::json!({ "text": text }).to_string())
            .create()
    }

    fn llm(server: &mut mockito::Server, text: &str) -> mockito::Mock {
        server
            .mock("POST", "/chat/completions")
            .with_body(
                serde_json::json!({ "choices": [{ "message": { "content": text } }] }).to_string(),
            )
            .create()
    }

    fn cloud_engine(
        dir: &tempfile::TempDir,
        server: &mockito::Server,
        signed_in: bool,
        stored_base_url: &str,
    ) -> Arc<PennelEngine> {
        use crate::cloud::REFRESH_ACCOUNT;
        use crate::cloud::account::Session;
        use crate::cloud::tests::MemorySecrets;
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        if signed_in {
            secrets.set(REFRESH_ACCOUNT.into(), Some("refresh-1".into()));
        }
        engine.set_secret_store(secrets.clone()).unwrap();
        engine.use_cloud_session(Session::new(server.url(), secrets));
        let mut settings = engine.settings();
        let mut account = crate::provider::new_account(ProviderKind::PennelCloud, vec![]);
        account.base_url = stored_base_url.into();
        let choice = ModelChoice {
            provider_id: account.id.clone(),
            model: "pennel-auto".into(),
        };
        settings.providers.push(account);
        settings.dictation_stt = choice;
        settings.mode = DictationMode::Fast;
        engine.update_settings(settings).unwrap();
        engine
    }

    fn refresh_mock(server: &mut mockito::Server) -> mockito::Mock {
        server
            .mock("POST", "/v1/auth/refresh")
            .with_body(
                r#"{"access_token":"access-1","refresh_token":"refresh-2","expires_in":900}"#,
            )
            .create()
    }

    #[test]
    fn pennel_cloud_dictation_uses_access_token() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = cloud_engine(&dir, &server, true, "");
        let _refresh = refresh_mock(&mut server);
        let stt = server
            .mock("POST", "/v1/audio/transcriptions")
            .match_header("authorization", "Bearer access-1")
            .match_body(Matcher::Regex("name=\"model\"\r\n\r\npennel-auto".into()))
            .with_body(r#"{"text":"hallo wereld"}"#)
            .create();
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "hallo wereld");
        stt.assert();
    }

    #[test]
    fn a_meeting_blocks_cloud_identity_changes_but_allows_same_account_token_refresh() {
        use crate::cloud::tests::MemorySecrets;
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = cloud_engine(&dir, &server, true, "");
        let mut settings = engine.settings();
        settings.meeting_stt = settings.dictation_stt.clone();
        settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        let refresh = refresh_mock(&mut server).expect(1);
        let device_login = server.mock("POST", "/v1/auth/device").expect(0).create();
        let poll_login = server.mock("POST", "/v1/auth/token").expect(0).create();
        let delete = server.mock("DELETE", "/v1/account").expect(0).create();
        let original = engine.cloud_session().unwrap();
        let secrets = engine.secret_store().unwrap();
        let session = engine
            .clone()
            .start_meeting("Zelfde account".into(), keys())
            .unwrap();
        assert!(
            engine
                .cloud_login_start("Mac".into(), "macos".into())
                .is_err()
        );
        assert!(engine.cloud_login_poll("ander-account".into()).is_err());
        assert!(engine.cloud_logout().is_err());
        assert!(engine.cloud_delete_account().is_err());
        assert!(
            engine
                .set_secret_store(Arc::new(MemorySecrets::default()))
                .is_err()
        );
        assert!(Arc::ptr_eq(&original, &engine.cloud_session().unwrap()));
        assert!(Arc::ptr_eq(&secrets, &engine.secret_store().unwrap()));
        assert_eq!(original.access_token().unwrap(), "access-1");
        session.stop().unwrap();
        engine.cloud_logout().unwrap();
        assert!(!engine.cloud_signed_in());
        device_login.assert();
        poll_login.assert();
        delete.assert();
        refresh.assert();
    }

    #[test]
    fn pennel_cloud_without_login_is_not_signed_in() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = cloud_engine(&dir, &server, false, "");
        let stt = server
            .mock("POST", "/v1/audio/transcriptions")
            .expect(0)
            .create();
        assert_eq!(
            engine.dictate(request(speech(), None)).unwrap_err(),
            PennelError::NotSignedIn
        );
        stt.assert();
    }

    #[test]
    fn pennel_cloud_quota_error_shows_server_message() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = cloud_engine(&dir, &server, true, "");
        let _refresh = refresh_mock(&mut server);
        server
            .mock("POST", "/v1/audio/transcriptions")
            .with_status(429)
            .with_body(r#"{"error":"quota","message":"Je tegoed voor deze maand is op."}"#)
            .create();
        assert_eq!(
            engine.dictate(request(speech(), None)).unwrap_err(),
            PennelError::Cloud {
                code: "quota".into(),
                message: "Je tegoed voor deze maand is op.".into()
            }
        );
    }

    #[test]
    fn stored_base_url_ignored_for_pennel_cloud() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = cloud_engine(&dir, &server, true, "http://127.0.0.1:1/v1");
        let _refresh = refresh_mock(&mut server);
        let stt = server
            .mock("POST", "/v1/audio/transcriptions")
            .with_body(r#"{"text":"hoi"}"#)
            .create();
        assert_eq!(engine.dictate(request(speech(), None)).unwrap().text, "hoi");
        stt.assert();
    }

    #[test]
    fn fast_mode_returns_raw_without_llm_call() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        stt(&mut server, "euh hallo");
        let llm_mock = llm(&mut server, "x").expect(0);
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "euh hallo");
        assert_eq!(result.cleanup, CleanupStatus::Skipped);
        llm_mock.assert();
    }

    fn history_writable(engine: &PennelEngine, writable: bool) {
        engine
            .history
            .lock()
            .unwrap()
            .connection_for_testing()
            .pragma_update(None, "query_only", !writable)
            .unwrap();
    }

    #[test]
    fn completed_text_survives_a_history_write_failure_and_save_does_not_recognize_again() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let recognition = stt(&mut server, "Bewaar deze tekst.").expect(1);
        history_writable(&engine, false);
        let result = engine
            .dictate(request(speech(), Some("test.editor")))
            .unwrap();
        assert!(!result.history_saved);
        assert_eq!(result.text, "Bewaar deze tekst.");
        assert_eq!(engine.unsaved_history_count(), 1);
        let entries = engine.history(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, result.history_id);
        assert_eq!(entries[0].status, HistoryStatus::Unsaved);
        assert_eq!(entries[0].text, result.text);
        assert_eq!(
            engine.history_entry(result.history_id.clone()).unwrap(),
            entries[0]
        );
        assert!(engine.save_history(result.history_id.clone()).is_err());
        assert_eq!(engine.unsaved_history_count(), 1);

        history_writable(&engine, true);
        engine.save_history(result.history_id.clone()).unwrap();
        engine.save_history(result.history_id.clone()).unwrap();
        assert_eq!(engine.unsaved_history_count(), 0);
        let entries = engine.history(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, result.history_id);
        assert_eq!(entries[0].status, HistoryStatus::Done);
        assert_eq!(
            engine.history_entry(result.history_id.clone()).unwrap(),
            entries[0]
        );
        assert_eq!(
            engine.history_entry("missing".into()),
            Err(PennelError::NotFound)
        );
        recognition.assert();
    }

    #[test]
    fn unsaved_history_remains_accessible_when_reading_sqlite_also_fails() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        stt(&mut server, "Tijdelijke tekst.");
        history_writable(&engine, false);
        let result = engine.dictate(request(speech(), None)).unwrap();
        history_writable(&engine, true);
        engine
            .history
            .lock()
            .unwrap()
            .connection_for_testing()
            .execute_batch("DROP TABLE dictations")
            .unwrap();
        assert_eq!(engine.history(10).unwrap()[0].id, result.history_id);
        engine.delete_history(result.history_id).unwrap();
        assert_eq!(engine.unsaved_history_count(), 0);
    }

    #[test]
    fn command_text_survives_a_history_write_failure() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "Schrijf het korter.");
        llm(&mut server, "Kort antwoord.");
        history_writable(&engine, false);
        let result = engine
            .command(request(speech(), None), Some("Een lang antwoord.".into()))
            .unwrap();
        assert!(!result.history_saved);
        assert_eq!(result.text, "Kort antwoord.");
        assert_eq!(
            engine.history(10).unwrap()[0].status,
            HistoryStatus::Unsaved
        );
    }

    #[test]
    fn retry_keeps_completed_text_and_original_audio_until_its_history_commit_succeeds() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let failing = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .create();
        assert!(engine.dictate(request(speech(), None)).is_err());
        let id = engine.history(1).unwrap()[0].id.clone();
        failing.remove();
        let recognition = stt(&mut server, "Herstelde tekst.").expect(1);
        history_writable(&engine, false);
        let recovered = engine.retry(id.clone(), keys()).unwrap();
        assert!(!recovered.history_saved);
        assert_eq!(recovered.history_id, id);
        assert_eq!(engine.history(10).unwrap().len(), 1);
        assert_eq!(engine.history(1).unwrap()[0].status, HistoryStatus::Unsaved);
        assert!(engine.history.lock().unwrap().failed_audio(&id).is_ok());
        history_writable(&engine, true);
        let saved = engine.retry(id.clone(), keys()).unwrap();
        assert!(saved.history_saved);
        assert_eq!(saved.text, "Herstelde tekst.");
        assert_eq!(
            engine.history.lock().unwrap().failed_audio(&id),
            Err(PennelError::NotFound)
        );
        recognition.assert();
    }

    #[test]
    fn retrying_all_temporary_history_preserves_failures_and_saves_each_result_once() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let recognition = stt(&mut server, "Nog niet bewaard.").expect(2);
        history_writable(&engine, false);
        engine.dictate(request(speech(), None)).unwrap();
        engine.dictate(request(speech(), None)).unwrap();
        assert!(engine.retry_unsaved_history().is_err());
        assert_eq!(engine.unsaved_history_count(), 2);
        history_writable(&engine, true);
        engine.retry_unsaved_history().unwrap();
        engine.retry_unsaved_history().unwrap();
        assert_eq!(engine.unsaved_history_count(), 0);
        assert_eq!(engine.history(10).unwrap().len(), 2);
        recognition.assert();
    }

    #[test]
    fn temporary_history_is_not_hidden_by_a_page_of_newer_saved_entries() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        stt(&mut server, "Een ouder tijdelijk dictaat.");
        history_writable(&engine, false);
        let result = engine.dictate(request(speech(), None)).unwrap();
        history_writable(&engine, true);
        let history = engine.history.lock().unwrap();
        for offset in 1..=250 {
            history
                .insert_done(None, "nieuwer", "Nieuwer.", now_ms() + offset)
                .unwrap();
        }
        drop(history);
        let page = engine.history(200).unwrap();
        assert_eq!(page.len(), 200);
        assert_eq!(page[0].id, result.history_id);
        assert_eq!(page[0].status, HistoryStatus::Unsaved);
    }

    #[test]
    fn recovery_capacity_blocks_before_recognition_and_explicit_discard_releases_it() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let recognition = stt(&mut server, "Bewaren lukt nog niet.").expect(20);
        history_writable(&engine, false);
        for _ in 0..20 {
            assert!(
                !engine
                    .dictate(request(speech(), None))
                    .unwrap()
                    .history_saved
            );
        }
        assert!(engine.ensure_history_capacity().is_err());
        assert!(engine.dictate(request(speech(), None)).is_err());
        assert_eq!(engine.unsaved_history_count(), 20);
        let discarded = engine.history(1).unwrap()[0].id.clone();
        engine.delete_history(discarded).unwrap();
        assert!(engine.ensure_history_capacity().is_ok());
        recognition.assert();
    }

    #[test]
    fn oversized_completed_text_is_never_truncated_and_blocks_the_next_capture() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let text = format!("{}Einde.", "Een groot resultaat. ".repeat(140_000));
        let recognition = stt(&mut server, &text).expect(1);
        history_writable(&engine, false);
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, text);
        assert_eq!(engine.history(1).unwrap()[0].text, text);
        assert!(engine.ensure_history_capacity().is_err());
        recognition.assert();
    }

    #[test]
    fn practice_uses_speech_only_and_preserves_the_normal_configuration() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let before = engine.settings();
        let transcription = stt(&mut server, "Een echt proefdictaat.");
        let cleanup = llm(&mut server, "Deze mag niet worden aangeroepen.").expect(0);
        let result = engine
            .dictate_practice(request(speech(), Some("app.pennel.practice")))
            .unwrap();
        assert_eq!(result.text, "Een echt proefdictaat.");
        assert_eq!(result.cleanup, CleanupStatus::Skipped);
        assert_eq!(engine.settings(), before);
        assert_eq!(engine.history(1).unwrap()[0].id, result.history_id);
        transcription.assert();
        cleanup.assert();
    }

    #[test]
    fn dictation_pad_keeps_edited_text_across_engine_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let engine = PennelEngine::new_preview(path.clone()).unwrap();
        assert!(engine.dictation_pad_text().is_empty());
        engine
            .save_dictation_pad("Handmatig aangepast.\n".into())
            .unwrap();
        assert_eq!(
            engine
                .append_dictation_pad("Een nieuw dictaat.".into())
                .unwrap(),
            "Handmatig aangepast.\nEen nieuw dictaat."
        );
        drop(engine);
        let reopened = PennelEngine::new_preview(path).unwrap();
        assert_eq!(
            reopened.dictation_pad_text(),
            "Handmatig aangepast.\nEen nieuw dictaat."
        );
        assert_eq!(
            reopened.append_dictation_pad(" ".into()).unwrap(),
            reopened.dictation_pad_text()
        );
    }

    #[test]
    fn clean_mode_applies_cleanup() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "euh ik kom morgen, nee overmorgen");
        llm(&mut server, "Ik kom overmorgen.");
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "Ik kom overmorgen.");
        assert_eq!(result.raw, "euh ik kom morgen, nee overmorgen");
        assert_eq!(result.cleanup, CleanupStatus::Applied);
        let history = engine.history(10).unwrap();
        assert_eq!(history[0].id, result.history_id);
        assert_eq!(history[0].text, "Ik kom overmorgen.");
    }

    #[test]
    fn llm_failure_falls_back_to_raw() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "hallo daar");
        server
            .mock("POST", "/chat/completions")
            .with_status(500)
            .with_body("boom")
            .create();
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "hallo daar");
        assert!(matches!(result.cleanup, CleanupStatus::Failed { .. }));
    }

    #[test]
    fn answer_like_cleanup_is_rejected() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "wat is de hoofdstad van frankrijk");
        llm(
            &mut server,
            &"De hoofdstad van Frankrijk is Parijs. ".repeat(6),
        );
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "wat is de hoofdstad van frankrijk");
        assert_eq!(result.cleanup, CleanupStatus::Rejected);
    }

    #[test]
    fn silence_returns_no_speech_without_network() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let stt_mock = stt(&mut server, "x").expect(0);
        assert_eq!(
            engine.dictate(request(vec![0.0; 16000], None)).unwrap_err(),
            PennelError::NoSpeech
        );
        stt_mock.assert();
        assert!(engine.history(10).unwrap().is_empty());
    }

    #[test]
    fn hallucination_returns_no_speech() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "Ondertiteld door de Amara.org gemeenschap");
        assert_eq!(
            engine.dictate(request(speech(), None)).unwrap_err(),
            PennelError::NoSpeech
        );
    }

    #[test]
    fn stt_failure_saves_audio_and_retry_succeeds() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let failing = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .with_body("down")
            .create();
        let err = engine
            .dictate(request(speech(), Some("com.apple.TextEdit")))
            .unwrap_err();
        assert!(matches!(err, PennelError::Http { status: 503, .. }));
        let entry = engine.history(10).unwrap().remove(0);
        assert_eq!(entry.status, HistoryStatus::Failed);
        assert!(entry.has_audio);

        failing.remove();
        stt(&mut server, "hallo");
        let result = engine.retry(entry.id.clone(), keys()).unwrap();
        assert_eq!(result.text, "hallo");
        assert_eq!(result.history_id, entry.id);
        let entry = engine.history(10).unwrap().remove(0);
        assert_eq!(entry.status, HistoryStatus::Done);
        assert_eq!(entry.text, "hallo");
    }

    #[test]
    fn missing_stt_key_errors_before_network() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let stt_mock = stt(&mut server, "x").expect(0);
        let mut req = request(speech(), None);
        req.keys.by_provider.clear();
        assert_eq!(
            engine.dictate(req).unwrap_err(),
            PennelError::MissingApiKey { task: "stt".into() }
        );
        stt_mock.assert();
    }

    #[test]
    fn custom_provider_without_key_sends_no_auth() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let mut settings = engine.settings();
        let mut custom =
            crate::provider::new_account(ProviderKind::Custom, settings.providers.clone());
        custom.base_url = server.url();
        settings.dictation_stt = crate::settings::ModelChoice {
            provider_id: custom.id.clone(),
            model: "w".into(),
        };
        settings.providers.push(custom);
        engine.update_settings(settings).unwrap();
        let mock = server
            .mock("POST", "/audio/transcriptions")
            .match_header("authorization", Matcher::Missing)
            .with_body(r#"{"text":"lokaal"}"#)
            .create();
        let mut req = request(speech(), None);
        req.keys.by_provider.clear();
        assert_eq!(engine.dictate(req).unwrap().text, "lokaal");
        mock.assert();
    }

    #[cfg(not(feature = "local-models"))]
    #[test]
    fn local_calls_without_feature_are_the_dutch_error() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let mut settings = engine.settings();
        settings.dictation_stt = crate::settings::ModelChoice {
            provider_id: "local".into(),
            model: "whisper-small-q5_1".into(),
        };
        engine.update_settings(settings.clone()).unwrap();
        let unavailable = PennelError::local_models_unavailable();
        assert_eq!(
            engine.dictate(request(speech(), None)).unwrap_err(),
            unavailable
        );
        assert_eq!(
            engine.preview_local(
                &settings,
                &[0.1; 16_000],
                &crate::local_engine::CancelToken::default()
            ),
            Err(unavailable)
        );
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn local_without_model_is_model_not_downloaded() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let mut settings = engine.settings();
        settings.dictation_stt = crate::settings::ModelChoice {
            provider_id: "local".into(),
            model: "whisper-small-q5_1".into(),
        };
        engine.update_settings(settings).unwrap();
        assert_eq!(
            engine.dictate(request(speech(), None)).unwrap_err(),
            PennelError::ModelNotDownloaded
        );
        let entry = engine.history(1).unwrap().remove(0);
        assert_eq!(entry.status, HistoryStatus::Failed);
        assert!(entry.has_audio);
    }

    struct Silent;
    impl crate::models::DownloadListener for Silent {
        fn on_progress(&self, _: u64, _: u64) {}
    }

    #[cfg(feature = "local-models")]
    struct DownloadGate {
        entered: std::sync::Barrier,
        release: std::sync::Barrier,
        first: std::sync::atomic::AtomicBool,
    }

    #[cfg(feature = "local-models")]
    impl DownloadGate {
        fn new() -> Self {
            Self {
                entered: std::sync::Barrier::new(2),
                release: std::sync::Barrier::new(2),
                first: std::sync::atomic::AtomicBool::new(true),
            }
        }
    }

    #[cfg(feature = "local-models")]
    impl DownloadListener for DownloadGate {
        fn on_progress(&self, _: u64, _: u64) {
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                self.entered.wait();
                self.release.wait();
            }
        }
    }

    #[cfg(feature = "local-models")]
    struct JoinedProgress {
        joined: std::sync::mpsc::Sender<()>,
        progress: Mutex<Vec<(u64, u64)>>,
    }

    #[cfg(feature = "local-models")]
    impl DownloadListener for JoinedProgress {
        fn on_progress(&self, downloaded: u64, total: u64) {
            self.progress.lock().unwrap().push((downloaded, total));
            let _ = self.joined.send(());
        }
    }

    fn download_engine(dir: &tempfile::TempDir, url: String, bytes: &[u8]) -> Arc<PennelEngine> {
        use sha2::Digest;
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut engine = Arc::try_unwrap(engine).ok().unwrap();
        engine.models = ModelStore::with_catalog(
            dir.path(),
            vec![crate::models::ModelEntry {
                id: "tiny".into(),
                name: "Tiny".into(),
                description: "test".into(),
                size_bytes: bytes.len() as u64,
                sha256: crate::models::hex(&sha2::Sha256::digest(bytes)),
                url,
                scores: None,
                legacy: false,
                kind: crate::models::ModelKind::Speech,
            }],
        );
        Arc::new(engine)
    }

    #[cfg(feature = "local-models")]
    fn shared_download(cancel: bool, join_only: bool) {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = vec![b'm'; 3 * 1024 * 1024];
        let request = server
            .mock("GET", "/model.bin")
            .with_body(&bytes)
            .expect(1)
            .create();
        let engine = download_engine(&dir, format!("{}/model.bin", server.url()), &bytes);
        let gate = Arc::new(DownloadGate::new());
        let owner = {
            let engine = engine.clone();
            let gate = gate.clone();
            std::thread::spawn(move || engine.download_model("tiny".into(), gate))
        };
        gate.entered.wait();
        assert!(engine.delete_model("tiny".into()).is_err());
        let (sender, receiver) = std::sync::mpsc::channel();
        let listener = Arc::new(JoinedProgress {
            joined: sender,
            progress: Mutex::new(Vec::new()),
        });
        let joined = {
            let engine = engine.clone();
            let listener = listener.clone();
            std::thread::spawn(move || {
                if join_only {
                    engine
                        .join_model_download("tiny".into(), listener)
                        .map(|joined| assert!(joined))
                } else {
                    engine.download_model("tiny".into(), listener)
                }
            })
        };
        receiver
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        if cancel {
            engine.cancel_download("tiny".into());
        }
        gate.release.wait();
        let (first, second) = (owner.join().unwrap(), joined.join().unwrap());
        assert_eq!(first, second);
        if cancel {
            assert_eq!(first, Err(PennelError::Cancelled));
            assert!(matches!(
                engine.model_state("tiny".into()),
                ModelState::Partial { .. }
            ));
        } else {
            assert_eq!(first, Ok(()));
            let total = bytes.len() as u64;
            assert_eq!(
                listener.progress.lock().unwrap().last(),
                Some(&(total, total))
            );
            assert_eq!(engine.model_state("tiny".into()), ModelState::Downloaded);
        }
        assert!(engine.downloads.lock().unwrap().is_empty());
        request.assert();
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn concurrent_model_downloads_share_one_writer_and_progress() {
        shared_download(false, false);
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn joining_an_active_download_waits_for_its_existing_worker() {
        shared_download(false, true);
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn cancelling_a_shared_download_releases_waiters_and_keeps_partial_model() {
        shared_download(true, true);
    }

    #[test]
    fn joining_an_absent_download_does_not_start_network_or_create_a_partial() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = b"model";
        let request = server.mock("GET", "/model.bin").expect(0).create();
        let engine = download_engine(&dir, format!("{}/model.bin", server.url()), bytes);
        assert_eq!(
            engine.join_model_download("tiny".into(), Arc::new(Silent)),
            Ok(false)
        );
        assert_eq!(engine.model_state("tiny".into()), ModelState::NotDownloaded);
        assert!(engine.active_model_downloads().is_empty());
        request.assert();
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn engine_model_api_reports_state_and_unknown_ids() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            engine.model_state("whisper-small-q5_1".into()),
            crate::models::ModelState::NotDownloaded
        );
        assert_eq!(
            engine
                .download_model("nope".into(), Arc::new(Silent))
                .unwrap_err(),
            PennelError::NotFound
        );
        engine.cancel_download("whisper-small-q5_1".into());
        engine.delete_model("whisper-small-q5_1".into()).unwrap();
    }

    #[cfg(not(feature = "local-models"))]
    #[test]
    fn local_models_list_empty_without_feature() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        assert!(engine.local_models(None, false).is_empty());
        assert_eq!(
            engine
                .download_model("whisper-small-q5_1".into(), Arc::new(Silent))
                .unwrap_err(),
            PennelError::local_models_unavailable()
        );
        assert_eq!(
            engine
                .download_model(engine.diarizer_model_id(), Arc::new(Silent))
                .unwrap_err(),
            PennelError::local_models_unavailable()
        );
    }

    #[test]
    fn unload_idle_models_without_loaded_model_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        assert!(!engine.unload_idle_models());
    }

    #[test]
    fn style_follows_app_bundle() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "hoi");
        let llm_mock = server
            .mock("POST", "/chat/completions")
            .match_body(Matcher::Regex("casual chat message".into()))
            .with_body(r#"{"choices":[{"message":{"content":"Hoi"}}]}"#)
            .create();
        engine
            .dictate(request(speech(), Some("com.tinyspeck.slackmacgap")))
            .unwrap();
        llm_mock.assert();
    }

    #[test]
    fn dictionary_is_sent_as_stt_prompt() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let mut settings = engine.settings();
        settings.dictionary = vec!["Pennel".into(), "Q8".into()];
        engine.update_settings(settings).unwrap();
        let stt_mock = server
            .mock("POST", "/audio/transcriptions")
            .match_body(Matcher::Regex("name=\"prompt\"\r\n\r\nPennel, Q8".into()))
            .with_body(r#"{"text":"ok"}"#)
            .create();
        engine.dictate(request(speech(), None)).unwrap();
        stt_mock.assert();
    }

    #[test]
    fn settings_persist_across_engines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let engine = PennelEngine::new(path.clone()).unwrap();
        let mut settings = engine.settings();
        settings.mode = DictationMode::Fast;
        engine.update_settings(settings).unwrap();
        assert_eq!(
            PennelEngine::new(path).unwrap().settings().mode,
            DictationMode::Fast
        );
    }

    #[test]
    fn opening_a_data_directory_twice_reuses_its_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let first = PennelEngine::new(path.clone()).unwrap();
        let second = PennelEngine::new(path).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn preview_meetings_stay_inside_the_preview_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            PathBuf::from(engine.settings().meetings_dir),
            dir.path().canonicalize().unwrap().join("Meetings")
        );
        let mut settings = engine.settings();
        settings.meetings_dir.clear();
        engine.update_settings(settings).unwrap();
        assert_eq!(
            PathBuf::from(engine.settings().meetings_dir),
            dir.path().canonicalize().unwrap().join("Meetings")
        );
        assert!(PennelEngine::new(dir.path().to_string_lossy().into_owned()).is_err());
    }

    #[test]
    fn preview_rejects_saved_external_library_before_touching_it() {
        let data = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let marker = external.path().join("meeting.md");
        std::fs::write(&marker, "bestaande echte meeting").unwrap();
        let mut settings = crate::default_settings();
        settings.meetings_dir = external.path().to_string_lossy().into_owned();
        settings.save(&data.path().join("settings.json")).unwrap();
        let before = std::fs::read(data.path().join("settings.json")).unwrap();
        assert!(PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).is_err());
        assert_eq!(
            std::fs::read(data.path().join("settings.json")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read_to_string(marker).unwrap(),
            "bestaande echte meeting"
        );
        assert!(!external.path().join(".pennel-library.lock").exists());
        assert!(!external.path().join("transcript.md").exists());
    }

    #[test]
    fn preview_rejects_external_library_updates_without_creating_a_folder() {
        let data = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let untouched = external.path().join("production-meetings");
        let engine = PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).unwrap();
        let original = engine.settings();
        let mut settings = original.clone();
        settings.meetings_dir = untouched.to_string_lossy().into_owned();
        assert!(engine.update_settings(settings).is_err());
        assert_eq!(engine.settings(), original);
        assert!(!untouched.exists());
        assert!(!data.path().join("settings.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn preview_rejects_a_meetings_symlink_into_real_data() {
        let data = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(external.path(), data.path().join("Meetings")).unwrap();
        assert!(PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).is_err());
        assert!(!external.path().join(".pennel-library.lock").exists());
        assert_eq!(std::fs::read_dir(external.path()).unwrap().count(), 0);
    }

    #[test]
    fn meeting_preflight_checks_cloud_key_without_network_or_meeting_files() {
        let data = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers[0].base_url = "http://127.0.0.1:1".into();
        settings.meeting_stt.provider_id = "groq".into();
        engine.update_settings(settings).unwrap();
        assert_eq!(
            engine.validate_meeting_setup(ApiKeys {
                by_provider: HashMap::new()
            }),
            Err(PennelError::MissingApiKey {
                task: "meeting_stt".into()
            })
        );
        assert_eq!(engine.validate_meeting_setup(keys()), Ok(()));
        assert!(engine.meetings().is_empty());
    }

    #[test]
    fn meeting_preflight_rejects_empty_models_and_llm_only_providers() {
        let data = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.meeting_stt.model.clear();
        engine.update_settings(settings.clone()).unwrap();
        assert_eq!(
            engine.validate_meeting_setup(keys()),
            Err(PennelError::NotFound)
        );
        let agent = crate::new_account(ProviderKind::ClaudeCode, settings.providers.clone());
        settings.meeting_stt = ModelChoice {
            provider_id: agent.id.clone(),
            model: "haiku".into(),
        };
        settings.providers.push(agent);
        engine.update_settings(settings).unwrap();
        assert_eq!(
            engine.validate_meeting_setup(keys()),
            Err(PennelError::NotFound)
        );
    }

    #[test]
    fn meeting_preflight_requires_a_downloaded_local_speech_model() {
        let data = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.meeting_stt = ModelChoice {
            provider_id: "local".into(),
            model: "whisper-small-q5_1".into(),
        };
        engine.update_settings(settings).unwrap();
        assert_eq!(
            engine.validate_meeting_setup(keys()),
            Err(PennelError::ModelNotDownloaded)
        );
        std::fs::create_dir_all(data.path().join("models")).unwrap();
        std::fs::write(
            data.path().join("models/whisper-small-q5_1.bin"),
            b"fixture, never loaded",
        )
        .unwrap();
        assert_eq!(
            engine.validate_meeting_setup(ApiKeys {
                by_provider: HashMap::new()
            }),
            Ok(())
        );
    }

    #[test]
    fn meeting_preflight_checks_system_provider_registration_without_transcribing() {
        struct System(std::sync::atomic::AtomicBool);
        impl SystemTranscriber for System {
            fn transcribe(
                &self,
                _: Vec<f32>,
                _: u32,
                _: Option<String>,
                _: Vec<String>,
            ) -> Result<String, PennelError> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(String::new())
            }
        }
        let data = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(data.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        let apple = crate::new_account(ProviderKind::AppleSpeech, settings.providers.clone());
        settings.meeting_stt = ModelChoice {
            provider_id: apple.id.clone(),
            model: "dictation".into(),
        };
        settings.providers.push(apple);
        engine.update_settings(settings).unwrap();
        assert!(matches!(
            engine.validate_meeting_setup(keys()),
            Err(PennelError::InvalidResponse { .. })
        ));
        let transcriber = Arc::new(System(std::sync::atomic::AtomicBool::new(false)));
        engine.set_system_transcriber(transcriber.clone());
        assert_eq!(
            engine.validate_meeting_setup(ApiKeys {
                by_provider: HashMap::new()
            }),
            Ok(())
        );
        assert!(!transcriber.0.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn invalid_existing_settings_are_backed_up_without_resetting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{invalid settings").unwrap();
        assert!(PennelEngine::new(dir.path().to_string_lossy().into_owned()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{invalid settings");
        assert!(!dir.path().join("settings.corrupt.json").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("migration-backup-v1/settings.json")).unwrap(),
            "{invalid settings"
        );
    }

    #[test]
    fn startup_snapshot_precedes_retention_of_existing_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path()).unwrap();
        let id = store
            .insert_failed(None, b"retry audio", "offline", 0)
            .unwrap();
        drop(store);
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        assert!(engine.history(10).unwrap().is_empty());
        let backup = HistoryStore::open(&dir.path().join("migration-backup-v1")).unwrap();
        assert_eq!(backup.list(10).unwrap().len(), 1);
        assert_eq!(backup.failed_audio(&id).unwrap(), b"retry audio");
    }

    #[test]
    fn local_vocabulary_is_capped() {
        let terms: Vec<String> = (0..300).map(|i| format!("term{i:03}")).collect();
        let capped = capped_terms(&terms, MAX_PROMPT_CHARS);
        assert!(capped.join(", ").chars().count() <= MAX_PROMPT_CHARS);
        assert_eq!(capped[0], "term000");
        assert!(capped.len() > 50);
        assert_eq!(capped_terms(&terms[..2], MAX_PROMPT_CHARS), terms[..2]);
    }

    fn meeting_engine(dir: &tempfile::TempDir, server: &mockito::Server) -> Arc<PennelEngine> {
        let engine = engine(dir, server, DictationMode::Clean);
        let mut settings = engine.settings();
        settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        engine
    }

    fn add_meeting(engine: &PennelEngine, title: &str, hour: u32, said: &str) -> String {
        use chrono::TimeZone;
        let store = engine.meeting_store();
        std::fs::create_dir_all(engine.settings().meetings_dir).unwrap();
        let id = store
            .create(
                title,
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 20, hour, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        let segment = crate::meetings::Segment {
            start_ms: 1000,
            end_ms: 2000,
            speaker: crate::meetings::Speaker::Others,
            text: said.into(),
            failed: false,
            voice: None,
        };
        store.write_transcript(&id, &[segment]).unwrap();
        id
    }

    fn meeting_with_recovery(engine: &PennelEngine, count: u64) -> (String, Vec<PathBuf>) {
        let id = add_meeting(engine, "Retry recording", 9, "Original placeholder.");
        let store = engine.meeting_store();
        let mut rows = Vec::new();
        let mut paths = Vec::new();
        for number in 0..count {
            let start_ms = number * 1_000;
            let path = store
                .write_recovery(
                    &id,
                    crate::meetings::Speaker::Others,
                    start_ms,
                    &encode_wav(&speech(), 16_000),
                )
                .unwrap();
            let mut row = crate::MeetingUtterance::new(
                crate::meetings::Segment {
                    start_ms,
                    end_ms: start_ms + 1_000,
                    speaker: crate::meetings::Speaker::Others,
                    text: String::new(),
                    failed: true,
                    voice: None,
                },
                Some("system_audio".into()),
            );
            row.metadata.recovery_audio = crate::meetings::recovery_provenance(
                &path,
                crate::meeting_utterances::RecoveryAudioDisposition::Pending,
            );
            rows.push(row);
            paths.push(path);
        }
        store.write_transcript_snapshot(&id, None, &rows).unwrap();
        (id, paths)
    }

    #[test]
    fn failed_recovery_plan_projection_is_coherent_and_keeps_provenance_private() {
        let (directory, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&directory, &server);
        let (id, _) = meeting_with_recovery(&engine, 2);

        let (json, plan) = engine
            .meeting_transcript_snapshot_with_recovery_plan(&id)
            .unwrap();

        assert_eq!(plan.failed_count, Some(2));
        assert_eq!(plan.retryable_count, Some(2));
        assert_eq!(
            plan.status,
            crate::FailedTranscriptionRecoveryStatus::Retryable
        );
        assert_eq!(json, engine.meeting_transcript_snapshot_json(id).unwrap());
        let projection: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(projection["transcript"].as_array().unwrap().len(), 2);
        assert!(!json.contains("recovery_audio"));
        assert!(!json.contains("file_name"));
        assert!(!json.contains("sha256"));
    }

    #[test]
    fn failed_recovery_plan_projection_keeps_corrupt_metadata_counts_unknown() {
        let (directory, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&directory, &server);
        let (id, _) = meeting_with_recovery(&engine, 1);
        engine
            .set_meeting_notes(id.clone(), "Private fixture note stays.".into())
            .unwrap();
        let root = engine.meeting_store().dir_of(&id);
        std::fs::write(root.join("transcript.meta.json"), b"owned invalid metadata").unwrap();
        let before = ["transcript.md", "transcript.meta.json", "notes.md"]
            .map(|name| std::fs::read(root.join(name)).unwrap());
        let recognition = stt(&mut server, "Must not be requested.").expect(0);

        let (json, plan) = engine
            .meeting_transcript_snapshot_with_recovery_plan(&id)
            .unwrap();

        assert_eq!(plan.failed_count, None);
        assert_eq!(plan.retryable_count, None);
        assert_eq!(
            plan.status,
            crate::FailedTranscriptionRecoveryStatus::Unavailable
        );
        assert_eq!(
            plan.reason_code,
            Some(crate::FailedTranscriptionRecoveryReason::MetadataUnavailable)
        );
        let projection: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(projection["metadata_status"], "corrupt");
        assert_eq!(
            ["transcript.md", "transcript.meta.json", "notes.md"]
                .map(|name| std::fs::read(root.join(name)).unwrap()),
            before
        );
        recognition.assert();
    }

    #[test]
    fn meeting_retry_revalidates_pcm_after_the_header_only_plan_before_provider_calls() {
        let (directory, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&directory, &server);
        let (id, paths) = meeting_with_recovery(&engine, 1);
        engine
            .set_meeting_notes(id.clone(), "Private fixture note stays.".into())
            .unwrap();
        let store = engine.meeting_store();
        let root = store.dir_of(&id);
        let before = ["transcript.md", "transcript.meta.json", "notes.md"]
            .map(|name| std::fs::read(root.join(name)).unwrap());
        let (_, plan) = engine
            .meeting_transcript_snapshot_with_recovery_plan(&id)
            .unwrap();
        assert_eq!(plan.retryable_count, Some(1));
        let mut changed = std::fs::read(&paths[0]).unwrap();
        changed[44] ^= 1;
        std::fs::write(&paths[0], &changed).unwrap();
        let (_, current) = engine
            .meeting_transcript_snapshot_with_recovery_plan(&id)
            .unwrap();
        assert_eq!(current.retryable_count, Some(1));
        let recognition = stt(&mut server, "Must not be requested.").expect(0);

        let error = engine.retranscribe_failed(id, keys()).unwrap_err();

        assert!(matches!(error, PennelError::Storage { .. }));
        assert_eq!(
            ["transcript.md", "transcript.meta.json", "notes.md"]
                .map(|name| std::fs::read(root.join(name)).unwrap()),
            before
        );
        assert_eq!(std::fs::read(&paths[0]).unwrap(), changed);
        recognition.assert();
    }

    #[test]
    fn meeting_retry_preserves_corrupt_audio_and_recovers_later_healthy_chunks() {
        let (directory, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&directory, &server);
        let (id, paths) = meeting_with_recovery(&engine, 2);
        engine
            .set_meeting_notes(id.clone(), "Private fixture note stays.".into())
            .unwrap();
        let store = engine.meeting_store();
        let root = store.dir_of(&id);
        let audio = root.join("audio");
        std::fs::create_dir_all(&audio).unwrap();
        let archive_paths = [audio.join("microphone.m4a"), audio.join("system.m4a")];
        for path in &archive_paths {
            std::fs::write(path, b"owned archive preservation fixture").unwrap();
        }
        let archives = archive_paths
            .each_ref()
            .map(|path| std::fs::read(path).unwrap());
        let notes = std::fs::read(root.join("notes.md")).unwrap();
        let healthy = std::fs::read(&paths[1]).unwrap();
        let mut corrupt = std::fs::read(&paths[0]).unwrap();
        corrupt[44] ^= 1;
        std::fs::write(&paths[0], &corrupt).unwrap();
        let expected = healthy.clone();
        let forbidden = corrupt.clone();
        let recognition = server
            .mock("POST", "/audio/transcriptions")
            .with_body_from_request(move |request| {
                let body = request.body().unwrap();
                assert!(body.windows(expected.len()).any(|part| part == expected));
                assert!(body.windows(forbidden.len()).all(|part| part != forbidden));
                br#"{"text":"Recovered healthy passage."}"#.to_vec()
            })
            .expect(1)
            .create();

        let error = engine.retranscribe_failed(id.clone(), keys()).unwrap_err();

        assert!(matches!(error, PennelError::Storage { .. }));
        let saved = store.read_committed_transcript_snapshot(&id).unwrap();
        assert!(saved.utterances[0].segment.failed);
        assert!(!saved.utterances[1].segment.failed);
        assert_eq!(
            saved.utterances[1].segment.text,
            "Recovered healthy passage."
        );
        assert_eq!(std::fs::read(root.join("notes.md")).unwrap(), notes);
        assert_eq!(
            archive_paths
                .each_ref()
                .map(|path| std::fs::read(path).unwrap()),
            archives
        );
        assert_eq!(std::fs::read(&paths[0]).unwrap(), corrupt);
        assert!(!paths[1].exists());
        recognition.assert();
        recognition.remove();

        let no_repeat = stt(&mut server, "Must not be repeated.").expect(0);
        let manifest = std::fs::read(root.join("transcript.meta.json")).unwrap();
        assert!(engine.retranscribe_failed(id.clone(), keys()).is_err());
        assert_eq!(
            store.read_committed_transcript_snapshot(&id).unwrap(),
            saved
        );
        assert_eq!(
            std::fs::read(root.join("transcript.meta.json")).unwrap(),
            manifest
        );
        assert_eq!(std::fs::read(root.join("notes.md")).unwrap(), notes);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), corrupt);
        no_repeat.assert();
    }

    #[test]
    fn meeting_retry_reports_provider_failure_without_sending_later_chunks() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let (id, paths) = meeting_with_recovery(&engine, 3);
        engine
            .set_meeting_notes(id.clone(), "Owner notes stay unchanged.".into())
            .unwrap();
        let store = engine.meeting_store();
        let before = store.read_committed_transcript_snapshot(&id).unwrap();
        let transcript = std::fs::read(store.dir_of(&id).join("transcript.md")).unwrap();
        let manifest = std::fs::read(store.dir_of(&id).join("transcript.meta.json")).unwrap();
        let notes = std::fs::read(store.dir_of(&id).join("notes.md")).unwrap();
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .with_body("Untrusted provider response.")
            .expect(1)
            .create();

        let error = engine.retranscribe_failed(id.clone(), keys()).unwrap_err();

        assert!(matches!(error, PennelError::Http { status: 503, .. }));
        assert_eq!(
            store.read_committed_transcript_snapshot(&id).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(store.dir_of(&id).join("transcript.md")).unwrap(),
            transcript
        );
        assert_eq!(
            std::fs::read(store.dir_of(&id).join("transcript.meta.json")).unwrap(),
            manifest
        );
        assert_eq!(
            std::fs::read(store.dir_of(&id).join("notes.md")).unwrap(),
            notes
        );
        assert!(paths.iter().all(|path| path.exists()));
        assert!(
            engine
                .meeting_transcript(id)
                .iter()
                .all(|segment| segment.failed)
        );
        request.assert();
    }

    #[test]
    fn meeting_retry_keeps_successful_prefix_and_later_retries_only_remaining_chunks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let (id, paths) = meeting_with_recovery(&engine, 3);
        let calls = Arc::new(AtomicUsize::new(0));
        let status_calls = calls.clone();
        let body_calls = calls.clone();
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_status_code_from_request(move |_| {
                if status_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    200
                } else {
                    503
                }
            })
            .with_body_from_request(move |_| {
                if body_calls.load(Ordering::SeqCst) == 1 {
                    br#"{"text":"Recovered prefix."}"#.to_vec()
                } else {
                    b"Provider unavailable.".to_vec()
                }
            })
            .expect(2)
            .create();

        let error = engine.retranscribe_failed(id.clone(), keys()).unwrap_err();

        assert!(matches!(error, PennelError::Http { status: 503, .. }));
        request.assert();
        let saved = engine.meeting_transcript(id.clone());
        assert_eq!(saved[0].text, "Recovered prefix.");
        assert!(!saved[0].failed);
        assert!(saved[1..].iter().all(|segment| segment.failed));
        assert!(!paths[0].exists());
        assert!(paths[1..].iter().all(|path| path.exists()));
        request.remove();
        let remaining = stt(&mut server, "Recovered remaining.").expect(2);

        assert_eq!(engine.retranscribe_failed(id.clone(), keys()).unwrap(), 2);

        remaining.assert();
        let saved = engine.meeting_transcript(id);
        assert_eq!(saved[0].text, "Recovered prefix.");
        assert!(saved.iter().all(|segment| !segment.failed));
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn meeting_retry_sanitizes_cloud_provider_errors() {
        let private =
            "Bearer synthetic-secret https://user:password@example.test/private dictated words";
        for (status, code, expected_code) in [
            (429, "quota", "quota"),
            (
                402,
                "upstream_payment_required",
                "upstream_payment_required",
            ),
            (503, "upstream", "upstream"),
            (503, private, "upstream"),
        ] {
            let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
            let engine = cloud_engine(&dir, &server, true, "");
            let mut settings = engine.settings();
            settings.meeting_stt = settings.dictation_stt.clone();
            settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
            engine.update_settings(settings).unwrap();
            let (id, paths) = meeting_with_recovery(&engine, 2);
            let refresh = refresh_mock(&mut server).expect(1);
            let request = server
                .mock("POST", "/v1/audio/transcriptions")
                .with_status(status)
                .with_body(serde_json::json!({"error": code, "message": private}).to_string())
                .expect(1)
                .create();

            let error = engine.retranscribe_failed(id, keys()).unwrap_err();

            assert!(matches!(&error, PennelError::Cloud { code, .. } if code == expected_code));
            if expected_code == "upstream_payment_required" {
                assert_eq!(
                    error.to_string(),
                    "The provider reports insufficient credit. Check your account."
                );
            }
            for token in [
                "synthetic-secret",
                "password",
                "example.test",
                "dictated words",
            ] {
                assert!(!format!("{error:?}").contains(token));
            }
            assert!(paths.iter().all(|path| path.exists()));
            refresh.assert();
            request.assert();
        }
    }

    #[test]
    fn meeting_retry_sanitizes_native_provider_errors() {
        struct FailingSpeech(PennelError);
        impl SystemTranscriber for FailingSpeech {
            fn transcribe(
                &self,
                _samples: Vec<f32>,
                _sample_rate: u32,
                _language: Option<String>,
                _vocabulary: Vec<String>,
            ) -> Result<String, PennelError> {
                Err(self.0.clone())
            }
        }
        let private =
            "Bearer synthetic-secret https://user:password@example.test/private dictated words";
        for failure in [
            PennelError::Network {
                message: private.into(),
            },
            PennelError::InvalidResponse {
                message: private.into(),
            },
            PennelError::Storage {
                message: private.into(),
            },
            PennelError::MissingApiKey {
                task: private.into(),
            },
        ] {
            let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
            let engine = meeting_engine(&dir, &server);
            let mut settings = engine.settings();
            let account = crate::provider::new_account(ProviderKind::AppleSpeech, vec![]);
            settings.meeting_stt = ModelChoice {
                provider_id: account.id.clone(),
                model: "apple".into(),
            };
            settings.providers.push(account);
            engine.update_settings(settings).unwrap();
            engine.set_system_transcriber(Arc::new(FailingSpeech(failure)));
            let (id, paths) = meeting_with_recovery(&engine, 1);

            let error = engine.retranscribe_failed(id, keys()).unwrap_err();

            for token in [
                "synthetic-secret",
                "password",
                "example.test",
                "dictated words",
            ] {
                assert!(!format!("{error:?}").contains(token));
            }
            assert!(paths.iter().all(|path| path.exists()));
        }
    }

    #[test]
    fn meeting_retry_never_removes_audio_before_the_transcript_commit() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Herstel", 9, "tijdelijk");
        let store = engine.meeting_store();
        let mut segment = store.read_transcript(&id).remove(0);
        segment.failed = true;
        store.write_transcript(&id, &[segment.clone()]).unwrap();
        let path =
            crate::session::failed_path(&store.dir_of(&id), segment.speaker, segment.start_ms);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, encode_wav(&speech(), 16_000)).unwrap();
        let transcript = store.dir_of(&id).join("transcript.md");
        let previous_transcript = std::fs::read(&transcript).unwrap();
        std::fs::remove_file(&transcript).unwrap();
        std::fs::create_dir(&transcript).unwrap();
        let recognition = stt(&mut server, "De herstelde zin.").expect(1);
        assert!(engine.retranscribe_failed(id.clone(), keys()).is_err());
        assert!(engine.retranscribe_failed(id.clone(), keys()).is_err());
        assert!(path.exists());
        std::fs::remove_dir(&transcript).unwrap();
        std::fs::write(&transcript, previous_transcript).unwrap();
        assert_eq!(engine.retranscribe_failed(id.clone(), keys()).unwrap(), 1);
        assert!(!path.exists());
        assert_eq!(store.read_transcript(&id)[0].text, "De herstelde zin.");
        recognition.assert();
    }

    #[test]
    fn meeting_retry_cleans_already_committed_recovery_audio_without_recognition() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Al bewaard", 9, "Deze zin is al bewaard.");
        let store = engine.meeting_store();
        let segment = store.read_transcript(&id).remove(0);
        let path =
            crate::session::failed_path(&store.dir_of(&id), segment.speaker, segment.start_ms);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, encode_wav(&speech(), 16_000)).unwrap();
        let mut rows = store.read_transcript_snapshot(&id).unwrap().utterances;
        rows.retain(|row| !row.segment.failed);
        rows[0].metadata.recovery_audio = crate::meetings::recovery_provenance(
            &path,
            crate::meeting_utterances::RecoveryAudioDisposition::Complete,
        );
        store.write_transcript_snapshot(&id, None, &rows).unwrap();
        let recognition = stt(&mut server, "Geen nieuwe aanvraag.").expect(0);
        assert_eq!(engine.retranscribe_failed(id.clone(), keys()).unwrap(), 0);
        assert!(!path.exists());
        assert_eq!(
            store.read_transcript(&id)[0].text,
            "Deze zin is al bewaard."
        );
        recognition.assert();
    }

    #[test]
    fn ambiguous_legacy_completion_retains_audio_and_does_not_send_it() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(
            &engine,
            "Owned unverified recovery",
            9,
            "Already readable text.",
        );
        let store = engine.meeting_store();
        let path = store
            .write_recovery(
                &id,
                crate::meetings::Speaker::Others,
                1_000,
                &encode_wav(&speech(), 16_000),
            )
            .unwrap();
        let recognition = stt(&mut server, "Must not be sent.").expect(0);
        assert!(engine.retranscribe_failed(id.clone(), keys()).is_err());
        assert!(engine.retranscribe_failed(id.clone(), keys()).is_err());
        assert!(path.exists());
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        assert!(
            snapshot
                .utterances
                .iter()
                .any(|row| row.segment.text == "Already readable text.")
        );
        assert!(snapshot.utterances.iter().any(|row| {
            row.segment.failed
                && row.metadata.recovery_audio.as_ref().is_some_and(|proof| {
                    proof.disposition
                        == crate::meeting_utterances::RecoveryAudioDisposition::Unverified
                })
        }));
        recognition.assert();
    }

    #[test]
    fn metadata_only_peer_rewrite_cannot_resurrect_private_origin_recovery_audio() {
        let (directory, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&directory, &server);
        let origin = engine.meeting_store();
        let id = add_meeting(
            &engine,
            "Owned portable private disposition",
            9,
            "Placeholder.",
        );
        let path = origin
            .write_recovery(
                &id,
                crate::meetings::Speaker::Me,
                0,
                &encode_wav(&speech(), 16_000),
            )
            .unwrap();
        let mut segment = origin.read_transcript(&id).remove(0);
        segment.start_ms = 0;
        segment.end_ms = 1_000;
        segment.text.clear();
        segment.failed = false;
        let mut row = crate::MeetingUtterance::new(segment, Some("microphone".into()));
        row.metadata.recovery_audio = crate::meetings::recovery_provenance(
            &path,
            crate::meeting_utterances::RecoveryAudioDisposition::Complete,
        );
        origin.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let peer_root = tempfile::tempdir().unwrap();
        let peer = MeetingStore::new(peer_root.path());
        fn transfer_metadata(source: &std::path::Path, destination: &std::path::Path) {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap().map(Result::unwrap) {
                if entry.file_type().unwrap().is_file() {
                    std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
                }
            }
        }
        transfer_metadata(&origin.dir_of(&id), &peer.dir_of(&id));
        assert!(!peer.dir_of(&id).join("failed").exists());
        peer.write_transcript_snapshot(&id, None, &[]).unwrap();
        assert_eq!(
            peer.read_committed_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .len(),
            1,
            "a peer without private PCM cannot retire the origin's durable completion"
        );
        transfer_metadata(&peer.dir_of(&id), &origin.dir_of(&id));
        let recognition = stt(&mut server, "Private audio must not be sent.").expect(0);
        assert!(origin.read_transcript(&id).is_empty());
        assert_eq!(engine.retranscribe_failed(id.clone(), keys()).unwrap(), 0);
        assert!(!path.exists());
        assert!(origin.read_transcript(&id).is_empty());
        recognition.assert();
    }

    #[test]
    fn search_meetings_uses_titles_then_full_text() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let a = add_meeting(&engine, "Budget review", 9, "niets bijzonders");
        let b = add_meeting(&engine, "Weekly", 10, "het budget is goedgekeurd");
        add_meeting(&engine, "Lunch", 11, "pasta");
        let ids: Vec<String> = engine
            .search_meetings("budget".into())
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, [a, b]);
        let hits = engine.search_passages("goedgekeurd".into());
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("«goedgekeurd»"));
    }

    #[test]
    fn no_terms_uses_recent_passages() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        add_meeting(&engine, "Oud", 8, "oude kiwi");
        add_meeting(&engine, "Nieuw", 11, "verse mango");
        let mock = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex("verse mango".into()))
            .with_body(r#"{"choices":[{"message":{"content":"Mango [1]."}}]}"#)
            .create();
        let answer = engine
            .ask_meetings("wat?".into(), vec![], None, keys())
            .unwrap();
        mock.assert();
        assert_eq!(answer.text, "Mango [1].");
        assert_eq!(answer.citations.len(), 1);
        assert_eq!(answer.citations[0].passage.title, "Nieuw");
    }

    #[test]
    fn single_meeting_sends_whole_meeting() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Sync", 9, "we verhuizen naar Gent");
        add_meeting(&engine, "Ander", 10, "geheime ananas");
        engine
            .meeting_store()
            .write_meeting(&id, 30, "algemeen", "## Besluiten\n- Verhuis naar Gent")
            .unwrap();
        let mock = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::Regex("we verhuizen naar Gent".into()),
                mockito::Matcher::Regex("Besluiten".into()),
            ]))
            .with_body(r#"{"choices":[{"message":{"content":"Gent."}}]}"#)
            .create();
        let other = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex("ananas".into()))
            .expect(0)
            .create();
        let answer = engine
            .ask_meetings(
                "Waar verhuizen we naartoe?".into(),
                vec![],
                Some(id),
                keys(),
            )
            .unwrap();
        mock.assert();
        other.assert();
        assert!(answer.citations.is_empty());
    }

    #[test]
    fn command_output_is_unwrapped_and_recorded() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "maak hier een mail van");
        let mock = server
            .mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Regex(
                "selection-[0-9a-f]{8}>.{2}ik ben later".into(),
            ))
            .with_body(r#"{"choices":[{"message":{"content":"\"Beste Jan, ik ben later.\""}}]}"#)
            .create();
        let result = engine
            .command(
                request(speech(), Some("com.apple.mail")),
                Some("ik ben later".into()),
            )
            .unwrap();
        mock.assert();
        assert_eq!(result.text, "Beste Jan, ik ben later.");
        assert_eq!(result.raw, "maak hier een mail van");
        let entry = &engine.history(1).unwrap()[0];
        assert_eq!(
            (entry.raw.as_str(), entry.text.as_str()),
            ("maak hier een mail van", "Beste Jan, ik ben later.")
        );
    }

    #[test]
    fn command_without_speech_is_no_speech() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let calls = server
            .mock("POST", mockito::Matcher::Any)
            .expect(0)
            .create();
        let err = engine
            .command(request(vec![0.0; 16000], None), Some("tekst".into()))
            .unwrap_err();
        assert_eq!(err, PennelError::NoSpeech);
        calls.assert();
    }

    #[test]
    fn empty_instruction_or_result_pastes_nothing() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let stt_mock = stt(&mut server, "  ");
        let llm_mock = llm(&mut server, "x").expect(0);
        assert_eq!(
            engine
                .command(request(speech(), None), Some("tekst".into()))
                .unwrap_err(),
            PennelError::NoSpeech
        );
        stt_mock.remove();
        llm_mock.assert();
        llm_mock.remove();
        stt(&mut server, "maak korter");
        llm(&mut server, "\"\"");
        assert!(matches!(
            engine.command(request(speech(), None), Some("tekst".into())),
            Err(PennelError::InvalidResponse { .. })
        ));
    }

    #[test]
    fn command_rejects_huge_selection() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let huge = "x".repeat(crate::command::MAX_SELECTION_CHARS + 1);
        assert!(matches!(
            engine.command(request(speech(), None), Some(huge)),
            Err(PennelError::InvalidResponse { .. })
        ));
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn diarize_without_model_is_model_not_downloaded() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Sync", 9, "hallo");
        let voice: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.05).sin() * 0.2).collect();
        assert_eq!(
            engine.diarize_meeting(id.clone(), vec![0.0; 16_000]),
            Ok(0),
            "a silent track has nobody to recognise"
        );
        let expected = if cfg!(feature = "speakers") {
            PennelError::ModelNotDownloaded
        } else {
            crate::speakers::unavailable()
        };
        assert_eq!(engine.diarize_meeting(id, voice), Err(expected));
    }

    struct EchoLength;
    impl SystemTranscriber for EchoLength {
        fn transcribe(
            &self,
            samples: Vec<f32>,
            sample_rate: u32,
            language: Option<String>,
            vocabulary: Vec<String>,
        ) -> Result<String, PennelError> {
            assert_eq!(vocabulary, vec!["Pennel Mobile"]);
            Ok(format!(
                " {} samples at {sample_rate} Hz in {} ",
                samples.len(),
                language.unwrap_or_default()
            ))
        }
    }

    #[test]
    fn apple_speech_goes_through_the_system_transcriber() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Fast);
        let mut settings = engine.settings();
        let apple =
            crate::provider::new_account(ProviderKind::AppleSpeech, settings.providers.clone());
        settings.dictation_stt = crate::settings::ModelChoice {
            provider_id: apple.id.clone(),
            model: "dictation".into(),
        };
        settings.providers.push(apple);
        settings.language = Some("nl".into());
        settings.dictionary = vec!["  Pennel   Mobile ".into(), "pennel mobile".into()];
        engine.update_settings(settings).unwrap();

        assert_eq!(
            engine.dictate(request(speech(), None)).unwrap_err(),
            PennelError::NotFound
        );
        engine.set_system_transcriber(Arc::new(EchoLength));
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "16000 samples at 16000 Hz in nl");
    }

    #[test]
    fn apple_intelligence_cleanup_uses_only_the_registered_system_model() {
        struct System;
        impl crate::SystemLanguageModel for System {
            fn complete(
                &self,
                messages: Vec<crate::LanguageModelMessage>,
                timeout_secs: u64,
            ) -> Result<String, PennelError> {
                assert_eq!(messages[0].role, "system");
                assert!(messages[1].content.contains("euh ik kom morgen"));
                assert!(timeout_secs > 0);
                Ok("Ik kom morgen.".into())
            }
        }
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        stt(&mut server, "euh ik kom morgen");
        let no_http = llm(&mut server, "unwanted HTTP").expect(0);
        let mut settings = engine.settings();
        let mut apple =
            crate::new_account(ProviderKind::AppleIntelligence, settings.providers.clone());
        apple.base_url = server.url();
        settings.llm = ModelChoice {
            provider_id: apple.id.clone(),
            model: "system".into(),
        };
        settings.providers.push(apple);
        engine.update_settings(settings).unwrap();
        let missing = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(missing.text, "euh ik kom morgen");
        assert!(matches!(missing.cleanup, CleanupStatus::Failed { .. }));
        engine.set_system_language_model(Arc::new(System));
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, "Ik kom morgen.");
        assert_eq!(result.cleanup, CleanupStatus::Applied);
        no_http.assert();
    }

    struct SystemText {
        output: String,
        calls: Mutex<Vec<Vec<crate::LanguageModelMessage>>>,
    }

    impl crate::SystemLanguageModel for SystemText {
        fn complete(
            &self,
            messages: Vec<crate::LanguageModelMessage>,
            _: u64,
        ) -> Result<String, PennelError> {
            assert!(
                messages
                    .iter()
                    .map(|message| message.content.len() + 64)
                    .sum::<usize>()
                    <= system_llm::APPLE_INPUT_BYTES
            );
            self.calls.lock().unwrap().push(messages);
            Ok(self.output.clone())
        }
    }

    fn use_apple_intelligence(engine: &PennelEngine, output: &str) -> Arc<SystemText> {
        let mut settings = engine.settings();
        let mut apple =
            crate::new_account(ProviderKind::AppleIntelligence, settings.providers.clone());
        // Even a populated URL must not turn a system provider into a network request.
        apple.base_url = settings.providers[0].base_url.clone();
        settings.llm = ModelChoice {
            provider_id: apple.id.clone(),
            model: "system".into(),
        };
        settings.providers.push(apple);
        engine.update_settings(settings).unwrap();
        let model = Arc::new(SystemText {
            output: output.into(),
            calls: Mutex::new(Vec::new()),
        });
        engine.set_system_language_model(model.clone());
        model
    }

    #[test]
    fn live_question_scope_discloses_large_ascii_excerpts_and_shortened_history() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let rows: Vec<_> = (0..160)
            .map(|index| {
                crate::MeetingUtterance::new(
                    Segment {
                        start_ms: index * 1_000,
                        end_ms: (index + 1) * 1_000,
                        speaker: Speaker::Others,
                        text: format!("Owned passage {index}: {}", "a".repeat(440)),
                        failed: false,
                        voice: None,
                    },
                    Some("system_audio".into()),
                )
            })
            .collect();
        let evidence =
            crate::MeetingEvidence::new_with_utterances("owned-scope", "Scope", "", "", &rows, &[])
                .unwrap();
        assert!(
            evidence
                .sources
                .iter()
                .map(|source| source.text.len())
                .sum::<usize>()
                > crate::meeting_chat::WHOLE_MEETING_CHARS
        );
        let history: Vec<_> = (0..8)
            .map(|index| ChatTurn {
                question: format!("Prior question {index}"),
                answer: "Prior answer. ".repeat(200),
            })
            .collect();
        let prepared = engine
            .prepare_meeting_evidence_question(
                "What was decided?".into(),
                history,
                &evidence,
                keys(),
            )
            .unwrap();
        let selection = prepared.selection();
        assert_eq!(selection.scope, MeetingQuestionScope::SelectedExcerpts);
        assert_eq!(selection.available_sources, 160);
        assert_eq!(selection.included_sources, prepared.sources().len() as u32);
        assert_eq!(
            selection.included_sources,
            crate::meeting_chat::MEETING_PASSAGES
        );
        assert_eq!(selection.available_history_turns, 8);
        assert_eq!(selection.included_history_turns, 6);
        assert!(selection.history_reduced);
        assert_eq!(
            evidence
                .anchors_for_sources(prepared.sources())
                .unwrap()
                .len(),
            prepared.sources().len()
        );
        let clipped = engine
            .prepare_meeting_evidence_question(
                "What was decided?".into(),
                vec![ChatTurn {
                    question: "One earlier turn".into(),
                    answer: "answer ".repeat(500),
                }],
                &evidence,
                keys(),
            )
            .unwrap();
        assert_eq!(clipped.selection().available_history_turns, 1);
        assert_eq!(clipped.selection().included_history_turns, 1);
        assert!(clipped.selection().history_reduced);
    }

    #[test]
    fn live_apple_question_scope_and_guards_match_only_complete_sources_actually_sent() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        use_apple_intelligence(&engine, "Unused");
        let rows: Vec<_> = (0..18)
            .map(|index| {
                crate::MeetingUtterance::new(
                    Segment {
                        start_ms: index * 1_000,
                        end_ms: (index + 1) * 1_000,
                        speaker: Speaker::Others,
                        text: format!("Owned Apple passage {index}: {}", "b".repeat(440)),
                        failed: false,
                        voice: None,
                    },
                    Some("system_audio".into()),
                )
            })
            .collect();
        let evidence = crate::MeetingEvidence::new_with_utterances(
            "owned-apple-scope",
            "Scope",
            "",
            "",
            &rows,
            &[],
        )
        .unwrap();
        let history: Vec<_> = (0..8)
            .map(|index| ChatTurn {
                question: format!("Prior {index}"),
                answer: "Long prior answer. ".repeat(200),
            })
            .collect();
        let question = "  Wat beslissen we nu? 😀  ";
        let prepared = engine
            .prepare_meeting_evidence_question(question.into(), history, &evidence, keys())
            .unwrap();
        let selection = prepared.selection();
        assert_eq!(selection.scope, MeetingQuestionScope::SelectedExcerpts);
        assert_eq!(selection.available_sources, 18);
        assert!(selection.included_sources > 0 && selection.included_sources < 18);
        assert_eq!(
            selection.included_sources as usize,
            prepared.sources().len()
        );
        assert_eq!(selection.available_history_turns, 8);
        assert_eq!(
            selection.included_history_turns as usize,
            (prepared.messages.len() - 2) / 2
        );
        assert!(selection.history_reduced);
        assert!(
            prepared
                .messages
                .last()
                .unwrap()
                .content
                .ends_with(question)
        );
        assert!(
            crate::system_llm::input_bytes(&prepared.messages)
                <= crate::system_llm::APPLE_INPUT_BYTES
        );
        let guarded: Vec<_> = prepared
            .source_guard
            .as_ref()
            .unwrap()
            .anchors
            .iter()
            .map(|anchor| anchor.passage.clone())
            .collect();
        assert_eq!(guarded, prepared.sources());
    }

    #[test]
    fn oversized_current_live_question_is_refused_before_source_or_history_selection() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let evidence = crate::MeetingEvidence::new("owned-empty", "Empty", "", "", &[]);
        let model = use_apple_intelligence(&engine, "Must not run");
        let error = engine
            .prepare_meeting_evidence_question("q".repeat(7_000), vec![], &evidence, keys())
            .err()
            .unwrap();
        assert!(error.to_string().contains("not truncated or sent"));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn live_evidence_question_is_cancelled_when_processing_configuration_changes() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Owned live question", 9, "An exact source.");
        let uid = engine.meeting_uid(id.clone()).unwrap();
        let info = engine.meeting_info(id.clone()).unwrap();
        let rows = engine.meeting_utterance_snapshot(id).unwrap();
        let evidence = crate::MeetingEvidence::new_with_utterances(
            &uid,
            &info.title,
            &info.start_iso,
            "",
            &rows.utterances,
            &[],
        )
        .unwrap();
        let first = use_apple_intelligence(&engine, "Old configured answer [1]");
        let prepared = engine
            .prepare_meeting_evidence_question("Question".into(), vec![], &evidence, keys())
            .unwrap();
        let second =
            use_apple_intelligence(&engine, "New destination must not receive old sources");
        assert!(matches!(
            prepared.complete(crate::InferenceCancellation::new(), ()),
            Err(PennelError::Cancelled)
        ));
        assert!(first.calls.lock().unwrap().is_empty());
        assert!(second.calls.lock().unwrap().is_empty());
    }

    fn owned_question_evidence(engine: &PennelEngine, id: &str) -> crate::MeetingEvidence {
        let uid = engine.meeting_uid(id.into()).unwrap();
        let info = engine.meeting_info(id.into()).unwrap();
        let rows = engine.meeting_utterance_snapshot(id.into()).unwrap();
        crate::MeetingEvidence::new_with_utterances(
            &uid,
            &info.title,
            &info.start_iso,
            &engine.meeting_notes_checked(id).unwrap(),
            &rows.utterances,
            &[],
        )
        .unwrap()
    }

    #[test]
    fn a_live_question_keeps_frozen_sources_after_append_and_rename() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Owned source", 9, "Earlier owned evidence");
        let model = use_apple_intelligence(&engine, "Earlier evidence [1]");
        let evidence = owned_question_evidence(&engine, &id);
        let prepared = engine
            .prepare_meeting_evidence_question("What was said?".into(), vec![], &evidence, keys())
            .unwrap();
        let store = engine.meeting_store();
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        let mut rows = snapshot.utterances;
        rows.push(crate::MeetingUtterance::new(
            Segment {
                start_ms: 10_000,
                end_ms: 11_000,
                speaker: Speaker::Others,
                text: "Later owned speech must remain outside the answer.".into(),
                failed: false,
                voice: None,
            },
            Some("system_audio".into()),
        ));
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &rows)
            .unwrap();
        store.rename(&id, "Renamed owned source").unwrap();
        let answer = prepared
            .complete(crate::InferenceCancellation::new(), ())
            .unwrap();
        assert_eq!(answer.citations[0].passage.text, evidence.sources[0].text);
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(
            !calls[0]
                .iter()
                .any(|message| message.content.contains("Later owned speech"))
        );
    }

    #[test]
    fn changed_transcript_sources_cannot_dispatch_a_queued_live_question() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Owned source", 9, "Original source");
        let model = use_apple_intelligence(&engine, "Must not be sent");
        let evidence = owned_question_evidence(&engine, &id);
        let prepared = engine
            .prepare_meeting_evidence_question("Question".into(), vec![], &evidence, keys())
            .unwrap();
        engine
            .meeting_store()
            .write_transcript(
                &id,
                &[Segment {
                    start_ms: 0,
                    end_ms: 3_000,
                    speaker: Speaker::Others,
                    text: "Replacement source.".into(),
                    failed: false,
                    voice: None,
                }],
            )
            .unwrap();
        assert!(matches!(
            prepared.complete(crate::InferenceCancellation::new(), ()),
            Err(PennelError::Cancelled)
        ));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn live_question_deadline_and_credential_epoch_begin_at_admission() {
        use std::time::{Duration, Instant};
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Owned source", 9, "Owned evidence");
        let model = use_apple_intelligence(&engine, "Must not run after expiry");
        let evidence = owned_question_evidence(&engine, &id);
        let admission = crate::ForegroundQuestionAdmission::for_meeting(
            crate::InferenceCancellation::new(),
            Instant::now() + Duration::from_millis(30),
            engine.meeting_uid(id.clone()).unwrap(),
        );
        let prepared = engine.prepare_meeting_evidence_question_admitted(
            "Question".into(),
            vec![],
            &evidence,
            keys(),
            admission,
        );
        std::thread::sleep(Duration::from_millis(40));
        if let Ok(prepared) = prepared {
            assert!(
                prepared
                    .complete(crate::InferenceCancellation::new(), ())
                    .is_err()
            );
        }
        let epoch = engine.inference_epoch();
        let admission = crate::ForegroundQuestionAdmission::begin_meeting(
            crate::InferenceCancellation::new(),
            engine.meeting_uid(id).unwrap(),
        )
        .with_inference_epoch(epoch);
        let mutation = engine.begin_inference_mutation();
        mutation.release();
        assert!(matches!(
            engine.prepare_meeting_evidence_question_admitted(
                "Question".into(),
                vec![],
                &evidence,
                keys(),
                admission
            ),
            Err(PennelError::Cancelled)
        ));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn oversized_live_question_context_is_refused_without_truncation_or_dispatch() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let id = add_meeting(&engine, "Owned source", 9, "Owned evidence");
        use_apple_intelligence(&engine, "Unused");
        let mut settings = engine.settings();
        settings.llm = ModelChoice {
            provider_id: settings.providers[0].id.clone(),
            model: "fixture".into(),
        };
        engine.update_settings(settings).unwrap();
        let evidence = owned_question_evidence(&engine, &id);
        let error = engine
            .prepare_meeting_evidence_question("q".repeat(70_000), vec![], &evidence, keys())
            .err()
            .unwrap();
        assert!(error.to_string().contains("not truncated or sent"));
    }

    #[test]
    fn live_question_mutation_cancels_logically_while_its_real_worker_keeps_ownership() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        use std::time::Duration;
        let _physical_gate = crate::meeting_scheduler::physical_test_gate();
        struct Blocking {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl crate::SystemLanguageModel for Blocking {
            fn complete(
                &self,
                _: Vec<crate::LanguageModelMessage>,
                _: u64,
            ) -> Result<String, PennelError> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok("Old source answer [1]".into())
            }
        }
        struct Ownership(Arc<AtomicBool>);
        impl Drop for Ownership {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        for replace_source in [false, true] {
            let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
            let engine = meeting_engine(&dir, &server);
            let id = add_meeting(&engine, "Owned source", 9, "Original owned source");
            use_apple_intelligence(&engine, "Unused");
            let (started, seen) = mpsc::channel();
            let (release, receive) = mpsc::channel();
            engine.set_system_language_model(Arc::new(Blocking {
                started,
                release: Mutex::new(receive),
            }));
            let prepared = engine
                .prepare_meeting_evidence_question(
                    "Question".into(),
                    vec![],
                    &owned_question_evidence(&engine, &id),
                    keys(),
                )
                .unwrap();
            let dropped = Arc::new(AtomicBool::new(false));
            let ownership = Ownership(dropped.clone());
            let (finished, result) = mpsc::channel();
            std::thread::spawn(move || {
                finished
                    .send(prepared.complete(crate::InferenceCancellation::new(), ownership))
                    .unwrap();
            });
            seen.recv_timeout(Duration::from_secs(5)).unwrap();
            if replace_source {
                engine
                    .meeting_store()
                    .write_transcript(
                        &id,
                        &[Segment {
                            start_ms: 0,
                            end_ms: 2_000,
                            speaker: Speaker::Others,
                            text: "Different source.".into(),
                            failed: false,
                            voice: None,
                        }],
                    )
                    .unwrap();
                release.send(()).unwrap();
                assert!(matches!(
                    result.recv_timeout(Duration::from_secs(5)).unwrap(),
                    Err(PennelError::Cancelled)
                ));
            } else {
                use_apple_intelligence(&engine, "A changed destination");
                assert!(matches!(
                    result.recv_timeout(Duration::from_secs(5)).unwrap(),
                    Err(PennelError::Cancelled)
                ));
                assert!(!dropped.load(Ordering::Acquire));
                release.send(()).unwrap();
            }
            let end = std::time::Instant::now() + Duration::from_secs(5);
            while !dropped.load(Ordering::Acquire) {
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    #[test]
    fn prepared_meeting_question_freezes_sources_and_provider() {
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server, DictationMode::Clean);
        let first = use_apple_intelligence(&engine, "Frozen answer [1]");
        let mut sources = vec![Passage {
            meeting_id: "uid-one".into(),
            title: "M".into(),
            start_iso: "2026-11-02".into(),
            kind: crate::PassageKind::Transcript,
            start_ms: Some(1_000),
            text: "Earlier evidence".into(),
            snippet: String::new(),
            meeting_uid: None,
            utterance_ids: Vec::new(),
            utterance_fingerprints: Vec::new(),
        }];
        let request = engine
            .prepare_meeting_question("What happened?".into(), vec![], sources.clone(), keys())
            .unwrap();
        sources[0].text = "Later evidence".into();
        let second = use_apple_intelligence(&engine, "Wrong provider");
        let answer = request
            .complete(crate::InferenceCancellation::new(), ())
            .unwrap();
        assert_eq!(answer.text, "Frozen answer [1]");
        assert_eq!(answer.citations[0].passage.text, "Earlier evidence");
        assert_eq!(first.calls.lock().unwrap().len(), 1);
        assert!(second.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_prepared_question_never_dispatches_the_provider() {
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server, DictationMode::Clean);
        let model = use_apple_intelligence(&engine, "Unexpected");
        let request = engine
            .prepare_meeting_question("Question".into(), vec![], vec![], keys())
            .unwrap();
        let cancellation = crate::InferenceCancellation::new();
        cancellation.cancel();
        assert!(matches!(
            request.complete(cancellation, ()),
            Err(PennelError::Cancelled)
        ));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_questions_keep_physical_capacity_until_the_provider_exits() {
        let _physical_gate = crate::meeting_scheduler::physical_test_gate();
        use std::sync::mpsc;
        use std::time::Duration;
        struct Blocking {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl crate::SystemLanguageModel for Blocking {
            fn complete(
                &self,
                _: Vec<crate::LanguageModelMessage>,
                _: u64,
            ) -> Result<String, PennelError> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok("Finished".into())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let server = mockito::Server::new();
        let engine = engine(&dir, &server, DictationMode::Clean);
        use_apple_intelligence(&engine, "unused");
        let prepare = || {
            let (started, seen) = mpsc::channel();
            let (release, released) = mpsc::channel();
            engine.set_system_language_model(Arc::new(Blocking {
                started,
                release: Mutex::new(released),
            }));
            let request = engine
                .prepare_meeting_question("Question".into(), vec![], vec![], keys())
                .unwrap();
            let cancellation = crate::InferenceCancellation::new();
            let cancelled = cancellation.clone();
            let (finished, result) = mpsc::channel();
            std::thread::spawn(move || {
                finished.send(request.complete(cancelled, ())).unwrap();
            });
            (seen, release, cancellation, result)
        };
        let (first_started, first_release, first_cancel, first_result) = prepare();
        first_started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (second_started, second_release, second_cancel, second_result) = prepare();
        second_started.recv_timeout(Duration::from_secs(5)).unwrap();
        first_cancel.cancel();
        second_cancel.cancel();
        assert!(matches!(
            first_result.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(PennelError::Cancelled)
        ));
        assert!(matches!(
            second_result.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(PennelError::Cancelled)
        ));
        let (third_started, third_release, _, third_result) = prepare();
        assert!(matches!(
            third_started.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        first_release.send(()).unwrap();
        third_started.recv_timeout(Duration::from_secs(5)).unwrap();
        second_release.send(()).unwrap();
        third_release.send(()).unwrap();
        assert!(
            third_result
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_ok()
        );
    }

    #[test]
    fn ordinary_dictation_cleanup_uses_speech_priority_while_optional_work_is_busy() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        const ISOLATED: &str = "PENNEL_TEST_ISOLATED_DICTATION_PRIORITY";
        if std::env::var_os(ISOLATED).is_none() {
            // Other speech fixtures legitimately preempt optional work before its closure starts.
            let test_name = std::thread::current().name().unwrap().to_owned();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test_name, "--test-threads=1", "--nocapture"])
                .env(ISOLATED, "1")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(50);
            let timed_out = loop {
                if child.try_wait().unwrap().is_some() {
                    break false;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    break true;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let output = child.wait_with_output().unwrap();
            assert!(
                !timed_out && output.status.success(),
                "isolated priority fixture failed (timed out: {timed_out}):\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        let _physical_gate = crate::meeting_scheduler::physical_test_gate();
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let model = use_apple_intelligence(&engine, "Dit is een zorgvuldig geschreven test.");
        let speech_request = stt(&mut server, "dit is een zorgvuldig geschreven test");
        let (started, seen) = mpsc::channel();
        let (release, receive) = mpsc::channel();
        let optional = crate::InferenceCancellation::new();
        let optional_worker = optional.clone();
        let handle = std::thread::spawn(move || {
            crate::inference::run_cancelled_before(
                optional_worker,
                std::time::Instant::now() + Duration::from_secs(20),
                move |_| {
                    started.send(()).unwrap();
                    receive.recv().unwrap();
                    Ok(())
                },
            )
        });
        if let Err(error) = seen.recv_timeout(Duration::from_secs(10)) {
            panic!(
                "optional worker did not start: {error:?}; cancelled: {}; result: {:?}",
                optional.is_cancelled(),
                handle.join().unwrap()
            );
        }
        engine.begin_meeting_preview_priority();
        let result = engine.dictate(request(speech(), None));
        engine.end_meeting_preview_priority();
        release.send(()).unwrap();
        let _ = handle.join();
        let result = result.unwrap();
        assert_eq!(result.text, "Dit is een zorgvuldig geschreven test.");
        assert_eq!(result.cleanup, CleanupStatus::Applied);
        assert_eq!(model.calls.lock().unwrap().len(), 1);
        assert!(optional.is_cancelled());
        speech_request.assert();
    }

    #[test]
    fn apple_intelligence_command_uses_the_system_model_without_http() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let model = use_apple_intelligence(&engine, "Beste Jan, ik kom morgen.");
        let no_http = llm(&mut server, "unwanted HTTP").expect(0);
        stt(&mut server, "maak hier een mail van");
        let result = engine
            .command(request(speech(), None), Some("ik kom morgen".into()))
            .unwrap();
        assert_eq!(result.text, model.output);
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0][1].content.contains("ik kom morgen"));
        no_http.assert();
    }

    #[test]
    fn apple_intelligence_summary_and_cited_chat_use_the_system_model() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let model = use_apple_intelligence(&engine, "Gent [1].");
        let no_http = llm(&mut server, "unwanted HTTP").expect(0);
        let id = add_meeting(&engine, "Sync", 9, "we verhuizen naar Gent");
        let body = engine
            .summarize_meeting(
                id.clone(),
                "algemeen".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        assert_eq!(body, "Gent [1].");
        assert!(engine.meeting_info(id.clone()).unwrap().has_summary);
        let answer = engine
            .ask_meetings(
                "Waar verhuizen we?".into(),
                vec![],
                Some(id),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        assert_eq!(answer.text, "Gent [1].");
        assert_eq!(answer.citations.len(), 1);
        assert!(answer.citations[0].passage.text.contains("Gent"));
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        no_http.assert();
    }

    #[test]
    fn apple_intelligence_oversized_cleanup_preserves_the_entire_raw_dictation() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server, DictationMode::Clean);
        let model = use_apple_intelligence(&engine, "must not be called");
        let raw = "veel tekst ".repeat(1_000);
        stt(&mut server, &raw);
        let result = engine.dictate(request(speech(), None)).unwrap();
        assert_eq!(result.text, raw.trim());
        assert!(
            matches!(result.cleanup, CleanupStatus::Failed { message } if message.contains("too long for Apple Intelligence"))
        );
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn apple_intelligence_errors_do_not_write_partial_summaries_or_reach_http() {
        struct Unavailable;
        impl crate::SystemLanguageModel for Unavailable {
            fn complete(
                &self,
                _: Vec<crate::LanguageModelMessage>,
                _: u64,
            ) -> Result<String, PennelError> {
                Err(PennelError::InvalidResponse {
                    message: "Apple Intelligence wordt nog gedownload.".into(),
                })
            }
        }
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        use_apple_intelligence(&engine, "unused");
        engine.set_system_language_model(Arc::new(Unavailable));
        let no_http = llm(&mut server, "unwanted HTTP").expect(0);
        let id = add_meeting(&engine, "Sync", 9, "het budget is goedgekeurd");
        let store = engine.meeting_store();
        store
            .write_meeting(&id, 30, "algemeen", "Bestaande samenvatting")
            .unwrap();
        let original = store.read_meeting(&id).unwrap();
        let error = engine
            .summarize_meeting(
                id.clone(),
                "algemeen".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("nog gedownload"));
        assert_eq!(store.read_meeting(&id).unwrap(), original);
        no_http.assert();
    }

    #[test]
    fn apple_intelligence_long_meetings_use_smaller_maps_and_retrieved_chat_sources() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = meeting_engine(&dir, &server);
        let model = use_apple_intelligence(&engine, &"Compacte samenvatting. ".repeat(50));
        let no_http = llm(&mut server, "unwanted HTTP").expect(0);
        let id = add_meeting(&engine, "Lang", 9, "placeholder");
        let store = engine.meeting_store();
        let segments = (0..100)
            .map(|n| crate::Segment {
                start_ms: n * 1_000,
                end_ms: n * 1_000 + 500,
                speaker: crate::Speaker::Others,
                text: if n == 0 {
                    "het sleutelbudget is twintig euro".into()
                } else {
                    "andere bespreking ".repeat(50)
                },
                failed: false,
                voice: None,
            })
            .collect::<Vec<_>>();
        store.write_transcript(&id, &segments).unwrap();
        let keys = ApiKeys {
            by_provider: HashMap::new(),
        };
        engine
            .summarize_meeting(id.clone(), "algemeen".into(), keys.clone())
            .unwrap();
        let summary_calls = model.calls.lock().unwrap().len();
        assert!(
            summary_calls > 3,
            "the Apple budget must force multiple map and reduction calls"
        );
        engine
            .ask_meetings("Wat is het sleutelbudget?".into(), vec![], Some(id), keys)
            .unwrap();
        let calls = model.calls.lock().unwrap();
        let question = calls.last().unwrap();
        assert!(
            question[1]
                .content
                .contains("sleutelbudget is twintig euro")
        );
        assert!(question[1].content.contains("[1]"));
        assert_eq!(calls.len(), summary_calls + 1);
        no_http.assert();
    }

    #[test]
    fn apple_intelligence_model_registration_is_isolated_per_engine() {
        let (a, b, server) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            mockito::Server::new(),
        );
        let first = meeting_engine(&a, &server);
        let second = meeting_engine(&b, &server);
        use_apple_intelligence(&first, "eerste");
        use_apple_intelligence(&second, "tweede");
        let id_a = add_meeting(&first, "Eerste", 9, "test");
        let id_b = add_meeting(&second, "Tweede", 9, "test");
        assert_eq!(
            first
                .summarize_meeting(id_a, "algemeen".into(), keys())
                .unwrap(),
            "eerste"
        );
        assert_eq!(
            second
                .summarize_meeting(id_b, "algemeen".into(), keys())
                .unwrap(),
            "tweede"
        );
    }
}
