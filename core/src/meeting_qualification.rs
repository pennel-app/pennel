use serde::{Deserialize, Serialize};

use crate::{InferenceIdentity, InferenceProtocol, PennelEngine};

const ASSET: &str = include_str!("../../assets/qualifications/meeting-assistance.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum MeetingAnalysisUseCase {
    ManualFinishedSuggestions,
    ManualLiveSuggestions,
    SourceAssessment,
    BackgroundLiveSuggestions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingConfiguredInference {
    pub protocol: InferenceProtocol,
    pub endpoint: String,
    pub model: String,
    pub upstream: Option<String>,
    pub expected_model: Option<String>,
    pub max_input_bytes: u32,
    pub timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingQualificationRequest {
    pub use_case: MeetingAnalysisUseCase,
    pub language_policy: String,
    pub task: String,
    pub configured: MeetingConfiguredInference,
    pub prompt_sha256: String,
    pub schema_sha256: String,
    pub label_order_sha256: String,
    pub grouping: String,
    pub preprocessing_sha256: String,
    pub resource_profile_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingActualInference {
    pub model: String,
    pub revision: String,
    pub provider: String,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingQualificationContext {
    #[serde(flatten)]
    pub request: MeetingQualificationRequest,
    pub actual: MeetingActualInference,
    pub calibration_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingQualificationEvidence {
    pub mode: String,
    pub protocol_sha256: String,
    pub corpus_sha256: String,
    pub split_sha256: String,
    pub report_sha256: String,
    pub prefix_isolation: bool,
    pub heldout_disjoint: bool,
    pub source_judgments_complete: bool,
    pub failures_counted: bool,
    pub resource_measurements_complete: bool,
    pub foreground_impact_accepted: bool,
    pub background_resource_accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingQualificationMetrics {
    pub macro_f1: Option<f64>,
    pub accepted_item_precision: Option<f64>,
    pub correct_owner_and_final_status: Option<bool>,
    pub selective_precision: Option<f64>,
    pub coverage: Option<f64>,
    pub warm_p95_ms: Option<f64>,
    pub peak_rss_bytes: Option<u64>,
    pub cpu_time_ms: Option<f64>,
    pub calibration_observations: u64,
    pub calibration_classes: u32,
    pub calibration_meeting_groups: u32,
    pub heldout_observations: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingQualificationRecord {
    pub schema_version: u32,
    pub qualification_id: String,
    pub context: MeetingQualificationContext,
    pub evidence: MeetingQualificationEvidence,
    pub metrics: MeetingQualificationMetrics,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    adopted_records: Vec<MeetingQualificationRecord>,
}

pub struct MeetingQualificationRegistry {
    records: Vec<MeetingQualificationRecord>,
    valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingQualificationError {
    Unconfigured,
    InvalidConfiguration,
    ConfigurationChanged,
    MutationInProgress,
    UnknownLanguage,
    InvalidContext,
    InvalidRegistry,
    Unqualified,
    AmbiguousQualification,
    IdentityChanged,
}

impl MeetingQualificationError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::ConfigurationChanged => "configuration_changed",
            Self::MutationInProgress => "mutation_in_progress",
            Self::UnknownLanguage => "unknown_language",
            Self::InvalidContext => "invalid_context",
            Self::InvalidRegistry => "invalid_registry",
            Self::Unqualified => "unqualified",
            Self::AmbiguousQualification => "ambiguous_qualification",
            Self::IdentityChanged => "identity_changed",
        }
    }
}

fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

impl MeetingQualificationRequest {
    pub(crate) fn validate(&self) -> Result<(), MeetingQualificationError> {
        if !matches!(self.language_policy.as_str(), "en" | "nl" | "mixed") {
            return Err(MeetingQualificationError::UnknownLanguage);
        }
        let task = match self.use_case {
            MeetingAnalysisUseCase::SourceAssessment => "meeting_source_assessment_v1",
            _ => "meeting_suggestions_v1",
        };
        if self.task != task
            || !text(&self.configured.endpoint, 2_048)
            || !text(&self.configured.model, 256)
            || self
                .configured
                .expected_model
                .as_ref()
                .is_some_and(|value| !text(value, 256))
            || self
                .configured
                .upstream
                .as_ref()
                .is_some_and(|value| !text(value, 128))
            || !(1..=65_536).contains(&self.configured.max_input_bytes)
            || !(1..=90).contains(&self.configured.timeout_secs)
            || !text(&self.grouping, 256)
            || [
                &self.prompt_sha256,
                &self.schema_sha256,
                &self.label_order_sha256,
                &self.preprocessing_sha256,
                &self.resource_profile_sha256,
            ]
            .into_iter()
            .any(|value| !hash(value))
        {
            return Err(MeetingQualificationError::InvalidContext);
        }
        let config = crate::InferenceConfig {
            provider_id: "meeting-qualification".into(),
            protocol: self.configured.protocol,
            model: self.configured.model.clone(),
            upstream: self.configured.upstream.clone(),
            expected_model: self.configured.expected_model.clone(),
            max_input_bytes: self.configured.max_input_bytes,
            timeout_secs: self.configured.timeout_secs,
        };
        let base = if config.protocol == InferenceProtocol::PennelCloud {
            self.configured
                .endpoint
                .strip_suffix("/v1/decisions")
                .unwrap_or(&self.configured.endpoint)
        } else {
            &self.configured.endpoint
        };
        if !crate::inference::validate_config(&config, base, crate::InferenceCapability::Decision)
            .is_ok_and(|endpoint| endpoint == self.configured.endpoint)
        {
            return Err(MeetingQualificationError::InvalidContext);
        }
        Ok(())
    }
}

fn probability(value: Option<f64>, minimum: f64) -> bool {
    value.is_some_and(|value| value.is_finite() && value >= minimum && value <= 1.0)
}

impl MeetingQualificationRecord {
    fn valid(&self) -> bool {
        let request = &self.context.request;
        let evidence = &self.evidence;
        let metrics = &self.metrics;
        if self.schema_version != 1
            || !text(&self.qualification_id, 256)
            || request.validate().is_err()
            || !text(&self.context.actual.model, 256)
            || !text(&self.context.actual.revision, 256)
            || !text(&self.context.actual.provider, 256)
            || [
                &self.context.actual.artifact_sha256,
                &self.context.calibration_sha256,
                &evidence.protocol_sha256,
                &evidence.corpus_sha256,
                &evidence.split_sha256,
                &evidence.report_sha256,
            ]
            .into_iter()
            .any(|value| !hash(value))
            || request
                .configured
                .expected_model
                .as_ref()
                .is_some_and(|model| *model != self.context.actual.model)
            || evidence.mode != "recorded_inference"
            || !evidence.prefix_isolation
            || !evidence.heldout_disjoint
            || !evidence.source_judgments_complete
            || !evidence.failures_counted
            || !evidence.resource_measurements_complete
            || metrics.calibration_observations < 10
            || metrics.calibration_classes < 2
            || metrics.calibration_meeting_groups < 2
            || metrics.heldout_observations == 0
            || !metrics.peak_rss_bytes.is_some_and(|value| value > 0)
            || !metrics
                .cpu_time_ms
                .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            return false;
        }
        if !matches!(
            request.use_case,
            MeetingAnalysisUseCase::ManualFinishedSuggestions
        ) && !evidence.foreground_impact_accepted
        {
            return false;
        }
        if request.use_case == MeetingAnalysisUseCase::BackgroundLiveSuggestions
            && !evidence.background_resource_accepted
        {
            return false;
        }
        let source_assessment = request.use_case == MeetingAnalysisUseCase::SourceAssessment;
        let latency_budget = if request.configured.protocol == InferenceProtocol::LocalDecider {
            if source_assessment { 750.0 } else { 5_000.0 }
        } else {
            f64::from(request.configured.timeout_secs) * 1_000.0
        };
        if !metrics
            .warm_p95_ms
            .is_some_and(|value| value.is_finite() && value > 0.0 && value <= latency_budget)
        {
            return false;
        }
        if source_assessment {
            probability(metrics.selective_precision, 0.98) && probability(metrics.coverage, 0.50)
        } else {
            probability(metrics.macro_f1, 0.90)
                && probability(metrics.accepted_item_precision, 0.95)
                && metrics.correct_owner_and_final_status == Some(true)
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualifiedMeetingAnalysis {
    record: MeetingQualificationRecord,
}

impl MeetingQualificationRegistry {
    pub fn bundled() -> &'static Self {
        static REGISTRY: std::sync::OnceLock<MeetingQualificationRegistry> =
            std::sync::OnceLock::new();
        REGISTRY.get_or_init(|| Self::from_asset(ASSET))
    }

    fn from_asset(asset: &str) -> Self {
        let manifest = serde_json::from_str::<Manifest>(asset);
        match manifest {
            Ok(manifest) if manifest.schema_version == 1 => {
                let mut ids = std::collections::HashSet::new();
                let valid = manifest
                    .adopted_records
                    .iter()
                    .all(|record| record.valid() && ids.insert(&record.qualification_id));
                Self {
                    records: manifest.adopted_records,
                    valid,
                }
            }
            _ => Self {
                records: Vec::new(),
                valid: false,
            },
        }
    }

    pub fn admit(
        &self,
        request: &MeetingQualificationRequest,
    ) -> Result<QualifiedMeetingAnalysis, MeetingQualificationError> {
        request.validate()?;
        if !self.valid || self.records.iter().any(|record| !record.valid()) {
            return Err(MeetingQualificationError::InvalidRegistry);
        }
        let mut matches = self
            .records
            .iter()
            .filter(|record| record.context.request == *request);
        let record = matches
            .next()
            .ok_or(MeetingQualificationError::Unqualified)?;
        if matches.next().is_some() {
            return Err(MeetingQualificationError::AmbiguousQualification);
        }
        Ok(QualifiedMeetingAnalysis {
            record: record.clone(),
        })
    }
}

impl QualifiedMeetingAnalysis {
    pub fn record(&self) -> &MeetingQualificationRecord {
        &self.record
    }

    pub fn context_fingerprint(&self) -> String {
        crate::evidence_revision(
            &serde_json::to_value(&self.record.context)
                .expect("application-owned qualification context")
                .to_string(),
        )
    }

    pub fn validate_identity(
        &self,
        identity: &InferenceIdentity,
    ) -> Result<(), MeetingQualificationError> {
        compare_inference_identity(
            &self.record.context.request.configured,
            &self.record.context.actual,
            identity,
        )
    }
}

/// Identity comparison alone cannot construct a qualified admission handle.
pub(crate) fn compare_inference_identity(
    configured: &MeetingConfiguredInference,
    actual: &MeetingActualInference,
    identity: &InferenceIdentity,
) -> Result<(), MeetingQualificationError> {
    if identity.protocol != configured.protocol
        || identity.endpoint != configured.endpoint
        || identity.requested_model != configured.model
        || identity.requested_upstream != configured.upstream
        || identity.actual_model.as_deref() != Some(actual.model.as_str())
        || identity.actual_revision.as_deref() != Some(actual.revision.as_str())
        || identity.actual_provider.as_deref() != Some(actual.provider.as_str())
    {
        return Err(MeetingQualificationError::IdentityChanged);
    }
    Ok(())
}

#[cfg(feature = "evaluation")]
pub(crate) fn valid_evaluation_identity_pin(actual: &MeetingActualInference) -> bool {
    text(&actual.model, 256)
        && text(&actual.revision, 256)
        && text(&actual.provider, 256)
        && hash(&actual.artifact_sha256)
}

impl PennelEngine {
    pub(crate) fn meeting_analysis_configuration(
        &self,
    ) -> Result<(String, MeetingConfiguredInference), MeetingQualificationError> {
        let settings = self.settings();
        let config = settings
            .decision
            .ok_or(MeetingQualificationError::Unconfigured)?;
        let account = settings
            .providers
            .iter()
            .find(|account| account.id == config.provider_id)
            .ok_or(MeetingQualificationError::InvalidConfiguration)?;
        let info = crate::inference::inference_protocols()
            .into_iter()
            .find(|info| info.protocol == config.protocol)
            .ok_or(MeetingQualificationError::InvalidConfiguration)?;
        if !info.allowed_provider_kinds.contains(&account.kind) {
            return Err(MeetingQualificationError::InvalidConfiguration);
        }
        let base = if account.kind == crate::ProviderKind::PennelCloud {
            self.current_cloud_session().map_or_else(
                || self.cloud_base_url(),
                |session| session.base_url().into(),
            )
        } else {
            account.base_url.clone()
        };
        let endpoint =
            crate::inference::validate_config(&config, &base, crate::InferenceCapability::Decision)
                .map_err(|_| MeetingQualificationError::InvalidConfiguration)?;
        Ok((
            config.provider_id,
            MeetingConfiguredInference {
                protocol: config.protocol,
                endpoint,
                model: config.model,
                upstream: config.upstream,
                expected_model: config.expected_model,
                max_input_bytes: config.max_input_bytes,
                timeout_secs: config.timeout_secs,
            },
        ))
    }

    /// Qualification does not reserve a worker or replace source/configuration cancellation guards.
    pub fn qualify_meeting_analysis(
        &self,
        request: &MeetingQualificationRequest,
    ) -> Result<QualifiedMeetingAnalysis, MeetingQualificationError> {
        let _capture = self.inference_capture_gate();
        let (_, current) = self.meeting_analysis_configuration()?;
        if self.inference_state().lock().unwrap().mutation_depth > 0 {
            return Err(MeetingQualificationError::MutationInProgress);
        }
        if current != request.configured {
            return Err(MeetingQualificationError::ConfigurationChanged);
        }
        MeetingQualificationRegistry::bundled().admit(request)
    }
}

#[uniffi::export]
impl PennelEngine {
    /// Reads configuration and qualification only; never probes a provider or reads meeting text.
    pub fn meeting_analysis_availability(
        &self,
        request_json: String,
    ) -> Result<MeetingAnalysisAvailability, crate::PennelError> {
        if request_json.len() > 16_384 {
            return Err(crate::PennelError::InvalidResponse {
                message: "The meeting analysis configuration is too large.".into(),
            });
        }
        let request: MeetingQualificationRequest =
            serde_json::from_str(&request_json).map_err(|_| {
                crate::PennelError::InvalidResponse {
                    message: "The meeting analysis configuration is invalid.".into(),
                }
            })?;
        let _capture = self.inference_capture_gate();
        let selection = self.meeting_analysis_configuration();
        let mut status = MeetingAnalysisAvailability {
            use_case: request.use_case,
            configured: selection.is_ok(),
            qualified: false,
            available: false,
            reason_code: "unconfigured".into(),
            provider_id: None,
            model: None,
            endpoint: None,
            qualification_id: None,
        };
        let (provider_id, current) = match selection {
            Ok(selection) => selection,
            Err(error) => {
                status.reason_code = error.reason_code().into();
                return Ok(status);
            }
        };
        status.provider_id = Some(provider_id);
        status.model = Some(current.model.clone());
        status.endpoint = Some(current.endpoint.clone());
        let result = if self.inference_state().lock().unwrap().mutation_depth > 0 {
            Err(MeetingQualificationError::MutationInProgress)
        } else if current != request.configured {
            Err(MeetingQualificationError::ConfigurationChanged)
        } else {
            MeetingQualificationRegistry::bundled().admit(&request)
        };
        match result {
            Ok(qualification) => {
                status.qualified = true;
                status.available = true;
                status.reason_code = "ready".into();
                status.qualification_id = Some(qualification.record.qualification_id);
            }
            Err(error) => status.reason_code = error.reason_code().into(),
        }
        Ok(status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct MeetingAnalysisAvailability {
    pub use_case: MeetingAnalysisUseCase,
    pub configured: bool,
    pub qualified: bool,
    pub available: bool,
    pub reason_code: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub qualification_id: Option<String>,
}

#[cfg(test)]
pub(crate) fn test_qualification_for(
    request: &MeetingQualificationRequest,
    actual: MeetingActualInference,
) -> QualifiedMeetingAnalysis {
    test_qualification_with_foreground_impact(request, actual, true)
}

#[cfg(test)]
pub(crate) fn test_qualification_with_foreground_impact(
    request: &MeetingQualificationRequest,
    actual: MeetingActualInference,
    foreground_impact_accepted: bool,
) -> QualifiedMeetingAnalysis {
    let mut record = tests::record();
    record.evidence.foreground_impact_accepted = foreground_impact_accepted;
    record.context.request = request.clone();
    record.context.actual = actual;
    if request.use_case == MeetingAnalysisUseCase::SourceAssessment {
        record.metrics.selective_precision = Some(0.99);
        record.metrics.coverage = Some(0.8);
    }
    MeetingQualificationRegistry {
        records: vec![record],
        valid: true,
    }
    .admit(request)
    .expect("synthetic qualification fixture must satisfy the real gate")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(value: char) -> String {
        std::iter::repeat_n(value, 64).collect()
    }

    fn request() -> MeetingQualificationRequest {
        MeetingQualificationRequest {
            use_case: MeetingAnalysisUseCase::ManualLiveSuggestions,
            language_policy: "nl".into(),
            task: "meeting_suggestions_v1".into(),
            configured: MeetingConfiguredInference {
                protocol: InferenceProtocol::LocalDecider,
                endpoint: "http://127.0.0.1:8080/v1/decisions".into(),
                model: "owned-model".into(),
                upstream: None,
                expected_model: Some("owned-model-v1".into()),
                max_input_bytes: 16_000,
                timeout_secs: 30,
            },
            prompt_sha256: sha('1'),
            schema_sha256: sha('2'),
            label_order_sha256: sha('3'),
            grouping: "bounded_context_v1".into(),
            preprocessing_sha256: sha('4'),
            resource_profile_sha256: sha('5'),
        }
    }

    pub(super) fn record() -> MeetingQualificationRecord {
        MeetingQualificationRecord {
            schema_version: 1,
            qualification_id: "owned-qualified-v1".into(),
            context: MeetingQualificationContext {
                request: request(),
                actual: MeetingActualInference {
                    model: "owned-model-v1".into(),
                    revision: "immutable-v1".into(),
                    provider: "owned-local-worker".into(),
                    artifact_sha256: sha('6'),
                },
                calibration_sha256: sha('7'),
            },
            evidence: MeetingQualificationEvidence {
                mode: "recorded_inference".into(),
                protocol_sha256: sha('8'),
                corpus_sha256: sha('9'),
                split_sha256: sha('a'),
                report_sha256: sha('b'),
                prefix_isolation: true,
                heldout_disjoint: true,
                source_judgments_complete: true,
                failures_counted: true,
                resource_measurements_complete: true,
                foreground_impact_accepted: true,
                background_resource_accepted: true,
            },
            metrics: MeetingQualificationMetrics {
                macro_f1: Some(0.94),
                accepted_item_precision: Some(0.98),
                correct_owner_and_final_status: Some(true),
                selective_precision: None,
                coverage: None,
                warm_p95_ms: Some(250.0),
                peak_rss_bytes: Some(500_000_000),
                cpu_time_ms: Some(120.0),
                calibration_observations: 30,
                calibration_classes: 3,
                calibration_meeting_groups: 4,
                heldout_observations: 30,
            },
        }
    }

    fn registry(record: MeetingQualificationRecord) -> MeetingQualificationRegistry {
        MeetingQualificationRegistry {
            records: vec![record],
            valid: true,
        }
    }

    fn identity() -> InferenceIdentity {
        InferenceIdentity {
            protocol: InferenceProtocol::LocalDecider,
            endpoint: request().configured.endpoint,
            requested_model: "owned-model".into(),
            requested_upstream: None,
            actual_model: Some("owned-model-v1".into()),
            actual_revision: Some("immutable-v1".into()),
            actual_provider: Some("owned-local-worker".into()),
            request_id: Some("ephemeral-request".into()),
        }
    }

    #[test]
    fn production_registry_has_no_adopted_routes() {
        assert!(MeetingQualificationRegistry::bundled().records.is_empty());
        assert_eq!(
            MeetingQualificationRegistry::bundled()
                .admit(&request())
                .unwrap_err(),
            MeetingQualificationError::Unqualified
        );
    }

    #[test]
    fn exact_reviewed_configuration_can_admit_and_rechecks_actual_identity() {
        let admitted = registry(record()).admit(&request()).unwrap();
        assert_eq!(admitted.record().qualification_id, "owned-qualified-v1");
        assert!(admitted.validate_identity(&identity()).is_ok());
        for change in 0..7 {
            let mut changed = identity();
            match change {
                0 => changed.protocol = InferenceProtocol::Mercury,
                1 => changed.endpoint.push_str("/changed"),
                2 => changed.requested_model = "other-alias".into(),
                3 => changed.requested_upstream = Some("other-route".into()),
                4 => changed.actual_model = None,
                5 => changed.actual_revision = Some("new-deployment".into()),
                _ => changed.actual_provider = Some("other-provider".into()),
            }
            assert_eq!(
                admitted.validate_identity(&changed),
                Err(MeetingQualificationError::IdentityChanged)
            );
        }
    }

    #[test]
    fn context_drift_cannot_reuse_a_record() {
        for change in 0..15 {
            let mut changed = request();
            match change {
                0 => changed.use_case = MeetingAnalysisUseCase::ManualFinishedSuggestions,
                1 => changed.language_policy = "en".into(),
                2 => changed.task = "meeting_source_assessment_v1".into(),
                3 => changed.configured.protocol = InferenceProtocol::Mercury,
                4 => changed.configured.endpoint.push_str("/other"),
                5 => changed.configured.model = "different-alias".into(),
                6 => changed.configured.expected_model = None,
                7 => changed.configured.max_input_bytes += 1,
                8 => changed.configured.timeout_secs += 1,
                9 => changed.prompt_sha256 = sha('c'),
                10 => changed.schema_sha256 = sha('c'),
                11 => changed.label_order_sha256 = sha('c'),
                12 => changed.grouping = "new-grouping".into(),
                13 => changed.preprocessing_sha256 = sha('c'),
                _ => changed.resource_profile_sha256 = sha('c'),
            }
            assert!(
                registry(record()).admit(&changed).is_err(),
                "context change {change}"
            );
        }
    }

    #[test]
    fn unavailable_language_and_incomplete_evidence_fail_closed() {
        let mut unknown = request();
        unknown.language_policy = "unknown".into();
        assert_eq!(
            registry(record()).admit(&unknown).unwrap_err(),
            MeetingQualificationError::UnknownLanguage
        );
        for change in 0..18 {
            let mut changed = record();
            match change {
                0 => changed.evidence.mode = "offline_fixture".into(),
                1 => changed.context.actual.revision.clear(),
                2 => changed.context.actual.artifact_sha256.clear(),
                3 => changed.context.calibration_sha256.clear(),
                4 => changed.evidence.report_sha256.clear(),
                5 => changed.evidence.protocol_sha256.clear(),
                6 => changed.evidence.prefix_isolation = false,
                7 => changed.evidence.heldout_disjoint = false,
                8 => changed.evidence.source_judgments_complete = false,
                9 => changed.evidence.failures_counted = false,
                10 => changed.evidence.resource_measurements_complete = false,
                11 => changed.evidence.foreground_impact_accepted = false,
                12 => changed.metrics.calibration_observations = 9,
                13 => changed.metrics.calibration_classes = 1,
                14 => changed.metrics.calibration_meeting_groups = 1,
                15 => changed.metrics.heldout_observations = 0,
                16 => changed.metrics.peak_rss_bytes = None,
                _ => changed.metrics.cpu_time_ms = None,
            }
            assert!(
                registry(changed).admit(&request()).is_err(),
                "evidence change {change}"
            );
        }
    }

    #[test]
    fn live_and_background_qualifications_have_independent_resource_gates() {
        let mut finished = record();
        finished.context.request.use_case = MeetingAnalysisUseCase::ManualFinishedSuggestions;
        finished.evidence.foreground_impact_accepted = false;
        finished.evidence.background_resource_accepted = false;
        assert!(
            registry(finished.clone())
                .admit(&finished.context.request)
                .is_ok()
        );
        let mut background = record();
        background.context.request.use_case = MeetingAnalysisUseCase::BackgroundLiveSuggestions;
        background.evidence.background_resource_accepted = false;
        assert!(
            registry(background.clone())
                .admit(&background.context.request)
                .is_err()
        );
        background.evidence.background_resource_accepted = true;
        assert!(
            registry(background.clone())
                .admit(&background.context.request)
                .is_ok()
        );
    }

    #[test]
    fn quality_and_local_latency_are_required_per_task() {
        for change in 0..7 {
            let mut changed = record();
            match change {
                0 => changed.metrics.macro_f1 = Some(0.89),
                1 => changed.metrics.accepted_item_precision = Some(0.949),
                2 => changed.metrics.correct_owner_and_final_status = Some(false),
                3 => changed.metrics.warm_p95_ms = Some(5_001.0),
                4 => changed.metrics.warm_p95_ms = None,
                5 => changed.metrics.macro_f1 = Some(f64::NAN),
                _ => changed.metrics.cpu_time_ms = Some(0.0),
            }
            assert!(
                registry(changed).admit(&request()).is_err(),
                "quality change {change}"
            );
        }
        let mut source = record();
        source.context.request.use_case = MeetingAnalysisUseCase::SourceAssessment;
        source.context.request.task = "meeting_source_assessment_v1".into();
        source.metrics.selective_precision = Some(0.98);
        source.metrics.coverage = Some(0.5);
        source.metrics.macro_f1 = None;
        source.metrics.accepted_item_precision = None;
        assert!(
            registry(source.clone())
                .admit(&source.context.request)
                .is_ok()
        );
        source.metrics.coverage = Some(0.49);
        assert!(
            registry(source.clone())
                .admit(&source.context.request)
                .is_err()
        );
    }

    #[test]
    fn multiple_deployments_for_one_request_are_ambiguous() {
        let mut newer = record();
        newer.qualification_id = "newer".into();
        newer.context.actual.revision = "newer-deployment".into();
        let records = MeetingQualificationRegistry {
            records: vec![record(), newer],
            valid: true,
        };
        assert_eq!(
            records.admit(&request()).unwrap_err(),
            MeetingQualificationError::AmbiguousQualification
        );
    }

    #[test]
    fn only_reviewed_registry_shape_accepts_records_and_unknown_versions_fail_closed() {
        let asset =
            serde_json::json!({"schema_version":1,"adopted_records":[record()]}).to_string();
        assert!(
            MeetingQualificationRegistry::from_asset(&asset)
                .admit(&request())
                .is_ok()
        );
        let future = asset.replacen("\"schema_version\":1", "\"schema_version\":2", 1);
        assert!(
            MeetingQualificationRegistry::from_asset(&future)
                .admit(&request())
                .is_err()
        );
        let report = serde_json::json!({"schema_version":1,"qualification_records":[record()],"adopted_records":[]}).to_string();
        assert_eq!(
            MeetingQualificationRegistry::from_asset(&report)
                .admit(&request())
                .unwrap_err(),
            MeetingQualificationError::InvalidRegistry
        );
    }

    #[test]
    fn availability_and_verified_connectivity_do_not_qualify_or_send_meeting_content() {
        use std::collections::HashMap;
        let mut server = mockito::Server::new();
        let directory = tempfile::tempdir().unwrap();
        let engine =
            crate::PennelEngine::new_preview(directory.path().to_string_lossy().into_owned())
                .unwrap();
        let mut settings = engine.settings();
        let account = crate::new_account(crate::ProviderKind::Custom, settings.providers.clone());
        let provider_id = account.id.clone();
        settings.providers.push(crate::ProviderAccount {
            base_url: format!("{}/v1", server.url()),
            ..account
        });
        let mut config =
            crate::new_inference_config(provider_id.clone(), InferenceProtocol::Perplexity);
        config.model = "fixture-model".into();
        settings.decision = Some(config.clone());
        engine.update_settings(settings.clone()).unwrap();
        let post = server
            .mock("POST", "/v1/decisions")
            .expect(1)
            .with_status(200)
            .with_body(
                r#"{"model":"fixture-model","answers":{"sample":{"type":"noul","noul":0.99}}}"#,
            )
            .create();
        let metadata = server.mock("GET", "/v1/models").expect(0).create();
        let mut context = request();
        context.configured = MeetingConfiguredInference {
            protocol: config.protocol,
            endpoint: format!("{}/v1/decisions", server.url()),
            model: config.model,
            expected_model: config.expected_model,
            upstream: config.upstream,
            max_input_bytes: config.max_input_bytes,
            timeout_secs: config.timeout_secs,
        };
        let encoded = serde_json::to_string(&context).unwrap();
        let before = engine
            .meeting_analysis_availability(encoded.clone())
            .unwrap();
        assert!(before.configured);
        assert!(!before.available && !before.qualified);
        assert_eq!(before.reason_code, "unqualified");
        let verified = engine
            .verify_inference(
                crate::InferenceCapability::Decision,
                crate::InferenceCancellation::new(),
                crate::ApiKeys {
                    by_provider: HashMap::from([(provider_id, "owned-fixture-key".into())]),
                },
            )
            .unwrap();
        assert!(verified.verified);
        assert_eq!(
            engine
                .meeting_analysis_availability(encoded.clone())
                .unwrap(),
            before
        );
        let mutation = engine.begin_inference_mutation();
        assert_eq!(
            engine
                .meeting_analysis_availability(encoded.clone())
                .unwrap()
                .reason_code,
            "mutation_in_progress"
        );
        mutation.release();
        assert_eq!(engine.settings(), settings);
        let mut changed = engine.settings();
        changed.decision.as_mut().unwrap().max_input_bytes += 1;
        engine.update_settings(changed).unwrap();
        assert_eq!(
            engine
                .meeting_analysis_availability(encoded)
                .unwrap()
                .reason_code,
            "configuration_changed"
        );
        post.assert();
        metadata.assert();
    }

    #[test]
    fn python_incremental_candidate_decodes_without_granting_adoption() {
        let record: MeetingQualificationRecord = serde_json::from_str(include_str!(
            "../../evaluations/decision-models/fixtures/incremental-qualification-record-v1.json"
        ))
        .unwrap();
        assert_eq!(record.evidence.mode, "offline_fixture");
        assert!(!record.valid());
        let request = record.context.request.clone();
        assert!(registry(record).admit(&request).is_err());
        assert!(
            MeetingQualificationRegistry::bundled()
                .admit(&request)
                .is_err()
        );
    }
}
