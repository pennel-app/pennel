//! Frozen support-only abstention. Fitting and state calibration never run in the app.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::meeting_qualification::{MeetingActualInference, compare_inference_identity};
use crate::{
    DecisionQuestion, DecisionQuestionKind, DecisionResult, InferenceIdentity, InferenceProtocol,
    MeetingConfiguredInference, MeetingQualificationRequest,
};

pub(crate) const POLICY: &str = "calibrated_support_raw_state_v1";
pub(crate) const ASSET_SHA256: &str =
    "a75e758259bab834cc848760c122acc5a9dbd990461749010c063886006e4c1f";
const ARTIFACT_SHA256: &str = "d7c5d03f73edfafcbeb15c8602196025ffa19ec033298282670eea469d8b51ee";
const SOURCE_MANIFEST: &str = "6a678edd97a67e2bae84734ffbffc72f4e938fdd2652c5059cf7d8193375ddd9";
const FITTED_FREEZE: &str = "ebee7a9eceb6478390998984e84df25417c115abaf5c33fc767b71d1b98b63b7";
const FITTED_COMMIT: &str = "0d4dcfa69c07b84769729e4dee97c42a83fe7436";
const FITTED_LOCK: &str = "ffb09545cbc373fedf571cae928f5d5d29ea2d1b7d98e85bebeb3699698590a0";
const CALIBRATION_RUNS: &str = "d63ea44267fc215c801967268e94d6debd7ed9b8ee5b977a6a41d039a1726c35";
const ASSET: &str = include_str!("../../../assets/qualifications/meeting-support-cloud-v1.json");
const LABELS: [&str; 3] = ["supported", "absent", "unclear"];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    source_manifest_sha256: String,
    fitted_freeze_sha256: String,
    fitted_lock_commit: String,
    fitted_lock_sha256: String,
    calibration_runs_sha256: String,
    calibration_artifact_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    configured: MeetingConfiguredInference,
    actual: MeetingActualInference,
    language_policy: String,
    grouping: String,
    input_prompt_sha256: String,
    input_label_order_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    schema_version: u32,
    runtime_policy: String,
    provenance: Provenance,
    binding: Binding,
    calibration_json: String,
    qualification: bool,
    production_admission: bool,
}

#[derive(Deserialize)]
struct RawArtifact {
    fits: HashMap<String, RawEntry>,
}
#[derive(Deserialize)]
struct RawEntry {
    fit: Option<RawFit>,
}
#[derive(Deserialize)]
struct RawFit {
    parameters: RawParameters,
    threshold: RawThreshold,
}
#[derive(Deserialize)]
struct RawParameters {
    temperature: Box<serde_json::value::RawValue>,
}
#[derive(Deserialize)]
struct RawThreshold {
    threshold: Option<Box<serde_json::value::RawValue>>,
}

fn exact_float(raw: &serde_json::value::RawValue) -> Result<f64, ()> {
    raw.get()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(())
}

struct Fit {
    context: Value,
    temperature: f64,
    threshold: Option<f64>,
}

pub(crate) struct SupportCalibration {
    binding: Binding,
    #[cfg(any(test, feature = "evaluation"))]
    provenance: Provenance,
    fits: HashMap<String, Fit>,
}

#[cfg(any(test, feature = "evaluation"))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct SupportDecision {
    question_id: String,
    context_sha256: String,
    raw_choice: Option<String>,
    calibrated_choice: String,
    calibrated_probability: f64,
    threshold: Option<f64>,
    accepted: bool,
    reason: &'static str,
}

pub(crate) struct GatedResult {
    /// A private decoding copy. The caller retains the original result for diagnostics.
    pub result: DecisionResult,
    #[cfg(any(test, feature = "evaluation"))]
    pub decisions: Vec<SupportDecision>,
}

pub(crate) fn required(configured: &MeetingConfiguredInference) -> bool {
    configured.protocol == InferenceProtocol::PennelCloud
        && configured.model == crate::inference::MANAGED_REVIEW_MODEL
}

fn digest(value: &Value) -> String {
    crate::evidence_revision(&value.to_string())
}

impl SupportCalibration {
    pub(crate) fn bundled() -> Result<&'static Self, ()> {
        static GATE: OnceLock<Result<SupportCalibration, ()>> = OnceLock::new();
        GATE.get_or_init(|| Self::parse(ASSET, ASSET_SHA256))
            .as_ref()
            .map_err(|_| ())
    }

    fn parse(raw: &str, expected_sha256: &str) -> Result<Self, ()> {
        if raw.len() > 2 * 1024 * 1024 || crate::evidence_revision(raw) != expected_sha256 {
            return Err(());
        }
        let asset: Asset = serde_json::from_str(raw).map_err(|_| ())?;
        let provenance = &asset.provenance;
        if asset.schema_version != 1 || asset.runtime_policy != POLICY
            || asset.qualification || asset.production_admission
            || provenance.calibration_artifact_sha256 != ARTIFACT_SHA256
            || provenance.source_manifest_sha256 != SOURCE_MANIFEST
            || provenance.fitted_freeze_sha256 != FITTED_FREEZE
            || provenance.fitted_lock_commit != FITTED_COMMIT
            || provenance.fitted_lock_sha256 != FITTED_LOCK
            || provenance.calibration_runs_sha256 != CALIBRATION_RUNS
            // Hash the original Python canonical bytes, never a Rust float reserialization.
            || crate::evidence_revision(&asset.calibration_json) != ARTIFACT_SHA256
        {
            return Err(());
        }
        let artifact: Value = serde_json::from_str(&asset.calibration_json).map_err(|_| ())?;
        let scalars: RawArtifact = serde_json::from_str(&asset.calibration_json).map_err(|_| ())?;
        if artifact["format_version"] != 1
            || artifact["fit_split"] != "calibration"
            || artifact["run_manifest_hash"] != SOURCE_MANIFEST
            || artifact["minimum_calibration_samples"] != 10
            || artifact["minimum_observed_classes"] != 2
            || artifact["minimum_meeting_groups"] != 2
        {
            return Err(());
        }
        let mut fits = HashMap::new();
        for (key, entry) in artifact["fits"].as_object().ok_or(())? {
            let context = &entry["context"];
            let subtask = context["subtask"].as_str().ok_or(())?;
            if !subtask.ends_with("_support") {
                continue;
            }
            let fit = &entry["fit"];
            if entry["status"] != "fitted"
                || fit["status"] != "fitted"
                || entry["sample_count"].as_u64().is_none_or(|n| n < 10)
                || entry["observed_class_count"].as_u64().is_none_or(|n| n < 2)
                || entry["meeting_group_count"].as_u64().is_none_or(|n| n < 2)
                || entry["threshold_target_precision"] != 0.95
                || fit["format_version"] != 1
                || fit["kind"] != "temperature"
                || fit["calibration_split"] != "calibration"
                || fit["threshold_scope"] != "all_class_predictions"
                || fit["context_hash"].as_str() != Some(key)
                || &fit["context"] != context
                || digest(context) != *key
                || context["labels"] != json!(LABELS)
                || context["task"] != "events"
                || context["language"] != "nl"
                || context["runner_id"] != "production-finished"
                || context["grouping"] != asset.binding.grouping
                || !super::EVENT_KINDS
                    .iter()
                    .any(|(kind, _)| subtask == format!("{kind}_support"))
            {
                return Err(());
            }
            let exact = scalars
                .fits
                .get(key)
                .and_then(|entry| entry.fit.as_ref())
                .ok_or(())?;
            let temperature = exact_float(&exact.parameters.temperature)?;
            let threshold = exact
                .threshold
                .threshold
                .as_deref()
                .map(exact_float)
                .transpose()?;
            if !(0.05..=20.0).contains(&temperature)
                || threshold.is_some_and(|value| !(0.0..=1.0).contains(&value))
            {
                return Err(());
            }
            fits.insert(
                key.clone(),
                Fit {
                    context: context.clone(),
                    temperature,
                    threshold,
                },
            );
        }
        if fits.len() != 144 {
            return Err(());
        }
        let gate = Self {
            binding: asset.binding,
            #[cfg(any(test, feature = "evaluation"))]
            provenance: asset.provenance,
            fits,
        };
        if !required(&gate.binding.configured)
            || gate.binding.language_policy != "nl"
            || gate.binding.configured.endpoint != "https://api.pennel.app/v1/decisions"
        {
            return Err(());
        }
        Ok(gate)
    }

    #[cfg(feature = "evaluation")]
    pub(crate) fn metadata(&self) -> Value {
        json!({"runtime_policy":POLICY,"asset_sha256":ASSET_SHA256,
            "artifact_sha256":self.provenance.calibration_artifact_sha256,
            "source_manifest_sha256":self.provenance.source_manifest_sha256,
            "run_manifest_hash":self.provenance.source_manifest_sha256,
            "fitted_freeze_sha256":self.provenance.fitted_freeze_sha256,
            "fitted_lock_commit":self.provenance.fitted_lock_commit,
            "fitted_lock_sha256":self.provenance.fitted_lock_sha256,
            "calibration_runs_sha256":self.provenance.calibration_runs_sha256,
            "configured_identity":self.binding.configured,"actual_identity":self.binding.actual})
    }

    pub(crate) fn validate_request(&self, request: &MeetingQualificationRequest) -> Result<(), ()> {
        if request.configured != self.binding.configured
            || request.language_policy != self.binding.language_policy
            || request.grouping != self.binding.grouping
            || request.prompt_sha256 != self.binding.input_prompt_sha256
            || request.label_order_sha256 != self.binding.input_label_order_sha256
        {
            return Err(());
        }
        Ok(())
    }

    pub(crate) fn validate_actual(&self, identity: &InferenceIdentity) -> Result<(), ()> {
        compare_inference_identity(&self.binding.configured, &self.binding.actual, identity)
            .map_err(|_| ())?;
        if identity.actual_revision.as_deref().unwrap_or("") != self.binding.actual.revision {
            return Err(());
        }
        Ok(())
    }

    pub(crate) fn validate_pin(&self, actual: &MeetingActualInference) -> Result<(), ()> {
        (actual == &self.binding.actual).then_some(()).ok_or(())
    }

    fn context(&self, question: &DecisionQuestion, physical_context: &str) -> Value {
        let mut identity =
            serde_json::to_value(&self.binding.actual).expect("owned calibration identity");
        identity["transport"] = json!(self.binding.configured.protocol);
        json!({"model":self.binding.actual.model,"transport":self.binding.configured.protocol,
            "upstream":self.binding.actual.provider,"precision":"unreported",
            "prompt_hash":digest(&json!({"question":question,"physical_prompt_context_sha256":physical_context})),
            "labels":question.options.iter().map(|option| &option.id).collect::<Vec<_>>(),
            "grouping":self.binding.grouping,"identity":identity,"task":"events",
            "subtask":question.id,"language":self.binding.language_policy,"runner_id":"production-finished"})
    }

    fn fit(
        &self,
        question: &DecisionQuestion,
        physical_context: &str,
    ) -> Result<(String, &Fit), ()> {
        if question.kind != DecisionQuestionKind::Choice || !question.id.ends_with("_support") {
            return Err(());
        }
        let context = self.context(question, physical_context);
        let key = digest(&context);
        let fit = self.fits.get(&key).ok_or(())?;
        if fit.context != context {
            return Err(());
        }
        Ok((key, fit))
    }

    pub(crate) fn validate_window(
        &self,
        questions: &[DecisionQuestion],
        physical_context: &str,
    ) -> Result<(), ()> {
        for (kind, _) in super::EVENT_KINDS {
            let question = questions
                .iter()
                .find(|question| question.id == format!("{kind}_support"))
                .ok_or(())?;
            self.fit(question, physical_context)?;
        }
        Ok(())
    }

    pub(crate) fn apply(
        &self,
        questions: &[DecisionQuestion],
        physical_context: &str,
        raw: &DecisionResult,
    ) -> Result<GatedResult, ()> {
        self.validate_actual(&raw.identity)?;
        self.validate_window(questions, physical_context)?;
        let mut result = raw.clone();
        #[cfg(any(test, feature = "evaluation"))]
        let mut decisions = Vec::new();
        for (kind, _) in super::EVENT_KINDS {
            let id = format!("{kind}_support");
            let question = questions
                .iter()
                .find(|question| question.id == id)
                .ok_or(())?;
            let (_key, fit) = self.fit(question, physical_context)?;
            let answer = raw
                .answers
                .iter()
                .find(|answer| answer.id == id)
                .ok_or(())?;
            if answer.kind != DecisionQuestionKind::Choice || answer.probabilities.len() != 3 {
                return Err(());
            }
            let mut probabilities = [0.0; 3];
            for (index, label) in LABELS.iter().enumerate() {
                let values = answer
                    .probabilities
                    .iter()
                    .filter(|value| value.option_id == *label)
                    .collect::<Vec<_>>();
                if values.len() != 1 {
                    return Err(());
                }
                probabilities[index] = values[0].probability;
            }
            let calibrated = temperature(probabilities, fit.temperature)?;
            // Python max(sorted(distribution), key=...) keeps the lexicographically first tie.
            let mut selected = 1; // absent, supported, unclear in lexical order
            for index in [0, 2] {
                if calibrated[index] > calibrated[selected] {
                    selected = index;
                }
            }
            let (accepted, _reason) = if answer.abstained {
                (false, "model_abstained")
            } else if answer.choice.as_deref() == Some("unclear") {
                (false, "raw_unclear")
            } else if answer.choice.as_deref() != Some(LABELS[selected]) {
                (false, "choice_not_argmax")
            } else if fit
                .threshold
                .is_some_and(|threshold| calibrated[selected] >= threshold)
            {
                (true, "accepted")
            } else {
                (false, "below_frozen_threshold")
            };
            if !accepted {
                result
                    .answers
                    .iter_mut()
                    .find(|answer| answer.id == id)
                    .ok_or(())?
                    .abstained = true;
            }
            #[cfg(any(test, feature = "evaluation"))]
            decisions.push(SupportDecision {
                question_id: id,
                context_sha256: _key,
                raw_choice: answer.choice.clone(),
                calibrated_choice: LABELS[selected].into(),
                calibrated_probability: calibrated[selected],
                threshold: fit.threshold,
                accepted,
                reason: _reason,
            });
        }
        Ok(GatedResult {
            result,
            #[cfg(any(test, feature = "evaluation"))]
            decisions,
        })
    }
}

/// Positive finite summands using the same error-free partials and final rounding as math.fsum.
fn fsum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut partials = Vec::<f64>::new();
    for mut value in values {
        let mut count = 0;
        for index in 0..partials.len() {
            let mut other = partials[index];
            if value.abs() < other.abs() {
                std::mem::swap(&mut value, &mut other);
            }
            let high = value + other;
            let low = other - (high - value);
            if low != 0.0 {
                partials[count] = low;
                count += 1;
            }
            value = high;
        }
        partials.truncate(count);
        partials.push(value);
    }
    let mut high = partials.pop().unwrap_or(0.0);
    let mut low = 0.0;
    while let Some(value) = partials.pop() {
        let previous = high;
        high = previous + value;
        low = value - (high - previous);
        if low != 0.0 {
            break;
        }
    }
    if let Some(last) = partials.last()
        && ((low < 0.0 && *last < 0.0) || (low > 0.0 && *last > 0.0))
    {
        let doubled = low * 2.0;
        let rounded = high + doubled;
        if doubled == rounded - high {
            high = rounded;
        }
    }
    high
}

fn temperature(probabilities: [f64; 3], temperature: f64) -> Result<[f64; 3], ()> {
    if probabilities
        .iter()
        .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
        || (probabilities.iter().sum::<f64>() - 1.0).abs() > 1e-6
        || !temperature.is_finite()
        || temperature <= 0.0
    {
        return Err(());
    }
    let logits = probabilities.map(|p| p.max(1e-12).ln() / temperature);
    let maximum = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let weights = logits.map(|value| (value - maximum).exp());
    let total = fsum(weights);
    Ok(weights.map(|value| value / total))
}

#[cfg(any(test, feature = "evaluation"))]
pub(crate) fn exact_json(raw: &str) -> Result<Value, ()> {
    if raw.len() > 32 * 1024 * 1024 {
        return Err(());
    }
    crate::inference::parse_unique_json(raw).map_err(|_| ())?;
    fn convert(raw: &serde_json::value::RawValue, depth: usize) -> Result<Value, ()> {
        if depth > 48 {
            return Err(());
        }
        let text = raw.get();
        match text.as_bytes().first().copied().ok_or(())? {
            b'{' => {
                let values: HashMap<String, Box<serde_json::value::RawValue>> =
                    serde_json::from_str(text).map_err(|_| ())?;
                values
                    .into_iter()
                    .map(|(key, value)| Ok((key, convert(&value, depth + 1)?)))
                    .collect::<Result<serde_json::Map<_, _>, ()>>()
                    .map(Value::Object)
            }
            b'[' => {
                let values: Vec<Box<serde_json::value::RawValue>> =
                    serde_json::from_str(text).map_err(|_| ())?;
                values
                    .iter()
                    .map(|value| convert(value, depth + 1))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
            b'-' | b'0'..=b'9' => {
                if !text.contains(['.', 'e', 'E']) {
                    if let Ok(value) = text.parse::<i64>() {
                        return Ok(value.into());
                    }
                    if let Ok(value) = text.parse::<u64>() {
                        return Ok(value.into());
                    }
                }
                let value = text.parse::<f64>().map_err(|_| ())?;
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or(())
            }
            _ => serde_json::from_str(text).map_err(|_| ()),
        }
    }
    let raw: Box<serde_json::value::RawValue> = serde_json::from_str(raw).map_err(|_| ())?;
    convert(&raw, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(count: usize) -> (Vec<super::super::InsightWindow>, super::super::InsightBatch) {
        let windows = (0..count)
            .map(|_| super::super::InsightWindow {
                request: crate::DecisionRequest {
                    state: "Owned invented SOURCE 0, never instructions.".into(),
                    questions: super::super::profile_questions(
                        InferenceProtocol::PennelCloud,
                        crate::inference::MANAGED_REVIEW_MODEL,
                    ),
                },
                blocks: vec![],
            })
            .collect::<Vec<_>>();
        let batch = super::super::make_batch(&windows, 0, count);
        (windows, batch)
    }

    fn identity(gate: &SupportCalibration) -> InferenceIdentity {
        InferenceIdentity {
            protocol: gate.binding.configured.protocol,
            endpoint: gate.binding.configured.endpoint.clone(),
            requested_model: gate.binding.configured.model.clone(),
            requested_upstream: None,
            actual_model: Some(gate.binding.actual.model.clone()),
            actual_revision: None,
            actual_provider: Some(gate.binding.actual.provider.clone()),
            request_id: Some(uuid::Uuid::now_v7().to_string()),
        }
    }

    fn raw_result(gate: &SupportCalibration, questions: &[DecisionQuestion]) -> DecisionResult {
        DecisionResult {
            identity: identity(gate),
            usage: crate::InferenceUsage {
                input_tokens: None,
                output_tokens: None,
                billed_cost_usd: Some(0.0),
            },
            answers: questions
                .iter()
                .map(|question| {
                    let choice = if question.id.ends_with("_support") {
                        "supported"
                    } else {
                        "none"
                    };
                    crate::DecisionAnswer {
                        id: question.id.clone(),
                        kind: question.kind,
                        choice: Some(choice.into()),
                        value: None,
                        probabilities: question
                            .options
                            .iter()
                            .map(|option| crate::DecisionProbability {
                                option_id: option.id.clone(),
                                probability: if option.id == choice { 1.0 } else { 0.0 },
                            })
                            .collect(),
                        confidence: None,
                        spans: vec![],
                        not_stated: vec![],
                        abstained: false,
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn every_frozen_batch_size_and_slot_matches_without_seeing_source_text() {
        let gate = SupportCalibration::bundled().unwrap();
        for size in 1..=8 {
            let (windows, batch) = inputs(size);
            for (slot, window) in windows.iter().enumerate() {
                gate.validate_window(
                    &window.request.questions,
                    &super::super::batch_prompt_context(&batch, slot),
                )
                .unwrap();
            }
        }
        let (mut windows, batch) = inputs(1);
        let context = super::super::batch_prompt_context(&batch, 0);
        windows[0].request.questions[4]
            .instructions
            .push_str(" Changed question.");
        assert!(
            gate.validate_window(&windows[0].request.questions, &context)
                .is_err()
        );
    }

    #[test]
    fn endpoint_configuration_identity_and_language_changes_never_fall_back_to_raw() {
        let gate = SupportCalibration::bundled().unwrap();
        let request = super::super::qualification_request(gate.binding.configured.clone(), "nl");
        gate.validate_request(&request).unwrap();
        for change in 0..8 {
            let mut changed = request.clone();
            match change {
                0 => {
                    changed.configured.endpoint =
                        "https://api-staging.pennel.app/v1/decisions".into()
                }
                1 => changed.configured.model = "pennel-auto".into(),
                2 => changed.configured.expected_model = Some("changed-version".into()),
                3 => changed.configured.timeout_secs -= 1,
                4 => changed.configured.max_input_bytes -= 1,
                5 => changed.language_policy = "en".into(),
                6 => changed.grouping.push('x'),
                _ => changed.label_order_sha256 = "0".repeat(64),
            }
            assert!(gate.validate_request(&changed).is_err(), "change {change}");
        }
        for change in 0..4 {
            let mut changed = identity(gate);
            match change {
                0 => changed.actual_model = Some("configured-alias-is-not-serving-identity".into()),
                1 => changed.actual_provider = Some("Other provider".into()),
                2 => changed.actual_revision = Some("newly-reported-serving-revision".into()),
                _ => changed.endpoint = "https://api-staging.pennel.app/v1/decisions".into(),
            }
            assert!(gate.validate_actual(&changed).is_err());
        }
        let mut unmeasured = request;
        unmeasured.resource_profile_sha256 = "d".repeat(64);
        assert!(matches!(
            crate::MeetingQualificationRegistry::bundled().admit(&unmeasured),
            Err(crate::MeetingQualificationError::Unqualified)
        ));
    }

    #[test]
    fn support_gate_only_abstains_and_leaves_all_raw_answers_states_and_probabilities_untouched() {
        let gate = SupportCalibration::bundled().unwrap();
        let (windows, batch) = inputs(1);
        let mut raw = raw_result(gate, &windows[0].request.questions);
        let decision = raw
            .answers
            .iter_mut()
            .find(|answer| answer.id == "decision_support")
            .unwrap();
        decision.choice = Some("absent".into());
        for probability in &mut decision.probabilities {
            probability.probability = if probability.option_id == "absent" {
                1.0
            } else {
                0.0
            };
        }
        let question = raw
            .answers
            .iter_mut()
            .find(|answer| answer.id == "open_question_support")
            .unwrap();
        question.choice = Some("unclear".into());
        for probability in &mut question.probabilities {
            probability.probability = if probability.option_id == "unclear" {
                1.0
            } else {
                0.0
            };
        }
        raw.answers
            .iter_mut()
            .find(|answer| answer.id == "changed_commitment_support")
            .unwrap()
            .abstained = true;
        let original = raw.clone();
        let gated = gate
            .apply(
                &windows[0].request.questions,
                &super::super::batch_prompt_context(&batch, 0),
                &raw,
            )
            .unwrap();
        assert_eq!(raw, original);
        for (before, after) in raw.answers.iter().zip(&gated.result.answers) {
            assert_eq!(before.choice, after.choice);
            assert_eq!(before.probabilities, after.probabilities);
            if !before.id.ends_with("_support") {
                assert_eq!(before, after);
            }
        }
        assert_eq!(
            gated
                .decisions
                .iter()
                .map(|decision| decision.accepted)
                .collect::<Vec<_>>(),
            [true, true, false, false]
        );
        let mut uncertain = original;
        let action = uncertain
            .answers
            .iter_mut()
            .find(|answer| answer.id == "action_support")
            .unwrap();
        for probability in &mut action.probabilities {
            probability.probability = if probability.option_id == "supported" {
                0.34
            } else {
                0.33
            };
        }
        let gated = gate
            .apply(
                &windows[0].request.questions,
                &super::super::batch_prompt_context(&batch, 0),
                &uncertain,
            )
            .unwrap();
        assert!(!gated.decisions[0].accepted);
        assert!(
            gated
                .result
                .answers
                .iter()
                .find(|answer| answer.id == "action_support")
                .unwrap()
                .abstained
        );
        assert!(
            !uncertain
                .answers
                .iter()
                .find(|answer| answer.id == "action_support")
                .unwrap()
                .abstained
        );
    }

    #[cfg(feature = "evaluation")]
    #[test]
    fn replay_uses_the_production_gate_and_preserves_raw_diagnostic_distributions() {
        use crate::{
            MeetingTranscriptSnapshot, MeetingUtterance, Segment, Speaker, TranscriptMetadataStatus,
        };
        let gate = SupportCalibration::bundled().unwrap();
        let mut row = MeetingUtterance::new(
            Segment {
                start_ms: 0,
                end_ms: 1000,
                speaker: Speaker::Others,
                text: "Deze verzonnen kaart ligt op de tafel.".into(),
                failed: false,
                voice: None,
            },
            None,
        );
        row.metadata.utterance_id = uuid::Uuid::now_v7().to_string();
        let source = MeetingTranscriptSnapshot {
            meeting_uid: uuid::Uuid::now_v7().to_string(),
            revision: 1,
            transcript_digest: "a".repeat(64),
            metadata_digest: None,
            metadata_status: TranscriptMetadataStatus::Valid,
            utterances: vec![row],
        };
        let prepared = super::super::evaluation::Prepared::new(
            source,
            gate.binding.configured.clone(),
            "nl".into(),
        )
        .unwrap();
        let exported = prepared.export().unwrap();
        assert_eq!(exported["runtime_policy"], POLICY);
        assert_eq!(
            exported["support_calibration"]["asset_sha256"],
            ASSET_SHA256
        );
        let request = prepared.batch_request(0).unwrap();
        let mut answers = serde_json::Map::new();
        for question in &request.questions {
            let choice = if question.id == "action_support" {
                "supported"
            } else if question.id.ends_with("_support") {
                "absent"
            } else if question.id == "action" {
                "confirmed"
            } else {
                "none"
            };
            let probabilities = question
                .options
                .iter()
                .map(|option| {
                    let p = if question.id == "action_support" {
                        if option.id == "supported" { 0.34 } else { 0.33 }
                    } else if option.id == choice {
                        1.0
                    } else {
                        0.0
                    };
                    (option.id.clone(), json!(p))
                })
                .collect::<serde_json::Map<_, _>>();
            answers.insert(
                question.id.clone(),
                json!({"type":"choice","choice":choice,"probabilities":probabilities}),
            );
        }
        let id = uuid::Uuid::now_v7().to_string();
        let response = json!({"schema_version":1,"request_id":id,"profile":"supported_passages_v3",
            "id":"gen-owned-upstream-request-id","model":gate.binding.actual.model,"provider":gate.binding.actual.provider,"answers":answers});
        let configured = &gate.binding.configured;
        let config = crate::InferenceConfig {
            provider_id: "owned-audit-fixture".into(),
            protocol: configured.protocol,
            model: configured.model.clone(),
            expected_model: configured.expected_model.clone(),
            upstream: None,
            max_input_bytes: configured.max_input_bytes,
            timeout_secs: configured.timeout_secs,
        };
        let typed = crate::inference::evaluation_managed_decision_value(
            &config,
            configured.endpoint.clone(),
            &request,
            &response,
            &id,
        )
        .unwrap();
        let audited = prepared.replay_observed_managed_result(
            0,
            typed.clone(),
            &response,
            Some(&gate.binding.actual),
            Some(&id),
        );
        assert_eq!(audited["status"], "ok");
        assert_eq!(
            audited["result"]["identity"]["request_id"],
            "gen-owned-upstream-request-id"
        );
        let captured = exact_json(&response.to_string()).unwrap();
        let recaptured = prepared.replay_observed_managed_result(
            0,
            typed.clone(),
            &captured,
            Some(&gate.binding.actual),
            Some(&id),
        );
        assert_eq!(audited, recaptured);
        let mut fabricated = typed;
        fabricated.answers[0].choice = Some("tentative".into());
        let rejected = prepared.replay_observed_managed_result(
            0,
            fabricated,
            &response,
            Some(&gate.binding.actual),
            Some(&id),
        );
        assert_eq!(rejected["status"], "typed_response_capture_mismatch");
        let replay = prepared.replay_with_request_id(
            0,
            &serde_json::to_vec(&response).unwrap(),
            Some(&gate.binding.actual),
            Some(&id),
        );
        assert_eq!(replay["status"], "ok");
        let window = &replay["windows"][0];
        assert!(window["candidates"].as_array().unwrap().is_empty());
        let action = window["result"]["answers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|answer| answer["id"] == "action")
            .unwrap();
        assert_eq!(action["choice"], "confirmed");
        let support = window["result"]["answers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|answer| answer["id"] == "action_support")
            .unwrap();
        assert_eq!(support["choice"], "supported");
        assert_eq!(support["abstained"], false);
        assert_eq!(window["support_gate"][0]["accepted"], false);
        assert!(
            !window["assessed_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|kind| kind == "action")
        );
    }

    #[test]
    fn immutable_asset_is_bound_to_the_original_precommitted_calibration_only() {
        let gate = SupportCalibration::bundled().unwrap();
        assert_eq!(gate.fits.len(), 144);
        assert_eq!(gate.provenance.calibration_artifact_sha256, ARTIFACT_SHA256);
        assert!(SupportCalibration::parse("", ASSET_SHA256).is_err());
        let mut asset: Value = serde_json::from_str(ASSET).unwrap();
        for field in ["calibration_json", "binding", "provenance"] {
            let mut changed = asset.clone();
            changed[field] = Value::Null;
            let raw = changed.to_string();
            assert!(SupportCalibration::parse(&raw, &crate::evidence_revision(&raw)).is_err());
        }
        asset["provenance"]["source_manifest_sha256"] = "0".repeat(64).into();
        let raw = asset.to_string();
        assert!(SupportCalibration::parse(&raw, &crate::evidence_revision(&raw)).is_err());
    }

    #[test]
    fn numerical_results_and_threshold_boundaries_match_precommitted_python_fixtures() {
        let fixture: Value =
            serde_json::from_str(include_str!("support_calibration_cases.json")).unwrap();
        let gate = SupportCalibration::bundled().unwrap();
        assert_eq!(fixture["source_artifact_sha256"], ARTIFACT_SHA256);
        for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            // JSON bit patterns avoid serde_json's optional fast decimal rounding path.
            let probabilities = LABELS
                .map(|label| f64::from_bits(case["probability_bits"][label].as_u64().unwrap()));
            let t = f64::from_bits(case["temperature_bits"].as_u64().unwrap());
            let threshold = f64::from_bits(case["threshold_bits"].as_u64().unwrap());
            let frozen_fit = &gate.fits[case["context_sha256"].as_str().unwrap()];
            assert_eq!(frozen_fit.temperature.to_bits(), t.to_bits());
            assert_eq!(frozen_fit.threshold.unwrap().to_bits(), threshold.to_bits());
            let calibrated = temperature(probabilities, t).unwrap();
            let mut selected = 1;
            for slot in [0, 2] {
                if calibrated[slot] > calibrated[selected] {
                    selected = slot;
                }
            }
            assert_eq!(
                LABELS[selected],
                case["expected_choice"].as_str().unwrap(),
                "case {index}"
            );
            for slot in 0..3 {
                assert_eq!(
                    calibrated[slot].to_bits(),
                    case["expected_probability_bits"][LABELS[slot]]
                        .as_u64()
                        .unwrap(),
                    "case {index} {}",
                    LABELS[slot]
                );
            }
            let accepted = calibrated[selected] >= threshold;
            assert_eq!(
                accepted,
                case["expected_accepted"].as_bool().unwrap(),
                "case {index}"
            );
        }
    }

    #[cfg(feature = "evaluation")]
    #[test]
    fn typed_capture_exact_replay_preserves_every_calibration_gate_decision_at_cutoffs() {
        let fixture: Value =
            serde_json::from_str(include_str!("support_calibration_cases.json")).unwrap();
        let gate = SupportCalibration::bundled().unwrap();
        let mut legacy_float_changes = 0;
        let mut legacy_decision_changes = 0;
        for case in fixture["cases"].as_array().unwrap() {
            let count = case["batch_size"].as_u64().unwrap() as usize;
            let slot = case["batch_slot"].as_u64().unwrap() as usize;
            let (windows, batch) = inputs(count);
            let questions = &windows[slot].request.questions;
            let mut typed = raw_result(gate, &batch.request.questions);
            let original_id = case["question_id"].as_str().unwrap();
            let question_id = if count == 1 {
                original_id.into()
            } else {
                format!("w{slot}_{original_id}")
            };
            let answer = typed
                .answers
                .iter_mut()
                .find(|answer| answer.id == question_id)
                .unwrap();
            for value in &mut answer.probabilities {
                value.probability =
                    f64::from_bits(case["probability_bits"][&value.option_id].as_u64().unwrap());
            }
            let mut selected = "absent";
            for candidate in ["supported", "unclear"] {
                let p = answer
                    .probabilities
                    .iter()
                    .find(|value| value.option_id == candidate)
                    .unwrap()
                    .probability;
                let best = answer
                    .probabilities
                    .iter()
                    .find(|value| value.option_id == selected)
                    .unwrap()
                    .probability;
                if p > best {
                    selected = candidate;
                }
            }
            answer.choice = Some(selected.into());
            let answers=typed.answers.iter().map(|answer| {
                let probabilities=answer.probabilities.iter().map(|p|(p.option_id.clone(),json!(p.probability))).collect::<serde_json::Map<_,_>>();
                (answer.id.clone(),json!({"type":"choice","choice":answer.choice,"probabilities":probabilities}))
            }).collect::<serde_json::Map<_,_>>();
            let response = json!({"schema_version":1,"request_id":typed.identity.request_id,"profile":"supported_passages_v3",
                "model":gate.binding.actual.model,"provider":gate.binding.actual.provider,
                "answers":answers,"usage":{"cost":0.0}});
            let configured = &gate.binding.configured;
            let config = crate::InferenceConfig {
                provider_id: "owned-parity-fixture".into(),
                protocol: configured.protocol,
                model: configured.model.clone(),
                expected_model: configured.expected_model.clone(),
                upstream: None,
                max_input_bytes: configured.max_input_bytes,
                timeout_secs: configured.timeout_secs,
            };
            let typed = crate::inference::evaluation_managed_decision_value(
                &config,
                configured.endpoint.clone(),
                &batch.request,
                &response,
                typed.identity.request_id.as_deref().unwrap(),
            )
            .unwrap();
            let logical = super::super::split_batch_result(&batch, &typed)
                .unwrap()
                .swap_remove(slot)
                .1;
            let context = super::super::batch_prompt_context(&batch, slot);
            let original = gate.apply(questions, &context, &logical).unwrap();
            let captured = serde_json::to_string(&typed).unwrap();
            let replayed: DecisionResult =
                serde_json::from_value(exact_json(&captured).unwrap()).unwrap();
            assert_eq!(typed, replayed);
            let replayed = super::super::split_batch_result(&batch, &replayed)
                .unwrap()
                .swap_remove(slot)
                .1;
            let replayed = gate.apply(questions, &context, &replayed).unwrap();
            assert_eq!(original.decisions, replayed.decisions);
            let legacy: DecisionResult =
                serde_json::from_value(serde_json::from_str::<Value>(&captured).unwrap()).unwrap();
            if legacy != typed {
                legacy_float_changes += 1;
            }
            let legacy = super::super::split_batch_result(&batch, &legacy)
                .unwrap()
                .swap_remove(slot)
                .1;
            if gate
                .apply(questions, &context, &legacy)
                .unwrap()
                .decisions
                .iter()
                .map(|d| d.accepted)
                .collect::<Vec<_>>()
                != original
                    .decisions
                    .iter()
                    .map(|d| d.accepted)
                    .collect::<Vec<_>>()
            {
                legacy_decision_changes += 1;
            }
        }
        eprintln!(
            "calibration-only typed JSON audit: {legacy_float_changes} legacy float drifts, {legacy_decision_changes} legacy gate-decision drifts; exact replay 0"
        );
        assert!(exact_json(r#"{"x":1,"x":2}"#).is_err());
        assert!(exact_json(r#"{"x":1e999}"#).is_err());
    }

    #[test]
    fn invalid_distributions_do_not_get_a_raw_fallback() {
        for probabilities in [
            [0.5, 0.3, 0.3],
            [f64::NAN, 0.5, 0.5],
            [-0.1, 0.5, 0.6],
            [0.0, 0.0, 0.0],
        ] {
            assert!(temperature(probabilities, 1.0).is_err());
        }
        assert!(temperature([0.5, 0.25, 0.25], 0.0).is_err());
    }

    #[test]
    fn exact_capture_parser_preserves_scientific_notation_ulp_boundaries() {
        for text in [
            "9.504618097742133e-07",
            "0.9999989507226293",
            "0.5384632111783747",
            "0.9998837846161159",
        ] {
            let expected = text.parse::<f64>().unwrap();
            let exact = exact_json(text).unwrap().as_f64().unwrap();
            assert_eq!(exact.to_bits(), expected.to_bits());
            let legacy: Value = serde_json::from_str(text).unwrap();
            eprintln!(
                "scalar JSON roundtrip {} ULP drift",
                legacy
                    .as_f64()
                    .unwrap()
                    .to_bits()
                    .abs_diff(expected.to_bits())
            );
        }
    }
}
