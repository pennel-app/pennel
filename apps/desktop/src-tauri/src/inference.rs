use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pennel_core::{
    DecisionRequest, DecisionResult, InferenceAvailability, InferenceCancellation,
    InferenceCapability, InferenceConfig, InferenceProtocol, InferenceProtocolInfo,
    InferenceVerification, ProviderAccount, ProviderKind, RerankRequest, RerankResult, Settings,
};
use tauri::{AppHandle, Emitter, State};

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
pub async fn qualified_finished_suggestion_presets(
    shared: State<'_, Arc<Shared>>,
) -> Result<serde_json::Value> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let presets = shared
            .engine
            .qualified_finished_suggestion_presets_json()
            .map_err(error)?;
        serde_json::from_str(&presets).map_err(error)
    })
    .await
    .map_err(|_| error("The inference availability could not be checked."))?
}

#[tauri::command]
pub async fn managed_meeting_review_profile(
    shared: State<'_, Arc<Shared>>,
) -> Result<serde_json::Value> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile = shared
            .engine
            .managed_meeting_review_profile_json()
            .map_err(error)?;
        serde_json::from_str(&profile).map_err(error)
    })
    .await
    .map_err(|_| error("The inference availability could not be checked."))?
}

#[tauri::command]
pub async fn managed_local_decider_status(
    shared: State<'_, Arc<Shared>>,
) -> Result<serde_json::Value> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let status = shared
            .engine
            .managed_local_decider_status_json()
            .map_err(error)?;
        serde_json::from_str(&status).map_err(error)
    })
    .await
    .map_err(|_| error("The inference availability could not be checked."))?
}

struct ManagedLocalDownloadEvents {
    app: AppHandle,
    id: String,
    last: Mutex<std::time::Instant>,
}

impl pennel_core::DownloadListener for ManagedLocalDownloadEvents {
    fn on_progress(&self, downloaded: u64, total: u64) {
        let mut last = self.last.lock().unwrap();
        if last.elapsed().as_millis() < 200 && downloaded < total {
            return;
        }
        *last = std::time::Instant::now();
        let _ = self.app.emit(
            "download-progress",
            serde_json::json!({
                "id": self.id, "downloaded": downloaded, "total": total,
            }),
        );
    }
}

#[tauri::command]
pub async fn download_managed_local_decider(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<()> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let status: serde_json::Value = serde_json::from_str(
            &shared
                .engine
                .managed_local_decider_status_json()
                .map_err(error)?,
        )
        .map_err(error)?;
        let id = status["model_id"]
            .as_str()
            .ok_or_else(|| error("The inference availability could not be checked."))?
            .to_string();
        let listener = Arc::new(ManagedLocalDownloadEvents {
            app: app.clone(),
            id: id.clone(),
            last: Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1)),
        });
        let result = shared
            .engine
            .download_managed_local_decider(listener)
            .map_err(error);
        let _ = app.emit(
            "download-done",
            serde_json::json!({"id": id, "error": result.as_ref().err()}),
        );
        result
    })
    .await
    .map_err(|_| error("The inference request could not be completed."))?
}

#[tauri::command]
pub fn cancel_managed_local_decider_download(shared: State<'_, Arc<Shared>>) {
    shared.engine.cancel_managed_local_decider_download();
}

#[tauri::command]
pub async fn remove_managed_local_decider(shared: State<'_, Arc<Shared>>) -> Result<()> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _mutation = mutation(&shared.engine);
        shared.engine.remove_managed_local_decider().map_err(error)
    })
    .await
    .map_err(|_| error("The inference request could not be completed."))?
}

fn review_config_matches(config: &InferenceConfig, value: &serde_json::Value) -> bool {
    let Ok(mut actual) = serde_json::to_value(config) else {
        return false;
    };
    actual.as_object_mut().unwrap().remove("provider_id");
    let Some(mut expected) = value.as_object().cloned() else {
        return false;
    };
    expected.remove("endpoint");
    actual == serde_json::Value::Object(expected)
}

fn review_assignment(
    settings: &mut Settings,
    expected_decision: &Option<InferenceConfig>,
    expected_provider: Option<&ProviderAccount>,
    config: &InferenceConfig,
    expected_cloud_epoch: Option<&str>,
    current_cloud_epoch: &str,
) -> Result<()> {
    if settings.decision != *expected_decision
        || config.protocol == InferenceProtocol::PennelCloud
            && (expected_cloud_epoch.is_none_or(str::is_empty)
                || expected_cloud_epoch != Some(current_cloud_epoch))
    {
        return Err(error("Review setup changed. Refresh and choose again."));
    }
    let mut next = config.clone();
    let create = if let Some(expected_provider) = expected_provider {
        if settings
            .providers
            .iter()
            .find(|provider| provider.id == config.provider_id)
            != Some(expected_provider)
            || !matches!(
                (config.protocol, expected_provider.kind),
                (InferenceProtocol::PennelCloud, ProviderKind::PennelCloud)
                    | (InferenceProtocol::LocalDecider, ProviderKind::Local)
            )
        {
            return Err(error("Review setup changed. Refresh and choose again."));
        }
        None
    } else if config.protocol == InferenceProtocol::PennelCloud
        && config.provider_id.is_empty()
        && !settings
            .providers
            .iter()
            .any(|provider| provider.kind == ProviderKind::PennelCloud)
    {
        let account =
            pennel_core::new_account(ProviderKind::PennelCloud, settings.providers.clone());
        next.provider_id = account.id.clone();
        Some(account)
    } else {
        return Err(error("Review setup changed. Refresh and choose again."));
    };
    if let Some(account) = create {
        settings.providers.push(account);
    }
    settings.decision = Some(next);
    Ok(())
}

#[tauri::command]
pub async fn set_meeting_review_configuration(
    config: InferenceConfig,
    expected_decision: Option<InferenceConfig>,
    expected_cloud_account_epoch: Option<String>,
    expected_provider: Option<ProviderAccount>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<crate::commands::AppInfo> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Capability discovery may use the network. Resolve it before the settings writer gate.
        let accepted = match config.protocol {
            InferenceProtocol::PennelCloud => {
                if expected_cloud_account_epoch.as_deref()
                    != Some(crate::cloud::account_epoch(&shared).as_str())
                {
                    return Err(error("Review setup changed. Refresh and choose again."));
                }
                let profile: serde_json::Value = serde_json::from_str(
                    &shared
                        .engine
                        .managed_meeting_review_profile_json()
                        .map_err(error)?,
                )
                .map_err(error)?;
                let presets: serde_json::Value = serde_json::from_str(
                    &shared
                        .engine
                        .qualified_finished_suggestion_presets_json()
                        .map_err(error)?,
                )
                .map_err(error)?;
                profile["available"] == true
                    && profile["profile"] == "supported_passages_v3"
                    && profile["expected_model"]
                        == config
                            .expected_model
                            .as_deref()
                            .map_or(serde_json::Value::Null, serde_json::Value::from)
                    && presets.as_array().is_some_and(|items| {
                        items.iter().any(|preset| {
                            preset["provider_kind"] == "pennel_cloud"
                                && preset["configured"]["endpoint"] == profile["endpoint"]
                                && review_config_matches(&config, &preset["configured"])
                        })
                    })
            }
            InferenceProtocol::LocalDecider => {
                let status: serde_json::Value = serde_json::from_str(
                    &shared
                        .engine
                        .managed_local_decider_status_json()
                        .map_err(error)?,
                )
                .map_err(error)?;
                status["available"] == true
                    && status["qualified"] == true
                    && status["configuration"] == serde_json::to_value(&config).map_err(error)?
            }
            _ => false,
        };
        if !accepted {
            return Err(error(
                "This Review setup is not available. Refresh and choose again.",
            ));
        }
        crate::commands::change_settings(&app, &shared, |settings| {
            review_assignment(
                settings,
                &expected_decision,
                expected_provider.as_ref(),
                &config,
                expected_cloud_account_epoch.as_deref(),
                &crate::cloud::account_epoch(&shared),
            )
        })
    })
    .await
    .map_err(|_| error("The inference request could not be completed."))?
}

#[tauri::command]
pub async fn set_meeting_suggestion_policy_checked(
    language_policy: String,
    expected_configuration_fingerprint: String,
    policy: pennel_core::SuggestionAutomation,
    fallback_language: Option<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<crate::commands::AppInfo> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .engine
            .set_meeting_suggestion_policy_checked(
                language_policy,
                expected_configuration_fingerprint,
                policy,
                fallback_language,
            )
            .map_err(error)?;
        let _ = app.emit("settings-changed", ());
        crate::commands::info(&shared)
    })
    .await
    .map_err(|_| error("The inference request could not be completed."))?
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
            .decide(
                request,
                cancellation,
                shared.keys_for_inference(InferenceCapability::Decision)?,
            )
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
            .rerank(
                request,
                cancellation,
                shared.keys_for_inference(InferenceCapability::Reranking)?,
            )
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
            .verify_inference(
                capability,
                cancellation,
                shared.keys_for_inference(capability)?,
            )
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

    fn owned_review_setup() -> (Settings, ProviderAccount, InferenceConfig) {
        let mut settings = pennel_core::default_settings();
        let account =
            pennel_core::new_account(ProviderKind::PennelCloud, settings.providers.clone());
        let mut config =
            pennel_core::new_inference_config(account.id.clone(), InferenceProtocol::PennelCloud);
        config.model = "pennel-review-v3".into();
        config.expected_model = Some("perplexity/pplx-decider-v1.2-20261016".into());
        config.max_input_bytes = 65536;
        config.timeout_secs = 90;
        settings
            .providers
            .retain(|provider| provider.id != account.id);
        settings.providers.push(account.clone());
        (settings, account, config)
    }

    #[test]
    fn scoped_review_assignment_preserves_concurrent_unrelated_settings_and_consent() {
        let (mut settings, account, config) = owned_review_setup();
        let expected = settings.decision.clone();
        settings.meeting_audio_retention_days = 17;
        settings.suggestion_language = Some("mixed".into());
        let mut before = settings.clone();
        review_assignment(
            &mut settings,
            &expected,
            Some(&account),
            &config,
            Some("owner-1"),
            "owner-1",
        )
        .unwrap();
        before.decision = Some(config);
        assert_eq!(settings, before);
    }

    #[test]
    fn scoped_review_assignment_rejects_changed_route_account_or_cloud_session() {
        for changed in 0..4 {
            let (mut settings, account, config) = owned_review_setup();
            let expected = settings.decision.clone();
            let epoch = if changed == 0 { "owner-2" } else { "owner-1" };
            match changed {
                1 => settings.decision = Some(config.clone()),
                2 => {
                    settings
                        .providers
                        .iter_mut()
                        .find(|provider| provider.id == account.id)
                        .unwrap()
                        .base_url = "https://changed.example".into()
                }
                3 => settings
                    .providers
                    .retain(|provider| provider.id != account.id),
                _ => {}
            }
            let before = settings.clone();
            assert!(
                review_assignment(
                    &mut settings,
                    &expected,
                    Some(&account),
                    &config,
                    Some("owner-1"),
                    epoch
                )
                .is_err()
            );
            assert_eq!(settings, before);
        }
    }

    #[test]
    fn a_managed_review_assignment_must_match_all_adopted_configuration_fields() {
        let (_, _, config) = owned_review_setup();
        let mut value = serde_json::to_value(&config).unwrap();
        value.as_object_mut().unwrap().remove("provider_id");
        value["endpoint"] = serde_json::json!("https://api.pennel.app/v1/decisions");
        assert!(review_config_matches(&config, &value));
        for field in [
            "model",
            "protocol",
            "expected_model",
            "upstream",
            "max_input_bytes",
            "timeout_secs",
        ] {
            let mut changed = value.clone();
            changed[field] = serde_json::json!("different");
            assert!(!review_config_matches(&config, &changed));
        }
    }

    #[test]
    fn first_cloud_review_save_creates_only_its_account_and_assignment() {
        let (_, _, mut config) = owned_review_setup();
        config.provider_id.clear();
        let mut settings = pennel_core::default_settings();
        let before = settings.clone();
        review_assignment(
            &mut settings,
            &None,
            None,
            &config,
            Some("owner-1"),
            "owner-1",
        )
        .unwrap();
        assert_eq!(settings.providers.len(), before.providers.len() + 1);
        let created = settings.providers.last().unwrap().clone();
        assert_eq!(created.kind, ProviderKind::PennelCloud);
        assert_eq!(settings.decision.as_ref().unwrap().provider_id, created.id);
        let mut expected = before;
        config.provider_id = created.id.clone();
        expected.providers.push(created);
        expected.decision = Some(config);
        assert_eq!(settings, expected);
    }

    #[test]
    fn first_cloud_review_save_rejects_a_concurrently_added_account() {
        let (mut settings, _, mut config) = owned_review_setup();
        config.provider_id.clear();
        let before = settings.clone();
        assert!(
            review_assignment(
                &mut settings,
                &None,
                None,
                &config,
                Some("owner-1"),
                "owner-1"
            )
            .is_err()
        );
        assert_eq!(settings, before);
    }

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
