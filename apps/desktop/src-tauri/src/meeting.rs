//! Records a meeting as two timestamped tracks. The native/runtime owns capture independently of
//! any webview; archived audio and transcript ingestion share the same monotonic session clock.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(target_os = "macos")]
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
#[cfg(target_os = "macos")]
use std::time::Duration;

use pennel_core::{MeetingSession, PennelEngine, Speaker};

#[cfg(not(target_os = "macos"))]
use crate::audio::{Capture, Source};
use crate::diagnostics::Log;
#[cfg(target_os = "linux")]
use crate::linux_meetings as archive_backend;
#[cfg(windows)]
use crate::windows_meetings as archive_backend;

const LOG: Log = Log::new("meeting");
#[cfg(any(not(target_os = "macos"), test))]
const BATCH: usize = 16_000;

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
struct MacCaptureHealth {
    microphone_active: bool,
    microphone_error: Option<String>,
    system_audio_available: bool,
    system_audio_error: Option<String>,
    echo_cancellation: bool,
    archive_error: Option<String>,
    capture_warning: Option<String>,
    terminal_error: Option<String>,
    capture_diagnostics: serde_json::Value,
}

#[cfg(target_os = "macos")]
impl MacCaptureHealth {
    fn from_status(status: serde_json::Value) -> Self {
        serde_json::from_value(status).unwrap_or_else(|_| Self {
            capture_warning: Some("The recording status could not be checked.".into()),
            ..Self::default()
        })
    }

    fn warning(&self) -> Option<String> {
        let mut warnings = Vec::new();
        for warning in [
            &self.microphone_error,
            &self.system_audio_error,
            &self.archive_error,
            &self.capture_warning,
        ]
        .into_iter()
        .flatten()
        {
            let warning = crate::commands::localize_error_message(warning);
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        (!warnings.is_empty()).then(|| warnings.join(" "))
    }
}

/// The status ABI marshals synchronously through AppKit. Its owner polls without Shared's locks;
/// status consumers only read the cache and normal capture finalization runs on a worker.
#[cfg(target_os = "macos")]
struct MacCaptureMonitor {
    capture: crate::macos::MeetingCaptureQuiescence,
    health: Arc<Mutex<MacCaptureHealth>>,
    stop: Option<SyncSender<()>>,
    worker: Option<std::thread::JoinHandle<Result<(), String>>>,
}

#[cfg(target_os = "macos")]
impl MacCaptureMonitor {
    fn start(native: crate::macos::MeetingCapture, engine: Arc<PennelEngine>, id: String) -> Self {
        let capture = native.quiescence();
        let health = Arc::new(Mutex::new(MacCaptureHealth::from_status(native.status())));
        let next_health = health.clone();
        let (stop, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let started = Instant::now();
            let mut last_sequence = 0;
            let mut lost_events = 0;
            loop {
                let mut snapshot = MacCaptureHealth::from_status(native.status());
                if crate::recording_metadata::persist_capture(
                    &engine,
                    &id,
                    &snapshot.capture_diagnostics,
                    started.elapsed().as_millis() as u64,
                    &mut last_sequence,
                    &mut lost_events,
                )
                .is_err()
                {
                    snapshot.capture_warning =
                        Some("The recording diagnostics could not be saved.".into());
                }
                *next_health.lock().unwrap() = snapshot;
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                        let snapshot = MacCaptureHealth::from_status(native.stop()?);
                        crate::recording_metadata::persist_capture(
                            &engine,
                            &id,
                            &snapshot.capture_diagnostics,
                            started.elapsed().as_millis() as u64,
                            &mut last_sequence,
                            &mut lost_events,
                        )?;
                        let result = snapshot.terminal_error.clone().map_or(Ok(()), Err);
                        *next_health.lock().unwrap() = snapshot;
                        break result;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        });
        Self {
            capture,
            health,
            stop: Some(stop),
            worker: Some(worker),
        }
    }

    fn snapshot(&self) -> MacCaptureHealth {
        self.health.lock().unwrap().clone()
    }

    fn quiesce(&self) {
        self.capture.quiesce();
    }

    fn stop(mut self) -> Result<(), String> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.try_send(());
        }
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .unwrap_or_else(|_| Err("The recording status stopped unexpectedly.".into()))
        })
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacCaptureMonitor {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.try_send(());
        }
        // Never join an AppKit-bound worker from a possible main-thread destruction path.
        let _ = self.worker.take();
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct ApplePreviewPending {
    tracks: [Option<pennel_core::MeetingPreviewAudio>; 2],
    next: usize,
    stopped: bool,
    reset: bool,
}

#[cfg(any(target_os = "macos", test))]
enum ApplePreviewDispatch {
    Audio(pennel_core::MeetingPreviewAudio),
    Reset,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct ApplePreviewQueue {
    pending: Mutex<ApplePreviewPending>,
    ready: std::sync::Condvar,
}

#[cfg(any(target_os = "macos", test))]
impl ApplePreviewQueue {
    fn push(&self, audio: pennel_core::MeetingPreviewAudio) {
        // Optional previews never back-pressure the authoritative ingestion path.
        let Ok(mut pending) = self.pending.try_lock() else {
            return;
        };
        if pending.stopped || audio.samples.is_empty() || audio.samples.len() > 6 * 16_000 {
            return;
        }
        let index = usize::from(audio.source == Speaker::Others);
        pending.tracks[index] = Some(audio);
        self.ready.notify_one();
    }

    fn next(&self) -> Option<ApplePreviewDispatch> {
        let mut pending = self.pending.lock().unwrap();
        loop {
            if pending.stopped {
                return None;
            }
            if std::mem::take(&mut pending.reset) {
                return Some(ApplePreviewDispatch::Reset);
            }
            for offset in 0..2 {
                let index = (pending.next + offset) % 2;
                if let Some(audio) = pending.tracks[index].take() {
                    pending.next = (index + 1) % 2;
                    return Some(ApplePreviewDispatch::Audio(audio));
                }
            }
            pending = self.ready.wait(pending).unwrap();
        }
    }

    fn reset(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.tracks = [None, None];
        pending.reset = true;
        self.ready.notify_one();
    }

    fn stop(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.stopped = true;
        pending.tracks = [None, None];
        self.ready.notify_all();
    }
}

#[cfg(target_os = "macos")]
impl pennel_core::MeetingPreviewAudioObserver for ApplePreviewQueue {
    fn audio(&self, audio: pennel_core::MeetingPreviewAudio) {
        self.push(audio);
    }

    fn reset(&self) {
        ApplePreviewQueue::reset(self);
    }
}

#[cfg(target_os = "macos")]
struct AppleMeetingPreview {
    session: Arc<MeetingSession>,
    queue: Arc<ApplePreviewQueue>,
}

#[cfg(target_os = "macos")]
impl AppleMeetingPreview {
    fn start(session: Arc<MeetingSession>) -> Option<Self> {
        use pennel_core::{MeetingPreviewMode, MeetingPreviewState};

        let snapshot = session.preview_snapshot();
        if !snapshot.tracks.iter().any(|track| {
            track.mode == MeetingPreviewMode::System && track.state != MeetingPreviewState::Disabled
        }) {
            return None;
        }
        let queue = Arc::new(ApplePreviewQueue::default());
        let worker_queue = queue.clone();
        let worker_session = session.clone();
        let language = session.recognition_config().language;
        let vocabulary = session.preview_vocabulary();
        let worker = std::thread::Builder::new()
            .name("pennel-meeting-preview".into())
            .spawn(move || {
                let mut services: [Option<crate::macos::AppleSpeechPreview>; 2] = [None, None];
                let mut epochs: [Option<(u64, u64)>; 2] = [None, None];
                let busy: [Arc<AtomicBool>; 2] =
                    std::array::from_fn(|_| Arc::new(AtomicBool::new(false)));
                let mut last_attempt = [0; 2];
                while let Some(dispatch) = worker_queue.next() {
                    let audio = match dispatch {
                        ApplePreviewDispatch::Audio(audio) => audio,
                        ApplePreviewDispatch::Reset => {
                            for service in services.iter_mut().filter_map(Option::take) {
                                service.stop();
                            }
                            epochs = [None, None];
                            continue;
                        }
                    };
                    let index = usize::from(audio.source == Speaker::Others);
                    let epoch = (audio.epoch, audio.chunk_start_ms);
                    if epochs[index] != Some(epoch)
                        || (busy[index].load(Ordering::SeqCst)
                            && audio.start_ms.saturating_sub(last_attempt[index]) >= 3_000)
                    {
                        if let Some(service) = services[index].take() {
                            service.stop();
                        }
                        epochs[index] = Some(epoch);
                        last_attempt[index] = audio.start_ms;
                        busy[index].store(false, Ordering::SeqCst);
                        let callback_busy = busy[index].clone();
                        let callback_session = worker_session.clone();
                        let session_id = audio.session_id.clone();
                        let source = audio.source;
                        let callback_epoch = audio.epoch;
                        let chunk_start_ms = audio.chunk_start_ms;
                        services[index] = crate::macos::AppleSpeechPreview::start_timed(
                            language.as_deref(),
                            &vocabulary,
                            move |event| match event {
                                crate::macos::AppleSpeechPreviewEvent::Fragment {
                                    start_ms,
                                    end_ms,
                                    text,
                                    ..
                                } => {
                                    callback_session.publish_preview(
                                        session_id.clone(),
                                        source,
                                        callback_epoch,
                                        chunk_start_ms,
                                        start_ms,
                                        end_ms,
                                        text,
                                    );
                                }
                                crate::macos::AppleSpeechPreviewEvent::Status {
                                    status,
                                    reason,
                                } => {
                                    use crate::macos::AppleSpeechPreviewStatus;
                                    let state = match status {
                                        AppleSpeechPreviewStatus::Warming => {
                                            MeetingPreviewState::Warming
                                        }
                                        AppleSpeechPreviewStatus::Ready => {
                                            MeetingPreviewState::Ready
                                        }
                                        AppleSpeechPreviewStatus::Unavailable
                                            if reason.as_deref() == Some("busy") =>
                                        {
                                            callback_busy.store(true, Ordering::SeqCst);
                                            MeetingPreviewState::Busy
                                        }
                                        AppleSpeechPreviewStatus::Unavailable => {
                                            MeetingPreviewState::Unsupported
                                        }
                                        AppleSpeechPreviewStatus::Failed => {
                                            MeetingPreviewState::Failed
                                        }
                                    };
                                    callback_session.set_preview_state(
                                        session_id.clone(),
                                        source,
                                        callback_epoch,
                                        state,
                                    );
                                }
                            },
                        )
                        .ok();
                        if services[index].is_none() {
                            worker_session.set_preview_state(
                                audio.session_id.clone(),
                                audio.source,
                                audio.epoch,
                                MeetingPreviewState::Failed,
                            );
                        }
                    }
                    if let Some(service) = &services[index] {
                        service.feed_at(&audio.samples, audio.start_ms);
                    }
                }
                for service in services.into_iter().flatten() {
                    service.stop();
                }
            });
        if worker.is_err() {
            for track in snapshot.tracks {
                session.set_preview_state(
                    snapshot.session_id.clone(),
                    track.source,
                    track.epoch,
                    MeetingPreviewState::Failed,
                );
            }
            return None;
        }
        session.set_preview_audio_observer(Some(queue.clone()));
        Some(Self { session, queue })
    }

    fn stop(&self) {
        self.session.set_preview_audio_observer(None);
        self.queue.stop();
    }
}

#[cfg(target_os = "macos")]
impl Drop for AppleMeetingPreview {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy)]
pub struct RecordingFinish {
    pub duration_ms: u64,
    pub complete: bool,
}

pub struct MeetingRecorder {
    pub session: Arc<MeetingSession>,
    engine: Arc<PennelEngine>,
    pub title: String,
    started: Instant,
    quiescent_ms: Option<u64>,
    capture_failures: Vec<String>,
    #[cfg(test)]
    quiescence_probe: Option<Arc<AtomicBool>>,
    #[cfg(target_os = "macos")]
    native: Option<MacCaptureMonitor>,
    #[cfg(target_os = "macos")]
    preview: Option<AppleMeetingPreview>,
    #[cfg(not(target_os = "macos"))]
    mic: Option<Microphone>,
    #[cfg(not(target_os = "macos"))]
    system: Option<Capture>,
    has_system: bool,
    #[cfg(not(target_os = "macos"))]
    tracks: [Arc<Mutex<TrackSink>>; 2],
    #[cfg(not(target_os = "macos"))]
    track_ingress: Vec<TrackIngress>,
    #[cfg(not(target_os = "macos"))]
    capture_warning: Option<String>,
    #[cfg(windows)]
    echo_active: bool,
    dictating_since: Mutex<Option<u64>>,
    heard_system: Arc<AtomicBool>,
}

#[cfg(not(target_os = "macos"))]
enum Microphone {
    Raw(Capture),
    #[cfg(windows)]
    Echo(crate::windows_meetings::EchoCapture),
}

#[cfg(not(target_os = "macos"))]
impl Microphone {
    fn request_stop(&self) {
        match self {
            Self::Raw(capture) => capture.request_stop(),
            #[cfg(windows)]
            Self::Echo(capture) => capture.request_stop(),
        }
    }
    fn error(&self) -> Option<String> {
        match self {
            Self::Raw(capture) => capture.error(),
            #[cfg(windows)]
            Self::Echo(capture) => capture.error(),
        }
    }

    fn stop(self) -> Result<(), String> {
        match self {
            Self::Raw(capture) => capture.stop_checked(),
            #[cfg(windows)]
            Self::Echo(capture) => capture.stop(),
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
trait CaptureStop {
    fn request_shutdown(&self);
    fn finish_shutdown(self) -> Result<(), String>;
}

#[cfg(not(target_os = "macos"))]
impl CaptureStop for Microphone {
    fn request_shutdown(&self) {
        self.request_stop();
    }
    fn finish_shutdown(self) -> Result<(), String> {
        self.stop()
    }
}

#[cfg(not(target_os = "macos"))]
impl CaptureStop for Capture {
    fn request_shutdown(&self) {
        self.request_stop();
    }
    fn finish_shutdown(self) -> Result<(), String> {
        self.stop_checked()
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn quiesce_sources<M: CaptureStop, S: CaptureStop>(
    microphone: Option<M>,
    system: Option<S>,
) -> Vec<String> {
    if let Some(source) = &microphone {
        source.request_shutdown();
    }
    if let Some(source) = &system {
        source.request_shutdown();
    }
    let mut failures = Vec::new();
    if let Some(source) = microphone
        && let Err(error) = source.finish_shutdown()
    {
        failures.push(error);
    }
    if let Some(source) = system
        && let Err(error) = source.finish_shutdown()
    {
        failures.push(error);
    }
    failures
}

#[cfg(any(not(target_os = "macos"), test))]
enum TrackBlock<'a> {
    Audio { start_ms: u64, samples: &'a [f32] },
    Gap { samples: u64 },
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Default)]
struct TimedTrack {
    position: u64,
    pending_start: u64,
    pending: Vec<f32>,
    output_samples: u64,
    inserted_silence_samples: u64,
    gap_count: u64,
}

#[cfg(any(not(target_os = "macos"), test))]
impl TimedTrack {
    fn push_at(
        &mut self,
        start_ms: u64,
        samples: &[f32],
        mut emit: impl FnMut(TrackBlock<'_>),
    ) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut start = start_ms.checked_mul(16).ok_or("Invalid audio timestamp")?;
        // The shared API timestamps have millisecond precision. Avoid accumulating sub-ms
        // clock-bridge rounding into repeated drops/insertions at every device callback.
        if start.abs_diff(self.position) < 16 {
            start = self.position;
        }
        let end = start
            .checked_add(samples.len() as u64)
            .ok_or("Invalid audio length")?;
        if end <= self.position {
            return Ok(());
        }
        if start > self.position {
            self.flush(&mut emit);
            self.inserted_silence_samples += start - self.position;
            self.gap_count += 1;
            emit(TrackBlock::Gap {
                samples: start - self.position,
            });
            self.position = start;
        }
        let overlap = self.position.saturating_sub(start) as usize;
        let mut remaining = &samples[overlap..];
        self.output_samples += remaining.len() as u64;
        while !remaining.is_empty() {
            if self.pending.is_empty() {
                self.pending_start = self.position;
            }
            let take = remaining.len().min(BATCH - self.pending.len());
            self.pending.extend_from_slice(&remaining[..take]);
            self.position += take as u64;
            remaining = &remaining[take..];
            if self.pending.len() == BATCH {
                self.flush(&mut emit);
            }
        }
        Ok(())
    }

    fn flush(&mut self, mut emit: impl FnMut(TrackBlock<'_>)) {
        if self.pending.is_empty() {
            return;
        }
        let samples = std::mem::take(&mut self.pending);
        emit(TrackBlock::Audio {
            start_ms: self.pending_start / 16,
            samples: &samples,
        });
    }

    fn finish_at(
        &mut self,
        end_ms: u64,
        mut emit: impl FnMut(TrackBlock<'_>),
    ) -> Result<(), String> {
        self.flush(&mut emit);
        let end = end_ms.checked_mul(16).ok_or("Invalid audio timestamp")?;
        if end > self.position {
            let samples = end - self.position;
            self.inserted_silence_samples += samples;
            self.gap_count += 1;
            emit(TrackBlock::Gap { samples });
            self.position = end;
        }
        Ok(())
    }
}

#[cfg(any(not(target_os = "macos"), test))]
struct TrackSink {
    timing: TimedTrack,
    #[cfg(any(windows, target_os = "linux"))]
    archive: Option<archive_backend::AudioArchive>,
    error: Option<String>,
}

#[cfg(any(not(target_os = "macos"), test))]
impl TrackSink {
    #[cfg(not(target_os = "macos"))]
    fn new(
        #[cfg(any(windows, target_os = "linux"))] archive: archive_backend::AudioArchive,
    ) -> Self {
        Self {
            timing: TimedTrack::default(),
            #[cfg(any(windows, target_os = "linux"))]
            archive: Some(archive),
            error: None,
        }
    }

    fn emit(
        session: &MeetingSession,
        speaker: Speaker,
        #[cfg(any(windows, target_os = "linux"))] archive: &mut Option<
            archive_backend::AudioArchive,
        >,
        error: &mut Option<String>,
        block: TrackBlock<'_>,
    ) {
        let archive_result = match block {
            TrackBlock::Gap { samples } => {
                #[cfg(any(windows, target_os = "linux"))]
                {
                    archive
                        .as_mut()
                        .map_or(Ok(()), |writer| writer.write_silence(samples))
                }
                #[cfg(not(any(windows, target_os = "linux")))]
                {
                    let _ = samples;
                    Ok::<(), String>(())
                }
            }
            TrackBlock::Audio { start_ms, samples } => {
                #[cfg(any(windows, target_os = "linux"))]
                let archived = archive
                    .as_mut()
                    .map_or(Ok(()), |writer| writer.write(samples));
                // An archive failure must never discard otherwise recoverable transcript audio.
                if let Err(failure) = session.push_audio_at(speaker, start_ms, samples.to_vec()) {
                    error.get_or_insert_with(|| failure.to_string());
                }
                #[cfg(any(windows, target_os = "linux"))]
                {
                    archived
                }
                #[cfg(not(any(windows, target_os = "linux")))]
                {
                    Ok::<(), String>(())
                }
            }
        };
        if let Err(failure) = archive_result {
            error.get_or_insert(failure);
        }
    }

    fn push_at(
        &mut self,
        session: &MeetingSession,
        speaker: Speaker,
        start_ms: u64,
        samples: &[f32],
    ) {
        let result = self.timing.push_at(start_ms, samples, |block| {
            Self::emit(
                session,
                speaker,
                #[cfg(any(windows, target_os = "linux"))]
                &mut self.archive,
                &mut self.error,
                block,
            );
        });
        if let Err(failure) = result {
            self.error.get_or_insert(failure);
        }
    }

    fn finish(
        &mut self,
        session: &MeetingSession,
        speaker: Speaker,
        end_ms: u64,
    ) -> Result<(), String> {
        let result = self.timing.finish_at(end_ms, |block| {
            Self::emit(
                session,
                speaker,
                #[cfg(any(windows, target_os = "linux"))]
                &mut self.archive,
                &mut self.error,
                block,
            );
        });
        if let Err(failure) = result {
            self.error.get_or_insert(failure);
        }
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(writer) = self.archive.take()
            && let Err(failure) = writer.finish()
        {
            self.error.get_or_insert(failure);
        }
        self.error.clone().map_or(Ok(()), Err)
    }
}

#[cfg(any(not(target_os = "macos"), test))]
struct TrackIngress {
    sender: Option<std::sync::mpsc::SyncSender<(u64, Vec<f32>)>>,
    worker: Option<std::thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    overloaded: Arc<AtomicBool>,
}

#[cfg(any(not(target_os = "macos"), test))]
impl TrackIngress {
    fn start(
        session: Arc<MeetingSession>,
        speaker: Speaker,
        track: Arc<Mutex<TrackSink>>,
    ) -> Result<Self, String> {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<(u64, Vec<f32>)>(64);
        let error = Arc::new(Mutex::new(None));
        let next_error = error.clone();
        let worker = std::thread::Builder::new()
            .name("pennel-track-archive".into())
            .spawn(move || {
                while let Ok((start_ms, samples)) = receiver.recv() {
                    let mut track = track.lock().unwrap();
                    track.push_at(&session, speaker, start_ms, &samples);
                    if let Some(failure) = &track.error {
                        *next_error.lock().unwrap() = Some(failure.clone());
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            error,
            overloaded: Arc::new(AtomicBool::new(false)),
        })
    }

    fn callback(
        &self,
        heard: Option<Arc<AtomicBool>>,
        started: Instant,
    ) -> impl FnMut(&[f32], Instant) + Send + 'static {
        let sender = self.sender.as_ref().unwrap().clone();
        let overloaded = self.overloaded.clone();
        move |chunk, captured_at| {
            if let Some(heard) = &heard
                && chunk.iter().any(|sample| *sample != 0.0)
            {
                heard.store(true, Ordering::Relaxed);
            }
            let start_ms = captured_at.saturating_duration_since(started).as_millis() as u64;
            if sender.try_send((start_ms, chunk.to_vec())).is_err() {
                overloaded.store(true, Ordering::SeqCst);
            }
        }
    }

    fn finish(mut self) -> Result<(), String> {
        self.sender.take();
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Audio processing stopped unexpectedly.".to_owned())?;
        if self.overloaded.load(Ordering::SeqCst) {
            return Err("Audio processing could not keep up with recording.".into());
        }
        self.error.lock().unwrap().clone().map_or(Ok(()), Err)
    }
}

impl MeetingRecorder {
    pub fn start(
        session: Arc<MeetingSession>,
        title: String,
        detected_app: Option<String>,
        echo: bool,
        engine: Arc<PennelEngine>,
        app_version: &str,
    ) -> Result<Self, String> {
        let tracks = engine
            .prepare_meeting_recording(
                session.id(),
                crate::recording_metadata::environment(app_version),
                false,
            )
            .map_err(|error| error.to_string())?;
        let directory = std::path::PathBuf::from(session.dir());
        let track_path = |speaker| {
            tracks
                .iter()
                .find(|track| track.speaker == speaker)
                .map(|track| directory.join(&track.relative_path))
                .ok_or_else(|| "The audio recording is unavailable.".to_owned())
        };
        let microphone_path = track_path(Speaker::Me)?;
        let system_path = track_path(Speaker::Others)?;
        #[cfg(target_os = "macos")]
        {
            let preview = AppleMeetingPreview::start(session.clone());
            let heard_system = Arc::new(AtomicBool::new(false));
            let heard = heard_system.clone();
            let sink = session.clone();
            let started = Instant::now();
            let native = crate::macos::MeetingCapture::start(
                std::path::Path::new(&session.dir()),
                &microphone_path,
                &system_path,
                detected_app.as_deref(),
                echo,
                move |track, samples, offset| {
                    let speaker = if track == 0 {
                        Speaker::Me
                    } else {
                        Speaker::Others
                    };
                    if track == 1 && samples.iter().any(|sample| *sample != 0.0) {
                        heard.store(true, Ordering::Relaxed);
                    }
                    if sink
                        .push_audio_at(speaker, offset, samples.to_vec())
                        .is_err()
                    {
                        LOG.error("An audio block could not be added to the meeting");
                    }
                },
            )?;
            let native = MacCaptureMonitor::start(native, engine.clone(), session.id());
            let has_system = native.snapshot().system_audio_available;
            Ok(Self {
                session,
                engine,
                title,
                started,
                quiescent_ms: None,
                capture_failures: Vec::new(),
                #[cfg(test)]
                quiescence_probe: None,
                native: Some(native),
                preview,
                has_system,
                dictating_since: Mutex::new(None),
                heard_system,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = detected_app;
            #[cfg(any(windows, target_os = "linux"))]
            let tracks = {
                [
                    Arc::new(Mutex::new(TrackSink::new(
                        archive_backend::AudioArchive::create(&microphone_path)?,
                    ))),
                    Arc::new(Mutex::new(TrackSink::new(
                        archive_backend::AudioArchive::create(&system_path)?,
                    ))),
                ]
            };
            #[cfg(not(any(windows, target_os = "linux")))]
            let tracks = {
                let _ = (microphone_path, system_path);
                [
                    Arc::new(Mutex::new(TrackSink::new())),
                    Arc::new(Mutex::new(TrackSink::new())),
                ]
            };
            let started = Instant::now();
            let track_ingress = vec![
                TrackIngress::start(session.clone(), Speaker::Me, tracks[0].clone())?,
                TrackIngress::start(session.clone(), Speaker::Others, tracks[1].clone())?,
            ];
            let heard_system = Arc::new(AtomicBool::new(false));
            #[cfg(windows)]
            let (mic, echo_active, mut capture_warning) = if echo {
                match crate::windows_meetings::EchoCapture::start_timed(
                    track_ingress[0].callback(None, started),
                ) {
                    Ok(capture) => (Microphone::Echo(capture), true, None),
                    Err(error) => {
                        LOG.error(format!("echo cancellation inactive: {error}"));
                        let capture = Capture::start_timed(
                            Source::Microphone,
                            track_ingress[0].callback(None, started),
                        )
                        .map_err(|error| format!("microphone: {error}"))?;
                        (
                            Microphone::Raw(capture),
                            false,
                            Some(format!(
                                "Echo cancellation is not active: {error}. Use headphones."
                            )),
                        )
                    }
                }
            } else {
                (
                    Microphone::Raw(
                        Capture::start_timed(
                            Source::Microphone,
                            track_ingress[0].callback(None, started),
                        )
                        .map_err(|error| format!("microphone: {error}"))?,
                    ),
                    false,
                    None,
                )
            };
            #[cfg(not(windows))]
            let (mic, mut capture_warning) = (
                Microphone::Raw(
                    Capture::start_timed(
                        Source::Microphone,
                        track_ingress[0].callback(None, started),
                    )
                    .map_err(|error| format!("microphone: {error}"))?,
                ),
                echo.then(|| {
                    "Echo cancellation is unavailable on this platform. Use headphones.".to_owned()
                }),
            );
            let system = match Capture::start_timed(
                Source::SystemOutput,
                track_ingress[1].callback(Some(heard_system.clone()), started),
            ) {
                Ok(capture) => Some(capture),
                Err(error) => {
                    LOG.error(format!("system audio unavailable: {error}"));
                    let warning = format!(
                        "System audio is unavailable: {error}. Only the microphone is being recorded."
                    );
                    capture_warning = Some(
                        capture_warning
                            .map_or(warning.clone(), |previous| format!("{previous} {warning}")),
                    );
                    None
                }
            };
            let has_system = system.is_some();
            for source in [
                pennel_core::RecordingSource::Microphone,
                pennel_core::RecordingSource::SystemAudio,
            ] {
                engine
                    .record_meeting_capture_event(
                        session.id(),
                        pennel_core::MeetingCaptureEvent {
                            source,
                            kind: pennel_core::CaptureEventKind::Started,
                            ..Default::default()
                        },
                    )
                    .map_err(|error| error.to_string())?;
            }
            if !has_system {
                engine
                    .record_meeting_capture_event(
                        session.id(),
                        pennel_core::MeetingCaptureEvent {
                            source: pennel_core::RecordingSource::SystemAudio,
                            kind: pennel_core::CaptureEventKind::CaptureError,
                            error_code: Some("system_capture_start_failed".into()),
                            ..Default::default()
                        },
                    )
                    .map_err(|error| error.to_string())?;
            }
            LOG.info(format!(
                "meeting started, system audio {}",
                if has_system { "on" } else { "off" }
            ));
            Ok(Self {
                session,
                engine,
                title,
                started,
                quiescent_ms: None,
                capture_failures: Vec::new(),
                #[cfg(test)]
                quiescence_probe: None,
                mic: Some(mic),
                system,
                has_system,
                tracks,
                track_ingress,
                capture_warning,
                #[cfg(windows)]
                echo_active,
                dictating_since: Mutex::new(None),
                heard_system,
            })
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.quiescent_ms.map_or_else(
            || self.started.elapsed().as_secs(),
            |duration| duration / 1_000,
        )
    }

    pub fn started_at(&self) -> Instant {
        self.started
    }

    pub fn has_system_audio(&self) -> bool {
        if self.quiescent_ms.is_some() {
            return self.has_system;
        }
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native {
            return native.snapshot().system_audio_available;
        }
        self.has_system
    }

    pub fn heard_system_audio(&self) -> bool {
        self.heard_system.load(Ordering::Relaxed)
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub fn echo_cancellation_active(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.native.as_ref().is_some_and(|native| {
                let health = native.snapshot();
                health.microphone_active && health.echo_cancellation
            })
        }
        #[cfg(windows)]
        {
            self.echo_active
                && self
                    .mic
                    .as_ref()
                    .is_some_and(|capture| capture.error().is_none())
        }
    }

    pub fn terminal_error(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            self.native
                .as_ref()
                .and_then(|native| native.snapshot().terminal_error)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.track_ingress
                .iter()
                .any(|ingress| ingress.overloaded.load(Ordering::SeqCst))
                .then(|| "Audio processing could not keep up with recording.".into())
        }
    }

    /// All values are cached/atomic or ordinary Rust mutexes; this is safe under Shared's lock.
    pub fn audio_warning(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            self.native
                .as_ref()
                .and_then(|native| native.snapshot().warning())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut warnings = Vec::new();
            if let Some(error) = self.mic.as_ref().and_then(Microphone::error) {
                warnings.push(format!("Microfoonopname is onderbroken: {error}"));
            }
            if let Some(error) = self.system.as_ref().and_then(Capture::error) {
                warnings.push(format!("Systeemgeluid is onderbroken: {error}"));
            }
            for ingress in &self.track_ingress {
                if let Some(error) = &*ingress.error.lock().unwrap() {
                    warnings.push(format!("Audio processing failed: {error}"));
                }
            }
            if let Some(warning) = &self.capture_warning {
                warnings.push(warning.clone());
            }
            (!warnings.is_empty()).then(|| warnings.join(" "))
        }
    }

    /// While dictating, microphone input is suppressed in the meeting transcript. Archived audio
    /// remains the captured track; the recorded interval also filters echoes from the transcript.
    pub fn set_dictating(&self, dictating: bool) {
        self.session.set_dictating(dictating);
        let now_ms = self.started.elapsed().as_millis() as u64;
        let mut since = self.dictating_since.lock().unwrap();
        match (dictating, since.take()) {
            (true, _) => *since = Some(now_ms),
            (false, Some(start)) => self.session.mark_dictation(start, now_ms),
            (false, None) => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn testing(
        session: Arc<MeetingSession>,
        engine: Arc<PennelEngine>,
        probe: Arc<AtomicBool>,
    ) -> Self {
        engine
            .prepare_meeting_recording(
                session.id(),
                crate::recording_metadata::environment("test"),
                false,
            )
            .unwrap();
        Self {
            session,
            engine,
            title: "Originele titel".into(),
            started: Instant::now(),
            quiescent_ms: None,
            capture_failures: Vec::new(),
            quiescence_probe: Some(probe),
            #[cfg(target_os = "macos")]
            native: None,
            #[cfg(target_os = "macos")]
            preview: None,
            #[cfg(not(target_os = "macos"))]
            mic: None,
            #[cfg(not(target_os = "macos"))]
            system: None,
            has_system: false,
            #[cfg(not(target_os = "macos"))]
            tracks: std::array::from_fn(|_| {
                Arc::new(Mutex::new(TrackSink {
                    timing: TimedTrack::default(),
                    #[cfg(any(windows, target_os = "linux"))]
                    archive: None,
                    error: None,
                }))
            }),
            #[cfg(not(target_os = "macos"))]
            track_ingress: Vec::new(),
            #[cfg(not(target_os = "macos"))]
            capture_warning: None,
            #[cfg(windows)]
            echo_active: false,
            dictating_since: Mutex::new(None),
            heard_system: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Hardware shutdown never waits for notes, diagnostics persistence or transcript drainage.
    pub fn quiesce(&mut self) -> u64 {
        if let Some(duration) = self.quiescent_ms {
            return duration;
        }
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native {
            self.has_system = native.snapshot().system_audio_available;
            native.quiesce();
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Signal both sources before joining either source's callback thread.
            self.capture_failures
                .extend(quiesce_sources(self.mic.take(), self.system.take()));
        }
        let duration = self.started.elapsed().as_millis() as u64;
        self.quiescent_ms = Some(duration);
        self.session.stop_preview();
        #[cfg(target_os = "macos")]
        if let Some(preview) = self.preview.take() {
            preview.stop();
        }
        #[cfg(test)]
        if let Some(probe) = &self.quiescence_probe {
            probe.store(true, Ordering::SeqCst);
        }
        duration
    }

    /// Capture, archives and transcript are all finalized even if one stage reports an error.
    pub fn stop(mut self) -> (Result<String, String>, RecordingFinish) {
        let duration_ms = self.quiesce();
        self.set_dictating(false);
        let mut failures = std::mem::take(&mut self.capture_failures);
        #[cfg(target_os = "macos")]
        if let Some(native) = self.native.take()
            && let Err(error) = native.stop()
        {
            failures.push(error);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let end_ms = duration_ms;
            for ingress in self.track_ingress.drain(..) {
                if let Err(error) = ingress.finish() {
                    failures.push(error);
                }
            }
            for (speaker, track) in [Speaker::Me, Speaker::Others].into_iter().zip(&self.tracks) {
                let mut track = track.lock().unwrap();
                let archive_result = track.finish(&self.session, speaker, end_ms);
                let source = if speaker == Speaker::Me {
                    pennel_core::RecordingSource::Microphone
                } else {
                    pennel_core::RecordingSource::SystemAudio
                };
                if let Err(error) = self.engine.record_meeting_capture_event(
                    self.session.id(),
                    pennel_core::MeetingCaptureEvent {
                        source,
                        elapsed_ms: duration_ms,
                        kind: if archive_result.is_ok() {
                            pennel_core::CaptureEventKind::Stopped
                        } else {
                            pennel_core::CaptureEventKind::CaptureError
                        },
                        output_samples: Some(track.timing.output_samples),
                        inserted_silence_samples: Some(track.timing.inserted_silence_samples),
                        gap_count: Some(track.timing.gap_count),
                        error_code: archive_result
                            .as_ref()
                            .err()
                            .map(|_| "audio_archive_failed".into()),
                        ..Default::default()
                    },
                ) {
                    failures.push(error.to_string());
                }
                if let Err(error) = archive_result {
                    failures.push(error);
                }
            }
        }
        let completion = RecordingFinish {
            duration_ms,
            complete: failures.is_empty() && self.has_system,
        };
        if let Err(error) = self.session.stop() {
            failures.push(error.to_string());
        }
        if let Err(error) = self.engine.finish_meeting_recording(
            self.session.id(),
            duration_ms,
            completion.complete,
        ) {
            failures.push(error.to_string());
        }
        LOG.info(format!(
            "meeting stopped after {} s",
            self.started.elapsed().as_secs()
        ));
        let result = if failures.is_empty() {
            Ok(self.session.id())
        } else {
            Err(failures.join("; "))
        };
        (result, completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview_audio(source: Speaker, start_ms: u64) -> pennel_core::MeetingPreviewAudio {
        pennel_core::MeetingPreviewAudio {
            session_id: "preview-session".into(),
            source,
            epoch: 2,
            chunk_start_ms: 0,
            start_ms,
            samples: vec![0.1; 160],
        }
    }

    #[test]
    fn apple_preview_queue_replaces_each_track_independently_and_alternates_sources() {
        let queue = ApplePreviewQueue::default();
        queue.push(preview_audio(Speaker::Me, 0));
        queue.push(preview_audio(Speaker::Others, 10));
        queue.push(preview_audio(Speaker::Me, 20));
        let Some(ApplePreviewDispatch::Audio(first)) = queue.next() else {
            panic!()
        };
        assert_eq!((first.source, first.start_ms), (Speaker::Me, 20));
        queue.push(preview_audio(Speaker::Me, 30));
        let Some(ApplePreviewDispatch::Audio(second)) = queue.next() else {
            panic!()
        };
        assert_eq!((second.source, second.start_ms), (Speaker::Others, 10));
        let Some(ApplePreviewDispatch::Audio(third)) = queue.next() else {
            panic!()
        };
        assert_eq!((third.source, third.start_ms), (Speaker::Me, 30));
        queue.stop();
        assert!(queue.next().is_none());
    }

    #[test]
    fn apple_preview_reset_drops_queued_audio_and_stop_does_not_wait_for_analyzer_work() {
        let queue = Arc::new(ApplePreviewQueue::default());
        queue.push(preview_audio(Speaker::Me, 0));
        queue.reset();
        assert!(matches!(queue.next(), Some(ApplePreviewDispatch::Reset)));
        queue.push(preview_audio(Speaker::Others, 10));
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let worker_queue = queue.clone();
        let worker = std::thread::spawn(move || {
            assert!(matches!(
                worker_queue.next(),
                Some(ApplePreviewDispatch::Audio(_))
            ));
            entered.send(()).unwrap();
            waiting
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert!(worker_queue.next().is_none());
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        queue.push(preview_audio(Speaker::Me, 20));
        queue.stop();
        assert!(
            queue
                .pending
                .lock()
                .unwrap()
                .tracks
                .iter()
                .all(Option::is_none)
        );
        release.send(()).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn oversized_optional_apple_packets_are_refused_without_entering_the_queue() {
        let queue = ApplePreviewQueue::default();
        let mut oversized = preview_audio(Speaker::Me, 0);
        oversized.samples = vec![0.1; 6 * 16_000 + 1];
        queue.push(oversized);
        assert!(
            queue
                .pending
                .lock()
                .unwrap()
                .tracks
                .iter()
                .all(Option::is_none)
        );
        queue.stop();
    }

    #[test]
    fn both_capture_sources_are_signalled_before_joining_and_one_failure_does_not_skip_the_other() {
        use std::sync::atomic::AtomicU8;
        struct Source {
            requested: Arc<AtomicU8>,
            finished: Arc<AtomicU8>,
            bit: u8,
        }
        impl CaptureStop for Source {
            fn request_shutdown(&self) {
                self.requested.fetch_or(self.bit, Ordering::SeqCst);
            }
            fn finish_shutdown(self) -> Result<(), String> {
                assert_eq!(self.requested.load(Ordering::SeqCst), 3);
                self.finished.fetch_or(self.bit, Ordering::SeqCst);
                if self.bit == 1 {
                    Err("microphone shutdown failed".into())
                } else {
                    Ok(())
                }
            }
        }
        let requested = Arc::new(AtomicU8::new(0));
        let finished = Arc::new(AtomicU8::new(0));
        let source = |bit| Source {
            requested: requested.clone(),
            finished: finished.clone(),
            bit,
        };
        assert_eq!(
            quiesce_sources(Some(source(1)), Some(source(2))),
            vec!["microphone shutdown failed"]
        );
        assert_eq!(finished.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn audio_callback_never_waits_on_a_blocked_archive_and_ingress_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let engine =
            PennelEngine::new_preview(root.path().join("preview").to_string_lossy().into_owned())
                .unwrap();
        let session = engine
            .clone()
            .start_meeting(
                "Synthetic capture".into(),
                pennel_core::ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        let track = Arc::new(Mutex::new(TrackSink {
            timing: TimedTrack::default(),
            #[cfg(any(windows, target_os = "linux"))]
            archive: None,
            error: None,
        }));
        let blocked_archive = track.lock().unwrap();
        let ingress = TrackIngress::start(session.clone(), Speaker::Me, track.clone()).unwrap();
        let mut callback = ingress.callback(None, Instant::now());
        let (finished, observed) = std::sync::mpsc::channel();
        let capture = std::thread::spawn(move || {
            for _ in 0..130 {
                callback(&[0.1; 1_600], Instant::now());
            }
            finished.send(()).unwrap();
        });
        let returned_before_archive = observed
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok();
        drop(blocked_archive);
        capture.join().unwrap();
        assert!(
            returned_before_archive,
            "audio callbacks must not acquire the archival lock"
        );
        assert!(
            ingress.overloaded.load(Ordering::SeqCst),
            "the queue must remain bounded"
        );
        assert!(ingress.finish().is_err());
        track
            .lock()
            .unwrap()
            .finish(&session, Speaker::Me, 1_000)
            .unwrap();
        session.stop().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_status_exposes_actual_echo_and_degraded_system_audio() {
        let status = MacCaptureHealth::from_status(serde_json::json!({
            "microphone_active": true,
            "system_audio_available": false,
            "echo_cancellation": false,
            "system_audio_error": "Alleen de microfoon wordt opgenomen.",
            "capture_warning": "Echo-onderdrukking is niet actief.",
            "terminal_error": null
        }));
        assert!(status.microphone_active);
        assert!(!status.system_audio_available);
        assert!(!status.echo_cancellation);
        assert_eq!(status.terminal_error, None);
        assert_eq!(
            status.warning().as_deref(),
            Some("Only the microphone is being recorded. Echo cancellation is not active.")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_archive_error_remains_actionable_in_cached_health() {
        let failure = "Audio bewaren mislukt.";
        let status = MacCaptureHealth::from_status(serde_json::json!({
            "microphone_active": true,
            "system_audio_available": true,
            "archive_error": failure,
            "terminal_error": failure
        }));
        assert_eq!(status.terminal_error.as_deref(), Some(failure));
        assert_eq!(status.warning().as_deref(), Some("Saving audio failed."));
        assert!(status.microphone_active);
        assert!(status.system_audio_available);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn malformed_native_health_is_visible_without_claiming_a_terminal_capture_failure() {
        let status = MacCaptureHealth::from_status(serde_json::Value::Null);
        assert!(status.warning().is_some());
        assert!(status.terminal_error.is_none());
    }

    #[derive(Default)]
    struct Collected {
        audio: Vec<f32>,
        starts: Vec<u64>,
        max_batch: usize,
    }

    impl Collected {
        fn block(&mut self, block: TrackBlock<'_>) {
            match block {
                TrackBlock::Gap { samples } => self
                    .audio
                    .extend(std::iter::repeat_n(0.0, samples as usize)),
                TrackBlock::Audio { start_ms, samples } => {
                    self.starts.push(start_ms);
                    self.max_batch = self.max_batch.max(samples.len());
                    self.audio.extend_from_slice(samples);
                }
            }
        }
    }

    #[test]
    fn delayed_track_has_leading_silence_and_shared_offsets() {
        let mut track = TimedTrack::default();
        let mut collected = Collected::default();
        track
            .push_at(250, &vec![0.5; 16_000], |block| collected.block(block))
            .unwrap();
        assert_eq!(collected.starts, [250]);
        assert_eq!(collected.audio.len(), 20_000);
        assert!(collected.audio[..4_000].iter().all(|sample| *sample == 0.0));
        assert!(collected.audio[4_000..].iter().all(|sample| *sample == 0.5));
    }

    #[test]
    fn device_gap_flushes_old_batch_then_preserves_archive_timeline() {
        let mut track = TimedTrack::default();
        let mut collected = Collected::default();
        track
            .push_at(0, &vec![0.5; 8_000], |block| collected.block(block))
            .unwrap();
        track
            .push_at(2_000, &vec![-0.5; 8_000], |block| collected.block(block))
            .unwrap();
        track
            .finish_at(3_000, |block| collected.block(block))
            .unwrap();
        assert_eq!(collected.audio.len(), 48_000);
        assert!(
            collected.audio[8_000..32_000]
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(
            collected.audio[32_000..40_000]
                .iter()
                .all(|sample| *sample == -0.5)
        );
        assert!(collected.max_batch <= BATCH);
        assert_eq!(collected.starts[..2], [0, 2_000]);
        assert_eq!(track.output_samples, 16_000);
        assert_eq!(track.inserted_silence_samples, 32_000);
        assert_eq!(track.gap_count, 2);
    }

    #[test]
    fn repeated_or_overlapping_packets_are_trimmed_once() {
        let mut track = TimedTrack::default();
        let mut collected = Collected::default();
        track
            .push_at(0, &vec![1.0; 8_000], |block| collected.block(block))
            .unwrap();
        track
            .push_at(250, &vec![2.0; 8_000], |block| collected.block(block))
            .unwrap();
        track
            .push_at(0, &[9.0; 16], |block| collected.block(block))
            .unwrap();
        track.flush(|block| collected.block(block));
        assert_eq!(collected.audio.len(), 12_000);
        assert!(collected.audio[..8_000].iter().all(|sample| *sample == 1.0));
        assert!(collected.audio[8_000..].iter().all(|sample| *sample == 2.0));
    }

    #[test]
    fn huge_packet_is_emitted_in_bounded_batches() {
        let mut track = TimedTrack::default();
        let mut collected = Collected::default();
        track
            .push_at(0, &vec![0.3; 100_000], |block| collected.block(block))
            .unwrap();
        assert!(track.pending.len() < BATCH);
        track.flush(|block| collected.block(block));
        assert_eq!(collected.audio.len(), 100_000);
        assert!(collected.max_batch <= BATCH);
        assert!(track.push_at(u64::MAX, &[0.0], |_| {}).is_err());
    }
}
