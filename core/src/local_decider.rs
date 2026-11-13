use std::collections::HashSet;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{ModelEntry, ModelKind};
use crate::{
    DecisionQuestionKind, DecisionRequest, DecisionResult, DownloadListener, InferenceCancellation,
    InferenceConfig, InferenceProtocol, MeetingConfiguredInference, MeetingIdentityEvidence,
    MeetingQualificationRegistry, ModelState, PennelEngine, PennelError, ProviderKind,
};

mod protocol;
pub use protocol::ManagedLocalDeciderProtocol;
mod native;
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
mod scopes;
pub use native::{BundledLocalDeciderRuntime, managed_local_decider_worker_main};

pub(crate) const MODEL_ID: &str = "pennel-decider-flash-v1.2-q8_0";
pub(crate) const MODEL_SHA256: &str =
    "ffb27ade8976765b0e6943a0b7f5f3fd0f7042d45ad90256a76d608bcd3942c2";
pub(crate) const MODEL_BYTES: u64 = 5_157_837_472;
pub(crate) const MODEL_FINGERPRINT: &str =
    "sha256:ffb27ade8976765b0e6943a0b7f5f3fd0f7042d45ad90256a76d608bcd3942c2";
pub(crate) const ENDPOINT: &str = "pennel-local://decider/decisions";
pub(crate) const PROVIDER: &str = "Pennel local Decider";
const MAX_INPUT_BYTES: usize = 65_536;
const MAX_QUESTIONS: usize = 64;

pub(crate) fn model_entry() -> ModelEntry {
    ModelEntry {
        id: MODEL_ID.into(),
        name: "Decider Flash".into(),
        description: "Local source-linked meeting Review".into(),
        size_bytes: MODEL_BYTES,
        sha256: MODEL_SHA256.into(),
        url: "https://huggingface.co/perplexity-ai/pplx-decider-flash-gguf-v1.2/resolve/2514d5f0b7fbbe1ee1bfec9ffb5801a60289b4cc/decider-flash-v1.2-q8_0.gguf".into(),
        scores: None,
        legacy: false,
        kind: ModelKind::Decision,
    }
}

fn invalid(message: &str) -> PennelError {
    PennelError::InvalidResponse {
        message: message.into(),
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn supported_device() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStamp {
    bytes: u64,
    modified_ns: u128,
    created_ns: Option<u128>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_secs: i64,
    #[cfg(unix)]
    changed_ns: i64,
}

impl FileStamp {
    fn from_metadata(metadata: &Metadata) -> Result<Self, PennelError> {
        if !metadata.is_file() {
            return Err(invalid(
                "The local Decider artifact must be a regular file.",
            ));
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            bytes: metadata.len(),
            modified_ns: metadata
                .modified()
                .map_err(PennelError::storage)?
                .duration_since(UNIX_EPOCH)
                .map_err(PennelError::storage)?
                .as_nanos(),
            created_ns: metadata
                .created()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|time| time.as_nanos()),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed_secs: metadata.ctime(),
            #[cfg(unix)]
            changed_ns: metadata.ctime_nsec(),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReceipt {
    schema_version: u32,
    artifact_sha256: String,
    file: FileStamp,
}

/// A model pin established by reading the same open file supplied to the native loader.
/// A catalog entry or download receipt cannot construct this serving input.
pub struct VerifiedLocalDeciderArtifact {
    file: File,
    path: Option<PathBuf>,
    stamp: FileStamp,
    sha256: String,
}

impl VerifiedLocalDeciderArtifact {
    fn open(
        path: &Path,
        bytes: u64,
        sha256: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, PennelError> {
        let path_metadata = std::fs::symlink_metadata(path).map_err(PennelError::storage)?;
        if path_metadata.file_type().is_symlink() {
            return Err(invalid(
                "The local Decider artifact must not be a symbolic link.",
            ));
        }
        let file = File::open(path).map_err(PennelError::storage)?;
        let stamp = FileStamp::from_metadata(&file.metadata().map_err(PennelError::storage)?)?;
        if stamp != FileStamp::from_metadata(&path_metadata)? || stamp.bytes != bytes {
            return Err(invalid(
                "The local Decider artifact does not match its expected size.",
            ));
        }
        Self::from_open_file(file, Some(path.into()), bytes, sha256, cancelled)
    }

    fn from_open_file(
        mut file: File,
        path: Option<PathBuf>,
        bytes: u64,
        sha256: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, PennelError> {
        let stamp = FileStamp::from_metadata(&file.metadata().map_err(PennelError::storage)?)?;
        if stamp.bytes != bytes {
            return Err(invalid(
                "The local Decider artifact does not match its expected size.",
            ));
        }
        file.seek(SeekFrom::Start(0))
            .map_err(PennelError::storage)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            if cancelled() {
                return Err(PennelError::Cancelled);
            }
            let length = file.read(&mut buffer).map_err(PennelError::storage)?;
            if length == 0 {
                break;
            }
            hasher.update(&buffer[..length]);
        }
        let actual = crate::models::hex(&hasher.finalize());
        if actual != sha256 {
            return Err(invalid(
                "The local Decider artifact checksum does not match.",
            ));
        }
        file.seek(SeekFrom::Start(0))
            .map_err(PennelError::storage)?;
        let artifact = Self {
            file,
            path,
            stamp,
            sha256: actual,
        };
        artifact.verify_unchanged()?;
        Ok(artifact)
    }

    /// Native runtimes must consume this handle, not reopen a configured pathname.
    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn model_fingerprint(&self) -> String {
        format!("sha256:{}", self.sha256)
    }

    pub fn verify_unchanged(&self) -> Result<(), PennelError> {
        let changed_path = self.path.as_ref().is_some_and(|path| {
            !std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                !metadata.file_type().is_symlink()
                    && FileStamp::from_metadata(&metadata).is_ok_and(|stamp| stamp == self.stamp)
            })
        });
        if changed_path
            || FileStamp::from_metadata(&self.file.metadata().map_err(PennelError::storage)?)?
                != self.stamp
        {
            return Err(invalid(
                "The local Decider artifact changed while it was in use.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ManagedLocalDeciderRuntimeStatus {
    pub available: bool,
    /// Actual verified native/runtime/protocol code fingerprint; never a configured revision.
    pub revision: Option<String>,
    pub reason_code: Option<String>,
}

/// Rust host boundary for a Pennel-owned, signed native runtime.
/// `status` must be metadata-only. It must not load a model or execute inference.
pub trait ManagedLocalDeciderRuntime: Send + Sync {
    fn status(&self) -> ManagedLocalDeciderRuntimeStatus;
    fn decide(
        &self,
        model: Arc<VerifiedLocalDeciderArtifact>,
        request_id: String,
        request: DecisionRequest,
        cancellation: Arc<InferenceCancellation>,
        deadline: std::time::Instant,
    ) -> Result<DecisionResult, PennelError>;
    /// Stop and reap the owned worker before returning. Never affect unrelated runtimes.
    fn unload(&self);
}

#[derive(Default)]
struct RuntimeState {
    runtime: Option<Arc<dyn ManagedLocalDeciderRuntime>>,
    artifact: Option<Arc<VerifiedLocalDeciderArtifact>>,
    requests: Vec<(u64, std::sync::Weak<InferenceCancellation>)>,
    next_request: u64,
}

struct LocalRequestRegistration<'a> {
    manager: &'a ManagedLocalDecider,
    id: u64,
}
impl Drop for LocalRequestRegistration<'_> {
    fn drop(&mut self) {
        self.manager
            .state
            .lock()
            .unwrap()
            .requests
            .retain(|(id, _)| *id != self.id);
    }
}

pub(crate) struct ManagedLocalDecider {
    receipt: PathBuf,
    state: Mutex<RuntimeState>,
    verifying: AtomicBool,
    verification_gate: Mutex<()>,
    error: Mutex<Option<&'static str>>,
}

impl ManagedLocalDecider {
    pub(crate) fn new(data_dir: &Path) -> Self {
        Self {
            receipt: data_dir
                .join("models")
                .join(format!("{MODEL_ID}.verified.json")),
            state: Mutex::default(),
            verifying: AtomicBool::new(false),
            verification_gate: Mutex::new(()),
            error: Mutex::new(None),
        }
    }

    fn receipt_matches(&self, path: &Path, bytes: u64, sha256: &str) -> bool {
        let result = (|| {
            let receipt = File::open(&self.receipt).ok()?;
            if receipt.metadata().ok()?.len() > 8192 {
                return None;
            }
            let receipt: ArtifactReceipt = serde_json::from_reader(receipt).ok()?;
            let metadata = std::fs::symlink_metadata(path).ok()?;
            (receipt.schema_version == 1
                && receipt.artifact_sha256 == sha256
                && receipt.file.bytes == bytes
                && !metadata.file_type().is_symlink()
                && FileStamp::from_metadata(&metadata).ok()? == receipt.file)
                .then_some(())
        })();
        result.is_some()
    }

    fn write_receipt(&self, artifact: &VerifiedLocalDeciderArtifact) -> Result<(), PennelError> {
        artifact.verify_unchanged()?;
        let parent = self
            .receipt
            .parent()
            .expect("owned model receipt has a parent");
        std::fs::create_dir_all(parent).map_err(PennelError::storage)?;
        let temporary = parent.join(format!(".decider-{}.tmp", uuid::Uuid::now_v7()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options.open(&temporary).map_err(PennelError::storage)?;
            serde_json::to_writer(
                &mut file,
                &ArtifactReceipt {
                    schema_version: 1,
                    artifact_sha256: artifact.sha256.clone(),
                    file: artifact.stamp.clone(),
                },
            )
            .map_err(PennelError::storage)?;
            file.write_all(b"\n").map_err(PennelError::storage)?;
            file.sync_all().map_err(PennelError::storage)?;
            artifact.verify_unchanged()?;
            std::fs::rename(&temporary, &self.receipt).map_err(PennelError::storage)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    pub(crate) fn verify_acquired_artifact(
        &self,
        path: &Path,
        entry: &ModelEntry,
        cancel: &AtomicBool,
    ) -> Result<(), PennelError> {
        let _verification = self.verification_gate.lock().unwrap();
        if cancel.load(Ordering::Acquire) {
            return Err(PennelError::Cancelled);
        }
        if self.receipt_matches(path, entry.size_bytes, &entry.sha256) {
            return Ok(());
        }
        self.verifying.store(true, Ordering::Release);
        let result =
            VerifiedLocalDeciderArtifact::open(path, entry.size_bytes, &entry.sha256, || {
                cancel.load(Ordering::Acquire)
            })
            .and_then(|artifact| self.write_receipt(&artifact));
        self.verifying.store(false, Ordering::Release);
        *self.error.lock().unwrap() = match result.as_ref().err() {
            Some(PennelError::Cancelled) | None => None,
            Some(_) => Some("artifact_verification_failed"),
        };
        result
    }

    fn invalidate(&self) {
        let runtime = {
            let mut state = self.state.lock().unwrap();
            for cancellation in state
                .requests
                .drain(..)
                .filter_map(|(_, weak)| weak.upgrade())
            {
                cancellation.cancel();
            }
            state.artifact = None;
            state.runtime.clone()
        };
        if let Some(runtime) = runtime {
            runtime.unload();
        }
    }

    fn runtime(&self) -> Option<Arc<dyn ManagedLocalDeciderRuntime>> {
        self.state.lock().unwrap().runtime.clone()
    }
}

#[derive(Serialize)]
struct ManagedLocalDeciderStatus {
    model_id: &'static str,
    display_name: &'static str,
    available: bool,
    qualified: bool,
    state: &'static str,
    reason_code: String,
    download_bytes: u64,
    downloaded_bytes: u64,
    runtime_available: bool,
    configuration: Option<InferenceConfig>,
}

fn safe_reason(reason: Option<String>) -> String {
    reason
        .filter(|reason| {
            !reason.is_empty()
                && reason.len() <= 64
                && reason
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        })
        .unwrap_or_else(|| "runtime_unavailable".into())
}

fn configuration(provider_id: String) -> InferenceConfig {
    InferenceConfig {
        provider_id,
        protocol: InferenceProtocol::LocalDecider,
        model: MODEL_ID.into(),
        expected_model: Some(MODEL_FINGERPRINT.into()),
        upstream: None,
        max_input_bytes: MAX_INPUT_BYTES as u32,
        timeout_secs: 90,
    }
}

impl PennelEngine {
    /// Register a trusted bundled runtime. This Rust-only boundary is not an arbitrary-path UI.
    pub fn set_managed_local_decider_runtime(&self, runtime: Arc<dyn ManagedLocalDeciderRuntime>) {
        let manager = &self.models.managed_decider;
        manager.invalidate();
        manager.state.lock().unwrap().runtime = Some(runtime);
    }

    /// Technical readiness only. A meeting still requires its separate qualification.
    /// Actual weight integrity is re-established from the consumed FD on first load.
    pub(crate) fn managed_local_decider_inference_readiness(&self) -> Result<(), PennelError> {
        if !supported_device() {
            return Err(PennelError::local_models_unavailable());
        }
        let runtime = self
            .models
            .managed_decider
            .runtime()
            .ok_or_else(|| invalid("The local Decider runtime is unavailable."))?;
        let status = runtime.status();
        if !status.available || !status.revision.as_deref().is_some_and(valid_hash) {
            return Err(invalid("The local Decider runtime is unavailable."));
        }
        let path = self
            .models
            .path(MODEL_ID)
            .ok_or(PennelError::ModelNotDownloaded)?;
        let metadata = std::fs::symlink_metadata(path).map_err(PennelError::storage)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != MODEL_BYTES
        {
            return Err(invalid(
                "The local Decider artifact must be verified again.",
            ));
        }
        Ok(())
    }

    pub fn managed_local_decider_status_json(&self) -> Result<String, PennelError> {
        let manager = &self.models.managed_decider;
        let model_state = self.model_state(MODEL_ID.into());
        let downloaded_bytes = match model_state {
            ModelState::NotDownloaded => 0,
            ModelState::Partial { bytes } => bytes,
            ModelState::Downloaded => self
                .models
                .path(MODEL_ID)
                .and_then(|path| std::fs::metadata(path).ok())
                .map_or(0, |metadata| metadata.len()),
        };
        let runtime = manager.runtime().map(|runtime| runtime.status());
        let revision = runtime
            .as_ref()
            .and_then(|runtime| runtime.revision.as_ref());
        let runtime_available = supported_device()
            && runtime.as_ref().is_some_and(|runtime| runtime.available)
            && revision.is_some_and(|revision| valid_hash(revision));
        let local_account = self
            .settings()
            .providers
            .into_iter()
            .find(|provider| provider.kind == ProviderKind::Local);
        let config = local_account.map(|account| configuration(account.id));
        let qualified = config.as_ref().is_some_and(|config| {
            let request = crate::meeting_insight_engine::qualification_request(
                MeetingConfiguredInference {
                    protocol: config.protocol,
                    endpoint: ENDPOINT.into(),
                    model: config.model.clone(),
                    upstream: None,
                    expected_model: config.expected_model.clone(),
                    max_input_bytes: config.max_input_bytes,
                    timeout_secs: config.timeout_secs,
                },
                "nl",
            );
            MeetingQualificationRegistry::bundled()
                .admit(&request)
                .is_ok_and(|admission| {
                    let actual = &admission.record().context.actual;
                    actual.identity_evidence == MeetingIdentityEvidence::ServingRevisionAndArtifact
                        && actual.model == MODEL_FINGERPRINT
                        && actual.artifact_sha256 == MODEL_SHA256
                        && actual.provider == PROVIDER
                        && Some(&actual.revision) == revision
                })
        });
        let installed = self
            .models
            .path(MODEL_ID)
            .is_some_and(|path| manager.receipt_matches(&path, MODEL_BYTES, MODEL_SHA256));
        let error = *manager.error.lock().unwrap();
        let (state, reason_code) = if !supported_device() {
            ("unsupported", "unsupported_device".into())
        } else if manager.verifying.load(Ordering::Acquire) {
            ("verifying", "verifying".into())
        } else if self
            .active_model_downloads()
            .iter()
            .any(|id| id == MODEL_ID)
        {
            ("downloading", "downloading".into())
        } else if let Some(error) = error {
            ("error", error.into())
        } else if !runtime_available {
            (
                "unsupported",
                safe_reason(runtime.and_then(|runtime| runtime.reason_code)),
            )
        } else if !qualified {
            ("unqualified", "unqualified".into())
        } else if !installed {
            ("not_installed", "not_downloaded".into())
        } else {
            ("ready", "available".into())
        };
        let available = state == "ready";
        serde_json::to_string(&ManagedLocalDeciderStatus {
            model_id: MODEL_ID,
            display_name: "Decider Flash",
            available,
            qualified,
            state,
            reason_code,
            download_bytes: MODEL_BYTES,
            downloaded_bytes,
            runtime_available,
            configuration: available.then_some(config).flatten(),
        })
        .map_err(PennelError::storage)
    }

    /// Explicit acquisition only. It reuses the core's cancellable, single-owner download.
    pub fn download_managed_local_decider(
        &self,
        listener: Arc<dyn DownloadListener>,
    ) -> Result<(), PennelError> {
        if !supported_device() {
            return Err(PennelError::local_models_unavailable());
        }
        self.download_model(MODEL_ID.into(), listener)
    }

    pub fn cancel_managed_local_decider_download(&self) {
        self.cancel_download(MODEL_ID.into());
    }

    pub fn remove_managed_local_decider(&self) -> Result<(), PennelError> {
        self.models.managed_decider.invalidate();
        self.delete_model(MODEL_ID.into())?;
        match std::fs::remove_file(&self.models.managed_decider.receipt) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(PennelError::storage(error)),
        }
        *self.models.managed_decider.error.lock().unwrap() = None;
        Ok(())
    }

    /// Internal managed dispatch. No hostname, external API key or provider fallback is used.
    pub fn decide_managed_local_decider(
        &self,
        request: DecisionRequest,
        cancellation: Arc<InferenceCancellation>,
        timeout_secs: u32,
    ) -> Result<DecisionResult, PennelError> {
        validate_request(&request)?;
        if !(1..=90).contains(&timeout_secs) {
            return Err(invalid(
                "The local Decider request has an unsupported timeout.",
            ));
        }
        let deadline = cancellation.effective_deadline(
            std::time::Instant::now() + std::time::Duration::from_secs(u64::from(timeout_secs)),
        );
        let manager = &self.models.managed_decider;
        let runtime = manager
            .runtime()
            .ok_or_else(|| invalid("The local Decider runtime is unavailable."))?;
        let status = runtime.status();
        let revision = status
            .revision
            .filter(|revision| valid_hash(revision))
            .filter(|_| status.available)
            .ok_or_else(|| invalid("The local Decider runtime is unavailable."))?;
        // Model preparation is already part of this request. Removal/replacement
        // must cancel it before a long weight hash or native cold start begins.
        let _registration = {
            let mut state = manager.state.lock().unwrap();
            if !state
                .runtime
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &runtime))
            {
                return Err(PennelError::Cancelled);
            }
            let id = state.next_request;
            state.next_request = state.next_request.wrapping_add(1);
            state.requests.retain(|(_, weak)| weak.strong_count() > 0);
            state.requests.push((id, Arc::downgrade(&cancellation)));
            LocalRequestRegistration { manager, id }
        };
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        let artifact = manager.state.lock().unwrap().artifact.clone();
        let artifact = match artifact {
            Some(artifact) => {
                artifact.verify_unchanged()?;
                artifact
            }
            None => {
                let path = self
                    .models
                    .path(MODEL_ID)
                    .ok_or(PennelError::ModelNotDownloaded)?;
                let artifact = Arc::new(VerifiedLocalDeciderArtifact::open(
                    &path,
                    MODEL_BYTES,
                    MODEL_SHA256,
                    || cancellation.is_cancelled() || std::time::Instant::now() >= deadline,
                )?);
                let mut state = manager.state.lock().unwrap();
                if cancellation.is_cancelled()
                    || !state
                        .runtime
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &runtime))
                {
                    return Err(PennelError::Cancelled);
                }
                state.artifact = Some(artifact.clone());
                artifact
            }
        };
        let request_id = uuid::Uuid::now_v7().to_string();
        if std::time::Instant::now() >= deadline {
            return Err(invalid("The local Decider request timed out."));
        }
        let result = runtime.decide(
            artifact.clone(),
            request_id.clone(),
            request.clone(),
            cancellation.clone(),
            deadline,
        )?;
        artifact.verify_unchanged()?;
        let current_runtime = runtime.status();
        if cancellation.is_cancelled()
            || !manager
                .runtime()
                .is_some_and(|current| Arc::ptr_eq(&current, &runtime))
            || !current_runtime.available
            || current_runtime.revision.as_deref() != Some(&revision)
        {
            return Err(PennelError::Cancelled);
        }
        if std::time::Instant::now() > deadline {
            return Err(invalid("The local Decider request timed out."));
        }
        let identity = &result.identity;
        if identity.protocol != InferenceProtocol::LocalDecider
            || identity.endpoint != ENDPOINT
            || identity.requested_model != MODEL_ID
            || identity.requested_upstream.is_some()
            || identity.actual_model.as_deref() != Some(artifact.model_fingerprint().as_str())
            || identity.actual_revision.as_deref() != Some(&revision)
            || identity.actual_provider.as_deref() != Some(PROVIDER)
            || identity.request_id.as_deref() != Some(&request_id)
        {
            return Err(invalid("The local Decider serving identity changed."));
        }
        validate_answers(&request, &result)?;
        Ok(result)
    }
}

fn validate_request(request: &DecisionRequest) -> Result<(), PennelError> {
    if request.questions.is_empty()
        || request.questions.len() > MAX_QUESTIONS
        || crate::inference::decision_body(&configuration(String::new()), request)?.len()
            > MAX_INPUT_BYTES
    {
        return Err(invalid(
            "The local Decider request exceeds its supported bounds.",
        ));
    }
    let mut ids = HashSet::new();
    for question in &request.questions {
        if question.kind != DecisionQuestionKind::Choice
            || question.id.is_empty()
            || question.id.len() > 128
            || !ids.insert(&question.id)
            || question.options.is_empty()
            || question.options.len() > 255
            || question.options.iter().any(|option| option.id.is_empty())
            || question
                .options
                .iter()
                .map(|option| &option.id)
                .collect::<HashSet<_>>()
                .len()
                != question.options.len()
        {
            return Err(invalid(
                "The local Decider request has unsupported questions.",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_answers(
    request: &DecisionRequest,
    result: &DecisionResult,
) -> Result<(), PennelError> {
    if request.questions.len() != result.answers.len() {
        return Err(invalid(
            "The local Decider returned an incomplete answer set.",
        ));
    }
    let mut seen = HashSet::new();
    for answer in &result.answers {
        let Some(question) = request
            .questions
            .iter()
            .find(|question| question.id == answer.id)
        else {
            return Err(invalid("The local Decider returned an unknown answer."));
        };
        let labels = question
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect::<HashSet<_>>();
        let probabilities = answer
            .probabilities
            .iter()
            .map(|value| value.option_id.as_str())
            .collect::<HashSet<_>>();
        let mass: f64 = answer
            .probabilities
            .iter()
            .map(|value| value.probability)
            .sum();
        if !seen.insert(&answer.id)
            || answer.kind != DecisionQuestionKind::Choice
            || probabilities != labels
            || answer.probabilities.len() != labels.len()
            || answer.probabilities.iter().any(|value| {
                !value.probability.is_finite() || !(0.0..=1.0).contains(&value.probability)
            })
            || !mass.is_finite()
            || (mass - 1.0).abs() > 0.001
            || answer
                .choice
                .as_ref()
                .is_some_and(|value| !labels.contains(value.as_str()))
            || (!answer.abstained && answer.choice.is_none())
            || !answer.spans.is_empty()
            || !answer.not_stated.is_empty()
            || answer.value.is_some()
            || answer
                .confidence
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(invalid(
                "The local Decider returned an invalid answer distribution.",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> String {
        crate::models::hex(&Sha256::digest(bytes))
    }

    #[test]
    fn a_verified_artifact_is_the_same_open_file_and_rejects_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.gguf");
        std::fs::write(&path, b"owned synthetic artifact").unwrap();
        let artifact = VerifiedLocalDeciderArtifact::open(
            &path,
            24,
            &digest(b"owned synthetic artifact"),
            || false,
        )
        .unwrap();
        assert_eq!(artifact.sha256(), digest(b"owned synthetic artifact"));
        assert_eq!(artifact.file().metadata().unwrap().len(), 24);
        artifact.verify_unchanged().unwrap();
        let replacement = dir.path().join("replacement.gguf");
        std::fs::write(&replacement, b"owned synthetic artifact").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert!(artifact.verify_unchanged().is_err());
    }

    #[test]
    fn verification_rejects_bad_checksum_cancellation_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned.gguf");
        std::fs::write(&path, b"owned").unwrap();
        assert!(VerifiedLocalDeciderArtifact::open(&path, 5, &digest(b"other"), || false).is_err());
        assert!(matches!(
            VerifiedLocalDeciderArtifact::open(&path, 5, &digest(b"owned"), || true),
            Err(PennelError::Cancelled)
        ));
        #[cfg(unix)]
        {
            let link = dir.path().join("alias.gguf");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(
                VerifiedLocalDeciderArtifact::open(&link, 5, &digest(b"owned"), || false).is_err()
            );
        }
    }

    #[test]
    fn a_download_receipt_is_not_model_serving_evidence_and_expires_on_changes() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ManagedLocalDecider::new(dir.path());
        let path = dir.path().join("owned.gguf");
        std::fs::write(&path, b"owned").unwrap();
        let artifact =
            VerifiedLocalDeciderArtifact::open(&path, 5, &digest(b"owned"), || false).unwrap();
        assert!(!manager.receipt_matches(&path, 5, &digest(b"owned")));
        manager.write_receipt(&artifact).unwrap();
        assert!(manager.receipt_matches(&path, 5, &digest(b"owned")));
        assert!(manager.runtime().is_none());
        std::fs::write(&path, b"changed").unwrap();
        assert!(!manager.receipt_matches(&path, 5, &digest(b"owned")));
    }

    #[test]
    fn opening_local_status_does_not_acquire_hash_or_load_a_model() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&engine.managed_local_decider_status_json().unwrap()).unwrap();
        assert_eq!(value["available"], false);
        assert_eq!(value["runtime_available"], false);
        assert_eq!(value["qualified"], false);
        assert!(value["configuration"].is_null());
        assert!(engine.models.path(MODEL_ID).is_none());
        assert!(!engine.models.managed_decider.receipt.exists());
        assert!(
            !engine
                .models
                .managed_decider
                .verifying
                .load(Ordering::Relaxed)
        );
    }

    #[test]
    fn decision_asset_never_appears_in_speech_model_choices() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::models::ModelStore::new(dir.path());
        assert!(
            !store
                .local_models(Some("nl"), false)
                .iter()
                .any(|model| model.id == MODEL_ID)
        );
        assert!(!store.downloaded().iter().any(|model| model == MODEL_ID));
        let entry = model_entry();
        assert_eq!(entry.kind, ModelKind::Decision);
        assert_eq!(entry.size_bytes, MODEL_BYTES);
        assert_eq!(entry.sha256, MODEL_SHA256);
        assert!(
            entry
                .url
                .contains("/2514d5f0b7fbbe1ee1bfec9ffb5801a60289b4cc/")
        );
    }

    struct Silent;
    impl DownloadListener for Silent {
        fn on_progress(&self, _: u64, _: u64) {}
    }

    #[test]
    fn explicit_cached_acquisition_verifies_before_success_and_honors_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = model_entry();
        entry.size_bytes = 5;
        entry.sha256 = digest(b"owned");
        entry.url = "https://example.invalid/owned.gguf".into();
        let store = crate::models::ModelStore::with_catalog(dir.path(), vec![entry.clone()]);
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        let path = dir.path().join("models").join(format!("{MODEL_ID}.gguf"));
        std::fs::write(&path, b"owned").unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            store.download(MODEL_ID, &Silent, &cancelled),
            Err(PennelError::Cancelled)
        ));
        assert!(!store.managed_decider.receipt.exists());
        cancelled.store(false, Ordering::Relaxed);
        store.download(MODEL_ID, &Silent, &cancelled).unwrap();
        assert!(
            store
                .managed_decider
                .receipt_matches(&path, 5, &entry.sha256)
        );
        std::fs::write(&path, b"other").unwrap();
        assert!(store.download(MODEL_ID, &Silent, &cancelled).is_err());
        assert!(
            !store
                .managed_decider
                .receipt_matches(&path, 5, &entry.sha256)
        );
    }

    fn question() -> crate::DecisionQuestion {
        crate::DecisionQuestion {
            id: "supported".into(),
            kind: DecisionQuestionKind::Choice,
            instructions: "Is the stated work supported?".into(),
            options: vec![
                crate::DecisionOption {
                    id: "supported".into(),
                    description: "Explicit work".into(),
                },
                crate::DecisionOption {
                    id: "absent".into(),
                    description: "No such work".into(),
                },
            ],
        }
    }

    fn answer_result() -> DecisionResult {
        DecisionResult {
            answers: vec![crate::DecisionAnswer {
                id: "supported".into(),
                kind: DecisionQuestionKind::Choice,
                choice: Some("supported".into()),
                value: None,
                probabilities: vec![
                    crate::DecisionProbability {
                        option_id: "supported".into(),
                        probability: 0.8,
                    },
                    crate::DecisionProbability {
                        option_id: "absent".into(),
                        probability: 0.2,
                    },
                ],
                confidence: Some(0.6),
                spans: vec![],
                not_stated: vec![],
                abstained: false,
            }],
            identity: crate::InferenceIdentity {
                protocol: InferenceProtocol::LocalDecider,
                endpoint: ENDPOINT.into(),
                requested_model: MODEL_ID.into(),
                requested_upstream: None,
                actual_model: None,
                actual_revision: None,
                actual_provider: None,
                request_id: None,
            },
            usage: crate::InferenceUsage {
                input_tokens: None,
                output_tokens: None,
                billed_cost_usd: None,
            },
        }
    }

    #[test]
    fn managed_readout_requires_complete_unique_finite_choice_distributions() {
        let request = DecisionRequest {
            state: "Invented source only.".into(),
            questions: vec![question()],
        };
        validate_request(&request).unwrap();
        validate_answers(&request, &answer_result()).unwrap();
        for change in 0..9 {
            let mut result = answer_result();
            match change {
                0 => result.answers.clear(),
                1 => result.answers.push(result.answers[0].clone()),
                2 => result.answers[0].id = "other".into(),
                3 => result.answers[0].probabilities[0].probability = f64::NAN,
                4 => result.answers[0].probabilities[0].probability = 0.3,
                5 => result.answers[0].probabilities[0].option_id = "absent".into(),
                6 => result.answers[0].choice = Some("invented".into()),
                7 => result.answers[0].choice = None,
                _ => result.answers[0].confidence = Some(f64::INFINITY),
            }
            assert!(
                validate_answers(&request, &result).is_err(),
                "change {change}"
            );
        }
    }

    #[test]
    fn managed_request_rejects_duplicate_questions_unbounded_text_and_other_tasks() {
        let mut request = DecisionRequest {
            state: "Invented source only.".into(),
            questions: vec![question()],
        };
        request.questions.push(question());
        assert!(validate_request(&request).is_err());
        request.questions.pop();
        request.questions[0].kind = DecisionQuestionKind::Extract;
        assert!(validate_request(&request).is_err());
        request.questions[0] = question();
        request.state = "x".repeat(MAX_INPUT_BYTES);
        assert!(validate_request(&request).is_err());
    }
}
