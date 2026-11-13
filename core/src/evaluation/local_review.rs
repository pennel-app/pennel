//! Opt-in, invented-fixture execution through the real managed local Engine path.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::local_decider::{BundledLocalDeciderRuntime, ManagedLocalDeciderRuntime};
use crate::{
    ApiKeys, DecisionRequest, DecisionResult, InferenceCancellation, InferenceConfig,
    InferencePriority, MeetingConfiguredInference, PennelEngine, PennelError, ProviderAccount,
    ProviderKind,
};

type Attempts = Arc<Mutex<Vec<Value>>>;
type CaptureScopes = HashMap<String, Weak<Mutex<Vec<Value>>>>;
static CAPTURES: OnceLock<Mutex<CaptureScopes>> = OnceLock::new();
static NATIVE: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();

fn captures() -> &'static Mutex<CaptureScopes> {
    CAPTURES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn native_execution_observed(request_id: Option<&str>, metadata: Option<&Value>) {
    let (Some(request_id), Some(metadata)) = (request_id, metadata) else {
        return;
    };
    if captures().lock().map_or(true, |scopes| scopes.is_empty()) {
        return;
    }
    if let Ok(mut native) = NATIVE.get_or_init(|| Mutex::new(HashMap::new())).lock()
        && native.len() < 64
    {
        native.insert(request_id.into(), metadata.clone());
    }
}

/// The actual validated typed request entering the managed local scheduler.
/// Native formatter and worker sources are frozen separately; raw worker I/O is not captured.
pub(crate) struct LocalAttempt {
    rows: Attempts,
    index: usize,
}

pub(crate) fn transport_start(scope: &str, request: &DecisionRequest) -> Option<LocalAttempt> {
    let rows = captures().lock().ok()?.get(scope).and_then(Weak::upgrade)?;
    let mut values = rows.lock().ok()?;
    let index = values.len();
    let body = serde_json::to_string(request).ok()?;
    values.push(json!({"request":request,
        "request_body":body,"request_sha256":crate::evidence_revision(&body),
        "request_capture":"validated_Engine_request_before_managed_local_scheduler",
        "response_capture":"validated_production_DecisionResult",
        "raw_worker_io_available":false,"result":null,"status":"pending"}));
    drop(values);
    Some(LocalAttempt { rows, index })
}

impl LocalAttempt {
    pub(crate) fn completed(&self, result: &Result<DecisionResult, PennelError>) {
        if let Ok(mut rows) = self.rows.lock() {
            let row = &mut rows[self.index];
            row["status"] = if result.is_ok() { "ok" } else { "failed" }.into();
            if let Ok(result) = result {
                row["result"] = json!(result);
                if let Some(id) = result.identity.request_id.as_ref()
                    && let Ok(mut native) = NATIVE.get_or_init(|| Mutex::new(HashMap::new())).lock()
                    && let Some(metadata) = native.remove(id)
                {
                    row["native_execution"] = metadata;
                }
            }
        }
    }
}

struct Job {
    id: uuid::Uuid,
    export_sha256: String,
    next_batch: usize,
    deadline: Instant,
    terminal: bool,
    prepared: crate::meeting_insight_engine::evaluation::Prepared,
    configured: MeetingConfiguredInference,
    expected_actual: Option<crate::meeting_qualification::MeetingActualInference>,
}

/// A retained Engine and native worker. Construction never loads model weights.
pub struct LocalReviewClient {
    engine: Arc<PennelEngine>,
    root: super::OwnedRoot,
    runtime: Arc<dyn ManagedLocalDeciderRuntime>,
    scope: String,
    attempts: Attempts,
    job: Option<Job>,
    run_ids: HashSet<uuid::Uuid>,
    started: Instant,
    finished: bool,
}

impl Drop for LocalReviewClient {
    fn drop(&mut self) {
        self.runtime.unload();
        captures().lock().unwrap().remove(&self.scope);
    }
}

fn checked_model(path: &Path) -> Result<PathBuf, &'static str> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "local_model_unavailable")?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != crate::local_decider::MODEL_BYTES
    {
        return Err("local_model_invalid");
    }
    path.canonicalize().map_err(|_| "local_model_unavailable")
}

fn clone_owned_model(source: &Path, destination: &Path) -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        // APFS clone creates an independent inode. Hard links would mutate the source ctime
        // and invalidate another owned worker's same-FD verification.
        let status = std::process::Command::new("/bin/cp")
            .arg("-c")
            .arg(source)
            .arg(destination)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|_| "local_model_clone_failed")?;
        if !status.success() {
            return Err("local_model_clone_failed");
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (source, destination);
        Err("local_runtime_unsupported")
    }
}

impl LocalReviewClient {
    pub fn new(bundle: PathBuf, executable: PathBuf, model: PathBuf) -> Result<Self, &'static str> {
        let model = checked_model(&model)?;
        let runtime = BundledLocalDeciderRuntime::new(bundle, executable)
            .map_err(|_| "local_runtime_unavailable")?;
        let root = super::OwnedRoot::new()?;
        let owned_models = root.0.join("models");
        std::fs::create_dir(&owned_models).map_err(|_| "fixture_io")?;
        let owned_model = owned_models.join(format!("{}.gguf", crate::local_decider::MODEL_ID));
        clone_owned_model(&model, &owned_model)?;
        checked_model(&owned_model)?;
        let engine = PennelEngine::new_preview(root.0.to_string_lossy().into_owned())
            .map_err(|_| "fixture_io")?;
        engine.set_managed_local_decider_runtime(runtime.clone());
        engine
            .managed_local_decider_inference_readiness()
            .map_err(|_| "local_runtime_unavailable")?;
        let scope = format!("evaluation-local-{}", uuid::Uuid::now_v7());
        let attempts = Arc::new(Mutex::new(Vec::new()));
        captures()
            .lock()
            .unwrap()
            .insert(scope.clone(), Arc::downgrade(&attempts));
        Ok(Self {
            engine,
            root,
            runtime,
            scope,
            attempts,
            job: None,
            run_ids: HashSet::new(),
            started: Instant::now(),
            finished: false,
        })
    }

    pub fn evaluate(&mut self, input: Value) -> Result<Value, &'static str> {
        if self.finished {
            return Err("local_client_finished");
        }
        match input["op"].as_str() {
            Some("local_review_preflight") => {
                let status = self.runtime.status();
                Ok(
                    json!({"runtime_available":status.available,"runtime_revision":status.revision,
                    "model_pin":crate::local_decider::MODEL_FINGERPRINT,"actual_loaded_model":null,
                    "preflight_model_loads":0,"provider_requests":0,"qualification":false,"production_admission":false,
                    "transport":"owned_native_worker","engine_lifecycle":"retained_across_original_jobs",
                    "artifact_preparation":"independent_owned_APFS_clone;sha256_verified_from_consumed_FD_at_first_decide"}),
                )
            }
            Some("finished_local_review_batch") => self.batch(input),
            Some("finished_local_review_finish") => {
                self.runtime.unload();
                self.finished = true;
                Ok(
                    json!({"finished":true,"native_worker_unloaded_and_reaped":true,
                    "resources":resource_usage(),"qualification":false,"production_admission":false}),
                )
            }
            _ => super::evaluate(input),
        }
    }

    fn batch(&mut self, input: Value) -> Result<Value, &'static str> {
        let preparation_started = Instant::now();
        let run = input["run_id"]
            .as_str()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .filter(|id| !id.is_nil())
            .ok_or("local_run_invalid")?;
        let index = input["batch_index"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("local_batch_invalid")?;
        let exported_sha = input["export"]["freeze_sha256"]
            .as_str()
            .or_else(|| input["export_sha256"].as_str())
            .ok_or("local_export_invalid")?
            .to_owned();
        if index == 0 {
            let exported = &input["export"];
            let prepared = super::finished::verified_prepared(exported)?;
            let configured: MeetingConfiguredInference =
                serde_json::from_value(exported["configuration"].clone())
                    .map_err(|_| "local_route_invalid")?;
            if configured != local_configuration() {
                return Err("local_route_invalid");
            }
            if self.job.as_ref().is_some_and(|job| !job.terminal) || !self.run_ids.insert(run) {
                return Err("local_duplicate_or_incomplete_run");
            }
            self.job = Some(Job {
                id: run,
                export_sha256: exported_sha.clone(),
                next_batch: 0,
                deadline: preparation_started + Duration::from_secs(configured.timeout_secs.into()),
                terminal: false,
                configured: configured.clone(),
                prepared,
                expected_actual: serde_json::from_value(exported["expected_actual"].clone())
                    .map_err(|_| "local_export_invalid")?,
            });
            let mut settings = self.engine.settings();
            settings
                .providers
                .retain(|provider| provider.id != self.scope);
            settings.providers.push(ProviderAccount {
                id: self.scope.clone(),
                kind: ProviderKind::Local,
                name: "Owned synthetic managed local Review evaluation".into(),
                base_url: String::new(),
            });
            settings.decision = Some(InferenceConfig {
                provider_id: self.scope.clone(),
                protocol: configured.protocol,
                model: configured.model,
                expected_model: configured.expected_model,
                upstream: None,
                max_input_bytes: configured.max_input_bytes,
                timeout_secs: configured.timeout_secs,
            });
            self.engine
                .update_settings(settings)
                .map_err(|_| "local_configuration_invalid")?;
        }
        let job = self.job.as_mut().ok_or("local_run_invalid")?;
        if job.id != run
            || job.export_sha256 != exported_sha
            || job.next_batch != index
            || job.terminal
        {
            return Err("local_batch_order_changed");
        }
        if Instant::now() >= job.deadline || self.started.elapsed() > Duration::from_secs(7200) {
            job.terminal = true;
            return Err("local_job_deadline_exceeded");
        }
        let request = job
            .prepared
            .batch_request(index)
            .ok_or("local_batch_invalid")?;
        let cancellation = InferenceCancellation::for_meeting_analysis(
            InferenceCancellation::new(),
            InferencePriority::ExplicitAnalysis,
            job.deadline,
            job.configured.clone(),
            self.engine.inference_epoch(),
            0,
        );
        self.attempts = Arc::new(Mutex::new(Vec::new()));
        captures()
            .lock()
            .unwrap()
            .insert(self.scope.clone(), Arc::downgrade(&self.attempts));
        let started = Instant::now();
        let result = self.engine.decide(
            request,
            cancellation,
            ApiKeys {
                by_provider: HashMap::new(),
            },
        );
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let attempts = self.attempts.lock().unwrap().clone();
        let outcome = match result {
            Ok(result) => {
                let replay = job.prepared.replay_production_result(
                    index,
                    result.clone(),
                    job.expected_actual.as_ref(),
                );
                job.next_batch += 1;
                job.terminal = job.next_batch == job.prepared.batch_count();
                json!({"status":"ok","result":result,"replay":replay})
            }
            Err(error) => {
                job.terminal = true;
                json!({"status":if matches!(error,PennelError::Cancelled) {"cancelled"} else {"local_request_failed"}})
            }
        };
        Ok(
            json!({"run_id":run.to_string(),"batch_index":index,"export_sha256":exported_sha,
            "outcome":outcome,"attempts":attempts,"elapsed_ms":elapsed_ms,
            "run_complete":job.terminal&&job.next_batch==job.prepared.batch_count(),
            "fixture_storage_owned":self.root.0.exists(),"qualification":false,"production_admission":false}),
        )
    }
}

fn local_configuration() -> MeetingConfiguredInference {
    MeetingConfiguredInference {
        protocol: crate::InferenceProtocol::LocalDecider,
        endpoint: crate::local_decider::ENDPOINT.into(),
        model: crate::local_decider::MODEL_ID.into(),
        expected_model: Some(crate::local_decider::MODEL_FINGERPRINT.into()),
        upstream: None,
        max_input_bytes: 65_536,
        timeout_secs: 90,
    }
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
fn resource_usage() -> Value {
    #[repr(C)]
    #[derive(Default)]
    struct Timeval {
        seconds: i64,
        microseconds: i32,
        padding: i32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Usage {
        user: Timeval,
        system: Timeval,
        counters: [i64; 14],
    }
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Usage) -> i32;
    }
    let mut own = Usage::default();
    let mut child = Usage::default();
    // Darwin's public 64-bit rusage layout: two timeval fields and fourteen longs.
    if unsafe { getrusage(0, &mut own) != 0 || getrusage(-1, &mut child) != 0 } {
        return Value::Null;
    }
    let cpu = |usage: &Usage| {
        (usage.user.seconds + usage.system.seconds) as f64 * 1000.0
            + f64::from(usage.user.microseconds + usage.system.microseconds) / 1000.0
    };
    json!({"cpu_time_ms":cpu(&own)+cpu(&child),"peak_rss_bytes":own.counters[0].max(child.counters[0]),
        "client_peak_rss_bytes":own.counters[0],"largest_child_peak_rss_bytes":child.counters[0],
        "aggregate_process_tree_rss_bytes":null,
        "rss_measurement":"maximum_individual_process_lifetime_high_water_mark_after_owned_worker_reap",
        "scope":"owned_Rust_Engine_and_waited_native_worker"})
}

#[cfg(not(all(target_os = "macos", target_pointer_width = "64")))]
fn resource_usage() -> Value {
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_preflight_rejects_missing_small_and_symlink_artifacts_without_loading() {
        let dir = tempfile::tempdir().unwrap();
        assert!(checked_model(&dir.path().join("missing")).is_err());
        let small = dir.path().join("small.gguf");
        std::fs::write(&small, b"owned fixture; no weights").unwrap();
        assert!(checked_model(&small).is_err());
        #[cfg(unix)]
        {
            let link = dir.path().join("linked.gguf");
            std::os::unix::fs::symlink(&small, &link).unwrap();
            assert!(checked_model(&link).is_err());
        }
    }

    #[test]
    fn trace_scopes_capture_only_the_actual_owned_typed_attempt() {
        let scope = uuid::Uuid::now_v7().to_string();
        let request = DecisionRequest {
            state: "Owned invented fixture".into(),
            questions: vec![],
        };
        assert!(transport_start(&scope, &request).is_none());
        let rows = Arc::new(Mutex::new(Vec::new()));
        captures()
            .lock()
            .unwrap()
            .insert(scope.clone(), Arc::downgrade(&rows));
        let attempt = transport_start(&scope, &request).unwrap();
        attempt.completed(&Err(PennelError::Cancelled));
        assert_eq!(rows.lock().unwrap()[0]["request"], json!(request));
        assert_eq!(rows.lock().unwrap()[0]["status"], "failed");
        assert_eq!(rows.lock().unwrap()[0]["raw_worker_io_available"], false);
        captures().lock().unwrap().remove(&scope);
    }

    #[test]
    fn resource_snapshot_has_real_nonzero_process_usage_without_inference() {
        #[cfg(all(target_os = "macos", target_pointer_width = "64"))]
        {
            let usage = resource_usage();
            assert!(usage["cpu_time_ms"].as_f64().unwrap() > 0.0);
            assert!(usage["client_peak_rss_bytes"].as_i64().unwrap() > 0);
            assert!(usage["aggregate_process_tree_rss_bytes"].is_null());
        }
    }
}
