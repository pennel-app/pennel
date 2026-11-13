use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use pennel_core::{ApiKeys, PennelEngine, RecordingLifecycle, Settings};

use crate::dictation::Msg;
use crate::hotkey::Listener;
use crate::meeting::MeetingRecorder;
use crate::meeting_runtime::MeetingContext;
use crate::secrets::Secrets;

pub struct Shared {
    pub system_interface_language: pennel_core::InterfaceLanguage,
    pub engine: Arc<PennelEngine>,
    pub version: String,
    pub preview: bool,
    pub migration_warning: Option<String>,
    pub secrets: Arc<Secrets>,
    pub data_dir: PathBuf,
    pub dictation: Sender<Msg>,
    pub hotkey: Mutex<Option<Listener>>,
    pub hotkey_error: Mutex<Option<String>>,
    pub meeting: Mutex<Option<MeetingRecorder>>,
    pub recording: Mutex<RecordingLifecycle>,
    pub meeting_context: Mutex<Option<MeetingContext>>,
    pub recording_cancel: AtomicBool,
    pub library_jobs: AtomicUsize,
    pub live_questions: Mutex<crate::live_questions::LiveQuestions>,
    pub updating: AtomicBool,
    pub quitting: AtomicBool,
    pub dictation_active: AtomicBool,
    pub dictation_jobs: AtomicUsize,
    pub paste_jobs: AtomicUsize,
    pub writing_profile_sharing_epoch: Mutex<u64>,
    pub writing_profile_source_gate: Mutex<()>,
}

impl Shared {
    #[cfg(test)]
    pub(crate) fn testing(data_dir: PathBuf) -> Arc<Self> {
        Self::testing_variant(data_dir, true)
    }

    #[cfg(test)]
    pub(crate) fn testing_production(data_dir: PathBuf) -> Arc<Self> {
        Self::testing_variant(data_dir, false)
    }

    #[cfg(test)]
    fn testing_variant(data_dir: PathBuf, preview: bool) -> Arc<Self> {
        let engine = if preview {
            PennelEngine::new_preview(data_dir.to_string_lossy().into_owned()).unwrap()
        } else {
            std::fs::create_dir_all(&data_dir).unwrap();
            Settings {
                meetings_dir: data_dir.join("Meetings").to_string_lossy().into_owned(),
                ..pennel_core::default_settings()
            }
            .save(&data_dir.join("settings.json"))
            .unwrap();
            PennelEngine::new(data_dir.to_string_lossy().into_owned()).unwrap()
        };
        let (dictation, _) = std::sync::mpsc::channel();
        Arc::new(Self {
            system_interface_language: pennel_core::InterfaceLanguage::English,
            engine,
            version: "0.0.0-test".into(),
            preview,
            migration_warning: None,
            data_dir,
            dictation,
            secrets: Arc::new(Secrets::in_memory()),
            hotkey: Mutex::new(None),
            hotkey_error: Mutex::new(None),
            meeting: Mutex::new(None),
            recording: Mutex::new(RecordingLifecycle::default()),
            meeting_context: Mutex::new(None),
            recording_cancel: AtomicBool::new(false),
            library_jobs: AtomicUsize::new(0),
            live_questions: Mutex::new(crate::live_questions::LiveQuestions::default()),
            updating: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            dictation_active: AtomicBool::new(false),
            dictation_jobs: AtomicUsize::new(0),
            paste_jobs: AtomicUsize::new(0),
            writing_profile_sharing_epoch: Mutex::new(0),
            writing_profile_source_gate: Mutex::new(()),
        })
    }

    pub fn interface_language(&self) -> pennel_core::InterfaceLanguage {
        self.settings()
            .ui_language
            .resolve(&[self.system_interface_language.code().to_owned()])
    }

    pub fn settings(&self) -> Settings {
        self.engine.settings()
    }

    pub fn keys_for_task(&self, task: pennel_core::ProviderTask) -> Result<ApiKeys, String> {
        let settings = self.settings();
        let choice = match task {
            pennel_core::ProviderTask::DictationStt => settings.dictation_stt,
            pennel_core::ProviderTask::MeetingStt => settings.meeting_stt,
            pennel_core::ProviderTask::Llm => settings.llm,
        };
        self.keys_for(&[choice.provider_id.as_str()])
    }

    pub fn keys_for_inference(
        &self,
        capability: pennel_core::InferenceCapability,
    ) -> Result<ApiKeys, String> {
        let settings = self.settings();
        let config = match capability {
            pennel_core::InferenceCapability::Decision => settings.decision,
            pennel_core::InferenceCapability::Reranking => settings.reranking,
        };
        match config {
            Some(config) => self.keys_for(&[config.provider_id.as_str()]),
            None => self.keys_for(&[]),
        }
    }

    /// Model listings must not fail because one unrelated saved key is blocked.
    pub fn accessible_keys(&self) -> ApiKeys {
        let settings = self.settings();
        self.secrets.configure_providers(&settings.providers);
        let mut keys = ApiKeys {
            by_provider: Default::default(),
        };
        for provider in &settings.providers {
            if provider.kind == pennel_core::ProviderKind::PennelCloud
                && self.secrets.ensure_cloud_access().is_err()
            {
                continue;
            }
            if let Ok(found) = self.secrets.keys_checked(std::slice::from_ref(provider)) {
                keys.by_provider.extend(found.by_provider);
            }
        }
        keys
    }

    pub fn keys_for(&self, ids: &[&str]) -> Result<ApiKeys, String> {
        let settings = self.settings();
        self.secrets.configure_providers(&settings.providers);
        let providers: Vec<_> = settings
            .providers
            .into_iter()
            .filter(|p| ids.contains(&p.id.as_str()))
            .collect();
        if providers
            .iter()
            .any(|p| p.kind == pennel_core::ProviderKind::PennelCloud)
        {
            self.secrets
                .ensure_cloud_access()
                .map_err(crate::commands::localize_error_message)?;
        }
        self.secrets
            .keys_checked(&providers)
            .map_err(crate::commands::localize_error_message)
    }

    pub fn refresh_system_speech_availability(&self) {
        #[cfg(target_os = "macos")]
        self.engine.set_system_speech_available(
            crate::capabilities::snapshot(None).apple_speech.status
                == crate::capabilities::CapabilityStatus::Available,
        );
    }

    pub fn configured_dictation_models(&self) -> Vec<pennel_core::ConfiguredDictationModel> {
        self.refresh_system_speech_availability();
        self.engine
            .configured_dictation_models(self.accessible_keys())
    }

    pub fn set_meeting_dictating(&self, dictating: bool) {
        if let Some(meeting) = self.meeting.lock().unwrap().as_ref() {
            meeting.set_dictating(dictating);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_task_keys_ignore_unrelated_blocked_credentials() {
        use pennel_core::{ModelChoice, ProviderAccount, ProviderKind, ProviderTask};
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let mut settings = shared.settings();
        settings.providers.push(ProviderAccount {
            id: "on-device-text".into(),
            kind: ProviderKind::AppleIntelligence,
            name: "On-device".into(),
            base_url: String::new(),
        });
        settings.providers.push(ProviderAccount {
            id: "blocked-speech".into(),
            kind: ProviderKind::Groq,
            name: "Speech".into(),
            base_url: "https://example.invalid".into(),
        });
        settings.llm = ModelChoice {
            provider_id: "on-device-text".into(),
            model: "system".into(),
        };
        settings.meeting_stt = ModelChoice {
            provider_id: "blocked-speech".into(),
            model: "fixture".into(),
        };
        shared.engine.update_settings(settings).unwrap();
        *shared.secrets.access_failure.lock().unwrap() =
            Some(crate::secrets::CredentialState::MigrationRequired);
        assert!(
            shared
                .keys_for_task(ProviderTask::Llm)
                .unwrap()
                .by_provider
                .is_empty()
        );
        assert!(shared.keys_for_task(ProviderTask::MeetingStt).is_err());
    }

    #[test]
    fn inference_key_resolution_checks_only_the_requested_capability() {
        use pennel_core::{InferenceCapability, InferenceProtocol, ProviderAccount, ProviderKind};
        let root = tempfile::tempdir().unwrap();
        let shared = Shared::testing(root.path().to_path_buf());
        let mut settings = shared.settings();
        settings.providers.push(ProviderAccount {
            id: "local-decision".into(),
            kind: ProviderKind::LmStudio,
            name: "Local".into(),
            base_url: "http://127.0.0.1:1".into(),
        });
        settings.providers.push(ProviderAccount {
            id: "blocked-ranking".into(),
            kind: ProviderKind::OpenAi,
            name: "Ranking".into(),
            base_url: "https://example.invalid".into(),
        });
        settings.decision = Some(pennel_core::new_inference_config(
            "local-decision".into(),
            InferenceProtocol::LocalDecider,
        ));
        settings.reranking = Some(pennel_core::new_inference_config(
            "blocked-ranking".into(),
            InferenceProtocol::Tei,
        ));
        shared.engine.update_settings(settings).unwrap();
        *shared.secrets.access_failure.lock().unwrap() =
            Some(crate::secrets::CredentialState::Locked);
        assert!(
            shared
                .keys_for_inference(InferenceCapability::Decision)
                .unwrap()
                .by_provider
                .is_empty()
        );
        assert!(
            shared
                .keys_for_inference(InferenceCapability::Reranking)
                .is_err()
        );
        let mut settings = shared.settings();
        settings.reranking = None;
        shared.engine.update_settings(settings).unwrap();
        assert!(
            shared
                .keys_for_inference(InferenceCapability::Reranking)
                .unwrap()
                .by_provider
                .is_empty()
        );
    }

    #[test]
    fn interface_preferences_keep_the_system_snapshot_and_active_session() {
        use pennel_core::{InterfaceLanguage, InterfaceLanguagePreference};
        let root = tempfile::tempdir().unwrap();
        let mut shared = Shared::testing(root.path().to_path_buf());
        Arc::get_mut(&mut shared).unwrap().system_interface_language = InterfaceLanguage::Dutch;
        let token = shared.recording.lock().unwrap().begin(None).unwrap();
        let mut settings = shared.settings();
        settings.language = Some("fr".into());
        for (preference, resolved) in [
            (
                InterfaceLanguagePreference::English,
                InterfaceLanguage::English,
            ),
            (InterfaceLanguagePreference::Dutch, InterfaceLanguage::Dutch),
            (
                InterfaceLanguagePreference::System,
                InterfaceLanguage::Dutch,
            ),
        ] {
            settings.ui_language = preference;
            shared.engine.update_settings(settings.clone()).unwrap();
            assert_eq!(shared.interface_language(), resolved);
            assert_eq!(shared.settings().ui_language, preference);
            assert_eq!(shared.settings().language.as_deref(), Some("fr"));
            assert_eq!(
                shared
                    .recording
                    .lock()
                    .unwrap()
                    .snapshot()
                    .session_id
                    .as_deref(),
                Some(token.as_str())
            );
        }
    }
}
