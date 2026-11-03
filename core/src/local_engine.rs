use std::path::Path;
#[cfg(feature = "local-models")]
use std::path::PathBuf;
#[cfg(feature = "local-models")]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "local-models")]
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};

#[cfg(feature = "local-models")]
pub(crate) use transcribe_cpp::CancelToken;
#[cfg(feature = "local-models")]
use transcribe_cpp::{Feature, Model, RunOptions, Session};

use crate::error::PennelError;
#[cfg(feature = "local-models")]
use crate::speech_result::SpeechTimingKind;
use crate::speech_result::SpeechTranscript;

#[cfg(feature = "local-models")]
struct Loaded {
    path: PathBuf,
    session: Session,
    languages: Vec<String>,
    supports_vocabulary: bool,
    supports_context: bool,
    last_used: Instant,
}

/// Local speech model kept in memory between dictations and released after inactivity.
#[derive(Default)]
pub struct LocalEngine {
    #[cfg(feature = "local-models")]
    loaded: Mutex<Option<Loaded>>,
    #[cfg(feature = "local-models")]
    authoritative: AtomicUsize,
    #[cfg(feature = "local-models")]
    preview_cancel: Mutex<Option<CancelToken>>,
}

#[cfg(feature = "local-models")]
struct SpeechDemand<'a>(&'a AtomicUsize);

#[cfg(feature = "local-models")]
impl Drop for SpeechDemand<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) fn ensure_available() -> Result<(), PennelError> {
    if cfg!(feature = "local-models") {
        Ok(())
    } else {
        Err(PennelError::local_models_unavailable())
    }
}

/// Stands in for the native engine's token where local models are not built in.
#[cfg(not(feature = "local-models"))]
#[derive(Default)]
pub(crate) struct CancelToken(std::sync::atomic::AtomicBool);

#[cfg(not(feature = "local-models"))]
impl CancelToken {
    pub(crate) fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(feature = "local-models")]
fn engine_error(e: transcribe_cpp::Error) -> PennelError {
    PennelError::InvalidResponse {
        message: format!("lokaal model: {e}"),
    }
}

#[cfg(feature = "local-models")]
fn loaded_model<'a>(
    loaded: &'a mut Option<Loaded>,
    path: &Path,
) -> Result<&'a mut Loaded, PennelError> {
    if loaded.as_ref().is_none_or(|model| model.path != path) {
        *loaded = None;
        crate::cpu_support::ensure_local_models_supported()?;
        static LOGGING: std::sync::Once = std::sync::Once::new();
        LOGGING.call_once(transcribe_cpp::init_logging);
        let model = Model::load(path).map_err(engine_error)?;
        let languages = model.capabilities().map_err(engine_error)?.languages;
        let supports_vocabulary = model.supports(Feature::Vocabulary);
        let supports_context = model.supports(Feature::ContextPrompt);
        let session = model.session().map_err(engine_error)?;
        *loaded = Some(Loaded {
            path: path.to_owned(),
            session,
            languages,
            supports_vocabulary,
            supports_context,
            last_used: Instant::now(),
        });
    }
    Ok(loaded.as_mut().expect("loaded above"))
}

#[cfg(feature = "local-models")]
fn run(
    model: &mut Loaded,
    samples: &[f32],
    language: Option<&str>,
    vocabulary: &[String],
    cancel: Option<&CancelToken>,
    require_complete: bool,
) -> Result<SpeechTranscript, PennelError> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(PennelError::Cancelled);
    }
    let options = run_options(
        language,
        &model.languages,
        vocabulary,
        model.supports_vocabulary,
        model.supports_context,
    );
    if let Some(cancel) = cancel {
        model.session.set_cancel_token(cancel);
    }
    let result = model.session.run(samples, &options);
    model.session.clear_cancel_token();
    model.last_used = Instant::now();
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(PennelError::Cancelled);
    }
    transcript_result(
        result,
        require_complete,
        (samples.len() as u64)
            .saturating_mul(1_000)
            .div_ceil(16_000),
    )
}

#[cfg(all(feature = "local-models", test))]
fn transcript_text(
    result: Result<transcribe_cpp::Transcript, transcribe_cpp::Error>,
    require_complete: bool,
) -> Result<String, PennelError> {
    transcript_result(result, require_complete, u64::MAX).map(|result| result.text)
}

#[cfg(feature = "local-models")]
fn transcript_result(
    result: Result<transcribe_cpp::Transcript, transcribe_cpp::Error>,
    require_complete: bool,
    audio_duration_ms: u64,
) -> Result<SpeechTranscript, PennelError> {
    match result {
        Ok(transcript) => Ok(retain_timing(&transcript, audio_duration_ms)),
        Err(error) => match error.partial() {
            Some(partial) if !require_complete && !partial.text.trim().is_empty() => {
                Ok(retain_timing(partial, audio_duration_ms))
            }
            _ => Err(engine_error(error)),
        },
    }
}

#[cfg(feature = "local-models")]
fn retain_timing(native: &transcribe_cpp::Transcript, audio_duration_ms: u64) -> SpeechTranscript {
    use transcribe_cpp::TimestampKind;
    if matches!(
        native.timestamp_kind,
        TimestampKind::None | TimestampKind::Auto
    ) {
        return SpeechTranscript::plain(native.text.trim());
    }
    let candidates = [
        (
            SpeechTimingKind::Token,
            native
                .tokens
                .iter()
                .map(|part| (part.text.clone(), part.t0_ms, part.t1_ms))
                .collect::<Vec<_>>(),
        ),
        (
            SpeechTimingKind::Word,
            native
                .words
                .iter()
                .map(|part| (part.text.clone(), part.t0_ms, part.t1_ms))
                .collect::<Vec<_>>(),
        ),
        (
            SpeechTimingKind::Segment,
            native
                .segments
                .iter()
                .map(|part| (part.text.clone(), part.t0_ms, part.t1_ms))
                .collect::<Vec<_>>(),
        ),
    ];
    for (kind, parts) in candidates {
        if (kind == SpeechTimingKind::Token && native.timestamp_kind != TimestampKind::Token)
            || (kind == SpeechTimingKind::Word
                && !matches!(
                    native.timestamp_kind,
                    TimestampKind::Word | TimestampKind::Token
                ))
        {
            continue;
        }
        let parts: Option<Vec<_>> = parts
            .into_iter()
            .map(|(text, start, end)| {
                Some((text, u64::try_from(start).ok()?, u64::try_from(end).ok()?))
            })
            .collect();
        if let Some(parts) = parts {
            let result =
                SpeechTranscript::from_parts(native.text.clone(), &parts, kind, audio_duration_ms)
                    .trimmed();
            if !result.spans.is_empty() {
                return result;
            }
        }
    }
    SpeechTranscript::plain(native.text.trim())
}

/// Only passes options the loaded model can use: an unsupported language falls back to autodetect,
/// and the dictionary goes wherever the model accepts custom terms.
#[cfg(feature = "local-models")]
pub(crate) fn run_options(
    language: Option<&str>,
    model_languages: &[String],
    vocabulary: &[String],
    supports_vocabulary: bool,
    supports_context: bool,
) -> RunOptions {
    let base = |code: &str| code.split(['-', '_']).next().unwrap_or(code).to_lowercase();
    let language = language.and_then(|lang| {
        if model_languages.is_empty() {
            return Some(lang.to_owned());
        }
        model_languages
            .iter()
            .find(|l| base(l) == base(lang))
            .cloned()
    });
    let mut options = RunOptions {
        language,
        ..Default::default()
    };
    if !vocabulary.is_empty() {
        if supports_vocabulary {
            options.vocabulary = vocabulary.to_vec();
        } else if supports_context {
            options.prompt = Some(vocabulary.join(", "));
        }
    }
    options
}

#[cfg(feature = "local-models")]
impl LocalEngine {
    #[cfg(test)]
    pub fn is_loaded(&self) -> bool {
        self.loaded.lock().unwrap().is_some()
    }

    pub fn unload_if_idle(&self, idle: Duration, now: Instant) -> bool {
        let mut loaded = self.loaded.lock().unwrap();
        match loaded.as_ref() {
            Some(model) if now.duration_since(model.last_used) >= idle => {
                *loaded = None;
                true
            }
            _ => false,
        }
    }

    pub fn transcribe(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<String, PennelError> {
        self.transcribe_result(model_path, samples, language, vocabulary, false)
            .map(|result| result.text)
    }

    /// Replacing an existing transcript requires successful inference through end-of-stream.
    #[allow(
        dead_code,
        reason = "Retained text-only compatibility projection over complete recognition"
    )]
    pub(crate) fn transcribe_complete(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<String, PennelError> {
        self.transcribe_result(model_path, samples, language, vocabulary, true)
            .map(|result| result.text)
    }

    pub(crate) fn transcribe_timed(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<SpeechTranscript, PennelError> {
        self.transcribe_result(model_path, samples, language, vocabulary, false)
    }

    pub(crate) fn transcribe_complete_timed(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<SpeechTranscript, PennelError> {
        self.transcribe_result(model_path, samples, language, vocabulary, true)
    }

    fn transcribe_result(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
        require_complete: bool,
    ) -> Result<SpeechTranscript, PennelError> {
        self.authoritative.fetch_add(1, Ordering::SeqCst);
        let _demand = SpeechDemand(&self.authoritative);
        if let Some(preview) = self.preview_cancel.lock().unwrap().take() {
            preview.cancel();
        }
        if !model_path.exists() {
            return Err(PennelError::ModelNotDownloaded);
        }
        let mut loaded = self.loaded.lock().unwrap();
        run(
            loaded_model(&mut loaded, model_path)?,
            samples,
            language,
            vocabulary,
            None,
            require_complete,
        )
    }

    pub(crate) fn preview(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
        cancel: &CancelToken,
    ) -> Result<Option<String>, PennelError> {
        if cancel.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        if !model_path.exists() {
            return Err(PennelError::ModelNotDownloaded);
        }
        let mut loaded = match self.loaded.try_lock() {
            Ok(loaded) => loaded,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        let model = loaded_model(&mut loaded, model_path)?;
        run(model, samples, language, vocabulary, Some(cancel), false)
            .map(|result| Some(result.text))
    }

    pub(crate) fn preview_warm(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
        cancel: &CancelToken,
    ) -> Result<Option<String>, PennelError> {
        if cancel.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        let mut preview_cancel = self.preview_cancel.lock().unwrap();
        if self.authoritative.load(Ordering::SeqCst) > 0 {
            return Ok(None);
        }
        let mut loaded = match self.loaded.try_lock() {
            Ok(loaded) => loaded,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        let Some(model) = loaded.as_mut().filter(|model| model.path == model_path) else {
            return Ok(None);
        };
        *preview_cancel = Some(cancel.clone());
        drop(preview_cancel);
        if self.authoritative.load(Ordering::SeqCst) > 0 {
            cancel.cancel();
            return Err(PennelError::Cancelled);
        }
        let result = run(model, samples, language, vocabulary, Some(cancel), false)
            .map(|result| Some(result.text));
        self.preview_cancel.lock().unwrap().take();
        result
    }
}

#[cfg(not(feature = "local-models"))]
impl LocalEngine {
    #[cfg(test)]
    pub fn is_loaded(&self) -> bool {
        false
    }

    pub fn unload_if_idle(&self, _idle: Duration, _now: Instant) -> bool {
        false
    }

    pub fn transcribe(
        &self,
        _model_path: &Path,
        _samples: &[f32],
        _language: Option<&str>,
        _vocabulary: &[String],
    ) -> Result<String, PennelError> {
        Err(PennelError::local_models_unavailable())
    }

    #[allow(
        dead_code,
        reason = "Retained text-only compatibility projection without local models"
    )]
    pub(crate) fn transcribe_complete(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<String, PennelError> {
        self.transcribe(model_path, samples, language, vocabulary)
    }

    pub(crate) fn transcribe_timed(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<SpeechTranscript, PennelError> {
        self.transcribe(model_path, samples, language, vocabulary)
            .map(SpeechTranscript::plain)
    }

    pub(crate) fn transcribe_complete_timed(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
    ) -> Result<SpeechTranscript, PennelError> {
        self.transcribe_timed(model_path, samples, language, vocabulary)
    }

    pub(crate) fn preview(
        &self,
        _model_path: &Path,
        _samples: &[f32],
        _language: Option<&str>,
        _vocabulary: &[String],
        _cancel: &CancelToken,
    ) -> Result<Option<String>, PennelError> {
        Err(PennelError::local_models_unavailable())
    }

    pub(crate) fn preview_warm(
        &self,
        model_path: &Path,
        samples: &[f32],
        language: Option<&str>,
        vocabulary: &[String],
        cancel: &CancelToken,
    ) -> Result<Option<String>, PennelError> {
        self.preview(model_path, samples, language, vocabulary, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "local-models")]
    #[test]
    fn meeting_preview_never_loads_a_cold_or_different_model() {
        let engine = LocalEngine::default();
        let invalid = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            engine.preview_warm(
                invalid.path(),
                &[0.1; 16_000],
                None,
                &[],
                &CancelToken::new()
            ),
            Ok(None)
        );
        assert!(!engine.is_loaded());
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn authoritative_speech_cancels_preview_before_waiting_for_the_model() {
        let engine = LocalEngine::default();
        let cancel = CancelToken::new();
        *engine.preview_cancel.lock().unwrap() = Some(cancel.clone());
        assert_eq!(
            engine.transcribe(Path::new("/not/an/owned/model"), &[0.1; 16_000], None, &[]),
            Err(PennelError::ModelNotDownloaded)
        );
        assert!(cancel.is_cancelled());
        assert_eq!(engine.authoritative.load(Ordering::SeqCst), 0);
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn complete_transcription_rejects_native_partial_results() {
        use transcribe_cpp::{Error, Transcript};
        let partial = || {
            Some(Box::new(Transcript {
                text: "alleen het begin".into(),
                ..Default::default()
            }))
        };
        for error in [
            Error::Aborted {
                message: "stopped".into(),
                partial: partial(),
            },
            Error::OutputTruncated {
                message: "truncated".into(),
                partial: partial(),
            },
            Error::OutputRepetition {
                message: "repeating".into(),
                partial: partial(),
            },
        ] {
            assert!(transcript_text(Err(error), true).is_err());
        }
        assert_eq!(
            transcript_text(
                Ok(Transcript {
                    text: " volledige zin ".into(),
                    ..Default::default()
                }),
                true
            ),
            Ok("volledige zin".into())
        );
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn native_word_timing_is_retained_without_another_inference() {
        let native = transcribe_cpp::Transcript {
            text: " café ja. ".into(),
            timestamp_kind: transcribe_cpp::TimestampKind::Word,
            words: vec![
                transcribe_cpp::Word {
                    t0_ms: 100,
                    t1_ms: 500,
                    text: "café".into(),
                    ..Default::default()
                },
                transcribe_cpp::Word {
                    t0_ms: 550,
                    t1_ms: 900,
                    text: "ja.".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let timed = transcript_result(Ok(native), true, 1000).unwrap();
        assert_eq!(timed.text, "café ja.");
        assert_eq!(timed.spans.len(), 2);
        assert_eq!(timed.spans[0].start_ms, 100);
        assert_eq!(timed.spans[1].end_ms, 900);
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn segment_only_native_results_never_claim_word_timing() {
        let native = transcribe_cpp::Transcript {
            text: "Hallo, café.".into(),
            timestamp_kind: transcribe_cpp::TimestampKind::Segment,
            segments: vec![transcribe_cpp::Segment {
                t0_ms: 100,
                t1_ms: 900,
                text: "Hallo, café.".into(),
                ..Default::default()
            }],
            words: vec![transcribe_cpp::Word {
                t0_ms: 100,
                t1_ms: 900,
                text: "Hallo, café.".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = transcript_result(Ok(native), true, 1000).unwrap();
        assert_eq!(result.timing_kind, SpeechTimingKind::Segment);
        assert_eq!(result.spans.len(), 1);
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn unreliable_word_mapping_falls_back_to_actual_segment_then_exact_text() {
        let mut native = transcribe_cpp::Transcript {
            text: "Hallo, café.".into(),
            timestamp_kind: transcribe_cpp::TimestampKind::Word,
            words: vec![transcribe_cpp::Word {
                t0_ms: 0,
                t1_ms: 900,
                text: "Hallo café".into(),
                ..Default::default()
            }],
            segments: vec![transcribe_cpp::Segment {
                t0_ms: 0,
                t1_ms: 900,
                text: "Hallo, café.".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = transcript_result(Ok(native.clone()), true, 1000).unwrap();
        assert_eq!(result.text, native.text);
        assert_eq!(result.timing_kind, SpeechTimingKind::Segment);
        native.segments[0].t0_ms = -1;
        assert_eq!(
            transcript_result(Ok(native), true, 1000).unwrap(),
            SpeechTranscript::plain("Hallo, café.")
        );
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn timed_complete_recognition_rejects_partial_native_output() {
        let error = || transcribe_cpp::Error::OutputTruncated {
            message: "truncated".into(),
            partial: Some(Box::new(transcribe_cpp::Transcript {
                text: " café ".into(),
                ..Default::default()
            })),
        };
        assert!(transcript_result(Err(error()), true, 1000).is_err());
        assert_eq!(
            transcript_result(Err(error()), false, 1000).unwrap().text,
            "café"
        );
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn dictation_can_still_salvage_a_native_partial_result() {
        let error = transcribe_cpp::Error::OutputTruncated {
            message: "truncated".into(),
            partial: Some(Box::new(transcribe_cpp::Transcript {
                text: " gedeeltelijke zin ".into(),
                ..Default::default()
            })),
        };
        assert_eq!(
            transcript_text(Err(error), false),
            Ok("gedeeltelijke zin".into())
        );
    }

    #[cfg(feature = "local-models")]
    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn unloaded_model_is_not_released_again() {
        let engine = LocalEngine::default();
        assert!(!engine.is_loaded());
        assert!(!engine.unload_if_idle(Duration::ZERO, Instant::now()));
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn missing_model_file_is_model_not_downloaded() {
        let engine = LocalEngine::default();
        let err = engine
            .transcribe(
                Path::new("/nonexistent/model.bin"),
                &[0.0; 16000],
                None,
                &[],
            )
            .unwrap_err();
        assert_eq!(err, PennelError::ModelNotDownloaded);
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn preview_does_not_wait_for_an_active_final_transcription() {
        let engine = std::sync::Arc::new(LocalEngine::default());
        let file = tempfile::NamedTempFile::new().unwrap();
        let held = engine.loaded.lock().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_engine = engine.clone();
        let path = file.path().to_owned();
        let worker = std::thread::spawn(move || {
            sender
                .send(worker_engine.preview(&path, &[0.1; 16_000], None, &[], &CancelToken::new()))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        drop(held);
        worker.join().unwrap();
        assert_eq!(result.unwrap(), Ok(None));
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn cancelled_preview_never_loads_a_model() {
        let engine = LocalEngine::default();
        let token = CancelToken::new();
        token.cancel();
        assert_eq!(
            engine.preview(
                Path::new("/nonexistent/model.bin"),
                &[0.1; 16_000],
                None,
                &[],
                &token
            ),
            Err(PennelError::Cancelled)
        );
        assert!(!engine.is_loaded());
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn run_options_drops_unsupported_language() {
        assert_eq!(
            run_options(Some("nl"), &langs(&["en"]), &[], false, false).language,
            None
        );
        assert_eq!(
            run_options(Some("nl"), &langs(&["nl", "en"]), &[], false, false)
                .language
                .as_deref(),
            Some("nl")
        );
        assert_eq!(
            run_options(Some("nl"), &[], &[], false, false)
                .language
                .as_deref(),
            Some("nl")
        );
        assert_eq!(
            run_options(None, &langs(&["nl"]), &[], false, false).language,
            None
        );
        assert_eq!(
            run_options(Some("nl"), &langs(&["en-US", "nl-NL"]), &[], false, false)
                .language
                .as_deref(),
            Some("nl-NL")
        );
    }

    #[cfg(feature = "local-models")]
    #[test]
    fn run_options_routes_vocabulary() {
        let terms = langs(&["Pennel"]);
        let vocab = run_options(None, &[], &terms, true, true);
        assert_eq!(vocab.vocabulary, terms);
        assert_eq!(vocab.prompt, None);
        let context = run_options(None, &[], &terms, false, true);
        assert!(context.vocabulary.is_empty());
        assert_eq!(context.prompt.as_deref(), Some("Pennel"));
        let neither = run_options(None, &[], &terms, false, false);
        assert!(neither.vocabulary.is_empty());
        assert_eq!(neither.prompt, None);
    }

    #[cfg(feature = "local-models")]
    fn real_model_transcribes(file: &str) {
        let home = std::env::var("HOME").unwrap();
        let model = PathBuf::from(home)
            .join("Library/Application Support/Pennel/models")
            .join(file);
        let wav = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/hello.wav"
        ))
        .unwrap();
        let (samples, _) = crate::wav::decode_wav(&wav).unwrap();
        let engine = LocalEngine::default();
        let started = Instant::now();
        let text = engine
            .transcribe(&model, &samples, Some("en"), &[])
            .unwrap();
        eprintln!("{file} transcribed in {:?}: {text}", started.elapsed());
        assert!(text.to_lowercase().contains("test"), "{text}");
        assert!(engine.is_loaded());
        assert!(engine.unload_if_idle(Duration::ZERO, Instant::now()));
        assert!(!engine.is_loaded());
    }

    /// Downloads Parakeet v3 into the app's model folder if missing; run with `cargo test -- --ignored`.
    #[cfg(feature = "local-models")]
    #[test]
    #[ignore]
    fn real_parakeet_transcribes() {
        let home = std::env::var("HOME").unwrap();
        let store = crate::models::ModelStore::new(
            &PathBuf::from(home).join("Library/Application Support/Pennel"),
        );
        struct Quiet;
        impl crate::models::DownloadListener for Quiet {
            fn on_progress(&self, _: u64, _: u64) {}
        }
        let started = Instant::now();
        store
            .download(
                "parakeet-tdt-0.6b-v3",
                &Quiet,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        eprintln!("download ready in {:?}", started.elapsed());
        real_model_transcribes("parakeet-tdt-0.6b-v3.gguf");
    }

    /// Needs the legacy small model in the app's model folder; run with `cargo test -- --ignored`.
    #[cfg(feature = "local-models")]
    #[test]
    #[ignore]
    fn real_legacy_whisper_transcribes() {
        real_model_transcribes("whisper-small-q5_1.bin");
    }

    #[cfg(not(feature = "local-models"))]
    #[test]
    fn local_transcribe_without_feature_is_dutch_error() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let engine = LocalEngine::default();
        assert_eq!(
            engine.transcribe(file.path(), &[0.0; 16_000], None, &[]),
            Err(PennelError::local_models_unavailable())
        );
    }
}
