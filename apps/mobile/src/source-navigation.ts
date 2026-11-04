import type { MeetingDetail, Passage } from './domain.ts';

export interface ConversationNavigationOrigin {
  conversation_id: string;
  message_id: string;
  citation_number: number;
  scope: 'meeting' | 'library';
  meeting_uid: string | null;
}

export function conversationSourceRoute(id: string, origin: ConversationNavigationOrigin) {
  return { pathname: '/meeting/[id]' as const, params: { id, conversation_id: origin.conversation_id,
    message_id: origin.message_id, citation_number: String(origin.citation_number), origin_scope: origin.scope,
    ...(origin.meeting_uid ? { origin_meeting_uid: origin.meeting_uid } : {}) } };
}

export function conversationOrigin(params: Record<string, unknown>): ConversationNavigationOrigin | null {
  const identity = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && value.length <= 256 && !/[\u0000-\u001f]/.test(value);
  const scope = params.origin_scope;
  const citation = typeof params.citation_number === 'string' && /^[1-9]\d*$/.test(params.citation_number) ? Number(params.citation_number) : NaN;
  if (!identity(params.conversation_id) || !identity(params.message_id) || !Number.isSafeInteger(citation)
    || scope !== 'meeting' && scope !== 'library' || scope === 'meeting' && !identity(params.origin_meeting_uid)) return null;
  return { conversation_id: params.conversation_id, message_id: params.message_id, citation_number: citation,
    scope, meeting_uid: scope === 'meeting' ? String(params.origin_meeting_uid) : null };
}

export function conversationReturnRoute(origin: ConversationNavigationOrigin, currentMeetingId?: string) {
  const params = { conversation_id: origin.conversation_id, message_id: origin.message_id };
  if (origin.scope === 'library') return { pathname: '/(tabs)/ask' as const, params };
  if (!currentMeetingId) return null;
  return { pathname: '/meeting/[id]' as const, params: { ...params, id: currentMeetingId, pane: 'questions' } };
}

let sequence = 0;
const sources = new Map<string, { passage: Passage; created: number }>();
export function rememberSource(passage: Passage, now = Date.now()): string {
  const token = `source-${++sequence}`;
  sources.set(token, { passage: { ...passage,
    ...(passage.utterance_ids ? { utterance_ids: [...passage.utterance_ids] } : {}),
    ...(passage.utterance_fingerprints ? { utterance_fingerprints: [...passage.utterance_fingerprints] } : {}) }, created: now });
  while (sources.size > 32) sources.delete(sources.keys().next().value as string);
  return token;
}

function normalized(text: string): string { return text.replace(/\s+/g, ' ').trim(); }
function durableTranscriptSource(detail: MeetingDetail, passage: Passage): number | null {
  const ids = passage.utterance_ids;
  const fingerprints = passage.utterance_fingerprints;
  if (!ids?.length || !fingerprints || fingerprints.length !== ids.length || new Set(ids).size !== ids.length
    || detail.metadata_status && detail.metadata_status !== 'valid') return null;
  let first: number | null = null;
  let previous = -1;
  for (const [offset, id] of ids.entries()) {
    const matches = detail.transcript.flatMap((segment, index) => segment.utterance_id === id ? [index] : []);
    if (matches.length !== 1 || matches[0] <= previous) return null;
    const segment = detail.transcript[matches[0]];
    if (segment.failed || !segment.source_fingerprint || segment.source_fingerprint !== fingerprints[offset]) return null;
    first ??= matches[0]; previous = matches[0];
  }
  return first;
}
function transcriptSource(detail: MeetingDetail, passage: Passage, cited: string): number | null {
  if (passage.utterance_ids != null) return durableTranscriptSource(detail, passage);
  const segments = detail.transcript.map((segment, index) => ({ segment, index })).filter(({ segment }) => !segment.failed && segment.text.trim());
  const candidates = segments.flatMap(({ segment }, index) => passage.start_ms == null || segment.start_ms === passage.start_ms ? [index] : []);
  const names = new Map((detail.speakers ?? []).map((speaker) => [speaker.voice, speaker.name]));
  const lines = segments.map(({ segment }) => {
    // Match the core's indexed labels, which are independent of interface language.
    const label = segment.speaker === 'me' ? 'Ik' : segment.voice == null ? 'Anderen' : names.get(segment.voice) ?? `Anderen ${segment.voice}`;
    return `${label}: ${segment.text.trim()}`;
  });
  const found = candidates.find((first) => {
    const offset = normalized(lines.slice(first).join('\n')).indexOf(cited);
    return offset >= 0 && offset < normalized(lines[first]).length;
  });
  return found === undefined ? null : segments[found].index;
}
export function resolveSource(token: string, detail: MeetingDetail, now = Date.now()): { changed: boolean; passage: Passage | null; transcriptIndex: number | null } {
  const source = sources.get(token);
  if (!source || now - source.created > 30 * 60 * 1000 || (source.passage.meeting_uid
    ? source.passage.meeting_uid !== detail.meeting_uid : source.passage.meeting_id !== detail.info.id)) return { changed: true, passage: null, transcriptIndex: null };
  const cited = normalized(source.passage.text);
  const transcriptIndex = source.passage.kind === 'transcript' && cited ? transcriptSource(detail, source.passage, cited) : null;
  const valid = cited && (source.passage.kind === 'transcript' ? transcriptIndex !== null : normalized(source.passage.kind === 'notes' ? detail.notes : detail.summary ?? '').includes(cited));
  return { changed: !valid, passage: source.passage, transcriptIndex };
}
