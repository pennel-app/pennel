use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::PennelEngine;
use crate::error::PennelError;
use crate::meeting_utterances::MeetingUtterance;
use crate::meetings::{Segment, SpeakerName, durable_write};
use crate::voices::MeetingVoiceprint;

const COMMITS: &str = "speaker-commits";
const RECEIPT: &str = "commit-receipt.json";

#[derive(Serialize, Deserialize)]
struct CommitReceipt {
    meeting_uid: String,
    backup: String,
    transcript_digest: String,
    names_digest: String,
    #[serde(default)]
    metadata_digest: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SpeakerCommit {
    version: u32,
    meeting_uid: String,
    old_hash: String,
    new_hash: String,
    old_prints: Vec<MeetingVoiceprint>,
    #[serde(default)]
    old_metadata_digest: Option<String>,
    #[serde(default)]
    new_metadata_digest: Option<String>,
    #[serde(default)]
    backup: Option<String>,
}

fn save(path: &Path, value: &SpeakerCommit) -> Result<(), PennelError> {
    durable_write(
        path,
        &serde_json::to_vec(value).map_err(PennelError::storage)?,
    )
}

impl PennelEngine {
    /// The caller holds the meeting-edit and sync guards. The existing transcript replacement
    /// journal restores text/names; this private journal restores the matching voice cache.
    #[allow(dead_code)] // Retained as the legacy text-only speaker commit wrapper.
    pub(crate) fn commit_speaker_result(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        prints: Vec<MeetingVoiceprint>,
        job_directory: Option<&Path>,
    ) -> Result<(), PennelError> {
        self.commit_speaker_result_with_utterances(id, segments, names, prints, job_directory, None)
    }

    pub(crate) fn commit_speaker_utterances(
        &self,
        id: &str,
        utterances: &[MeetingUtterance],
        names: &[SpeakerName],
        prints: Vec<MeetingVoiceprint>,
        job_directory: Option<&Path>,
    ) -> Result<(), PennelError> {
        let segments: Vec<_> = utterances.iter().map(|row| row.segment.clone()).collect();
        self.commit_speaker_result_with_utterances(
            id,
            &segments,
            names,
            prints,
            job_directory,
            Some(utterances),
        )
    }

    fn commit_speaker_result_with_utterances(
        &self,
        id: &str,
        segments: &[Segment],
        names: &[SpeakerName],
        prints: Vec<MeetingVoiceprint>,
        job_directory: Option<&Path>,
        utterances: Option<&[MeetingUtterance]>,
    ) -> Result<(), PennelError> {
        let store = self.meeting_store();
        let voices = self.voice_store();
        let uid = store.ensure_uid(id)?;
        let old_hash = store.transcript_fingerprint(id)?;
        let old_metadata_digest = store.read_transcript_snapshot(id)?.metadata_digest;
        let old_prints = voices.meeting_voiceprints(&uid, Some(&old_hash));
        let mut sorted = segments.to_vec();
        sorted.sort_by_key(|segment| segment.start_ms);
        let new_hash = crate::models::hex(&Sha256::digest(
            crate::meetings::render_transcript(&sorted).as_bytes(),
        ));
        let directory = self.data_dir().join(COMMITS);
        fs::create_dir_all(&directory).map_err(PennelError::storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(PennelError::storage)?;
        }
        let path = directory.join(format!("{}.json", uuid::Uuid::now_v7()));
        let mut pending = SpeakerCommit {
            version: 1,
            meeting_uid: uid.clone(),
            old_hash: old_hash.clone(),
            new_hash,
            old_prints: old_prints.clone(),
            old_metadata_digest,
            new_metadata_digest: None,
            backup: None,
        };
        save(&path, &pending)?;
        let tracks = store.transcript_tracks(id)?.unwrap_or_else(|| {
            [crate::Speaker::Me, crate::Speaker::Others]
                .into_iter()
                .filter(|speaker| segments.iter().any(|segment| segment.speaker == *speaker))
                .collect()
        });
        let replacement = match utterances {
            Some(rows) => store.replace_speaker_utterances(id, rows, names, &tracks),
            None => store.replace_speaker_labels(id, segments, names, &tracks),
        };
        let backup = match replacement {
            Ok(backup) => backup,
            Err(error) => {
                let _ = fs::remove_file(path);
                return Err(error);
            }
        };
        pending.backup = backup
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        pending.new_metadata_digest = store.read_transcript_snapshot(id)?.metadata_digest;
        if let Err(error) = save(&path, &pending) {
            store.restore_retranscription(id, &backup)?;
            let _ = fs::remove_file(path);
            return Err(error);
        }
        let applied = (|| {
            voices.set_meeting_voiceprints(
                &uid,
                prints,
                Some(&store.transcript_fingerprint(id)?),
            )?;
            if let Some(directory) = job_directory {
                let receipt = CommitReceipt {
                    meeting_uid: uid.clone(),
                    backup: pending.backup.clone().expect("recorded backup"),
                    transcript_digest: pending.new_hash.clone(),
                    names_digest: crate::speaker_jobs::speaker_names_digest(names)?,
                    metadata_digest: pending.new_metadata_digest.clone(),
                };
                durable_write(
                    &directory.join(RECEIPT),
                    &serde_json::to_vec(&receipt).map_err(PennelError::storage)?,
                )?;
            }
            store.finish_retranscription(id)
        })();
        if let Err(error) = applied {
            store.restore_retranscription(id, &backup)?;
            voices.set_meeting_voiceprints(&uid, old_prints, Some(&old_hash))?;
            fs::remove_file(path).map_err(PennelError::storage)?;
            return Err(error);
        }
        // Authoritative data and the matching cache are committed. A leftover private journal
        // can be cleaned at startup; reporting a failed recognition here would be misleading.
        let _ = fs::remove_file(path);
        Ok(())
    }

    pub(crate) fn speaker_commit_finished(
        &self,
        uid: &str,
        directory: &Path,
        transcript: &str,
        names: &str,
    ) -> bool {
        let Some(receipt) = fs::read(directory.join(RECEIPT))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CommitReceipt>(&bytes).ok())
        else {
            return false;
        };
        if receipt.meeting_uid != uid
            || receipt.transcript_digest != transcript
            || receipt.names_digest != names
            || uuid::Uuid::parse_str(&receipt.backup).is_err()
            || Path::new(&receipt.backup).components().count() != 1
        {
            return false;
        }
        let store = self.meeting_store();
        let Some(meeting) = store
            .list()
            .into_iter()
            .find(|meeting| store.uid_of(&meeting.id).as_deref() == Some(uid))
        else {
            return false;
        };
        !store
            .dir_of(&meeting.id)
            .join(".pennel-transcript-backups")
            .join(receipt.backup)
            .join("rolled-back")
            .exists()
            && store.transcript_fingerprint(&meeting.id).ok().as_deref() == Some(transcript)
            && crate::speaker_jobs::speaker_names_digest(&store.speaker_names(&meeting.id))
                .ok()
                .as_deref()
                == Some(names)
            && receipt.metadata_digest.as_ref().is_none_or(|expected| {
                store
                    .read_transcript_snapshot(&meeting.id)
                    .is_ok_and(|snapshot| {
                        snapshot.metadata_status
                            == crate::meeting_utterances::TranscriptMetadataStatus::Valid
                            && snapshot.metadata_digest.as_ref() == Some(expected)
                    })
            })
    }

    pub(crate) fn recover_speaker_commits(&self) -> Result<(), PennelError> {
        let directory = self.data_dir().join(COMMITS);
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(PennelError::storage(error)),
        };
        let store = self.meeting_store();
        for entry in entries {
            let entry = entry.map_err(PennelError::storage)?;
            if !entry.file_type().map_err(PennelError::storage)?.is_file()
                || entry.path().extension().is_none_or(|ext| ext != "json")
            {
                continue;
            }
            let pending: SpeakerCommit =
                serde_json::from_slice(&fs::read(entry.path()).map_err(PennelError::storage)?)
                    .map_err(PennelError::storage)?;
            if pending.version != 1 || !crate::meetings::valid_uid(&pending.meeting_uid) {
                return Err(PennelError::storage("The saved speaker job is invalid."));
            }
            let Some(meeting) = store.list().into_iter().find(|meeting| {
                store.uid_of(&meeting.id).as_deref() == Some(pending.meeting_uid.as_str())
            }) else {
                // iOS can move its sandbox before the host restores the current library path.
                // Keep the private rollback material until that library becomes available.
                continue;
            };
            // MeetingStore has already rolled back any unfinished authoritative replacement.
            // A finished replacement keeps its new cache; external edits invalidate both.
            let rolled_back = pending.backup.as_ref().is_some_and(|backup| {
                uuid::Uuid::parse_str(backup).is_ok()
                    && Path::new(backup).components().count() == 1
                    && store
                        .dir_of(&meeting.id)
                        .join(".pennel-transcript-backups")
                        .join(backup)
                        .join("rolled-back")
                        .is_file()
            });
            if rolled_back {
                self.voice_store().set_meeting_voiceprints(
                    &pending.meeting_uid,
                    pending.old_prints,
                    Some(&pending.old_hash),
                )?;
            }
            fs::remove_file(entry.path()).map_err(PennelError::storage)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PennelEngine, Speaker};

    #[test]
    fn each_commit_crash_boundary_keeps_matching_text_names_and_voiceprints() {
        for same_transcript in [false, true] {
            for boundary in 0..4 {
                let directory = tempfile::tempdir().unwrap();
                let engine =
                    PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
                let mut settings = engine.settings();
                settings.meetings_dir = directory
                    .path()
                    .join("meetings")
                    .to_string_lossy()
                    .into_owned();
                engine.update_settings(settings).unwrap();
                let store = engine.meeting_store();
                let id = store
                    .create("Journal fixture", chrono::Local::now())
                    .unwrap();
                let old = vec![Segment {
                    start_ms: 0,
                    end_ms: 0,
                    speaker: Speaker::Others,
                    text: "Original words.".into(),
                    failed: false,
                    voice: Some(1),
                }];
                let old_names = vec![SpeakerName {
                    voice: 1,
                    name: "Person A".into(),
                }];
                let old_prints = vec![MeetingVoiceprint {
                    voice: 1,
                    embedding: vec![1.0, 0.0],
                    known_id: None,
                }];
                store.write_transcript(&id, &old).unwrap();
                store.set_speaker_names(&id, &old_names).unwrap();
                store.write_notes(&id, "Personal notes survive.").unwrap();
                let uid = store.ensure_uid(&id).unwrap();
                let old_hash = store.transcript_fingerprint(&id).unwrap();
                engine
                    .voice_store()
                    .set_meeting_voiceprints(&uid, old_prints.clone(), Some(&old_hash))
                    .unwrap();
                let voice = if same_transcript { 1 } else { 2 };
                let mut new = old.clone();
                new[0].voice = Some(voice);
                let new_names = vec![SpeakerName {
                    voice,
                    name: "Person B".into(),
                }];
                let new_prints = vec![MeetingVoiceprint {
                    voice,
                    embedding: vec![0.0, 1.0],
                    known_id: None,
                }];
                let new_hash = crate::models::hex(&Sha256::digest(
                    crate::meetings::render_transcript(&new).as_bytes(),
                ));
                let commits = directory.path().join(COMMITS);
                fs::create_dir_all(&commits).unwrap();
                let path = commits.join(format!("{}.json", uuid::Uuid::now_v7()));
                let mut pending = SpeakerCommit {
                    version: 1,
                    meeting_uid: uid.clone(),
                    old_hash: old_hash.clone(),
                    new_hash: new_hash.clone(),
                    old_prints: old_prints.clone(),
                    old_metadata_digest: store
                        .read_transcript_snapshot(&id)
                        .unwrap()
                        .metadata_digest,
                    new_metadata_digest: None,
                    backup: None,
                };
                save(&path, &pending).unwrap();
                if boundary >= 1 {
                    let backup = store
                        .replace_speaker_labels(&id, &new, &new_names, &[Speaker::Others])
                        .unwrap();
                    pending.backup = backup
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    save(&path, &pending).unwrap();
                }
                if boundary >= 2 {
                    engine
                        .voice_store()
                        .set_meeting_voiceprints(&uid, new_prints.clone(), Some(&new_hash))
                        .unwrap();
                }
                if boundary >= 3 {
                    store.finish_retranscription(&id).unwrap();
                }
                drop(engine);
                let reopened =
                    PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
                let (segments, names, prints, hash) = if boundary == 3 {
                    (&new, &new_names, &new_prints, &new_hash)
                } else {
                    (&old, &old_names, &old_prints, &old_hash)
                };
                assert_eq!(
                    &reopened.meeting_transcript(id.clone()),
                    segments,
                    "boundary {boundary}, same transcript {same_transcript}"
                );
                assert_eq!(
                    &reopened.meeting_speakers(id.clone()),
                    names,
                    "boundary {boundary}, same transcript {same_transcript}"
                );
                assert_eq!(
                    &reopened.voice_store().meeting_voiceprints(&uid, Some(hash)),
                    prints,
                    "boundary {boundary}, same transcript {same_transcript}"
                );
                assert_eq!(reopened.meeting_notes(id), "Personal notes survive.");
                assert!(!path.exists());
            }
        }
    }
}
