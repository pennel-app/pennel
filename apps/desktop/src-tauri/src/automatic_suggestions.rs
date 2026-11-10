//! The postprocess owner invokes this once after speaker recognition and automatic summary return.
//! Availability, save/retry and retranscription paths must never schedule this helper.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pennel_core::{ApiKeys, MeetingInsightAnalysisAdmission, PennelEngine, PennelError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::state::Shared;

static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
const POLL_INTERVAL: Duration = Duration::from_millis(150);

type Result<T> = std::result::Result<T, RuntimeError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeError {
    Cancelled,
    Failed,
}
impl From<PennelError> for RuntimeError {
    fn from(error: PennelError) -> Self {
        if matches!(error, PennelError::Cancelled) {
            Self::Cancelled
        } else {
            Self::Failed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JobState {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed,
}
impl JobState {
    fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
    fn progress(self) -> ProgressState {
        match self {
            Self::Completed => ProgressState::Finished,
            Self::Cancelled => ProgressState::Cancelled,
            _ => ProgressState::Failed,
        }
    }
}

/// Only routing and job lifecycle metadata crosses the event boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Job {
    job_id: String,
    meeting_uid: String,
    revision: u64,
    state: JobState,
    physical_running: bool,
    finalization_finished: bool,
}
fn identifier(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
fn parse_job(raw: &str) -> Result<Job> {
    let job: Job = serde_json::from_str(raw).map_err(|_| RuntimeError::Failed)?;
    if !identifier(&job.job_id) || !identifier(&job.meeting_uid) {
        return Err(RuntimeError::Failed);
    }
    Ok(job)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AttemptStamp {
    transcript_digest: String,
    admitted_at: String,
    outcome: Option<String>,
    finished_at: Option<String>,
    error_code: Option<String>,
    items_found: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct DurableStamp {
    meeting_uid: String,
    revision: u64,
    automatic_attempt: Option<AttemptStamp>,
}
fn parse_durable(raw: &str, uid: &str) -> Option<DurableStamp> {
    let stamp: DurableStamp = serde_json::from_str(raw).ok()?;
    (stamp.meeting_uid == uid).then_some(stamp)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProgressState {
    Started,
    Finished,
    Failed,
    Skipped,
    Cancelled,
}
#[derive(Debug, Clone, Serialize)]
struct ProgressEvent {
    id: String,
    meeting_uid: String,
    operation_id: String,
    stage: &'static str,
    state: ProgressState,
}
#[derive(Debug, Clone, Serialize)]
enum Notice {
    Progress(ProgressEvent),
    Job(Job),
    DurableChanged { id: String, meeting_uid: String },
}
struct Operation {
    id: String,
    meeting_uid: String,
    operation_id: String,
}
impl Operation {
    fn current<B: Backend>(&mut self, backend: &B) -> bool {
        let Some(id) = backend.owner_id(&self.meeting_uid, &self.id) else {
            return false;
        };
        self.id = id;
        true
    }
    fn progress(&self, state: ProgressState) -> Notice {
        Notice::Progress(ProgressEvent {
            id: self.id.clone(),
            meeting_uid: self.meeting_uid.clone(),
            operation_id: self.operation_id.clone(),
            stage: "suggestions",
            state,
        })
    }
    fn changed(&self) -> Notice {
        Notice::DurableChanged {
            id: self.id.clone(),
            meeting_uid: self.meeting_uid.clone(),
        }
    }
}

/// Core owns qualification, admission, policy/source guards, job limits and durable attempt markers.
trait Backend {
    type Admission;
    fn admit(&self, id: &str) -> Result<Option<Self::Admission>>;
    fn keys(&self) -> ApiKeys;
    fn begin(&self, admission: Self::Admission, keys: ApiKeys) -> Result<Job>;
    fn poll(&self, job_id: &str) -> Result<Job>;
    fn cancel(&self, job_id: &str);
    fn owner_id(&self, uid: &str, previous_id: &str) -> Option<String>;
    fn durable(&self, id: &str, uid: &str) -> Option<DurableStamp>;
}
struct CoreBackend<K> {
    engine: Arc<PennelEngine>,
    read_keys: K,
}
impl<K: Fn() -> ApiKeys> Backend for CoreBackend<K> {
    type Admission = Arc<MeetingInsightAnalysisAdmission>;
    fn admit(&self, id: &str) -> Result<Option<Self::Admission>> {
        self.engine
            .clone()
            .admit_automatic_meeting_insight_analysis(id.into())
            .map_err(Into::into)
    }
    fn keys(&self) -> ApiKeys {
        (self.read_keys)()
    }
    fn begin(&self, admission: Self::Admission, keys: ApiKeys) -> Result<Job> {
        parse_job(
            &self
                .engine
                .clone()
                .begin_meeting_insight_analysis_json(admission, keys)
                .map_err(RuntimeError::from)?,
        )
    }
    fn poll(&self, job_id: &str) -> Result<Job> {
        parse_job(
            &self
                .engine
                .meeting_insight_job_json(job_id.into())
                .map_err(RuntimeError::from)?,
        )
    }
    fn cancel(&self, job_id: &str) {
        let _ = self
            .engine
            .cancel_meeting_insight_analysis_json(job_id.into());
    }
    fn owner_id(&self, uid: &str, previous_id: &str) -> Option<String> {
        if self.engine.meeting_uid(previous_id.into()).as_deref() == Some(uid) {
            Some(previous_id.into())
        } else {
            self.engine.meeting_id_for_uid(uid)
        }
    }
    fn durable(&self, id: &str, uid: &str) -> Option<DurableStamp> {
        parse_durable(&self.engine.meeting_insights_json(id.into()).ok()?, uid)
    }
}

fn refresh_durable<B: Backend>(
    backend: &B,
    operation: &mut Operation,
    previous: &mut Option<DurableStamp>,
    publish: &mut impl FnMut(Notice),
) -> bool {
    if !operation.current(backend) {
        return false;
    }
    let Some(current) = backend.durable(&operation.id, &operation.meeting_uid) else {
        return false;
    };
    let changed = previous
        .as_ref()
        .is_some_and(|previous| previous != &current);
    *previous = Some(current);
    if changed {
        publish(operation.changed());
    }
    changed
}

fn drive<B: Backend>(
    backend: &B,
    mut operation: Operation,
    stopping: impl Fn() -> bool,
    mut publish: impl FnMut(Notice),
    mut wait: impl FnMut(),
) -> ProgressState {
    if stopping() || !operation.current(backend) {
        return ProgressState::Skipped;
    }
    let mut durable = backend.durable(&operation.id, &operation.meeting_uid);
    let admission = match backend.admit(&operation.id) {
        Ok(Some(admission)) => admission,
        Ok(None) => {
            publish(operation.progress(ProgressState::Skipped));
            return ProgressState::Skipped;
        }
        Err(error) => {
            let changed = refresh_durable(backend, &mut operation, &mut durable, &mut publish);
            let state = if !changed {
                ProgressState::Skipped
            } else if error == RuntimeError::Cancelled {
                ProgressState::Cancelled
            } else {
                ProgressState::Failed
            };
            publish(operation.progress(state));
            return state;
        }
    };
    // Some means core has durably consumed this digest before any credential access or request.
    if !refresh_durable(backend, &mut operation, &mut durable, &mut publish)
        && operation.current(backend)
    {
        publish(operation.changed());
    }
    if stopping() || !operation.current(backend) {
        drop(admission);
        refresh_durable(backend, &mut operation, &mut durable, &mut publish);
        publish(operation.progress(ProgressState::Cancelled));
        return ProgressState::Cancelled;
    }
    publish(operation.progress(ProgressState::Started));
    let mut job = match backend.begin(admission, backend.keys()) {
        Ok(job) if job.meeting_uid == operation.meeting_uid => job,
        Ok(_) => {
            // No returned identifier is owned until its meeting binding has been verified.
            publish(operation.progress(ProgressState::Failed));
            return ProgressState::Failed;
        }
        Err(error) => {
            refresh_durable(backend, &mut operation, &mut durable, &mut publish);
            let state = if error == RuntimeError::Cancelled {
                ProgressState::Cancelled
            } else {
                ProgressState::Failed
            };
            publish(operation.progress(state));
            return state;
        }
    };
    let job_id = job.job_id.clone();
    let mut cancelled = false;
    let mut published = None;
    loop {
        if published.as_ref() != Some(&job) {
            if operation.current(backend) {
                publish(Notice::Job(job.clone()));
            }
            published = Some(job.clone());
        }
        if job.state.terminal() && !job.physical_running && job.finalization_finished {
            refresh_durable(backend, &mut operation, &mut durable, &mut publish);
            let state = job.state.progress();
            publish(operation.progress(state));
            return state;
        }
        wait();
        if (stopping() || !operation.current(backend)) && !cancelled {
            backend.cancel(&job_id);
            cancelled = true;
        }
        match backend.poll(&job_id) {
            Ok(next)
                if next.job_id == job_id
                    && next.meeting_uid == operation.meeting_uid
                    && next.revision >= job.revision =>
            {
                job = next
            }
            Ok(_) => {
                if !cancelled {
                    backend.cancel(&job_id);
                }
                publish(operation.progress(ProgressState::Cancelled));
                return ProgressState::Cancelled;
            }
            Err(_) => {
                if !cancelled {
                    backend.cancel(&job_id);
                }
                refresh_durable(backend, &mut operation, &mut durable, &mut publish);
                publish(operation.progress(ProgressState::Failed));
                return ProgressState::Failed;
            }
        }
    }
}

/// Staged for the postprocess owner: call exactly once after automatic summary returns, even on error.
#[allow(dead_code)] // The T6a/postprocess owner wires this entry point in its separate worktree.
pub(crate) fn run_after_summary(app: AppHandle, shared: Arc<Shared>, id: String) {
    let stopping =
        || shared.quitting.load(Ordering::SeqCst) || shared.updating.load(Ordering::SeqCst);
    if stopping() {
        return;
    }
    let Ok(_library_job) = crate::meeting_runtime::LibraryJob::new(&shared) else {
        return;
    };
    let Some(meeting_uid) = shared.engine.meeting_uid(id.clone()) else {
        return;
    };
    let operation = Operation {
        id,
        meeting_uid,
        operation_id: format!(
            "suggestions-{}",
            NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
        ),
    };
    let backend = CoreBackend {
        engine: shared.engine.clone(),
        read_keys: || shared.keys(),
    };
    drive(
        &backend,
        operation,
        stopping,
        |notice| match notice {
            Notice::Progress(event) => {
                let _ = app.emit_to("main", "meeting-postprocess", event);
            }
            Notice::Job(job) => {
                if let Ok(value) = serde_json::to_value(job) {
                    crate::insights::publish(&app, &value);
                }
            }
            Notice::DurableChanged { id, .. } => {
                let _ = app.emit("meeting-updated", id);
                crate::cloud::poke(&app);
            }
        },
        || std::thread::sleep(POLL_INTERVAL),
    );
}

fn unavailable_setup() -> String {
    pennel_core::ui_text(crate::ui::language(), "insights.operation_failed", &[])
}

fn read_availability(engine: &PennelEngine, id: &str) -> std::result::Result<Value, PennelError> {
    serde_json::from_str(&engine.meeting_automatic_insight_availability_json(id.into())?).map_err(
        |_| PennelError::InvalidResponse {
            message: unavailable_setup(),
        },
    )
}

#[tauri::command]
pub async fn automatic_insights_availability(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> std::result::Result<Value, String> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::commands::checked_meeting_dir(&shared.engine, &id)?;
        read_availability(&shared.engine, &id)
            .map_err(|error| crate::commands::localize_error_message(error.to_string()))
    })
    .await
    .map_err(|_| unavailable_setup())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    const UID: &str = "0192f3a1-7c1e-7b4e-9d55-2f0c8a6e41aa";
    const JOB: &str = "0192f3a1-7c1e-7b4e-9d55-2f0c8a6e41ab";
    const OTHER_JOB: &str = "0192f3a1-7c1e-7b4e-9d55-2f0c8a6e41ac";
    const OTHER_UID: &str = "0192f3a1-7c1e-7b4e-9d55-2f0c8a6e41ad";
    const PRIVATE: &str = "PRIVATE_TRANSCRIPT_NOTE_PROVIDER_SENTINEL";

    fn operation() -> Operation {
        Operation {
            id: "finished-meeting".into(),
            meeting_uid: UID.into(),
            operation_id: "suggestions-operation-1".into(),
        }
    }
    fn job(id: &str, uid: &str, state: &str, revision: u64, physical: bool) -> Job {
        parse_job(&json!({"job_id":id,"meeting_uid":uid,"state":state,"revision":revision,"physical_running":physical,"finalization_finished":matches!(state,"completed"|"cancelled"|"failed") && !physical,
            "items":[{"text":PRIVATE}],"notes":PRIVATE,"endpoint":PRIVATE,"error_code":PRIVATE}).to_string()).unwrap()
    }
    fn stamp(revision: u64, outcome: Option<&str>) -> DurableStamp {
        parse_durable(&json!({"meeting_uid":UID,"revision":revision,"items":[{"text":PRIVATE}],
            "automatic_attempt":outcome.map(|outcome| if outcome=="pending" { json!({"transcript_digest":"a".repeat(64),"admitted_at":"2026-11-10T10:00:00Z"}) } else { json!({"transcript_digest":"a".repeat(64),"admitted_at":"2026-11-10T10:00:00Z","outcome":outcome,"finished_at":"2026-11-10T10:01:00Z"}) })}).to_string(), UID).unwrap()
    }
    struct FakeBackend {
        trace: RefCell<Vec<String>>,
        jobs: RefCell<VecDeque<Job>>,
        owner: RefCell<Option<String>>,
        durable: RefCell<DurableStamp>,
        write_outcome: bool,
    }
    impl FakeBackend {
        fn new(jobs: Vec<Job>) -> Self {
            Self {
                trace: RefCell::new(Vec::new()),
                jobs: RefCell::new(jobs.into()),
                owner: RefCell::new(Some("finished-meeting".into())),
                durable: RefCell::new(stamp(0, None)),
                write_outcome: true,
            }
        }
        fn next(&self) -> Job {
            let job = self
                .jobs
                .borrow_mut()
                .pop_front()
                .expect("runtime must stop at the owned terminal response");
            if job.state.terminal()
                && !job.physical_running
                && job.finalization_finished
                && self.write_outcome
            {
                *self.durable.borrow_mut() = stamp(
                    if job.state == JobState::Completed {
                        1
                    } else {
                        0
                    },
                    Some(match job.state {
                        JobState::Completed => "completed",
                        JobState::Cancelled => "cancelled",
                        _ => "failed",
                    }),
                );
            }
            job
        }
    }
    impl Backend for FakeBackend {
        type Admission = ();
        fn admit(&self, _: &str) -> Result<Option<()>> {
            self.trace.borrow_mut().push("admit".into());
            *self.durable.borrow_mut() = stamp(0, Some("pending"));
            Ok(Some(()))
        }
        fn keys(&self) -> pennel_core::ApiKeys {
            self.trace.borrow_mut().push("keys".into());
            pennel_core::ApiKeys {
                by_provider: Default::default(),
            }
        }
        fn begin(&self, _: (), _: pennel_core::ApiKeys) -> Result<Job> {
            self.trace.borrow_mut().push("begin".into());
            Ok(self.next())
        }
        fn poll(&self, id: &str) -> Result<Job> {
            self.trace.borrow_mut().push(format!("poll:{id}"));
            Ok(self.next())
        }
        fn cancel(&self, id: &str) {
            self.trace.borrow_mut().push(format!("cancel:{id}"));
        }
        fn owner_id(&self, _: &str, _: &str) -> Option<String> {
            self.owner.borrow().clone()
        }
        fn durable(&self, _: &str, _: &str) -> Option<DurableStamp> {
            Some(self.durable.borrow().clone())
        }
    }

    fn finalization_job(state: &str, finished: bool, revision: u64) -> Job {
        parse_job(&json!({"job_id":JOB,"meeting_uid":UID,"state":state,"revision":revision,"physical_running":false,"finalization_finished":finished}).to_string()).unwrap()
    }

    #[test]
    fn terminal_job_waits_for_the_receipt_latch_before_final_durable_notification() {
        for state in ["completed", "cancelled", "failed"] {
            let mut backend = FakeBackend::new(vec![
                finalization_job(state, false, 2),
                finalization_job(state, true, 3),
            ]);
            backend.write_outcome = false;
            let waits = Cell::new(0);
            let notices = RefCell::new(Vec::new());
            let result = drive(
                &backend,
                operation(),
                || false,
                |notice| notices.borrow_mut().push(notice),
                || {
                    waits.set(waits.get() + 1);
                    *backend.durable.borrow_mut() =
                        stamp(if state == "completed" { 1 } else { 0 }, Some(state));
                },
            );
            assert_eq!(
                waits.get(),
                1,
                "{state}: a terminal request count is not receipt completion"
            );
            assert_eq!(
                result,
                match state {
                    "completed" => ProgressState::Finished,
                    "cancelled" => ProgressState::Cancelled,
                    _ => ProgressState::Failed,
                }
            );
            assert_eq!(
                notices
                    .borrow()
                    .iter()
                    .filter(|notice| matches!(notice, Notice::DurableChanged { .. }))
                    .count(),
                2
            );
            assert!(matches!(notices.borrow().last(), Some(Notice::Progress(_))));
        }
    }

    #[test]
    fn failed_receipt_write_completes_without_waiting_for_an_outcome_or_retrying() {
        let mut backend = FakeBackend::new(vec![
            finalization_job("failed", false, 2),
            finalization_job("failed", true, 3),
        ]);
        backend.write_outcome = false;
        let waits = Cell::new(0);
        let notices = RefCell::new(Vec::new());
        assert_eq!(
            drive(
                &backend,
                operation(),
                || false,
                |notice| notices.borrow_mut().push(notice),
                || {
                    waits.set(waits.get() + 1);
                }
            ),
            ProgressState::Failed
        );
        assert_eq!(waits.get(), 1);
        assert_eq!(
            notices
                .borrow()
                .iter()
                .filter(|notice| matches!(notice, Notice::DurableChanged { .. }))
                .count(),
            1
        );
        assert_eq!(
            backend
                .trace
                .borrow()
                .iter()
                .filter(|entry| entry.as_str() == "admit")
                .count(),
            1
        );
        assert_eq!(
            backend
                .durable
                .borrow()
                .automatic_attempt
                .as_ref()
                .unwrap()
                .outcome,
            None
        );
    }

    #[test]
    fn credentials_follow_actual_admission_and_progress_never_exposes_content() {
        let backend = FakeBackend::new(vec![
            job(JOB, UID, "running", 1, true),
            job(JOB, UID, "completed", 2, false),
        ]);
        let notices = RefCell::new(Vec::new());
        let result = drive(
            &backend,
            operation(),
            || false,
            |notice| notices.borrow_mut().push(notice),
            || {},
        );
        assert_eq!(result, ProgressState::Finished);
        assert_eq!(&backend.trace.borrow()[..3], ["admit", "keys", "begin"]);
        let notices = notices.into_inner();
        assert_eq!(
            notices
                .iter()
                .filter(|notice| matches!(notice, Notice::DurableChanged { .. }))
                .count(),
            2
        );
        assert!(!serde_json::to_string(&notices).unwrap().contains(PRIVATE));
        let progress: Vec<_> = notices
            .iter()
            .filter_map(|notice| {
                if let Notice::Progress(event) = notice {
                    Some(event.state)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(progress, [ProgressState::Started, ProgressState::Finished]);
    }

    #[test]
    fn quitting_cancels_only_the_owned_job_and_waits_for_physical_exit() {
        let backend = FakeBackend::new(vec![
            job(JOB, UID, "running", 1, true),
            job(JOB, UID, "cancelled", 2, true),
            job(JOB, UID, "cancelled", 3, false),
        ]);
        let stopping = Cell::new(false);
        let waits = Cell::new(0);
        let notices = RefCell::new(Vec::new());
        let result = drive(
            &backend,
            operation(),
            || stopping.get(),
            |notice| notices.borrow_mut().push(notice),
            || {
                waits.set(waits.get() + 1);
                stopping.set(true);
            },
        );
        assert_eq!(result, ProgressState::Cancelled);
        assert_eq!(waits.get(), 2);
        assert_eq!(
            backend
                .trace
                .borrow()
                .iter()
                .filter(|entry| entry.starts_with("cancel:"))
                .collect::<Vec<_>>(),
            vec![&format!("cancel:{JOB}")]
        );
        assert!(
            matches!(notices.borrow().last(),Some(Notice::Progress(event)) if event.state==ProgressState::Cancelled)
        );
    }

    #[test]
    fn superseded_job_response_cannot_publish_or_cancel_the_replacement() {
        let backend = FakeBackend::new(vec![
            job(JOB, UID, "running", 1, true),
            job(OTHER_JOB, OTHER_UID, "completed", 2, false),
        ]);
        let notices = RefCell::new(Vec::new());
        assert_eq!(
            drive(
                &backend,
                operation(),
                || false,
                |notice| notices.borrow_mut().push(notice),
                || {}
            ),
            ProgressState::Cancelled
        );
        assert!(backend.trace.borrow().contains(&format!("cancel:{JOB}")));
        assert!(
            !backend
                .trace
                .borrow()
                .contains(&format!("cancel:{OTHER_JOB}"))
        );
        let events = serde_json::to_string(&notices.into_inner()).unwrap();
        assert!(!events.contains(OTHER_JOB));
        assert!(!events.contains(OTHER_UID));
    }

    #[test]
    fn foreign_begin_response_never_cancels_someone_elses_job() {
        let backend = FakeBackend::new(vec![job(OTHER_JOB, OTHER_UID, "running", 1, true)]);
        let notices = RefCell::new(Vec::new());
        assert_eq!(
            drive(
                &backend,
                operation(),
                || false,
                |notice| notices.borrow_mut().push(notice),
                || {}
            ),
            ProgressState::Failed
        );
        assert!(
            !backend
                .trace
                .borrow()
                .iter()
                .any(|entry| entry.starts_with("cancel:"))
        );
        assert!(
            !serde_json::to_string(&notices.into_inner())
                .unwrap()
                .contains(OTHER_JOB)
        );
    }

    #[test]
    fn shutdown_after_admission_drops_it_before_looking_up_keys() {
        let backend = FakeBackend::new(Vec::new());
        let checks = Cell::new(0);
        assert_eq!(
            drive(
                &backend,
                operation(),
                || {
                    checks.set(checks.get() + 1);
                    checks.get() > 1
                },
                |_| {},
                || {}
            ),
            ProgressState::Cancelled
        );
        assert_eq!(*backend.trace.borrow(), vec!["admit"]);
    }

    #[test]
    fn completion_follows_the_owned_meeting_uid_after_a_folder_rename() {
        let backend = FakeBackend::new(vec![
            job(JOB, UID, "running", 1, true),
            job(JOB, UID, "completed", 2, false),
        ]);
        let notices = RefCell::new(Vec::new());
        assert_eq!(
            drive(
                &backend,
                operation(),
                || false,
                |notice| notices.borrow_mut().push(notice),
                || {
                    *backend.owner.borrow_mut() = Some("renamed-meeting".into());
                }
            ),
            ProgressState::Finished
        );
        assert!(
            matches!(notices.borrow().last(),Some(Notice::Progress(event)) if event.id=="renamed-meeting" && event.meeting_uid==UID)
        );
        assert!(
            !backend
                .trace
                .borrow()
                .iter()
                .any(|entry| entry.starts_with("cancel:"))
        );
    }

    #[test]
    fn job_progress_without_a_new_durable_revision_does_not_poke_sync_or_retry() {
        let mut backend = FakeBackend::new(vec![
            job(JOB, UID, "running", 1, true),
            job(JOB, UID, "running", 2, true),
            job(JOB, UID, "failed", 3, false),
        ]);
        backend.write_outcome = false;
        let notices = RefCell::new(Vec::new());
        assert_eq!(
            drive(
                &backend,
                operation(),
                || false,
                |notice| notices.borrow_mut().push(notice),
                || {}
            ),
            ProgressState::Failed
        );
        assert_eq!(
            notices
                .borrow()
                .iter()
                .filter(|notice| matches!(notice, Notice::DurableChanged { .. }))
                .count(),
            1
        );
        assert_eq!(
            backend
                .trace
                .borrow()
                .iter()
                .filter(|entry| entry.as_str() == "admit")
                .count(),
            1
        );
    }

    #[test]
    fn quitting_before_admission_reads_no_credentials_and_starts_no_job() {
        let backend = FakeBackend::new(Vec::new());
        assert_eq!(
            drive(&backend, operation(), || true, |_| {}, || {}),
            ProgressState::Skipped
        );
        assert!(backend.trace.borrow().is_empty());
    }

    #[test]
    fn actual_core_finished_profiles_and_unqualified_availability_never_read_keys_or_connect() {
        use pennel_core::{
            ApiKeys, InferenceProtocol, ProviderKind, SuggestionAutomation, SummaryAutomation,
        };
        let root = tempfile::tempdir().unwrap();
        let shared = crate::state::Shared::testing(root.path().to_path_buf());
        let session = shared
            .engine
            .clone()
            .start_meeting(
                "PRIVATE_TITLE".into(),
                ApiKeys {
                    by_provider: Default::default(),
                },
            )
            .unwrap();
        session.set_notes("PRIVATE_NOTES".into()).unwrap();
        session.stop().unwrap();
        let id = session.id();
        let directory = std::path::PathBuf::from(shared.engine.meeting_dir(id.clone()));
        std::fs::write(
            directory.join("transcript.md"),
            "[00:00] **Anderen:** PRIVATE_TRANSCRIPT_SENTINEL\n",
        )
        .unwrap();
        let _ = std::fs::remove_file(directory.join("transcript.meta.json"));
        assert_eq!(
            shared
                .engine
                .meeting_utterance_snapshot(id.clone())
                .unwrap()
                .utterances
                .len(),
            1
        );
        let uid = shared.engine.meeting_uid(id.clone()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut settings = shared.settings();
        let mut provider =
            pennel_core::new_account(ProviderKind::Custom, settings.providers.clone());
        provider.base_url = format!("http://{}", listener.local_addr().unwrap());
        let mut config =
            pennel_core::new_inference_config(provider.id.clone(), InferenceProtocol::Mercury);
        config.model = "unqualified-fixture".into();
        settings.providers.push(provider);
        settings.decision = Some(config);
        settings.summary_automation = SummaryAutomation::Automatic;
        settings.suggestion_language = Some("en".into());
        for policy in [
            SuggestionAutomation::Unanswered,
            SuggestionAutomation::Manual,
            SuggestionAutomation::Automatic,
        ] {
            settings.suggestion_automation = policy;
            shared.engine.update_settings(settings.clone()).unwrap();
            let backend = CoreBackend {
                engine: shared.engine.clone(),
                read_keys: || panic!("credentials must follow a real admitted run"),
            };
            let events = RefCell::new(Vec::new());
            let operation = Operation {
                id: id.clone(),
                meeting_uid: uid.clone(),
                operation_id: "test-operation".into(),
            };
            assert_eq!(
                drive(
                    &backend,
                    operation,
                    || false,
                    |notice| events.borrow_mut().push(notice),
                    || panic!("unadmitted work must not poll")
                ),
                ProgressState::Skipped
            );
            assert!(
                !events
                    .borrow()
                    .iter()
                    .any(|notice| matches!(notice, Notice::Job(_) | Notice::DurableChanged { .. }))
            );
            let status = read_availability(&shared.engine, &id).unwrap();
            assert_eq!(status["available"], false);
            assert_eq!(status["configured"], true);
            assert_eq!(status["reason_code"], "unqualified");
        }
        assert!(
            matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
        );
        let directory = std::path::PathBuf::from(shared.engine.meeting_dir(id));
        assert!(!std::fs::read_dir(directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("pennel.suggestions.automatic.")
        }));
    }
}
