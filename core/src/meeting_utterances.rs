use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::PennelError;
use crate::meetings::{Segment, durable_write};

pub(crate) const MANIFEST: &str = "transcript.meta.json";
pub(crate) const SNAPSHOT_JOURNAL: &str = ".pennel-transcript-snapshot-pending.json";
pub const METADATA_PART_BYTES: usize = 1_048_576;
const MAX_MANIFEST_BYTES: u64 = 1_048_576;

fn supported_metadata_schema(version: u64) -> bool {
    matches!(version, 1..=4)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAudioTrack {
    Microphone,
    OtherCapture,
}

impl RecoveryAudioTrack {
    pub(crate) fn from_speaker(source: crate::meetings::Speaker) -> Self {
        match source {
            crate::meetings::Speaker::Me => Self::Microphone,
            crate::meetings::Speaker::Others => Self::OtherCapture,
        }
    }

    pub(crate) fn accepts_source(self, source: Option<&str>) -> bool {
        matches!(
            (self, source),
            (_, None)
                | (Self::Microphone, Some("microphone"))
                | (
                    Self::OtherCapture,
                    Some("system" | "system_audio" | "room_microphone")
                )
        )
    }

    fn file_prefix(self) -> &'static str {
        match self {
            Self::Microphone => "me",
            Self::OtherCapture => "others",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAudioDisposition {
    Pending,
    Complete,
    Unverified,
}

/// Capture-lane identity, not an inferred person label. OtherCapture may be system or room audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryAudioProvenance {
    pub file_name: String,
    pub track: RecoveryAudioTrack,
    pub start_ms: u64,
    pub end_ms: u64,
    pub sha256: String,
    pub disposition: RecoveryAudioDisposition,
}

impl RecoveryAudioProvenance {
    pub(crate) fn valid(&self) -> bool {
        self.file_name == format!("{}-{}.wav", self.track.file_prefix(), self.start_ms)
            && self.end_ms >= self.start_ms
            && self.end_ms - self.start_ms <= 31_000
            && (valid_digest(&self.sha256)
                || self.sha256.is_empty() && self.disposition != RecoveryAudioDisposition::Complete)
    }

    pub(crate) fn same_audio(&self, other: &Self) -> bool {
        self.same_chunk(other) && self.sha256 == other.sha256
    }

    pub(crate) fn same_chunk(&self, other: &Self) -> bool {
        self.file_name == other.file_name
            && self.track == other.track
            && self.start_ms == other.start_ms
            && self.end_ms == other.end_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionKind {
    Unknown,
    Single,
    Mixed,
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcousticTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub voice: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomaticAttribution {
    pub kind: AttributionKind,
    pub voices: Vec<u32>,
    #[serde(default)]
    pub self_voice: bool,
    #[serde(default)]
    pub provisional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<AcousticTurn>,
}

impl Default for AutomaticAttribution {
    fn default() -> Self {
        Self {
            kind: AttributionKind::Unknown,
            voices: Vec::new(),
            self_voice: false,
            provisional: false,
            turns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualCorrection {
    pub name: Option<String>,
    pub voice: Option<u32>,
    pub source_fingerprint: String,
    pub review_required: bool,
}

/// UTF-8 byte bounds in exact authoritative text followed by absolute audio milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedTextSpan(pub u32, pub u32, pub u64, pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtteranceMetadata {
    pub utterance_id: String,
    pub source_id: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text_fingerprint: String,
    pub source_fingerprint: String,
    pub spans: Vec<TimedTextSpan>,
    pub attribution: AutomaticAttribution,
    pub attribution_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_voice_profile_fingerprint: Option<String>,
    pub lineage: Vec<String>,
    pub manual: Option<ManualCorrection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_manual: Vec<ManualCorrection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_audio: Option<RecoveryAudioProvenance>,
}

impl UtteranceMetadata {
    pub fn new(segment: &Segment, source_id: Option<String>) -> Self {
        let mut metadata = Self {
            utterance_id: uuid::Uuid::now_v7().to_string(),
            source_id,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text_fingerprint: String::new(),
            source_fingerprint: String::new(),
            spans: Vec::new(),
            attribution: AutomaticAttribution::default(),
            attribution_revision: 0,
            self_voice_profile_fingerprint: None,
            lineage: Vec::new(),
            manual: None,
            unresolved_manual: Vec::new(),
            original_text: None,
            recovery_audio: None,
        };
        metadata.refresh_fingerprints(segment);
        metadata
    }

    pub fn refresh_fingerprints(&mut self, segment: &Segment) {
        self.start_ms = segment.start_ms;
        self.end_ms = segment.end_ms;
        self.text_fingerprint = digest(segment.text.as_bytes());
        self.source_fingerprint = digest(
            &serde_json::to_vec(&(
                &self.source_id,
                self.start_ms,
                self.end_ms,
                &self.text_fingerprint,
            ))
            .expect("primitive utterance identity"),
        );
        let projected = segment
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.original_text =
            (!segment.failed && projected != segment.text).then(|| segment.text.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeetingUtterance {
    pub segment: Segment,
    pub metadata: UtteranceMetadata,
}

impl MeetingUtterance {
    pub fn new(segment: Segment, source_id: Option<String>) -> Self {
        let metadata = UtteranceMetadata::new(&segment, source_id);
        Self { segment, metadata }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptMetadataStatus {
    Valid,
    Absent,
    Pending,
    Stale,
    Unsupported,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeetingTranscriptSnapshot {
    pub meeting_uid: String,
    pub revision: u64,
    pub transcript_digest: String,
    pub metadata_digest: Option<String>,
    pub metadata_status: TranscriptMetadataStatus,
    pub utterances: Vec<MeetingUtterance>,
}

#[derive(Serialize, Deserialize)]
struct MetadataManifest {
    schema_version: u32,
    meeting_uid: String,
    transcript_sha256: String,
    revision: u64,
    sources: Vec<String>,
    parts: Vec<MetadataPart>,
}

#[derive(Serialize, Deserialize)]
struct MetadataPart {
    sha256: String,
    rows: usize,
}

pub(crate) struct PreparedMetadata {
    manifest: Vec<u8>,
    parts: Vec<(String, Vec<u8>)>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotJournal {
    version: u32,
    token: String,
    transcript_sha256: String,
    manifest_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct MetadataBackup {
    version: u32,
    files: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedParts {
    version: u32,
    hashes: Vec<String>,
}

const OWNED_PARTS: &str = "transcript-part-ownership.json";

fn ownership(root: &Path) -> Option<OwnedParts> {
    let private = root.join(".pennel");
    if let Ok(metadata) = fs::symlink_metadata(&private)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return None;
    }
    let path = private.join(OWNED_PARTS);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(OwnedParts {
                version: 1,
                hashes: Vec::new(),
            });
        }
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        _ => return None,
    }
    let state: OwnedParts =
        serde_json::from_slice(&read_bounded(&path, MAX_MANIFEST_BYTES).ok()?).ok()?;
    if state.version != 1 || state.hashes.iter().any(|hash| !valid_digest(hash)) {
        return None;
    }
    Some(state)
}

fn save_ownership(root: &Path, state: &OwnedParts) -> Result<(), PennelError> {
    let private = root.join(".pennel");
    if !private.exists() {
        fs::create_dir(&private).map_err(PennelError::storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
                .map_err(PennelError::storage)?;
        }
    }
    let entry = fs::symlink_metadata(&private).map_err(PennelError::storage)?;
    if !entry.is_dir() || entry.file_type().is_symlink() {
        return Err(PennelError::storage(
            "The private transcript metadata folder is invalid.",
        ));
    }
    let bytes = serde_json::to_vec(state).map_err(PennelError::storage)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(PennelError::storage(
            "The transcript part ownership index is too large.",
        ));
    }
    durable_write(&private.join(OWNED_PARTS), &bytes)
}

fn manifest_references(root: &Path, path: &Path, complete: bool) -> Option<HashSet<String>> {
    let manifest: MetadataManifest =
        serde_json::from_slice(&read_bounded(path, MAX_MANIFEST_BYTES).ok()?).ok()?;
    if !supported_metadata_schema(u64::from(manifest.schema_version))
        || manifest.revision == 0
        || !crate::meetings::valid_uid(&manifest.meeting_uid)
        || !valid_digest(&manifest.transcript_sha256)
        || manifest.parts.len() > 4096
    {
        return None;
    }
    if complete && digest(&fs::read(root.join("transcript.md")).ok()?) != manifest.transcript_sha256
    {
        return None;
    }
    let mut references = HashSet::new();
    for part in manifest.parts {
        let name = part_name(&part.sha256)?;
        if !references.insert(name.clone()) {
            return None;
        }
        if complete
            && digest(&read_bounded(&root.join(&name), METADATA_PART_BYTES as u64).ok()?)
                != part.sha256
        {
            return None;
        }
    }
    Some(references)
}

fn root_references(root: &Path, complete: bool) -> Option<HashSet<String>> {
    let mut references = manifest_references(root, &root.join(MANIFEST), complete)?;
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("transcript.meta (conflict ") && name.ends_with(").json") {
            references.extend(manifest_references(root, &entry.path(), false)?);
        }
    }
    Some(references)
}

fn backup_copies_complete(root: &Path) -> bool {
    let directory = root.join(crate::meetings::TRANSCRIPT_BACKUPS);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    if fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return false;
    }
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            return false;
        }
        let backup = entry.path();
        let manifest_path = backup.join(MANIFEST);
        if !manifest_path.exists() {
            continue;
        }
        let Ok(bytes) = read_bounded(
            &backup.join("transcript-metadata-backup.json"),
            MAX_MANIFEST_BYTES,
        ) else {
            return false;
        };
        let Ok(state) = serde_json::from_slice::<MetadataBackup>(&bytes) else {
            return false;
        };
        if state.version != 1 || !state.files.iter().any(|name| name == MANIFEST) {
            return false;
        }
        let Some(references) = manifest_references(&backup, &manifest_path, true) else {
            return false;
        };
        if references.iter().any(|name| !state.files.contains(name)) {
            return false;
        }
    }
    true
}

pub(crate) fn part_is_referenced(root: &Path, name: &str) -> bool {
    let Some(hash) = name
        .strip_prefix("transcript.meta.")
        .and_then(|name| name.strip_suffix(".json"))
    else {
        return false;
    };
    if !valid_digest(hash) {
        return false;
    }
    if !root.join(MANIFEST).exists()
        && fs::read_dir(root).is_ok_and(|entries| {
            !entries.filter_map(Result::ok).any(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("transcript.meta (conflict ") && name.ends_with(").json")
            })
        })
    {
        return false;
    }
    root_references(root, false).is_none_or(|references| references.contains(name))
}

pub(crate) fn prune_owned_parts(root: &Path) -> Result<usize, PennelError> {
    if root.join(SNAPSHOT_JOURNAL).exists()
        || root.join(crate::meetings::REPLACEMENT_JOURNAL).exists()
    {
        return Ok(0);
    }
    let Some(mut owned) = ownership(root) else {
        return Ok(0);
    };
    let Some(references) = root_references(root, true) else {
        return Ok(0);
    };
    if !backup_copies_complete(root) {
        return Ok(0);
    }
    let mut removed = 0;
    let mut kept = Vec::new();
    for hash in owned.hashes {
        let name = part_name(&hash).expect("validated ownership hash");
        let path = root.join(&name);
        if references.contains(&name) {
            kept.push(hash);
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                kept.push(hash);
                continue;
            }
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || !read_bounded(&path, METADATA_PART_BYTES as u64)
                .is_ok_and(|bytes| digest(&bytes) == hash)
        {
            kept.push(hash);
            continue;
        }
        let after = fs::symlink_metadata(&path).map_err(PennelError::storage)?;
        if after.len() != metadata.len()
            || after.modified().ok() != metadata.modified().ok()
            || after.file_type().is_symlink()
        {
            kept.push(hash);
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if after.dev() != metadata.dev() || after.ino() != metadata.ino() {
                kept.push(hash);
                continue;
            }
        }
        fs::remove_file(&path).map_err(PennelError::storage)?;
        removed += 1;
    }
    owned.hashes = kept;
    if removed > 0 {
        crate::meetings::sync_directory(root)?;
    }
    save_ownership(root, &owned)?;
    Ok(removed)
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    crate::models::hex(&Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn part_name(hash: &str) -> Option<String> {
    valid_digest(hash).then(|| format!("transcript.meta.{hash}.json"))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, PennelError> {
    let metadata = fs::symlink_metadata(path).map_err(PennelError::storage)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return Err(PennelError::storage(
            "The transcript metadata file is invalid.",
        ));
    }
    let bytes = fs::read(path).map_err(PennelError::storage)?;
    if bytes.len() as u64 > limit {
        return Err(PennelError::storage(
            "The transcript metadata file is too large.",
        ));
    }
    Ok(bytes)
}

fn fallback_id(uid: &str, raw: &str, occurrence: usize) -> String {
    let hash = Sha256::digest(
        serde_json::to_vec(&(uid, raw, occurrence)).expect("primitive fallback identity"),
    );
    let mut bytes: [u8; 16] = hash[..16].try_into().expect("digest length");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

pub(crate) fn legacy_utterance(
    uid: &str,
    raw: &str,
    occurrence: usize,
    segment: Segment,
) -> MeetingUtterance {
    let mut row = MeetingUtterance::new(segment, None);
    row.metadata.utterance_id = fallback_id(uid, raw, occurrence);
    preserve_legacy_label(&mut row);
    row
}

fn preserve_legacy_label(row: &mut MeetingUtterance) {
    if let Some(voice) = row.segment.voice {
        row.metadata.attribution.kind = AttributionKind::Single;
        row.metadata.attribution.voices = vec![voice];
    }
}

fn source_valid(source: &Option<String>) -> bool {
    source.as_ref().is_none_or(|source| {
        !source.is_empty()
            && source.len() <= 128
            && source
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
    })
}

fn validate_row(row: &MeetingUtterance) -> bool {
    let metadata = &row.metadata;
    let segment = &row.segment;
    if uuid::Uuid::parse_str(&metadata.utterance_id).is_err()
        || !source_valid(&metadata.source_id)
        || metadata.start_ms != segment.start_ms
        || metadata.end_ms != segment.end_ms
        || metadata.end_ms < metadata.start_ms
        || metadata.text_fingerprint != digest(segment.text.as_bytes())
        || metadata.source_fingerprint
            != digest(
                &serde_json::to_vec(&(
                    &metadata.source_id,
                    metadata.start_ms,
                    metadata.end_ms,
                    &metadata.text_fingerprint,
                ))
                .expect("primitive utterance identity"),
            )
        || metadata
            .self_voice_profile_fingerprint
            .as_ref()
            .is_some_and(|fingerprint| !valid_digest(fingerprint))
        || metadata
            .attribution
            .voices
            .iter()
            .any(|voice| !crate::meetings::VOICES.contains(voice))
        || metadata
            .attribution
            .voices
            .iter()
            .collect::<HashSet<_>>()
            .len()
            != metadata.attribution.voices.len()
        || metadata
            .lineage
            .iter()
            .any(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return false;
    }
    if let Some(recovery) = &metadata.recovery_audio
        && (!recovery.valid()
            || !recovery.track.accepts_source(metadata.source_id.as_deref())
            || metadata.start_ms < recovery.start_ms
            || metadata.end_ms > recovery.end_ms
            || segment.failed == (recovery.disposition == RecoveryAudioDisposition::Complete))
    {
        return false;
    }
    if metadata.attribution.kind == AttributionKind::Single
        && !((metadata.attribution.voices.len() == 1 && !metadata.attribution.self_voice)
            || metadata.attribution.voices.is_empty() && metadata.attribution.self_voice)
    {
        return false;
    }
    if metadata.attribution.kind == AttributionKind::Unknown
        && (!metadata.attribution.voices.is_empty() || metadata.attribution.self_voice)
    {
        return false;
    }
    if metadata
        .unresolved_manual
        .iter()
        .any(|correction| !correction.review_required)
    {
        return false;
    }
    let mut previous = 0;
    for TimedTextSpan(start, end, start_ms, end_ms) in &metadata.spans {
        if start >= end
            || *start < previous
            || !segment.text.is_char_boundary(*start as usize)
            || !segment.text.is_char_boundary(*end as usize)
            || *end as usize > segment.text.len()
            || start_ms > end_ms
            || *start_ms < metadata.start_ms
            || *end_ms > metadata.end_ms
        {
            return false;
        }
        previous = *end;
    }
    if metadata.attribution.turns.iter().any(|turn| {
        turn.start_ms > turn.end_ms
            || turn.start_ms < metadata.start_ms
            || turn.end_ms > metadata.end_ms
            || turn
                .voice
                .is_some_and(|voice| !crate::meetings::VOICES.contains(&voice))
    }) {
        return false;
    }
    std::iter::once(metadata.manual.as_ref())
        .flatten()
        .chain(metadata.unresolved_manual.iter())
        .all(|correction| {
            valid_digest(&correction.source_fingerprint)
                && (correction.review_required
                    || correction.source_fingerprint == metadata.source_fingerprint)
                && correction
                    .voice
                    .is_none_or(|voice| crate::meetings::VOICES.contains(&voice))
                && correction
                    .name
                    .as_ref()
                    .is_none_or(|name| name.chars().count() <= 40 && !name.contains(['\r', '\n']))
        })
}

pub(crate) fn load_snapshot(
    root: &Path,
    uid: &str,
    text: &[u8],
    fallback: Vec<MeetingUtterance>,
) -> MeetingTranscriptSnapshot {
    let mut snapshot = MeetingTranscriptSnapshot {
        meeting_uid: uid.into(),
        revision: 0,
        transcript_digest: digest(text),
        metadata_digest: None,
        metadata_status: TranscriptMetadataStatus::Absent,
        utterances: fallback,
    };
    let path = root.join(MANIFEST);
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        if root.join(SNAPSHOT_JOURNAL).exists() {
            snapshot.metadata_status = TranscriptMetadataStatus::Pending;
        }
        return snapshot;
    }
    snapshot.metadata_status = TranscriptMetadataStatus::Corrupt;
    let Ok(bytes) = read_bounded(&path, MAX_MANIFEST_BYTES) else {
        return snapshot;
    };
    snapshot.metadata_digest = Some(digest(&bytes));
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return snapshot;
    };
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| !supported_metadata_schema(version))
    {
        snapshot.metadata_status = TranscriptMetadataStatus::Unsupported;
        return snapshot;
    }
    let Ok(manifest) = serde_json::from_value::<MetadataManifest>(value) else {
        return snapshot;
    };
    snapshot.revision = manifest.revision;
    if !supported_metadata_schema(u64::from(manifest.schema_version))
        || manifest.revision == 0
        || manifest.meeting_uid != uid
        || !valid_digest(&manifest.transcript_sha256)
        || manifest.parts.len() > 4096
        || manifest
            .sources
            .iter()
            .any(|source| !source_valid(&Some(source.clone())))
    {
        return snapshot;
    }
    if manifest.transcript_sha256 != snapshot.transcript_digest {
        snapshot.metadata_status = TranscriptMetadataStatus::Stale;
        return snapshot;
    }
    if root.join(SNAPSHOT_JOURNAL).exists() {
        snapshot.metadata_status = TranscriptMetadataStatus::Pending;
        return snapshot;
    }
    let metadata_schema = manifest.schema_version;
    let mut metadata = Vec::new();
    let mut parts = HashSet::new();
    for part in manifest.parts {
        let Some(name) = part_name(&part.sha256) else {
            return snapshot;
        };
        if !parts.insert(name.clone()) {
            return snapshot;
        }
        let part_path = root.join(name);
        let bytes = match read_bounded(&part_path, METADATA_PART_BYTES as u64) {
            Ok(bytes) => bytes,
            Err(_) => {
                snapshot.metadata_status = if part_path.exists() {
                    TranscriptMetadataStatus::Corrupt
                } else {
                    TranscriptMetadataStatus::Pending
                };
                return snapshot;
            }
        };
        if digest(&bytes) != part.sha256 {
            return snapshot;
        }
        let Ok(rows) = serde_json::from_slice::<Vec<UtteranceMetadata>>(&bytes) else {
            return snapshot;
        };
        if rows.len() != part.rows
            || metadata.len().saturating_add(rows.len()) > snapshot.utterances.len()
        {
            return snapshot;
        }
        metadata.extend(rows);
    }
    if metadata.len() != snapshot.utterances.len() {
        return snapshot;
    }
    let mut restored = Vec::with_capacity(metadata.len());
    let mut ids = HashSet::new();
    for (previous, metadata) in snapshot.utterances.iter().zip(metadata) {
        let mut segment = previous.segment.clone();
        if let Some(original) = &metadata.original_text {
            if segment.failed
                || original.split_whitespace().collect::<Vec<_>>().join(" ") != segment.text
            {
                return snapshot;
            }
            segment.text = original.clone();
        }
        segment.end_ms = metadata.end_ms;
        let row = MeetingUtterance { segment, metadata };
        if matches!(metadata_schema, 1 | 3) && row.metadata.attribution.provisional
            || matches!(metadata_schema, 1 | 2) && row.metadata.recovery_audio.is_some()
        {
            return snapshot;
        }
        if !ids.insert(row.metadata.utterance_id.clone()) || !validate_row(&row) {
            return snapshot;
        }
        restored.push(row);
    }
    snapshot.utterances = restored;
    snapshot.metadata_status = TranscriptMetadataStatus::Valid;
    snapshot
}

pub(crate) fn assert_writable(snapshot: &MeetingTranscriptSnapshot) -> Result<(), PennelError> {
    if !matches!(
        snapshot.metadata_status,
        TranscriptMetadataStatus::Valid | TranscriptMetadataStatus::Absent
    ) {
        return Err(PennelError::storage(
            "The transcript metadata is unavailable or changed. Keep the existing files and resolve the source before editing attribution.",
        ));
    }
    Ok(())
}

pub(crate) fn preserve_manual_work(
    previous: &MeetingTranscriptSnapshot,
    rows: &mut [MeetingUtterance],
) -> Result<(), PennelError> {
    let same = |left: &ManualCorrection, right: &ManualCorrection| {
        left.name == right.name
            && left.voice == right.voice
            && left.source_fingerprint == right.source_fingerprint
    };
    let mut retained = Vec::new();
    let indices: std::collections::HashMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.metadata.utterance_id.clone(), index))
        .collect();
    for old in &previous.utterances {
        if old.metadata.manual.is_none() && old.metadata.unresolved_manual.is_empty() {
            continue;
        }
        let current = indices
            .get(&old.metadata.utterance_id)
            .map(|index| &mut rows[*index]);
        if let Some(current) = current {
            if current.metadata.manual.is_none() {
                current.metadata.manual = old.metadata.manual.clone();
            }
            for correction in &old.metadata.unresolved_manual {
                if !current
                    .metadata
                    .unresolved_manual
                    .iter()
                    .any(|value| same(value, correction))
                {
                    current.metadata.unresolved_manual.push(correction.clone());
                }
            }
        } else {
            retained.extend(
                old.metadata
                    .manual
                    .iter()
                    .chain(old.metadata.unresolved_manual.iter())
                    .cloned(),
            );
        }
    }
    for row in rows.iter_mut() {
        if let Some(manual) = row.metadata.manual.as_mut()
            && manual.source_fingerprint != row.metadata.source_fingerprint
        {
            manual.review_required = true;
        }
    }
    for mut correction in retained {
        if rows.iter().any(|row| {
            row.metadata
                .manual
                .iter()
                .chain(row.metadata.unresolved_manual.iter())
                .any(|value| same(value, &correction))
        }) {
            continue;
        }
        correction.review_required = true;
        rows.first_mut()
            .ok_or_else(|| {
                PennelError::storage(
                    "Removing this transcript would discard retained speaker corrections.",
                )
            })?
            .metadata
            .unresolved_manual
            .push(correction);
    }
    Ok(())
}

pub(crate) fn refresh_for_update(
    previous: &MeetingTranscriptSnapshot,
    rows: &mut [MeetingUtterance],
) -> Result<(), PennelError> {
    let old: std::collections::HashMap<_, _> = previous
        .utterances
        .iter()
        .map(|row| (row.metadata.utterance_id.as_str(), &row.metadata))
        .collect();
    for row in rows.iter_mut() {
        if row.segment.failed {
            row.segment.text.clear();
            row.metadata.spans.clear();
        }
        if let Some(previous) = old.get(row.metadata.utterance_id.as_str()) {
            if previous.source_id != row.metadata.source_id {
                return Err(PennelError::storage(
                    "A speaker edit cannot change an utterance's physical source.",
                ));
            }
            if row.metadata.attribution_revision < previous.attribution_revision {
                return Err(PennelError::storage(
                    "A newer speaker assignment is already saved.",
                ));
            }
            if let Some(recovery) = &previous.recovery_audio {
                if let Some(current) = &row.metadata.recovery_audio {
                    if !recovery.same_chunk(current)
                        || !recovery.sha256.is_empty() && recovery.sha256 != current.sha256
                        || recovery.disposition == RecoveryAudioDisposition::Complete
                            && current.disposition != RecoveryAudioDisposition::Complete
                        || recovery.disposition == RecoveryAudioDisposition::Unverified
                            && current.disposition != RecoveryAudioDisposition::Unverified
                    {
                        return Err(PennelError::storage(
                            "An utterance's original recovery audio cannot be changed by an edit.",
                        ));
                    }
                } else {
                    row.metadata.recovery_audio = Some(recovery.clone());
                }
            }
        }
        row.metadata.refresh_fingerprints(&row.segment);
    }
    preserve_manual_work(previous, rows)
}

pub(crate) fn prepare_metadata(
    uid: &str,
    revision: u64,
    text: &[u8],
    rows: &[MeetingUtterance],
) -> Result<PreparedMetadata, PennelError> {
    let mut ids = HashSet::new();
    let mut parts = Vec::new();
    let mut current = Vec::from(&b"["[..]);
    let mut current_rows = 0;
    let mut descriptors = Vec::new();
    let mut sources = HashSet::new();
    for row in rows {
        if !ids.insert(row.metadata.utterance_id.clone()) || !validate_row(row) {
            return Err(PennelError::storage(
                "The timed transcript metadata is invalid.",
            ));
        }
        if let Some(source) = &row.metadata.source_id {
            sources.insert(source.clone());
        }
        let encoded = serde_json::to_vec(&row.metadata).map_err(PennelError::storage)?;
        if encoded.len() + 2 > METADATA_PART_BYTES {
            return Err(PennelError::storage(
                "One transcript metadata entry is too large to sync safely.",
            ));
        }
        if current.len() + encoded.len() + usize::from(current_rows > 0) + 1 > METADATA_PART_BYTES {
            current.push(b']');
            let sha256 = digest(&current);
            descriptors.push(MetadataPart {
                sha256: sha256.clone(),
                rows: current_rows,
            });
            parts.push((part_name(&sha256).expect("generated digest"), current));
            current = Vec::from(&b"["[..]);
            current_rows = 0;
        }
        if current_rows > 0 {
            current.push(b',');
        }
        current.extend(encoded);
        current_rows += 1;
    }
    if current_rows > 0 {
        current.push(b']');
        let sha256 = digest(&current);
        descriptors.push(MetadataPart {
            sha256: sha256.clone(),
            rows: current_rows,
        });
        parts.push((part_name(&sha256).expect("generated digest"), current));
    }
    let mut sources: Vec<_> = sources.into_iter().collect();
    sources.sort();
    if descriptors.len() > 4096 {
        return Err(PennelError::storage(
            "The transcript metadata contains too many parts.",
        ));
    }
    let manifest = serde_json::to_vec(&MetadataManifest {
        schema_version: match (
            rows.iter().any(|row| row.metadata.recovery_audio.is_some()),
            rows.iter().any(|row| row.metadata.attribution.provisional),
        ) {
            (true, true) => 4,
            (true, false) => 3,
            (false, true) => 2,
            (false, false) => 1,
        },
        meeting_uid: uid.into(),
        transcript_sha256: digest(text),
        revision,
        sources,
        parts: descriptors,
    })
    .map_err(PennelError::storage)?;
    if manifest.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(PennelError::storage(
            "The transcript metadata index is too large to sync safely.",
        ));
    }
    Ok(PreparedMetadata { manifest, parts })
}

pub(crate) fn install_parts(root: &Path, metadata: &PreparedMetadata) -> Result<(), PennelError> {
    if let Some(mut state) = ownership(root) {
        let mut hashes: HashSet<_> = state.hashes.into_iter().collect();
        hashes.extend(metadata.parts.iter().map(|(_, bytes)| digest(bytes)));
        state.hashes = hashes.into_iter().collect();
        state.hashes.sort();
        save_ownership(root, &state)?;
    }

    for (name, bytes) in &metadata.parts {
        let path = root.join(name);
        if path.exists() {
            if read_bounded(&path, METADATA_PART_BYTES as u64)? != *bytes {
                return Err(PennelError::storage(
                    "A transcript metadata part changed unexpectedly.",
                ));
            }
        } else {
            durable_write(&path, bytes)?;
        }
    }
    Ok(())
}

pub(crate) fn install_manifest(
    root: &Path,
    metadata: &PreparedMetadata,
) -> Result<(), PennelError> {
    durable_write(&root.join(MANIFEST), &metadata.manifest)
}

pub(crate) fn commit_snapshot(
    root: &Path,
    text: &[u8],
    metadata: &PreparedMetadata,
) -> Result<(), PennelError> {
    if root.join(SNAPSHOT_JOURNAL).exists() {
        return Err(PennelError::storage(
            "A transcript save needs recovery before another write.",
        ));
    }
    install_parts(root, metadata)?;
    let token = uuid::Uuid::now_v7().to_string();
    let staged_text = root.join(format!(".pennel-transcript-{token}.md"));
    let staged_manifest = root.join(format!(".pennel-transcript-{token}.json"));
    durable_write(&staged_text, text)?;
    durable_write(&staged_manifest, &metadata.manifest)?;
    let journal = SnapshotJournal {
        version: 1,
        token,
        transcript_sha256: digest(text),
        manifest_sha256: digest(&metadata.manifest),
    };
    durable_write(
        &root.join(SNAPSHOT_JOURNAL),
        &serde_json::to_vec(&journal).map_err(PennelError::storage)?,
    )?;
    recover_snapshot(root)
}

pub(crate) fn recover_snapshot(root: &Path) -> Result<(), PennelError> {
    let bytes = match fs::read(root.join(SNAPSHOT_JOURNAL)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(PennelError::storage(error)),
    };
    let journal: SnapshotJournal = serde_json::from_slice(&bytes).map_err(PennelError::storage)?;
    if journal.version != 1
        || uuid::Uuid::parse_str(&journal.token).is_err()
        || !valid_digest(&journal.transcript_sha256)
        || !valid_digest(&journal.manifest_sha256)
    {
        return Err(PennelError::storage(
            "The transcript recovery journal is invalid.",
        ));
    }
    let staged_text = root.join(format!(".pennel-transcript-{}.md", journal.token));
    let staged_manifest = root.join(format!(".pennel-transcript-{}.json", journal.token));
    let text = fs::read(&staged_text).map_err(PennelError::storage)?;
    let manifest = read_bounded(&staged_manifest, MAX_MANIFEST_BYTES)?;
    if digest(&text) != journal.transcript_sha256 || digest(&manifest) != journal.manifest_sha256 {
        return Err(PennelError::storage(
            "The transcript recovery files changed.",
        ));
    }
    let parsed: MetadataManifest =
        serde_json::from_slice(&manifest).map_err(PennelError::storage)?;
    if !supported_metadata_schema(u64::from(parsed.schema_version))
        || parsed.revision == 0
        || parsed.transcript_sha256 != journal.transcript_sha256
        || !crate::meetings::valid_uid(&parsed.meeting_uid)
        || parsed.parts.len() > 4096
    {
        return Err(PennelError::storage(
            "The staged transcript metadata is invalid.",
        ));
    }
    for part in parsed.parts {
        let name = part_name(&part.sha256)
            .ok_or_else(|| PennelError::storage("The staged transcript part is invalid."))?;
        if digest(&read_bounded(&root.join(name), METADATA_PART_BYTES as u64)?) != part.sha256 {
            return Err(PennelError::storage("A staged transcript part changed."));
        }
    }
    durable_write(&root.join("transcript.md"), &text)?;
    durable_write(&root.join(MANIFEST), &manifest)?;
    fs::remove_file(root.join(SNAPSHOT_JOURNAL)).map_err(PennelError::storage)?;
    crate::meetings::sync_directory(root)?;
    // These exact temporary paths belong to this completed transaction; immutable parts are retained.
    let _ = fs::remove_file(staged_text);
    let _ = fs::remove_file(staged_manifest);
    let _ = prune_owned_parts(root);
    Ok(())
}

pub(crate) fn backup_metadata(root: &Path, backup: &Path) -> Result<(), PennelError> {
    let mut files = Vec::new();
    if root.join(MANIFEST).exists() {
        let manifest = read_bounded(&root.join(MANIFEST), MAX_MANIFEST_BYTES)?;
        let parsed: MetadataManifest =
            serde_json::from_slice(&manifest).map_err(PennelError::storage)?;
        for part in parsed.parts {
            let name = part_name(&part.sha256)
                .ok_or_else(|| PennelError::storage("The transcript metadata part is invalid."))?;
            let bytes = read_bounded(&root.join(&name), METADATA_PART_BYTES as u64)?;
            if digest(&bytes) != part.sha256 {
                return Err(PennelError::storage(
                    "The transcript metadata part changed.",
                ));
            }
            durable_write(&backup.join(&name), &bytes)?;
            files.push(name);
        }
        durable_write(&backup.join(MANIFEST), &manifest)?;
        files.push(MANIFEST.into());
    }
    durable_write(
        &backup.join("transcript-metadata-backup.json"),
        &serde_json::to_vec(&MetadataBackup { version: 1, files }).map_err(PennelError::storage)?,
    )
}

pub(crate) fn restore_metadata(root: &Path, backup: &Path) -> Result<(), PennelError> {
    let state = match fs::read(backup.join("transcript-metadata-backup.json")) {
        Ok(bytes) => {
            serde_json::from_slice::<MetadataBackup>(&bytes).map_err(PennelError::storage)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(PennelError::storage(error)),
    };
    if state.version != 1
        || state.files.iter().any(|name| {
            name != MANIFEST
                && name
                    .strip_prefix("transcript.meta.")
                    .and_then(|value| value.strip_suffix(".json"))
                    .is_none_or(|hash| !valid_digest(hash))
        })
    {
        return Err(PennelError::storage(
            "The transcript metadata backup is invalid.",
        ));
    }
    let saved = state
        .files
        .iter()
        .map(|name| {
            read_bounded(&backup.join(name), METADATA_PART_BYTES as u64).map(|bytes| (name, bytes))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (name, bytes) in &saved {
        if name.as_str() != MANIFEST {
            durable_write(&root.join(name), bytes)?;
        }
    }
    if let Some((_, manifest)) = saved.iter().find(|(name, _)| name.as_str() == MANIFEST) {
        durable_write(&root.join(MANIFEST), manifest)?;
    } else if root.join(MANIFEST).exists() {
        fs::remove_file(root.join(MANIFEST)).map_err(PennelError::storage)?;
    }
    Ok(())
}

pub(crate) fn reconcile(
    previous: &MeetingTranscriptSnapshot,
    segments: &[Segment],
) -> Result<Vec<MeetingUtterance>, PennelError> {
    let normalized = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut used = HashSet::new();
    let mut rows = Vec::with_capacity(segments.len());
    for segment in segments {
        let candidates: Vec<_> = previous
            .utterances
            .iter()
            .enumerate()
            .filter(|(_, old)| {
                old.segment.start_ms == segment.start_ms
                    && old.segment.failed == segment.failed
                    && normalized(&old.segment.text) == normalized(&segment.text)
            })
            .collect();
        let candidate = (candidates.len() == 1).then(|| candidates[0]);
        if let Some((index, old)) = candidate.filter(|(index, _)| !used.contains(index)) {
            used.insert(index);
            let mut row = old.clone();
            row.segment.speaker = segment.speaker;
            row.segment.voice = segment.voice;
            if row.segment.speaker != old.segment.speaker || row.segment.voice != old.segment.voice
            {
                row.metadata.attribution_revision =
                    row.metadata.attribution_revision.saturating_add(1);
            }
            if row.segment.end_ms == row.segment.start_ms && segment.end_ms > segment.start_ms {
                row.segment.end_ms = segment.end_ms;
                row.metadata.refresh_fingerprints(&row.segment);
            }
            rows.push(row);
        } else {
            let mut row = MeetingUtterance::new(segment.clone(), None);
            preserve_legacy_label(&mut row);
            rows.push(row);
        }
    }
    let mut unresolved = Vec::new();
    for (index, row) in previous
        .utterances
        .iter()
        .enumerate()
        .filter(|(index, _)| !used.contains(index))
    {
        let _ = index;
        unresolved.extend(
            row.metadata
                .manual
                .iter()
                .chain(row.metadata.unresolved_manual.iter())
                .cloned()
                .map(|mut correction| {
                    correction.review_required = true;
                    correction
                }),
        );
    }
    if !unresolved.is_empty() {
        let row = rows.first_mut().ok_or_else(|| {
            PennelError::storage(
                "Removing this transcript would discard retained speaker corrections.",
            )
        })?;
        for correction in unresolved {
            if !row.metadata.unresolved_manual.contains(&correction) {
                row.metadata.unresolved_manual.push(correction);
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::{MeetingStore, Speaker};

    fn fixture() -> (tempfile::TempDir, MeetingStore, String) {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store.create("Durable turns", chrono::Local::now()).unwrap();
        (directory, store, id)
    }

    fn utterance(start_ms: u64, text: &str) -> MeetingUtterance {
        MeetingUtterance::new(
            Segment {
                start_ms,
                end_ms: start_ms + 400,
                speaker: Speaker::Others,
                text: text.into(),
                failed: false,
                voice: Some(2),
            },
            Some("microphone".into()),
        )
    }

    #[test]
    fn canonical_identity_source_end_and_exact_text_survive_reopen_and_legacy_write() {
        let (_directory, store, id) = fixture();
        let mut row = utterance(1_050, "Ja,  zeker.\nVolgende regel.");
        row.metadata.spans = vec![TimedTextSpan(0, 3, 1_050, 1_170)];
        let first = store
            .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
            .unwrap();
        let reopened = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(reopened, first);
        assert_eq!(reopened.utterances[0], row);
        let mut projected = reopened.utterances[0].segment.clone();
        projected.voice = Some(3);
        store.write_transcript(&id, &[projected]).unwrap();
        let changed = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(
            changed.utterances[0].metadata.utterance_id,
            row.metadata.utterance_id
        );
        assert_eq!(
            changed.utterances[0].metadata.source_id,
            Some("microphone".into())
        );
        assert_eq!(changed.utterances[0].segment.end_ms, 1_450);
        assert!(
            store
                .write_transcript_snapshot(&id, Some(0), &[row])
                .is_err()
        );
    }

    #[test]
    fn provisional_attribution_is_versioned_and_cannot_be_cleared_by_an_ordinary_save() {
        let (_directory, store, id) = fixture();
        let mut row = utterance(1_000, "Owned provisional voice.");
        row.metadata.attribution.kind = AttributionKind::Single;
        row.metadata.attribution.voices = vec![2];
        row.metadata.attribution.provisional = true;
        store
            .write_transcript_snapshot(&id, Some(0), &[row])
            .unwrap();
        let root = store.dir_of(&id);
        let manifest = || {
            serde_json::from_slice::<serde_json::Value>(&fs::read(root.join(MANIFEST)).unwrap())
                .unwrap()
        };
        assert_eq!(manifest()["schema_version"], 2);
        let mut projection = store.read_transcript(&id);
        projection[0].voice = Some(3);
        store.write_transcript(&id, &projection).unwrap();
        let current = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(current.metadata_status, TranscriptMetadataStatus::Valid);
        assert!(current.utterances[0].metadata.attribution.provisional);
        assert_eq!(manifest()["schema_version"], 2);
        let mut final_rows = current.utterances;
        final_rows[0].metadata.attribution.provisional = false;
        final_rows[0].metadata.attribution_revision += 1;
        store
            .write_transcript_snapshot(&id, Some(current.revision), &final_rows)
            .unwrap();
        assert_eq!(manifest()["schema_version"], 1);
        assert!(
            !store.read_transcript_snapshot(&id).unwrap().utterances[0]
                .metadata
                .attribution
                .provisional
        );
    }

    #[test]
    fn unsupported_and_partial_metadata_never_overwrite_or_apply_a_subset() {
        let (_directory, store, id) = fixture();
        store
            .write_transcript(&id, &[utterance(0, "Keep words.").segment])
            .unwrap();
        let root = store.dir_of(&id);
        fs::write(root.join(MANIFEST), br#"{"schema_version":9000}"#).unwrap();
        let before = fs::read(root.join(MANIFEST)).unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().metadata_status,
            TranscriptMetadataStatus::Unsupported
        );
        assert!(store.write_transcript(&id, &[]).is_err());
        assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
    }

    #[test]
    fn recovery_proof_and_provisional_attribution_require_distinct_schema_capabilities() {
        let (_directory, store, id) = fixture();
        let path = store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.2; 16_000], 16_000),
            )
            .unwrap();
        let mut row = utterance(0, "Owned versioned recovery.");
        row.segment.end_ms = 1_000;
        row.metadata.refresh_fingerprints(&row.segment);
        row.metadata.recovery_audio =
            crate::meetings::recovery_provenance(&path, RecoveryAudioDisposition::Complete);
        let snapshot = store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let root = store.dir_of(&id);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], 3);
        let mut rows = snapshot.utterances;
        rows[0].metadata.attribution.provisional = true;
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &rows)
            .unwrap();
        manifest = serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], 4);
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().metadata_status,
            TranscriptMetadataStatus::Valid
        );
        for unsupported_combination in [1, 2, 3] {
            manifest["schema_version"] = unsupported_combination.into();
            fs::write(root.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert_eq!(
                store.read_transcript_snapshot(&id).unwrap().metadata_status,
                TranscriptMetadataStatus::Corrupt
            );
            assert!(store.write_transcript_snapshot(&id, None, &rows).is_err());
            assert!(path.exists());
        }
    }

    #[test]
    fn speaker_edits_cannot_replace_or_downgrade_original_recovery_proof() {
        let (_directory, store, id) = fixture();
        let path = store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.2; 16_000], 16_000),
            )
            .unwrap();
        let mut row = utterance(0, "Owned immutable recovery.");
        row.segment.end_ms = 1_000;
        row.metadata.refresh_fingerprints(&row.segment);
        row.metadata.recovery_audio =
            crate::meetings::recovery_provenance(&path, RecoveryAudioDisposition::Complete);
        let snapshot = store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        for mutate in [false, true] {
            let mut rows = snapshot.utterances.clone();
            let proof = rows[0].metadata.recovery_audio.as_mut().unwrap();
            if mutate {
                proof.sha256 = "f".repeat(64);
            } else {
                proof.disposition = RecoveryAudioDisposition::Pending;
            }
            assert!(
                store
                    .write_transcript_snapshot(&id, Some(snapshot.revision), &rows)
                    .is_err()
            );
        }
        let mut rows = snapshot.utterances.clone();
        rows[0].metadata.recovery_audio = None;
        let saved = store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &rows)
            .unwrap();
        assert_eq!(
            saved.utterances[0].metadata.recovery_audio,
            snapshot.utterances[0].metadata.recovery_audio
        );
    }

    #[test]
    fn legacy_ids_are_deterministic_without_mutating_files_and_unparsed_text_is_protected() {
        let (_directory, store, id) = fixture();
        let root = store.dir_of(&id);
        fs::write(
            root.join("transcript.md"),
            "[00:01] **Ik:** Kept.\nCustom opaque paragraph\n",
        )
        .unwrap();
        let first = store.read_transcript_snapshot(&id).unwrap();
        let second = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(first, second);
        assert!(!root.join(MANIFEST).exists());
        let original = fs::read(root.join("transcript.md")).unwrap();
        assert!(
            store
                .write_transcript(&id, &[utterance(0, "Replacement").segment])
                .is_err()
        );
        assert_eq!(fs::read(root.join("transcript.md")).unwrap(), original);
    }

    #[test]
    fn missing_or_changed_part_retains_text_and_refuses_attribution_writes() {
        let (_directory, store, id) = fixture();
        let row = utterance(500, "Exact source.");
        let first = store
            .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
            .unwrap();
        let root = store.dir_of(&id);
        let manifest: MetadataManifest =
            serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
        let part = root.join(part_name(&manifest.parts[0].sha256).unwrap());
        let bytes = fs::read(&part).unwrap();
        fs::remove_file(&part).unwrap();
        let pending = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(pending.metadata_status, TranscriptMetadataStatus::Pending);
        assert_eq!(pending.utterances[0].segment.text, row.segment.text);
        assert_eq!(pending.utterances[0].metadata.source_id, None);
        assert!(store.write_transcript(&id, &[]).is_err());
        fs::write(&part, &bytes).unwrap();
        assert_eq!(store.read_transcript_snapshot(&id).unwrap(), first);
        fs::write(&part, b"[]").unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().metadata_status,
            TranscriptMetadataStatus::Corrupt
        );
        assert!(
            store
                .write_transcript_snapshot(&id, Some(1), &[row])
                .is_err()
        );
    }

    #[test]
    fn interrupted_snapshot_save_rolls_forward_without_partial_metadata() {
        for (boundary, provisional, with_recovery) in (0..3).flat_map(|boundary| {
            [false, true].into_iter().flat_map(move |provisional| {
                [false, true].map(move |with_recovery| (boundary, provisional, with_recovery))
            })
        }) {
            let (_directory, store, id) = fixture();
            let old = store
                .write_transcript_snapshot(&id, Some(0), &[utterance(0, "Previous.")])
                .unwrap();
            let root = store.dir_of(&id);
            let mut next = utterance(700, "Committed new words.");
            next.metadata.attribution.provisional = provisional;
            let recovery = with_recovery.then(|| {
                store
                    .write_recovery(
                        &id,
                        Speaker::Me,
                        700,
                        &crate::wav::encode_wav(&vec![0.25; 16_000], 16_000),
                    )
                    .unwrap()
            });
            if let Some(path) = &recovery {
                next.metadata.recovery_audio =
                    crate::meetings::recovery_provenance(path, RecoveryAudioDisposition::Complete);
            }
            let text = crate::meetings::render_transcript(std::slice::from_ref(&next.segment));
            let prepared = prepare_metadata(
                &old.meeting_uid,
                2,
                text.as_bytes(),
                std::slice::from_ref(&next),
            )
            .unwrap();
            install_parts(&root, &prepared).unwrap();
            let token = uuid::Uuid::now_v7().to_string();
            fs::write(
                root.join(format!(".pennel-transcript-{token}.md")),
                text.as_bytes(),
            )
            .unwrap();
            fs::write(
                root.join(format!(".pennel-transcript-{token}.json")),
                &prepared.manifest,
            )
            .unwrap();
            let journal = SnapshotJournal {
                version: 1,
                token,
                transcript_sha256: digest(text.as_bytes()),
                manifest_sha256: digest(&prepared.manifest),
            };
            durable_write(
                &root.join(SNAPSHOT_JOURNAL),
                &serde_json::to_vec(&journal).unwrap(),
            )
            .unwrap();
            if boundary >= 1 {
                durable_write(&root.join("transcript.md"), text.as_bytes()).unwrap();
            }
            if boundary >= 2 {
                durable_write(&root.join(MANIFEST), &prepared.manifest).unwrap();
            }
            assert_ne!(
                store.read_transcript_snapshot(&id).unwrap().metadata_status,
                TranscriptMetadataStatus::Valid
            );
            store.recover_retranscriptions().unwrap();
            let recovered = store.read_transcript_snapshot(&id).unwrap();
            assert_eq!(recovered.metadata_status, TranscriptMetadataStatus::Valid);
            assert_eq!(recovered.utterances, vec![next]);
            assert_eq!(recovered.revision, 2);
            assert!(!root.join(SNAPSHOT_JOURNAL).exists());
            if let Some(path) = recovery {
                assert_eq!(store.cleanup_committed_audio(&id).unwrap(), 1);
                assert!(!path.exists());
            }
        }
    }

    #[test]
    fn replacement_rollback_restores_exact_metadata_and_prior_absence() {
        for rich in [false, true] {
            let (_directory, store, id) = fixture();
            let root = store.dir_of(&id);
            let row = utterance(1_000, "Original source.");
            if rich {
                store
                    .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
                    .unwrap();
            } else {
                fs::write(
                    root.join("transcript.md"),
                    crate::meetings::render_transcript(std::slice::from_ref(&row.segment)),
                )
                .unwrap();
            }
            let original = store.read_transcript_snapshot(&id).unwrap();
            let mut replacement = original.utterances.clone();
            replacement[0].segment.voice = Some(4);
            let backup = store
                .replace_speaker_utterances(&id, &replacement, &[], &[Speaker::Me])
                .unwrap();
            assert_eq!(
                store.read_transcript_snapshot(&id).unwrap().utterances[0]
                    .segment
                    .voice,
                Some(4)
            );
            store.restore_retranscription(&id, &backup).unwrap();
            assert_eq!(store.read_transcript_snapshot(&id).unwrap(), original);
            assert_eq!(root.join(MANIFEST).exists(), rich);
        }
    }

    #[test]
    fn sixty_and_120_minute_metadata_remain_bounded_parts_with_complete_identity() {
        for minutes in [60, 120] {
            let (_directory, store, id) = fixture();
            let rows: Vec<_> = (0..minutes * 120)
                .map(|turn| {
                    let mut row = utterance(turn * 500, "Ja. Yes.");
                    row.metadata.spans = vec![
                        TimedTextSpan(0, 3, turn * 500, turn * 500 + 200),
                        TimedTextSpan(4, 8, turn * 500 + 200, turn * 500 + 400),
                    ];
                    row
                })
                .collect();
            let saved = store
                .write_transcript_snapshot(&id, Some(0), &rows)
                .unwrap();
            let manifest: MetadataManifest =
                serde_json::from_slice(&fs::read(store.dir_of(&id).join(MANIFEST)).unwrap())
                    .unwrap();
            assert!(manifest.parts.len() > 1);
            let mut count = 0;
            for part in manifest.parts {
                let bytes =
                    fs::read(store.dir_of(&id).join(part_name(&part.sha256).unwrap())).unwrap();
                assert!(bytes.len() <= METADATA_PART_BYTES);
                assert!(bytes.len().div_ceil(3) * 4 + 65_536 < 5_242_880);
                assert_eq!(digest(&bytes), part.sha256);
                count += part.rows;
            }
            assert_eq!(count, rows.len());
            assert_eq!(saved.utterances, rows);
            assert_eq!(store.read_transcript_snapshot(&id).unwrap(), saved);
        }
    }

    #[test]
    fn mutable_role_never_establishes_an_ambiguous_legacy_physical_source() {
        let (_directory, store, id) = fixture();
        fs::write(
            store.dir_of(&id).join("transcript.md"),
            "[00:01] **Ik:** A person in the room.\n",
        )
        .unwrap();
        let original = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(original.utterances[0].metadata.source_id, None);
        store
            .write_transcript(&id, &[original.utterances[0].segment.clone()])
            .unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().utterances[0]
                .metadata
                .source_id,
            None
        );
        let mut invalid = store.read_transcript_snapshot(&id).unwrap().utterances;
        invalid[0].metadata.source_id = Some("microphone".into());
        assert!(
            store
                .write_transcript_snapshot(&id, None, &invalid)
                .is_err()
        );
    }

    #[test]
    fn renamed_voice_records_manual_provenance_without_changing_source_or_notes() {
        let (_directory, store, id) = fixture();
        let row = utterance(0, "One actual person.");
        store
            .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
            .unwrap();
        store
            .write_notes(&id, "Personal note remains exact.")
            .unwrap();
        store.rename_speaker(&id, 2, "Confirmed person").unwrap();
        let saved = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(
            saved.utterances[0].metadata.utterance_id,
            row.metadata.utterance_id
        );
        assert_eq!(
            saved.utterances[0].metadata.source_id,
            row.metadata.source_id
        );
        assert_eq!(
            saved.utterances[0]
                .metadata
                .manual
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("Confirmed person")
        );
        assert_eq!(
            saved.utterances[0]
                .metadata
                .manual
                .as_ref()
                .unwrap()
                .source_fingerprint,
            row.metadata.source_fingerprint
        );
        assert_eq!(store.read_notes(&id), "Personal note remains exact.");
    }

    #[test]
    fn explicit_acoustic_self_remains_distinct_from_manual_person_and_physical_source() {
        let (_directory, store, id) = fixture();
        let mut row = utterance(0, "An identified voice.");
        row.segment.speaker = Speaker::Me;
        row.segment.voice = None;
        row.metadata.attribution = AutomaticAttribution {
            kind: AttributionKind::Single,
            voices: Vec::new(),
            self_voice: true,
            provisional: false,
            turns: Vec::new(),
        };
        store
            .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
            .unwrap();
        row.segment.speaker = Speaker::Others;
        row.segment.voice = Some(3);
        row.metadata.manual = Some(ManualCorrection {
            name: Some("Corrected person".into()),
            voice: Some(3),
            source_fingerprint: row.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        let saved = store
            .write_transcript_snapshot(&id, Some(1), &[row])
            .unwrap();
        assert!(saved.utterances[0].metadata.attribution.self_voice);
        assert_eq!(
            saved.utterances[0].metadata.manual.as_ref().unwrap().voice,
            Some(3)
        );
        assert_eq!(
            saved.utterances[0].metadata.source_id.as_deref(),
            Some("microphone")
        );
    }

    #[test]
    fn replacement_cannot_discard_a_correction_when_identity_remapping_is_ambiguous() {
        let (_directory, store, id) = fixture();
        let mut original = utterance(0, "Old confirmed source.");
        original.metadata.manual = Some(ManualCorrection {
            name: Some("Reviewed person".into()),
            voice: Some(2),
            source_fingerprint: original.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        store
            .write_transcript_snapshot(&id, Some(0), &[original])
            .unwrap();
        let replacement = utterance(500, "Changed source with uncertain mapping.");
        let saved = store
            .write_transcript_snapshot(&id, Some(1), &[replacement])
            .unwrap();
        assert!(saved.utterances[0].metadata.manual.is_none());
        let retained = &saved.utterances[0].metadata.unresolved_manual;
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].name.as_deref(), Some("Reviewed person"));
        assert!(retained[0].review_required);
        assert!(store.write_transcript_snapshot(&id, Some(2), &[]).is_err());
        assert_eq!(store.read_transcript_snapshot(&id).unwrap(), saved);
    }

    #[test]
    fn first_legacy_mutation_preserves_the_id_seen_before_a_meeting_uid_existed() {
        let (_directory, store, id) = fixture();
        let root = store.dir_of(&id);
        let meeting = fs::read_to_string(root.join("meeting.md")).unwrap();
        let without_uid = meeting
            .lines()
            .filter(|line| !line.starts_with("pennel_id:"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("meeting.md"), without_uid).unwrap();
        fs::write(
            root.join("transcript.md"),
            "[00:01] **Anderen 2:** Saved legacy words.\n",
        )
        .unwrap();
        let original = store.read_transcript_snapshot(&id).unwrap();
        assert!(store.uid_of(&id).is_none());
        store
            .write_transcript(&id, &[original.utterances[0].segment.clone()])
            .unwrap();
        let saved = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(
            saved.utterances[0].metadata.utterance_id,
            original.utterances[0].metadata.utterance_id
        );
        assert_eq!(saved.utterances[0].metadata.source_id, None);
        assert_eq!(saved.utterances[0].metadata.attribution.voices, vec![2]);
        assert!(store.uid_of(&id).is_some());
    }

    #[test]
    fn hour_of_appends_reclaims_only_owned_superseded_parts_and_keeps_backup_copies() {
        let (_directory, store, id) = fixture();
        let root = store.dir_of(&id);
        let mut rows = vec![utterance(0, "First confirmed text.")];
        store
            .write_transcript_snapshot(&id, Some(0), &rows)
            .unwrap();
        let backup = store
            .replace_speaker_utterances(&id, &rows, &[], &[Speaker::Me])
            .unwrap();
        store.finish_retranscription(&id).unwrap();
        let saved_backup = fs::read(backup.join(MANIFEST)).unwrap();
        let unknown_bytes =
            serde_json::to_vec(&vec![utterance(0, "User-kept metadata.").metadata]).unwrap();
        let unknown = root.join(part_name(&digest(&unknown_bytes)).unwrap());
        fs::write(&unknown, &unknown_bytes).unwrap();
        for chunk in 1..=240 {
            rows.push(utterance(chunk * 15_000, "A new finalized turn."));
            store.write_transcript_snapshot(&id, None, &rows).unwrap();
            let manifest: MetadataManifest =
                serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
            let parts = fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.starts_with("transcript.meta.") && name != MANIFEST
                })
                .count();
            assert_eq!(
                parts,
                manifest.parts.len() + 1,
                "Unbounded generated parts after chunk {chunk}"
            );
        }
        assert_eq!(fs::read(&unknown).unwrap(), unknown_bytes);
        assert_eq!(fs::read(backup.join(MANIFEST)).unwrap(), saved_backup);
        assert!(backup_copies_complete(&root));
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().utterances,
            rows
        );
    }

    #[test]
    fn unknown_ownership_version_fails_closed_without_rewriting_the_ledger() {
        let (_directory, store, id) = fixture();
        let root = store.dir_of(&id);
        store
            .write_transcript_snapshot(&id, Some(0), &[utterance(0, "Kept generation.")])
            .unwrap();
        let owned = root.join(".pennel").join(OWNED_PARTS);
        let future = br#"{"version":500,"hashes":[]}"#;
        fs::write(&owned, future).unwrap();
        let before: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        store
            .write_transcript_snapshot(&id, Some(1), &[utterance(1_000, "Next generation.")])
            .unwrap();
        assert_eq!(fs::read(&owned).unwrap(), future);
        assert!(before.iter().all(|path| path.exists()));
    }

    #[test]
    fn legacy_failure_marking_with_old_text_and_recovery_audio_keeps_valid_metadata() {
        let (_directory, store, id) = fixture();
        let mut segment = utterance(0, "Previous successful source.").segment;
        store
            .write_transcript(&id, std::slice::from_ref(&segment))
            .unwrap();
        segment.failed = true;
        segment.text = "Old text must not become failed evidence.".into();
        store.write_transcript(&id, &[segment]).unwrap();
        let path = crate::session::failed_path(&store.dir_of(&id), Speaker::Others, 0);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, crate::wav::encode_wav(&vec![0.25; 32_000], 16_000)).unwrap();
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(snapshot.metadata_status, TranscriptMetadataStatus::Valid);
        assert_eq!(snapshot.utterances.len(), 1);
        assert!(snapshot.utterances[0].segment.failed);
        assert!(snapshot.utterances[0].segment.text.is_empty());
        assert_eq!(snapshot.utterances[0].segment.end_ms, 2_000);
    }
    #[test]
    fn recovery_schema_preserves_future_formats_read_only() {
        let (_directory, store, id) = fixture();
        let path = store
            .write_recovery(
                &id,
                Speaker::Me,
                0,
                &crate::wav::encode_wav(&vec![0.2; 16_000], 16_000),
            )
            .unwrap();
        let mut row = utterance(0, "Owned versioned recovery.");
        row.segment.end_ms = 1_000;
        row.metadata.refresh_fingerprints(&row.segment);
        row.metadata.recovery_audio =
            crate::meetings::recovery_provenance(&path, RecoveryAudioDisposition::Complete);
        let saved = store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        let root = store.dir_of(&id);
        let original = fs::read(root.join(MANIFEST)).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(manifest["schema_version"], 3);
        assert_eq!(saved.metadata_status, TranscriptMetadataStatus::Valid);
        let text = fs::read(root.join("transcript.md")).unwrap();
        for version in [5, 9_000] {
            let mut future = manifest.clone();
            future["schema_version"] = version.into();
            let bytes = serde_json::to_vec(&future).unwrap();
            fs::write(root.join(MANIFEST), &bytes).unwrap();
            assert_eq!(
                store.read_transcript_snapshot(&id).unwrap().metadata_status,
                TranscriptMetadataStatus::Unsupported
            );
            assert!(
                store
                    .write_transcript_snapshot(&id, None, &saved.utterances)
                    .is_err()
            );
            assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), bytes);
            assert_eq!(fs::read(root.join("transcript.md")).unwrap(), text);
            assert!(path.exists());
        }
        let mut invalid = manifest.clone();
        invalid["schema_version"] = 1.into();
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(root.join(MANIFEST), &bytes).unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().metadata_status,
            TranscriptMetadataStatus::Corrupt
        );
        assert!(
            store
                .write_transcript_snapshot(&id, None, &saved.utterances)
                .is_err()
        );
        assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), bytes);
        fs::write(root.join(MANIFEST), original).unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().metadata_status,
            TranscriptMetadataStatus::Valid
        );
    }
}
