//! Speaker recognition: who spoke in a meeting, including the user's own trained voice and the
//! voices they chose to remember. The exported surface is the same with and without the `speakers`
//! feature; without it, local recognition answers with an English diagnostic and Pennel Cloud still works.

use std::path::Path;

use pennel_speakers::{Diarization, Identity};

use crate::diarize::{
    Track, apply_utterances, carry_utterance_names, from_sortformer, mostly_silent,
    preserve_manual_names,
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

pub(crate) fn unavailable() -> PennelError {
    PennelError::InvalidResponse {
        message: UNAVAILABLE.into(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct SpeakerRecognition {
    /// Distinct other voices in the transcript after recognition.
    pub voices: u32,
    /// Voices that carry a name: a remembered voice or one the user named before.
    pub named: u32,
    /// The user's trained voice was heard on a shared microphone.
    pub me_found: bool,
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

#[cfg(feature = "speakers")]
pub(crate) fn diarize_local(
    segmentation: &Path,
    embedding: &Path,
    samples: &[f32],
) -> Result<Diarization, PennelError> {
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
    /// Hybrid meetings analyze both physical sources. Enrollment only helps identify the user;
    /// it is never required to obtain neutral numbered speakers from a shared microphone.
    pub fn wants_microphone_track(&self, _system: Option<&[f32]>) -> bool {
        self.speaker_recognition_ready()
    }

    pub(crate) fn voice_store(&self) -> VoiceStore {
        VoiceStore::new(&self.data_dir())
    }

    /// Meeting voiceprints follow the meeting's sync identity, which survives a rename.
    fn meeting_key(&self, id: &str) -> String {
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
        if route == SpeakerRecognitionMode::Cloud {
            return match cloud {
                Some(session) => crate::cloud::diarize::diarize(session, samples),
                None => crate::cloud::diarize::diarize(&*self.cloud_session()?, samples),
            };
        }
        let models = (
            self.models.path(SEGMENTATION_ID),
            self.models.path(EMBEDDING_ID),
        );
        if let (Some(segmentation), Some(embedding)) = models {
            return diarize_local(&segmentation, &embedding, samples);
        }
        if !cfg!(feature = "speakers") {
            return Err(unavailable());
        }
        match self.models.path(DIARIZER_ID) {
            Some(sortformer) if !voiceprints => Ok(from_sortformer(&crate::diarize::diarize(
                &sortformer,
                samples,
            )?)),
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
    /// Decode and infer one physical track at a time, then apply both in one recoverable commit.
    pub fn recognize_speakers_lazy(
        &self,
        id: String,
        mut decode: impl FnMut(crate::Speaker) -> Result<Option<Vec<f32>>, PennelError>,
    ) -> Result<SpeakerRecognition, PennelError> {
        self.claim_library()?;
        let _edit = self.edit_meeting(&id)?;
        let store = self.meeting_store();
        store.info(&id).ok_or(PennelError::NotFound)?;
        if store.unparsed_transcript_lines(&id) > 0 {
            return Err(PennelError::InvalidResponse {
                message: "the transcript contains lines that Pennel does not recognize".into(),
            });
        }
        let route = self.settings().speaker_recognition;
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
                let diarization =
                    self.speaker_diarization_selected(&samples, false, route, cloud.as_deref())?;
                results.push((diarization, track));
            }
        }
        if results.is_empty() {
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
        self.apply_recognized_tracks_observed(&id, results, my_voice.as_deref(), None, |_, _| {
            ensure_current()
        })
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
        self.apply_recognized_tracks_observed(
            id,
            vec![(diarization, track)],
            my_voice,
            job_directory,
            before_commit,
        )
    }

    fn apply_recognized_tracks_observed(
        &self,
        id: &str,
        diarizations: Vec<(Diarization, Track)>,
        my_voice: Option<&[f32]>,
        job_directory: Option<&Path>,
        before_commit: impl FnOnce(&[crate::Segment], &[crate::SpeakerName]) -> Result<(), PennelError>,
    ) -> Result<SpeakerRecognition, PennelError> {
        let store = self.meeting_store();
        let voices = self.voice_store();
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
        let mut me_found = false;
        for (diarization, track) in diarizations {
            let identities = pennel_speakers::identify(
                &diarization.speakers,
                my_voice.filter(|_| track == Track::Microphone),
                &known_prints,
            );
            let (updated, applied) =
                apply_utterances(&after, &diarization.turns, &identities, track);
            after = updated;
            me_found |= applied.me_found;
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
        let names = carry_utterance_names(&before, &after, &old_names, &recognised);
        let segments: Vec<_> = after.iter().map(|row| row.segment.clone()).collect();
        before_commit(&segments, &names)?;
        self.commit_speaker_utterances(id, &after, &names, prints, job_directory)?;
        let mut present: Vec<_> = after
            .iter()
            .flat_map(|row| row.metadata.attribution.voices.iter().copied())
            .chain(after.iter().filter_map(|row| row.segment.voice))
            .collect();
        present.sort_unstable();
        present.dedup();
        Ok(SpeakerRecognition {
            voices: present.len() as u32,
            named: names.len() as u32,
            me_found,
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
                    crate::meeting_utterances::MeetingUtterance::new(segment, Some(source.into()))
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
                    turn(0, 4_500, 0),
                    turn(5_000, 9_500, 1),
                    turn(10_000, 14_000, 2),
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
                me_found: true
            }
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
            .with_body(cloud_result(&[turn(0, 5_000, 0)], &[(0, vec![1.0, 0.0])]))
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
        let body = cloud_result(&[turn(0, 3_000, 0)], &[(0, vec![1.0, 0.0])]);
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
            .with_body(cloud_result(&[turn(0, 3_000, 0)], &[(0, vec![0.0, 1.0])]))
            .create();
        let mut order = Vec::new();
        let result = engine
            .recognize_speakers_lazy(id.clone(), |source| {
                if source == Speaker::Me {
                    assert!(first_completed.load(Ordering::SeqCst));
                }
                order.push(source);
                Ok(Some(speech(3)))
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
                &[turn(0, 4_500, 0), turn(5_000, 9_500, 1)],
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
