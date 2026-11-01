use super::*;
use mockito::Matcher;

fn config(protocol: InferenceProtocol) -> InferenceConfig {
    let mut config = new_inference_config("example".into(), protocol);
    config.model = "example-model".into();
    config.upstream = router(protocol).then(|| "example/default".into());
    config
}

fn request() -> DecisionRequest {
    DecisionRequest {
        state: "Synthetic decision example".into(),
        questions: vec![DecisionQuestion {
            id: "route".into(),
            kind: DecisionQuestionKind::Choice,
            instructions: "Which label describes this example?".into(),
            options: vec![
                DecisionOption {
                    id: "b".into(),
                    description: "Second".into(),
                },
                DecisionOption {
                    id: "a".into(),
                    description: "First".into(),
                },
            ],
        }],
    }
}

fn response() -> Value {
    json!({"model":"example-model", "provider":"Example", "id":"example-request", "answers": {
        "route": {"type":"choice", "choice":"a", "probabilities":{"a":0.7,"b":0.3},"confidence":0.4}
    }, "usage":{"input_tokens":10,"output_tokens":0,"cost":0.0001}})
}

#[test]
fn settings_helpers_do_not_select_a_model_and_protocol_capabilities_are_distinct() {
    let configured = new_inference_config("old-provider".into(), InferenceProtocol::Mercury);
    assert_eq!(configured.model, "");
    assert!(configured.expected_model.is_none());
    let all = inference_protocols();
    assert_eq!(all.len(), 8);
    assert!(
        all.iter()
            .filter(|p| !p.managed)
            .all(|p| p.decision != p.reranking)
    );
    let managed = all
        .iter()
        .find(|p| p.protocol == InferenceProtocol::PennelCloud)
        .unwrap();
    assert!(managed.managed && managed.decision && managed.reranking);
    assert!(!managed.extraction && !managed.requires_key);
    assert_eq!(
        managed.allowed_provider_kinds,
        vec![crate::ProviderKind::PennelCloud]
    );
    assert_eq!(
        new_inference_config("cloud".into(), InferenceProtocol::PennelCloud).model,
        "pennel-auto"
    );
    assert!(
        all.iter()
            .filter(|p| p.extraction)
            .all(|p| p.protocol == InferenceProtocol::Mercury)
    );
}

#[test]
fn managed_configuration_has_fixed_routing_and_safe_endpoints() {
    let mut configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
    for (capability, suffix) in [
        (InferenceCapability::Decision, "decisions"),
        (InferenceCapability::Reranking, "rerank"),
    ] {
        assert_eq!(
            validate_config(&configured, "https://api.pennel.app", capability).unwrap(),
            format!("https://api.pennel.app/v1/{suffix}")
        );
        assert!(validate_config(&configured, "http://example.com", capability).is_err());
        assert!(validate_config(&configured, "https://secret@example.com", capability).is_err());
    }
    configured.upstream = Some("".into());
    assert!(
        validate_config(
            &configured,
            "https://api.pennel.app",
            InferenceCapability::Decision
        )
        .is_err()
    );
    configured.upstream = None;
    configured.model = "user-selected-route".into();
    assert!(
        validate_config(
            &configured,
            "https://api.pennel.app",
            InferenceCapability::Decision
        )
        .is_err()
    );
}

#[test]
fn endpoint_resolution_preserves_explicit_origin_and_rejects_unsafe_urls() {
    use InferenceProtocol::*;
    for (protocol, base, endpoint) in [
        (
            OpenRouterDecisions,
            "https://eu.openrouter.ai/api/v1",
            "https://eu.openrouter.ai/api/alpha/decisions",
        ),
        (
            OpenRouterSystemOne,
            "https://openrouter.ai/api/v1",
            "https://openrouter.ai/api/v1/systemone",
        ),
        (
            TypeSafe,
            "https://api.typesafe.ai/v1",
            "https://api.typesafe.ai/v1/systemone",
        ),
        (
            Mercury,
            "https://api.inceptionlabs.ai/v1/decisions",
            "https://api.inceptionlabs.ai/v1/decisions",
        ),
        (
            Perplexity,
            "https://api.perplexity.ai/v1/",
            "https://api.perplexity.ai/v1/decisions",
        ),
        (
            LocalDecider,
            "http://[::1]:8080/v1",
            "http://[::1]:8080/v1/decisions",
        ),
    ] {
        assert_eq!(
            validate_config(&config(protocol), base, InferenceCapability::Decision).unwrap(),
            endpoint
        );
    }
    for base in [
        "http://example.com/v1",
        "http://127.0.0.1.example.com",
        "https://secret@example.com/v1",
        "https://example.com?key=private",
        "https://example.com/#fragment",
        "file:///tmp/example",
        "https://",
    ] {
        assert!(
            validate_config(&config(Perplexity), base, InferenceCapability::Decision).is_err(),
            "{base}"
        );
    }
    assert!(
        validate_config(
            &config(LocalDecider),
            "https://example.com",
            InferenceCapability::Decision
        )
        .is_err()
    );
    assert!(
        validate_config(
            &config(Tei),
            "https://example.com",
            InferenceCapability::Decision
        )
        .is_err()
    );
}

#[test]
fn openrouter_origins_cannot_use_a_direct_protocol_to_bypass_route_policy() {
    for host in [
        "openrouter.ai",
        "eu.openrouter.ai",
        "us.openrouter.ai",
        "OPENROUTER.AI.",
    ] {
        for protocol in [
            InferenceProtocol::TypeSafe,
            InferenceProtocol::Perplexity,
            InferenceProtocol::Mercury,
            InferenceProtocol::LocalDecider,
            InferenceProtocol::Tei,
        ] {
            let capability = if protocol == InferenceProtocol::Tei {
                InferenceCapability::Reranking
            } else {
                InferenceCapability::Decision
            };
            let error = validate_config(
                &config(protocol),
                &format!("https://{host}/api/v1"),
                capability,
            )
            .unwrap_err();
            assert!(error.to_string().contains("Choose an OpenRouter protocol"));
        }
    }
    assert!(
        validate_config(
            &config(InferenceProtocol::OpenRouterDecisions),
            "https://eu.openrouter.ai/api/v1",
            InferenceCapability::Decision
        )
        .is_ok()
    );
    assert!(
        validate_config(
            &config(InferenceProtocol::TypeSafe),
            "https://explicit-custom.example/v1",
            InferenceCapability::Decision
        )
        .is_ok()
    );
}

#[test]
fn all_decision_adapters_use_their_dedicated_path_and_keep_actual_metadata() {
    use InferenceProtocol::*;
    for (protocol, path) in [
        (OpenRouterDecisions, "/api/alpha/decisions"),
        (OpenRouterSystemOne, "/api/v1/systemone"),
        (TypeSafe, "/v1/systemone"),
        (Perplexity, "/v1/decisions"),
        (Mercury, "/v1/decisions"),
        (LocalDecider, "/v1/decisions"),
    ] {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", path)
            .match_header("authorization", "Bearer synthetic-key")
            .match_body(Matcher::PartialJson(
                json!({"model":"example-model","questions":{"route":{"type":"choice"}}}),
            ))
            .with_status(200)
            .with_body(response().to_string())
            .create();
        let result = decide(
            &config(protocol),
            &format!(
                "{}/{}",
                server.url(),
                if router(protocol) { "api/v1" } else { "v1" }
            ),
            Some("synthetic-key"),
            request(),
            InferenceCancellation::new(),
        )
        .unwrap();
        assert_eq!(
            result.identity.actual_model.as_deref(),
            Some("example-model")
        );
        assert_eq!(
            result.identity.request_id.as_deref(),
            Some("example-request")
        );
        assert_eq!(result.answers[0].choice.as_deref(), Some("a"));
        assert_eq!(result.answers[0].probabilities[0].option_id, "b");
        assert_eq!(result.usage.input_tokens, Some(10));
        assert_eq!(result.usage.billed_cost_usd, Some(0.0001));
        mock.assert();
    }
}

#[test]
fn openrouter_requires_a_fixed_upstream_and_sends_no_fallback_or_data_collection() {
    let mut configured = config(InferenceProtocol::OpenRouterDecisions);
    configured.upstream = None;
    assert!(
        validate_config(
            &configured,
            "https://openrouter.ai/api/v1",
            InferenceCapability::Decision
        )
        .is_err()
    );
    configured.upstream = Some("example/default".into());
    let mut server = mockito::Server::new();
    let mock = server.mock("POST", "/api/alpha/decisions")
        .match_body(Matcher::PartialJson(json!({"provider":{"only":["example/default"],"allow_fallbacks":false,"data_collection":"deny","zdr":true}})))
        .with_status(200).with_body(response().to_string()).create();
    decide(
        &configured,
        &server.url(),
        Some("synthetic-key"),
        request(),
        InferenceCancellation::new(),
    )
    .unwrap();
    mock.assert();
    let body = String::from_utf8(decision_body(&configured, &request()).unwrap()).unwrap();
    assert!(body.find("\"b\":").unwrap() < body.find("\"a\":").unwrap());
}

#[test]
fn response_question_ids_types_and_probabilities_are_validated_without_recalibration() {
    let question = &request().questions[0];
    for changed in [
        json!({"choice":"other","probabilities":{"a":0.7,"b":0.3}}),
        json!({"choice":"a","probabilities":{"a":0.7}}),
        json!({"choice":"a","probabilities":{"a":0.7,"b":0.3,"extra":0.0}}),
        json!({"choice":"a","probabilities":{"a":0.7,"b":0.1}}),
        json!({"choice":"a","probabilities":{"a":1.1,"b":-0.1}}),
        json!({"choice":"b","probabilities":{"a":0.7,"b":0.3}}),
        json!({"type":"noul","choice":"a","probabilities":{"a":0.7,"b":0.3}}),
        json!({"choice":"a","probabilities":{"a":0.7,"b":0.3},"confidence":2.0}),
    ] {
        assert!(parse_answer(question, &changed, "example").is_err());
    }
    let rounded = json!({"choice":"a","probabilities":{"a":0.7001,"b":0.3001}});
    let parsed = parse_answer(question, &rounded, "example").unwrap();
    assert_eq!(parsed.probabilities[0].probability, 0.3001);
    assert_eq!(parsed.probabilities[1].probability, 0.7001);
    assert_eq!(parsed.confidence, None);
    assert!(parse_unique_json(r#"{"answers":{"q":1,"q":2}}"#).is_err());
    assert!(parse_unique_json(r#"{"p":NaN}"#).is_err());
}

#[test]
fn binary_and_ordinal_scores_keep_their_own_meaning() {
    let mut question = request().questions.remove(0);
    question.kind = DecisionQuestionKind::Binary;
    question.options.clear();
    let answer = parse_answer(&question, &json!({"noul":0.63}), "example").unwrap();
    assert_eq!(answer.value, Some(0.63));
    assert!(answer.probabilities.is_empty());
    question.kind = DecisionQuestionKind::Ordinal;
    question.options = vec![
        DecisionOption {
            id: "low".into(),
            description: "Low".into(),
        },
        DecisionOption {
            id: "high".into(),
            description: "High".into(),
        },
    ];
    let answer = parse_answer(
        &question,
        &json!({"score":0.75,"probabilities":{"0":0.25,"1":0.75}}),
        "example",
    )
    .unwrap();
    assert_eq!(answer.value, Some(0.75));
    assert_eq!(answer.probabilities[0].option_id, "low");
    assert!(
        parse_answer(
            &question,
            &json!({"score":2,"probabilities":{"0":0.25,"1":0.75}}),
            "example"
        )
        .is_err()
    );
}

#[test]
fn ordinal_adapters_reject_a_score_that_contradicts_its_distribution() {
    use InferenceProtocol::*;
    for (protocol, path, base_path) in [
        (OpenRouterDecisions, "/api/alpha/decisions", "/api/v1"),
        (OpenRouterSystemOne, "/api/v1/systemone", "/api/v1"),
        (TypeSafe, "/v1/systemone", "/v1"),
        (Perplexity, "/v1/decisions", "/v1"),
        (Mercury, "/v1/decisions", "/v1"),
        (LocalDecider, "/v1/decisions", "/v1"),
    ] {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", path)
            .with_status(200)
            .with_body(
                json!({
                    "model":"example-model","provider":"Example",
                    "answers":{"route":{"type":"score","score":1,"probabilities":{"0":1,"1":0}}}
                })
                .to_string(),
            )
            .create();
        let mut input = request();
        input.questions[0].kind = DecisionQuestionKind::Ordinal;
        let error = decide(
            &config(protocol),
            &format!("{}{base_path}", server.url()),
            Some("synthetic-key"),
            input,
            InferenceCancellation::new(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("probability-weighted expected level")
        );
        mock.assert();
    }
    let mut question = request().questions.remove(0);
    question.kind = DecisionQuestionKind::Ordinal;
    question.options.push(DecisionOption {
        id: "third".into(),
        description: "Third".into(),
    });
    let response = json!({"score":1.501,"probabilities":{"0":0.1,"1":0.3,"2":0.6}});
    let parsed = parse_answer(&question, &response, "example").unwrap();
    assert_eq!(parsed.value, Some(1.501));
    assert_eq!(parsed.probabilities[2].probability, 0.6);
}

#[test]
fn abstention_is_explicit_and_does_not_erase_the_returned_evidence() {
    let mut question = request().questions.remove(0);
    question.options[0].id = "abstain".into();
    let answer =
        json!({"choice":"abstain","probabilities":{"abstain":0.9,"a":0.1},"confidence":0.8});
    let parsed = parse_answer(&question, &answer, "example").unwrap();
    assert!(parsed.abstained);
    assert_eq!(parsed.choice.as_deref(), Some("abstain"));
    assert_eq!(parsed.probabilities.len(), 2);
    assert!(parse_answer(&request().questions[0], &answer, "example").is_err());
    question.options[0].id = "unknown".into();
    let parsed = parse_answer(
        &question,
        &json!({"choice":"unknown","probabilities":{"unknown":0.9,"a":0.1},"confidence":0.0}),
        "example",
    )
    .unwrap();
    assert!(!parsed.abstained);
    let explicit =
        json!({"choice":"unknown","probabilities":{"unknown":0.9,"a":0.1},"abstained":true});
    assert!(
        parse_answer(&question, &explicit, "example")
            .unwrap()
            .abstained
    );
    let invalid =
        json!({"choice":"unknown","probabilities":{"unknown":0.9,"a":0.1},"abstained":"yes"});
    assert!(parse_answer(&question, &invalid, "example").is_err());
}

#[test]
fn mercury_extraction_uses_unicode_scalar_offsets_and_rejects_hallucinated_spans() {
    let question = DecisionQuestion {
        id: "names".into(),
        kind: DecisionQuestionKind::Extract,
        instructions: "Find the name".into(),
        options: vec![DecisionOption {
            id: "person".into(),
            description: "Person".into(),
        }],
    };
    let state = "🙂 Zoë werkt.";
    let good = json!({"type":"extract","spans":{"person":[{"text":"Zoë","start":2,"end":5,"confidence":0.8}]},"not_stated":{"person":0.1}});
    let result = parse_answer(&question, &good, state).unwrap();
    assert_eq!(result.spans[0].start_char, 2);
    assert_eq!(result.spans[0].end_char, 5);
    let mut wrong = good.clone();
    wrong["spans"]["person"][0]["text"] = json!("Jan");
    assert!(parse_answer(&question, &wrong, state).is_err());
    wrong = good.clone();
    wrong["spans"]["person"][0]["start"] = json!(3);
    assert!(parse_answer(&question, &wrong, state).is_err());
    wrong = good.clone();
    wrong["spans"]["person"]
        .as_array_mut()
        .unwrap()
        .push(good["spans"]["person"][0].clone());
    assert!(parse_answer(&question, &wrong, state).is_err());
    let absent = json!({"spans":{"person":[]},"not_stated":{"person":1.0}});
    assert!(parse_answer(&question, &absent, state).unwrap().abstained);
    let mut req = request();
    req.questions = vec![question];
    assert!(decision_body(&config(InferenceProtocol::Perplexity), &req).is_err());
}

#[test]
fn invalid_or_oversized_input_is_never_sent_or_truncated() {
    let mut configured = config(InferenceProtocol::Perplexity);
    let mut req = request();
    req.questions.push(req.questions[0].clone());
    assert!(decision_body(&configured, &req).is_err());
    req = request();
    configured.max_input_bytes = 100;
    assert!(decision_body(&configured, &req).is_err());
    configured = config(InferenceProtocol::OpenRouterDecisions);
    configured.model = "cloudflare/clef-flash".into();
    req.state = "x".repeat(2001);
    assert!(decision_body(&configured, &req).is_err());
    let no_key = decide(
        &config(InferenceProtocol::Perplexity),
        "http://127.0.0.1:1/v1",
        None,
        request(),
        InferenceCancellation::new(),
    );
    assert!(matches!(no_key, Err(PennelError::MissingApiKey { .. })));
}

#[test]
fn model_and_upstream_identity_are_not_silently_substituted() {
    let mut configured = config(InferenceProtocol::OpenRouterDecisions);
    configured.expected_model = Some("frozen-model".into());
    assert!(identity(&configured, "https://example.com".into(), &response()).is_err());
    configured.expected_model = Some("example-model".into());
    let mut drift = response();
    drift["provider"] = json!("Another provider");
    assert!(identity(&configured, "https://example.com".into(), &drift).is_err());
    assert!(identity(&configured, "https://example.com".into(), &response()).is_ok());
    assert_eq!(usage(&Value::Null).unwrap().billed_cost_usd, None);
    assert!(usage(&json!({"usage":{"cost":-1}})).is_err());
    assert!(usage(&json!({"usage":{"input_tokens":-1}})).is_err());
}

#[test]
fn failed_http_requests_do_not_echo_content_or_follow_redirects() {
    for status in [301, 401, 402, 429, 502] {
        let mut server = mockito::Server::new();
        let target = server
            .mock("POST", "/must-not-run")
            .expect(0)
            .with_status(200)
            .with_body(response().to_string())
            .create();
        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(status)
            .with_header("location", &format!("{}/must-not-run", server.url()))
            .with_body("synthetic private sentence synthetic-key")
            .create();
        let result = decide(
            &config(InferenceProtocol::Perplexity),
            &format!("{}/v1", server.url()),
            Some("synthetic-key"),
            request(),
            InferenceCancellation::new(),
        )
        .unwrap_err();
        assert!(matches!(result, PennelError::Http{status:s,..} if s == status as u16));
        assert!(!result.to_string().contains("synthetic"));
        mock.assert();
        target.assert();
    }
}

#[test]
fn malformed_unknown_and_oversized_response_bodies_are_rejected() {
    for body in [
        "not JSON".into(),
        r#"{"answers":{"route":{},"route":{}}}"#.into(),
        json!({"model":"example-model","answers":{"unknown":{}}}).to_string(),
        " ".repeat(MAX_RESPONSE_BYTES as usize + 1),
    ] {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(200)
            .with_body(body)
            .create();
        assert!(
            decide(
                &config(InferenceProtocol::Perplexity),
                &format!("{}/v1", server.url()),
                Some("synthetic-key"),
                request(),
                InferenceCancellation::new()
            )
            .is_err()
        );
        mock.assert();
    }
}

#[test]
fn cancellation_before_send_never_contacts_a_provider() {
    let cancel = InferenceCancellation::new();
    cancel.cancel();
    assert_eq!(
        decide(
            &config(InferenceProtocol::Perplexity),
            "https://example.invalid/v1",
            Some("synthetic-key"),
            request(),
            cancel
        )
        .unwrap_err(),
        PennelError::Cancelled
    );
}

#[test]
fn cancellation_during_a_request_discards_late_results() {
    let mut server = mockito::Server::new();
    let body = response().to_string();
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let mock = server
        .mock("POST", "/v1/decisions")
        .with_status(200)
        .with_chunked_body(move |writer| {
            let _ = started_tx.send(());
            std::thread::sleep(std::time::Duration::from_millis(600));
            writer.write_all(body.as_bytes())
        })
        .create();
    let cancel = InferenceCancellation::new();
    let trigger = cancel.clone();
    let (cancelled_tx, cancelled_rx) = std::sync::mpsc::sync_channel(1);
    let signal = std::thread::spawn(move || {
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let cancelled_at = std::time::Instant::now();
        trigger.cancel();
        cancelled_tx.send(cancelled_at).unwrap();
    });
    let result = decide(
        &config(InferenceProtocol::Perplexity),
        &format!("{}/v1", server.url()),
        Some("synthetic-key"),
        request(),
        cancel,
    );
    assert_eq!(result.unwrap_err(), PennelError::Cancelled);
    assert!(cancelled_rx.recv().unwrap().elapsed() < std::time::Duration::from_millis(300));
    signal.join().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(650));
    // The already-sent call may finish, but its result has no consumer.
    mock.assert();
}

fn rank_request() -> RerankRequest {
    RerankRequest {
        query: "Find the deadline".into(),
        passages: vec![
            RerankPassage {
                id: "p1".into(),
                text: "The task is due Thursday.".into(),
            },
            RerankPassage {
                id: "p2".into(),
                text: "No date is stated here.".into(),
            },
        ],
    }
}

#[test]
fn tei_preserves_relevance_logits_and_checks_its_served_model() {
    let mut server = mockito::Server::new();
    let info = server
        .mock("GET", "/info")
        .with_status(200)
        .with_body(r#"{"model_id":"example-model","model_sha":"version-one"}"#)
        .create();
    let score = server
        .mock("POST", "/rerank")
        .match_body(Matcher::PartialJson(
            json!({"raw_scores":true,"truncate":false}),
        ))
        .with_status(200)
        .with_body(r#"[{"index":1,"score":-4.2},{"index":0,"score":8.7}]"#)
        .create();
    let result = rerank(
        &config(InferenceProtocol::Tei),
        &server.url(),
        None,
        rank_request(),
        InferenceCancellation::new(),
    )
    .unwrap();
    assert_eq!(result.scores[0].passage_id, "p2");
    assert_eq!(result.scores[0].score, -4.2);
    assert_eq!(
        result.identity.actual_revision.as_deref(),
        Some("version-one")
    );
    assert_eq!(result.usage.input_tokens, None);
    info.assert();
    score.assert();
}

#[test]
fn tei_rejects_unknown_duplicate_and_missing_passages() {
    for body in [
        r#"[{"index":0,"score":1},{"index":0,"score":2}]"#,
        r#"[{"index":0,"score":1},{"index":2,"score":2}]"#,
        r#"[{"index":0,"score":1}]"#,
    ] {
        let mut server = mockito::Server::new();
        let info = server
            .mock("GET", "/info")
            .with_status(200)
            .with_body(r#"{"model_id":"example-model"}"#)
            .create();
        let score = server
            .mock("POST", "/rerank")
            .with_status(200)
            .with_body(body)
            .create();
        assert!(
            rerank(
                &config(InferenceProtocol::Tei),
                &server.url(),
                None,
                rank_request(),
                InferenceCancellation::new()
            )
            .is_err()
        );
        info.assert();
        score.assert();
    }
}

fn managed_fixture(
    server: &mockito::Server,
) -> (ManagedInference, Arc<crate::cloud::tests::MemorySecrets>) {
    use crate::cloud::SecretStore;
    let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
    secrets.set(
        crate::cloud::REFRESH_ACCOUNT.into(),
        Some("synthetic-refresh".into()),
    );
    let session = Arc::new(crate::cloud::account::Session::new(
        server.url(),
        secrets.clone(),
    ));
    (
        ManagedInference {
            generation: session.generation(),
            session,
        },
        secrets,
    )
}

fn managed_refresh(server: &mut mockito::Server) -> mockito::Mock {
    server.mock("POST", "/v1/auth/refresh")
        .match_body(Matcher::Json(json!({"refresh_token":"synthetic-refresh"})))
        .with_body(r#"{"access_token":"synthetic-access","refresh_token":"rotated-refresh","expires_in":900}"#)
        .create()
}

#[test]
fn managed_decisions_preserve_raw_results_and_bind_response_to_request() {
    let mut server = mockito::Server::new();
    let (managed, _) = managed_fixture(&server);
    let refresh = managed_refresh(&mut server);
    let post = server
        .mock("POST", "/v1/decisions")
        .match_header("authorization", "Bearer synthetic-access")
        .with_body_from_request(|request| {
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            assert_eq!(body["model"], "pennel-auto");
            assert!(body.get("provider").is_none());
            let id = body["request_id"].as_str().unwrap();
            assert!(uuid::Uuid::parse_str(id).is_ok());
            let mut value = response();
            value["schema_version"] = json!(1);
            value["request_id"] = json!(id);
            value["model_revision"] = json!("synthetic-revision");
            value.as_object_mut().unwrap().remove("provider");
            value.to_string().into_bytes()
        })
        .create();
    let configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
    let result = decide_managed(
        &configured,
        managed,
        request(),
        InferenceCancellation::new(),
    )
    .unwrap();
    assert_eq!(result.answers[0].probabilities[0].probability, 0.3);
    assert_eq!(result.answers[0].probabilities[1].probability, 0.7);
    assert_eq!(result.identity.requested_model, "pennel-auto");
    assert_eq!(
        result.identity.actual_model.as_deref(),
        Some("example-model")
    );
    assert_eq!(
        result.identity.actual_revision.as_deref(),
        Some("synthetic-revision")
    );
    assert_eq!(result.identity.actual_provider, None);
    refresh.assert();
    post.assert();
}

#[test]
fn managed_reranking_preserves_logits_without_inventing_provider_or_usage() {
    let mut server = mockito::Server::new();
    let (managed, _) = managed_fixture(&server);
    let refresh = managed_refresh(&mut server);
    let info = server.mock("GET", "/info").expect(0).create();
    let post = server.mock("POST", "/v1/rerank")
        .match_header("authorization", "Bearer synthetic-access")
        .with_body_from_request(|request| {
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            assert_eq!(body["model"], "pennel-auto");
            assert_eq!(body["documents"], json!(["The task is due Thursday.", "No date is stated here."]));
            json!({"schema_version":1,"request_id":body["request_id"],"model":"served-reranker","results":[{"index":1,"score":-4.2},{"index":0,"score":8.7}]}).to_string().into_bytes()
        }).create();
    let configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
    let result = rerank_managed(
        &configured,
        managed,
        rank_request(),
        InferenceCancellation::new(),
    )
    .unwrap();
    assert_eq!(
        result.scores[0],
        RerankScore {
            passage_id: "p2".into(),
            score: -4.2
        }
    );
    assert_eq!(result.scores[1].score, 8.7);
    assert_eq!(result.identity.actual_provider, None);
    assert_eq!(result.usage.input_tokens, None);
    assert_eq!(result.usage.billed_cost_usd, None);
    refresh.assert();
    post.assert();
    info.assert();
}

#[test]
fn managed_envelopes_reject_missing_or_changed_identity_and_response_contract() {
    let configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
    for change in [
        json!({"schema_version":2}),
        json!({"request_id":"different"}),
        json!({"model":null}),
    ] {
        let mut server = mockito::Server::new();
        let (managed, _) = managed_fixture(&server);
        let _refresh = managed_refresh(&mut server);
        let post = server
            .mock("POST", "/v1/decisions")
            .with_body_from_request(move |request| {
                let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
                let mut value = response();
                value["schema_version"] = json!(1);
                value["request_id"] = body["request_id"].clone();
                for (key, value_change) in change.as_object().unwrap() {
                    value[key] = value_change.clone();
                }
                value.to_string().into_bytes()
            })
            .create();
        assert!(matches!(
            decide_managed(
                &configured,
                managed,
                request(),
                InferenceCancellation::new()
            ),
            Err(PennelError::InvalidResponse { .. })
        ));
        post.assert();
    }
    for scores in [
        json!([{"index":0,"score":1},{"index":0,"score":2}]),
        json!([{"index":2,"score":1},{"index":0,"score":2}]),
        json!([{"index":0,"score":1}]),
    ] {
        assert!(rerank_scores(&rank_request(), &scores).is_err());
    }
}

#[test]
fn managed_auth_retries_once_and_preserves_the_request_identifier() {
    let mut server = mockito::Server::new();
    let (managed, _) = managed_fixture(&server);
    let _refresh = managed_refresh(&mut server);
    let old_request = Arc::new(std::sync::Mutex::new(None));
    let captured = old_request.clone();
    let denied = server
        .mock("POST", "/v1/decisions")
        .match_header("authorization", "Bearer synthetic-access")
        .with_status(401)
        .with_body_from_request(move |request| {
            *captured.lock().unwrap() = Some(request.body().unwrap().to_vec());
            b"private-error-must-not-escape".to_vec()
        })
        .create();
    let refresh = server.mock("POST", "/v1/auth/refresh")
        .match_body(Matcher::Json(json!({"refresh_token":"rotated-refresh"})))
        .with_body(r#"{"access_token":"replacement-access","refresh_token":"next-refresh","expires_in":900}"#).create();
    let accepted = server
        .mock("POST", "/v1/decisions")
        .match_header("authorization", "Bearer replacement-access")
        .with_body_from_request(move |request| {
            assert_eq!(
                Some(request.body().unwrap().to_vec()),
                *old_request.lock().unwrap()
            );
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            let mut value = response();
            value["schema_version"] = json!(1);
            value["request_id"] = body["request_id"].clone();
            value.to_string().into_bytes()
        })
        .create();
    let configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
    assert!(
        decide_managed(
            &configured,
            managed,
            request(),
            InferenceCancellation::new()
        )
        .is_ok()
    );
    denied.assert();
    refresh.assert();
    accepted.assert();
}

#[test]
fn cancellation_or_account_change_during_refresh_never_posts_content() {
    use crate::cloud::SecretStore;
    for change_account in [false, true] {
        let mut server = mockito::Server::new();
        let (managed, secrets) = managed_fixture(&server);
        let session = managed.session.clone();
        let cancellation = InferenceCancellation::new();
        let change = cancellation.clone();
        let refresh = server.mock("POST", "/v1/auth/refresh").with_body_from_request(move |_| {
            if change_account { session.cancel_login(); } else { change.cancel(); }
            br#"{"access_token":"synthetic-access","refresh_token":"rotated-refresh","expires_in":900}"#.to_vec()
        }).create();
        let post = server.mock("POST", "/v1/decisions").expect(0).create();
        let configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
        assert_eq!(
            decide_managed(&configured, managed, request(), cancellation),
            Err(PennelError::Cancelled)
        );
        if !change_account {
            assert_eq!(
                secrets.get(crate::cloud::REFRESH_ACCOUNT.into()).as_deref(),
                Some("rotated-refresh")
            );
        }
        refresh.assert();
        post.assert();
    }
}

#[test]
fn managed_auth_and_content_never_follow_redirects_or_echo_error_bodies() {
    for redirect_auth in [true, false] {
        let mut server = mockito::Server::new();
        let (managed, _) = managed_fixture(&server);
        let _refresh = (!redirect_auth).then(|| managed_refresh(&mut server));
        let path = if redirect_auth {
            "/v1/auth/refresh"
        } else {
            "/v1/decisions"
        };
        let redirected = server
            .mock("POST", path)
            .with_status(307)
            .with_header("location", &format!("{}/must-not-run", server.url()))
            .with_body("private-content synthetic-refresh synthetic-access")
            .create();
        let target = server.mock("POST", "/must-not-run").expect(0).create();
        let configured = new_inference_config("cloud".into(), InferenceProtocol::PennelCloud);
        let error = decide_managed(
            &configured,
            managed,
            request(),
            InferenceCancellation::new(),
        )
        .unwrap_err();
        assert!(matches!(error, PennelError::Http { status: 307, .. }));
        assert!(!error.to_string().contains("private"));
        assert!(!error.to_string().contains("synthetic"));
        redirected.assert();
        target.assert();
    }
}

#[test]
fn managed_auth_and_content_share_one_deadline() {
    let mut server = mockito::Server::new();
    let (managed, _) = managed_fixture(&server);
    let refresh = server.mock("POST", "/v1/auth/refresh").with_body_from_request(|_| {
        std::thread::sleep(std::time::Duration::from_millis(120));
        br#"{"access_token":"synthetic-access","refresh_token":"rotated-refresh","expires_in":900}"#.to_vec()
    }).create();
    let post = server
        .mock("POST", "/v1/decisions")
        .with_body_from_request(|_| {
            std::thread::sleep(std::time::Duration::from_millis(180));
            b"{}".to_vec()
        })
        .create();
    let cancellation = InferenceCancellation::new();
    let start = std::time::Instant::now();
    let result = managed.send(
        &format!("{}/v1/decisions", server.url()),
        Some(b"{}"),
        &cancellation,
        start + std::time::Duration::from_millis(220),
    );
    assert!(matches!(result, Err(PennelError::Network { .. })));
    assert!(start.elapsed() < std::time::Duration::from_millis(350));
    refresh.assert();
    post.assert();
}
