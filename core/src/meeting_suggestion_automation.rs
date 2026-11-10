use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::meetings::{MeetingStore, durable_write, replacement_gate, sync_directory};
use crate::{MeetingTranscriptSnapshot, PennelError};

const PREFIX: &str = "pennel.suggestions.automatic.";
const MAX_MARKER_BYTES: u64 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomaticInsightOutcome {
    Completed,
    Failed,
    Cancelled,
}

/// A consumed attempt is separate from insights.json so older readers retain writable review data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AutomaticInsightAttempt {
    pub schema_version: u32,
    pub meeting_uid: String,
    pub transcript_digest: String,
    pub language_policy: String,
    pub admitted_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<AutomaticInsightOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_found: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

fn unavailable() -> PennelError {
    PennelError::storage("The automatic suggestion attempt could not be read or saved.")
}

fn digest_valid(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn marker_path(directory: &Path, digest: &str) -> Result<PathBuf, PennelError> {
    if !digest_valid(digest) {
        return Err(unavailable());
    }
    Ok(directory.join(format!("{PREFIX}{digest}.json")))
}

fn error_code_valid(code: &str) -> bool {
    matches!(
        code,
        "cancelled"
            | "failed"
            | "configuration_changed"
            | "deadline_exceeded"
            | "source_changed"
            | "meeting_busy"
            | "source_unavailable"
            | "invalid_output"
            | "identity_changed"
            | "save_failed"
            | "provider_failed"
            | "missing_api_key"
            | "not_signed_in"
            | "insufficient_evidence"
    )
}

fn read_marker(path: &Path, uid: &str) -> Result<AutomaticInsightAttempt, PennelError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_MARKER_BYTES
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_MARKER_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| unavailable())?;
    let attempt: AutomaticInsightAttempt =
        serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if attempt.schema_version != 1
        || attempt.meeting_uid != uid
        || !digest_valid(&attempt.transcript_digest)
        || path.file_name()
            != marker_path(
                path.parent().ok_or_else(unavailable)?,
                &attempt.transcript_digest,
            )?
            .file_name()
        || !matches!(attempt.language_policy.as_str(), "en" | "nl" | "mixed")
        || chrono::DateTime::parse_from_rfc3339(&attempt.admitted_at).is_err()
        || attempt
            .finished_at
            .as_ref()
            .is_some_and(|time| chrono::DateTime::parse_from_rfc3339(time).is_err())
        || attempt
            .error_code
            .as_deref()
            .is_some_and(|code| !error_code_valid(code))
        || attempt.items_found.is_some_and(|count| count > 512 * 32)
        || match attempt.outcome {
            None => {
                attempt.finished_at.is_some()
                    || attempt.error_code.is_some()
                    || attempt.items_found.is_some()
            }
            Some(AutomaticInsightOutcome::Completed) => {
                attempt.finished_at.is_none()
                    || attempt.items_found.is_none()
                    || attempt.error_code.is_some()
            }
            Some(_) => {
                attempt.finished_at.is_none()
                    || attempt.items_found.is_none()
                    || attempt.error_code.is_none()
            }
        }
    {
        return Err(unavailable());
    }
    Ok(attempt)
}

impl MeetingStore {
    pub(crate) fn has_automatic_insight_attempt(
        &self,
        id: &str,
        digest: &str,
    ) -> Result<bool, PennelError> {
        let directory = crate::meeting_summary::checked_directory(self, id)?;
        // Even a partial file or a symlink consumes this attempt. Never reinterpret corruption as permission to retry.
        match fs::symlink_metadata(marker_path(&directory, digest)?) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(unavailable()),
        }
    }

    pub(crate) fn admit_automatic_insight_attempt(
        &self,
        id: &str,
        source: &MeetingTranscriptSnapshot,
        language_policy: &str,
    ) -> Result<Option<AutomaticInsightAttempt>, PennelError> {
        let directory = crate::meeting_summary::checked_directory(self, id)?;
        let gate = replacement_gate(&directory);
        let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
        crate::meeting_insights::require_current_source(&directory, source)?;
        if self.uid_of_locked(id).as_deref() != Some(&source.meeting_uid) {
            return Err(PennelError::storage("source_changed"));
        }
        let attempt = AutomaticInsightAttempt {
            schema_version: 1,
            meeting_uid: source.meeting_uid.clone(),
            transcript_digest: source.transcript_digest.clone(),
            language_policy: language_policy.into(),
            admitted_at: chrono::Utc::now().to_rfc3339(),
            outcome: None,
            error_code: None,
            items_found: None,
            finished_at: None,
        };
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(marker_path(&directory, &source.transcript_digest)?) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
            result => result.map_err(|_| unavailable())?,
        };
        let bytes = serde_json::to_vec(&attempt).map_err(|_| unavailable())?;
        file.write_all(&bytes).map_err(|_| unavailable())?;
        file.sync_all().map_err(|_| unavailable())?;
        sync_directory(&directory)?;
        Ok(Some(attempt))
    }

    pub(crate) fn finish_automatic_insight_attempt(
        &self,
        id: &str,
        attempt: &AutomaticInsightAttempt,
    ) -> Result<(), PennelError> {
        let directory = crate::meeting_summary::checked_directory(self, id)?;
        let gate = replacement_gate(&directory);
        let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
        if self.uid_of_locked(id).as_deref() != Some(&attempt.meeting_uid) {
            return Err(unavailable());
        }
        let path = marker_path(&directory, &attempt.transcript_digest)?;
        let current = read_marker(&path, &attempt.meeting_uid)?;
        if current.admitted_at != attempt.admitted_at || current.outcome.is_some() {
            return Err(unavailable());
        }
        durable_write(
            &path,
            &serde_json::to_vec(attempt).map_err(|_| unavailable())?,
        )
    }

    pub(crate) fn automatic_insight_attempt(
        &self,
        id: &str,
        source: &MeetingTranscriptSnapshot,
    ) -> Result<Option<AutomaticInsightAttempt>, PennelError> {
        let directory = crate::meeting_summary::checked_directory(self, id)?;
        let current_path = marker_path(&directory, &source.transcript_digest)?;
        if self.has_automatic_insight_attempt(id, &source.transcript_digest)? {
            return read_marker(&current_path, &source.meeting_uid).map(Some);
        }
        let mut latest: Option<AutomaticInsightAttempt> = None;
        for entry in fs::read_dir(&directory).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            let name = entry.file_name();
            let Some(digest) = name
                .to_str()
                .and_then(|name| name.strip_prefix(PREFIX))
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if !digest_valid(digest) {
                continue;
            }
            // A damaged old attempt cannot hide a current review or disclose untrusted file content.
            if let Ok(attempt) = read_marker(&entry.path(), &source.meeting_uid)
                && latest
                    .as_ref()
                    .is_none_or(|old| old.admitted_at < attempt.admitted_at)
            {
                latest = Some(attempt);
            }
        }
        Ok(latest)
    }
}
