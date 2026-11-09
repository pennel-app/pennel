//! Speaker recognition: who spoke in a meeting, including the user's own trained voice and the
//! voices they chose to remember. The exported surface is the same with and without the `speakers`
//! feature; without it, local recognition answers with an English diagnostic and Pennel Cloud still works.

use std::path::Path;

use pennel_speakers::{Diarization, Identity};
use sha2::{Digest, Sha256};

#[cfg(test)]
use crate::diarize::apply_utterances;
use crate::diarize::{
    DiarizedTrack, Track, apply_utterance_tracks_with_reserved, carry_utterance_names,
    from_sortformer, mostly_silent, preserve_manual_names,
};
use crate::engine::PennelEngine;
use crate::error::PennelError;
#[cfg(test)]
use crate::meetings::SpeakerName;
use crate::meetings::VOICES;
use crate::models::{DIARIZER_ID, EMBEDDING_ID, SEGMENTATION_ID};
use crate::settings::SpeakerRecognitionMode;
use crate::voices::{KnownVoice, MeetingVoiceprint, VoiceProfile, VoiceStore};

pub(crate) const UNAVAILABLE: &str = "Speaker recognition on this device is unavailable in this version of Pennel. Choose Pennel Cloud.";

/// About 25 seconds of calm reading; the interface translates this enrollment text.
pub const ENROLLMENT_TEXT: &str = "I am reading this text so Pennel can learn my voice. This helps Pennel recognize when I am speaking in each meeting, even when we are sharing one microphone. Today is a quiet day: the coffee is still warm, sunlight is coming through the window, and a notebook full of ideas is on my desk. Later we will review the schedule, divide the work, and agree on who will answer each question.";

const TOO_LITTLE_SPEECH: &str = "Pennel heard too little speech. Read the whole text at a steady pace, preferably in a quiet room.";
const NO_VOICEPRINT: &str =
    "Pennel has no voiceprint for this voice. Run speaker recognition for this meeting again.";

struct PreparedTrack {
    diarization: Diarization,
    track: Track,
    eligible_speakers: Option<std::collections::BTreeSet<u32>>,
}

impl PreparedTrack {
    fn new(mut diarization: Diarization, track: Track, identity_model: bool) -> Self {
        let eligible_speakers = identity_model.then(|| {
            diarization
                .speakers
                .iter()
                .filter_map(|speaker| {
                    if pennel_speakers::normalized(&speaker.embedding).is_empty() {
                        return None;
                    }
                    let spans = pennel_speakers::voiceprint_spans(
                        &diarization.turns,
                        speaker.speaker,
                        pennel_speakers::VOICEPRINT_BUDGET_MS,
                    );
                    let clean_ms: u64 = spans.iter().map(|(start, end)| end - start).sum();
                    ((spans.len() >= 2 && clean_ms >= 3_000)
                        || spans
                            .iter()
                            .any(|(start, end)| end - start >= pennel_speakers::MIN_ENROLLMENT_MS))
                    .then_some(speaker.speaker)
                })
                .collect::<std::collections::BTreeSet<_>>()
        });
        if let Some(eligible) = &eligible_speakers {
            // Low-evidence prints must not acquire Known/Me authority before admission runs.
            for speaker in &mut diarization.speakers {
                if !eligible.contains(&speaker.speaker) {
                    speaker.embedding.clear();
                }
            }
        }
        Self {
            diarization,
            track,
            eligible_speakers,
        }
    }
}

struct SpeakerApplication {
    recognition: SpeakerRecognition,
    limited_evidence: bool,
}

pub(crate) fn unavailable() -> PennelError {
    PennelError::InvalidResponse {
        message: UNAVAILABLE.into(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct SpeakerRecognition {
    /// Distinct other voices in the transcript after recognition.
    pub voices: u32,
    /// Voices named through remembered, manual or qualified meeting-app evidence.
    pub named: u32,
    /// Owner attribution from a matching self voice or explicit microphone-owner confirmation.
    pub me_found: bool,
    /// Every admitted acoustic voice received a portable meeting ID. This is not an accuracy score.
    pub complete: bool,
    /// Acoustic voices left without an ID because capacity or a prior mapping was ambiguous.
    pub unassigned_voices: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize, serde::Deserialize)]
pub struct SpeakerProcessingStatus {
    pub phase: String,
    pub reason: Option<String>,
    pub route: String,
    pub model_ids: Vec<String>,
    pub complete: bool,
    pub unassigned_voices: u32,
    pub source_revision: String,
    pub updated_at: String,
    pub meeting_uid: String,
}

const SPEAKER_PROCESSING_FILE: &str = ".pennel/speaker-processing.json";

fn processing_path(
    store: &crate::meetings::MeetingStore,
    id: &str,
) -> Result<std::path::PathBuf, PennelError> {
    let root = store.dir_of(id);
    for path in [root.join(".pennel"), root.join(SPEAKER_PROCESSING_FILE)] {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PennelError::storage(
                    "Speaker status storage is not a regular local path.",
                ));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(PennelError::storage(error));
            }
            _ => {}
        }
    }
    Ok(root.join(SPEAKER_PROCESSING_FILE))
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct MeetingVoice {
    pub voice: u32,
    pub name: Option<String>,
    /// Linked to a voice the user chose to remember.
    pub remembered: bool,
    /// This device holds the voiceprint, so "Remember this voice" can work.
    pub can_remember: bool,
}

/// One recognition or "Remember this voice" at a time: each reads, relabels and writes the transcript,
/// its names and the meeting's voiceprints, which must not interleave.
pub(crate) static RECOGNITION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn voice_profile_fingerprint(embedding: &[f32]) -> String {
    let mut hash = Sha256::new();
    for value in embedding {
        hash.update(value.to_bits().to_le_bytes());
    }
    crate::models::hex(&hash.finalize())
}

#[cfg(feature = "speakers")]
pub(crate) fn diarize_local(
    segmentation: &Path,
    embedding: &Path,
    samples: &[f32],
) -> Result<Diarization, PennelError> {
    let _physical = crate::live_speakers::offline_acoustic_permit();
    if !segmentation.is_file() || !embedding.is_file() {
        return Err(PennelError::ModelNotDownloaded);
    }
    let models = pennel_speakers::Models {
        segmentation,
        embedding,
    };
    pennel_speakers::diarize(&models, samples, &pennel_speakers::Options::default()).map_err(
        |error| PennelError::InvalidResponse {
            message: error.to_string(),
        },
    )
}

#[cfg(feature = "speakers")]
#[derive(Clone, PartialEq, Eq)]
struct SpeakerAssetStamp {
    length: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
}

#[cfg(feature = "speakers")]
fn speaker_asset_stamp(path: &Path) -> Result<SpeakerAssetStamp, PennelError> {
    let metadata = std::fs::metadata(path).map_err(|_| PennelError::ModelNotDownloaded)?;
    if !metadata.is_file() {
        return Err(PennelError::ModelNotDownloaded);
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    Ok(SpeakerAssetStamp {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        identity,
    })
}

#[cfg(feature = "speakers")]
fn speaker_asset_digest(path: &Path) -> Result<String, PennelError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| PennelError::ModelNotDownloaded)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| PennelError::ModelNotDownloaded)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(crate::models::hex(&digest.finalize()))
}

#[cfg(feature = "speakers")]
struct LocalLiveSpeakerInference {
    diarizer: pennel_speakers::Diarizer,
}
#[cfg(feature = "speakers")]
impl crate::live_speakers::LiveSpeakerInference for LocalLiveSpeakerInference {
    fn infer(&mut self, samples: &[f32]) -> Result<Diarization, PennelError> {
        self.diarizer
            .process(samples)
            .map_err(|_| PennelError::InvalidResponse {
                message: "Live speaker recognition failed.".into(),
            })
    }
}

#[cfg(not(feature = "speakers"))]
pub(crate) fn diarize_local(
    _segmentation: &Path,
    _embedding: &Path,
    _samples: &[f32],
) -> Result<Diarization, PennelError> {
    Err(unavailable())
}

/// The speaker that spoke most, if it spoke long enough for a reliable voiceprint.
fn enrollment_voiceprint(diarization: &Diarization) -> Result<(Vec<f32>, u64), PennelError> {
    let main = diarization
        .speakers
        .iter()
        .max_by_key(|s| s.speech_ms)
        .filter(|s| s.speech_ms >= pennel_speakers::MIN_ENROLLMENT_MS && !s.embedding.is_empty())
        .ok_or_else(|| PennelError::InvalidResponse {
            message: TOO_LITTLE_SPEECH.into(),
        })?;
    Ok((main.embedding.clone(), main.speech_ms))
}

fn establish_single_archive_source(
    store: &crate::meetings::MeetingStore,
    id: &str,
    rows: &mut [crate::meeting_utterances::MeetingUtterance],
) -> Result<(), PennelError> {
    let available: Vec<_> = store
        .audio_tracks(id)?
        .into_iter()
        .filter(|track| track.available)
        .collect();
    let [archive] = available.as_slice() else {
        return Ok(());
    };
    if store
        .transcript_tracks(id)?
        .is_some_and(|sources| sources.len() > 1)
    {
        return Ok(());
    }
    let declared = store.recording_metadata(id)?.is_some_and(|metadata| {
        metadata.tracks.len() == 1 && metadata.tracks[0].source == archive.source
    });
    let explicit_room = archive.source == crate::meeting_recording::RecordingSource::RoomMicrophone
        && store.dir_of(id).join("audio/room.m4a").is_file();
    if !declared && !explicit_room {
        return Ok(());
    }
    let source = match archive.source {
        crate::meeting_recording::RecordingSource::Microphone => "microphone",
        crate::meeting_recording::RecordingSource::SystemAudio => "system_audio",
        crate::meeting_recording::RecordingSource::RoomMicrophone => "room_microphone",
    };
    for row in rows
        .iter_mut()
        .filter(|row| row.metadata.source_id.is_none() && row.segment.speaker == archive.speaker)
    {
        let old = row.metadata.utterance_id.clone();
        row.metadata.source_id = Some(source.into());
        row.metadata.utterance_id = uuid::Uuid::now_v7().to_string();
        if !row.metadata.lineage.contains(&old) {
            row.metadata.lineage.push(old);
        }
        row.metadata.refresh_fingerprints(&row.segment);
    }
    Ok(())
}

impl PennelEngine {
    /// Readiness is metadata-only; saving a setup or loading this state never starts acoustic work.
    pub fn live_speaker_availability(&self) -> crate::live_speakers::LiveSpeakerStatus {
        use crate::live_speakers::LiveSpeakerStatus;
        if self.settings().speaker_recognition != SpeakerRecognitionMode::Local {
            return LiveSpeakerStatus::availability(false, Some("local_required"), None);
        }
        if !cfg!(feature = "speakers") {
            return LiveSpeakerStatus::availability(false, Some("unsupported"), None);
        }
        if self.models.path(SEGMENTATION_ID).is_none() || self.models.path(EMBEDDING_ID).is_none() {
            return LiveSpeakerStatus::availability(false, Some("models_missing"), None);
        }
        LiveSpeakerStatus::availability(
            true,
            None,
            Some(format!("{SEGMENTATION_ID}/{EMBEDDING_ID}")),
        )
    }

    /// Called only for explicit activation, outside capture/UI threads. No model is acquired here.
    pub fn start_live_speaker_worker(
        self: &std::sync::Arc<Self>,
        session_id: String,
        generation: u64,
        observer: std::sync::Arc<dyn crate::live_speakers::LiveSpeakerObserver>,
    ) -> Result<std::sync::Arc<crate::live_speakers::LiveSpeakerWorker>, PennelError> {
        self.start_live_speaker_worker_with_memory(
            session_id,
            generation,
            std::sync::Arc::new(crate::live_speakers::LiveSpeakerMemory::default()),
            observer,
        )
    }

    pub fn start_live_speaker_worker_with_memory(
        self: &std::sync::Arc<Self>,
        session_id: String,
        generation: u64,
        memory: std::sync::Arc<crate::live_speakers::LiveSpeakerMemory>,
        observer: std::sync::Arc<dyn crate::live_speakers::LiveSpeakerObserver>,
    ) -> Result<std::sync::Arc<crate::live_speakers::LiveSpeakerWorker>, PennelError> {
        if !self.live_speaker_availability().available {
            return Err(PennelError::InvalidResponse {message:"Live speaker attribution requires the selected local speaker route and installed compatible models.".into()});
        }
        #[cfg(feature = "speakers")]
        {
            let segmentation = self
                .models
                .path(SEGMENTATION_ID)
                .ok_or(PennelError::ModelNotDownloaded)?;
            let embedding = self
                .models
                .path(EMBEDDING_ID)
                .ok_or(PennelError::ModelNotDownloaded)?;
            let segmentation_stamp = speaker_asset_stamp(&segmentation)?;
            let embedding_stamp = speaker_asset_stamp(&embedding)?;
            let options = pennel_speakers::Options::default();
            let model = format!(
                "{SEGMENTATION_ID}@{}:{EMBEDDING_ID}@{};{}:{}:{}",
                speaker_asset_digest(&segmentation)?,
                speaker_asset_digest(&embedding)?,
                options.threads,
                options.cluster_threshold.to_bits(),
                options.window_shift.to_bits()
            );
            if !memory.bind(&model, &session_id) {
                return Err(PennelError::InvalidResponse {message:"The live speaker configuration changed. Existing session voices were preserved.".into()});
            }
            let owner = std::sync::Arc::downgrade(self);
            let (segmentation_check, embedding_check) = (segmentation.clone(), embedding.clone());
            let current = std::sync::Arc::new(move || {
                owner.upgrade().is_some_and(|engine| {
                    engine.settings().speaker_recognition == SpeakerRecognitionMode::Local
                        && engine.models.path(SEGMENTATION_ID).as_ref() == Some(&segmentation_check)
                        && engine.models.path(EMBEDDING_ID).as_ref() == Some(&embedding_check)
                        && speaker_asset_stamp(&segmentation_check).as_ref()
                            == Ok(&segmentation_stamp)
                        && speaker_asset_stamp(&embedding_check).as_ref() == Ok(&embedding_stamp)
                })
            });
            let factory = Box::new(move || {
                let models = pennel_speakers::Models {
                    segmentation: &segmentation,
                    embedding: &embedding,
                };
                let diarizer =
                    pennel_speakers::Diarizer::load(&models, &options).map_err(|_| {
                        PennelError::InvalidResponse {
                            message: "Live speaker models could not be loaded.".into(),
                        }
                    })?;
                Ok(Box::new(LocalLiveSpeakerInference { diarizer })
                    as Box<dyn crate::live_speakers::LiveSpeakerInference>)
            });
            Ok(crate::live_speakers::LiveSpeakerWorker::start_with_memory(
                session_id, generation, model, current, factory, observer, memory,
            ))
        }
        #[cfg(not(feature = "speakers"))]
        {
            let _ = (session_id, generation, memory, observer);
            Err(unavailable())
        }
    }

    /// Hybrid meetings analyze both physical sources. Enrollment only helps identify the user;
    /// it is never required to obtain neutral numbered speakers from a shared microphone.
    pub fn wants_microphone_track(&self, _system: Option<&[f32]>) -> bool {
        self.speaker_recognition_ready()
    }

    pub(crate) fn voice_store(&self) -> VoiceStore {
        VoiceStore::new(&self.data_dir())
    }

    /// Meeting voiceprints follow the meeting's sync identity, which survives a rename.
    pub(crate) fn meeting_key(&self, id: &str) -> String {
        self.meeting_store()
            .uid_of(id)
            .unwrap_or_else(|| id.to_owned())
    }

    /// Diarization by the configured route. `voiceprints` refuses the Sortformer fallback, which
    /// cannot tell whose voice it heard.
    fn speaker_diarization(
        &self,
        samples: &[f32],
        voiceprints: bool,
    ) -> Result<Diarization, PennelError> {
        self.speaker_diarization_selected(
            samples,
            voiceprints,
            self.settings().speaker_recognition,
            None,
        )
    }

    fn speaker_diarization_selected(
        &self,
        samples: &[f32],
        voiceprints: bool,
        route: SpeakerRecognitionMode,
        cloud: Option<&crate::cloud::account::Session>,
    ) -> Result<Diarization, PennelError> {
        self.speaker_diarization_prepared(samples, voiceprints, route, cloud)
            .map(|(diarization, _)| diarization)
    }

    fn speaker_diarization_prepared(
        &self,
        samples: &[f32],
        voiceprints: bool,
        route: SpeakerRecognitionMode,
        cloud: Option<&crate::cloud::account::Session>,
    ) -> Result<(Diarization, bool), PennelError> {
        if route == SpeakerRecognitionMode::Cloud {
            return match cloud {
                Some(session) => crate::cloud::diarize::diarize(session, samples),
                None => crate::cloud::diarize::diarize(&*self.cloud_session()?, samples),
            }
            .map(|diarization| (diarization, true));
        }
        let models = (
            self.models.path(SEGMENTATION_ID),
            self.models.path(EMBEDDING_ID),
        );
        if let (Some(segmentation), Some(embedding)) = models {
            return diarize_local(&segmentation, &embedding, samples)
                .map(|diarization| (diarization, true));
        }
        if !cfg!(feature = "speakers") {
            return Err(unavailable());
        }
        match self.models.path(DIARIZER_ID) {
            Some(sortformer) if !voiceprints => Ok((
                from_sortformer(&crate::diarize::diarize(&sortformer, samples)?),
                false,
            )),
            _ => Err(PennelError::ModelNotDownloaded),
        }
    }
}

#[uniffi::export]
impl PennelEngine {
    /// The two downloads of local speaker recognition: segmentation, then voiceprints.
    pub fn speaker_model_ids(&self) -> Vec<String> {
        vec![SEGMENTATION_ID.to_owned(), EMBEDDING_ID.to_owned()]
    }

    /// Whether this build can recognise speakers on the device.
    pub fn local_speaker_recognition_available(&self) -> bool {
        cfg!(feature = "speakers")
    }

    /// Whether recognition can run now with the chosen route.
    pub fn speaker_recognition_ready(&self) -> bool {
        match self.settings().speaker_recognition {
            SpeakerRecognitionMode::Cloud => self.cloud_signed_in(),
            SpeakerRecognitionMode::Local => {
                let pipeline = self.models.path(SEGMENTATION_ID).is_some()
                    && self.models.path(EMBEDDING_ID).is_some();
                cfg!(feature = "speakers") && (pipeline || self.models.path(DIARIZER_ID).is_some())
            }
        }
    }

    pub fn voice_enrollment_text(&self) -> String {
        ENROLLMENT_TEXT.to_owned()
    }

    /// Learns the user's voice from the enrollment text read aloud (16 kHz mono). Only the
    /// voiceprint is kept, never the audio.
    pub fn enroll_voice(&self, samples: Vec<f32>) -> Result<VoiceProfile, PennelError> {
        if (samples.len() as u64) * 1000 / u64::from(pennel_speakers::SAMPLE_RATE)
            < pennel_speakers::MIN_ENROLLMENT_MS
        {
            return Err(PennelError::InvalidResponse {
                message: TOO_LITTLE_SPEECH.into(),
            });
        }
        let diarization = self.speaker_diarization(&samples, true)?;
        let (embedding, speech_ms) = enrollment_voiceprint(&diarization)?;
        self.voice_store().set_my_voiceprint(embedding, speech_ms)
    }

    pub fn voice_profile(&self) -> Option<VoiceProfile> {
        self.voice_store().profile()
    }

    pub fn forget_voice_profile(&self) -> Result<(), PennelError> {
        let _edit = self.edit_voice_settings()?;
        self.voice_store().forget_my_voiceprint()
    }

    pub fn known_voices(&self) -> Vec<KnownVoice> {
        self.voice_store().known()
    }

    pub fn rename_known_voice(&self, id: String, name: String) -> Result<(), PennelError> {
        self.voice_store().rename(&id, &name)
    }

    pub fn forget_known_voice(&self, id: String) -> Result<(), PennelError> {
        let _edit = self.edit_voice_settings()?;
        self.voice_store().forget(&id)
    }

    /// Removes the user's voice, every remembered voice and every meeting voiceprint.
    pub fn forget_all_voices(&self) -> Result<(), PennelError> {
        let _edit = self.edit_voice_settings()?;
        self.discard_speaker_recognition_data()?;
        match std::fs::remove_dir_all(self.data_dir().join("voices")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(PennelError::storage(error))
            }
            _ => Ok(()),
        }
    }

    /// Recognises both physical sources of a finished meeting (16 kHz mono). Microphone voices
    /// remain neutral unless an enrolled voice actually matches; room voices never inherit "Me".
    pub fn recognize_speakers(
        &self,
        id: String,
        microphone: Option<Vec<f32>>,
        system: Option<Vec<f32>>,
    ) -> Result<SpeakerRecognition, PennelError> {
        let mut microphone = microphone;
        let mut system = system;
        self.recognize_speakers_lazy(id, |source| {
            Ok(match source {
                crate::Speaker::Me => microphone.take(),
                crate::Speaker::Others => system.take(),
            })
        })
    }

    /// The other voices of a meeting, with their names and whether they can be remembered.
    pub fn meeting_voices(&self, id: String) -> Vec<MeetingVoice> {
        let store = self.meeting_store();
        let names = store.speaker_names(&id);
        let prints = self.voice_store().meeting_voiceprints(
            &self.meeting_key(&id),
            Some(&store.transcript_fingerprint(&id).unwrap_or_default()),
        );
        let known: Vec<String> = self.known_voices().into_iter().map(|v| v.id).collect();
        let mut voices: Vec<u32> = store
            .read_transcript_snapshot(&id)
            .map(|snapshot| {
                snapshot
                    .utterances
                    .into_iter()
                    .flat_map(|row| {
                        row.metadata
                            .attribution
                            .voices
                            .into_iter()
                            .chain(row.segment.voice)
                    })
                    .collect()
            })
            .unwrap_or_default();
        voices.sort_unstable();
        voices.dedup();
        voices
            .into_iter()
            .map(|voice| {
                let print = prints.iter().find(|p| p.voice == voice);
                MeetingVoice {
                    voice,
                    name: names
                        .iter()
                        .find(|n| n.voice == voice)
                        .map(|n| n.name.clone()),
                    remembered: print
                        .and_then(|p| p.known_id.as_ref())
                        .is_some_and(|id| known.contains(id)),
                    can_remember: print.is_some(),
                }
            })
            .collect()
    }

    /// Names a voice in this meeting and remembers its voiceprint under that name, so later
    /// meetings name it automatically.
    pub fn remember_speaker(
        &self,
        id: String,
        voice: u32,
        name: String,
    ) -> Result<(), PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        if !VOICES.contains(&voice) {
            return Err(PennelError::NotFound);
        }
        let _gate = RECOGNITION.lock().unwrap_or_else(|e| e.into_inner());
        let voices = self.voice_store();
        let key = self.meeting_key(&id);
        let transcript_hash = self.meeting_store().transcript_fingerprint(&id)?;
        let mut prints = voices.meeting_voiceprints(&key, Some(&transcript_hash));
        let print = prints
            .iter_mut()
            .find(|p| p.voice == voice)
            .ok_or_else(|| PennelError::InvalidResponse {
                message: NO_VOICEPRINT.into(),
            })?;
        let clean = crate::voices::clean_name(&name).to_lowercase();
        let already = print.known_id.as_ref().is_some_and(|known| {
            voices
                .known()
                .iter()
                .any(|v| v.id == *known && v.name.to_lowercase() == clean)
        });
        // The same meeting voice learned twice would weigh one meeting double.
        if !already {
            print.known_id = Some(voices.remember(&name, &print.embedding)?);
        }
        self.meeting_store().rename_speaker(&id, voice, &name)?;
        voices.set_meeting_voiceprints(&key, prints, Some(&transcript_hash))
    }
}

impl PennelEngine {
    pub fn meeting_speaker_processing_status(&self, id: String) -> Option<SpeakerProcessingStatus> {
        let store = self.meeting_store();
        store.info(&id)?;
        let path = processing_path(&store, &id).ok()?;
        if std::fs::metadata(&path).ok()?.len() > 65_536 {
            return None;
        }
        let mut status: SpeakerProcessingStatus =
            serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        if store.uid_of(&id).as_deref() != Some(status.meeting_uid.as_str()) {
            return None;
        }
        if matches!(status.phase.as_str(), "complete" | "incomplete")
            && store.transcript_fingerprint(&id).ok().as_deref()
                != Some(status.source_revision.as_str())
        {
            status.phase = "outdated".into();
            status.complete = false;
            status.reason = Some("source_changed".into());
        }
        Some(status)
    }

    /// Content-free state for manual and automatic speaker runs. Callers pass the route captured
    /// when the run began, rather than whichever setting happens to be selected at completion.
    pub fn set_meeting_speaker_processing_status(
        &self,
        id: String,
        phase: String,
        reason: Option<String>,
        route: SpeakerRecognitionMode,
        unassigned_voices: u32,
    ) -> Result<SpeakerProcessingStatus, PennelError> {
        if !matches!(
            phase.as_str(),
            "running" | "complete" | "incomplete" | "failed" | "unavailable" | "skipped"
        ) || reason.as_deref().is_some_and(|reason| {
            !matches!(
                reason,
                "disabled"
                    | "models_missing"
                    | "unsupported"
                    | "not_ready"
                    | "failed"
                    | "cancelled"
                    | "no_speech"
                    | "unassigned_voices"
                    | "source_changed"
                    | "permission_denied"
                    | "limited_evidence"
            )
        }) {
            return Err(PennelError::storage(
                "Unsupported speaker processing status.",
            ));
        }
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        let meeting_uid = store.ensure_uid(&id)?;
        let path = processing_path(&store, &id)?;
        std::fs::create_dir_all(path.parent().ok_or(PennelError::NotFound)?)
            .map_err(PennelError::storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                path.parent().ok_or(PennelError::NotFound)?,
                std::fs::Permissions::from_mode(0o700),
            )
            .map_err(PennelError::storage)?;
        }
        let mut model_ids = if route == SpeakerRecognitionMode::Cloud {
            vec![EMBEDDING_ID.into()]
        } else if self.models.path(SEGMENTATION_ID).is_some()
            && self.models.path(EMBEDDING_ID).is_some()
        {
            vec![SEGMENTATION_ID.into(), EMBEDDING_ID.into()]
        } else if self.models.path(DIARIZER_ID).is_some() {
            vec![DIARIZER_ID.into()]
        } else {
            Vec::new()
        };
        let route_name = if route == SpeakerRecognitionMode::Cloud {
            "cloud"
        } else {
            "local"
        };
        if phase != "running"
            && let Some(running) = self
                .meeting_speaker_processing_status(id.clone())
                .filter(|status| status.phase == "running" && status.route == route_name)
        {
            model_ids = running.model_ids;
        }
        let status = SpeakerProcessingStatus {
            complete: phase == "complete" && unassigned_voices == 0,
            phase,
            reason,
            route: route_name.into(),
            model_ids,
            unassigned_voices,
            source_revision: store.transcript_fingerprint(&id)?,
            updated_at: chrono::Utc::now().to_rfc3339(),
            meeting_uid,
        };
        crate::meetings::durable_write(
            &path,
            &serde_json::to_vec(&status).map_err(PennelError::storage)?,
        )?;
        Ok(status)
    }

    /// Confirms that this meeting's personal microphone contains the owner's speech. Room and
    /// system sources are excluded, and prior deliberate assignments must be resolved first.
    pub fn confirm_meeting_microphone_owner(&self, id: String) -> Result<u32, PennelError> {
        use crate::meeting_utterances::{
            AcousticTurn, AttributionKind, AutomaticAttribution, ManualCorrection,
        };
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let _sync = self.sync_gate.lock().unwrap();
        let _gate = RECOGNITION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let store = self.meeting_store();
        let snapshot = store.read_transcript_snapshot(&id)?;
        crate::meeting_utterances::assert_writable(&snapshot)?;
        let mut rows = snapshot.utterances;
        let affected = |row: &crate::meeting_utterances::MeetingUtterance| {
            row.metadata.source_id.as_deref() == Some("microphone")
                && !row.segment.failed
                && !row.segment.text.trim().is_empty()
        };
        if !rows.iter().any(affected) {
            return Err(PennelError::storage(
                "This meeting has no attributable personal microphone speech.",
            ));
        }
        if rows.iter().filter(|row| affected(row)).any(|row| {
            !row.metadata.unresolved_manual.is_empty()
                || row.metadata.manual.as_ref().is_some_and(|manual| {
                    !manual.self_voice
                        || manual.review_required
                        || manual.source_fingerprint != row.metadata.source_fingerprint
                })
        }) {
            return Err(PennelError::storage(
                "Review the microphone's existing manual speaker assignments before confirming its owner.",
            ));
        }
        if rows.iter().filter(|row| affected(row)).all(|row| {
            row.segment.speaker == crate::Speaker::Me
                && row.segment.voice.is_none()
                && row.metadata.attribution.self_voice
                && row
                    .metadata
                    .manual
                    .as_ref()
                    .is_some_and(|manual| manual.self_voice)
        }) {
            return Ok(0);
        }
        let mut count = 0;
        for row in rows.iter_mut().filter(|row| affected(row)) {
            count += 1;
            row.segment.speaker = crate::Speaker::Me;
            row.segment.voice = None;
            row.metadata.attribution = AutomaticAttribution {
                kind: AttributionKind::Single,
                voices: Vec::new(),
                self_voice: true,
                provisional: false,
                turns: row
                    .metadata
                    .attribution
                    .turns
                    .iter()
                    .map(|turn| AcousticTurn {
                        start_ms: turn.start_ms,
                        end_ms: turn.end_ms,
                        voice: None,
                    })
                    .collect(),
            };
            row.metadata.attribution_revision = row.metadata.attribution_revision.saturating_add(1);
            row.metadata.self_voice_profile_fingerprint = None;
            row.metadata.automatic_name = None;
            row.metadata.refresh_fingerprints(&row.segment);
            row.metadata.manual = Some(ManualCorrection {
                self_voice: true,
                name: None,
                voice: None,
                review_required: false,
                source_fingerprint: row.metadata.source_fingerprint.clone(),
            });
        }
        let present: std::collections::BTreeSet<_> = rows
            .iter()
            .flat_map(|row| {
                row.segment
                    .voice
                    .into_iter()
                    .chain(row.metadata.attribution.voices.iter().copied())
            })
            .collect();
        let names: Vec<_> = store
            .speaker_names(&id)
            .into_iter()
            .filter(|name| present.contains(&name.voice))
            .collect();
        let prints = self
            .voice_store()
            .meeting_voiceprints(&self.meeting_key(&id), Some(&snapshot.transcript_digest))
            .into_iter()
            .filter(|print| present.contains(&print.voice))
            .collect();
        self.commit_speaker_utterances(&id, &rows, &names, prints, None)?;
        Ok(count)
    }

    /// Decode and infer one physical track at a time, then apply both in one recoverable commit.
    pub fn recognize_speakers_lazy(
        &self,
        id: String,
        decode: impl FnMut(crate::Speaker) -> Result<Option<Vec<f32>>, PennelError>,
    ) -> Result<SpeakerRecognition, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let route = self.settings().speaker_recognition;
        self.set_meeting_speaker_processing_status(id.clone(), "running".into(), None, route, 0)?;
        let result = self.recognize_speakers_lazy_inner(id.clone(), route, decode);
        if let Err(error) = &result {
            let reason = if *error == PennelError::Cancelled {
                "cancelled"
            } else {
                "failed"
            };
            let _ = self.set_meeting_speaker_processing_status(
                id,
                "failed".into(),
                Some(reason.into()),
                route,
                0,
            );
        }
        result
    }

    fn recognize_speakers_lazy_inner(
        &self,
        id: String,
        route: SpeakerRecognitionMode,
        mut decode: impl FnMut(crate::Speaker) -> Result<Option<Vec<f32>>, PennelError>,
    ) -> Result<SpeakerRecognition, PennelError> {
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        if store.unparsed_transcript_lines(&id) > 0 {
            return Err(PennelError::InvalidResponse {
                message: "the transcript contains lines that Pennel does not recognize".into(),
            });
        }
        let cloud = (route == SpeakerRecognitionMode::Cloud)
            .then(|| self.cloud_session())
            .transpose()?;
        let account_epoch = cloud.as_ref().map(|session| session.generation());
        let ensure_current = || {
            if self.settings().speaker_recognition != route
                || account_epoch.is_some_and(|epoch| self.cloud_account_epoch() != epoch)
            {
                Err(PennelError::Cancelled)
            } else {
                Ok(())
            }
        };
        let my_voice = self.voice_store().my_voiceprint();
        let snapshot = store.read_transcript_snapshot(&id)?;
        crate::meeting_utterances::assert_writable(&snapshot)?;
        let revision = (
            store.transcript_fingerprint(&id)?,
            crate::speaker_jobs::speaker_names_digest(&store.speaker_names(&id))?,
            snapshot.metadata_digest,
        );
        let mut results = Vec::new();
        for source in [crate::Speaker::Others, crate::Speaker::Me] {
            ensure_current()?;
            if let Some(samples) = decode(source)?.filter(|samples| !mostly_silent(samples)) {
                ensure_current()?;
                let track = if source == crate::Speaker::Others
                    && store.audio_tracks(&id)?.iter().any(|track| {
                        track.source == crate::meeting_recording::RecordingSource::RoomMicrophone
                    }) {
                    Track::Microphone
                } else {
                    Track::from(source)
                };
                let (diarization, identity_model) =
                    self.speaker_diarization_prepared(&samples, false, route, cloud.as_deref())?;
                results.push(PreparedTrack::new(diarization, track, identity_model));
            }
        }
        if results.is_empty() {
            self.set_meeting_speaker_processing_status(
                id,
                "unavailable".into(),
                Some("no_speech".into()),
                route,
                0,
            )?;
            return Ok(SpeakerRecognition::default());
        }
        let _sync = self.sync_gate.lock().unwrap();
        ensure_current()?;
        if store.transcript_fingerprint(&id)? != revision.0
            || crate::speaker_jobs::speaker_names_digest(&store.speaker_names(&id))? != revision.1
            || store.read_transcript_snapshot(&id)?.metadata_digest != revision.2
        {
            return Err(PennelError::storage(
                "The meeting has changed. The speaker names were not replaced.",
            ));
        }
        let result = self.apply_prepared_tracks_observed(
            &id,
            results,
            my_voice.as_deref(),
            None,
            |_, _| ensure_current(),
        )?;
        self.record_recognition_result(&id, route, &result)?;
        Ok(result.recognition)
    }

    fn record_recognition_result(
        &self,
        id: &str,
        route: SpeakerRecognitionMode,
        application: &SpeakerApplication,
    ) -> Result<(), PennelError> {
        let result = &application.recognition;
        self.set_meeting_speaker_processing_status(
            id.to_owned(),
            if result.complete {
                "complete"
            } else {
                "incomplete"
            }
            .into(),
            if application.limited_evidence {
                Some("limited_evidence".into())
            } else {
                (!result.complete).then(|| "unassigned_voices".into())
            },
            route,
            result.unassigned_voices,
        )?;
        Ok(())
    }

    pub(crate) fn apply_recognized_speakers_observed(
        &self,
        id: &str,
        diarization: Diarization,
        track: Track,
        my_voice: Option<&[f32]>,
        job_directory: Option<&Path>,
        before_commit: impl FnOnce(&[crate::Segment], &[crate::SpeakerName]) -> Result<(), PennelError>,
    ) -> Result<SpeakerRecognition, PennelError> {
        let application = self.apply_prepared_tracks_observed(
            id,
            vec![PreparedTrack::new(diarization, track, true)],
            my_voice,
            job_directory,
            before_commit,
        )?;
        self.record_recognition_result(id, SpeakerRecognitionMode::Cloud, &application)?;
        Ok(application.recognition)
    }

    #[cfg(test)]
    fn apply_recognized_tracks_observed(
        &self,
        id: &str,
        diarizations: Vec<(Diarization, Track)>,
        my_voice: Option<&[f32]>,
        job_directory: Option<&Path>,
        before_commit: impl FnOnce(&[crate::Segment], &[crate::SpeakerName]) -> Result<(), PennelError>,
    ) -> Result<SpeakerRecognition, PennelError> {
        self.apply_prepared_tracks_observed(
            id,
            diarizations
                .into_iter()
                .map(|(diarization, track)| PreparedTrack::new(diarization, track, false))
                .collect(),
            my_voice,
            job_directory,
            before_commit,
        )
        .map(|application| application.recognition)
    }

    fn apply_prepared_tracks_observed(
        &self,
        id: &str,
        diarizations: Vec<PreparedTrack>,
        my_voice: Option<&[f32]>,
        job_directory: Option<&Path>,
        before_commit: impl FnOnce(&[crate::Segment], &[crate::SpeakerName]) -> Result<(), PennelError>,
    ) -> Result<SpeakerApplication, PennelError> {
        let store = self.meeting_store();
        let voices = self.voice_store();
        let original_route = self.settings().speaker_recognition;
        let known = voices.known_voiceprints();
        let known_prints: Vec<&[f32]> =
            known.iter().map(|(_, _, print)| print.as_slice()).collect();
        let _gate = RECOGNITION.lock().unwrap_or_else(|e| e.into_inner());
        if store.unparsed_transcript_lines(id) > 0 {
            return Err(PennelError::InvalidResponse {
                message: "the transcript contains lines that Pennel does not recognize".into(),
            });
        }
        store.ensure_uid(id)?;
        let snapshot = store.read_transcript_snapshot(id)?;
        let original_metadata_digest = snapshot.metadata_digest.clone();
        let old_names = store.speaker_names(id);
        let old_hash = store.transcript_fingerprint(id)?;
        let legacy_names =
            snapshot.metadata_status == crate::meeting_utterances::TranscriptMetadataStatus::Absent;
        let before = snapshot.utterances;
        let mut after = before.clone();
        if legacy_names {
            preserve_manual_names(&mut after, &old_names);
        }
        establish_single_archive_source(&store, id, &mut after)?;
        store.preserve_transcript_tracks(id)?;
        let mut recognised = Vec::new();
        let mut prints = voices.meeting_voiceprints(&self.meeting_key(id), Some(&old_hash));
        let identities: Vec<_> = diarizations
            .iter()
            .map(|prepared| {
                pennel_speakers::identify(
                    &prepared.diarization.speakers,
                    my_voice.filter(|_| prepared.track == Track::Microphone),
                    &known_prints,
                )
            })
            .collect();
        let inputs: Vec<_> = diarizations
            .iter()
            .zip(&identities)
            .map(|(prepared, identities)| DiarizedTrack {
                turns: &prepared.diarization.turns,
                identities,
                track: prepared.track,
                eligible_speakers: prepared.eligible_speakers.as_ref(),
            })
            .collect();
        // Public transcript snapshots hide successful-empty completion markers. Their retained
        // IDs still belong to those private rows restored by the replacement transaction.
        let retained_ids: Vec<_> = store
            .read_committed_transcript_snapshot(id)?
            .utterances
            .iter()
            .filter(|row| row.segment.failed || row.segment.text.trim().is_empty())
            .flat_map(|row| {
                row.segment
                    .voice
                    .into_iter()
                    .chain(row.metadata.attribution.voices.iter().copied())
                    .chain(
                        row.metadata
                            .attribution
                            .turns
                            .iter()
                            .filter_map(|turn| turn.voice),
                    )
                    .chain(row.metadata.manual.iter().filter_map(|manual| manual.voice))
                    .chain(
                        row.metadata
                            .unresolved_manual
                            .iter()
                            .filter_map(|manual| manual.voice),
                    )
            })
            .collect();
        let (mut after, applications, unassigned_voices, limited_evidence) =
            apply_utterance_tracks_with_reserved(&after, &inputs, &retained_ids);
        let me_found = applications.iter().any(|applied| applied.me_found);
        for ((prepared, identities), applied) in
            diarizations.iter().zip(&identities).zip(applications)
        {
            let diarization = &prepared.diarization;
            let track = prepared.track;
            let owner_fingerprint = my_voice.map(voice_profile_fingerprint);
            for row in &mut after {
                let analyzed_source = matches!(
                    (track, row.metadata.source_id.as_deref()),
                    (Track::Microphone, Some("microphone" | "room_microphone"))
                        | (Track::System, Some("system" | "system_audio"))
                );
                if analyzed_source && !row.segment.failed {
                    row.metadata.self_voice_profile_fingerprint =
                        (row.metadata.attribution.self_voice
                            && track == Track::Microphone
                            && !row
                                .metadata
                                .manual
                                .as_ref()
                                .is_some_and(|manual| manual.self_voice))
                        .then(|| owner_fingerprint.clone())
                        .flatten();
                }
            }
            for (voice, speaker) in applied.voices {
                let known_voice = match identities.get(speaker) {
                    Some(Identity::Known(index, _)) => known.get(*index),
                    _ => None,
                };
                if let Some((_, name, _)) = known_voice {
                    recognised.push((voice, name.clone()));
                }
                prints.retain(|print| print.voice != voice);
                if let Some(speaker) = diarization
                    .speakers
                    .get(speaker)
                    .filter(|speaker| !speaker.embedding.is_empty())
                {
                    prints.push(MeetingVoiceprint {
                        voice,
                        embedding: speaker.embedding.clone(),
                        known_id: known_voice.map(|(id, _, _)| id.clone()),
                    });
                }
            }
        }
        let settings = self.settings();
        let original_naming_policy = (
            settings.meeting_app_speakers,
            settings.meeting_app_name_policy,
        );
        let original_evidence = voices.app_speaker_evidence(&self.meeting_key(id));
        let mut names = carry_utterance_names(&before, &after, &old_names, &recognised);
        self.prepare_meeting_automatic_names(id, &mut after, &mut names, &prints)?;
        let segments: Vec<_> = after.iter().map(|row| row.segment.clone()).collect();
        before_commit(&segments, &names)?;
        let _policy = self.speaker_commit_policy_guard(original_naming_policy)?;
        let current_settings = self.settings();
        if original_naming_policy
            != (
                current_settings.meeting_app_speakers,
                current_settings.meeting_app_name_policy,
            )
            || original_evidence != voices.app_speaker_evidence(&self.meeting_key(id))
            || current_settings.speaker_recognition != original_route
            || my_voice.is_some_and(|expected| voices.my_voiceprint().as_deref() != Some(expected))
        {
            return Err(PennelError::Cancelled);
        }
        if store.transcript_fingerprint(id)? != old_hash
            || store.speaker_names(id) != old_names
            || store.read_transcript_snapshot(id)?.metadata_digest != original_metadata_digest
        {
            return Err(PennelError::storage(
                "The meeting changed before speaker labels could be saved.",
            ));
        }
        self.commit_speaker_utterances(id, &after, &names, prints, job_directory)?;
        let mut present: Vec<_> = after
            .iter()
            .flat_map(|row| row.metadata.attribution.voices.iter().copied())
            .chain(after.iter().filter_map(|row| row.segment.voice))
            .collect();
        present.sort_unstable();
        present.dedup();
        Ok(SpeakerApplication {
            recognition: SpeakerRecognition {
                voices: present.len() as u32,
                named: names.len() as u32,
                me_found,
                complete: unassigned_voices == 0,
                unassigned_voices,
            },
            limited_evidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::{Segment, Speaker};
    use pennel_speakers::{Speaker as Voice, Turn};

    impl crate::meetings::MeetingStore {
        fn write_owned_transcript(
            &self,
            id: &str,
            segments: &[Segment],
        ) -> Result<(), PennelError> {
            let rows: Vec<_> = segments
                .iter()
                .cloned()
                .map(|segment| {
                    let source = if segment.speaker == Speaker::Me {
                        "microphone"
                    } else {
                        "system_audio"
                    };
                    let mut row = crate::meeting_utterances::MeetingUtterance::new(
                        segment,
                        Some(source.into()),
                    );
                    if let Some(voice) = row.segment.voice {
                        row.metadata.attribution.kind =
                            crate::meeting_utterances::AttributionKind::Single;
                        row.metadata.attribution.voices = vec![voice];
                        row.metadata.attribution.turns =
                            vec![crate::meeting_utterances::AcousticTurn {
                                start_ms: row.segment.start_ms,
                                end_ms: row.segment.end_ms,
                                voice: Some(voice),
                            }];
                    }
                    row
                })
                .collect();
            self.write_transcript_snapshot(id, None, &rows).map(|_| ())
        }
    }

    fn name(voice: u32, name: &str) -> SpeakerName {
        SpeakerName {
            voice,
            name: name.into(),
        }
    }

    #[test]
    fn enrollment_needs_enough_speech_and_takes_the_main_voice() {
        let speaker = |speaker, speech_ms, embedding: Vec<f32>| Voice {
            speaker,
            speech_ms,
            embedding,
        };
        let short = Diarization {
            turns: vec![],
            speakers: vec![speaker(0, 8_000, vec![1.0])],
        };
        assert!(enrollment_voiceprint(&short).is_err());
        let fine = Diarization {
            turns: vec![],
            speakers: vec![
                speaker(0, 3_000, vec![0.0, 1.0]),
                speaker(1, 21_000, vec![1.0, 0.0]),
            ],
        };
        assert_eq!(
            enrollment_voiceprint(&fine).unwrap(),
            (vec![1.0, 0.0], 21_000)
        );
        let no_print = Diarization {
            turns: vec![],
            speakers: vec![speaker(0, 21_000, vec![])],
        };
        assert!(enrollment_voiceprint(&no_print).is_err());
    }

    #[cfg(feature = "speakers")]
    #[test]
    fn local_diarization_without_models_needs_a_download() {
        let missing = Path::new("/nonexistent/pennel/model.onnx");
        assert_eq!(
            diarize_local(missing, missing, &[0.0; 32_000]),
            Err(PennelError::ModelNotDownloaded)
        );
    }

    #[cfg(not(feature = "speakers"))]
    #[test]
    fn local_diarization_without_feature_returns_an_english_error() {
        let missing = Path::new("/nonexistent/pennel/model.onnx");
        assert_eq!(
            diarize_local(missing, missing, &[0.0; 32_000]),
            Err(unavailable())
        );
    }

    fn engine_with_cloud(
        dir: &tempfile::TempDir,
        server: &mut mockito::ServerGuard,
    ) -> std::sync::Arc<PennelEngine> {
        use crate::cloud::SecretStore;
        let engine =
            PennelEngine::new(dir.path().join("data").to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
        settings.speaker_recognition = SpeakerRecognitionMode::Cloud;
        engine.update_settings(settings).unwrap();
        let secrets = std::sync::Arc::new(crate::cloud::tests::MemorySecrets::default());
        secrets.set(crate::cloud::REFRESH_ACCOUNT.into(), Some("ref".into()));
        server
            .mock("POST", "/v1/auth/refresh")
            .with_body(r#"{"access_token":"acc","refresh_token":"ref2","expires_in":900}"#)
            .create();
        engine.use_cloud_session(crate::cloud::account::Session::new(server.url(), secrets));
        engine
    }

    fn cloud_result(turns: &[Turn], speakers: &[(u32, Vec<f32>)]) -> String {
        let speakers: Vec<Voice> = speakers
            .iter()
            .map(|(speaker, embedding)| Voice {
                speaker: *speaker,
                speech_ms: 20_000,
                embedding: embedding.clone(),
            })
            .collect();
        serde_json::json!({ "model": EMBEDDING_ID, "turns": turns, "speakers": speakers })
            .to_string()
    }

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    fn line(start_ms: u64, speaker: Speaker, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 3_000,
            speaker,
            text: text.into(),
            failed: false,
            voice: None,
        }
    }

    fn speech(seconds: usize) -> Vec<f32> {
        (0..seconds * 16_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.2)
            .collect()
    }

    #[test]
    fn enrolls_then_finds_me_and_a_remembered_voice_on_one_microphone() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        let me = vec![1.0, 0.0, 0.0];
        let jan = vec![0.0, 1.0, 0.0];
        let room = vec![0.0, 0.0, 1.0];

        let enrollment = server
            .mock("POST", "/v1/audio/diarize")
            .with_body(cloud_result(&[turn(0, 20_000, 0)], &[(0, me.clone())]))
            .create();
        let profile = engine.enroll_voice(speech(22)).unwrap();
        assert_eq!(profile.speech_seconds, 20);
        enrollment.remove();
        engine.voice_store().remember("Jan", &jan).unwrap();

        let id = engine
            .meeting_store()
            .create("Overleg", chrono::Local::now())
            .unwrap();
        engine
            .meeting_store()
            .write_owned_transcript(
                &id,
                &[
                    line(0, Speaker::Me, "goedemorgen allemaal"),
                    line(5_000, Speaker::Me, "dag iedereen"),
                    line(10_000, Speaker::Me, "zullen we beginnen"),
                ],
            )
            .unwrap();
        server
            .mock("POST", "/v1/audio/diarize")
            .with_body(cloud_result(
                &[
                    turn(0, 2_000, 0),
                    turn(2_500, 4_500, 0),
                    turn(5_000, 7_000, 1),
                    turn(7_500, 9_500, 1),
                    turn(10_000, 11_500, 2),
                    turn(12_000, 14_000, 2),
                ],
                &[(0, me.clone()), (1, jan.clone()), (2, room.clone())],
            ))
            .create();
        let result = engine
            .recognize_speakers(id.clone(), Some(speech(15)), Some(vec![0.0; 16_000 * 15]))
            .unwrap();
        assert_eq!(
            result,
            SpeakerRecognition {
                voices: 2,
                named: 1,
                me_found: true,
                complete: true,
                unassigned_voices: 0,
            }
        );
        let snapshot = engine
            .meeting_store()
            .read_transcript_snapshot(&id)
            .unwrap();
        let owner_fingerprint = voice_profile_fingerprint(&me);
        assert_eq!(
            snapshot.utterances[0]
                .metadata
                .self_voice_profile_fingerprint
                .as_deref(),
            Some(owner_fingerprint.as_str())
        );
        assert!(
            snapshot.utterances[1]
                .metadata
                .self_voice_profile_fingerprint
                .is_none()
        );
        let labels: Vec<(Speaker, Option<u32>)> = engine
            .meeting_store()
            .read_transcript(&id)
            .iter()
            .map(|s| (s.speaker, s.voice))
            .collect();
        assert_eq!(
            labels,
            [
                (Speaker::Me, None),
                (Speaker::Others, Some(1)),
                (Speaker::Others, Some(2))
            ]
        );
        assert_eq!(engine.meeting_speakers(id.clone()), [name(1, "Jan")]);
        assert_eq!(
            engine.meeting_store().transcript_tracks(&id).unwrap(),
            Some(vec![Speaker::Me])
        );

        let voices = engine.meeting_voices(id.clone());
        assert!(voices[0].remembered && voices[0].can_remember);
        assert!(!voices[1].remembered && voices[1].can_remember);
        engine
            .remember_speaker(id.clone(), 2, "Anna".into())
            .unwrap();
        engine
            .remember_speaker(id.clone(), 2, "anna".into())
            .unwrap();
        engine
            .remember_speaker(id.clone(), 1, "Jan".into())
            .unwrap();
        assert!(engine.known_voices().iter().all(|v| v.meetings == 1));
        assert!(engine.meeting_voices(id.clone())[1].remembered);
        let known: Vec<String> = engine.known_voices().into_iter().map(|v| v.name).collect();
        assert_eq!(known, ["Anna", "Jan"]);
        assert_eq!(
            engine.meeting_speakers(id),
            [name(1, "Jan"), name(2, "anna")]
        );
    }

    #[test]
    fn shared_microphone_works_without_enrollment_even_with_a_call() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        let id = engine
            .meeting_store()
            .create("Call", chrono::Local::now())
            .unwrap();
        engine
            .meeting_store()
            .write_owned_transcript(&id, &[line(0, Speaker::Me, "hallo")])
            .unwrap();
        assert!(engine.wants_microphone_track(None));
        assert!(engine.wants_microphone_track(Some(&speech(2))));
        let recognition = server
            .mock("POST", "/v1/audio/diarize")
            .with_body(cloud_result(
                &[turn(0, 2_000, 0), turn(2_500, 5_000, 0)],
                &[(0, vec![1.0, 0.0])],
            ))
            .create();
        let result = engine
            .recognize_speakers(id.clone(), Some(speech(5)), None)
            .unwrap();
        assert_eq!(result.voices, 1);
        assert!(!result.me_found);
        let rows = engine
            .meeting_store()
            .read_transcript_snapshot(&id)
            .unwrap()
            .utterances;
        assert_eq!(rows[0].segment.speaker, Speaker::Others);
        assert_eq!(rows[0].segment.voice, Some(1));
        assert_eq!(rows[0].metadata.source_id.as_deref(), Some("microphone"));
        recognition.assert();
    }

    #[test]
    fn hybrid_tracks_decode_sequentially_and_keep_distinct_physical_sources() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        let store = engine.meeting_store();
        let id = store.create("Hybrid", chrono::Local::now()).unwrap();
        store
            .write_owned_transcript(
                &id,
                &[
                    line(0, Speaker::Me, "Room speech."),
                    line(0, Speaker::Others, "Call speech."),
                ],
            )
            .unwrap();
        let first_completed = Arc::new(AtomicBool::new(false));
        let marker = first_completed.clone();
        let body = cloud_result(
            &[turn(0, 1_500, 0), turn(2_000, 4_000, 0)],
            &[(0, vec![1.0, 0.0])],
        );
        let first = server
            .mock("POST", "/v1/audio/diarize")
            .expect(1)
            .with_chunked_body(move |writer| {
                marker.store(true, Ordering::SeqCst);
                writer.write_all(body.as_bytes())
            })
            .create();
        let second = server
            .mock("POST", "/v1/audio/diarize")
            .expect(1)
            .with_body(cloud_result(
                &[turn(0, 1_500, 0), turn(2_000, 4_000, 0)],
                &[(0, vec![0.0, 1.0])],
            ))
            .create();
        let mut order = Vec::new();
        let result = engine
            .recognize_speakers_lazy(id.clone(), |source| {
                if source == Speaker::Me {
                    assert!(first_completed.load(Ordering::SeqCst));
                }
                order.push(source);
                Ok(Some(speech(4)))
            })
            .unwrap();
        assert_eq!(order, [Speaker::Others, Speaker::Me]);
        assert_eq!(result.voices, 2);
        assert!(!result.me_found);
        let rows = store.read_transcript_snapshot(&id).unwrap().utterances;
        assert_eq!(rows[0].metadata.source_id.as_deref(), Some("microphone"));
        assert_eq!(rows[1].metadata.source_id.as_deref(), Some("system_audio"));
        assert_eq!(rows[0].segment.voice, Some(2));
        assert_eq!(rows[1].segment.voice, Some(1));
        assert!(
            rows.iter()
                .all(|row| row.segment.speaker == Speaker::Others)
        );
        first.assert();
        second.assert();
    }

    #[test]
    fn meeting_app_naming_marks_rooms_lists_candidates_and_is_forgotten() {
        use crate::meeting_app_evidence::{AppSpeakerEvidence, CandidateSource, SpeakingSample};
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Teams call", chrono::Local::now()).unwrap();
        let voiced = |start_ms, voice| Segment {
            end_ms: start_ms + 6_000,
            voice: Some(voice),
            ..line(start_ms, Speaker::Others, "Call speech.")
        };
        store
            .write_owned_transcript(&id, &[voiced(0, 1), voiced(6_000, 2), voiced(12_000, 3)])
            .unwrap();
        store.rename_speaker(&id, 3, "Cas").unwrap();
        store
            .set_attendees(
                &id,
                &["Ann".into(), "Dirk Invited".into(), "Room 0.7".into()],
            )
            .unwrap();
        engine
            .set_meeting_calendar_context_json(
                id.clone(),
                serde_json::json!({
                    "source": "eventkit", "source_id": "s", "calendar_id": "c", "event_id": "e",
                    "source_revision": "r1",
                    "participants": [
                        {"source_id": "mailto:dirk@example.com", "name": "Dirk Invited", "is_self": false, "kind": "person"},
                        {"source_id": "mailto:room@example.com", "name": "Room 0.7", "is_self": false, "kind": "room"}
                    ]
                })
                .to_string(),
            )
            .unwrap();
        let samples = (0..72)
            .map(|step| SpeakingSample {
                at_ms: step * 250,
                available: true,
                speaking: vec![["Ann", "Room 0.7", "Cas Wouters"][(step / 24) as usize].into()],
            })
            .collect();
        engine
            .set_meeting_app_speaker_evidence(
                &id,
                AppSpeakerEvidence {
                    app_id: "com.microsoft.teams2".into(),
                    samples,
                    participants: vec!["Ann".into(), "Eva Silent".into(), "Room 0.7".into()],
                    ..AppSpeakerEvidence::default()
                },
            )
            .unwrap();

        let naming = engine.meeting_voice_naming(&id);

        let suggestions: Vec<_> = naming
            .suggestions
            .iter()
            .map(|s| (s.voice, s.name.as_str(), s.shared_device))
            .collect();
        assert_eq!(suggestions, [(1, "Ann", false), (2, "Room 0.7", true)]);
        let candidates: Vec<_> = naming
            .candidates
            .iter()
            .map(|c| (c.name.as_str(), c.source.clone()))
            .collect();
        assert!(candidates.contains(&("Dirk Invited", CandidateSource::Invited)));
        assert!(candidates.contains(&("Eva Silent", CandidateSource::MeetingApp)));
        assert!(!candidates.iter().any(|(name, _)| *name == "Room 0.7"));
        let key = engine.meeting_key(&id);
        assert!(engine.voice_store().app_speaker_evidence(&key).is_some());
        engine.delete_meeting(id.clone()).unwrap();
        assert!(engine.voice_store().app_speaker_evidence(&key).is_none());
    }

    #[test]
    fn mixed_call_voices_do_not_get_reused_or_overwrite_microphone_voiceprints() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Mixed hybrid", chrono::Local::now()).unwrap();
        store
            .write_owned_transcript(
                &id,
                &[
                    line(0, Speaker::Me, "Room speech."),
                    line(0, Speaker::Others, "Multiple call voices."),
                ],
            )
            .unwrap();
        let voice = |speaker, embedding| Voice {
            speaker,
            speech_ms: 1_500,
            embedding,
        };
        let call = Diarization {
            turns: vec![turn(0, 1_500, 0), turn(1_500, 3_000, 1)],
            speakers: vec![voice(0, vec![1.0, 0.0, 0.0]), voice(1, vec![0.0, 1.0, 0.0])],
        };
        let room = Diarization {
            turns: vec![turn(0, 3_000, 0)],
            speakers: vec![voice(0, vec![0.0, 0.0, 1.0])],
        };
        let _edit = engine.edit_meeting(&id).unwrap();
        let _sync = engine.sync_gate.lock().unwrap();
        let result = engine
            .apply_recognized_tracks_observed(
                &id,
                vec![(call, Track::System), (room, Track::Microphone)],
                None,
                None,
                |_, _| Ok(()),
            )
            .unwrap();
        assert_eq!(result.voices, 3);
        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(snapshot.utterances[0].segment.voice, Some(3));
        assert_eq!(snapshot.utterances[1].segment.voice, None);
        assert_eq!(snapshot.utterances[1].metadata.attribution.voices, [1, 2]);
        let prints = engine
            .voice_store()
            .meeting_voiceprints(&snapshot.meeting_uid, Some(&snapshot.transcript_digest));
        assert_eq!(prints.len(), 3);
        assert_eq!(
            prints
                .iter()
                .find(|print| print.voice == 1)
                .unwrap()
                .embedding,
            [1.0, 0.0, 0.0]
        );
        assert_eq!(
            prints
                .iter()
                .find(|print| print.voice == 2)
                .unwrap()
                .embedding,
            [0.0, 1.0, 0.0]
        );
        assert_eq!(
            prints
                .iter()
                .find(|print| print.voice == 3)
                .unwrap()
                .embedding,
            [0.0, 0.0, 1.0]
        );
        assert_eq!(engine.meeting_voices(id).len(), 3);
    }

    #[test]
    fn both_tracks_share_capacity_without_starving_the_microphone() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Crowded call", chrono::Local::now()).unwrap();
        let mut segments = vec![line(0, Speaker::Me, "Microphone speech.")];
        segments
            .extend((0..12).map(|index| line(index * 3_000, Speaker::Others, "One remote voice.")));
        let expected_words = segments
            .iter()
            .map(|row| row.text.split_whitespace().count())
            .sum::<usize>();
        store.write_owned_transcript(&id, &segments).unwrap();
        let remote = Diarization {
            turns: (0..12)
                .map(|index| turn(index * 3_000, (index + 1) * 3_000, index as u32))
                .collect(),
            speakers: (0..12)
                .map(|speaker| Voice {
                    speaker,
                    speech_ms: 3_000,
                    embedding: vec![],
                })
                .collect(),
        };
        let mic = Diarization {
            turns: vec![turn(0, 3_000, 0)],
            speakers: vec![Voice {
                speaker: 0,
                speech_ms: 3_000,
                embedding: vec![],
            }],
        };
        let _edit = engine.edit_meeting(&id).unwrap();
        let _sync = engine.sync_gate.lock().unwrap();
        let result = engine
            .apply_recognized_tracks_observed(
                &id,
                vec![(remote, Track::System), (mic, Track::Microphone)],
                None,
                None,
                |_, _| Ok(()),
            )
            .unwrap();
        assert!(!result.complete);
        assert_eq!(result.unassigned_voices, 1);
        let rows = store.read_transcript_snapshot(&id).unwrap().utterances;
        let microphone = rows
            .iter()
            .find(|row| row.metadata.source_id.as_deref() == Some("microphone"))
            .unwrap();
        assert!(
            microphone.segment.voice.is_some(),
            "the remote track must not consume every meeting voice ID"
        );
        assert!(
            !microphone.metadata.attribution.self_voice,
            "a microphone is not automatically the owner"
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.segment.text.split_whitespace().count())
                .sum::<usize>(),
            expected_words
        );
    }

    #[test]
    fn speaker_commit_reserves_ids_of_private_empty_completion_markers() {
        use crate::meeting_utterances::{AcousticTurn, AttributionKind, MeetingUtterance};
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Private completion marker", chrono::Local::now())
            .unwrap();
        let mut empty = MeetingUtterance::new(
            Segment {
                voice: Some(1),
                ..line(0, Speaker::Others, "")
            },
            Some("microphone".into()),
        );
        empty.metadata.attribution.kind = AttributionKind::Single;
        empty.metadata.attribution.voices = vec![1];
        empty.metadata.attribution.turns = vec![AcousticTurn {
            start_ms: 0,
            end_ms: 3_000,
            voice: Some(1),
        }];
        let remote = MeetingUtterance::new(
            line(0, Speaker::Others, "Remote speech."),
            Some("system_audio".into()),
        );
        store
            .write_transcript_snapshot(&id, None, &[empty.clone(), remote])
            .unwrap();
        assert_eq!(
            store
                .read_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .len(),
            1
        );
        let diarization = || Diarization {
            turns: vec![turn(0, 3_000, 0)],
            speakers: vec![Voice {
                speaker: 0,
                speech_ms: 3_000,
                embedding: vec![],
            }],
        };
        let _edit = engine.edit_meeting(&id).unwrap();
        let _sync = engine.sync_gate.lock().unwrap();
        let result = engine
            .apply_recognized_tracks_observed(
                &id,
                vec![
                    (diarization(), Track::System),
                    (diarization(), Track::Microphone),
                ],
                None,
                None,
                |_, _| Ok(()),
            )
            .unwrap();
        let visible = store.read_transcript_snapshot(&id).unwrap().utterances;
        assert_eq!(visible[0].segment.voice, Some(2));
        assert_eq!(result.voices, 1);
        let committed = store.read_committed_transcript_snapshot(&id).unwrap();
        assert_eq!(
            committed
                .utterances
                .iter()
                .find(|row| row.segment.text.is_empty())
                .unwrap(),
            &empty
        );
    }

    #[test]
    fn modern_brief_phantoms_and_weak_old_numbers_cannot_starve_repeated_clean_speech() {
        use crate::meeting_utterances::{AcousticTurn, AttributionKind, MeetingUtterance};
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Clean identity admission", chrono::Local::now())
            .unwrap();
        let mut rows = Vec::new();
        let mut turns = Vec::new();
        let mut speakers = Vec::new();
        for speaker in 0..13u32 {
            let start = u64::from(speaker) * 1_000;
            let end = if speaker == 12 { 17_000 } else { start + 400 };
            let mut segment = line(
                start,
                Speaker::Others,
                if speaker == 12 {
                    "Repeated clean speech."
                } else {
                    "Yes."
                },
            );
            segment.end_ms = end;
            segment.voice = (speaker < 12).then_some(speaker + 1);
            let mut row = MeetingUtterance::new(segment, Some("system_audio".into()));
            if speaker < 12 {
                row.metadata.attribution.kind = AttributionKind::Single;
                row.metadata.attribution.voices = vec![speaker + 1];
                row.metadata.attribution.turns = vec![AcousticTurn {
                    start_ms: start,
                    end_ms: end,
                    voice: Some(speaker + 1),
                }];
                turns.push(turn(start, end, speaker));
            } else {
                turns.extend([turn(12_000, 14_000, speaker), turn(15_000, 17_000, speaker)]);
            }
            rows.push(row);
            let mut embedding = vec![0.0; 13];
            embedding[speaker as usize] = 1.0;
            speakers.push(Voice {
                speaker,
                speech_ms: end - start,
                embedding,
            });
        }
        store.write_transcript_snapshot(&id, None, &rows).unwrap();
        let before_text: Vec<_> = rows.iter().map(|row| row.segment.text.clone()).collect();
        let _edit = engine.edit_meeting(&id).unwrap();
        let _sync = engine.sync_gate.lock().unwrap();
        let result = engine
            .apply_recognized_speakers_observed(
                &id,
                Diarization { turns, speakers },
                Track::System,
                None,
                None,
                |_, _| Ok(()),
            )
            .unwrap();
        let after = store.read_transcript_snapshot(&id).unwrap().utterances;
        assert_eq!(result.voices, 1);
        assert!(result.complete);
        assert_eq!(
            result.unassigned_voices, 0,
            "intentional low-evidence abstentions are not capacity failures"
        );
        assert!(after[..12].iter().all(|row| row.segment.voice.is_none()
            && row.metadata.attribution.kind == AttributionKind::Unknown));
        assert_eq!(after[12].segment.voice, Some(1));
        assert_eq!(
            engine
                .meeting_speaker_processing_status(id.clone())
                .unwrap()
                .reason
                .as_deref(),
            Some("limited_evidence")
        );
        assert_eq!(
            after
                .iter()
                .map(|row| row.segment.text.clone())
                .collect::<Vec<_>>(),
            before_text
        );
    }

    #[test]
    fn modern_admission_keeps_repeated_speech_and_clear_long_monologues() {
        let diarization = Diarization {
            turns: vec![
                turn(0, 400, 0),
                turn(1_000, 3_000, 1),
                turn(4_000, 6_000, 1),
                turn(7_000, 17_000, 2),
                turn(18_000, 38_000, 3),
                turn(39_000, 43_000, 4),
            ],
            speakers: (0..5)
                .map(|speaker| Voice {
                    speaker,
                    speech_ms: 20_000,
                    embedding: vec![1.0, 0.0],
                })
                .collect(),
        };
        let prepared = PreparedTrack::new(diarization, Track::System, true);
        assert_eq!(
            prepared.eligible_speakers.unwrap(),
            [1, 2, 3].into_iter().collect()
        );
        assert!(prepared.diarization.speakers[0].embedding.is_empty());
        assert!(prepared.diarization.speakers[4].embedding.is_empty());
        assert_eq!(
            prepared.diarization.turns.len(),
            6,
            "admission must preserve short and uncertain turns"
        );
    }

    #[test]
    fn modern_admission_rejects_missing_zero_nonfinite_and_overlapping_prints() {
        for embedding in [
            vec![],
            vec![0.0, 0.0],
            vec![f32::NAN, 1.0],
            vec![f32::INFINITY, 1.0],
        ] {
            let prepared = PreparedTrack::new(
                Diarization {
                    turns: vec![turn(0, 2_000, 0), turn(3_000, 5_000, 0)],
                    speakers: vec![Voice {
                        speaker: 0,
                        speech_ms: 4_000,
                        embedding,
                    }],
                },
                Track::System,
                true,
            );
            assert!(prepared.eligible_speakers.unwrap().is_empty());
            assert!(prepared.diarization.speakers[0].embedding.is_empty());
        }
        let prepared = PreparedTrack::new(
            Diarization {
                turns: vec![turn(0, 12_000, 0), turn(0, 12_000, 1)],
                speakers: (0..2)
                    .map(|speaker| Voice {
                        speaker,
                        speech_ms: 12_000,
                        embedding: vec![1.0, 0.0],
                    })
                    .collect(),
            },
            Track::System,
            true,
        );
        assert!(
            prepared.eligible_speakers.unwrap().is_empty(),
            "overlap cannot establish clean identity duration"
        );
    }

    #[test]
    fn weak_modern_prints_cannot_gain_known_or_self_identity_before_admission() {
        let prepared = PreparedTrack::new(
            Diarization {
                turns: vec![turn(0, 1_200, 0), turn(2_000, 3_200, 1)],
                speakers: vec![
                    Voice {
                        speaker: 0,
                        speech_ms: 1_200,
                        embedding: vec![1.0, 0.0],
                    },
                    Voice {
                        speaker: 1,
                        speech_ms: 1_200,
                        embedding: vec![0.0, 1.0],
                    },
                ],
            },
            Track::Microphone,
            true,
        );
        assert_eq!(
            pennel_speakers::identify(
                &prepared.diarization.speakers,
                Some(&[1.0, 0.0]),
                &[&[0.0, 1.0]]
            ),
            [Identity::Unknown, Identity::Unknown]
        );
        assert!(prepared.eligible_speakers.unwrap().is_empty());
    }

    #[test]
    fn explicit_legacy_mode_does_not_treat_empty_prints_as_modern_identity_evidence() {
        let diarization = from_sortformer(&[(0, 400, 1)]);
        let legacy = PreparedTrack::new(diarization.clone(), Track::System, false);
        let modern = PreparedTrack::new(diarization, Track::System, true);
        assert!(legacy.eligible_speakers.is_none());
        assert_eq!(
            modern.eligible_speakers,
            Some(std::collections::BTreeSet::new())
        );
    }

    #[test]
    fn genuine_manual_identity_survives_modern_low_evidence_abstention() {
        use crate::meeting_utterances::{AttributionKind, MeetingUtterance};
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Manual authority", chrono::Local::now())
            .unwrap();
        let mut row = MeetingUtterance::new(
            Segment {
                end_ms: 400,
                voice: Some(7),
                ..line(0, Speaker::Others, "Yes.")
            },
            Some("system_audio".into()),
        );
        row.metadata.attribution.kind = AttributionKind::Single;
        row.metadata.attribution.voices = vec![7];
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        store
            .rename_speaker(&id, 7, "Explicitly confirmed")
            .unwrap();
        let _edit = engine.edit_meeting(&id).unwrap();
        let _sync = engine.sync_gate.lock().unwrap();
        let result = engine
            .apply_recognized_speakers_observed(
                &id,
                Diarization {
                    turns: vec![turn(0, 400, 0)],
                    speakers: vec![Voice {
                        speaker: 0,
                        speech_ms: 400,
                        embedding: vec![1.0, 0.0],
                    }],
                },
                Track::System,
                None,
                None,
                |_, _| Ok(()),
            )
            .unwrap();
        assert_eq!(result.voices, 1);
        assert_eq!(result.named, 1);
        assert!(result.complete);
        assert_eq!(result.unassigned_voices, 0);
        assert_eq!(store.speaker_names(&id), [name(7, "Explicitly confirmed")]);
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().utterances[0]
                .segment
                .voice,
            Some(7)
        );
        assert!(engine.known_voices().is_empty());
    }

    #[test]
    fn explicit_microphone_owner_is_transactional_and_preserves_other_people_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Personal headset", chrono::Local::now())
            .unwrap();
        store
            .write_owned_transcript(
                &id,
                &[
                    line(0, Speaker::Me, "My own words."),
                    Segment {
                        voice: Some(1),
                        ..line(0, Speaker::Others, "Remote words.")
                    },
                ],
            )
            .unwrap();
        store.rename_speaker(&id, 1, "Confirmed remote").unwrap();
        std::fs::write(store.dir_of(&id).join("notes.md"), "Private typed note").unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        let remote = before
            .utterances
            .iter()
            .find(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
            .unwrap()
            .clone();
        assert_eq!(
            engine.confirm_meeting_microphone_owner(id.clone()).unwrap(),
            1
        );
        let after = store.read_transcript_snapshot(&id).unwrap();
        let owner = after
            .utterances
            .iter()
            .find(|row| row.metadata.source_id.as_deref() == Some("microphone"))
            .unwrap();
        assert_eq!(owner.segment.text, "My own words.");
        assert_eq!(owner.segment.speaker, Speaker::Me);
        assert!(owner.metadata.manual.as_ref().unwrap().self_voice);
        assert!(owner.metadata.self_voice_profile_fingerprint.is_none());
        assert_eq!(
            after
                .utterances
                .iter()
                .find(|row| row.metadata.source_id.as_deref() == Some("system_audio"))
                .unwrap(),
            &remote
        );
        assert_eq!(
            std::fs::read_to_string(store.dir_of(&id).join("notes.md")).unwrap(),
            "Private typed note"
        );
        assert!(
            store
                .dir_of(&id)
                .join(".pennel-transcript-backups")
                .is_dir()
        );
        assert!(engine.known_voices().is_empty());
    }

    #[test]
    fn owner_confirmation_refuses_existing_manual_assignments_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Shared microphone", chrono::Local::now())
            .unwrap();
        let mut row = crate::meeting_utterances::MeetingUtterance::new(
            Segment {
                voice: Some(2),
                ..line(0, Speaker::Others, "Another person.")
            },
            Some("microphone".into()),
        );
        row.metadata.attribution.kind = crate::meeting_utterances::AttributionKind::Single;
        row.metadata.attribution.voices = vec![2];
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        store.rename_speaker(&id, 2, "Manually confirmed").unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        assert!(engine.confirm_meeting_microphone_owner(id.clone()).is_err());
        assert_eq!(store.read_transcript_snapshot(&id).unwrap(), before);
    }

    #[test]
    fn processing_status_is_route_bound_persistent_and_invalidated_by_a_new_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Status", chrono::Local::now()).unwrap();
        store
            .write_owned_transcript(&id, &[line(0, Speaker::Me, "First words.")])
            .unwrap();
        let status = engine
            .set_meeting_speaker_processing_status(
                id.clone(),
                "incomplete".into(),
                Some("unassigned_voices".into()),
                SpeakerRecognitionMode::Cloud,
                2,
            )
            .unwrap();
        assert_eq!(status.route, "cloud");
        assert_eq!(status.model_ids, [EMBEDDING_ID]);
        assert!(!status.complete);
        assert_eq!(
            engine.meeting_speaker_processing_status(id.clone()),
            Some(status)
        );
        store
            .write_owned_transcript(&id, &[line(0, Speaker::Me, "Changed words.")])
            .unwrap();
        let stale = engine
            .meeting_speaker_processing_status(id.clone())
            .unwrap();
        assert_eq!(stale.phase, "outdated");
        assert_eq!(stale.reason.as_deref(), Some("source_changed"));
        assert!(!stale.complete);
        let bytes = std::fs::read(store.dir_of(&id).join(SPEAKER_PROCESSING_FILE)).unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("words"));
        assert!(
            engine
                .set_meeting_speaker_processing_status(
                    id,
                    "failed".into(),
                    Some("transcript content must never become a diagnostic".into()),
                    SpeakerRecognitionMode::Cloud,
                    0
                )
                .is_err()
        );
    }

    #[test]
    fn rejected_concurrent_recognition_cannot_replace_the_active_runs_status() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.speaker_recognition = SpeakerRecognitionMode::Local;
        engine.update_settings(settings).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Concurrent speaker run", chrono::Local::now())
            .unwrap();
        store
            .write_owned_transcript(&id, &[line(0, Speaker::Me, "Owned speech.")])
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let active_engine = engine.clone();
        let active_id = id.clone();
        let active = std::thread::spawn(move || {
            active_engine.recognize_speakers_lazy(active_id, |source| {
                if source == Speaker::Others {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                }
                Ok(None)
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let running = engine
            .meeting_speaker_processing_status(id.clone())
            .unwrap();
        assert_eq!(running.phase, "running");
        let rejected = engine.recognize_speakers_lazy(id.clone(), |_| {
            panic!("a rejected concurrent run must never decode audio")
        });
        let while_blocked = engine.meeting_speaker_processing_status(id).unwrap();
        release_tx.send(()).unwrap();
        active.join().unwrap().unwrap();
        assert!(rejected.is_err());
        assert_eq!(
            while_blocked, running,
            "only the run holding the meeting edit guard owns its status"
        );
    }

    #[test]
    fn naming_opt_out_before_the_joint_commit_cancels_without_replacing_labels() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.meeting_app_speakers = true;
        engine.update_settings(settings).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Late opt-out", chrono::Local::now()).unwrap();
        store
            .write_owned_transcript(&id, &[line(0, Speaker::Others, "Call speech.")])
            .unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        let diarization = Diarization {
            turns: vec![turn(0, 3_000, 0)],
            speakers: vec![Voice {
                speaker: 0,
                speech_ms: 3_000,
                embedding: vec![],
            }],
        };
        let _edit = engine.edit_meeting(&id).unwrap();
        let _sync = engine.sync_gate.lock().unwrap();
        let result = engine.apply_recognized_tracks_observed(
            &id,
            vec![(diarization, Track::System)],
            None,
            None,
            |_, _| {
                let mut settings = engine.settings();
                settings.meeting_app_speakers = false;
                engine.update_settings(settings)
            },
        );
        assert_eq!(result, Err(PennelError::Cancelled));
        assert_eq!(store.read_transcript_snapshot(&id).unwrap(), before);
        assert!(
            !store
                .dir_of(&id)
                .join(".pennel-transcript-backups")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn speaker_status_does_not_follow_private_directory_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let store = engine.meeting_store();
        let id = store
            .create("Unsafe status path", chrono::Local::now())
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), store.dir_of(&id).join(".pennel")).unwrap();
        assert!(
            engine
                .set_meeting_speaker_processing_status(
                    id.clone(),
                    "running".into(),
                    None,
                    SpeakerRecognitionMode::Local,
                    0
                )
                .is_err()
        );
        assert!(engine.meeting_speaker_processing_status(id).is_none());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn legacy_others_or_missing_hybrid_archive_cannot_establish_physical_source() {
        for missing_hybrid in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = crate::meetings::MeetingStore::new(directory.path());
            let id = store
                .create("Ambiguous archive", chrono::Local::now())
                .unwrap();
            let mut segments = vec![line(0, Speaker::Others, "Uncertain source.")];
            if missing_hybrid {
                segments.push(line(4_000, Speaker::Me, "Missing microphone."));
            }
            store.write_transcript(&id, &segments).unwrap();
            if missing_hybrid {
                store.preserve_transcript_tracks(&id).unwrap();
            }
            let archive = store.dir_of(&id).join(if missing_hybrid {
                "audio/system.m4a"
            } else {
                "anderen.m4a"
            });
            std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
            std::fs::write(archive, b"owned ambiguous archive").unwrap();
            let before = store.read_transcript_snapshot(&id).unwrap();
            let mut rows = before.utterances.clone();
            establish_single_archive_source(&store, &id, &mut rows).unwrap();
            assert_eq!(rows, before.utterances);
            let (after, applied) = apply_utterances(
                &rows,
                &[turn(0, 10_000, 0)],
                &[Identity::Unknown],
                Track::System,
            );
            assert_eq!(after, rows);
            assert!(applied.voices.is_empty());
            assert!(after.iter().all(|row| row.metadata.source_id.is_none()));
            assert_eq!(store.read_transcript(&id), segments);
        }
    }

    #[test]
    fn explicit_modern_room_archive_establishes_source_with_lineage_and_neutral_voice() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::meetings::MeetingStore::new(directory.path());
        let id = store.create("Explicit room", chrono::Local::now()).unwrap();
        store
            .write_transcript(&id, &[line(0, Speaker::Others, "Room source.")])
            .unwrap();
        let archive = store.dir_of(&id).join("audio/room.m4a");
        std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
        std::fs::write(archive, b"owned declared room archive").unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        let mut rows = before.utterances.clone();
        establish_single_archive_source(&store, &id, &mut rows).unwrap();
        assert_eq!(
            rows[0].metadata.source_id.as_deref(),
            Some("room_microphone")
        );
        assert_ne!(
            rows[0].metadata.utterance_id,
            before.utterances[0].metadata.utterance_id
        );
        assert_eq!(
            rows[0].metadata.lineage,
            [before.utterances[0].metadata.utterance_id.clone()]
        );
        let (rows, _) = apply_utterances(
            &rows,
            &[turn(0, 3_000, 0)],
            &[Identity::Unknown],
            Track::Microphone,
        );
        assert_eq!(rows[0].segment.voice, Some(1));
        assert_eq!(rows[0].segment.speaker, Speaker::Others);
        store
            .write_transcript_snapshot(&id, Some(before.revision), &rows)
            .unwrap();
        assert_eq!(
            store.read_transcript_snapshot(&id).unwrap().utterances,
            rows
        );
    }

    #[test]
    fn speaker_route_change_before_inference_cancels_without_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        let store = engine.meeting_store();
        let id = store.create("Frozen route", chrono::Local::now()).unwrap();
        store
            .write_owned_transcript(&id, &[line(0, Speaker::Others, "Keep source.")])
            .unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        let no_inference = server.mock("POST", "/v1/audio/diarize").expect(0).create();
        let result = engine.recognize_speakers_lazy(id.clone(), |_| {
            let mut settings = engine.settings();
            settings.speaker_recognition = SpeakerRecognitionMode::Local;
            engine.update_settings(settings).unwrap();
            Ok(Some(speech(3)))
        });
        assert_eq!(result, Err(PennelError::Cancelled));
        assert_eq!(store.read_transcript_snapshot(&id).unwrap(), before);
        no_inference.assert();
    }

    #[test]
    fn account_change_before_inference_cancels_and_retains_original_attribution() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        let store = engine.meeting_store();
        let id = store
            .create("Frozen account", chrono::Local::now())
            .unwrap();
        store
            .write_owned_transcript(&id, &[line(0, Speaker::Others, "Keep original.")])
            .unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        let no_inference = server.mock("POST", "/v1/audio/diarize").expect(0).create();
        let result = engine.recognize_speakers_lazy(id.clone(), |_| {
            engine.cloud_logout().unwrap();
            Ok(Some(speech(3)))
        });
        assert_eq!(result, Err(PennelError::Cancelled));
        assert_eq!(store.read_transcript_snapshot(&id).unwrap(), before);
        no_inference.assert();
    }

    #[test]
    fn a_shared_microphone_without_a_matching_enrollment_uses_neutral_voices() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        engine
            .voice_store()
            .set_my_voiceprint(vec![1.0, 0.0], 20_000)
            .unwrap();
        let id = engine
            .meeting_store()
            .create("Zaal", chrono::Local::now())
            .unwrap();
        let lines = [
            line(0, Speaker::Me, "eerste"),
            line(5_000, Speaker::Me, "tweede"),
        ];
        engine
            .meeting_store()
            .write_owned_transcript(&id, &lines)
            .unwrap();
        server
            .mock("POST", "/v1/audio/diarize")
            .with_body(cloud_result(
                &[
                    turn(0, 2_000, 0),
                    turn(2_500, 4_500, 0),
                    turn(5_000, 7_000, 1),
                    turn(7_500, 9_500, 1),
                ],
                &[(0, vec![0.0, 1.0]), (1, vec![-1.0, 0.0])],
            ))
            .create();
        let result = engine
            .recognize_speakers(id.clone(), Some(speech(10)), None)
            .unwrap();
        assert_eq!(result.voices, 2);
        assert!(!result.me_found);
        let rows = engine
            .meeting_store()
            .read_transcript_snapshot(&id)
            .unwrap()
            .utterances;
        assert_eq!(
            rows.iter()
                .map(|row| row.segment.text.as_str())
                .collect::<Vec<_>>(),
            lines
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>()
        );
        assert!(rows.iter().all(|row| row.segment.speaker == Speaker::Others
            && row.segment.voice.is_some()
            && row.metadata.source_id.as_deref() == Some("microphone")));
    }

    #[test]
    fn remembering_needs_a_voiceprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        let id = engine
            .meeting_store()
            .create("Oud", chrono::Local::now())
            .unwrap();
        assert_eq!(
            engine.remember_speaker(id, 1, "Jan".into()),
            Err(PennelError::InvalidResponse {
                message: NO_VOICEPRINT.into()
            })
        );
    }

    #[test]
    fn forgetting_everything_removes_the_voice_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = mockito::Server::new();
        let engine = engine_with_cloud(&dir, &mut server);
        engine
            .voice_store()
            .set_my_voiceprint(vec![1.0], 12_000)
            .unwrap();
        engine.voice_store().remember("Jan", &[1.0]).unwrap();
        engine.forget_all_voices().unwrap();
        assert!(engine.voice_profile().is_none());
        assert!(engine.known_voices().is_empty());
        engine.forget_all_voices().unwrap();
    }

    #[test]
    fn local_mode_without_models_asks_for_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(dir.path().to_string_lossy().into_owned()).unwrap();
        assert!(!engine.speaker_recognition_ready());
        let expected = if cfg!(feature = "speakers") {
            PennelError::ModelNotDownloaded
        } else {
            unavailable()
        };
        assert_eq!(engine.enroll_voice(speech(12)).unwrap_err(), expected);
        assert_eq!(engine.speaker_model_ids(), [SEGMENTATION_ID, EMBEDDING_ID]);
    }
}
