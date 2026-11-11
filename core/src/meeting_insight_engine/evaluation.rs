//! Pure access to the shipped Suggestions contract, without admission or execution.

use super::*;
use crate::meeting_qualification::{
    MeetingActualInference, compare_inference_identity, observed_identity_complete,
};
use serde_json::{Value, json};

pub(crate) struct Prepared {
    source: MeetingTranscriptSnapshot,
    configured: MeetingConfiguredInference,
    config: InferenceConfig,
    language: String,
    windows: Vec<InsightWindow>,
    batches: Vec<InsightBatch>,
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
        let batches = batches(&windows, &config)?;
        Ok(Self {
            source,
            configured,
            config,
            language,
            windows,
            batches,
        })
    }

    pub(crate) fn export(&self) -> Result<Value, PennelError> {
        let windows = self.windows.iter().enumerate().map(|(index, window)| {
            let (batch_index, batch, slot) = self.batches.iter().enumerate().find_map(|(batch_index, batch)| {
                batch.window_indices.iter().position(|value| *value == index).map(|slot| (batch_index, batch, slot))
            }).expect("every logical window belongs to one physical batch");
            json!({
                "index":index,
                "request":window.request,
                "batch_index":batch_index,
                "batch_slot":slot,
                "prompt_context_sha256":batch_prompt_context(batch, slot),
                "sources":window.blocks.iter().map(|block|json!({
                    "source":block.source,"start_char":block.start_char,"end_char":block.end_char
                })).collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>();
        let batches = self.batches.iter().enumerate().map(|(index, batch)| {
            let wire_body = String::from_utf8(crate::inference::decision_body(&self.config, &batch.request)?)
                .map_err(|_| invalid("The request is not UTF-8."))?;
            Ok(json!({"index":index,"window_indices":batch.window_indices,"answer_map":batch.answer_map,
                "request":batch.request,"wire_body_sha256":crate::meeting_utterances::digest(wire_body.as_bytes()),
                "wire_body":wire_body}))
        }).collect::<Result<Vec<_>, PennelError>>()?;
        Ok(json!({
            "qualification_request":qualification_request(self.configured.clone(),&self.language),
            "snapshot":{"meeting_uid":self.source.meeting_uid,"revision":self.source.revision,
                "transcript_digest":self.source.transcript_digest},
            "windows":windows,
            "batches":batches,
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
        let Some(batch) = self.batches.get(index) else {
            return json!({"status":"unknown_batch","batch_index":index,"windows":[]});
        };
        let result = match crate::inference::evaluation_decision_response(
            &self.config,
            self.configured.endpoint.clone(),
            &batch.request,
            raw,
        ) {
            Ok(result) => result,
            Err(_) => return json!({"status":"invalid_response","batch_index":index,"windows":[]}),
        };
        if !observed_identity_complete(expected, &result.identity) {
            return json!({"status":"identity_unavailable","batch_index":index,"observed_identity":result.identity,"windows":[]});
        }
        let Some(expected) = expected else {
            return json!({"status":"identity_unpinned","batch_index":index,"observed_identity":result.identity,"windows":[]});
        };
        if compare_inference_identity(&self.configured, expected, &result.identity).is_err() {
            return json!({"status":"identity_changed","batch_index":index,"observed_identity":result.identity,"windows":[]});
        }
        let logical = match split_batch_result(batch, &result) {
            Ok(logical) => logical,
            Err(_) => {
                return json!({"status":"invalid_response","batch_index":index,"observed_identity":result.identity,"windows":[]});
            }
        };
        let rows = logical.into_iter().map(|(window_index, logical)| {
            let processing = InsightProcessing {
                identity: logical.identity.clone(),
                qualification_id: "evaluation-unqualified".into(),
                language_policy: self.language.clone(),
                task: TASK.into(),
            };
            match decode_candidates(&self.windows[window_index], &logical, &self.source, processing) {
                Ok(candidates) => json!({"status":"ok","window_index":window_index,
                    "observed_identity":logical.identity,"result":logical,"candidates":candidates,
                    "artifact_attestation":false,"identity_evidence":expected.identity_evidence}),
                Err(_) => json!({"status":"invalid_output","window_index":window_index,"observed_identity":logical.identity}),
            }
        }).collect::<Vec<_>>();
        let status = rows
            .iter()
            .find_map(|row| row["status"].as_str().filter(|status| *status != "ok"))
            .unwrap_or("ok");
        json!({"status":status,"batch_index":index,"window_indices":batch.window_indices,
            "observed_identity":result.identity,"result":result,"windows":rows,"artifact_attestation":false,
            "identity_evidence":expected.identity_evidence})
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
            identity_evidence: Default::default(),
        }
    }

    fn pinned_prepared(texts: &[&str]) -> Prepared {
        let source = MeetingTranscriptSnapshot {
            meeting_uid: "a5bcb9c6-12e3-4b3b-b7d9-6d54d540d406".into(),
            revision: 1,
            transcript_digest: "a".repeat(64),
            metadata_digest: None,
            metadata_status: TranscriptMetadataStatus::Valid,
            utterances: texts
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let mut row = MeetingUtterance::new(
                        Segment {
                            start_ms: index as u64 * 1_000,
                            end_ms: (index as u64 + 1) * 1_000,
                            speaker: Speaker::Others,
                            text: (*text).into(),
                            failed: false,
                            voice: None,
                        },
                        Some("room_microphone".into()),
                    );
                    row.metadata.utterance_id =
                        format!("a5bcb9c6-12e3-4b3b-b7d9-{:012}", index + 1);
                    row
                })
                .collect(),
        };
        Prepared::new(
            source,
            MeetingConfiguredInference {
                protocol: InferenceProtocol::OpenRouterDecisions,
                endpoint: "https://openrouter.ai/api/alpha/decisions".into(),
                model: "perplexity/pplx-decider-v1.2".into(),
                upstream: Some("perplexity".into()),
                expected_model: Some("perplexity/pplx-decider-v1.2-20261016".into()),
                max_input_bytes: 65_536,
                timeout_secs: 90,
            },
            "nl".into(),
        )
        .unwrap()
    }

    fn pinned_actual() -> MeetingActualInference {
        MeetingActualInference {
            model: "perplexity/pplx-decider-v1.2-20261016".into(),
            provider: "Perplexity".into(),
            revision: String::new(),
            artifact_sha256: String::new(),
            identity_evidence:
                crate::meeting_qualification::MeetingIdentityEvidence::ProviderReportedVersion,
        }
    }

    #[test]
    fn provider_versioned_metadata_passes_without_inventing_revision_or_weights() {
        // Actual invented-only response from pr-assets/pr-250/hosted-preflight.json.
        // Replaying metadata here is a parser regression, not a new quality measurement.
        let raw = include_str!("fixtures/provider-versioned-response.json");
        let single = pinned_prepared(&["Ik zal de offerte sturen."]);
        let replay = single.replay(0, raw.as_bytes(), Some(&pinned_actual()));
        assert_eq!(replay["status"], "ok");
        assert_eq!(replay["observed_identity"]["actual_revision"], Value::Null);
        assert_eq!(replay["identity_evidence"], "provider_reported_version");
        assert_eq!(replay["artifact_attestation"], false);

        let multiple = pinned_prepared(&["Ik zal de offerte sturen.", "We kiezen maandag."]);
        let mut synthetic: Value = serde_json::from_str(raw).unwrap();
        let original = synthetic["answers"].as_object().unwrap().clone();
        synthetic["answers"] = Value::Object(
            multiple.batches[0]
                .answer_map
                .iter()
                .map(|mapping| {
                    (
                        mapping.batch_id.clone(),
                        original[&mapping.original_id].clone(),
                    )
                })
                .collect(),
        );
        let replay = multiple.replay(0, synthetic.to_string().as_bytes(), Some(&pinned_actual()));
        assert_eq!(replay["status"], "ok");
        assert_eq!(replay["windows"].as_array().unwrap().len(), 2);
        for row in replay["windows"].as_array().unwrap() {
            assert_eq!(row["observed_identity"]["actual_revision"], Value::Null);
            assert_eq!(row["artifact_attestation"], false);
            assert!(
                row["result"]["answers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|answer| !answer["id"].as_str().unwrap().starts_with('w'))
            );
        }
        let mut drift = synthetic.clone();
        drift["model"] = "perplexity/pplx-decider-v1.2-20261017".into();
        assert_eq!(
            multiple.replay(0, drift.to_string().as_bytes(), Some(&pinned_actual()))["status"],
            "invalid_response"
        );
    }

    #[test]
    fn actual_batched_provider_reply_restores_all_logical_ids_with_revision_unavailable() {
        // First actual invented-Dutch v2 development reply, retained before qualification.
        // These scaffold passages exercise parsing/mapping, not the reply's semantic quality.
        let raw = include_str!("fixtures/batched-provider-versioned-response.json");
        let prepared = pinned_prepared(&["Owned parser scaffold."; 8]);
        let replay = prepared.replay(0, raw.as_bytes(), Some(&pinned_actual()));
        assert_eq!(replay["status"], "ok");
        assert_eq!(replay["result"]["answers"].as_array().unwrap().len(), 32);
        assert_eq!(replay["windows"].as_array().unwrap().len(), 8);
        assert_eq!(replay["observed_identity"]["actual_revision"], Value::Null);
        assert_eq!(replay["identity_evidence"], "provider_reported_version");
        assert_eq!(replay["artifact_attestation"], false);
        for (index, row) in replay["windows"].as_array().unwrap().iter().enumerate() {
            assert_eq!(row["window_index"], index);
            assert_eq!(row["result"]["answers"].as_array().unwrap().len(), 4);
            assert!(
                row["result"]["answers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|answer| { EVENT_KINDS.iter().any(|(id, _)| answer["id"] == *id) })
            );
            assert_eq!(row["result"]["usage"]["billed_cost_usd"], Value::Null);
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
            .match_body(export["batches"][0]["wire_body"].as_str().unwrap())
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
            replay["windows"][0]["candidates"],
            serde_json::to_value(production).unwrap()
        );
        assert_eq!(
            replay["windows"][0]["result"],
            serde_json::to_value(result).unwrap()
        );
        assert_eq!(replay["windows"][0]["candidates"][0]["owner"], Value::Null);
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
