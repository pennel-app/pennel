//! Local repair driver for an explicitly owned Preview fixture, never an ordinary user profile.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pennel_core::{PennelEngine, SpeakerRecognitionMode};
use sha2::{Digest, Sha256};

fn digest(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    Ok(Sha256::digest(fs::read(path)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn words(path: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut words: Vec<_> = fs::read_to_string(path)?
        .lines()
        .filter_map(|line| line.split_once(":** ").map(|(_, text)| text))
        .flat_map(|text| text.split_whitespace().map(str::to_owned))
        .collect();
    words.sort();
    Ok(words)
}

fn decode(path: &Path) -> Result<Option<Vec<f32>>, Box<dyn std::error::Error>> {
    if !path.is_file() {
        return Ok(None);
    }
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-ar", "16000", "-ac", "1", "-f", "f32le", "pipe:1"])
        .output()?;
    if !output.status.success() || output.stdout.len() % 4 != 0 {
        return Err("The owned audio fixture could not be decoded.".into());
    }
    Ok(Some(
        output
            .stdout
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect(),
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() < 2
        || arguments[2..].iter().any(|arg| {
            !matches!(
                arg.as_str(),
                "--confirm-personal-microphone" | "--recognize"
            )
        })
    {
        return Err("Usage: speaker_quality_repair OWNED_PREVIEW_PROFILE MEETING [--confirm-personal-microphone] [--recognize]".into());
    }
    let profile = PathBuf::from(&arguments[0]).canonicalize()?;
    if fs::read_to_string(profile.join(".speaker-quality-fixture"))?
        != "pennel-speaker-quality-fixture-v1\n"
    {
        return Err("An owned Preview fixture marker is required.".into());
    }
    let id = &arguments[1];
    let library = profile.join("Meetings").canonicalize()?;
    let root = library.join(id).canonicalize()?;
    if root.parent() != Some(library.as_path()) || root == library {
        return Err("The meeting must be an immediate child of the owned Preview library.".into());
    }
    let before_words = words(&root.join("transcript.md"))?;
    let notes_before = digest(&root.join("notes.md"))?;
    let mut audio_before = BTreeMap::new();
    for name in ["microphone.m4a", "system.m4a"] {
        let path = root.join("audio").join(name);
        if path.is_file() {
            audio_before.insert(name, digest(&path)?);
        }
    }
    let engine = PennelEngine::new_preview(profile.display().to_string())?;
    let mut settings = engine.settings();
    settings.speaker_recognition = SpeakerRecognitionMode::Local;
    settings.meeting_app_speakers = false;
    engine.update_settings(settings)?;
    let mut confirmed_rows = 0;
    if arguments
        .iter()
        .any(|arg| arg == "--confirm-personal-microphone")
    {
        let reference: serde_json::Value =
            serde_json::from_slice(&fs::read(profile.join("reference.json"))?)?;
        if reference["personal_microphone_owner_confirmed"] != true {
            return Err(
                "This fixture needs an explicit personal-microphone owner assertion.".into(),
            );
        }
        confirmed_rows = engine.confirm_meeting_microphone_owner(id.clone())?;
    }
    let recognition = if arguments.iter().any(|arg| arg == "--recognize") {
        Some(engine.recognize_speakers(
            id.clone(),
            decode(&root.join("audio/microphone.m4a"))?,
            decode(&root.join("audio/system.m4a"))?,
        )?)
    } else {
        None
    };
    if words(&root.join("transcript.md"))? != before_words
        || digest(&root.join("notes.md"))? != notes_before
    {
        return Err(
            "Repair changed recognized words or typed notes; retained backups need review.".into(),
        );
    }
    for (name, before) in &audio_before {
        if digest(&root.join("audio").join(name))? != *before {
            return Err("The retained audio changed.".into());
        }
    }
    let snapshot: serde_json::Value =
        serde_json::from_str(&engine.meeting_transcript_snapshot_json(id.clone())?)?;
    fs::write(
        profile.join("repair-snapshot.json"),
        serde_json::to_vec(&snapshot)?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1, "confirmed_microphone_rows": confirmed_rows,
            "recognition": recognition, "processing": engine.meeting_speaker_processing_status(id.clone()),
            "recognized_words_preserved": true, "typed_notes_preserved": true, "audio_preserved": true,
            "accuracy_reference_available": false
        }))?
    );
    Ok(())
}
