//! Recoverable, bounded cloud recognition. Audio and voiceprints stay below the native boundary.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use pennel_speakers::{Diarization, MERGE_THRESHOLD, SAMPLE_RATE, merge_chunks};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::PennelEngine;
use crate::error::PennelError;
use crate::meetings::{MeetingStore, SpeakerName, durable_write};
use crate::models::EMBEDDING_ID;
use crate::provider::ProviderKind;
use crate::settings::{Settings, SpeakerRecognitionMode};

const VERSION: u32 = 1;
const ALGORITHM: u32 = 1;
const MAX_FRAMES: u32 = 64_000;
const JOBS: &str = "speaker-jobs";
const MANIFEST: &str = "job.json";
const SPOOL: &str = "upload.wav";
const STALE: &str =
    "The recording, text or speaker names have changed. Start speaker recognition again.";
static JOB_CLAIM_GATE: Mutex<()> = Mutex::new(());

// Identity changes take this before the engine meeting/sync locks, matching job registration.
pub(crate) fn speaker_identity_guard() -> std::sync::MutexGuard<'static, ()> {
    JOB_CLAIM_GATE.lock().unwrap()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecognitionAudioInfo {
    pub total_frames: u64,
    /// SHA-256 of the archive, not of the decoded samples.
    pub audio_digest: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RecognitionAudioBlock {
    pub samples: Vec<f32>,
    pub eof: bool,
}

#[uniffi::export(with_foreign)]
pub trait RecognitionAudioReader: Send + Sync {
    fn info(&self) -> Result<RecognitionAudioInfo, PennelError>;
    fn read_frames(
        &self,
        offset: u64,
        max_frames: u32,
    ) -> Result<RecognitionAudioBlock, PennelError>;
}

#[derive(Clone, uniffi::Record)]
pub struct RecognitionUploadRequest {
    pub request_id: String,
    pub url: String,
    pub bearer_token: String,
    pub file_path: String,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct RecognitionUploadResponse {
    pub status: u16,
    pub body: String,
}

#[uniffi::export(with_foreign)]
pub trait RecognitionUploadProgress: Send + Sync {
    fn sent(&self, sent_bytes: u64, total_bytes: u64);
}

#[uniffi::export(with_foreign)]
pub trait RecognitionFileTransport: Send + Sync {
    /// Called on the Rust worker. The native implementation streams from the file and blocks
    /// this worker until completion; cancellation must unblock it without waiting for the server.
    fn upload(
        &self,
        request: RecognitionUploadRequest,
        progress: Arc<dyn RecognitionUploadProgress>,
    ) -> Result<RecognitionUploadResponse, PennelError>;
    /// Remember cancellation even if the request has not started yet; `upload` must refuse a
    /// pre-cancelled identifier as well as unblocking a task that is already running.
    fn cancel(&self, request_id: String);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerRecognitionStage {
    Preparing,
    Uploading,
    Recognizing,
    ResponseUnknown,
    Interrupted,
    Applying,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, Serialize, Deserialize)]
pub struct SpeakerRecognitionJobSnapshot {
    pub job_id: String,
    pub meeting_uid: String,
    pub meeting_id: String,
    pub stage: SpeakerRecognitionStage,
    pub decoded_frames: u64,
    pub total_frames: u64,
    pub uploaded_bytes: u64,
    pub upload_total_bytes: u64,
    pub confirmed_chunks: u32,
    pub total_chunks: u32,
    pub recoverable: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfirmedChunk {
    offset_ms: u64,
    diarization: Diarization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    algorithm: u32,
    model: String,
    library: PathBuf,
    meeting_id: String,
    audio_digest: String,
    transcript_digest: String,
    names_digest: String,
    #[serde(default)]
    metadata_digest: Option<String>,
    ranges: Vec<(u64, u64)>,
    confirmed: Vec<ConfirmedChunk>,
    #[serde(default)]
    applied_transcript_digest: Option<String>,
    #[serde(default)]
    applied_names_digest: Option<String>,
    snapshot: SpeakerRecognitionJobSnapshot,
}

struct JobState {
    manifest: Manifest,
    running: bool,
}

fn invalid_checkpoint(job_id: &str, library: &Path) -> Manifest {
    Manifest {
        version: VERSION,
        algorithm: ALGORITHM,
        model: EMBEDDING_ID.into(),
        library: library.to_owned(),
        meeting_id: String::new(),
        audio_digest: String::new(),
        transcript_digest: String::new(),
        names_digest: String::new(),
        metadata_digest: None,
        ranges: Vec::new(),
        confirmed: Vec::new(),
        applied_transcript_digest: None,
        applied_names_digest: None,
        snapshot: SpeakerRecognitionJobSnapshot {
            job_id: job_id.into(),
            meeting_uid: String::new(),
            meeting_id: String::new(),
            stage: SpeakerRecognitionStage::Failed,
            decoded_frames: 0,
            total_frames: 0,
            uploaded_bytes: 0,
            upload_total_bytes: 0,
            confirmed_chunks: 0,
            total_chunks: 0,
            recoverable: false,
            error_code: Some("speaker_checkpoint_invalid".into()),
            error_message: Some(
                "The saved speaker job is damaged. Delete this job and start again.".into(),
            ),
        },
    }
}

#[derive(uniffi::Object)]
pub struct SpeakerRecognitionJob {
    state: Mutex<JobState>,
    cancelled: AtomicBool,
    interrupted: AtomicBool,
    transport: Mutex<Option<Arc<dyn RecognitionFileTransport>>>,
    request: Mutex<Option<String>>,
    directory: PathBuf,
    engine: Weak<PennelEngine>,
}

fn invalid(message: impl Into<String>) -> PennelError {
    PennelError::InvalidResponse {
        message: message.into(),
    }
}

fn code(error: &PennelError) -> String {
    match error {
        PennelError::Cloud { code, .. } => code.clone(),
        PennelError::NotSignedIn => "signed_out".into(),
        PennelError::Cancelled => "cancelled".into(),
        PennelError::Network { .. } => "network".into(),
        PennelError::Storage { .. } => "storage".into(),
        PennelError::NotFound => "not_found".into(),
        _ => "recognition".into(),
    }
}

fn private_directory(path: &Path) -> Result<(), PennelError> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(PennelError::storage(
            "The processing folder is not a valid local folder.",
        ));
    }
    fs::create_dir_all(path).map_err(PennelError::storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(PennelError::storage)?;
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<String, PennelError> {
    let mut hash = Sha256::new();
    let mut file = File::open(path).map_err(PennelError::storage)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(PennelError::storage)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(crate::models::hex(&hash.finalize()))
}

pub(crate) fn speaker_names_digest(names: &[SpeakerName]) -> Result<String, PennelError> {
    let mut names = names.to_vec();
    names.sort_by_key(|name| name.voice);
    let bytes = serde_json::to_vec(&names).map_err(PennelError::storage)?;
    Ok(crate::models::hex(&Sha256::digest(bytes)))
}

fn resolve_meeting(store: &MeetingStore, id: &str) -> Result<String, PennelError> {
    store
        .list()
        .into_iter()
        .find(|meeting| meeting.id == id || store.uid_of(&meeting.id).as_deref() == Some(id))
        .map(|meeting| meeting.id)
        .ok_or(PennelError::NotFound)
}

fn upload_ranges(total: u64) -> Vec<(u64, u64)> {
    let chunk = 600 * u64::from(SAMPLE_RATE);
    let tail = 30 * u64::from(SAMPLE_RATE);
    let mut ranges = Vec::new();
    let mut offset = 0;
    while offset < total {
        let mut length = chunk.min(total - offset);
        if total - offset - length < tail {
            length = total - offset;
        }
        ranges.push((offset, length));
        offset += length;
    }
    ranges
}

fn meeting_audio_digest(
    store: &crate::meetings::MeetingStore,
    id: &str,
) -> Result<String, PennelError> {
    digest_file(&store.audio_path(id, crate::Speaker::Others)?)
}

fn commit_is_complete(engine: &PennelEngine, manifest: &Manifest, directory: &Path) -> bool {
    let (Some(transcript), Some(names)) = (
        &manifest.applied_transcript_digest,
        &manifest.applied_names_digest,
    ) else {
        return false;
    };
    if !engine.speaker_commit_finished(&manifest.snapshot.meeting_uid, directory, transcript, names)
    {
        return false;
    }
    let store = engine.meeting_store();
    resolve_meeting(&store, &manifest.snapshot.meeting_uid)
        .ok()
        .is_some_and(|id| {
            meeting_audio_digest(&store, &id).ok().as_ref() == Some(&manifest.audio_digest)
        })
}

fn restarted_stage(stage: SpeakerRecognitionStage) -> SpeakerRecognitionStage {
    match stage {
        SpeakerRecognitionStage::Uploading | SpeakerRecognitionStage::Recognizing => {
            SpeakerRecognitionStage::ResponseUnknown
        }
        SpeakerRecognitionStage::Preparing | SpeakerRecognitionStage::Applying => {
            SpeakerRecognitionStage::Interrupted
        }
        other => other,
    }
}

impl SpeakerRecognitionJob {
    fn persist(&self) -> Result<(), PennelError> {
        let state = self.state.lock().unwrap();
        durable_write(
            &self.directory.join(MANIFEST),
            &serde_json::to_vec(&state.manifest).map_err(PennelError::storage)?,
        )
    }

    fn update(&self, stage: SpeakerRecognitionStage) -> Result<(), PennelError> {
        {
            let mut state = self.state.lock().unwrap();
            self.check_cancelled()?;
            state.manifest.snapshot.stage = stage;
        }
        self.persist()
    }

    fn check_cancelled(&self) -> Result<(), PennelError> {
        if self.cancelled.load(Ordering::SeqCst) || self.interrupted.load(Ordering::SeqCst) {
            Err(PennelError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn validate_source(
        &self,
        engine: &PennelEngine,
        reader: &dyn RecognitionAudioReader,
    ) -> Result<String, PennelError> {
        self.check_cancelled()?;
        let manifest = self.state.lock().unwrap().manifest.clone();
        if manifest.version != VERSION
            || manifest.algorithm != ALGORITHM
            || manifest.model != EMBEDDING_ID
        {
            return Err(PennelError::Cloud {
                code: "speaker_checkpoint_version".into(),
                message: STALE.into(),
            });
        }
        let store = engine.meeting_store();
        let id = resolve_meeting(&store, &manifest.snapshot.meeting_uid)?;
        let info = reader.info()?;
        if info.total_frames != manifest.snapshot.total_frames
            || info.audio_digest != manifest.audio_digest
            || meeting_audio_digest(&store, &id)? != manifest.audio_digest
            || store.transcript_fingerprint(&id)? != manifest.transcript_digest
            || speaker_names_digest(&store.speaker_names(&id))? != manifest.names_digest
            || store.read_transcript_snapshot(&id)?.metadata_digest != manifest.metadata_digest
        {
            return Err(PennelError::Cloud {
                code: "speaker_stale".into(),
                message: STALE.into(),
            });
        }
        Ok(id)
    }

    fn spool(
        &self,
        reader: &dyn RecognitionAudioReader,
        offset: u64,
        length: u64,
    ) -> Result<PathBuf, PennelError> {
        self.update(SpeakerRecognitionStage::Preparing)?;
        let path = self.directory.join(SPOOL);
        match fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(PennelError::storage(error));
            }
            _ => {}
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(PennelError::storage)?;
        let data_length = u32::try_from(
            length
                .checked_mul(2)
                .ok_or_else(|| invalid("The recording is too long."))?,
        )
        .map_err(|_| invalid("The audio chunk is too large."))?;
        let mut header = crate::wav::encode_wav(&[], SAMPLE_RATE);
        header[4..8].copy_from_slice(&(36 + data_length).to_le_bytes());
        header[40..44].copy_from_slice(&data_length.to_le_bytes());
        file.write_all(&header).map_err(PennelError::storage)?;
        let mut read = 0;
        while read < length {
            self.check_cancelled()?;
            let maximum = u64::from(MAX_FRAMES).min(length - read) as u32;
            let block = reader.read_frames(offset + read, maximum)?;
            if block.samples.is_empty()
                || block.samples.len() > maximum as usize
                || block.samples.iter().any(|sample| !sample.is_finite())
                || (block.eof && read + (block.samples.len() as u64) < length)
            {
                return Err(invalid("The complete recording could not be read."));
            }
            let mut bytes = Vec::with_capacity(block.samples.len() * 2);
            for sample in &block.samples {
                bytes
                    .extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
            file.write_all(&bytes).map_err(PennelError::storage)?;
            read += block.samples.len() as u64;
            self.state.lock().unwrap().manifest.snapshot.decoded_frames = offset + read;
        }
        file.sync_all().map_err(PennelError::storage)?;
        Ok(path)
    }

    fn run(
        self: Arc<Self>,
        reader: Arc<dyn RecognitionAudioReader>,
        transport: Arc<dyn RecognitionFileTransport>,
    ) {
        let result = self.run_inner(reader.as_ref(), transport.clone());
        let _ = fs::remove_file(self.directory.join(SPOOL));
        let mut state = self.state.lock().unwrap();
        if self.cancelled.load(Ordering::SeqCst) {
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Cancelled;
            state.manifest.snapshot.recoverable = false;
        } else if self.interrupted.load(Ordering::SeqCst) {
            state.manifest.snapshot.stage = restarted_stage(state.manifest.snapshot.stage);
            state.manifest.snapshot.recoverable = true;
            state.manifest.snapshot.error_code = Some("interrupted".into());
            state.manifest.snapshot.error_message =
                Some("De verwerking is onderbroken. Je kunt ze later hervatten.".into());
        } else if let Err(error) = &result {
            let unknown = state.manifest.snapshot.stage == SpeakerRecognitionStage::ResponseUnknown;
            state.manifest.snapshot.stage = if unknown {
                SpeakerRecognitionStage::ResponseUnknown
            } else {
                SpeakerRecognitionStage::Failed
            };
            state.manifest.snapshot.error_code = Some(code(error));
            state.manifest.snapshot.error_message = Some(error.to_string());
            state.manifest.snapshot.recoverable = !matches!(error, PennelError::Cloud { code, .. } if matches!(code.as_str(), "speaker_stale" | "speaker_checkpoint_version" | "speaker_model"))
                && !matches!(error, PennelError::NotFound);
        }
        let terminal = matches!(
            state.manifest.snapshot.stage,
            SpeakerRecognitionStage::Cancelled | SpeakerRecognitionStage::Completed
        );
        if terminal {
            state.manifest.confirmed.clear();
        }
        drop(state);
        *self.transport.lock().unwrap() = None;
        *self.request.lock().unwrap() = None;
        if terminal {
            if let Err(error) = fs::remove_dir_all(&self.directory)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                {
                    let mut state = self.state.lock().unwrap();
                    state.manifest.snapshot.stage = SpeakerRecognitionStage::Failed;
                    state.manifest.snapshot.recoverable = false;
                    state.manifest.snapshot.error_code = Some("speaker_cleanup".into());
                    state.manifest.snapshot.error_message = Some(
                        "The temporary audio files could not be deleted. Try deleting them again."
                            .into(),
                    );
                }
                let _ = self.persist();
            }
        } else {
            let _ = self.persist();
        }
        self.state.lock().unwrap().running = false;
    }

    fn run_inner(
        self: &Arc<Self>,
        reader: &dyn RecognitionAudioReader,
        transport: Arc<dyn RecognitionFileTransport>,
    ) -> Result<(), PennelError> {
        let engine = self.engine.upgrade().ok_or(PennelError::NotFound)?;
        self.validate_source(&engine, reader)?;
        let ranges = self.state.lock().unwrap().manifest.ranges.clone();
        let first = self.state.lock().unwrap().manifest.confirmed.len();
        let session = if first < ranges.len() {
            if engine.settings().speaker_recognition != SpeakerRecognitionMode::Cloud {
                return Err(invalid(
                    "Explicitly choose ‘Through Pennel Cloud’ for speaker recognition first.",
                ));
            }
            Some(engine.cloud_session()?)
        } else {
            None
        };
        let mut token = match session.as_ref() {
            Some(session) => session.access_token()?,
            None => String::new(),
        };
        for (index, (offset, length)) in ranges.into_iter().enumerate().skip(first) {
            let session = session.as_ref().expect("pending upload has a session");
            if engine.settings().speaker_recognition != SpeakerRecognitionMode::Cloud {
                return Err(invalid("Cloud speaker recognition was turned off."));
            }
            self.validate_source(&engine, reader)?;
            let path = self.spool(reader, offset, length)?;
            self.check_cancelled()?;
            let request_id = format!("{}-{index}", self.id());
            *self.request.lock().unwrap() = Some(request_id.clone());
            let mut response = None;
            for attempt in 0..2 {
                self.check_cancelled()?;
                {
                    let mut state = self.state.lock().unwrap();
                    state.manifest.snapshot.uploaded_bytes = 0;
                    state.manifest.snapshot.upload_total_bytes = 44 + length * 2;
                }
                self.update(SpeakerRecognitionStage::Uploading)?;
                let sent = transport.upload(
                    RecognitionUploadRequest {
                        request_id: request_id.clone(),
                        url: format!("{}/v1/audio/diarize", session.base_url()),
                        bearer_token: token.clone(),
                        file_path: path.to_string_lossy().into_owned(),
                        timeout_seconds: 300,
                    },
                    Arc::new(UploadProgress {
                        job: Arc::downgrade(self),
                    }),
                );
                self.check_cancelled()?;
                match sent {
                    Err(error) => {
                        self.update(SpeakerRecognitionStage::ResponseUnknown)?;
                        return Err(error);
                    }
                    Ok(reply) if reply.status == 401 && attempt == 0 => {
                        self.update(SpeakerRecognitionStage::Preparing)?;
                        token = session.refreshed_token()?;
                    }
                    Ok(reply) => {
                        response = Some(reply);
                        break;
                    }
                }
            }
            let reply = response.ok_or(PennelError::NotSignedIn)?;
            let parsed = crate::cloud::diarize::parse_response(reply.status, reply.body.as_bytes());
            let diarization = match parsed {
                Ok(result) => result,
                Err(error) => {
                    if (200..300).contains(&reply.status)
                        || reply.status == 408
                        || reply.status >= 500
                    {
                        self.update(SpeakerRecognitionStage::ResponseUnknown)?;
                    }
                    return Err(error);
                }
            };
            self.check_cancelled()?;
            {
                let mut state = self.state.lock().unwrap();
                self.check_cancelled()?;
                state.manifest.confirmed.push(ConfirmedChunk {
                    offset_ms: offset * 1000 / u64::from(SAMPLE_RATE),
                    diarization,
                });
                state.manifest.snapshot.confirmed_chunks = state.manifest.confirmed.len() as u32;
                state.manifest.snapshot.stage = SpeakerRecognitionStage::Preparing;
                state.manifest.snapshot.uploaded_bytes = state.manifest.snapshot.upload_total_bytes;
            }
            self.persist()?;
            fs::remove_file(path).map_err(PennelError::storage)?;
            *self.request.lock().unwrap() = None;
            let _ = index;
        }
        let id = self.validate_source(&engine, reader)?;
        let _edit = engine.edit_meeting(&id)?;
        let _sync = engine.sync_gate.lock().unwrap();
        self.validate_source(&engine, reader)?;
        self.update(SpeakerRecognitionStage::Applying)?;
        let manifest = self.state.lock().unwrap().manifest.clone();
        let merged = merge_chunks(
            manifest
                .confirmed
                .into_iter()
                .map(|chunk| (chunk.offset_ms, chunk.diarization))
                .collect(),
            MERGE_THRESHOLD,
        );
        engine.apply_recognized_speakers_observed(
            &id,
            merged,
            if engine
                .meeting_store()
                .audio_tracks(&id)?
                .iter()
                .any(|track| {
                    track.source == crate::meeting_recording::RecordingSource::RoomMicrophone
                })
            {
                crate::diarize::Track::Microphone
            } else {
                crate::diarize::Track::System
            },
            None,
            Some(&self.directory),
            |segments, names| {
                let mut sorted = segments.to_vec();
                sorted.sort_by_key(|segment| segment.start_ms);
                {
                    let mut state = self.state.lock().unwrap();
                    state.manifest.applied_transcript_digest = Some(crate::models::hex(
                        &Sha256::digest(crate::meetings::render_transcript(&sorted).as_bytes()),
                    ));
                    state.manifest.applied_names_digest = Some(speaker_names_digest(names)?);
                }
                self.persist()
            },
        )?;
        {
            let mut state = self.state.lock().unwrap();
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Completed;
            state.manifest.snapshot.recoverable = false;
        }
        Ok(())
    }
}

#[uniffi::export]
impl SpeakerRecognitionJob {
    pub fn id(&self) -> String {
        self.state.lock().unwrap().manifest.snapshot.job_id.clone()
    }

    pub fn snapshot(&self) -> SpeakerRecognitionJobSnapshot {
        let mut snapshot = self.state.lock().unwrap().manifest.snapshot.clone();
        if let Some(engine) = self.engine.upgrade()
            && let Ok(id) = resolve_meeting(&engine.meeting_store(), &snapshot.meeting_uid)
        {
            snapshot.meeting_id = id;
        }
        snapshot
    }

    pub fn interrupt(&self) -> Result<(), PennelError> {
        {
            let mut state = self.state.lock().unwrap();
            if !state.running {
                return Ok(());
            }
            if state.manifest.snapshot.stage == SpeakerRecognitionStage::Applying {
                return Err(invalid(
                    "Speaker names are being saved. Wait until this finishes.",
                ));
            }
            self.interrupted.store(true, Ordering::SeqCst);
            state.manifest.snapshot.stage = restarted_stage(state.manifest.snapshot.stage);
        }
        self.persist()?;
        if let Some(request) = self.request.lock().unwrap().clone()
            && let Some(transport) = self.transport.lock().unwrap().clone()
        {
            transport.cancel(request);
        }
        Ok(())
    }

    pub fn cancel(&self) -> Result<(), PennelError> {
        let running = {
            let mut state = self.state.lock().unwrap();
            if state.manifest.snapshot.stage == SpeakerRecognitionStage::Applying {
                return Err(invalid(
                    "Speaker names are being saved. Wait until this finishes.",
                ));
            }
            if state.manifest.snapshot.stage == SpeakerRecognitionStage::Completed {
                return Ok(());
            }
            self.cancelled.store(true, Ordering::SeqCst);
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Cancelled;
            state.manifest.snapshot.recoverable = false;
            state.manifest.confirmed.clear();
            state.manifest.snapshot.confirmed_chunks = 0;
            state.running
        };
        // A process can disappear while native transport cancellation is in progress. Persist
        // the user's discard first so restart never presents it as resumable paid work.
        let persisted = self.persist();
        if let Some(request) = self.request.lock().unwrap().clone()
            && let Some(transport) = self.transport.lock().unwrap().clone()
        {
            transport.cancel(request);
        }
        if !running {
            match fs::remove_dir_all(&self.directory) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(PennelError::storage(error));
                }
                _ => {}
            }
        }
        persisted.or_else(|error| {
            if self.directory.exists() {
                Err(error)
            } else {
                Ok(())
            }
        })
    }
}

struct UploadProgress {
    job: Weak<SpeakerRecognitionJob>,
}

impl RecognitionUploadProgress for UploadProgress {
    fn sent(&self, sent_bytes: u64, total_bytes: u64) {
        if let Some(job) = self.job.upgrade() {
            if job.cancelled.load(Ordering::SeqCst) || job.interrupted.load(Ordering::SeqCst) {
                return;
            }
            let mut state = job.state.lock().unwrap();
            if !state.running
                || !matches!(
                    state.manifest.snapshot.stage,
                    SpeakerRecognitionStage::Uploading | SpeakerRecognitionStage::Recognizing
                )
            {
                return;
            }
            state.manifest.snapshot.uploaded_bytes = sent_bytes.min(total_bytes);
            state.manifest.snapshot.upload_total_bytes = total_bytes;
            if total_bytes > 0 && sent_bytes >= total_bytes {
                state.manifest.snapshot.stage = SpeakerRecognitionStage::Recognizing;
            }
        }
    }
}

impl PennelEngine {
    pub(crate) fn load_speaker_recognition_jobs(self: &Arc<Self>) -> Result<(), PennelError> {
        self.recover_speaker_commits()?;
        let directory = self.data_dir().join(JOBS);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(PennelError::storage(error)),
        };
        for entry in entries {
            let entry = entry.map_err(PennelError::storage)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if uuid::Uuid::parse_str(&name).is_err()
                || !entry.file_type().map_err(PennelError::storage)?.is_dir()
            {
                continue;
            }
            let directory = entry.path();
            let bytes = match fs::read(directory.join(MANIFEST)) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let parsed = serde_json::from_slice::<Manifest>(&bytes)
                .ok()
                .filter(|manifest| manifest.snapshot.job_id == name);
            let mut manifest = match parsed {
                Some(manifest) => manifest,
                None => {
                    let _ = fs::rename(
                        directory.join(MANIFEST),
                        directory.join(format!("job-invalid-{}.json", uuid::Uuid::now_v7())),
                    );
                    invalid_checkpoint(&name, &self.meeting_store().root())
                }
            };
            let _ = fs::remove_file(directory.join(SPOOL));
            if manifest.snapshot.stage == SpeakerRecognitionStage::Applying
                && commit_is_complete(self, &manifest, &directory)
            {
                manifest.snapshot.stage = SpeakerRecognitionStage::Completed;
            }
            manifest.snapshot.stage = restarted_stage(manifest.snapshot.stage);
            if matches!(
                manifest.snapshot.stage,
                SpeakerRecognitionStage::Cancelled | SpeakerRecognitionStage::Completed
            ) {
                fs::remove_dir_all(directory).map_err(PennelError::storage)?;
                continue;
            }
            let job = Arc::new(SpeakerRecognitionJob {
                state: Mutex::new(JobState {
                    manifest,
                    running: false,
                }),
                cancelled: AtomicBool::new(false),
                interrupted: AtomicBool::new(false),
                transport: Mutex::new(None),
                request: Mutex::new(None),
                directory,
                engine: Arc::downgrade(self),
            });
            if let Err(error) = job.persist() {
                let mut state = job.state.lock().unwrap();
                state.manifest.snapshot.stage = SpeakerRecognitionStage::Failed;
                state.manifest.snapshot.recoverable = false;
                state.manifest.snapshot.error_code = Some("storage".into());
                state.manifest.snapshot.error_message = Some(error.to_string());
            }
            self.speaker_jobs.lock().unwrap().insert(name, job);
        }
        Ok(())
    }

    pub(crate) fn active_speaker_jobs(&self) -> bool {
        self.speaker_jobs
            .lock()
            .unwrap()
            .values()
            .any(|job| job.state.lock().unwrap().running)
    }

    pub(crate) fn discard_speaker_recognition_data(&self) -> Result<(), PennelError> {
        let jobs: Vec<_> = self
            .speaker_jobs
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        for job in jobs {
            job.cancel()?;
        }
        for name in [JOBS, "speaker-commits"] {
            match fs::remove_dir_all(self.data_dir().join(name)) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(PennelError::storage(error));
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn discard_speaker_jobs_for(&self, uid: &str) -> Result<(), PennelError> {
        let jobs: Vec<_> = self
            .speaker_jobs
            .lock()
            .unwrap()
            .values()
            .filter(|job| job.state.lock().unwrap().manifest.snapshot.meeting_uid == uid)
            .cloned()
            .collect();
        for job in jobs {
            job.cancel()?;
        }
        Ok(())
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn start_speaker_recognition(
        self: Arc<Self>,
        id: String,
        reader: Arc<dyn RecognitionAudioReader>,
        transport: Arc<dyn RecognitionFileTransport>,
    ) -> Result<Arc<SpeakerRecognitionJob>, PennelError> {
        let _claim = JOB_CLAIM_GATE.lock().unwrap();
        self.claim_library()?;
        if self.settings().speaker_recognition != SpeakerRecognitionMode::Cloud {
            return Err(invalid(
                "Explicitly choose ‘Through Pennel Cloud’ for speaker recognition first.",
            ));
        }
        if !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        let store = self.meeting_store();
        let id = resolve_meeting(&store, &id)?;
        let _edit = self.edit_meeting(&id)?;
        let _sync = self.sync_gate.lock().unwrap();
        let uid = store.ensure_uid(&id)?;
        if self.speaker_jobs.lock().unwrap().values().any(|job| {
            let state = job.state.lock().unwrap();
            state.running && state.manifest.snapshot.meeting_uid == uid
        }) {
            return Err(invalid("This meeting is already being processed."));
        }
        if store.unparsed_transcript_lines(&id) > 0 {
            return Err(invalid(
                "The transcript contains lines that Pennel does not recognize.",
            ));
        }
        crate::meeting_utterances::assert_writable(&store.read_transcript_snapshot(&id)?)?;
        let info = reader.info()?;
        if info.total_frames == 0 || info.total_frames / (600 * u64::from(SAMPLE_RATE)) > 10_000 {
            return Err(invalid("The recording does not have a valid duration."));
        }
        let audio_digest = meeting_audio_digest(&store, &id)?;
        if audio_digest != info.audio_digest {
            return Err(invalid(STALE));
        }
        let job_id = uuid::Uuid::now_v7().to_string();
        let directory = self.data_dir().join(JOBS).join(&job_id);
        private_directory(directory.parent().expect("job root"))?;
        private_directory(&directory)?;
        let ranges = upload_ranges(info.total_frames);
        let snapshot = SpeakerRecognitionJobSnapshot {
            job_id: job_id.clone(),
            meeting_uid: uid,
            meeting_id: id.clone(),
            stage: SpeakerRecognitionStage::Preparing,
            decoded_frames: 0,
            total_frames: info.total_frames,
            uploaded_bytes: 0,
            upload_total_bytes: 0,
            confirmed_chunks: 0,
            total_chunks: ranges.len() as u32,
            recoverable: true,
            error_code: None,
            error_message: None,
        };
        let manifest = Manifest {
            version: VERSION,
            algorithm: ALGORITHM,
            model: EMBEDDING_ID.into(),
            library: store.root(),
            meeting_id: id,
            audio_digest,
            transcript_digest: store.transcript_fingerprint(&_edit.id)?,
            names_digest: speaker_names_digest(&store.speaker_names(&_edit.id))?,
            metadata_digest: store.read_transcript_snapshot(&_edit.id)?.metadata_digest,
            ranges,
            confirmed: Vec::new(),
            applied_transcript_digest: None,
            applied_names_digest: None,
            snapshot,
        };
        let job = Arc::new(SpeakerRecognitionJob {
            state: Mutex::new(JobState {
                manifest,
                running: true,
            }),
            cancelled: AtomicBool::new(false),
            interrupted: AtomicBool::new(false),
            transport: Mutex::new(Some(transport.clone())),
            request: Mutex::new(None),
            directory,
            engine: Arc::downgrade(&self),
        });
        job.persist()?;
        self.speaker_jobs
            .lock()
            .unwrap()
            .insert(job_id, job.clone());
        drop(_sync);
        drop(_edit);
        let worker = job.clone();
        let engine = self.clone();
        std::thread::spawn(move || {
            let _engine = engine;
            worker.run(reader, transport)
        });
        Ok(job)
    }

    pub fn speaker_recognition_jobs(&self) -> Vec<SpeakerRecognitionJobSnapshot> {
        let mut jobs: Vec<_> = self
            .speaker_jobs
            .lock()
            .unwrap()
            .values()
            .map(|job| job.snapshot())
            .collect();
        jobs.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        jobs
    }

    pub fn speaker_recognition_job(&self, job_id: String) -> Option<Arc<SpeakerRecognitionJob>> {
        self.speaker_jobs.lock().unwrap().get(&job_id).cloned()
    }

    pub fn speaker_recognition_meeting_id(&self, job_id: String) -> Result<String, PennelError> {
        let job = self
            .speaker_recognition_job(job_id)
            .ok_or(PennelError::NotFound)?;
        let uid = job
            .state
            .lock()
            .unwrap()
            .manifest
            .snapshot
            .meeting_uid
            .clone();
        resolve_meeting(&self.meeting_store(), &uid)
    }

    pub fn resume_speaker_recognition(
        self: Arc<Self>,
        job_id: String,
        reader: Arc<dyn RecognitionAudioReader>,
        transport: Arc<dyn RecognitionFileTransport>,
        confirm_uncertain_retry: bool,
    ) -> Result<Arc<SpeakerRecognitionJob>, PennelError> {
        let _claim = JOB_CLAIM_GATE.lock().unwrap();
        let job = self
            .speaker_recognition_job(job_id.clone())
            .ok_or(PennelError::NotFound)?;
        {
            let mut state = job.state.lock().unwrap();
            if !state.running
                && matches!(
                    state.manifest.snapshot.stage,
                    SpeakerRecognitionStage::Applying
                        | SpeakerRecognitionStage::Interrupted
                        | SpeakerRecognitionStage::Failed
                )
                && commit_is_complete(&self, &state.manifest, &job.directory)
            {
                state.manifest.snapshot.stage = SpeakerRecognitionStage::Completed;
                state.manifest.snapshot.recoverable = false;
                state.manifest.confirmed.clear();
                drop(state);
                let _ = fs::remove_dir_all(&job.directory);
                return Ok(job);
            }
        }
        let needs_upload = {
            let state = job.state.lock().unwrap();
            state.manifest.confirmed.len() < state.manifest.ranges.len()
        };
        if needs_upload && self.settings().speaker_recognition != SpeakerRecognitionMode::Cloud {
            return Err(invalid(
                "Explicitly choose ‘Through Pennel Cloud’ for speaker recognition first.",
            ));
        }
        if needs_upload && !self.cloud_signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        let uid = job
            .state
            .lock()
            .unwrap()
            .manifest
            .snapshot
            .meeting_uid
            .clone();
        if self
            .speaker_jobs
            .lock()
            .unwrap()
            .iter()
            .any(|(other_id, other)| {
                other_id != &job_id && {
                    let state = other.state.lock().unwrap();
                    state.running && state.manifest.snapshot.meeting_uid == uid
                }
            })
        {
            return Err(invalid("This meeting is already being processed."));
        }
        {
            let mut state = job.state.lock().unwrap();
            if state.running {
                return Err(invalid("This job is already running."));
            }
            if !state.manifest.snapshot.recoverable {
                return Err(invalid("Start speaker recognition again."));
            }
            if state.manifest.snapshot.stage == SpeakerRecognitionStage::ResponseUnknown
                && !confirm_uncertain_retry
            {
                return Err(PennelError::Cloud { code: "speaker_retry_confirmation".into(), message: "The server may already have processed this audio chunk. Retrying may charge your allowance for this chunk again.".into() });
            }
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Preparing;
            state.manifest.snapshot.error_code = None;
            state.manifest.snapshot.error_message = None;
            state.running = true;
            job.cancelled.store(false, Ordering::SeqCst);
            job.interrupted.store(false, Ordering::SeqCst);
        }
        *job.transport.lock().unwrap() = Some(transport.clone());
        if let Err(error) = job.persist() {
            job.state.lock().unwrap().running = false;
            *job.transport.lock().unwrap() = None;
            return Err(error);
        }
        let worker = job.clone();
        let engine = self.clone();
        std::thread::spawn(move || {
            let _engine = engine;
            worker.run(reader, transport)
        });
        Ok(job)
    }

    pub fn mobile_settings_json(&self) -> Result<String, PennelError> {
        serde_json::to_string(&self.settings()).map_err(PennelError::storage)
    }

    pub fn patch_mobile_settings(&self, json: String) -> Result<String, PennelError> {
        let patch: serde_json::Value =
            serde_json::from_str(&json).map_err(|_| invalid("The settings are invalid."))?;
        let object = patch
            .as_object()
            .ok_or_else(|| invalid("The settings must be an object."))?;
        self.update_settings_with(|current_settings| {
            let mut current =
                serde_json::to_value(current_settings).map_err(PennelError::storage)?;
            let current_object = current.as_object_mut().expect("settings object");
            for (key, value) in object {
                if key == "meetings_dir" || !current_object.contains_key(key) {
                    return Err(invalid("This setting cannot be changed here."));
                }
                current_object.insert(key.clone(), value.clone());
            }
            let settings: Settings = serde_json::from_value(current)
                .map_err(|_| invalid("The settings are invalid."))?;
            for key in ["dictation_stt", "meeting_stt", "llm"] {
                if object.contains_key(key) {
                    let choice = match key {
                        "dictation_stt" => &settings.dictation_stt,
                        "meeting_stt" => &settings.meeting_stt,
                        _ => &settings.llm,
                    };
                    if choice.provider_id.is_empty() && choice.model.is_empty() {
                        continue;
                    }
                    let (account, _) = settings
                        .resolve(choice)
                        .ok_or_else(|| invalid("The selected provider is missing."))?;
                    if account.kind.is_cli_agent() || account.kind == ProviderKind::Local {
                        return Err(invalid("This provider only works on a computer."));
                    }
                }
            }
            Ok(settings)
        })?;
        self.mobile_settings_json()
    }

    pub fn mobile_meeting_voices(&self, id: String) -> Vec<crate::speakers::MeetingVoice> {
        self.meeting_voices(id)
    }

    pub fn mobile_meeting_summary_outdated(&self, id: String) -> bool {
        self.meeting_summary_outdated(&id)
    }

    pub fn mobile_set_meeting_notes(
        &self,
        id: String,
        text: String,
        expected: Option<String>,
    ) -> Result<(), PennelError> {
        self.save_owned_meeting_notes(&id, &text, expected.as_deref(), None)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognition_chunks_never_make_a_tiny_tail_or_exceed_the_service_limit() {
        let minute = 60 * 16_000;
        assert_eq!(upload_ranges(0), Vec::<(u64, u64)>::new());
        assert_eq!(upload_ranges(10 * minute + 1), vec![(0, 10 * minute + 1)]);
        let ranges = upload_ranges(60 * minute);
        assert_eq!(ranges.len(), 6);
        assert!(ranges.iter().all(|(_, length)| *length <= 630 * 16_000));
        assert_eq!(
            ranges.iter().map(|(_, length)| length).sum::<u64>(),
            60 * minute
        );
    }

    #[test]
    fn an_in_flight_request_becomes_unknown_after_restart() {
        for stage in [
            SpeakerRecognitionStage::Uploading,
            SpeakerRecognitionStage::Recognizing,
        ] {
            assert_eq!(
                restarted_stage(stage),
                SpeakerRecognitionStage::ResponseUnknown
            );
        }
        assert_eq!(
            restarted_stage(SpeakerRecognitionStage::Preparing),
            SpeakerRecognitionStage::Interrupted
        );
    }

    use crate::cloud::SecretStore;
    use std::collections::VecDeque;
    use std::sync::Condvar;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    struct Reader {
        info: RecognitionAudioInfo,
        requests: Mutex<Vec<(u64, u32)>>,
        oversized: bool,
    }

    impl RecognitionAudioReader for Reader {
        fn info(&self) -> Result<RecognitionAudioInfo, PennelError> {
            Ok(self.info.clone())
        }
        fn read_frames(
            &self,
            offset: u64,
            max_frames: u32,
        ) -> Result<RecognitionAudioBlock, PennelError> {
            self.requests.lock().unwrap().push((offset, max_frames));
            let count = u64::from(max_frames).min(self.info.total_frames - offset) as usize;
            Ok(RecognitionAudioBlock {
                samples: vec![0.25; count + usize::from(self.oversized)],
                eof: offset + count as u64 == self.info.total_frames,
            })
        }
    }

    fn reply() -> RecognitionUploadResponse {
        RecognitionUploadResponse {
            status: 200,
            body: serde_json::json!({
                "model": EMBEDDING_ID,
                "turns": [{"start_ms":0,"end_ms":2000,"speaker":0}],
                "speakers": [{"speaker":0,"speech_ms":2000,"embedding":[1.0,0.0]}],
            })
            .to_string(),
        }
    }

    type OnUpload = Box<dyn FnMut() + Send>;
    struct Transport {
        replies: Mutex<VecDeque<Result<RecognitionUploadResponse, PennelError>>>,
        calls: AtomicUsize,
        paths: Mutex<Vec<String>>,
        on_upload: Mutex<Option<OnUpload>>,
    }

    impl Transport {
        fn new(replies: Vec<Result<RecognitionUploadResponse, PennelError>>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                calls: AtomicUsize::new(0),
                paths: Mutex::new(Vec::new()),
                on_upload: Mutex::new(None),
            })
        }
    }

    impl RecognitionFileTransport for Transport {
        fn upload(
            &self,
            request: RecognitionUploadRequest,
            progress: Arc<dyn RecognitionUploadProgress>,
        ) -> Result<RecognitionUploadResponse, PennelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.timeout_seconds, 300);
            assert_eq!(request.bearer_token, "access");
            assert!(request.url.ends_with("/v1/audio/diarize"));
            let size = fs::metadata(&request.file_path).unwrap().len();
            assert!(size <= 44 + 630 * 16000 * 2);
            let mut header = [0u8; 44];
            File::open(&request.file_path)
                .unwrap()
                .read_exact(&mut header)
                .unwrap();
            assert_eq!(&header[..4], b"RIFF");
            assert_eq!(
                u32::from_le_bytes(header[40..44].try_into().unwrap()) as u64,
                size - 44
            );
            self.paths.lock().unwrap().push(request.file_path);
            progress.sent(size, size);
            if let Some(action) = self.on_upload.lock().unwrap().as_mut() {
                action();
            }
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(reply()))
        }
        fn cancel(&self, _request_id: String) {}
    }

    struct Fixture {
        directory: tempfile::TempDir,
        server: mockito::ServerGuard,
        engine: Arc<PennelEngine>,
        id: String,
        reader: Arc<Reader>,
    }

    fn attach_session(engine: &PennelEngine, server: &mut mockito::ServerGuard) {
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        secrets.set(crate::cloud::REFRESH_ACCOUNT.into(), Some("refresh".into()));
        engine.set_secret_store(secrets.clone()).unwrap();
        server
            .mock("POST", "/v1/auth/refresh")
            .with_body(r#"{"access_token":"access","refresh_token":"rotated","expires_in":900}"#)
            .create();
        engine.use_cloud_session(crate::cloud::account::Session::new(server.url(), secrets));
    }

    fn fixture(frames: u64) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.meetings_dir = directory
            .path()
            .join("meetings")
            .to_string_lossy()
            .into_owned();
        settings.speaker_recognition = SpeakerRecognitionMode::Cloud;
        engine.update_settings(settings).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Fixture", chrono::Local::now()).unwrap();
        store
            .write_transcript_snapshot(
                &id,
                None,
                &[crate::meeting_utterances::MeetingUtterance::new(
                    crate::Segment {
                        start_ms: 0,
                        end_ms: 2000,
                        speaker: crate::Speaker::Others,
                        text: "A fixture sentence.".into(),
                        failed: false,
                        voice: None,
                    },
                    Some("room_microphone".into()),
                )],
            )
            .unwrap();
        store.write_notes(&id, "Original personal note.").unwrap();
        let audio = store.dir_of(&id).join("audio/room.m4a");
        fs::create_dir_all(audio.parent().unwrap()).unwrap();
        fs::write(&audio, b"unit-test archive identity; decoding is injected").unwrap();
        let reader = Arc::new(Reader {
            info: RecognitionAudioInfo {
                total_frames: frames,
                audio_digest: digest_file(&audio).unwrap(),
            },
            requests: Mutex::new(Vec::new()),
            oversized: false,
        });
        let mut server = mockito::Server::new();
        attach_session(&engine, &mut server);
        Fixture {
            directory,
            server,
            engine,
            id,
            reader,
        }
    }

    fn settled(job: &SpeakerRecognitionJob) -> SpeakerRecognitionJobSnapshot {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if !job.state.lock().unwrap().running {
                return job.snapshot();
            }
            assert!(
                Instant::now() < deadline,
                "job did not stop: {:?}",
                job.snapshot()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn unsupported_or_corrupt_metadata_refuses_before_audio_upload() {
        for bytes in [
            br#"{"schema_version":9000}"#.as_slice(),
            b"invalid metadata".as_slice(),
        ] {
            let f = fixture(16_000);
            let store = f.engine.meeting_store();
            fs::write(store.dir_of(&f.id).join("transcript.meta.json"), bytes).unwrap();
            let transport = Transport::new(Vec::new());
            assert!(
                f.engine
                    .clone()
                    .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
                    .is_err()
            );
            assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
            assert_eq!(
                f.engine.meeting_transcript(f.id.clone())[0].text,
                "A fixture sentence."
            );
            assert_eq!(
                f.engine.meeting_notes(f.id.clone()),
                "Original personal note."
            );
        }
    }

    #[test]
    fn shared_room_recognition_is_bounded_and_needs_no_enrollment_or_llm() {
        let f = fixture(660 * 16000);
        let transport = Transport::new(Vec::new());
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        assert!(
            f.reader
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|(_, maximum)| *maximum <= MAX_FRAMES)
        );
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, Some(1));
        assert_eq!(
            f.engine.meeting_notes(f.id.clone()),
            "Original personal note."
        );
        assert!(f.engine.known_voices().is_empty());
        assert!(f.engine.voice_profile().is_none());
        assert!(!job.directory.exists());
        assert!(!f.engine.meeting_voices(f.id.clone())[0].remembered);
        assert!(
            transport
                .paths
                .lock()
                .unwrap()
                .iter()
                .all(|path| !Path::new(path).starts_with(f.engine.meeting_store().root()))
        );
    }

    #[test]
    fn naming_and_rerunning_does_not_enroll_a_voice() {
        let f = fixture(16000);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), Transport::new(Vec::new()))
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        f.engine
            .rename_speaker(f.id.clone(), 1, "Person A".into())
            .unwrap();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), Transport::new(Vec::new()))
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        assert_eq!(f.engine.meeting_speakers(f.id.clone())[0].name, "Person A");
        assert!(f.engine.known_voices().is_empty());
    }

    #[test]
    fn lost_response_requires_confirmation_and_resume_skips_confirmed_chunks() {
        let mut f = fixture(660 * 16000);
        let transport = Transport::new(vec![
            Ok(reply()),
            Err(PennelError::Network {
                message: "connection closed".into(),
            }),
        ]);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport)
            .unwrap();
        let state = settled(&job);
        assert_eq!(state.stage, SpeakerRecognitionStage::ResponseUnknown);
        assert_eq!(state.confirmed_chunks, 1);
        let job_id = job.id();
        drop(job);
        let path = f.directory.path().to_string_lossy().into_owned();
        drop(f.engine);
        f.engine = PennelEngine::new(path).unwrap();
        attach_session(&f.engine, &mut f.server);
        assert_eq!(
            f.engine.speaker_recognition_jobs()[0].stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
        let transport = Transport::new(Vec::new());
        assert!(
            matches!(f.engine.clone().resume_speaker_recognition(job_id.clone(),f.reader.clone(),transport.clone(),false),Err(PennelError::Cloud {code,..}) if code=="speaker_retry_confirmation")
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        f.reader.requests.lock().unwrap().clear();
        let job = f
            .engine
            .clone()
            .resume_speaker_recognition(job_id, f.reader.clone(), transport.clone(), true)
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.reader.requests.lock().unwrap()[0].0, 600 * 16000);
    }

    #[test]
    fn changed_manual_metadata_cannot_be_overwritten_when_text_is_unchanged() {
        let f = fixture(16_000);
        let transport = Transport::new(Vec::new());
        let engine = f.engine.clone();
        let id = f.id.clone();
        *transport.on_upload.lock().unwrap() = Some(Box::new(move || {
            let store = engine.meeting_store();
            let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
            let row = &mut snapshot.utterances[0];
            row.metadata.manual = Some(crate::meeting_utterances::ManualCorrection {
                self_voice: false,
                name: Some("User correction".into()),
                voice: Some(4),
                source_fingerprint: row.metadata.source_fingerprint.clone(),
                review_required: false,
            });
            store
                .write_transcript_snapshot(&id, Some(snapshot.revision), &snapshot.utterances)
                .unwrap();
        }));
        let original_text = f.engine.meeting_transcript(f.id.clone())[0].text.clone();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport)
            .unwrap();
        let state = settled(&job);
        assert_eq!(state.stage, SpeakerRecognitionStage::Failed);
        assert_eq!(state.error_code.as_deref(), Some("speaker_stale"));
        assert_eq!(
            f.engine.meeting_transcript(f.id.clone())[0].text,
            original_text
        );
        let snapshot = f
            .engine
            .meeting_store()
            .read_transcript_snapshot(&f.id)
            .unwrap();
        assert_eq!(
            snapshot.utterances[0]
                .metadata
                .manual
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("User correction")
        );
    }

    #[test]
    fn changed_transcript_cannot_be_overwritten_by_a_late_result() {
        let f = fixture(16000);
        let transport = Transport::new(Vec::new());
        let engine = f.engine.clone();
        let id = f.id.clone();
        *transport.on_upload.lock().unwrap() = Some(Box::new(move || {
            let mut transcript = engine.meeting_transcript(id.clone());
            transcript[0].text = "Edited while recognizing.".into();
            engine
                .meeting_store()
                .write_transcript(&id, &transcript)
                .unwrap();
        }));
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport)
            .unwrap();
        let state = settled(&job);
        assert_eq!(state.stage, SpeakerRecognitionStage::Failed);
        assert_eq!(state.error_code.as_deref(), Some("speaker_stale"));
        assert!(!state.recoverable);
        assert_eq!(
            f.engine.meeting_transcript(f.id.clone())[0].text,
            "Edited while recognizing."
        );
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, None);
    }

    #[test]
    fn renamed_meeting_resolves_by_uid_and_personal_notes_are_not_stale() {
        let f = fixture(16000);
        let transport = Transport::new(Vec::new());
        let engine = f.engine.clone();
        let id = f.id.clone();
        let new_id = Arc::new(Mutex::new(String::new()));
        let changed = new_id.clone();
        *transport.on_upload.lock().unwrap() = Some(Box::new(move || {
            engine
                .set_meeting_notes(id.clone(), "New personal note.".into())
                .unwrap();
            *changed.lock().unwrap() = engine
                .rename_meeting(id.clone(), "Renamed fixture".into())
                .unwrap();
        }));
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport)
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        let id = new_id.lock().unwrap().clone();
        assert_eq!(
            f.engine.speaker_recognition_meeting_id(job.id()).unwrap(),
            id
        );
        assert_eq!(f.engine.meeting_notes(id), "New personal note.");
    }

    #[test]
    fn decoder_overflow_is_rejected_before_upload_and_does_not_modify_text() {
        let f = fixture(16000);
        let reader = Arc::new(Reader {
            info: f.reader.info.clone(),
            requests: Mutex::new(Vec::new()),
            oversized: true,
        });
        let transport = Transport::new(Vec::new());
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), reader, transport.clone())
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Failed);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, None);
        assert!(!job.directory.join(SPOOL).exists());
    }

    #[test]
    fn mobile_settings_can_clear_each_task_without_accepting_incomplete_choices() {
        let f = fixture(16000);
        for task in ["dictation_stt", "meeting_stt", "llm"] {
            let cleared = serde_json::json!({task: {"provider_id": "", "model": ""}});
            f.engine.patch_mobile_settings(cleared.to_string()).unwrap();
            let current: serde_json::Value =
                serde_json::from_str(&f.engine.mobile_settings_json().unwrap()).unwrap();
            assert_eq!(current[task], cleared[task]);
            let loaded = Settings::load_strict(&f.engine.data_dir().join("settings.json")).unwrap();
            assert_eq!(serde_json::to_value(loaded).unwrap()[task], cleared[task]);
            let incomplete =
                serde_json::json!({task: {"provider_id": "", "model": "orphan-model"}});
            assert!(
                f.engine
                    .patch_mobile_settings(incomplete.to_string())
                    .is_err()
            );
            let after: serde_json::Value =
                serde_json::from_str(&f.engine.mobile_settings_json().unwrap()).unwrap();
            assert_eq!(after[task], cleared[task]);
        }
    }

    #[test]
    fn mobile_live_text_preference_survives_unrelated_patch_and_persisted_reload() {
        let f = fixture(16000);
        f.engine
            .patch_mobile_settings(r#"{"live_text_enabled":false}"#.into())
            .unwrap();
        f.engine
            .patch_mobile_settings(r#"{"dictionary":["caption preference"]}"#.into())
            .unwrap();
        let current: serde_json::Value =
            serde_json::from_str(&f.engine.mobile_settings_json().unwrap()).unwrap();
        assert_eq!(current["live_text_enabled"], false);
        let loaded = Settings::load_strict(&f.engine.data_dir().join("settings.json")).unwrap();
        assert_eq!(
            serde_json::to_value(loaded).unwrap()["live_text_enabled"],
            false
        );
        assert!(
            f.engine
                .patch_mobile_settings(r#"{"live_text_enabled":"false"}"#.into())
                .is_err()
        );
        f.engine
            .patch_mobile_settings(r#"{"live_text_enabled":true}"#.into())
            .unwrap();
        let enabled: serde_json::Value =
            serde_json::from_str(&f.engine.mobile_settings_json().unwrap()).unwrap();
        assert_eq!(enabled["live_text_enabled"], true);
    }

    #[test]
    fn mobile_settings_patch_preserves_fields_and_rejects_unknown_path_or_cli_selection() {
        let f = fixture(16000);
        f.engine
            .patch_mobile_settings(r#"{"dictionary":["example"]}"#.into())
            .unwrap();
        assert_eq!(f.engine.settings().dictionary, vec!["example"]);
        assert_eq!(
            f.engine.settings().speaker_recognition,
            SpeakerRecognitionMode::Cloud
        );
        for patch in [
            r#"[]"#,
            r#"{"unknown":1}"#,
            r#"{"meetings_dir":"/tmp/other"}"#,
            r#"{"dictionary":false}"#,
        ] {
            assert!(f.engine.patch_mobile_settings(patch.into()).is_err());
        }
        let mut settings = f.engine.settings();
        let cli = crate::new_account(ProviderKind::Codex, settings.providers.clone());
        settings.providers.push(cli.clone());
        f.engine.update_settings(settings).unwrap();
        let patch = serde_json::json!({"llm":{"provider_id":cli.id,"model":"codex"}});
        assert!(f.engine.patch_mobile_settings(patch.to_string()).is_err());
        assert!(!f.engine.mobile_settings_json().unwrap().contains("refresh"));
    }

    struct BlockingTransport {
        state: Mutex<(bool, bool)>,
        signal: Condvar,
        file_path: Mutex<Option<PathBuf>>,
        cancel_manifest: Mutex<Option<Vec<u8>>>,
    }
    impl BlockingTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new((false, false)),
                signal: Condvar::new(),
                file_path: Mutex::new(None),
                cancel_manifest: Mutex::new(None),
            })
        }
        fn entered(&self) {
            let (state, _) = self
                .signal
                .wait_timeout_while(
                    self.state.lock().unwrap(),
                    Duration::from_secs(10),
                    |state| !state.0,
                )
                .unwrap();
            assert!(state.0);
        }
    }
    impl RecognitionFileTransport for BlockingTransport {
        fn upload(
            &self,
            request: RecognitionUploadRequest,
            progress: Arc<dyn RecognitionUploadProgress>,
        ) -> Result<RecognitionUploadResponse, PennelError> {
            *self.file_path.lock().unwrap() = Some(PathBuf::from(request.file_path));
            progress.sent(1, 2);
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.signal.notify_all();
            while !state.1 {
                state = self.signal.wait(state).unwrap();
            }
            Ok(reply())
        }
        fn cancel(&self, _id: String) {
            if let Some(path) = self.file_path.lock().unwrap().as_ref() {
                *self.cancel_manifest.lock().unwrap() =
                    fs::read(path.parent().unwrap().join(MANIFEST)).ok();
            }
            self.state.lock().unwrap().1 = true;
            self.signal.notify_all();
        }
    }

    #[test]
    fn user_cancel_discards_checkpoints_and_prevents_late_commit() {
        let f = fixture(16000);
        let transport = BlockingTransport::new();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        job.cancel().unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Cancelled);
        assert!(!job.directory.exists());
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, None);
    }

    #[test]
    fn os_interrupt_retains_an_unknown_request_for_explicit_recovery() {
        let f = fixture(16000);
        let transport = BlockingTransport::new();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        job.interrupt().unwrap();
        assert_eq!(
            settled(&job).stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
        assert!(job.directory.join(MANIFEST).exists());
        assert!(!job.directory.join(SPOOL).exists());
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, None);
        let result = f.engine.clone().resume_speaker_recognition(
            job.id(),
            f.reader.clone(),
            Transport::new(Vec::new()),
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn quota_failure_is_actionable_and_not_retried_automatically() {
        let f = fixture(16000);
        let transport = Transport::new(vec![Ok(RecognitionUploadResponse {
            status: 429,
            body: r#"{"error":"quota","message":"Monthly quota reached."}"#.into(),
        })]);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        let snapshot = settled(&job);
        assert_eq!(snapshot.stage, SpeakerRecognitionStage::Failed);
        assert_eq!(snapshot.error_code.as_deref(), Some("quota"));
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, None);
    }

    #[test]
    fn checkpoint_model_change_refuses_to_merge_or_upload() {
        let f = fixture(16000);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![Err(PennelError::Network {
                    message: "lost response".into(),
                })]),
            )
            .unwrap();
        assert_eq!(
            settled(&job).stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
        job.state.lock().unwrap().manifest.model = "incompatible-model".into();
        job.persist().unwrap();
        let transport = Transport::new(Vec::new());
        let resumed = f
            .engine
            .clone()
            .resume_speaker_recognition(job.id(), f.reader.clone(), transport.clone(), true)
            .unwrap();
        let state = settled(&resumed);
        assert_eq!(state.stage, SpeakerRecognitionStage::Failed);
        assert_eq!(
            state.error_code.as_deref(),
            Some("speaker_checkpoint_version")
        );
        assert!(!state.recoverable);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn forgetting_voices_discards_derived_checkpoints_too() {
        let f = fixture(660 * 16000);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![
                    Ok(reply()),
                    Err(PennelError::Network {
                        message: "lost response".into(),
                    }),
                ]),
            )
            .unwrap();
        assert_eq!(settled(&job).confirmed_chunks, 1);
        assert!(job.directory.join(MANIFEST).exists());
        f.engine.forget_all_voices().unwrap();
        assert_eq!(job.snapshot().stage, SpeakerRecognitionStage::Cancelled);
        assert!(!f.directory.path().join(JOBS).exists());
        assert!(!f.directory.path().join("speaker-commits").exists());
    }

    #[test]
    fn app_container_relocation_preserves_resumable_jobs() {
        let mut f = fixture(16000);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![Err(PennelError::Network {
                    message: "lost response".into(),
                })]),
            )
            .unwrap();
        assert_eq!(
            settled(&job).stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
        let job_id = job.id();
        drop(job);
        drop(f.engine);
        let relocated = tempfile::tempdir().unwrap();
        let root = relocated.path().join("new-container");
        fs::rename(f.directory.path(), &root).unwrap();
        f.engine = PennelEngine::new(root.to_string_lossy().into_owned()).unwrap();
        let mut settings = f.engine.settings();
        settings.meetings_dir = root.join("meetings").to_string_lossy().into_owned();
        f.engine.update_settings(settings).unwrap();
        attach_session(&f.engine, &mut f.server);
        let job = f
            .engine
            .clone()
            .resume_speaker_recognition(job_id, f.reader.clone(), Transport::new(Vec::new()), true)
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
    }

    #[test]
    fn corrupt_optional_checkpoint_does_not_block_opening_the_app() {
        let f = fixture(16000);
        let job_id = uuid::Uuid::now_v7().to_string();
        let directory = f.directory.path().join(JOBS).join(&job_id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(MANIFEST), b"not-json").unwrap();
        drop(f.engine);
        let engine = PennelEngine::new(f.directory.path().to_string_lossy().into_owned()).unwrap();
        let jobs = engine.speaker_recognition_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].stage, SpeakerRecognitionStage::Failed);
        assert!(!jobs[0].recoverable);
        assert_eq!(
            jobs[0].error_code.as_deref(),
            Some("speaker_checkpoint_invalid")
        );
        assert_eq!(
            engine.meeting_transcript(f.id)[0].text,
            "A fixture sentence."
        );
        engine
            .speaker_recognition_job(job_id)
            .unwrap()
            .cancel()
            .unwrap();
        assert!(!directory.exists());
    }

    #[test]
    fn gateway_timeout_is_unknown_and_cannot_be_silently_replayed() {
        let f = fixture(16000);
        let transport = Transport::new(vec![Ok(RecognitionUploadResponse {
            status: 504,
            body: "gateway timeout".into(),
        })]);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        assert_eq!(
            settled(&job).stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert!(
            f.engine
                .clone()
                .resume_speaker_recognition(job.id(), f.reader.clone(), transport.clone(), false)
                .is_err()
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_expired_upload_token_refreshes_once_without_redecoding() {
        let f = fixture(16000);
        let transport = Transport::new(vec![
            Ok(RecognitionUploadResponse {
                status: 401,
                body: r#"{"error":"expired"}"#.into(),
            }),
            Ok(reply()),
        ]);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        assert_eq!(f.reader.requests.lock().unwrap().len(), 1);
        let paths = transport.paths.lock().unwrap().clone();
        assert_eq!(paths[0], paths[1]);
    }

    #[test]
    fn cancelled_checkpoint_is_durable_before_native_cancel_and_removed_on_restart() {
        let f = fixture(16000);
        let transport = BlockingTransport::new();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        job.cancel().unwrap();
        let bytes = transport.cancel_manifest.lock().unwrap().clone().unwrap();
        let manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(manifest.snapshot.stage, SpeakerRecognitionStage::Cancelled);
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Cancelled);
        let restart = tempfile::tempdir().unwrap();
        let directory = restart.path().join(JOBS).join(job.id());
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(MANIFEST), bytes).unwrap();
        let engine = PennelEngine::new(restart.path().to_string_lossy().into_owned()).unwrap();
        assert!(engine.speaker_recognition_jobs().is_empty());
        assert!(!directory.exists());
    }

    #[test]
    fn renamed_active_meeting_cannot_start_a_second_job() {
        let f = fixture(16000);
        let transport = BlockingTransport::new();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        let renamed = f
            .engine
            .rename_meeting(f.id.clone(), "Renamed during upload".into())
            .unwrap();
        let second = Transport::new(Vec::new());
        let result =
            f.engine
                .clone()
                .start_speaker_recognition(renamed, f.reader.clone(), second.clone());
        job.cancel().unwrap();
        settled(&job);
        assert!(result.is_err());
        assert_eq!(second.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn old_job_cannot_resume_while_a_new_job_for_its_uid_runs() {
        let f = fixture(16000);
        let old = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![Err(PennelError::Network {
                    message: "lost reply".into(),
                })]),
            )
            .unwrap();
        assert_eq!(
            settled(&old).stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
        let transport = BlockingTransport::new();
        let current = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        let retry = Transport::new(Vec::new());
        let result = f.engine.clone().resume_speaker_recognition(
            old.id(),
            f.reader.clone(),
            retry.clone(),
            true,
        );
        current.cancel().unwrap();
        settled(&current);
        assert!(result.is_err());
        assert_eq!(retry.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            old.snapshot().stage,
            SpeakerRecognitionStage::ResponseUnknown
        );
    }

    #[test]
    fn recognition_preserves_live_failed_audio_for_transcription_retry() {
        let f = fixture(16000);
        let failed = f
            .engine
            .meeting_store()
            .dir_of(&f.id)
            .join("failed")
            .join("fixture.wav");
        fs::create_dir_all(failed.parent().unwrap()).unwrap();
        fs::write(&failed, b"saved retry audio").unwrap();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), Transport::new(Vec::new()))
            .unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Completed);
        assert_eq!(fs::read(failed).unwrap(), b"saved retry audio");
    }

    #[test]
    fn mobile_notes_compare_and_set_preserves_a_newer_edit() {
        let f = fixture(16000);
        f.engine
            .mobile_set_meeting_notes(
                f.id.clone(),
                "Changed note.".into(),
                Some("Original personal note.".into()),
            )
            .unwrap();
        assert!(
            f.engine
                .mobile_set_meeting_notes(
                    f.id.clone(),
                    "Stale edit.".into(),
                    Some("Original personal note.".into())
                )
                .is_err()
        );
        assert_eq!(f.engine.meeting_notes(f.id), "Changed note.");
    }

    #[test]
    fn cloud_identity_changes_are_refused_while_a_native_job_runs() {
        let f = fixture(16000);
        let transport = BlockingTransport::new();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        assert!(f.engine.cloud_logout().is_err());
        assert!(
            f.engine
                .cloud_login_start("Other account".into(), "ios".into())
                .is_err()
        );
        assert!(f.engine.cloud_login_poll("other-code".into()).is_err());
        assert!(f.engine.cloud_delete_account().is_err());
        assert!(
            f.engine
                .set_secret_store(Arc::new(crate::cloud::tests::MemorySecrets::default()))
                .is_err()
        );
        assert!(f.engine.cloud_signed_in());
        job.cancel().unwrap();
        settled(&job);
    }

    #[test]
    fn deleting_a_meeting_cancels_its_job_and_discards_processing_data() {
        let f = fixture(16000);
        let transport = BlockingTransport::new();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(f.id.clone(), f.reader.clone(), transport.clone())
            .unwrap();
        transport.entered();
        f.engine.delete_meeting(f.id.clone()).unwrap();
        assert_eq!(settled(&job).stage, SpeakerRecognitionStage::Cancelled);
        assert!(!job.directory.exists());
        assert!(f.engine.meeting_info(f.id).is_none());
        assert!(job.state.lock().unwrap().manifest.confirmed.is_empty());
    }

    #[test]
    fn successful_equal_transcript_commit_is_reconciled_after_crash_before_job_cleanup() {
        let f = fixture(16000);
        let store = f.engine.meeting_store();
        let mut segments = store.read_transcript(&f.id);
        segments[0].voice = Some(1);
        store.write_transcript(&f.id, &segments).unwrap();
        store
            .set_speaker_names(
                &f.id,
                &[SpeakerName {
                    voice: 1,
                    name: "Person A".into(),
                }],
            )
            .unwrap();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![Err(PennelError::Network {
                    message: "lost reply".into(),
                })]),
            )
            .unwrap();
        settled(&job);
        let names = vec![SpeakerName {
            voice: 1,
            name: "Person B".into(),
        }];
        {
            let mut state = job.state.lock().unwrap();
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Applying;
            state.manifest.applied_transcript_digest =
                Some(store.transcript_fingerprint(&f.id).unwrap());
            state.manifest.applied_names_digest = Some(speaker_names_digest(&names).unwrap());
        }
        job.persist().unwrap();
        let prints = vec![crate::voices::MeetingVoiceprint {
            voice: 1,
            embedding: vec![0.0, 1.0],
            known_id: None,
        }];
        f.engine
            .commit_speaker_result(
                &f.id,
                &segments,
                &names,
                prints.clone(),
                Some(&job.directory),
            )
            .unwrap();
        let directory = job.directory.clone();
        let uid = job.snapshot().meeting_uid;
        drop(job);
        drop(f.engine);
        let reopened =
            PennelEngine::new(f.directory.path().to_string_lossy().into_owned()).unwrap();
        assert!(reopened.speaker_recognition_jobs().is_empty());
        assert!(!directory.exists());
        assert_eq!(reopened.meeting_speakers(f.id.clone()), names);
        assert_eq!(reopened.meeting_transcript(f.id.clone()), segments);
        assert_eq!(
            reopened
                .voice_store()
                .meeting_voiceprints(&uid, Some(&store.transcript_fingerprint(&f.id).unwrap())),
            prints
        );
    }

    #[test]
    fn rolled_back_equal_transcript_job_is_not_mistaken_for_a_completed_commit() {
        let f = fixture(16000);
        let store = f.engine.meeting_store();
        let mut segments = store.read_transcript(&f.id);
        segments[0].voice = Some(1);
        let old_names = vec![SpeakerName {
            voice: 1,
            name: "Person A".into(),
        }];
        store.write_transcript(&f.id, &segments).unwrap();
        store.set_speaker_names(&f.id, &old_names).unwrap();
        let uid = store.ensure_uid(&f.id).unwrap();
        let hash = store.transcript_fingerprint(&f.id).unwrap();
        let old_prints = vec![crate::voices::MeetingVoiceprint {
            voice: 1,
            embedding: vec![1.0, 0.0],
            known_id: None,
        }];
        f.engine
            .voice_store()
            .set_meeting_voiceprints(&uid, old_prints.clone(), Some(&hash))
            .unwrap();
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![Err(PennelError::Network {
                    message: "lost reply".into(),
                })]),
            )
            .unwrap();
        settled(&job);
        let names = vec![SpeakerName {
            voice: 1,
            name: "Person B".into(),
        }];
        {
            let mut state = job.state.lock().unwrap();
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Applying;
            state.manifest.applied_transcript_digest = Some(hash.clone());
            state.manifest.applied_names_digest = Some(speaker_names_digest(&names).unwrap());
        }
        job.persist().unwrap();
        let backup = store
            .replace_speaker_labels(&f.id, &segments, &names, &[crate::Speaker::Others])
            .unwrap();
        let backup_id = backup.file_name().unwrap().to_string_lossy().into_owned();
        let commits = f.directory.path().join("speaker-commits");
        fs::create_dir_all(&commits).unwrap();
        fs::write(commits.join(format!("{}.json",uuid::Uuid::now_v7())), serde_json::json!({"version":1,"meeting_uid":uid,"old_hash":hash,"new_hash":hash,"old_prints":old_prints,"backup":backup_id}).to_string()).unwrap();
        f.engine
            .voice_store()
            .set_meeting_voiceprints(
                &uid,
                vec![crate::voices::MeetingVoiceprint {
                    voice: 1,
                    embedding: vec![0.0, 1.0],
                    known_id: None,
                }],
                Some(&hash),
            )
            .unwrap();
        fs::write(job.directory.join("commit-receipt.json"), serde_json::json!({"meeting_uid":uid,"backup":backup_id,"transcript_digest":hash,"names_digest":speaker_names_digest(&names).unwrap()}).to_string()).unwrap();
        let job_id = job.id();
        drop(job);
        drop(f.engine);
        let reopened =
            PennelEngine::new(f.directory.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            reopened
                .speaker_recognition_job(job_id)
                .unwrap()
                .snapshot()
                .stage,
            SpeakerRecognitionStage::Interrupted
        );
        assert_eq!(reopened.meeting_speakers(f.id), old_names);
        assert_eq!(
            reopened
                .voice_store()
                .meeting_voiceprints(&uid, Some(&hash)),
            old_prints
        );
    }

    #[test]
    fn fully_confirmed_work_can_finalize_without_network_or_a_cloud_login() {
        let f = fixture(16000);
        let job = f
            .engine
            .clone()
            .start_speaker_recognition(
                f.id.clone(),
                f.reader.clone(),
                Transport::new(vec![Err(PennelError::Network {
                    message: "lost response".into(),
                })]),
            )
            .unwrap();
        settled(&job);
        {
            let mut state = job.state.lock().unwrap();
            state.manifest.confirmed = vec![ConfirmedChunk {
                offset_ms: 0,
                diarization: crate::cloud::diarize::parse_response(200, reply().body.as_bytes())
                    .unwrap(),
            }];
            state.manifest.snapshot.confirmed_chunks = 1;
            state.manifest.snapshot.stage = SpeakerRecognitionStage::Interrupted;
        }
        job.persist().unwrap();
        f.engine
            .set_secret_store(Arc::new(crate::cloud::tests::MemorySecrets::default()))
            .unwrap();
        let mut settings = f.engine.settings();
        settings.speaker_recognition = SpeakerRecognitionMode::Local;
        f.engine.update_settings(settings).unwrap();
        let transport = Transport::new(Vec::new());
        let resumed = f
            .engine
            .clone()
            .resume_speaker_recognition(job.id(), f.reader.clone(), transport.clone(), false)
            .unwrap();
        assert_eq!(settled(&resumed).stage, SpeakerRecognitionStage::Completed);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.engine.meeting_transcript(f.id.clone())[0].voice, Some(1));
    }

    #[test]
    fn mobile_patch_preserves_an_internal_cloud_setting_committed_concurrently() {
        let f = fixture(16000);
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let engine = f.engine.clone();
        let cloud = std::thread::spawn(move || {
            engine.update_settings_with(|mut settings| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                settings.cloud_sync = true;
                Ok(settings)
            })
        });
        entered_rx.recv().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let engine = f.engine.clone();
        let mobile = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            engine.patch_mobile_settings(r#"{"dictionary":["Concurrent fixture"]}"#.into())
        });
        started_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        release_tx.send(()).unwrap();
        cloud.join().unwrap().unwrap();
        mobile.join().unwrap().unwrap();
        assert!(f.engine.settings().cloud_sync);
        assert_eq!(f.engine.settings().dictionary, vec!["Concurrent fixture"]);
    }
}
