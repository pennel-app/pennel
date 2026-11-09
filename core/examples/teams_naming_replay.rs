//! Offline validation through the shipped naming API. Observations are explicitly authored
//! fixtures, never recovered Teams history. Only an isolated, marked Preview profile is writable.
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pennel_core::{AppSpeakerEvidence, MeetingAppNamePolicy, PennelEngine};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 4 {
        return Err(
            "Usage: teams_naming_replay OWNED_PROFILE MEETING SCENARIO_JSON RESULT_JSON".into(),
        );
    }
    let profile = PathBuf::from(&arguments[0]).canonicalize()?;
    if fs::read_to_string(profile.join(".teams-naming-replay"))?
        != "pennel-authored-teams-naming-replay-v1\n"
    {
        return Err("An isolated naming-replay profile marker is required.".into());
    }
    let library = profile.join("Meetings").canonicalize()?;
    let id = &arguments[1];
    let meeting = library.join(id).canonicalize()?;
    if meeting.parent() != Some(library.as_path()) {
        return Err("Only an immediate child of the owned library can be replayed.".into());
    }
    let raw_settings: Value = serde_json::from_slice(&fs::read(profile.join("settings.json"))?)?;
    if raw_settings["meetings_dir"].as_str() != library.to_str() {
        return Err("The fixture settings must point to its isolated library.".into());
    }
    let scenario: Value = serde_json::from_slice(&fs::read(&arguments[2])?)?;
    if !matches!(
        scenario["observation_origin"].as_str(),
        Some(
            "authored_fixture_not_captured_teams_evidence"
                | "model_derived_sortformer_timeline_not_captured_teams_evidence"
        )
    ) {
        return Err(
            "The replay must identify its fixture or model-derived observation origin.".into(),
        );
    }
    let engine = PennelEngine::new_preview(profile.display().to_string())?;
    let mut settings = engine.settings();
    settings.meeting_app_speakers = scenario["enabled"].as_bool().unwrap_or(true);
    settings.meeting_app_name_policy = match scenario["policy"].as_str().unwrap_or("automatic") {
        "automatic" => MeetingAppNamePolicy::Automatic,
        "suggest" => MeetingAppNamePolicy::Suggest,
        _ => return Err("Unsupported fixture policy.".into()),
    };
    engine.update_settings(settings)?;
    if let Some(rename) = scenario.get("manual_rename") {
        engine.rename_speaker(
            id.clone(),
            rename["voice"].as_u64().ok_or("Missing manual voice.")? as u32,
            rename["name"]
                .as_str()
                .ok_or("Missing manual name.")?
                .into(),
        )?;
    }
    let before = engine.meeting_utterance_snapshot(id.clone())?;
    let known_before = engine.known_voices();
    let self_before = engine.voice_profile();
    let enrollment_bytes_before = fs::read(profile.join("voices/voiceprints.json")).ok();
    let notes_before = fs::read(meeting.join("notes.md"))?;
    let evidence: AppSpeakerEvidence = serde_json::from_value(scenario["evidence"].clone())?;
    let evidence_revision = digest(&serde_json::to_vec(&evidence)?);
    engine.set_meeting_app_speaker_evidence(id, evidence.clone())?;
    let prepared = engine.meeting_voice_naming(id);
    let result = engine.apply_meeting_automatic_names(id);
    let (applied, error) = match result {
        Ok(naming) => (Some(naming), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let after = engine.meeting_utterance_snapshot(id.clone())?;
    let known_after = engine.known_voices();
    let self_after = engine.voice_profile();
    let notes_after = fs::read(meeting.join("notes.md"))?;
    let same_rows = before.utterances.len() == after.utterances.len()
        && before
            .utterances
            .iter()
            .zip(&after.utterances)
            .all(|(a, b)| {
                a.segment == b.segment
                    && a.metadata.utterance_id == b.metadata.utterance_id
                    && a.metadata.source_id == b.metadata.source_id
                    && a.metadata.source_fingerprint == b.metadata.source_fingerprint
                    && a.metadata.attribution == b.metadata.attribution
                    && a.metadata.attribution_revision == b.metadata.attribution_revision
                    && a.metadata.lineage == b.metadata.lineage
                    && a.metadata.manual == b.metadata.manual
                    && a.metadata.unresolved_manual == b.metadata.unresolved_manual
            });
    let proofs_valid = after.utterances.iter().all(|row| {
        row.metadata.automatic_name.as_ref().is_none_or(|proof| {
            row.metadata.source_id.as_deref() == Some("system_audio")
                && row.metadata.manual.is_none()
                && proof.source_fingerprint == row.metadata.source_fingerprint
                && proof.attribution_revision == row.metadata.attribution_revision
                && proof.evidence_revision == evidence_revision
                && Some(&proof.binding) == evidence.binding.as_ref()
                && proof.app_id == evidence.app_id
        })
    });
    let enrollment_unchanged = known_before == known_after
        && self_before == self_after
        && enrollment_bytes_before == fs::read(profile.join("voices/voiceprints.json")).ok();
    if !same_rows || !proofs_valid || !enrollment_unchanged || notes_before != notes_after {
        return Err("The naming replay changed source speech, manual work, notes or enrollment, or lost provenance.".into());
    }
    let idempotent = if error.is_none() {
        engine.apply_meeting_automatic_names(id)?;
        engine.meeting_utterance_snapshot(id.clone())?.revision == after.revision
    } else {
        before.revision == after.revision && before.metadata_digest == after.metadata_digest
    };
    if !idempotent {
        return Err("The unchanged naming replay was not idempotent.".into());
    }
    let mut source_ms = BTreeMap::<String, u64>::new();
    let rows: Vec<_> = after
        .utterances
        .iter()
        .map(|row| {
            let source = row.metadata.source_id.clone().unwrap_or_default();
            *source_ms.entry(source.clone()).or_default() +=
                row.segment.end_ms - row.segment.start_ms;
            json!({
                "utterance_id": row.metadata.utterance_id,
                "source_id": source,
                "voice": row.segment.voice,
                "failed": row.segment.failed,
                "nonempty_text": !row.segment.text.trim().is_empty(),
                "recognized_word_count": row.segment.text.split_whitespace().count(),
                "start_ms": row.segment.start_ms, "end_ms": row.segment.end_ms,
                "source_fingerprint": row.metadata.source_fingerprint,
                "attribution": row.metadata.attribution,
                "attribution_revision": row.metadata.attribution_revision,
                "lineage": row.metadata.lineage,
                "manual": row.metadata.manual,
                "unresolved_manual": row.metadata.unresolved_manual,
                "automatic_name": row.metadata.automatic_name,
            })
        })
        .collect();
    fs::write(
        &arguments[3],
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "observation_origin": scenario["observation_origin"],
            "live_teams_capture_validated": false,
            "prepared": prepared, "applied": applied, "error": error,
            "names": engine.meeting_speakers(id.clone()),
            "before_revision": before.revision, "after_revision": after.revision,
            "metadata_status": after.metadata_status,
            "source_ms_including_gaps": source_ms,
            "evidence_revision": evidence_revision,
            "words_sources_attribution_and_manual_work_preserved": same_rows,
            "notes_preserved": notes_before == notes_after,
            "voice_enrollment_unchanged": enrollment_unchanged,
            "provenance_valid": proofs_valid, "idempotent": idempotent,
            "rows": rows,
        }))?,
    )?;
    println!("Authored naming replay completed; no live Teams acceptance claimed.");
    Ok(())
}
