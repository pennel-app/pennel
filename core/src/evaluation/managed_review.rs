//! Explicit synthetic execution through the shipped Engine and managed transport.
//! Session secrets never enter the JSON-lines request/response protocol.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::cloud::{SecretStore, account::Session};
use crate::{
    ApiKeys, InferenceCancellation, InferenceConfig, InferencePriority, InferenceProtocol,
    MeetingConfiguredInference, PennelEngine, PennelError, ProviderAccount, ProviderKind,
};

const SESSION_MAX_BYTES: usize = 64 * 1024;
const ACCESS_MARGIN_SECS: u64 = 60;
type Attempts = Arc<Mutex<Vec<Value>>>;
type CaptureScopes = HashMap<String, Weak<Mutex<Vec<Value>>>>;
static CAPTURES: OnceLock<Mutex<CaptureScopes>> = OnceLock::new();

fn captures() -> &'static Mutex<CaptureScopes> {
    CAPTURES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One actual HTTP attempt. Authorization and refresh traffic are deliberately unavailable.
pub(crate) struct TransportAttempt {
    rows: Attempts,
    index: usize,
}

pub(crate) fn transport_start(
    scope: Option<&str>,
    endpoint: &str,
    body: Option<&[u8]>,
) -> Option<TransportAttempt> {
    let rows = captures()
        .lock()
        .ok()?
        .get(scope?)
        .and_then(Weak::upgrade)?;
    let body = std::str::from_utf8(body?).ok()?;
    let mut values = rows.lock().ok()?;
    let index = values.len();
    values.push(json!({"endpoint":endpoint,"wire_body":body,
        "wire_body_sha256":crate::meeting_utterances::digest(body.as_bytes()),
        "http_status":null,"response":null,"response_capture":"parsed_service_json_before_decision_decoder",
        "raw_http_bytes_available":false}));
    drop(values);
    Some(TransportAttempt { rows, index })
}

impl TransportAttempt {
    pub(crate) fn completed(&self, result: &Result<(u16, Value), PennelError>) {
        let Ok((status, response)) = result else {
            return;
        };
        let mut sanitized = serde_json::Map::new();
        for key in [
            "schema_version",
            "request_id",
            "profile",
            "model",
            "provider",
            "model_revision",
            "id",
            "answers",
        ] {
            if let Some(value) = response.get(key) {
                sanitized.insert(key.into(), value.clone());
            }
        }
        if let Some(usage) = response.get("usage").and_then(Value::as_object) {
            sanitized.insert(
                "usage".into(),
                Value::Object(
                    usage
                        .iter()
                        .filter(|(key, _)| {
                            matches!(
                                key.as_str(),
                                "input_tokens"
                                    | "output_tokens"
                                    | "prompt_tokens"
                                    | "completion_tokens"
                                    | "total_tokens"
                                    | "cost"
                                    | "search_units"
                            )
                        })
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                ),
            );
        }
        if let Ok(mut rows) = self.rows.lock() {
            rows[self.index]["http_status"] = json!(status);
            rows[self.index]["response"] = Value::Object(sanitized);
        }
    }
}

struct EmptySecrets;
impl SecretStore for EmptySecrets {
    fn get(&self, _account: String) -> Option<String> {
        None
    }
    fn set(&self, _account: String, _value: Option<String>) {}
}

// Deserialize-only: no Debug/Serialize implementation can accidentally emit the bearer.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessSession {
    schema_version: u32,
    access_token_only: bool,
    origin: String,
    access_token: String,
    expires_at_unix_seconds: u64,
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

/// Absent from ordinary builds; requires a separately supplied, short-lived device access token.
pub struct CloudReviewClient {
    engine: Arc<PennelEngine>,
    root: super::OwnedRoot,
    origin: String,
    scope: String,
    attempts: Attempts,
    usable_until: Instant,
    provided_expiry: u64,
    started: Instant,
    job: Option<Job>,
    run_ids: HashSet<uuid::Uuid>,
}

impl Drop for CloudReviewClient {
    fn drop(&mut self) {
        captures().lock().unwrap().remove(&self.scope);
    }
}

impl CloudReviewClient {
    pub fn from_session_json(raw: &[u8]) -> Result<Self, &'static str> {
        Self::from_session(raw, false)
    }

    fn from_session(raw: &[u8], allow_loopback: bool) -> Result<Self, &'static str> {
        if raw.len() > SESSION_MAX_BYTES {
            return Err("cloud_session_invalid");
        }
        let input: AccessSession =
            serde_json::from_slice(raw).map_err(|_| "cloud_session_invalid")?;
        let official = matches!(
            input.origin.as_str(),
            "https://api.pennel.app" | "https://api-staging.pennel.app"
        );
        let local = allow_loopback
            && crate::inference::validate_origin(&input.origin).is_ok_and(|(_, local)| local);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "cloud_session_invalid")?
            .as_secs();
        let remaining = input
            .expires_at_unix_seconds
            .checked_sub(now)
            .ok_or("cloud_session_expired")?;
        if input.schema_version != 1
            || !input.access_token_only
            || !(official || local)
            || !(ACCESS_MARGIN_SECS + 1..=900).contains(&remaining)
        {
            return Err("cloud_session_invalid");
        }
        let root = super::OwnedRoot::new()?;
        let engine = PennelEngine::new_preview(root.0.to_string_lossy().into_owned())
            .map_err(|_| "fixture_io")?;
        let secrets: Arc<dyn SecretStore> = Arc::new(EmptySecrets);
        engine
            .set_secret_store(secrets.clone())
            .map_err(|_| "cloud_session_invalid")?;
        let session = Session::evaluation_access_only(
            input.origin.clone(),
            secrets,
            input.access_token,
            Duration::from_secs(remaining),
        )
        .map_err(|_| "cloud_session_invalid")?;
        engine.use_cloud_session(session);
        let scope = format!("evaluation-cloud-{}", uuid::Uuid::now_v7());
        let attempts = Arc::new(Mutex::new(Vec::new()));
        captures()
            .lock()
            .unwrap()
            .insert(scope.clone(), Arc::downgrade(&attempts));
        Ok(Self {
            engine,
            root,
            origin: input.origin,
            scope,
            attempts,
            usable_until: Instant::now() + Duration::from_secs(remaining - ACCESS_MARGIN_SECS),
            provided_expiry: input.expires_at_unix_seconds,
            started: Instant::now(),
            job: None,
            run_ids: HashSet::new(),
        })
    }

    pub fn evaluate(&mut self, input: Value) -> Result<Value, &'static str> {
        match input["op"].as_str() {
            Some("cloud_review_preflight") => self.preflight(),
            Some("finished_cloud_review_batch") => self.batch(input),
            _ => super::evaluate(input),
        }
    }

    fn preflight(&self) -> Result<Value, &'static str> {
        if Instant::now() >= self.usable_until {
            return Err("cloud_session_expired");
        }
        let profile: Value = serde_json::from_str(
            &self
                .engine
                .managed_meeting_review_profile_json()
                .map_err(|_| "cloud_capabilities_unavailable")?,
        )
        .map_err(|_| "cloud_capabilities_invalid")?;
        let session = self
            .engine
            .current_cloud_session()
            .ok_or("cloud_identity_unavailable")?;
        let generation = session.generation();
        let (user_id, device_id) = session
            .identity()
            .map_err(|_| "cloud_identity_unavailable")?;
        let valid_uuid = |id: &str| {
            uuid::Uuid::parse_str(id).is_ok_and(|value| !value.is_nil() && value.to_string() == id)
        };
        if !valid_uuid(&user_id)
            || !valid_uuid(&device_id)
            || session.generation() != generation
            || !self
                .engine
                .current_cloud_session()
                .is_some_and(|current| Arc::ptr_eq(&current, &session))
        {
            return Err("cloud_identity_invalid");
        }
        Ok(
            json!({"profile":profile,"session_mode":"access_token_only","refresh_available":false,
            "caller":{"user_id":user_id,"device_id":device_id},
            "session_origin":self.origin,"provided_access_expiry_unix_seconds":self.provided_expiry,
            "usable_remaining_seconds":self.usable_until.saturating_duration_since(Instant::now()).as_secs(),
            "provider_requests":0,"qualification":false,"production_admission":false}),
        )
    }

    fn batch(&mut self, input: Value) -> Result<Value, &'static str> {
        let preparation_started = Instant::now();
        let run = input["run_id"]
            .as_str()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .filter(|id| !id.is_nil())
            .ok_or("cloud_run_invalid")?;
        let index = input["batch_index"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("cloud_batch_invalid")?;
        let exported_sha = input["export"]["freeze_sha256"]
            .as_str()
            .or_else(|| input["export_sha256"].as_str())
            .ok_or("cloud_export_invalid")?
            .to_owned();
        if index == 0 {
            let exported = &input["export"];
            let prepared = super::finished::verified_prepared(exported)?;
            let configured: MeetingConfiguredInference =
                serde_json::from_value(exported["configuration"].clone())
                    .map_err(|_| "cloud_route_invalid")?;
            if configured.protocol != InferenceProtocol::PennelCloud
                || configured.model != crate::inference::MANAGED_REVIEW_MODEL
                || configured.endpoint != format!("{}/v1/decisions", self.origin)
            {
                return Err("cloud_route_invalid");
            }
            if self.job.as_ref().is_some_and(|job| !job.terminal) || !self.run_ids.insert(run) {
                return Err("cloud_duplicate_or_incomplete_run");
            }
            self.job = Some(Job {
                id: run,
                export_sha256: exported_sha.clone(),
                next_batch: 0,
                deadline: (preparation_started
                    + Duration::from_secs(configured.timeout_secs.into()))
                .min(self.usable_until),
                terminal: false,
                configured: configured.clone(),
                prepared,
                expected_actual: serde_json::from_value(exported["expected_actual"].clone())
                    .map_err(|_| "cloud_export_invalid")?,
            });
            let mut settings = self.engine.settings();
            settings
                .providers
                .retain(|provider| provider.id != self.scope);
            settings.providers.push(ProviderAccount {
                id: self.scope.clone(),
                kind: ProviderKind::PennelCloud,
                name: "Owned synthetic Cloud Review evaluation".into(),
                base_url: self.origin.clone(),
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
                .map_err(|_| "cloud_configuration_invalid")?;
        }
        let job = self.job.as_mut().ok_or("cloud_run_invalid")?;
        if job.id != run
            || job.export_sha256 != exported_sha
            || job.next_batch != index
            || job.terminal
        {
            return Err("cloud_batch_order_changed");
        }
        if Instant::now() >= job.deadline || self.started.elapsed() > Duration::from_secs(7200) {
            job.terminal = true;
            return Err("cloud_job_deadline_exceeded");
        }
        let request = job
            .prepared
            .batch_request(index)
            .ok_or("cloud_batch_invalid")?;
        let cancellation = InferenceCancellation::for_meeting_analysis(
            InferenceCancellation::new(),
            InferencePriority::ExplicitAnalysis,
            job.deadline,
            job.configured.clone(),
            self.engine.inference_epoch(),
            self.engine.cloud_account_epoch(),
        );
        // A late timed-out worker retains its own capture; a following explicit run cannot overwrite it.
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
        let result = match result {
            Ok(result) => {
                let replay = attempts
                    .last()
                    .and_then(|attempt| {
                        let request: Value =
                            serde_json::from_str(attempt["wire_body"].as_str()?).ok()?;
                        let response = serde_json::to_vec(&attempt["response"]).ok()?;
                        Some(job.prepared.replay_with_request_id(
                            index,
                            &response,
                            job.expected_actual.as_ref(),
                            request["request_id"].as_str(),
                        ))
                    })
                    .ok_or("cloud_response_capture_missing")?;
                job.next_batch += 1;
                job.terminal = job.next_batch == job.prepared.batch_count();
                json!({"status":"ok","result":result,"replay":replay})
            }
            Err(error) => {
                job.terminal = true;
                json!({"status":match error{PennelError::NotSignedIn=>"cloud_access_unavailable",PennelError::Cancelled=>"cancelled",_=>"cloud_request_failed"}})
            }
        };
        Ok(
            json!({"run_id":run.to_string(),"batch_index":index,"export_sha256":exported_sha,"outcome":result,
            "attempts":attempts,"elapsed_ms":elapsed_ms,"run_complete":job.terminal&&job.next_batch==job.prepared.batch_count(),
            "fixture_storage_owned":self.root.0.exists(),"qualification":false,"production_admission":false,
            "session_mode":"access_token_only","refresh_available":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEARER: &str = "owned-fake-device-access-token";

    fn session(origin: &str) -> Value {
        json!({"schema_version":1,"access_token_only":true,"origin":origin,"access_token":BEARER,
            "expires_at_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()+800})
    }

    fn exported(origin: &str) -> Value {
        super::super::finished::export(json!({"op":"finished_meeting_export",
            "fixture":{"id":"owned-cloud-develop-0","family_id":"owned-cloud-family-0","split":"development","language":"nl",
                "utterances":[{"start_ms":0,"end_ms":1000,"speaker":"others","text":"De koffie staat op de tafel.","failed":false,"voice":null}]},
            "configuration":{"protocol":"pennel_cloud","endpoint":format!("{origin}/v1/decisions"),"model":"pennel-review-v3",
                "expected_model":"perplexity/pplx-decider-v1.2-20261016","upstream":null,"max_input_bytes":65536,"timeout_secs":90},
            "expected_actual":{"model":"perplexity/pplx-decider-v1.2-20261016","provider":"Perplexity",
                "revision":"owned-mock-runtime","artifact_sha256":"a".repeat(64)}
        })).unwrap()
    }

    fn response(request: &mockito::Request) -> Vec<u8> {
        let request: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
        assert_eq!(
            request["expected_model"],
            "perplexity/pplx-decider-v1.2-20261016"
        );
        let answers: serde_json::Map<String, Value> = request["questions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, question)| {
                let choice = if id.ends_with("_support") {
                    "absent"
                } else {
                    "none"
                };
                let probabilities: serde_json::Map<String, Value> = question["criteria"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(|key| (key.clone(), json!(if key == choice { 1.0 } else { 0.0 })))
                    .collect();
                (
                    id.clone(),
                    json!({"type":"choice","choice":choice,"probabilities":probabilities}),
                )
            })
            .collect();
        json!({"schema_version":1,"request_id":request["request_id"],"profile":"supported_passages_v3",
            "model":"perplexity/pplx-decider-v1.2-20261016","provider":"Perplexity","model_revision":"owned-mock-runtime",
            "id":"owned-upstream-request","answers":answers,"usage":{"input_tokens":500,"output_tokens":8,"cost":0.0003},
            "access_token":"must-not-be-captured"}).to_string().into_bytes()
    }

    #[test]
    fn access_only_session_rejects_refresh_credentials_wrong_origins_and_unbounded_expiry() {
        for change in 0..5 {
            let mut value = session("https://api-staging.pennel.app");
            match change {
                0 => value["refresh_token"] = json!("not-accepted"),
                1 => value["origin"] = json!("https://other.example"),
                2 => value["access_token_only"] = json!(false),
                3 => value["expires_at_unix_seconds"] = json!(0),
                _ => value["expires_at_unix_seconds"] = json!(u64::MAX),
            }
            assert!(
                CloudReviewClient::from_session_json(&serde_json::to_vec(&value).unwrap()).is_err()
            );
        }
    }

    #[test]
    fn preflight_reads_only_capabilities_and_authenticated_uuid_metadata() {
        let mut server = mockito::Server::new();
        let no_content = server.mock("POST", "/v1/decisions").expect(0).create();
        let no_refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        let caps = server.mock("GET", "/v1/inference/capabilities")
            .match_header("authorization", format!("Bearer {BEARER}").as_str())
            .with_body(json!({"schema_version":1,"meeting_review":{
                "available":false,"reason_code":"disabled","profile":"supported_passages_v3",
                "model":"pennel-review-v3","configured_model":null,"expected_model":null,"upstream":null,
                "limits":{"max_input_bytes":65536,"max_questions":64,"max_batch_windows":8,"request_timeout_secs":35}
            }}).to_string()).create();
        let user = uuid::Uuid::now_v7().to_string();
        let device = uuid::Uuid::now_v7().to_string();
        let identity = server
            .mock("GET", "/v1/whoami")
            .match_header("authorization", format!("Bearer {BEARER}").as_str())
            .with_body(
                json!({"user_id":user,"device_id":device,"email":"not-exported@example.test"})
                    .to_string(),
            )
            .create();
        let client = CloudReviewClient::from_session(
            &serde_json::to_vec(&session(&server.url())).unwrap(),
            true,
        )
        .unwrap();
        let value = client.preflight().unwrap();
        assert_eq!(value["caller"], json!({"user_id":user,"device_id":device}));
        assert_eq!(value["provider_requests"], 0);
        assert!(!value.to_string().contains("not-exported"));
        assert!(!value.to_string().contains(BEARER));
        caps.assert();
        identity.assert();
        no_content.assert();
        no_refresh.assert();
    }

    #[test]
    fn actual_engine_transport_capture_and_replay_preserve_cost_without_capturing_credentials() {
        let mut server = mockito::Server::new();
        let refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        let post = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", format!("Bearer {BEARER}").as_str())
            .with_body_from_request(response)
            .create();
        let mut client = CloudReviewClient::from_session(
            &serde_json::to_vec(&session(&server.url())).unwrap(),
            true,
        )
        .unwrap();
        let export = exported(&server.url());
        assert_eq!(
            export["runtime_policy"],
            "raw_supported_passage_decisions_v3"
        );
        assert_eq!(
            export["batches"][0]["managed_transport"]["canonical_upstream_contract"]["model"],
            "perplexity/pplx-decider-v1.2"
        );
        let run = uuid::Uuid::now_v7().to_string();
        let value = client.evaluate(json!({"op":"finished_cloud_review_batch","run_id":run,"export":export,"batch_index":0})).unwrap();
        assert_eq!(value["outcome"]["status"], "ok");
        assert_eq!(value["outcome"]["replay"]["status"], "ok");
        assert_eq!(
            value["outcome"]["result"]["usage"]["billed_cost_usd"],
            0.0003
        );
        assert_eq!(value["attempts"][0]["response"]["usage"]["cost"], 0.0003);
        assert_eq!(value["run_complete"], true);
        assert_eq!(value["attempts"].as_array().unwrap().len(), 1);
        assert!(!value.to_string().contains(BEARER));
        assert!(!value.to_string().contains("must-not-be-captured"));
        assert!(client.evaluate(json!({"op":"finished_cloud_review_batch","run_id":run,"export":export,"batch_index":0})).is_err());
        refresh.assert();
        post.assert();
    }

    #[test]
    fn unauthorized_access_only_token_stops_without_refresh_or_a_second_content_request() {
        let mut server = mockito::Server::new();
        let refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        let post = server
            .mock("POST", "/v1/decisions")
            .with_status(401)
            .expect(1)
            .create();
        let mut client = CloudReviewClient::from_session(
            &serde_json::to_vec(&session(&server.url())).unwrap(),
            true,
        )
        .unwrap();
        let value = client
            .evaluate(
                json!({"op":"finished_cloud_review_batch","run_id":uuid::Uuid::now_v7().to_string(),
            "export":exported(&server.url()),"batch_index":0}),
            )
            .unwrap();
        assert_eq!(value["outcome"]["status"], "cloud_access_unavailable");
        assert_eq!(value["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(value["attempts"][0]["http_status"], 401);
        refresh.assert();
        post.assert();
    }
    #[test]
    fn qa_access_lease_refreshes_the_same_normal_session_without_exporting_refresh_credentials() {
        let mut server = mockito::Server::new();
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        secrets.set(
            crate::cloud::REFRESH_ACCOUNT.into(),
            Some("owned-qa-refresh-before".into()),
        );
        let refresh = server.mock("POST", "/v1/auth/refresh")
            .match_body(mockito::Matcher::Json(json!({"refresh_token":"owned-qa-refresh-before"})))
            .with_body(json!({"access_token":BEARER,"refresh_token":"owned-qa-refresh-after","expires_in":900}).to_string())
            .expect(1).create();
        let normal = Session::new(server.url(), secrets.clone());
        let lease = normal.evaluation_access_token_lease(600).unwrap();
        let mut written = Vec::new();
        lease.write_secret(&mut written).unwrap();
        let value: Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(value["access_token"], BEARER);
        assert_eq!(value["access_token_only"], true);
        assert!(value.get("refresh_token").is_none());
        assert!(
            !String::from_utf8(written)
                .unwrap()
                .contains("owned-qa-refresh")
        );
        assert_eq!(normal.access_token().unwrap(), BEARER);
        assert_eq!(
            secrets.get(crate::cloud::REFRESH_ACCOUNT.into()).as_deref(),
            Some("owned-qa-refresh-after")
        );
        assert_eq!(lease.origin(), server.url());
        assert!(
            lease.expires_at_unix_seconds()
                > SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 800
        );
        refresh.assert();
    }
}
