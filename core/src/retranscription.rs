use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use crate::chunker::Chunker;
use crate::engine::PennelEngine;
use crate::error::PennelError;
use crate::meetings::SpeakerName;
use crate::meetings::{MeetingStore, Segment, Speaker};
use crate::provider::ProviderKind;
use crate::session::MeetingRecognition;
use crate::settings::Settings;
use crate::speech::is_hallucination;
use crate::voices::MeetingVoiceprint;
use pennel_speakers::{Diarization, Identity};

pub(crate) struct MeetingEditGuard<'a> {
    pub(crate) jobs: &'a Mutex<HashSet<String>>,
    pub(crate) id: String,
}

impl Drop for MeetingEditGuard<'_> {
    fn drop(&mut self) {
        self.jobs.lock().unwrap().remove(&self.id);
    }
}

struct FrozenLocal {
    settings: Settings,
    recognition: MeetingRecognition,
    model_path: PathBuf,
}

impl FrozenLocal {
    fn new(engine: &PennelEngine) -> Result<Self, PennelError> {
        let settings = engine.settings();
        let (account, model) = settings
            .resolve(&settings.meeting_stt)
            .ok_or(PennelError::NotFound)?;
        if account.kind != ProviderKind::Local {
            return Err(PennelError::InvalidResponse {
                message: "Choose a local speech model to transcribe a saved recording again."
                    .into(),
            });
        }
        crate::local_engine::ensure_available()?;
        if !engine
            .models
            .local_models(None, false)
            .iter()
            .any(|entry| entry.id == model)
        {
            return Err(PennelError::NotFound);
        }
        let model_path = engine
            .models
            .path(&model)
            .ok_or(PennelError::ModelNotDownloaded)?;
        let recognition = MeetingRecognition::from_settings(&settings);
        Ok(Self {
            settings,
            recognition,
            model_path,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MeetingRetranscriptionStatus {
    pub running: bool,
    pub recognition: MeetingRecognition,
    pub progress: RetranscriptionProgress,
    pub error: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MeetingRetranscriptionPlan {
    pub recognition: MeetingRecognition,
    pub available: bool,
    pub reason: Option<String>,
}

struct VoiceCandidate {
    name: Option<String>,
    embedding: Vec<f32>,
    known_id: Option<String>,
}

#[derive(Default)]
struct SpeakerLabels {
    names: Vec<SpeakerName>,
    prints: Vec<MeetingVoiceprint>,
}

#[cfg(test)]
fn relabel_speakers(
    segments: &mut [Segment],
    diarization: &Diarization,
    track: crate::diarize::Track,
    me: Option<&[f32]>,
    candidates: &[VoiceCandidate],
) -> SpeakerLabels {
    let prints: Vec<&[f32]> = candidates
        .iter()
        .map(|candidate| candidate.embedding.as_slice())
        .collect();
    let identities = pennel_speakers::identify(&diarization.speakers, me, &prints);
    if track == crate::diarize::Track::Microphone && !identities.contains(&Identity::Me) {
        return SpeakerLabels::default();
    }
    let applied = crate::diarize::apply(segments, &diarization.turns, &identities, track);
    let mut result = SpeakerLabels::default();
    for (voice, speaker) in applied.voices {
        let candidate = match identities.get(speaker) {
            Some(Identity::Known(index, _)) => candidates.get(*index),
            _ => None,
        };
        if let Some(name) = candidate.and_then(|candidate| candidate.name.as_ref()) {
            result.names.push(SpeakerName {
                voice,
                name: name.clone(),
            });
        }
        if let Some(speaker) = diarization
            .speakers
            .get(speaker)
            .filter(|speaker| !speaker.embedding.is_empty())
        {
            result.prints.push(MeetingVoiceprint {
                voice,
                embedding: speaker.embedding.clone(),
                known_id: candidate.and_then(|candidate| candidate.known_id.clone()),
            });
        }
    }
    result
}

fn relabel_utterances(
    rows: &[crate::meeting_utterances::MeetingUtterance],
    diarization: &Diarization,
    track: crate::diarize::Track,
    me: Option<&[f32]>,
    candidates: &[VoiceCandidate],
) -> (
    Vec<crate::meeting_utterances::MeetingUtterance>,
    SpeakerLabels,
) {
    let prints: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.embedding.as_slice())
        .collect();
    let identities = pennel_speakers::identify(&diarization.speakers, me, &prints);
    let (rows, applied) =
        crate::diarize::apply_utterances(rows, &diarization.turns, &identities, track);
    let mut labels = SpeakerLabels::default();
    for (voice, speaker) in applied.voices {
        let candidate = match identities.get(speaker) {
            Some(Identity::Known(index, _)) => candidates.get(*index),
            _ => None,
        };
        if let Some(name) = candidate.and_then(|candidate| candidate.name.as_ref()) {
            labels.names.push(SpeakerName {
                voice,
                name: name.clone(),
            });
        }
        if let Some(speaker) = diarization
            .speakers
            .get(speaker)
            .filter(|speaker| !speaker.embedding.is_empty())
        {
            labels.prints.push(MeetingVoiceprint {
                voice,
                embedding: speaker.embedding.clone(),
                known_id: candidate.and_then(|candidate| candidate.known_id.clone()),
            });
        }
    }
    (rows, labels)
}

/// A rerun may preserve identity/corrections only for an exact, unique physical source match.
/// Changed or split text keeps the old correction as review material, never as a remembered guess.
pub(crate) fn reconcile_rerun_utterances(
    before: &[crate::meeting_utterances::MeetingUtterance],
    after: &mut [crate::meeting_utterances::MeetingUtterance],
) {
    use crate::meetings::VOICES;
    fn source(value: &Option<String>) -> Option<&str> {
        value.as_deref().map(|source| {
            if source == "system" {
                "system_audio"
            } else {
                source
            }
        })
    }
    let mut used = HashSet::new();
    for row in after.iter_mut() {
        let candidates: Vec<_> = before
            .iter()
            .enumerate()
            .filter(|(_, old)| {
                old.metadata.source_id.is_some()
                    && source(&old.metadata.source_id) == source(&row.metadata.source_id)
                    && old.metadata.start_ms == row.metadata.start_ms
                    && old.metadata.end_ms == row.metadata.end_ms
                    && old.segment.text == row.segment.text
                    && old.segment.failed == row.segment.failed
            })
            .collect();
        if let [(index, old)] = candidates.as_slice()
            && !used.contains(index)
        {
            used.insert(*index);
            row.metadata.utterance_id = old.metadata.utterance_id.clone();
            row.metadata.source_id = old.metadata.source_id.clone();
            row.metadata.lineage = old.metadata.lineage.clone();
            row.metadata.recovery_audio = old.metadata.recovery_audio.clone();
            row.metadata.manual = old.metadata.manual.clone();
            row.metadata.unresolved_manual = old.metadata.unresolved_manual.clone();
            row.metadata.attribution_revision = row.metadata.attribution_revision.max(
                old.metadata.attribution_revision.saturating_add(u64::from(
                    row.metadata.attribution != old.metadata.attribution,
                )),
            );
            row.metadata.refresh_fingerprints(&row.segment);
            if let Some(manual) = row.metadata.manual.as_ref().filter(|manual| {
                !manual.review_required
                    && manual.source_fingerprint == row.metadata.source_fingerprint
            }) && let Some(voice) = manual.voice.filter(|voice| VOICES.contains(voice))
            {
                row.segment.speaker = Speaker::Others;
                row.segment.voice = Some(voice);
            }
        } else {
            for old in before.iter().filter(|old| {
                old.metadata.source_id.is_some()
                    && source(&old.metadata.source_id) == source(&row.metadata.source_id)
                    && old.metadata.end_ms.min(row.metadata.end_ms)
                        > old.metadata.start_ms.max(row.metadata.start_ms)
            }) {
                if !row.metadata.lineage.contains(&old.metadata.utterance_id) {
                    row.metadata.lineage.push(old.metadata.utterance_id.clone());
                }
            }
        }
    }
    let resolved: HashSet<_> = before
        .iter()
        .enumerate()
        .filter(|(index, _)| used.contains(index))
        .filter_map(|(_, row)| {
            row.metadata.manual.as_ref().filter(|manual| {
                !manual.review_required
                    && manual.source_fingerprint == row.metadata.source_fingerprint
            })
        })
        .map(|manual| manual.source_fingerprint.clone())
        .collect();
    for row in after.iter_mut() {
        row.metadata
            .unresolved_manual
            .retain(|manual| !resolved.contains(&manual.source_fingerprint));
    }
    for (_, old) in before
        .iter()
        .enumerate()
        .filter(|(index, _)| !used.contains(index))
    {
        let target = after
            .iter()
            .position(|row| {
                old.metadata.source_id.is_some()
                    && source(&old.metadata.source_id) == source(&row.metadata.source_id)
                    && old.metadata.end_ms.min(row.metadata.end_ms)
                        > old.metadata.start_ms.max(row.metadata.start_ms)
            })
            .or_else(|| (!after.is_empty()).then_some(0));
        if let Some(index) = target {
            for mut correction in old
                .metadata
                .manual
                .iter()
                .chain(old.metadata.unresolved_manual.iter())
                .cloned()
            {
                correction.review_required = true;
                if !after[index]
                    .metadata
                    .unresolved_manual
                    .contains(&correction)
                {
                    after[index].metadata.unresolved_manual.push(correction);
                }
            }
        }
    }
}

impl PennelEngine {
    pub fn meeting_summary_outdated(&self, id: &str) -> bool {
        self.meeting_store().summary_outdated(id)
    }

    pub fn meeting_retranscription_status(&self, id: &str) -> Option<MeetingRetranscriptionStatus> {
        self.retranscriptions.lock().unwrap().get(id).cloned()
    }

    pub fn meeting_retranscription_plan(&self, id: &str) -> MeetingRetranscriptionPlan {
        let recognition = MeetingRecognition::from_settings(&self.settings());
        let result = self
            .ensure_meeting_inactive(id)
            .and_then(|_| saved_tracks(&self.meeting_store(), id))
            .and_then(|_| FrozenLocal::new(self));
        MeetingRetranscriptionPlan {
            recognition,
            available: result.is_ok(),
            reason: result.err().map(|error| match error {
                PennelError::ModelNotDownloaded => {
                    "Download the selected local speech model first.".into()
                }
                error => error.to_string(),
            }),
        }
    }

    fn publish_retranscription(
        &self,
        id: &str,
        status: &MeetingRetranscriptionStatus,
        notify: &mut impl FnMut(MeetingRetranscriptionStatus),
    ) {
        self.retranscriptions
            .lock()
            .unwrap()
            .insert(id.to_owned(), status.clone());
        notify(status.clone());
    }

    pub fn retranscribe_meeting_audio(
        &self,
        id: &str,
        mut decode: impl FnMut(&Path) -> Result<Vec<f32>, PennelError>,
        mut notify: impl FnMut(MeetingRetranscriptionStatus),
    ) -> Result<u32, PennelError> {
        self.claim_library()?;
        let edit = self.edit_meeting(id)?;
        // A sync pass must not install another version while a replacement is being prepared.
        let sync = self.sync_gate.lock().unwrap();
        let frozen = FrozenLocal::new(self)?;
        let store = self.meeting_store();
        let tracks = saved_tracks(&store, id)?;
        let exclusions = store.dictation_intervals(id)?;
        let sources: Vec<_> = [
            "transcript.md",
            "transcript.meta.json",
            "meeting.md",
            "notes.md",
            ".pennel-dictation-intervals.json",
            "ik.m4a",
            "anderen.m4a",
            "audio/microphone.m4a",
            "audio/system.m4a",
            "audio/room.m4a",
            crate::meeting_recording::RECORDING_FILE,
        ]
        .into_iter()
        .map(|name| (name, store.file_stamp(id, name)))
        .collect();
        let before_snapshot = store.read_transcript_snapshot(id)?;
        let legacy_names = before_snapshot.metadata_status
            == crate::meeting_utterances::TranscriptMetadataStatus::Absent;
        let old_metadata = before_snapshot.metadata_digest;
        let mut before = before_snapshot.utterances;
        let old_names = store.speaker_names(id);
        if legacy_names {
            crate::diarize::preserve_manual_names(&mut before, &old_names);
        }
        let voices = self.voice_store();
        let key = store.uid_of(id).unwrap_or_else(|| id.to_owned());
        let old_hash = store.transcript_fingerprint(id)?;
        let attempt_started = chrono::Utc::now().to_rfc3339();
        let old_prints = voices.meeting_voiceprints(&key, Some(&old_hash));
        // Bind legacy caches before replacing text: after an interruption stale voice IDs must
        // never offer to remember a different person's voice from the new transcript.
        voices.set_meeting_voiceprints(&key, old_prints.clone(), Some(&old_hash))?;
        let mut status = MeetingRetranscriptionStatus {
            running: true,
            recognition: frozen.recognition.clone(),
            progress: RetranscriptionProgress {
                phase: RetranscriptionPhase::Decoding,
                speaker: None,
                completed: 0,
                total: 0,
            },
            error: None,
            warning: None,
        };
        self.publish_retranscription(id, &status, &mut notify);
        let mut result = (|| {
            let mut utterances = stage_audio_timed(
                &tracks,
                &exclusions,
                &mut decode,
                |samples| {
                    self.transcribe_complete_local_timed(
                        &frozen.settings,
                        &frozen.model_path,
                        samples,
                    )
                },
                |progress| {
                    status.progress = progress;
                    self.publish_retranscription(id, &status, &mut notify);
                },
            )?;
            if utterances.is_empty() {
                return Err(PennelError::NoSpeech);
            }
            reconcile_rerun_utterances(&before, &mut utterances);
            let mut labels = SpeakerLabels::default();
            if before.iter().any(|row| {
                row.segment.voice.is_some()
                    || row.metadata.manual.is_some()
                    || !row.metadata.unresolved_manual.is_empty()
            }) {
                let local_models = (
                    self.models.path(crate::models::SEGMENTATION_ID),
                    self.models.path(crate::models::EMBEDDING_ID),
                );
                let me = voices.my_voiceprint();
                if let (Some(segmentation), Some(embedding), true) =
                    (local_models.0, local_models.1, cfg!(feature = "speakers"))
                {
                    let mut candidates: Vec<_> = voices
                        .known_voiceprints()
                        .into_iter()
                        .map(|(known_id, name, embedding)| VoiceCandidate {
                            name: Some(name),
                            embedding,
                            known_id: Some(known_id),
                        })
                        .collect();
                    candidates.extend(old_prints.iter().map(|print| {
                        VoiceCandidate {
                            name: old_names
                                .iter()
                                .find(|name| name.voice == print.voice)
                                .map(|name| name.name.clone()),
                            embedding: print.embedding.clone(),
                            known_id: print.known_id.clone(),
                        }
                    }));
                    for (speaker, path) in &tracks {
                        status.progress = RetranscriptionProgress {
                            phase: RetranscriptionPhase::Speakers,
                            speaker: Some(*speaker),
                            completed: 0,
                            total: 1,
                        };
                        self.publish_retranscription(id, &status, &mut notify);
                        let mut samples = decode(path)?;
                        for &(start, end) in &exclusions {
                            let first = start.saturating_mul(16).min(samples.len() as u64) as usize;
                            let last = end.saturating_mul(16).min(samples.len() as u64) as usize;
                            if last > first {
                                samples[first..last].fill(0.0);
                            }
                        }
                        let diarization =
                            crate::speakers::diarize_local(&segmentation, &embedding, &samples)?;
                        drop(samples);
                        let track = if path.file_name().is_some_and(|name| name == "room.m4a") {
                            crate::diarize::Track::Microphone
                        } else {
                            crate::diarize::Track::from(*speaker)
                        };
                        let (updated, mut applied) = relabel_utterances(
                            &utterances,
                            &diarization,
                            track,
                            me.as_deref()
                                .filter(|_| track == crate::diarize::Track::Microphone),
                            &candidates,
                        );
                        utterances = updated;
                        labels.names.append(&mut applied.names);
                        labels.prints.append(&mut applied.prints);
                    }
                } else {
                    status.warning=Some("The text has been transcribed again. Download local speaker models to rebuild the speaker labels.".into());
                }
            }
            reconcile_rerun_utterances(&before, &mut utterances);
            let recognized: Vec<_> = labels
                .names
                .iter()
                .map(|name| (name.voice, name.name.clone()))
                .collect();
            labels.names = crate::diarize::carry_utterance_names(
                &before,
                &utterances,
                &old_names,
                &recognized,
            );
            if sources
                .iter()
                .any(|(name, stamp)| store.file_stamp(id, name) != *stamp)
                || store.read_transcript_snapshot(id)?.metadata_digest != old_metadata
            {
                return Err(PennelError::storage(
                    "The recording or meeting changed during processing. The transcript has not been replaced.",
                ));
            }
            status.progress = RetranscriptionProgress {
                phase: RetranscriptionPhase::Saving,
                speaker: None,
                completed: 0,
                total: 1,
            };
            self.publish_retranscription(id, &status, &mut notify);
            let sources: Vec<_> = tracks.iter().map(|(speaker, _)| *speaker).collect();
            let backup =
                store.replace_retranscribed_utterances(id, &utterances, &labels.names, &sources)?;
            let finish = (|| {
                voices.set_meeting_voiceprints(
                    &key,
                    labels.prints,
                    Some(&store.transcript_fingerprint(id)?),
                )?;
                store.append_transcription_attempt(
                    id,
                    crate::meeting_recording::transcription_attempt(
                        self,
                        &frozen.recognition,
                        attempt_started.clone(),
                        true,
                        Some(store.transcript_fingerprint(id)?),
                    )?,
                )?;
                store.finish_retranscription(id)
            })();
            if let Err(error) = finish {
                // Restore the authoritative files before reporting a failed optional cache write.
                store.restore_retranscription(id, &backup)?;
                voices.set_meeting_voiceprints(&key, old_prints.clone(), Some(&old_hash))?;
                return Err(error);
            }
            Ok(utterances.len() as u32)
        })();
        if result.is_err() {
            match store.recording_metadata(id) {
                Ok(Some(_)) => {
                    let attempt = crate::meeting_recording::transcription_attempt(
                        self,
                        &frozen.recognition,
                        attempt_started,
                        false,
                        None,
                    )
                    .and_then(|attempt| store.append_transcription_attempt(id, attempt));
                    if let Err(error) = attempt {
                        result = Err(error);
                    }
                }
                Err(error) => result = Err(error),
                Ok(None) => {}
            }
        }
        status.running = false;
        status.progress = RetranscriptionProgress {
            phase: if result.is_ok() {
                RetranscriptionPhase::Completed
            } else {
                RetranscriptionPhase::Failed
            },
            speaker: None,
            completed: 1,
            total: 1,
        };
        status.error = result.as_ref().err().map(ToString::to_string);
        self.publish_retranscription(id, &status, &mut notify);
        drop(sync);
        drop(edit);
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum RetranscriptionPhase {
    Decoding,
    Transcribing,
    Speakers,
    Saving,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
pub struct RetranscriptionProgress {
    pub phase: RetranscriptionPhase,
    pub speaker: Option<Speaker>,
    pub completed: u64,
    pub total: u64,
}

pub(crate) fn saved_tracks(
    store: &MeetingStore,
    id: &str,
) -> Result<Vec<(Speaker, PathBuf)>, PennelError> {
    let mut components = Path::new(id).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || id.contains('\\')
    {
        return Err(PennelError::NotFound);
    }
    store.info(id).ok_or(PennelError::NotFound)?;
    let before = store.read_transcript(id);
    let mut tracks = Vec::new();
    for track in store.audio_tracks(id)? {
        if track.available {
            tracks.push((track.speaker, store.audio_path(id, track.speaker)?));
        }
    }
    if tracks.is_empty() {
        return Err(PennelError::InvalidResponse {
            message: "There is no saved audio to transcribe again.".into(),
        });
    }
    let required = match store.transcript_tracks(id)? {
        Some(required) => required,
        None if before.iter().any(|segment| segment.voice.is_some()) => {
            if tracks.len() != 2 {
                return Err(PennelError::InvalidResponse {
                    message: "The original recording tracks could not be determined. Both tracks are required for older speaker-labeled transcripts."
                        .into(),
                });
            }
            vec![Speaker::Me, Speaker::Others]
        }
        None => before.iter().map(|segment| segment.speaker).collect(),
    };
    if required
        .iter()
        .any(|required| !tracks.iter().any(|(speaker, _)| speaker == required))
    {
        return Err(PennelError::InvalidResponse {
            message: "Some original audio is missing. The transcript has not been changed.".into(),
        });
    }
    Ok(tracks)
}

#[cfg(test)]
pub(crate) fn stage_audio(
    tracks: &[(Speaker, PathBuf)],
    exclusions: &[(u64, u64)],
    decode: impl FnMut(&Path) -> Result<Vec<f32>, PennelError>,
    mut transcribe: impl FnMut(&[f32]) -> Result<String, PennelError>,
    progress: impl FnMut(RetranscriptionProgress),
) -> Result<Vec<Segment>, PennelError> {
    stage_audio_timed(
        tracks,
        exclusions,
        decode,
        |samples| transcribe(samples).map(crate::SpeechTranscript::plain),
        progress,
    )
    .map(|rows| rows.into_iter().map(|row| row.segment).collect())
}

pub(crate) fn stage_audio_timed(
    tracks: &[(Speaker, PathBuf)],
    exclusions: &[(u64, u64)],
    mut decode: impl FnMut(&Path) -> Result<Vec<f32>, PennelError>,
    mut transcribe: impl FnMut(&[f32]) -> Result<crate::SpeechTranscript, PennelError>,
    mut progress: impl FnMut(RetranscriptionProgress),
) -> Result<Vec<crate::meeting_utterances::MeetingUtterance>, PennelError> {
    let mut segments = Vec::new();
    for (speaker, path) in tracks {
        progress(RetranscriptionProgress {
            phase: RetranscriptionPhase::Decoding,
            speaker: Some(*speaker),
            completed: 0,
            total: 0,
        });
        let mut samples = decode(path)?;
        if samples.is_empty() || samples.iter().any(|sample| !sample.is_finite()) {
            return Err(PennelError::InvalidResponse {
                message: "The saved audio could not be read. The transcript has not been changed."
                    .into(),
            });
        }
        for &(start, end) in exclusions {
            let first = start.saturating_mul(16).min(samples.len() as u64) as usize;
            let last = end.saturating_mul(16).min(samples.len() as u64) as usize;
            if last > first {
                samples[first..last].fill(0.0);
            }
        }
        let total = samples.len() as u64 / 16;
        progress(RetranscriptionProgress {
            phase: RetranscriptionPhase::Transcribing,
            speaker: Some(*speaker),
            completed: 0,
            total,
        });
        let mut chunker = Chunker::new(16_000);
        let mut recognize = |chunk: crate::chunker::Chunk| -> Result<(), PennelError> {
            let end_ms = chunk.start_ms + chunk.samples.len() as u64 / 16;
            let transcript = transcribe(&chunk.samples)?;
            let transcript = crate::SpeechTranscript::validated(
                transcript.text,
                transcript.spans,
                transcript.timing_kind,
                end_ms - chunk.start_ms,
            )
            .trimmed();
            if !is_hallucination(&transcript.text) {
                let source = if path.file_name().is_some_and(|name| name == "room.m4a") {
                    "room_microphone"
                } else if *speaker == Speaker::Me {
                    "microphone"
                } else {
                    "system_audio"
                };
                let mut row = crate::meeting_utterances::MeetingUtterance::new(
                    Segment {
                        start_ms: chunk.start_ms,
                        end_ms,
                        speaker: *speaker,
                        text: transcript.text,
                        failed: false,
                        voice: None,
                    },
                    Some(source.into()),
                );
                row.metadata.spans = transcript
                    .spans
                    .into_iter()
                    .map(|span| {
                        let start = chunk.start_ms.checked_add(span.start_ms);
                        let end = chunk.start_ms.checked_add(span.end_ms);
                        match (start, end) {
                            (Some(start), Some(end)) => {
                                Ok(crate::meeting_utterances::TimedTextSpan(
                                    span.start_byte,
                                    span.end_byte,
                                    start,
                                    end,
                                ))
                            }
                            _ => Err(PennelError::InvalidResponse {
                                message: "The transcription timing is outside the recording."
                                    .into(),
                            }),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                segments.push(row);
            }
            progress(RetranscriptionProgress {
                phase: RetranscriptionPhase::Transcribing,
                speaker: Some(*speaker),
                completed: end_ms,
                total,
            });
            Ok(())
        };
        for packet in samples.chunks(16_000) {
            for chunk in chunker.push(packet) {
                recognize(chunk)?;
            }
        }
        if let Some(chunk) = chunker.flush() {
            recognize(chunk)?;
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::{MeetingStore, Segment, Speaker, SpeakerName};
    use std::cell::RefCell;

    fn segment(speaker: Speaker, text: &str) -> Segment {
        Segment {
            start_ms: 0,
            end_ms: 1_000,
            speaker,
            text: text.into(),
            failed: false,
            voice: None,
        }
    }

    #[test]
    fn timed_retranscription_retains_word_bounds_and_adds_chunk_offset_once() {
        use crate::{SpeechTextSpan, SpeechTimingKind, SpeechTranscript};
        let tracks = vec![(Speaker::Others, PathBuf::from("audio/room.m4a"))];
        let rows = stage_audio_timed(
            &tracks,
            &[],
            |_| {
                Ok((0..31 * 16_000)
                    .map(|sample| (sample as f32 * 0.05).sin() * 0.2)
                    .collect())
            },
            |_| {
                Ok(SpeechTranscript::validated(
                    "Ja".into(),
                    vec![SpeechTextSpan {
                        start_byte: 0,
                        end_byte: 2,
                        start_ms: 0,
                        end_ms: 500,
                    }],
                    SpeechTimingKind::Word,
                    30_000,
                ))
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.metadata.source_id.as_deref() == Some("room_microphone"))
        );
        assert_eq!(
            rows[0].metadata.spans,
            [crate::meeting_utterances::TimedTextSpan(0, 2, 0, 500)]
        );
        assert_eq!(
            rows[1].metadata.spans,
            [crate::meeting_utterances::TimedTextSpan(
                0, 2, 30_000, 30_500
            )]
        );
        assert_eq!(rows[1].segment.end_ms, 31_000);
    }

    #[test]
    fn invalid_retranscription_timing_preserves_every_authoritative_word() {
        use crate::{SpeechTextSpan, SpeechTimingKind, SpeechTranscript};
        let tracks = vec![(Speaker::Me, PathBuf::from("audio/microphone.m4a"))];
        let rows = stage_audio_timed(
            &tracks,
            &[],
            |_| {
                Ok((0..2 * 16_000)
                    .map(|sample| (sample as f32 * 0.05).sin() * 0.2)
                    .collect())
            },
            |_| {
                Ok(SpeechTranscript {
                    text: "All spoken words stay.".into(),
                    spans: vec![SpeechTextSpan {
                        start_byte: 0,
                        end_byte: 500,
                        start_ms: 0,
                        end_ms: 1_000,
                    }],
                    timing_kind: SpeechTimingKind::Word,
                })
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(rows[0].segment.text, "All spoken words stay.");
        assert!(rows[0].metadata.spans.is_empty());
    }

    #[test]
    fn exact_rerun_keeps_manual_identity_and_source_alias_without_remembered_guess() {
        use crate::meeting_utterances::{ManualCorrection, MeetingUtterance};
        let mut old = MeetingUtterance::new(
            segment(Speaker::Others, "Exact source."),
            Some("system".into()),
        );
        old.metadata.manual = Some(ManualCorrection {
            name: Some("Manual".into()),
            voice: Some(4),
            source_fingerprint: old.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        let mut new = vec![MeetingUtterance::new(
            old.segment.clone(),
            Some("system_audio".into()),
        )];
        new[0].segment.voice = Some(2);
        reconcile_rerun_utterances(&[old.clone()], &mut new);
        assert_eq!(new[0].metadata.utterance_id, old.metadata.utterance_id);
        assert_eq!(new[0].metadata.source_id, old.metadata.source_id);
        assert_eq!(new[0].segment.voice, Some(4));
        assert_eq!(new[0].metadata.manual, old.metadata.manual);
    }

    #[test]
    fn changed_rerun_keeps_original_manual_correction_for_review() {
        use crate::meeting_utterances::{ManualCorrection, MeetingUtterance};
        let mut old = MeetingUtterance::new(
            segment(Speaker::Others, "Original words."),
            Some("system_audio".into()),
        );
        old.metadata.manual = Some(ManualCorrection {
            name: Some("Retained correction".into()),
            voice: Some(4),
            source_fingerprint: old.metadata.source_fingerprint.clone(),
            review_required: false,
        });
        let mut new = vec![MeetingUtterance::new(
            segment(Speaker::Others, "Changed words."),
            Some("system_audio".into()),
        )];
        reconcile_rerun_utterances(&[old.clone()], &mut new);
        assert_ne!(new[0].metadata.utterance_id, old.metadata.utterance_id);
        assert_eq!(new[0].metadata.lineage, [old.metadata.utterance_id]);
        assert!(new[0].metadata.manual.is_none());
        let pending = &new[0].metadata.unresolved_manual[0];
        assert_eq!(pending.name.as_deref(), Some("Retained correction"));
        assert!(pending.review_required);
        assert_eq!(pending.source_fingerprint, old.metadata.source_fingerprint);
    }

    #[test]
    fn persisted_rerun_preserves_revision_after_repeated_manual_naming() {
        use crate::meeting_utterances::{AttributionKind, AutomaticAttribution, MeetingUtterance};
        let (_directory, store, id) = meeting();
        let mut row = MeetingUtterance::new(
            segment(Speaker::Others, "Same exact spoken text."),
            Some("system_audio".into()),
        );
        row.segment.voice = Some(4);
        row.metadata.attribution = AutomaticAttribution {
            kind: AttributionKind::Single,
            voices: vec![4],
            ..AutomaticAttribution::default()
        };
        row.metadata.attribution_revision = 7;
        store.write_transcript_snapshot(&id, None, &[row]).unwrap();
        store
            .rename_speaker(&id, 4, "First confirmed name")
            .unwrap();
        store
            .rename_speaker(&id, 4, "Edited confirmed name")
            .unwrap();
        let before = store.read_transcript_snapshot(&id).unwrap();
        let mut after = vec![MeetingUtterance::new(
            segment(Speaker::Others, "Same exact spoken text."),
            Some("system_audio".into()),
        )];
        reconcile_rerun_utterances(&before.utterances, &mut after);
        assert!(
            after[0].metadata.attribution_revision
                >= before.utterances[0].metadata.attribution_revision
        );
        let names = crate::diarize::carry_utterance_names(
            &before.utterances,
            &after,
            &store.speaker_names(&id),
            &[],
        );
        store
            .replace_retranscribed_utterances(&id, &after, &names, &[Speaker::Others])
            .unwrap();
        store.finish_retranscription(&id).unwrap();
        let reopened = store.read_transcript_snapshot(&id).unwrap();
        assert_eq!(
            reopened.utterances[0].metadata.utterance_id,
            before.utterances[0].metadata.utterance_id
        );
        assert!(
            reopened.utterances[0].metadata.attribution_revision
                >= before.utterances[0].metadata.attribution_revision
        );
        assert_eq!(
            reopened.utterances[0]
                .metadata
                .manual
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("Edited confirmed name")
        );
    }

    fn meeting() -> (tempfile::TempDir, MeetingStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store.create("Herkenning", chrono::Local::now()).unwrap();
        store
            .write_transcript(&id, &[segment(Speaker::Me, "oorspronkelijke tekst")])
            .unwrap();
        store.write_notes(&id, "mijn eigen notities").unwrap();
        store
            .write_meeting(&id, 1, "algemeen", "bestaande samenvatting")
            .unwrap();
        (dir, store, id)
    }

    #[test]
    fn archived_tracks_keep_silence_offsets_and_are_processed_sequentially() {
        let trace = RefCell::new(Vec::new());
        let tracks = [
            (Speaker::Me, std::path::PathBuf::from("ik.m4a")),
            (Speaker::Others, std::path::PathBuf::from("anderen.m4a")),
        ];
        let segments = stage_audio(
            &tracks,
            &[],
            |path| {
                trace.borrow_mut().push(path.to_string_lossy().into_owned());
                Ok(if path.ends_with("ik.m4a") {
                    [vec![0.0; 15 * 16_000], vec![0.1; 20 * 16_000]].concat()
                } else {
                    vec![0.2; 33 * 16_000]
                })
            },
            |samples| {
                trace
                    .borrow_mut()
                    .push(format!("samples:{}", samples.len()));
                Ok("Nederlandse zin".into())
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(
            *trace.borrow(),
            [
                "ik.m4a",
                "samples:320000",
                "anderen.m4a",
                "samples:480000",
                "samples:48000"
            ]
        );
        assert_eq!(
            segments
                .iter()
                .map(|s| (s.speaker, s.start_ms, s.end_ms))
                .collect::<Vec<_>>(),
            [
                (Speaker::Me, 15_000, 35_000),
                (Speaker::Others, 0, 30_000),
                (Speaker::Others, 30_000, 33_000)
            ]
        );
    }

    #[test]
    fn audio_and_recognizer_failures_never_write_a_partial_transcript() {
        let (_dir, store, id) = meeting();
        let before = std::fs::read(store.dir_of(&id).join("transcript.md")).unwrap();
        let tracks = [
            (Speaker::Me, std::path::PathBuf::from("ik.m4a")),
            (Speaker::Others, std::path::PathBuf::from("anderen.m4a")),
        ];
        for failed_decode in [true, false] {
            let mut calls = 0;
            let result = stage_audio(
                &tracks,
                &[],
                |path| {
                    if failed_decode && path.ends_with("anderen.m4a") {
                        Err(crate::error::PennelError::NotFound)
                    } else {
                        Ok(vec![0.2; 16_000])
                    }
                },
                |_| {
                    calls += 1;
                    if !failed_decode && calls == 2 {
                        Err(crate::error::PennelError::ModelNotDownloaded)
                    } else {
                        Ok("nieuwe tekst".into())
                    }
                },
                |_| {},
            );
            assert!(result.is_err());
            assert_eq!(
                std::fs::read(store.dir_of(&id).join("transcript.md")).unwrap(),
                before
            );
        }
    }

    #[test]
    fn required_tracks_cannot_silently_disappear_or_escape_the_meeting() {
        let (_dir, store, id) = meeting();
        let root = store.dir_of(&id);
        std::fs::write(root.join("ik.m4a"), b"recorded audio").unwrap();
        assert_eq!(saved_tracks(&store, &id).unwrap().len(), 1);
        store
            .write_transcript(
                &id,
                &[
                    segment(Speaker::Me, "ik"),
                    segment(Speaker::Others, "anderen"),
                ],
            )
            .unwrap();
        assert!(saved_tracks(&store, &id).is_err());
        std::fs::write(root.join("anderen.m4a"), b"recorded audio").unwrap();
        assert_eq!(saved_tracks(&store, &id).unwrap().len(), 2);
        assert!(saved_tracks(&store, "../other").is_err());
    }

    #[test]
    fn speaker_labels_do_not_change_the_required_recording_tracks() {
        let (_dir, store, id) = meeting();
        let root = store.dir_of(&id);
        std::fs::write(root.join("ik.m4a"), b"microphone audio").unwrap();
        store.preserve_transcript_tracks(&id).unwrap();
        let mut other = segment(Speaker::Others, "iemand op dezelfde microfoon");
        other.voice = Some(1);
        store
            .write_transcript(&id, &[segment(Speaker::Me, "ik"), other.clone()])
            .unwrap();
        assert_eq!(
            saved_tracks(&store, &id).unwrap(),
            [(Speaker::Me, root.join("ik.m4a").canonicalize().unwrap())]
        );

        let (_dir, store, id) = meeting();
        let root = store.dir_of(&id);
        store
            .write_transcript(
                &id,
                &[
                    segment(Speaker::Me, "ik"),
                    segment(Speaker::Others, "systeemaudio"),
                ],
            )
            .unwrap();
        store.preserve_transcript_tracks(&id).unwrap();
        store
            .write_transcript(&id, &[segment(Speaker::Me, "ik"), other])
            .unwrap();
        std::fs::write(root.join("ik.m4a"), b"microphone audio").unwrap();
        assert!(saved_tracks(&store, &id).is_err());
        std::fs::write(root.join("anderen.m4a"), b"system audio").unwrap();
        assert_eq!(saved_tracks(&store, &id).unwrap().len(), 2);
    }

    #[test]
    fn legacy_voice_labels_do_not_prove_a_missing_track_is_optional() {
        let (_dir, store, id) = meeting();
        let root = store.dir_of(&id);
        let mut voiced = segment(Speaker::Others, "onbekende fysieke herkomst");
        voiced.voice = Some(1);
        store.write_transcript(&id, &[voiced]).unwrap();
        std::fs::write(root.join("ik.m4a"), b"microphone").unwrap();
        assert!(saved_tracks(&store, &id).is_err());
        std::fs::write(root.join("anderen.m4a"), b"system").unwrap();
        assert_eq!(saved_tracks(&store, &id).unwrap().len(), 2);
    }

    #[test]
    fn successful_replacement_keeps_a_backup_notes_and_summary_provenance() {
        let (_dir, store, id) = meeting();
        store
            .set_speaker_names(
                &id,
                &[SpeakerName {
                    voice: 1,
                    name: "Oude naam".into(),
                }],
            )
            .unwrap();
        let root = store.dir_of(&id);
        let original = std::fs::read(root.join("transcript.md")).unwrap();
        let metadata = std::fs::read(root.join("meeting.md")).unwrap();
        let backup = store
            .replace_retranscribed(
                &id,
                &[segment(Speaker::Me, "nieuwe tekst")],
                &[],
                &[Speaker::Me],
            )
            .unwrap();
        store.finish_retranscription(&id).unwrap();
        store.recover_retranscriptions().unwrap();
        assert_eq!(
            std::fs::read(backup.join("transcript.md")).unwrap(),
            original
        );
        assert_eq!(std::fs::read(backup.join("meeting.md")).unwrap(), metadata);
        assert_eq!(store.read_notes(&id), "mijn eigen notities");
        assert_eq!(store.read_transcript(&id)[0].text, "nieuwe tekst");
        assert!(store.speaker_names(&id).is_empty());
        assert!(store.summary_outdated(&id));
        assert_eq!(
            store.read_meeting(&id).as_deref(),
            Some("bestaande samenvatting")
        );
        store
            .write_meeting(&id, 1, "algemeen", "nieuwe samenvatting")
            .unwrap();
        assert!(!store.summary_outdated(&id));
    }

    #[test]
    fn backup_failure_leaves_all_existing_files_unchanged() {
        let (_dir, store, id) = meeting();
        let root = store.dir_of(&id);
        let original = std::fs::read(root.join("transcript.md")).unwrap();
        let metadata = std::fs::read(root.join("meeting.md")).unwrap();
        std::fs::write(root.join(".pennel-transcript-backups"), b"blocked").unwrap();
        assert!(
            store
                .replace_retranscribed(&id, &[segment(Speaker::Me, "new")], &[], &[Speaker::Me])
                .is_err()
        );
        assert_eq!(std::fs::read(root.join("transcript.md")).unwrap(), original);
        assert_eq!(std::fs::read(root.join("meeting.md")).unwrap(), metadata);
    }

    #[test]
    fn startup_recovers_each_interrupted_replacement_boundary() {
        for boundary in 0..4 {
            let (_dir, store, id) = meeting();
            let root = store.dir_of(&id);
            store
                .rename_speaker(&id, 1, "Oorspronkelijke naam")
                .unwrap();
            store
                .write_recovery(
                    &id,
                    Speaker::Others,
                    0,
                    &crate::wav::encode_wav(&vec![0.1; 16_000], 16_000),
                )
                .unwrap();
            let original = std::fs::read(root.join("transcript.md")).unwrap();
            let metadata = std::fs::read(root.join("meeting.md")).unwrap();
            let backup_id = uuid::Uuid::now_v7().to_string();
            let backup = root.join(".pennel-transcript-backups").join(&backup_id);
            std::fs::create_dir_all(&backup).unwrap();
            std::fs::write(backup.join("transcript.md"), &original).unwrap();
            std::fs::write(backup.join("meeting.md"), &metadata).unwrap();
            let journal = serde_json::json!({"version":1,"backup":backup_id,"had_failed":true});
            std::fs::write(
                root.join(".pennel-retranscription-pending.json"),
                journal.to_string(),
            )
            .unwrap();
            if boundary >= 1 {
                std::fs::rename(root.join("failed"), backup.join("failed")).unwrap();
            }
            if boundary >= 2 {
                store
                    .set_speaker_names(
                        &id,
                        &[SpeakerName {
                            voice: 1,
                            name: "Verkeerde nieuwe naam".into(),
                        }],
                    )
                    .unwrap();
            }
            if boundary >= 3 {
                let replacement = [segment(Speaker::Me, "nieuwe tekst")];
                assert!(store.write_transcript(&id, &replacement).is_err());
                // Simulate the interrupted internal install, bypassing the public save barrier.
                std::fs::write(
                    root.join("transcript.md"),
                    crate::meetings::render_transcript(&replacement),
                )
                .unwrap();
            }
            let reopened = MeetingStore::new(&store.root());
            reopened.recover_retranscriptions().unwrap();
            reopened.recover_retranscriptions().unwrap();
            assert_eq!(
                std::fs::read(root.join("transcript.md")).unwrap(),
                original,
                "boundary {boundary}"
            );
            assert_eq!(
                std::fs::read(root.join("meeting.md")).unwrap(),
                metadata,
                "boundary {boundary}"
            );
            assert!(root.join("failed/others-0.wav").is_file());
            assert!(!root.join(".pennel-retranscription-pending.json").exists());
            assert_eq!(reopened.read_notes(&id), "mijn eigen notities");
        }
    }

    #[test]
    fn interrupted_recovery_remains_retryable_and_cannot_escape_the_backup_folder() {
        let (_dir, store, id) = meeting();
        let root = store.dir_of(&id);
        let original = std::fs::read(root.join("transcript.md")).unwrap();
        let metadata = std::fs::read(root.join("meeting.md")).unwrap();
        let backup = store
            .replace_retranscribed(&id, &[segment(Speaker::Me, "new")], &[], &[Speaker::Me])
            .unwrap();
        std::fs::remove_file(root.join("meeting.md")).unwrap();
        std::fs::create_dir(root.join("meeting.md")).unwrap();
        assert!(store.recover_retranscriptions().is_err());
        assert!(root.join(".pennel-retranscription-pending.json").exists());
        std::fs::remove_dir(root.join("meeting.md")).unwrap();
        store.recover_retranscriptions().unwrap();
        assert_eq!(std::fs::read(root.join("transcript.md")).unwrap(), original);
        assert_eq!(std::fs::read(root.join("meeting.md")).unwrap(), metadata);
        assert!(backup.is_dir());

        std::fs::write(
            root.join(".pennel-retranscription-pending.json"),
            r#"{"version":1,"backup":"../../outside","had_failed":false}"#,
        )
        .unwrap();
        assert!(store.recover_retranscriptions().is_err());
        assert_eq!(std::fs::read(root.join("meeting.md")).unwrap(), metadata);
    }

    #[test]
    fn reopening_the_engine_restores_an_uncommitted_transcript_before_reads() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data").to_string_lossy().into_owned();
        let engine = PennelEngine::new_preview(data.clone()).unwrap();
        let store = engine.meeting_store();
        let id = store.create("Herstart", chrono::Local::now()).unwrap();
        store
            .write_transcript(&id, &[segment(Speaker::Me, "origineel")])
            .unwrap();
        store
            .rename_speaker(&id, 1, "Oorspronkelijke naam")
            .unwrap();
        let backup = store
            .replace_retranscribed(&id, &[segment(Speaker::Me, "nieuw")], &[], &[Speaker::Me])
            .unwrap();
        drop(engine);
        let reopened = PennelEngine::new_preview(data).unwrap();
        assert_eq!(reopened.meeting_transcript(id.clone())[0].text, "origineel");
        assert_eq!(
            reopened.meeting_speakers(id)[0].name,
            "Oorspronkelijke naam"
        );
        assert!(backup.is_dir());
    }

    #[test]
    fn known_dictation_intervals_are_excluded_from_both_archived_tracks() {
        let tracks = [
            (Speaker::Me, std::path::PathBuf::from("ik.m4a")),
            (Speaker::Others, std::path::PathBuf::from("anderen.m4a")),
        ];
        let result = stage_audio(
            &tracks,
            &[(0, 20_000)],
            |_| Ok(vec![0.2; 20 * 16_000]),
            |_| panic!("excluded audio must not reach recognition"),
            |_| {},
        )
        .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    #[cfg(feature = "local-models")]
    fn local_configuration_is_frozen_and_remote_recognition_is_rejected() {
        use crate::{PennelEngine, ProviderAccount, ProviderKind};
        let dir = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(dir.path().join("data").to_string_lossy().into_owned())
                .unwrap();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "test-local".into(),
            kind: ProviderKind::Local,
            name: "Lokaal".into(),
            base_url: String::new(),
        });
        settings.meeting_stt.provider_id = "test-local".into();
        settings.meeting_stt.model = "parakeet-tdt-0.6b-v3".into();
        settings.language = Some("nl".into());
        settings.dictionary = vec!["voorbeeldterm".into()];
        engine.update_settings(settings.clone()).unwrap();
        assert!(FrozenLocal::new(&engine).is_err());
        std::fs::create_dir_all(dir.path().join("data/models")).unwrap();
        std::fs::write(
            dir.path().join("data/models/parakeet-tdt-0.6b-v3.gguf"),
            b"model placeholder; inference is not used in this test",
        )
        .unwrap();
        let frozen = FrozenLocal::new(&engine).unwrap();
        settings.language = Some("en".into());
        settings.dictionary.clear();
        settings.meeting_stt.provider_id = "groq".into();
        engine.update_settings(settings).unwrap();
        assert_eq!(frozen.settings.language.as_deref(), Some("nl"));
        assert_eq!(frozen.settings.dictionary, ["voorbeeldterm"]);
        assert_eq!(frozen.recognition.model, "parakeet-tdt-0.6b-v3");
        assert!(FrozenLocal::new(&engine).is_err());
    }

    #[test]
    fn an_in_flight_meeting_edit_blocks_conflicting_writes_but_not_reads() {
        use crate::PennelEngine;
        let dir = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(dir.path().join("data").to_string_lossy().into_owned())
                .unwrap();
        let store = engine.meeting_store();
        let id = store.create("Verwerken", chrono::Local::now()).unwrap();
        let guard = engine.edit_meeting(&id).unwrap();
        assert!(engine.edit_meeting(&id).is_err());
        assert!(
            engine
                .rename_meeting(id.clone(), "andere naam".into())
                .is_err()
        );
        assert!(engine.delete_meeting(id.clone()).is_err());
        assert!(
            engine
                .set_meeting_notes(id.clone(), "onverwachte wijziging".into())
                .is_err()
        );
        assert!(engine.rename_speaker(id.clone(), 1, "Naam".into()).is_err());
        assert!(engine.forget_all_voices().is_err());
        assert!(engine.meeting_info(id.clone()).is_some());
        drop(guard);
        engine
            .set_meeting_notes(id.clone(), "bewaarde notities".into())
            .unwrap();
        assert_eq!(engine.meeting_notes(id), "bewaarde notities");
    }

    #[test]
    fn editing_a_finished_meeting_does_not_prevent_an_unrelated_recording() {
        let dir = tempfile::tempdir().unwrap();
        let engine = crate::PennelEngine::new_preview(
            dir.path().join("data").to_string_lossy().into_owned(),
        )
        .unwrap();
        let id = engine
            .meeting_store()
            .create("Eerdere meeting", chrono::Local::now())
            .unwrap();
        let _edit = engine.edit_meeting(&id).unwrap();
        let session = engine
            .clone()
            .start_meeting(
                "Nieuwe opname".into(),
                crate::ApiKeys {
                    by_provider: std::collections::HashMap::new(),
                },
            )
            .unwrap();
        session.stop().unwrap();
        assert_ne!(session.id(), id);
    }

    #[test]
    fn dictation_exclusions_roundtrip_and_corruption_is_not_silently_ignored() {
        let (_dir, store, id) = meeting();
        assert!(store.dictation_intervals(&id).unwrap().is_empty());
        store
            .write_dictation_intervals(&id, &[(200, 1_200)])
            .unwrap();
        assert_eq!(store.dictation_intervals(&id).unwrap(), [(200, 1_200)]);
        std::fs::write(
            store.dir_of(&id).join(".pennel-dictation-intervals.json"),
            b"broken metadata",
        )
        .unwrap();
        assert!(store.dictation_intervals(&id).is_err());
    }

    #[test]
    #[cfg(feature = "local-models")]
    fn decoder_failure_clears_running_state_and_preserves_the_original_meeting() {
        use crate::{PennelEngine, ProviderAccount, ProviderKind};
        let dir = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(dir.path().join("data").to_string_lossy().into_owned())
                .unwrap();
        let store = engine.meeting_store();
        let id = store.create("Opname", chrono::Local::now()).unwrap();
        store
            .write_transcript(&id, &[segment(Speaker::Me, "oude zin")])
            .unwrap();
        std::fs::write(store.dir_of(&id).join("ik.m4a"), b"audio placeholder").unwrap();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "test-local".into(),
            kind: ProviderKind::Local,
            name: "Lokaal".into(),
            base_url: String::new(),
        });
        settings.meeting_stt.provider_id = "test-local".into();
        settings.meeting_stt.model = "parakeet-tdt-0.6b-v3".into();
        engine.update_settings(settings).unwrap();
        assert!(!engine.meeting_retranscription_plan(&id).available);
        std::fs::create_dir_all(dir.path().join("data/models")).unwrap();
        std::fs::write(
            dir.path().join("data/models/parakeet-tdt-0.6b-v3.gguf"),
            b"placeholder",
        )
        .unwrap();
        assert!(engine.meeting_retranscription_plan(&id).available);
        let before = std::fs::read(store.dir_of(&id).join("transcript.md")).unwrap();
        let result = engine.retranscribe_meeting_audio(
            &id,
            |_| Err(PennelError::storage("decoder failed")),
            |status| {
                if !status.running {
                    assert!(
                        engine.edit_meeting(&id).is_err(),
                        "publish completion before another job can start"
                    );
                }
            },
        );
        assert!(result.is_err());
        let status = engine.meeting_retranscription_status(&id).unwrap();
        assert!(!status.running);
        assert_eq!(status.progress.phase, RetranscriptionPhase::Failed);
        assert!(status.error.as_deref().unwrap().contains("decoder failed"));
        assert_eq!(
            std::fs::read(store.dir_of(&id).join("transcript.md")).unwrap(),
            before
        );
        engine
            .set_meeting_notes(id, "nog steeds bewerkbaar".into())
            .unwrap();
    }

    #[test]
    fn fresh_speaker_numbers_never_inherit_an_unmatched_old_name() {
        let mut segments = vec![
            segment(Speaker::Others, "eerste stem"),
            Segment {
                start_ms: 4_000,
                end_ms: 6_000,
                ..segment(Speaker::Others, "andere stem")
            },
        ];
        let diarization = pennel_speakers::Diarization {
            turns: vec![
                pennel_speakers::Turn {
                    start_ms: 0,
                    end_ms: 2_000,
                    speaker: 0,
                },
                pennel_speakers::Turn {
                    start_ms: 4_000,
                    end_ms: 6_000,
                    speaker: 1,
                },
            ],
            speakers: vec![
                pennel_speakers::Speaker {
                    speaker: 0,
                    speech_ms: 20_000,
                    embedding: vec![1.0, 0.0],
                },
                pennel_speakers::Speaker {
                    speaker: 1,
                    speech_ms: 20_000,
                    embedding: vec![-1.0, 0.0],
                },
            ],
        };
        let candidates = vec![
            VoiceCandidate {
                name: Some("Onjuiste oude naam".into()),
                embedding: vec![0.0, 1.0],
                known_id: None,
            },
            VoiceCandidate {
                name: Some("Betrouwbare naam".into()),
                embedding: vec![1.0, 0.0],
                known_id: Some("known".into()),
            },
        ];
        let labels = relabel_speakers(
            &mut segments,
            &diarization,
            crate::diarize::Track::System,
            None,
            &candidates,
        );
        assert_eq!(
            labels.names,
            vec![SpeakerName {
                voice: 1,
                name: "Betrouwbare naam".into()
            }]
        );
        assert_eq!(
            segments.iter().map(|s| s.voice).collect::<Vec<_>>(),
            [Some(1), Some(2)]
        );
        assert_eq!(labels.prints.len(), 2);
        assert_eq!(labels.prints[0].known_id.as_deref(), Some("known"));
        assert_eq!(labels.prints[1].known_id, None);
    }
}
