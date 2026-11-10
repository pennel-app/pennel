export type InsightKind = "action" | "decision" | "open_question" | "changed_commitment";
export type InsightReviewState = "proposed" | "accepted" | "dismissed";
export type InsightLanguagePolicy = "unknown" | "en" | "nl" | "mixed";
export interface InsightSource {
  meeting_uid: string; utterance_id: string; source_fingerprint: string; quote: string;
  start_byte: number; end_byte: number; start_ms: number; end_ms: number;
}
export interface SupportedInsightDetail { text: string; source_index: number; start_byte: number; end_byte: number }
export interface InsightIdentity {
  protocol: string; endpoint: string; requested_model: string; requested_upstream: string | null;
  actual_model: string | null; actual_provider: string | null; actual_revision: string | null; request_id: string | null;
}
export interface InsightItem {
  id: string; dedup_key: string; revision: number; kind: InsightKind;
  commitment: "tentative" | "confirmed" | "retracted" | "unresolved" | "changed" | "unknown";
  text: string; owner: SupportedInsightDetail | null; due: SupportedInsightDetail | null; sources: InsightSource[];
  evidence_cutoff_ms: number; transcript_revision: number; transcript_digest: string;
  processing: { identity: InsightIdentity; qualification_id: string; language_policy: string; task: string };
  related_item_ids: string[]; review_state: InsightReviewState; edited_text: string | null; edit_claim: string | null;
  source_state: "current" | "stale" | "missing" | "unavailable"; source_history: InsightSource[]; source_reassociated: boolean;
  extraction_scope?: "passage" | "exact_span";
  prior_evidence?: { kind: InsightKind; source: InsightSource; target_item_id?: string | null }[];
}
export interface InsightInsertionReceipt {
  attempt_id: string; item_id: string; item_revision: number; before_notes_hash: string; intended_notes_hash: string;
  markdown: string; state: "pending" | "acknowledged" | "not_inserted" | "needs_review";
  manual_resolution?: { state: "acknowledged" | "not_inserted"; notes_hash: string } | null;
}
export interface AutomaticInsightAttempt {
  schema_version: 1; meeting_uid: string; transcript_digest: string;
  language_policy: Exclude<InsightLanguagePolicy, "unknown">; admitted_at: string;
  outcome?: "completed" | "failed" | "cancelled"; error_code?: string;
  items_found?: number; finished_at?: string;
}
export interface InsightSnapshot {
  meeting_uid: string; revision: number; status: "missing" | "ready" | "unsupported" | "corrupt" | "too_large" | "wrong_meeting";
  items: InsightItem[]; insertion_receipts: InsightInsertionReceipt[];
  analysis_job?: InsightJob | null;
  automatic_attempt?: AutomaticInsightAttempt | null;
  resolved_language?: Exclude<InsightLanguagePolicy, "unknown"> | null;
  automatic_attempt_stale?: boolean;
  automatic_attempt_reason_code?: "attempt_unavailable" | "source_unavailable" | null;
}
export interface InsightAvailability {
  available: boolean; configured: boolean; qualified: boolean; reason_code: string;
  provider_id: string | null; model: string | null; endpoint: string | null; language_policy: string;
  task: string; qualification_id: string | null; use_case: string;
  resolved_language?: Exclude<InsightLanguagePolicy, "unknown"> | null;
  extraction_scope?: "passage" | "exact_span"; complete_extraction?: boolean; coverage_notice?: string;
}
export interface InsightJob {
  job_id: string; meeting_uid: string; state: "queued" | "running" | "completed" | "cancelled" | "failed";
  revision: number; error_code: string | null; physical_running: boolean; included_through_ms: number; completed_windows: number; total_windows: number;
}
export interface InsightInsertionPreparation { snapshot: InsightSnapshot; receipt: InsightInsertionReceipt; stage: boolean; intended_notes: string | null }
export interface InsightReviewUpdate { review_state?: InsightReviewState | null; edited_text?: string | null; clear_edit?: boolean; edit_claim?: string | null; release_edit_claim?: boolean }
export interface InsightReviewRequest { store_revision: number; item_id: string; item_revision: number; update: InsightReviewUpdate }
export interface InsightClaimRequest { store_revision: number; item_id: string; item_revision: number; claim_id?: string | null }
export interface InsightInsertionRequest {
  store_revision: number; item_id: string; item_revision: number; attempt_id?: string | null;
  expected_notes: string; intended_notes: string; markdown: string;
}
export interface InsightInsertionResolution { store_revision: number; attempt_id: string; decision: "acknowledged" | "not_inserted"; expected_notes: string }
export interface InsightReassociationRequest {
  store_revision: number; item_id: string; item_revision: number; source_index: number; source: InsightSource;
  evidence_cutoff_ms: number; edit_claim?: string | null;
}
export interface InsightTranscriptRow {
  utterance_id?: string | null; source_fingerprint?: string | null; text: string; start_ms: number; end_ms: number; failed: boolean;
}
export interface InsightNoteLatch {
  text: string; revision: number; selection_start: number; selection_end: number; selection_revision: number; composing: boolean;
  interaction_revision?: number;
}

export function insightText(item: InsightItem): string { return item.edited_text ?? item.text; }
export function insightsWritable(snapshot: InsightSnapshot | null): boolean { return snapshot?.status === "ready" || snapshot?.status === "missing"; }
export function insightReceipt(snapshot: InsightSnapshot | null, itemId: string): InsightInsertionReceipt | undefined {
  const receipts = snapshot?.insertion_receipts ?? [];
  const blocking = receipts.find(receipt => receipt.item_id === itemId && receipt.state !== "not_inserted");
  if (blocking) return blocking;
  for (let index = receipts.length - 1; index >= 0; index--) if (receipts[index].item_id === itemId) return receipts[index];
  return undefined;
}
export function appendInsightMarkdown(notes: string, markdown: string): string {
  const separator = !notes.length || notes.endsWith("\n\n") ? "" : notes.endsWith("\n") ? "\n" : "\n\n";
  return notes + separator + markdown + (markdown.endsWith("\n") ? "" : "\n");
}
export function insertionStillCurrent(before: InsightNoteLatch, current: InsightNoteLatch): boolean {
  return !before.composing && !current.composing && before.text === current.text && before.revision === current.revision
    && before.selection_start === current.selection_start && before.selection_end === current.selection_end && before.selection_revision === current.selection_revision
    && before.interaction_revision === current.interaction_revision;
}
export function utf8Length(text: string): number {
  let bytes = 0;
  for (const character of text) { const point = character.codePointAt(0)!; bytes += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4; }
  return bytes;
}
export function utf8Range(text: string, start: number, end: number): { start: number; end: number } | null {
  if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end < start) return null;
  let bytes = 0, offset = 0, first: number | null = start === 0 ? 0 : null;
  for (const character of text) {
    bytes += utf8Length(character); offset += character.length;
    if (bytes === start) first = offset;
    if (bytes === end) return first === null ? null : { start: first, end: offset };
    if (bytes > end) return null;
  }
  return end === 0 && first === 0 ? { start: 0, end: 0 } : null;
}
export function insightSourceMatches(source: InsightSource, row: InsightTranscriptRow | undefined, meetingUid: string): boolean {
  if (!row || row.failed || source.meeting_uid !== meetingUid || row.utterance_id !== source.utterance_id
    || row.source_fingerprint !== source.source_fingerprint || row.start_ms !== source.start_ms || row.end_ms !== source.end_ms) return false;
  const range = utf8Range(row.text, source.start_byte, source.end_byte);
  return !!range && row.text.slice(range.start, range.end) === source.quote;
}
export function sourceCandidates(source: InsightSource, rows: InsightTranscriptRow[], meetingUid: string): InsightSource[] {
  const candidates: InsightSource[] = [];
  for (const row of rows) {
    if (!row.utterance_id || !row.source_fingerprint || row.failed) continue;
    for (let index = row.text.indexOf(source.quote); index >= 0; index = row.text.indexOf(source.quote, index + 1)) {
      const start = utf8Length(row.text.slice(0, index));
      candidates.push({ meeting_uid: meetingUid, utterance_id: row.utterance_id, source_fingerprint: row.source_fingerprint,
        quote: source.quote, start_byte: start, end_byte: start + utf8Length(source.quote), start_ms: row.start_ms, end_ms: row.end_ms });
      if (candidates.length >= 100) return candidates;
    }
  }
  return candidates;
}
export function safeInsightDestination(endpoint: string | null | undefined): { host: string | null; on_device: boolean } {
  try {
    if (!endpoint) return { host: null, on_device: false };
    const url = new URL(endpoint);
    if (!["http:", "https:"].includes(url.protocol)) return { host: null, on_device: false };
    const host = url.hostname.replace(/^\[|\]$/g, "").toLowerCase();
    const octets = host.split(".");
    const local = host === "localhost" || host === "::1" || octets.length === 4 && octets[0] === "127" && octets.every(part => /^\d{1,3}$/.test(part) && Number(part) <= 255);
    return { host: url.host, on_device: local };
  } catch { return { host: null, on_device: false }; }
}

export class InsightDraft {
  readonly item_id: string;
  claim_id: string | null;
  text: string;
  baseline: string;
  item_revision: number;
  local_revision = 0;
  composing = false;
  conflict = false;
  constructor(itemId: string, claimId: string | null, text: string, revision: number) {
    this.item_id = itemId; this.claim_id = claimId; this.text = text; this.baseline = text; this.item_revision = revision;
  }
  get dirty() { return this.text !== this.baseline; }
  edit(text: string) { this.text = text; ++this.local_revision; }
  acknowledge(text: string, revision: number, savedLocalRevision: number, released: boolean) {
    this.baseline = text; this.item_revision = revision; this.conflict = false;
    if (released) this.claim_id = null;
    return this.local_revision === savedLocalRevision && this.text === text;
  }
}
