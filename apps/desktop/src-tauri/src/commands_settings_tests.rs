use super::persist_settings_and_publish;
use crate::state::Shared;
use pennel_core::{
    ApiKeys, DecisionQuestion, DecisionQuestionKind, DecisionRequest, InferenceCancellation,
    InferenceProtocol, LanguageModelMessage, PennelError, ProviderKind, Settings,
    SuggestionAutomation, SummaryAutomation, SystemLanguageModel,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const SUMMARY: &str = "## Summary\n\nThe team approved a Friday launch.";
const WAIT: Duration = Duration::from_secs(10);

struct HeldLanguageModel {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl SystemLanguageModel for HeldLanguageModel {
    fn complete(&self, _: Vec<LanguageModelMessage>, _: u64) -> Result<String, PennelError> {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv_timeout(WAIT).unwrap();
        Ok(SUMMARY.into())
    }
}

struct SummaryObservation {
    shared: Arc<Shared>,
    id: String,
    result: Result<Option<String>, PennelError>,
    before_epoch: u64,
    after_epoch: u64,
    _root: tempfile::TempDir,
}

fn no_keys() -> ApiKeys {
    ApiKeys {
        by_provider: Default::default(),
    }
}

fn save(shared: &Shared, settings: Settings) -> Result<(), String> {
    // Only AppHandle-dependent publication/refresh effects are replaced. Validation,
    // mutation guards, durable settings, policy revocation and core tokens stay real.
    persist_settings_and_publish(settings, shared, |_| Ok(()), || Ok(()))
}

fn summary_during_save(change: impl FnOnce(&mut Settings)) -> SummaryObservation {
    let root = tempfile::tempdir().unwrap();
    let shared = Shared::testing(root.path().to_path_buf());
    let mut settings = shared.settings();
    let account =
        pennel_core::new_account(ProviderKind::AppleIntelligence, settings.providers.clone());
    settings.llm = pennel_core::ModelChoice {
        provider_id: account.id.clone(),
        model: "apple-intelligence".into(),
    };
    settings.providers.push(account);
    let mut decision = pennel_core::new_account(ProviderKind::Custom, settings.providers.clone());
    decision.base_url = "http://127.0.0.1:9/v1".into();
    let mut config =
        pennel_core::new_inference_config(decision.id.clone(), InferenceProtocol::Perplexity);
    config.model = "fixture-model".into();
    settings.decision = Some(config);
    settings.providers.push(decision);
    settings.summary_automation = SummaryAutomation::Automatic;
    settings.suggestion_automation = SuggestionAutomation::Automatic;
    settings.suggestion_language = Some("en".into());
    shared.engine.update_settings(settings).unwrap();
    let session = shared
        .engine
        .clone()
        .start_meeting("Settings fixture".into(), no_keys())
        .unwrap();
    session
        .set_notes("The team approved a Friday launch.".into())
        .unwrap();
    session.stop().unwrap();
    let id = session.id();
    let (entered, await_entered) = mpsc::channel();
    let (release, await_release) = mpsc::channel();
    shared
        .engine
        .set_system_language_model(Arc::new(HeldLanguageModel {
            entered,
            release: Mutex::new(await_release),
        }));
    let engine = shared.engine.clone();
    let running_id = id.clone();
    let work = std::thread::spawn(move || {
        engine.summarize_meeting_automatically(running_id, "algemeen".into(), no_keys())
    });
    await_entered.recv_timeout(WAIT).unwrap();
    let before_epoch = shared.engine.inference_epoch();
    let mut settings = shared.settings();
    change(&mut settings);
    let saved = save(&shared, settings);
    let after_epoch = shared.engine.inference_epoch();
    // Always release the provider before checking assertions, including failed saves.
    release.send(()).unwrap();
    let result = work.join().unwrap();
    saved.unwrap();
    SummaryObservation {
        shared,
        id,
        result,
        before_epoch,
        after_epoch,
        _root: root,
    }
}

fn assert_original_summary_saved(observation: SummaryObservation) {
    assert_eq!(observation.result, Ok(Some(SUMMARY.into())));
    assert_eq!(
        observation
            .shared
            .engine
            .meeting_summary(observation.id)
            .unwrap()
            .markdown,
        SUMMARY
    );
    assert_eq!(observation.after_epoch, observation.before_epoch);
}

#[test]
fn suggestion_policy_save_preserves_inflight_automatic_summary() {
    assert_original_summary_saved(summary_during_save(|settings| {
        settings.suggestion_automation = SuggestionAutomation::Manual;
    }));
}

#[test]
fn suggestion_fallback_language_save_preserves_inflight_automatic_summary() {
    assert_original_summary_saved(summary_during_save(|settings| {
        settings.suggestion_language = Some("nl".into());
    }));
}

#[test]
fn own_policy_revocation_cancels_automatic_summary_and_consumes_its_attempt() {
    let observation = summary_during_save(|settings| {
        settings.summary_automation = SummaryAutomation::Manual;
    });
    assert_eq!(observation.result, Err(PennelError::Cancelled));
    assert!(
        observation
            .shared
            .engine
            .meeting_summary(observation.id.clone())
            .unwrap()
            .markdown
            .is_empty()
    );
    assert_eq!(observation.after_epoch, observation.before_epoch);
    let mut settings = observation.shared.settings();
    settings.summary_automation = SummaryAutomation::Automatic;
    save(&observation.shared, settings).unwrap();
    assert_eq!(
        observation.shared.engine.summarize_meeting_automatically(
            observation.id,
            "algemeen".into(),
            no_keys()
        ),
        Ok(None),
        "revoked attempts must not retry automatically"
    );
}

#[test]
fn provider_model_and_inference_config_saves_still_cancel_automatic_summary() {
    let changes: [fn(&mut Settings); 4] = [
        |settings| {
            settings
                .providers
                .iter_mut()
                .find(|provider| provider.id == settings.llm.provider_id)
                .unwrap()
                .base_url = "https://changed.example/v1".into();
        },
        |settings| settings.llm.model = "replacement-model".into(),
        |settings| settings.decision.as_mut().unwrap().timeout_secs += 1,
        |settings| {
            settings.suggestion_automation = SuggestionAutomation::Manual;
            settings.decision.as_mut().unwrap().timeout_secs += 1;
        },
    ];
    for (case, change) in changes.into_iter().enumerate() {
        let observation = summary_during_save(change);
        assert_eq!(
            observation.result,
            Err(PennelError::Cancelled),
            "case {case}"
        );
        assert!(
            observation
                .shared
                .engine
                .meeting_summary(observation.id)
                .unwrap()
                .markdown
                .is_empty(),
            "case {case}"
        );
        assert!(
            observation.after_epoch > observation.before_epoch,
            "case {case}"
        );
    }
}

#[test]
fn summary_policy_save_preserves_a_real_registered_inflight_decision() {
    let root = tempfile::tempdir().unwrap();
    let shared = Shared::testing(root.path().to_path_buf());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut settings = shared.settings();
    let mut provider = pennel_core::new_account(ProviderKind::Custom, settings.providers.clone());
    provider.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let mut config =
        pennel_core::new_inference_config(provider.id.clone(), InferenceProtocol::Perplexity);
    config.model = "fixture-model".into();
    settings.decision = Some(config);
    settings.providers.push(provider.clone());
    settings.summary_automation = SummaryAutomation::Automatic;
    settings.suggestion_automation = SuggestionAutomation::Automatic;
    shared.engine.update_settings(settings).unwrap();
    let (entered, await_entered) = mpsc::channel();
    let (release, await_release) = mpsc::channel();
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + WAIT;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "decision request did not arrive"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_read_timeout(Some(WAIT)).unwrap();
        stream.set_write_timeout(Some(WAIT)).unwrap();
        let mut request = BufReader::new(&mut stream);
        let mut first = String::new();
        request.read_line(&mut first).unwrap();
        assert_eq!(first, "POST /v1/decisions HTTP/1.1\r\n");
        let mut length = None;
        loop {
            let mut header = String::new();
            assert!(
                request.read_line(&mut header).unwrap() > 0,
                "incomplete request headers"
            );
            if header == "\r\n" {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.unwrap()];
        request.read_exact(&mut body).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["model"], "fixture-model");
        assert_eq!(body["state"], "Synthetic settings request.");
        entered.send(()).unwrap();
        await_release.recv_timeout(WAIT).unwrap();
        let response =
            r#"{"model":"fixture-model","answers":{"sample":{"type":"noul","noul":0.99}}}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
    });
    let token = InferenceCancellation::new();
    let engine = shared.engine.clone();
    let running_token = token.clone();
    let work = std::thread::spawn(move || {
        engine.decide(
            DecisionRequest {
                state: "Synthetic settings request.".into(),
                questions: vec![DecisionQuestion {
                    id: "sample".into(),
                    kind: DecisionQuestionKind::Binary,
                    instructions: "Does the sample contain text?".into(),
                    options: vec![],
                }],
            },
            running_token,
            ApiKeys {
                by_provider: [(provider.id, "synthetic-key".into())].into(),
            },
        )
    });
    await_entered.recv_timeout(WAIT).unwrap();
    let before_epoch = shared.engine.inference_epoch();
    let mut settings = shared.settings();
    settings.summary_automation = SummaryAutomation::Manual;
    let saved = save(&shared, settings);
    let after_epoch = shared.engine.inference_epoch();
    let cancelled = token.is_cancelled();
    release.send(()).unwrap();
    let result = work.join().unwrap();
    server.join().unwrap();
    saved.unwrap();
    // This is a real registered Core decision, not an admitted meeting Suggestion:
    // the production qualification registry remains empty and is never bypassed.
    assert!(
        !cancelled,
        "Summary policy must not cancel unrelated registered work"
    );
    assert_eq!(after_epoch, before_epoch);
    let result = result.unwrap();
    assert_eq!(result.answers[0].value, Some(0.99));
    assert_eq!(
        result.identity.actual_model.as_deref(),
        Some("fixture-model")
    );
}

#[test]
fn configuration_mutation_keeps_admission_closed_until_native_publication_finishes() {
    let root = tempfile::tempdir().unwrap();
    let shared = Shared::testing(root.path().to_path_buf());
    let mut settings = shared.settings();
    let mut provider = pennel_core::new_account(ProviderKind::Custom, settings.providers.clone());
    provider.base_url = "http://127.0.0.1:9/v1".into();
    let mut config =
        pennel_core::new_inference_config(provider.id.clone(), InferenceProtocol::Perplexity);
    config.model = "fixture-model".into();
    settings.providers.push(provider);
    settings.decision = Some(config);
    shared.engine.update_settings(settings).unwrap();
    let mut settings = shared.settings();
    settings.decision.as_mut().unwrap().timeout_secs += 1;
    let request = || DecisionRequest {
        state: "Synthetic settings request.".into(),
        questions: vec![DecisionQuestion {
            id: "sample".into(),
            kind: DecisionQuestionKind::Binary,
            instructions: "Does the sample contain text?".into(),
            options: vec![],
        }],
    };
    persist_settings_and_publish(
        settings,
        &shared,
        |_| {
            let token = InferenceCancellation::new();
            assert_eq!(
                shared.engine.decide(request(), token.clone(), no_keys()),
                Err(PennelError::Cancelled)
            );
            assert!(
                token.is_cancelled(),
                "the real registration must see the native mutation"
            );
            Ok(())
        },
        || {
            let token = InferenceCancellation::new();
            assert!(matches!(
                shared.engine.decide(request(), token.clone(), no_keys()),
                Err(PennelError::MissingApiKey { .. })
            ));
            assert!(
                !token.is_cancelled(),
                "admission must reopen before verification refresh"
            );
            Ok(())
        },
    )
    .unwrap();
}
