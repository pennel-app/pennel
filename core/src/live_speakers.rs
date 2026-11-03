//! Optional rolling acoustic work. PCM and embeddings stay private to this bounded worker.
// Model-free builds retain the unavailable API; native/default and model-free tests lint the worker.
#![cfg_attr(not(any(feature = "speakers", test)), allow(dead_code))]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use pennel_speakers::{Diarization, Speaker as AcousticSpeaker, Turn};

use crate::PennelError;
use crate::meetings::{Speaker, VOICES};

const RATE: u64 = 16_000;
const WINDOW: usize = 30 * RATE as usize;
const CADENCE: u64 = 10 * RATE;
const MAX_DIMENSION: usize = 4_096;
const MAX_TURNS: usize = 4_096;
const MAX_SPEAKERS: usize = 64;
const MATCH_MARGIN: f32 = 0.10;
const MIN_CLEAN_MS: u64 = 1_000;

static ACOUSTIC_GATE: Mutex<()> = Mutex::new(());
static LIVE_WORKERS: Mutex<LiveWorkers> = Mutex::new(LiveWorkers {
    active: false,
    pending: None,
});
static OFFLINE_DEMAND: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static WORKER_TEST: Mutex<()> = Mutex::new(());
#[cfg(test)]
pub(crate) fn test_worker_guard() -> MutexGuard<'static, ()> {
    WORKER_TEST.lock().unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveSpeakerState {
    Off,
    Warming,
    Running,
    Busy,
    Paused,
    Unavailable,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LiveSpeakerStatus {
    pub enabled: bool,
    pub available: bool,
    pub state: LiveSpeakerState,
    pub reason_code: Option<String>,
    pub revision: u64,
    pub model: Option<String>,
    pub destination: Option<String>,
}

impl LiveSpeakerStatus {
    pub(crate) fn availability(
        available: bool,
        reason: Option<&str>,
        model: Option<String>,
    ) -> Self {
        Self {
            enabled: false,
            available,
            state: if available {
                LiveSpeakerState::Off
            } else {
                LiveSpeakerState::Unavailable
            },
            reason_code: reason.map(str::to_owned),
            revision: 0,
            model,
            destination: available.then(|| "local".into()),
        }
    }
}

/// No embeddings, audio or person-name guesses cross this callback.
#[derive(Debug, Clone)]
pub struct LiveSpeakerUpdate {
    pub session_id: String,
    pub generation: u64,
    pub revision: u64,
    pub source: Speaker,
    pub start_ms: u64,
    pub end_ms: u64,
    pub turns: Vec<Turn>,
    /// Acoustic speaker index -> stable session voice number.
    pub voice_map: Vec<(u32, u32)>,
}

pub trait LiveSpeakerObserver: Send + Sync {
    fn changed(&self, update: LiveSpeakerUpdate);
}

pub(crate) trait LiveSpeakerInference {
    fn infer(&mut self, samples: &[f32]) -> Result<Diarization, PennelError>;
}

pub(crate) type LiveSpeakerFactory =
    Box<dyn FnOnce() -> Result<Box<dyn LiveSpeakerInference>, PennelError> + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LiveSpeakerMetrics {
    pub ring_samples: usize,
    pub pending_samples: usize,
    pub active_samples: usize,
    pub physical_active: bool,
    pub native_calls: u64,
    pub dropped_packets: u64,
    pub clusters: usize,
}

pub(crate) fn offline_acoustic_permit() -> MutexGuard<'static, ()> {
    OFFLINE_DEMAND.fetch_add(1, Ordering::AcqRel);
    let guard = ACOUSTIC_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    OFFLINE_DEMAND.fetch_sub(1, Ordering::AcqRel);
    guard
}

fn try_live_acoustic_permit() -> Option<MutexGuard<'static, ()>> {
    if OFFLINE_DEMAND.load(Ordering::Acquire) > 0 {
        return None;
    }
    let guard = match ACOUSTIC_GATE.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    (OFFLINE_DEMAND.load(Ordering::Acquire) == 0).then_some(guard)
}

fn index(source: Speaker) -> usize {
    if source == Speaker::Me { 0 } else { 1 }
}

#[derive(Default)]
struct AudioRing {
    samples: Vec<f32>,
    next: usize,
    len: usize,
    end: u64,
}
impl AudioRing {
    fn clear(&mut self) {
        self.next = 0;
        self.len = 0;
        self.end = 0;
    }
    fn release(&mut self) {
        self.clear();
        self.samples = Vec::new();
    }
    fn feed(&mut self, start_ms: u64, samples: &[f32]) {
        if self.samples.is_empty() {
            self.samples = vec![0.0; WINDOW];
        }
        let Some(mut start) = start_ms.checked_mul(16) else {
            return;
        };
        let mut samples = samples;
        if self.len > 0 {
            if start.abs_diff(self.end) < 16 {
                start = self.end;
            } else if start < self.end {
                let skipped = (self.end - start).min(samples.len() as u64) as usize;
                samples = &samples[skipped..];
                start += skipped as u64;
                if samples.is_empty() {
                    return;
                }
            } else {
                self.clear();
            }
        }
        if samples.len() > WINDOW {
            let skipped = samples.len() - WINDOW;
            samples = &samples[skipped..];
            start += skipped as u64;
            self.clear();
        }
        for sample in samples {
            self.samples[self.next] = if sample.is_finite() { *sample } else { 0.0 };
            self.next = (self.next + 1) % WINDOW;
        }
        self.len = (self.len + samples.len()).min(WINDOW);
        self.end = start.saturating_add(samples.len() as u64);
    }
    fn snapshot(&self, source: Speaker) -> Option<AudioWindow> {
        if self.len < RATE as usize {
            return None;
        }
        let oldest = (self.next + WINDOW - self.len) % WINDOW;
        let mut samples = Vec::with_capacity(self.len);
        let first = self.len.min(WINDOW - oldest);
        samples.extend_from_slice(&self.samples[oldest..oldest + first]);
        samples.extend_from_slice(&self.samples[..self.len - first]);
        Some(AudioWindow {
            source,
            start_ms: (self.end - self.len as u64) / 16,
            end_ms: self.end / 16,
            samples,
            authoritative: false,
        })
    }
}

struct AudioWindow {
    source: Speaker,
    start_ms: u64,
    end_ms: u64,
    samples: Vec<f32>,
    authoritative: bool,
}
struct Batch {
    epoch: u64,
    tracks: Vec<AudioWindow>,
}
impl Batch {
    fn samples(&self) -> usize {
        self.tracks.iter().map(|track| track.samples.len()).sum()
    }
}

#[derive(Default)]
struct Queue {
    rings: [AudioRing; 2],
    pending: Option<Batch>,
    scheduled: u64,
    epoch: u64,
    admitted_clock: Option<u64>,
}
impl Queue {
    fn clock(&self) -> u64 {
        self.rings
            .iter()
            .map(|ring| ring.end)
            .chain(self.pending.iter().flat_map(|batch| {
                batch
                    .tracks
                    .iter()
                    .map(|track| track.end_ms.saturating_mul(16))
            }))
            .max()
            .unwrap_or(0)
    }
    fn ready(&self) -> bool {
        self.pending.is_some()
            && self
                .admitted_clock
                .is_none_or(|last| self.clock().saturating_sub(last) >= CADENCE)
    }
    fn mark_admitted(&mut self, batch: &Batch) {
        self.admitted_clock = Some(
            self.clock().max(
                batch
                    .tracks
                    .iter()
                    .map(|track| track.end_ms.saturating_mul(16))
                    .max()
                    .unwrap_or(0),
            ),
        );
    }
    fn clear(&mut self) {
        for ring in &mut self.rings {
            ring.clear();
        }
        self.pending = None;
    }
    fn release(&mut self) {
        for ring in &mut self.rings {
            ring.release();
        }
        self.pending = None;
    }
    fn feed(&mut self, source: Speaker, start_ms: u64, samples: &[f32], epoch: u64) -> bool {
        if self.epoch != epoch {
            self.clear();
            self.epoch = epoch;
        }
        self.rings[index(source)].feed(start_ms, samples);
        let end = self.rings.iter().map(|ring| ring.end).max().unwrap_or(0);
        if end.saturating_sub(self.scheduled) < CADENCE {
            return false;
        }
        // Exact authoritative boundaries stay pinned while their STT work has priority.
        let pinned = self
            .pending
            .take()
            .map(|batch| {
                batch
                    .tracks
                    .into_iter()
                    .filter(|track| track.authoritative)
                    .collect()
            })
            .unwrap_or_default();
        let tracks = self.snapshot_with_pins(pinned);
        if tracks.is_empty() {
            return false;
        }
        self.pending = Some(Batch { epoch, tracks });
        self.scheduled = end;
        true
    }
    fn snapshot_with_pins(&self, mut pinned: Vec<AudioWindow>) -> Vec<AudioWindow> {
        [Speaker::Me, Speaker::Others]
            .into_iter()
            .filter_map(|source| {
                if let Some(index) = pinned.iter().position(|track| track.source == source) {
                    Some(pinned.swap_remove(index))
                } else {
                    self.rings[index(source)].snapshot(source)
                }
            })
            .collect()
    }
    fn submit_authoritative(
        &mut self,
        source: Speaker,
        start_ms: u64,
        samples: &[f32],
        epoch: u64,
    ) -> bool {
        if samples.is_empty() || samples.len() > WINDOW {
            return false;
        }
        if self.epoch != epoch {
            self.clear();
            self.epoch = epoch;
        }
        let end_ms = start_ms.saturating_add(samples.len() as u64 / 16);
        let batch = self.pending.get_or_insert_with(|| Batch {
            epoch,
            tracks: Vec::new(),
        });
        if let Some(index) = batch.tracks.iter().position(|track| track.source == source) {
            if batch.tracks[index].authoritative && batch.tracks[index].end_ms >= end_ms {
                return false;
            }
            // Release the old owned PCM before copying the replacement.
            batch.tracks.swap_remove(index);
        }
        batch.tracks.push(AudioWindow {
            source,
            start_ms,
            end_ms,
            samples: samples
                .iter()
                .map(|sample| if sample.is_finite() { *sample } else { 0.0 })
                .collect(),
            authoritative: true,
        });
        self.scheduled = self.scheduled.max(end_ms.saturating_mul(16));
        true
    }
    fn metrics(
        &self,
        active_samples: usize,
        physical_active: bool,
        native_calls: u64,
        dropped_packets: u64,
        clusters: usize,
    ) -> LiveSpeakerMetrics {
        LiveSpeakerMetrics {
            ring_samples: self.rings.iter().map(|ring| ring.samples.len()).sum(),
            pending_samples: self.pending.as_ref().map_or(0, Batch::samples),
            active_samples,
            physical_active,
            native_calls,
            dropped_packets,
            clusters,
        }
    }
    fn take_latest(&mut self) -> Option<Batch> {
        let old = self.pending.take()?;
        let epoch = old.epoch;
        let pinned = old
            .tracks
            .into_iter()
            .filter(|track| track.authoritative)
            .collect();
        let tracks = self.snapshot_with_pins(pinned);
        self.scheduled = self.scheduled.max(
            self.rings
                .iter()
                .map(|ring| ring.end)
                .max()
                .unwrap_or(self.scheduled),
        );
        Some(Batch { epoch, tracks })
    }
}

#[derive(Clone)]
struct Cluster {
    voice: u32,
    centroid: Vec<f32>,
    weight: u64,
}
#[derive(Clone, Default)]
struct SessionClusters {
    groups: Vec<Cluster>,
    last_end: [u64; 2],
}

#[derive(Default)]
struct MemoryState {
    model: Option<String>,
    session_id: Option<String>,
    admitted_clock: Option<u64>,
    clusters: SessionClusters,
}

/// Session-owned private acoustic state. It is deliberately not serializable or exposed in JSON.
#[derive(Default)]
pub struct LiveSpeakerMemory {
    state: Mutex<MemoryState>,
}
impl LiveSpeakerMemory {
    pub(crate) fn bind(&self, model: &str, session_id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        if let Some(previous) = &state.model {
            return previous == model && state.session_id.as_deref() == Some(session_id);
        }
        state.model = Some(model.into());
        state.session_id = Some(session_id.into());
        true
    }
    fn clusters(&self) -> SessionClusters {
        self.state.lock().unwrap().clusters.clone()
    }
    fn commit(&self, clusters: SessionClusters) {
        self.state.lock().unwrap().clusters = clusters;
    }
    fn cadence(&self) -> Option<u64> {
        self.state.lock().unwrap().admitted_clock
    }
    fn mark_admitted(&self, clock: Option<u64>) {
        if let Some(clock) = clock {
            let mut state = self.state.lock().unwrap();
            state.admitted_clock = Some(state.admitted_clock.unwrap_or(0).max(clock));
        }
    }
}

fn clean_speech(diarization: &Diarization, speaker: u32, after: u64) -> u64 {
    let mut intervals: Vec<_> = diarization
        .turns
        .iter()
        .filter(|turn| turn.speaker == speaker && turn.end_ms > turn.start_ms)
        .map(|turn| (turn.start_ms.max(after), turn.end_ms))
        .filter(|(start, end)| end > start)
        .collect();
    intervals.sort_unstable();
    let mut covered = 0;
    let mut reached = 0;
    for (start, end) in intervals {
        let start = start.max(reached);
        if end > start {
            covered += end - start;
            reached = end;
        }
    }
    covered
}

impl SessionClusters {
    fn assign_at(
        &mut self,
        diarization: &Diarization,
        source: Speaker,
        start_ms: u64,
        end_ms: u64,
    ) -> Vec<(u32, u32)> {
        let overlap = |speaker: u32| {
            diarization
                .turns
                .iter()
                .filter(|turn| turn.speaker == speaker)
                .any(|own| {
                    diarization.turns.iter().any(|other| {
                        other.speaker != speaker
                            && own.end_ms.min(other.end_ms) > own.start_ms.max(other.start_ms)
                    })
                })
        };
        let valid = |speaker: &AcousticSpeaker| {
            !speaker.embedding.is_empty()
                && speaker.embedding.len() <= MAX_DIMENSION
                && speaker.embedding.iter().all(|value| value.is_finite())
                && speaker.speech_ms >= MIN_CLEAN_MS
                && clean_speech(diarization, speaker.speaker, 0) >= MIN_CLEAN_MS
                && !overlap(speaker.speaker)
                && self
                    .groups
                    .first()
                    .is_none_or(|group| group.centroid.len() == speaker.embedding.len())
        };
        let candidates: Vec<_> = diarization
            .speakers
            .iter()
            .filter(|speaker| valid(speaker))
            .collect();
        let mut mappings = Vec::new();
        let mut used = std::collections::BTreeSet::new();
        for speaker in &candidates {
            let embedding = pennel_speakers::normalized(&speaker.embedding);
            if embedding.is_empty() {
                continue;
            }
            // Two already-separated local voices that sound too alike cannot be safely joined or
            // made into near-duplicate session identities without more acoustic evidence.
            if candidates.iter().any(|other| {
                other.speaker != speaker.speaker
                    && pennel_speakers::cosine(&embedding, &other.embedding)
                        >= pennel_speakers::MERGE_THRESHOLD - MATCH_MARGIN
            }) {
                continue;
            }
            let mut matches: Vec<_> = self
                .groups
                .iter()
                .enumerate()
                .map(|(index, cluster)| {
                    (
                        index,
                        pennel_speakers::cosine(&cluster.centroid, &embedding),
                    )
                })
                .collect();
            matches.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let best = matches.first().copied();
            let second = matches.get(1).map_or(-1.0, |entry| entry.1);
            let fresh_after = self.last_end[index(source)].saturating_sub(start_ms);
            let weight = clean_speech(diarization, speaker.speaker, fresh_after);
            let group = if let Some((group, score)) = best.filter(|(_, score)| {
                *score >= pennel_speakers::MERGE_THRESHOLD && *score - second >= MATCH_MARGIN
            }) {
                if used.contains(&group) {
                    continue;
                }
                let _ = score;
                group
            } else {
                if best.is_some_and(|(_, score)| {
                    score >= pennel_speakers::MERGE_THRESHOLD - MATCH_MARGIN
                }) || self.groups.len() >= VOICES.clone().count()
                    || weight == 0
                {
                    continue;
                }
                self.groups.push(Cluster {
                    voice: self.groups.len() as u32 + 1,
                    centroid: embedding.clone(),
                    weight: 0,
                });
                self.groups.len() - 1
            };
            used.insert(group);
            let cluster = &mut self.groups[group];
            if weight > 0 {
                cluster.centroid = pennel_speakers::weighted_mean([
                    (cluster.centroid.as_slice(), cluster.weight as f32),
                    (embedding.as_slice(), weight as f32),
                ]);
                cluster.weight = cluster.weight.saturating_add(weight);
            }
            mappings.push((speaker.speaker, cluster.voice));
        }
        self.last_end[index(source)] = self.last_end[index(source)].max(end_ms);
        mappings
    }
    #[cfg(test)]
    fn assign(&mut self, result: &Diarization) -> Vec<(u32, u32)> {
        let start = self.last_end[0];
        self.assign_at(result, Speaker::Me, start, start + 5_000)
    }
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    status: Mutex<LiveSpeakerStatus>,
    stopped: AtomicBool,
    finished: AtomicBool,
    admitted: AtomicBool,
    paused: AtomicBool,
    deferred: AtomicBool,
    epoch: AtomicU64,
    revision: AtomicU64,
    active_samples: AtomicUsize,
    physical_active: AtomicBool,
    native_calls: AtomicU64,
    dropped_packets: AtomicU64,
    clusters: AtomicUsize,
    memory: Arc<LiveSpeakerMemory>,
}

struct PendingStart {
    shared: Arc<Shared>,
    session_id: String,
    generation: u64,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
    factory: LiveSpeakerFactory,
    observer: Arc<dyn LiveSpeakerObserver>,
}
struct LiveWorkers {
    active: bool,
    pending: Option<PendingStart>,
}

fn launch(start: PendingStart) {
    if start.shared.stopped.load(Ordering::Acquire) || !(start.current)() {
        start.shared.finished.store(true, Ordering::Release);
        if !start.shared.stopped.load(Ordering::Acquire) {
            {
                start.shared.status.lock().unwrap().available = false;
            }
            start
                .shared
                .state(LiveSpeakerState::Unavailable, Some("configuration_changed"));
        }
        release_live_slot();
        return;
    }
    start.shared.admitted.store(true, Ordering::Release);
    let shared = start.shared.clone();
    if std::thread::Builder::new()
        .name("pennel-live-speakers".into())
        .spawn(move || run(start))
        .is_err()
    {
        shared.admitted.store(false, Ordering::Release);
        shared.finished.store(true, Ordering::Release);
        shared.state(LiveSpeakerState::Failed, Some("failed"));
        release_live_slot();
    }
}

fn submit(start: PendingStart) {
    let mut workers = LIVE_WORKERS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if workers.active {
        start.shared.state(LiveSpeakerState::Busy, Some("busy"));
        let previous = workers.pending.replace(start);
        drop(workers);
        if let Some(previous) = previous {
            previous.shared.stopped.store(true, Ordering::Release);
            previous.shared.finished.store(true, Ordering::Release);
            {
                previous.shared.status.lock().unwrap().enabled = false;
            }
            previous
                .shared
                .state(LiveSpeakerState::Stopped, Some("stopped"));
        }
    } else {
        workers.active = true;
        drop(workers);
        launch(start);
    }
}

fn release_live_slot() {
    let next = {
        let mut workers = LIVE_WORKERS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(next) = workers.pending.take() {
            Some(next)
        } else {
            workers.active = false;
            None
        }
    };
    if let Some(next) = next {
        launch(next);
    }
}

struct LiveLifetime;
impl Drop for LiveLifetime {
    fn drop(&mut self) {
        release_live_slot();
    }
}
struct WorkerExit(Arc<Shared>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        if std::thread::panicking() && !self.0.stopped.load(Ordering::Acquire) {
            let mut status = self
                .0
                .status
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            status.state = LiveSpeakerState::Failed;
            status.reason_code = Some("failed".into());
            status.revision = self.0.revision.fetch_add(1, Ordering::AcqRel) + 1;
        }
        self.0.finished.store(true, Ordering::Release);
        self.0.admitted.store(false, Ordering::Release);
        self.0.active_samples.store(0, Ordering::Release);
        self.0.physical_active.store(false, Ordering::Release);
        self.0
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .release();
    }
}

impl Shared {
    fn control_state(&self) {
        if self.paused.load(Ordering::Acquire) {
            self.state(LiveSpeakerState::Paused, Some("paused"));
        } else if self.deferred.load(Ordering::Acquire) {
            self.state(LiveSpeakerState::Paused, Some("foreground_work"));
        } else {
            self.state(LiveSpeakerState::Warming, None);
        }
    }
    fn state(&self, state: LiveSpeakerState, reason: Option<&str>) {
        let mut status = self.status.lock().unwrap();
        if self.stopped.load(Ordering::Acquire) && state != LiveSpeakerState::Stopped {
            return;
        }
        if (self.paused.load(Ordering::Acquire) || self.deferred.load(Ordering::Acquire))
            && matches!(
                state,
                LiveSpeakerState::Warming | LiveSpeakerState::Running | LiveSpeakerState::Busy
            )
        {
            return;
        }
        if status.state != state || status.reason_code.as_deref() != reason {
            status.state = state;
            status.reason_code = reason.map(str::to_owned);
            status.revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        }
    }
    fn valid(&self, epoch: u64, current: &dyn Fn() -> bool) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && !self.paused.load(Ordering::Acquire)
            && self.epoch.load(Ordering::Acquire) == epoch
            && current()
    }
}

pub struct LiveSpeakerWorker {
    shared: Arc<Shared>,
}

impl LiveSpeakerWorker {
    #[cfg(test)]
    pub(crate) fn start(
        session_id: String,
        generation: u64,
        model: String,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
        factory: LiveSpeakerFactory,
        observer: Arc<dyn LiveSpeakerObserver>,
    ) -> Arc<Self> {
        Self::start_with_memory(
            session_id,
            generation,
            model,
            current,
            factory,
            observer,
            Arc::new(LiveSpeakerMemory::default()),
        )
    }
    pub(crate) fn start_with_memory(
        session_id: String,
        generation: u64,
        model: String,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
        factory: LiveSpeakerFactory,
        observer: Arc<dyn LiveSpeakerObserver>,
        memory: Arc<LiveSpeakerMemory>,
    ) -> Arc<Self> {
        let compatible = memory.bind(&model, &session_id);
        let cluster_count = memory.clusters().groups.len();
        let queue = Queue {
            admitted_clock: memory.cadence(),
            ..Queue::default()
        };
        let shared = Arc::new(Shared {
            queue: Mutex::new(queue),
            ready: Condvar::new(),
            status: Mutex::new(LiveSpeakerStatus {
                enabled: true,
                available: compatible,
                state: if compatible {
                    LiveSpeakerState::Warming
                } else {
                    LiveSpeakerState::Unavailable
                },
                reason_code: (!compatible).then(|| "configuration_changed".into()),
                revision: 0,
                model: Some(model),
                destination: Some("local".into()),
            }),
            stopped: AtomicBool::new(false),
            finished: AtomicBool::new(!compatible),
            admitted: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            deferred: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            active_samples: AtomicUsize::new(0),
            physical_active: AtomicBool::new(false),
            native_calls: AtomicU64::new(0),
            dropped_packets: AtomicU64::new(0),
            clusters: AtomicUsize::new(cluster_count),
            memory,
        });
        let worker = Arc::new(Self {
            shared: shared.clone(),
        });
        if compatible {
            submit(PendingStart {
                shared,
                session_id,
                generation,
                current,
                factory,
                observer,
            });
        }
        worker
    }
    pub fn memory(&self) -> Arc<LiveSpeakerMemory> {
        self.shared.memory.clone()
    }
    #[cfg(test)]
    pub(crate) fn is_admitted(&self) -> bool {
        self.shared.admitted.load(Ordering::Acquire)
    }
    pub fn feed(&self, source: Speaker, start_ms: u64, samples: &[f32]) {
        if self.shared.stopped.load(Ordering::Acquire)
            || self.shared.finished.load(Ordering::Acquire)
            || !self.shared.admitted.load(Ordering::Acquire)
            || self.shared.paused.load(Ordering::Acquire)
            || samples.is_empty()
        {
            return;
        }
        let Ok(mut queue) = self.shared.queue.try_lock() else {
            self.shared.dropped_packets.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if self.shared.stopped.load(Ordering::Acquire)
            || self.shared.finished.load(Ordering::Acquire)
            || self.shared.paused.load(Ordering::Acquire)
        {
            return;
        }
        if queue.feed(
            source,
            start_ms,
            samples,
            self.shared.epoch.load(Ordering::Acquire),
        ) {
            self.shared.ready.notify_one();
        }
        if self.shared.stopped.load(Ordering::Acquire) {
            queue.release();
        } else if self.shared.paused.load(Ordering::Acquire) {
            queue.clear();
        }
    }
    /// Called with the actual bounded chunk, after authoritative work has deferred admission.
    pub fn submit_authoritative(&self, source: Speaker, start_ms: u64, samples: &[f32]) -> bool {
        if self.shared.stopped.load(Ordering::Acquire)
            || self.shared.finished.load(Ordering::Acquire)
            || !self.shared.admitted.load(Ordering::Acquire)
            || self.shared.paused.load(Ordering::Acquire)
        {
            return false;
        }
        let Ok(mut queue) = self.shared.queue.try_lock() else {
            self.shared.dropped_packets.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if self.shared.stopped.load(Ordering::Acquire) || self.shared.paused.load(Ordering::Acquire)
        {
            return false;
        }
        let queued = queue.submit_authoritative(
            source,
            start_ms,
            samples,
            self.shared.epoch.load(Ordering::Acquire),
        );
        if self.shared.stopped.load(Ordering::Acquire) {
            queue.release();
            return false;
        }
        if self.shared.paused.load(Ordering::Acquire) {
            queue.clear();
            return false;
        }
        if queued {
            self.shared.ready.notify_one();
        }
        queued
    }
    pub fn set_paused(&self, paused: bool) {
        if self.shared.stopped.load(Ordering::Acquire)
            || self.shared.finished.load(Ordering::Acquire)
            || self.shared.paused.swap(paused, Ordering::AcqRel) == paused
        {
            return;
        }
        self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut queue) = self.shared.queue.try_lock() {
            queue.release();
        }
        self.shared.control_state();
        self.shared.ready.notify_all();
    }
    /// Foreground speech keeps the context, but prevents admission of another optional batch.
    pub fn set_deferred(&self, deferred: bool) {
        if self.shared.stopped.load(Ordering::Acquire)
            || self.shared.finished.load(Ordering::Acquire)
            || self.shared.deferred.swap(deferred, Ordering::AcqRel) == deferred
        {
            return;
        }
        self.shared.control_state();
        self.shared.ready.notify_all();
    }
    pub fn stop(&self) {
        if self.shared.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut queue) = self.shared.queue.try_lock() {
            queue.release();
        }
        {
            let mut status = self.shared.status.lock().unwrap();
            status.enabled = false;
        }
        self.shared
            .state(LiveSpeakerState::Stopped, Some("stopped"));
        self.shared.ready.notify_all();
    }
    pub fn status(&self) -> LiveSpeakerStatus {
        self.shared.status.lock().unwrap().clone()
    }
    pub fn metrics(&self) -> LiveSpeakerMetrics {
        let active = self.shared.active_samples.load(Ordering::Acquire);
        let physical = self.shared.physical_active.load(Ordering::Acquire);
        let calls = self.shared.native_calls.load(Ordering::Acquire);
        let dropped = self.shared.dropped_packets.load(Ordering::Acquire);
        let clusters = self.shared.clusters.load(Ordering::Acquire);
        self.shared
            .queue
            .lock()
            .unwrap()
            .metrics(active, physical, calls, dropped, clusters)
    }
}

impl Drop for LiveSpeakerWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn valid_result(result: &Diarization, duration: u64) -> bool {
    result.speakers.len() <= MAX_SPEAKERS
        && result.turns.len() <= MAX_TURNS
        && result
            .speakers
            .iter()
            .map(|speaker| speaker.speaker)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == result.speakers.len()
        && result.speakers.iter().all(|speaker| {
            speaker.embedding.len() <= MAX_DIMENSION
                && speaker.embedding.iter().all(|value| value.is_finite())
        })
        && result.turns.iter().all(|turn| {
            turn.end_ms > turn.start_ms
                && turn.end_ms <= duration
                && result
                    .speakers
                    .iter()
                    .any(|speaker| speaker.speaker == turn.speaker)
        })
}

fn run(start: PendingStart) {
    let _lifetime = LiveLifetime;
    let _exit = WorkerExit(start.shared.clone());
    let PendingStart {
        shared,
        session_id,
        generation,
        current,
        factory,
        observer,
    } = start;
    let mut factory = Some(factory);
    let mut inference: Option<Box<dyn LiveSpeakerInference>> = None;
    loop {
        if shared.stopped.load(Ordering::Acquire) {
            break;
        }
        if !current() {
            {
                shared.status.lock().unwrap().available = false;
            }
            shared.state(LiveSpeakerState::Unavailable, Some("configuration_changed"));
            break;
        }
        let mut queue = shared.queue.lock().unwrap();
        if shared.paused.load(Ordering::Acquire) {
            queue.clear();
        }
        if !queue.ready() || shared.deferred.load(Ordering::Acquire) {
            let (next, _) = shared
                .ready
                .wait_timeout(queue, Duration::from_millis(100))
                .unwrap();
            drop(next);
            continue;
        }
        let mut batch = queue.take_latest().expect("checked pending batch");
        drop(queue);
        shared
            .active_samples
            .store(batch.samples(), Ordering::Release);
        let permit = loop {
            if !shared.valid(batch.epoch, &*current) {
                break None;
            }
            if shared.deferred.load(Ordering::Acquire) {
                shared.state(LiveSpeakerState::Paused, Some("foreground_work"));
            } else if let Some(permit) = try_live_acoustic_permit() {
                break Some(permit);
            }
            if !shared.deferred.load(Ordering::Acquire) {
                shared.state(LiveSpeakerState::Busy, Some("busy"));
            }
            if let Ok(mut queue) = shared.queue.try_lock()
                && let Some(newer) = queue.take_latest()
            {
                batch = newer;
                shared
                    .active_samples
                    .store(batch.samples(), Ordering::Release);
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let Some(permit) = permit else {
            shared.active_samples.store(0, Ordering::Release);
            continue;
        };
        if !shared.valid(batch.epoch, &*current) {
            drop(permit);
            shared.active_samples.store(0, Ordering::Release);
            continue;
        }
        if shared.deferred.load(Ordering::Acquire) {
            drop(permit);
            drop(batch);
            shared.active_samples.store(0, Ordering::Release);
            continue;
        }
        {
            let mut queue = shared.queue.lock().unwrap();
            queue.mark_admitted(&batch);
            shared.memory.mark_admitted(queue.admitted_clock);
        }
        shared.physical_active.store(true, Ordering::Release);
        if inference.is_none() {
            match factory.take().expect("one native factory")() {
                Ok(loaded) => inference = Some(loaded),
                Err(_) => {
                    shared.physical_active.store(false, Ordering::Release);
                    drop(permit);
                    shared.state(LiveSpeakerState::Failed, Some("failed"));
                    break;
                }
            }
        }
        shared.state(LiveSpeakerState::Running, None);
        let mut candidate = shared.memory.clusters();
        let mut updates = Vec::new();
        let mut failed = false;
        for track in &batch.tracks {
            if !shared.valid(batch.epoch, &*current) {
                break;
            }
            if shared.deferred.load(Ordering::Acquire) || OFFLINE_DEMAND.load(Ordering::Acquire) > 0
            {
                if shared.deferred.load(Ordering::Acquire) {
                    shared.state(LiveSpeakerState::Paused, Some("foreground_work"));
                } else {
                    shared.state(LiveSpeakerState::Busy, Some("busy"));
                }
                break;
            }
            if crate::diarize::mostly_silent(&track.samples) {
                continue;
            }
            shared.native_calls.fetch_add(1, Ordering::AcqRel);
            let result = inference
                .as_mut()
                .expect("loaded inference")
                .infer(&track.samples);
            if !shared.valid(batch.epoch, &*current) {
                break;
            }
            match result {
                Ok(result) if valid_result(&result, track.end_ms - track.start_ms) => {
                    let voice_map =
                        candidate.assign_at(&result, track.source, track.start_ms, track.end_ms);
                    let turns = result
                        .turns
                        .into_iter()
                        .map(|mut turn| {
                            turn.start_ms += track.start_ms;
                            turn.end_ms += track.start_ms;
                            turn
                        })
                        .collect();
                    updates.push(LiveSpeakerUpdate {
                        session_id: session_id.clone(),
                        generation,
                        revision: 0,
                        source: track.source,
                        start_ms: track.start_ms,
                        end_ms: track.end_ms,
                        turns,
                        voice_map,
                    });
                }
                _ => {
                    failed = true;
                    break;
                }
            }
        }
        shared.physical_active.store(false, Ordering::Release);
        drop(permit);
        let epoch = batch.epoch;
        drop(batch);
        shared.active_samples.store(0, Ordering::Release);
        if !shared.valid(epoch, &*current) {
            continue;
        }
        if failed {
            shared.state(LiveSpeakerState::Failed, Some("failed"));
            break;
        }
        let cluster_count = candidate.groups.len();
        shared.memory.commit(candidate);
        shared.clusters.store(cluster_count, Ordering::Release);
        for mut update in updates {
            if !shared.valid(epoch, &*current) {
                break;
            }
            update.revision = shared.revision.fetch_add(1, Ordering::AcqRel) + 1;
            {
                shared.status.lock().unwrap().revision = update.revision;
            }
            observer.changed(update);
        }
    }
    // Local inference drops before WorkerExit frees PCM, then LiveLifetime admits its replacement.
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    fn voice(index: u32, embedding: Vec<f32>, speech_ms: u64) -> AcousticSpeaker {
        AcousticSpeaker {
            speaker: index,
            embedding,
            speech_ms,
        }
    }
    fn result(embedding: Vec<f32>) -> Diarization {
        Diarization {
            turns: vec![Turn {
                start_ms: 0,
                end_ms: 5_000,
                speaker: 0,
            }],
            speakers: vec![voice(0, embedding, 5_000)],
        }
    }
    #[track_caller]
    fn until(check: impl FnMut() -> bool) {
        until_phase("owned worker", check);
    }
    #[track_caller]
    fn until_phase(phase: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(
                Instant::now() < deadline,
                "The owned worker did not reach checkpoint: {phase}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[track_caller]
    fn feed_accepted(worker: &LiveSpeakerWorker, start_ms: u64, samples: &[f32], phase: &str) {
        // Capture may discard a contended packet; this fixture must acknowledge its one audio range.
        until_phase(phase, || {
            worker.feed(Speaker::Me, start_ms, samples);
            let metrics = worker.metrics();
            metrics.ring_samples > 0 || metrics.active_samples > 0 || metrics.native_calls > 0
        });
    }

    #[derive(Default)]
    struct PhysicalActivity {
        active: AtomicUsize,
        maximum: AtomicUsize,
    }
    struct PhysicalCall(Arc<PhysicalActivity>);
    impl PhysicalActivity {
        fn enter(activity: Arc<Self>) -> PhysicalCall {
            let active = activity.active.fetch_add(1, Ordering::SeqCst) + 1;
            activity.maximum.fetch_max(active, Ordering::SeqCst);
            PhysicalCall(activity)
        }
    }
    impl Drop for PhysicalCall {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct TrackedInference<T> {
        inner: T,
        activity: Arc<PhysicalActivity>,
    }
    impl<T: LiveSpeakerInference> LiveSpeakerInference for TrackedInference<T> {
        fn infer(&mut self, samples: &[f32]) -> Result<Diarization, PennelError> {
            let _physical = PhysicalActivity::enter(self.activity.clone());
            self.inner.infer(samples)
        }
    }

    #[derive(Default)]
    struct PriorityFixtureCleanup {
        releases: Vec<Arc<AtomicBool>>,
        workers: Vec<Arc<LiveSpeakerWorker>>,
        foreground: Option<std::thread::JoinHandle<()>>,
    }
    impl PriorityFixtureCleanup {
        fn finish(&mut self) -> bool {
            for release in &self.releases {
                release.store(true, Ordering::SeqCst);
            }
            for worker in &self.workers {
                worker.stop();
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self
                .workers
                .iter()
                .all(|worker| worker.shared.finished.load(Ordering::Acquire))
                || !self.foreground.as_ref().is_none_or(|job| job.is_finished())
                || !LIVE_WORKERS.try_lock().is_ok_and(|workers| !workers.active)
            {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.foreground.take().is_none_or(|job| job.join().is_ok())
        }
    }
    impl Drop for PriorityFixtureCleanup {
        fn drop(&mut self) {
            let _ = self.finish();
        }
    }
    #[derive(Default)]
    struct Observer(Mutex<Vec<LiveSpeakerUpdate>>);
    impl LiveSpeakerObserver for Observer {
        fn changed(&self, update: LiveSpeakerUpdate) {
            self.0.lock().unwrap().push(update);
        }
    }
    struct Blocking {
        entered: Arc<AtomicUsize>,
        released: Arc<AtomicBool>,
    }
    struct Immediate(Arc<AtomicUsize>);
    impl LiveSpeakerInference for Immediate {
        fn infer(&mut self, _: &[f32]) -> Result<Diarization, PennelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(result(vec![1.0, 0.0]))
        }
    }
    impl LiveSpeakerInference for Blocking {
        fn infer(&mut self, _: &[f32]) -> Result<Diarization, PennelError> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            until(|| self.released.load(Ordering::SeqCst));
            Ok(result(vec![1.0, 0.0]))
        }
    }

    #[test]
    fn stable_clusters_keep_two_track_and_rolling_identity_without_members() {
        let mut clusters = SessionClusters::default();
        assert_eq!(clusters.assign(&result(vec![1.0, 0.0])), vec![(0, 1)]);
        assert_eq!(clusters.assign(&result(vec![0.0, 1.0])), vec![(0, 2)]);
        assert_eq!(clusters.assign(&result(vec![0.0, 1.0])), vec![(0, 2)]);
        assert_eq!(clusters.assign(&result(vec![1.0, 0.0])), vec![(0, 1)]);
        assert_eq!(clusters.groups.len(), 2);
        assert!(
            clusters
                .groups
                .iter()
                .all(|cluster| cluster.centroid.len() == 2)
        );
    }

    #[test]
    fn short_invalid_similar_and_overlapping_voices_abstain() {
        let mut clusters = SessionClusters::default();
        let short = Diarization {
            turns: vec![Turn {
                start_ms: 0,
                end_ms: 300,
                speaker: 0,
            }],
            speakers: vec![voice(0, vec![1.0, 0.0], 300)],
        };
        assert!(clusters.assign(&short).is_empty());
        assert!(clusters.assign(&result(vec![f32::NAN, 0.0])).is_empty());
        assert!(
            clusters
                .assign(&result(vec![1.0; MAX_DIMENSION + 1]))
                .is_empty()
        );
        let overlap = Diarization {
            turns: vec![
                Turn {
                    start_ms: 0,
                    end_ms: 5_000,
                    speaker: 0,
                },
                Turn {
                    start_ms: 0,
                    end_ms: 5_000,
                    speaker: 1,
                },
            ],
            speakers: vec![
                voice(0, vec![1.0, 0.0], 5_000),
                voice(1, vec![0.0, 1.0], 5_000),
            ],
        };
        assert!(clusters.assign(&overlap).is_empty());
        assert_eq!(clusters.assign(&result(vec![1.0, 0.0])), vec![(0, 1)]);
        assert!(clusters.assign(&result(vec![0.50, 0.866])).is_empty());
        assert_eq!(clusters.groups.len(), 1);
    }

    #[test]
    fn twelve_clusters_are_never_recycled_or_forced() {
        let mut clusters = SessionClusters::default();
        for index in 0..12 {
            let mut vector = vec![0.0; 13];
            vector[index] = 1.0;
            assert_eq!(
                clusters.assign(&result(vector)),
                vec![(0, index as u32 + 1)]
            );
        }
        let mut unseen = vec![0.0; 13];
        unseen[12] = 1.0;
        assert!(clusters.assign(&result(unseen)).is_empty());
        assert_eq!(clusters.groups.len(), 12);
    }

    #[test]
    fn fixed_rings_and_replaceable_batches_bound_owned_pcm() {
        let mut queue = Queue::default();
        for second in 0..3_600 {
            for source in [Speaker::Me, Speaker::Others] {
                queue.feed(source, second * 1_000, &vec![0.2; 16_000], 0);
            }
            let metrics = queue.metrics(0, false, 0, 0, 0);
            assert!(metrics.ring_samples <= 2 * WINDOW);
            assert!(metrics.pending_samples <= 2 * WINDOW);
            assert!(metrics.ring_samples + metrics.pending_samples + 2 * WINDOW <= 6 * WINDOW);
        }
        let pending = queue.pending.as_ref().unwrap();
        assert!(pending.tracks.iter().all(|track| track.end_ms >= 3_590_000));
    }

    #[test]
    fn logical_cancel_rejects_late_output_and_retains_physical_occupancy() {
        let _serial = test_worker_guard();
        let entered = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let observer = Arc::new(Observer::default());
        let (e, r) = (entered.clone(), released.clone());
        let worker = LiveSpeakerWorker::start(
            "owned".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || {
                Ok(Box::new(Blocking {
                    entered: e,
                    released: r,
                }))
            }),
            observer.clone(),
        );
        worker.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| entered.load(Ordering::SeqCst) == 1);
        worker.stop();
        assert!(worker.metrics().physical_active);
        assert!(!worker.status().enabled);
        released.store(true, Ordering::SeqCst);
        until(|| !worker.metrics().physical_active);
        assert!(observer.0.lock().unwrap().is_empty());
    }

    #[test]
    fn actual_two_track_windows_share_stable_centroids_without_double_weighting_replay() {
        let mut clusters = SessionClusters::default();
        let first = result(vec![1.0, 0.0]);
        let second = result(vec![0.0, 1.0]);
        assert_eq!(clusters.assign_at(&first, Speaker::Me, 0, 10_000), [(0, 1)]);
        assert_eq!(
            clusters.assign_at(&second, Speaker::Others, 0, 10_000),
            [(0, 2)]
        );
        let weights: Vec<_> = clusters.groups.iter().map(|group| group.weight).collect();
        assert_eq!(clusters.assign_at(&first, Speaker::Me, 0, 10_000), [(0, 1)]);
        assert_eq!(
            clusters.assign_at(&second, Speaker::Others, 0, 10_000),
            [(0, 2)]
        );
        assert_eq!(
            clusters
                .groups
                .iter()
                .map(|group| group.weight)
                .collect::<Vec<_>>(),
            weights
        );
        assert_eq!(
            clusters.assign_at(&second, Speaker::Me, 10_000, 20_000),
            [(0, 2)]
        );
        assert_eq!(
            clusters.assign_at(&first, Speaker::Others, 10_000, 20_000),
            [(0, 1)]
        );
        assert_eq!(clusters.groups.len(), 2);
    }

    #[test]
    fn privacy_pause_epoch_clears_audio_even_if_old_clear_lost_a_lock_race() {
        let mut queue = Queue::default();
        queue.feed(Speaker::Me, 0, &vec![0.75; 10 * 16_000], 0);
        queue.feed(Speaker::Me, 20_000, &vec![0.25; 10 * 16_000], 2);
        let pending = queue.pending.as_ref().unwrap();
        assert_eq!(pending.epoch, 2);
        assert!(
            pending.tracks.iter().all(|track| track.start_ms >= 20_000
                && track.samples.iter().all(|sample| *sample == 0.25))
        );
    }

    #[test]
    fn authoritative_boundaries_stay_pinned_when_continuous_rings_advance() {
        let mut queue = Queue::default();
        let chunk = vec![0.2; WINDOW];
        for source in [Speaker::Me, Speaker::Others] {
            queue.feed(source, 0, &chunk, 0);
            assert!(queue.submit_authoritative(source, 0, &chunk, 0));
        }
        for source in [Speaker::Me, Speaker::Others] {
            queue.feed(source, 30_000, &vec![0.3; 8_000], 0);
        }
        let batch = queue.take_latest().unwrap();
        assert_eq!(batch.tracks.len(), 2);
        assert!(
            batch
                .tracks
                .iter()
                .all(|track| track.start_ms == 0 && track.end_ms == 30_000 && track.authoritative)
        );
        assert_eq!(batch.samples(), 2 * WINDOW);
        assert!(
            queue.metrics(batch.samples(), false, 0, 0, 0).ring_samples + batch.samples()
                <= 6 * WINDOW
        );
        queue.mark_admitted(&batch);
        assert!(queue.submit_authoritative(Speaker::Me, 30_000, &vec![0.2; 10 * 16_000], 0));
        assert!(!queue.ready());
        queue.feed(Speaker::Me, 30_500, &vec![0.2; 10 * 16_000], 0);
        assert!(queue.ready());
        let next = queue.take_latest().unwrap();
        assert_eq!(
            next.tracks
                .iter()
                .find(|track| track.source == Speaker::Me)
                .unwrap()
                .start_ms,
            30_000
        );
    }

    #[test]
    fn foreground_defer_keeps_context_and_prevents_native_admission() {
        let _serial = test_worker_guard();
        let calls = Arc::new(AtomicUsize::new(0));
        let observer = Arc::new(Observer::default());
        let counter = calls.clone();
        let worker = LiveSpeakerWorker::start(
            "deferred".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || Ok(Box::new(Immediate(counter)))),
            observer.clone(),
        );
        worker.set_deferred(true);
        worker.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| worker.metrics().pending_samples == 10 * 16_000);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        worker.set_deferred(false);
        until(|| observer.0.lock().unwrap().len() == 1);
        assert_eq!(observer.0.lock().unwrap()[0].start_ms, 0);
        assert_eq!(worker.metrics().native_calls, 1);
        worker.stop();
    }

    #[test]
    fn pause_and_resume_cannot_publish_an_old_in_flight_window() {
        let _serial = test_worker_guard();
        let entered = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let observer = Arc::new(Observer::default());
        let (e, r) = (entered.clone(), released.clone());
        let worker = LiveSpeakerWorker::start(
            "resumed".into(),
            4,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || {
                Ok(Box::new(Blocking {
                    entered: e,
                    released: r,
                }))
            }),
            observer.clone(),
        );
        worker.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| entered.load(Ordering::SeqCst) == 1);
        worker.set_paused(true);
        worker.set_paused(false);
        worker.feed(Speaker::Me, 20_000, &vec![0.3; 10 * 16_000]);
        released.store(true, Ordering::SeqCst);
        until(|| observer.0.lock().unwrap().len() == 1);
        let updates = observer.0.lock().unwrap();
        assert_eq!(updates[0].start_ms, 20_000);
        assert_eq!(updates[0].generation, 4);
        assert_eq!(worker.metrics().clusters, 1);
        drop(updates);
        worker.stop();
    }

    #[test]
    fn changed_configuration_discards_native_result_and_does_not_retry() {
        let _serial = test_worker_guard();
        let entered = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let configured = Arc::new(AtomicBool::new(true));
        let observer = Arc::new(Observer::default());
        let (e, r, c) = (entered.clone(), released.clone(), configured.clone());
        let worker = LiveSpeakerWorker::start(
            "configuration".into(),
            1,
            "fixture".into(),
            Arc::new(move || c.load(Ordering::SeqCst)),
            Box::new(move || {
                Ok(Box::new(Blocking {
                    entered: e,
                    released: r,
                }))
            }),
            observer.clone(),
        );
        worker.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| entered.load(Ordering::SeqCst) == 1);
        configured.store(false, Ordering::SeqCst);
        released.store(true, Ordering::SeqCst);
        until(|| worker.status().state == LiveSpeakerState::Unavailable);
        assert!(observer.0.lock().unwrap().is_empty());
        assert_eq!(worker.metrics().clusters, 0);
        worker.feed(Speaker::Me, 20_000, &vec![0.2; 10 * 16_000]);
        assert_eq!(entered.load(Ordering::SeqCst), 1);
        worker.stop();
    }

    #[test]
    fn retired_native_worker_keeps_its_slot_and_offline_work_has_priority() {
        let _serial = test_worker_guard();
        let mut cleanup = PriorityFixtureCleanup::default();
        let activity = Arc::new(PhysicalActivity::default());
        let samples = vec![0.2; 10 * 16_000];
        // A previous fixture can release the test mutex before its native worker actually exits.
        let previous_entered = Arc::new(AtomicUsize::new(0));
        let previous_release = Arc::new(AtomicBool::new(false));
        cleanup.releases.push(previous_release.clone());
        let (e, r, a) = (
            previous_entered.clone(),
            previous_release.clone(),
            activity.clone(),
        );
        let previous = LiveSpeakerWorker::start(
            "previous-fixture".into(),
            0,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || {
                Ok(Box::new(TrackedInference {
                    inner: Blocking {
                        entered: e,
                        released: r,
                    },
                    activity: a,
                }))
            }),
            Arc::new(Observer::default()),
        );
        cleanup.workers.push(previous.clone());
        until_phase("predecessor admission", || previous.is_admitted());
        feed_accepted(&previous, 0, &samples, "predecessor accepted audio");
        until_phase("predecessor inference entry", || {
            previous_entered.load(Ordering::SeqCst) == 1
        });
        previous.stop();

        let entered = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        cleanup.releases.push(released.clone());
        let old_observer = Arc::new(Observer::default());
        let (e, r, a) = (entered.clone(), released.clone(), activity.clone());
        let old = LiveSpeakerWorker::start(
            "retired".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || {
                Ok(Box::new(TrackedInference {
                    inner: Blocking {
                        entered: e,
                        released: r,
                    },
                    activity: a,
                }))
            }),
            old_observer.clone(),
        );
        cleanup.workers.push(old.clone());
        assert!(!old.is_admitted());
        old.feed(Speaker::Me, 0, &samples);
        assert_eq!(old.metrics().ring_samples, 0);
        assert_eq!(old.metrics().pending_samples, 0);
        assert!(previous.metrics().physical_active);
        previous_release.store(true, Ordering::SeqCst);
        until_phase("initial worker admission after predecessor exit", || {
            old.is_admitted()
        });
        feed_accepted(&old, 0, &samples, "initial worker accepted audio");
        until_phase("initial worker inference entry", || {
            entered.load(Ordering::SeqCst) == 1
        });
        old.stop();
        let offline_entered = Arc::new(AtomicBool::new(false));
        let offline_release = Arc::new(AtomicBool::new(false));
        cleanup.releases.push(offline_release.clone());
        let (ready, finish, a) = (
            offline_entered.clone(),
            offline_release.clone(),
            activity.clone(),
        );
        cleanup.foreground = Some(std::thread::spawn(move || {
            let _permit = offline_acoustic_permit();
            let _physical = PhysicalActivity::enter(a);
            ready.store(true, Ordering::SeqCst);
            until_phase("offline fixture release", || finish.load(Ordering::SeqCst));
        }));
        until_phase("offline demand registered", || {
            OFFLINE_DEMAND.load(Ordering::Acquire) > 0
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let a = activity.clone();
        let new_observer = Arc::new(Observer::default());
        let new = LiveSpeakerWorker::start(
            "new".into(),
            2,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || {
                Ok(Box::new(TrackedInference {
                    inner: Immediate(counter),
                    activity: a,
                }))
            }),
            new_observer.clone(),
        );
        cleanup.workers.push(new.clone());
        new.feed(Speaker::Me, 0, &samples);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(old.metrics().physical_active);
        released.store(true, Ordering::SeqCst);
        until_phase("offline physical permit acquired", || {
            offline_entered.load(Ordering::SeqCst)
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(old_observer.0.lock().unwrap().is_empty());
        offline_release.store(true, Ordering::SeqCst);
        cleanup.foreground.take().unwrap().join().unwrap();
        until_phase("replacement worker admission", || new.is_admitted());
        feed_accepted(&new, 10_000, &samples, "replacement worker accepted audio");
        until_phase("replacement publication", || {
            new_observer.0.lock().unwrap().len() == 1
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(previous_entered.load(Ordering::SeqCst), 1);
        assert_eq!(entered.load(Ordering::SeqCst), 1);
        assert_eq!(activity.maximum.load(Ordering::SeqCst), 1);
        assert!(
            cleanup.finish(),
            "physical priority fixture cleanup did not finish"
        );
        assert_eq!(activity.active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn replacement_storm_has_only_one_pending_activation_and_no_retiring_pcm_overlap() {
        let _serial = test_worker_guard();
        let entered = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let (e, r) = (entered.clone(), released.clone());
        let old = LiveSpeakerWorker::start(
            "old".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || {
                Ok(Box::new(Blocking {
                    entered: e,
                    released: r,
                }))
            }),
            Arc::new(Observer::default()),
        );
        until(|| old.shared.admitted.load(Ordering::Acquire));
        old.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| entered.load(Ordering::SeqCst) == 1);
        old.stop();
        let calls = Arc::new(AtomicUsize::new(0));
        let observer = Arc::new(Observer::default());
        let mut handles = Vec::new();
        for generation in 0..40 {
            let counter = calls.clone();
            let next = LiveSpeakerWorker::start(
                "replacement".into(),
                generation,
                "fixture".into(),
                Arc::new(|| true),
                Box::new(move || Ok(Box::new(Immediate(counter)))),
                observer.clone(),
            );
            next.feed(Speaker::Me, 20_000, &vec![0.2; 10 * 16_000]);
            let metrics = next.metrics();
            assert_eq!(
                metrics.ring_samples + metrics.pending_samples + metrics.active_samples,
                0
            );
            assert!(old.metrics().physical_active);
            handles.push(next);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(handles[..39].iter().all(|worker| !worker.status().enabled));
        released.store(true, Ordering::SeqCst);
        let latest = handles.last().unwrap();
        until(|| latest.shared.admitted.load(Ordering::Acquire));
        assert_eq!(old.metrics().ring_samples, 0);
        latest.feed(Speaker::Me, 30_000, &vec![0.2; 10 * 16_000]);
        until(|| calls.load(Ordering::SeqCst) == 1);
        latest.stop();
    }

    #[test]
    fn private_session_memory_survives_toggle_and_refuses_model_or_session_reuse() {
        let _serial = test_worker_guard();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let first_observer = Arc::new(Observer::default());
        let first = LiveSpeakerWorker::start(
            "same-session".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || Ok(Box::new(Immediate(count)))),
            first_observer.clone(),
        );
        until(|| first.shared.admitted.load(Ordering::Acquire));
        first.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| first_observer.0.lock().unwrap().len() == 1);
        let memory = first.memory();
        first.stop();
        struct OtherVoice;
        impl LiveSpeakerInference for OtherVoice {
            fn infer(&mut self, _: &[f32]) -> Result<Diarization, PennelError> {
                Ok(result(vec![0.0, 1.0]))
            }
        }
        let next_observer = Arc::new(Observer::default());
        let next = LiveSpeakerWorker::start_with_memory(
            "same-session".into(),
            2,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(|| Ok(Box::new(OtherVoice))),
            next_observer.clone(),
            memory.clone(),
        );
        until(|| next.shared.admitted.load(Ordering::Acquire));
        next.feed(Speaker::Me, 10_000, &vec![0.2; 10 * 16_000]);
        until(|| next_observer.0.lock().unwrap().len() == 1);
        assert_eq!(next_observer.0.lock().unwrap()[0].voice_map, [(0, 2)]);
        assert_eq!(next.metrics().clusters, 2);
        next.stop();
        for (session, model) in [
            ("same-session", "different-model"),
            ("another-session", "fixture"),
        ] {
            let rejected = LiveSpeakerWorker::start_with_memory(
                session.into(),
                3,
                model.into(),
                Arc::new(|| true),
                Box::new(|| panic!("An incompatible session must not load acoustic models")),
                Arc::new(Observer::default()),
                memory.clone(),
            );
            assert_eq!(rejected.status().state, LiveSpeakerState::Unavailable);
            assert_eq!(rejected.metrics().ring_samples, 0);
        }
    }

    #[test]
    fn privacy_and_foreground_pause_reasons_compose_truthfully() {
        let _serial = test_worker_guard();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let worker = LiveSpeakerWorker::start(
            "paused".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || Ok(Box::new(Immediate(count)))),
            Arc::new(Observer::default()),
        );
        worker.set_paused(true);
        worker.set_deferred(true);
        worker.set_deferred(false);
        assert_eq!(worker.status().reason_code.as_deref(), Some("paused"));
        worker.set_deferred(true);
        worker.set_paused(false);
        assert_eq!(
            worker.status().reason_code.as_deref(),
            Some("foreground_work")
        );
        worker.set_deferred(false);
        assert_eq!(worker.status().state, LiveSpeakerState::Warming);
        worker.stop();
    }

    #[test]
    fn inference_failure_is_explicit_and_does_not_create_automatic_retries() {
        let _serial = test_worker_guard();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        struct Failure(Arc<AtomicUsize>);
        impl LiveSpeakerInference for Failure {
            fn infer(&mut self, _: &[f32]) -> Result<Diarization, PennelError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(PennelError::InvalidResponse {
                    message: "Owned inference failure".into(),
                })
            }
        }
        let observer = Arc::new(Observer::default());
        let worker = LiveSpeakerWorker::start(
            "failed".into(),
            1,
            "fixture".into(),
            Arc::new(|| true),
            Box::new(move || Ok(Box::new(Failure(count)))),
            observer.clone(),
        );
        until(|| worker.shared.admitted.load(Ordering::Acquire));
        worker.feed(Speaker::Me, 0, &vec![0.2; 10 * 16_000]);
        until(|| worker.status().state == LiveSpeakerState::Failed);
        worker.feed(Speaker::Me, 20_000, &vec![0.2; 10 * 16_000]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(observer.0.lock().unwrap().is_empty());
        assert!(!worker.metrics().physical_active);
        worker.stop();
    }
}
