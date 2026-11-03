use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
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
}

impl StoredJob {
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
    segments: Vec<Segment>,
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
    fn visible(&self) -> Vec<Segment> {
        filter_segments(&self.segments, &self.dictations)
            .into_iter()
            .filter(|segment| segment.failed || !segment.text.trim().is_empty())
            .collect()
    }

    fn persisted(&self) -> Vec<Segment> {
        let visible = self.visible();
        self.segments
            .iter()
            .map(|segment| {
                let mut segment = segment.clone();
                if !segment.failed
                    && !visible.iter().any(|shown| {
                        shown.speaker == segment.speaker && shown.start_ms == segment.start_ms
                    })
                {
                    segment.text.clear();
                }
                segment
            })
            .collect()
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }
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
    })
}

fn commit_progress(
    store: &MeetingStore,
    id: &str,
    progress: &Mutex<Progress>,
) -> Result<(), PennelError> {
    let (segments, cleanup, revision, dictations) = {
        let progress = progress.lock().unwrap();
        (
            progress.persisted(),
            progress.cleanup.clone(),
            progress.revision,
            progress.dictations.clone(),
        )
    };
    if !dictations.is_empty() {
        retry_storage(|| store.write_dictation_intervals(id, &dictations))?;
    }
    retry_storage(|| store.write_transcript(id, &segments))?;
    if let Err(error) = retry_storage(|| store.cleanup_committed_audio(id)) {
        progress.lock().unwrap().persistence_error = Some(format!(
            "The text is saved, but recovery audio could not be cleaned up: {error}"
        ));
    }
    let removed: Vec<_> = cleanup
        .into_iter()
        .filter(|path| path.try_exists().is_ok_and(|exists| !exists))
        .collect();
    let mut progress = progress.lock().unwrap();
    progress.cleanup.retain(|path| !removed.contains(path));
    if progress.revision == revision {
        progress.dirty = false;
    }
    Ok(())
}

/// A recording in progress: PCM is bounded in memory; recognition waits on durable local WAV files.
#[derive(uniffi::Object)]
pub struct MeetingSession {
    id: String,
    store: MeetingStore,
    recognition: MeetingRecognition,
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
        let worker_terminal = terminal.clone();
        let worker_store = MeetingStore::new(&root);
        let worker_id = id.clone();
        let worker = std::thread::spawn(move || {
            let mut failed_chunks = 0;
            let mut paused = false;
            for job in stored_receiver {
                let attempted = !paused && !worker_terminal.load(Ordering::SeqCst);
                let segment = match if attempted {
                    transcribe_job(&engine, &settings, &keys, &job, &worker_terminal)
                } else {
                    Ok(job.failed())
                } {
                    Ok(segment) => {
                        if attempted {
                            failed_chunks = if segment.failed { failed_chunks + 1 } else { 0 };
                            if failed_chunks >= 2 {
                                paused = true;
                                worker_progress.lock().unwrap().transcription_warning = Some(
                                    "Transcription is paused. Your audio is being saved; try again after recording.".into()
                                );
                            }
                        }
                        segment
                    }
                    Err(error) => {
                        storage_failure(&worker_progress, &worker_terminal, &error);
                        job.failed()
                    }
                };
                {
                    let mut progress = worker_progress.lock().unwrap();
                    progress
                        .preview
                        .complete(segment.speaker, segment.start_ms, segment.end_ms);
                    if !segment.failed {
                        progress.cleanup.push(job.path);
                    }
                    progress.segments.push(segment);
                    progress.segments.sort_by_key(|segment| segment.start_ms);
                    progress.changed();
                }
                if let Err(error) = commit_progress(&worker_store, &worker_id, &worker_progress) {
                    storage_failure(&worker_progress, &worker_terminal, &error);
                }
                let mut progress = worker_progress.lock().unwrap();
                progress.pending = progress.pending.saturating_sub(1);
            }
        });
        Arc::new(Self {
            id,
            store,
            recognition,
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
            self.progress.lock().unwrap().pending += 1;
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
        commit_progress(&self.store, &self.id, &self.progress)?;
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
) -> Result<Segment, PennelError> {
    let wav = std::fs::read(&job.path).map_err(PennelError::storage)?;
    let mut result = Err(PennelError::NotFound);
    for _ in 0..ATTEMPTS {
        if terminal.load(Ordering::SeqCst) {
            break;
        }
        result = engine.stt(settings, &settings.meeting_stt, &wav, keys);
        if result.is_ok() {
            break;
        }
    }
    let mut segment = job.failed();
    if let Ok(text) = result {
        segment.failed = false;
        segment.text = if is_hallucination(&text) {
            String::new()
        } else {
            text
        };
    }
    Ok(segment)
}

impl MeetingSession {
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
        self.progress.lock().unwrap().preview.pause(paused);
        if let Some(local) = &self.preview_local {
            local.pause(paused);
        }
        if paused && let Some(observer) = self.preview_observer.lock().unwrap().clone() {
            observer.reset();
        }
    }
}

#[uniffi::export]
impl MeetingSession {
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
                    .segments
                    .iter()
                    .any(|segment| segment.failed);
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
                    progress.segments.push(stored.failed());
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
        let recovered = self.store.read_transcript(&self.id);
        {
            let mut progress = self.progress.lock().unwrap();
            for segment in recovered {
                if !progress.segments.iter().any(|existing| {
                    existing.speaker == segment.speaker && existing.start_ms == segment.start_ms
                }) {
                    progress.segments.push(segment);
                    progress.changed();
                }
            }
            progress.pending = 0;
        }
        let result = self.persist_final().and_then(|_| {
            let successful = !self
                .progress
                .lock()
                .unwrap()
                .segments
                .iter()
                .any(|segment| segment.failed);
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
            _: Vec<f32>,
            _: u32,
            _: Option<String>,
            _: Vec<String>,
        ) -> Result<String, PennelError> {
            Ok("Finalized.".into())
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
            session.push_audio_at(source, 0, tone(3.0)).unwrap();
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
            progress.segments.push(Segment {
                start_ms: 0,
                end_ms: 1_000,
                speaker: Speaker::Me,
                text: "fixture".into(),
                failed: false,
                voice: None,
            });
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
        session.progress.lock().unwrap().segments.push(Segment {
            start_ms: 0,
            end_ms: 1_000,
            speaker: Speaker::Others,
            text: "0".into(),
            failed: false,
            voice: None,
        });
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..500 {
                    let mut progress = session.progress.lock().unwrap();
                    progress.changed();
                    progress.segments[0].text = progress.revision.to_string();
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
        let request = server
            .mock("POST", "/audio/transcriptions")
            .with_body_from_request(move |_| {
                let (lock, ready) = &*latch;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = ready.wait(released).unwrap();
                }
                r#"{"text":"eerste tekst"}"#.as_bytes().to_vec()
            })
            .expect(1)
            .create();
        let transcript_path = PathBuf::from(session.dir()).join("transcript.md");
        std::fs::create_dir(&transcript_path).unwrap();
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
}
