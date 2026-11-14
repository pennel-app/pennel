use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::meeting_insight_engine::evaluation::Prepared;
use crate::meeting_qualification::{MeetingActualInference, MeetingConfiguredInference};
use crate::{
    MeetingTranscriptSnapshot, MeetingUtterance, Segment, Speaker, TranscriptMetadataStatus,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Utterance {
    start_ms: u64,
    end_ms: u64,
    speaker: Speaker,
    text: String,
    #[serde(default)]
    failed: bool,
    #[serde(default)]
    voice: Option<u32>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    id: String,
    family_id: String,
    split: String,
    language: String,
    utterances: Vec<Utterance>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportRequest {
    op: String,
    fixture: Fixture,
    configuration: MeetingConfiguredInference,
    #[serde(default)]
    expected_actual: Option<MeetingActualInference>,
}

fn uuid(input: &str) -> String {
    let digest = crate::meeting_utterances::digest(input.as_bytes());
    format!(
        "{}-{}-4{}-a{}-{}",
        &digest[..8],
        &digest[8..12],
        &digest[13..16],
        &digest[17..20],
        &digest[20..32]
    )
}

fn prepared(input: &ExportRequest) -> Result<Prepared, &'static str> {
    if input.op != "finished_meeting_export"
        || !super::valid_id(&input.fixture.id)
        || !super::valid_id(&input.fixture.family_id)
        || !matches!(
            input.fixture.split.as_str(),
            "development" | "calibration" | "heldout"
        )
        || !matches!(input.fixture.language.as_str(), "en" | "nl" | "mixed")
        || input.expected_actual.as_ref().is_some_and(|identity| {
            !crate::meeting_qualification::valid_evaluation_identity_pin(identity)
        })
    {
        return Err("finished_fixture_invalid");
    }
    let mut rows = Vec::new();
    for (index, u) in input.fixture.utterances.iter().enumerate() {
        if u.end_ms < u.start_ms {
            return Err("finished_fixture_invalid");
        }
        let mut row = MeetingUtterance::new(
            Segment {
                start_ms: u.start_ms,
                end_ms: u.end_ms,
                speaker: u.speaker,
                text: u.text.clone(),
                failed: u.failed,
                voice: u.voice,
            },
            Some("room_microphone".into()),
        );
        row.metadata.utterance_id = uuid(&format!("{}:{index}", input.fixture.id));
        rows.push(row);
    }
    let digest = crate::evidence_revision(
        &serde_json::to_string(&rows).map_err(|_| "finished_fixture_invalid")?,
    );
    Prepared::new(
        MeetingTranscriptSnapshot {
            meeting_uid: uuid(&input.fixture.id),
            revision: 1,
            transcript_digest: digest,
            metadata_digest: None,
            metadata_status: TranscriptMetadataStatus::Valid,
            utterances: rows,
        },
        input.configuration.clone(),
        input.fixture.language.clone(),
    )
    .map_err(|_| "finished_contract_invalid")
}

fn source_hashes(configuration: &MeetingConfiguredInference) -> Value {
    let mut value = json!({
        "meeting_insight_engine.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_insight_engine.rs")),
        "meeting_insight_engine/evaluation.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_insight_engine/evaluation.rs")),
        "inference.rs":crate::meeting_utterances::digest(include_bytes!("../inference.rs")),
        "meeting_qualification.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_qualification.rs")),
        "meeting_utterances.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_utterances.rs")),
        "meeting_insights.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_insights.rs")),
        "evaluation/finished.rs":crate::meeting_utterances::digest(include_bytes!("finished.rs"))
    });
    let managed = configuration.protocol == crate::InferenceProtocol::PennelCloud;
    let local = configuration.protocol == crate::InferenceProtocol::LocalDecider
        && configuration.model == crate::local_decider::MODEL_ID;
    if managed || local {
        let object = value.as_object_mut().expect("source manifest");
        for (name, bytes) in [
            (
                "evaluation.rs",
                include_bytes!("../evaluation.rs").as_slice(),
            ),
            (
                "inference_engine.rs",
                include_bytes!("../inference_engine.rs").as_slice(),
            ),
            (
                "bin/pennel-eval.rs",
                include_bytes!("../bin/pennel-eval.rs").as_slice(),
            ),
        ] {
            object.insert(name.into(), crate::meeting_utterances::digest(bytes).into());
        }
    }
    if managed {
        let object = value.as_object_mut().expect("source manifest");
        for (name, bytes) in [
            (
                "meeting_insight_engine/support_calibration.rs",
                include_bytes!("../meeting_insight_engine/support_calibration.rs").as_slice(),
            ),
            (
                "assets/qualifications/meeting-support-cloud-v1.json",
                include_bytes!("../../../assets/qualifications/meeting-support-cloud-v1.json")
                    .as_slice(),
            ),
            (
                "cloud/account.rs",
                include_bytes!("../cloud/account.rs").as_slice(),
            ),
            (
                "evaluation/managed_review.rs",
                include_bytes!("managed_review.rs").as_slice(),
            ),
        ] {
            object.insert(name.into(), crate::meeting_utterances::digest(bytes).into());
        }
    }
    if local {
        let object = value.as_object_mut().expect("source manifest");
        for (name, bytes) in [
            ("engine.rs", include_bytes!("../engine.rs").as_slice()),
            ("models.rs", include_bytes!("../models.rs").as_slice()),
            (
                "evaluation/local_review.rs",
                include_bytes!("local_review.rs").as_slice(),
            ),
            (
                "local_decider.rs",
                include_bytes!("../local_decider.rs").as_slice(),
            ),
            (
                "local_decider/native.rs",
                include_bytes!("../local_decider/native.rs").as_slice(),
            ),
            (
                "local_decider/native/mac.rs",
                include_bytes!("../local_decider/native/mac.rs").as_slice(),
            ),
            (
                "local_decider/protocol.rs",
                include_bytes!("../local_decider/protocol.rs").as_slice(),
            ),
            (
                "local_decider/scopes.rs",
                include_bytes!("../local_decider/scopes.rs").as_slice(),
            ),
            (
                "native/decider/owned_decider.cpp",
                include_bytes!("../../native/decider/owned_decider.cpp").as_slice(),
            ),
            (
                "native/decider/CMakeLists.txt",
                include_bytes!("../../native/decider/CMakeLists.txt").as_slice(),
            ),
            (
                "native/decider/exports.txt",
                include_bytes!("../../native/decider/exports.txt").as_slice(),
            ),
            (
                "native/decider/runtime-lock.json",
                include_bytes!("../../native/decider/runtime-lock.json").as_slice(),
            ),
            (
                "bin/pennel-decider.rs",
                include_bytes!("../bin/pennel-decider.rs").as_slice(),
            ),
            (
                "scripts/prepare-decider-runtime.py",
                include_bytes!("../../../scripts/prepare-decider-runtime.py").as_slice(),
            ),
        ] {
            object.insert(name.into(), crate::meeting_utterances::digest(bytes).into());
        }
    }
    value
}

fn exported(input: &ExportRequest) -> Result<Value, &'static str> {
    let p = prepared(input)?;
    let mut value = p.export().map_err(|_| "finished_contract_invalid")?;
    let object = value.as_object_mut().ok_or("finished_contract_invalid")?;
    object.insert("schema_version".into(), json!(3));
    object.insert(
        "protocol_id".into(),
        json!("production-finished-meeting-v3"),
    );
    object.insert(
        "fixture".into(),
        serde_json::to_value(&input.fixture).map_err(|_| "finished_fixture_invalid")?,
    );
    object.insert(
        "configuration".into(),
        serde_json::to_value(&input.configuration).map_err(|_| "finished_contract_invalid")?,
    );
    object.insert(
        "expected_actual".into(),
        serde_json::to_value(&input.expected_actual).map_err(|_| "finished_contract_invalid")?,
    );
    object.insert("source_sha256".into(), source_hashes(&input.configuration));
    let digest = crate::evidence_revision(&value.to_string());
    value["freeze_sha256"] = digest.into();
    Ok(value)
}

pub(super) fn export(input: Value) -> Result<Value, &'static str> {
    let input: ExportRequest =
        serde_json::from_value(input).map_err(|_| "finished_fixture_invalid")?;
    exported(&input)
}

pub(super) fn verified_prepared(value: &Value) -> Result<Prepared, &'static str> {
    let request: ExportRequest = serde_json::from_value(json!({"op":"finished_meeting_export",
        "fixture":value["fixture"],"configuration":value["configuration"],
        "expected_actual":value["expected_actual"]}))
    .map_err(|_| "finished_replay_invalid")?;
    if exported(&request)? != *value {
        return Err("finished_export_changed");
    }
    prepared(&request)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    batch_index: usize,
    http_status: Option<u16>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    production_result: Option<crate::DecisionResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayRequest {
    op: String,
    export: Value,
    responses: Vec<Response>,
}

pub(super) fn replay(input: Value) -> Result<Value, &'static str> {
    let input: ReplayRequest =
        serde_json::from_value(input).map_err(|_| "finished_replay_invalid")?;
    if input.op != "finished_meeting_replay" {
        return Err("finished_replay_invalid");
    }
    let request: ExportRequest = serde_json::from_value(json!({"op":"finished_meeting_export",
        "fixture":input.export["fixture"],"configuration":input.export["configuration"],
        "expected_actual":input.export["expected_actual"]}))
    .map_err(|_| "finished_replay_invalid")?;
    if exported(&request)? != input.export {
        return Err("finished_export_changed");
    }
    let p = prepared(&request)?;
    let count = input.export["batches"]
        .as_array()
        .ok_or("finished_replay_invalid")?
        .len();
    if input.responses.len() > count
        || input
            .responses
            .iter()
            .enumerate()
            .any(|(index, response)| index != response.batch_index)
    {
        return Err("finished_response_order_changed");
    }
    let mut physical = Vec::new();
    let mut rows = Vec::new();
    for response in &input.responses {
        let batch = if let Some(result) = &response.production_result {
            if p.uses_support_calibration() {
                if response.body.len() > 2 * 1024 * 1024
                    || !response
                        .http_status
                        .is_some_and(|status| (200..300).contains(&status))
                {
                    return Err("finished_replay_invalid");
                }
                let captured =
                    crate::meeting_insight_engine::support_calibration::exact_json(&response.body)
                        .map_err(|_| "finished_replay_invalid")?;
                p.replay_observed_managed_result(
                    response.batch_index,
                    result.clone(),
                    &captured,
                    request.expected_actual.as_ref(),
                    response.request_id.as_deref(),
                )
            } else {
                if response.http_status.is_some()
                    || !response.body.is_empty()
                    || response.request_id.is_some()
                {
                    return Err("finished_replay_invalid");
                }
                p.replay_production_result(
                    response.batch_index,
                    result.clone(),
                    request.expected_actual.as_ref(),
                )
            }
        } else if p.uses_support_calibration() {
            json!({"status":"production_result_missing","batch_index":response.batch_index,"windows":[]})
        } else if !response
            .http_status
            .is_some_and(|status| (200..300).contains(&status))
        {
            json!({"batch_index":response.batch_index,"status":"provider_failed","windows":[]})
        } else {
            if request.configuration.protocol == crate::InferenceProtocol::PennelCloud {
                p.replay_with_request_id(
                    response.batch_index,
                    response.body.as_bytes(),
                    request.expected_actual.as_ref(),
                    response.request_id.as_deref(),
                )
            } else {
                p.replay(
                    response.batch_index,
                    response.body.as_bytes(),
                    request.expected_actual.as_ref(),
                )
            }
        };
        if let Some(windows) = batch["windows"].as_array() {
            rows.extend(windows.iter().cloned());
        }
        physical.push(batch);
    }
    let mut status = physical
        .iter()
        .find_map(|batch| batch["status"].as_str().filter(|status| *status != "ok"))
        .unwrap_or("ok");
    if status == "ok"
        && input.responses.len() == count
        && rows
            .iter()
            .all(|row| row["assessed_kinds"].as_array().is_none_or(Vec::is_empty))
    {
        status = "insufficient_evidence";
    }
    Ok(
        json!({"schema_version":3,"protocol_id":"production-finished-meeting-v3","status":status,
        "runtime_policy":input.export["runtime_policy"],
        "complete":input.responses.len()==count,"windows":rows,"batches":physical,
        "received_batches":input.responses.len(),"production_admission":false,
        "qualification":false,"freeze_sha256":input.export["freeze_sha256"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        json!({"op":"finished_meeting_export",
            "fixture":{"id":"synthetic_en","family_id":"synthetic_family","split":"development","language":"en",
                "utterances":[{"start_ms":0,"end_ms":1000,"speaker":"others","text":"Invented café meeting 😀."}]},
            "configuration":{"protocol":"local_decider","endpoint":"http://127.0.0.1:8080/v1/decisions","model":"fixture-model","max_input_bytes":16000,"timeout_secs":30}})
    }

    #[test]
    fn deterministic_exports_refuse_changed_source_maps_context_and_hashes() {
        let first = export(input()).unwrap();
        assert_eq!(first, export(input()).unwrap());
        for field in [
            "source_sha256",
            "qualification_request",
            "windows",
            "snapshot",
            "freeze_sha256",
        ] {
            let mut changed = first.clone();
            changed[field] = Value::Null;
            assert_eq!(
                replay(json!({"op":"finished_meeting_replay","export":changed,"responses":[]})),
                Err("finished_export_changed")
            );
        }
    }

    #[test]
    fn managed_local_replays_the_typed_production_result_with_the_shared_validator_and_sources() {
        let mut request = input();
        request["configuration"] = json!({"protocol":"local_decider",
            "endpoint":crate::local_decider::ENDPOINT,"model":crate::local_decider::MODEL_ID,
            "expected_model":crate::local_decider::MODEL_FINGERPRINT,"upstream":null,
            "max_input_bytes":65536,"timeout_secs":90});
        request["fixture"]["language"] = "nl".into();
        request["expected_actual"] = json!({"model":crate::local_decider::MODEL_FINGERPRINT,
            "provider":crate::local_decider::PROVIDER,"revision":"a".repeat(64),
            "artifact_sha256":crate::local_decider::MODEL_SHA256,
            "identity_evidence":"serving_revision_and_artifact"});
        let exported = export(request).unwrap();
        for name in [
            "evaluation/local_review.rs",
            "local_decider.rs",
            "local_decider/native/mac.rs",
            "local_decider/protocol.rs",
            "local_decider/scopes.rs",
            "native/decider/owned_decider.cpp",
            "native/decider/runtime-lock.json",
            "bin/pennel-decider.rs",
        ] {
            assert!(
                exported["source_sha256"][name]
                    .as_str()
                    .is_some_and(|value| value.len() == 64)
            );
        }
        let native_request: crate::DecisionRequest =
            serde_json::from_value(exported["batches"][0]["request"].clone()).unwrap();
        let result = crate::DecisionResult {
            identity: crate::InferenceIdentity {
                protocol: crate::InferenceProtocol::LocalDecider,
                endpoint: crate::local_decider::ENDPOINT.into(),
                requested_model: crate::local_decider::MODEL_ID.into(),
                requested_upstream: None,
                actual_model: Some(crate::local_decider::MODEL_FINGERPRINT.into()),
                actual_revision: Some("a".repeat(64)),
                actual_provider: Some(crate::local_decider::PROVIDER.into()),
                request_id: Some(uuid::Uuid::now_v7().to_string()),
            },
            answers: native_request
                .questions
                .iter()
                .map(|question| {
                    let choice = if question.id.ends_with("_support") {
                        "absent"
                    } else {
                        "none"
                    };
                    crate::DecisionAnswer {
                        id: question.id.clone(),
                        kind: crate::DecisionQuestionKind::Choice,
                        choice: Some(choice.into()),
                        value: None,
                        confidence: Some(1.0),
                        spans: vec![],
                        not_stated: vec![],
                        abstained: false,
                        probabilities: question
                            .options
                            .iter()
                            .map(|option| crate::DecisionProbability {
                                option_id: option.id.clone(),
                                probability: if option.id == choice { 1.0 } else { 0.0 },
                            })
                            .collect(),
                    }
                })
                .collect(),
            usage: crate::InferenceUsage {
                input_tokens: Some(100),
                output_tokens: Some(0),
                billed_cost_usd: Some(0.0),
            },
        };
        let replayed = replay(json!({"op":"finished_meeting_replay","export":exported,
            "responses":[{"batch_index":0,"production_result":result}]}))
        .unwrap();
        assert_eq!(replayed["complete"], true);
        assert_eq!(replayed["batches"][0]["status"], "ok");
        assert_eq!(replayed["windows"].as_array().unwrap().len(), 1);
        let mut invalid = result.clone();
        invalid.answers[0].probabilities.pop();
        let invalid = replay(json!({"op":"finished_meeting_replay","export":exported,
            "responses":[{"batch_index":0,"production_result":invalid}]}))
        .unwrap();
        assert_eq!(invalid["status"], "invalid_response");
        assert!(
            replay(json!({"op":"finished_meeting_replay","export":exported,
            "responses":[{"batch_index":0,"http_status":200,"production_result":result}]}))
            .is_err()
        );
    }

    #[test]
    fn fixture_content_cannot_select_real_paths_notes_credentials_or_admission() {
        for field in ["profile", "path", "notes", "credentials"] {
            let mut bad = input();
            bad["fixture"][field] = "forbidden".into();
            assert_eq!(export(bad), Err("finished_fixture_invalid"));
        }
        let value = export(input()).unwrap();
        let request: crate::MeetingQualificationRequest =
            serde_json::from_value(value["qualification_request"].clone()).unwrap();
        assert!(matches!(
            crate::MeetingQualificationRegistry::bundled().admit(&request),
            Err(crate::MeetingQualificationError::Unqualified)
        ));
        assert_eq!(value["production_admission"], false);
        assert_eq!(value["qualification"], false);
    }

    #[test]
    fn failed_and_empty_rows_are_not_exported_and_oversize_is_refused() {
        let mut value = input();
        value["fixture"]["utterances"].as_array_mut().unwrap().push(
            json!({"start_ms":1,"end_ms":2,"speaker":"others","text":"Unfinished","failed":true}),
        );
        assert_eq!(
            export(value.clone()).unwrap()["windows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        value["configuration"]["max_input_bytes"] = 100.into();
        assert_eq!(export(value), Err("finished_contract_invalid"));
    }

    #[test]
    fn complete_transport_cannot_turn_wholly_unclear_analysis_into_a_success() {
        for protocol in ["local_decider", "open_router_decisions"] {
            let mut request = input();
            request["configuration"]["protocol"] = protocol.into();
            request["configuration"]["expected_model"] = "fixture-model".into();
            if protocol == "open_router_decisions" {
                request["configuration"]["endpoint"] =
                    "https://openrouter.ai/api/alpha/decisions".into();
                request["configuration"]["upstream"] = "fixture-provider".into();
            }
            request["expected_actual"] = json!({"model":"fixture-model","provider":"fixture-provider",
                "revision":"fixture-revision","artifact_sha256":"a".repeat(64)});
            let exported = export(request).unwrap();
            assert_eq!(exported["schema_version"], 3);
            assert_eq!(exported["protocol_id"], "production-finished-meeting-v3");
            for assessed in [false, true] {
                let answers = exported["batches"][0]["request"]["questions"].as_array().unwrap().iter().map(|question| {
                    let id=question["id"].as_str().unwrap();
                    let support=id.ends_with("_support");
                    let chosen=if support {if assessed {"absent"}else{"unclear"}}else{"none"};
                    (id.to_owned(),json!({"type":"choice","choice":chosen,
                        "abstained":!support&&!assessed&&protocol=="local_decider",
                        "probabilities":question["options"].as_array().unwrap().iter().map(|option|{
                            let key=option["id"].as_str().unwrap();(key.to_owned(),json!(if key==chosen {1.0}else{0.0}))
                        }).collect::<serde_json::Map<_,_>>()}))
                }).collect::<serde_json::Map<_,_>>();
                let body=json!({"model":"fixture-model","model_revision":"fixture-revision","provider":"fixture-provider","answers":answers}).to_string();
                let replay = replay(
                    json!({"op":"finished_meeting_replay","export":exported,"responses":[{
                    "batch_index":0,"http_status":200,"body":body}]}),
                )
                .unwrap();
                assert_eq!(replay["complete"], true);
                assert_eq!(
                    replay["status"],
                    if assessed {
                        "ok"
                    } else {
                        "insufficient_evidence"
                    }
                );
                assert_eq!(
                    replay["windows"][0]["assessed_kinds"]
                        .as_array()
                        .unwrap()
                        .len(),
                    if assessed { 4 } else { 0 }
                );
                assert!(
                    replay["windows"][0]["candidates"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn incompatible_positive_kind_states_cannot_establish_assessment_or_empty_success() {
        let mut request = input();
        request["configuration"] = json!({"protocol":"open_router_decisions",
            "endpoint":"https://openrouter.ai/api/alpha/decisions","upstream":"fixture-provider",
            "model":"fixture-model","expected_model":"fixture-model","max_input_bytes":16000,"timeout_secs":30});
        request["expected_actual"] = json!({"model":"fixture-model","provider":"fixture-provider",
            "revision":"fixture-revision","artifact_sha256":"a".repeat(64)});
        let exported = export(request).unwrap();
        let answers=exported["batches"][0]["request"]["questions"].as_array().unwrap().iter().map(|question|{
            let id=question["id"].as_str().unwrap();
            let chosen=if id.ends_with("_support"){"supported"}else if id=="open_question"{"confirmed"}else{"unresolved"};
            (id.to_owned(),json!({"type":"choice","choice":chosen,"probabilities":
                question["options"].as_array().unwrap().iter().map(|option|{
                    let key=option["id"].as_str().unwrap();(key.to_owned(),json!(if key==chosen{1.0}else{0.0}))
                }).collect::<serde_json::Map<_,_>>()}))
        }).collect::<serde_json::Map<_,_>>();
        let body=json!({"model":"fixture-model","model_revision":"fixture-revision","provider":"fixture-provider","answers":answers}).to_string();
        let replay = replay(
            json!({"op":"finished_meeting_replay","export":exported,"responses":[{
            "batch_index":0,"http_status":200,"body":body}]}),
        )
        .unwrap();
        assert_eq!(replay["status"], "insufficient_evidence");
        assert_eq!(replay["complete"], true);
        assert!(
            replay["windows"][0]["assessed_kinds"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            replay["windows"][0]["candidates"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            replay["windows"][0]["result"]["answers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|answer| { answer["id"] == "decision" && answer["choice"] == "unresolved" })
        );
    }
}
