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
    support_gate: Option<&'static support_calibration::SupportCalibration>,
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
        if config.protocol == InferenceProtocol::PennelCloud
            && !crate::inference::managed_review(&config)
        {
            return Err(invalid(
                "Managed inference needs a separate authenticated evaluation contract.",
            ));
        }
        let windows = windows(&source, &config)?;
        let batches = batches(&windows, &config)?;
        let support_gate = if support_calibration::required(&configured) {
            // Owned loopback actor regression fixtures explicitly exercise the raw transport.
            // No shipping or evaluation-CLI build contains this mock-only exception.
            #[cfg(test)]
            let mock_raw = configured
                .endpoint
                .strip_suffix("/v1/decisions")
                .is_some_and(|origin| {
                    crate::inference::validate_origin(origin).is_ok_and(|(_, local)| local)
                });
            #[cfg(not(test))]
            let mock_raw = false;
            if mock_raw {
                None
            } else {
                let gate = support_calibration::SupportCalibration::bundled()
                    .map_err(|_| invalid("support_calibration_unavailable"))?;
                gate.validate_request(&request)
                    .map_err(|_| invalid("support_calibration_mismatch"))?;
                for batch in &batches {
                    for (slot, index) in batch.window_indices.iter().enumerate() {
                        gate.validate_window(
                            &windows[*index].request.questions,
                            &batch_prompt_context(batch, slot),
                        )
                        .map_err(|_| invalid("support_calibration_mismatch"))?;
                    }
                }
                Some(gate)
            }
        } else {
            None
        };
        Ok(Self {
            source,
            configured,
            config,
            language,
            windows,
            batches,
            support_gate,
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
            let mut value = json!({"index":index,"window_indices":batch.window_indices,"answer_map":batch.answer_map,
                "request":batch.request,"wire_body_sha256":crate::meeting_utterances::digest(wire_body.as_bytes()),
                "wire_body":wire_body});
            if crate::inference::managed_review(&self.config) {
                let mut upstream: Value = serde_json::from_str(value["wire_body"].as_str().expect("encoded UTF-8"))
                    .map_err(|_| invalid("The request is not UTF-8."))?;
                let object = upstream.as_object_mut().expect("wire request object");
                object.remove("expected_model");
                object.insert("model".into(), json!("perplexity/pplx-decider-v1.2"));
                object.insert("provider".into(), json!({"only":["perplexity"],"allow_fallbacks":false,
                    "data_collection":"deny","zdr":true}));
                value["managed_transport"] = json!({
                    "profile":"supported_passages_v3","session_origin":self.configured.endpoint.strip_suffix("/v1/decisions"),
                    "runtime_request_id":"unique UUID per physical decision; preserved across authentication refresh",
                    "canonical_upstream_contract":upstream,
                    "upstream_contract_is_observed_wire":false,
                    "request_timeout_secs":crate::inference::MANAGED_REVIEW_TIMEOUT_SECS,
                    "whole_job_timeout_secs":self.config.timeout_secs,
                    "response_profile_required":"supported_passages_v3"
                });
            }
            Ok(value)
        }).collect::<Result<Vec<_>, PennelError>>()?;
        let mut value = json!({
            "runtime_policy":if self.support_gate.is_some() {
                support_calibration::POLICY
            } else if crate::inference::supported_passage_profile(self.config.protocol, &self.config.model) {
                "raw_supported_passage_decisions_v3"
            } else { "raw_closed_set_passage_decisions_v2" },
            "qualification_request":qualification_request(self.configured.clone(),&self.language),
            "snapshot":{"meeting_uid":self.source.meeting_uid,"revision":self.source.revision,
                "transcript_digest":self.source.transcript_digest},
            "windows":windows,
            "batches":batches,
            "production_admission":false,
            "qualification":false
        });
        if let Some(gate) = self.support_gate {
            value["support_calibration"] = gate.metadata();
        }
        Ok(value)
    }

    pub(crate) fn batch_request(&self, index: usize) -> Option<DecisionRequest> {
        self.batches.get(index).map(|batch| batch.request.clone())
    }

    pub(crate) fn batch_count(&self) -> usize {
        self.batches.len()
    }

    pub(crate) fn uses_support_calibration(&self) -> bool {
        self.support_gate.is_some()
    }

    pub(crate) fn replay(
        &self,
        index: usize,
        raw: &[u8],
        expected: Option<&MeetingActualInference>,
    ) -> Value {
        self.replay_with_request_id(index, raw, expected, None)
    }

    pub(crate) fn replay_with_request_id(
        &self,
        index: usize,
        raw: &[u8],
        expected: Option<&MeetingActualInference>,
        request_id: Option<&str>,
    ) -> Value {
        let Some(batch) = self.batches.get(index) else {
            return json!({"status":"unknown_batch","batch_index":index,"windows":[]});
        };
        let parsed = if crate::inference::managed_review(&self.config) {
            let Some(request_id) = request_id else {
                return json!({"status":"request_identity_unpinned","batch_index":index,"windows":[]});
            };
            crate::inference::evaluation_managed_decision_response(
                &self.config,
                self.configured.endpoint.clone(),
                &batch.request,
                raw,
                request_id,
            )
        } else {
            crate::inference::evaluation_decision_response(
                &self.config,
                self.configured.endpoint.clone(),
                &batch.request,
                raw,
            )
        };
        let result = match parsed {
            Ok(result) => result,
            Err(_) => return json!({"status":"invalid_response","batch_index":index,"windows":[]}),
        };
        self.project_result(index, result, expected)
    }

    pub(crate) fn replay_production_result(
        &self,
        index: usize,
        result: DecisionResult,
        expected: Option<&MeetingActualInference>,
    ) -> Value {
        let Some(batch) = self.batches.get(index) else {
            return json!({"status":"unknown_batch","batch_index":index,"windows":[]});
        };
        if !(crate::inference::managed_local_review(&self.config) || self.support_gate.is_some())
            || crate::local_decider::validate_answers(&batch.request, &result).is_err()
        {
            return json!({"status":"invalid_response","batch_index":index,"windows":[]});
        }
        self.project_result(index, result, expected)
    }

    pub(crate) fn replay_observed_managed_result(
        &self,
        index: usize,
        result: DecisionResult,
        response: &Value,
        expected: Option<&MeetingActualInference>,
        request_id: Option<&str>,
    ) -> Value {
        let Some(batch) = self.batches.get(index) else {
            return json!({"status":"unknown_batch","batch_index":index,"windows":[]});
        };
        let audited = request_id
            .filter(|id| {
                uuid::Uuid::parse_str(id)
                    .is_ok_and(|value| !value.is_nil() && value.to_string() == *id)
            })
            .and_then(|request_id| {
                crate::inference::evaluation_managed_decision_value(
                    &self.config,
                    self.configured.endpoint.clone(),
                    &batch.request,
                    response,
                    request_id,
                )
                .ok()
            });
        if self.support_gate.is_none() || audited.as_ref() != Some(&result) {
            return json!({"status":"typed_response_capture_mismatch","batch_index":index,"windows":[]});
        }
        self.replay_production_result(index, result, expected)
    }

    fn project_result(
        &self,
        index: usize,
        result: DecisionResult,
        expected: Option<&MeetingActualInference>,
    ) -> Value {
        let Some(batch) = self.batches.get(index) else {
            return json!({"status":"unknown_batch","batch_index":index,"windows":[]});
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
        if self.support_gate.is_some_and(|gate| {
            gate.validate_pin(expected).is_err() || gate.validate_actual(&result.identity).is_err()
        }) {
            return json!({"status":"support_calibration_mismatch","batch_index":index,"observed_identity":result.identity,"windows":[]});
        }
        let logical = match split_batch_result(batch, &result) {
            Ok(logical) => logical,
            Err(_) => {
                return json!({"status":"invalid_response","batch_index":index,"observed_identity":result.identity,"windows":[]});
            }
        };
        let rows = logical.into_iter().enumerate().map(|(slot,(window_index, logical))| {
            let processing = InsightProcessing {
                identity: logical.identity.clone(),
                qualification_id: "evaluation-unqualified".into(),
                language_policy: self.language.clone(),
                task: TASK.into(),
            };
            let window=&self.windows[window_index];
            let gated=match self.support_gate.map(|gate|gate.apply(&window.request.questions,&batch_prompt_context(batch,slot),&logical)).transpose() {
                Ok(gated)=>gated,
                Err(_)=>return json!({"status":"support_calibration_mismatch","window_index":window_index,"observed_identity":logical.identity}),
            };
            let effective=gated.as_ref().map_or(&logical,|gated|&gated.result);
            match decode_candidates(window, effective, &self.source, processing) {
                Ok(candidates) => match assessed_kinds(window, effective, &candidates) {
                    Ok(kinds) => {
                        let mut value=json!({"status":"ok","window_index":window_index,
                        "observed_identity":logical.identity,"result":logical,"candidates":candidates,
                        "assessed_kinds":kinds,"artifact_attestation":false,"identity_evidence":expected.identity_evidence});
                        if let Some(gated)=gated {value["support_gate"]=json!(gated.decisions);}
                        value
                    },
                    Err(_) => json!({"status":"invalid_output","window_index":window_index,"observed_identity":logical.identity}),
                },
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
        assert_eq!(
            single.replay(0, raw.as_bytes(), Some(&pinned_actual()))["status"],
            "invalid_response"
        );
        // Preserve the actual v2 reply unchanged. New support answers below are
        // explicitly synthetic parser scaffolding, not measured model evidence.
        let mut synthetic: Value = serde_json::from_str(raw).unwrap();
        for (id, _) in EVENT_KINDS {
            let chosen = if synthetic["answers"][id]["choice"] == "none"
                || synthetic["answers"][id]["choice"] == "negated"
            {
                "absent"
            } else {
                "supported"
            };
            synthetic["answers"][format!("{id}_support")] = json!({"type":"choice","choice":chosen,
                "probabilities":{"supported":if chosen=="supported"{1.0}else{0.0},
                    "absent":if chosen=="absent"{1.0}else{0.0},"unclear":0.0}});
        }
        let replay = single.replay(0, synthetic.to_string().as_bytes(), Some(&pinned_actual()));
        assert_eq!(replay["status"], "ok");
        assert_eq!(replay["observed_identity"]["actual_revision"], Value::Null);
        assert_eq!(replay["identity_evidence"], "provider_reported_version");
        assert_eq!(replay["artifact_attestation"], false);

        let multiple = pinned_prepared(&["Ik zal de offerte sturen.", "We kiezen maandag."]);
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
        assert_eq!(
            prepared.replay(0, raw.as_bytes(), Some(&pinned_actual()))["status"],
            "invalid_response"
        );
        let mut old_windows = prepared.windows.clone();
        for window in &mut old_windows {
            window
                .request
                .questions
                .retain(|question| !question.id.ends_with("_support"));
        }
        let historical_batch = make_batch(&old_windows, 0, 8);
        let parsed = crate::inference::evaluation_decision_response(
            &prepared.config,
            prepared.configured.endpoint.clone(),
            &historical_batch.request,
            raw.as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed.answers.len(), 32);
        assert_eq!(parsed.identity.actual_revision, None);
        assert_eq!(
            parsed.identity.actual_model.as_deref(),
            Some("perplexity/pplx-decider-v1.2-20261016")
        );
        let logical = split_batch_result(&historical_batch, &parsed).unwrap();
        assert_eq!(logical.len(), 8);
        for (index, row) in logical {
            assert_eq!(row.answers.len(), 4);
            assert_eq!(index, historical_batch.window_indices[index]);
            assert!(
                row.answers
                    .iter()
                    .all(|answer| EVENT_KINDS.iter().any(|(id, _)| answer.id == *id))
            );
            assert_eq!(row.usage.billed_cost_usd, None);
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
                } else if q.id == "action_support" {
                    "supported"
                } else if q.id.ends_with("_support") {
                    "absent"
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
