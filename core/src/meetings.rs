use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::SystemTime;

use chrono::{DateTime, Local};

use crate::error::PennelError;
use crate::meeting_utterances::{self, MeetingTranscriptSnapshot, MeetingUtterance};

const TITLE_MAX: usize = 60;
const RECOVERY_AUDIO_MAX_BYTES: u64 = 16_000 * 2 * 31 + 44;
const FAILED_MARK: &str = "_(transcriptie mislukt)_";
pub(crate) const REPLACEMENT_JOURNAL: &str = ".pennel-retranscription-pending.json";
pub(crate) const TRANSCRIPT_BACKUPS: &str = ".pennel-transcript-backups";
#[derive(Clone, Hash, PartialEq, Eq)]
enum ReplacementKey {
    Path(PathBuf),
    Uid(PathBuf, String),
}

// Keep one meeting atomic without making unrelated libraries wait for its metadata or fsyncs.
pub(crate) fn replacement_gate(root: &Path) -> Arc<Mutex<()>> {
    static GATES: OnceLock<Mutex<std::collections::HashMap<ReplacementKey, Weak<Mutex<()>>>>> =
        OnceLock::new();
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path_key = ReplacementKey::Path(canonical.clone());
    let uid_key = fs::read_to_string(canonical.join("meeting.md"))
        .ok()
        .and_then(|text| parse_meeting(&text).map(|(front, _)| front.uid))
        .filter(|uid| valid_uid(uid))
        .map(|uid| {
            ReplacementKey::Uid(canonical.parent().unwrap_or(&canonical).to_path_buf(), uid)
        });
    let mut gates = GATES
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    gates.retain(|_, gate| gate.strong_count() > 0);
    let gate = uid_key
        .as_ref()
        .and_then(|key| gates.get(key))
        .and_then(Weak::upgrade)
        .or_else(|| gates.get(&path_key).and_then(Weak::upgrade))
        .unwrap_or_else(|| Arc::new(Mutex::new(())));
    gates.insert(path_key, Arc::downgrade(&gate));
    if let Some(uid_key) = uid_key {
        gates.insert(uid_key, Arc::downgrade(&gate));
    }
    gate
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingReplacement {
    version: u32,
    backup: String,
    had_failed: bool,
}

impl PendingReplacement {
    fn backup_path(&self, root: &Path) -> Result<PathBuf, PennelError> {
        if self.version != 1
            || uuid::Uuid::parse_str(&self.backup).is_err()
            || Path::new(&self.backup).components().count() != 1
        {
            return Err(PennelError::storage(
                "The recovery folder is not a valid local folder.",
            ));
        }
        let backups = root.join(TRANSCRIPT_BACKUPS);
        let backup = backups.join(&self.backup);
        for path in [&backups, &backup] {
            let entry = fs::symlink_metadata(path).map_err(PennelError::storage)?;
            if !entry.is_dir() || entry.file_type().is_symlink() {
                return Err(PennelError::storage(
                    "The recovery folder is not a valid local folder.",
                ));
            }
        }
        Ok(backup)
    }
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), PennelError> {
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(PennelError::storage)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn clear_replacement_journal(root: &Path) -> Result<(), PennelError> {
    fs::remove_file(root.join(REPLACEMENT_JOURNAL)).map_err(PennelError::storage)?;
    sync_directory(root)
}

fn rollback_replacement(root: &Path, pending: &PendingReplacement) -> Result<(), PennelError> {
    let backup = pending.backup_path(root)?;
    // Read every authoritative backup before changing any live file.
    let metadata = fs::read(backup.join("meeting.md")).map_err(PennelError::storage)?;
    let transcript = fs::read(backup.join("transcript.md")).map_err(PennelError::storage)?;
    durable_write(&root.join("meeting.md"), &metadata)?;
    durable_write(&root.join("transcript.md"), &transcript)?;
    meeting_utterances::restore_metadata(root, &backup)?;
    crate::meeting_recording::restore_recording_metadata(root, &backup)?;
    if pending.had_failed {
        let saved = backup.join("failed");
        let live = root.join("failed");
        if saved.is_dir() {
            // An interrupted rollback may already have moved the original folder back.
            fs::rename(&saved, &live).map_err(PennelError::storage)?;
            sync_directory(&backup)?;
            sync_directory(root)?;
        } else if !live.is_dir() {
            return Err(PennelError::storage(
                "Recovery could not be completed. The previous transcript is in the local recovery folder.",
            ));
        }
    }
    durable_write(&backup.join("rolled-back"), b"1")?;
    clear_replacement_journal(root)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    Me,
    Others,
}

impl Speaker {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Speaker::Me => "Ik",
            Speaker::Others => "Anderen",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: Speaker,
    pub text: String,
    pub failed: bool,
    /// Which of the other participants spoke, after diarization (1-based); `None` for "Ik" or undiarized.
    pub voice: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct SpeakerName {
    pub voice: u32,
    pub name: String,
}

const SPEAKER_NAME_MAX: usize = 40;
/// Voices per meeting. Sortformer told four apart; the speaker pipeline has no fixed limit, and
/// more than twelve distinct voices in one recording is almost always over-splitting.
pub const VOICES: std::ops::RangeInclusive<u32> = 1..=12;

/// "Ik", a participant's name, "Anderen 2" or "Anderen".
pub fn speaker_label(segment: &Segment, names: &[SpeakerName]) -> String {
    match (segment.speaker, segment.voice) {
        (Speaker::Me, _) => "Ik".to_owned(),
        (Speaker::Others, Some(voice)) => names
            .iter()
            .find(|n| n.voice == voice)
            .map_or_else(|| format!("Anderen {voice}"), |n| n.name.clone()),
        (Speaker::Others, None) => "Anderen".to_owned(),
    }
}

fn sanitize_speaker(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !matches!(c, ';' | '=' | ':' | '\n' | '\r' | '[' | ']' | '*'))
        .collect();
    clean.trim().chars().take(SPEAKER_NAME_MAX).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct MeetingInfo {
    pub id: String,
    pub title: String,
    pub start_iso: String,
    pub duration_min: u32,
    pub has_summary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoteLine {
    pub text: String,
    pub mine: bool,
}

struct Frontmatter {
    title: String,
    start_iso: String,
    duration_min: u32,
    template: String,
    /// `pennel_id`: the stable sync identity of the meeting, independent of its folder name.
    uid: String,
    /// Lines the user added (tags, attendees, …), written back untouched.
    extra: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailedTranscriptionRecoveryStatus {
    Complete,
    Retryable,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailedTranscriptionRecoveryReason {
    MetadataUnavailable,
    UnverifiedProvenance,
    AudioChanged,
    UnreadableAudio,
    NoRetainedAudio,
}

/// Current retained candidates, not a guarantee of audio integrity or recognition success.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FailedTranscriptionRecoveryPlan {
    pub failed_count: Option<u32>,
    pub retryable_count: Option<u32>,
    pub status: FailedTranscriptionRecoveryStatus,
    pub reason_code: Option<FailedTranscriptionRecoveryReason>,
}

impl FailedTranscriptionRecoveryPlan {
    fn metadata_unavailable() -> Self {
        Self {
            failed_count: None,
            retryable_count: None,
            status: FailedTranscriptionRecoveryStatus::Unavailable,
            reason_code: Some(FailedTranscriptionRecoveryReason::MetadataUnavailable),
        }
    }
}

impl FailedTranscriptionRecoveryReason {
    fn priority(self) -> u8 {
        match self {
            Self::MetadataUnavailable => 5,
            Self::UnverifiedProvenance => 4,
            Self::AudioChanged => 3,
            Self::UnreadableAudio => 2,
            Self::NoRetainedAudio => 1,
        }
    }
}

pub struct MeetingStore {
    dir: PathBuf,
}

pub(crate) fn durable_write(path: &Path, bytes: &[u8]) -> Result<(), PennelError> {
    durable_write_checked(path, bytes, || Ok(()))
}

pub(crate) fn durable_write_checked(
    path: &Path,
    bytes: &[u8],
    verify: impl FnOnce() -> Result<(), PennelError>,
) -> Result<(), PennelError> {
    let parent = path.parent().ok_or_else(|| PennelError::Storage {
        message: "the storage folder is missing".into(),
    })?;
    let temporary = parent.join(format!(".pennel-{}.tmp", uuid::Uuid::now_v7()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(PennelError::storage)?;
        file.write_all(bytes).map_err(PennelError::storage)?;
        file.sync_all().map_err(PennelError::storage)?;
        drop(file);
        verify()?;
        fs::rename(&temporary, path).map_err(PennelError::storage)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(PennelError::storage)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn recovery_audio(path: &Path) -> Option<Segment> {
    let stem = path.file_stem()?.to_str()?;
    let (speaker, stamp) = if let Some(stamp) = stem.strip_prefix("me-") {
        (Speaker::Me, stamp)
    } else {
        (Speaker::Others, stem.strip_prefix("others-")?)
    };
    let start_ms = stamp.parse::<u64>().ok()?;
    // Recovery files use our fixed PCM header. Listing a long offline meeting must not decode every
    // audio chunk, or allocate an arbitrary dropped-in file just to discover its duration.
    let length = fs::metadata(path).ok()?.len();
    if length > RECOVERY_AUDIO_MAX_BYTES {
        return None;
    }
    let mut header = [0u8; 44];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF"
        || u64::from(u32::from_le_bytes(header[4..8].try_into().ok()?)) + 8 != length
        || &header[8..16] != b"WAVEfmt "
        || u32::from_le_bytes(header[16..20].try_into().ok()?) != 16
        || &header[36..40] != b"data"
        || u16::from_le_bytes(header[20..22].try_into().ok()?) != 1
        || u16::from_le_bytes(header[22..24].try_into().ok()?) != 1
        || u16::from_le_bytes(header[34..36].try_into().ok()?) != 16
        || u16::from_le_bytes(header[32..34].try_into().ok()?) != 2
    {
        return None;
    }
    let rate = u32::from_le_bytes(header[24..28].try_into().ok()?);
    let data_length = u64::from(u32::from_le_bytes(header[40..44].try_into().ok()?));
    if rate != 16_000
        || u32::from_le_bytes(header[28..32].try_into().ok()?) != 32_000
        || data_length % 2 != 0
        || data_length + 44 != length
    {
        return None;
    }
    Some(Segment {
        start_ms,
        end_ms: start_ms.saturating_add(data_length / 2 * 1000 / u64::from(rate)),
        speaker,
        text: String::new(),
        failed: true,
        voice: None,
    })
}

#[cfg(test)]
thread_local! {
    static RECOVERY_PCM_READ_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn read_recovery_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    read_recovery_payload(fs::File::open(path)?)
}

fn read_recovery_payload(reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(RECOVERY_AUDIO_MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    #[cfg(test)]
    RECOVERY_PCM_READ_BYTES.with(|count| count.set(count.get().saturating_add(bytes.len())));
    if bytes.len() as u64 > RECOVERY_AUDIO_MAX_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "The recovery audio exceeds its original chunk limit.",
        ));
    }
    Ok(bytes)
}

pub(crate) fn recovery_provenance(
    path: &Path,
    disposition: crate::meeting_utterances::RecoveryAudioDisposition,
) -> Option<crate::meeting_utterances::RecoveryAudioProvenance> {
    let mut proof = recovery_header_provenance(path)?;
    let bytes = read_recovery_bytes(path).ok()?;
    proof.sha256 = meeting_utterances::digest(&bytes);
    proof.disposition = disposition;
    proof.valid().then_some(proof)
}

fn recovery_header_provenance(
    path: &Path,
) -> Option<crate::meeting_utterances::RecoveryAudioProvenance> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    let segment = recovery_audio(path)?;
    let proof = crate::meeting_utterances::RecoveryAudioProvenance {
        file_name: path.file_name()?.to_str()?.into(),
        track: crate::meeting_utterances::RecoveryAudioTrack::from_speaker(segment.speaker),
        start_ms: segment.start_ms,
        end_ms: segment.end_ms,
        sha256: String::new(),
        disposition: crate::meeting_utterances::RecoveryAudioDisposition::Pending,
    };
    proof.valid().then_some(proof)
}

fn recovery_files(
    root: &Path,
) -> Vec<(PathBuf, crate::meeting_utterances::RecoveryAudioProvenance)> {
    let directory = root.join("failed");
    if !fs::symlink_metadata(&directory)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Vec::new();
    }
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            recovery_header_provenance(&entry.path()).map(|proof| (entry.path(), proof))
        })
        .collect()
}

fn completed_recovery(
    snapshot: &MeetingTranscriptSnapshot,
    path: &Path,
    audio: &crate::meeting_utterances::RecoveryAudioProvenance,
) -> bool {
    use crate::meeting_utterances::RecoveryAudioDisposition;
    snapshot.metadata_status == crate::TranscriptMetadataStatus::Valid
        && snapshot.utterances.iter().any(|row| {
            !row.segment.failed
                && row.metadata.recovery_audio.as_ref().is_some_and(|proof| {
                    proof.disposition == RecoveryAudioDisposition::Complete
                        && proof.same_chunk(audio)
                })
        })
        && !snapshot.utterances.iter().any(|row| {
            row.segment.failed
                && row
                    .metadata
                    .recovery_audio
                    .as_ref()
                    .is_some_and(|proof| proof.file_name == audio.file_name)
        })
        && recovery_provenance(path, RecoveryAudioDisposition::Complete).is_some_and(|actual| {
            snapshot.utterances.iter().any(|row| {
                !row.segment.failed
                    && row.metadata.recovery_audio.as_ref().is_some_and(|proof| {
                        proof.disposition == RecoveryAudioDisposition::Complete
                            && proof.same_audio(&actual)
                    })
            })
        })
}

fn preserve_completion_markers(
    previous: &MeetingTranscriptSnapshot,
    rows: &mut Vec<MeetingUtterance>,
) {
    for row in &previous.utterances {
        if row.segment.failed || !row.segment.text.trim().is_empty() {
            continue;
        }
        // PCM is local, while transcript metadata is portable. A peer without the WAV cannot
        // retire the origin's private completion disposition by saving its visible empty view.
        if !rows
            .iter()
            .any(|candidate| candidate.metadata.utterance_id == row.metadata.utterance_id)
        {
            rows.push(row.clone());
        }
    }
}

// Local cleanup bookkeeping requires verified absence; dangling entries and unsafe parents remain
// pending. This never retires the portable transcript completion disposition.
fn recovery_entry_may_exist(root: &Path, name: &str) -> bool {
    if !fs::symlink_metadata(root)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return true;
    }
    let directory = root.join("failed");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        _ => return true,
    }
    match fs::symlink_metadata(directory.join(name)) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

pub(crate) fn timestamp(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn transcript_timestamp(ms: u64) -> String {
    let stamp = timestamp(ms);
    if ms.is_multiple_of(1000) {
        stamp
    } else {
        format!("{stamp}.{:03}", ms % 1000)
    }
}

pub(crate) fn parse_timestamp(text: &str) -> Option<u64> {
    let (text, fraction) = match text.split_once('.') {
        Some((stamp, fraction))
            if fraction.len() == 3 && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (stamp, fraction.parse::<u64>().ok()?)
        }
        Some(_) => return None,
        None => (text, 0),
    };
    let parts: Vec<u64> = text
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let seconds = match parts.as_slice() {
        [m, s] => m.checked_mul(60)?.checked_add(*s)?,
        [h, m, s] => h
            .checked_mul(3600)?
            .checked_add(m.checked_mul(60)?)?
            .checked_add(*s)?,
        _ => return None,
    };
    seconds.checked_mul(1000)?.checked_add(fraction)
}

fn sanitize(title: &str) -> String {
    let clean: String = title
        .chars()
        .filter(|c| !matches!(c, '/' | ':' | '\\'))
        .collect();
    let clean = clean.trim();
    let clean = if clean.is_empty() { "Meeting" } else { clean };
    clean.chars().take(TITLE_MAX).collect()
}

/// Folder names avoid characters Windows forbids, so a meetings folder also works when synced there.
fn folder_name(title: &str) -> String {
    let clean: String = sanitize(title)
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | '"' | '|' | '?' | '*') && !c.is_control())
        .collect();
    let clean = clean.trim_end_matches(['.', ' ']).trim();
    if clean.is_empty() {
        "Meeting".into()
    } else {
        clean.into()
    }
}

fn normalize_note(line: &str) -> String {
    let mut text = line.trim();
    for prefix in ["- [ ] ", "- [x] ", "- ", "* ", "[ ] "] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest;
            break;
        }
    }
    text.trim().to_lowercase()
}

#[uniffi::export]
pub fn mark_lines(enhanced: &str, notes: &str) -> Vec<NoteLine> {
    let own: Vec<String> = notes
        .lines()
        .map(normalize_note)
        .filter(|l| !l.is_empty())
        .collect();
    enhanced
        .lines()
        .map(|line| {
            let key = normalize_note(line);
            NoteLine {
                text: line.to_owned(),
                mine: !key.is_empty() && own.contains(&key),
            }
        })
        .collect()
}

impl MeetingStore {
    pub(crate) fn set_recording_provenance(
        &self,
        id: &str,
        metadata: &crate::meeting_recording::MeetingRecordingMetadata,
    ) -> Result<(), PennelError> {
        let (mut front, body) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        for field in [
            "pennel_recording_schema",
            "pennel_recording_app",
            "pennel_stt_provider",
            "pennel_stt_model",
            "pennel_stt_language",
            "pennel_recording_duration_ms",
            "pennel_recording_complete",
        ] {
            front
                .extra
                .retain(|line| !line.starts_with(&format!("{field}:")));
        }
        front.extra.extend([
            "pennel_recording_schema: 1".into(),
            format!("pennel_recording_app: {}", metadata.environment.app_version),
            format!(
                "pennel_stt_provider: {}",
                metadata.recognition.provider_kind
            ),
            format!("pennel_stt_model: {}", metadata.recognition.model),
            format!(
                "pennel_stt_language: {}",
                metadata
                    .recognition
                    .requested_language
                    .as_deref()
                    .unwrap_or("auto")
            ),
        ]);
        if let Some(duration) = metadata.duration_ms {
            front
                .extra
                .push(format!("pennel_recording_duration_ms: {duration}"));
        }
        if let Some(complete) = metadata.complete {
            front
                .extra
                .push(format!("pennel_recording_complete: {complete}"));
        }
        self.write_file(id, "meeting.md", &render_meeting(&front, &body))
    }

    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.dir.clone()
    }

    pub fn dir_of(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    pub fn create(&self, title: &str, start: DateTime<Local>) -> Result<String, PennelError> {
        let base = format!("{} {}", start.format("%Y-%m-%d %H%M"), folder_name(title));
        let id = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} ({n})")))
            .find(|candidate| !self.dir.join(candidate).exists())
            .expect("unbounded iterator");
        std::fs::create_dir_all(self.dir_of(&id)).map_err(PennelError::storage)?;
        let front = Frontmatter {
            title: sanitize(title),
            start_iso: start.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
            duration_min: 0,
            template: String::new(),
            uid: uuid::Uuid::now_v7().to_string(),
            extra: Vec::new(),
        };
        self.write_file(&id, "meeting.md", &render_meeting(&front, ""))?;
        Ok(id)
    }

    pub fn uid_of(&self, id: &str) -> Option<String> {
        self.frontmatter(id)
            .map(|(front, _)| front.uid)
            .filter(|uid| valid_uid(uid))
    }

    pub fn ensure_uid(&self, id: &str) -> Result<String, PennelError> {
        if let Some(uid) = self.uid_of(id) {
            return Ok(uid);
        }
        let uid = uuid::Uuid::now_v7().to_string();
        self.set_uid(id, &uid)?;
        Ok(uid)
    }

    /// Only the `pennel_id` line changes, so the rest of a hand-edited file stays byte-identical
    /// and two devices adding the same id to the same file produce the same bytes.
    pub(crate) fn set_uid(&self, id: &str, uid: &str) -> Result<(), PennelError> {
        let text = std::fs::read_to_string(self.dir_of(id).join("meeting.md"))
            .map_err(|_| PennelError::NotFound)?;
        let (head, body, newline) = split_frontmatter(&text).ok_or(PennelError::NotFound)?;
        let mut lines: Vec<&str> = head
            .split(newline)
            .filter(|line| line.split(':').next().map(str::trim) != Some(UID_FIELD))
            .collect();
        let uid_line = format!("{UID_FIELD}: {uid}");
        lines.push(&uid_line);
        let path = self.dir_of(id).join("meeting.md");
        // UID updates and ordinary edits must never share a temporary file.
        let tmp = path.with_file_name(format!(".meeting-{}.tmp", uuid::Uuid::now_v7()));
        let head = lines.join(newline);
        std::fs::write(
            &tmp,
            format!("---{newline}{head}{newline}---{newline}{body}"),
        )
        .map_err(PennelError::storage)?;
        // Sync adds the id in the background; an edit saved meanwhile must not be replaced.
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(PennelError::Storage {
                message: UID_RACE.into(),
            });
        }
        std::fs::rename(&tmp, &path).map_err(PennelError::storage)
    }

    fn write_file(&self, id: &str, name: &str, content: &str) -> Result<(), PennelError> {
        durable_write(&self.dir_of(id).join(name), content.as_bytes())
    }

    pub(crate) fn write_recovery(
        &self,
        id: &str,
        speaker: Speaker,
        start_ms: u64,
        bytes: &[u8],
    ) -> Result<PathBuf, PennelError> {
        let meeting = self.dir_of(id);
        let gate = replacement_gate(&meeting);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        if !meeting.is_dir() {
            return Err(PennelError::Storage {
                message: "the meeting folder is unavailable".into(),
            });
        }
        let path = crate::session::failed_path(&meeting, speaker, start_ms);
        if fs::symlink_metadata(path.parent().expect("recovery directory"))
            .is_ok_and(|metadata| !metadata.is_dir() || metadata.file_type().is_symlink())
        {
            return Err(PennelError::storage(
                "The recovery folder is not a valid local folder.",
            ));
        }
        fs::create_dir_all(path.parent().expect("recovery directory"))
            .map_err(PennelError::storage)?;
        #[cfg(unix)]
        fs::File::open(&meeting)
            .and_then(|directory| directory.sync_all())
            .map_err(PennelError::storage)?;
        durable_write(&path, bytes)?;
        Ok(path)
    }

    /// Modification time (ns) and length, enough to notice any rewrite of a meeting file.
    pub fn file_stamp(&self, id: &str, name: &str) -> Option<(i64, u64)> {
        let meta = std::fs::metadata(self.dir_of(id).join(name)).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some((modified.as_nanos() as i64, meta.len()))
    }

    fn read_file(&self, id: &str, name: &str) -> String {
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        std::fs::read_to_string(self.dir_of(id).join(name)).unwrap_or_default()
    }

    pub fn write_notes(&self, id: &str, notes: &str) -> Result<(), PennelError> {
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        self.write_file(id, "notes.md", notes)
    }

    pub(crate) fn write_notes_if_unchanged(
        &self,
        id: &str,
        expected: &str,
        notes: &str,
    ) -> Result<(), PennelError> {
        self.write_notes_if_any_unchanged(id, &[expected], notes)
    }

    pub(crate) fn write_notes_if_any_unchanged(
        &self,
        id: &str,
        expected: &[&str],
        notes: &str,
    ) -> Result<(), PennelError> {
        let directory = self.dir_of(id);
        let gate = replacement_gate(&directory);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        if !directory.is_dir() {
            return Err(PennelError::NotFound);
        }
        let path = directory.join("notes.md");
        let read_current = || match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                fs::read_to_string(&path).map_err(PennelError::storage)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            _ => Err(PennelError::storage(
                "The saved notes could not be verified.",
            )),
        };
        let before = read_current()?;
        if !expected.contains(&before.as_str()) {
            return Err(PennelError::storage(
                "The notes changed. Your current text was kept.",
            ));
        }
        if before == notes {
            return Ok(());
        }
        durable_write_checked(&path, notes.as_bytes(), || {
            if read_current()? != before {
                return Err(PennelError::storage(
                    "The notes changed. Your current text was kept.",
                ));
            }
            Ok(())
        })
    }

    pub fn summary_outdated(&self, id: &str) -> bool {
        self.frontmatter(id).is_some_and(|(front, body)| {
            !body.trim().is_empty()
                && front
                    .extra
                    .iter()
                    .any(|line| line.trim() == "pennel_summary_outdated: true")
        })
    }

    pub(crate) fn transcript_tracks(&self, id: &str) -> Result<Option<Vec<Speaker>>, PennelError> {
        let (front, _) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        let Some(value) = front
            .extra
            .iter()
            .find_map(|line| line.strip_prefix("pennel_transcript_tracks:"))
        else {
            return Ok(None);
        };
        value
            .trim()
            .split(',')
            .map(|track| match track.trim() {
                "microphone" => Ok(Speaker::Me),
                "system" => Ok(Speaker::Others),
                _ => Err(PennelError::storage(
                    "The original recording tracks could not be determined. Both tracks are required for older speaker-labeled transcripts.",
                )),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Keep physical sources before diarization can turn a microphone line into "Anderen".
    pub(crate) fn preserve_transcript_tracks(&self, id: &str) -> Result<(), PennelError> {
        if self.transcript_tracks(id)?.is_some() {
            return Ok(());
        }
        let segments = self.read_transcript(id);
        // A legacy diarized transcript no longer proves which physical channels it came from.
        if segments.is_empty() || segments.iter().any(|segment| segment.voice.is_some()) {
            return Ok(());
        }
        let tracks: Vec<_> = [Speaker::Me, Speaker::Others]
            .into_iter()
            .filter(|speaker| segments.iter().any(|segment| segment.speaker == *speaker))
            .collect();
        let (mut front, body) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        set_transcript_tracks(&mut front, &tracks);
        self.write_file(id, "meeting.md", &render_meeting(&front, &body))
    }

    pub(crate) fn transcript_fingerprint(&self, id: &str) -> Result<String, PennelError> {
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        use sha2::{Digest, Sha256};
        let bytes = match fs::read(self.dir_of(id).join("transcript.md")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(PennelError::storage(error)),
        };
        Ok(crate::models::hex(&Sha256::digest(&bytes)))
    }

    pub(crate) fn write_dictation_intervals(
        &self,
        id: &str,
        intervals: &[(u64, u64)],
    ) -> Result<(), PennelError> {
        let bytes = serde_json::to_vec(intervals).map_err(PennelError::storage)?;
        durable_write(
            &self.dir_of(id).join(".pennel-dictation-intervals.json"),
            &bytes,
        )
    }

    pub(crate) fn dictation_intervals(&self, id: &str) -> Result<Vec<(u64, u64)>, PennelError> {
        match fs::read(self.dir_of(id).join(".pennel-dictation-intervals.json")) {
            Ok(bytes) => {
                let intervals: Vec<(u64, u64)> = serde_json::from_slice(&bytes).map_err(|_| {
                    PennelError::storage("The saved dictation intervals could not be read.")
                })?;
                if intervals.iter().any(|(start, end)| end < start) {
                    return Err(PennelError::storage(
                        "The saved dictation intervals are invalid.",
                    ));
                }
                Ok(intervals)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(PennelError::storage(error)),
        }
    }

    #[allow(dead_code)] // Retained for callers of the text-only replacement contract.
    pub(crate) fn replace_retranscribed(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        tracks: &[Speaker],
    ) -> Result<PathBuf, PennelError> {
        self.replace_transcript(id, segments, names, tracks, true, None)
    }

    pub(crate) fn replace_speaker_labels(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        tracks: &[Speaker],
    ) -> Result<PathBuf, PennelError> {
        self.replace_transcript(id, segments, names, tracks, false, None)
    }

    pub(crate) fn replace_speaker_utterances(
        &self,
        id: &str,
        utterances: &[MeetingUtterance],
        names: &[SpeakerName],
        tracks: &[Speaker],
    ) -> Result<PathBuf, PennelError> {
        let segments: Vec<_> = utterances.iter().map(|row| row.segment.clone()).collect();
        self.replace_transcript(id, &segments, names, tracks, false, Some(utterances))
    }

    pub(crate) fn replace_retranscribed_utterances(
        &self,
        id: &str,
        utterances: &[MeetingUtterance],
        names: &[SpeakerName],
        tracks: &[Speaker],
    ) -> Result<PathBuf, PennelError> {
        let segments: Vec<_> = utterances.iter().map(|row| row.segment.clone()).collect();
        self.replace_transcript(id, &segments, names, tracks, true, Some(utterances))
    }

    fn replace_transcript(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        tracks: &[Speaker],
        move_failed_audio: bool,
        utterances: Option<&[MeetingUtterance]>,
    ) -> Result<PathBuf, PennelError> {
        let legacy_source = self.unique_legacy_source(id);
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        if root.join(REPLACEMENT_JOURNAL).exists()
            || root.join(meeting_utterances::SNAPSHOT_JOURNAL).exists()
        {
            return Err(PennelError::storage(
                "Recovery could not be completed. The previous transcript is in the local recovery folder.",
            ));
        }
        let original = fs::read(root.join("transcript.md"))
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(Vec::new())
                } else {
                    Err(error)
                }
            })
            .map_err(PennelError::storage)?;
        let metadata = fs::read(root.join("meeting.md")).map_err(PennelError::storage)?;
        let (mut front, body) =
            parse_meeting(&String::from_utf8_lossy(&metadata)).ok_or(PennelError::NotFound)?;
        refuse_unparsed(&original)?;
        let original_uid = if valid_uid(&front.uid) {
            front.uid.clone()
        } else {
            format!(
                "legacy-{}",
                &meeting_utterances::digest(id.as_bytes())[..32]
            )
        };
        let mut previous = snapshot_from_text(&root, &original_uid, &original);
        establish_legacy_source(&mut previous, legacy_source);
        meeting_utterances::assert_writable(&previous)?;
        if !valid_uid(&front.uid) {
            front.uid = uuid::Uuid::now_v7().to_string();
        }
        set_names(&mut front, names);
        set_transcript_tracks(&mut front, tracks);
        front
            .extra
            .retain(|line| !line.starts_with("pennel_summary_outdated:"));
        if !body.trim().is_empty() {
            front.extra.push("pennel_summary_outdated: true".into());
        }
        let replacement_metadata = render_meeting(&front, &body);
        let mut sorted = segments.to_vec();
        sorted.sort_by_key(|segment| segment.start_ms);
        let mut rows = match utterances {
            Some(rows) => rows.to_vec(),
            None => meeting_utterances::reconcile(&previous, &sorted)?,
        };
        preserve_completion_markers(&previous, &mut rows);
        rows.sort_by_key(|row| row.segment.start_ms);
        meeting_utterances::refresh_for_update(&previous, &mut rows)?;
        sorted = rows.iter().map(|row| row.segment.clone()).collect();
        let replacement_transcript = render_transcript(&sorted);
        let replacement_attribution = meeting_utterances::prepare_metadata(
            &front.uid,
            previous.revision.checked_add(1).ok_or_else(|| {
                PennelError::storage("The transcript revision cannot be advanced.")
            })?,
            replacement_transcript.as_bytes(),
            &rows,
        )?;
        let backups = root.join(TRANSCRIPT_BACKUPS);
        if fs::symlink_metadata(&backups).is_ok_and(|entry| entry.file_type().is_symlink()) {
            return Err(PennelError::storage(
                "The recovery folder is not a valid local folder.",
            ));
        }
        let pending = PendingReplacement {
            version: 1,
            backup: uuid::Uuid::now_v7().to_string(),
            had_failed: move_failed_audio && root.join("failed").is_dir(),
        };
        let backup = backups.join(&pending.backup);
        fs::create_dir_all(&backup).map_err(PennelError::storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&backups, fs::Permissions::from_mode(0o700))
                .map_err(PennelError::storage)?;
            fs::set_permissions(&backup, fs::Permissions::from_mode(0o700))
                .map_err(PennelError::storage)?;
        }
        durable_write(&backup.join("transcript.md"), &original)?;
        durable_write(&backup.join("meeting.md"), &metadata)?;
        meeting_utterances::backup_metadata(&root, &backup)?;
        crate::meeting_recording::backup_recording_metadata(&root, &backup)?;
        meeting_utterances::install_parts(&root, &replacement_attribution)?;
        let journal = serde_json::to_vec(&pending).map_err(PennelError::storage)?;
        durable_write(&backup.join("replacement.json"), &journal)?;
        sync_directory(&backups)?;
        sync_directory(&root)?;
        // This must be durable before moving recovery audio or changing names/text.
        durable_write(&root.join(REPLACEMENT_JOURNAL), &journal)?;
        let failed = root.join("failed");
        let result = (|| {
            if pending.had_failed {
                fs::rename(&failed, backup.join("failed")).map_err(PennelError::storage)?;
                sync_directory(&backup)?;
                sync_directory(&root)?;
            }
            durable_write(&root.join("meeting.md"), replacement_metadata.as_bytes())?;
            durable_write(
                &root.join("transcript.md"),
                replacement_transcript.as_bytes(),
            )?;
            meeting_utterances::install_manifest(&root, &replacement_attribution)
        })();
        if let Err(error) = result {
            if rollback_replacement(&root, &pending).is_err() {
                return Err(PennelError::storage(
                    "Recovery could not be completed. The previous transcript is in the local recovery folder.",
                ));
            }
            return Err(error);
        }
        Ok(backup)
    }

    pub(crate) fn restore_retranscription(
        &self,
        id: &str,
        backup: &Path,
    ) -> Result<(), PennelError> {
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        let journal = fs::read(backup.join("replacement.json")).map_err(PennelError::storage)?;
        let pending: PendingReplacement =
            serde_json::from_slice(&journal).map_err(PennelError::storage)?;
        if pending.backup_path(&root)? != backup {
            return Err(PennelError::storage(
                "The recovery folder is not a valid local folder.",
            ));
        }
        durable_write(&root.join(REPLACEMENT_JOURNAL), &journal)?;
        rollback_replacement(&root, &pending)
    }

    pub(crate) fn finish_retranscription(&self, id: &str) -> Result<(), PennelError> {
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        clear_replacement_journal(&root)?;
        let _ = meeting_utterances::prune_owned_parts(&root);
        Ok(())
    }

    /// Called only while acquiring the library lease, before reads, indexing or sync.
    pub(crate) fn recover_retranscriptions(&self) -> Result<(), PennelError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(PennelError::storage(error)),
        };
        for entry in entries {
            let entry = entry.map_err(PennelError::storage)?;
            if !entry.file_type().map_err(PennelError::storage)?.is_dir() {
                continue;
            }
            let root = entry.path();
            let gate = replacement_gate(&root);
            let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
            meeting_utterances::recover_snapshot(&root)?;
            let journal = match fs::read(root.join(REPLACEMENT_JOURNAL)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(PennelError::storage(error)),
            };
            let pending: PendingReplacement =
                serde_json::from_slice(&journal).map_err(PennelError::storage)?;
            rollback_replacement(&root, &pending)?;
        }
        Ok(())
    }

    pub fn read_notes(&self, id: &str) -> String {
        self.read_file(id, "notes.md")
    }

    #[allow(dead_code)] // Text-only compatibility callers retain this projection.
    pub fn write_transcript(&self, id: &str, segments: &[Segment]) -> Result<(), PennelError> {
        let legacy_source = self.unique_legacy_source(id);
        let (original_uid, uid) = self.transcript_write_uid(id)?;
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        let original = transcript_bytes(&root)?;
        refuse_unparsed(&original)?;
        let mut previous = snapshot_from_text(&root, &original_uid, &original);
        establish_legacy_source(&mut previous, legacy_source);
        meeting_utterances::assert_writable(&previous)?;
        let mut sorted = segments.to_vec();
        sorted.sort_by_key(|s| s.start_ms);
        let rows = meeting_utterances::reconcile(&previous, &sorted)?;
        self.write_snapshot_locked(id, &uid, &previous, rows)
            .map(|_| ())
    }

    pub fn read_transcript_snapshot(
        &self,
        id: &str,
    ) -> Result<MeetingTranscriptSnapshot, PennelError> {
        let legacy_source = self.unique_legacy_source(id);
        // Existing meetings without an id remain readable without a read-side migration.
        let uid = self.uid_of(id).unwrap_or_else(|| legacy_meeting_uid(id));
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        if !root.is_dir() {
            return Err(PennelError::NotFound);
        }
        let original = transcript_bytes(&root)?;
        let mut snapshot = snapshot_from_text(&root, &uid, &original);
        establish_legacy_source(&mut snapshot, legacy_source);
        self.add_recovery_utterances(id, &mut snapshot);
        snapshot
            .utterances
            .retain(|row| row.segment.failed || !row.segment.text.trim().is_empty());
        snapshot.utterances.sort_by_key(|row| row.segment.start_ms);
        Ok(snapshot)
    }

    /// Exact durable rows, including private completion markers, without synthetic recovery rows.
    pub(crate) fn read_committed_transcript_snapshot(
        &self,
        id: &str,
    ) -> Result<MeetingTranscriptSnapshot, PennelError> {
        let legacy_source = self.unique_legacy_source(id);
        let uid = self.uid_of(id).unwrap_or_else(|| legacy_meeting_uid(id));
        let root = self.dir_of(id);
        let gate = replacement_gate(&root);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        if !root.is_dir() {
            return Err(PennelError::NotFound);
        }
        let original = transcript_bytes(&root)?;
        let mut snapshot = snapshot_from_text(&root, &uid, &original);
        establish_legacy_source(&mut snapshot, legacy_source);
        Ok(snapshot)
    }

    pub(crate) fn recovery_path_may_exist(&self, id: &str, path: &Path) -> bool {
        let root = self.dir_of(id);
        if path.parent() != Some(root.join("failed").as_path()) {
            return true;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return true;
        };
        recovery_entry_may_exist(&root, name)
    }

    pub fn write_transcript_snapshot(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        rows: &[MeetingUtterance],
    ) -> Result<MeetingTranscriptSnapshot, PennelError> {
        let legacy_source = self.unique_legacy_source(id);
        let (original_uid, uid) = self.transcript_write_uid(id)?;
        let gate = replacement_gate(&self.dir_of(id));
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        let original = transcript_bytes(&root)?;
        refuse_unparsed(&original)?;
        let mut previous = snapshot_from_text(&root, &original_uid, &original);
        establish_legacy_source(&mut previous, legacy_source);
        meeting_utterances::assert_writable(&previous)?;
        if expected_revision.is_some_and(|revision| revision != previous.revision) {
            return Err(PennelError::storage(
                "The transcript changed before this edit could be saved.",
            ));
        }
        self.write_snapshot_locked(id, &uid, &previous, rows.to_vec())
    }

    fn write_snapshot_locked(
        &self,
        id: &str,
        uid: &str,
        previous: &MeetingTranscriptSnapshot,
        mut rows: Vec<MeetingUtterance>,
    ) -> Result<MeetingTranscriptSnapshot, PennelError> {
        let root = self.dir_of(id);
        if root.join(REPLACEMENT_JOURNAL).exists() {
            return Err(PennelError::storage(
                "The previous transcript replacement needs recovery before saving.",
            ));
        }
        // Empty successful rows can contain private filtered text dispositions. Their origin's
        // unsynced PCM may still exist even when this device has no recovery directory.
        preserve_completion_markers(previous, &mut rows);
        meeting_utterances::refresh_for_update(previous, &mut rows)?;
        rows.sort_by_key(|row| row.segment.start_ms);
        let segments: Vec<_> = rows.iter().map(|row| row.segment.clone()).collect();
        let text = render_transcript(&segments);
        let metadata = meeting_utterances::prepare_metadata(
            uid,
            previous.revision.checked_add(1).ok_or_else(|| {
                PennelError::storage("The transcript revision cannot be advanced.")
            })?,
            text.as_bytes(),
            &rows,
        )?;
        meeting_utterances::commit_snapshot(&root, text.as_bytes(), &metadata)?;
        let mut snapshot = snapshot_from_text(&root, uid, text.as_bytes());
        snapshot
            .utterances
            .retain(|row| row.segment.failed || !row.segment.text.trim().is_empty());
        Ok(snapshot)
    }

    fn add_recovery_utterances(&self, id: &str, snapshot: &mut MeetingTranscriptSnapshot) {
        let root = self.dir_of(id);
        let files = recovery_files(&root);
        for (path, actual) in &files {
            let mut proof = actual.clone();
            if completed_recovery(snapshot, path, &proof) {
                continue;
            }
            let Some(recovered) = recovery_audio(path) else {
                continue;
            };
            let unknown_failed = snapshot
                .utterances
                .iter()
                .filter(|row| {
                    row.segment.failed
                        && row.metadata.source_id.is_none()
                        && row.metadata.start_ms == proof.start_ms
                })
                .count();
            let files_at_start = files
                .iter()
                .filter(|(_, candidate)| candidate.start_ms == proof.start_ms)
                .count();
            let existing = snapshot.utterances.iter().position(|row| {
                row.segment.failed
                    && row.metadata.start_ms == proof.start_ms
                    && (row.metadata.recovery_audio.as_ref().is_some_and(|old| {
                        old.file_name == proof.file_name && old.track == proof.track
                    }) || row.metadata.source_id.is_some()
                        && proof
                            .track
                            .accepts_source(row.metadata.source_id.as_deref())
                        || row.metadata.source_id.is_none()
                            && unknown_failed == 1
                            && files_at_start == 1)
            });
            if let Some(index) = existing {
                let unbound_legacy = snapshot.utterances[index]
                    .metadata
                    .recovery_audio
                    .as_ref()
                    .is_none_or(|old| old.sha256.is_empty());
                let ambiguous_completion = unbound_legacy
                    && snapshot.utterances.iter().any(|row| {
                        !row.segment.failed
                            && proof
                                .track
                                .accepts_source(row.metadata.source_id.as_deref())
                            && row.metadata.start_ms < proof.end_ms
                            && row.metadata.end_ms >= proof.start_ms
                    });
                let row = &mut snapshot.utterances[index];
                if let Some(old) = &row.metadata.recovery_audio {
                    if !old.same_chunk(&proof)
                        || old.disposition
                            == crate::meeting_utterances::RecoveryAudioDisposition::Unverified
                    {
                        proof = old.clone();
                        proof.disposition =
                            crate::meeting_utterances::RecoveryAudioDisposition::Unverified;
                    } else {
                        proof.sha256 = old.sha256.clone();
                    }
                } else {
                    row.segment.end_ms = recovered.end_ms;
                }
                if ambiguous_completion {
                    proof.disposition =
                        crate::meeting_utterances::RecoveryAudioDisposition::Unverified;
                }
                row.metadata.refresh_fingerprints(&row.segment);
                if let Some(manual) = row.metadata.manual.as_mut()
                    && manual.source_fingerprint != row.metadata.source_fingerprint
                {
                    manual.review_required = true;
                }
                row.metadata.recovery_audio = Some(proof.clone());
                continue;
            }
            // A display label or an acoustic subrange cannot prove that this whole WAV completed.
            // Retain overlapping legacy successes for review instead of automatically retransmitting.
            if snapshot.utterances.iter().any(|row| {
                !row.segment.failed
                    && proof
                        .track
                        .accepts_source(row.metadata.source_id.as_deref())
                    && row.metadata.start_ms < proof.end_ms
                    && row.metadata.end_ms >= proof.start_ms
            }) {
                proof.disposition = crate::meeting_utterances::RecoveryAudioDisposition::Unverified;
            }
            let mut row = meeting_utterances::legacy_utterance(
                &snapshot.meeting_uid,
                &proof.file_name,
                0,
                recovered,
            );
            row.metadata.source_id = match proof.track {
                crate::meeting_utterances::RecoveryAudioTrack::Microphone => {
                    Some("microphone".into())
                }
                crate::meeting_utterances::RecoveryAudioTrack::OtherCapture => None,
            };
            row.metadata.recovery_audio = Some(proof.clone());
            row.metadata.refresh_fingerprints(&row.segment);
            snapshot.utterances.push(row);
        }
    }

    pub(crate) fn failed_recovery_plan(
        &self,
        id: &str,
        snapshot: &MeetingTranscriptSnapshot,
    ) -> FailedTranscriptionRecoveryPlan {
        if !matches!(
            snapshot.metadata_status,
            crate::TranscriptMetadataStatus::Valid | crate::TranscriptMetadataStatus::Absent
        ) {
            return FailedTranscriptionRecoveryPlan::metadata_unavailable();
        }
        let failed = snapshot
            .utterances
            .iter()
            .filter(|row| row.segment.failed)
            .collect::<Vec<_>>();
        let Ok(failed_count) = u32::try_from(failed.len()) else {
            return FailedTranscriptionRecoveryPlan::metadata_unavailable();
        };
        let mut retryable_count = 0;
        let mut reason_code: Option<FailedTranscriptionRecoveryReason> = None;
        for row in failed {
            match self.recovery_audio_candidate(id, row) {
                Ok(_) => retryable_count += 1,
                Err(reason) => {
                    if reason_code.is_none_or(|previous| reason.priority() > previous.priority()) {
                        reason_code = Some(reason);
                    }
                }
            }
        }
        FailedTranscriptionRecoveryPlan {
            failed_count: Some(failed_count),
            retryable_count: Some(retryable_count),
            status: if failed_count == 0 {
                FailedTranscriptionRecoveryStatus::Complete
            } else if retryable_count > 0 {
                FailedTranscriptionRecoveryStatus::Retryable
            } else {
                FailedTranscriptionRecoveryStatus::Unavailable
            },
            reason_code,
        }
    }

    /// Header-only selection keeps ordinary detail reads independent of retained PCM size.
    /// Recognition must verify the complete bytes again before and after its provider call.
    pub(crate) fn recovery_audio_candidate(
        &self,
        id: &str,
        row: &MeetingUtterance,
    ) -> Result<PathBuf, FailedTranscriptionRecoveryReason> {
        use FailedTranscriptionRecoveryReason as Reason;
        let proof = row
            .metadata
            .recovery_audio
            .as_ref()
            .ok_or(Reason::NoRetainedAudio)?;
        if !proof.valid()
            || proof.disposition != crate::meeting_utterances::RecoveryAudioDisposition::Pending
        {
            return Err(Reason::UnverifiedProvenance);
        }
        let directory = self.dir_of(id).join("failed");
        let unavailable = |error: std::io::Error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Reason::NoRetainedAudio
            } else {
                Reason::UnreadableAudio
            }
        };
        let metadata = fs::symlink_metadata(&directory).map_err(unavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(Reason::UnverifiedProvenance);
        }
        if !metadata.is_dir() {
            return Err(Reason::UnreadableAudio);
        }
        let path = directory.join(&proof.file_name);
        let metadata = fs::symlink_metadata(&path).map_err(unavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(Reason::UnverifiedProvenance);
        }
        if !metadata.is_file() {
            return Err(Reason::UnreadableAudio);
        }
        let actual = recovery_header_provenance(&path).ok_or(Reason::UnreadableAudio)?;
        if !proof.same_chunk(&actual) {
            return Err(Reason::AudioChanged);
        }
        Ok(path)
    }

    #[cfg(test)]
    pub(crate) fn recovery_audio_path(&self, id: &str, row: &MeetingUtterance) -> Option<PathBuf> {
        let path = self.recovery_audio_candidate(id, row).ok()?;
        let proof = row.metadata.recovery_audio.as_ref()?;
        let actual = recovery_provenance(&path, proof.disposition)?;
        (proof.same_chunk(&actual) && (proof.sha256.is_empty() || proof.sha256 == actual.sha256))
            .then_some(path)
    }

    fn transcript_write_uid(&self, id: &str) -> Result<(String, String), PennelError> {
        if let Some(uid) = self.uid_of(id) {
            return Ok((uid.clone(), uid));
        }
        let original_uid = legacy_meeting_uid(id);
        {
            let gate = replacement_gate(&self.dir_of(id));
            let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
            let root = self.dir_of(id);
            let bytes = transcript_bytes(&root)?;
            refuse_unparsed(&bytes)?;
            meeting_utterances::assert_writable(&snapshot_from_text(&root, &original_uid, &bytes))?;
        }
        Ok((original_uid, self.ensure_uid(id)?))
    }

    fn unique_legacy_source(&self, id: &str) -> Option<String> {
        let Ok(tracks) = self.audio_tracks(id) else {
            return None;
        };
        if tracks.len() != 1 || !tracks[0].available {
            return None;
        }
        Some(
            match tracks[0].source {
                crate::meeting_recording::RecordingSource::Microphone => "microphone",
                crate::meeting_recording::RecordingSource::SystemAudio => "system_audio",
                crate::meeting_recording::RecordingSource::RoomMicrophone => "room_microphone",
            }
            .into(),
        )
    }

    /// Number of non-empty transcript lines that did not parse; rewriting the transcript would drop them.
    pub fn unparsed_transcript_lines(&self, id: &str) -> usize {
        self.read_file(id, "transcript.md")
            .lines()
            .filter(|l| !l.trim().is_empty() && parse_segment(l).is_none())
            .count()
    }

    pub fn read_transcript(&self, id: &str) -> Vec<Segment> {
        self.read_transcript_snapshot(id)
            .map(|snapshot| {
                snapshot
                    .utterances
                    .into_iter()
                    .map(|row| {
                        let mut segment = row.segment;
                        segment.text = segment
                            .text
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ");
                        segment
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn restore_recovery_duration(&self, id: &str) -> Result<(), PennelError> {
        let info = self.info(id).ok_or(PennelError::NotFound)?;
        if info.duration_min > 0 {
            return Ok(());
        }
        let mut longest = self
            .read_transcript(id)
            .iter()
            .map(|segment| segment.end_ms)
            .max()
            .unwrap_or(0);
        // Successful empty rows are intentionally hidden from the visible transcript. Their saved
        // audio still proves the recording's duration when a crash interrupted finalization.
        if let Ok(entries) = fs::read_dir(self.dir_of(id).join("failed")) {
            for entry in entries.filter_map(Result::ok) {
                if entry.file_type().is_ok_and(|kind| kind.is_file())
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("wav")
                    && let Some(segment) = recovery_audio(&entry.path())
                {
                    longest = longest.max(segment.end_ms);
                }
            }
        }
        if longest > 0 {
            self.set_duration(id, longest.div_ceil(60_000).min(u64::from(u32::MAX)) as u32)?;
        }
        Ok(())
    }

    pub(crate) fn cleanup_committed_audio(&self, id: &str) -> Result<u32, PennelError> {
        let mut removed = 0;
        let mut failure = None;
        let root = self.dir_of(id);
        let uid = self.uid_of(id).unwrap_or_else(|| legacy_meeting_uid(id));
        let gate = replacement_gate(&root);
        let _gate = gate.lock().unwrap_or_else(|error| error.into_inner());
        let transcript = transcript_bytes(&root)?;
        let snapshot = snapshot_from_text(&root, &uid, &transcript);
        meeting_utterances::assert_writable(&snapshot)?;
        for (path, proof) in recovery_files(&root) {
            if !completed_recovery(&snapshot, &path, &proof) {
                continue;
            }
            match fs::remove_file(path) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    failure.get_or_insert_with(|| PennelError::storage(error));
                }
            }
        }
        failure.map_or(Ok(removed), Err)
    }

    fn frontmatter(&self, id: &str) -> Option<(Frontmatter, String)> {
        parse_meeting(&self.read_file(id, "meeting.md"))
    }

    #[cfg(any(test, feature = "evaluation"))]
    pub fn write_meeting(
        &self,
        id: &str,
        duration_min: u32,
        template: &str,
        body: &str,
    ) -> Result<(), PennelError> {
        let (mut front, _) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        front.duration_min = duration_min;
        front.template = template.to_owned();
        front
            .extra
            .retain(|line| !line.starts_with("pennel_summary_outdated:"));
        self.write_file(id, "meeting.md", &render_meeting(&front, body))
    }

    pub fn set_attendees(&self, id: &str, attendees: &[String]) -> Result<(), PennelError> {
        let (mut front, body) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        front.extra.retain(|line| !line.starts_with("attendees:"));
        if !attendees.is_empty() {
            let names: Vec<String> = attendees
                .iter()
                .map(|a| a.replace([',', '[', ']'], " ").trim().to_owned())
                .collect();
            front
                .extra
                .push(format!("attendees: [{}]", names.join(", ")));
        }
        self.write_file(id, "meeting.md", &render_meeting(&front, &body))
    }

    pub fn attendees(&self, id: &str) -> Vec<String> {
        let Some((front, _)) = self.frontmatter(id) else {
            return Vec::new();
        };
        front
            .extra
            .iter()
            .find_map(|line| line.strip_prefix("attendees:"))
            .map(|list| {
                list.trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn speaker_names(&self, id: &str) -> Vec<SpeakerName> {
        let Some((front, _)) = self.frontmatter(id) else {
            return Vec::new();
        };
        let mut names: Vec<SpeakerName> = front
            .extra
            .iter()
            .find_map(|line| line.strip_prefix("speakers:"))
            .map(|list| {
                list.split(';')
                    .filter_map(|pair| {
                        let (voice, name) = pair.split_once('=')?;
                        let voice = voice.trim().parse().ok().filter(|v| VOICES.contains(v))?;
                        Some(SpeakerName {
                            voice,
                            name: name.trim().to_owned(),
                        })
                    })
                    .filter(|n| !n.name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by_key(|n| n.voice);
        names
    }

    /// An empty name forgets the voice's name.
    pub fn rename_speaker(&self, id: &str, voice: u32, name: &str) -> Result<(), PennelError> {
        if !VOICES.contains(&voice) {
            return Err(PennelError::NotFound);
        }
        let mut names = self.speaker_names(id);
        names.retain(|n| n.voice != voice);
        names.push(SpeakerName {
            voice,
            name: name.to_owned(),
        });
        let mut snapshot = self.read_transcript_snapshot(id)?;
        if snapshot
            .utterances
            .iter()
            .any(|row| row.segment.voice == Some(voice))
        {
            let clean = sanitize_speaker(name);
            for row in &mut snapshot.utterances {
                if row.segment.voice == Some(voice) {
                    row.metadata.manual = Some(crate::meeting_utterances::ManualCorrection {
                        name: (!clean.is_empty()).then(|| clean.clone()),
                        voice: Some(voice),
                        source_fingerprint: row.metadata.source_fingerprint.clone(),
                        review_required: false,
                        self_voice: false,
                    });
                    row.metadata.attribution_revision =
                        row.metadata.attribution_revision.saturating_add(1);
                }
            }
            let tracks = self.transcript_tracks(id)?.unwrap_or_default();
            let backup =
                self.replace_speaker_utterances(id, &snapshot.utterances, &names, &tracks)?;
            if let Err(error) = self.finish_retranscription(id) {
                self.restore_retranscription(id, &backup)?;
                return Err(error);
            }
            Ok(())
        } else {
            self.set_speaker_names(id, &names)
        }
    }

    /// Replaces every voice name; empty names and voices outside [`VOICES`] are dropped.
    pub fn set_speaker_names(&self, id: &str, names: &[SpeakerName]) -> Result<(), PennelError> {
        let (mut front, body) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        set_names(&mut front, names);
        self.write_file(id, "meeting.md", &render_meeting(&front, &body))
    }

    pub fn set_duration(&self, id: &str, duration_min: u32) -> Result<(), PennelError> {
        let (mut front, body) = self.frontmatter(id).ok_or(PennelError::NotFound)?;
        front.duration_min = duration_min;
        self.write_file(id, "meeting.md", &render_meeting(&front, &body))
    }

    pub fn read_meeting(&self, id: &str) -> Option<String> {
        self.frontmatter(id)
            .map(|(_, body)| body)
            .filter(|b| !b.trim().is_empty())
    }

    pub fn info(&self, id: &str) -> Option<MeetingInfo> {
        let (front, body) = self.frontmatter(id)?;
        Some(MeetingInfo {
            id: id.to_owned(),
            title: front.title,
            start_iso: front.start_iso,
            duration_min: front.duration_min,
            has_summary: !body.trim().is_empty(),
        })
    }

    pub fn list(&self) -> Vec<MeetingInfo> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut list: Vec<MeetingInfo> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| self.info(&e.file_name().to_string_lossy()))
            .collect();
        list.sort_by(|a, b| b.start_iso.cmp(&a.start_iso).then(b.id.cmp(&a.id)));
        list
    }

    pub fn search(&self, query: &str) -> Vec<MeetingInfo> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return self.list();
        }
        self.list()
            .into_iter()
            .filter(|m| {
                [
                    m.title.clone(),
                    self.read_notes(&m.id),
                    self.read_file(&m.id, "transcript.md"),
                    self.read_file(&m.id, "meeting.md"),
                ]
                .iter()
                .any(|text| text.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub fn rename(&self, id: &str, title: &str) -> Result<String, PennelError> {
        self.rename_with_write(id, title, |target, metadata| {
            self.write_file(target, "meeting.md", metadata)
        })
    }

    fn rename_with_write(
        &self,
        id: &str,
        title: &str,
        persist: impl FnOnce(&str, &str) -> Result<(), PennelError>,
    ) -> Result<String, PennelError> {
        let original = self.read_file(id, "meeting.md");
        let (mut front, body) = parse_meeting(&original).ok_or(PennelError::NotFound)?;
        let prefix: String = id.chars().take("YYYY-MM-DD HHmm ".len()).collect();
        let base = format!("{prefix}{}", folder_name(title));
        let new_id = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} ({n})")))
            .find(|candidate| candidate == id || !self.dir.join(candidate).exists())
            .expect("unbounded iterator");
        front.title = sanitize(title);
        // A failed metadata write must leave the retained session's original folder usable.
        persist(id, &render_meeting(&front, &body))?;
        if new_id != id
            && let Err(error) = std::fs::rename(self.dir_of(id), self.dir_of(&new_id))
        {
            let _ = self.write_file(id, "meeting.md", &original);
            return Err(PennelError::storage(error));
        }
        Ok(new_id)
    }

    pub fn delete(&self, id: &str) -> Result<(), PennelError> {
        std::fs::remove_dir_all(self.dir_of(id)).map_err(PennelError::storage)
    }

    pub fn prune_audio(&self, retention_days: u32, now: SystemTime) -> u32 {
        if retention_days == 0 {
            return 0;
        }
        let max_age = std::time::Duration::from_secs(u64::from(retention_days) * 86_400);
        let mut removed = 0;
        for meeting in self.list() {
            let root = self.dir_of(&meeting.id);
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            let mut paths: Vec<_> = entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "m4a"))
                .collect();
            if let Ok(tracks) = self.audio_tracks(&meeting.id) {
                paths.extend(
                    tracks
                        .into_iter()
                        .filter(|track| track.available)
                        .map(|track| root.join(track.relative_path)),
                );
            }
            paths.sort();
            paths.dedup();
            for path in paths {
                let old = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age >= max_age);
                if old && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }
}

pub(crate) fn render_transcript(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| {
            let text = if s.failed {
                FAILED_MARK.to_owned()
            } else {
                s.text.split_whitespace().collect::<Vec<_>>().join(" ")
            };
            let label = match (s.speaker, s.voice) {
                (Speaker::Others, Some(voice)) => format!("Anderen {voice}"),
                _ => s.speaker.label().to_owned(),
            };
            format!(
                "[{}] **{label}:** {text}\n",
                transcript_timestamp(s.start_ms)
            )
        })
        .collect()
}

fn transcript_bytes(root: &Path) -> Result<Vec<u8>, PennelError> {
    match fs::read(root.join("transcript.md")) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(PennelError::storage(error)),
    }
}

fn legacy_meeting_uid(id: &str) -> String {
    format!(
        "legacy-{}",
        &meeting_utterances::digest(id.as_bytes())[..32]
    )
}

fn establish_legacy_source(snapshot: &mut MeetingTranscriptSnapshot, source: Option<String>) {
    if snapshot.metadata_status != crate::meeting_utterances::TranscriptMetadataStatus::Absent {
        return;
    }
    let Some(source) = source else {
        return;
    };
    for row in &mut snapshot.utterances {
        row.metadata.source_id = Some(source.clone());
        row.metadata.refresh_fingerprints(&row.segment);
    }
}

fn refuse_unparsed(bytes: &[u8]) -> Result<(), PennelError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        PennelError::storage("The transcript contains unreadable text; the original file was kept.")
    })?;
    if text
        .lines()
        .any(|line| !line.trim().is_empty() && parse_segment(line).is_none())
    {
        return Err(PennelError::storage(
            "The transcript contains unrecognized lines. Keep the original text before changing attribution.",
        ));
    }
    Ok(())
}

fn snapshot_from_text(root: &Path, uid: &str, bytes: &[u8]) -> MeetingTranscriptSnapshot {
    let mut occurrences = std::collections::HashMap::new();
    let fallback = String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let segment = parse_segment(line)?;
            let occurrence = occurrences.entry(line.to_owned()).or_insert(0);
            let row = meeting_utterances::legacy_utterance(uid, line, *occurrence, segment);
            *occurrence += 1;
            Some(row)
        })
        .collect();
    meeting_utterances::load_snapshot(root, uid, bytes, fallback)
}

fn set_names(front: &mut Frontmatter, names: &[SpeakerName]) {
    let mut names: Vec<SpeakerName> = names
        .iter()
        .filter(|n| VOICES.contains(&n.voice))
        .map(|n| SpeakerName {
            voice: n.voice,
            name: sanitize_speaker(&n.name),
        })
        .filter(|n| !n.name.is_empty())
        .collect();
    names.sort_by_key(|n| n.voice);
    names.dedup_by_key(|n| n.voice);
    front.extra.retain(|line| !line.starts_with("speakers:"));
    if !names.is_empty() {
        let list: Vec<String> = names
            .iter()
            .map(|n| format!("{}={}", n.voice, n.name))
            .collect();
        front.extra.push(format!("speakers: {}", list.join("; ")));
    }
}

fn set_transcript_tracks(front: &mut Frontmatter, tracks: &[Speaker]) {
    front
        .extra
        .retain(|line| !line.starts_with("pennel_transcript_tracks:"));
    let sources: Vec<_> = tracks
        .iter()
        .map(|speaker| match speaker {
            Speaker::Me => "microphone",
            Speaker::Others => "system",
        })
        .collect();
    if !sources.is_empty() {
        front
            .extra
            .push(format!("pennel_transcript_tracks: {}", sources.join(",")));
    }
}

fn render_meeting(front: &Frontmatter, body: &str) -> String {
    let uid = if front.uid.is_empty() {
        String::new()
    } else {
        format!("{UID_FIELD}: {}\n", front.uid)
    };
    let extra: String = front.extra.iter().map(|line| format!("{line}\n")).collect();
    // Once edited as a document, whitespace belongs to the owner (including trailing newlines).
    let exact = front
        .extra
        .iter()
        .any(|line| line.trim() == "pennel_summary_format: exact_v1");
    let content = if exact {
        body.to_owned()
    } else {
        format!("\n{}\n", body.trim())
    };
    format!(
        "---\ntitle: {}\nstart: {}\nduration_min: {}\ntemplate: {}\n{uid}{extra}---\n{}",
        front.title, front.start_iso, front.duration_min, front.template, content
    )
}

const UID_FIELD: &str = "pennel_id";
pub(crate) const UID_RACE: &str = "meeting.md changed while adding its id";
const KNOWN_FIELDS: &[&str] = &["title", "start", "duration_min", "template", UID_FIELD];

/// Uids become part of sync keys (`meeting/<uid>/…`), so only plain id characters are accepted.
pub(crate) fn valid_uid(uid: &str) -> bool {
    (1..=64).contains(&uid.len()) && uid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Head, body and the line ending the front matter uses: files edited on Windows use CRLF.
fn split_frontmatter(text: &str) -> Option<(&str, &str, &'static str)> {
    let newline = if text.starts_with("---\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let rest = text.strip_prefix("---")?.strip_prefix(newline)?;
    let (head, body) = rest.split_once(&format!("{newline}---{newline}"))?;
    Some((head, body, newline))
}

fn parse_meeting(text: &str) -> Option<(Frontmatter, String)> {
    let (head, body, _) = split_frontmatter(text)?;
    let field = |key: &str| {
        head.lines()
            .find_map(|l| {
                l.strip_prefix(key)
                    .and_then(|v| v.strip_prefix(": ").or(v.strip_prefix(":")))
            })
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let front = Frontmatter {
        title: field("title"),
        start_iso: field("start"),
        duration_min: field("duration_min").parse().unwrap_or(0),
        template: field("template"),
        uid: field(UID_FIELD),
        extra: head
            .lines()
            .filter(|l| {
                !KNOWN_FIELDS
                    .iter()
                    .any(|k| l.split(':').next().is_some_and(|key| key.trim() == *k))
            })
            .map(str::to_owned)
            .collect(),
    };
    let exact = front
        .extra
        .iter()
        .any(|line| line.trim() == "pennel_summary_format: exact_v1");
    Some((
        front,
        if exact {
            body.to_owned()
        } else {
            body.trim().to_owned()
        },
    ))
}

fn parse_segment(line: &str) -> Option<Segment> {
    let rest = line.strip_prefix('[')?;
    let (stamp, rest) = rest.split_once("] **")?;
    let (label, text) = rest.split_once(":** ")?;
    let (speaker, voice) = match label {
        "Ik" => (Speaker::Me, None),
        "Anderen" => (Speaker::Others, None),
        other => (
            Speaker::Others,
            Some(
                other
                    .strip_prefix("Anderen ")?
                    .parse()
                    .ok()
                    .filter(|v| VOICES.contains(v))?,
            ),
        ),
    };
    let start_ms = parse_timestamp(stamp)?;
    let failed = text.trim() == FAILED_MARK;
    Some(Segment {
        start_ms,
        end_ms: start_ms,
        speaker,
        text: if failed {
            String::new()
        } else {
            text.to_owned()
        },
        failed,
        voice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn recovery_cleanup_does_not_follow_a_person_label_to_another_track() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned ambiguous recovery lanes", start())
            .unwrap();
        let microphone = store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.2; 16_000], 16_000),
            )
            .unwrap();
        let system = store
            .write_recovery(
                &id,
                Speaker::Others,
                0,
                &crate::wav::encode_wav(&vec![0.3; 16_000], 16_000),
            )
            .unwrap();
        let row = crate::MeetingUtterance::new(
            Segment {
                start_ms: 0,
                end_ms: 1_000,
                speaker: Speaker::Others,
                text: "Microphone speech with a neutral person label.".into(),
                failed: false,
                voice: Some(1),
            },
            Some("microphone".into()),
        );
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        store.cleanup_committed_audio(&id).unwrap();
        assert!(
            system.exists(),
            "A person label deleted the other capture lane"
        );
        assert!(
            microphone.exists(),
            "Unproven recovery completion must remain"
        );
    }

    #[test]
    fn meeting_storage_gates_do_not_block_other_meetings_and_survive_folder_rename() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let a = store.create("First independent meeting", start()).unwrap();
        let b = store.create("Second independent meeting", start()).unwrap();
        let gate = replacement_gate(&store.dir_of(&a));
        let held = gate.lock().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let other_root = directory.path().to_owned();
        let worker = std::thread::spawn(move || {
            let result = MeetingStore::new(&other_root)
                .write_transcript(&b, &[seg(0, Speaker::Others, "Independent final speech.")]);
            sent.send(result).unwrap();
        });
        received
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("Another meeting waited for unrelated metadata storage")
            .unwrap();
        drop(held);
        worker.join().unwrap();
        let renamed = store.rename(&a, "Renamed same meeting").unwrap();
        assert!(Arc::ptr_eq(
            &gate,
            &replacement_gate(&store.dir_of(&renamed))
        ));
    }

    #[test]
    fn queued_recovery_audio_is_discovered_after_restart_without_a_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Niet verwerkt", start()).unwrap();
        let path = crate::session::failed_path(&store.dir_of(&id), Speaker::Others, 12_000);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, crate::wav::encode_wav(&vec![0.25; 32_000], 16_000)).unwrap();
        let recovered = MeetingStore::new(dir.path()).read_transcript(&id);
        assert_eq!(recovered.len(), 1);
        assert!(recovered[0].failed);
        assert_eq!(recovered[0].speaker, Speaker::Others);
        assert_eq!(
            (recovered[0].start_ms, recovered[0].end_ms),
            (12_000, 14_000)
        );
        assert!(recovered[0].text.is_empty());
    }

    #[test]
    fn stored_subsecond_timestamps_preserve_legacy_display_and_parser_compatibility() {
        assert_eq!(timestamp(400), "00:00");
        assert_eq!(transcript_timestamp(400), "00:00.400");
        assert_eq!(transcript_timestamp(60_000), "01:00");
        assert_eq!(parse_timestamp("00:00.400"), Some(400));
        assert_eq!(parse_timestamp("1:02:03.004"), Some(3_723_004));
        assert_eq!(parse_timestamp("00:01"), Some(1_000));
        assert_eq!(parse_timestamp("00:01.4"), None);
        assert_eq!(parse_timestamp("18446744073709551615:01"), None);
    }

    #[test]
    fn transcript_writes_do_not_replace_a_legacy_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Tijdelijk bestand", start()).unwrap();
        let temporary = store.dir_of(&id).join("transcript.tmp");
        fs::write(&temporary, b"ander werk").unwrap();
        store
            .write_transcript(&id, &[seg(0, Speaker::Me, "tekst")])
            .unwrap();
        assert_eq!(fs::read(temporary).unwrap(), b"ander werk");
        assert_eq!(store.read_transcript(&id)[0].text, "tekst");
    }

    #[test]
    fn committed_recovery_audio_restores_end_times_and_missing_duration() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Herstelde duur", start()).unwrap();
        let path = store
            .write_recovery(
                &id,
                Speaker::Others,
                60_400,
                &crate::wav::encode_wav(&vec![0.25; 32_000], 16_000),
            )
            .unwrap();
        let proof = recovery_provenance(
            &path,
            crate::meeting_utterances::RecoveryAudioDisposition::Complete,
        )
        .unwrap();
        let mut segment = seg(60_400, Speaker::Others, "hersteld");
        segment.end_ms = proof.end_ms;
        let mut row = MeetingUtterance::new(segment, Some("system_audio".into()));
        row.metadata.recovery_audio = Some(proof);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        assert_eq!(store.read_transcript(&id)[0].end_ms, 62_400);
        store.restore_recovery_duration(&id).unwrap();
        assert_eq!(store.info(&id).unwrap().duration_min, 2);
        store.cleanup_committed_audio(&id).unwrap();
        assert!(!crate::session::failed_path(&store.dir_of(&id), Speaker::Others, 60_400).exists());
    }

    #[test]
    fn recovery_cleanup_does_not_use_a_microphone_rows_person_label() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned recovery source fixture", start())
            .unwrap();
        let microphone = store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.2; 16_000], 16_000),
            )
            .unwrap();
        let system = store
            .write_recovery(
                &id,
                Speaker::Others,
                0,
                &crate::wav::encode_wav(&vec![0.4; 16_000], 16_000),
            )
            .unwrap();
        let mut recognized = seg(0, Speaker::Others, "Owned microphone speech.");
        recognized.voice = Some(2);
        let mut failed = seg(0, Speaker::Others, "");
        failed.failed = true;
        store
            .write_transcript_snapshot(
                &id,
                None,
                &[
                    MeetingUtterance::new(recognized, Some("microphone".into())),
                    MeetingUtterance::new(failed, Some("system_audio".into())),
                ],
            )
            .unwrap();
        store.cleanup_committed_audio(&id).unwrap();
        assert!(
            system.exists(),
            "the failed physical system chunk must survive"
        );
        assert!(
            microphone.exists(),
            "unproven original chunk audio is retained"
        );
    }

    fn proven_row(
        store: &MeetingStore,
        id: &str,
        source: Speaker,
        start_ms: u64,
        failed: bool,
    ) -> (PathBuf, MeetingUtterance) {
        let path = store
            .write_recovery(
                id,
                source,
                start_ms,
                &crate::wav::encode_wav(&vec![0.25; 16_000], 16_000),
            )
            .unwrap();
        let mut segment = seg(start_ms, source, if failed { "" } else { "Yes no" });
        segment.failed = failed;
        let mut row = MeetingUtterance::new(
            segment,
            Some(
                match source {
                    Speaker::Me => "microphone",
                    Speaker::Others => "system_audio",
                }
                .into(),
            ),
        );
        row.metadata.recovery_audio = recovery_provenance(
            &path,
            if failed {
                crate::meeting_utterances::RecoveryAudioDisposition::Pending
            } else {
                crate::meeting_utterances::RecoveryAudioDisposition::Complete
            },
        );
        (path, row)
    }

    #[test]
    fn failed_recovery_plan_counts_eligible_audio_without_reading_pcm_or_writing() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store.create("Owned recovery plan", start()).unwrap();
        let (_, pending) = proven_row(&store, &id, Speaker::Me, 0, true);
        let (_, mut unverified) = proven_row(&store, &id, Speaker::Others, 2_000, true);
        unverified
            .metadata
            .recovery_audio
            .as_mut()
            .unwrap()
            .disposition = crate::meeting_utterances::RecoveryAudioDisposition::Unverified;
        store
            .write_transcript_snapshot(&id, None, &[pending, unverified])
            .unwrap();
        store.write_notes(&id, "Private fixture note.").unwrap();
        let root = store.dir_of(&id);
        let before = ["transcript.md", "transcript.meta.json", "notes.md"]
            .map(|name| fs::read(root.join(name)).unwrap());
        let snapshot = store.read_transcript_snapshot(&id).unwrap();

        RECOVERY_PCM_READ_BYTES.with(|count| count.set(0));
        let plan = store.failed_recovery_plan(&id, &snapshot);

        assert_eq!(plan.failed_count, Some(2));
        assert_eq!(plan.retryable_count, Some(1));
        assert_eq!(plan.status, FailedTranscriptionRecoveryStatus::Retryable);
        assert_eq!(
            plan.reason_code,
            Some(FailedTranscriptionRecoveryReason::UnverifiedProvenance)
        );
        assert_eq!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get), 0);
        assert_eq!(
            ["transcript.md", "transcript.meta.json", "notes.md"]
                .map(|name| fs::read(root.join(name)).unwrap()),
            before
        );
        let public = serde_json::to_value(plan).unwrap();
        assert_eq!(public.as_object().unwrap().len(), 4);
        assert_eq!(public["status"], "retryable");
        assert_eq!(public["reason_code"], "unverified_provenance");
        assert!(!public.to_string().contains(".wav"));
        assert!(!public.to_string().contains("sha256"));
    }

    #[test]
    fn failed_recovery_plan_does_not_substitute_full_archives_for_missing_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store.create("Owned missing failed audio", start()).unwrap();
        let mut failed = seg(0, Speaker::Others, "");
        failed.failed = true;
        store.write_transcript(&id, &[failed]).unwrap();
        let audio = store.dir_of(&id).join("audio");
        fs::create_dir_all(&audio).unwrap();
        fs::write(audio.join("system.m4a"), b"owned archive placeholder").unwrap();

        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        let plan = store.failed_recovery_plan(&id, &snapshot);

        assert_eq!(plan.failed_count, Some(1));
        assert_eq!(plan.retryable_count, Some(0));
        assert_eq!(plan.status, FailedTranscriptionRecoveryStatus::Unavailable);
        assert_eq!(
            plan.reason_code,
            Some(FailedTranscriptionRecoveryReason::NoRetainedAudio)
        );
    }

    #[test]
    fn failed_recovery_plan_distinguishes_header_changes_and_unreadable_audio() {
        for changed_header in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store
                .create("Owned changed recovery audio", start())
                .unwrap();
            let (path, row) = proven_row(&store, &id, Speaker::Me, 0, true);
            store.write_transcript_snapshot(&id, None, &[row]).unwrap();
            let snapshot = store.read_transcript_snapshot(&id).unwrap();
            fs::write(
                path,
                if changed_header {
                    crate::wav::encode_wav(&vec![0.25; 32_000], 16_000)
                } else {
                    b"unreadable owned audio".to_vec()
                },
            )
            .unwrap();

            let plan = store.failed_recovery_plan(&id, &snapshot);

            assert_eq!(plan.failed_count, Some(1));
            assert_eq!(plan.retryable_count, Some(0));
            assert_eq!(plan.status, FailedTranscriptionRecoveryStatus::Unavailable);
            assert_eq!(
                plan.reason_code,
                Some(if changed_header {
                    FailedTranscriptionRecoveryReason::AudioChanged
                } else {
                    FailedTranscriptionRecoveryReason::UnreadableAudio
                })
            );
        }
    }

    #[test]
    fn failed_recovery_plan_keeps_counts_unknown_for_unavailable_metadata() {
        use crate::TranscriptMetadataStatus;
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned unknown recovery metadata", start())
            .unwrap();
        let (_, row) = proven_row(&store, &id, Speaker::Me, 0, true);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let mut snapshot = store.read_transcript_snapshot(&id).unwrap();
        for status in [
            TranscriptMetadataStatus::Pending,
            TranscriptMetadataStatus::Stale,
            TranscriptMetadataStatus::Corrupt,
            TranscriptMetadataStatus::Unsupported,
        ] {
            snapshot.metadata_status = status;
            let plan = store.failed_recovery_plan(&id, &snapshot);
            assert_eq!(plan.failed_count, None);
            assert_eq!(plan.retryable_count, None);
            assert_eq!(plan.status, FailedTranscriptionRecoveryStatus::Unavailable);
            assert_eq!(
                plan.reason_code,
                Some(FailedTranscriptionRecoveryReason::MetadataUnavailable)
            );
        }
    }

    #[test]
    fn failed_recovery_plan_reports_known_completion_without_recovery_audio() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store.create("Owned complete transcript", start()).unwrap();
        store
            .write_transcript(&id, &[seg(0, Speaker::Me, "Saved result.")])
            .unwrap();
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        let plan = store.failed_recovery_plan(&id, &snapshot);
        assert_eq!(plan.failed_count, Some(0));
        assert_eq!(plan.retryable_count, Some(0));
        assert_eq!(plan.status, FailedTranscriptionRecoveryStatus::Complete);
        assert_eq!(plan.reason_code, None);
    }

    #[test]
    fn recovery_completion_proof_selects_only_its_original_physical_file() {
        for source in [Speaker::Me, Speaker::Others] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store
                .create("Owned crossed recovery lanes", start())
                .unwrap();
            let other = if source == Speaker::Me {
                Speaker::Others
            } else {
                Speaker::Me
            };
            let (complete_path, mut complete) = proven_row(&store, &id, source, 0, false);
            let (failed_path, failed) = proven_row(&store, &id, other, 0, true);
            complete.segment.speaker = other;
            complete.segment.voice = Some(2);
            store
                .write_transcript_snapshot(&id, None, &[complete, failed])
                .unwrap();
            let reopened = MeetingStore::new(directory.path())
                .read_transcript_snapshot(&id)
                .unwrap();
            assert_eq!(
                reopened
                    .utterances
                    .iter()
                    .filter(|row| row.segment.failed)
                    .count(),
                1
            );
            let retry = reopened
                .utterances
                .iter()
                .find(|row| row.segment.failed)
                .unwrap();
            assert_eq!(
                store.recovery_audio_path(&id, retry),
                Some(failed_path.clone())
            );
            assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 1);
            assert!(!complete_path.exists());
            assert!(failed_path.exists());
        }
    }

    #[test]
    fn pending_and_unverified_recovery_listing_reads_headers_not_pcm_payloads() {
        use crate::meeting_utterances::RecoveryAudioDisposition;
        for disposition in [
            RecoveryAudioDisposition::Pending,
            RecoveryAudioDisposition::Unverified,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store
                .create("Owned bounded recovery listing", start())
                .unwrap();
            let (path, mut row) = proven_row(&store, &id, Speaker::Me, 0, true);
            row.metadata.recovery_audio.as_mut().unwrap().disposition = disposition;
            store.write_transcript_snapshot(&id, None, &[row]).unwrap();
            RECOVERY_PCM_READ_BYTES.with(|count| count.set(0));
            for _ in 0..3 {
                assert_eq!(
                    store
                        .read_transcript_snapshot(&id)
                        .unwrap()
                        .utterances
                        .len(),
                    1
                );
            }
            assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 0);
            assert_eq!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get), 0);
            let row = store
                .read_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .remove(0);
            if disposition == RecoveryAudioDisposition::Pending {
                assert_eq!(store.recovery_audio_path(&id, &row), Some(path));
                assert!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get) > 0);
            } else {
                assert!(store.recovery_audio_path(&id, &row).is_none());
                assert_eq!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get), 0);
            }
        }
    }

    #[test]
    fn compatibility_projection_cannot_promote_unverified_audio_to_pending() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned legacy projection recovery", start())
            .unwrap();
        store
            .write_transcript(&id, &[seg(0, Speaker::Others, "Already readable.")])
            .unwrap();
        let path = store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.2; 16_000], 16_000),
            )
            .unwrap();
        let public = store.read_transcript(&id);
        store.write_transcript(&id, &public).unwrap();
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        let failed = snapshot
            .utterances
            .iter()
            .find(|row| row.segment.failed)
            .unwrap();
        assert_eq!(
            failed.metadata.recovery_audio.as_ref().unwrap().disposition,
            crate::meeting_utterances::RecoveryAudioDisposition::Unverified
        );
        assert!(store.recovery_audio_path(&id, failed).is_none());
        assert!(path.exists());
    }

    #[test]
    fn a_physical_lane_without_original_chunk_proof_cannot_authorize_parent_retry() {
        for failed_source in [Speaker::Me, Speaker::Others] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store
                .create("Owned legacy parent ambiguity", start())
                .unwrap();
            let (path, mut failed) = proven_row(&store, &id, failed_source, 0, true);
            failed.metadata.recovery_audio = None;
            let mut segment = seg(100, Speaker::Others, "Old timed subrange.");
            segment.end_ms = 400;
            let success = MeetingUtterance::new(segment, Some("microphone".into()));
            store
                .write_transcript_snapshot(&id, None, &[success, failed])
                .unwrap();
            let snapshot = store.read_transcript_snapshot(&id).unwrap();
            let failed = snapshot
                .utterances
                .iter()
                .find(|row| row.segment.failed)
                .unwrap();
            if failed_source == Speaker::Me {
                assert_eq!(
                    failed.metadata.recovery_audio.as_ref().unwrap().disposition,
                    crate::meeting_utterances::RecoveryAudioDisposition::Unverified
                );
                assert!(store.recovery_audio_path(&id, failed).is_none());
            } else {
                assert_eq!(
                    failed.metadata.recovery_audio.as_ref().unwrap().disposition,
                    crate::meeting_utterances::RecoveryAudioDisposition::Pending
                );
                assert_eq!(store.recovery_audio_path(&id, failed), Some(path));
            }
        }
    }

    #[test]
    fn completed_recovery_candidates_are_hashed_before_suppression_and_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned completion byte verification", start())
            .unwrap();
        let (path, row) = proven_row(&store, &id, Speaker::Me, 0, false);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let size = fs::metadata(&path).unwrap().len() as usize;
        RECOVERY_PCM_READ_BYTES.with(|count| count.set(0));
        assert_eq!(
            store
                .read_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .len(),
            1
        );
        assert_eq!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get), size);
        RECOVERY_PCM_READ_BYTES.with(|count| count.set(0));
        assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 1);
        assert_eq!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get), size);
    }

    #[test]
    fn recovery_payload_reads_remain_bounded_if_a_file_grows_after_header_validation() {
        struct GrowingReader(std::rc::Rc<std::cell::Cell<usize>>);
        impl Read for GrowingReader {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                output.fill(0);
                self.0.set(self.0.get().saturating_add(output.len()));
                Ok(output.len())
            }
        }
        let read = std::rc::Rc::new(std::cell::Cell::new(0));
        let error = read_recovery_payload(GrowingReader(read.clone())).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(read.get() as u64, RECOVERY_AUDIO_MAX_BYTES + 1);
    }

    #[test]
    fn corrupt_or_truncated_recovery_headers_are_retained_without_payload_reads() {
        for corrupt_at in [4, 16, 28, 32, 40, usize::MAX] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store.create("Owned malformed recovery", start()).unwrap();
            let (path, row) = proven_row(&store, &id, Speaker::Me, 0, false);
            store.write_transcript_snapshot(&id, None, &[row]).unwrap();
            let mut bytes = fs::read(&path).unwrap();
            if corrupt_at == usize::MAX {
                bytes.pop();
            } else {
                bytes[corrupt_at] ^= 0xff;
            }
            fs::write(&path, bytes).unwrap();
            RECOVERY_PCM_READ_BYTES.with(|count| count.set(0));
            assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 0);
            assert!(path.exists());
            assert_eq!(RECOVERY_PCM_READ_BYTES.with(std::cell::Cell::get), 0);
        }
    }

    #[test]
    fn recovery_parent_completion_survives_acoustic_split_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned interrupted split cleanup", start())
            .unwrap();
        let (microphone, mut original) = proven_row(&store, &id, Speaker::Me, 0, false);
        let (system, failed) = proven_row(&store, &id, Speaker::Others, 0, true);
        original.metadata.spans = vec![
            crate::TimedTextSpan(0, 3, 100, 400),
            crate::TimedTextSpan(4, 6, 600, 900),
        ];
        let (mut split, _) = crate::diarize::apply_utterances(
            &[original],
            &[
                pennel_speakers::Turn {
                    start_ms: 100,
                    end_ms: 400,
                    speaker: 0,
                },
                pennel_speakers::Turn {
                    start_ms: 600,
                    end_ms: 900,
                    speaker: 1,
                },
            ],
            &[],
            crate::diarize::Track::Microphone,
        );
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].segment.start_ms, 100);
        assert_eq!(
            split
                .iter()
                .map(|row| row.segment.text.as_str())
                .collect::<String>(),
            "Yes no"
        );
        split.push(failed);
        store.write_transcript_snapshot(&id, None, &split).unwrap();
        let reopened = MeetingStore::new(directory.path());
        let snapshot = reopened.read_transcript_snapshot(&id).unwrap();
        assert_eq!(snapshot.utterances.len(), 3);
        assert_eq!(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.segment.failed)
                .count(),
            1
        );
        assert!(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.segment.failed)
                .all(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
        );
        assert_eq!(reopened.cleanup_committed_audio(&id).unwrap(), 1);
        assert!(!microphone.exists());
        assert!(system.exists());
    }

    #[test]
    fn recovery_cleanup_preserves_same_source_overlapping_failure() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned overlapping original chunks", start())
            .unwrap();
        let (complete_path, mut complete) = proven_row(&store, &id, Speaker::Me, 0, false);
        let (failed_path, failed) = proven_row(&store, &id, Speaker::Me, 400, true);
        complete.segment.speaker = Speaker::Others;
        store
            .write_transcript_snapshot(&id, None, &[complete, failed])
            .unwrap();
        assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 1);
        assert!(!complete_path.exists());
        assert!(failed_path.exists());
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.segment.failed)
                .count(),
            1
        );
        assert_eq!(
            store.recovery_audio_path(
                &id,
                snapshot
                    .utterances
                    .iter()
                    .find(|row| row.segment.failed)
                    .unwrap()
            ),
            Some(failed_path)
        );
    }

    #[test]
    fn filtered_completion_proof_survives_empty_save_and_restart() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned private completion marker", start())
            .unwrap();
        let (path, mut row) = proven_row(&store, &id, Speaker::Me, 0, false);
        row.segment.text.clear();
        row.metadata.refresh_fingerprints(&row.segment);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        assert!(store.read_transcript(&id).is_empty());
        store.write_transcript_snapshot(&id, None, &[]).unwrap();
        let reopened = MeetingStore::new(directory.path());
        assert!(reopened.read_transcript(&id).is_empty());
        assert_eq!(reopened.cleanup_committed_audio(&id).unwrap(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn speaker_replacement_keeps_private_completion_markers_chronological() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned sorted completion marker", start())
            .unwrap();
        let (path, mut empty) = proven_row(&store, &id, Speaker::Me, 0, false);
        empty.segment.text.clear();
        empty.metadata.refresh_fingerprints(&empty.segment);
        let spoken = MeetingUtterance::new(
            seg(2_000, Speaker::Others, "A later spoken passage."),
            Some("system_audio".into()),
        );
        store
            .write_transcript_snapshot(&id, None, &[empty, spoken])
            .unwrap();
        let visible = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(visible.utterances.len(), 1);

        store
            .replace_speaker_utterances(
                &id,
                &visible.utterances,
                &[],
                &[Speaker::Me, Speaker::Others],
            )
            .unwrap();
        store.finish_retranscription(&id).unwrap();

        let committed = store.read_committed_transcript_snapshot(&id).unwrap();
        assert_eq!(
            committed
                .utterances
                .iter()
                .map(|row| row.segment.start_ms)
                .collect::<Vec<_>>(),
            [0, 2_000]
        );
        assert_eq!(store.read_transcript(&id).len(), 1);
        assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 1);
        assert!(
            !path.exists(),
            "the hidden completion proof must survive replacement"
        );
    }

    #[test]
    fn private_completion_survives_corrupt_audio_then_empty_save_and_repair() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned private repair fixture", start())
            .unwrap();
        let (path, mut row) = proven_row(&store, &id, Speaker::Me, 0, false);
        row.segment.text.clear();
        row.metadata.refresh_fingerprints(&row.segment);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let original = fs::read(&path).unwrap();
        let mut corrupt = original.clone();
        corrupt[0] = b'X';
        fs::write(&path, corrupt).unwrap();
        store.write_transcript_snapshot(&id, None, &[]).unwrap();
        let root = store.dir_of(&id);
        let raw = snapshot_from_text(
            &root,
            &store.uid_of(&id).unwrap(),
            &transcript_bytes(&root).unwrap(),
        );
        assert_eq!(
            raw.utterances.len(),
            1,
            "invalid audio must not erase its private completion disposition"
        );
        fs::write(&path, original).unwrap();
        let reopened = MeetingStore::new(directory.path());
        assert!(reopened.read_transcript(&id).is_empty());
        assert_eq!(reopened.cleanup_committed_audio(&id).unwrap(), 1);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_completion_survives_dangling_file_and_directory_symlinks() {
        for directory_link in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store
                .create("Owned uncertain recovery entry", start())
                .unwrap();
            let (path, mut row) = proven_row(&store, &id, Speaker::Me, 0, false);
            row.segment.text.clear();
            row.metadata.refresh_fingerprints(&row.segment);
            store.write_transcript_snapshot(&id, None, &[row]).unwrap();
            let original = fs::read(&path).unwrap();
            fs::remove_file(&path).unwrap();
            let link = if directory_link {
                let parent = path.parent().unwrap();
                fs::remove_dir(parent).unwrap();
                parent.to_path_buf()
            } else {
                path.clone()
            };
            std::os::unix::fs::symlink(directory.path().join("missing-owned-fixture"), &link)
                .unwrap();
            store.write_transcript_snapshot(&id, None, &[]).unwrap();
            let root = store.dir_of(&id);
            let raw = snapshot_from_text(
                &root,
                &store.uid_of(&id).unwrap(),
                &transcript_bytes(&root).unwrap(),
            );
            assert_eq!(raw.utterances.len(), 1);
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(link).unwrap();
            if directory_link {
                fs::create_dir(path.parent().unwrap()).unwrap();
            }
            fs::write(&path, original).unwrap();
            assert!(
                MeetingStore::new(directory.path())
                    .read_transcript(&id)
                    .is_empty()
            );
            assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 1);
        }
    }

    #[test]
    fn recovery_cleanup_refuses_replaced_bytes_and_unsupported_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned changed recovery bytes", start())
            .unwrap();
        let (path, row) = proven_row(&store, &id, Speaker::Me, 0, false);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.5; 16_000], 16_000),
            )
            .unwrap();
        assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 0);
        assert!(path.exists());
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        let unknown = snapshot
            .utterances
            .iter()
            .find(|row| row.segment.failed)
            .unwrap();
        assert_eq!(
            unknown
                .metadata
                .recovery_audio
                .as_ref()
                .unwrap()
                .disposition,
            crate::meeting_utterances::RecoveryAudioDisposition::Unverified
        );
        assert!(store.recovery_audio_path(&id, unknown).is_none());
        fs::write(
            store.dir_of(&id).join(meeting_utterances::MANIFEST),
            br#"{"schema_version":9000}"#,
        )
        .unwrap();
        assert!(store.cleanup_committed_audio(&id).is_err());
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_cleanup_never_follows_files_or_directory_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store.create("Owned symbolic recovery", start()).unwrap();
        let (path, row) = proven_row(&store, &id, Speaker::Me, 0, false);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let external = directory.path().join("unrelated.wav");
        fs::rename(&path, &external).unwrap();
        std::os::unix::fs::symlink(&external, &path).unwrap();
        assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 0);
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(external.exists());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
        let external_dir = directory.path().join("unrelated-recovery");
        fs::create_dir(&external_dir).unwrap();
        fs::copy(&external, external_dir.join("me-0.wav")).unwrap();
        std::os::unix::fs::symlink(&external_dir, path.parent().unwrap()).unwrap();
        assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 0);
        assert!(external_dir.join("me-0.wav").exists());
    }

    #[test]
    fn filtered_recovery_audio_still_proves_duration_before_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Gefilterde duur", start()).unwrap();
        let path = store
            .write_recovery(
                &id,
                Speaker::Me,
                60_400,
                &crate::wav::encode_wav(&vec![0.25; 32_000], 16_000),
            )
            .unwrap();
        let proof = recovery_provenance(
            &path,
            crate::meeting_utterances::RecoveryAudioDisposition::Complete,
        )
        .unwrap();
        let mut segment = seg(60_400, Speaker::Me, "");
        segment.end_ms = proof.end_ms;
        let mut row = MeetingUtterance::new(segment, Some("microphone".into()));
        row.metadata.recovery_audio = Some(proof);
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        assert!(store.read_transcript(&id).is_empty());
        store.restore_recovery_duration(&id).unwrap();
        assert_eq!(store.info(&id).unwrap().duration_min, 2);
    }

    #[test]
    fn committed_and_filtered_audio_is_not_reprocessed_after_cleanup_is_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Bewaard", start()).unwrap();
        let path = crate::session::failed_path(&store.dir_of(&id), Speaker::Me, 0);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, crate::wav::encode_wav(&vec![0.25; 16_000], 16_000)).unwrap();
        let mut row = MeetingUtterance::new(seg(0, Speaker::Me, ""), Some("microphone".into()));
        row.metadata.recovery_audio = recovery_provenance(
            &path,
            crate::meeting_utterances::RecoveryAudioDisposition::Complete,
        );
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        assert!(store.read_transcript(&id).is_empty());
        store.write_transcript(&id, &[]).unwrap();
        assert!(store.read_transcript(&id).is_empty());
    }

    fn start() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 23, 14, 0, 0).unwrap()
    }

    fn seg(start_ms: u64, speaker: Speaker, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 1000,
            speaker,
            text: text.into(),
            failed: false,
            voice: None,
        }
    }

    #[test]
    fn transcript_voice_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Sync", start()).unwrap();
        let mut two = seg(5_000, Speaker::Others, "Klopt");
        two.voice = Some(2);
        store
            .write_transcript(
                &id,
                &[
                    seg(0, Speaker::Others, "Hallo"),
                    two.clone(),
                    seg(9_000, Speaker::Me, "Ok"),
                ],
            )
            .unwrap();
        let read = store.read_transcript(&id);
        assert_eq!(read[0].voice, None);
        assert_eq!(read[1].voice, Some(2));
        assert_eq!(read[1].speaker, Speaker::Others);
        assert!(
            std::fs::read_to_string(store.dir_of(&id).join("transcript.md"))
                .unwrap()
                .contains("[00:05] **Anderen 2:** Klopt")
        );
        assert_eq!(parse_segment("[00:01] **Anderen 0:** x"), None);
        assert_eq!(parse_segment("[00:01] **Anderen 13:** x"), None);
        assert_eq!(
            parse_segment("[00:01] **Anderen 12:** x").unwrap().voice,
            Some(12)
        );
        assert_eq!(
            store.rename_speaker(&id, 0, "x"),
            Err(PennelError::NotFound)
        );
    }

    #[test]
    fn set_speaker_names_replaces_all_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Namen", start()).unwrap();
        store.rename_speaker(&id, 1, "Oud").unwrap();
        let name = |voice, name: &str| SpeakerName {
            voice,
            name: name.into(),
        };
        store
            .set_speaker_names(
                &id,
                &[
                    name(3, "Jan"),
                    name(2, " "),
                    name(13, "Te veel"),
                    name(10, "An;na"),
                ],
            )
            .unwrap();
        assert_eq!(store.speaker_names(&id), [name(3, "Jan"), name(10, "Anna")]);
        store.set_speaker_names(&id, &[]).unwrap();
        assert!(store.speaker_names(&id).is_empty());
        assert!(!store.read_file(&id, "meeting.md").contains("speakers:"));
    }

    #[test]
    fn speaker_names_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Sync", start()).unwrap();
        store.rename_speaker(&id, 2, "Jan; De=Smet: CEO").unwrap();
        store.rename_speaker(&id, 1, "Els").unwrap();
        assert_eq!(
            store.speaker_names(&id),
            vec![
                SpeakerName {
                    voice: 1,
                    name: "Els".into()
                },
                SpeakerName {
                    voice: 2,
                    name: "Jan DeSmet CEO".into()
                }
            ]
        );
        store.rename_speaker(&id, 1, "  ").unwrap();
        assert_eq!(store.speaker_names(&id).len(), 1);
        assert_eq!(store.info(&id).unwrap().title, "Sync");
    }

    #[test]
    fn labels_use_names() {
        let names = [SpeakerName {
            voice: 2,
            name: "Jan".into(),
        }];
        let mut two = seg(0, Speaker::Others, "x");
        two.voice = Some(2);
        let mut three = two.clone();
        three.voice = Some(3);
        assert_eq!(speaker_label(&two, &names), "Jan");
        assert_eq!(speaker_label(&three, &names), "Anderen 3");
        assert_eq!(
            speaker_label(&seg(0, Speaker::Others, "x"), &names),
            "Anderen"
        );
        assert_eq!(speaker_label(&seg(0, Speaker::Me, "x"), &names), "Ik");
    }

    #[test]
    fn create_uses_dated_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Sprint review", start()).unwrap();
        assert_eq!(id, "2026-10-23 1400 Sprint review");
        assert!(dir.path().join(&id).join("meeting.md").exists());
    }

    #[test]
    fn create_sanitizes_and_dedupes_title() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        assert_eq!(
            store.create("a/b:c", start()).unwrap(),
            "2026-10-23 1400 abc"
        );
        assert_eq!(
            store.create("a/b:c", start()).unwrap(),
            "2026-10-23 1400 abc (2)"
        );
        assert_eq!(
            store.create("Wat nu? <plan> \"A|B\"*.", start()).unwrap(),
            "2026-10-23 1400 Wat nu plan AB"
        );
        let long = "x".repeat(100);
        assert_eq!(
            store.create(&long, start()).unwrap().len(),
            "2026-10-23 1400 ".len() + 60
        );
    }

    #[test]
    fn transcript_roundtrips_with_speakers_and_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        let mut failed = seg(65_000, Speaker::Others, "");
        failed.failed = true;
        store
            .write_transcript(&id, &[seg(5_000, Speaker::Me, "Hallo allemaal"), failed])
            .unwrap();
        let md = std::fs::read_to_string(dir.path().join(&id).join("transcript.md")).unwrap();
        assert!(md.contains("[00:05] **Ik:** Hallo allemaal"), "{md}");
        assert!(
            md.contains("[01:05] **Anderen:** _(transcriptie mislukt)_"),
            "{md}"
        );
        let back = store.read_transcript(&id);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].speaker, Speaker::Me);
        assert_eq!(back[0].text, "Hallo allemaal");
        assert_eq!(back[0].start_ms, 5_000);
        assert!(back[1].failed);
    }

    #[test]
    fn transcript_keeps_multiline_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        store
            .write_transcript(&id, &[seg(0, Speaker::Me, "eerste zin\ntweede zin")])
            .unwrap();
        assert_eq!(store.read_transcript(&id)[0].text, "eerste zin tweede zin");
    }

    #[test]
    fn rewriting_keeps_frontmatter_added_by_user() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        let path = dir.path().join(&id).join("meeting.md");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("template: \n", "template: \ntags: [q8, klant]\n");
        std::fs::write(&path, edited).unwrap();
        store.write_meeting(&id, 5, "algemeen", "body").unwrap();
        let id = store.rename(&id, "Nieuw").unwrap();
        let md = std::fs::read_to_string(dir.path().join(&id).join("meeting.md")).unwrap();
        assert!(md.contains("tags: [q8, klant]"), "{md}");
        assert!(md.contains("title: Nieuw"), "{md}");
    }

    #[test]
    fn attendees_are_written_to_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        store
            .set_attendees(&id, &["Sarah Peeters".into(), "Tom".into()])
            .unwrap();
        store.write_meeting(&id, 3, "algemeen", "body").unwrap();
        let md = std::fs::read_to_string(dir.path().join(&id).join("meeting.md")).unwrap();
        assert_eq!(md.matches("attendees:").count(), 1, "{md}");
        assert!(md.contains("attendees: [Sarah Peeters, Tom]"), "{md}");
    }

    #[test]
    fn transcript_hours_format() {
        assert_eq!(timestamp(3_723_000), "1:02:03");
        assert_eq!(timestamp(59_000), "00:59");
        assert_eq!(parse_timestamp("1:02:03"), Some(3_723_000));
    }

    #[test]
    fn meeting_md_has_frontmatter_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Klant X", start()).unwrap();
        store
            .write_meeting(&id, 42, "klantgesprek", "## Samenvatting\n- ok")
            .unwrap();
        let md = std::fs::read_to_string(dir.path().join(&id).join("meeting.md")).unwrap();
        assert!(
            md.starts_with("---\ntitle: Klant X\nstart: 2026-10-23T14:00:00"),
            "{md}"
        );
        assert!(
            md.contains("duration_min: 42\ntemplate: klantgesprek\npennel_id: "),
            "{md}"
        );
        assert_eq!(
            store.read_meeting(&id).as_deref(),
            Some("## Samenvatting\n- ok")
        );
        let info = &store.list()[0];
        assert_eq!(info.title, "Klant X");
        assert_eq!(info.duration_min, 42);
        assert!(info.has_summary);
    }

    #[test]
    fn read_meeting_returns_external_edit() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        store
            .write_meeting(&id, 1, "algemeen", "origineel")
            .unwrap();
        let path = dir.path().join(&id).join("meeting.md");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("origineel", "bewerkt in Obsidian");
        std::fs::write(&path, edited).unwrap();
        assert_eq!(
            store.read_meeting(&id).as_deref(),
            Some("bewerkt in Obsidian")
        );
    }

    #[test]
    fn list_newest_first_and_includes_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        store
            .create(
                "Oud",
                Local.with_ymd_and_hms(2026, 10, 20, 9, 0, 0).unwrap(),
            )
            .unwrap();
        store.create("Nieuw", start()).unwrap();
        let list = store.list();
        assert_eq!(
            list.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
            ["Nieuw", "Oud"]
        );
        assert!(!list[0].has_summary);
    }

    #[test]
    fn search_matches_transcript_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let a = store.create("A", start()).unwrap();
        store.create("B", start()).unwrap();
        store
            .write_transcript(&a, &[seg(0, Speaker::Others, "De Excel-export is traag")])
            .unwrap();
        let hits = store.search("excel");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, a);
        assert_eq!(store.search("").len(), 2);
    }

    #[test]
    fn notes_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        assert_eq!(store.read_notes(&id), "");
        store.write_notes(&id, "- export naar Excel").unwrap();
        assert_eq!(store.read_notes(&id), "- export naar Excel");
    }

    #[test]
    fn rename_moves_folder_and_updates_title() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        let new_id = store.rename(&id, "Kickoff Q8").unwrap();
        assert_eq!(new_id, "2026-10-23 1400 Kickoff Q8");
        assert_eq!(store.list()[0].title, "Kickoff Q8");
        assert!(!dir.path().join(&id).exists());
    }

    #[test]
    fn failed_rename_metadata_write_keeps_the_original_path_uid_and_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Before", start()).unwrap();
        store.write_notes(&id, "Saved notes").unwrap();
        store
            .write_transcript(&id, &[seg(1_000, Speaker::Me, "Saved speech")])
            .unwrap();
        let uid = store.uid_of(&id).unwrap();
        let original = std::fs::read(store.dir_of(&id).join("meeting.md")).unwrap();
        let blocked = dir.path().join("blocked-metadata");
        std::fs::create_dir(&blocked).unwrap();
        let result = store.rename_with_write(&id, "After", |_, metadata| {
            durable_write(&blocked, metadata.as_bytes())
        });
        assert!(result.is_err());
        assert!(
            store.dir_of(&id).is_dir(),
            "A failed metadata write must not relocate the retained session"
        );
        assert!(!store.dir_of("2026-10-23 1400 After").exists());
        assert_eq!(
            std::fs::read(store.dir_of(&id).join("meeting.md")).unwrap(),
            original
        );
        assert_eq!(store.uid_of(&id), Some(uid.clone()));
        assert_eq!(store.read_notes(&id), "Saved notes");
        assert_eq!(store.read_transcript(&id)[0].text, "Saved speech");
        store.write_notes(&id, "Retained draft retry").unwrap();
        let renamed = store.rename(&id, "After").unwrap();
        assert_eq!(store.uid_of(&renamed), Some(uid));
        assert_eq!(store.read_notes(&renamed), "Retained draft retry");
        assert!(!store.dir_of(&id).exists());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn failed_folder_move_restores_exact_metadata_and_keeps_notes_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Before", start()).unwrap();
        store.write_notes(&id, "Saved notes").unwrap();
        let original = std::fs::read(store.dir_of(&id).join("meeting.md")).unwrap();
        let uid = store.uid_of(&id).unwrap();
        let collision = store.dir_of("2026-10-23 1400 After");
        let result = store.rename_with_write(&id, "After", |target, metadata| {
            store.write_file(target, "meeting.md", metadata)?;
            std::fs::create_dir(&collision).map_err(PennelError::storage)?;
            std::fs::write(collision.join("owned-blocker"), b"occupied")
                .map_err(PennelError::storage)
        });
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(store.dir_of(&id).join("meeting.md")).unwrap(),
            original
        );
        assert_eq!(store.uid_of(&id), Some(uid.clone()));
        assert_eq!(store.read_notes(&id), "Saved notes");
        store.write_notes(&id, "Retained draft retry").unwrap();
        std::fs::remove_file(collision.join("owned-blocker")).unwrap();
        std::fs::remove_dir(&collision).unwrap();
        let renamed = store.rename(&id, "After").unwrap();
        assert_eq!(store.uid_of(&renamed), Some(uid));
        assert_eq!(store.read_notes(&renamed), "Retained draft retry");
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn prune_audio_keeps_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("T", start()).unwrap();
        let audio = dir.path().join(&id).join("ik.m4a");
        std::fs::write(&audio, b"aac").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(40 * 86_400);
        assert_eq!(store.prune_audio(0, later), 0);
        assert_eq!(store.prune_audio(30, later), 1);
        assert!(!audio.exists());
        assert!(dir.path().join(&id).join("meeting.md").exists());
    }

    #[test]
    fn create_writes_uid_that_survives_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Uid", start()).unwrap();
        let uid = store.uid_of(&id).expect("uid");
        assert_eq!(uid.len(), 36);
        let text = std::fs::read_to_string(dir.path().join(&id).join("meeting.md")).unwrap();
        assert!(text.contains(&format!("pennel_id: {uid}\n")));
        store.set_duration(&id, 12).unwrap();
        store.set_attendees(&id, &["Els".into()]).unwrap();
        let renamed = store.rename(&id, "Nieuw").unwrap();
        assert_eq!(store.uid_of(&renamed).as_deref(), Some(uid.as_str()));
        let text = std::fs::read_to_string(dir.path().join(&renamed).join("meeting.md")).unwrap();
        assert_eq!(text.matches("pennel_id:").count(), 1);
        assert_eq!(store.ensure_uid(&renamed).unwrap(), uid);
    }

    #[test]
    fn ensure_uid_adds_one_line_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("Oud")).unwrap();
        let original =
            "---\ntitle:Oud\nstart: 2026-01-01T10:00:00+01:00\ntags: [x]\n---\n\n  Inhoud\n\n";
        std::fs::write(dir.path().join("Oud/meeting.md"), original).unwrap();
        assert_eq!(store.uid_of("Oud"), None);
        let uid = store.ensure_uid("Oud").unwrap();
        let text = std::fs::read_to_string(dir.path().join("Oud/meeting.md")).unwrap();
        assert_eq!(
            text,
            format!(
                "---\ntitle:Oud\nstart: 2026-01-01T10:00:00+01:00\ntags: [x]\npennel_id: {uid}\n---\n\n  Inhoud\n\n"
            )
        );
        assert_eq!(store.ensure_uid("Oud").unwrap(), uid);
        assert_eq!(store.uid_of("Oud").as_deref(), Some(uid.as_str()));
        std::fs::create_dir_all(dir.path().join("Kapot")).unwrap();
        std::fs::write(dir.path().join("Kapot/meeting.md"), "geen frontmatter").unwrap();
        assert_eq!(store.ensure_uid("Kapot"), Err(PennelError::NotFound));
        assert_eq!(store.ensure_uid("Bestaat niet"), Err(PennelError::NotFound));
    }

    #[test]
    fn crlf_meeting_gets_a_uid_and_keeps_its_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("Win")).unwrap();
        let original =
            "---\r\ntitle: Win\r\nstart: 2026-01-01T10:00:00+01:00\r\n---\r\n\r\nInhoud\r\n";
        std::fs::write(dir.path().join("Win/meeting.md"), original).unwrap();
        assert_eq!(store.frontmatter("Win").unwrap().0.title, "Win");
        let uid = store.ensure_uid("Win").unwrap();
        let text = std::fs::read_to_string(dir.path().join("Win/meeting.md")).unwrap();
        assert_eq!(
            text,
            format!(
                "---\r\ntitle: Win\r\nstart: 2026-01-01T10:00:00+01:00\r\npennel_id: {uid}\r\n---\r\n\r\nInhoud\r\n"
            )
        );
        assert_eq!(store.uid_of("Win").as_deref(), Some(uid.as_str()));
        assert_eq!(store.ensure_uid("Win").unwrap(), uid);
    }

    #[test]
    fn set_uid_leaves_the_store_temp_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Tmp", start()).unwrap();
        let tmp = dir.path().join(&id).join("meeting.tmp");
        std::fs::write(&tmp, "schrijven bezig").unwrap();
        store.set_uid(&id, "0190-aa").unwrap();
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "schrijven bezig");
        assert_eq!(store.uid_of(&id).as_deref(), Some("0190-aa"));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join(&id))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp") && n != "meeting.tmp")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn invalid_uid_is_ignored_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("M")).unwrap();
        std::fs::write(
            dir.path().join("M/meeting.md"),
            "---\ntitle: M\npennel_id: ../x\n---\n\nBody\n",
        )
        .unwrap();
        assert_eq!(store.uid_of("M"), None);
        let uid = store.ensure_uid("M").unwrap();
        let text = std::fs::read_to_string(dir.path().join("M/meeting.md")).unwrap();
        assert_eq!(
            text,
            format!("---\ntitle: M\npennel_id: {uid}\n---\n\nBody\n")
        );
    }

    #[test]
    fn mark_lines_matches_own_notes() {
        let lines = mark_lines(
            "- Export naar Excel\n- Tom vond de oorzaak\n",
            "- export naar Excel",
        );
        assert_eq!(
            lines,
            vec![
                NoteLine {
                    text: "- Export naar Excel".into(),
                    mine: true
                },
                NoteLine {
                    text: "- Tom vond de oorzaak".into(),
                    mine: false
                },
            ]
        );
    }
}
