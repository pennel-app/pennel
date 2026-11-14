use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::{
    ENDPOINT, FileStamp, MODEL_BYTES, MODEL_ID, MODEL_SHA256, ManagedLocalDeciderProtocol,
    ManagedLocalDeciderRuntimeStatus, PROVIDER, VerifiedLocalDeciderArtifact, invalid,
};
use crate::{
    DecisionRequest, DecisionResult, InferenceCancellation, InferenceIdentity, InferenceProtocol,
    InferenceUsage, PennelError,
};

const ABI: &str = "pennel-managed-decider-cabi-v1";
const COMPONENT: &str = "libpennel_decider.dylib";
const FRAME_LIMIT: usize = 2 * 1024 * 1024;
const MAX_TOKENS: u32 = 32768;
const MAX_SEQUENCE_TOKENS: usize = 4096;
const GPU_LAYERS: i32 = 999;
const POLL: Duration = Duration::from_millis(25);

fn hash(bytes: &[u8]) -> String {
    crate::models::hex(&Sha256::digest(bytes))
}
fn production_hash(source: &str) -> String {
    let marker = ["#[cfg", "(test)]"].concat();
    hash(
        source
            .split_once(&marker)
            .map_or(source, |(production, _)| production)
            .as_bytes(),
    )
}
fn expected_build() -> (String, Value) {
    let lock: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/native/decider/runtime-lock.json"
    )))
    .expect("owned native lock");
    let inputs = json!({"lock":lock,
        "native_source_sha256":hash(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/native/decider/owned_decider.cpp"))),
        "build_definition_sha256":hash(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/native/decider/CMakeLists.txt"))),
        "exported_abi_sha256":hash(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/native/decider/exports.txt")))});
    (
        hash(&serde_json::to_vec(&inputs).expect("owned native build inputs")),
        inputs,
    )
}
fn serving_revision(signature: &str, threads: i32) -> String {
    hash(
        &serde_json::to_vec(&json!({"native_build_signature":signature,
        "rust_runtime_sha256":production_hash(include_str!("mac.rs")),
        "rust_host_sha256":production_hash(include_str!("../native.rs")),
        "rust_protocol_sha256":production_hash(include_str!("../protocol.rs")),
        "rust_artifact_manager_sha256":production_hash(include_str!("../../local_decider.rs")),
        "window_scope_sha256":production_hash(include_str!("../scopes.rs")),
        "max_context_tokens":MAX_TOKENS,"max_sequence_tokens":MAX_SEQUENCE_TOKENS,"max_sequences":8,"kv_unified":false,
        "question_mode":"independent-single-question-v1","questions_per_sequence":1,"gpu_layers":GPU_LAYERS,"threads":threads,
        "protocol":"joint-question-inline-readout-v1","transport":"owned-process-jsonl-v1"}))
        .expect("owned serving revision"),
    )
}

struct Bundle {
    directory: PathBuf,
    component: VerifiedLocalDeciderArtifact,
    signature: String,
}
impl Bundle {
    fn open(directory: PathBuf) -> Result<Arc<Self>, PennelError> {
        let metadata = std::fs::symlink_metadata(directory.join("manifest.json"))
            .map_err(|_| invalid("The local Decider runtime manifest is missing."))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65536 {
            return Err(invalid("The local Decider runtime manifest is invalid."));
        }
        let value: Value = serde_json::from_reader(
            File::open(directory.join("manifest.json")).map_err(PennelError::storage)?,
        )
        .map_err(|_| invalid("The local Decider runtime manifest is invalid."))?;
        let (signature, inputs) = expected_build();
        if value["schema_version"] != 1
            || value["abi"] != ABI
            || value["platform"] != "macos-aarch64"
            || value["minimum_macos"] != "14.4"
            || value["build_signature"] != signature
            || value["source_inputs"] != inputs
            || value["component"]["file"] != COMPONENT
        {
            return Err(invalid(
                "The local Decider runtime does not match this application.",
            ));
        }
        let bytes = value["component"]["bytes"]
            .as_u64()
            .filter(|value| *value > 0 && *value <= 64 * 1024 * 1024)
            .ok_or_else(|| invalid("The local Decider runtime size is invalid."))?;
        let sha = value["component"]["sha256"]
            .as_str()
            .filter(|value| super::super::valid_hash(value))
            .ok_or_else(|| invalid("The local Decider runtime digest is invalid."))?;
        let path = directory.join(COMPONENT);
        if std::fs::symlink_metadata(&path)
            .map_err(PennelError::storage)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid(
                "The local Decider runtime must not be a symbolic link.",
            ));
        }
        let component = VerifiedLocalDeciderArtifact::from_open_file(
            File::open(&path).map_err(PennelError::storage)?,
            Some(path),
            bytes,
            sha,
            || false,
        )?;
        Ok(Arc::new(Self {
            directory,
            component,
            signature,
        }))
    }
    fn valid(&self) -> bool {
        self.component.verify_unchanged().is_ok()
    }
}

pub(super) struct Runtime {
    bundle: Arc<Bundle>,
    executable: PathBuf,
    threads: i32,
    gate: Mutex<()>,
    worker: Mutex<Option<Arc<Worker>>>,
    revocation: AtomicU64,
}
impl Runtime {
    pub(super) fn new(bundle: PathBuf, executable: PathBuf) -> Result<Self, PennelError> {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(invalid("The owned local Decider worker is unavailable."));
        }
        Ok(Self {
            bundle: Bundle::open(bundle)?,
            executable,
            threads: std::thread::available_parallelism()
                .map_or(1, usize::from)
                .clamp(1, 8) as i32,
            gate: Mutex::new(()),
            worker: Mutex::new(None),
            revocation: AtomicU64::new(0),
        })
    }
    pub(super) fn bundle_path(&self) -> &Path {
        &self.bundle.directory
    }
    pub(super) fn status(&self) -> ManagedLocalDeciderRuntimeStatus {
        let available = self.bundle.valid() && self.executable.is_file();
        ManagedLocalDeciderRuntimeStatus {
            available,
            revision: available.then(|| serving_revision(&self.bundle.signature, self.threads)),
            reason_code: (!available).then(|| "runtime_changed".into()),
        }
    }
    pub(super) fn unload(&self) {
        self.revocation.fetch_add(1, Ordering::AcqRel);
        // Spawn/publication and reaping share this short lifecycle boundary.
        // Seeing None therefore means no unpublished/stopping child remains.
        let mut slot = self.worker.lock().unwrap();
        if let Some(worker) = slot.as_ref() {
            worker.stop();
        }
        *slot = None;
    }
    fn retire(&self, worker: &Arc<Worker>) {
        let mut slot = self.worker.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, worker))
        {
            worker.stop();
            *slot = None;
        } else {
            worker.stop();
        }
    }
    fn current(&self, epoch: u64, worker: &Arc<Worker>) -> bool {
        self.revocation.load(Ordering::Acquire) == epoch
            && self
                .worker
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, worker))
    }
    fn acquire_gate(
        &self,
        cancellation: &InferenceCancellation,
        deadline: Instant,
    ) -> Result<std::sync::MutexGuard<'_, ()>, PennelError> {
        loop {
            if cancellation.is_cancelled() {
                return Err(PennelError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(invalid("The local Decider request timed out."));
            }
            match self.gate.try_lock() {
                Ok(gate) => return Ok(gate),
                Err(std::sync::TryLockError::WouldBlock) => {
                    thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())))
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(invalid("The local Decider runtime is unavailable."));
                }
            }
        }
    }
    pub(super) fn decide(
        &self,
        model: Arc<VerifiedLocalDeciderArtifact>,
        request_id: String,
        request: DecisionRequest,
        cancellation: Arc<InferenceCancellation>,
        deadline: Instant,
    ) -> Result<DecisionResult, PennelError> {
        let epoch = self.revocation.load(Ordering::Acquire);
        let _gate = self.acquire_gate(&cancellation, deadline)?;
        if self.revocation.load(Ordering::Acquire) != epoch {
            return Err(PennelError::Cancelled);
        }
        if !self.bundle.valid() {
            return Err(invalid("The local Decider runtime changed."));
        }
        let (worker, started) = {
            let mut slot = self.worker.lock().unwrap();
            if cancellation.is_cancelled()
                || Instant::now() >= deadline
                || self.revocation.load(Ordering::Acquire) != epoch
            {
                return Err(PennelError::Cancelled);
            }
            if let Some(worker) = slot
                .as_ref()
                .filter(|worker| Arc::ptr_eq(&worker.model, &model) && worker.running())
            {
                (worker.clone(), false)
            } else {
                if let Some(previous) = slot.as_ref() {
                    previous.stop();
                }
                *slot = None;
                // Publish immediately after OS spawn, before any readiness/model-load wait.
                let worker = Worker::start(
                    &self.bundle.directory,
                    &self.executable,
                    model,
                    self.threads,
                )?;
                *slot = Some(worker.clone());
                (worker, true)
            }
        };
        if started
            && let Err(error) = worker.ready(&self.bundle, self.threads, &cancellation, deadline)
        {
            self.retire(&worker);
            if cancellation.is_cancelled() || self.revocation.load(Ordering::Acquire) != epoch {
                return Err(PennelError::Cancelled);
            }
            return Err(error);
        }
        if !self.current(epoch, &worker) {
            self.retire(&worker);
            return Err(PennelError::Cancelled);
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(90000) as u64;
        let frame = InputFrame {
            request_id: request_id.clone(),
            timeout_ms: remaining,
            request,
        };
        let encoded = serde_json::to_vec(&frame).map_err(PennelError::storage)?;
        if encoded.len() > FRAME_LIMIT {
            return Err(invalid("The local Decider worker request is too large."));
        }
        if let Err(error) = worker.send(&encoded, &cancellation, deadline) {
            self.retire(&worker);
            return Err(error);
        }
        let result = worker.receive(&cancellation, deadline).and_then(|frame| {
            if frame.event != "result" || frame.request_id.as_deref() != Some(&request_id) {
                return Err(invalid(
                    "The local Decider worker returned an unexpected response.",
                ));
            }
            let result = frame
                .result
                .ok_or_else(|| invalid("The local Decider could not complete this request."))?;
            #[cfg(feature = "evaluation")]
            crate::evaluation::local_review::native_execution_observed(
                result.identity.request_id.as_deref(),
                frame.backend_metadata.as_ref(),
            );
            Ok(result)
        });
        if result.is_err() {
            self.retire(&worker);
            if cancellation.is_cancelled() || self.revocation.load(Ordering::Acquire) != epoch {
                return Err(PennelError::Cancelled);
            }
            return result;
        }
        if !self.current(epoch, &worker) {
            self.retire(&worker);
            return Err(
                if cancellation.is_cancelled() || self.revocation.load(Ordering::Acquire) != epoch {
                    PennelError::Cancelled
                } else {
                    invalid("The local Decider worker stopped.")
                },
            );
        }
        if !self.bundle.valid() {
            self.retire(&worker);
            return Err(invalid("The local Decider runtime changed."));
        }
        result
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.unload();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFrame {
    request_id: String,
    timeout_ms: u64,
    request: DecisionRequest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputFrame {
    event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<InferenceIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<DecisionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend_metadata: Option<Value>,
}
fn read_frame(reader: &mut impl BufRead) -> Result<Vec<u8>, PennelError> {
    let mut value = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| invalid("The local Decider worker stopped."))?;
        if available.is_empty() {
            return Err(invalid("The local Decider worker stopped."));
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if value.len() + count > FRAME_LIMIT {
            return Err(invalid("The local Decider worker response is too large."));
        }
        let finished = available[count - 1] == b'\n';
        value.extend_from_slice(&available[..count]);
        reader.consume(count);
        if finished {
            return Ok(value);
        }
    }
}
struct Worker {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    responses: Mutex<mpsc::Receiver<Result<OutputFrame, PennelError>>>,
    model: Arc<VerifiedLocalDeciderArtifact>,
}
unsafe extern "C" {
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
}
impl Worker {
    fn start(
        directory: &Path,
        executable: &Path,
        model: Arc<VerifiedLocalDeciderArtifact>,
        threads: i32,
    ) -> Result<Arc<Self>, PennelError> {
        model.verify_unchanged()?;
        let fd = model.file().as_raw_fd();
        let mut command = Command::new(executable);
        command
            .args(["--pennel-decider-worker", "--runtime-dir"])
            .arg(directory)
            .args([
                "--model-fd",
                &fd.to_string(),
                "--threads",
                &threads.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear();
        for name in [
            "HOME",
            "TMPDIR",
            "USER",
            "LOGNAME",
            "LANG",
            "LC_ALL",
            "__CF_USER_TEXT_ENCODING",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        // Change only the forked child's descriptor flags; no parent/global FD window.
        unsafe {
            command.pre_exec(move || {
                let flags = fcntl(fd, 1);
                if flags < 0 || fcntl(fd, 2, flags & !1) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|_| invalid("The owned local Decider worker could not start."))?;
        let stdin = child.stdin.take().expect("owned child stdin");
        let stdout = child.stdout.take().expect("owned child stdout");
        // Bound writes even when an owned worker stops reading its input pipe.
        let flags = unsafe { fcntl(stdin.as_raw_fd(), 3) };
        if flags < 0 || unsafe { fcntl(stdin.as_raw_fd(), 4, flags | 4) } < 0 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid(
                "The local Decider worker pipe could not be bounded.",
            ));
        }
        let (send, receive) = mpsc::sync_channel(2);
        let reader = thread::Builder::new()
            .name("pennel-decider-output".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let frame = read_frame(&mut reader).and_then(|value| {
                        serde_json::from_slice(&value).map_err(|_| {
                            invalid("The local Decider worker returned malformed data.")
                        })
                    });
                    let failed = frame.is_err();
                    if send.send(frame).is_err() || failed {
                        break;
                    }
                }
            });
        if reader.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid("The local Decider worker output could not start."));
        }
        let worker = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            responses: Mutex::new(receive),
            model,
        });
        Ok(worker)
    }
    fn ready(
        &self,
        bundle: &Bundle,
        threads: i32,
        cancellation: &InferenceCancellation,
        deadline: Instant,
    ) -> Result<(), PennelError> {
        let ready = self.receive(cancellation, deadline)?;
        let identity = ready
            .identity
            .ok_or_else(|| invalid("The local Decider worker omitted its loaded identity."))?;
        if ready.event != "ready"
            || ready.artifact_sha256.as_deref() != Some(self.model.sha256())
            || identity.actual_model.as_deref() != Some(self.model.model_fingerprint().as_str())
            || identity.actual_revision.as_deref()
                != Some(serving_revision(&bundle.signature, threads).as_str())
            || identity.actual_provider.as_deref() != Some(PROVIDER)
        {
            self.stop();
            return Err(invalid(
                "The local Decider worker loaded an unexpected model or runtime.",
            ));
        }
        Ok(())
    }
    fn running(&self) -> bool {
        self.child
            .lock()
            .unwrap()
            .try_wait()
            .is_ok_and(|value| value.is_none())
    }
    fn stop(&self) {
        let mut child = self.child.lock().unwrap();
        if child.try_wait().is_ok_and(|value| value.is_none()) {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
    fn send(
        &self,
        encoded: &[u8],
        cancellation: &InferenceCancellation,
        deadline: Instant,
    ) -> Result<(), PennelError> {
        let mut bytes = encoded.to_vec();
        bytes.push(b'\n');
        let mut offset = 0;
        let mut stdin = self.stdin.lock().unwrap();
        while offset < bytes.len() {
            if cancellation.is_cancelled() {
                self.stop();
                return Err(PennelError::Cancelled);
            }
            if Instant::now() >= deadline {
                self.stop();
                return Err(invalid("The local Decider request timed out."));
            }
            match stdin.write(&bytes[offset..]) {
                Ok(0) => return Err(invalid("The local Decider worker stopped.")),
                Ok(count) => offset += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return Err(invalid("The local Decider worker stopped.")),
            }
        }
        Ok(())
    }
    fn receive(
        &self,
        cancellation: &InferenceCancellation,
        deadline: Instant,
    ) -> Result<OutputFrame, PennelError> {
        loop {
            if cancellation.is_cancelled() {
                self.stop();
                return Err(PennelError::Cancelled);
            }
            if Instant::now() >= deadline {
                self.stop();
                return Err(invalid("The local Decider request timed out."));
            }
            match self.responses.lock().unwrap().recv_timeout(POLL) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(invalid("The local Decider worker stopped.")),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

type Load = unsafe extern "C" fn(
    c_int,
    u32,
    c_int,
    c_int,
    *const c_char,
    *mut c_char,
    usize,
) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Abort = unsafe extern "C" fn(*mut c_void);
type Tokenize =
    unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut i32, i32, bool, bool) -> i32;
type Logits = unsafe extern "C" fn(
    *mut c_void,
    *const i32,
    i32,
    *const i32,
    *const i32,
    *const i32,
    i32,
    *const i32,
    i32,
    *mut f32,
    *mut c_char,
    usize,
) -> i32;

type MetadataFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char, usize) -> c_int;
type SourceStat = unsafe extern "C" fn(*mut c_void, *mut u64, usize) -> c_int;
type BufferMetadata = unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> c_int;
unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}
struct Library {
    handle: *mut c_void,
    load: Load,
    destroy: Destroy,
    abort: Abort,
    tokenize: Tokenize,
    logits: Logits,
    metadata: MetadataFn,
    source_stat: SourceStat,
    buffer_metadata: BufferMetadata,
    signature: String,
}
// The narrow ABI serializes decoder access; abort is the shim's atomic-only operation.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Library {
    fn open(bundle: &Bundle) -> Result<Arc<Self>, PennelError> {
        if !bundle.valid() {
            return Err(invalid("The local Decider runtime changed."));
        }
        let path = CString::new(bundle.directory.join(COMPONENT).as_os_str().as_bytes())
            .map_err(|_| invalid("The local Decider runtime path is invalid."))?;
        let handle = unsafe { dlopen(path.as_ptr(), 2 | 4) };
        if handle.is_null() {
            return Err(invalid("The local Decider native runtime could not load."));
        }
        let result = (|| {
            unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &[u8]) -> Result<T, PennelError> {
                let pointer = unsafe { dlsym(handle, name.as_ptr().cast()) };
                if pointer.is_null()
                    || std::mem::size_of::<T>() != std::mem::size_of::<*mut c_void>()
                {
                    return Err(invalid("The local Decider native ABI is incomplete."));
                }
                Ok(unsafe { std::mem::transmute_copy(&pointer) })
            }
            type Text = unsafe extern "C" fn() -> *const c_char;
            let abi: Text = unsafe { symbol(handle, b"od_abi\0")? };
            let signature: Text = unsafe { symbol(handle, b"od_build_signature\0")? };
            let abi = unsafe { abi() };
            let signature = unsafe { signature() };
            if abi.is_null() || signature.is_null() {
                return Err(invalid("The local Decider native ABI is incomplete."));
            }
            let abi = unsafe { CStr::from_ptr(abi) }
                .to_str()
                .map_err(|_| invalid("The local Decider native ABI is invalid."))?;
            let signature = unsafe { CStr::from_ptr(signature) }
                .to_str()
                .map_err(|_| invalid("The local Decider native revision is invalid."))?
                .to_owned();
            if abi != ABI || signature != bundle.signature {
                return Err(invalid(
                    "The local Decider loaded code does not match its verified build.",
                ));
            }
            if !bundle.valid() {
                return Err(invalid("The local Decider runtime changed while loading."));
            }
            Ok(Self {
                handle,
                load: unsafe { symbol(handle, b"od_load\0")? },
                destroy: unsafe { symbol(handle, b"od_destroy\0")? },
                abort: unsafe { symbol(handle, b"od_abort\0")? },
                tokenize: unsafe { symbol(handle, b"od_tokenize\0")? },
                logits: unsafe { symbol(handle, b"od_logits_sequences\0")? },
                metadata: unsafe { symbol(handle, b"od_metadata\0")? },
                source_stat: unsafe { symbol(handle, b"od_source_stat\0")? },
                buffer_metadata: unsafe { symbol(handle, b"od_model_buffer_metadata\0")? },
                signature,
            })
        })();
        if result.is_err() {
            unsafe {
                dlclose(handle);
            }
        }
        result.map(Arc::new)
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.handle);
        }
    }
}
struct Instance {
    library: Arc<Library>,
    handle: *mut c_void,
}
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}
impl Instance {
    fn new(
        library: Arc<Library>,
        model: &VerifiedLocalDeciderArtifact,
        threads: i32,
    ) -> Result<Arc<Self>, PennelError> {
        let mut error = vec![0u8; 256];
        let handle = unsafe {
            (library.load)(
                model.file().as_raw_fd(),
                MAX_TOKENS,
                GPU_LAYERS,
                threads,
                std::ptr::null(),
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        if handle.is_null() {
            return Err(invalid("The local Decider model could not load."));
        }
        let instance = Arc::new(Self { library, handle });
        model.verify_unchanged()?;
        let mut loaded = [0u64; 7];
        if unsafe { (instance.library.source_stat)(handle, loaded.as_mut_ptr(), loaded.len()) } != 0
        {
            return Err(invalid("The local Decider lost its loaded file binding."));
        }
        let stamp =
            FileStamp::from_metadata(&model.file().metadata().map_err(PennelError::storage)?)?;
        if loaded
            != [
                stamp.device,
                stamp.inode,
                stamp.bytes,
                (stamp.modified_ns / 1_000_000_000) as u64,
                (stamp.modified_ns % 1_000_000_000) as u64,
                stamp.changed_secs as u64,
                stamp.changed_ns as u64,
            ]
        {
            return Err(invalid("The local Decider loaded a different file handle."));
        }
        for (key, expected) in [
            ("general.architecture", "qwen35"),
            ("pplx_decider.template", "joint-question-inline-readout-v1"),
            ("pplx_decider.marker", "<|fim_suffix|>"),
        ] {
            let key = CString::new(key).expect("owned metadata key");
            let mut text = vec![0u8; 4096];
            let count = unsafe {
                (instance.library.metadata)(
                    handle,
                    key.as_ptr(),
                    text.as_mut_ptr().cast(),
                    text.len(),
                )
            };
            if count < 0
                || count as usize >= text.len()
                || &text[..count as usize] != expected.as_bytes()
            {
                return Err(invalid(
                    "The local Decider loaded metadata differs from the verified artifact.",
                ));
            }
        }
        Ok(instance)
    }
    fn buffer_metadata(&self) -> Result<Value, PennelError> {
        let mut output = vec![0u8; 16384];
        let count = unsafe {
            (self.library.buffer_metadata)(self.handle, output.as_mut_ptr().cast(), output.len())
        };
        if count < 0 || count as usize >= output.len() {
            return Err(invalid(
                "The local Decider backend metadata is unavailable.",
            ));
        }
        serde_json::from_slice(&output[..count as usize])
            .map_err(|_| invalid("The local Decider backend metadata is invalid."))
    }
    fn tokens(&self, text: &str) -> Result<Vec<i32>, PennelError> {
        let mut tokens = vec![0; MAX_TOKENS as usize];
        let count = unsafe {
            (self.library.tokenize)(
                self.handle,
                text.as_ptr().cast(),
                i32::try_from(text.len())
                    .map_err(|_| invalid("The local Decider prompt is too large."))?,
                tokens.as_mut_ptr(),
                MAX_TOKENS as i32,
                false,
                true,
            )
        };
        if count <= 0 || count > MAX_TOKENS as i32 {
            return Err(invalid(
                "The local Decider prompt exceeds its token limit; it was not truncated.",
            ));
        }
        tokens.truncate(count as usize);
        Ok(tokens)
    }
    fn readout(
        &self,
        tokens: &[i32],
        sequences: &[i32],
        positions: &[i32],
        markers: &[i32],
        codes: &[i32],
    ) -> Result<Vec<Vec<f32>>, PennelError> {
        let mut output = vec![0.0; markers.len() * codes.len()];
        let mut error = vec![0u8; 256];
        let status = unsafe {
            (self.library.logits)(
                self.handle,
                tokens.as_ptr(),
                tokens.len() as i32,
                sequences.as_ptr(),
                positions.as_ptr(),
                markers.as_ptr(),
                markers.len() as i32,
                codes.as_ptr(),
                codes.len() as i32,
                output.as_mut_ptr(),
                error.as_mut_ptr().cast(),
                error.len(),
            )
        };
        if status != 0 {
            return Err(invalid(
                "The local Decider could not complete its bounded readout.",
            ));
        }
        Ok(output.chunks(codes.len()).map(<[f32]>::to_vec).collect())
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            (self.library.destroy)(self.handle);
        }
    }
}

fn write_frame(frame: &OutputFrame) -> Result<(), PennelError> {
    let encoded = serde_json::to_vec(frame).map_err(PennelError::storage)?;
    if encoded.len() > FRAME_LIMIT {
        return Err(invalid("The local Decider result is too large."));
    }
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&encoded)
        .and_then(|()| stdout.write_all(b"\n"))
        .and_then(|()| stdout.flush())
        .map_err(PennelError::storage)
}
fn parse_arguments(arguments: &[String]) -> Result<(PathBuf, c_int, i32), PennelError> {
    if arguments.len() != 6
        || arguments[0] != "--runtime-dir"
        || arguments[2] != "--model-fd"
        || arguments[4] != "--threads"
    {
        return Err(invalid("The local Decider worker arguments are invalid."));
    }
    let fd = arguments[3]
        .parse::<c_int>()
        .ok()
        .filter(|fd| *fd >= 3)
        .ok_or_else(|| invalid("The local Decider file descriptor is invalid."))?;
    let threads = arguments[5]
        .parse::<i32>()
        .ok()
        .filter(|value| (1..=8).contains(value))
        .ok_or_else(|| invalid("The local Decider worker bounds are invalid."))?;
    if unsafe { fcntl(fd, 1) } < 0 {
        return Err(invalid("The local Decider file descriptor is unavailable."));
    }
    Ok((PathBuf::from(&arguments[1]), fd, threads))
}
fn worker(arguments: &[String]) -> Result<(), PennelError> {
    let (directory, fd, threads) = parse_arguments(arguments)?;
    let bundle = Bundle::open(directory)?;
    let model = Arc::new(VerifiedLocalDeciderArtifact::from_open_file(
        unsafe { File::from_raw_fd(fd) },
        None,
        MODEL_BYTES,
        MODEL_SHA256,
        || false,
    )?);
    let protocol = ManagedLocalDeciderProtocol::from_artifact(&model)?;
    let library = Library::open(&bundle)?;
    let instance = Instance::new(library.clone(), &model, threads)?;
    let marker = instance.tokens(protocol.marker())?;
    if marker.len() != 1 {
        return Err(invalid(
            "The local Decider marker vocabulary is incompatible.",
        ));
    }
    // Validate the pinned GGUF vocabulary against the actual loaded tokenizer.
    for (code, token) in protocol.codes().iter().zip(protocol.code_tokens()) {
        if instance.tokens(code)? != [*token] {
            return Err(invalid("The local Decider loaded code vocabulary differs."));
        }
    }
    let revision = serving_revision(&library.signature, threads);
    let identity = InferenceIdentity {
        protocol: InferenceProtocol::LocalDecider,
        endpoint: ENDPOINT.into(),
        requested_model: MODEL_ID.into(),
        requested_upstream: None,
        actual_model: Some(model.model_fingerprint()),
        actual_revision: Some(revision),
        actual_provider: Some(PROVIDER.into()),
        request_id: None,
    };
    write_frame(&OutputFrame {
        event: "ready".into(),
        identity: Some(identity.clone()),
        artifact_sha256: Some(model.sha256().into()),
        request_id: None,
        result: None,
        error_code: None,
        backend_metadata: Some(instance.buffer_metadata()?),
    })?;
    let mut reader = BufReader::new(std::io::stdin().lock());
    loop {
        if reader.fill_buf().map_err(PennelError::storage)?.is_empty() {
            return Ok(());
        }
        let bytes = read_frame(&mut reader)?;
        let frame: InputFrame = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("The local Decider request is malformed."))?;
        if uuid::Uuid::parse_str(&frame.request_id).is_err()
            || frame.timeout_ms == 0
            || frame.timeout_ms > 90000
        {
            return Err(invalid("The local Decider request bounds are invalid."));
        }
        model.verify_unchanged()?;
        if !bundle.valid() {
            return Err(invalid("The local Decider runtime changed."));
        }
        let deadline = Instant::now() + Duration::from_millis(frame.timeout_ms);
        let scoped = super::super::scopes::questions(&frame.request)?;
        let mut mapped = vec![None; frame.request.questions.len()];
        let mut actual_input_tokens = 0u64;
        let mut native_decode_calls = 0u64;
        for chunk in scoped.chunks(8) {
            let mut tokens = Vec::new();
            let mut sequences = Vec::new();
            let mut positions = Vec::new();
            let mut markers = Vec::new();
            for (sequence, question) in chunk.iter().enumerate() {
                let text = protocol.prompt(&question.request)?;
                let question_tokens = instance.tokens(&text)?;
                if question_tokens.len() > MAX_SEQUENCE_TOKENS {
                    return Err(invalid(
                        "The local Decider question exceeds its 4096-token sequence budget; it was not truncated.",
                    ));
                }
                let offset = tokens.len();
                if offset + question_tokens.len() > MAX_TOKENS as usize {
                    return Err(invalid(
                        "The independent local Decider questions exceed their native token budget; they were not truncated.",
                    ));
                }
                let question_markers = question_tokens
                    .iter()
                    .enumerate()
                    .filter_map(|(index, token)| {
                        (*token == marker[0]).then_some((offset + index) as i32)
                    })
                    .collect::<Vec<_>>();
                if question_markers.len() != 1 {
                    return Err(invalid(
                        "The local Decider single-question prompt contains an unexpected marker.",
                    ));
                }
                markers.extend(question_markers);
                positions.extend((0..question_tokens.len()).map(|index| index as i32));
                sequences.extend(std::iter::repeat_n(sequence as i32, question_tokens.len()));
                tokens.extend(question_tokens);
            }
            if Instant::now() >= deadline {
                return Err(invalid("The local Decider request timed out."));
            }
            let finished = Arc::new(AtomicBool::new(false));
            let timer_done = finished.clone();
            let timer_model = instance.clone();
            let timer = thread::spawn(move || {
                while !timer_done.load(Ordering::Acquire) {
                    if Instant::now() >= deadline {
                        unsafe {
                            (timer_model.library.abort)(timer_model.handle);
                        }
                        break;
                    }
                    thread::sleep(POLL);
                }
            });
            let values = instance.readout(
                &tokens,
                &sequences,
                &positions,
                &markers,
                protocol.code_tokens(),
            );
            finished.store(true, Ordering::Release);
            let _ = timer.join();
            let values = values?;
            actual_input_tokens += tokens.len() as u64;
            native_decode_calls += 1;
            if Instant::now() > deadline {
                return Err(invalid("The local Decider request timed out."));
            }
            model.verify_unchanged()?;
            if !bundle.valid() {
                return Err(invalid("The local Decider runtime changed."));
            }
            for (question, row) in chunk.iter().zip(values) {
                let mut answers = protocol.answers(&question.request, &[row])?;
                let mut answer = answers
                    .pop()
                    .ok_or_else(|| invalid("The local Decider question readout is missing."))?;
                let index = question.original_indices[0];
                answer.id = frame.request.questions[index].id.clone();
                if mapped[index].replace(answer).is_some() {
                    return Err(invalid(
                        "The local Decider produced a duplicate question mapping.",
                    ));
                }
            }
        }
        let answers = mapped
            .into_iter()
            .map(|answer| {
                answer.ok_or_else(|| invalid("The local Decider window mapping is incomplete."))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut call_identity = identity.clone();
        call_identity.request_id = Some(frame.request_id.clone());
        write_frame(&OutputFrame {
            event: "result".into(),
            request_id: Some(frame.request_id),
            identity: None,
            artifact_sha256: None,
            error_code: None,
            backend_metadata: Some(
                json!({"scope":"actual native execution for this logical request","native_decode_calls":native_decode_calls,"independent_question_sequences":scoped.len(),"expanded_input_tokens":actual_input_tokens}),
            ),
            result: Some(DecisionResult {
                answers,
                identity: call_identity,
                usage: InferenceUsage {
                    input_tokens: Some(actual_input_tokens),
                    output_tokens: Some(0),
                    billed_cost_usd: Some(0.0),
                },
            }),
        })?;
    }
}
pub(super) fn worker_main(arguments: &[String]) -> i32 {
    if worker(arguments).is_ok() {
        0
    } else {
        let _ = write_frame(&OutputFrame {
            event: "error".into(),
            request_id: None,
            identity: None,
            artifact_sha256: None,
            result: None,
            error_code: Some("local_decider_failed".into()),
            backend_metadata: None,
        });
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controlled_runtime() -> (
        tempfile::TempDir,
        Arc<Runtime>,
        Arc<VerifiedLocalDeciderArtifact>,
    ) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let component = directory.path().join("owned-component");
        std::fs::write(&component, b"owned").unwrap();
        let component =
            VerifiedLocalDeciderArtifact::open(&component, 5, &hash(b"owned"), || false).unwrap();
        let model = directory.path().join("owned-synthetic-model");
        std::fs::write(&model, b"model").unwrap();
        let model = Arc::new(
            VerifiedLocalDeciderArtifact::open(&model, 5, &hash(b"model"), || false).unwrap(),
        );
        let executable = directory.path().join("controlled-cold-worker");
        let marker = directory
            .path()
            .join("spawned")
            .to_string_lossy()
            .replace('\'', "'\\''");
        std::fs::write(
            &executable,
            format!("#!/bin/sh\nprintf started > '{marker}'\nexec /bin/sleep 60\n"),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = Arc::new(Runtime {
            bundle: Arc::new(Bundle {
                directory: directory.path().into(),
                component,
                signature: expected_build().0,
            }),
            executable,
            threads: 1,
            gate: Mutex::new(()),
            worker: Mutex::new(None),
            revocation: AtomicU64::new(0),
        });
        (directory, runtime, model)
    }
    fn synthetic_request() -> DecisionRequest {
        DecisionRequest {
            state: "Owned lifecycle fixture; no model inference.".into(),
            questions: vec![],
        }
    }

    #[test]
    fn queued_deadline_and_cancellation_do_not_wait_for_or_kill_active_worker() {
        for cancel in [false, true] {
            let (_directory, runtime, model) = controlled_runtime();
            let active =
                Worker::start(runtime.bundle_path(), &runtime.executable, model.clone(), 1)
                    .unwrap();
            *runtime.worker.lock().unwrap() = Some(active.clone());
            let held = runtime.gate.lock().unwrap();
            let token = InferenceCancellation::new();
            let call_token = token.clone();
            let call_runtime = runtime.clone();
            let (send, receive) = mpsc::channel();
            let thread = thread::spawn(move || {
                let duration = if cancel {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(80)
                };
                let result = call_runtime.decide(
                    model,
                    uuid::Uuid::now_v7().to_string(),
                    synthetic_request(),
                    call_token,
                    Instant::now() + duration,
                );
                send.send(result).unwrap();
            });
            if cancel {
                thread::sleep(Duration::from_millis(30));
                token.cancel();
            }
            let result = receive
                .recv_timeout(Duration::from_millis(500))
                .expect("queued call obeys its own bound");
            assert!(result.is_err());
            if cancel {
                assert_eq!(result.unwrap_err(), PennelError::Cancelled);
            }
            assert!(
                active.running(),
                "a cancelled queued call must not stop the active call's worker"
            );
            drop(held);
            thread.join().unwrap();
            runtime.unload();
            assert!(!active.running());
        }
    }

    #[test]
    fn unload_reaps_published_cold_worker_before_return_and_never_reinstalls_it() {
        let (_directory, runtime, model) = controlled_runtime();
        let call_runtime = runtime.clone();
        let (send, receive) = mpsc::channel();
        let thread = thread::spawn(move || {
            send.send(call_runtime.decide(
                model,
                uuid::Uuid::now_v7().to_string(),
                synthetic_request(),
                InferenceCancellation::new(),
                Instant::now() + Duration::from_secs(10),
            ))
            .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let cold = loop {
            if let Some(worker) = runtime.worker.lock().unwrap().clone() {
                break worker;
            }
            assert!(
                Instant::now() < deadline,
                "cold OS child must be published before Ready"
            );
            thread::sleep(POLL);
        };
        assert!(cold.running());
        let before = Instant::now();
        runtime.unload();
        assert!(before.elapsed() < Duration::from_secs(1));
        assert!(!cold.running(), "unload must reap the actual owned process");
        assert_eq!(
            receive
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap_err(),
            PennelError::Cancelled
        );
        thread.join().unwrap();
        assert!(runtime.worker.lock().unwrap().is_none());
    }

    #[test]
    fn revocation_before_spawn_cannot_publish_a_child_after_unload() {
        let (directory, runtime, model) = controlled_runtime();
        let held = runtime.worker.lock().unwrap();
        let call_runtime = runtime.clone();
        let (send, receive) = mpsc::channel();
        let call = thread::spawn(move || {
            send.send(call_runtime.decide(
                model,
                uuid::Uuid::now_v7().to_string(),
                synthetic_request(),
                InferenceCancellation::new(),
                Instant::now() + Duration::from_secs(3),
            ))
            .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while runtime.gate.try_lock().is_ok() {
            assert!(Instant::now() < deadline);
            thread::sleep(POLL);
        }
        let stop_runtime = runtime.clone();
        let stop = thread::spawn(move || stop_runtime.unload());
        while runtime.revocation.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline);
            thread::sleep(POLL);
        }
        drop(held);
        stop.join().unwrap();
        assert_eq!(
            receive
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap_err(),
            PennelError::Cancelled
        );
        call.join().unwrap();
        assert!(runtime.worker.lock().unwrap().is_none());
        assert!(!directory.path().join("spawned").exists());
    }
    #[test]
    fn bounded_frames_reject_oversized_unterminated_input() {
        assert!(read_frame(&mut vec![b'x'; FRAME_LIMIT + 1].as_slice()).is_err());
        assert!(read_frame(&mut b"unterminated".as_slice()).is_err());
        assert_eq!(read_frame(&mut b"{}\n".as_slice()).unwrap(), b"{}\n");
    }
    #[test]
    fn source_build_identity_is_canonical_and_not_a_model_name() {
        let (signature, inputs) = expected_build();
        assert_eq!(signature.len(), 64);
        assert_eq!(
            inputs["lock"]["source"]["revision"],
            "cd959bae99ff4bed22bef2604f45241c09bbb8c8"
        );
        assert_ne!(
            serving_revision(&signature, 4),
            serving_revision(&signature, 8)
        );
    }
    #[test]
    fn ordinary_cli_does_not_touch_runtime_or_model() {
        assert!(super::super::managed_local_decider_worker_main(vec!["pennel".into()]).is_none());
        assert_eq!(
            parse_arguments(&[]).err().unwrap().to_string(),
            "invalid response: The local Decider worker arguments are invalid."
        );
    }
}
