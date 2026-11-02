use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Local};

use crate::error::PennelError;

const TITLE_MAX: usize = 60;
const FAILED_MARK: &str = "_(transcriptie mislukt)_";
const REPLACEMENT_JOURNAL: &str = ".pennel-retranscription-pending.json";
const TRANSCRIPT_BACKUPS: &str = ".pennel-transcript-backups";
// Readers must see matching text, speaker names and recovery chunks during the short commit.
static REPLACEMENT_GATE: Mutex<()> = Mutex::new(());

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

fn sync_directory(path: &Path) -> Result<(), PennelError> {
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

pub struct MeetingStore {
    dir: PathBuf,
}

pub(crate) fn durable_write(path: &Path, bytes: &[u8]) -> Result<(), PennelError> {
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
    if length > 16_000 * 2 * 31 + 44 {
        return None;
    }
    let mut header = [0u8; 44];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || &header[36..40] != b"data"
        || u16::from_le_bytes(header[20..22].try_into().ok()?) != 1
        || u16::from_le_bytes(header[22..24].try_into().ok()?) != 1
        || u16::from_le_bytes(header[34..36].try_into().ok()?) != 16
    {
        return None;
    }
    let rate = u32::from_le_bytes(header[24..28].try_into().ok()?);
    let data_length = u64::from(u32::from_le_bytes(header[40..44].try_into().ok()?));
    if rate != 16_000 || data_length % 2 != 0 || data_length + 44 != length {
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
        if !meeting.is_dir() {
            return Err(PennelError::Storage {
                message: "the meeting folder is unavailable".into(),
            });
        }
        let path = crate::session::failed_path(&meeting, speaker, start_ms);
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
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::fs::read_to_string(self.dir_of(id).join(name)).unwrap_or_default()
    }

    pub fn write_notes(&self, id: &str, notes: &str) -> Result<(), PennelError> {
        self.write_file(id, "notes.md", notes)
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
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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

    pub(crate) fn replace_retranscribed(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        tracks: &[Speaker],
    ) -> Result<PathBuf, PennelError> {
        self.replace_transcript(id, segments, names, tracks, true)
    }

    pub(crate) fn replace_speaker_labels(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        tracks: &[Speaker],
    ) -> Result<PathBuf, PennelError> {
        self.replace_transcript(id, segments, names, tracks, false)
    }

    fn replace_transcript(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        tracks: &[Speaker],
        move_failed_audio: bool,
    ) -> Result<PathBuf, PennelError> {
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = self.dir_of(id);
        if root.join(REPLACEMENT_JOURNAL).exists() {
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
        let replacement_transcript = render_transcript(&sorted);
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
        crate::meeting_recording::backup_recording_metadata(&root, &backup)?;
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
            )
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
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        clear_replacement_journal(&self.dir_of(id))
    }

    /// Called only while acquiring the library lease, before reads, indexing or sync.
    pub(crate) fn recover_retranscriptions(&self) -> Result<(), PennelError> {
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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

    pub fn write_transcript(&self, id: &str, segments: &[Segment]) -> Result<(), PennelError> {
        let mut sorted = segments.to_vec();
        // Empty successful lines are completion markers for intentionally filtered audio. Keep them
        // while cleanup is pending, so a crash cannot resurrect a private dictation as a failed chunk.
        for previous in self
            .read_file(id, "transcript.md")
            .lines()
            .filter_map(parse_segment)
            .filter(|segment| !segment.failed && segment.text.trim().is_empty())
        {
            if crate::session::failed_path(&self.dir_of(id), previous.speaker, previous.start_ms)
                .exists()
                && !sorted.iter().any(|segment| {
                    segment.speaker == previous.speaker && segment.start_ms == previous.start_ms
                })
            {
                sorted.push(previous);
            }
        }
        sorted.sort_by_key(|s| s.start_ms);
        let body = render_transcript(&sorted);
        self.write_file(id, "transcript.md", &body)
    }

    /// Number of non-empty transcript lines that did not parse; rewriting the transcript would drop them.
    pub fn unparsed_transcript_lines(&self, id: &str) -> usize {
        self.read_file(id, "transcript.md")
            .lines()
            .filter(|l| !l.trim().is_empty() && parse_segment(l).is_none())
            .count()
    }

    pub fn read_transcript(&self, id: &str) -> Vec<Segment> {
        let _gate = REPLACEMENT_GATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut segments: Vec<_> = fs::read_to_string(self.dir_of(id).join("transcript.md"))
            .unwrap_or_default()
            .lines()
            .filter_map(parse_segment)
            .collect();
        if let Ok(entries) = fs::read_dir(self.dir_of(id).join("failed")) {
            for entry in entries.filter_map(Result::ok) {
                if !entry.file_type().is_ok_and(|kind| kind.is_file())
                    || entry.path().extension().and_then(|value| value.to_str()) != Some("wav")
                {
                    continue;
                }
                let Some(recovered) = recovery_audio(&entry.path()) else {
                    continue;
                };
                if let Some(existing) = segments.iter_mut().find(|segment| {
                    segment.speaker == recovered.speaker && segment.start_ms == recovered.start_ms
                }) {
                    existing.end_ms = recovered.end_ms;
                } else {
                    segments.push(recovered);
                }
            }
        }
        segments.retain(|segment| segment.failed || !segment.text.trim().is_empty());
        segments.sort_by_key(|segment| segment.start_ms);
        segments
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
        let transcript = fs::read_to_string(self.dir_of(id).join("transcript.md"))
            .map_err(PennelError::storage)?;
        for segment in transcript.lines().filter_map(parse_segment) {
            if segment.failed {
                continue;
            }
            let path =
                crate::session::failed_path(&self.dir_of(id), segment.speaker, segment.start_ms);
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
        self.set_speaker_names(id, &names)
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
    format!(
        "---\ntitle: {}\nstart: {}\nduration_min: {}\ntemplate: {}\n{uid}{extra}---\n\n{}\n",
        front.title,
        front.start_iso,
        front.duration_min,
        front.template,
        body.trim()
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
    Some((front, body.trim().to_owned()))
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
        store
            .write_recovery(
                &id,
                Speaker::Others,
                60_400,
                &crate::wav::encode_wav(&vec![0.25; 32_000], 16_000),
            )
            .unwrap();
        store
            .write_transcript(&id, &[seg(60_400, Speaker::Others, "hersteld")])
            .unwrap();
        assert_eq!(store.read_transcript(&id)[0].end_ms, 62_400);
        store.restore_recovery_duration(&id).unwrap();
        assert_eq!(store.info(&id).unwrap().duration_min, 2);
        store.cleanup_committed_audio(&id).unwrap();
        assert!(!crate::session::failed_path(&store.dir_of(&id), Speaker::Others, 60_400).exists());
    }

    #[test]
    fn filtered_recovery_audio_still_proves_duration_before_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Gefilterde duur", start()).unwrap();
        store
            .write_recovery(
                &id,
                Speaker::Me,
                60_400,
                &crate::wav::encode_wav(&vec![0.25; 32_000], 16_000),
            )
            .unwrap();
        store
            .write_transcript(&id, &[seg(60_400, Speaker::Me, "")])
            .unwrap();
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
        store
            .write_transcript(&id, &[seg(0, Speaker::Me, "")])
            .unwrap();
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
