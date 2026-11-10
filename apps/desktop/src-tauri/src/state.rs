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

    pub fn keys(&self) -> ApiKeys {
        self.secrets.keys(&self.engine.settings().providers)
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
        self.engine.configured_dictation_models(self.keys())
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
