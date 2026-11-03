import type { MeetingDetail, Passage } from './domain.ts';

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
