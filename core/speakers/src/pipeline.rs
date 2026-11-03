use std::path::Path;

use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
};

use crate::{
    CLUSTER_THRESHOLD, Diarization, Error, SAMPLE_RATE, Speaker, VOICEPRINT_BUDGET_MS, normalized,
    renumber, speech_ms, voiceprint_spans,
};

pub struct Models<'a> {
    /// pyannote segmentation-3.0 (ONNX).
    pub segmentation: &'a Path,
    /// Speaker embedding model (ONNX).
    pub embedding: &'a Path,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub threads: i32,
    pub cluster_threshold: f32,
    /// Step of the 10 s segmentation window, as a fraction of it. 0.25 is four times faster than the
    /// default 0.1 and found the same speakers on the reference recordings.
    pub window_shift: f32,
}

impl Default for Options {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            threads: cores.clamp(1, 4) as i32,
            cluster_threshold: CLUSTER_THRESHOLD,
            window_shift: 0.25,
        }
    }
}

fn existing(path: &Path) -> Result<String, Error> {
    if !path.is_file() {
        return Err(Error::Model("file missing".into()));
    }
    Ok(path.to_string_lossy().into_owned())
}

pub struct Embedder {
    extractor: SpeakerEmbeddingExtractor,
}

impl Embedder {
    pub fn load(model: &Path, threads: i32) -> Result<Self, Error> {
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(existing(model)?),
            num_threads: threads.max(1),
            debug: false,
            provider: Some("cpu".into()),
        };
        let extractor = SpeakerEmbeddingExtractor::create(&config)
            .ok_or_else(|| Error::Model("embeddingmodel".into()))?;
        Ok(Self { extractor })
    }

    /// Normalised voiceprint of 16 kHz mono audio; empty when it is too short for one.
    pub fn embed(&self, samples: &[f32]) -> Result<Vec<f32>, Error> {
        let stream = self
            .extractor
            .create_stream()
            .ok_or_else(|| Error::Audio("no stream".into()))?;
        stream.accept_waveform(SAMPLE_RATE as i32, samples);
        stream.input_finished();
        if !self.extractor.is_ready(&stream) {
            return Ok(Vec::new());
        }
        let embedding = self
            .extractor
            .compute(&stream)
            .ok_or_else(|| Error::Audio("no voiceprint".into()))?;
        Ok(normalized(&embedding))
    }

    /// One voiceprint over several spans (ms) of the same recording.
    pub fn embed_spans(&self, samples: &[f32], spans: &[(u64, u64)]) -> Result<Vec<f32>, Error> {
        let at = |ms: u64| ((ms * u64::from(SAMPLE_RATE) / 1000) as usize).min(samples.len());
        let joined: Vec<f32> = spans
            .iter()
            .flat_map(|(start, end)| samples[at(*start)..at(*end)].iter().copied())
            .collect();
        if joined.is_empty() {
            return Ok(Vec::new());
        }
        self.embed(&joined)
    }
}

/// Voiceprint of a whole recording, e.g. the enrollment text.
pub fn embed(model: &Path, samples: &[f32]) -> Result<Vec<f32>, Error> {
    Embedder::load(model, Options::default().threads)?.embed(samples)
}

/// Loaded local acoustic models, owned by a worker and reused for bounded windows.
pub struct Diarizer {
    diarizer: OfflineSpeakerDiarization,
    embedder: Embedder,
}

impl Diarizer {
    pub fn load(models: &Models, options: &Options) -> Result<Self, Error> {
        let segmentation = existing(models.segmentation)?;
        let embedding = existing(models.embedding)?;
        let config = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: Some(segmentation),
                    window_shift_ratio: options.window_shift,
                },
                num_threads: options.threads.max(1),
                debug: false,
                provider: Some("cpu".into()),
            },
            embedding: SpeakerEmbeddingExtractorConfig {
                model: Some(embedding),
                num_threads: options.threads.max(1),
                debug: false,
                provider: Some("cpu".into()),
            },
            clustering: FastClusteringConfig {
                num_clusters: -1,
                threshold: options.cluster_threshold,
                compute_confidence: false,
            },
            min_duration_on: 0.3,
            min_duration_off: 0.5,
        };
        let diarizer = OfflineSpeakerDiarization::create(&config)
            .ok_or_else(|| Error::Model("segmentatiemodel".into()))?;
        let embedder = Embedder::load(models.embedding, options.threads)?;
        Ok(Self { diarizer, embedder })
    }

    pub fn process(&self, samples: &[f32]) -> Result<Diarization, Error> {
        if samples.len() < SAMPLE_RATE as usize {
            return Ok(Diarization::default());
        }
        let result = self
            .diarizer
            .process(samples)
            .ok_or_else(|| Error::Audio("no result".into()))?;
        let raw = result.sort_by_start_time().into_iter().map(|segment| {
            (
                (segment.start.max(0.0) * 1000.0).round() as u64,
                (segment.end.max(0.0) * 1000.0).round() as u64,
                i64::from(segment.speaker),
            )
        });
        let (turns, count) = renumber(raw);
        let speakers = (0..count as u32)
            .map(|speaker| {
                let spans = voiceprint_spans(&turns, speaker, VOICEPRINT_BUDGET_MS);
                Ok(Speaker {
                    speaker,
                    speech_ms: speech_ms(&turns, speaker),
                    embedding: self.embedder.embed_spans(samples, &spans)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Diarization { turns, speakers })
    }
}

/// Who spoke when in 16 kHz mono audio, with a voiceprint per speaker.
pub fn diarize(models: &Models, samples: &[f32], options: &Options) -> Result<Diarization, Error> {
    existing(models.segmentation)?;
    existing(models.embedding)?;
    if samples.len() < SAMPLE_RATE as usize {
        return Ok(Diarization::default());
    }
    Diarizer::load(models, options)?.process(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_models_are_a_model_error() {
        let missing = Path::new("/nonexistent/pennel/model.onnx");
        let models = Models {
            segmentation: missing,
            embedding: missing,
        };
        assert!(matches!(
            diarize(&models, &[0.0; 32_000], &Options::default()),
            Err(Error::Model(_))
        ));
        assert!(matches!(
            embed(missing, &[0.0; 32_000]),
            Err(Error::Model(_))
        ));
    }

    /// Needs the models and a 16 kHz mono WAV of two people; run with
    /// `PENNEL_SPEAKER_MODELS=<dir with segmentation.onnx, embedding.onnx> PENNEL_SPEAKER_WAV=<wav> cargo test --features onnx -- --ignored`.
    #[test]
    #[ignore]
    fn real_recording_splits_voices() {
        let dir = std::path::PathBuf::from(std::env::var("PENNEL_SPEAKER_MODELS").unwrap());
        let wav = std::fs::read(std::env::var("PENNEL_SPEAKER_WAV").unwrap()).unwrap();
        let samples: Vec<f32> = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0)
            .collect();
        let segmentation = dir.join("segmentation.onnx");
        let embedding = dir.join("embedding.onnx");
        let models = Models {
            segmentation: &segmentation,
            embedding: &embedding,
        };
        let result = diarize(&models, &samples, &Options::default()).unwrap();
        eprintln!(
            "{} speakers, {} turns",
            result.speakers.len(),
            result.turns.len()
        );
        assert!(result.speakers.len() >= 2);
        assert!(result.speakers.iter().all(|s| !s.embedding.is_empty()));
    }
}
