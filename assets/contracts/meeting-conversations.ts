export type ConversationScope = { kind: "meeting"; meeting_uid: string; meeting_id?: string } | { kind: "library" };
export type ConversationMessageStatus = "completed" | "pending" | "cancelled" | "interrupted" | "failed";
export type ConversationSourceStatus = "current" | "stale" | "unavailable";

export interface ConversationPassage {
  meeting_id: string;
  meeting_uid?: string | null;
  title: string;
  start_iso: string;
  kind: "notes" | "transcript" | "summary";
  start_ms: number | null;
  text: string;
  snippet: string;
  utterance_ids?: string[];
  utterance_fingerprints?: string[];
}
export interface ConversationSourceAnchor {
  passage: ConversationPassage;
  segment_keys: string[];
  segment_fingerprints: string[];
  notes_start: number | null;
  notes_end: number | null;
  source_text: string;
}
export interface ConversationAnswer {
  text: string;
  citations: { number: number; passage: ConversationPassage }[];
}
export interface ConversationSelection {
  scope: "whole_meeting" | "selected_excerpts";
  available_sources: number;
  included_sources: number;
  available_history_turns: number;
  included_history_turns: number;
  history_reduced: boolean;
}
export interface ConversationStoredSource {
  number: number;
  passage: ConversationPassage;
  anchor: ConversationSourceAnchor | null;
  status: ConversationSourceStatus;
  included_through_ms: number;
  revision: string;
}
export interface ConversationEvidence {
  request_id: string;
  meeting_uid: string | null;
  session_id: string | null;
  transcript_revision: number;
  notes_revision: string;
  included_through_ms: number;
  pending_transcription: number;
  anchors: ConversationSourceAnchor[];
  sources: ConversationStoredSource[];
  selection: ConversationSelection;
}
export interface ConversationSummary {
  id: string;
  title: string;
  draft: string;
  draft_revision: number;
  created_at_ms: number;
  updated_at_ms: number;
  active_request_id: string | null;
}
export interface ConversationMessage {
  id: string;
  conversation_id: string;
  request_id: string;
  role: "user" | "assistant";
  text: string;
  status: ConversationMessageStatus;
  created_at_ms: number;
  reply_to: string | null;
  answer: ConversationAnswer | null;
  evidence: ConversationEvidence | null;
  error: string | null;
}
export interface ConversationSnapshot {
  schema_version: 1;
  scope: ConversationScope;
  storage_status: "saved" | "unsaved";
  storage_error: string | null;
  selected_conversation_id: string;
  current_meeting_id: string | null;
  conversations: ConversationSummary[];
  conversation: ConversationSummary;
  messages: ConversationMessage[];
  next_messages_cursor: string | null;
  next_conversations_cursor: string | null;
}
export interface ConversationSnapshotRequest {
  scope: ConversationScope;
  conversation_id?: string;
  messages_before?: string;
  conversations_before?: string;
  focus_message_id?: string;
}
export type ConversationUpdateRequest = { scope: ConversationScope } & (
  | { action: "create"; title?: string }
  | { action: "select"; conversation_id: string }
  | { action: "rename"; conversation_id: string; title: string }
  | { action: "delete"; conversation_id: string }
  | { action: "draft"; conversation_id: string; draft: string; expected_draft_revision: number }
  | { action: "retry_save"; conversation_id?: string }
);
export interface ConversationBeginRequest {
  scope: ConversationScope;
  conversation_id: string;
  request_id: string;
  question: string;
  expected_notes_revision?: string;
  session_id?: string;
}
export interface ConversationCancelRequest {
  scope: ConversationScope;
  conversation_id: string;
  request_id: string;
}
export interface ConversationSourceRequest {
  scope: ConversationScope;
  conversation_id: string;
  message_id: string;
  citation_number: number;
}
export interface ConversationResolvedSource {
  status: ConversationSourceStatus;
  passage: ConversationPassage;
  anchor: ConversationSourceAnchor | null;
  current_meeting_id: string | null;
}

export const conversationRequests = {
  snapshot: (scope: ConversationScope, conversation_id?: string, cursors: { messages_before?: string; conversations_before?: string; focus_message_id?: string } = {}): ConversationSnapshotRequest =>
    ({ scope, ...conversation_id ? { conversation_id } : {}, ...cursors }),
  create: (scope: ConversationScope, title?: string): ConversationUpdateRequest => ({ scope, action: "create", ...title !== undefined ? { title } : {} }),
  select: (scope: ConversationScope, conversation_id: string): ConversationUpdateRequest => ({ scope, action: "select", conversation_id }),
  rename: (scope: ConversationScope, conversation_id: string, title: string): ConversationUpdateRequest => ({ scope, action: "rename", conversation_id, title }),
  delete: (scope: ConversationScope, conversation_id: string): ConversationUpdateRequest => ({ scope, action: "delete", conversation_id }),
  draft: (scope: ConversationScope, conversation_id: string, draft: string, expected_draft_revision: number): ConversationUpdateRequest => ({ scope, action: "draft", conversation_id, draft, expected_draft_revision }),
  retry: (scope: ConversationScope, conversation_id?: string): ConversationUpdateRequest => ({ scope, action: "retry_save", ...conversation_id ? { conversation_id } : {} }),
  begin: (scope: ConversationScope, conversation_id: string, request_id: string, question: string, evidence: { expected_notes_revision?: string; session_id?: string } = {}): ConversationBeginRequest =>
    ({ scope, conversation_id, request_id, question, ...evidence }),
  cancel: (scope: ConversationScope, conversation_id: string, request_id: string): ConversationCancelRequest => ({ scope, conversation_id, request_id }),
  source: (scope: ConversationScope, conversation_id: string, message_id: string, citation_number: number): ConversationSourceRequest => ({ scope, conversation_id, message_id, citation_number }),
};

export function conversationScopeKey(scope: ConversationScope): string {
  return scope.kind === "library" ? "library" : `meeting:${scope.meeting_uid}`;
}

export type ConversationOperation = "snapshot" | "update" | "begin" | "cancel" | "source";
/** The fixture and clients share wire requirements; Rust still owns validation and permissions. */
export function assertConversationRequest(operation: ConversationOperation, value: unknown): void {
  const invalid = () => { throw new Error("The conversation request is invalid."); };
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const request = value as Record<string, unknown>;
  const scope = request.scope as Record<string, unknown> | undefined;
  if (!scope || typeof scope !== "object" || (scope.kind !== "library" && scope.kind !== "meeting")) return invalid();
  if (scope.kind === "meeting" && (typeof scope.meeting_uid !== "string" || !scope.meeting_uid)) return invalid();
  if (scope.meeting_id !== undefined && typeof scope.meeting_id !== "string") return invalid();
  const string = (key: string) => { if (typeof request[key] !== "string" || !request[key]) invalid(); };
  const optionalString = (key: string) => { if (request[key] !== undefined) string(key); };
  if (operation === "snapshot") { optionalString("conversation_id"); optionalString("messages_before"); optionalString("conversations_before"); optionalString("focus_message_id"); return; }
  if (operation === "update") {
    const action = request.action;
    if (action === "create") { if (request.title !== undefined && typeof request.title !== "string") invalid(); return; }
    if (action === "retry_save") { optionalString("conversation_id"); return; }
    if (action !== "select" && action !== "rename" && action !== "delete" && action !== "draft") return invalid();
    string("conversation_id");
    if (action === "rename" && typeof request.title !== "string") invalid();
    if (action === "draft" && (typeof request.draft !== "string" || !Number.isSafeInteger(request.expected_draft_revision) || (request.expected_draft_revision as number) < 0)) invalid();
    return;
  }
  string("conversation_id");
  if (operation === "source") { string("message_id"); if (!Number.isSafeInteger(request.citation_number) || (request.citation_number as number) < 1) invalid(); return; }
  string("request_id");
  if (operation === "begin") { string("question"); optionalString("expected_notes_revision"); optionalString("session_id"); }
}
