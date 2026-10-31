use std::sync::Arc;

use crate::llm::{ChatMessage, Reasoning};
use crate::provider::{requires_key, supports};
use crate::{ApiKeys, PennelEngine, PennelError, ProviderTask};

const SPEECH_SAMPLE: &[u8] = include_bytes!("../tests/fixtures/hello.wav");
const TEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Record)]
pub struct ProviderTaskTestResult {
    pub verified: bool,
    pub message: String,
}

#[uniffi::export]
pub fn test_provider_task(
    engine: Arc<PennelEngine>,
    task: ProviderTask,
    keys: ApiKeys,
) -> Result<ProviderTaskTestResult, PennelError> {
    let mut settings = engine.settings();
    let choice = match task {
        ProviderTask::DictationStt => settings.dictation_stt.clone(),
        ProviderTask::MeetingStt => settings.meeting_stt.clone(),
        ProviderTask::Llm => settings.llm.clone(),
    };
    let (account, model) = settings.resolve(&choice).ok_or(PennelError::NotFound)?;
    if !supports(account.kind, task) {
        return Err(PennelError::InvalidResponse {
            message: "This provider does not support the selected task.".into(),
        });
    }
    if model.trim().is_empty() && !account.kind.is_cli_agent() {
        return Err(PennelError::InvalidResponse {
            message: "Choose a model to test first.".into(),
        });
    }

    let outcome = if task == ProviderTask::Llm {
        engine.credential(&account, &keys).and_then(|key| {
            if requires_key(account.kind) && key.is_none() {
                return Err(PennelError::MissingApiKey { task: "llm".into() });
            }
            let messages = [
                ChatMessage {
                    role: "system",
                    content:
                        "This is a synthetic Pennel connection test. Reply with only the word OK."
                            .into(),
                },
                ChatMessage {
                    role: "user",
                    content: "Pennel example: the meeting starts at ten.".into(),
                },
            ];
            engine.complete_llm(
                &engine.endpoint(&account, model),
                key.as_deref(),
                &messages,
                TEST_TIMEOUT_SECS,
                Reasoning::Off,
            )
        })
    } else {
        // Vocabulary is user content; a connection test sends only the bundled sample.
        settings.dictionary.clear();
        engine.stt(&settings, &choice, SPEECH_SAMPLE, &keys)
    };
    let result = match outcome {
        Ok(text) if !text.trim().is_empty() => ProviderTaskTestResult {
            verified: true,
            message: if task == ProviderTask::Llm {
                "The selected language model processed the sample text."
            } else {
                "The selected speech model processed the audio sample."
            }
            .into(),
        },
        Ok(_) | Err(PennelError::NoSpeech) => {
            failed("The selected model returned no text. Check the model and try again.")
        }
        Err(error) => failed(failure_message(&error)),
    };
    Ok(result)
}

fn failed(message: &str) -> ProviderTaskTestResult {
    ProviderTaskTestResult {
        verified: false,
        message: message.into(),
    }
}

fn failure_message(error: &PennelError) -> &'static str {
    // Native/provider error details are untrusted and may echo request data or credentials.
    match error {
        PennelError::MissingApiKey { .. } => "Configure this provider’s API key first.",
        PennelError::Http {
            status: 401 | 403, ..
        } => "The provider denied access. Check your API key and access to the model.",
        PennelError::Http { status: 402, .. } => {
            "The provider reports insufficient credit. Check your account."
        }
        PennelError::Http { status: 429, .. } => {
            "The provider has reached its usage limit. Try again later."
        }
        PennelError::Http { .. } | PennelError::Cloud { .. } => {
            "The provider could not run the sample test. Check your provider and model."
        }
        PennelError::Network { .. } => {
            "The provider is unreachable. Check your connection and try again."
        }
        PennelError::InvalidResponse { .. } => {
            "The selected model returned no usable answer to the sample test."
        }
        PennelError::ModelNotDownloaded => "Download the selected speech model first.",
        PennelError::NotSignedIn => "Sign in to Pennel Cloud under Settings → Account first.",
        PennelError::VaultLocked => "Unlock Pennel Cloud under Settings → Account first.",
        PennelError::Cancelled => "The sample test was cancelled. Try again.",
        PennelError::Storage { .. } => {
            "The local model could not be read. Check the model and try again."
        }
        PennelError::NotFound => {
            "The selected model is unavailable. Check your provider and model."
        }
        PennelError::NoSpeech => "The selected speech model returned no text.",
        PennelError::UnsupportedProcessor => crate::cpu_support::UNSUPPORTED_PROCESSOR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::{
        LanguageModelMessage, ModelChoice, ProviderAccount, ProviderKind, SystemLanguageModel,
        SystemTranscriber,
    };

    const SAMPLE: &[u8] = include_bytes!("../tests/fixtures/hello.wav");
    const PRIVATE_TERM: &str = "synthetic-private-vocabulary-must-not-leave";
    const PRIVATE_DRAFT: &str = "synthetic-private-pad-must-not-leave";

    fn configured(server: &mockito::Server) -> (tempfile::TempDir, Arc<PennelEngine>, ApiKeys) {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers = vec![
            ProviderAccount {
                id: "dictation-test".into(),
                kind: ProviderKind::Custom,
                name: "Dictation example".into(),
                base_url: format!("{}/dictation", server.url()),
            },
            ProviderAccount {
                id: "meeting-test".into(),
                kind: ProviderKind::Custom,
                name: "Meeting example".into(),
                base_url: format!("{}/meeting", server.url()),
            },
            ProviderAccount {
                id: "text-test".into(),
                kind: ProviderKind::Custom,
                name: "Text example".into(),
                base_url: format!("{}/text", server.url()),
            },
        ];
        settings.dictation_stt = ModelChoice {
            provider_id: "dictation-test".into(),
            model: "dictation-model".into(),
        };
        settings.meeting_stt = ModelChoice {
            provider_id: "meeting-test".into(),
            model: "meeting-model".into(),
        };
        settings.llm = ModelChoice {
            provider_id: "text-test".into(),
            model: "text-model".into(),
        };
        settings.language = Some("nl".into());
        settings.dictionary = vec![PRIVATE_TERM.into()];
        engine.update_settings(settings).unwrap();
        engine.save_dictation_pad(PRIVATE_DRAFT.into()).unwrap();
        let keys = ApiKeys {
            by_provider: HashMap::from([
                ("dictation-test".into(), "synthetic-dictation-key".into()),
                ("meeting-test".into(), "synthetic-meeting-key".into()),
                ("text-test".into(), "synthetic-text-key".into()),
            ]),
        };
        (dir, engine, keys)
    }

    #[test]
    fn speech_test_uses_its_selected_slot_and_only_the_bundled_sample() {
        for (task, path, model, key) in [
            (
                ProviderTask::DictationStt,
                "/dictation/audio/transcriptions",
                "dictation-model",
                "synthetic-dictation-key",
            ),
            (
                ProviderTask::MeetingStt,
                "/meeting/audio/transcriptions",
                "meeting-model",
                "synthetic-meeting-key",
            ),
        ] {
            let mut server = mockito::Server::new();
            let (_dir, engine, keys) = configured(&server);
            let settings = engine.settings();
            let response = server
                .mock("POST", path)
                .match_header("authorization", format!("Bearer {key}").as_str())
                .with_body_from_request(move |request| {
                    let body = request.body().unwrap();
                    assert!(body.windows(SAMPLE.len()).any(|window| window == SAMPLE));
                    let text = String::from_utf8_lossy(body);
                    assert!(text.contains(&format!("name=\"model\"\r\n\r\n{model}\r\n")));
                    assert!(text.contains("name=\"language\"\r\n\r\nnl\r\n"));
                    assert!(!text.contains("name=\"prompt\""));
                    assert!(!text.contains(PRIVATE_TERM));
                    assert!(!text.contains(PRIVATE_DRAFT));
                    br#"{"text":"Synthetic transcription"}"#.to_vec()
                })
                .create();
            let result = test_provider_task(engine.clone(), task, keys).unwrap();
            assert!(result.verified);
            assert!(!result.message.contains("Synthetic transcription"));
            assert_eq!(engine.settings(), settings);
            assert_eq!(engine.dictation_pad_text(), PRIVATE_DRAFT);
            assert!(engine.history(10).unwrap().is_empty());
            response.assert();
        }
    }

    #[test]
    fn text_test_sends_only_a_fixed_sample_and_does_not_change_user_data() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server);
        let settings = engine.settings();
        let response = server.mock("POST", "/text/chat/completions")
            .match_header("authorization", "Bearer synthetic-text-key")
            .with_body_from_request(|request| {
                let body: serde_json::Value = serde_json::from_slice(request.body().unwrap()).unwrap();
                assert_eq!(body["model"], "text-model");
                assert_eq!(body["messages"], serde_json::json!([
                    {"role":"system", "content":"This is a synthetic Pennel connection test. Reply with only the word OK."},
                    {"role":"user", "content":"Pennel example: the meeting starts at ten."}
                ]));
                let serialized = body.to_string();
                assert!(!serialized.contains(PRIVATE_TERM));
                assert!(!serialized.contains(PRIVATE_DRAFT));
                br#"{"choices":[{"message":{"content":"OK"}}]}"#.to_vec()
            }).create();
        let result = test_provider_task(engine.clone(), ProviderTask::Llm, keys).unwrap();
        assert!(result.verified);
        assert_eq!(engine.settings(), settings);
        assert_eq!(engine.dictation_pad_text(), PRIVATE_DRAFT);
        assert!(engine.history(10).unwrap().is_empty());
        response.assert();
    }

    #[test]
    fn an_empty_transcription_is_not_verified() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server);
        let response = server
            .mock("POST", "/dictation/audio/transcriptions")
            .with_body(r#"{"text":"  "}"#)
            .create();
        let result = test_provider_task(engine, ProviderTask::DictationStt, keys).unwrap();
        assert!(!result.verified);
        assert!(result.message.contains("returned no text"));
        response.assert();
    }

    #[test]
    fn an_empty_text_response_is_not_verified() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server);
        let response = server
            .mock("POST", "/text/chat/completions")
            .with_body(r#"{"choices":[{"message":{"content":"  "}}]}"#)
            .create();
        let result = test_provider_task(engine, ProviderTask::Llm, keys).unwrap();
        assert!(!result.verified);
        response.assert();
    }

    #[test]
    fn failed_provider_responses_do_not_echo_server_content_or_use_another_provider() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server);
        let response = server
            .mock("POST", "/dictation/audio/transcriptions")
            .with_status(401)
            .with_body(
                serde_json::json!({"error":{"message":format!("{PRIVATE_TERM} {PRIVATE_DRAFT}")}})
                    .to_string(),
            )
            .create();
        let other = server
            .mock("POST", "/meeting/audio/transcriptions")
            .expect(0)
            .create();
        let result = test_provider_task(engine, ProviderTask::DictationStt, keys).unwrap();
        assert!(!result.verified);
        assert!(!result.message.contains(PRIVATE_TERM));
        assert!(!result.message.contains(PRIVATE_DRAFT));
        response.assert();
        other.assert();
    }

    #[test]
    fn speech_test_uses_the_selected_system_transcriber_without_remote_fallback() {
        struct Capture(Mutex<Vec<(usize, u32, Option<String>)>>);
        impl SystemTranscriber for Capture {
            fn transcribe(
                &self,
                samples: Vec<f32>,
                rate: u32,
                language: Option<String>,
                vocabulary: Vec<String>,
            ) -> Result<String, PennelError> {
                assert!(
                    vocabulary.is_empty(),
                    "provider qualification must not send private terms"
                );
                self.0.lock().unwrap().push((samples.len(), rate, language));
                Ok("Synthetic local example".into())
            }
        }
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server);
        let capture = Arc::new(Capture(Mutex::new(Vec::new())));
        engine.set_system_transcriber(capture.clone());
        let mut settings = engine.settings();
        settings.providers[0].kind = ProviderKind::AppleSpeech;
        engine.update_settings(settings).unwrap();
        let remote = server
            .mock("POST", mockito::Matcher::Any)
            .expect(0)
            .create();
        assert!(
            test_provider_task(engine.clone(), ProviderTask::DictationStt, keys)
                .unwrap()
                .verified
        );
        let calls = capture.0.lock().unwrap();
        let (expected, _) = crate::wav::decode_wav(SAMPLE).unwrap();
        assert_eq!(*calls, vec![(expected.len(), 16000, Some("nl".into()))]);
        assert!(engine.history(10).unwrap().is_empty());
        remote.assert();
    }

    #[test]
    fn text_test_uses_the_selected_system_model_and_hides_native_error_details() {
        struct Model;
        impl SystemLanguageModel for Model {
            fn complete(
                &self,
                messages: Vec<LanguageModelMessage>,
                timeout_secs: u64,
            ) -> Result<String, PennelError> {
                assert_eq!(messages.len(), 2);
                assert!(timeout_secs <= 30);
                Err(PennelError::InvalidResponse {
                    message: PRIVATE_TERM.into(),
                })
            }
        }
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server);
        engine.set_system_language_model(Arc::new(Model));
        let mut settings = engine.settings();
        settings.providers[2].kind = ProviderKind::AppleIntelligence;
        engine.update_settings(settings).unwrap();
        let remote = server
            .mock("POST", mockito::Matcher::Any)
            .expect(0)
            .create();
        let result = test_provider_task(engine, ProviderTask::Llm, keys).unwrap();
        assert!(!result.verified);
        assert!(!result.message.contains(PRIVATE_TERM));
        remote.assert();
    }

    #[test]
    fn missing_api_key_is_reported_without_sending_a_request() {
        let mut server = mockito::Server::new();
        let (_dir, engine, _keys) = configured(&server);
        let mut settings = engine.settings();
        settings.providers[2].kind = ProviderKind::Groq;
        engine.update_settings(settings).unwrap();
        let remote = server
            .mock("POST", mockito::Matcher::Any)
            .expect(0)
            .create();
        let result = test_provider_task(
            engine,
            ProviderTask::Llm,
            ApiKeys {
                by_provider: HashMap::new(),
            },
        )
        .unwrap();
        assert!(!result.verified);
        assert!(result.message.contains("API key"));
        remote.assert();
    }
}
