//! Local presentation metadata. These events carry no source text or provider replies.
use crate::state::Shared;
use pennel_core::PennelError;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProgressState {
    Started,
    Finished,
    Failed,
    Skipped,
    Cancelled,
}
#[derive(Serialize)]
struct ProgressEvent {
    id: String,
    meeting_uid: String,
    operation_id: String,
    stage: &'static str,
    state: ProgressState,
}
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
fn operation_id() -> String {
    format!("summary-{}", NEXT_OPERATION.fetch_add(1, Ordering::Relaxed))
}
fn outcome<T>(result: &Result<T, PennelError>) -> ProgressState {
    match result {
        Ok(_) => ProgressState::Finished,
        Err(PennelError::Cancelled) => ProgressState::Cancelled,
        Err(_) => ProgressState::Failed,
    }
}
pub(crate) struct SummaryProgress {
    app: AppHandle,
    event: ProgressEvent,
    finished: bool,
}
impl SummaryProgress {
    pub(crate) fn start(app: &AppHandle, shared: &Shared, id: &str) -> Option<Self> {
        let meeting_uid = shared.engine.meeting_uid(id.to_owned())?;
        let progress = Self {
            app: app.clone(),
            event: ProgressEvent {
                id: id.to_owned(),
                meeting_uid,
                operation_id: operation_id(),
                stage: "summary",
                state: ProgressState::Started,
            },
            finished: false,
        };
        progress.emit();
        Some(progress)
    }
    fn emit(&self) {
        let _ = self.app.emit_to("main", "meeting-postprocess", &self.event);
    }
    pub(crate) fn finish(mut self, state: ProgressState) {
        self.event.state = state;
        self.finished = true;
        self.emit();
    }
    pub(crate) fn complete<T>(self, result: &Result<T, PennelError>) {
        self.finish(outcome(result));
    }
}
impl Drop for SummaryProgress {
    fn drop(&mut self) {
        if !self.finished {
            self.event.state = ProgressState::Failed;
            self.emit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_identity_is_unique_within_the_host() {
        assert_ne!(operation_id(), operation_id());
    }
    #[test]
    fn completion_retains_cancelled_or_failed_outcomes_without_error_content() {
        assert_eq!(
            outcome(&Err::<(), _>(PennelError::Cancelled)),
            ProgressState::Cancelled
        );
        let state = outcome(&Err::<(), _>(PennelError::Storage {
            message: "PRIVATE_PROVIDER_REPLY".into(),
        }));
        assert_eq!(state, ProgressState::Failed);
        let event = ProgressEvent {
            id: "safe-id".into(),
            meeting_uid: "uid".into(),
            operation_id: operation_id(),
            stage: "summary",
            state,
        };
        let json = serde_json::to_value(event).unwrap();
        assert!(!json.to_string().contains("PRIVATE"));
        assert_eq!(
            json.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["id", "meeting_uid", "operation_id", "stage", "state"]
        );
    }
    #[test]
    fn started_and_skipped_are_explicit_metadata_states() {
        for state in [ProgressState::Started, ProgressState::Skipped] {
            let event = ProgressEvent {
                id: "safe".into(),
                meeting_uid: "uid".into(),
                operation_id: operation_id(),
                stage: "summary",
                state,
            };
            let json = serde_json::to_value(event).unwrap();
            assert_eq!(json["stage"], "summary");
            assert_eq!(json.as_object().unwrap().len(), 5);
        }
    }
}
