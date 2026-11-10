use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::PennelEngine;
use crate::error::PennelError;
use crate::meetings::{MeetingStore, Speaker, durable_write};
use crate::provider::ProviderKind;
use crate::session::MeetingRecognition;

pub(crate) const RECORDING_FILE: &str = ".pennel/recording.json";
const EVENTS_FILE: &str = ".pennel/capture-events.jsonl";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_EVENT_BYTES: u64 = 4 * 1024 * 1024;
static RECORDING_GATE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum RecordingSource {
    #[default]
    Microphone,
    SystemAudio,
    RoomMicrophone,
}

impl RecordingSource {
    fn speaker(self) -> Speaker {
        match self {
            Self::Microphone => Speaker::Me,
            Self::SystemAudio | Self::RoomMicrophone => Speaker::Others,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Microphone => "audio/microphone.m4a",
            Self::SystemAudio => "audio/system.m4a",
            Self::RoomMicrophone => "audio/room.m4a",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum RecordingTransport {
    BuiltIn,
    Bluetooth,
    Usb,
    Wired,
    Display,
    Virtual,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum CaptureEventKind {
    Started,
    FormatChanged,
    DeviceChanged,
    Interruption,
    SilenceInserted,
    RateMismatch,
    Restarted,
    CaptureError,
    Stopped,
    #[default]
    Metrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(default)]
pub struct MeetingCaptureEvent {
    pub sequence: u64,
    pub elapsed_ms: u64,
    pub kind: CaptureEventKind,
    pub source: RecordingSource,
    pub input_sample_rate: Option<u32>,
    pub tap_sample_rate: Option<u32>,
    pub aggregate_sample_rate: Option<u32>,
    pub input_channels: Option<u32>,
    pub input_interleaved: Option<bool>,
    pub transport: Option<RecordingTransport>,
    pub captured_frames: Option<u64>,
    pub output_samples: Option<u64>,
    pub inserted_silence_samples: Option<u64>,
    pub gap_count: Option<u64>,
    pub rate_mismatch_count: Option<u64>,
    pub dropped_frames: Option<u64>,
    pub dropped_events: Option<u64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct RecordingEnvironment {
    pub app_version: String,
    pub build: String,
    pub platform: String,
    pub os_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct RecordingRecognition {
    pub provider_kind: String,
    pub model: String,
    pub requested_language: Option<String>,
    pub local_model_sha256: Option<String>,
}

impl RecordingRecognition {
    fn frozen(
        engine: &PennelEngine,
        recognition: &MeetingRecognition,
    ) -> Result<Self, PennelError> {
        let local_model_sha256 = if recognition.provider_kind == Some(ProviderKind::Local) {
            engine
                .models
                .path(&recognition.model)
                .map(|path| {
                    let mut file = fs::File::open(path).map_err(PennelError::storage)?;
                    let mut digest = Sha256::new();
                    let mut buffer = [0u8; 64 * 1024];
                    loop {
                        let n = file.read(&mut buffer).map_err(PennelError::storage)?;
                        if n == 0 {
                            break;
                        }
                        digest.update(&buffer[..n]);
                    }
                    Ok::<_, PennelError>(crate::models::hex(&digest.finalize()))
                })
                .transpose()?
        } else {
            None
        };
        Ok(Self {
            provider_kind: recognition
                .provider_kind
                .map_or("unknown", ProviderKind::slug)
                .into(),
            model: recognition.model.clone(),
            requested_language: recognition.language.clone(),
            local_model_sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingAudioTrack {
    pub source: RecordingSource,
    pub speaker: Speaker,
    pub relative_path: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct CaptureFormatEpoch {
    pub started_ms: u64,
    pub input_sample_rate: u32,
    pub tap_sample_rate: Option<u32>,
    pub aggregate_sample_rate: Option<u32>,
    pub input_channels: u32,
    pub input_interleaved: bool,
    pub transport: RecordingTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct RecordingTrackMetadata {
    pub source: RecordingSource,
    pub relative_path: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub captured_frames: u64,
    pub output_samples: u64,
    pub inserted_silence_samples: u64,
    pub dropped_frames: u64,
    pub gap_count: u64,
    pub rate_mismatch_count: u64,
    pub capture_errors: u64,
    pub format_epochs: Vec<CaptureFormatEpoch>,
}

impl RecordingTrackMetadata {
    fn new(source: RecordingSource) -> Self {
        Self {
            source,
            relative_path: source.path().into(),
            codec: "aac_lc".into(),
            sample_rate: 16_000,
            channels: 1,
            captured_frames: 0,
            output_samples: 0,
            inserted_silence_samples: 0,
            dropped_frames: 0,
            gap_count: 0,
            rate_mismatch_count: 0,
            capture_errors: 0,
            format_epochs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct RecordingTranscriptionAttempt {
    pub started_at: String,
    pub ended_at: Option<String>,
    pub recognition: RecordingRecognition,
    pub successful: Option<bool>,
    pub transcript_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingTranscriptProvenance {
    pub provider_kind: String,
    pub model: String,
    pub requested_language: Option<String>,
    pub generated_at: String,
}

pub(crate) fn matching_transcript_provenance(
    metadata: &MeetingRecordingMetadata,
    meeting_uid: &str,
    transcript_sha256: &str,
) -> Option<MeetingTranscriptProvenance> {
    if metadata.schema_version != 1 || metadata.meeting_uid != meeting_uid {
        return None;
    }
    let recording_start = DateTime::parse_from_rfc3339(&metadata.started_at).ok()?;
    let recording_end = DateTime::parse_from_rfc3339(metadata.ended_at.as_deref()?).ok()?;
    if recording_end < recording_start {
        return None;
    }
    // A later attempt is relevant only if it successfully produced these exact current bytes.
    let attempt = metadata
        .transcription_attempts
        .iter()
        .rev()
        .find(|attempt| {
            attempt.successful == Some(true)
                && attempt.ended_at.is_some()
                && attempt.transcript_sha256.as_deref() == Some(transcript_sha256)
        })?;
    let generated_at = attempt.ended_at.as_ref()?;
    if DateTime::parse_from_rfc3339(generated_at).ok()?
        < DateTime::parse_from_rfc3339(&attempt.started_at).ok()?
    {
        return None;
    }
    let recognition = &attempt.recognition;
    validate_recognition(recognition).ok()?;
    if recognition.model.trim().is_empty()
        || recognition
            .requested_language
            .as_ref()
            .is_some_and(|language| language.trim().is_empty())
        || ![
            ProviderKind::OpenAi,
            ProviderKind::OpenRouter,
            ProviderKind::Groq,
            ProviderKind::Mistral,
            ProviderKind::Ollama,
            ProviderKind::LmStudio,
            ProviderKind::Custom,
            ProviderKind::Local,
            ProviderKind::AppleSpeech,
            ProviderKind::PennelCloud,
        ]
        .iter()
        .any(|kind| kind.slug() == recognition.provider_kind)
    {
        return None;
    }
    Some(MeetingTranscriptProvenance {
        provider_kind: recognition.provider_kind.clone(),
        model: recognition.model.clone(),
        requested_language: recognition.requested_language.clone(),
        generated_at: generated_at.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingRecordingMetadata {
    pub schema_version: u32,
    pub meeting_uid: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub complete: Option<bool>,
    pub environment: RecordingEnvironment,
    pub recognition: RecordingRecognition,
    pub tracks: Vec<RecordingTrackMetadata>,
    pub transcription_attempts: Vec<RecordingTranscriptionAttempt>,
    pub diagnostic_events_dropped: u64,
}

fn invalid() -> PennelError {
    PennelError::storage("The recording details or audio path are invalid.")
}

fn checked_root(store: &MeetingStore, id: &str) -> Result<PathBuf, PennelError> {
    let mut components = Path::new(id).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || id.contains('\\')
    {
        return Err(invalid());
    }
    store.info(id).ok_or(PennelError::NotFound)?;
    let root = store.root().canonicalize().map_err(PennelError::storage)?;
    let path = store
        .dir_of(id)
        .canonicalize()
        .map_err(PennelError::storage)?;
    if path.parent() != Some(root.as_path()) {
        return Err(invalid());
    }
    Ok(path)
}

fn check_component(path: &Path) -> Result<(), PennelError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(invalid()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PennelError::storage(error)),
    }
}

fn checked_relative(root: &Path, relative: &str) -> Result<PathBuf, PennelError> {
    let mut path = root.to_owned();
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            return Err(invalid());
        };
        path.push(component);
        check_component(&path)?;
    }
    Ok(path)
}

fn private_directory(root: &Path, name: &str) -> Result<PathBuf, PennelError> {
    let path = checked_relative(root, name)?;
    fs::create_dir_all(&path).map_err(PennelError::storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(PennelError::storage)?;
    }
    Ok(path)
}

fn safe_label(value: &str, limit: usize, allow_slash: bool) -> bool {
    value.len() <= limit
        && !value
            .chars()
            .any(|c| c.is_control() || c == '\\' || c == ':' || (!allow_slash && c == '/'))
        && !value.starts_with('/')
}

fn validate_recognition(recognition: &RecordingRecognition) -> Result<(), PennelError> {
    if !safe_label(&recognition.provider_kind, 32, false)
        || !safe_label(&recognition.model, 256, true)
        || recognition
            .requested_language
            .as_ref()
            .is_some_and(|v| !safe_label(v, 32, false))
        || recognition
            .local_model_sha256
            .as_ref()
            .is_some_and(|v| v.len() != 64 || !v.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(invalid());
    }
    Ok(())
}

fn read_metadata(root: &Path) -> Result<Option<MeetingRecordingMetadata>, PennelError> {
    let path = checked_relative(root, RECORDING_FILE)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES => {
            return Err(invalid());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(PennelError::storage(error)),
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(PennelError::storage(error)),
    };
    if file.metadata().map_err(PennelError::storage)?.len() > MAX_METADATA_BYTES {
        return Err(invalid());
    }
    let metadata: MeetingRecordingMetadata =
        serde_json::from_reader(file.take(MAX_METADATA_BYTES + 1)).map_err(|_| invalid())?;
    if metadata.schema_version != 1
        || uuid::Uuid::parse_str(&metadata.meeting_uid).is_err()
        || metadata.tracks.is_empty()
        || metadata.tracks.len() > 2
        || metadata
            .tracks
            .iter()
            .any(|track| track.relative_path != track.source.path())
        || !(metadata
            .tracks
            .iter()
            .map(|track| track.source)
            .eq([RecordingSource::Microphone, RecordingSource::SystemAudio])
            || metadata
                .tracks
                .iter()
                .map(|track| track.source)
                .eq([RecordingSource::RoomMicrophone]))
    {
        return Err(invalid());
    }
    Ok(Some(metadata))
}

fn write_metadata(root: &Path, metadata: &MeetingRecordingMetadata) -> Result<(), PennelError> {
    let bytes = serde_json::to_vec_pretty(metadata).map_err(PennelError::storage)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(invalid());
    }
    durable_write(&checked_relative(root, RECORDING_FILE)?, &bytes)
}

pub(crate) fn backup_recording_metadata(root: &Path, backup: &Path) -> Result<(), PennelError> {
    if read_metadata(root)?.is_some() {
        let bytes =
            fs::read(checked_relative(root, RECORDING_FILE)?).map_err(PennelError::storage)?;
        durable_write(&backup.join("recording.json"), &bytes)?;
    }
    Ok(())
}

pub(crate) fn restore_recording_metadata(root: &Path, backup: &Path) -> Result<(), PennelError> {
    let path = backup.join("recording.json");
    if path.is_file() {
        check_component(&path)?;
        let bytes = fs::read(path).map_err(PennelError::storage)?;
        private_directory(root, ".pennel")?;
        durable_write(&checked_relative(root, RECORDING_FILE)?, &bytes)?;
    }
    Ok(())
}

fn tracks_at(root: &Path) -> Result<Vec<MeetingAudioTrack>, PennelError> {
    let sources: Vec<_> = match read_metadata(root)? {
        Some(metadata) => metadata
            .tracks
            .into_iter()
            .map(|track| (track.source, track.relative_path))
            .collect(),
        None => {
            let exists = |name: &str| {
                checked_relative(root, name)
                    .ok()
                    .is_some_and(|path| path.is_file())
            };
            let room = exists(RecordingSource::RoomMicrophone.path());
            let desktop = exists(RecordingSource::Microphone.path())
                || exists(RecordingSource::SystemAudio.path());
            let legacy = exists("ik.m4a") || exists("anderen.m4a");
            if (room && desktop) || ((room || desktop) && legacy) {
                return Err(invalid());
            }
            if room {
                vec![(
                    RecordingSource::RoomMicrophone,
                    RecordingSource::RoomMicrophone.path().into(),
                )]
            } else if desktop {
                vec![
                    (
                        RecordingSource::Microphone,
                        RecordingSource::Microphone.path().into(),
                    ),
                    (
                        RecordingSource::SystemAudio,
                        RecordingSource::SystemAudio.path().into(),
                    ),
                ]
            } else {
                vec![
                    (RecordingSource::Microphone, "ik.m4a".into()),
                    (RecordingSource::SystemAudio, "anderen.m4a".into()),
                ]
            }
        }
    };
    Ok(sources
        .into_iter()
        .map(|(source, relative_path)| {
            let available = checked_relative(root, &relative_path)
                .ok()
                .is_some_and(|path| path.is_file());
            MeetingAudioTrack {
                source,
                speaker: source.speaker(),
                relative_path,
                available,
            }
        })
        .collect())
}

/// Uses only fixed recording paths and refuses symlinks, including on legacy folders.
pub fn resolve_meeting_audio_tracks(directory: &Path) -> Vec<MeetingAudioTrack> {
    directory
        .canonicalize()
        .ok()
        .and_then(|root| tracks_at(&root).ok())
        .unwrap_or_default()
}

impl MeetingStore {
    pub(crate) fn audio_tracks(&self, id: &str) -> Result<Vec<MeetingAudioTrack>, PennelError> {
        tracks_at(&checked_root(self, id)?)
    }

    pub(crate) fn audio_path(&self, id: &str, speaker: Speaker) -> Result<PathBuf, PennelError> {
        let root = checked_root(self, id)?;
        let track = tracks_at(&root)?
            .into_iter()
            .find(|track| track.speaker == speaker && track.available)
            .ok_or(PennelError::NotFound)?;
        checked_relative(&root, &track.relative_path)?
            .canonicalize()
            .map_err(PennelError::storage)
    }

    pub(crate) fn recording_metadata(
        &self,
        id: &str,
    ) -> Result<Option<MeetingRecordingMetadata>, PennelError> {
        read_metadata(&checked_root(self, id)?)
    }

    pub(crate) fn prepare_recording(
        &self,
        id: &str,
        environment: RecordingEnvironment,
        recognition: RecordingRecognition,
        room: bool,
    ) -> Result<Vec<MeetingAudioTrack>, PennelError> {
        let _gate = RECORDING_GATE.lock().unwrap_or_else(|e| e.into_inner());
        let root = checked_root(self, id)?;
        for value in [
            &environment.app_version,
            &environment.build,
            &environment.platform,
            &environment.os_version,
        ] {
            if !safe_label(value, 128, false) {
                return Err(invalid());
            }
        }
        validate_recognition(&recognition)?;
        if read_metadata(&root)?.is_some()
            || tracks_at(&root)?.iter().any(|track| track.available)
            || [
                RecordingSource::Microphone,
                RecordingSource::SystemAudio,
                RecordingSource::RoomMicrophone,
            ]
            .iter()
            .any(|source| root.join(source.path()).exists())
        {
            return Err(PennelError::storage(
                "This meeting already contains a recording.",
            ));
        }
        private_directory(&root, "audio")?;
        private_directory(&root, ".pennel")?;
        for name in ["transcript.md", "notes.md", EVENTS_FILE] {
            let path = checked_relative(&root, name)?;
            if !path.exists() {
                durable_write(&path, b"")?;
            }
        }
        let sources = if room {
            vec![RecordingSource::RoomMicrophone]
        } else {
            vec![RecordingSource::Microphone, RecordingSource::SystemAudio]
        };
        let started_at = self.info(id).ok_or(PennelError::NotFound)?.start_iso;
        let initial_attempt = RecordingTranscriptionAttempt {
            started_at: started_at.clone(),
            ended_at: None,
            recognition: recognition.clone(),
            successful: None,
            transcript_sha256: None,
        };
        let metadata = MeetingRecordingMetadata {
            schema_version: 1,
            meeting_uid: self.ensure_uid(id)?,
            started_at,
            ended_at: None,
            duration_ms: None,
            complete: None,
            environment,
            recognition,
            tracks: sources
                .into_iter()
                .map(RecordingTrackMetadata::new)
                .collect(),
            transcription_attempts: vec![initial_attempt],
            diagnostic_events_dropped: 0,
        };
        write_metadata(&root, &metadata)?;
        self.set_recording_provenance(id, &metadata)?;
        tracks_at(&root)
    }

    pub(crate) fn record_capture_event(
        &self,
        id: &str,
        event: MeetingCaptureEvent,
    ) -> Result<(), PennelError> {
        let _gate = RECORDING_GATE.lock().unwrap_or_else(|e| e.into_inner());
        if event.error_code.as_ref().is_some_and(|code| {
            code.is_empty()
                || code.len() > 64
                || !code
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        }) || [
            event.input_sample_rate,
            event.tap_sample_rate,
            event.aggregate_sample_rate,
        ]
        .into_iter()
        .flatten()
        .any(|rate| rate == 0 || rate > 384_000)
            || event
                .input_channels
                .is_some_and(|channels| channels == 0 || channels > 128)
        {
            return Err(invalid());
        }
        let root = checked_root(self, id)?;
        let Some(mut metadata) = read_metadata(&root)? else {
            return Ok(());
        };
        let track = metadata
            .tracks
            .iter_mut()
            .find(|track| track.source == event.source)
            .ok_or_else(invalid)?;
        if let (Some(rate), Some(channels)) = (event.input_sample_rate, event.input_channels) {
            let previous = track.format_epochs.last();
            let epoch = CaptureFormatEpoch {
                started_ms: event.elapsed_ms,
                input_sample_rate: rate,
                tap_sample_rate: event
                    .tap_sample_rate
                    .or_else(|| previous.and_then(|old| old.tap_sample_rate)),
                aggregate_sample_rate: event
                    .aggregate_sample_rate
                    .or_else(|| previous.and_then(|old| old.aggregate_sample_rate)),
                input_channels: channels,
                input_interleaved: event
                    .input_interleaved
                    .or_else(|| previous.map(|old| old.input_interleaved))
                    .unwrap_or(false),
                transport: event
                    .transport
                    .or_else(|| previous.map(|old| old.transport))
                    .unwrap_or_default(),
            };
            let changed = track.format_epochs.last().is_none_or(|old| {
                old.input_sample_rate != epoch.input_sample_rate
                    || old.tap_sample_rate != epoch.tap_sample_rate
                    || old.aggregate_sample_rate != epoch.aggregate_sample_rate
                    || old.input_channels != epoch.input_channels
                    || old.input_interleaved != epoch.input_interleaved
                    || old.transport != epoch.transport
            });
            if changed && track.format_epochs.len() < 1024 {
                track.format_epochs.push(epoch);
            }
        }
        for (counter, value) in [
            (&mut track.captured_frames, event.captured_frames),
            (&mut track.output_samples, event.output_samples),
            (
                &mut track.inserted_silence_samples,
                event.inserted_silence_samples,
            ),
            (&mut track.gap_count, event.gap_count),
            (&mut track.rate_mismatch_count, event.rate_mismatch_count),
            (&mut track.dropped_frames, event.dropped_frames),
        ] {
            if let Some(value) = value {
                *counter = (*counter).max(value);
            }
        }
        if event.kind == CaptureEventKind::CaptureError {
            track.capture_errors = track.capture_errors.saturating_add(1);
        }
        if event.kind == CaptureEventKind::RateMismatch && event.rate_mismatch_count.is_none() {
            track.rate_mismatch_count = track.rate_mismatch_count.saturating_add(1);
        }
        if let Some(dropped) = event.dropped_events {
            metadata.diagnostic_events_dropped = metadata.diagnostic_events_dropped.max(dropped);
        }
        if event.kind != CaptureEventKind::Metrics {
            let path = checked_relative(&root, EVENTS_FILE)?;
            if fs::symlink_metadata(&path).is_ok_and(|entry| !entry.is_file()) {
                return Err(invalid());
            }
            if fs::metadata(&path).map_or(0, |m| m.len()) < MAX_EVENT_BYTES {
                let mut options = OpenOptions::new();
                options.create(true).append(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(path).map_err(PennelError::storage)?;
                let mut line = serde_json::to_vec(&event).map_err(PennelError::storage)?;
                line.push(b'\n');
                file.write_all(&line)
                    .and_then(|_| file.sync_all())
                    .map_err(PennelError::storage)?;
            } else {
                metadata.diagnostic_events_dropped =
                    metadata.diagnostic_events_dropped.saturating_add(1);
            }
        }
        write_metadata(&root, &metadata)
    }

    pub(crate) fn finish_recording(
        &self,
        id: &str,
        duration_ms: u64,
        complete: bool,
    ) -> Result<(), PennelError> {
        let _gate = RECORDING_GATE.lock().unwrap_or_else(|e| e.into_inner());
        let root = checked_root(self, id)?;
        let Some(mut metadata) = read_metadata(&root)? else {
            return Ok(());
        };
        metadata.duration_ms = Some(duration_ms);
        let start = DateTime::parse_from_rfc3339(&metadata.started_at).map_err(|_| invalid())?;
        let end = start
            .checked_add_signed(chrono::Duration::milliseconds(
                i64::try_from(duration_ms).map_err(|_| invalid())?,
            ))
            .ok_or_else(invalid)?;
        metadata.ended_at = Some(end.to_rfc3339_opts(SecondsFormat::Millis, false));
        metadata.complete = Some(
            complete
                && metadata.complete != Some(false)
                && metadata.tracks.iter().all(|track| {
                    track.capture_errors == 0
                        && track.rate_mismatch_count == 0
                        && track.dropped_frames == 0
                        && checked_relative(&root, &track.relative_path)
                            .ok()
                            .is_some_and(|path| path.is_file())
                }),
        );
        write_metadata(&root, &metadata)?;
        self.set_recording_provenance(id, &metadata)
    }

    pub(crate) fn append_transcription_attempt(
        &self,
        id: &str,
        attempt: RecordingTranscriptionAttempt,
    ) -> Result<(), PennelError> {
        let _gate = RECORDING_GATE.lock().unwrap_or_else(|e| e.into_inner());
        let root = checked_root(self, id)?;
        let Some(mut metadata) = read_metadata(&root)? else {
            return Ok(());
        };
        validate_recognition(&attempt.recognition)?;
        if metadata.transcription_attempts.len() >= 512 {
            return Err(invalid());
        }
        metadata.transcription_attempts.push(attempt);
        write_metadata(&root, &metadata)
    }

    pub(crate) fn finish_initial_transcription(
        &self,
        id: &str,
        successful: bool,
    ) -> Result<(), PennelError> {
        let _gate = RECORDING_GATE.lock().unwrap_or_else(|e| e.into_inner());
        let root = checked_root(self, id)?;
        let Some(mut metadata) = read_metadata(&root)? else {
            return Ok(());
        };
        if let Some(attempt) = metadata
            .transcription_attempts
            .first_mut()
            .filter(|attempt| attempt.ended_at.is_none())
        {
            attempt.ended_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
            attempt.successful = Some(successful);
            attempt.transcript_sha256 = successful
                .then(|| self.transcript_fingerprint(id))
                .transpose()?;
        }
        write_metadata(&root, &metadata)
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn prepare_meeting_recording(
        &self,
        id: String,
        environment: RecordingEnvironment,
        room_recording: bool,
    ) -> Result<Vec<MeetingAudioTrack>, PennelError> {
        self.claim_library()?;
        let recognition = self.recording_recognition(&id);
        self.meeting_store().prepare_recording(
            &id,
            environment,
            RecordingRecognition::frozen(self, &recognition)?,
            room_recording,
        )
    }

    pub fn meeting_audio_tracks(&self, id: String) -> Vec<MeetingAudioTrack> {
        self.meeting_store().audio_tracks(&id).unwrap_or_default()
    }

    pub fn meeting_recording_metadata(
        &self,
        id: String,
    ) -> Result<Option<MeetingRecordingMetadata>, PennelError> {
        self.meeting_store().recording_metadata(&id)
    }

    pub fn record_meeting_capture_event(
        &self,
        id: String,
        event: MeetingCaptureEvent,
    ) -> Result<(), PennelError> {
        self.claim_library()?;
        self.meeting_store().record_capture_event(&id, event)
    }

    pub fn finish_meeting_recording(
        &self,
        id: String,
        duration_ms: u64,
        complete: bool,
    ) -> Result<(), PennelError> {
        self.claim_library()?;
        self.meeting_store()
            .finish_recording(&id, duration_ms, complete)
    }
}

pub(crate) fn transcription_attempt(
    engine: &PennelEngine,
    recognition: &MeetingRecognition,
    started_at: String,
    successful: bool,
    transcript_sha256: Option<String>,
) -> Result<RecordingTranscriptionAttempt, PennelError> {
    Ok(RecordingTranscriptionAttempt {
        started_at,
        ended_at: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        recognition: RecordingRecognition::frozen(engine, recognition)?,
        successful: Some(successful),
        transcript_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn environment() -> RecordingEnvironment {
        RecordingEnvironment {
            app_version: "0.1.12".into(),
            build: "test-build".into(),
            platform: "macos".into(),
            os_version: "26.0".into(),
        }
    }

    fn recognition() -> RecordingRecognition {
        RecordingRecognition {
            provider_kind: "local".into(),
            model: "parakeet-tdt-0.6b-v3".into(),
            requested_language: Some("nl".into()),
            local_model_sha256: Some("a".repeat(64)),
        }
    }

    #[test]
    fn new_desktop_recording_groups_audio_and_retains_original_recognition() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Meeting", Local::now()).unwrap();
        let tracks = store
            .prepare_recording(&id, environment(), recognition(), false)
            .unwrap();
        assert_eq!(tracks[0].relative_path, "audio/microphone.m4a");
        assert_eq!(tracks[1].relative_path, "audio/system.m4a");
        assert!(store.dir_of(&id).join(".pennel/recording.json").is_file());
        assert!(!store.dir_of(&id).join("ik.m4a").exists());
        let metadata = store.recording_metadata(&id).unwrap().unwrap();
        assert_eq!(metadata.schema_version, 1);
        assert_eq!(metadata.recognition, recognition());
        assert_eq!(metadata.meeting_uid, store.uid_of(&id).unwrap());
        assert!(
            !store
                .prepare_recording(&id, environment(), recognition(), false)
                .is_ok()
        );
    }

    #[test]
    fn legacy_audio_remains_readable_and_room_source_is_explicit() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let old = store.create("Old", Local::now()).unwrap();
        std::fs::write(store.dir_of(&old).join("anderen.m4a"), b"old").unwrap();
        let tracks = store.audio_tracks(&old).unwrap();
        assert_eq!(tracks[1].relative_path, "anderen.m4a");
        assert!(tracks[1].available);
        assert!(store.recording_metadata(&old).unwrap().is_none());
        let new = store.create("Room", Local::now()).unwrap();
        let room = store
            .prepare_recording(&new, environment(), recognition(), true)
            .unwrap();
        assert_eq!(room.len(), 1);
        assert_eq!(room[0].source, RecordingSource::RoomMicrophone);
        assert_eq!(room[0].speaker, Speaker::Others);
        assert_eq!(room[0].relative_path, "audio/room.m4a");
    }

    #[test]
    fn capture_metrics_are_local_and_finalize_precise_duration() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Meeting", Local::now()).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), false)
            .unwrap();
        let mut event = MeetingCaptureEvent {
            source: RecordingSource::SystemAudio,
            kind: CaptureEventKind::FormatChanged,
            input_sample_rate: Some(24_000),
            input_channels: Some(1),
            transport: Some(RecordingTransport::Bluetooth),
            ..Default::default()
        };
        store.record_capture_event(&id, event.clone()).unwrap();
        event.kind = CaptureEventKind::Metrics;
        event.output_samples = Some(160_000);
        event.inserted_silence_samples = Some(1600);
        event.gap_count = Some(1);
        store.record_capture_event(&id, event).unwrap();
        for name in ["audio/microphone.m4a", "audio/system.m4a"] {
            fs::write(store.dir_of(&id).join(name), b"archive").unwrap();
        }
        let event_file = store.dir_of(&id).join(".pennel/capture-events.jsonl");
        assert_eq!(
            std::fs::read_to_string(event_file).unwrap().lines().count(),
            1
        );
        store.finish_recording(&id, 10_123, true).unwrap();
        let metadata = store.recording_metadata(&id).unwrap().unwrap();
        assert_eq!(metadata.duration_ms, Some(10_123));
        assert_eq!(metadata.complete, Some(true));
        assert!(metadata.ended_at.is_some());
        assert_eq!(metadata.tracks[1].output_samples, 160_000);
        assert_eq!(
            metadata.tracks[1].format_epochs[0].input_sample_rate,
            24_000
        );
    }

    #[cfg(unix)]
    #[test]
    fn audio_and_diagnostic_symlink_escapes_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Meeting", Local::now()).unwrap();
        std::os::unix::fs::symlink(outside.path(), store.dir_of(&id).join("audio")).unwrap();
        assert!(
            store
                .prepare_recording(&id, environment(), recognition(), false)
                .is_err()
        );
        let old = store.create("Old", Local::now()).unwrap();
        let target = outside.path().join("secret.m4a");
        std::fs::write(&target, b"secret").unwrap();
        std::os::unix::fs::symlink(&target, store.dir_of(&old).join("anderen.m4a")).unwrap();
        assert!(!store.audio_tracks(&old).unwrap()[1].available);
        let diagnostics = store.create("Linked diagnostics", Local::now()).unwrap();
        std::os::unix::fs::symlink(outside.path(), store.dir_of(&diagnostics).join(".pennel"))
            .unwrap();
        assert!(
            store
                .prepare_recording(&diagnostics, environment(), recognition(), false)
                .is_err()
        );
        assert!(!outside.path().join("recording.json").exists());
    }

    #[test]
    fn nested_audio_is_retained_by_identity_and_removed_by_retention() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Room", Local::now()).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), true)
            .unwrap();
        fs::write(store.dir_of(&id).join("audio/room.m4a"), b"archive").unwrap();
        let uid = store.uid_of(&id).unwrap();
        let renamed = store.rename(&id, "Renamed room").unwrap();
        assert_eq!(store.uid_of(&renamed).as_deref(), Some(uid.as_str()));
        assert_eq!(
            store
                .recording_metadata(&renamed)
                .unwrap()
                .unwrap()
                .meeting_uid,
            uid
        );
        assert_eq!(
            crate::retranscription::saved_tracks(&store, &renamed).unwrap()[0].0,
            Speaker::Others
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(31 * 86_400);
        assert_eq!(store.prune_audio(30, later), 1);
        assert!(store.recording_metadata(&renamed).unwrap().is_some());
        assert!(store.dir_of(&renamed).join("notes.md").exists());
    }

    #[test]
    fn diagnostics_are_private_and_error_details_are_constrained_to_codes() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Meeting", Local::now()).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), false)
            .unwrap();
        let event = MeetingCaptureEvent {
            kind: CaptureEventKind::CaptureError,
            error_code: Some("private words or /a/path".into()),
            ..Default::default()
        };
        assert!(store.record_capture_event(&id, event).is_err());
        let files = crate::cloud::sync::scan::syncable_files(&store.dir_of(&id)).unwrap();
        assert_eq!(
            files
                .into_iter()
                .map(|(name, _)| name)
                .collect::<std::collections::BTreeSet<_>>(),
            [
                "meeting.md".into(),
                "notes.md".into(),
                "transcript.md".into()
            ]
            .into_iter()
            .collect()
        );
        assert!(!crate::cloud::sync::scan::is_syncable(RECORDING_FILE));
        assert!(!crate::cloud::sync::scan::is_syncable(EVENTS_FILE));
        let metadata = fs::read_to_string(store.dir_of(&id).join(RECORDING_FILE)).unwrap();
        assert!(!metadata.contains("private words"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.dir_of(&id).join(RECORDING_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(store.dir_of(&id).join(".pennel"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn metadata_and_original_recognition_rollback_with_transcript_after_interruption() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Meeting", Local::now()).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), false)
            .unwrap();
        let segment = crate::Segment {
            start_ms: 0,
            end_ms: 1000,
            speaker: Speaker::Me,
            text: "original".into(),
            failed: false,
            voice: None,
        };
        store
            .write_transcript(&id, std::slice::from_ref(&segment))
            .unwrap();
        let original = fs::read(store.dir_of(&id).join(RECORDING_FILE)).unwrap();
        let mut replaced = segment;
        replaced.text = "replacement".into();
        let backup = store
            .replace_retranscribed(&id, &[replaced], &[], &[Speaker::Me])
            .unwrap();
        let mut updated = recognition();
        updated.model = "other-local-model".into();
        store
            .append_transcription_attempt(
                &id,
                RecordingTranscriptionAttempt {
                    started_at: Utc::now().to_rfc3339(),
                    ended_at: None,
                    recognition: updated,
                    successful: None,
                    transcript_sha256: None,
                },
            )
            .unwrap();
        assert_eq!(
            store
                .recording_metadata(&id)
                .unwrap()
                .unwrap()
                .transcription_attempts
                .len(),
            2
        );
        assert_eq!(
            store.recording_metadata(&id).unwrap().unwrap().recognition,
            recognition()
        );
        assert!(backup.join("recording.json").is_file());
        store.recover_retranscriptions().unwrap();
        assert_eq!(
            fs::read(store.dir_of(&id).join(RECORDING_FILE)).unwrap(),
            original
        );
        assert_eq!(store.read_transcript(&id)[0].text, "original");
    }

    #[test]
    fn unknown_versions_or_ambiguous_audio_paths_are_not_guessed() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Room", Local::now()).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), true)
            .unwrap();
        fs::write(store.dir_of(&id).join("audio/room.m4a"), b"archive").unwrap();
        fs::remove_file(store.dir_of(&id).join(RECORDING_FILE)).unwrap();
        assert_eq!(
            store.audio_tracks(&id).unwrap()[0].source,
            RecordingSource::RoomMicrophone
        );
        fs::write(store.dir_of(&id).join("anderen.m4a"), b"legacy").unwrap();
        assert!(store.audio_tracks(&id).is_err());
        fs::remove_file(store.dir_of(&id).join("anderen.m4a")).unwrap();
        fs::remove_file(store.dir_of(&id).join("audio/room.m4a")).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), true)
            .unwrap();
        let path = store.dir_of(&id).join(RECORDING_FILE);
        let modified = fs::read_to_string(&path)
            .unwrap()
            .replace("\"schema_version\": 1", "\"schema_version\": 2");
        fs::write(path, modified).unwrap();
        assert!(store.recording_metadata(&id).is_err());
        assert!(store.audio_tracks(&id).is_err());
    }

    #[test]
    fn rate_mismatch_remains_incomplete_after_recovery_and_retains_both_rates() {
        let root = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(root.path());
        let id = store.create("Meeting", Local::now()).unwrap();
        store
            .prepare_recording(&id, environment(), recognition(), false)
            .unwrap();
        for name in ["audio/microphone.m4a", "audio/system.m4a"] {
            fs::write(store.dir_of(&id).join(name), b"archive").unwrap();
        }
        store
            .record_capture_event(
                &id,
                MeetingCaptureEvent {
                    source: RecordingSource::SystemAudio,
                    kind: CaptureEventKind::RateMismatch,
                    input_sample_rate: Some(24_000),
                    tap_sample_rate: Some(48_000),
                    aggregate_sample_rate: Some(24_000),
                    input_channels: Some(2),
                    dropped_events: Some(3),
                    ..Default::default()
                },
            )
            .unwrap();
        store.finish_recording(&id, 10_000, true).unwrap();
        let metadata = store.recording_metadata(&id).unwrap().unwrap();
        assert_eq!(metadata.complete, Some(false));
        assert_eq!(metadata.diagnostic_events_dropped, 3);
        assert_eq!(
            metadata.tracks[1].format_epochs[0].tap_sample_rate,
            Some(48_000)
        );
        assert_eq!(
            metadata.tracks[1].format_epochs[0].aggregate_sample_rate,
            Some(24_000)
        );
    }

    #[test]
    fn engine_preparation_uses_the_active_sessions_frozen_recognition() {
        let root = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(root.path().join("data").to_string_lossy().into_owned())
                .unwrap();
        let mut settings = engine.settings();
        settings.providers.push(crate::ProviderAccount {
            id: "test-recording-provider".into(),
            kind: ProviderKind::Custom,
            name: "Private account name".into(),
            base_url: "http://127.0.0.1:1/private-endpoint".into(),
        });
        settings.meeting_stt = crate::ModelChoice {
            provider_id: "test-recording-provider".into(),
            model: "original-model".into(),
        };
        settings.language = Some("nl".into());
        engine.update_settings(settings.clone()).unwrap();
        let session = engine
            .clone()
            .start_meeting(
                "Meeting".into(),
                crate::ApiKeys {
                    by_provider: std::collections::HashMap::new(),
                },
            )
            .unwrap();
        settings.meeting_stt.model = "changed-model".into();
        settings.language = Some("en".into());
        engine.update_settings(settings).unwrap();
        engine
            .prepare_meeting_recording(session.id(), environment(), true)
            .unwrap();
        session.stop().unwrap();
        let metadata = engine
            .meeting_recording_metadata(session.id())
            .unwrap()
            .unwrap();
        assert_eq!(metadata.recognition.model, "original-model");
        assert_eq!(
            metadata.recognition.requested_language.as_deref(),
            Some("nl")
        );
        assert_eq!(metadata.transcription_attempts[0].successful, Some(true));
        let serialized = serde_json::to_string(&metadata).unwrap();
        assert!(!serialized.contains("Private account name"));
        assert!(!serialized.contains("private-endpoint"));
    }
}
