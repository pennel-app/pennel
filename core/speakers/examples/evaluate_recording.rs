//! Local-only evaluation bridge. It emits timing/anonymous turns, never PCM or voiceprints.

#[cfg(feature = "onnx")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pennel_speakers::{Diarizer, FRAGMENT_THRESHOLD, Models, Options, reconcile_fragments};
    use std::{path::PathBuf, time::Instant};

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() != 5 && !(arguments.len() == 6 && arguments[5] == "--voice-diagnostics") {
        return Err("usage: evaluate_recording AUDIO.wav SEGMENTATION.onnx EMBEDDING.onnx WINDOW_SHIFT CLUSTER_THRESHOLD [--voice-diagnostics]".into());
    }
    let audio = std::fs::read(&arguments[0])?;
    let samples = pcm16_wave(&audio)?;
    let segmentation = PathBuf::from(&arguments[1]);
    let embedding = PathBuf::from(&arguments[2]);
    let window_shift: f32 = arguments[3].parse()?;
    let cluster_threshold: f32 = arguments[4].parse()?;
    if !window_shift.is_finite()
        || !(0.0..=1.0).contains(&window_shift)
        || window_shift == 0.0
        || !cluster_threshold.is_finite()
        || !(0.0..=2.0).contains(&cluster_threshold)
    {
        return Err("invalid diarization options".into());
    }
    let options = Options {
        window_shift,
        cluster_threshold,
        ..Options::default()
    };
    let started = Instant::now();
    let diarizer = Diarizer::load(
        &Models {
            segmentation: &segmentation,
            embedding: &embedding,
        },
        &options,
    )?;
    let model_load_ms = started.elapsed().as_millis();
    let inference = Instant::now();
    let raw = diarizer.process_unreconciled(&samples)?;
    let inference_ms = inference.elapsed().as_millis();
    let merging = Instant::now();
    let reconciled = reconcile_fragments(raw.clone(), FRAGMENT_THRESHOLD);
    let reconciliation_ms = merging.elapsed().as_millis();
    let diagnostic_started = Instant::now();
    let diagnostics = if arguments.len() == 6 {
        Some(voice_diagnostics(
            &raw,
            &samples,
            &embedding,
            options.threads,
        )?)
    } else {
        None
    };
    let diagnostic_ms = diagnostics
        .as_ref()
        .map(|_| diagnostic_started.elapsed().as_millis());
    let result = |diarization: &pennel_speakers::Diarization| {
        serde_json::json!({
            "speaker_count": diarization.speakers.len(),
            "speakers_with_clean_voiceprints": diarization.speakers.iter().filter(|voice| !voice.embedding.is_empty()).count(),
            "turns": diarization.turns,
        })
    };
    println!(
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "sample_count": samples.len(),
            "sample_rate": pennel_speakers::SAMPLE_RATE,
            "duration_ms": samples.len() as u64 * 1_000 / u64::from(pennel_speakers::SAMPLE_RATE),
            "options": { "window_shift": options.window_shift, "cluster_threshold": options.cluster_threshold,
                         "fragment_threshold": FRAGMENT_THRESHOLD, "threads": options.threads },
            "model_load_ms": model_load_ms,
            "inference_ms": inference_ms,
            "reconciliation_ms": reconciliation_ms,
        "voice_diagnostics": diagnostics,
        "diagnostic_ms": diagnostic_ms,
            "raw": result(&raw),
            "reconciled": result(&reconciled),
        })
    );
    Ok(())
}

#[cfg(feature = "onnx")]
fn voice_diagnostics(
    diarization: &pennel_speakers::Diarization,
    samples: &[f32],
    model: &std::path::Path,
    threads: i32,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use pennel_speakers::{Embedder, cosine, voiceprint_spans};
    let embedder = Embedder::load(model, threads)?;
    let mut voices = Vec::new();
    for voice in &diarization.speakers {
        let mut clean = voiceprint_spans(&diarization.turns, voice.speaker, u64::MAX);
        clean.retain(|(start, end)| end - start >= 1_500);
        clean.sort_unstable();
        let count = clean.len().min(12);
        let mut points = Vec::new();
        let mut embeddings = Vec::new();
        for index in 0..count {
            let selected = if count == 1 {
                0
            } else {
                index * (clean.len() - 1) / (count - 1)
            };
            let (start, end) = clean[selected];
            let length = (end - start).min(5_000);
            let start = start + (end - start - length) / 2;
            let end = start + length;
            let embedding = embedder.embed_spans(samples, &[(start, end)])?;
            if embedding.is_empty() {
                continue;
            }
            let own = cosine(&embedding, &voice.embedding);
            let nearest = diarization
                .speakers
                .iter()
                .filter(|other| other.speaker != voice.speaker && !other.embedding.is_empty())
                .map(|other| (other.speaker, cosine(&embedding, &other.embedding)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            points.push(serde_json::json!({ "start_ms": start, "end_ms": end,
                "own_similarity": own,
                "nearest_other": nearest.map(|(speaker, similarity)| serde_json::json!({ "speaker": speaker, "similarity": similarity })),
                "own_minus_other": nearest.map(|(_, similarity)| own - similarity) }));
            embeddings.push(embedding);
        }
        let mut similarities = Vec::new();
        for a in 0..embeddings.len() {
            for b in a + 1..embeddings.len() {
                similarities.push(cosine(&embeddings[a], &embeddings[b]));
            }
        }
        similarities.sort_by(f32::total_cmp);
        let nearest = diarization
            .speakers
            .iter()
            .filter(|other| {
                other.speaker != voice.speaker
                    && !voice.embedding.is_empty()
                    && !other.embedding.is_empty()
            })
            .map(|other| (other.speaker, cosine(&voice.embedding, &other.embedding)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        voices.push(serde_json::json!({ "speaker": voice.speaker,
            "eligible_clean_span_count": clean.len(), "sampled_span_count": embeddings.len(),
            "pair_similarity_min": similarities.first(),
            "pair_similarity_median": similarities.get(similarities.len() / 2),
            "nearest_centroid": nearest.map(|(speaker, similarity)| serde_json::json!({ "speaker": speaker, "similarity": similarity })),
            "points": points }));
    }
    Ok(serde_json::Value::Array(voices))
}

#[cfg(not(feature = "onnx"))]
fn main() {
    eprintln!("evaluate_recording requires --features onnx; it does not download models");
    std::process::exit(1);
}

#[cfg(any(feature = "onnx", test))]
fn pcm16_wave(bytes: &[u8]) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("audio must be a RIFF PCM16 mono 16000 Hz WAVE file".into());
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into()?) as usize + 8;
    if declared > bytes.len() {
        return Err("truncated RIFF file".into());
    }
    let mut cursor = 12usize;
    let mut format = false;
    let mut data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= declared) {
        let length = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into()?) as usize;
        let start = cursor + 8;
        let end = start.checked_add(length).ok_or("WAVE chunk overflow")?;
        if end > declared {
            return Err("truncated WAVE chunk".into());
        }
        match &bytes[cursor..cursor + 4] {
            b"fmt " => {
                if length < 16 {
                    return Err("truncated WAVE format".into());
                }
                let word = |offset| {
                    u16::from_le_bytes(
                        bytes[start + offset..start + offset + 2]
                            .try_into()
                            .unwrap(),
                    )
                };
                let rate = u32::from_le_bytes(bytes[start + 4..start + 8].try_into()?);
                if word(0) != 1 || word(2) != 1 || rate != 16_000 || word(12) != 2 || word(14) != 16
                {
                    return Err("audio must be PCM16 mono 16000 Hz".into());
                }
                format = true;
            }
            b"data" => {
                if data.is_some() {
                    return Err("multiple WAVE data chunks are unsupported".into());
                }
                data = Some(&bytes[start..end]);
            }
            _ => {}
        }
        cursor = end.checked_add(length % 2).ok_or("WAVE chunk overflow")?;
    }
    if !format {
        return Err("missing WAVE format".into());
    }
    let data = data.ok_or("missing WAVE data")?;
    if data.len() % 2 != 0 {
        return Err("incomplete PCM16 sample".into());
    }
    Ok(data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::pcm16_wave;

    #[test]
    fn reads_chunks_instead_of_assuming_a_44_byte_wave_header() {
        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"JUNK\x03\x00\x00\x00abc\0");
        chunks.extend_from_slice(
            b"fmt \x10\x00\x00\x00\x01\x00\x01\x00\x80\x3e\x00\x00\x00\x7d\x00\x00\x02\x00\x10\x00",
        );
        chunks.extend_from_slice(b"data\x04\x00\x00\x00\x00\x40\x00\xc0");
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
        wave.extend_from_slice(b"WAVE");
        wave.extend_from_slice(&chunks);
        assert_eq!(pcm16_wave(&wave).unwrap(), [0.5, -0.5]);
        assert!(pcm16_wave(&wave[..wave.len() - 1]).is_err());
        wave[34] = 2; // stereo channel count in the fmt chunk
        assert!(pcm16_wave(&wave).is_err());
    }
}
