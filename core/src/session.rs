use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::chunker::{Chunk, Chunker};
use crate::engine::{ApiKeys, PennelEngine};
use crate::error::PennelError;
use crate::meeting_preview::{
    LocalPreviewQueue, MeetingPreviewAudio, MeetingPreviewAudioObserver, MeetingPreviewMode,
    MeetingPreviewSnapshot, MeetingPreviewState, PreviewDisplay,
};
use crate::meetings::{MeetingStore, Segment, Speaker};
use crate::provider::ProviderKind;
use crate::settings::Settings;
use crate::speech::is_hallucination;
use crate::transcript_filter::filter_segments;
use crate::wav::encode_wav;

const SAMPLE_RATE: u32 = 16_000;
const ATTEMPTS: usize = 2;
const STORAGE_ATTEMPTS: usize = 3;
const INGRESS_CAPACITY: usize = 4;
const MAX_PACKET_SAMPLES: usize = SAMPLE_RATE as usize * 90;
type LiveWorkerSlot = Arc<Mutex<Option<(u64, Arc<crate::live_speakers::LiveSpeakerWorker>)>>>;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct MeetingHealth {
    pub persistence_error: Option<String>,
    pub terminal_error: Option<String>,
    pub transcription_warning: Option<String>,
    pub can_retry_persistence: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct MeetingRecognition {
    pub provider_kind: Option<ProviderKind>,
    pub provider_name: String,
    pub model: String,
    pub language: Option<String>,
}

impl MeetingRecognition {
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        let resolved = settings.resolve(&settings.meeting_stt);
        Self {
            provider_kind: resolved.as_ref().map(|(account, _)| account.kind),
            provider_name: resolved.as_ref().map_or_else(
                || "Not configured".into(),
                |(account, _)| account.name.clone(),
            ),
            model: resolved.map_or_else(|| settings.meeting_stt.model.clone(), |(_, model)| model),
            language: settings.language.clone(),
        }
    }
}

struct Job {
    speaker: Speaker,
    chunk: Chunk,
}

struct StoredJob {
    speaker: Speaker,
    start_ms: u64,
    end_ms: u64,
    path: PathBuf,
    sha256: String,
}

impl StoredJob {
    fn recovery_provenance(
        &self,
        complete: bool,
    ) -> crate::meeting_utterances::RecoveryAudioProvenance {
        crate::meeting_utterances::RecoveryAudioProvenance {
            file_name: self
                .path
                .file_name()
                .expect("owned recovery file")
                .to_string_lossy()
                .into(),
            track: crate::meeting_utterances::RecoveryAudioTrack::from_speaker(self.speaker),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            sha256: self.sha256.clone(),
            disposition: if complete {
                crate::meeting_utterances::RecoveryAudioDisposition::Complete
            } else {
                crate::meeting_utterances::RecoveryAudioDisposition::Pending
            },
        }
    }

    fn failed(&self) -> Segment {
        Segment {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            speaker: self.speaker,
            text: String::new(),
            failed: true,
            voice: None,
        }
    }
}

#[derive(Default)]
struct Progress {
    preview: PreviewDisplay,
    utterances: Vec<crate::MeetingUtterance>,
    metadata_revision: u64,
    live_status: Option<crate::live_speakers::LiveSpeakerStatus>,
    live_updates: [Option<crate::live_speakers::LiveSpeakerUpdate>; 2],
    dictations: Vec<(u64, u64)>,
    pending: u32,
    unpersisted: Vec<Job>,
    cleanup: Vec<PathBuf>,
    dirty: bool,
    revision: u64,
    persistence_error: Option<String>,
    terminal_error: Option<String>,
    transcription_warning: Option<String>,
}

impl Progress {
    fn replace_live_status(&mut self, mut status: crate::live_speakers::LiveSpeakerStatus) {
        if let Some(previous) = &self.live_status {
            status.revision = previous.revision;
            if previous != &status {
                status.revision = status.revision.saturating_add(1);
            }
        }
        self.live_status = Some(status);
    }

    #[cfg(test)]
    fn insert(&mut self, segment: Segment, source: Speaker, spans: Vec<crate::TimedTextSpan>) {
        self.insert_recovery(segment, source, spans, None);
    }

    fn insert_recovery(
        &mut self,
        segment: Segment,
        source: Speaker,
        spans: Vec<crate::TimedTextSpan>,
        recovery: Option<crate::meeting_utterances::RecoveryAudioProvenance>,
    ) {
        let mut row =
            crate::MeetingUtterance::new(segment, Some(physical_source_id(source).into()));
        row.metadata.spans = spans;
        row.metadata.recovery_audio = recovery;
        self.utterances.push(row);
        self.utterances.sort_by_key(|row| row.segment.start_ms);
        if self
            .live_status
            .as_ref()
            .is_some_and(|status| status.enabled)
            && let Some(update) = self.live_updates[index(source)].clone()
        {
            self.apply_live_window(&update);
        }
    }

    fn apply_live_window(&mut self, update: &crate::live_speakers::LiveSpeakerUpdate) -> bool {
        let selected: Vec<_> = self
            .utterances
            .iter()
            .filter(|row| {
                row_source(row) == Some(update.source)
                    && !row.segment.failed
                    && row.metadata.start_ms >= update.start_ms
                    && row.metadata.end_ms <= update.end_ms
            })
            .cloned()
            .collect();
        if selected.is_empty() {
            return false;
        }
        let track = if update.source == Speaker::Me
            || selected
                .iter()
                .all(|row| row.metadata.source_id.as_deref() == Some("room_microphone"))
        {
            crate::diarize::Track::Microphone
        } else {
            crate::diarize::Track::System
        };
        let (after, _) = crate::diarize::apply_utterances_with_voice_map(
            &selected,
            &update.turns,
            track,
            &update.voice_map,
            true,
        );
        let before_text: String = selected
            .iter()
            .map(|row| row.segment.text.as_str())
            .collect();
        let after_text: String = after.iter().map(|row| row.segment.text.as_str()).collect();
        if before_text != after_text
            || after
                .iter()
                .any(|row| row_source(row) != Some(update.source))
        {
            return false;
        }
        if after == selected {
            return false;
        }
        let selected_ids: std::collections::HashSet<_> = selected
            .iter()
            .map(|row| row.metadata.utterance_id.as_str())
            .collect();
        let mut updated: Vec<_> = self
            .utterances
            .iter()
            .filter(|row| !selected_ids.contains(row.metadata.utterance_id.as_str()))
            .cloned()
            .collect();
        updated.extend(after);
        let ids: std::collections::HashSet<_> = updated
            .iter()
            .map(|row| row.metadata.utterance_id.as_str())
            .collect();
        if ids.len() != updated.len() {
            return false;
        }
        updated.sort_by_key(|row| row.segment.start_ms);
        self.utterances = updated;
        self.changed();
        true
    }

    fn visible_rows(&self) -> Vec<crate::MeetingUtterance> {
        let physical: Vec<_> = self
            .utterances
            .iter()
            .filter_map(physical_segment)
            .collect();
        let visible = filter_segments(&physical, &self.dictations);
        let visible_keys: std::collections::HashSet<_> = visible
            .iter()
            .map(|segment| {
                (
                    index(segment.speaker),
                    segment.start_ms,
                    segment.end_ms,
                    segment.text.as_str(),
                    segment.failed,
                    segment.voice,
                )
            })
            .collect();
        self.utterances
            .iter()
            .filter(|row| {
                row_source(row).is_none_or(|source| {
                    visible_keys.contains(&(
                        index(source),
                        row.segment.start_ms,
                        row.segment.end_ms,
                        row.segment.text.as_str(),
                        row.segment.failed,
                        row.segment.voice,
                    ))
                })
            })
            .filter(|row| row.segment.failed || !row.segment.text.trim().is_empty())
            .cloned()
            .collect()
    }

    fn visible(&self) -> Vec<Segment> {
        self.visible_rows()
            .into_iter()
            .map(|row| row.segment)
            .collect()
    }

    fn persisted_rows(&self) -> Vec<crate::MeetingUtterance> {
        let visible = self.visible_rows();
        let ids: std::collections::HashSet<_> = visible
            .iter()
            .map(|row| row.metadata.utterance_id.as_str())
            .collect();
        self.utterances
            .iter()
            .map(|row| {
                let mut row = row.clone();
                if !row.segment.failed && !ids.contains(row.metadata.utterance_id.as_str()) {
                    row.segment.text.clear();
                    row.metadata.spans.clear();
                    row.metadata.refresh_fingerprints(&row.segment);
                }
                row
            })
            .collect()
    }

    fn has_source_start(&self, source: Speaker, start_ms: u64) -> bool {
        self.utterances
            .iter()
            .any(|row| row_source(row) == Some(source) && row.metadata.start_ms == start_ms)
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }
}

fn row_source(row: &crate::MeetingUtterance) -> Option<Speaker> {
    match row.metadata.source_id.as_deref() {
        Some("microphone") => Some(Speaker::Me),
        Some("system" | "system_audio" | "room_microphone") => Some(Speaker::Others),
        _ => None,
    }
}

fn physical_segment(row: &crate::MeetingUtterance) -> Option<Segment> {
    row_source(row).map(|source| Segment {
        speaker: source,
        ..row.segment.clone()
    })
}

/// New chunks retain milliseconds; old whole-second file names remain valid.
pub(crate) fn failed_path(meeting_dir: &Path, speaker: Speaker, start_ms: u64) -> PathBuf {
    let who = match speaker {
        Speaker::Me => "me",
        Speaker::Others => "others",
    };
    meeting_dir
        .join("failed")
        .join(format!("{who}-{start_ms}.wav"))
}

fn retry_storage<T>(mut work: impl FnMut() -> Result<T, PennelError>) -> Result<T, PennelError> {
    let mut result = work();
    for attempt in 1..STORAGE_ATTEMPTS {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20 * attempt as u64));
        result = work();
    }
    result
}

fn storage_failure(progress: &Mutex<Progress>, terminal: &AtomicBool, error: &PennelError) {
    terminal.store(true, Ordering::SeqCst);
    let mut progress = progress.lock().unwrap();
    progress.persistence_error = Some(format!("Saving failed: {error}"));
    progress.terminal_error.get_or_insert_with(|| {
        "Recording stopped because audio or text could not be saved. Try saving again.".into()
    });
}

fn persist_job(store: &MeetingStore, id: &str, job: &Job) -> Result<StoredJob, PennelError> {
    let wav = encode_wav(&job.chunk.samples, SAMPLE_RATE);
    let path = retry_storage(|| store.write_recovery(id, job.speaker, job.chunk.start_ms, &wav))?;
    Ok(StoredJob {
        speaker: job.speaker,
        start_ms: job.chunk.start_ms,
        end_ms: job.chunk.start_ms + job.chunk.samples.len() as u64 * 1000 / u64::from(SAMPLE_RATE),
        path,
        sha256: crate::meeting_utterances::digest(&wav),
    })
}

fn commit_progress(
    store: &MeetingStore,
    id: &str,
    progress: &Mutex<Progress>,
    write_gate: &Mutex<()>,
) -> Result<(), PennelError> {
    let _writing = write_gate.lock().unwrap();
    commit_progress_owned(store, id, progress)
}

fn commit_progress_owned(
    store: &MeetingStore,
    id: &str,
    progress: &Mutex<Progress>,
) -> Result<(), PennelError> {
    let (rows, metadata_revision, cleanup, revision, dictations) = {
        let progress = progress.lock().unwrap();
        let rows = progress.persisted_rows();
        (
            rows,
            progress.metadata_revision,
            progress.cleanup.clone(),
            progress.revision,
            progress.dictations.clone(),
        )
    };
    if !dictations.is_empty() {
        retry_storage(|| store.write_dictation_intervals(id, &dictations))?;
    }
    let saved = retry_storage(|| {
        store
            .write_transcript_snapshot(id, Some(metadata_revision), &rows)
            .or_else(|error| {
                let current = store.read_committed_transcript_snapshot(id)?;
                if current.metadata_status == crate::TranscriptMetadataStatus::Valid
                    && current.utterances == rows
                {
                    Ok(current)
                } else {
                    Err(error)
                }
            })
    })?;
    let cleanup_error = retry_storage(|| store.cleanup_committed_audio(id))
        .err()
        .map(|error| {
            format!("The text is saved, but recovery audio could not be cleaned up: {error}")
        });
    let removed: Vec<_> = cleanup
        .into_iter()
        .filter(|path| !store.recovery_path_may_exist(id, path))
        .collect();
    let mut progress = progress.lock().unwrap();
    progress.metadata_revision = saved.revision;
    progress.cleanup.retain(|path| !removed.contains(path));
    if progress.revision == revision {
        progress.dirty = false;
        if progress.terminal_error.is_none() && cleanup_error.is_none() {
            progress.persistence_error = None;
        }
    }
    if let Some(error) = cleanup_error {
        progress.persistence_error = Some(error);
    }
    Ok(())
}

/// A recording in progress: PCM is bounded in memory; recognition waits on durable local WAV files.
#[derive(uniffi::Object)]
pub struct MeetingSession {
    id: String,
    meeting_uid: String,
    store: MeetingStore,
    recognition: MeetingRecognition,
    engine: Weak<PennelEngine>,
    live_id: String,
    live_generation: AtomicU64,
    live_closed: AtomicBool,
    live_worker: LiveWorkerSlot,
    live_memory: Mutex<Option<Arc<crate::live_speakers::LiveSpeakerMemory>>>,
    write_gate: Arc<Mutex<()>>,
    preview_vocabulary: Vec<String>,
    preview_observer: Mutex<Option<Arc<dyn MeetingPreviewAudioObserver>>>,
    preview_local: Option<Arc<LocalPreviewQueue>>,
    preview_worker: Mutex<Option<JoinHandle<()>>>,
    preview_paused: AtomicBool,
    chunkers: Mutex<[Chunker; 2]>,
    pushed_samples: Mutex<[u64; 2]>,
    dictating: AtomicBool,
    sender: Mutex<Option<SyncSender<Job>>>,
    spool_worker: Mutex<Option<JoinHandle<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    progress: Arc<Mutex<Progress>>,
    terminal: Arc<AtomicBool>,
    ingestion: Mutex<()>,
    stopping: AtomicBool,
    finalized: AtomicBool,
    stop_lock: Mutex<()>,
    stop_result: Mutex<Option<Result<(), PennelError>>>,
}

fn index(speaker: Speaker) -> usize {
    match speaker {
        Speaker::Me => 0,
        Speaker::Others => 1,
    }
}

fn physical_source_id(speaker: Speaker) -> &'static str {
    match speaker {
        Speaker::Me => "microphone",
        Speaker::Others if cfg!(target_os = "ios") => "room_microphone",
        Speaker::Others => "system_audio",
    }
}

struct SessionLiveObserver(Weak<MeetingSession>);

impl crate::live_speakers::LiveSpeakerObserver for SessionLiveObserver {
    fn changed(&self, update: crate::live_speakers::LiveSpeakerUpdate) {
        if let Some(session) = self.0.upgrade() {
            session.apply_live_speaker_update(update);
        }
    }
}

fn unavailable_live_status() -> crate::live_speakers::LiveSpeakerStatus {
    crate::live_speakers::LiveSpeakerStatus {
        enabled: false,
        available: false,
        state: crate::live_speakers::LiveSpeakerState::Unavailable,
        reason_code: Some("unsupported".into()),
        revision: 0,
        model: None,
        destination: None,
    }
}

impl MeetingSession {
    pub(crate) fn start(
        engine: Arc<PennelEngine>,
        store: MeetingStore,
        id: String,
        keys: ApiKeys,
    ) -> Arc<Self> {
        let settings = engine.settings();
        let recognition = MeetingRecognition::from_settings(&settings);
        let (sender, receiver) = mpsc::sync_channel::<Job>(INGRESS_CAPACITY);
        let (stored_sender, stored_receiver) = mpsc::channel::<StoredJob>();
        let progress = Arc::new(Mutex::new(Progress {
            preview: PreviewDisplay::new(recognition.provider_kind, settings.live_text_enabled),
            live_status: Some(engine.live_speaker_availability()),
            ..Default::default()
        }));
        let preview_vocabulary = crate::system_stt::bounded_vocabulary(&settings.dictionary);
        let preview_local = (settings.live_text_enabled
            && recognition.provider_kind == Some(ProviderKind::Local))
        .then(|| Arc::new(LocalPreviewQueue::default()));
        let preview_worker = preview_local.as_ref().map(|queue| {
            let queue = queue.clone();
            let progress = progress.clone();
            let engine = Arc::downgrade(&engine);
            let settings = settings.clone();
            std::thread::spawn(move || {
                while let Some(audio) = queue.next() {
                    let session_id = {
                        let progress = progress.lock().unwrap();
                        let snapshot = progress.preview.snapshot(progress.revision);
                        let track = &snapshot.tracks[index(audio.source)];
                        if progress.pending > 0
                            || track.epoch != audio.epoch
                            || matches!(
                                track.state,
                                MeetingPreviewState::Busy | MeetingPreviewState::Stopped
                            )
                            || audio.cancel.is_cancelled()
                        {
                            queue.finished();
                            continue;
                        }
                        snapshot.session_id
                    };
                    if let Some(engine) = engine.upgrade() {
                        let result = if crate::speech::has_speech(&audio.samples, SAMPLE_RATE) {
                            engine.preview_meeting_local(&settings, &audio.samples, &audio.cancel)
                        } else {
                            Ok(None)
                        };
                        if !audio.cancel.is_cancelled() {
                            let mut progress = progress.lock().unwrap();
                            let excluded = audio.source == Speaker::Me
                                && progress.dictations.iter().any(|(start, end)| {
                                    audio.start_ms < *end && audio.end_ms > *start
                                });
                            match result {
                                _ if excluded => {}
                                Ok(Some(text)) if !is_hallucination(&text) => {
                                    progress.preview.publish(
                                        &session_id,
                                        audio.source,
                                        audio.epoch,
                                        audio.chunk_start_ms,
                                        audio.start_ms,
                                        audio.end_ms,
                                        &text,
                                    );
                                }
                                Ok(_) => {
                                    progress.preview.state(
                                        &session_id,
                                        audio.source,
                                        audio.epoch,
                                        MeetingPreviewState::Warming,
                                    );
                                }
                                Err(PennelError::Cancelled) => {}
                                Err(_) => {
                                    progress.preview.state(
                                        &session_id,
                                        audio.source,
                                        audio.epoch,
                                        MeetingPreviewState::Failed,
                                    );
                                }
                            }
                        }
                    }
                    queue.finished();
                }
            })
        });
        let terminal = Arc::new(AtomicBool::new(false));
        let root = store.root();
        let spool_progress = progress.clone();
        let spool_terminal = terminal.clone();
        let spool_store = MeetingStore::new(&root);
        let spool_id = id.clone();
        let spool_worker = std::thread::spawn(move || {
            let mut failed = false;
            for job in receiver {
                if failed {
                    spool_progress.lock().unwrap().unpersisted.push(job);
                    continue;
                }
                match persist_job(&spool_store, &spool_id, &job) {
                    Ok(stored) => {
                        if stored_sender.send(stored).is_err() {
                            let error = PennelError::Storage {
                                message: "the transcription worker stopped; audio remains available locally".into(),
                            };
                            storage_failure(&spool_progress, &spool_terminal, &error);
                            failed = true;
                        }
                    }
                    Err(error) => {
                        storage_failure(&spool_progress, &spool_terminal, &error);
                        spool_progress.lock().unwrap().unpersisted.push(job);
                        failed = true;
                    }
                }
            }
        });
        let worker_progress = progress.clone();
        let live_worker: LiveWorkerSlot = Arc::new(Mutex::new(None));
        let transcription_live_worker = live_worker.clone();
        let engine_ref = Arc::downgrade(&engine);
        let write_gate = Arc::new(Mutex::new(()));
        let worker_write_gate = write_gate.clone();
        let worker_terminal = terminal.clone();
        let worker_store = MeetingStore::new(&root);
        let worker_id = id.clone();
        let worker = std::thread::spawn(move || {
            let mut failed_chunks = 0;
            let mut paused = false;
            for job in stored_receiver {
                let attempted = !paused && !worker_terminal.load(Ordering::SeqCst);
                let (segment, spans) = match if attempted {
                    transcribe_job(&engine, &settings, &keys, &job, &worker_terminal)
                } else {
                    Ok((job.failed(), Vec::new()))
                } {
                    Ok((segment, spans)) => {
                        if attempted {
                            failed_chunks = if segment.failed { failed_chunks + 1 } else { 0 };
                            if failed_chunks >= 2 {
                                paused = true;
                                worker_progress.lock().unwrap().transcription_warning = Some(
                                    "Transcription is paused. Your audio is being saved; try again after recording.".into()
                                );
                            }
                        }
                        (segment, spans)
                    }
                    Err(error) => {
                        storage_failure(&worker_progress, &worker_terminal, &error);
                        (job.failed(), Vec::new())
                    }
                };
                {
                    let mut progress = worker_progress.lock().unwrap();
                    progress
                        .preview
                        .complete(segment.speaker, segment.start_ms, segment.end_ms);
                    let recovery = job.recovery_provenance(!segment.failed);
                    if !segment.failed {
                        progress.cleanup.push(job.path.clone());
                    }
                    progress.insert_recovery(segment, job.speaker, spans, Some(recovery));
                    progress.changed();
                }
                if let Err(error) = commit_progress(
                    &worker_store,
                    &worker_id,
                    &worker_progress,
                    &worker_write_gate,
                ) {
                    storage_failure(&worker_progress, &worker_terminal, &error);
                }
                let mut progress = worker_progress.lock().unwrap();
                progress.pending = progress.pending.saturating_sub(1);
                if let Some((_, worker)) = transcription_live_worker.lock().unwrap().clone() {
                    worker.set_deferred(progress.pending > 0);
                }
            }
        });
        Arc::new(Self {
            meeting_uid: store.uid_of(&id).unwrap_or_else(|| id.clone()),
            id,
            store,
            recognition,
            engine: engine_ref,
            live_id: uuid::Uuid::now_v7().to_string(),
            live_generation: AtomicU64::new(0),
            live_closed: AtomicBool::new(false),
            live_worker,
            live_memory: Mutex::new(None),
            write_gate,
            preview_vocabulary,
            preview_observer: Mutex::new(None),
            preview_local,
            preview_worker: Mutex::new(preview_worker),
            preview_paused: AtomicBool::new(false),
            chunkers: Mutex::new([Chunker::new(SAMPLE_RATE), Chunker::new(SAMPLE_RATE)]),
            pushed_samples: Mutex::new([0, 0]),
            dictating: AtomicBool::new(false),
            sender: Mutex::new(Some(sender)),
            spool_worker: Mutex::new(Some(spool_worker)),
            worker: Mutex::new(Some(worker)),
            progress,
            terminal,
            ingestion: Mutex::new(()),
            stopping: AtomicBool::new(false),
            finalized: AtomicBool::new(false),
            stop_lock: Mutex::new(()),
            stop_result: Mutex::new(None),
        })
    }

    fn enqueue(&self, speaker: Speaker, chunks: Vec<Chunk>) -> Result<(), PennelError> {
        if !chunks.is_empty()
            && let Some(preview) = &self.preview_local
        {
            preview.preempt();
        }
        let sender = self.sender.lock().unwrap();
        let mut failure = false;
        for chunk in chunks {
            let job = Job { speaker, chunk };
            {
                let mut progress = self.progress.lock().unwrap();
                progress.pending += 1;
                if let Some((_, worker)) = self.live_worker.lock().unwrap().clone() {
                    worker.set_deferred(true);
                    worker.submit_authoritative(speaker, job.chunk.start_ms, &job.chunk.samples);
                }
            }
            let result = if failure {
                Err(TrySendError::Full(job))
            } else if let Some(sender) = sender.as_ref() {
                sender.try_send(job)
            } else {
                Err(TrySendError::Disconnected(job))
            };
            if let Err(error) = result {
                let job = match error {
                    TrySendError::Full(job) | TrySendError::Disconnected(job) => job,
                };
                let mut progress = self.progress.lock().unwrap();
                progress.unpersisted.push(job);
                progress.terminal_error.get_or_insert_with(|| {
                    "Recording stopped because storage could not keep up. Save the remaining audio again.".into()
                });
                self.terminal.store(true, Ordering::SeqCst);
                failure = true;
            }
        }
        if failure {
            Err(PennelError::Storage {
                message: "audio storage could not keep up".into(),
            })
        } else {
            Ok(())
        }
    }

    fn ingest(
        &self,
        speaker: Speaker,
        start_ms: Option<u64>,
        samples: Vec<f32>,
    ) -> Result<(), PennelError> {
        let _ingestion = self.ingestion.lock().unwrap();
        if self.stopping.load(Ordering::SeqCst) {
            return Err(PennelError::Cancelled);
        }
        if self.terminal.load(Ordering::SeqCst) {
            return Err(PennelError::Storage {
                message: "the recording is waiting for storage recovery".into(),
            });
        }
        if samples.is_empty() {
            return Ok(());
        }
        if samples.len() > MAX_PACKET_SAMPLES {
            let error = PennelError::Storage {
                message: "the audio chunk is too large to process safely".into(),
            };
            storage_failure(&self.progress, &self.terminal, &error);
            return Err(error);
        }
        let mut counters = self.pushed_samples.lock().unwrap();
        let count = &mut counters[index(speaker)];
        let mut start = match start_ms {
            Some(ms) => {
                ms.checked_mul(u64::from(SAMPLE_RATE)).ok_or_else(|| {
                    PennelError::InvalidResponse {
                        message: "invalid audio timestamp".into(),
                    }
                })? / 1000
            }
            None => *count,
        };
        // Native sample offsets lose up to 15 samples in the millisecond bridge.
        // Match the desktop timing layer: this rounding is continuity, not a device gap.
        if start.abs_diff(*count) < u64::from(SAMPLE_RATE / 1000) {
            start = *count;
        }
        let end = start.checked_add(samples.len() as u64).ok_or_else(|| {
            PennelError::InvalidResponse {
                message: "invalid audio length".into(),
            }
        })?;
        if end <= *count {
            return Ok(());
        }
        let overlap = count.saturating_sub(start) as usize;
        let mut chunkers = self.chunkers.lock().unwrap();
        let chunker = &mut chunkers[index(speaker)];
        let mut chunks: Vec<Chunk> = chunker.skip_to(start).into_iter().collect();
        let audio = &samples[overlap..];
        let suppressed = speaker == Speaker::Me && self.dictating.load(Ordering::SeqCst);
        if suppressed {
            chunks.extend(chunker.push(&vec![0.0; audio.len()]));
        } else {
            chunks.extend(chunker.push(audio));
        }
        let chunk_start_ms = chunker.current_start_ms();
        let current_start = chunker.current_start_sample();
        let audio_start = end - audio.len() as u64;
        let tail_offset = current_start
            .saturating_sub(audio_start)
            .min(audio.len() as u64) as usize;
        let preview_start = audio_start + tail_offset as u64;
        let preview_audio = &audio[tail_offset..];
        *count = end;
        drop(chunkers);
        drop(counters);
        self.enqueue(speaker, chunks)?;
        let live_worker = self.live_worker.lock().unwrap().clone();
        if !suppressed
            && !self.preview_paused.load(Ordering::SeqCst)
            && let Some((_, worker)) = live_worker
        {
            worker.feed(speaker, audio_start * 1000 / u64::from(SAMPLE_RATE), audio);
        }
        let (update, allows_audio) = {
            let mut progress = self.progress.lock().unwrap();
            progress.preview.advance(
                speaker,
                chunk_start_ms,
                preview_start * 1000 / u64::from(SAMPLE_RATE),
                end * 1000 / u64::from(SAMPLE_RATE),
            );
            (
                progress.preview.snapshot(progress.revision),
                progress.preview.allows_audio(speaker),
            )
        };
        let track = &update.tracks[index(speaker)];
        if !suppressed && !preview_audio.is_empty() && allows_audio {
            if let Some(local) = &self.preview_local {
                local.feed(
                    speaker,
                    track.epoch,
                    chunk_start_ms,
                    preview_start,
                    preview_audio,
                );
            }
            if track.mode == MeetingPreviewMode::System
                && let Some(observer) = self.preview_observer.lock().unwrap().clone()
            {
                observer.audio(MeetingPreviewAudio {
                    session_id: update.session_id,
                    source: speaker,
                    epoch: track.epoch,
                    chunk_start_ms,
                    start_ms: preview_start * 1000 / u64::from(SAMPLE_RATE),
                    samples: preview_audio.to_vec(),
                });
            }
        }
        Ok(())
    }

    fn persist_final(&self) -> Result<(), PennelError> {
        commit_progress(&self.store, &self.id, &self.progress, &self.write_gate)?;
        if !self.progress.lock().unwrap().unpersisted.is_empty() {
            return Err(PennelError::Storage {
                message: "some audio has not been saved yet; try saving again".into(),
            });
        }
        let longest = *self
            .pushed_samples
            .lock()
            .unwrap()
            .iter()
            .max()
            .unwrap_or(&0);
        let minutes = longest.div_ceil(u64::from(SAMPLE_RATE) * 60) as u32;
        retry_storage(|| self.store.set_duration(&self.id, minutes))
    }
}

fn transcribe_job(
    engine: &PennelEngine,
    settings: &Settings,
    keys: &ApiKeys,
    job: &StoredJob,
    terminal: &AtomicBool,
) -> Result<(Segment, Vec<crate::TimedTextSpan>), PennelError> {
    if !job.path.parent().is_some_and(|parent| {
        std::fs::symlink_metadata(parent)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    }) || !std::fs::symlink_metadata(&job.path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        return Err(PennelError::storage(
            "The saved recovery audio is not a valid local file.",
        ));
    }
    let wav = crate::meetings::read_recovery_bytes(&job.path).map_err(PennelError::storage)?;
    if crate::meeting_utterances::digest(&wav) != job.sha256 {
        return Err(PennelError::storage(
            "The saved recovery audio changed before recognition.",
        ));
    }
    let mut result = Err(PennelError::NotFound);
    for _ in 0..ATTEMPTS {
        if terminal.load(Ordering::SeqCst) {
            break;
        }
        result = engine.stt_meeting(settings, &wav, keys);
        if result.is_ok() {
            break;
        }
    }
    let mut segment = job.failed();
    let mut spans = Vec::new();
    if let Ok(transcript) = result {
        segment.failed = false;
        segment.text = if is_hallucination(&transcript.text) {
            String::new()
        } else {
            spans = transcript
                .spans
                .into_iter()
                .map(|span| {
                    crate::TimedTextSpan(
                        span.start_byte,
                        span.end_byte,
                        job.start_ms.saturating_add(span.start_ms),
                        job.start_ms.saturating_add(span.end_ms),
                    )
                })
                .collect();
            transcript.text
        };
    }
    Ok((segment, spans))
}

impl MeetingSession {
    fn apply_live_speaker_update(&self, update: crate::live_speakers::LiveSpeakerUpdate) {
        if update.session_id != self.live_id
            || update.end_ms <= update.start_ms
            || update.end_ms.saturating_sub(update.start_ms) > 30_000
            || self.live_closed.load(Ordering::SeqCst)
            || self.stopping.load(Ordering::SeqCst)
            || self.preview_paused.load(Ordering::SeqCst)
            || self.dictating.load(Ordering::SeqCst)
            || self.engine.upgrade().is_none_or(|engine| {
                engine.settings().speaker_recognition
                    != crate::settings::SpeakerRecognitionMode::Local
            })
        {
            return;
        }
        let save = {
            let mut progress = self.progress.lock().unwrap();
            if self.live_generation.load(Ordering::SeqCst) != update.generation
                || !progress
                    .live_status
                    .as_ref()
                    .is_some_and(|status| status.enabled)
                || progress.live_updates[index(update.source)]
                    .as_ref()
                    .is_some_and(|previous| previous.revision >= update.revision)
            {
                return;
            }
            let changed = progress.apply_live_window(&update);
            let source_index = index(update.source);
            progress.live_updates[source_index] = Some(update);
            (changed || progress.dirty) && progress.pending == 0
        };
        if save
            && let Ok(_writing) = self.write_gate.try_lock()
            && let Err(error) = commit_progress_owned(&self.store, &self.id, &self.progress)
        {
            // Optional attribution saving cannot stop capture or discard the authoritative transcript.
            let mut progress = self.progress.lock().unwrap();
            progress.persistence_error = Some(format!("Saving failed: {error}"));
        }
    }

    pub fn live_speaker_status(&self) -> crate::live_speakers::LiveSpeakerStatus {
        use crate::live_speakers::LiveSpeakerState;
        let worker = self.live_worker.lock().unwrap().clone();
        let availability = self
            .engine
            .upgrade()
            .map(|engine| engine.live_speaker_availability());
        if !self.live_closed.load(Ordering::SeqCst)
            && availability
                .as_ref()
                .is_some_and(|status| !status.available)
            && worker.is_some()
        {
            self.deactivate_live_speakers(false);
        }
        let generation = self.live_generation.load(Ordering::SeqCst);
        let worker_status = worker
            .filter(|(owner, _)| *owner == generation)
            .map(|(_, worker)| worker.status());
        let mut progress = self.progress.lock().unwrap();
        let mut desired = progress
            .live_status
            .clone()
            .unwrap_or_else(unavailable_live_status);
        if self.live_closed.load(Ordering::SeqCst) {
            desired.enabled = false;
            desired.state = LiveSpeakerState::Stopped;
            desired.reason_code = Some("stopped".into());
        } else if let Some(status) = worker_status {
            desired = status;
        } else if !desired.enabled
            && let Some(status) = availability
            && (desired.state != LiveSpeakerState::Failed || !status.available)
        {
            desired = status;
        }
        progress.replace_live_status(desired);
        progress.live_status.clone().unwrap()
    }

    fn deactivate_live_speakers(&self, permanent: bool) {
        use crate::live_speakers::LiveSpeakerState;
        self.live_generation.fetch_add(1, Ordering::SeqCst);
        if permanent {
            self.live_closed.store(true, Ordering::SeqCst);
            self.live_memory.lock().unwrap().take();
        }
        let worker = self.live_worker.lock().unwrap().take();
        if let Some((_, worker)) = worker {
            worker.stop();
        }
        let mut progress = self.progress.lock().unwrap();
        progress.live_updates = [None, None];
        let mut status = progress
            .live_status
            .clone()
            .unwrap_or_else(unavailable_live_status);
        status.enabled = false;
        status.state = if self.live_closed.load(Ordering::SeqCst) {
            LiveSpeakerState::Stopped
        } else {
            LiveSpeakerState::Off
        };
        status.reason_code = (status.state == LiveSpeakerState::Stopped).then(|| "stopped".into());
        progress.replace_live_status(status);
    }

    pub fn utterance_snapshot(&self) -> (u64, Vec<crate::MeetingUtterance>) {
        let progress = self.progress.lock().unwrap();
        (progress.revision, progress.visible_rows())
    }

    pub fn transcript_revision(&self) -> u64 {
        self.progress.lock().unwrap().revision
    }

    /// Return the revision under the same lock as its content, so readers cannot miss an update.
    pub fn transcript_snapshot(&self) -> (u64, Vec<Segment>) {
        let progress = self.progress.lock().unwrap();
        (progress.revision, progress.visible())
    }

    pub fn display_snapshot(&self) -> (u64, Vec<Segment>, MeetingPreviewSnapshot) {
        let progress = self.progress.lock().unwrap();
        (
            progress.revision,
            progress.visible(),
            progress.preview.snapshot(progress.revision),
        )
    }

    pub fn preview_status_snapshot(&self) -> (u64, MeetingPreviewSnapshot) {
        let progress = self.progress.lock().unwrap();
        (
            progress.revision,
            progress.preview.snapshot(progress.revision),
        )
    }

    pub fn set_preview_audio_observer(
        &self,
        observer: Option<Arc<dyn MeetingPreviewAudioObserver>>,
    ) {
        *self.preview_observer.lock().unwrap() = observer;
    }

    pub fn set_preview_paused(&self, paused: bool) {
        let _ingestion = self.ingestion.lock().unwrap();
        self.preview_paused.store(paused, Ordering::SeqCst);
        self.update_preview_pause();
    }

    fn update_preview_pause(&self) {
        let paused =
            self.preview_paused.load(Ordering::SeqCst) || self.dictating.load(Ordering::SeqCst);
        {
            let mut progress = self.progress.lock().unwrap();
            progress.preview.pause(paused);
            if paused {
                progress.live_updates = [None, None];
            }
        }
        if let Some(local) = &self.preview_local {
            local.pause(paused);
        }
        if paused && let Some(observer) = self.preview_observer.lock().unwrap().clone() {
            observer.reset();
        }
        if let Some((_, worker)) = self.live_worker.lock().unwrap().clone() {
            worker.set_paused(paused);
        }
    }
}

#[uniffi::export]
impl MeetingSession {
    pub fn live_speaker_status_json(&self) -> Result<String, PennelError> {
        serde_json::to_string(&self.live_speaker_status()).map_err(PennelError::storage)
    }

    pub fn set_live_speaker_attribution(
        self: Arc<Self>,
        enabled: bool,
    ) -> Result<String, PennelError> {
        use crate::live_speakers::LiveSpeakerState;
        if !enabled {
            self.deactivate_live_speakers(false);
            return self.live_speaker_status_json();
        }
        if self.stopping.load(Ordering::SeqCst) || self.live_closed.load(Ordering::SeqCst) {
            return Err(PennelError::Cancelled);
        }
        if self.live_speaker_status().enabled {
            return self.live_speaker_status_json();
        }
        let engine = self.engine.upgrade().ok_or(PennelError::Cancelled)?;
        let mut status = engine.live_speaker_availability();
        if !status.available {
            self.progress.lock().unwrap().replace_live_status(status);
            return self.live_speaker_status_json();
        }
        let generation = self
            .live_generation
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        status.enabled = true;
        status.state = LiveSpeakerState::Warming;
        status.reason_code = None;
        self.progress
            .lock()
            .unwrap()
            .replace_live_status(status.clone());
        let memory = {
            let mut memory = self.live_memory.lock().unwrap();
            if self.live_closed.load(Ordering::SeqCst)
                || self.live_generation.load(Ordering::SeqCst) != generation
            {
                return Err(PennelError::Cancelled);
            }
            memory
                .get_or_insert_with(|| Arc::new(crate::live_speakers::LiveSpeakerMemory::default()))
                .clone()
        };
        let result = engine.start_live_speaker_worker_with_memory(
            self.live_id.clone(),
            generation,
            memory,
            Arc::new(SessionLiveObserver(Arc::downgrade(&self))),
        );
        let worker = match result {
            Ok(worker) => worker,
            Err(error) => {
                let mut progress = self.progress.lock().unwrap();
                if self.live_generation.load(Ordering::SeqCst) == generation {
                    status.enabled = false;
                    status.state = LiveSpeakerState::Failed;
                    status.reason_code = Some("failed".into());
                    progress.replace_live_status(status);
                }
                return Err(error);
            }
        };
        {
            let progress = self.progress.lock().unwrap();
            let mut slot = self.live_worker.lock().unwrap();
            if self.live_generation.load(Ordering::SeqCst) != generation
                || self.live_closed.load(Ordering::SeqCst)
                || self.stopping.load(Ordering::SeqCst)
            {
                drop(slot);
                worker.stop();
                return Err(PennelError::Cancelled);
            }
            worker.set_deferred(progress.pending > 0);
            *slot = Some((generation, worker.clone()));
        }
        worker.set_paused(
            self.preview_paused.load(Ordering::SeqCst) || self.dictating.load(Ordering::SeqCst),
        );
        self.live_speaker_status_json()
    }

    pub fn stop_live_speaker_attribution(&self) {
        self.deactivate_live_speakers(true);
    }

    pub fn transcript_snapshot_json(&self) -> Result<String, PennelError> {
        self.live_speaker_status();
        let (revision, rows, preview, live_status) = {
            let progress = self.progress.lock().unwrap();
            (
                progress.revision,
                progress.visible_rows(),
                progress.preview.snapshot(progress.revision),
                progress.live_status.clone(),
            )
        };
        serde_json::to_string(&serde_json::json!({
            "meeting_uid": self.meeting_uid,
            "revision": revision,
            "metadata_status": "valid",
            "transcript": crate::transcript_projection::rows(&rows)?,
            "preview": preview,
            "live_speaker_attribution": live_status,
        }))
        .map_err(PennelError::storage)
    }

    pub fn preview_snapshot(&self) -> MeetingPreviewSnapshot {
        let progress = self.progress.lock().unwrap();
        progress.preview.snapshot(progress.revision)
    }

    pub fn preview_vocabulary(&self) -> Vec<String> {
        self.preview_vocabulary.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_preview(
        &self,
        session_id: String,
        source: Speaker,
        epoch: u64,
        chunk_start_ms: u64,
        start_ms: u64,
        end_ms: u64,
        text: String,
    ) -> bool {
        let mut progress = self.progress.lock().unwrap();
        if progress
            .dictations
            .iter()
            .any(|(start, end)| source == Speaker::Me && start_ms < *end && end_ms > *start)
        {
            return false;
        }
        progress.preview.publish(
            &session_id,
            source,
            epoch,
            chunk_start_ms,
            start_ms,
            end_ms,
            &text,
        )
    }

    pub fn set_preview_state(
        &self,
        session_id: String,
        source: Speaker,
        epoch: u64,
        state: MeetingPreviewState,
    ) -> bool {
        self.progress
            .lock()
            .unwrap()
            .preview
            .state(&session_id, source, epoch, state)
    }

    pub fn stop_preview(&self) {
        self.progress.lock().unwrap().preview.stop();
        if let Some(local) = &self.preview_local {
            local.stop();
        }
        self.preview_worker.lock().unwrap().take();
        if let Some(observer) = self.preview_observer.lock().unwrap().clone() {
            observer.reset();
        }
    }

    pub fn recognition_config(&self) -> MeetingRecognition {
        self.recognition.clone()
    }

    pub fn is_finalized(&self) -> bool {
        self.finalized.load(Ordering::SeqCst)
    }

    pub fn id(&self) -> String {
        self.id.clone()
    }
    pub fn dir(&self) -> String {
        self.store.dir_of(&self.id).to_string_lossy().into_owned()
    }

    /// Accepts 16 kHz mono samples; failure is available through the session health snapshot.
    pub fn push_audio(&self, speaker: Speaker, samples: Vec<f32>) {
        let _ = self.ingest(speaker, None, samples);
    }

    pub fn push_audio_at(
        &self,
        speaker: Speaker,
        start_ms: u64,
        samples: Vec<f32>,
    ) -> Result<(), PennelError> {
        self.ingest(speaker, Some(start_ms), samples)
    }

    pub fn set_notes(&self, text: String) -> Result<(), PennelError> {
        self.store.write_notes(&self.id, &text)
    }
    pub fn transcript(&self) -> Vec<Segment> {
        self.progress.lock().unwrap().visible()
    }
    pub fn set_dictating(&self, dictating: bool) {
        let _ingestion = self.ingestion.lock().unwrap();
        self.dictating.store(dictating, Ordering::SeqCst);
        self.update_preview_pause();
    }

    pub fn mark_dictation(&self, start_ms: u64, end_ms: u64) {
        let mut progress = self.progress.lock().unwrap();
        progress.dictations.push((start_ms, end_ms));
        progress.preview.exclude(Speaker::Me, start_ms, end_ms);
        progress.changed();
    }

    pub fn pending(&self) -> u32 {
        self.progress.lock().unwrap().pending
    }

    pub fn health(&self) -> MeetingHealth {
        let progress = self.progress.lock().unwrap();
        MeetingHealth {
            persistence_error: progress.persistence_error.clone(),
            terminal_error: progress.terminal_error.clone(),
            transcription_warning: progress.transcription_warning.clone(),
            can_retry_persistence: progress.dirty
                || !progress.unpersisted.is_empty()
                || !progress.cleanup.is_empty()
                || (self.stopping.load(Ordering::SeqCst) && !self.finalized.load(Ordering::SeqCst)),
        }
    }

    /// Flushes capture, joins persistence/recognition workers, then commits the final transcript.
    pub fn stop(&self) -> Result<(), PennelError> {
        let _stop = self.stop_lock.lock().unwrap();
        if let Some(result) = self.stop_result.lock().unwrap().as_ref() {
            return result.clone();
        }
        {
            let _ingestion = self.ingestion.lock().unwrap();
            self.stopping.store(true, Ordering::SeqCst);
            self.stop_live_speaker_attribution();
            self.stop_preview();
            for speaker in [Speaker::Me, Speaker::Others] {
                let rest = self.chunkers.lock().unwrap()[index(speaker)].flush();
                let _ = self.enqueue(speaker, rest.into_iter().collect());
            }
            self.sender.lock().unwrap().take();
        }
        let mut worker_error = None;
        for handle in [
            self.spool_worker.lock().unwrap().take(),
            self.worker.lock().unwrap().take(),
        ]
        .into_iter()
        .flatten()
        {
            if handle.join().is_err() {
                worker_error = Some(PennelError::Storage {
                    message:
                        "a recording worker stopped unexpectedly; saved audio remains available"
                            .into(),
                });
            }
        }
        let result = worker_error
            .map_or_else(|| self.persist_final(), Err)
            .and_then(|_| {
                let successful = !self
                    .progress
                    .lock()
                    .unwrap()
                    .utterances
                    .iter()
                    .any(|row| row.segment.failed);
                self.store
                    .finish_initial_transcription(&self.id, successful)
            });
        if let Err(error) = &result {
            storage_failure(&self.progress, &self.terminal, error);
        }
        self.finalized.store(result.is_ok(), Ordering::SeqCst);
        *self.stop_result.lock().unwrap() = Some(result.clone());
        result
    }

    /// Saves retained material without recording again or repeating successful recognition.
    pub fn retry_persistence(&self) -> Result<(), PennelError> {
        let _stop = self.stop_lock.lock().unwrap();
        if !self.stopping.load(Ordering::SeqCst) {
            return Err(PennelError::Storage {
                message: "finish recording first".into(),
            });
        }
        let retained = std::mem::take(&mut self.progress.lock().unwrap().unpersisted);
        let mut retained = retained.into_iter();
        while let Some(job) = retained.next() {
            match persist_job(&self.store, &self.id, &job) {
                Ok(stored) => {
                    let mut progress = self.progress.lock().unwrap();
                    progress.insert_recovery(
                        stored.failed(),
                        stored.speaker,
                        Vec::new(),
                        Some(stored.recovery_provenance(false)),
                    );
                    progress.pending = progress.pending.saturating_sub(1);
                    progress.changed();
                }
                Err(error) => {
                    let mut progress = self.progress.lock().unwrap();
                    progress.unpersisted.push(job);
                    progress.unpersisted.extend(retained);
                    drop(progress);
                    storage_failure(&self.progress, &self.terminal, &error);
                    return Err(error);
                }
            }
        }
        // A crashed worker may have queued durable files without publishing their segment yet.
        let recovered = self.store.read_transcript_snapshot(&self.id)?.utterances;
        {
            let mut progress = self.progress.lock().unwrap();
            for row in recovered {
                let descendants: Vec<_> = progress
                    .utterances
                    .iter()
                    .filter(|existing| {
                        existing
                            .metadata
                            .lineage
                            .contains(&row.metadata.utterance_id)
                    })
                    .collect();
                let lineage_match = !descendants.is_empty()
                    && descendants
                        .iter()
                        .all(|existing| row_source(existing) == row_source(&row))
                    && descendants
                        .iter()
                        .map(|existing| existing.segment.text.as_str())
                        .collect::<String>()
                        == row.segment.text;
                let exists =
                    progress.utterances.iter().any(|existing| {
                        existing.metadata.utterance_id == row.metadata.utterance_id
                    }) || row_source(&row).is_some_and(|source| {
                        progress.has_source_start(source, row.metadata.start_ms)
                    }) || lineage_match;
                if !exists && !descendants.is_empty() {
                    return Err(PennelError::storage(
                        "The transcript changed during recovery. Review the retained speaker turns before saving again.",
                    ));
                }
                if !exists {
                    progress.utterances.push(row);
                    progress.changed();
                }
            }
            progress.utterances.sort_by_key(|row| row.segment.start_ms);
            progress.pending = 0;
        }
        let result = self.persist_final().and_then(|_| {
            let successful = !self
                .progress
                .lock()
                .unwrap()
                .utterances
                .iter()
                .any(|row| row.segment.failed);
            self.store
                .finish_initial_transcription(&self.id, successful)
        });
        match &result {
            Ok(()) => {
                let mut progress = self.progress.lock().unwrap();
                if progress.cleanup.is_empty() {
                    progress.persistence_error = None;
                }
                progress.terminal_error = None;
                self.terminal.store(false, Ordering::SeqCst);
            }
            Err(error) => storage_failure(&self.progress, &self.terminal, error),
        }
        self.finalized.store(result.is_ok(), Ordering::SeqCst);
        *self.stop_result.lock().unwrap() = Some(result.clone());
        result
    }
}

impl Drop for MeetingSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{ApiKeys, PennelEngine};
    use crate::meetings::Speaker;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    const SR: usize = 16_000;

    fn live_fixture() -> (tempfile::TempDir, Arc<PennelEngine>, Arc<MeetingSession>) {
        let directory = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(directory.path().to_string_lossy().into_owned()).unwrap();
        let session = engine
            .clone()
            .start_meeting(
                "Owned live speaker fixture".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        (directory, engine, session)
    }

    fn synthetic_live_status(enabled: bool) -> crate::live_speakers::LiveSpeakerStatus {
        crate::live_speakers::LiveSpeakerStatus {
            enabled,
            available: true,
            state: if enabled {
                crate::live_speakers::LiveSpeakerState::Running
            } else {
                crate::live_speakers::LiveSpeakerState::Off
            },
            reason_code: None,
            revision: 0,
            model: Some("owned-synthetic-speaker".into()),
            destination: Some("local".into()),
        }
    }

    fn seed_live_row(session: &MeetingSession, source: Speaker) -> String {
        let mut progress = session.progress.lock().unwrap();
        progress.insert(
            Segment {
                start_ms: 0,
                end_ms: 1_000,
                speaker: source,
                text: "Yes no".into(),
                failed: false,
                voice: None,
            },
            source,
            vec![
                crate::TimedTextSpan(0, 3, 0, 500),
                crate::TimedTextSpan(4, 6, 500, 1_000),
            ],
        );
        progress.replace_live_status(synthetic_live_status(true));
        progress.changed();
        progress.utterances[0].metadata.utterance_id.clone()
    }

    fn live_update(
        session: &MeetingSession,
        revision: u64,
        source: Speaker,
        voice: u32,
    ) -> crate::live_speakers::LiveSpeakerUpdate {
        crate::live_speakers::LiveSpeakerUpdate {
            session_id: session.live_id.clone(),
            generation: session.live_generation.load(Ordering::SeqCst),
            revision,
            source,
            start_ms: 0,
            end_ms: 1_000,
            turns: vec![pennel_speakers::Turn {
                start_ms: 0,
                end_ms: 1_000,
                speaker: 7,
            }],
            voice_map: vec![(7, voice)],
        }
    }

    #[test]
    fn live_status_revision_never_rewinds_when_workers_or_enablement_change() {
        let mut progress = Progress::default();
        progress.replace_live_status(synthetic_live_status(false));
        let first = progress.live_status.as_ref().unwrap().revision;
        progress.replace_live_status(synthetic_live_status(true));
        assert!(progress.live_status.as_ref().unwrap().revision > first);
        let enabled = progress.live_status.as_ref().unwrap().revision;
        progress.replace_live_status(synthetic_live_status(false));
        assert!(progress.live_status.as_ref().unwrap().revision > enabled);
        let disabled = progress.live_status.as_ref().unwrap().revision;
        progress.replace_live_status(synthetic_live_status(true));
        assert!(progress.live_status.as_ref().unwrap().revision > disabled);
    }

    #[test]
    fn live_split_cleanup_waits_for_durable_commit_and_preserves_other_track_audio() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned live recovery commit", chrono::Local::now())
            .unwrap();
        let microphone = persist_job(
            &store,
            &id,
            &Job {
                speaker: Speaker::Me,
                chunk: Chunk {
                    start_ms: 0,
                    samples: tone(1.0),
                },
            },
        )
        .unwrap();
        let system = persist_job(
            &store,
            &id,
            &Job {
                speaker: Speaker::Others,
                chunk: Chunk {
                    start_ms: 0,
                    samples: vec![0.3; SR],
                },
            },
        )
        .unwrap();
        let mut progress = Progress::default();
        progress.insert_recovery(
            Segment {
                text: "Yes no".into(),
                failed: false,
                ..microphone.failed()
            },
            Speaker::Me,
            vec![
                crate::TimedTextSpan(0, 3, 100, 400),
                crate::TimedTextSpan(4, 6, 600, 900),
            ],
            Some(microphone.recovery_provenance(true)),
        );
        progress.insert_recovery(
            system.failed(),
            Speaker::Others,
            Vec::new(),
            Some(system.recovery_provenance(false)),
        );
        progress.cleanup.push(microphone.path.clone());
        assert!(
            progress.apply_live_window(&crate::live_speakers::LiveSpeakerUpdate {
                session_id: "owned".into(),
                generation: 0,
                revision: 1,
                source: Speaker::Me,
                start_ms: 0,
                end_ms: 1_000,
                turns: vec![
                    pennel_speakers::Turn {
                        start_ms: 100,
                        end_ms: 400,
                        speaker: 0
                    },
                    pennel_speakers::Turn {
                        start_ms: 600,
                        end_ms: 900,
                        speaker: 1
                    }
                ],
                voice_map: vec![(0, 2), (1, 3)],
            })
        );
        let progress = Mutex::new(progress);
        let transcript = store.dir_of(&id).join("transcript.md");
        std::fs::create_dir(&transcript).unwrap();
        assert!(commit_progress(&store, &id, &progress, &Mutex::new(())).is_err());
        assert!(microphone.path.exists());
        assert!(system.path.exists());
        std::fs::remove_dir(&transcript).unwrap();
        commit_progress(&store, &id, &progress, &Mutex::new(())).unwrap();
        assert!(!microphone.path.exists());
        assert!(system.path.exists());
        assert!(progress.lock().unwrap().cleanup.is_empty());
        let snapshot = MeetingStore::new(directory.path())
            .read_transcript_snapshot(&id)
            .unwrap();
        assert_eq!(snapshot.utterances.len(), 3);
        assert_eq!(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.segment.failed)
                .count(),
            1
        );
        let failed = snapshot
            .utterances
            .iter()
            .find(|row| row.segment.failed)
            .unwrap();
        assert_eq!(failed.metadata.source_id.as_deref(), Some("system_audio"));
        assert_eq!(store.recovery_audio_path(&id, failed), Some(system.path));
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(store.dir_of(&id).join(crate::meeting_utterances::MANIFEST)).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["schema_version"], 4);
    }

    #[test]
    fn stale_write_cannot_claim_an_uncommitted_private_completion_was_saved() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned raw completion CAS", chrono::Local::now())
            .unwrap();
        store.write_transcript_snapshot(&id, Some(0), &[]).unwrap();
        let job = persist_job(
            &store,
            &id,
            &Job {
                speaker: Speaker::Me,
                chunk: Chunk {
                    start_ms: 0,
                    samples: tone(1.0),
                },
            },
        )
        .unwrap();
        let mut progress = Progress::default();
        progress.insert_recovery(
            Segment {
                failed: false,
                ..job.failed()
            },
            Speaker::Me,
            Vec::new(),
            Some(job.recovery_provenance(true)),
        );
        progress.cleanup.push(job.path.clone());
        progress.changed();
        let mut corrupt = std::fs::read(&job.path).unwrap();
        corrupt[0] = b'X';
        std::fs::write(&job.path, corrupt).unwrap();
        let progress = Mutex::new(progress);
        assert!(
            commit_progress(&store, &id, &progress, &Mutex::new(())).is_err(),
            "an old empty public projection is not a durable completion proof"
        );
        assert!(progress.lock().unwrap().dirty);
        assert_eq!(progress.lock().unwrap().metadata_revision, 0);
        assert!(job.path.exists());
        assert!(
            store
                .read_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .is_empty()
        );
    }

    #[test]
    fn stale_write_can_acknowledge_the_same_full_private_completion_already_saved() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned durable completion replay", chrono::Local::now())
            .unwrap();
        let job = persist_job(
            &store,
            &id,
            &Job {
                speaker: Speaker::Me,
                chunk: Chunk {
                    start_ms: 0,
                    samples: tone(1.0),
                },
            },
        )
        .unwrap();
        let mut progress = Progress::default();
        progress.insert_recovery(
            Segment {
                failed: false,
                ..job.failed()
            },
            Speaker::Me,
            Vec::new(),
            Some(job.recovery_provenance(true)),
        );
        progress.cleanup.push(job.path.clone());
        progress.changed();
        store
            .write_transcript_snapshot(&id, Some(0), &progress.persisted_rows())
            .unwrap();
        let mut corrupt = std::fs::read(&job.path).unwrap();
        corrupt[0] = b'X';
        std::fs::write(&job.path, corrupt).unwrap();
        let progress = Mutex::new(progress);
        commit_progress(&store, &id, &progress, &Mutex::new(())).unwrap();
        assert!(!progress.lock().unwrap().dirty);
        assert_eq!(progress.lock().unwrap().metadata_revision, 1);
        assert_eq!(progress.lock().unwrap().cleanup, vec![job.path]);
        assert_eq!(
            store
                .read_committed_transcript_snapshot(&id)
                .unwrap()
                .utterances
                .len(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_bookkeeping_retains_dangling_file_and_directory_entries() {
        for directory_link in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = MeetingStore::new(directory.path());
            let id = store
                .create("Owned no-follow cleanup", chrono::Local::now())
                .unwrap();
            let job = persist_job(
                &store,
                &id,
                &Job {
                    speaker: Speaker::Me,
                    chunk: Chunk {
                        start_ms: 0,
                        samples: tone(1.0),
                    },
                },
            )
            .unwrap();
            let mut progress = Progress::default();
            progress.insert_recovery(
                Segment {
                    failed: false,
                    ..job.failed()
                },
                Speaker::Me,
                Vec::new(),
                Some(job.recovery_provenance(true)),
            );
            progress.cleanup.push(job.path.clone());
            progress.changed();
            std::fs::remove_file(&job.path).unwrap();
            let link = if directory_link {
                let parent = job.path.parent().unwrap();
                std::fs::remove_dir(parent).unwrap();
                parent.to_path_buf()
            } else {
                job.path.clone()
            };
            std::os::unix::fs::symlink(directory.path().join("missing-owned-data"), &link).unwrap();
            let progress = Mutex::new(progress);
            commit_progress(&store, &id, &progress, &Mutex::new(())).unwrap();
            assert_eq!(progress.lock().unwrap().cleanup, vec![job.path]);
            assert!(
                std::fs::symlink_metadata(link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[test]
    fn live_source_updates_preserve_durable_identity_and_valid_manual_corrections() {
        let (_directory, _engine, session) = live_fixture();
        let id = seed_live_row(&session, Speaker::Me);
        session.apply_live_speaker_update(live_update(&session, 1, Speaker::Me, 2));
        let (first_revision, first) = session.utterance_snapshot();
        assert_eq!(first[0].metadata.utterance_id, id);
        assert_eq!(first[0].metadata.source_id.as_deref(), Some("microphone"));
        assert_eq!(first[0].segment.voice, Some(2));
        {
            let mut progress = session.progress.lock().unwrap();
            let row = &mut progress.utterances[0];
            row.metadata.manual = Some(crate::ManualCorrection {
                name: Some("Owned correction".into()),
                voice: Some(2),
                source_fingerprint: row.metadata.source_fingerprint.clone(),
                review_required: false,
            });
        }
        session.apply_live_speaker_update(live_update(&session, 2, Speaker::Me, 3));
        let (second_revision, second) = session.utterance_snapshot();
        assert!(second_revision >= first_revision);
        assert_eq!(second[0].metadata.utterance_id, id);
        assert_eq!(second[0].segment.voice, Some(2));
        assert_eq!(
            second[0].metadata.manual.as_ref().unwrap().name.as_deref(),
            Some("Owned correction")
        );
        session.stop().unwrap();
    }

    #[test]
    fn live_updates_reject_stale_sessions_generations_sources_and_stop_completions() {
        let (_directory, _engine, session) = live_fixture();
        seed_live_row(&session, Speaker::Me);
        let initial = session.utterance_snapshot();
        let mut foreign = live_update(&session, 1, Speaker::Me, 2);
        foreign.session_id = "different-session".into();
        session.apply_live_speaker_update(foreign);
        let mut stale = live_update(&session, 2, Speaker::Me, 2);
        stale.generation += 1;
        session.apply_live_speaker_update(stale);
        session.apply_live_speaker_update(live_update(&session, 3, Speaker::Others, 2));
        assert_eq!(session.utterance_snapshot(), initial);
        let pending = live_update(&session, 4, Speaker::Me, 2);
        session.stop_live_speaker_attribution();
        session.apply_live_speaker_update(pending);
        assert_eq!(session.utterance_snapshot(), initial);
        assert!(!session.live_speaker_status().enabled);
        session.stop().unwrap();
    }

    #[test]
    fn partially_overlapping_rolling_windows_leave_unmeasured_words_and_labels_unchanged() {
        let (_directory, _engine, session) = live_fixture();
        seed_live_row(&session, Speaker::Me);
        let initial = session.utterance_snapshot();
        let mut partial = live_update(&session, 1, Speaker::Me, 2);
        partial.start_ms = 500;
        session.apply_live_speaker_update(partial);
        assert_eq!(session.utterance_snapshot(), initial);
        session.stop().unwrap();
    }

    #[test]
    fn optional_live_commit_skips_a_blocked_writer_and_authoritative_save_keeps_latest_rows() {
        let (_directory, _engine, session) = live_fixture();
        let microphone_id = seed_live_row(&session, Speaker::Me);
        let writing = session.write_gate.lock().unwrap();
        let live_session = session.clone();
        let update = live_update(&session, 1, Speaker::Me, 2);
        let (finished, completion) = mpsc::channel();
        let live = std::thread::spawn(move || {
            live_session.apply_live_speaker_update(update);
            finished.send(()).unwrap();
        });
        completion
            .recv_timeout(Duration::from_secs(2))
            .expect("optional attribution must not wait on the authoritative writer");
        assert!(session.progress.lock().unwrap().dirty);
        {
            let mut progress = session.progress.lock().unwrap();
            progress.insert(
                Segment {
                    start_ms: 2_000,
                    end_ms: 3_000,
                    speaker: Speaker::Others,
                    text: "Owned remote speech.".into(),
                    failed: false,
                    voice: None,
                },
                Speaker::Others,
                Vec::new(),
            );
            progress.changed();
        }
        let authoritative = session.clone();
        let save = std::thread::spawn(move || {
            commit_progress(
                &authoritative.store,
                &authoritative.id,
                &authoritative.progress,
                &authoritative.write_gate,
            )
        });
        drop(writing);
        live.join().unwrap();
        save.join().unwrap().unwrap();
        let saved = session.store.read_transcript_snapshot(&session.id).unwrap();
        assert_eq!(
            saved.metadata_status,
            crate::TranscriptMetadataStatus::Valid
        );
        assert_eq!(saved.utterances.len(), 2);
        assert_eq!(saved.utterances[0].metadata.utterance_id, microphone_id);
        assert_eq!(
            saved.utterances[0].metadata.source_id.as_deref(),
            Some("microphone")
        );
        assert_eq!(saved.utterances[0].segment.voice, Some(2));
        assert_eq!(saved.utterances[1].segment.text, "Owned remote speech.");
        assert!(session.health().terminal_error.is_none());
        session.stop().unwrap();
    }

    #[test]
    fn live_enablement_is_not_carried_to_a_following_meeting() {
        let (_directory, engine, session) = live_fixture();
        assert!(!session.live_speaker_status().enabled);
        seed_live_row(&session, Speaker::Me);
        session.clone().set_live_speaker_attribution(false).unwrap();
        assert!(!session.live_speaker_status().enabled);
        session.stop().unwrap();
        let next = engine
            .start_meeting(
                "Following owned meeting".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        assert!(!next.live_speaker_status().enabled);
        next.stop().unwrap();
    }

    #[test]
    fn session_voice_memory_survives_disable_but_is_released_at_capture_stop() {
        let (_directory, _engine, session) = live_fixture();
        let memory = Arc::new(crate::live_speakers::LiveSpeakerMemory::default());
        *session.live_memory.lock().unwrap() = Some(memory.clone());
        session.clone().set_live_speaker_attribution(false).unwrap();
        assert!(Arc::ptr_eq(
            session.live_memory.lock().unwrap().as_ref().unwrap(),
            &memory
        ));
        session.stop_live_speaker_attribution();
        assert!(session.live_memory.lock().unwrap().is_none());
        session.stop().unwrap();
    }

    #[test]
    fn live_audio_uses_accepted_clock_without_caption_enablement_and_excludes_dictation() {
        let _serial = crate::live_speakers::test_worker_guard();
        struct OwnedInference;
        impl crate::live_speakers::LiveSpeakerInference for OwnedInference {
            fn infer(
                &mut self,
                samples: &[f32],
            ) -> Result<pennel_speakers::Diarization, PennelError> {
                let duration = samples.len() as u64 * 1_000 / 16_000;
                Ok(pennel_speakers::Diarization {
                    turns: vec![pennel_speakers::Turn {
                        start_ms: 0,
                        end_ms: duration,
                        speaker: 0,
                    }],
                    speakers: vec![pennel_speakers::Speaker {
                        speaker: 0,
                        speech_ms: duration,
                        embedding: vec![1.0, 0.0],
                    }],
                })
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(directory.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.live_text_enabled = false;
        engine.update_settings(settings).unwrap();
        let session = engine
            .start_meeting(
                "Owned live accepted audio".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        let worker = crate::live_speakers::LiveSpeakerWorker::start(
            session.live_id.clone(),
            0,
            "owned-test-inference".into(),
            Arc::new(|| true),
            Box::new(|| Ok(Box::new(OwnedInference))),
            Arc::new(SessionLiveObserver(Arc::downgrade(&session))),
        );
        *session.live_worker.lock().unwrap() = Some((0, worker.clone()));
        session
            .progress
            .lock()
            .unwrap()
            .replace_live_status(synthetic_live_status(true));
        let admission_deadline = Instant::now() + Duration::from_secs(10);
        while !worker.is_admitted() && Instant::now() < admission_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.is_admitted());
        session
            .push_audio_at(Speaker::Me, 5_000, tone(10.0))
            .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while session.progress.lock().unwrap().live_updates[0].is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        let update = session.progress.lock().unwrap().live_updates[0]
            .clone()
            .expect("owned native fixture must receive accepted audio");
        assert_eq!((update.start_ms, update.end_ms), (5_000, 15_000));
        assert!(
            session
                .preview_snapshot()
                .tracks
                .iter()
                .all(|track| track.state == MeetingPreviewState::Disabled)
        );
        let calls = worker.metrics().native_calls;
        session.set_dictating(true);
        session
            .push_audio_at(Speaker::Me, 15_000, tone(1.0))
            .unwrap();
        session
            .push_audio_at(Speaker::Others, 0, tone(1.0))
            .unwrap();
        assert_eq!(worker.metrics().pending_samples, 0);
        assert_eq!(worker.metrics().native_calls, calls);
        assert!(
            session
                .progress
                .lock()
                .unwrap()
                .live_updates
                .iter()
                .all(Option::is_none)
        );
        session.set_dictating(false);
        session.stop().unwrap();
    }

    #[test]
    fn delayed_two_track_transcription_keeps_exact_hard_chunk_windows_for_live_attribution() {
        let _serial = crate::live_speakers::test_worker_guard();
        struct DelayedSpeech {
            entered: mpsc::Sender<usize>,
            release: Mutex<mpsc::Receiver<()>>,
            calls: std::sync::atomic::AtomicUsize,
        }
        impl crate::SystemTranscriber for DelayedSpeech {
            fn transcribe(
                &self,
                _: Vec<f32>,
                _: u32,
                _: Option<String>,
                _: Vec<String>,
            ) -> Result<String, PennelError> {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                if call < 2 {
                    self.entered.send(call).unwrap();
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                }
                Ok(match call {
                    0 => "Owned microphone speech remains complete.",
                    1 => "The remote participant confirms another topic.",
                    _ => "Owned final tail.",
                }
                .into())
            }
        }
        struct SingleVoice {
            calls: usize,
        }
        impl crate::live_speakers::LiveSpeakerInference for SingleVoice {
            fn infer(
                &mut self,
                samples: &[f32],
            ) -> Result<pennel_speakers::Diarization, PennelError> {
                let duration = samples.len() as u64 * 1_000 / 16_000;
                let embedding = if self.calls == 0 {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                };
                self.calls += 1;
                Ok(pennel_speakers::Diarization {
                    turns: vec![pennel_speakers::Turn {
                        start_ms: 0,
                        end_ms: duration,
                        speaker: 0,
                    }],
                    speakers: vec![pennel_speakers::Speaker {
                        speaker: 0,
                        speech_ms: duration,
                        embedding,
                    }],
                })
            }
        }
        let (_directory, engine) = preview_engine();
        let mut settings = engine.settings();
        settings.live_text_enabled = false;
        engine.update_settings(settings).unwrap();
        let (entered, requests) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        engine.set_system_transcriber(Arc::new(DelayedSpeech {
            entered,
            release: Mutex::new(blocked),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }));
        let session = engine
            .clone()
            .start_meeting(
                "Owned delayed two-track live attribution".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        let worker = crate::live_speakers::LiveSpeakerWorker::start(
            session.live_id.clone(),
            0,
            "owned-chunk-boundary-inference".into(),
            Arc::new(|| true),
            Box::new(|| Ok(Box::new(SingleVoice { calls: 0 }))),
            Arc::new(SessionLiveObserver(Arc::downgrade(&session))),
        );
        *session.live_worker.lock().unwrap() = Some((0, worker.clone()));
        session
            .progress
            .lock()
            .unwrap()
            .replace_live_status(synthetic_live_status(true));
        let admission_deadline = Instant::now() + Duration::from_secs(10);
        while !worker.is_admitted() && Instant::now() < admission_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.is_admitted());
        session.push_audio_at(Speaker::Me, 0, tone(30.0)).unwrap();
        assert_eq!(requests.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        session
            .push_audio_at(Speaker::Others, 0, tone(30.0))
            .unwrap();
        session
            .push_audio_at(Speaker::Me, 30_000, tone(0.5))
            .unwrap();
        session
            .push_audio_at(Speaker::Others, 30_000, tone(0.5))
            .unwrap();
        assert_eq!(worker.metrics().native_calls, 0);
        release.send(()).unwrap();
        assert_eq!(requests.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        assert_eq!(worker.metrics().native_calls, 0);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let rows = session.utterance_snapshot().1;
            if rows.len() == 2 && rows.iter().all(|row| row.segment.voice.is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let rows = session.utterance_snapshot().1;
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.segment.voice.is_some() && row.metadata.attribution.provisional)
        );
        assert!(
            rows.iter()
                .all(|row| (row.metadata.start_ms, row.metadata.end_ms) == (0, 30_000))
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.segment.text.as_str())
                .collect::<Vec<_>>(),
            [
                "Owned microphone speech remains complete.",
                "The remote participant confirms another topic."
            ]
        );
        let progress = session.progress.lock().unwrap();
        assert!(
            progress
                .live_updates
                .iter()
                .all(|update| update
                    .as_ref()
                    .is_some_and(|update| (update.start_ms, update.end_ms) == (0, 30_000)))
        );
        drop(progress);
        session.stop().unwrap();
    }

    #[test]
    fn canonical_identity_and_dictation_filter_follow_physical_source_after_person_relabel() {
        let mut progress = Progress::default();
        progress.insert(
            Segment {
                start_ms: 0,
                end_ms: 1_000,
                speaker: Speaker::Me,
                text: "Owned microphone fixture.".into(),
                failed: false,
                voice: None,
            },
            Speaker::Me,
            Vec::new(),
        );
        let before = progress.utterances.clone();
        progress.utterances[0].segment.speaker = Speaker::Others;
        let after = progress.utterances.clone();
        assert_eq!(
            after[0].metadata.utterance_id,
            before[0].metadata.utterance_id
        );
        assert_eq!(after[0].metadata.source_id.as_deref(), Some("microphone"));
        progress.dictations.push((0, 1_000));
        assert!(progress.visible().is_empty());
    }

    #[test]
    fn system_speech_relabelled_as_me_never_becomes_microphone_dictation() {
        let mut progress = Progress::default();
        progress.insert(
            Segment {
                start_ms: 0,
                end_ms: 1_000,
                speaker: Speaker::Others,
                text: "Owned remote fixture.".into(),
                failed: false,
                voice: None,
            },
            Speaker::Others,
            Vec::new(),
        );
        let before = progress.utterances.clone();
        progress.utterances[0].segment.speaker = Speaker::Me;
        progress.dictations.push((0, 1_000));
        assert_eq!(progress.visible().len(), 1);
        let after = progress.utterances.clone();
        assert_eq!(
            after[0].metadata.utterance_id,
            before[0].metadata.utterance_id
        );
        assert_eq!(after[0].metadata.source_id, before[0].metadata.source_id);
    }

    #[derive(Default)]
    struct PreviewObserver(Mutex<Vec<MeetingPreviewAudio>>);

    impl MeetingPreviewAudioObserver for PreviewObserver {
        fn audio(&self, audio: MeetingPreviewAudio) {
            self.0.lock().unwrap().push(audio);
        }
    }

    struct FinalSpeech;

    impl crate::SystemTranscriber for FinalSpeech {
        fn transcribe(
            &self,
            samples: Vec<f32>,
            _: u32,
            _: Option<String>,
            _: Vec<String>,
        ) -> Result<String, PennelError> {
            Ok(if samples.len() <= 3 * SR {
                "The room speaker proposes checking the documents."
            } else {
                "A remote participant asks about tomorrow."
            }
            .into())
        }
    }

    fn preview_engine() -> (tempfile::TempDir, Arc<PennelEngine>) {
        let directory = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(directory.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        let account = crate::new_account(ProviderKind::AppleSpeech, settings.providers.clone());
        settings.meeting_stt = crate::ModelChoice {
            provider_id: account.id.clone(),
            model: "dictation".into(),
        };
        settings.providers.push(account);
        settings.dictionary = vec!["Pennel".into()];
        engine.update_settings(settings).unwrap();
        engine.set_system_transcriber(Arc::new(FinalSpeech));
        (directory, engine)
    }

    #[test]
    fn preview_audio_uses_the_trimmed_clock_and_never_forwards_suppressed_pcm() {
        let (_directory, engine) = preview_engine();
        let session = engine
            .start_meeting(
                "Owned caption clock".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        let observer = Arc::new(PreviewObserver::default());
        session.set_preview_audio_observer(Some(observer.clone()));
        session.push_audio_at(Speaker::Me, 0, tone(3.0)).unwrap();
        session
            .push_audio_at(Speaker::Me, 2_000, tone(3.0))
            .unwrap();
        {
            let audio = observer.0.lock().unwrap();
            assert_eq!(audio.len(), 2);
            assert_eq!(audio[1].start_ms, 3_000);
            assert_eq!(audio[1].samples.len(), 2 * SR);
        }
        let before = session.preview_snapshot();
        assert!(session.set_preview_state(
            before.session_id.clone(),
            Speaker::Me,
            before.tracks[0].epoch,
            MeetingPreviewState::Busy
        ));
        session
            .push_audio_at(Speaker::Me, 5_000, tone(1.0))
            .unwrap();
        assert_eq!(
            observer.0.lock().unwrap().len(),
            3,
            "transient native busy must not suppress recovery input"
        );
        assert!(session.set_preview_state(
            before.session_id.clone(),
            Speaker::Me,
            before.tracks[0].epoch,
            MeetingPreviewState::Ready
        ));
        session.set_dictating(true);
        session
            .push_audio_at(Speaker::Me, 6_000, tone(2.0))
            .unwrap();
        assert_eq!(observer.0.lock().unwrap().len(), 3);
        session.set_dictating(false);
        session.mark_dictation(6_000, 8_000);
        assert!(!session.publish_preview(
            before.session_id,
            Speaker::Me,
            before.tracks[0].epoch,
            0,
            0,
            6_000,
            "stale".into()
        ));
        session
            .push_audio_at(Speaker::Me, 8_000, tone(2.0))
            .unwrap();
        assert_eq!(observer.0.lock().unwrap().last().unwrap().start_ms, 8_000);
        session.stop().unwrap();
    }

    #[test]
    fn preview_is_display_only_and_final_snapshot_reconciles_both_tracks() {
        const SENTINEL: &str = "OWNED_PROVISIONAL_SENTINEL";
        let (_directory, engine) = preview_engine();
        let session = engine
            .start_meeting(
                "Owned caption isolation".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        for source in [Speaker::Me, Speaker::Others] {
            session
                .push_audio_at(
                    source,
                    0,
                    tone(if source == Speaker::Me { 3.0 } else { 4.0 }),
                )
                .unwrap();
            let snapshot = session.preview_snapshot();
            assert!(session.publish_preview(
                snapshot.session_id,
                source,
                snapshot.tracks[index(source)].epoch,
                0,
                0,
                3_000,
                SENTINEL.into()
            ));
        }
        let (revision, transcript, preview) = session.display_snapshot();
        assert_eq!(preview.transcript_revision, revision);
        assert!(transcript.is_empty());
        assert!(session.transcript().is_empty());
        assert_eq!(
            preview
                .tracks
                .iter()
                .filter(|track| track.text == SENTINEL)
                .count(),
            2
        );
        session.stop().unwrap();
        let (revision, transcript, preview) = session.display_snapshot();
        assert_eq!(preview.transcript_revision, revision);
        assert_eq!(transcript.len(), 2);
        assert!(
            transcript
                .iter()
                .any(|segment| segment.speaker == Speaker::Me
                    && segment.end_ms == 3_000
                    && segment.text == "The room speaker proposes checking the documents.")
        );
        assert!(
            transcript
                .iter()
                .any(|segment| segment.speaker == Speaker::Others
                    && segment.end_ms == 4_000
                    && segment.text == "A remote participant asks about tomorrow.")
        );
        assert!(preview.tracks.iter().all(|track| track.text.is_empty()));
        fn inspect(path: &Path) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    inspect(&path);
                } else {
                    assert!(
                        !std::fs::read(&path)
                            .unwrap()
                            .windows(SENTINEL.len())
                            .any(|window| window == SENTINEL.as_bytes())
                    );
                }
            }
        }
        inspect(Path::new(&session.dir()));
    }

    #[test]
    fn preview_priority_is_counted_and_vocabulary_stays_frozen() {
        let (_directory, engine) = preview_engine();
        let session = engine
            .clone()
            .start_meeting(
                "Owned caption priority".into(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        engine.begin_meeting_preview_priority();
        engine.begin_meeting_preview_priority();
        engine.end_meeting_preview_priority();
        assert_eq!(
            session.preview_snapshot().tracks[0].state,
            MeetingPreviewState::Busy
        );
        session.set_dictating(true);
        engine.end_meeting_preview_priority();
        assert_eq!(
            session.preview_snapshot().tracks[0].state,
            MeetingPreviewState::Busy
        );
        session.set_dictating(false);
        assert_eq!(
            session.preview_snapshot().tracks[0].state,
            MeetingPreviewState::Warming
        );
        let mut settings = engine.settings();
        settings.dictionary = vec!["changed after recording started".into()];
        engine.update_settings(settings).unwrap();
        assert_eq!(session.preview_vocabulary(), vec!["Pennel"]);
        session.stop().unwrap();
    }

    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * SR as f32) as usize)
            .map(|i| 0.2 * (i as f32 * 440.0 * std::f32::consts::TAU / SR as f32).sin())
            .collect()
    }

    fn keys() -> ApiKeys {
        ApiKeys {
            by_provider: HashMap::from([("groq".to_owned(), "k".to_owned())]),
        }
    }

    fn engine(dir: &tempfile::TempDir, server: &mockito::Server) -> Arc<PennelEngine> {
        let engine =
            PennelEngine::new(dir.path().join("data").to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers[0].base_url = server.url();
        settings.meeting_stt.provider_id = "groq".into();
        settings.meetings_dir = dir.path().join("meetings").to_string_lossy().into_owned();
        engine.update_settings(settings).unwrap();
        engine
    }

    fn stt_ok(server: &mut mockito::Server, text: &str) -> mockito::Mock {
        server
            .mock("POST", "/audio/transcriptions")
            .with_body(serde_json::json!({ "text": text }).to_string())
            .create()
    }

    fn wait_idle(session: &MeetingSession) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.pending() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn transcript_revision_tracks_content_and_dictation_filters_but_not_notes() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let session = engine(&dir, &server)
            .start_meeting("Revision".into(), keys())
            .unwrap();
        assert_eq!(session.transcript_revision(), 0);
        assert!(session.transcript_snapshot().1.is_empty());
        {
            let mut progress = session.progress.lock().unwrap();
            progress.insert(
                Segment {
                    start_ms: 0,
                    end_ms: 1_000,
                    speaker: Speaker::Me,
                    text: "fixture".into(),
                    failed: false,
                    voice: None,
                },
                Speaker::Me,
                Vec::new(),
            );
            progress.changed();
        }
        let (revision, transcript) = session.transcript_snapshot();
        assert_eq!(revision, 1);
        assert_eq!(transcript[0].text, "fixture");
        session.set_notes("Saved notes".into()).unwrap();
        assert_eq!(session.transcript_revision(), revision);
        session.mark_dictation(0, 1_000);
        let (filtered_revision, transcript) = session.transcript_snapshot();
        assert_eq!(filtered_revision, revision + 1);
        assert!(transcript.is_empty());
        session.stop().unwrap();
    }

    #[test]
    fn transcript_snapshot_pairs_revision_with_its_own_content() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let session = engine(&dir, &server)
            .start_meeting("Atomic revision".into(), keys())
            .unwrap();
        session.progress.lock().unwrap().insert(
            Segment {
                start_ms: 0,
                end_ms: 1_000,
                speaker: Speaker::Others,
                text: "0".into(),
                failed: false,
                voice: None,
            },
            Speaker::Others,
            Vec::new(),
        );
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..500 {
                    let mut progress = session.progress.lock().unwrap();
                    progress.changed();
                    progress.utterances[0].segment.text = progress.revision.to_string();
                    let row = &mut progress.utterances[0];
                    row.metadata.refresh_fingerprints(&row.segment);
                }
            });
            for _ in 0..500 {
                let (revision, transcript) = session.transcript_snapshot();
                assert_eq!(transcript[0].text, revision.to_string());
            }
        });
        session.stop().unwrap();
    }

    #[test]
    fn session_transcribes_both_speakers_in_order() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        stt_ok(&mut server, "hallo");
        let session = engine.clone().start_meeting("Sync".into(), keys()).unwrap();
        session.push_audio(Speaker::Me, tone(20.0));
        session.push_audio(Speaker::Others, [vec![0.0; 5 * SR], tone(20.0)].concat());
        session.stop().unwrap();
        let transcript = session.transcript();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].speaker, Speaker::Me);
        assert_eq!(transcript[1].speaker, Speaker::Others);
        assert_eq!(transcript[0].end_ms, 20_000);
        let md =
            std::fs::read_to_string(PathBuf::from(session.dir()).join("transcript.md")).unwrap();
        assert!(
            md.contains("**Ik:** hallo") && md.contains("**Anderen:** hallo"),
            "{md}"
        );
        assert_eq!(engine.meetings()[0].duration_min, 1);
    }

    #[test]
    fn transcript_is_written_after_each_chunk() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        stt_ok(&mut server, "eerste stuk");
        let session = engine
            .clone()
            .start_meeting("Crash".into(), keys())
            .unwrap();
        session.push_audio(Speaker::Me, tone(31.0));
        wait_idle(&session);
        let md =
            std::fs::read_to_string(PathBuf::from(session.dir()).join("transcript.md")).unwrap();
        assert!(md.contains("eerste stuk"), "{md}");
    }

    #[test]
    fn failed_chunk_is_saved_and_retranscribed() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let failing = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .expect(2)
            .create();
        let session = engine
            .clone()
            .start_meeting("Wifi weg".into(), keys())
            .unwrap();
        session.push_audio(Speaker::Me, tone(10.0));
        session.stop().unwrap();
        failing.assert();
        let transcript = session.transcript();
        assert!(transcript[0].failed);
        assert!(
            PathBuf::from(session.dir())
                .join("failed/me-0.wav")
                .exists()
        );

        failing.remove();
        stt_ok(&mut server, "hersteld");
        assert_eq!(engine.retranscribe_failed(session.id(), keys()).unwrap(), 1);
        let fixed = engine.meeting_transcript(session.id());
        assert!(!fixed[0].failed);
        assert_eq!(fixed[0].text, "hersteld");
        assert!(
            !PathBuf::from(session.dir())
                .join("failed/me-0.wav")
                .exists()
        );
    }

    #[test]
    fn failed_chunk_off_the_second_is_retranscribed() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let first = stt_ok(&mut server, "eerste");
        let session = engine.clone().start_meeting("Gat".into(), keys()).unwrap();
        let quiet = vec![0.0; SR];
        session.push_audio(Speaker::Me, [tone(20.0), quiet, tone(25.0)].concat());
        wait_idle(&session);
        first.remove();
        let failing = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .expect(2)
            .create();
        session.stop().unwrap();
        failing.assert();
        let failed = session.transcript().into_iter().find(|s| s.failed).unwrap();
        assert_ne!(
            failed.start_ms % 1000,
            0,
            "test needs an off-second chunk start"
        );

        failing.remove();
        stt_ok(&mut server, "tweede");
        assert_eq!(engine.retranscribe_failed(session.id(), keys()).unwrap(), 1);
        assert!(
            engine
                .meeting_transcript(session.id())
                .iter()
                .all(|s| !s.failed)
        );
    }

    #[test]
    fn audio_while_dictating_is_silenced() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let mock = stt_ok(&mut server, "stuur de planning naar Sarah").expect(0);
        let session = engine
            .clone()
            .start_meeting("Dictaat".into(), keys())
            .unwrap();
        session.set_dictating(true);
        session.push_audio(Speaker::Me, tone(8.0));
        session.set_dictating(false);
        session.stop().unwrap();
        mock.assert();
        assert!(session.transcript().is_empty());
    }

    #[test]
    fn session_applies_filters_to_transcript_md() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        stt_ok(&mut server, "stuur de planning naar Sarah vandaag");
        let session = engine
            .clone()
            .start_meeting("Dictaat".into(), keys())
            .unwrap();
        session.push_audio(Speaker::Me, tone(10.0));
        session.mark_dictation(0, 10_000);
        session.stop().unwrap();
        assert!(session.transcript().is_empty());
        let md =
            std::fs::read_to_string(PathBuf::from(session.dir()).join("transcript.md")).unwrap();
        assert!(!md.contains("Sarah"), "{md}");
    }

    #[test]
    fn stop_flushes_remainder() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let mock = stt_ok(&mut server, "kort").expect(1);
        let session = engine.clone().start_meeting("Kort".into(), keys()).unwrap();
        session.push_audio(Speaker::Me, tone(3.0));
        assert_eq!(session.pending(), 0);
        session.stop().unwrap();
        mock.assert();
        assert_eq!(session.transcript()[0].text, "kort");
    }

    #[test]
    fn notes_are_written_and_meeting_is_listed() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let session = engine
            .clone()
            .start_meeting("Notities".into(), keys())
            .unwrap();
        session.set_notes("- export naar Excel".into()).unwrap();
        assert_eq!(engine.meeting_notes(session.id()), "- export naar Excel");
        assert_eq!(engine.meetings()[0].id, session.id());
        assert_eq!(engine.search_meetings("notities".into()).len(), 1);
        session.stop().unwrap();
    }

    #[test]
    fn timestamped_tracks_share_start_and_preserve_device_gaps() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        stt_ok(&mut server, "gesprek");
        let session = engine
            .clone()
            .start_meeting("Tijdlijn".into(), keys())
            .unwrap();
        session.push_audio_at(Speaker::Me, 0, tone(3.0)).unwrap();
        session
            .push_audio_at(Speaker::Others, 60_000, tone(2.0))
            .unwrap();
        session
            .push_audio_at(Speaker::Me, 3_600_000, tone(2.0))
            .unwrap();
        session.stop().unwrap();
        let transcript = session.transcript();
        assert_eq!(
            transcript
                .iter()
                .map(|segment| segment.start_ms)
                .collect::<Vec<_>>(),
            [0, 60_000, 3_600_000]
        );
        assert_eq!(engine.meetings()[0].duration_min, 61);
    }

    #[test]
    fn millisecond_timestamps_preserve_contiguous_resampled_packets() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let lengths = Arc::new(Mutex::new(Vec::new()));
        let received = lengths.clone();
        server
            .mock("POST", "/audio/transcriptions")
            .with_body_from_request(move |request| {
                let body = request.body().unwrap();
                let start = body.windows(4).position(|bytes| bytes == b"RIFF").unwrap();
                let (samples, rate) = crate::wav::decode_wav(&body[start..]).unwrap();
                assert_eq!(rate, 16_000);
                received.lock().unwrap().push(samples.len());
                br#"{"text":"doorlopend gesprek"}"#.to_vec()
            })
            .create();
        let session = engine
            .clone()
            .start_meeting("Afronding".into(), keys())
            .unwrap();
        let packet: Vec<f32> = (0..17_288)
            .map(|i| 0.2 * (i as f32 * 440.0 * std::f32::consts::TAU / SR as f32).sin())
            .collect();
        for speaker in [Speaker::Me, Speaker::Others] {
            for index in 0..60 {
                let offset_ms = index * 17_288 * 1_000 / 16_000;
                session
                    .push_audio_at(speaker, offset_ms, packet.clone())
                    .unwrap();
                wait_idle(&session);
            }
        }
        session.stop().unwrap();
        let transcript = session.transcript();
        assert_eq!(
            transcript.len(),
            6,
            "continuous audio must not flush on rounding"
        );
        for speaker in [Speaker::Me, Speaker::Others] {
            assert_eq!(
                transcript
                    .iter()
                    .filter(|segment| segment.speaker == speaker)
                    .map(|segment| (segment.start_ms, segment.end_ms))
                    .collect::<Vec<_>>(),
                [(0, 30_000), (30_000, 60_000), (60_000, 64_830)]
            );
        }
        let mut lengths = lengths.lock().unwrap().clone();
        lengths.sort_unstable();
        assert_eq!(
            lengths,
            [77_280, 77_280, 480_000, 480_000, 480_000, 480_000]
        );
        assert_eq!(lengths.iter().sum::<usize>(), 2_074_560);
    }

    #[test]
    fn overlapping_audio_packets_are_trimmed_and_late_packets_ignored() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        stt_ok(&mut server, "gesprek");
        let session = engine
            .clone()
            .start_meeting("Overlap".into(), keys())
            .unwrap();
        session.push_audio_at(Speaker::Me, 0, tone(3.0)).unwrap();
        session
            .push_audio_at(Speaker::Me, 2_000, tone(3.0))
            .unwrap();
        session
            .push_audio_at(Speaker::Me, 1_000, tone(2.0))
            .unwrap();
        session.stop().unwrap();
        assert_eq!(session.transcript()[0].end_ms, 5_000);
        assert_eq!(
            session.push_audio_at(Speaker::Me, 5_000, tone(1.0)),
            Err(PennelError::Cancelled)
        );
        session.stop().unwrap();
        assert_eq!(session.transcript().len(), 1);
    }

    #[test]
    fn active_session_blocks_duplicate_start_library_move_rename_and_delete() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let session = engine
            .clone()
            .start_meeting("Actief".into(), keys())
            .unwrap();
        assert!(
            engine
                .clone()
                .start_meeting("Dubbel".into(), keys())
                .is_err()
        );
        assert!(
            engine
                .rename_meeting(session.id(), "Verplaatst".into())
                .is_err()
        );
        assert!(engine.delete_meeting(session.id()).is_err());
        let mut settings = engine.settings();
        settings.meetings_dir = dir.path().join("other").to_string_lossy().into_owned();
        assert!(engine.update_settings(settings.clone()).is_err());
        session.stop().unwrap();
        engine.update_settings(settings).unwrap();
    }

    #[test]
    fn repeated_stop_preserves_a_finalization_failure() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let session = engine.start_meeting("Opslag weg".into(), keys()).unwrap();
        std::fs::remove_dir_all(session.dir()).unwrap();
        let first = session.stop();
        assert!(first.is_err());
        assert_eq!(session.stop(), first);
    }

    #[test]
    fn recognition_configuration_is_frozen_when_the_meeting_starts() {
        let (dir, mut original) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let mut replacement = mockito::Server::new();
        let engine = engine(&dir, &original);
        let mut settings = engine.settings();
        settings.language = Some("nl".into());
        settings.meeting_stt.model = "original-model".into();
        engine.update_settings(settings).unwrap();
        let original_request = original
            .mock("POST", "/audio/transcriptions")
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::Regex("name=\"language\"\r\n\r\nnl".into()),
                mockito::Matcher::Regex("name=\"model\"\r\n\r\noriginal-model".into()),
            ]))
            .with_body(r#"{"text":"gesprek"}"#)
            .expect(1)
            .create();
        let replacement_request = replacement
            .mock("POST", "/audio/transcriptions")
            .expect(0)
            .create();
        let session = engine.clone().start_meeting("Vast".into(), keys()).unwrap();
        let mut settings = engine.settings();
        settings.providers[0].base_url = replacement.url();
        settings.language = Some("en".into());
        settings.meeting_stt.model = "replacement-model".into();
        engine.update_settings(settings).unwrap();
        let frozen = session.recognition_config();
        assert_eq!(frozen.provider_kind, Some(ProviderKind::Groq));
        assert_eq!(frozen.model, "original-model");
        assert_eq!(frozen.language.as_deref(), Some("nl"));
        assert_eq!(
            serde_json::to_value(&frozen)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            4
        );
        session.push_audio(Speaker::Me, tone(3.0));
        session.stop().unwrap();
        assert_eq!(session.recognition_config(), frozen);
        original_request.assert();
        replacement_request.assert();
        assert_eq!(session.transcript()[0].text, "gesprek");
    }

    #[test]
    fn failed_transcript_commit_retains_audio_and_retries_without_recognition() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let request = stt_ok(&mut server, "tekst blijft beschikbaar").expect(1);
        let session = engine.start_meeting("Opslag".into(), keys()).unwrap();
        let transcript = PathBuf::from(session.dir()).join("transcript.md");
        std::fs::create_dir(&transcript).unwrap();
        session.push_audio(Speaker::Me, tone(3.0));
        assert!(session.stop().is_err());
        assert_eq!(session.transcript()[0].text, "tekst blijft beschikbaar");
        assert!(failed_path(Path::new(&session.dir()), Speaker::Me, 0).exists());
        assert!(session.health().can_retry_persistence);
        assert!(!session.is_finalized());
        std::fs::remove_dir(&transcript).unwrap();
        session.retry_persistence().unwrap();
        request.assert();
        assert!(session.is_finalized());
        assert!(!session.health().can_retry_persistence);
        assert!(!failed_path(Path::new(&session.dir()), Speaker::Me, 0).exists());
        assert!(
            std::fs::read_to_string(transcript)
                .unwrap()
                .contains("tekst blijft beschikbaar")
        );
        session.stop().unwrap();
    }

    #[test]
    fn recovery_audio_is_durable_before_the_provider_receives_it() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let session = engine
            .start_meeting("Eerst bewaren".into(), keys())
            .unwrap();
        let path = failed_path(Path::new(&session.dir()), Speaker::Me, 0);
        let provider_path = path.clone();
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_body_from_request(move |_| {
                let audio =
                    std::fs::read(&provider_path).expect("audio must be saved before upload");
                assert!(crate::wav::decode_wav(&audio).is_some());
                r#"{"text":"bewaard"}"#.as_bytes().to_vec()
            })
            .expect(1)
            .create();
        session.push_audio(Speaker::Me, tone(3.0));
        session.stop().unwrap();
        request.assert();
        assert!(!path.exists());
    }

    #[test]
    fn failed_audio_storage_stops_ingestion_and_retains_chunks_until_save_retry() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let request = stt_ok(&mut server, "mag niet worden opgevraagd").expect(0);
        let session = engine.start_meeting("Schijf vol".into(), keys()).unwrap();
        let recovery = PathBuf::from(session.dir()).join("failed");
        std::fs::write(&recovery, b"blocked").unwrap();
        session.push_audio_at(Speaker::Me, 0, tone(31.0)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while session.health().terminal_error.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(session.health().terminal_error.is_some());
        assert!(session.transcript().is_empty());
        assert!(
            session
                .push_audio_at(Speaker::Me, 31_000, tone(1.0))
                .is_err()
        );
        assert!(session.stop().is_err());
        assert!(!session.is_finalized());
        std::fs::remove_file(&recovery).unwrap();
        session.retry_persistence().unwrap();
        request.assert();
        assert!(session.is_finalized());
        assert_eq!(session.transcript().len(), 2);
        assert!(session.transcript().iter().all(|segment| segment.failed));
        assert_eq!(std::fs::read_dir(recovery).unwrap().count(), 2);
        assert!(session.health().terminal_error.is_none());
        assert!(!session.health().can_retry_persistence);
    }

    #[test]
    fn slow_recognition_backlogs_file_references_while_capture_keeps_persisting() {
        use std::sync::Condvar;

        struct Release(Arc<(Mutex<bool>, Condvar)>);
        impl Drop for Release {
            fn drop(&mut self) {
                *self.0.0.lock().unwrap() = true;
                self.0.1.notify_all();
            }
        }
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let session = engine
            .start_meeting("Trage herkenning".into(), keys())
            .unwrap();
        let latch = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Release(latch.clone());
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_body_from_request(move |_| {
                let (lock, ready) = &*latch;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = ready.wait(released).unwrap();
                }
                r#"{"text":"gesprek"}"#.as_bytes().to_vec()
            })
            .expect(8)
            .create();
        for number in 0..8 {
            session
                .push_audio_at(Speaker::Me, number * 30_000, tone(30.0))
                .unwrap();
            let path = failed_path(Path::new(&session.dir()), Speaker::Me, number * 30_000);
            let deadline = Instant::now() + Duration::from_secs(3);
            while !path.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                path.exists(),
                "each chunk must persist independently of the provider"
            );
        }
        assert_eq!(session.pending(), 8);
        assert!(session.health().terminal_error.is_none());
        assert!(session.progress.lock().unwrap().unpersisted.is_empty());
        drop(release);
        session.stop().unwrap();
        request.assert();
        assert_eq!(session.transcript().len(), 8);
    }

    #[test]
    fn a_full_pcm_queue_retains_overflow_without_blocking_the_capture_caller() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(dir.path());
        let id = store
            .create("Volle wachtrij", chrono::Local::now())
            .unwrap();
        let (sender, _receiver) = mpsc::sync_channel(INGRESS_CAPACITY);
        let session = MeetingSession {
            engine: Weak::new(),
            live_id: "owned-full-queue".into(),
            live_generation: AtomicU64::new(0),
            live_closed: AtomicBool::new(false),
            live_worker: Arc::new(Mutex::new(None)),
            live_memory: Mutex::new(None),
            write_gate: Arc::new(Mutex::new(())),
            meeting_uid: store.uid_of(&id).unwrap(),
            id,
            store,
            preview_vocabulary: Vec::new(),
            preview_observer: Mutex::new(None),
            preview_local: None,
            preview_worker: Mutex::new(None),
            preview_paused: AtomicBool::new(false),
            recognition: MeetingRecognition {
                provider_kind: None,
                provider_name: "Not configured".into(),
                model: String::new(),
                language: None,
            },
            chunkers: Mutex::new([Chunker::new(SAMPLE_RATE), Chunker::new(SAMPLE_RATE)]),
            pushed_samples: Mutex::new([0, 0]),
            dictating: AtomicBool::new(false),
            sender: Mutex::new(Some(sender)),
            spool_worker: Mutex::new(None),
            worker: Mutex::new(None),
            progress: Arc::new(Mutex::new(Progress::default())),
            terminal: Arc::new(AtomicBool::new(false)),
            ingestion: Mutex::new(()),
            stopping: AtomicBool::new(false),
            finalized: AtomicBool::new(false),
            stop_lock: Mutex::new(()),
            stop_result: Mutex::new(None),
        };
        for number in 0..INGRESS_CAPACITY {
            session
                .enqueue(
                    Speaker::Me,
                    vec![Chunk {
                        start_ms: number as u64 * 1_000,
                        samples: vec![0.25; 16_000],
                    }],
                )
                .unwrap();
        }
        let started = Instant::now();
        assert!(
            session
                .enqueue(
                    Speaker::Others,
                    vec![Chunk {
                        start_ms: 0,
                        samples: vec![0.25; 16_000]
                    }]
                )
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(session.progress.lock().unwrap().unpersisted.len(), 1);
        assert!(session.health().terminal_error.is_some());
        assert!(
            session
                .push_audio_at(Speaker::Me, 4_000, tone(1.0))
                .is_err()
        );
    }

    #[test]
    fn repeated_provider_failures_pause_transcription_without_stopping_audio_capture() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .expect(4)
            .create();
        let session = engine.start_meeting("Offline".into(), keys()).unwrap();
        for number in 0..8 {
            session
                .push_audio_at(Speaker::Me, number * 30_000, tone(30.0))
                .unwrap();
            let path = failed_path(Path::new(&session.dir()), Speaker::Me, number * 30_000);
            let deadline = Instant::now() + Duration::from_secs(3);
            while !path.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(path.exists());
        }
        let stopped = Instant::now();
        session.stop().unwrap();
        assert!(stopped.elapsed() < Duration::from_secs(3));
        request.assert();
        assert_eq!(session.transcript().len(), 8);
        assert!(session.transcript().iter().all(|segment| segment.failed));
        assert!(session.health().transcription_warning.is_some());
        assert!(session.health().terminal_error.is_none());
        assert_eq!(
            std::fs::read_dir(PathBuf::from(session.dir()).join("failed"))
                .unwrap()
                .count(),
            8
        );
    }

    #[test]
    fn interruptions_in_the_same_second_keep_distinct_recovery_audio() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_status(503)
            .expect(4)
            .create();
        let session = engine
            .clone()
            .start_meeting("Korte onderbreking".into(), keys())
            .unwrap();
        session.push_audio_at(Speaker::Me, 0, tone(0.35)).unwrap();
        session.push_audio_at(Speaker::Me, 400, tone(0.35)).unwrap();
        session.stop().unwrap();
        request.assert();
        let transcript = engine.meeting_transcript(session.id());
        assert_eq!(transcript.len(), 2);
        assert_eq!((transcript[0].start_ms, transcript[1].start_ms), (0, 400));
        assert!(failed_path(Path::new(&session.dir()), Speaker::Me, 0).exists());
        assert!(failed_path(Path::new(&session.dir()), Speaker::Me, 400).exists());
        request.remove();
        let successful = stt_ok(&mut server, "hersteld").expect(2);
        assert_eq!(engine.retranscribe_failed(session.id(), keys()).unwrap(), 2);
        successful.assert();
        assert!(
            engine
                .meeting_transcript(session.id())
                .iter()
                .all(|segment| !segment.failed)
        );
    }

    #[test]
    fn terminal_storage_failure_skips_provider_calls_for_queued_durable_audio() {
        use std::sync::Condvar;
        struct Release(Arc<(Mutex<bool>, Condvar)>);
        impl Drop for Release {
            fn drop(&mut self) {
                *self.0.0.lock().unwrap() = true;
                self.0.1.notify_all();
            }
        }
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let session = engine.start_meeting("Opslag stop".into(), keys()).unwrap();
        let latch = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Release(latch.clone());
        let (provider_entered, provider_entry) = mpsc::channel();
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_body_from_request(move |_| {
                provider_entered.send(()).unwrap();
                let (lock, ready) = &*latch;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = ready.wait(released).unwrap();
                }
                r#"{"text":"eerste tekst"}"#.as_bytes().to_vec()
            })
            .expect(1)
            .create();
        #[derive(Debug, PartialEq, Eq)]
        enum SpoolCheckpoint {
            Queued(u64),
            Durable,
        }
        // This fixture needs a durable backlog, not a capture-throughput deadline. A one-slot
        // relay preserves real ingestion while acknowledging handoff to the bounded spool.
        let (capture_sender, capture_receiver) = mpsc::sync_channel(1);
        let spool_sender = session
            .sender
            .lock()
            .unwrap()
            .replace(capture_sender)
            .unwrap();
        let spool_worker = session.spool_worker.lock().unwrap().take().unwrap();
        let (checkpoint_sender, checkpoints) = mpsc::channel();
        let relay = std::thread::spawn(move || {
            for (number, job) in capture_receiver.into_iter().enumerate() {
                spool_sender.send(job).unwrap();
                checkpoint_sender
                    .send(SpoolCheckpoint::Queued(number as u64))
                    .unwrap();
            }
            drop(spool_sender);
            spool_worker.join().unwrap();
            checkpoint_sender.send(SpoolCheckpoint::Durable).unwrap();
        });
        let setup_deadline = Instant::now() + Duration::from_secs(30);
        let await_checkpoint = |expected| {
            assert_eq!(
                checkpoints.recv_timeout(setup_deadline.saturating_duration_since(Instant::now())),
                Ok(expected),
                "spool setup: pending={} health={:?}",
                session.pending(),
                session.health()
            );
        };
        let transcript_path = PathBuf::from(session.dir()).join("transcript.md");
        std::fs::create_dir(&transcript_path).unwrap();
        for number in 0..8 {
            let result = session.push_audio_at(Speaker::Me, number * 30_000, tone(30.0));
            assert!(
                result.is_ok(),
                "chunk {number}/8 admission: {result:?}; pending={} health={:?}",
                session.pending(),
                session.health()
            );
            await_checkpoint(SpoolCheckpoint::Queued(number));
        }
        session.sender.lock().unwrap().take();
        await_checkpoint(SpoolCheckpoint::Durable);
        relay.join().unwrap();
        for number in 0..8 {
            let path = failed_path(Path::new(&session.dir()), Speaker::Me, number * 30_000);
            assert!(
                path.is_file(),
                "chunk {number}/8 missing after durable spool checkpoint: pending={} health={:?}",
                session.pending(),
                session.health()
            );
        }
        assert_eq!(session.pending(), 8);
        assert!(
            session.health().terminal_error.is_none(),
            "{:?}",
            session.health()
        );
        assert!(session.progress.lock().unwrap().unpersisted.is_empty());
        assert!(
            provider_entry
                .recv_timeout(setup_deadline.saturating_duration_since(Instant::now()))
                .is_ok(),
            "first provider did not enter: pending={} health={:?}",
            session.pending(),
            session.health()
        );
        drop(release);
        let stopped = Instant::now();
        assert!(session.stop().is_err());
        assert!(stopped.elapsed() < Duration::from_secs(3));
        request.assert();
        assert_eq!(session.transcript()[0].text, "eerste tekst");
        assert_eq!(
            session
                .transcript()
                .iter()
                .filter(|segment| segment.failed)
                .count(),
            7
        );
        std::fs::remove_dir(transcript_path).unwrap();
        session.retry_persistence().unwrap();
        request.assert();
        assert!(session.is_finalized());
        assert_eq!(
            std::fs::read_dir(PathBuf::from(session.dir()).join("failed"))
                .unwrap()
                .count(),
            7
        );
    }

    #[test]
    fn restart_recovery_restores_duration_before_removing_saved_audio() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let engine = engine(&dir, &server);
        let store = MeetingStore::new(Path::new(&engine.settings().meetings_dir));
        let id = store.create("Herstart", chrono::Local::now()).unwrap();
        store
            .write_recovery(
                &id,
                Speaker::Others,
                60_400,
                &encode_wav(&tone(2.0), SAMPLE_RATE),
            )
            .unwrap();
        assert_eq!(engine.meeting_info(id.clone()).unwrap().duration_min, 0);
        let request = stt_ok(&mut server, "herstelde tekst").expect(1);
        assert_eq!(engine.retranscribe_failed(id.clone(), keys()).unwrap(), 1);
        request.assert();
        assert_eq!(engine.meeting_info(id.clone()).unwrap().duration_min, 2);
        assert!(!failed_path(&store.dir_of(&id), Speaker::Others, 60_400).exists());
    }

    #[test]
    fn recovery_proof_is_durable_before_cleanup_and_survives_a_later_timed_split() {
        let directory = tempfile::tempdir().unwrap();
        let store = MeetingStore::new(directory.path());
        let id = store
            .create("Owned timed recovery commit", chrono::Local::now())
            .unwrap();
        let microphone = persist_job(
            &store,
            &id,
            &Job {
                speaker: Speaker::Me,
                chunk: Chunk {
                    start_ms: 0,
                    samples: tone(1.0),
                },
            },
        )
        .unwrap();
        let system = persist_job(
            &store,
            &id,
            &Job {
                speaker: Speaker::Others,
                chunk: Chunk {
                    start_ms: 0,
                    samples: vec![0.3; SR],
                },
            },
        )
        .unwrap();
        let microphone_bytes = std::fs::read(&microphone.path).unwrap();
        let mut progress = Progress::default();
        progress.insert_recovery(
            Segment {
                text: "Yes no".into(),
                failed: false,
                ..microphone.failed()
            },
            Speaker::Me,
            vec![
                crate::TimedTextSpan(0, 3, 100, 400),
                crate::TimedTextSpan(4, 6, 600, 900),
            ],
            Some(microphone.recovery_provenance(true)),
        );
        progress.insert_recovery(
            system.failed(),
            Speaker::Others,
            Vec::new(),
            Some(system.recovery_provenance(false)),
        );
        progress.cleanup.push(microphone.path.clone());
        let progress = Mutex::new(progress);
        let transcript = store.dir_of(&id).join("transcript.md");
        std::fs::create_dir(&transcript).unwrap();
        assert!(commit_progress(&store, &id, &progress, &Mutex::new(())).is_err());
        assert!(microphone.path.exists());
        assert!(system.path.exists());
        std::fs::remove_dir(&transcript).unwrap();
        commit_progress(&store, &id, &progress, &Mutex::new(())).unwrap();
        assert!(!microphone.path.exists());
        assert!(system.path.exists());
        assert!(progress.lock().unwrap().cleanup.is_empty());

        let snapshot = store.read_transcript_snapshot(&id).unwrap();
        let microphone_row = snapshot
            .utterances
            .iter()
            .find(|row| row.metadata.source_id.as_deref() == Some("microphone"))
            .unwrap();
        let (mut split, _) = crate::diarize::apply_utterances(
            std::slice::from_ref(microphone_row),
            &[
                pennel_speakers::Turn {
                    start_ms: 100,
                    end_ms: 400,
                    speaker: 0,
                },
                pennel_speakers::Turn {
                    start_ms: 600,
                    end_ms: 900,
                    speaker: 1,
                },
            ],
            &[],
            crate::diarize::Track::Microphone,
        );
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].segment.start_ms, 100);
        split.extend(
            snapshot
                .utterances
                .iter()
                .filter(|row| row.segment.failed)
                .cloned(),
        );
        store
            .write_transcript_snapshot(&id, Some(snapshot.revision), &split)
            .unwrap();
        std::fs::write(&microphone.path, microphone_bytes).unwrap();
        let reopened = MeetingStore::new(directory.path());
        let snapshot = reopened.read_transcript_snapshot(&id).unwrap();
        assert_eq!(snapshot.utterances.len(), 3);
        let failed = snapshot
            .utterances
            .iter()
            .filter(|row| row.segment.failed)
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 1);
        assert_eq!(
            failed[0].metadata.source_id.as_deref(),
            Some("system_audio")
        );
        assert_eq!(
            reopened.recovery_audio_path(&id, failed[0]),
            Some(system.path.clone())
        );
        assert_eq!(reopened.cleanup_committed_audio(&id).unwrap(), 1);
        assert!(!microphone.path.exists());
        assert!(system.path.exists());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                reopened
                    .dir_of(&id)
                    .join(crate::meeting_utterances::MANIFEST),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["schema_version"], 3);
    }
}
