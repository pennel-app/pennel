//! Offline native-component replay. Private results are unscored without annotated audio.
//! Usage: local_speech_quality MODEL_GGUF PRIVATE_MANIFEST_JSON PRIVATE_RESULT_JSON

#[cfg(feature = "local-models")]
#[path = "../src/wav.rs"]
mod wav;

#[cfg(not(feature = "local-models"))]
fn main() {
    eprintln!("Local speech replay requires the local-models feature.");
    std::process::exit(2);
}

#[cfg(feature = "local-models")]
mod replay {
    use std::collections::HashSet;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use serde::Deserialize;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use transcribe_cpp::{Model, RunOptions, Transcript};

    type Error = Box<dyn std::error::Error>;

    #[derive(Deserialize)]
    struct Manifest {
        schema_version: u32,
        consented_private_fixture: bool,
        clips: Vec<Clip>,
    }

    #[derive(Deserialize)]
    struct Clip {
        id: String,
        wav_path: PathBuf,
        wav_sha256: String,
        source_id: String,
        source_audio_sha256: String,
        source_start_ms: u64,
        source_end_ms: u64,
    }

    fn digest(path: &Path) -> Result<String, Error> {
        let mut file = File::open(path)?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            digest.update(&buffer[..size]);
        }
        Ok(digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn validate_manifest(manifest: &Manifest) -> Result<(), Error> {
        let mut ids = HashSet::new();
        if manifest.schema_version != 1
            || !manifest.consented_private_fixture
            || manifest.clips.is_empty()
            || manifest.clips.len() > 20
        {
            return Err(
                "An explicitly consented private fixture with 1–20 clips is required.".into(),
            );
        }
        for clip in &manifest.clips {
            if clip.id.is_empty()
                || !ids.insert(&clip.id)
                || !matches!(clip.source_id.as_str(), "microphone" | "system_audio")
                || clip.source_end_ms <= clip.source_start_ms
                || clip.source_end_ms - clip.source_start_ms > 30_000
                || [&clip.wav_sha256, &clip.source_audio_sha256]
                    .iter()
                    .any(|value| {
                        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
            {
                return Err(
                    "Clip identifiers, physical source, hashes and bounded times must be valid."
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn transcript_json(transcript: &Transcript) -> Value {
        json!({
            "text": transcript.text,
            "raw_text": transcript.raw_text,
            "detected_language": transcript.language,
            "timestamp_kind": format!("{:?}", transcript.timestamp_kind),
            "segments": transcript.segments.iter().map(|part| json!({
                "start_ms": part.t0_ms, "end_ms": part.t1_ms, "text": part.text,
            })).collect::<Vec<_>>(),
            "words": transcript.words.iter().map(|part| json!({
                "start_ms": part.t0_ms, "end_ms": part.t1_ms, "text": part.text,
            })).collect::<Vec<_>>(),
            "tokens": transcript.tokens.iter().map(|part| json!({
                "id": part.id, "start_ms": part.t0_ms, "end_ms": part.t1_ms,
                "text": part.text, "probability_hint": part.p.is_finite().then_some(part.p),
            })).collect::<Vec<_>>(),
            "native_timings_ms": {
                "load": transcript.timings.load_ms, "mel": transcript.timings.mel_ms,
                "encode": transcript.timings.encode_ms, "decode": transcript.timings.decode_ms,
            },
        })
    }

    fn write_private(path: &Path, result: &Value) -> Result<(), Error> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        serde_json::to_writer_pretty(&mut file, result)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }

    pub fn run() -> Result<(), Error> {
        let arguments: Vec<_> = std::env::args().skip(1).collect();
        if arguments.len() != 3 {
            return Err(
                "Usage: local_speech_quality MODEL_GGUF PRIVATE_MANIFEST_JSON PRIVATE_RESULT_JSON"
                    .into(),
            );
        }
        let model_path = Path::new(&arguments[0]).canonicalize()?;
        let manifest_path = Path::new(&arguments[1]).canonicalize()?;
        let output_path = Path::new(&arguments[2]);
        if output_path.exists() {
            return Err("The private result already exists; choose a fresh attempt path.".into());
        }
        let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        validate_manifest(&manifest)?;
        let model_sha256 = digest(&model_path)?;
        let load_started = Instant::now();
        let model = Model::load(&model_path)?;
        let model_load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
        let capabilities = model.capabilities()?;
        let mut results = Vec::new();
        for clip in &manifest.clips {
            let clip_path = if clip.wav_path.is_absolute() {
                clip.wav_path.clone()
            } else {
                manifest_path.parent().unwrap().join(&clip.wav_path)
            };
            if digest(&clip_path)? != clip.wav_sha256 {
                return Err("A private audio clip changed after its manifest was frozen.".into());
            }
            let (samples, sample_rate) = super::wav::decode_wav(&fs::read(&clip_path)?)
                .ok_or("A clip must be a valid mono PCM16 WAV.")?;
            let duration_ms = samples.len() as u64 * 1000 / u64::from(sample_rate.max(1));
            if sample_rate != 16_000
                || samples.is_empty()
                || duration_ms.abs_diff(clip.source_end_ms - clip.source_start_ms) > 1
            {
                return Err(
                    "A clip must be 16 kHz mono with the manifest's exact duration.".into(),
                );
            }
            for language in [None, Some("nl")] {
                // New sessions prevent a preceding run's text/context from influencing its pair.
                let mut session = model.session()?;
                let options = RunOptions {
                    language: language.map(str::to_owned),
                    timestamps: capabilities.max_timestamp_kind,
                    ..Default::default()
                };
                let started = Instant::now();
                let result = session.run(&samples, &options);
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                let (transcript, complete, error) = match result {
                    Ok(transcript) => (Some(transcript), true, None),
                    Err(error) => (
                        error.partial().cloned(),
                        false,
                        Some(json!({
                            "status": error.raw_status(), "message": error.to_string(),
                        })),
                    ),
                };
                results.push(json!({
                    "clip_id": clip.id, "source_id": clip.source_id,
                    "source_audio_sha256": clip.source_audio_sha256,
                    "source_start_ms": clip.source_start_ms, "source_end_ms": clip.source_end_ms,
                    "wav_sha256": clip.wav_sha256, "sample_rate": sample_rate,
                    "sample_count": samples.len(), "duration_ms": duration_ms,
                    "requested_language": language,
                    "requested_timestamp_kind": format!("{:?}", capabilities.max_timestamp_kind),
                    "options": {"vocabulary": [], "prompt": null, "family": "native_defaults", "fresh_session": true},
                    "complete": complete, "elapsed_ms": elapsed_ms,
                    "error": error, "transcript": transcript.as_ref().map(transcript_json),
                }));
            }
        }
        let report = json!({
            "schema_version": 1, "scope": "offline_native_component_replay",
            "quality_status": "unscored_no_audio_ground_truth",
            "limits": ["Not the shipped chunking/transcription workflow", "No WER or language-accuracy claim", "Paired elapsed times are descriptive, not cold/warm performance qualification"],
            "network_used": false, "hosted_spend": 0,
            "model_sha256": model_sha256, "model_arch": model.arch(),
            "model_backend": model.backend(), "model_load_ms": model_load_ms,
            "native_version": transcribe_cpp::version(), "native_commit": transcribe_cpp::version_commit(),
            "native_max_timestamp_kind": format!("{:?}", capabilities.max_timestamp_kind),
            "manifest_sha256": digest(&manifest_path)?,
            "harness_sha256": digest(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/local_speech_quality.rs"))?,
            "attempts": results,
        });
        write_private(output_path, &report)?;
        println!(
            "Saved {} unscored local attempts to the private result file.",
            results.len()
        );
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn fixture() -> Manifest {
            serde_json::from_value(json!({
                "schema_version": 1, "consented_private_fixture": true,
                "clips": [{"id":"owned", "wav_path":"clip.wav", "wav_sha256":"a".repeat(64),
                    "source_id":"microphone", "source_audio_sha256":"b".repeat(64),
                    "source_start_ms":0,"source_end_ms":1000}],
            }))
            .unwrap()
        }

        #[test]
        fn private_replay_rejects_unconsented_and_ambiguous_clip_manifests() {
            assert!(validate_manifest(&fixture()).is_ok());
            let mut value = fixture();
            value.consented_private_fixture = false;
            assert!(validate_manifest(&value).is_err());
            let mut value = fixture();
            value.clips[0].source_id = "person_name".into();
            assert!(validate_manifest(&value).is_err());
            let mut value = fixture();
            value.clips[0].source_end_ms = 30_001;
            assert!(validate_manifest(&value).is_err());
        }

        #[test]
        fn private_receipt_never_overwrites_an_existing_attempt() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("attempt.json");
            write_private(&path, &json!({"attempt": 1})).unwrap();
            assert!(write_private(&path, &json!({"attempt": 2})).is_err());
            assert_eq!(
                serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),
                json!({"attempt":1})
            );
        }
    }
}

#[cfg(feature = "local-models")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    replay::run()
}
