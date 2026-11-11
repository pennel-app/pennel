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

fn source_hashes() -> Value {
    json!({
        "meeting_insight_engine.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_insight_engine.rs")),
        "meeting_insight_engine/evaluation.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_insight_engine/evaluation.rs")),
        "inference.rs":crate::meeting_utterances::digest(include_bytes!("../inference.rs")),
        "meeting_qualification.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_qualification.rs")),
        "meeting_utterances.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_utterances.rs")),
        "meeting_insights.rs":crate::meeting_utterances::digest(include_bytes!("../meeting_insights.rs")),
        "evaluation/finished.rs":crate::meeting_utterances::digest(include_bytes!("finished.rs"))
    })
}

fn exported(input: &ExportRequest) -> Result<Value, &'static str> {
    let p = prepared(input)?;
    let mut value = p.export().map_err(|_| "finished_contract_invalid")?;
    let object = value.as_object_mut().ok_or("finished_contract_invalid")?;
    object.insert("schema_version".into(), json!(2));
    object.insert(
        "protocol_id".into(),
        json!("production-finished-meeting-v2"),
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
    object.insert("source_sha256".into(), source_hashes());
    let digest = crate::evidence_revision(&value.to_string());
    value["freeze_sha256"] = digest.into();
    Ok(value)
}

pub(super) fn export(input: Value) -> Result<Value, &'static str> {
    let input: ExportRequest =
        serde_json::from_value(input).map_err(|_| "finished_fixture_invalid")?;
    exported(&input)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    batch_index: usize,
    http_status: u16,
    body: String,
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
        let batch = if !(200..300).contains(&response.http_status) {
            json!({"batch_index":response.batch_index,"status":"provider_failed","windows":[]})
        } else {
            p.replay(
                response.batch_index,
                response.body.as_bytes(),
                request.expected_actual.as_ref(),
            )
        };
        if let Some(windows) = batch["windows"].as_array() {
            rows.extend(windows.iter().cloned());
        }
        physical.push(batch);
    }
    let status = physical
        .iter()
        .find_map(|batch| batch["status"].as_str().filter(|status| *status != "ok"))
        .unwrap_or("ok");
    Ok(
        json!({"schema_version":2,"protocol_id":"production-finished-meeting-v2","status":status,
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
}
