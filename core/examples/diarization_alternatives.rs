//! Local-only comparison of the existing Sortformer runtime and CAM++ clean-turn coherence.
//! Does not name voices, enroll voiceprints, modify meetings, or request a speaker count.

#[cfg(all(feature = "local-models", feature = "speakers"))]
#[allow(dead_code)]
#[path = "../src/wav.rs"]
mod wav;

#[cfg(all(feature = "local-models", feature = "speakers"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pennel_speakers::{Embedder, Speaker, cosine, renumber, speech_ms, voiceprint_spans};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{fs, io::Write, path::Path, time::Instant};
    use transcribe_cpp::{
        DiarizeExtension, DiarizeOptions, DiarizeSessionOptions, Model, SortformerDiarizeOptions,
        SortformerPreset,
    };

    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 5 {
        return Err("usage: diarization_alternatives AUDIO.wav SORTFORMER.gguf EMBEDDING.onnx default|offline PRIVATE_RESULT.json".into());
    }
    let preset = match arguments[3].as_str() {
        "default" => SortformerPreset::Default,
        "offline" => SortformerPreset::VeryHighLatency,
        _ => return Err("preset must be default or offline".into()),
    };
    let result_path = Path::new(&arguments[4]);
    if result_path.exists() {
        return Err("result already exists; choose a new private result path".into());
    }
    let audio = fs::read(&arguments[0])?;
    let (samples, rate) = wav::decode_wav(&audio).ok_or("invalid PCM16 mono WAVE")?;
    if rate != pennel_speakers::SAMPLE_RATE || samples.is_empty() {
        return Err("audio must be nonempty 16000 Hz mono PCM16 WAVE".into());
    }
    let hash = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let started = Instant::now();
    let model = Model::load(&arguments[1])?;
    let capability = model.diarize_info()?;
    let mut session = model.diarize_session_with(&DiarizeSessionOptions { n_threads: 4 })?;
    let model_load_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let inferred = session.run(
        &samples,
        &DiarizeOptions {
            family: Some(DiarizeExtension::Sortformer(SortformerDiarizeOptions {
                preset: Some(preset),
            })),
        },
    )?;
    let inference_ms = started.elapsed().as_millis();
    let (turns, count) = renumber(inferred.iter().filter_map(|turn| {
        (turn.speaker_id > 0 && turn.t1_ms > turn.t0_ms).then_some((
            turn.t0_ms.max(0) as u64,
            turn.t1_ms as u64,
            i64::from(turn.speaker_id),
        ))
    }));
    let started = Instant::now();
    let embedder = Embedder::load(Path::new(&arguments[2]), 4)?;
    let speakers = (0..count as u32)
        .map(|speaker| {
            let spans = voiceprint_spans(&turns, speaker, 60_000);
            Ok(Speaker {
                speaker,
                speech_ms: speech_ms(&turns, speaker),
                embedding: embedder.embed_spans(&samples, &spans)?,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let mut diagnostics = Vec::new();
    for voice in &speakers {
        let mut clean = voiceprint_spans(&turns, voice.speaker, u64::MAX);
        clean.retain(|(start, end)| end - start >= 1_500);
        clean.sort_unstable();
        let count = clean.len().min(12);
        let mut points = Vec::new();
        let mut vectors = Vec::new();
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
            let embedding = embedder.embed_spans(&samples, &[(start, end)])?;
            if embedding.is_empty() {
                continue;
            }
            let own = cosine(&embedding, &voice.embedding);
            let nearest = speakers
                .iter()
                .filter(|other| other.speaker != voice.speaker && !other.embedding.is_empty())
                .map(|other| (other.speaker, cosine(&embedding, &other.embedding)))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            points.push(json!({
                "start_ms": start, "end_ms": end, "own_similarity": own,
                "nearest_other": nearest.map(|(speaker, similarity)| json!({ "speaker": speaker, "similarity": similarity })),
                "own_minus_other": nearest.map(|(_, similarity)| own - similarity),
            }));
            vectors.push(embedding);
        }
        let mut similarities = Vec::new();
        for a in 0..vectors.len() {
            for b in a + 1..vectors.len() {
                similarities.push(cosine(&vectors[a], &vectors[b]));
            }
        }
        similarities.sort_by(f32::total_cmp);
        diagnostics.push(json!({
            "speaker": voice.speaker, "speech_ms": voice.speech_ms,
            "clean_ms": clean.iter().map(|(start, end)| end - start).sum::<u64>(),
            "eligible_clean_span_count": clean.len(), "sampled_span_count": vectors.len(),
            "pair_similarity_min": similarities.first(),
            "pair_similarity_median": similarities.get(similarities.len() / 2),
            "points": points,
        }));
    }
    let diagnostic_ms = started.elapsed().as_millis();
    let output = json!({
        "schema_version": 1, "route": "transcribe_cpp_sortformer_campp_diagnostics",
        "preset": arguments[3], "configured_speaker_count": null,
        "capability_max_speakers": capability.max_speakers,
        "sample_count": samples.len(), "sample_rate": rate,
        "duration_ms": samples.len() as u64 * 1_000 / u64::from(rate),
        "audio_sha256": hash(&audio),
        "sortformer_sha256": hash(&fs::read(&arguments[1])?),
        "embedding_sha256": hash(&fs::read(&arguments[2])?),
        "model_load_ms": model_load_ms, "inference_ms": inference_ms,
        "diagnostic_ms": diagnostic_ms,
        "raw": { "speaker_count": count, "turns": turns },
        "voice_diagnostics": diagnostics,
        "reference_status": "unscored_until_frozen_reference_evaluation",
    });
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(result_path)?;
    serde_json::to_writer_pretty(&mut file, &output)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!(
        "{}",
        json!({ "speakers": count, "inference_ms": inference_ms,
                "diagnostic_ms": diagnostic_ms, "result_saved": true })
    );
    Ok(())
}

#[cfg(not(all(feature = "local-models", feature = "speakers")))]
fn main() {
    eprintln!("diarization_alternatives requires the local-models and speakers features");
    std::process::exit(2);
}
