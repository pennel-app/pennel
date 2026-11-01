use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PennelError;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INPUT_BYTES: u32 = 64 * 1024;
// Providers document approximately normalized distributions. Keep their raw values.
const DISTRIBUTION_TOLERANCE: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum InferenceProtocol {
    OpenRouterDecisions,
    OpenRouterSystemOne,
    TypeSafe,
    Perplexity,
    Mercury,
    LocalDecider,
    Tei,
    PennelCloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum InferenceCapability {
    Decision,
    Reranking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct InferenceConfig {
    pub provider_id: String,
    pub protocol: InferenceProtocol,
    pub model: String,
    #[serde(default)]
    pub expected_model: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default = "default_input_bytes")]
    pub max_input_bytes: u32,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
}

fn default_input_bytes() -> u32 {
    16_000
}
fn default_timeout() -> u32 {
    30
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct InferenceProtocolInfo {
    pub protocol: InferenceProtocol,
    pub name: String,
    pub default_base_url: String,
    pub decision: bool,
    pub reranking: bool,
    pub extraction: bool,
    pub requires_key: bool,
    pub allowed_provider_kinds: Vec<crate::ProviderKind>,
    pub managed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum DecisionQuestionKind {
    Binary,
    Choice,
    Ordinal,
    Extract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionOption {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionQuestion {
    pub id: String,
    pub kind: DecisionQuestionKind,
    pub instructions: String,
    pub options: Vec<DecisionOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionRequest {
    pub state: String,
    pub questions: Vec<DecisionQuestion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionProbability {
    pub option_id: String,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionSpan {
    pub field_id: String,
    pub text: String,
    pub start_char: u32,
    pub end_char: u32,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionAnswer {
    pub id: String,
    pub kind: DecisionQuestionKind,
    pub choice: Option<String>,
    pub value: Option<f64>,
    pub probabilities: Vec<DecisionProbability>,
    pub confidence: Option<f64>,
    pub spans: Vec<DecisionSpan>,
    pub not_stated: Vec<DecisionProbability>,
    pub abstained: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct InferenceIdentity {
    pub protocol: InferenceProtocol,
    pub endpoint: String,
    pub requested_model: String,
    pub requested_upstream: Option<String>,
    pub actual_model: Option<String>,
    pub actual_revision: Option<String>,
    pub actual_provider: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct InferenceUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub billed_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DecisionResult {
    pub answers: Vec<DecisionAnswer>,
    pub identity: InferenceIdentity,
    pub usage: InferenceUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct RerankPassage {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct RerankRequest {
    pub query: String,
    pub passages: Vec<RerankPassage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct RerankScore {
    pub passage_id: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct RerankResult {
    pub scores: Vec<RerankScore>,
    pub identity: InferenceIdentity,
    pub usage: InferenceUsage,
}

#[derive(Debug, uniffi::Object)]
pub struct InferenceCancellation {
    cancelled: AtomicBool,
}

#[uniffi::export]
impl InferenceCancellation {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
        })
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[uniffi::export]
pub fn new_inference_config(provider_id: String, protocol: InferenceProtocol) -> InferenceConfig {
    InferenceConfig {
        provider_id,
        protocol,
        model: if protocol == InferenceProtocol::PennelCloud {
            "pennel-auto".into()
        } else {
            String::new()
        },
        expected_model: None,
        upstream: None,
        max_input_bytes: default_input_bytes(),
        timeout_secs: default_timeout(),
    }
}

#[uniffi::export]
pub fn inference_protocols() -> Vec<InferenceProtocolInfo> {
    use InferenceProtocol::*;
    [
        (
            OpenRouterDecisions,
            "OpenRouter Decisions",
            "https://openrouter.ai/api/v1",
            true,
        ),
        (
            OpenRouterSystemOne,
            "OpenRouter System One",
            "https://openrouter.ai/api/v1",
            true,
        ),
        (TypeSafe, "TypeSafe Jev", "https://api.typesafe.ai/v1", true),
        (
            Perplexity,
            "Perplexity Decisions",
            "https://api.perplexity.ai/v1",
            true,
        ),
        (
            Mercury,
            "Mercury Decide",
            "https://api.inceptionlabs.ai/v1",
            true,
        ),
        (
            LocalDecider,
            "Local Decider",
            "http://127.0.0.1:8080/v1",
            false,
        ),
        (Tei, "TEI reranking", "http://127.0.0.1:8080", false),
        (PennelCloud, "Pennel Cloud", "https://api.pennel.app", false),
    ]
    .into_iter()
    .map(
        |(protocol, name, base, requires_key)| InferenceProtocolInfo {
            protocol,
            name: name.into(),
            default_base_url: base.into(),
            decision: protocol != Tei,
            reranking: matches!(protocol, Tei | PennelCloud),
            extraction: protocol == Mercury,
            requires_key,
            allowed_provider_kinds: match protocol {
                PennelCloud => vec![crate::ProviderKind::PennelCloud],
                OpenRouterDecisions | OpenRouterSystemOne => {
                    vec![crate::ProviderKind::OpenRouter, crate::ProviderKind::Custom]
                }
                _ => vec![crate::ProviderKind::Custom],
            },
            managed: protocol == PennelCloud,
        },
    )
    .collect()
}

fn invalid(message: &'static str) -> PennelError {
    PennelError::InvalidResponse {
        message: message.into(),
    }
}

pub(crate) fn network() -> PennelError {
    PennelError::Network {
        message: "The inference provider could not complete the request.".into(),
    }
}

fn router(protocol: InferenceProtocol) -> bool {
    matches!(
        protocol,
        InferenceProtocol::OpenRouterDecisions | InferenceProtocol::OpenRouterSystemOne
    )
}

fn usable_text(text: &str, maximum: usize) -> bool {
    !text.trim().is_empty() && text.len() <= maximum && !text.chars().any(char::is_control)
}

pub(crate) fn validate_config(
    config: &InferenceConfig,
    base_url: &str,
    capability: InferenceCapability,
) -> Result<String, PennelError> {
    if !usable_text(&config.provider_id, 256)
        || !usable_text(&config.model, 256)
        || config
            .expected_model
            .as_ref()
            .is_some_and(|m| !usable_text(m, 256))
        || !(1..=90).contains(&config.timeout_secs)
        || !(1..=MAX_INPUT_BYTES).contains(&config.max_input_bytes)
    {
        return Err(invalid(
            "Choose a provider, model and valid inference limits first.",
        ));
    }
    let protocol = inference_protocols()
        .into_iter()
        .find(|info| info.protocol == config.protocol)
        .expect("every inference protocol has metadata");
    if !match capability {
        InferenceCapability::Decision => protocol.decision,
        InferenceCapability::Reranking => protocol.reranking,
    } {
        return Err(invalid(
            "This protocol does not support the selected inference capability.",
        ));
    }
    if config.protocol == InferenceProtocol::PennelCloud
        && (config.model != "pennel-auto" || config.upstream.is_some())
    {
        return Err(invalid(
            "Pennel Cloud manages its inference model and routing.",
        ));
    }
    if router(config.protocol)
        && !config.upstream.as_ref().is_some_and(|v| {
            usable_text(v, 128)
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
        })
    {
        return Err(invalid(
            "Choose the exact OpenRouter upstream before testing decisions.",
        ));
    }
    if !router(config.protocol)
        && config
            .upstream
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
    {
        return Err(invalid("Upstream routing is only supported by OpenRouter."));
    }
    let (host, loopback) = validate_origin(base_url)?;
    let canonical_host = host.trim_end_matches('.').to_ascii_lowercase();
    if (canonical_host == "openrouter.ai" || canonical_host.ends_with(".openrouter.ai"))
        && !router(config.protocol)
    {
        return Err(invalid(
            "Choose an OpenRouter protocol for this server so fixed upstream routing and data policies are enforced.",
        ));
    }
    if config.protocol == InferenceProtocol::LocalDecider && !loopback {
        return Err(invalid(
            "Local Decider must use a loopback server on this device.",
        ));
    }
    let base = base_url.trim_end_matches('/');
    let endpoint = match config.protocol {
        InferenceProtocol::PennelCloud => format!(
            "{}/v1/{}",
            base.strip_suffix("/v1").unwrap_or(base),
            match capability {
                InferenceCapability::Decision => "decisions",
                InferenceCapability::Reranking => "rerank",
            }
        ),
        InferenceProtocol::OpenRouterDecisions => {
            if base.ends_with("/api/alpha/decisions") {
                base.into()
            } else {
                format!(
                    "{}/api/alpha/decisions",
                    base.strip_suffix("/api/v1")
                        .or_else(|| base.strip_suffix("/api"))
                        .unwrap_or(base)
                )
            }
        }
        InferenceProtocol::OpenRouterSystemOne => {
            if base.ends_with("/api/v1/systemone") {
                base.into()
            } else {
                format!(
                    "{}/api/v1/systemone",
                    base.strip_suffix("/api/v1")
                        .or_else(|| base.strip_suffix("/api"))
                        .unwrap_or(base)
                )
            }
        }
        protocol => {
            let suffix = match protocol {
                InferenceProtocol::TypeSafe => "/systemone",
                InferenceProtocol::Tei => "/rerank",
                _ => "/decisions",
            };
            if base.ends_with(suffix) {
                base.into()
            } else {
                format!("{base}{suffix}")
            }
        }
    };
    Ok(endpoint)
}

pub(crate) fn validate_origin(base_url: &str) -> Result<(String, bool), PennelError> {
    let uri: ureq::http::Uri = base_url
        .parse()
        .map_err(|_| invalid("Enter a valid inference server URL."))?;
    let scheme = uri.scheme_str().unwrap_or("");
    let host = uri.host().unwrap_or("");
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if host.is_empty()
        || base_url.chars().any(char::is_control)
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
        || uri.query().is_some()
        || base_url.contains('#')
        || !(scheme == "https" || scheme == "http" && loopback)
    {
        return Err(invalid(
            "Use HTTPS, or an explicitly configured loopback HTTP server, without URL credentials or query parameters.",
        ));
    }
    Ok((host.to_owned(), loopback))
}

struct WireOptions<'a>(&'a [DecisionOption]);
impl Serialize for WireOptions<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for option in self.0 {
            map.serialize_entry(&option.id, &option.description)?;
        }
        map.end()
    }
}

struct WireQuestion<'a>(&'a DecisionQuestion);
impl Serialize for WireQuestion<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let question = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry(
            "type",
            match question.kind {
                DecisionQuestionKind::Binary => "noul",
                DecisionQuestionKind::Choice => "choice",
                DecisionQuestionKind::Ordinal => "score",
                DecisionQuestionKind::Extract => "extract",
            },
        )?;
        map.serialize_entry("instructions", &question.instructions)?;
        if question.kind == DecisionQuestionKind::Ordinal {
            map.serialize_entry(
                "criteria",
                &question
                    .options
                    .iter()
                    .map(|o| &o.description)
                    .collect::<Vec<_>>(),
            )?;
        } else if !question.options.is_empty() {
            map.serialize_entry("criteria", &WireOptions(&question.options))?;
        }
        map.end()
    }
}

struct WireQuestions<'a>(&'a [DecisionQuestion]);
impl Serialize for WireQuestions<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for question in self.0 {
            map.serialize_entry(&question.id, &WireQuestion(question))?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    state: &'a str,
    questions: WireQuestions<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<&'a str>,
}

fn decision_body(
    config: &InferenceConfig,
    request: &DecisionRequest,
) -> Result<Vec<u8>, PennelError> {
    decision_body_with_id(config, request, None)
}

fn decision_body_with_id(
    config: &InferenceConfig,
    request: &DecisionRequest,
    request_id: Option<&str>,
) -> Result<Vec<u8>, PennelError> {
    if request.questions.is_empty() || request.questions.len() > 128 || request.state.is_empty() {
        return Err(invalid(
            "Provide text and between one and 128 decision questions.",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for question in &request.questions {
        if !usable_text(&question.id, 128)
            || !ids.insert(&question.id)
            || question.instructions.trim().is_empty()
            || question.instructions.len() > 8000
        {
            return Err(invalid(
                "Decision questions need unique IDs and bounded instructions.",
            ));
        }
        let mut options = std::collections::HashSet::new();
        if question.options.iter().any(|o| {
            !usable_text(&o.id, 128) || o.description.len() > 8000 || !options.insert(o.id.as_str())
        }) {
            return Err(invalid(
                "Decision options need unique IDs and bounded descriptions.",
            ));
        }
        let count = question.options.len();
        let supported = match question.kind {
            DecisionQuestionKind::Binary => {
                count == 0 || count == 2 && options.contains("true") && options.contains("false")
            }
            DecisionQuestionKind::Choice => (2..=255).contains(&count),
            DecisionQuestionKind::Ordinal => (2..=10).contains(&count),
            DecisionQuestionKind::Extract => {
                config.protocol == InferenceProtocol::Mercury && (1..=26).contains(&count)
            }
        };
        if !supported {
            return Err(invalid(
                "The decision question shape is unsupported by this protocol.",
            ));
        }
    }
    if router(config.protocol)
        && config.model.starts_with("cloudflare/clef")
        && request.state.len() > 2000
    {
        return Err(invalid(
            "This Clef route requires a shorter state; the request was not truncated or sent.",
        ));
    }
    let provider = router(config.protocol).then(|| {
        json!({
            "only": [config.upstream.as_deref().unwrap_or("")], "allow_fallbacks": false,
            "data_collection": "deny", "zdr": true,
        })
    });
    let body = serde_json::to_vec(&WireRequest {
        model: &config.model,
        state: &request.state,
        questions: WireQuestions(&request.questions),
        provider,
        request_id,
    })
    .map_err(|_| invalid("The decision request could not be encoded."))?;
    let protocol_bytes = match config.protocol {
        InferenceProtocol::TypeSafe => 32_000,
        InferenceProtocol::LocalDecider => 32_768,
        _ => MAX_INPUT_BYTES as usize,
    };
    if body.len() > protocol_bytes {
        return Err(invalid(
            "The decision request exceeds this protocol's conservative context budget; it was not sent.",
        ));
    }
    validate_size(config, &body)?;
    Ok(body)
}

fn validate_size(config: &InferenceConfig, body: &[u8]) -> Result<(), PennelError> {
    if body.len() > config.max_input_bytes as usize {
        Err(invalid(
            "The inference request exceeds its configured byte budget; it was not truncated or sent.",
        ))
    } else {
        Ok(())
    }
}

struct WorkerPermit;
static ACTIVE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn run_cancelled<T: Send + 'static>(
    cancellation: Arc<InferenceCancellation>,
    timeout_secs: u32,
    operation: impl FnOnce(std::time::Instant) -> Result<T, PennelError> + Send + 'static,
) -> Result<T, PennelError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.into());
    run_cancelled_before(cancellation, deadline, operation)
}

pub(crate) fn run_cancelled_before<T: Send + 'static>(
    cancellation: Arc<InferenceCancellation>,
    deadline: std::time::Instant,
    operation: impl FnOnce(std::time::Instant) -> Result<T, PennelError> + Send + 'static,
) -> Result<T, PennelError> {
    loop {
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        if std::time::Instant::now() >= deadline {
            return Err(network());
        }
        let active = ACTIVE_WORKERS.load(Ordering::Acquire);
        if active < 2
            && ACTIVE_WORKERS
                .compare_exchange(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            break;
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(std::time::Duration::from_millis(20)),
        );
    }
    let permit = WorkerPermit;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker_cancellation = cancellation.clone();
    std::thread::Builder::new()
        .name("pennel-inference".into())
        .spawn(move || {
            let _permit = permit;
            let result = if worker_cancellation.is_cancelled() {
                Err(PennelError::Cancelled)
            } else {
                operation(deadline)
            };
            let _ = sender.send(result);
        })
        .map_err(|_| network())?;
    loop {
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        if std::time::Instant::now() >= deadline {
            return Err(network());
        }
        match receiver.recv_timeout(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(std::time::Duration::from_millis(20)),
        ) {
            Ok(result) => {
                return if cancellation.is_cancelled() {
                    Err(PennelError::Cancelled)
                } else if std::time::Instant::now() >= deadline {
                    Err(network())
                } else {
                    result
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(network()),
        }
    }
}

fn send_json(
    endpoint: &str,
    key: Option<&str>,
    body: Option<&[u8]>,
    deadline: std::time::Instant,
) -> Result<Value, PennelError> {
    let (status, response) = send_json_status(endpoint, key, body, deadline)?;
    if !(200..300).contains(&status) {
        return Err(crate::http::map_status(status, ""));
    }
    Ok(response)
}

pub(crate) fn send_json_status(
    endpoint: &str,
    key: Option<&str>,
    body: Option<&[u8]>,
    deadline: std::time::Instant,
) -> Result<(u16, Value), PennelError> {
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    if remaining.is_zero() {
        return Err(network());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(remaining))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = if let Some(body) = body {
        crate::http::with_auth(
            agent
                .post(endpoint)
                .header("Content-Type", "application/json"),
            key,
        )
        .send(body)
    } else {
        let request = agent.get(endpoint);
        match key {
            Some(key) => request.header("Authorization", format!("Bearer {key}")),
            None => request,
        }
        .call()
    }
    .map_err(|_| network())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Ok((status, Value::Null));
    }
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|_| invalid("The inference response is unreadable or exceeds its size limit."))?;
    Ok((status, parse_unique_json(&text)?))
}

#[derive(Clone)]
pub(crate) struct ManagedInference {
    pub(crate) session: Arc<crate::cloud::account::Session>,
    pub(crate) generation: u64,
}

impl ManagedInference {
    fn send(
        &self,
        endpoint: &str,
        body: Option<&[u8]>,
        cancellation: &InferenceCancellation,
        deadline: std::time::Instant,
    ) -> Result<Value, PennelError> {
        let token = zeroize::Zeroizing::new(self.session.inference_token(
            self.generation,
            false,
            cancellation,
            deadline,
        )?);
        self.session
            .check_inference(self.generation, cancellation, deadline)?;
        let mut response = send_json_status(endpoint, Some(&token), body, deadline)?;
        self.session
            .check_inference(self.generation, cancellation, deadline)?;
        if response.0 == 401 {
            let token = zeroize::Zeroizing::new(self.session.inference_token(
                self.generation,
                true,
                cancellation,
                deadline,
            )?);
            self.session
                .check_inference(self.generation, cancellation, deadline)?;
            response = send_json_status(endpoint, Some(&token), body, deadline)?;
            self.session
                .check_inference(self.generation, cancellation, deadline)?;
        }
        if !(200..300).contains(&response.0) {
            return Err(crate::http::map_status(response.0, ""));
        }
        Ok(response.1)
    }
}

fn managed_response(response: &Value, request_id: &str) -> Result<(), PennelError> {
    if response["schema_version"].as_u64() != Some(1)
        || response["request_id"].as_str() != Some(request_id)
    {
        return Err(invalid(
            "Pennel Cloud returned an unsupported or mismatched inference response.",
        ));
    }
    Ok(())
}

pub(crate) fn managed_capability_available(
    config: &InferenceConfig,
    managed: ManagedInference,
    capability: InferenceCapability,
    cancellation: Arc<InferenceCancellation>,
) -> Result<bool, PennelError> {
    validate_config(config, managed.session.base_url(), capability)?;
    let base = managed.session.base_url().trim_end_matches('/');
    let endpoint = format!(
        "{}/v1/inference/capabilities",
        base.strip_suffix("/v1").unwrap_or(base)
    );
    let worker_cancellation = cancellation.clone();
    run_cancelled(cancellation, config.timeout_secs, move |deadline| {
        let response = managed.send(&endpoint, None, &worker_cancellation, deadline)?;
        if response["schema_version"].as_u64() != Some(1) {
            return Err(invalid(
                "Pennel Cloud returned unsupported inference capabilities.",
            ));
        }
        let field = match capability {
            InferenceCapability::Decision => "decision",
            InferenceCapability::Reranking => "reranking",
        };
        response[field]["available"]
            .as_bool()
            .ok_or_else(|| invalid("Pennel Cloud omitted this inference capability."))
    })
}

pub(crate) fn decide_managed(
    config: &InferenceConfig,
    managed: ManagedInference,
    request: DecisionRequest,
    cancellation: Arc<InferenceCancellation>,
) -> Result<DecisionResult, PennelError> {
    let endpoint = validate_config(
        config,
        managed.session.base_url(),
        InferenceCapability::Decision,
    )?;
    let request_id = uuid::Uuid::now_v7().to_string();
    let body = decision_body_with_id(config, &request, Some(&request_id))?;
    let config = config.clone();
    let worker_cancellation = cancellation.clone();
    run_cancelled(cancellation, config.timeout_secs, move |deadline| {
        let response = managed.send(&endpoint, Some(&body), &worker_cancellation, deadline)?;
        managed_response(&response, &request_id)?;
        decision_result(&config, endpoint, &request, &response)
    })
}

pub(crate) fn rerank_managed(
    config: &InferenceConfig,
    managed: ManagedInference,
    request: RerankRequest,
    cancellation: Arc<InferenceCancellation>,
) -> Result<RerankResult, PennelError> {
    let endpoint = validate_config(
        config,
        managed.session.base_url(),
        InferenceCapability::Reranking,
    )?;
    validate_rerank_request(&request)?;
    let request_id = uuid::Uuid::now_v7().to_string();
    let body = serde_json::to_vec(&json!({
        "model": config.model, "request_id": request_id, "query": request.query,
        "documents": request.passages.iter().map(|p| &p.text).collect::<Vec<_>>()
    }))
    .map_err(|_| invalid("The reranking request could not be encoded."))?;
    validate_size(config, &body)?;
    let config = config.clone();
    let worker_cancellation = cancellation.clone();
    run_cancelled(cancellation, config.timeout_secs, move |deadline| {
        let response = managed.send(&endpoint, Some(&body), &worker_cancellation, deadline)?;
        managed_response(&response, &request_id)?;
        Ok(RerankResult {
            scores: rerank_scores(&request, &response["results"])?,
            identity: identity(&config, endpoint, &response)?,
            usage: usage(&response)?,
        })
    })
}

fn probability(value: &Value) -> Result<f64, PennelError> {
    let number = value
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n));
    number.ok_or_else(|| invalid("The inference response contains an invalid probability."))
}

fn confidence(value: &Value) -> Result<Option<f64>, PennelError> {
    if value.is_null() {
        Ok(None)
    } else {
        probability(value).map(Some)
    }
}

fn probabilities(value: &Value, keys: &[String]) -> Result<Vec<DecisionProbability>, PennelError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("The decision response omitted its probability distribution."))?;
    if object.len() != keys.len() || object.keys().any(|key| !keys.contains(key)) {
        return Err(invalid(
            "The decision response changed the allowed answer IDs.",
        ));
    }
    let entries = keys
        .iter()
        .map(|key| {
            Ok(DecisionProbability {
                option_id: key.clone(),
                probability: probability(&object[key])?,
            })
        })
        .collect::<Result<Vec<_>, PennelError>>()?;
    if (entries.iter().map(|e| e.probability).sum::<f64>() - 1.0).abs() > DISTRIBUTION_TOLERANCE {
        return Err(invalid(
            "The decision probabilities do not approximately sum to one.",
        ));
    }
    Ok(entries)
}

fn parse_answer(
    question: &DecisionQuestion,
    answer: &Value,
    state: &str,
) -> Result<DecisionAnswer, PennelError> {
    if !answer.is_object() {
        return Err(invalid("The decision response contains an invalid answer."));
    }
    let expected_type = match question.kind {
        DecisionQuestionKind::Binary => "noul",
        DecisionQuestionKind::Choice => "choice",
        DecisionQuestionKind::Ordinal => "score",
        DecisionQuestionKind::Extract => "extract",
    };
    if answer
        .get("type")
        .is_some_and(|v| v.as_str() != Some(expected_type))
    {
        return Err(invalid(
            "The decision response changed the requested question type.",
        ));
    }
    let explicitly_abstained = match answer.get("abstained") {
        None | Some(Value::Null) => false,
        Some(value) => value.as_bool().ok_or_else(|| {
            invalid("The decision response contains invalid abstention metadata.")
        })?,
    };
    let mut result = DecisionAnswer {
        id: question.id.clone(),
        kind: question.kind,
        choice: None,
        value: None,
        probabilities: vec![],
        confidence: confidence(&answer["confidence"])?,
        spans: vec![],
        not_stated: vec![],
        abstained: explicitly_abstained,
    };
    match question.kind {
        DecisionQuestionKind::Binary => {
            result.value = Some(probability(&answer["noul"])?);
        }
        DecisionQuestionKind::Choice => {
            let keys = question
                .options
                .iter()
                .map(|o| o.id.clone())
                .collect::<Vec<_>>();
            result.probabilities = probabilities(&answer["probabilities"], &keys)?;
            let choice = answer["choice"]
                .as_str()
                .filter(|s| keys.iter().any(|key| key == s))
                .ok_or_else(|| invalid("The decision response chose an unknown answer ID."))?;
            let chosen = result
                .probabilities
                .iter()
                .find(|p| p.option_id == choice)
                .unwrap()
                .probability;
            if result
                .probabilities
                .iter()
                .any(|p| p.probability > chosen + DISTRIBUTION_TOLERANCE)
            {
                return Err(invalid(
                    "The selected answer disagrees with the returned probabilities.",
                ));
            }
            result.choice = Some(choice.into());
            // Only this explicitly offered ID has reserved abstention semantics.
            result.abstained |= choice == "abstain";
        }
        DecisionQuestionKind::Ordinal => {
            let wire_keys = (0..question.options.len())
                .map(|n| n.to_string())
                .collect::<Vec<_>>();
            result.probabilities = probabilities(&answer["probabilities"], &wire_keys)?;
            for (entry, option) in result.probabilities.iter_mut().zip(&question.options) {
                entry.option_id = option.id.clone();
            }
            let score = answer["score"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0 && *n <= (question.options.len() - 1) as f64)
                .ok_or_else(|| {
                    invalid("The decision response contains an invalid ordinal score.")
                })?;
            let expected = result
                .probabilities
                .iter()
                .enumerate()
                .map(|(index, entry)| index as f64 * entry.probability)
                .sum::<f64>();
            // All supported score protocols specify the expected level. Scale the
            // rounding tolerance with the largest possible level, preserving raw values.
            let tolerance = DISTRIBUTION_TOLERANCE * (question.options.len() - 1) as f64;
            if (score - expected).abs() > tolerance {
                return Err(invalid(
                    "The ordinal score disagrees with its probability-weighted expected level.",
                ));
            }
            result.value = Some(score);
        }
        DecisionQuestionKind::Extract => {
            let fields = question
                .options
                .iter()
                .map(|o| o.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            let spans = answer["spans"]
                .as_object()
                .ok_or_else(|| invalid("The extraction response omitted its source spans."))?;
            let absent = answer["not_stated"].as_object().ok_or_else(|| {
                invalid("The extraction response omitted missing-field probabilities.")
            })?;
            if spans.len() != fields.len()
                || absent.len() != fields.len()
                || spans.keys().any(|f| !fields.contains(f.as_str()))
                || absent.keys().any(|f| !fields.contains(f.as_str()))
            {
                return Err(invalid(
                    "The extraction response changed the allowed field IDs.",
                ));
            }
            let chars = state.chars().collect::<Vec<_>>();
            for option in &question.options {
                result.not_stated.push(DecisionProbability {
                    option_id: option.id.clone(),
                    probability: probability(&absent[&option.id])?,
                });
                let rows = spans[&option.id]
                    .as_array()
                    .filter(|a| a.len() <= 256)
                    .ok_or_else(|| {
                        invalid("The extraction response contains invalid source spans.")
                    })?;
                let mut seen = std::collections::HashSet::new();
                for row in rows {
                    let start = row["start"].as_u64().and_then(|n| usize::try_from(n).ok());
                    let end = row["end"].as_u64().and_then(|n| usize::try_from(n).ok());
                    let (Some(start), Some(end), Some(text)) = (start, end, row["text"].as_str())
                    else {
                        return Err(invalid(
                            "The extraction response contains invalid source offsets.",
                        ));
                    };
                    if start >= end
                        || end > chars.len()
                        || !seen.insert((start, end))
                        || chars[start..end].iter().collect::<String>() != text
                    {
                        return Err(invalid(
                            "An extracted span does not match the original Unicode source text.",
                        ));
                    }
                    result.spans.push(DecisionSpan {
                        field_id: option.id.clone(),
                        text: text.into(),
                        start_char: start as u32,
                        end_char: end as u32,
                        confidence: confidence(&row["confidence"])?,
                    });
                }
            }
            result.abstained |= result.spans.is_empty();
        }
    }
    Ok(result)
}

fn optional_string(value: &Value) -> Result<Option<String>, PennelError> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .filter(|s| usable_text(s, 512))
        .map(|s| Some(s.into()))
        .ok_or_else(|| invalid("The inference response contains invalid identity metadata."))
}

fn identity(
    config: &InferenceConfig,
    endpoint: String,
    response: &Value,
) -> Result<InferenceIdentity, PennelError> {
    let actual_model = optional_string(&response["model"])?;
    let actual_revision = optional_string(
        response
            .get("model_sha")
            .or_else(|| response.get("model_revision"))
            .unwrap_or(&Value::Null),
    )?;
    let actual_provider = optional_string(&response["provider"])?;
    if config.protocol == InferenceProtocol::PennelCloud && actual_model.is_none() {
        return Err(invalid("Pennel Cloud omitted its actual model identity."));
    }
    if config
        .expected_model
        .as_ref()
        .is_some_and(|expected| actual_model.as_ref() != Some(expected))
    {
        return Err(invalid(
            "The inference provider returned a different model than the pinned identity.",
        ));
    }
    if router(config.protocol) {
        let normalized = |s: &str| {
            s.chars()
                .filter(char::is_ascii_alphanumeric)
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        let expected = config
            .upstream
            .as_deref()
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        if actual_provider
            .as_deref()
            .is_none_or(|p| normalized(p) != normalized(expected))
        {
            return Err(invalid(
                "OpenRouter returned a different or unknown upstream provider.",
            ));
        }
        if actual_model.is_none() {
            return Err(invalid("OpenRouter omitted its actual model identity."));
        }
    }
    let request_id = optional_string(
        response
            .get("id")
            .or_else(|| response.get("request_id"))
            .unwrap_or(&Value::Null),
    )?;
    Ok(InferenceIdentity {
        protocol: config.protocol,
        endpoint,
        requested_model: config.model.clone(),
        requested_upstream: config.upstream.clone(),
        actual_model,
        actual_revision,
        actual_provider,
        request_id,
    })
}

fn usage(response: &Value) -> Result<InferenceUsage, PennelError> {
    let value = &response["usage"];
    if !value.is_null() && !value.is_object() {
        return Err(invalid(
            "The inference response contains invalid usage metadata.",
        ));
    }
    let count = |first: &str, second: &str| -> Result<Option<u64>, PennelError> {
        match value.get(first).or_else(|| value.get(second)) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_u64()
                .map(Some)
                .ok_or_else(|| invalid("The inference response contains an invalid token count.")),
        }
    };
    let cost = match value.get("cost").or_else(|| value.get("cost_usd")) {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| {
                    invalid("The inference response contains an invalid billed cost.")
                })?,
        ),
    };
    Ok(InferenceUsage {
        input_tokens: count("input_tokens", "prompt_tokens")?,
        output_tokens: count("output_tokens", "completion_tokens")?,
        billed_cost_usd: cost,
    })
}

pub(crate) fn decide(
    config: &InferenceConfig,
    base_url: &str,
    key: Option<&str>,
    request: DecisionRequest,
    cancellation: Arc<InferenceCancellation>,
) -> Result<DecisionResult, PennelError> {
    decide_before(
        config,
        base_url,
        key,
        request,
        cancellation,
        std::time::Instant::now() + std::time::Duration::from_secs(config.timeout_secs.into()),
    )
}

pub(crate) fn decide_before(
    config: &InferenceConfig,
    base_url: &str,
    key: Option<&str>,
    request: DecisionRequest,
    cancellation: Arc<InferenceCancellation>,
    deadline: std::time::Instant,
) -> Result<DecisionResult, PennelError> {
    let deadline = deadline.min(
        std::time::Instant::now() + std::time::Duration::from_secs(config.timeout_secs.into()),
    );
    if cancellation.is_cancelled() {
        return Err(PennelError::Cancelled);
    }
    if config.protocol == InferenceProtocol::PennelCloud {
        return Err(invalid(
            "Use the signed-in Pennel Cloud inference connection.",
        ));
    }
    let endpoint = validate_config(config, base_url, InferenceCapability::Decision)?;
    let body = decision_body(config, &request)?;
    if inference_protocols()
        .iter()
        .any(|p| p.protocol == config.protocol && p.requires_key)
        && key.is_none_or(str::is_empty)
    {
        return Err(PennelError::MissingApiKey {
            task: "decision".into(),
        });
    }
    let config = config.clone();
    let key = key.map(str::to_owned);
    run_cancelled_before(cancellation, deadline, move |deadline| {
        let response = send_json(&endpoint, key.as_deref(), Some(&body), deadline)?;
        decision_result(&config, endpoint, &request, &response)
    })
}

fn decision_result(
    config: &InferenceConfig,
    endpoint: String,
    request: &DecisionRequest,
    response: &Value,
) -> Result<DecisionResult, PennelError> {
    let answers = response["answers"]
        .as_object()
        .ok_or_else(|| invalid("The decision response omitted its answers."))?;
    if answers.len() != request.questions.len()
        || answers
            .keys()
            .any(|id| !request.questions.iter().any(|q| &q.id == id))
    {
        return Err(invalid(
            "The decision response omitted, duplicated or added question IDs.",
        ));
    }
    let parsed = request
        .questions
        .iter()
        .map(|q| parse_answer(q, &answers[&q.id], &request.state))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecisionResult {
        answers: parsed,
        identity: identity(config, endpoint, response)?,
        usage: usage(response)?,
    })
}

fn validate_rerank_request(request: &RerankRequest) -> Result<(), PennelError> {
    let mut ids = std::collections::HashSet::new();
    if request.query.trim().is_empty()
        || request.passages.is_empty()
        || request.passages.len() > 128
        || request
            .passages
            .iter()
            .any(|p| !usable_text(&p.id, 128) || !ids.insert(&p.id) || p.text.trim().is_empty())
    {
        return Err(invalid(
            "Provide a query and between one and 128 passages with unique IDs.",
        ));
    }
    Ok(())
}

fn rerank_scores(
    request: &RerankRequest,
    response: &Value,
) -> Result<Vec<RerankScore>, PennelError> {
    let rows = response
        .as_array()
        .ok_or_else(|| invalid("The reranking provider returned invalid scores."))?;
    let mut seen = std::collections::HashSet::new();
    if rows.len() != request.passages.len() {
        return Err(invalid("The reranking response omitted passages."));
    }
    rows.iter()
        .map(|row| {
            let index = row["index"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n < request.passages.len() && seen.insert(*n))
                .ok_or_else(|| {
                    invalid("The reranking response contains duplicate or unknown passage IDs.")
                })?;
            let score = row["score"]
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| {
                    invalid("The reranking response contains an invalid relevance score.")
                })?;
            Ok(RerankScore {
                passage_id: request.passages[index].id.clone(),
                score,
            })
        })
        .collect()
}

pub(crate) fn rerank(
    config: &InferenceConfig,
    base_url: &str,
    key: Option<&str>,
    request: RerankRequest,
    cancellation: Arc<InferenceCancellation>,
) -> Result<RerankResult, PennelError> {
    if cancellation.is_cancelled() {
        return Err(PennelError::Cancelled);
    }
    if config.protocol == InferenceProtocol::PennelCloud {
        return Err(invalid(
            "Use the signed-in Pennel Cloud inference connection.",
        ));
    }
    let endpoint = validate_config(config, base_url, InferenceCapability::Reranking)?;
    validate_rerank_request(&request)?;
    let body = serde_json::to_vec(&json!({"query": request.query, "texts": request.passages.iter().map(|p| &p.text).collect::<Vec<_>>(), "raw_scores": true, "truncate": false}))
        .map_err(|_| invalid("The reranking request could not be encoded."))?;
    validate_size(config, &body)?;
    let config = config.clone();
    let key = key.map(str::to_owned);
    let cancellation_for_request = cancellation.clone();
    run_cancelled(cancellation, config.timeout_secs, move |deadline| {
        let info_url = format!(
            "{}/info",
            endpoint.strip_suffix("/rerank").unwrap_or(&endpoint)
        );
        let info = send_json(&info_url, key.as_deref(), None, deadline)?;
        let actual_model = optional_string(&info["model_id"])?
            .ok_or_else(|| invalid("TEI omitted its model identity."))?;
        if actual_model != config.model {
            return Err(invalid(
                "TEI is serving a different model than the selected one.",
            ));
        }
        if cancellation_for_request.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        let response = send_json(&endpoint, key.as_deref(), Some(&body), deadline)?;
        let scores = rerank_scores(&request, &response)?;
        let metadata = json!({"model": actual_model, "model_sha": info.get("model_sha").cloned().unwrap_or(Value::Null), "provider": "TEI"});
        Ok(RerankResult {
            scores,
            identity: identity(&config, endpoint, &metadata)?,
            usage: usage(&Value::Null)?,
        })
    })
}

// Reject duplicate object keys before typed validation can lose them in a map.
fn parse_unique_json(text: &str) -> Result<Value, PennelError> {
    struct Unique(Value);
    impl<'de> Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON without duplicate keys")
                }
                fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Unique, E> {
                    Ok(Unique(json!(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Unique, E> {
                    Ok(Unique(json!(v)))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Unique, E> {
                    Ok(Unique(json!(v)))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Unique, E> {
                    Ok(Unique(json!(v)))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Unique, E> {
                    Ok(Unique(json!(v)))
                }
                fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Unique, E> {
                    Ok(Unique(json!(v)))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_none<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(value)) = seq.next_element()? {
                        values.push(value);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                        if values.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate key"));
                        }
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_str::<Unique>(text)
        .map(|v| v.0)
        .map_err(|_| {
            invalid("The inference provider returned malformed JSON or duplicate object keys.")
        })
}

#[cfg(test)]
#[path = "inference_tests.rs"]
mod tests;
