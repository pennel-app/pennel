import type { ConfiguredDictationModel, PillModelPresentation } from "./pill-model-state";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type Event } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { ClaudeStatus } from "./claude-access";
import type { LoginPoll } from "./cloud-login";
export type { ClaudeStatus };
import type { WorkspaceRoute } from "./workspace-state";
import { meetingLevels, RecordingStopGate } from "./recording-control-state";
import { localizeUiResponse } from "./ui-response.ts";
import type { DesktopCommandState, DesktopEnvironment } from "./desktop-chrome-state.ts";
import type { PeopleSnapshot, PeopleRequest, PeopleUpdateResult, ContactPickResult } from "./people-state";
import type { ConversationSnapshotRequest, ConversationUpdateRequest, ConversationBeginRequest, ConversationCancelRequest,
  ConversationSourceRequest, ConversationSnapshot, ConversationResolvedSource } from "../../../assets/contracts/meeting-conversations";
export type { PeopleSnapshot, PeopleRequest, PeopleUpdateResult, ContactPickResult } from "./people-state";
import type { InsightSnapshot, InsightAvailability, InsightJob, InsightLanguagePolicy, InsightReviewRequest, InsightClaimRequest,
  InsightInsertionRequest, InsightInsertionPreparation, InsightReassociationRequest, InsightInsertionResolution } from "../../../assets/contracts/meeting-insights";

interface PlatformClient {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
  listen(event: string, handler: (event: { payload: unknown }) => void): Promise<() => void>;
  fixture?: { name: string; [key: string]: unknown };
}

declare global {
  interface Window { __PENNEL_PLATFORM__?: PlatformClient }
}

export function fixtureClient(): PlatformClient | undefined {
  return import.meta.env.DEV ? window.__PENNEL_PLATFORM__ : undefined;
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const client = fixtureClient();
  const value = client ? await client.invoke(command, args) : await tauriInvoke<T>(command, args);
  return localizeUiResponse(command, value) as T;
}

export function listen<T>(event: string, handler: (event: Event<T>) => void): Promise<() => void> {
  const client = fixtureClient();
  return client ? client.listen(event, (value) => handler({ event, id: 0, payload: value.payload as T })) : tauriListen<T>(event, handler);
}

/**
 * For events the backend sends to one window with `emit_to`. A plain `listen` also receives every
 * other window's targeted events, so a route meant for one window would make all windows navigate.
 */
export function listenToWindow<T>(event: string, handler: (event: Event<T>) => void): Promise<() => void> {
  const client = fixtureClient();
  return client ? listen(event, handler) : tauriListen<T>(event, handler, { target: getCurrentWebviewWindow().label });
}

export interface AppIdentity { name: string; icon: string }

export type ProviderKind =
  | "open_ai"
  | "open_router"
  | "groq"
  | "mistral"
  | "ollama"
  | "lm_studio"
  | "custom"
  | "local"
  | "apple_speech"
  | "apple_intelligence"
  | "claude_code"
  | "codex"
  | "open_code"
  | "copilot"
  | "cursor_agent"
  | "gemini_cli"
  | "pennel_cloud";

export type HotkeyTrigger = "fn" | "right_option" | "right_command" | "right_control" | "control_super";
export type Style = "casual" | "formal" | "literal" | "neutral";
export type RecordAction = "ask" | "always" | "never";
export type CapabilityStatus = "available" | "permission_required" | "unsupported" | "unavailable";
export type PermissionKind = "microphone" | "accessibility" | "calendar" | "speech" | "system_audio";
export type PermissionStatus = "granted" | "not_determined" | "denied" | "restricted" | "not_required" | "unsupported" | "unknown";
export type AppWindow = "library" | "settings" | "history" | "dictation" | "live" | "onboarding";
export interface MeetingLevel { session_id: string; level: number }
const recordingStops = new RecordingStopGate();

export type MeetingPhase = "idle" | "starting" | "recording" | "stopping" | "finalizing" | "error";
export type Speaker = "me" | "others";
export type AppearancePreference = "system" | "light" | "dark";
export type OnboardingStep = "language" | "welcome" | "engine" | "permissions" | "practice" | "voice";
export type OnboardingGoal = "dictation" | "meeting";
export type SpeakerRecognitionMode = "local" | "cloud";
export type SummaryAutomation = "unanswered" | "manual" | "automatic";
export type MeetingOutputScope = "personal" | "shareable";

export interface OnboardingProgress {
  version: number;
  status: "pending" | "skipped" | "completed";
  resume_step: OnboardingStep;
  goal?: OnboardingGoal;
}

export interface SetupReadiness {
  configured: boolean;
  can_record: boolean;
  can_use_shortcut: boolean;
  can_paste_externally: boolean;
  reason: string | null;
  resume_step: OnboardingStep;
}

export interface DictationSessionSnapshot {
  session_id: string;
  kind: "pad" | "practice" | "home_practice";
  phase: "idle" | "listening" | "working" | "done" | "error";
  text: string;
  error: string | null;
  target_name: string | null;
  external_paste_available: boolean;
  external_paste_reason: string | null;
  cleanup_verification?: CleanupVerificationAvailability | null;
}

export interface Capability {
  status: CapabilityStatus;
  reason: string | null;
}

export interface PermissionInfo {
  kind: PermissionKind;
  status: PermissionStatus;
}

export interface PlatformCapabilities {
  pad_external_paste: Capability;
  global_hotkey: Capability;
  record_shortcut: Capability;
  microphone: Capability;
  system_audio: Capability;
  echo_cancellation: Capability;
  call_detection: Capability;
  calendar: Capability;
  apple_speech: Capability;
  apple_intelligence: Capability;
  selection: Capability;
  rich_clipboard: Capability;
  audio_playback: Capability;
  diarization: Capability;
  meeting_app_speakers: Capability;
  local_models: Capability;
  local_acceleration: "metal" | "vulkan" | "cpu";
}

export interface ProviderAccount {
  id: string;
  kind: ProviderKind;
  name: string;
  base_url: string;
}

export interface ModelChoice {
  provider_id: string;
  model: string;
}

export interface ProviderTaskTestResult {
  verified: boolean;
  message: string;
}

export type InferenceCapability = "decision" | "reranking";
export type InferenceProtocol = "open_router_decisions" | "open_router_system_one" | "type_safe" | "perplexity" | "mercury" | "local_decider" | "tei" | "pennel_cloud";
export interface InferenceConfig {
  provider_id: string;
  protocol: InferenceProtocol;
  model: string;
  expected_model: string | null;
  upstream: string | null;
  max_input_bytes: number;
  timeout_secs: number;
}
export interface InferenceProtocolInfo {
  protocol: InferenceProtocol;
  name: string;
  default_base_url: string;
  decision: boolean;
  reranking: boolean;
  extraction: boolean;
  requires_key: boolean;
  allowed_provider_kinds: ProviderKind[];
  managed: boolean;
}
export interface InferenceAvailability {
  capability: InferenceCapability;
  configured: boolean;
  available: boolean;
  reason: string;
  provider_id: string | null;
  model: string | null;
  protocol: InferenceProtocol | null;
  endpoint: string | null;
}
export interface InferenceIdentity {
  protocol: InferenceProtocol;
  endpoint: string;
  requested_model: string;
  requested_upstream: string | null;
  actual_model: string | null;
  actual_provider: string | null;
  actual_revision: string | null;
  request_id: string | null;
}
export interface CleanupVerificationAvailability {
  enabled: boolean;
  available: boolean;
  reason_code: string;
  provider_id: string | null;
  model: string | null;
  endpoint: string | null;
}
export interface InferenceUsage { input_tokens: number | null; output_tokens: number | null; billed_cost_usd: number | null }
export type DecisionQuestionKind = "binary" | "choice" | "ordinal" | "extract";
export interface DecisionQuestion { id: string; kind: DecisionQuestionKind; instructions: string; options: { id: string; description: string }[] }
export interface DecisionRequest { state: string; questions: DecisionQuestion[] }
export interface DecisionProbability { option_id: string; probability: number }
export interface DecisionAnswer {
  id: string; kind: DecisionQuestionKind; choice: string | null; value: number | null;
  probabilities: DecisionProbability[]; confidence: number | null;
  spans: { field_id: string; text: string; start_char: number; end_char: number; confidence: number | null }[];
  not_stated: DecisionProbability[]; abstained: boolean;
}
export interface DecisionResult { answers: DecisionAnswer[]; identity: InferenceIdentity; usage: InferenceUsage }
export interface RerankRequest { query: string; passages: { id: string; text: string }[] }
export interface RerankResult { scores: { passage_id: string; score: number }[]; identity: InferenceIdentity; usage: InferenceUsage }
export interface InferenceVerification { verified: boolean; message: string; identity: InferenceIdentity | null }
export type { InsightItem, InsightSource, InsightSnapshot, InsightAvailability, InsightJob, InsightLanguagePolicy, InsightReviewState,
  InsightReviewRequest, InsightClaimRequest, InsightInsertionRequest, InsightInsertionPreparation, InsightReassociationRequest } from "../../../assets/contracts/meeting-insights";

export interface ClaudeAccessView {
  enabled: boolean;
  status: ClaudeStatus;
  mcp_url: string;
}

export interface Settings {
  ui_language: "system" | "en" | "nl";
  appearance: AppearancePreference;
  onboarding: OnboardingProgress;
  dictation_pad_auto_paste: boolean;
  live_text_enabled: boolean;
  providers: ProviderAccount[];
  dictation_stt: ModelChoice;
  dictation_models?: ModelChoice[];
  pill_model_switch_enabled?: boolean;
  meeting_stt: ModelChoice;
  llm: ModelChoice;
  decision?: InferenceConfig | null;
  reranking?: InferenceConfig | null;
  cleanup_verification_enabled?: boolean;
  mode: "fast" | "clean";
  language: string | null;
  dictionary: string[];
  style_rules: { bundle_id: string; style: Style }[];
  history_retention_days: number;
  hotkey: HotkeyTrigger;
  shortcuts?: Record<string, ShortcutBinding | null>;
  local_unload_minutes: number;
  meetings_dir: string;
  default_template: string;
  summary_automation: SummaryAutomation;
  meeting_audio_retention_days: number;
  app_rules: { bundle_id: string; action: RecordAction }[];
  calendar_rules: { calendar_id: string; calendar_name: string; action: RecordAction }[];
  event_rules: { title: string; action: RecordAction }[];
  calendar_enabled: boolean;
  echo_cancellation: boolean;
  record_shortcut_enabled: boolean;
  consent_enabled: boolean;
  auto_detect: boolean;
  diarization: boolean;
  meeting_app_speakers: boolean;
  meeting_app_name_policy?: "suggest" | "automatic";
  speaker_recognition: SpeakerRecognitionMode;
  claude_access?: boolean;
  [key: string]: unknown;
}

export interface KindInfo {
  kind: ProviderKind;
  label: string;
  requires_key: boolean;
  default_base_url: string;
  stt: boolean;
  llm: boolean;
  default_stt_model: string;
  default_llm_model: string;
  is_cli_agent: boolean;
  availability: Capability;
}

export interface ShortcutBinding { mods: string[]; code: string | null; key: string | null }
export interface ShortcutLabel { keycaps: string[]; text: string }
export interface ShortcutRow {
  id: string;
  scope: "global_hold" | "listening" | "global" | "application";
  binding: ShortcutBinding | null;
  default_binding: ShortcutBinding | null;
  label: ShortcutLabel;
  editable: boolean;
  capability: { available: boolean; reason: string | null };
  changed: boolean;
}
export interface AppInfo {
  shortcuts?: ShortcutRow[];
  interface_language: "en" | "nl";
  system_interface_language: "en" | "nl";
  settings: Settings;
  keys: Record<string, boolean>;
  provider_revisions?: Record<string, number>;
  cloud_account_epoch?: string;
  cloud_signed_in?: boolean;
  cleanup_verification?: CleanupVerificationAvailability;
  active_model_downloads?: string[];
  unsaved_history_count?: number;
  kinds: KindInfo[];
  recording_app_ids: string[];
  platform: string;
  version: string;
  hotkey_error: string | null;
  log_path: string | null;
  data_dir: string;
  capabilities: PlatformCapabilities;
  diarizer_model_id: string;
  preview: boolean;
  migration_warning: string | null;
}

export interface RemoteModel {
  id: string;
  name: string | null;
  audio_input: boolean;
  is_router: boolean;
}

export type ModelState = { kind: "not_downloaded" } | { kind: "partial"; bytes: number } | { kind: "downloaded" };

export interface LocalModel {
  id: string;
  name: string;
  description: string;
  size_bytes: number;
  parameters: string | null;
  speed: number;
  accuracy: number | null;
  languages: string[];
  recommended: boolean;
  legacy: boolean;
  state: ModelState;
}

export interface HistoryEntry {
  id: string;
  created_at_ms: number;
  app_bundle_id: string | null;
  raw: string;
  text: string;
  status: "done" | "failed" | "unsaved";
  error: string | null;
  has_audio: boolean;
}

export interface MeetingInfo {
  id: string;
  title: string;
  start_iso: string;
  duration_min: number;
  has_summary: boolean;
}

export interface Segment {
  start_ms: number;
  end_ms: number;
  speaker: Speaker;
  text: string;
  failed: boolean;
  voice: number | null;
  utterance_id?: string | null;
  source_id?: MeetingAudioSource | null;
  source_fingerprint?: string | null;
  attribution?: SpeakerAttribution | null;
  attribution_revision?: number;
  correction_review_required?: boolean;
  confirmed_speaker_name?: string | null;
  automatic_speaker_name?: string | null;
}

export type MeetingAudioSource = "microphone" | "system_audio" | "system" | "room_microphone";
export interface SpeakerAttribution { kind: "unknown" | "single" | "mixed" | "overlap"; voices: number[]; self_voice?: boolean; provisional?: boolean }
export type TranscriptMetadataStatus = "valid" | "absent" | "pending" | "stale" | "unsupported" | "corrupt";
export interface LiveSpeakerAttributionStatus {
  enabled: boolean;
  available: boolean;
  state: "off" | "warming" | "running" | "busy" | "paused" | "unavailable" | "failed" | "stopped";
  reason_code?: "local_required" | "models_missing" | "unsupported" | "configuration_changed" | "foreground_work" | "busy" | "paused" | "failed" | "stopped" | null;
  revision: number;
  model?: string | null;
  destination: "local" | null;
}

export interface MeetingStatus {
  meeting_uid?: string | null;
  transcript_revision?: number;
  metadata_status?: TranscriptMetadataStatus;
  live_speaker_attribution?: LiveSpeakerAttributionStatus | null;
  preview?: MeetingPreviewSnapshot | null;
  recognition?: { provider_kind: ProviderKind | null; provider_name: string; model: string; language: string | null };
  recording: boolean;
  phase: MeetingPhase;
  busy: boolean;
  error: string | null;
  notes: string;
  session_id: string | null;
  id: string | null;
  title: string | null;
  elapsed_secs: number;
  system_audio: boolean;
  heard_system_audio: boolean;
  echo_cancellation_active?: boolean;
  audio_warning: string | null;
  persistence_error?: string | null;
  transcription_warning?: string | null;
  terminal_error?: string | null;
  can_retry_persistence?: boolean;
  consent_copied?: boolean;
  pending: number;
  transcript: Segment[];
}

export type MeetingPreviewState = "disabled" | "warming" | "ready" | "busy" | "failed" | "unsupported" | "stopped";
export interface MeetingTrackPreview {
  source: Speaker;
  revision: number;
  epoch: number;
  chunk_start_ms: number;
  start_ms: number;
  end_ms: number;
  text: string;
  mode: "local" | "system" | "after_recording";
  state: MeetingPreviewState;
}
export interface MeetingPreviewSnapshot {
  session_id: string;
  revision: number;
  transcript_revision: number;
  tracks: MeetingTrackPreview[];
}

export type MeetingRuntimeStatus = Omit<MeetingStatus, "notes" | "transcript" | "transcript_revision"> & { transcript_revision: number };

export interface MeetingFinished {
  meeting_uid?: string | null;
  id: string | null;
  session_id: string;
  error: string | null;
  flush_warning: string | null;
}

export interface LiveSourceAnchor {
  passage: Passage;
  segment_keys: string[];
  segment_fingerprints: string[];
  notes_start: number | null;
  notes_end: number | null;
  source_text: string;
}

export interface MeetingQuestionSelection {
  scope: "whole_meeting" | "selected_excerpts";
  available_sources: number;
  included_sources: number;
  available_history_turns: number;
  included_history_turns: number;
  history_reduced: boolean;
}

export interface LiveQuestionSnapshot {
  request_id: string;
  meeting_uid: string;
  session_id: string;
  transcript_revision: number;
  notes_revision: string;
  included_through_ms: number;
  pending_transcription: number;
  anchors: LiveSourceAnchor[];
  selection?: MeetingQuestionSelection;
}

export interface LiveQuestionResult {
  snapshot: LiveQuestionSnapshot;
  answer: MeetingAnswer | null;
  error: string | null;
}

export interface LiveQuestionRequest {
  requestId: string;
  meetingUid: string;
  sessionId: string;
  expectedNotesText: string;
  question: string;
  history: ChatTurn[];
}

export interface MeetingDetail {
  meeting_uid?: string;
  metadata_status?: TranscriptMetadataStatus;
  revision?: number;
  info: MeetingInfo | null;
  notes: string;
  summary: string | null;
  transcript: Segment[];
  speakers: { voice: number; name: string }[];
  dir: string;
  attendees: string[];
  audio_tracks: { speaker: Speaker; source_id?: MeetingAudioSource | null; available: boolean }[];
  summary_outdated: boolean;
  retranscription_plan: MeetingRetranscriptionPlan;
  retranscription_status: MeetingRetranscriptionStatus | null;
  failed_recovery?: FailedTranscriptionRecoveryPlan;
}

export interface FailedTranscriptionRecoveryPlan {
  failed_count: number | null;
  retryable_count: number | null;
  status: "complete" | "retryable" | "unavailable";
  reason_code: string | null;
}

export interface MeetingRetranscriptionPlan {
  recognition: NonNullable<MeetingStatus["recognition"]>;
  available: boolean;
  reason: string | null;
}

export interface MeetingRetranscriptionStatus {
  running: boolean;
  recognition: NonNullable<MeetingStatus["recognition"]>;
  progress: {
    phase: "decoding" | "transcribing" | "speakers" | "saving" | "completed" | "failed";
    speaker: Speaker | null;
    completed: number;
    total: number;
  };
  error: string | null;
  warning: string | null;
}

export interface MeetingTemplate {
  id: string;
  name: string;
  builtin: boolean;
  customized: boolean;
}

export interface Passage {
  meeting_id: string;
  title: string;
  start_iso: string;
  kind: "notes" | "transcript" | "summary";
  start_ms: number | null;
  text: string;
  snippet: string;
  meeting_uid?: string | null;
  utterance_ids?: string[];
  utterance_fingerprints?: string[];
}

export interface LibrarySearch {
  meetings: MeetingInfo[];
  passages: Passage[];
}

export interface ChatTurn {
  question: string;
  answer: string;
}

export interface MeetingAnswer {
  text: string;
  citations: { number: number; passage: Passage }[];
}

export interface MeetingExport {
  title: string;
  markdown: string;
  html: string;
}

export interface SummaryProvenance {
  provider_id: string;
  provider_kind: ProviderKind;
  model: string;
  generated_at: string;
  template_id: string;
}
export interface SummarySource { utterance_id: string; start_ms: number; speaker: string | null }
export interface SummaryItem { text: string; source: SummarySource | null }
export interface SummaryAction extends SummaryItem {
  owner: string | null;
  due: string | null;
  state: { done: boolean; owner: string | null };
}
export interface SummaryStructure {
  labels: { summary: string; decisions: string; actions: string; open_questions: string; none: string };
  lead: string;
  topics: { title: string; body: string; source: SummarySource | null }[];
  decisions: SummaryItem[];
  actions: SummaryAction[];
  open_questions: SummaryItem[];
}
export interface MeetingSummaryRecord {
  schema_version: number;
  meeting_uid: string;
  body_revision: string;
  provenance: SummaryProvenance;
  structure: SummaryStructure | null;
}

export interface SpeakerTimelineSpan { start_ms: number; end_ms: number }
export interface SpeakerTimelineLane {
  key: string;
  speaker: string | null;
  kind: "person" | "unknown" | "mixed" | "overlap";
  duration_ms: number;
  spans: SpeakerTimelineSpan[];
}
export interface MeetingTranscriptProvenance {
  provider_kind: string;
  model: string;
  requested_language: string | null;
  generated_at: string;
}

export interface MeetingSpeakerTimeline {
  duration_ms: number;
  duration_is_recorded: boolean;
  lanes: SpeakerTimelineLane[];
}

export interface MeetingSummaryDocument {
  record?: MeetingSummaryRecord | null;
  meeting_uid: string;
  markdown: string;
  revision: string;
  source_revision: string;
  edited: boolean;
  outdated: boolean;
}

export interface MeetingSummaryCandidate {
  record?: MeetingSummaryRecord | null;
  meeting_uid: string;
  markdown: string;
  revision: string;
  source_revision: string;
}

export interface MeetingSummaryRevision {
  revision: string;
  markdown: string;
  source_revision: string;
  created_at: string;
}

export interface ShareableMeetingSummary {
  meeting_uid: string;
  markdown: string;
  source_revision: string;
  generated_at: string;
  provider_id: string;
  model: string;
  provenance: string;
  outdated: boolean;
}

export interface DetectedAgent {
  kind: ProviderKind;
  path: string;
}

export interface CalendarSnapshot {
  has_access: boolean;
  calendars: { id: string; name: string }[];
  current_event: {
    title: string;
    calendar_id: string;
    calendar_name: string;
    attendees: string[];
  } | null;
}

export interface RecordPrompt {
  id: number;
  app_id: string;
  app_name: string;
  title: string | null;
  expires_at_ms: number;
}

export interface PillPreview {
  session_id: string;
  revision: number;
  text: string;
}

export type PillState = (
  | { state: "hidden" | "listening" | "hands_free" | "command" | "no_sound" | "working" }
  | { state: "inserted"; app?: string | null }
  | { state: "copied"; paste_key: string }
  | { state: "problem"; title: string; detail?: string | null }
) & { preview?: PillPreview | null; cleanup_verification?: CleanupVerificationAvailability | null; speech_model?: PillModelPresentation | null; cancel_shortcut?: ShortcutLabel | null };

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  body: string | null;
}

export type SyncStatus =
  | { kind: "off" | "signed_out" | "needs_setup" | "needs_recovery_code" | "vault_reset" | "pending" | "synced" | "offline" }
  | { kind: "storage_full" | "error"; message: string };

export interface SyncReport {
  status: SyncStatus;
  brake: boolean;
  uploaded: number;
  downloaded: number;
  conflicts: number;
  last_success_ms: number | null;
}

export interface CloudSyncSetup {
  phase: "signed_out" | "new_library" | "locked" | "ready" | "paused" | "active";
  sync_enabled: boolean;
  generation: number | null;
  can_approve_devices: boolean;
  uploaded: number;
  downloaded: number;
  conflicts: number;
  last_success_ms: number | null;
}

export type CloudLinkRole = "new_device" | "trusted_device";
export interface CloudLinkAvailability { available: boolean; reason_code: string | null }
export type CloudLinkPhase = "idle" | "waiting_peer" | "confirming" | "awaiting_approval" | "waiting_approval" | "provisioning" | "complete" | "cancelled" | "expired" | "failed";
export interface CloudLinkView {
  phase: CloudLinkPhase;
  role: CloudLinkRole | null;
  request_id: string | null;
  connection_code: string | null;
  qr_payload: string | null;
  qr_svg: string | null;
  verification_code: string | null;
  peer_name: string | null;
  peer_platform: string | null;
  account_email: string | null;
  expires_at_ms: number | null;
  error_code: string | null;
  error_message: string | null;
}

export interface SpeakerRecognition {
  voices: number;
  named: number;
  me_found: boolean;
  complete?: boolean;
  unassigned_voices?: number;
}

export interface SpeakerProcessingStatus {
  phase: string;
  reason: string | null;
  route: string;
  model_ids: string[];
  complete: boolean;
  unassigned_voices: number;
}

export interface VoiceProfile {
  created_iso: string;
  speech_seconds: number;
}

export interface KnownVoice {
  id: string;
  name: string;
  meetings: number;
  updated_iso: string;
}

export interface MeetingVoice {
  voice: number;
  name: string | null;
  remembered: boolean;
  can_remember: boolean;
}

export interface VoiceNameSuggestion {
  voice: number;
  name: string;
  app_id: string;
  agreement: number;
  coverage: number;
  shared_device: boolean;
}

export interface VoiceNameCandidate {
  name: string;
  source: "invited" | "meeting_app" | "known_voice";
}

export interface VoiceNaming {
  suggestions: VoiceNameSuggestion[];
  candidates: VoiceNameCandidate[];
  automatic_names?: VoiceNameSuggestion[];
  status?: "disabled" | "unsupported" | "permission_denied" | "unavailable" | "unbound" | "insufficient_evidence" | "conflict" | "suggested" | "automatic_ready" | "applied";
  reason?: string | null;
}

export interface SpeakerSetup {
  local_available: boolean;
  ready: boolean;
  signed_in: boolean;
  model_ids: string[];
  legacy_model: boolean;
  profile: VoiceProfile | null;
  known: KnownVoice[];
  enrollment_text: string;
}

export interface EnrollmentProgress {
  seconds: number;
  level: number;
  stopped: boolean;
}

export interface CloudLoginStart {
  user_code: string;
  verification_uri: string;
  device_code: string;
  interval_secs: number;
}

export interface CloudAccount {
  email: string;
  audio_seconds: number;
  tokens: number;
  storage_bytes: number;
  limit_audio_seconds: number;
  limit_tokens: number;
  limit_storage_bytes: number;
}

export interface CloudDevice {
  id: string;
  name: string;
  platform: string;
  last_seen_iso: string;
  current: boolean;
}

export interface MeetingSyncState {
  id: string;
  state: "synced" | "pending" | "conflict";
}

export interface CloudError {
  code: string;
  message: string;
}

export interface WritingProfileTrait {
  key: string;
  value: string;
  support: number;
  overridden: boolean;
}

export interface WritingProfileLanguage {
  language: string;
  word_count: number;
  session_count: number;
  status: "ready" | "insufficient_evidence";
  traits: WritingProfileTrait[];
  guidance: string[];
}

export interface WritingProfileSource {
  id: string;
  session_id: string;
  kind: "dictation" | "meeting";
  created_at_ms: number;
  title: string;
  fingerprint: string;
  word_count: number;
  language: string | null;
  excluded: boolean;
  analyzed: boolean;
}

export interface WritingProfileStatus {
  available: boolean;
  learning_enabled: boolean;
  sharing_enabled: boolean;
  started_at_ms: number | null;
  revision: number;
  processing?: boolean;
  error: string | null;
  min_words: number;
  min_sessions: number;
  languages: WritingProfileLanguage[];
  sources: WritingProfileSource[];
  trait_options: { key: string; values: string[] }[];
}

export interface WritingProfileConnection {
  enabled: boolean;
  signed_in: boolean;
  designated: boolean;
  mcp_url: string;
  error?: string | null;
}

export const api = {
  writingProfileStatus: () => invoke<WritingProfileStatus>("writing_profile_status"),
  setWritingProfileLearning: (enabled: boolean) => invoke<WritingProfileStatus>("set_writing_profile_learning", { enabled }),
  refreshWritingProfile: () => invoke<WritingProfileStatus>("refresh_writing_profile"),
  setWritingProfileSourceExcluded: (sourceId: string, excluded: boolean) => invoke<WritingProfileStatus>("set_writing_profile_source_excluded", { sourceId, excluded }),
  setWritingProfileTrait: (language: string, traitKey: string, value: string | null) => invoke<WritingProfileStatus>("set_writing_profile_trait", { language, traitKey, value }),
  deleteWritingProfile: () => invoke<WritingProfileStatus>("delete_writing_profile"),
  writingProfileConnection: () => invoke<WritingProfileConnection>("writing_profile_connection"),
  setWritingProfileSharing: (enabled: boolean) => invoke<WritingProfileConnection>("set_writing_profile_sharing", { enabled }),
  onWritingProfileChanged: (handler: () => void): Promise<() => void> =>
    listen<null>("writing-profile-changed", handler),
  insightsSnapshot: (id: string) => invoke<InsightSnapshot>("insights_snapshot", { id }),
  automaticInsightsAvailability: (id: string) => invoke<InsightAvailability>("automatic_insights_availability", { id }),
  insightsAvailability: (id: string, languagePolicy: InsightLanguagePolicy) => invoke<InsightAvailability>("insights_availability", { id, languagePolicy }),
  beginInsightsAnalysis: (id: string, languagePolicy: InsightLanguagePolicy) => invoke<InsightJob>("begin_insights_analysis", { id, languagePolicy }),
  insightsJob: (jobId: string) => invoke<InsightJob>("insights_job", { jobId }),
  cancelInsightsAnalysis: (jobId: string) => invoke<InsightJob>("cancel_insights_analysis", { jobId }),
  reviewInsight: (id: string, request: InsightReviewRequest) => invoke<InsightSnapshot>("review_insight", { id, request }),
  claimInsightEdit: (id: string, request: InsightClaimRequest) => invoke<InsightSnapshot>("claim_insight_edit", { id, request }),
  prepareInsightInsertion: (id: string, request: InsightInsertionRequest) => invoke<InsightInsertionPreparation>("prepare_insight_insertion", { id, request }),
  reconcileInsightInsertions: (id: string, storeRevision: number) => invoke<InsightSnapshot>("reconcile_insight_insertions", { id, storeRevision }),
  resolveInsightInsertion: (id: string, request: InsightInsertionResolution) => invoke<InsightSnapshot>("resolve_insight_insertion", { id, request }),
  reassociateInsightSource: (id: string, request: InsightReassociationRequest) => invoke<InsightSnapshot>("reassociate_insight_source", { id, request }),
  exportInsights: (id: string) => invoke<string>("insights_export", { id }),
  insightMarkdown: (id: string, itemId: string, itemRevision: number) => invoke<string>("insight_markdown", { id, itemId, itemRevision }),
  desktopEnvironment: () => invoke<DesktopEnvironment>("desktop_environment"),
  setDesktopCommandState: (state: DesktopCommandState) => invoke<void>("set_desktop_command_state", { state }),
  info: () => invoke<AppInfo>("app_info"),
  setShortcut: (action: string, binding: ShortcutBinding | null, replace = false) => invoke<AppInfo>("set_shortcut", { action, binding, replace }),
  resetShortcut: (action: string) => invoke<AppInfo>("reset_shortcut", { action }),
  resetAllShortcuts: () => invoke<AppInfo>("reset_all_shortcuts"),
  shortcutCaptureStart: (action: string) => invoke<import("./shortcut-capture-state").CaptureStarted>("shortcut_capture_start", { action }),
  shortcutCaptureStop: () => invoke<void>("shortcut_capture_stop"),
  saveSettings: (settings: Settings) => invoke<AppInfo>("save_settings", { settings }),
  configuredDictationModels: () => invoke<ConfiguredDictationModel[]>("configured_dictation_models"),
  selectDictationModel: (choice: ModelChoice) => invoke<AppInfo>("select_dictation_model", { choice }),
  applyTaskModelChoice: (slot: "dictation_stt" | "meeting_stt" | "llm", choice: ModelChoice) =>
    invoke<AppInfo>("apply_task_model_choice", { slot, choice }),
  addProvider: (kind: ProviderKind) => invoke<AppInfo>("add_provider", { kind }),
  removeProvider: (id: string) => invoke<AppInfo>("remove_provider", { id }),
  setKey: (account: string, key: string) => invoke<AppInfo>("set_key", { account, key }),
  permissions: () => invoke<PermissionInfo[]>("get_permissions"),
  requestPermission: (kind: PermissionKind) => invoke<PermissionInfo[]>("request_permission", { kind }),
  testAppleIntelligence: () => invoke<void>("test_apple_intelligence"),
  calendar: () => invoke<CalendarSnapshot>("calendar_snapshot"),
  chooseMeetingsDir: () => invoke<string | null>("choose_meetings_dir"),
  showWindow: (kind: AppWindow, tab: string | null = null) => invoke<void>("show_window", { kind, tab }),
  copySecretText: (text: string) => invoke<void>("copy_secret_text", { text }),
  showWorkspace: (route: WorkspaceRoute) => invoke<void>("show_workspace", { route }),
  copyText: (text: string, html: string | null = null) => invoke<void>("copy_text", { text, html }),
  playMeetingAudio: (id: string, speaker: Speaker, startMs: number) =>
    invoke<void>("play_meeting_audio", { id, speaker, startMs }),
  playMeetingUtteranceAudio: (id: string, utteranceId: string) =>
    invoke<void>("play_meeting_utterance_audio", { id, utteranceId }),
  stopAudio: () => invoke<void>("stop_audio"),
  recordPrompt: () => invoke<RecordPrompt | null>("record_prompt"),
  answerRecordPrompt: (answer: "yes" | "no" | "never", promptId: number) =>
    invoke<void>("answer_record_prompt", { answer, promptId }),
  toggleDictation: (copyOnly = true) => invoke<void>("toggle_dictation", { copyOnly }),
  dictationState: () => invoke<PillState>("dictation_state"),
  onboardingReadiness: (target: "dictation" | "global_dictation" | "dictation_pad" | "practice" | "meeting") =>
    invoke<SetupReadiness>("onboarding_readiness", { target }),
  openDictationSession: (kind: DictationSessionSnapshot["kind"]) => invoke<DictationSessionSnapshot>("open_dictation_session", { kind }),
  getDictationSession: (sessionId: string) => invoke<DictationSessionSnapshot>("get_dictation_session", { sessionId }),
  toggleDictationSession: (sessionId: string) => invoke<void>("toggle_dictation_session", { sessionId }),
  cancelDictationSession: (sessionId: string) => invoke<void>("cancel_dictation_session", { sessionId }),
  closeDictationSession: (sessionId: string, onlyIfIdle = false) => invoke<void>("close_dictation_session", { sessionId, onlyIfIdle }),
  padText: () => invoke<string>("pad_text"),
  savePadText: (text: string) => invoke<void>("save_pad_text", { text }),
  checkForUpdates: () => invoke<UpdateInfo>("check_for_updates"),
  installUpdate: () => invoke<void>("install_update"),
  frontendReady: () => invoke<void>("frontend_ready"),
  frontendFlushed: (requestId: number, success: boolean, error: string | null = null) =>
    invoke<void>("frontend_flushed", { requestId, success, error }),
  shareMeeting: (id: string, includeTranscript: boolean, action: "mail" | "share") =>
    invoke<void>("share_meeting", { id, includeTranscript, action }),
  sharePreparedMeeting: (output: MeetingExport, action: "mail" | "share") =>
    invoke<void>("share_prepared_meeting", { output, action }),
  exportPreparedMeeting: (output: MeetingExport) => invoke<boolean>("export_prepared_meeting", { output }),
  markLines: (enhanced: string, notes: string) => invoke<{ text: string; mine: boolean }[]>("mark_lines", { enhanced, notes }),
  detectCliAgents: () => invoke<DetectedAgent[]>("detect_cli_agents"),
  appIdentity: (id: string) => invoke<AppIdentity | null>("app_identity", { id }),
  pickStyleApplication: () => invoke<{ id: string; name: string } | null>("pick_style_application"),
  checkCliAgent: (id: string, model: string) => invoke<string>("check_cli_agent", { id, model }),
  providerModels: (id: string) => invoke<RemoteModel[]>("provider_models", { id }),
  localModels: (language: string | null, supportedOnly: boolean) =>
    invoke<LocalModel[]>("local_models", { language, supportedOnly }),
  modelState: (id: string) => invoke<ModelState>("model_state", { id }),
  downloadModel: (id: string) => invoke<void>("download_model", { id }),
  joinModelDownload: (id: string) => invoke<boolean>("join_model_download", { id }),
  cancelDownload: (id: string) => invoke<void>("cancel_download", { id }),
  deleteModel: (id: string) => invoke<AppInfo>("delete_model", { id }),
  history: (limit: number) => invoke<HistoryEntry[]>("history", { limit }),
  retryHistory: (id: string) => invoke<string>("retry_history", { id }),
  saveHistory: (id: string) => invoke<void>("save_history", { id }),
  copyHistory: (id: string) => invoke<void>("copy_history", { id }),
  testProviderTask: (slot: "dictation_stt" | "meeting_stt" | "llm") =>
    invoke<ProviderTaskTestResult>("test_provider_task", { slot }),
  testProviderTaskChoice: (slot: "dictation_stt" | "meeting_stt" | "llm", choice: ModelChoice) =>
    invoke<ProviderTaskTestResult>("test_provider_task_choice", { slot, choice }),
  inferenceProtocols: () => invoke<InferenceProtocolInfo[]>("inference_protocols"),
  inferenceAvailability: (capability: InferenceCapability) => invoke<InferenceAvailability>("inference_availability", { capability }),
  cleanupVerificationAvailability: () => invoke<CleanupVerificationAvailability>("cleanup_verification_availability"),
  decide: (request: DecisionRequest, requestId: string) => invoke<DecisionResult>("inference_decide", { request, requestId }),
  rerank: (request: RerankRequest, requestId: string) => invoke<RerankResult>("inference_rerank", { request, requestId }),
  verifyInference: (capability: InferenceCapability, requestId: string) => invoke<InferenceVerification>("verify_inference", { capability, requestId }),
  cancelInference: (requestId: string) => invoke<boolean>("cancel_inference", { requestId }),
  deleteHistory: (id: string) => invoke<void>("delete_history", { id }),
  openLog: () => invoke<void>("open_log"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  meetingStatus: () => invoke<MeetingStatus>("meeting_status"),
  setLiveSpeakerAttribution: (sessionId: string, enabled: boolean) =>
    invoke<LiveSpeakerAttributionStatus>("set_live_speaker_attribution", { sessionId, enabled }),
  meetingRuntimeStatus: () => invoke<MeetingRuntimeStatus>("meeting_runtime_status"),
  startMeeting: (title: string) => invoke<string>("start_meeting", { title }),
  stopMeeting: (sessionId?: string) => {
    if (!sessionId) return invoke<void>("stop_meeting");
    meetingLevels.stop(sessionId);
    return recordingStops.run(sessionId, () => invoke<void>("stop_meeting", { sessionId }));
  },
  retryMeetingPersistence: (sessionId: string) => invoke<void>("retry_meeting_persistence", { sessionId }),
  setMeetingNotes: (text: string, sessionId: string) => invoke<void>("set_meeting_notes", { text, sessionId }),
  setMeetingTitle: (title: string, sessionId: string) => invoke<void>("set_meeting_title", { title, sessionId }),
  meetings: (query: string) => invoke<MeetingInfo[]>("meetings", { query }),
  searchLibrary: (query: string) => invoke<LibrarySearch>("search_library", { query }),
  askMeetings: (question: string, meetingId: string | null, history: ChatTurn[]) =>
    invoke<MeetingAnswer>("ask_meetings", { question, meetingId, history }),
  conversationSnapshot: (request: ConversationSnapshotRequest) => invoke<ConversationSnapshot>("conversation_snapshot", { request }),
  updateConversation: (request: ConversationUpdateRequest) => invoke<ConversationSnapshot>("update_conversation", { request }),
  beginConversationQuestion: (request: ConversationBeginRequest) => invoke<ConversationSnapshot>("begin_conversation_question", { request }),
  cancelConversationQuestion: (request: ConversationCancelRequest) => invoke<ConversationSnapshot>("cancel_conversation_question", { request }),
  conversationSource: (request: ConversationSourceRequest) => invoke<ConversationResolvedSource>("conversation_source", { request }),
  beginLiveQuestion: (request: LiveQuestionRequest) => invoke<LiveQuestionSnapshot>("begin_live_question", { ...request }),
  cancelLiveQuestion: (requestId: string, meetingUid: string, sessionId: string) =>
    invoke<boolean>("cancel_live_question", { requestId, meetingUid, sessionId }),
  meetingDetail: (id: string) => invoke<MeetingDetail>("meeting_detail", { id }),
  meetingTemplates: () => invoke<MeetingTemplate[]>("meeting_templates"),
  templateSource: (id: string) => invoke<string | null>("template_source", { id }),
  saveTemplate: (id: string | null, name: string, body: string) =>
    invoke<string>("save_template", { id, name, body }),
  deleteTemplate: (id: string) => invoke<void>("delete_template", { id }),
  summarizeMeeting: (id: string, templateId: string) =>
    invoke<string>("summarize_meeting", { id, templateId }),
  meetingTranscriptProvenance: (id: string) => invoke<MeetingTranscriptProvenance | null>("meeting_transcript_provenance", { id }),
  meetingSpeakerTimeline: (id: string) => invoke<MeetingSpeakerTimeline>("meeting_speaker_timeline", { id }),
  meetingSummary: (id: string) => invoke<MeetingSummaryDocument>("meeting_summary", { id }),
  saveMeetingSummary: (id: string, expectedRevision: string, expectedSourceRevision: string, markdown: string) =>
    invoke<MeetingSummaryDocument>("save_meeting_summary", { id, expectedRevision, expectedSourceRevision, markdown }),
  acceptMeetingSummary: (id: string, expectedRevision: string, expectedSourceRevision: string, markdown: string, record?: MeetingSummaryRecord | null) =>
    invoke<MeetingSummaryDocument>("accept_meeting_summary", { id, expectedRevision, expectedSourceRevision, markdown, record }),
  updateMeetingSummaryAction: (id: string, expectedRevision: string, expectedSourceRevision: string, actionIndex: number, done: boolean, owner: string | null) =>
    invoke<MeetingSummaryDocument>("update_meeting_summary_action", { id, expectedRevision, expectedSourceRevision, actionIndex, done, owner }),
  meetingSummaryHistory: (id: string) => invoke<MeetingSummaryRevision[]>("meeting_summary_history", { id }),
  restoreMeetingSummary: (id: string, revision: string, expectedRevision: string, expectedSourceRevision: string) =>
    invoke<MeetingSummaryDocument>("restore_meeting_summary", { id, revision, expectedRevision, expectedSourceRevision }),
  generateMeetingSummary: (id: string, templateId?: string) =>
    invoke<MeetingSummaryCandidate>("generate_meeting_summary", { id, templateId }),
  shareableMeetingSummary: (id: string) => invoke<ShareableMeetingSummary | null>("shareable_meeting_summary", { id }),
  generateShareableMeetingSummary: (id: string, templateId?: string) =>
    invoke<ShareableMeetingSummary>("generate_shareable_meeting_summary", { id, templateId }),
  previewMeetingExport: (id: string, scope: MeetingOutputScope, includeTranscript: boolean, language?: "en" | "nl") =>
    invoke<MeetingExport>("preview_meeting_export", { id, scope, includeTranscript, language }),
  retranscribeMeeting: (id: string) => invoke<number>("retranscribe_meeting", { id }),
  retranscribeMeetingAudio: (id: string) => invoke<number>("retranscribe_meeting_audio", { id }),
  saveMeetingNotes: (id: string, text: string, insightReceipt?: string) => invoke<void>("save_meeting_notes", { id, text, insightReceipt }),
  setMeetingAttendees: (id: string, attendees: string[]) => invoke<void>("set_meeting_attendees", { id, attendees }),
  meetingPeople: (id: string) => invoke<PeopleSnapshot>("meeting_people_snapshot", { id }),
  updateMeetingPeople: (id: string, request: PeopleRequest, sessionId?: string) => invoke<PeopleUpdateResult>("update_meeting_people", { id, request, sessionId }),
  pickMeetingContact: () => invoke<ContactPickResult>("pick_meeting_contact"),
  renameSpeaker: (id: string, voice: number, name: string) => invoke<void>("rename_speaker", { id, voice, name }),
  recognizeSpeakers: (id: string) => invoke<SpeakerRecognition>("diarize_meeting", { id }),
  confirmMicrophoneOwner: (id: string) => invoke<void>("confirm_meeting_microphone_owner", { id }),
  speakerProcessingStatus: (id: string) => invoke<SpeakerProcessingStatus | null>("speaker_processing_status", { id }),
  meetingVoices: (id: string) => invoke<MeetingVoice[]>("meeting_voices", { id }),
  meetingVoiceNaming: (id: string) => invoke<VoiceNaming>("meeting_voice_naming", { id }),
  rememberSpeaker: (id: string, voice: number, name: string) => invoke<void>("remember_speaker", { id, voice, name }),
  speakerSetup: () => invoke<SpeakerSetup>("speaker_setup"),
  startVoiceEnrollment: () => invoke<void>("start_voice_enrollment"),
  finishVoiceEnrollment: () => invoke<VoiceProfile>("finish_voice_enrollment"),
  cancelVoiceEnrollment: () => invoke<void>("cancel_voice_enrollment"),
  forgetVoiceProfile: () => invoke<void>("forget_voice_profile"),
  renameKnownVoice: (id: string, name: string) => invoke<void>("rename_known_voice", { id, name }),
  forgetKnownVoice: (id: string) => invoke<void>("forget_known_voice", { id }),
  forgetAllVoices: () => invoke<void>("forget_all_voices"),
  exportMeeting: (id: string, includeTranscript: boolean) =>
    invoke<MeetingExport>("export_meeting", { id, includeTranscript }),
  renameMeeting: (id: string, title: string) => invoke<string>("rename_meeting", { id, title }),
  deleteMeeting: (id: string) => invoke<void>("delete_meeting", { id }),
  cloudLoginStart: () => invoke<CloudLoginStart>("cloud_login_start"),
  cloudLoginPoll: (deviceCode: string) => invoke<LoginPoll>("cloud_login_poll", { deviceCode }),
  cloudLogout: () => invoke<void>("cloud_logout"),
  cloudAccount: () => invoke<CloudAccount>("cloud_account"),
  cloudDevices: () => invoke<CloudDevice[]>("cloud_devices"),
  cloudRenameDevice: (id: string, name: string) => invoke<void>("cloud_rename_device", { id, name }),
  cloudRevokeDevice: (id: string) => invoke<void>("cloud_revoke_device", { id }),
  cloudDeleteAccount: () => invoke<void>("cloud_delete_account"),
  cloudSyncEnable: () => invoke<string>("cloud_sync_enable"),
  cloudSyncSetup: () => invoke<CloudSyncSetup>("cloud_sync_setup"),
  cloudSyncPrepare: () => invoke<string | null>("cloud_sync_prepare"),
  cloudRecoveryCodeContext: (code: string) => invoke<string>("cloud_recovery_code_context", { code }),
  cloudRecoveryConfirm: (code: string, context: string) => invoke<void>("cloud_recovery_confirm", { code, context }),
  cloudSyncActivate: () => invoke<void>("cloud_sync_activate"),
  cloudSyncResume: () => invoke<void>("cloud_sync_resume"),
  cloudLinkAvailability: () => invoke<CloudLinkAvailability>("cloud_link_availability"),
  cloudLinkStart: (role: CloudLinkRole) => invoke<CloudLinkView>("cloud_link_start", { role }),
  cloudLinkJoin: (code: string, role: CloudLinkRole) => invoke<CloudLinkView>("cloud_link_join", { code, role }),
  cloudLinkPoll: () => invoke<CloudLinkView>("cloud_link_poll"),
  cloudLinkView: () => invoke<CloudLinkView>("cloud_link_view"),
  cloudLinkApprove: () => invoke<CloudLinkView>("cloud_link_approve"),
  cloudLinkCancel: () => invoke<CloudLinkView>("cloud_link_cancel"),
  cloudSyncUnlock: (code: string) => invoke<void>("cloud_sync_unlock", { code }),
  cloudSyncNewRecoveryCode: () => invoke<string>("cloud_sync_new_recovery_code"),
  cloudSyncReset: () => invoke<string>("cloud_sync_reset"),
  cloudSyncDisable: () => invoke<void>("cloud_sync_disable"),
  cloudSyncNow: () => invoke<SyncReport>("cloud_sync_now"),
  cloudSyncLast: () => invoke<SyncReport>("cloud_sync_last"),
  cloudSyncConfirmDeletes: () => invoke<void>("cloud_sync_confirm_deletes"),
  meetingSyncStates: () => invoke<MeetingSyncState[]>("meeting_sync_states"),
  meetingConflictFiles: (id: string) => invoke<string[]>("meeting_conflict_files", { id }),
  deleteConflictCopy: (id: string, name: string) => invoke<void>("delete_conflict_copy", { id, name }),
  cloudFolderWarning: () => invoke<string | null>("cloud_folder_warning"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  claudeAccess: () => invoke<ClaudeAccessView>("claude_access"),
  setClaudeAccess: (enabled: boolean) => invoke<ClaudeAccessView>("set_claude_access", { enabled }),
  onClaudeAccessChanged: (handler: (status: ClaudeStatus) => void): Promise<() => void> =>
    listen<ClaudeStatus>("claude-access-changed", (event) => handler(event.payload)),
  onMeetingLevel: (handler: (level: MeetingLevel) => void): Promise<() => void> =>
    listen<MeetingLevel>("meeting-level", (event) => handler(event.payload)),
  onSyncChanged: (handler: (report: SyncReport) => void): Promise<() => void> =>
    listen<SyncReport>("sync-changed", (event) => handler(event.payload)),
};
