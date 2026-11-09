//! Voiceprints: the user's own voice and the voices they chose to remember. They live in the data
//! folder, never in the (synced) meeting folder, and hold embeddings only, never audio.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::PennelError;
use crate::meeting_app_evidence::AppSpeakerEvidence;
use crate::meetings::durable_write;
use crate::models::EMBEDDING_ID;

const VERSION: u32 = 1;
const NAME_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct VoiceProfile {
    pub created_iso: String,
    pub speech_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct KnownVoice {
    pub id: String,
    pub name: String,
    /// Meetings this voice was confirmed in.
    pub meetings: u32,
    pub updated_iso: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Profile {
    embedding: Vec<f32>,
    speech_ms: u64,
    created: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Voice {
    id: String,
    name: String,
    embedding: Vec<f32>,
    meetings: u32,
    updated: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VoiceFile {
    version: u32,
    /// Voiceprints of different embedding models cannot be compared.
    model: String,
    me: Option<Profile>,
    known: Vec<Voice>,
}

impl Default for VoiceFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            model: EMBEDDING_ID.into(),
            me: None,
            known: Vec::new(),
        }
    }
}

/// The voiceprint of one voice in one meeting, kept so the user can still choose to remember it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingVoiceprint {
    pub voice: u32,
    pub embedding: Vec<f32>,
    pub known_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct MeetingFile {
    model: String,
    voices: Vec<MeetingVoiceprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transcript_hash: Option<String>,
}

pub struct VoiceStore {
    dir: PathBuf,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub(crate) fn clean_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !matches!(c, ';' | '=' | ':' | '\n' | '\r' | '[' | ']' | '*'))
        .collect();
    clean.trim().chars().take(NAME_MAX).collect()
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl VoiceStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("voices"),
        }
    }

    fn file_path(&self) -> PathBuf {
        self.dir.join("voiceprints.json")
    }

    fn meeting_path(&self, meeting: &str) -> PathBuf {
        let digest = crate::models::hex(&Sha256::digest(meeting.as_bytes()));
        self.dir.join("meetings").join(format!("{digest}.json"))
    }

    /// A missing file is empty; voiceprints of another model are ignored. An unreadable file is an
    /// error, so a write can never replace voices the user still has.
    fn load(&self) -> Result<VoiceFile, PennelError> {
        let json = match std::fs::read_to_string(self.file_path()) {
            Ok(json) => json,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(VoiceFile::default());
            }
            Err(error) => return Err(PennelError::storage(error)),
        };
        let file: VoiceFile = serde_json::from_str(&json).map_err(PennelError::storage)?;
        if file.model != EMBEDDING_ID {
            return Ok(VoiceFile::default());
        }
        Ok(file)
    }

    fn save(&self, file: &VoiceFile) -> Result<(), PennelError> {
        std::fs::create_dir_all(&self.dir).map_err(PennelError::storage)?;
        let json = serde_json::to_vec(file).map_err(PennelError::storage)?;
        durable_write(&self.file_path(), &json)
    }

    pub fn profile(&self) -> Option<VoiceProfile> {
        let me = self.load().ok()?.me?;
        Some(VoiceProfile {
            created_iso: me.created,
            speech_seconds: (me.speech_ms / 1000) as u32,
        })
    }

    pub fn my_voiceprint(&self) -> Option<Vec<f32>> {
        self.load()
            .ok()?
            .me
            .map(|me| me.embedding)
            .filter(|e| !e.is_empty())
    }

    pub fn set_my_voiceprint(
        &self,
        embedding: Vec<f32>,
        speech_ms: u64,
    ) -> Result<VoiceProfile, PennelError> {
        let mut file = self.load()?;
        let me = Profile {
            embedding,
            speech_ms,
            created: now(),
        };
        let profile = VoiceProfile {
            created_iso: me.created.clone(),
            speech_seconds: (speech_ms / 1000) as u32,
        };
        file.me = Some(me);
        self.save(&file)?;
        Ok(profile)
    }

    pub fn forget_my_voiceprint(&self) -> Result<(), PennelError> {
        let mut file = self.load()?;
        file.me = None;
        self.save(&file)
    }

    pub fn known(&self) -> Vec<KnownVoice> {
        let mut voices: Vec<KnownVoice> = self
            .load()
            .map(|file| file.known)
            .unwrap_or_default()
            .into_iter()
            .map(|v| KnownVoice {
                id: v.id,
                name: v.name,
                meetings: v.meetings,
                updated_iso: v.updated,
            })
            .collect();
        voices.sort_by_key(|v| v.name.to_lowercase());
        voices
    }

    /// `(id, name, voiceprint)` of every remembered voice.
    pub fn known_voiceprints(&self) -> Vec<(String, String, Vec<f32>)> {
        self.load()
            .map(|file| file.known)
            .unwrap_or_default()
            .into_iter()
            .filter(|v| !v.embedding.is_empty())
            .map(|v| (v.id, v.name, v.embedding))
            .collect()
    }

    /// Remembers a voice under `name`; a voice already known by that name learns from this one.
    pub fn remember(&self, name: &str, embedding: &[f32]) -> Result<String, PennelError> {
        let name = clean_name(name);
        if name.is_empty() || embedding.is_empty() {
            return Err(PennelError::InvalidResponse {
                message: "give the voice a name".into(),
            });
        }
        let mut file = self.load()?;
        let id = match file.known.iter_mut().find(|v| same_name(&v.name, &name)) {
            Some(voice) => {
                voice.embedding =
                    pennel_speakers::blend(&voice.embedding, voice.meetings, embedding);
                voice.meetings = voice.meetings.saturating_add(1);
                voice.updated = now();
                voice.id.clone()
            }
            None => {
                let id = uuid::Uuid::now_v7().to_string();
                file.known.push(Voice {
                    id: id.clone(),
                    name,
                    embedding: pennel_speakers::normalized(embedding),
                    meetings: 1,
                    updated: now(),
                });
                id
            }
        };
        self.save(&file)?;
        Ok(id)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), PennelError> {
        let name = clean_name(name);
        if name.is_empty() {
            return Err(PennelError::InvalidResponse {
                message: "give the voice a name".into(),
            });
        }
        let mut file = self.load()?;
        let voice = file
            .known
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or(PennelError::NotFound)?;
        voice.name = name;
        voice.updated = now();
        self.save(&file)
    }

    pub fn forget(&self, id: &str) -> Result<(), PennelError> {
        let mut file = self.load()?;
        let before = file.known.len();
        file.known.retain(|v| v.id != id);
        if file.known.len() == before {
            return Err(PennelError::NotFound);
        }
        self.save(&file)
    }

    pub fn meeting_voiceprints(
        &self,
        meeting: &str,
        transcript_hash: Option<&str>,
    ) -> Vec<MeetingVoiceprint> {
        std::fs::read_to_string(self.meeting_path(meeting))
            .ok()
            .and_then(|json| serde_json::from_str::<MeetingFile>(&json).ok())
            .filter(|file| file.model == EMBEDDING_ID)
            .filter(|file| {
                transcript_hash.is_none_or(|expected| {
                    file.transcript_hash
                        .as_deref()
                        .is_none_or(|actual| actual == expected)
                })
            })
            .map(|file| file.voices)
            .unwrap_or_default()
    }

    pub fn set_meeting_voiceprints(
        &self,
        meeting: &str,
        voices: Vec<MeetingVoiceprint>,
        transcript_hash: Option<&str>,
    ) -> Result<(), PennelError> {
        let path = self.meeting_path(meeting);
        if voices.is_empty() {
            return self.forget_meeting(meeting);
        }
        std::fs::create_dir_all(path.parent().expect("meetings folder"))
            .map_err(PennelError::storage)?;
        let file = MeetingFile {
            model: EMBEDDING_ID.into(),
            voices,
            transcript_hash: transcript_hash.map(str::to_owned),
        };
        durable_write(
            &path,
            &serde_json::to_vec(&file).map_err(PennelError::storage)?,
        )
    }

    pub fn forget_meeting(&self, meeting: &str) -> Result<(), PennelError> {
        remove_if_present(&self.app_evidence_path(meeting))?;
        remove_if_present(&self.meeting_path(meeting))
    }

    fn app_evidence_path(&self, meeting: &str) -> PathBuf {
        let digest = crate::models::hex(&Sha256::digest(meeting.as_bytes()));
        self.dir.join("app-evidence").join(format!("{digest}.json"))
    }

    pub fn app_speaker_evidence(&self, meeting: &str) -> Option<AppSpeakerEvidence> {
        let json = std::fs::read_to_string(self.app_evidence_path(meeting)).ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn set_app_speaker_evidence(
        &self,
        meeting: &str,
        evidence: &AppSpeakerEvidence,
    ) -> Result<(), PennelError> {
        let path = self.app_evidence_path(meeting);
        if evidence.samples.is_empty() {
            return remove_if_present(&path);
        }
        std::fs::create_dir_all(path.parent().expect("evidence folder"))
            .map_err(PennelError::storage)?;
        durable_write(
            &path,
            &serde_json::to_vec(evidence).map_err(PennelError::storage)?,
        )
    }
}

fn remove_if_present(path: &Path) -> Result<(), PennelError> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(PennelError::storage(error))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, VoiceStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = VoiceStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn my_voiceprint_round_trips_and_can_be_forgotten() {
        let (_dir, store) = store();
        assert!(store.profile().is_none());
        let profile = store.set_my_voiceprint(vec![0.6, 0.8], 24_500).unwrap();
        assert_eq!(profile.speech_seconds, 24);
        assert_eq!(store.profile(), Some(profile));
        assert_eq!(store.my_voiceprint(), Some(vec![0.6, 0.8]));
        store.forget_my_voiceprint().unwrap();
        assert!(store.my_voiceprint().is_none());
    }

    #[test]
    fn remembering_the_same_name_learns_instead_of_duplicating() {
        let (_dir, store) = store();
        let id = store.remember(" Jan ", &[1.0, 0.0]).unwrap();
        assert_eq!(store.remember("jan", &[0.0, 1.0]).unwrap(), id);
        let known = store.known();
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].name, "Jan");
        assert_eq!(known[0].meetings, 2);
        let (_, _, print) = &store.known_voiceprints()[0];
        assert!(
            (print[0] - print[1]).abs() < 1e-6,
            "two meetings weigh the same"
        );
    }

    #[test]
    fn known_voices_can_be_renamed_and_forgotten() {
        let (_dir, store) = store();
        let jan = store.remember("Jan", &[1.0, 0.0]).unwrap();
        store.remember("Anna", &[0.0, 1.0]).unwrap();
        store.rename(&jan, "Jan Peeters; x").unwrap();
        let names: Vec<String> = store.known().into_iter().map(|v| v.name).collect();
        assert_eq!(names, ["Anna", "Jan Peeters x"]);
        store.forget(&jan).unwrap();
        assert_eq!(store.known().len(), 1);
        assert_eq!(store.forget(&jan), Err(PennelError::NotFound));
        assert!(store.rename("missing", "X").is_err());
        assert!(store.remember("  ", &[1.0]).is_err());
    }

    #[test]
    fn voiceprints_of_another_model_are_ignored() {
        let (dir, store) = store();
        store.set_my_voiceprint(vec![1.0], 12_000).unwrap();
        let path = dir.path().join("voices/voiceprints.json");
        let json = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, json.replace(EMBEDDING_ID, "older-model")).unwrap();
        assert!(store.profile().is_none());
    }

    #[test]
    fn unreadable_file_is_never_overwritten() {
        let (dir, store) = store();
        std::fs::create_dir_all(dir.path().join("voices")).unwrap();
        std::fs::write(dir.path().join("voices/voiceprints.json"), b"{broken").unwrap();
        assert!(store.remember("Jan", &[1.0]).is_err());
        assert_eq!(
            std::fs::read(dir.path().join("voices/voiceprints.json")).unwrap(),
            b"{broken"
        );
    }

    #[test]
    fn meeting_voiceprints_are_stored_outside_the_meeting_by_hashed_id() {
        let (dir, store) = store();
        let voices = vec![MeetingVoiceprint {
            voice: 2,
            embedding: vec![0.0, 1.0],
            known_id: None,
        }];
        store
            .set_meeting_voiceprints("2026-10-26 Overleg", voices.clone(), None)
            .unwrap();
        assert_eq!(
            store.meeting_voiceprints("2026-10-26 Overleg", None),
            voices
        );
        let names: Vec<String> = std::fs::read_dir(dir.path().join("voices/meetings"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.contains("Overleg")));
        store.forget_meeting("2026-10-26 Overleg").unwrap();
        assert!(
            store
                .meeting_voiceprints("2026-10-26 Overleg", None)
                .is_empty()
        );
        store.forget_meeting("never").unwrap();
    }

    #[test]
    fn meeting_voiceprints_are_rejected_for_a_different_transcript() {
        let (_dir, store) = store();
        let voices = vec![MeetingVoiceprint {
            voice: 1,
            embedding: vec![1.0, 0.0],
            known_id: None,
        }];
        store
            .set_meeting_voiceprints("meeting", voices.clone(), Some("original"))
            .unwrap();
        assert_eq!(
            store.meeting_voiceprints("meeting", Some("original")),
            voices
        );
        assert!(
            store
                .meeting_voiceprints("meeting", Some("replacement"))
                .is_empty()
        );
    }
}
