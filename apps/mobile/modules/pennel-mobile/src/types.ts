export type InterfaceLanguage = 'en' | 'nl';
export type Appearance = 'system' | 'light' | 'dark';
export type ProviderKind = 'open_ai' | 'open_router' | 'groq' | 'mistral' | 'ollama' | 'lm_studio' | 'custom' | 'local' | 'apple_speech' | 'apple_intelligence' | 'claude_code' | 'codex' | 'open_code' | 'copilot' | 'cursor_agent' | 'gemini_cli' | 'pennel_cloud';
export interface Capability { status: 'available' | 'permission_required' | 'unsupported' | 'unavailable'; reason: string | null }
export interface ModelChoice { provider_id: string; model: string }
export type InferenceProtocol = 'open_router_decisions' | 'open_router_system_one' | 'type_safe' | 'perplexity' | 'mercury' | 'local_decider' | 'tei';
export type InferenceCapability = 'decision' | 'reranking';
export interface InferenceConfig { provider_id: string; protocol: InferenceProtocol; model: string; expected_model: string | null; upstream: string | null; max_input_bytes: number; timeout_secs: number }
export interface InferenceProtocolInfo { protocol: InferenceProtocol; name: string; default_base_url: string; decision: boolean; reranking: boolean; extraction: boolean; requires_key: boolean }
export interface InferenceAvailability { capability: InferenceCapability; configured: boolean; available: boolean; reason: string | null; provider_id: string | null; model: string | null; protocol: InferenceProtocol | null; endpoint: string | null }
export interface CleanupVerificationAvailability { enabled: boolean; available: boolean; reason_code: string; provider_id: string | null; model: string | null; endpoint: string | null }
export interface DecisionOption { id: string; description: string }
export interface DecisionQuestion { id: string; kind: 'binary' | 'choice' | 'ordinal' | 'extract'; instructions: string; options: DecisionOption[] }
export interface DecisionRequest { state: string; questions: DecisionQuestion[] }
export interface DecisionProbability { option_id: string; probability: number }
export interface DecisionSpan { field_id: string; text: string; start_char: number; end_char: number; confidence: number | null }
export interface DecisionAnswer { id: string; kind: DecisionQuestion['kind']; choice: string | null; value: number | null; probabilities: DecisionProbability[]; confidence: number | null; spans: DecisionSpan[]; not_stated: DecisionProbability[]; abstained: boolean }
export interface InferenceIdentity { protocol: InferenceProtocol; endpoint: string; requested_model: string; requested_upstream: string | null; actual_model: string | null; actual_revision: string | null; actual_provider: string | null; request_id: string | null }
export interface InferenceUsage { input_tokens: number | null; output_tokens: number | null; billed_cost_usd: number | null }
export interface DecisionResult { answers: DecisionAnswer[]; identity: InferenceIdentity; usage: InferenceUsage }
export interface RerankRequest { query: string; passages: { id: string; text: string }[] }
export interface RerankResult { scores: { passage_id: string; score: number }[]; identity: InferenceIdentity; usage: InferenceUsage }
export interface InferenceVerification { verified: boolean; message: string; identity: InferenceIdentity | null }
export interface ProviderAccount { id: string; kind: ProviderKind; name: string; base_url: string }
export interface ProviderKindInfo { kind: ProviderKind; label: string; requires_key: boolean; stt: boolean; llm: boolean; availability: Capability; default_base_url: string }
export interface Settings {
  ui_language: 'system' | InterfaceLanguage;
  appearance: Appearance;
  onboarding: { status: 'pending' | 'skipped' | 'completed'; version: number; goal?: 'dictation' | 'meeting'; resume_step?: 'language' | 'welcome' | 'engine' | 'permissions' | 'practice' | 'voice' };
  providers: ProviderAccount[];
  dictation_stt: ModelChoice;
  meeting_stt: ModelChoice;
  llm: ModelChoice;
  decision?: InferenceConfig | null;
  reranking?: InferenceConfig | null;
  cleanup_verification_enabled: boolean;
  mode: 'fast' | 'clean';
  language: string | null;
  dictionary: string[];
  style_rules: { bundle_id: string; style: string }[];
  history_retention_days: number;
  default_template: string;
  meeting_audio_retention_days: number;
  diarization: boolean;
  speaker_recognition: 'local' | 'cloud';
  cloud_sync?: boolean;
  haptic_feedback?: boolean;
  keyboard_haptics?: boolean;
  [key: string]: unknown;
}
export interface Segment { start_ms: number; end_ms: number; speaker: 'me' | 'others'; text: string; failed: boolean; voice: number | null }
export interface MeetingInfo { id: string; title: string; start_iso: string; duration_min: number; has_summary: boolean; sync_state?: 'synced' | 'pending' | 'conflict' }
export interface MeetingVoice { voice: number; name: string | null; remembered: boolean; can_remember: boolean }
export interface MeetingDetail { info: MeetingInfo; notes: string; summary: string | null; transcript: Segment[]; speakers: { voice: number; name: string }[]; voices?: MeetingVoice[]; attendees: string[]; audio_tracks: { speaker: 'me' | 'others'; available: boolean }[]; summary_outdated?: boolean; conflicts?: ConflictFile[]; interruption?: string | null; interruption_code?: string | null }
export interface ConflictFile { name: string; text?: string }
export interface MeetingTemplate { id: string; name: string; builtin: boolean; customized: boolean }
export interface HistoryEntry { id: string; created_at_ms: number; raw: string; text: string; status: 'done' | 'failed' | 'unsaved'; error: string | null; has_audio: boolean }
export interface Passage { meeting_id: string; title: string; kind: 'notes' | 'transcript' | 'summary'; start_ms: number | null; text: string; snippet: string }
export interface MeetingAnswer { text: string; citations: { number: number; passage: Passage }[] }
export interface ChatTurn { question: string; answer: string }
export interface CloudAccount { email: string; audio_seconds: number; tokens: number; storage_bytes: number; limit_audio_seconds: number; limit_tokens: number; limit_storage_bytes: number }
export interface CloudDevice { id: string; name: string; platform: string; last_seen_iso: string; current: boolean }
export interface LoginStart { user_code: string; verification_uri: string; interval_secs: number; expires_at_ms?: number }
export interface CloudState { signed_in: boolean; account: CloudAccount | null; account_epoch?: string; recovery_display_token?: string | null; syncing: boolean; sync_enabled: boolean; status: string; report?: SyncReport; recovery_confirmed: boolean; brake: boolean; device_name: string; notice: string | null; login?: { phase: 'idle' | 'code' | 'expired'; start?: LoginStart } }
export interface CloudRecoveryDisplay { recovery_code: string | null; confirmation_token: string | null; account_epoch: string }
export interface CloudSyncSetup { phase: 'signed_out' | 'new_library' | 'locked' | 'ready' | 'paused' | 'active'; sync_enabled: boolean; generation: number | null; can_approve_devices: boolean; uploaded: number; downloaded: number; conflicts: number; last_success_ms: number | null }
export type CloudLinkRole = 'new_device' | 'trusted_device';
export type CloudLinkPhase = 'idle' | 'waiting_peer' | 'confirming' | 'awaiting_approval' | 'waiting_approval' | 'provisioning' | 'complete' | 'cancelled' | 'expired' | 'failed';
export interface CloudLinkView { phase: CloudLinkPhase; role: CloudLinkRole | null; request_id: string | null; connection_code: string | null; qr_payload: string | null; qr_svg: string | null; qr_image?: string | null; verification_code: string | null; peer_name: string | null; peer_platform: string | null; account_email: string | null; expires_at_ms: number | null; error_code: string | null; error_message: string | null }
export type RecognitionStage = 'preparing' | 'uploading' | 'recognizing' | 'response_unknown' | 'interrupted' | 'applying' | 'completed' | 'cancelled' | 'failed';
export interface SpeakerJob { id: string; meeting_uid: string; meeting_id?: string; stage: RecognitionStage; decoded_frames: number; uploaded_bytes: number; total_bytes?: number; confirmed_chunks: number; total_chunks?: number; recoverable: boolean; error: { code: string; message: string } | null }
export interface RuntimeSnapshot {
  revision: number;
  ready: boolean;
  platform: 'ios' | 'android';
  settings: Settings;
  capabilities: Record<string, Capability>;
  kinds: ProviderKindInfo[];
  keys: Record<string, boolean>;
  inference_key_revisions?: Record<string, number>;
  cleanup_verification?: CleanupVerificationAvailability;
  dictation: { phase: 'idle' | 'listening' | 'processing'; text: string | null; level: number; session_id: string | null; practice: boolean; result_session_id: string | null; error: string | null; error_code?: string | null; cleanup_verification?: CleanupVerificationAvailability | null };
  session: { phase: 'off' | 'ready' | 'listening' | 'processing'; expires_at_ms: number | null; full_access_seen: boolean; keyboard_enabled?: boolean; error: string | null; error_code?: string | null; cleanup_verification?: CleanupVerificationAvailability | null };
  meeting: { phase: 'idle' | 'starting' | 'recording' | 'interrupted' | 'finishing' | 'error'; id: string | null; retained_id?: string | null; last_finished_id?: string | null; title: string; elapsed_secs: number; notes: string; transcript: Segment[]; pending: number; error: string | null; error_code?: string | null };
  cloud: CloudState;
  recognition_jobs: SpeakerJob[];
  navigation?: { sequence: number; route: 'dictation' | 'meeting' | 'sync'; meeting_id?: string } | null;
}
export interface NativeFailure { code: string; message: string; recoverable?: boolean }
export interface RuntimeClient { bootstrap(): Promise<RuntimeSnapshot>; getSnapshot(): Promise<RuntimeSnapshot>; invoke<T>(method: string, payload?: Record<string, unknown>): Promise<T>; subscribe(listener: (snapshot: RuntimeSnapshot) => void): () => void }

export type MobileSettings = Settings;
export type RecognitionSnapshot = SpeakerJob;
export interface SyncReport { status: string; uploaded: number; downloaded: number; conflicts: number; last_success_ms: number | null; error: string | null }

export interface KeyboardSettings {
  layout: 'qwerty' | 'azerty';
  suggestions: boolean;
  duration: number;
  keyboard_typing_languages: ('en' | 'nl')[];
  keyboard_autocorrection: boolean;
  keyboard_learning: boolean;
  keyboard_swipe: boolean;
  keyboard_snippet_expansion: boolean;
  keyboard_snippets: { id: string; title: string; keyword: string; text: string }[];
  keyboard_learned_count: number;
  keyboard_haptic_style: 'off' | 'gentle' | 'crisp';
  feedback_style: 'off' | 'gentle' | 'crisp';
  trial_active: boolean;
}

export interface MobileCommandResults {
  'keyboard.feedbackPreview': void;
  'keyboard.feedbackTrial': KeyboardSettings;
  'keyboard.learning.remove': KeyboardSettings;
  'inference.protocols': InferenceProtocolInfo[];
  'inference.availability': InferenceAvailability;
  'inference.decide': DecisionResult;
  'inference.rerank': RerankResult;
  'inference.verify': InferenceVerification;
  'inference.cancel': boolean;
  'meetings.list': MeetingInfo[];
  'meetings.read': MeetingDetail;
  'meetings.start': RuntimeSnapshot;
  'meetings.stop': RuntimeSnapshot;
  'meetings.retrySave': RuntimeSnapshot;
  'meetings.notes': void;
  'meetings.rename': string;
  'meetings.delete': void;
  'meetings.summarize': MeetingDetail;
  'meetings.templates': MeetingTemplate[];
  'meetings.ask': MeetingAnswer;
  'meetings.search': Passage[];
  'meetings.export': { title: string; markdown: string; html: string };
  'meetings.play': void;
  'meetings.stopPlayback': void;
  'meetings.retryTranscription': number;
  'meetings.conflictDelete': void;
  'dictation.start': RuntimeSnapshot;
  'dictation.stop': RuntimeSnapshot;
  'dictation.cancel': RuntimeSnapshot;
  'dictation.verificationAvailability': CleanupVerificationAvailability;
  'dictation.history': HistoryEntry[];
  'dictation.retry': { text: string; history_id: string };
  'dictation.deleteHistory': void;
  'dictation.copy': void;
  'session.start': RuntimeSnapshot;
  'session.stop': RuntimeSnapshot;
  'settings.read': Settings;
  'settings.update': Settings;
  'settings.keyboard': KeyboardSettings;
  'providers.add': Settings;
  'providers.remove': Settings;
  'providers.setKey': void;
  'providers.test': { ok: boolean; message: string };
  'providers.models': { id: string; name: string }[];
  'permissions.microphone': { granted: boolean };
  'system.openSettings': void;
  'ui.feedback': void;
  'cloud.status': { signed_in: boolean; account: CloudAccount | null; report: SyncReport };
  'cloud.loginStart': LoginStart;
  'cloud.loginPoll': { status: 'pending' | 'slow_down' | 'done' | 'expired' };
  'cloud.loginCancel': void;
  'cloud.logout': void;
  'cloud.sync': SyncReport;
  'cloud.setup': CloudSyncSetup;
  'cloud.prepareSync': CloudRecoveryDisplay;
  'cloud.activateSync': void;
  'cloud.resumeSync': void;
  'cloud.pendingRecovery': CloudRecoveryDisplay;
  'cloud.linkStart': CloudLinkView;
  'cloud.linkView': CloudLinkView;
  'cloud.linkJoin': CloudLinkView;
  'cloud.linkPoll': CloudLinkView;
  'cloud.linkApprove': CloudLinkView;
  'cloud.linkCancel': CloudLinkView;
  'cloud.scanConnectionCode': { code: string };
  'cloud.setSync': CloudRecoveryDisplay;
  'cloud.devices': CloudDevice[];
  'cloud.renameDevice': void;
  'cloud.revoke': void;
  'cloud.recoveryCreate': CloudRecoveryDisplay;
  'cloud.recoveryReset': CloudRecoveryDisplay;
  'cloud.recoveryPrint': void;
  'cloud.confirmDeletes': void;
  'navigation.consume': void;
  'cloud.recoveryUnlock': void;
  'cloud.recoveryConfirm': void;
  'cloud.recoveryCopy': void;
  'cloud.deleteAccount': void;
  'recognition.start': SpeakerJob;
  'recognition.get': SpeakerJob;
  'recognition.cancel': SpeakerJob;
  'recognition.resume': SpeakerJob;
  'recognition.list': SpeakerJob[];
  'recognition.renameSpeaker': void;
}
export type MobileCommand = keyof MobileCommandResults;
