use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pennel_core::{
    DecisionRequest, DecisionResult, InferenceAvailability, InferenceCancellation,
    InferenceCapability, InferenceProtocolInfo, InferenceVerification, RerankRequest, RerankResult,
};
use tauri::State;

use crate::state::Shared;

type Result<T> = std::result::Result<T, String>;

#[derive(Default)]
struct JobState {
    running: HashMap<String, Arc<InferenceCancellation>>,
    mutation_depth: usize,
}

#[derive(Default)]
struct Jobs(Mutex<JobState>);

struct Job {
    id: String,
    cancellation: Arc<InferenceCancellation>,
    owner: Arc<Jobs>,
}

#[must_use = "Keep this guard alive until the connection update and publication finish"]
pub(crate) struct JobMutation {
    owner: Arc<Jobs>,
    core_mutation: Option<Arc<pennel_core::InferenceMutation>>,
}

impl Jobs {
    fn begin(self: &Arc<Self>, id: String) -> Result<Job> {
        if id.is_empty()
            || id.len() > 96
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            return Err("Invalid inference request ID.".into());
        }
        let mut state = self.0.lock().unwrap();
        if state.mutation_depth > 0 {
            return Err(
                "The connection is being updated. Try again when the update finishes.".into(),
            );
        }
        if state.running.contains_key(&id) {
            return Err("This inference request is already running.".into());
        }
        if state.running.len() >= 2 {
            return Err(
                "Two inference requests are already running. Try again when one finishes.".into(),
            );
        }
        let cancellation = InferenceCancellation::new();
        state.running.insert(id.clone(), cancellation.clone());
        Ok(Job {
            id,
            cancellation,
            owner: self.clone(),
        })
    }

    fn cancel(&self, id: &str) -> bool {
        let state = self.0.lock().unwrap();
        if let Some(cancellation) = state.running.get(id) {
            cancellation.cancel();
            true
        } else {
            false
        }
    }

    fn mutation(self: &Arc<Self>) -> JobMutation {
        let mut state = self.0.lock().unwrap();
        state.mutation_depth += 1;
        for cancellation in state.running.values() {
            cancellation.cancel();
        }
        JobMutation {
            owner: self.clone(),
            core_mutation: None,
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let mut state = self.owner.0.lock().unwrap();
        if state
            .running
            .get(&self.id)
            .is_some_and(|value| Arc::ptr_eq(value, &self.cancellation))
        {
            state.running.remove(&self.id);
        }
    }
}

impl Drop for JobMutation {
    fn drop(&mut self) {
        if let Some(core_mutation) = self.core_mutation.take() {
            core_mutation.release();
        }
        self.owner.0.lock().unwrap().mutation_depth -= 1;
    }
}

fn jobs() -> &'static Arc<Jobs> {
    static JOBS: OnceLock<Arc<Jobs>> = OnceLock::new();
    JOBS.get_or_init(|| Arc::new(Jobs::default()))
}

pub(crate) fn mutation(engine: &Arc<pennel_core::PennelEngine>) -> JobMutation {
    let mut guard = jobs().mutation();
    guard.core_mutation = Some(engine.begin_inference_mutation());
    guard
}

fn error(value: impl std::fmt::Display) -> String {
    crate::commands::localize_error_message(value.to_string())
}

async fn run<T: Send + 'static>(
    request_id: String,
    work: impl FnOnce(Arc<InferenceCancellation>) -> Result<T> + Send + 'static,
) -> Result<T> {
    let job = jobs().begin(request_id).map_err(error)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = work(job.cancellation.clone());
        drop(job);
        result
    })
    .await
    .map_err(|_| error("The inference request could not be completed."))?
}

#[tauri::command]
pub fn inference_protocols() -> Vec<InferenceProtocolInfo> {
    pennel_core::inference_protocols()
}

#[tauri::command]
pub async fn inference_availability(
    shared: State<'_, Arc<Shared>>,
    capability: InferenceCapability,
) -> Result<InferenceAvailability> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.engine.inference_availability(capability))
        .await
        .map_err(|_| error("The inference availability could not be checked."))
}

#[tauri::command]
pub fn cleanup_verification_availability(
    shared: State<'_, Arc<Shared>>,
) -> pennel_core::CleanupVerificationAvailability {
    shared.engine.cleanup_verification_availability()
}

#[tauri::command]
pub async fn inference_decide(
    shared: State<'_, Arc<Shared>>,
    request: DecisionRequest,
    request_id: String,
) -> Result<DecisionResult> {
    let shared = shared.inner().clone();
    run(request_id, move |cancellation| {
        shared
            .engine
            .decide(request, cancellation, shared.keys())
            .map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn inference_rerank(
    shared: State<'_, Arc<Shared>>,
    request: RerankRequest,
    request_id: String,
) -> Result<RerankResult> {
    let shared = shared.inner().clone();
    run(request_id, move |cancellation| {
        shared
            .engine
            .rerank(request, cancellation, shared.keys())
            .map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn verify_inference(
    shared: State<'_, Arc<Shared>>,
    capability: InferenceCapability,
    request_id: String,
) -> Result<InferenceVerification> {
    let shared = shared.inner().clone();
    run(request_id, move |cancellation| {
        shared
            .engine
            .verify_inference(capability, cancellation, shared.keys())
            .map_err(error)
    })
    .await
}

#[tauri::command]
pub fn cancel_inference(request_id: String) -> bool {
    jobs().cancel(&request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoPreview;
    impl pennel_core::DictationPreviewObserver for NoPreview {
        fn changed(&self, _update: pennel_core::DictationPreviewUpdate) {}
    }

    #[test]
    fn native_credential_mutations_invalidate_recording_verification_without_changing_destination()
    {
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let mut settings = shared.settings();
        settings.mode = pennel_core::DictationMode::Clean;
        settings.cleanup_verification_enabled = true;
        let mut provider = pennel_core::new_account(
            pennel_core::ProviderKind::Custom,
            settings.providers.clone(),
        );
        provider.base_url = "https://frozen.example/v1".into();
        let mut connection = pennel_core::new_inference_config(
            provider.id.clone(),
            pennel_core::InferenceProtocol::Mercury,
        );
        connection.model = "synthetic-verifier".into();
        settings.providers.push(provider.clone());
        settings.decision = Some(connection);
        shared.engine.update_settings(settings).unwrap();
        let session = shared
            .engine
            .clone()
            .begin_dictation_session(Arc::new(NoPreview), false);
        let frozen = session.cleanup_verification_availability();
        assert_eq!(frozen.reason_code, "unqualified");
        {
            let _mutation = mutation(&shared.engine);
            shared
                .secrets
                .set(&provider.id, "synthetic-replacement-key")
                .unwrap();
            let during = shared.engine.cleanup_verification_availability();
            assert!(!during.available);
            assert_ne!(during.reason_code, "unqualified");
        }
        let invalidated = session.cleanup_verification_availability();
        assert_eq!(invalidated.reason_code, "configuration_changed");
        assert_eq!(invalidated.endpoint, frozen.endpoint);
        assert_eq!(invalidated.provider_id, frozen.provider_id);
        assert_eq!(
            shared
                .engine
                .cleanup_verification_availability()
                .reason_code,
            "unqualified"
        );
        session.cancel();
    }

    #[test]
    fn jobs_are_bounded_cancelled_and_release_their_owned_slots() {
        let jobs = Arc::new(Jobs::default());
        let first = jobs.begin("one".into()).unwrap();
        let second = jobs.begin("two".into()).unwrap();
        assert!(jobs.begin("three".into()).is_err());
        assert!(jobs.begin("one".into()).is_err());
        assert!(jobs.cancel("one"));
        assert!(first.cancellation.is_cancelled());
        assert!(!second.cancellation.is_cancelled());
        drop(first);
        assert!(!jobs.cancel("one"));
        assert!(jobs.begin("three".into()).is_ok());
        drop(second);
        assert!(jobs.begin("one".into()).is_ok());
    }

    #[test]
    fn invalid_request_ids_never_claim_a_job_slot() {
        let jobs = Arc::new(Jobs::default());
        for id in ["", "bad id", "../../other"] {
            assert!(jobs.begin(id.into()).is_err());
        }
        assert!(jobs.begin("a".repeat(97)).is_err());
        assert!(jobs.0.lock().unwrap().running.is_empty());
    }

    #[test]
    fn connection_mutation_cancels_every_queued_job_and_keeps_admission_closed() {
        let jobs = Arc::new(Jobs::default());
        let first = jobs.begin("first".into()).unwrap();
        let second = jobs.begin("second".into()).unwrap();
        let mutation = jobs.mutation();
        assert!(first.cancellation.is_cancelled());
        assert!(second.cancellation.is_cancelled());
        assert!(jobs.begin("third".into()).is_err());
        drop(first);
        drop(second);
        assert!(jobs.begin("third".into()).is_err());
        drop(mutation);
        assert!(
            !jobs
                .begin("third".into())
                .unwrap()
                .cancellation
                .is_cancelled()
        );
    }

    #[test]
    fn admission_stays_closed_through_a_suspended_connection_mutation() {
        let jobs = Arc::new(Jobs::default());
        let updated = jobs.clone();
        let (entered, await_entered) = std::sync::mpsc::channel();
        let (release, await_release) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _mutation = updated.mutation();
            entered.send(()).unwrap();
            await_release.recv().unwrap();
        });
        await_entered.recv().unwrap();
        assert!(jobs.begin("during-update".into()).is_err());
        release.send(()).unwrap();
        thread.join().unwrap();
        assert!(jobs.begin("after-update".into()).is_ok());
    }

    #[test]
    fn nested_mutations_and_early_errors_release_only_their_own_gates() {
        let jobs = Arc::new(Jobs::default());
        let outer = jobs.mutation();
        let inner = jobs.mutation();
        drop(outer);
        assert!(jobs.begin("nested".into()).is_err());
        drop(inner);
        assert!(jobs.begin("released".into()).is_ok());

        fn failed_update(jobs: &Arc<Jobs>) -> std::result::Result<(), &'static str> {
            let _mutation = jobs.mutation();
            Err("Synthetic storage failure")
        }
        assert!(failed_update(&jobs).is_err());
        assert!(jobs.begin("after-error".into()).is_ok());
        assert_eq!(jobs.0.lock().unwrap().mutation_depth, 0);
    }
}
