use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::*;

struct TimedSpeech(AtomicUsize);

impl SystemTranscriber for TimedSpeech {
    fn transcribe(
        &self,
        _: Vec<f32>,
        _: u32,
        _: Option<String>,
        _: Vec<String>,
    ) -> Result<String, PennelError> {
        panic!("The selected timed recognizer must run exactly once instead of text fallback.");
    }
}

impl TimedSystemTranscriber for TimedSpeech {
    fn transcribe_timed(
        &self,
        _: Vec<f32>,
        _: u32,
        _: Option<String>,
        _: Vec<String>,
    ) -> Result<SpeechTranscript, PennelError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(SpeechTranscript::validated(
            "One. Two.".into(),
            vec![
                SpeechTextSpan {
                    start_byte: 0,
                    end_byte: 5,
                    start_ms: 0,
                    end_ms: 1_000,
                },
                SpeechTextSpan {
                    start_byte: 5,
                    end_byte: 9,
                    start_ms: 1_000,
                    end_ms: 2_000,
                },
            ],
            SpeechTimingKind::Segment,
            3_000,
        ))
    }
}

#[test]
fn authoritative_timing_reaches_live_rows_storage_and_physical_playback_once() {
    let dir = tempfile::tempdir().unwrap();
    let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
    let mut settings = engine.settings();
    let provider = new_account(ProviderKind::AppleSpeech, settings.providers.clone());
    settings.meeting_stt = ModelChoice {
        provider_id: provider.id.clone(),
        model: "dictation".into(),
    };
    settings.providers.push(provider);
    engine.update_settings(settings).unwrap();
    let recognizer = Arc::new(TimedSpeech(AtomicUsize::new(0)));
    engine.set_system_transcriber(recognizer.clone());
    engine.set_timed_system_transcriber(recognizer.clone());
    let session = engine
        .clone()
        .start_meeting(
            "Owned timed transcript".into(),
            ApiKeys {
                by_provider: HashMap::new(),
            },
        )
        .unwrap();
    let audio: Vec<f32> = (0..48_000).map(|n| (n as f32 * 0.17).sin() * 0.2).collect();
    session
        .push_audio_at(Speaker::Others, 5_000, audio)
        .unwrap();
    session.stop().unwrap();
    assert_eq!(recognizer.0.load(Ordering::SeqCst), 1);
    let live: serde_json::Value =
        serde_json::from_str(&session.transcript_snapshot_json().unwrap()).unwrap();
    let persisted: serde_json::Value = serde_json::from_str(
        &engine
            .meeting_transcript_snapshot_json(session.id())
            .unwrap(),
    )
    .unwrap();
    let row = &live["transcript"][0];
    assert_eq!(row["text"], "One. Two.");
    assert_eq!(row["source_id"], "system_audio");
    assert_eq!(row["start_ms"], 5_000);
    assert_eq!(row["end_ms"], 8_000);
    assert_eq!(
        row["utterance_id"],
        persisted["transcript"][0]["utterance_id"]
    );
    assert_eq!(
        row["source_fingerprint"],
        persisted["transcript"][0]["source_fingerprint"]
    );
    let snapshot = engine
        .meeting_store()
        .read_transcript_snapshot(&session.id())
        .unwrap();
    assert_eq!(
        snapshot.utterances[0].metadata.spans[0],
        TimedTextSpan(0, 5, 5_000, 6_000)
    );
    let playback: serde_json::Value = serde_json::from_str(
        &engine
            .meeting_utterance_audio_json(
                session.id(),
                row["utterance_id"].as_str().unwrap().into(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(playback["source"], "others");
    assert_eq!(playback["start_ms"], 5_000);
}

#[test]
fn unknown_legacy_physical_source_refuses_utterance_playback() {
    let dir = tempfile::tempdir().unwrap();
    let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
    let store = engine.meeting_store();
    let id = store
        .create("Unknown legacy source", chrono::Local::now())
        .unwrap();
    let segment = Segment {
        start_ms: 0,
        end_ms: 500,
        speaker: Speaker::Me,
        text: "An uncertain source.".into(),
        failed: false,
        voice: Some(2),
    };
    let row = MeetingUtterance::new(segment, None);
    store
        .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
        .unwrap();
    assert!(
        engine
            .meeting_utterance_audio_json(id, row.metadata.utterance_id)
            .is_err()
    );
}

#[test]
fn failed_chunk_retry_retains_utterance_identity_and_authoritative_timing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
    let mut settings = engine.settings();
    let provider = new_account(ProviderKind::AppleSpeech, settings.providers.clone());
    settings.meeting_stt = ModelChoice {
        provider_id: provider.id.clone(),
        model: "dictation".into(),
    };
    settings.providers.push(provider);
    engine.update_settings(settings).unwrap();
    let recognizer = Arc::new(TimedSpeech(AtomicUsize::new(0)));
    engine.set_system_transcriber(recognizer.clone());
    engine.set_timed_system_transcriber(recognizer.clone());
    let store = engine.meeting_store();
    let id = store
        .create("Owned failed timed chunk", chrono::Local::now())
        .unwrap();
    let row = MeetingUtterance::new(
        Segment {
            start_ms: 5_000,
            end_ms: 8_000,
            speaker: Speaker::Others,
            text: String::new(),
            failed: true,
            voice: None,
        },
        Some("system_audio".into()),
    );
    store
        .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
        .unwrap();
    let audio: Vec<f32> = (0..48_000).map(|n| (n as f32 * 0.17).sin() * 0.2).collect();
    let path = crate::session::failed_path(&store.dir_of(&id), Speaker::Others, 5_000);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, crate::wav::encode_wav(&audio, 16_000)).unwrap();
    assert_eq!(
        engine
            .retranscribe_failed(
                id.clone(),
                ApiKeys {
                    by_provider: HashMap::new()
                }
            )
            .unwrap(),
        1
    );
    let saved = store.read_transcript_snapshot(&id).unwrap();
    assert_eq!(
        saved.utterances[0].metadata.utterance_id,
        row.metadata.utterance_id
    );
    assert_eq!(
        saved.utterances[0].metadata.source_id,
        row.metadata.source_id
    );
    assert_eq!(
        saved.utterances[0].metadata.spans[0],
        TimedTextSpan(0, 5, 5_000, 6_000)
    );
    assert!(!path.exists());
    assert_eq!(recognizer.0.load(Ordering::SeqCst), 1);
}
