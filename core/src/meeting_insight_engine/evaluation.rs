//! Pure access to the shipped Suggestions contract, without admission or execution.

use super::*;
use crate::meeting_qualification::{MeetingActualInference, compare_inference_identity};
use serde_json::{Value, json};

pub(crate) struct Prepared {
    source: MeetingTranscriptSnapshot,
    configured: MeetingConfiguredInference,
    config: InferenceConfig,
    language: String,
    windows: Vec<InsightWindow>,
}

impl Prepared {
    pub(crate) fn new(
        source: MeetingTranscriptSnapshot,
        configured: MeetingConfiguredInference,
        language: String,
    ) -> Result<Self, PennelError> {
        let request = qualification_request(configured.clone(), &language);
        request
            .validate()
            .map_err(|_| invalid("The evaluation context is invalid."))?;
        let config = InferenceConfig {
            provider_id: "finished-evaluation".into(),
            protocol: configured.protocol,
            model: configured.model.clone(),
            expected_model: configured.expected_model.clone(),
            upstream: configured.upstream.clone(),
            max_input_bytes: configured.max_input_bytes,
            timeout_secs: configured.timeout_secs,
        };
        if config.protocol == InferenceProtocol::PennelCloud {
            return Err(invalid(
                "Managed inference needs a separate authenticated evaluation contract.",
            ));
        }
        let windows = windows(&source, &config)?;
        Ok(Self {
            source,
            configured,
            config,
            language,
            windows,
        })
    }

    pub(crate) fn export(&self) -> Result<Value, PennelError> {
        let windows = self.windows.iter().enumerate().map(|(index, window)| {
            let body = crate::inference::decision_body(&self.config, &window.request)?;
            let wire_body = String::from_utf8(body).map_err(|_| invalid("The request is not UTF-8."))?;
            Ok(json!({
                "index":index,
                "request":window.request,
                "wire_body_sha256":crate::meeting_utterances::digest(wire_body.as_bytes()),
                "wire_body":wire_body,
                "sources":window.blocks.iter().map(|block|json!({
                    "source":block.source,"start_char":block.start_char,"end_char":block.end_char
                })).collect::<Vec<_>>()
            }))
        }).collect::<Result<Vec<_>, PennelError>>()?;
        Ok(json!({
            "qualification_request":qualification_request(self.configured.clone(),&self.language),
            "snapshot":{"meeting_uid":self.source.meeting_uid,"revision":self.source.revision,
                "transcript_digest":self.source.transcript_digest},
            "windows":windows,
            "production_admission":false,
            "qualification":false
        }))
    }

    pub(crate) fn replay(
        &self,
        index: usize,
        raw: &[u8],
        expected: Option<&MeetingActualInference>,
    ) -> Value {
        let Some(window) = self.windows.get(index) else {
            return json!({"status":"unknown_window","window_index":index});
        };
        let result = match crate::inference::evaluation_decision_response(
            &self.config,
            self.configured.endpoint.clone(),
            &window.request,
            raw,
        ) {
            Ok(result) => result,
            Err(_) => return json!({"status":"invalid_response","window_index":index}),
        };
        let identity_complete = [
            &result.identity.actual_model,
            &result.identity.actual_revision,
            &result.identity.actual_provider,
        ]
        .iter()
        .all(|value| value.as_ref().is_some_and(|text| !text.trim().is_empty()));
        if !identity_complete {
            return json!({"status":"identity_unavailable","window_index":index,"observed_identity":result.identity});
        }
        let Some(expected) = expected else {
            return json!({"status":"identity_unpinned","window_index":index,"observed_identity":result.identity});
        };
        if compare_inference_identity(&self.configured, expected, &result.identity).is_err() {
            return json!({"status":"identity_changed","window_index":index,"observed_identity":result.identity});
        }
        let processing = InsightProcessing {
            identity: result.identity.clone(),
            qualification_id: "evaluation-unqualified".into(),
            language_policy: self.language.clone(),
            task: TASK.into(),
        };
        match decode_candidates(window, &result, &self.source, processing) {
            Ok(candidates) => {
                json!({"status":"ok","window_index":index,"observed_identity":result.identity,
                "result":result,"candidates":candidates,"artifact_attestation":false})
            }
            Err(_) => {
                json!({"status":"invalid_output","window_index":index,"observed_identity":result.identity})
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingUtterance, Segment, Speaker, TranscriptMetadataStatus};

    fn prepared(endpoint: String, protocol: InferenceProtocol) -> Prepared {
        let mut row = MeetingUtterance::new(
            Segment {
                start_ms: 0,
                end_ms: 1000,
                speaker: Speaker::Others,
                text: "Zoë will prepare the invented café demo 😀.".into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        );
        row.metadata.utterance_id = "a5bcb9c6-12e3-4b3b-b7d9-6d54d540d405".into();
        Prepared::new(
            MeetingTranscriptSnapshot {
                meeting_uid: "a5bcb9c6-12e3-4b3b-b7d9-6d54d540d406".into(),
                revision: 1,
                transcript_digest: "a".repeat(64),
                metadata_digest: None,
                metadata_status: TranscriptMetadataStatus::Valid,
                utterances: vec![row],
            },
            MeetingConfiguredInference {
                protocol,
                endpoint,
                model: "fixture-model".into(),
                upstream: None,
                expected_model: Some("fixture-model".into()),
                max_input_bytes: 16000,
                timeout_secs: 30,
            },
            "mixed".into(),
        )
        .unwrap()
    }

    fn actual() -> MeetingActualInference {
        MeetingActualInference {
            model: "fixture-model".into(),
            revision: "fixture-revision".into(),
            provider: "fixture-provider".into(),
            artifact_sha256: "a".repeat(64),
        }
    }

    fn response(request: &DecisionRequest) -> Value {
        let mut answers = serde_json::Map::new();
        for q in &request.questions {
            if q.kind == DecisionQuestionKind::Extract {
                answers.insert(q.id.clone(),json!({"type":"extract",
                    "spans":q.options.iter().map(|o|(o.id.clone(),json!([]))).collect::<serde_json::Map<_,_>>(),
                    "not_stated":q.options.iter().map(|o|(o.id.clone(),json!(1.0))).collect::<serde_json::Map<_,_>>() }));
            } else {
                let choice = if q.id == "action" {
                    "confirmed"
                } else {
                    "none"
                };
                answers.insert(q.id.clone(),json!({"type":"choice","choice":choice,
                    "probabilities":q.options.iter().map(|o|(o.id.clone(),json!(if o.id==choice {1.0}else{0.0}))).collect::<serde_json::Map<_,_>>() }));
            }
        }
        json!({"model":"fixture-model","model_revision":"fixture-revision","provider":"fixture-provider","answers":answers})
    }

    #[test]
    fn exported_bytes_match_captured_production_transport_and_decoding() {
        let mut server = mockito::Server::new();
        let prepared = prepared(
            format!("{}/v1/decisions", server.url()),
            InferenceProtocol::LocalDecider,
        );
        let export = prepared.export().unwrap();
        let raw = response(&prepared.windows[0].request).to_string();
        let mock = server
            .mock("POST", "/v1/decisions")
            .match_body(export["windows"][0]["wire_body"].as_str().unwrap())
            .with_body(&raw)
            .with_status(200)
            .expect(1)
            .create();
        let result = crate::inference::decide(
            &prepared.config,
            &prepared.configured.endpoint,
            None,
            prepared.windows[0].request.clone(),
            InferenceCancellation::for_priority(crate::InferencePriority::ForegroundQuestion),
        )
        .unwrap();
        let processing = InsightProcessing {
            identity: result.identity.clone(),
            qualification_id: "evaluation-unqualified".into(),
            language_policy: "mixed".into(),
            task: TASK.into(),
        };
        let production =
            decode_candidates(&prepared.windows[0], &result, &prepared.source, processing).unwrap();
        let replay = prepared.replay(0, raw.as_bytes(), Some(&actual()));
        assert_eq!(replay["status"], "ok");
        assert_eq!(
            replay["candidates"],
            serde_json::to_value(production).unwrap()
        );
        assert_eq!(replay["result"], serde_json::to_value(result).unwrap());
        assert_eq!(replay["candidates"][0]["owner"], Value::Null);
        mock.assert();
    }

    #[test]
    fn identity_missing_unpinned_and_drift_are_distinct_blockers() {
        let p = prepared(
            "http://127.0.0.1:8080/v1/decisions".into(),
            InferenceProtocol::LocalDecider,
        );
        let raw = response(&p.windows[0].request);
        assert_eq!(
            p.replay(0, raw.to_string().as_bytes(), None)["status"],
            "identity_unpinned"
        );
        let mut incomplete = raw.clone();
        incomplete.as_object_mut().unwrap().remove("model_revision");
        assert_eq!(
            p.replay(0, incomplete.to_string().as_bytes(), Some(&actual()))["status"],
            "identity_unavailable"
        );
        let mut drift = actual();
        drift.revision = "changed".into();
        assert_eq!(
            p.replay(0, raw.to_string().as_bytes(), Some(&drift))["status"],
            "identity_changed"
        );
    }

    #[test]
    fn raw_parser_preserves_duplicate_type_id_and_size_rejection() {
        let p = prepared(
            "http://127.0.0.1:8080/v1/decisions".into(),
            InferenceProtocol::LocalDecider,
        );
        let raw = response(&p.windows[0].request);
        let duplicate = raw
            .to_string()
            .replacen("\"answers\":{", "\"answers\":{\"action\":{},", 1);
        assert_eq!(
            p.replay(0, duplicate.as_bytes(), Some(&actual()))["status"],
            "invalid_response"
        );
        for change in ["missing", "unknown", "wrong_type"] {
            let mut invalid = raw.clone();
            match change {
                "missing" => {
                    invalid["answers"]
                        .as_object_mut()
                        .unwrap()
                        .remove("decision");
                }
                "unknown" => {
                    invalid["answers"]["surprise"] = json!({});
                }
                _ => invalid["answers"]["action"]["type"] = "score".into(),
            }
            assert_eq!(
                p.replay(0, invalid.to_string().as_bytes(), Some(&actual()))["status"],
                "invalid_response"
            );
        }
        assert_eq!(
            p.replay(0, &vec![b' '; 2 * 1024 * 1024 + 1], Some(&actual()))["status"],
            "invalid_response"
        );
    }

    #[test]
    fn mercury_exports_existing_extract_shape_and_rejects_header_spans() {
        let p = prepared(
            "http://127.0.0.1:8080/v1/decisions".into(),
            InferenceProtocol::Mercury,
        );
        assert_eq!(p.windows[0].request.questions.len(), 6);
        assert_eq!(p.windows[0].request.questions[4].options.len(), 24);
        let mut raw = response(&p.windows[0].request);
        let block = &p.windows[0].blocks[0];
        raw["answers"]["event_spans"]["spans"]["action_confirmed"] = json!([{"text":block.source.quote,"start":block.start_char,"end":block.end_char,"confidence":1.0}]);
        raw["answers"]["event_spans"]["not_stated"]["action_confirmed"] = json!(0.0);
        assert_eq!(
            p.replay(0, raw.to_string().as_bytes(), Some(&actual()))["status"],
            "ok"
        );
        raw["answers"]["details"]["spans"]["owner"] =
            json!([{"text":"The","start":0,"end":3,"confidence":1.0}]);
        raw["answers"]["details"]["not_stated"]["owner"] = json!(0.0);
        assert_eq!(
            p.replay(0, raw.to_string().as_bytes(), Some(&actual()))["status"],
            "invalid_output"
        );
    }
}
