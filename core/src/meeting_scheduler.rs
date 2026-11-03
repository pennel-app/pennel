use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{InferenceCancellation, PennelEngine, PennelError, QualifiedMeetingAnalysis};

const MAX_WAITING_WORKERS: usize = 32;
const MAX_MEETINGS: usize = 16;
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(60);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "snake_case")]
pub enum InferencePriority {
    Speech,
    ForegroundQuestion,
    ExplicitAnalysis,
    BackgroundAnalysis,
}

impl InferencePriority {
    fn optional(self) -> bool {
        matches!(self, Self::ExplicitAnalysis | Self::BackgroundAnalysis)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum InferenceWorkerOutcome {
    Completed,
    Failed,
    Cancelled,
}

pub(crate) trait InferencePhysicalObserver: Send + Sync {
    fn starting(&self) -> Result<(), PennelError>;
    fn finished(&self, outcome: InferenceWorkerOutcome);
    fn current(&self) -> bool {
        true
    }
}

struct ActiveWorker {
    id: u64,
    priority: InferencePriority,
    cancellation: Weak<InferenceCancellation>,
}

struct WaitingWorker {
    id: u64,
    priority: InferencePriority,
    cancellation: Weak<InferenceCancellation>,
    deadline: Instant,
}

#[derive(Default)]
struct PhysicalState {
    next_id: u64,
    active: Vec<ActiveWorker>,
    waiting: VecDeque<WaitingWorker>,
    speech: usize,
    foreground: usize,
}

#[derive(Default)]
pub(crate) struct PhysicalInferencePool {
    state: Mutex<PhysicalState>,
    changed: Condvar,
}

pub(crate) struct PhysicalInferencePermit {
    pool: Arc<PhysicalInferencePool>,
    id: u64,
}

impl Drop for PhysicalInferencePermit {
    fn drop(&mut self) {
        self.pool
            .state
            .lock()
            .unwrap()
            .active
            .retain(|worker| worker.id != self.id);
        self.pool.changed.notify_all();
    }
}

impl PhysicalInferencePool {
    fn capacity(state: &PhysicalState, priority: InferencePriority) -> bool {
        if state.active.len() >= 2 {
            return false;
        }
        if priority.optional() {
            if state.active.iter().any(|worker| {
                worker.priority.optional() || worker.priority == InferencePriority::Speech
            }) || state.speech > 0
                || state.foreground > 0
                || state
                    .waiting
                    .iter()
                    .any(|worker| worker.priority <= InferencePriority::ForegroundQuestion)
            {
                return false;
            }
            if priority == InferencePriority::BackgroundAnalysis
                && (state
                    .active
                    .iter()
                    .any(|worker| !worker.priority.optional())
                    || state
                        .waiting
                        .iter()
                        .any(|worker| worker.priority == InferencePriority::ExplicitAnalysis))
            {
                return false;
            }
        }
        true
    }

    fn cancel_lower(state: &PhysicalState, priority: InferencePriority) {
        for worker in &state.active {
            let preempt = match priority {
                InferencePriority::Speech => worker.priority.optional(),
                InferencePriority::ForegroundQuestion | InferencePriority::ExplicitAnalysis => {
                    worker.priority == InferencePriority::BackgroundAnalysis
                }
                InferencePriority::BackgroundAnalysis => false,
            };
            if preempt && let Some(cancellation) = worker.cancellation.upgrade() {
                cancellation.cancel();
            }
        }
    }

    #[cfg(test)]
    fn try_acquire(
        self: &Arc<Self>,
        priority: InferencePriority,
        cancellation: &Arc<InferenceCancellation>,
    ) -> Option<PhysicalInferencePermit> {
        let mut state = self.state.lock().unwrap();
        if cancellation.is_cancelled()
            || !Self::capacity(&state, priority)
            || !state.waiting.is_empty()
        {
            return None;
        }
        let id = state.next_id;
        state.next_id += 1;
        state.active.push(ActiveWorker {
            id,
            priority,
            cancellation: Arc::downgrade(cancellation),
        });
        Some(PhysicalInferencePermit {
            pool: self.clone(),
            id,
        })
    }

    pub(crate) fn acquire(
        self: &Arc<Self>,
        priority: InferencePriority,
        cancellation: &Arc<InferenceCancellation>,
        deadline: Instant,
    ) -> Result<PhysicalInferencePermit, PennelError> {
        let mut state = self.state.lock().unwrap();
        state.waiting.retain(|worker| {
            worker.deadline > Instant::now()
                && worker
                    .cancellation
                    .upgrade()
                    .is_some_and(|token| !token.is_cancelled())
        });
        if state.waiting.len() >= MAX_WAITING_WORKERS {
            if let Some(index) = state
                .waiting
                .iter()
                .enumerate()
                .filter(|(_, worker)| worker.priority > priority)
                .max_by_key(|(_, worker)| (worker.priority, worker.id))
                .map(|(index, _)| index)
            {
                if let Some(displaced) = state
                    .waiting
                    .remove(index)
                    .and_then(|worker| worker.cancellation.upgrade())
                {
                    displaced.cancel();
                }
            } else {
                return Err(PennelError::InvalidResponse {
                    message:
                        "Text processing is busy. Try again after the current request finishes."
                            .into(),
                });
            }
        }
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        Self::cancel_lower(&state, priority);
        state.waiting.push_back(WaitingWorker {
            id,
            priority,
            cancellation: Arc::downgrade(cancellation),
            deadline,
        });
        loop {
            let now = Instant::now();
            if cancellation.is_cancelled() || now >= deadline {
                state.waiting.retain(|worker| worker.id != id);
                self.changed.notify_all();
                return if cancellation.is_cancelled() {
                    Err(PennelError::Cancelled)
                } else {
                    Err(PennelError::Network {
                        message: "The text processing deadline was exceeded.".into(),
                    })
                };
            }
            state.waiting.retain(|worker| {
                worker.deadline > now
                    && worker
                        .cancellation
                        .upgrade()
                        .is_some_and(|token| !token.is_cancelled())
            });
            let next = state
                .waiting
                .iter()
                .filter(|worker| Self::capacity(&state, worker.priority))
                .min_by_key(|worker| (worker.priority, worker.id))
                .map(|worker| worker.id);
            if next == Some(id) {
                state.waiting.retain(|worker| worker.id != id);
                state.active.push(ActiveWorker {
                    id,
                    priority,
                    cancellation: Arc::downgrade(cancellation),
                });
                self.changed.notify_all();
                return Ok(PhysicalInferencePermit {
                    pool: self.clone(),
                    id,
                });
            }
            let duration = deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(20));
            state = self.changed.wait_timeout(state, duration).unwrap().0;
        }
    }

    fn optional_available(&self, priority: InferencePriority) -> bool {
        let state = self.state.lock().unwrap();
        Self::capacity(&state, priority)
    }

    fn demand(self: &Arc<Self>, priority: InferencePriority) -> InferencePriorityDemand {
        let mut state = self.state.lock().unwrap();
        if priority == InferencePriority::Speech {
            state.speech += 1;
        } else {
            state.foreground += 1;
        }
        Self::cancel_lower(&state, priority);
        self.changed.notify_all();
        InferencePriorityDemand {
            pool: self.clone(),
            priority,
            released: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub(crate) fn speech_started(&self) {
        let mut state = self.state.lock().unwrap();
        state.speech += 1;
        Self::cancel_lower(&state, InferencePriority::Speech);
        self.changed.notify_all();
    }

    pub(crate) fn speech_finished(&self) {
        let mut state = self.state.lock().unwrap();
        state.speech = state.speech.saturating_sub(1);
        self.changed.notify_all();
    }

    fn resource_demand(&self) -> MeetingAnalysisResourceDemand {
        let state = self.state.lock().unwrap();
        MeetingAnalysisResourceDemand {
            speech_busy: state.speech > 0
                || state
                    .active
                    .iter()
                    .any(|worker| worker.priority == InferencePriority::Speech),
            foreground_busy: state.foreground > 0
                || state
                    .active
                    .iter()
                    .any(|worker| worker.priority == InferencePriority::ForegroundQuestion)
                || state
                    .waiting
                    .iter()
                    .any(|worker| worker.priority <= InferencePriority::ForegroundQuestion),
        }
    }
}

pub(crate) struct InferencePriorityDemand {
    pool: Arc<PhysicalInferencePool>,
    priority: InferencePriority,
    released: std::sync::atomic::AtomicBool,
}

impl InferencePriorityDemand {
    pub(crate) fn release(&self) {
        use std::sync::atomic::Ordering;
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut state = self.pool.state.lock().unwrap();
        if self.priority == InferencePriority::Speech {
            state.speech = state.speech.saturating_sub(1);
        } else {
            state.foreground = state.foreground.saturating_sub(1);
        }
        self.pool.changed.notify_all();
    }
}

impl Drop for InferencePriorityDemand {
    fn drop(&mut self) {
        self.release();
    }
}

pub(crate) fn speech_priority() -> InferencePriorityDemand {
    physical_pool().demand(InferencePriority::Speech)
}

pub struct ForegroundQuestionAdmission {
    cancellation: Arc<InferenceCancellation>,
    deadline: Instant,
    demand: InferencePriorityDemand,
    meeting_uid: Option<String>,
    inference_epoch: Option<u64>,
}

impl ForegroundQuestionAdmission {
    pub fn begin_meeting(parent: Arc<InferenceCancellation>, meeting_uid: String) -> Self {
        Self::for_meeting(
            parent,
            Instant::now() + Duration::from_secs(crate::meeting_chat::ASK_TIMEOUT_SECS),
            meeting_uid,
        )
    }
    pub fn new(parent: Arc<InferenceCancellation>, deadline: Instant) -> Self {
        Self {
            cancellation: InferenceCancellation::scheduled(
                parent,
                InferencePriority::ForegroundQuestion,
                deadline,
            ),
            deadline,
            demand: physical_pool().demand(InferencePriority::ForegroundQuestion),
            meeting_uid: None,
            inference_epoch: None,
        }
    }
    pub fn for_meeting(
        parent: Arc<InferenceCancellation>,
        deadline: Instant,
        meeting_uid: String,
    ) -> Self {
        let mut admission = Self::new(parent, deadline);
        admission.meeting_uid = Some(meeting_uid);
        admission
    }
    pub fn cancellation(&self) -> Arc<InferenceCancellation> {
        self.cancellation.clone()
    }
    pub fn with_inference_epoch(mut self, epoch: u64) -> Self {
        self.inference_epoch = Some(epoch);
        self
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn dispatched(&self) {
        self.demand.release();
    }
    pub(crate) fn meeting_uid(&self) -> Option<&str> {
        self.meeting_uid.as_deref()
    }
    pub(crate) fn inference_epoch(&self) -> Option<u64> {
        self.inference_epoch
    }
}

pub(crate) fn physical_pool() -> &'static Arc<PhysicalInferencePool> {
    static POOL: std::sync::OnceLock<Arc<PhysicalInferencePool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| Arc::new(PhysicalInferencePool::default()))
}

#[cfg(test)]
pub(crate) fn physical_test_gate() -> std::sync::MutexGuard<'static, ()> {
    static GATE: Mutex<()> = Mutex::new(());
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingAnalysisIdentity {
    pub meeting_uid: String,
    pub session_id: String,
    pub source_epoch: u64,
    pub source_fingerprint: String,
    pub included_through_ms: u64,
    pub processing_fingerprint: String,
    pub credential_epoch: u64,
    pub account_epoch: u64,
}

impl MeetingAnalysisIdentity {
    fn valid(&self) -> bool {
        [&self.meeting_uid, &self.session_id]
            .into_iter()
            .all(|value| !value.is_empty() && value.len() <= 256)
            && [&self.source_fingerprint, &self.processing_fingerprint]
                .into_iter()
                .all(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
    }
    fn same_binding(&self, other: &Self) -> bool {
        self.meeting_uid == other.meeting_uid
            && self.session_id == other.session_id
            && self.source_epoch == other.source_epoch
            && self.processing_fingerprint == other.processing_fingerprint
            && self.credential_epoch == other.credential_epoch
            && self.account_epoch == other.account_epoch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingAnalysisResourceHints {
    pub enabled: bool,
    pub workspace_visible: bool,
    pub capture_healthy: bool,
    pub transcription_caught_up: bool,
    pub speech_busy: bool,
    pub foreground_busy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingAnalysisResourceDemand {
    pub speech_busy: bool,
    pub foreground_busy: bool,
}

#[uniffi::export]
impl PennelEngine {
    pub fn meeting_analysis_resource_demand(&self) -> MeetingAnalysisResourceDemand {
        physical_pool().resource_demand()
    }
}

impl MeetingAnalysisResourceHints {
    fn background_reason(self) -> Option<&'static str> {
        if !self.enabled {
            Some("disabled")
        } else if !self.workspace_visible {
            Some("workspace_hidden")
        } else if !self.capture_healthy {
            Some("capture_unhealthy")
        } else if !self.transcription_caught_up {
            Some("transcription_pending")
        } else if self.speech_busy {
            Some("speech_busy")
        } else if self.foreground_busy {
            Some("foreground_busy")
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum MeetingAnalysisState {
    NotChecked,
    Unavailable,
    Unqualified,
    Busy,
    Paused,
    Running,
    Cancelled,
    Failed,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingAnalysisStatus {
    pub state: MeetingAnalysisState,
    pub reason_code: String,
    pub active: bool,
    pub pending: bool,
    pub physical_running: bool,
    pub revision: u64,
    pub started_batches: u64,
    pub replaced_snapshots: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingAnalysisScheduleError {
    Busy,
    Unavailable,
    Unqualified,
    Stale,
    Expired,
}

impl MeetingAnalysisScheduleError {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Unavailable => "paused",
            Self::Unqualified => "unqualified",
            Self::Stale => "stale",
            Self::Expired => "deadline_expired",
        }
    }
}

pub struct ScheduledMeetingSnapshot<T> {
    pub identity: MeetingAnalysisIdentity,
    pub payload: T,
    pub deadline: Instant,
}

pub struct MeetingAnalysisAdmission {
    cancellation: Arc<InferenceCancellation>,
    deadline: Instant,
    observer: Arc<dyn InferencePhysicalObserver>,
    _registration: crate::inference::InferenceObserverRegistration,
    identity: MeetingAnalysisIdentity,
}

impl MeetingAnalysisAdmission {
    pub fn cancellation(&self) -> Arc<InferenceCancellation> {
        self.cancellation.clone()
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn is_current(&self) -> bool {
        self.observer.current()
    }
    pub fn identity(&self) -> &MeetingAnalysisIdentity {
        &self.identity
    }
}

pub struct ReadyMeetingAnalysis<T> {
    pub payload: T,
    pub admission: MeetingAnalysisAdmission,
}

struct PendingAnalysis<T> {
    snapshot: ScheduledMeetingSnapshot<T>,
    ticket: u64,
}

struct ActiveAnalysis {
    id: u64,
    cancellation: Arc<InferenceCancellation>,
    priority: InferencePriority,
    physical: usize,
}

struct MeetingQueue<T> {
    identity: MeetingAnalysisIdentity,
    hints: MeetingAnalysisResourceHints,
    active: Option<ActiveAnalysis>,
    pending: Option<PendingAnalysis<T>>,
    last_started: Option<Instant>,
    status: MeetingAnalysisStatus,
}

struct SchedulerState<T> {
    meetings: HashMap<String, MeetingQueue<T>>,
    next_id: u64,
}

struct SchedulerInner<T> {
    state: Mutex<SchedulerState<T>>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    pool: Arc<PhysicalInferencePool>,
}

struct AnalysisRun<T: Send + 'static> {
    inner: Weak<SchedulerInner<T>>,
    identity: MeetingAnalysisIdentity,
    id: u64,
    cancellation: Arc<InferenceCancellation>,
    deadline: Instant,
    priority: InferencePriority,
    started: std::sync::atomic::AtomicBool,
    outcome: std::sync::atomic::AtomicU8,
}

impl<T: Send + 'static> InferencePhysicalObserver for AnalysisRun<T> {
    fn starting(&self) -> Result<(), PennelError> {
        use std::sync::atomic::Ordering;
        let Some(inner) = self.inner.upgrade() else {
            return Err(PennelError::Cancelled);
        };
        let now = (inner.clock)();
        let mut state = inner.state.lock().unwrap();
        let Some(meeting) = state.meetings.get_mut(&self.identity.meeting_uid) else {
            return Err(PennelError::Cancelled);
        };
        if self.cancellation.is_cancelled()
            || now >= self.deadline
            || !meeting.identity.same_binding(&self.identity)
            || !meeting
                .active
                .as_ref()
                .is_some_and(|active| active.id == self.id)
            || self.priority == InferencePriority::BackgroundAnalysis
                && meeting.hints.background_reason().is_some()
        {
            return Err(PennelError::Cancelled);
        }
        let first = !self.started.swap(true, Ordering::AcqRel);
        if first && self.priority == InferencePriority::BackgroundAnalysis {
            if meeting
                .last_started
                .is_some_and(|last| now.saturating_duration_since(last) < BACKGROUND_INTERVAL)
            {
                self.started.store(false, Ordering::Release);
                return Err(PennelError::Cancelled);
            }
            meeting.last_started = Some(now);
            meeting.status.started_batches += 1;
        }
        meeting.active.as_mut().unwrap().physical += 1;
        meeting.status.state = MeetingAnalysisState::Running;
        meeting.status.reason_code = "running".into();
        meeting.status.revision += 1;
        Ok(())
    }

    fn finished(&self, outcome: InferenceWorkerOutcome) {
        use std::sync::atomic::Ordering;
        self.outcome.store(
            match outcome {
                InferenceWorkerOutcome::Completed => 1,
                InferenceWorkerOutcome::Failed => 2,
                InferenceWorkerOutcome::Cancelled => 3,
            },
            Ordering::Release,
        );
        if let Some(inner) = self.inner.upgrade() {
            let mut state = inner.state.lock().unwrap();
            if let Some(meeting) = state.meetings.get_mut(&self.identity.meeting_uid)
                && let Some(active) = meeting
                    .active
                    .as_mut()
                    .filter(|active| active.id == self.id)
            {
                active.physical = active.physical.saturating_sub(1);
                meeting.status.revision += 1;
            }
        }
    }

    fn current(&self) -> bool {
        let Some(inner) = self.inner.upgrade() else {
            return false;
        };
        if self.cancellation.is_cancelled() || (inner.clock)() >= self.deadline {
            return false;
        }
        inner
            .state
            .lock()
            .unwrap()
            .meetings
            .get(&self.identity.meeting_uid)
            .is_some_and(|meeting| {
                meeting.identity.same_binding(&self.identity)
                    && meeting
                        .active
                        .as_ref()
                        .is_some_and(|active| active.id == self.id)
            })
    }
}

impl<T: Send + 'static> Drop for AnalysisRun<T> {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let mut state = inner.state.lock().unwrap();
        if let Some(meeting) = state.meetings.get_mut(&self.identity.meeting_uid)
            && meeting
                .active
                .as_ref()
                .is_some_and(|active| active.id == self.id)
        {
            meeting.active = None;
            let expired = (inner.clock)() >= self.deadline;
            meeting.status.state = if self.cancellation.is_cancelled()
                || !meeting.identity.same_binding(&self.identity)
            {
                MeetingAnalysisState::Cancelled
            } else if expired || self.outcome.load(Ordering::Acquire) == 2 {
                MeetingAnalysisState::Failed
            } else if self.outcome.load(Ordering::Acquire) == 1 {
                MeetingAnalysisState::Completed
            } else {
                MeetingAnalysisState::NotChecked
            };
            meeting.status.reason_code = if expired {
                "deadline_expired"
            } else {
                match meeting.status.state {
                    MeetingAnalysisState::Cancelled => "cancelled",
                    MeetingAnalysisState::Failed => "failed",
                    MeetingAnalysisState::Completed => "completed",
                    _ => "not_checked",
                }
            }
            .into();
            meeting.status.revision += 1;
        }
    }
}

pub struct MeetingAnalysisCoordinator<T: Send + 'static> {
    inner: Arc<SchedulerInner<T>>,
}

impl<T: Send + 'static> Default for MeetingAnalysisCoordinator<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> MeetingAnalysisCoordinator<T> {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(Instant::now))
    }

    fn with_clock(clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        Self::with_resources(clock, physical_pool().clone())
    }

    fn with_resources(
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
        pool: Arc<PhysicalInferencePool>,
    ) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                state: Mutex::new(SchedulerState {
                    meetings: HashMap::new(),
                    next_id: 0,
                }),
                clock,
                pool,
            }),
        }
    }

    pub fn update_context(
        &self,
        identity: MeetingAnalysisIdentity,
        hints: MeetingAnalysisResourceHints,
    ) -> Result<(), MeetingAnalysisScheduleError> {
        if !identity.valid() {
            return Err(MeetingAnalysisScheduleError::Unavailable);
        }
        let mut state = self.inner.state.lock().unwrap();
        if !state.meetings.contains_key(&identity.meeting_uid)
            && state.meetings.len() >= MAX_MEETINGS
        {
            state
                .meetings
                .retain(|_, meeting| meeting.active.is_some() || meeting.pending.is_some());
            if state.meetings.len() >= MAX_MEETINGS {
                return Err(MeetingAnalysisScheduleError::Busy);
            }
        }
        let meeting = state
            .meetings
            .entry(identity.meeting_uid.clone())
            .or_insert_with(|| MeetingQueue {
                identity: identity.clone(),
                hints,
                active: None,
                pending: None,
                last_started: None,
                status: empty_status(),
            });
        let replaced = !meeting.identity.same_binding(&identity);
        if replaced {
            if let Some(active) = &meeting.active {
                active.cancellation.cancel();
            }
            meeting.pending = None;
        }
        meeting.identity = identity;
        meeting.hints = hints;
        if let Some(reason) = hints.background_reason() {
            meeting.pending = None;
            if let Some(active) = meeting
                .active
                .as_ref()
                .filter(|active| active.priority == InferencePriority::BackgroundAnalysis)
            {
                active.cancellation.cancel();
            }
            meeting.status.state = MeetingAnalysisState::Paused;
            meeting.status.reason_code = reason.into();
        } else if replaced {
            meeting.status.state = MeetingAnalysisState::Cancelled;
            meeting.status.reason_code = "context_changed".into();
        }
        meeting.status.revision += 1;
        Ok(())
    }

    fn admission(
        &self,
        identity: MeetingAnalysisIdentity,
        deadline: Instant,
        parent: Arc<InferenceCancellation>,
        priority: InferencePriority,
        state: &mut SchedulerState<T>,
    ) -> MeetingAnalysisAdmission {
        let cancellation = InferenceCancellation::scheduled(parent, priority, deadline);
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        let meeting = state.meetings.get_mut(&identity.meeting_uid).unwrap();
        meeting.active = Some(ActiveAnalysis {
            id,
            cancellation: cancellation.clone(),
            priority,
            physical: 0,
        });
        meeting.status.state = MeetingAnalysisState::Busy;
        meeting.status.reason_code = "admitted".into();
        meeting.status.revision += 1;
        let observer: Arc<dyn InferencePhysicalObserver> = Arc::new(AnalysisRun {
            inner: Arc::downgrade(&self.inner),
            identity: identity.clone(),
            id,
            cancellation: cancellation.clone(),
            deadline,
            priority,
            started: std::sync::atomic::AtomicBool::new(false),
            outcome: std::sync::atomic::AtomicU8::new(0),
        });
        let registration = cancellation.observe_physical(observer.clone());
        MeetingAnalysisAdmission {
            cancellation,
            deadline,
            observer,
            _registration: registration,
            identity,
        }
    }

    pub fn begin_manual(
        &self,
        identity: MeetingAnalysisIdentity,
        deadline: Instant,
        parent: Arc<InferenceCancellation>,
        qualification: &QualifiedMeetingAnalysis,
    ) -> Result<MeetingAnalysisAdmission, MeetingAnalysisScheduleError> {
        if deadline <= (self.inner.clock)() {
            return Err(MeetingAnalysisScheduleError::Expired);
        }
        if !identity.valid() || parent.is_cancelled() {
            return Err(MeetingAnalysisScheduleError::Stale);
        }
        if !matches!(
            qualification.record().context.request.use_case,
            crate::MeetingAnalysisUseCase::ManualFinishedSuggestions
                | crate::MeetingAnalysisUseCase::ManualLiveSuggestions
        ) || qualification.context_fingerprint() != identity.processing_fingerprint
        {
            return Err(MeetingAnalysisScheduleError::Unqualified);
        }
        let mut state = self.inner.state.lock().unwrap();
        let meeting = state
            .meetings
            .get(&identity.meeting_uid)
            .ok_or(MeetingAnalysisScheduleError::Unavailable)?;
        if !meeting.identity.same_binding(&identity) {
            return Err(MeetingAnalysisScheduleError::Stale);
        }
        if let Some(active) = &meeting.active {
            if active.priority == InferencePriority::BackgroundAnalysis {
                active.cancellation.cancel();
            }
            return Err(MeetingAnalysisScheduleError::Busy);
        }
        if meeting.hints.speech_busy
            || meeting.hints.foreground_busy
            || state.meetings.values().any(|meeting| {
                meeting
                    .active
                    .as_ref()
                    .is_some_and(|active| active.priority == InferencePriority::ExplicitAnalysis)
            })
        {
            return Err(MeetingAnalysisScheduleError::Busy);
        }
        for meeting in state.meetings.values() {
            if let Some(active) = meeting
                .active
                .as_ref()
                .filter(|active| active.priority == InferencePriority::BackgroundAnalysis)
            {
                active.cancellation.cancel();
            }
        }
        state
            .meetings
            .get_mut(&identity.meeting_uid)
            .unwrap()
            .pending = None;
        Ok(self.admission(
            identity,
            deadline,
            parent,
            InferencePriority::ExplicitAnalysis,
            &mut state,
        ))
    }

    pub fn offer_background(
        &self,
        snapshot: ScheduledMeetingSnapshot<T>,
        qualification: Option<&QualifiedMeetingAnalysis>,
        now: Instant,
    ) -> Result<(), MeetingAnalysisScheduleError> {
        let mut state = self.inner.state.lock().unwrap();
        let ticket = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        let meeting = state
            .meetings
            .get_mut(&snapshot.identity.meeting_uid)
            .ok_or(MeetingAnalysisScheduleError::Unavailable)?;
        if !snapshot.identity.valid() || !meeting.identity.same_binding(&snapshot.identity) {
            return Err(MeetingAnalysisScheduleError::Stale);
        }
        if !qualification.is_some_and(|proof| {
            proof.record().context.request.use_case
                == crate::MeetingAnalysisUseCase::BackgroundLiveSuggestions
                && proof.context_fingerprint() == snapshot.identity.processing_fingerprint
        }) {
            meeting.pending = None;
            if let Some(active) = meeting
                .active
                .as_ref()
                .filter(|active| active.priority == InferencePriority::BackgroundAnalysis)
            {
                active.cancellation.cancel();
            }
            meeting.status.state = MeetingAnalysisState::Unqualified;
            meeting.status.reason_code = "unqualified".into();
            meeting.status.revision += 1;
            return Err(MeetingAnalysisScheduleError::Unqualified);
        }
        if snapshot.deadline <= now {
            meeting.pending = None;
            return Err(MeetingAnalysisScheduleError::Expired);
        }
        if let Some(reason) = meeting.hints.background_reason() {
            meeting.pending = None;
            meeting.status.state = MeetingAnalysisState::Paused;
            meeting.status.reason_code = reason.into();
            meeting.status.revision += 1;
            return Err(MeetingAnalysisScheduleError::Unavailable);
        }
        let ticket = if let Some(previous) = &meeting.pending {
            meeting.status.replaced_snapshots += 1;
            previous.ticket
        } else {
            ticket
        };
        meeting.pending = Some(PendingAnalysis { snapshot, ticket });
        meeting.status.revision += 1;
        Ok(())
    }

    pub fn take_background(&self, now: Instant) -> Option<ReadyMeetingAnalysis<T>> {
        if !self
            .inner
            .pool
            .optional_available(InferencePriority::BackgroundAnalysis)
        {
            return None;
        }
        let mut state = self.inner.state.lock().unwrap();
        if state
            .meetings
            .values()
            .any(|meeting| meeting.active.is_some())
        {
            return None;
        }
        for meeting in state.meetings.values_mut() {
            if meeting
                .pending
                .as_ref()
                .is_some_and(|pending| pending.snapshot.deadline <= now)
            {
                meeting.pending = None;
                meeting.status.state = MeetingAnalysisState::Failed;
                meeting.status.reason_code = "deadline_expired".into();
                meeting.status.revision += 1;
            }
        }
        let uid = state
            .meetings
            .iter()
            .filter(|(_, meeting)| {
                meeting.hints.background_reason().is_none()
                    && meeting.last_started.is_none_or(|last| {
                        now.saturating_duration_since(last) >= BACKGROUND_INTERVAL
                    })
            })
            .filter_map(|(uid, meeting)| {
                meeting
                    .pending
                    .as_ref()
                    .map(|pending| (uid.clone(), pending.ticket))
            })
            .min_by_key(|(_, ticket)| *ticket)
            .map(|(uid, _)| uid)?;
        let pending = state.meetings.get_mut(&uid)?.pending.take()?;
        let admission = self.admission(
            pending.snapshot.identity,
            pending.snapshot.deadline,
            InferenceCancellation::new(),
            InferencePriority::BackgroundAnalysis,
            &mut state,
        );
        Some(ReadyMeetingAnalysis {
            payload: pending.snapshot.payload,
            admission,
        })
    }

    pub fn cancel(&self, uid: &str) {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(meeting) = state.meetings.get_mut(uid) {
            meeting.pending = None;
            if let Some(active) = &meeting.active {
                active.cancellation.cancel();
            }
            meeting.status.state = MeetingAnalysisState::Cancelled;
            meeting.status.reason_code = "cancelled".into();
            meeting.status.revision += 1;
        }
    }

    pub fn update_resources(
        &self,
        uid: &str,
        hints: MeetingAnalysisResourceHints,
    ) -> Result<(), MeetingAnalysisScheduleError> {
        let identity = self
            .inner
            .state
            .lock()
            .unwrap()
            .meetings
            .get(uid)
            .map(|meeting| meeting.identity.clone())
            .ok_or(MeetingAnalysisScheduleError::Unavailable)?;
        self.update_context(identity, hints)
    }

    pub fn invalidate_source(&self, uid: &str) {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(meeting) = state.meetings.get_mut(uid) {
            meeting.identity.source_epoch = meeting.identity.source_epoch.wrapping_add(1);
            meeting.pending = None;
            if let Some(active) = &meeting.active {
                active.cancellation.cancel();
            }
            meeting.status.state = MeetingAnalysisState::Cancelled;
            meeting.status.reason_code = "source_changed".into();
            meeting.status.revision += 1;
        }
    }

    pub fn status(&self, uid: &str) -> MeetingAnalysisStatus {
        let state = self.inner.state.lock().unwrap();
        let Some(meeting) = state.meetings.get(uid) else {
            return empty_status();
        };
        let mut status = meeting.status.clone();
        status.active = meeting.active.is_some();
        status.pending = meeting.pending.is_some();
        status.physical_running = meeting
            .active
            .as_ref()
            .is_some_and(|active| active.physical > 0);
        if meeting
            .active
            .as_ref()
            .is_some_and(|active| active.cancellation.is_cancelled())
        {
            status.state = MeetingAnalysisState::Cancelled;
            status.reason_code = "cancelled".into();
        }
        status
    }
}

fn empty_status() -> MeetingAnalysisStatus {
    MeetingAnalysisStatus {
        state: MeetingAnalysisState::NotChecked,
        reason_code: "not_checked".into(),
        active: false,
        pending: false,
        physical_running: false,
        revision: 0,
        started_batches: 0,
        replaced_snapshots: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InferenceProtocol, MeetingActualInference, MeetingAnalysisUseCase,
        MeetingConfiguredInference, MeetingQualificationRequest,
    };

    fn qualified(use_case: MeetingAnalysisUseCase) -> QualifiedMeetingAnalysis {
        let request = MeetingQualificationRequest {
            use_case,
            language_policy: "nl".into(),
            task: "meeting_suggestions_v1".into(),
            configured: MeetingConfiguredInference {
                protocol: InferenceProtocol::LocalDecider,
                endpoint: "http://127.0.0.1:8080/v1/decisions".into(),
                model: "owned-model".into(),
                upstream: None,
                expected_model: None,
                max_input_bytes: 16_000,
                timeout_secs: 30,
            },
            prompt_sha256: "1".repeat(64),
            schema_sha256: "2".repeat(64),
            label_order_sha256: "3".repeat(64),
            grouping: "one_batch".into(),
            preprocessing_sha256: "4".repeat(64),
            resource_profile_sha256: "5".repeat(64),
        };
        crate::meeting_qualification::test_qualification_for(
            &request,
            MeetingActualInference {
                model: "owned-model".into(),
                revision: "v1".into(),
                provider: "owned-local-worker".into(),
                artifact_sha256: "6".repeat(64),
            },
        )
    }

    fn identity(qualification: &QualifiedMeetingAnalysis) -> MeetingAnalysisIdentity {
        MeetingAnalysisIdentity {
            meeting_uid: "owned-meeting".into(),
            session_id: "owned-session".into(),
            source_epoch: 1,
            source_fingerprint: "7".repeat(64),
            included_through_ms: 10_000,
            processing_fingerprint: qualification.context_fingerprint(),
            credential_epoch: 1,
            account_epoch: 1,
        }
    }

    fn hints() -> MeetingAnalysisResourceHints {
        MeetingAnalysisResourceHints {
            enabled: true,
            workspace_visible: true,
            capture_healthy: true,
            transcription_caught_up: true,
            speech_busy: false,
            foreground_busy: false,
        }
    }

    fn queue<T: Send + 'static>() -> MeetingAnalysisCoordinator<T> {
        MeetingAnalysisCoordinator::with_resources(
            Arc::new(Instant::now),
            Arc::new(PhysicalInferencePool::default()),
        )
    }

    fn until(mut check: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !check() {
            assert!(
                Instant::now() < end,
                "owned worker did not reach the expected boundary"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn one_optional_worker_preserves_a_foreground_slot() {
        let pool = Arc::new(PhysicalInferencePool::default());
        let first = pool
            .try_acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert!(
            pool.try_acquire(
                InferencePriority::BackgroundAnalysis,
                &InferenceCancellation::new()
            )
            .is_none()
        );
        let foreground = pool
            .try_acquire(
                InferencePriority::ForegroundQuestion,
                &InferenceCancellation::new(),
            )
            .unwrap();
        assert!(
            pool.try_acquire(
                InferencePriority::ForegroundQuestion,
                &InferenceCancellation::new()
            )
            .is_none()
        );
        drop(first);
        drop(foreground);
    }

    #[test]
    fn logical_cancellation_does_not_free_a_physical_optional_slot() {
        let pool = Arc::new(PhysicalInferencePool::default());
        let cancelled = InferenceCancellation::new();
        let physical = pool
            .try_acquire(InferencePriority::BackgroundAnalysis, &cancelled)
            .unwrap();
        cancelled.cancel();
        assert!(
            pool.try_acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new()
            )
            .is_none()
        );
        drop(physical);
        assert!(
            pool.try_acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new()
            )
            .is_some()
        );
    }

    #[test]
    fn background_snapshots_coalesce_without_starting_unqualified_work() {
        let queue = queue();
        let proof = qualified(MeetingAnalysisUseCase::BackgroundLiveSuggestions);
        let now = Instant::now();
        let context = identity(&proof);
        queue.update_context(context.clone(), hints()).unwrap();
        assert_eq!(
            queue.offer_background(
                ScheduledMeetingSnapshot {
                    identity: context.clone(),
                    payload: 1,
                    deadline: now + Duration::from_secs(90)
                },
                None,
                now
            ),
            Err(MeetingAnalysisScheduleError::Unqualified)
        );
        for value in 2..=10 {
            queue
                .offer_background(
                    ScheduledMeetingSnapshot {
                        identity: context.clone(),
                        payload: value,
                        deadline: now + Duration::from_secs(90),
                    },
                    Some(&proof),
                    now,
                )
                .unwrap();
        }
        assert!(queue.status(&context.meeting_uid).pending);
        assert_eq!(queue.status(&context.meeting_uid).replaced_snapshots, 8);
        let ready = queue.take_background(now).unwrap();
        assert_eq!(ready.payload, 10);
        assert!(queue.take_background(now).is_none());
    }

    #[test]
    fn context_replacement_cancels_manual_work_but_appending_does_not() {
        let queue = queue::<()>();
        let proof = qualified(MeetingAnalysisUseCase::ManualFinishedSuggestions);
        let mut context = identity(&proof);
        let now = Instant::now();
        queue.update_context(context.clone(), hints()).unwrap();
        let active = queue
            .begin_manual(
                context.clone(),
                now + Duration::from_secs(30),
                InferenceCancellation::new(),
                &proof,
            )
            .unwrap();
        context.source_fingerprint = "8".repeat(64);
        context.included_through_ms += 5_000;
        queue.update_context(context.clone(), hints()).unwrap();
        assert!(!active.cancellation().is_cancelled());
        context.source_epoch += 1;
        queue.update_context(context, hints()).unwrap();
        assert!(active.cancellation().is_cancelled());
    }

    #[test]
    fn background_minute_cadence_uses_actual_start_and_never_catches_up() {
        let now = Instant::now();
        let clock = Arc::new(Mutex::new(now));
        let captured = clock.clone();
        let queue = MeetingAnalysisCoordinator::with_resources(
            Arc::new(move || *captured.lock().unwrap()),
            Arc::new(PhysicalInferencePool::default()),
        );
        let proof = qualified(MeetingAnalysisUseCase::BackgroundLiveSuggestions);
        let context = identity(&proof);
        queue.update_context(context.clone(), hints()).unwrap();
        let offer = |value| {
            queue
                .offer_background(
                    ScheduledMeetingSnapshot {
                        identity: context.clone(),
                        payload: value,
                        deadline: now + Duration::from_secs(600),
                    },
                    Some(&proof),
                    *clock.lock().unwrap(),
                )
                .unwrap()
        };
        offer(1);
        let ready = queue.take_background(now).unwrap();
        *clock.lock().unwrap() = now + Duration::from_secs(10);
        ready.admission.observer.starting().unwrap();
        ready
            .admission
            .observer
            .finished(InferenceWorkerOutcome::Completed);
        drop(ready);
        offer(2);
        assert!(
            queue
                .take_background(now + Duration::from_secs(69))
                .is_none()
        );
        *clock.lock().unwrap() = now + Duration::from_secs(70);
        let next = queue.take_background(*clock.lock().unwrap()).unwrap();
        assert_eq!(next.payload, 2);
        next.admission.observer.starting().unwrap();
        for value in 3..=20 {
            offer(value);
        }
        next.admission
            .observer
            .finished(InferenceWorkerOutcome::Completed);
        drop(next);
        *clock.lock().unwrap() = now + Duration::from_secs(300);
        let newest = queue.take_background(*clock.lock().unwrap()).unwrap();
        assert_eq!(newest.payload, 20);
        newest.admission.observer.starting().unwrap();
        newest
            .admission
            .observer
            .finished(InferenceWorkerOutcome::Completed);
        drop(newest);
        assert_eq!(queue.status(&context.meeting_uid).started_batches, 3);
        assert!(queue.take_background(*clock.lock().unwrap()).is_none());
    }

    #[test]
    fn paused_or_hidden_background_drops_pending_without_cancelling_manual_work() {
        let proof = qualified(MeetingAnalysisUseCase::BackgroundLiveSuggestions);
        for change in 0..6 {
            let queue = queue();
            let context = identity(&proof);
            let now = Instant::now();
            queue.update_context(context.clone(), hints()).unwrap();
            queue
                .offer_background(
                    ScheduledMeetingSnapshot {
                        identity: context.clone(),
                        payload: (),
                        deadline: now + Duration::from_secs(30),
                    },
                    Some(&proof),
                    now,
                )
                .unwrap();
            let active = queue.take_background(now).unwrap();
            queue
                .offer_background(
                    ScheduledMeetingSnapshot {
                        identity: context.clone(),
                        payload: (),
                        deadline: now + Duration::from_secs(30),
                    },
                    Some(&proof),
                    now,
                )
                .unwrap();
            let mut paused = hints();
            match change {
                0 => paused.enabled = false,
                1 => paused.workspace_visible = false,
                2 => paused.capture_healthy = false,
                3 => paused.transcription_caught_up = false,
                4 => paused.speech_busy = true,
                _ => paused.foreground_busy = true,
            }
            queue
                .update_resources(&context.meeting_uid, paused)
                .unwrap();
            assert!(active.admission.cancellation().is_cancelled());
            assert!(!queue.status(&context.meeting_uid).pending);
            drop(active);
            assert!(queue.take_background(now).is_none());
        }
        let queue = queue::<()>();
        let proof = qualified(MeetingAnalysisUseCase::ManualFinishedSuggestions);
        let context = identity(&proof);
        queue.update_context(context.clone(), hints()).unwrap();
        let active = queue
            .begin_manual(
                context.clone(),
                Instant::now() + Duration::from_secs(30),
                InferenceCancellation::new(),
                &proof,
            )
            .unwrap();
        let mut hidden = hints();
        hidden.workspace_visible = false;
        hidden.enabled = false;
        queue
            .update_resources(&context.meeting_uid, hidden)
            .unwrap();
        assert!(active.is_current());
    }

    #[test]
    fn account_credentials_and_same_meeting_session_replacement_reject_old_completions() {
        let proof = qualified(MeetingAnalysisUseCase::ManualFinishedSuggestions);
        for change in 0..4 {
            let queue = queue::<()>();
            let mut context = identity(&proof);
            queue.update_context(context.clone(), hints()).unwrap();
            let active = queue
                .begin_manual(
                    context.clone(),
                    Instant::now() + Duration::from_secs(30),
                    InferenceCancellation::new(),
                    &proof,
                )
                .unwrap();
            match change {
                0 => context.credential_epoch += 1,
                1 => context.account_epoch += 1,
                2 => context.session_id = "replacement-session".into(),
                _ => context.processing_fingerprint = "c".repeat(64),
            }
            queue.update_context(context, hints()).unwrap();
            assert!(!active.is_current());
            assert!(active.cancellation().is_cancelled());
        }
        let queue = queue::<()>();
        let old = identity(&proof);
        queue.update_context(old.clone(), hints()).unwrap();
        let active = queue
            .begin_manual(
                old.clone(),
                Instant::now() + Duration::from_secs(30),
                InferenceCancellation::new(),
                &proof,
            )
            .unwrap();
        let mut new_meeting = old;
        new_meeting.meeting_uid = "different-meeting".into();
        new_meeting.session_id = "different-session".into();
        queue.update_context(new_meeting, hints()).unwrap();
        assert!(active.is_current());
    }

    #[test]
    fn expired_pending_snapshots_and_bounded_contexts_do_not_create_work() {
        let queue = queue();
        let proof = qualified(MeetingAnalysisUseCase::BackgroundLiveSuggestions);
        let now = Instant::now();
        let context = identity(&proof);
        queue.update_context(context.clone(), hints()).unwrap();
        queue
            .offer_background(
                ScheduledMeetingSnapshot {
                    identity: context.clone(),
                    payload: (),
                    deadline: now + Duration::from_secs(1),
                },
                Some(&proof),
                now,
            )
            .unwrap();
        assert!(
            queue
                .take_background(now + Duration::from_secs(2))
                .is_none()
        );
        assert_eq!(
            queue.status(&context.meeting_uid).reason_code,
            "deadline_expired"
        );
        for value in 0..100 {
            let mut next = context.clone();
            next.meeting_uid = format!("owned-{value}");
            queue.update_context(next, hints()).unwrap();
        }
        assert!(queue.inner.state.lock().unwrap().meetings.len() <= MAX_MEETINGS);
    }

    #[test]
    fn cancellation_keeps_the_real_worker_and_library_attachment_alive() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        struct Lease(Arc<AtomicBool>);
        impl Drop for Lease {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let pool = Arc::new(PhysicalInferencePool::default());
        let queue = Arc::new(MeetingAnalysisCoordinator::<()>::with_resources(
            Arc::new(Instant::now),
            pool.clone(),
        ));
        let proof = qualified(MeetingAnalysisUseCase::ManualFinishedSuggestions);
        let context = identity(&proof);
        queue.update_context(context.clone(), hints()).unwrap();
        let admission = queue
            .begin_manual(
                context.clone(),
                Instant::now() + Duration::from_secs(10),
                InferenceCancellation::new(),
                &proof,
            )
            .unwrap();
        let cancellation = admission.cancellation();
        let worker_cancellation = cancellation.clone();
        let released = Arc::new(AtomicBool::new(false));
        let flag = released.clone();
        let (started, seen) = mpsc::channel();
        let (release, receive) = mpsc::channel();
        let (finished, result) = mpsc::channel();
        let worker_pool = pool.clone();
        std::thread::spawn(move || {
            let _admission = admission;
            let _hold = worker_cancellation.retain_until_physical_exit(Arc::new(Lease(flag)));
            let output = crate::inference::run_cancelled_in_pool(
                &worker_pool,
                worker_cancellation,
                Instant::now() + Duration::from_secs(10),
                move |_| {
                    started.send(()).unwrap();
                    receive.recv().unwrap();
                    Ok(())
                },
            );
            finished.send(output).unwrap();
        });
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        cancellation.cancel();
        assert_eq!(
            result.recv_timeout(Duration::from_secs(5)).unwrap(),
            Err(PennelError::Cancelled)
        );
        assert!(!released.load(Ordering::Acquire));
        assert!(
            queue.status(&context.meeting_uid).active
                && queue.status(&context.meeting_uid).physical_running
        );
        assert!(
            pool.try_acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new()
            )
            .is_none()
        );
        let foreground = pool
            .try_acquire(
                InferencePriority::ForegroundQuestion,
                &InferenceCancellation::new(),
            )
            .unwrap();
        drop(foreground);
        assert!(
            queue
                .begin_manual(
                    context.clone(),
                    Instant::now() + Duration::from_secs(5),
                    InferenceCancellation::new(),
                    &proof
                )
                .is_err()
        );
        release.send(()).unwrap();
        until(|| released.load(Ordering::Acquire) && !queue.status(&context.meeting_uid).active);
        assert!(!queue.status(&context.meeting_uid).physical_running);
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn queued_priority_is_fair_and_an_expired_waiter_never_dispatches() {
        use std::sync::mpsc;
        let pool = Arc::new(PhysicalInferencePool::default());
        let first = pool
            .try_acquire(
                InferencePriority::ForegroundQuestion,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let second = pool
            .try_acquire(
                InferencePriority::ForegroundQuestion,
                &InferenceCancellation::new(),
            )
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        for (count, priority) in [
            InferencePriority::BackgroundAnalysis,
            InferencePriority::ExplicitAnalysis,
            InferencePriority::ForegroundQuestion,
        ]
        .into_iter()
        .enumerate()
        {
            let worker_pool = pool.clone();
            let sender = sender.clone();
            std::thread::spawn(move || {
                let cancellation = InferenceCancellation::new();
                let _permit = worker_pool
                    .acquire(
                        priority,
                        &cancellation,
                        Instant::now() + Duration::from_secs(5),
                    )
                    .unwrap();
                sender.send(priority).unwrap();
            });
            until(|| pool.state.lock().unwrap().waiting.len() == count + 1);
        }
        drop(first);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            InferencePriority::ForegroundQuestion
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            InferencePriority::ExplicitAnalysis
        );
        drop(second);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            InferencePriority::BackgroundAnalysis
        );
        assert!(
            pool.acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new(),
                Instant::now()
            )
            .is_err()
        );
    }

    #[test]
    fn speech_preempts_optional_work_without_freeing_its_physical_slot() {
        let pool = Arc::new(PhysicalInferencePool::default());
        let optional = InferenceCancellation::new();
        let active = pool
            .try_acquire(InferencePriority::BackgroundAnalysis, &optional)
            .unwrap();
        pool.speech_started();
        assert!(optional.is_cancelled());
        let speech = pool
            .try_acquire(InferencePriority::Speech, &InferenceCancellation::new())
            .unwrap();
        assert!(
            pool.try_acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new()
            )
            .is_none()
        );
        pool.speech_finished();
        drop(speech);
        drop(active);
        assert!(
            pool.try_acquire(
                InferencePriority::ExplicitAnalysis,
                &InferenceCancellation::new()
            )
            .is_some()
        );
    }

    #[test]
    fn a_full_optional_wait_queue_cannot_reject_foreground_admission() {
        let pool = Arc::new(PhysicalInferencePool::default());
        let pending: Vec<_> = (0..MAX_WAITING_WORKERS)
            .map(|_| InferenceCancellation::new())
            .collect();
        {
            let mut state = pool.state.lock().unwrap();
            for (index, cancellation) in pending.iter().enumerate() {
                state.waiting.push_back(WaitingWorker {
                    id: index as u64,
                    priority: InferencePriority::BackgroundAnalysis,
                    cancellation: Arc::downgrade(cancellation),
                    deadline: Instant::now() + Duration::from_secs(10),
                });
            }
            state.next_id = MAX_WAITING_WORKERS as u64;
        }
        let foreground = pool
            .acquire(
                InferencePriority::ForegroundQuestion,
                &InferenceCancellation::new(),
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(
            pending.iter().filter(|token| token.is_cancelled()).count(),
            1
        );
        assert_eq!(
            pool.state.lock().unwrap().waiting.len(),
            MAX_WAITING_WORKERS - 1
        );
        drop(foreground);
    }
}
