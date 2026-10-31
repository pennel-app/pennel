uniffi::setup_scaffolding!();

mod call_detector;
mod catalog;
mod chunker;
mod claude_access;
mod cleanup;
#[cfg(test)]
mod cleanup_fidelity_tests;
mod cleanup_verification;
mod cli_agent;
mod cloud;
mod command;
mod cpu_support;
mod diarize;
mod dictation_pad;
mod engine;
mod error;
#[cfg(feature = "evaluation")]
pub mod evaluation;
mod export;
mod gesture;
mod history;
mod http;
mod inference;
mod inference_engine;
mod keyboard_writing;
mod live_dictation;
mod llm;
mod local_engine;
mod localization;
mod meeting_chat;
mod meeting_index;
mod meeting_recording;
mod meetings;
mod models;
mod paste_queue;
mod provider;
mod provider_test;
mod recording;
mod remote_models;
mod retranscription;
mod rules;
mod session;
mod settings;
mod setup;
mod speaker_commit;
mod speaker_jobs;
mod speakers;
mod speech;
mod status;
mod storage;
mod stt;
mod summary;
mod system_llm;
mod system_stt;
mod templates;
mod transcript_filter;
mod voices;
mod wav;

pub use call_detector::{CallDetector, CallEvent, CallObservation};
pub use cleanup_verification::CleanupVerificationAvailability;
pub use cli_agent::{DetectedAgent, check_cli_agent, detect_cli_agents};
pub use cloud::SecretStore;
pub use cloud::account::{CloudAccount, CloudDevice, CloudLoginPoll, CloudLoginStart};
pub use cloud::link::{CloudLinkPhase, CloudLinkRole, CloudLinkView};
pub use cloud::relay::{RelayAnswer, RelayRequest};
pub use cloud::setup::{CloudSyncSetup, CloudSyncSetupPhase};
pub use cloud::sync::{MASS_DELETE, MeetingSyncState, SyncItemState, SyncReport, SyncStatus};
pub use cpu_support::{UNSUPPORTED_PROCESSOR, local_models_unavailable_reason};
pub use engine::{
    ApiKeys, CleanupStatus, DictationRequest, DictationResult, LibrarySearch, PennelEngine,
};
pub use error::PennelError;
pub use export::MeetingExport;
pub use gesture::{GestureAction, HotkeyGesture, KeyEdge};
pub use history::{HistoryEntry, HistoryStatus};
pub use inference::{
    DecisionAnswer, DecisionOption, DecisionProbability, DecisionQuestion, DecisionQuestionKind,
    DecisionRequest, DecisionResult, DecisionSpan, InferenceCancellation, InferenceCapability,
    InferenceConfig, InferenceIdentity, InferenceProtocol, InferenceProtocolInfo, InferenceUsage,
    RerankPassage, RerankRequest, RerankResult, RerankScore, inference_protocols,
    new_inference_config,
};
pub use inference_engine::{InferenceAvailability, InferenceMutation, InferenceVerification};
pub use keyboard_writing::{KeyboardRewriteAction, KeyboardWritingJob};
pub use live_dictation::{
    DictationPreviewMode, DictationPreviewObserver, DictationPreviewUpdate, LiveDictationSession,
};
pub use localization::{
    InterfaceLanguage, InterfaceLanguagePreference, localize_ui_text, resolve_interface_language,
    ui_text,
};
pub use meeting_chat::{ChatTurn, Citation, MeetingAnswer};
pub use meeting_index::{Passage, PassageKind};
pub use meeting_recording::{
    CaptureEventKind, CaptureFormatEpoch, MeetingAudioTrack, MeetingCaptureEvent,
    MeetingRecordingMetadata, RecordingEnvironment, RecordingRecognition, RecordingSource,
    RecordingTrackMetadata, RecordingTranscriptionAttempt, RecordingTransport,
    resolve_meeting_audio_tracks,
};
pub use meetings::{MeetingInfo, NoteLine, Segment, Speaker, SpeakerName, mark_lines};
pub use models::{DownloadListener, LocalModel, ModelState};
pub use paste_queue::PasteQueue;
pub use pennel_typing as keyboard;
pub use provider::{
    ProviderAccount, ProviderKind, ProviderTask, default_base_url, default_model, new_account,
    requires_key, supports,
};
pub use provider_test::{ProviderTaskTestResult, test_provider_task};
pub use recording::{RecordingLifecycle, RecordingPhase, RecordingState};
pub use remote_models::{RemoteModel, provider_models};
pub use retranscription::{
    MeetingRetranscriptionPlan, MeetingRetranscriptionStatus, RetranscriptionPhase,
    RetranscriptionProgress,
};
pub use rules::{
    AppRule, CalendarRule, EventRule, RecordAction, browser_apps, consent_sentence,
    consent_sentence_for_language, decide_recording, known_call_apps,
};
pub use session::{MeetingHealth, MeetingRecognition, MeetingSession};
pub use settings::{
    AppearancePreference, DictationMode, HotkeyTrigger, ModelChoice, OnboardingGoal,
    OnboardingProgress, OnboardingStatus, OnboardingStep, Settings, SpeakerRecognitionMode, Style,
    StyleRule, default_settings,
};
pub use setup::{SetupAccess, SetupFacts, SetupReadiness, dictation_readiness, meeting_readiness};
pub use speaker_jobs::{
    RecognitionAudioBlock, RecognitionAudioInfo, RecognitionAudioReader, RecognitionFileTransport,
    RecognitionUploadProgress, RecognitionUploadRequest, RecognitionUploadResponse,
    SpeakerRecognitionJob, SpeakerRecognitionJobSnapshot, SpeakerRecognitionStage,
};
pub use speakers::{MeetingVoice, SpeakerRecognition};
pub use status::{CommandIntent, DictationStatus, PillState};
pub use system_llm::{LanguageModelMessage, SystemLanguageModel};
pub use system_stt::SystemTranscriber;
pub use templates::MeetingTemplate;
pub use voices::{KnownVoice, VoiceProfile};
