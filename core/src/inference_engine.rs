use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};

use crate::inference;
use crate::{
    ApiKeys, DecisionQuestion, DecisionQuestionKind, DecisionRequest, DecisionResult,
    InferenceCancellation, InferenceCapability, InferenceConfig, InferenceIdentity,
    InferenceProtocol, PennelEngine, PennelError, ProviderAccount, ProviderKind, RerankPassage,
    RerankRequest, RerankResult,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct InferenceAvailability {
    pub capability: InferenceCapability,
    pub configured: bool,
    pub available: bool,
    pub reason: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub protocol: Option<InferenceProtocol>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct InferenceVerification {
    pub verified: bool,
    pub message: String,
    pub identity: Option<InferenceIdentity>,
}

#[derive(Default)]
pub(crate) struct InferenceState {
    pub(crate) requests: Vec<Weak<InferenceCancellation>>,
    pub(crate) mutation_depth: usize,
}

/// Keeps inference admission closed throughout a native configuration or credential write.
#[derive(uniffi::Object)]
pub struct InferenceMutation {
    state: Arc<Mutex<InferenceState>>,
    released: AtomicBool,
}

#[uniffi::export]
impl InferenceMutation {
    pub fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.state.lock().unwrap().mutation_depth -= 1;
        }
    }
}

impl Drop for InferenceMutation {
    fn drop(&mut self) {
        self.release();
    }
}

impl PennelEngine {
    pub(crate) fn inference_state(&self) -> Arc<Mutex<InferenceState>> {
        self.inference_requests.clone()
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn cloud_account_epoch(&self) -> u64 {
        self.cloud_session()
            .map_or(0, |session| session.generation())
    }

    pub fn begin_inference_mutation(&self) -> Arc<InferenceMutation> {
        let state = self.inference_state();
        {
            let mut active = state.lock().unwrap();
            active.mutation_depth += 1;
            for request in &active.requests {
                if let Some(cancellation) = request.upgrade() {
                    cancellation.cancel();
                }
            }
        }
        Arc::new(InferenceMutation {
            state,
            released: AtomicBool::new(false),
        })
    }
}

fn invalid(message: &str) -> PennelError {
    PennelError::InvalidResponse {
        message: message.into(),
    }
}

struct InferenceRegistration<'a> {
    engine: &'a PennelEngine,
    cancellation: &'a Arc<InferenceCancellation>,
}

impl Drop for InferenceRegistration<'_> {
    fn drop(&mut self) {
        self.engine.finish_inference(self.cancellation);
    }
}

impl PennelEngine {
    fn inference_selection(
        &self,
        capability: InferenceCapability,
    ) -> Result<(InferenceConfig, ProviderAccount), PennelError> {
        let settings = self.settings();
        let config = match capability {
            InferenceCapability::Decision => settings.decision,
            InferenceCapability::Reranking => settings.reranking,
        }
        .ok_or_else(|| invalid("Configure this optional connection first."))?;
        let account = settings
            .providers
            .into_iter()
            .find(|p| p.id == config.provider_id)
            .ok_or_else(|| invalid("The selected provider is missing."))?;
        let protocol = inference::inference_protocols()
            .into_iter()
            .find(|info| info.protocol == config.protocol)
            .expect("every inference protocol has metadata");
        if !protocol.allowed_provider_kinds.contains(&account.kind) {
            return Err(invalid(
                "Choose a supported provider account for this inference protocol.",
            ));
        }
        inference::validate_config(&config, &self.inference_base_url(&account), capability)?;
        Ok((config, account))
    }

    fn inference_base_url(&self, account: &ProviderAccount) -> String {
        if account.kind != ProviderKind::PennelCloud {
            return account.base_url.clone();
        }
        self.current_cloud_session().as_ref().map_or_else(
            || self.cloud_base_url(),
            |session| session.base_url().into(),
        )
    }

    fn managed_inference(
        &self,
        account: &ProviderAccount,
    ) -> Result<Option<inference::ManagedInference>, PennelError> {
        if account.kind != ProviderKind::PennelCloud {
            return Ok(None);
        }
        let session = self.cloud_session()?;
        if !session.signed_in() {
            return Err(PennelError::NotSignedIn);
        }
        Ok(Some(inference::ManagedInference {
            generation: session.generation(),
            session,
        }))
    }

    fn managed_inference_unchanged(
        &self,
        managed: Option<&inference::ManagedInference>,
    ) -> Result<(), PennelError> {
        if let Some(managed) = managed {
            let slot = self.current_cloud_session();
            if !slot.as_ref().is_some_and(|current| {
                Arc::ptr_eq(current, &managed.session)
                    && current.generation() == managed.generation
                    && current.signed_in()
            }) {
                return Err(PennelError::Cancelled);
            }
        }
        Ok(())
    }

    fn inference_unchanged(
        &self,
        capability: InferenceCapability,
        config: &InferenceConfig,
        account: &ProviderAccount,
        cancellation: &InferenceCancellation,
    ) -> Result<(), PennelError> {
        let unchanged = self
            .inference_selection(capability)
            .is_ok_and(|(current, provider)| current == *config && provider == *account);
        if cancellation.is_cancelled() || !unchanged {
            return Err(PennelError::Cancelled);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;
    use std::collections::HashMap;

    fn configured(
        server: &mockito::Server,
        protocol: InferenceProtocol,
    ) -> (tempfile::TempDir, Arc<PennelEngine>, ApiKeys) {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "typed-fixture".into(),
            kind: ProviderKind::Custom,
            name: "Synthetic server".into(),
            base_url: format!("{}/v1", server.url()),
        });
        let mut config = crate::new_inference_config("typed-fixture".into(), protocol);
        config.model = "fixture-model".into();
        settings.decision = Some(config);
        settings.dictionary = vec!["private-vocabulary-must-not-be-sent".into()];
        engine.update_settings(settings).unwrap();
        let keys = ApiKeys {
            by_provider: HashMap::from([("typed-fixture".into(), "synthetic-typed-key".into())]),
        };
        (dir, engine, keys)
    }

    fn response() -> &'static str {
        r#"{"model":"fixture-model","answers":{"sample":{"type":"noul","noul":0.99}},"usage":{"input_tokens":12,"output_tokens":0,"cost":0.0001}}"#
    }

    #[test]
    fn missing_configuration_and_cancellation_never_enable_a_provider() {
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        for capability in [
            InferenceCapability::Decision,
            InferenceCapability::Reranking,
        ] {
            let availability = engine.inference_availability(capability);
            assert!(!availability.configured);
            assert!(!availability.available);
            assert!(availability.endpoint.is_none());
        }
        let cancel = InferenceCancellation::new();
        cancel.cancel();
        assert_eq!(
            engine.verify_inference(
                InferenceCapability::Decision,
                cancel,
                ApiKeys {
                    by_provider: HashMap::new()
                }
            ),
            Err(PennelError::Cancelled)
        );
    }

    #[test]
    fn synthetic_verification_preserves_choices_and_sends_no_user_vocabulary() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server, InferenceProtocol::Perplexity);
        let before = engine.settings();
        let mock = server.mock("POST", "/v1/decisions")
            .match_header("authorization", "Bearer synthetic-typed-key")
            .match_body(Matcher::Json(serde_json::json!({"model":"fixture-model","state":"Synthetic Pennel sample: the meeting starts at ten.","questions":{"sample":{"type":"noul","instructions":"Does the sample state that the meeting starts at ten?"}}})))
            .with_status(200).with_body(response()).create();
        let result = engine
            .verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                keys,
            )
            .unwrap();
        assert!(result.verified);
        assert_eq!(
            result.identity.unwrap().actual_model.as_deref(),
            Some("fixture-model")
        );
        assert_eq!(engine.settings(), before);
        mock.assert();
    }

    #[test]
    fn a_changed_configuration_cannot_verify_the_new_selection() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server, InferenceProtocol::Perplexity);
        let changed = engine.clone();
        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(200)
            .with_body_from_request(move |_| {
                let mut settings = changed.settings();
                settings.decision.as_mut().unwrap().model = "changed-model".into();
                changed.update_settings(settings).unwrap();
                response().as_bytes().to_vec()
            })
            .create();
        assert_eq!(
            engine.verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                keys
            ),
            Err(PennelError::Cancelled)
        );
        mock.assert();
    }

    #[test]
    fn malformed_requests_and_provider_errors_do_not_echo_content() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server, InferenceProtocol::Perplexity);
        let error = engine
            .decide_json(
                "private input not valid JSON".into(),
                InferenceCancellation::new(),
                keys.clone(),
            )
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private input"));
        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(401)
            .with_body("private transcript synthetic-typed-key")
            .create();
        let result = engine
            .verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                keys,
            )
            .unwrap();
        assert!(!result.verified);
        assert!(!result.message.contains("private transcript"));
        assert!(!result.message.contains("synthetic-typed-key"));
        mock.assert();
    }

    #[test]
    fn settings_upgrades_and_account_removal_do_not_select_another_connection() {
        let mut server = mockito::Server::new();
        let (_dir, engine, _) = configured(&server, InferenceProtocol::Perplexity);
        let settings = engine.settings();
        let mut previous = serde_json::to_value(&settings).unwrap();
        previous.as_object_mut().unwrap().remove("decision");
        previous.as_object_mut().unwrap().remove("reranking");
        let migrated: crate::Settings = serde_json::from_value(previous).unwrap();
        assert!(migrated.decision.is_none());
        assert!(migrated.reranking.is_none());
        assert_eq!(migrated.llm, settings.llm);
        assert_eq!(migrated.dictation_stt, settings.dictation_stt);
        assert_eq!(migrated.meeting_stt, settings.meeting_stt);
        let mut removed = settings.clone();
        removed.providers.retain(|p| p.id != "typed-fixture");
        let removed = removed.normalized();
        assert!(removed.decision.is_none());
        assert!(removed.reranking.is_none());
        assert_eq!(removed.llm, settings.llm);
        assert_eq!(
            removed.providers,
            migrated
                .providers
                .into_iter()
                .filter(|p| p.id != "typed-fixture")
                .collect::<Vec<_>>()
        );
        // The account is gone, so no later operation can send a sample to another account.
        engine.update_settings(removed).unwrap();
        server
            .mock("POST", "/v1/decisions")
            .expect(0)
            .create()
            .assert();
    }

    #[test]
    fn mobile_patch_roundtrips_optional_configuration_without_secrets() {
        let server = mockito::Server::new();
        let (_dir, engine, _) = configured(&server, InferenceProtocol::Perplexity);
        let serialized = engine.mobile_settings_json().unwrap();
        assert!(serialized.contains("fixture-model"));
        assert!(!serialized.contains("synthetic-typed-key"));
        let updated = engine
            .patch_mobile_settings(r#"{"decision":null,"dictionary":["Pennel"]}"#.into())
            .unwrap();
        let updated: crate::Settings = serde_json::from_str(&updated).unwrap();
        assert!(updated.decision.is_none());
        assert_eq!(updated.dictionary, vec!["Pennel"]);
    }

    #[test]
    fn changed_settings_cancel_registered_work_before_it_can_send() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server, InferenceProtocol::Perplexity);
        let cancellation = InferenceCancellation::new();
        engine.register_inference(&cancellation);
        let mut settings = engine.settings();
        settings.decision.as_mut().unwrap().model = "replacement".into();
        engine.update_settings(settings).unwrap();
        assert!(cancellation.is_cancelled());
        let mock = server.mock("POST", "/v1/decisions").expect(0).create();
        assert_eq!(
            engine.verify_inference(InferenceCapability::Decision, cancellation.clone(), keys),
            Err(PennelError::Cancelled)
        );
        engine.finish_inference(&cancellation);
        mock.assert();
    }

    #[test]
    fn completing_one_call_keeps_a_shared_cancellation_token_registered() {
        let server = mockito::Server::new();
        let (_dir, engine, _) = configured(&server, InferenceProtocol::Perplexity);
        let cancellation = InferenceCancellation::new();
        engine.register_inference(&cancellation);
        engine.register_inference(&cancellation);
        engine.finish_inference(&cancellation);
        let mut settings = engine.settings();
        settings.decision = None;
        engine.update_settings(settings).unwrap();
        assert!(cancellation.is_cancelled());
        engine.finish_inference(&cancellation);
    }

    #[test]
    fn changing_a_tei_account_during_metadata_lookup_prevents_content_post() {
        let mut server = mockito::Server::new();
        let (_dir, engine, keys) = configured(&server, InferenceProtocol::Tei);
        let mut settings = engine.settings();
        settings.reranking = settings.decision.take();
        settings
            .providers
            .iter_mut()
            .find(|p| p.id == "typed-fixture")
            .unwrap()
            .base_url = server.url();
        engine.update_settings(settings).unwrap();
        let changed = engine.clone();
        let info = server
            .mock("GET", "/info")
            .with_status(200)
            .with_body_from_request(move |_| {
                let mut settings = changed.settings();
                settings.reranking = None;
                changed.update_settings(settings).unwrap();
                br#"{"model_id":"fixture-model","model_sha":"pinned-revision"}"#.to_vec()
            })
            .create();
        let post = server.mock("POST", "/rerank").expect(0).create();
        assert_eq!(
            engine.verify_inference(
                InferenceCapability::Reranking,
                InferenceCancellation::new(),
                keys
            ),
            Err(PennelError::Cancelled)
        );
        info.assert();
        post.assert();
    }

    fn cloud_configured(
        server: &mockito::Server,
    ) -> (
        tempfile::TempDir,
        Arc<PennelEngine>,
        Arc<crate::cloud::tests::MemorySecrets>,
    ) {
        use crate::cloud::SecretStore;
        let dir = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        secrets.set(
            crate::cloud::REFRESH_ACCOUNT.into(),
            Some("fixture-refresh".into()),
        );
        engine.use_cloud_session(crate::cloud::account::Session::new(
            server.url(),
            secrets.clone(),
        ));
        let mut settings = engine.settings();
        settings.providers.push(ProviderAccount {
            id: "managed-fixture".into(),
            kind: ProviderKind::PennelCloud,
            name: "Pennel Cloud".into(),
            base_url: "https://must-not-receive-a-bearer.invalid/v1".into(),
        });
        settings.decision = Some(crate::new_inference_config(
            "managed-fixture".into(),
            InferenceProtocol::PennelCloud,
        ));
        settings.dictionary = vec!["private-vocabulary-must-not-be-sent".into()];
        engine.update_settings(settings).unwrap();
        (dir, engine, secrets)
    }

    fn cloud_refresh(server: &mut mockito::Server) -> mockito::Mock {
        server.mock("POST", "/v1/auth/refresh")
            .match_body(Matcher::Json(serde_json::json!({"refresh_token":"fixture-refresh"})))
            .with_body(r#"{"access_token":"fixture-access","refresh_token":"rotated-refresh","expires_in":900}"#).create()
    }

    fn cloud_sample(request: &mockito::Request) -> Vec<u8> {
        let body: serde_json::Value = serde_json::from_slice(request.body().unwrap()).unwrap();
        assert_eq!(body["model"], "pennel-auto");
        assert_eq!(
            body["state"],
            "Synthetic Pennel sample: the meeting starts at ten."
        );
        assert!(body.get("provider").is_none());
        assert!(!body.to_string().contains("private-vocabulary"));
        serde_json::json!({"schema_version":1,"request_id":body["request_id"],"model":"actual-fixture","answers":{"sample":{"type":"noul","noul":0.99}}}).to_string().into_bytes()
    }

    #[test]
    fn managed_sample_uses_only_session_origin_and_auth_without_a_provider_key() {
        let mut server = mockito::Server::new();
        let mut trap = mockito::Server::new();
        let (_dir, engine, _) = cloud_configured(&server);
        let mut settings = engine.settings();
        let before_llm = settings.llm.clone();
        let before_stt = settings.dictation_stt.clone();
        settings
            .providers
            .iter_mut()
            .find(|p| p.id == "managed-fixture")
            .unwrap()
            .base_url = format!("{}/v1", trap.url());
        engine.update_settings(settings.clone()).unwrap();
        let refresh = cloud_refresh(&mut server);
        let sample = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", "Bearer fixture-access")
            .with_body_from_request(cloud_sample)
            .create();
        let forbidden = trap.mock("POST", mockito::Matcher::Any).expect(0).create();
        let result = engine
            .verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                ApiKeys {
                    by_provider: HashMap::from([(
                        "managed-fixture".into(),
                        "must-not-be-used".into(),
                    )]),
                },
            )
            .unwrap();
        assert!(result.verified);
        let identity = result.identity.unwrap();
        assert_eq!(identity.endpoint, format!("{}/v1/decisions", server.url()));
        assert_eq!(identity.actual_model.as_deref(), Some("actual-fixture"));
        assert_eq!(identity.actual_provider, None);
        assert_eq!(engine.settings(), settings);
        assert_eq!(engine.settings().llm, before_llm);
        assert_eq!(engine.settings().dictation_stt, before_stt);
        refresh.assert();
        sample.assert();
        forbidden.assert();
    }

    #[test]
    fn managed_availability_requires_sign_in_and_server_capability_without_content() {
        use crate::cloud::SecretStore;
        let mut server = mockito::Server::new();
        let (_dir, engine, secrets) = cloud_configured(&server);
        secrets.set(crate::cloud::REFRESH_ACCOUNT.into(), None);
        let signed_out = engine.inference_availability(InferenceCapability::Decision);
        assert!(signed_out.configured && !signed_out.available);
        assert!(signed_out.reason.contains("Sign in"));
        assert_eq!(
            signed_out.endpoint,
            Some(format!("{}/v1/decisions", server.url()))
        );
        let no_content = server.mock("POST", "/v1/decisions").expect(0).create();
        secrets.set(
            crate::cloud::REFRESH_ACCOUNT.into(),
            Some("fixture-refresh".into()),
        );
        let refresh = cloud_refresh(&mut server);
        for (body, available) in [
            (
                r#"{"schema_version":1,"decision":{"available":true},"reranking":{"available":false}}"#,
                true,
            ),
            (
                r#"{"schema_version":1,"decision":{"available":false},"reranking":{"available":true}}"#,
                false,
            ),
            (
                r#"{"schema_version":2,"decision":{"available":true}}"#,
                false,
            ),
            (r#"{"schema_version":1,"decision":{}}"#, false),
        ] {
            let capabilities = server
                .mock("GET", "/v1/inference/capabilities")
                .match_header("authorization", "Bearer fixture-access")
                .with_body(body)
                .create();
            assert_eq!(
                engine
                    .inference_availability(InferenceCapability::Decision)
                    .available,
                available
            );
            capabilities.assert();
            capabilities.remove();
        }
        let unavailable = server
            .mock("GET", "/v1/inference/capabilities")
            .with_status(404)
            .with_body("private response")
            .create();
        let result = engine.inference_availability(InferenceCapability::Decision);
        assert!(!result.available && !result.reason.contains("private"));
        unavailable.assert();
        unavailable.remove();
        let mut settings = engine.settings();
        settings.decision = None;
        engine.update_settings(settings).unwrap();
        let unused = server
            .mock("GET", "/v1/inference/capabilities")
            .expect(0)
            .create();
        assert!(
            !engine
                .inference_availability(InferenceCapability::Decision)
                .configured
        );
        refresh.assert();
        no_content.assert();
        unused.assert();
    }

    #[test]
    fn managed_protocol_and_account_kind_cannot_be_mixed() {
        let mut server = mockito::Server::new();
        let (_dir, engine, _) = cloud_configured(&server);
        let refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        let mut settings = engine.settings();
        settings
            .providers
            .iter_mut()
            .find(|p| p.id == "managed-fixture")
            .unwrap()
            .kind = ProviderKind::Custom;
        engine.update_settings(settings).unwrap();
        assert!(
            engine
                .inference_selection(InferenceCapability::Decision)
                .is_err()
        );
        let mut settings = engine.settings();
        settings
            .providers
            .iter_mut()
            .find(|p| p.id == "managed-fixture")
            .unwrap()
            .kind = ProviderKind::PennelCloud;
        settings.decision.as_mut().unwrap().protocol = InferenceProtocol::Perplexity;
        engine.update_settings(settings).unwrap();
        assert!(
            engine
                .inference_selection(InferenceCapability::Decision)
                .is_err()
        );
        refresh.assert();
    }

    #[test]
    fn unsafe_session_origin_is_rejected_before_auth_or_content() {
        let mut server = mockito::Server::new();
        let (_dir, engine, secrets) = cloud_configured(&server);
        engine.use_cloud_session(crate::cloud::account::Session::new(
            format!("{}?private=credential", server.url()),
            secrets,
        ));
        let never = server.mock("POST", Matcher::Any).expect(0).create();
        let result = engine
            .verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                ApiKeys {
                    by_provider: HashMap::new(),
                },
            )
            .unwrap();
        assert!(!result.verified);
        assert!(!result.message.contains("credential"));
        never.assert();
    }

    #[test]
    fn replacing_cloud_credentials_during_refresh_cancels_before_content() {
        let mut server = mockito::Server::new();
        let (_dir, engine, _) = cloud_configured(&server);
        let changed = engine.clone();
        let refresh = server.mock("POST", "/v1/auth/refresh").with_body_from_request(move |_| {
            changed.set_secret_store(Arc::new(crate::cloud::tests::MemorySecrets::default())).unwrap();
            br#"{"access_token":"fixture-access","refresh_token":"rotated-refresh","expires_in":900}"#.to_vec()
        }).create();
        let content = server.mock("POST", "/v1/decisions").expect(0).create();
        assert_eq!(
            engine.verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                ApiKeys {
                    by_provider: HashMap::new()
                }
            ),
            Err(PennelError::Cancelled)
        );
        refresh.assert();
        content.assert();
    }

    #[test]
    fn replacing_session_after_content_prevents_verifying_the_new_session() {
        let mut server = mockito::Server::new();
        let (_dir, engine, secrets) = cloud_configured(&server);
        let _refresh = cloud_refresh(&mut server);
        let replacement_url = server.url();
        let changed = engine.clone();
        let content = server
            .mock("POST", "/v1/decisions")
            .with_body_from_request(move |request| {
                changed.use_cloud_session(crate::cloud::account::Session::new(
                    replacement_url.clone(),
                    secrets.clone(),
                ));
                cloud_sample(request)
            })
            .create();
        assert_eq!(
            engine.verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                ApiKeys {
                    by_provider: HashMap::new()
                }
            ),
            Err(PennelError::Cancelled)
        );
        content.assert();
    }

    #[test]
    fn cloud_epoch_initializes_only_session_metadata_and_mutation_blocks_auth() {
        let mut server = mockito::Server::new();
        let (_dir, engine, secrets) = cloud_configured(&server);
        let before = engine.cloud_account_epoch();
        engine.cloud_session().unwrap().cancel_login();
        assert_ne!(engine.cloud_account_epoch(), before);
        let previous_session = engine.cloud_account_epoch();
        engine.use_cloud_session(crate::cloud::account::Session::new(server.url(), secrets));
        assert_ne!(engine.cloud_account_epoch(), previous_session);
        assert_ne!(engine.cloud_account_epoch(), before);
        let mutation = engine.begin_inference_mutation();
        let refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        assert_eq!(
            engine.verify_inference(
                InferenceCapability::Decision,
                InferenceCancellation::new(),
                ApiKeys {
                    by_provider: HashMap::new()
                }
            ),
            Err(PennelError::Cancelled)
        );
        mutation.release();
        refresh.assert();
        let dir = tempfile::tempdir().unwrap();
        let untouched =
            PennelEngine::new_preview(dir.path().to_string_lossy().into_owned()).unwrap();
        assert_eq!(untouched.cloud_account_epoch(), 0);
        assert!(untouched.current_cloud_session().is_none());
        struct UnreadStore;
        impl crate::cloud::SecretStore for UnreadStore {
            fn get(&self, _account: String) -> Option<String> {
                panic!("reading the account epoch must not read credentials");
            }
            fn set(&self, _account: String, _value: Option<String>) {
                panic!("reading the account epoch must not write credentials");
            }
        }
        untouched.set_secret_store(Arc::new(UnreadStore)).unwrap();
        let initialized = untouched.cloud_account_epoch();
        assert_ne!(initialized, 0);
        assert_eq!(untouched.cloud_account_epoch(), initialized);
        assert!(untouched.current_cloud_session().is_some());
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn inference_availability(&self, capability: InferenceCapability) -> InferenceAvailability {
        let settings = self.settings();
        let config = match capability {
            InferenceCapability::Decision => settings.decision,
            InferenceCapability::Reranking => settings.reranking,
        };
        let mut result = InferenceAvailability {
            capability,
            configured: config.is_some(),
            available: false,
            reason: "Configure this optional connection first.".into(),
            provider_id: config.as_ref().map(|c| c.provider_id.clone()),
            model: config.as_ref().map(|c| c.model.clone()),
            protocol: config.as_ref().map(|c| c.protocol),
            endpoint: None,
        };
        match self.inference_selection(capability) {
            Ok((config, account)) => {
                result.endpoint = inference::validate_config(
                    &config,
                    &self.inference_base_url(&account),
                    capability,
                )
                .ok();
                if account.kind == ProviderKind::PennelCloud {
                    let cancellation = InferenceCancellation::new();
                    self.register_inference(&cancellation);
                    let _registration = InferenceRegistration {
                        engine: self,
                        cancellation: &cancellation,
                    };
                    let available = self.managed_inference(&account).and_then(|managed| {
                        let managed = managed.expect("Cloud account has a managed session");
                        let available = inference::managed_capability_available(
                            &config,
                            managed.clone(),
                            capability,
                            cancellation.clone(),
                        )?;
                        self.inference_unchanged(capability, &config, &account, &cancellation)?;
                        self.managed_inference_unchanged(Some(&managed))?;
                        Ok(available)
                    });
                    match available {
                        Ok(true) => {
                            result.available = true;
                            result.reason = "Pennel Cloud supports this connection. Test with the synthetic sample to verify it.".into();
                        }
                        Ok(false) => result.reason = "Pennel Cloud has not enabled this inference capability for this account.".into(),
                        Err(PennelError::NotSignedIn) => result.reason = "Sign in to Pennel Cloud to use this connection.".into(),
                        Err(PennelError::Cancelled) => result.reason = "The Pennel Cloud account or connection changed. Check availability again.".into(),
                        Err(_) => result.reason = "Pennel Cloud inference availability could not be confirmed. Check the connection and try again.".into(),
                    }
                } else {
                    result.available = true;
                    result.reason =
                        "Configured. Test with the synthetic sample to verify this connection."
                            .into();
                }
            }
            Err(PennelError::InvalidResponse { message }) => result.reason = message,
            Err(_) => result.reason = "This optional connection is unavailable.".into(),
        }
        result
    }

    pub fn decide(
        &self,
        request: DecisionRequest,
        cancellation: Arc<InferenceCancellation>,
        keys: ApiKeys,
    ) -> Result<DecisionResult, PennelError> {
        let capability = InferenceCapability::Decision;
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        self.register_inference(&cancellation);
        let _registration = InferenceRegistration {
            engine: self,
            cancellation: &cancellation,
        };
        let (config, account) = self.inference_selection(capability)?;
        let managed = self.managed_inference(&account)?;
        let result = if let Some(managed) = managed.as_ref() {
            inference::decide_managed(&config, managed.clone(), request, cancellation.clone())?
        } else {
            let key = self.credential(&account, &keys)?;
            inference::decide(
                &config,
                &account.base_url,
                key.as_deref(),
                request,
                cancellation.clone(),
            )?
        };
        self.inference_unchanged(capability, &config, &account, &cancellation)?;
        self.managed_inference_unchanged(managed.as_ref())?;
        Ok(result)
    }

    pub fn rerank(
        &self,
        request: RerankRequest,
        cancellation: Arc<InferenceCancellation>,
        keys: ApiKeys,
    ) -> Result<RerankResult, PennelError> {
        let capability = InferenceCapability::Reranking;
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        self.register_inference(&cancellation);
        let _registration = InferenceRegistration {
            engine: self,
            cancellation: &cancellation,
        };
        let (config, account) = self.inference_selection(capability)?;
        let managed = self.managed_inference(&account)?;
        let result = if let Some(managed) = managed.as_ref() {
            inference::rerank_managed(&config, managed.clone(), request, cancellation.clone())?
        } else {
            let key = self.credential(&account, &keys)?;
            inference::rerank(
                &config,
                &account.base_url,
                key.as_deref(),
                request,
                cancellation.clone(),
            )?
        };
        self.inference_unchanged(capability, &config, &account, &cancellation)?;
        self.managed_inference_unchanged(managed.as_ref())?;
        Ok(result)
    }

    pub fn decide_json(
        &self,
        request_json: String,
        cancellation: Arc<InferenceCancellation>,
        keys: ApiKeys,
    ) -> Result<String, PennelError> {
        if request_json.len() > 256 * 1024 {
            return Err(invalid("The decision request is too large."));
        }
        let request = serde_json::from_str(&request_json)
            .map_err(|_| invalid("The decision request is invalid."))?;
        serde_json::to_string(&self.decide(request, cancellation, keys)?)
            .map_err(|_| invalid("The decision response could not be encoded."))
    }

    pub fn rerank_json(
        &self,
        request_json: String,
        cancellation: Arc<InferenceCancellation>,
        keys: ApiKeys,
    ) -> Result<String, PennelError> {
        if request_json.len() > 256 * 1024 {
            return Err(invalid("The reranking request is too large."));
        }
        let request = serde_json::from_str(&request_json)
            .map_err(|_| invalid("The reranking request is invalid."))?;
        serde_json::to_string(&self.rerank(request, cancellation, keys)?)
            .map_err(|_| invalid("The reranking response could not be encoded."))
    }

    pub fn verify_inference(
        &self,
        capability: InferenceCapability,
        cancellation: Arc<InferenceCancellation>,
        keys: ApiKeys,
    ) -> Result<InferenceVerification, PennelError> {
        if cancellation.is_cancelled() {
            return Err(PennelError::Cancelled);
        }
        let outcome = match capability {
            InferenceCapability::Decision => self
                .decide(
                    DecisionRequest {
                        state: "Synthetic Pennel sample: the meeting starts at ten.".into(),
                        questions: vec![DecisionQuestion {
                            id: "sample".into(),
                            kind: DecisionQuestionKind::Binary,
                            instructions: "Does the sample state that the meeting starts at ten?"
                                .into(),
                            options: vec![],
                        }],
                    },
                    cancellation,
                    keys,
                )
                .map(|r| {
                    let valid = r.answers.len() == 1
                        && r.answers[0].id == "sample"
                        && !r.answers[0].abstained
                        && r.answers[0].value.is_some_and(|v| v > 0.5);
                    (valid, r.identity)
                }),
            InferenceCapability::Reranking => self
                .rerank(
                    RerankRequest {
                        query: "When does the meeting start?".into(),
                        passages: vec![
                            RerankPassage {
                                id: "meeting".into(),
                                text: "The meeting starts at ten.".into(),
                            },
                            RerankPassage {
                                id: "weather".into(),
                                text: "The sky is clear.".into(),
                            },
                        ],
                    },
                    cancellation,
                    keys,
                )
                .map(|r| {
                    let meeting = r.scores.iter().find(|s| s.passage_id == "meeting");
                    let other = r.scores.iter().find(|s| s.passage_id == "weather");
                    (
                        meeting.zip(other).is_some_and(|(a, b)| a.score > b.score),
                        r.identity,
                    )
                }),
        };
        match outcome {
            Ok((verified, identity)) => Ok(InferenceVerification {
                verified,
                message: if verified { "The selected connection processed the synthetic sample. This does not establish model accuracy." } else { "The selected connection did not produce the expected sample result." }.into(),
                identity: Some(identity),
            }),
            Err(PennelError::Cancelled) => Err(PennelError::Cancelled),
            Err(error) => {
                let message = match error {
                    PennelError::MissingApiKey { .. } => "Configure this provider's API key first.",
                    PennelError::NotSignedIn => "Sign in to Pennel Cloud to use this connection.",
                    PennelError::Http { status: 401 | 403, .. } => "The provider denied access. Check the key and model access.",
                    PennelError::Http { status: 402, .. } => "The provider reports insufficient credit.",
                    PennelError::Http { status: 429, .. } => "The provider has reached its usage limit. Try again later.",
                    PennelError::Http { status: 503, .. } => "This inference capability is not available from the selected service yet.",
                    PennelError::InvalidResponse { .. } => "The connection or typed response is invalid. Check its configuration and model.",
                    _ => "The provider could not process the sample. Check the connection and try again.",
                };
                Ok(InferenceVerification { verified: false, message: message.into(), identity: None })
            }
        }
    }
}
