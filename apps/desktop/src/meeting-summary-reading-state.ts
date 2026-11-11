import type { MeetingSummaryDocument, MeetingSummaryRecord, SpeakerTimelineLane, SummaryAction } from "./api";

export type SummaryLength = "brief" | "standard" | "detailed";
export const SUMMARY_LENGTH_KEY = "pennel.summary.length.v1";
export function summaryLength(value: unknown): SummaryLength {
  return value === "brief" || value === "detailed" ? value : "standard";
}
export function summarySections(length: SummaryLength) {
  return { map: length === "detailed", topics: length !== "brief", open: length !== "brief" };
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const nullableText = (value: unknown) => value === null || text(value);
const time = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
function source(value: unknown): boolean {
  return value === null || object(value) && text(value.utterance_id) && !!value.utterance_id.trim() && time(value.start_ms) && nullableText(value.speaker);
}
function structure(value: unknown): boolean {
  if (value === null) return true;
  if (!object(value) || !text(value.lead) || !value.lead.trim() || !object(value.labels)
    || !["summary", "decisions", "actions", "open_questions", "none"].every(key => text(value.labels && (value.labels as Record<string, unknown>)[key]))) return false;
  const items = (items: unknown, topic = false, action = false): boolean => Array.isArray(items) && items.length <= (topic ? 12 : 40) && items.every(item => object(item)
    && (topic ? text(item.title) && text(item.body) : text(item.text)) && source(item.source)
    && (!action || nullableText(item.owner) && nullableText(item.due) && object(item.state) && typeof item.state.done === "boolean" && nullableText(item.state.owner)));
  return items(value.topics, true) && items(value.decisions) && items(value.actions, false, true) && items(value.open_questions);
}

/** Optional display data must describe this exact canonical body; otherwise keep its Markdown. */
export async function attachedSummaryRecord(document: Pick<MeetingSummaryDocument, "meeting_uid" | "markdown" | "record">): Promise<MeetingSummaryRecord | null> {
  const record: unknown = document.record;
  if (!object(record) || record.schema_version !== 1 || record.meeting_uid !== document.meeting_uid || !text(record.body_revision)
    || !/^[a-f0-9]{64}$/.test(record.body_revision) || !object(record.provenance) || !structure(record.structure)) return null;
  const provenance = record.provenance;
  if (!["provider_id", "provider_kind", "model", "generated_at", "template_id"].every(key => text(provenance[key]))
    || !Number.isFinite(Date.parse(provenance.generated_at as string))) return null;
  try {
    const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(document.markdown));
    const digest = [...new Uint8Array(bytes)].map(byte => byte.toString(16).padStart(2, "0")).join("");
    return digest === record.body_revision ? record as unknown as MeetingSummaryRecord : null;
  } catch { return null; }
}

export function actionState(action: SummaryAction, change: { done?: boolean; owner?: string | null }): { done: boolean; owner: string | null } {
  return { done: change.done ?? action.state.done, owner: change.owner === undefined ? action.state.owner : change.owner };
}
export function laneSpans(lane: SpeakerTimelineLane, duration: number): { left: number; width: number }[] {
  if (!time(duration) || !duration) return [];
  return lane.spans.filter(span => time(span.start_ms) && time(span.end_ms) && span.end_ms > span.start_ms && span.end_ms <= duration)
    .map(span => ({ left: span.start_ms / duration * 100, width: (span.end_ms - span.start_ms) / duration * 100 }));
}
