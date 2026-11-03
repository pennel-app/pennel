import type { MeetingRuntimeStatus, MeetingStatus, Segment } from "./api";

function currentPreview(previous: MeetingStatus["preview"], incoming: MeetingStatus["preview"], transcriptRevision: number | undefined): MeetingStatus["preview"] {
  if (incoming === null) return null;
  if (!incoming || incoming.transcript_revision !== transcriptRevision) return previous;
  if (previous && (incoming.session_id !== previous.session_id || incoming.revision < previous.revision)) return previous;
  return incoming;
}

export function segmentKeys(segments: Segment[]): string[] {
  const occurrences = new Map<string, number>();
  const identities = new Map<string, number>();
  for (const segment of segments) if (segment.utterance_id) identities.set(segment.utterance_id, (identities.get(segment.utterance_id) ?? 0) + 1);
  return segments.map((segment) => {
    if (segment.utterance_id && identities.get(segment.utterance_id) === 1) return `utterance:${segment.utterance_id}`;
    const base = `${segment.speaker}:${segment.start_ms}`;
    const occurrence = occurrences.get(base) ?? 0;
    occurrences.set(base, occurrence + 1);
    return `${base}:${occurrence}`;
  });
}

export function sameSegment(left: Segment, right: Segment): boolean {
  return left.start_ms === right.start_ms && left.end_ms === right.end_ms && left.speaker === right.speaker
    && left.text === right.text && left.failed === right.failed && left.voice === right.voice
    && left.utterance_id === right.utterance_id && left.source_id === right.source_id
    && left.attribution_revision === right.attribution_revision
    && left.correction_review_required === right.correction_review_required
    && left.confirmed_speaker_name === right.confirmed_speaker_name
    && JSON.stringify(left.attribution ?? null) === JSON.stringify(right.attribution ?? null);
}

/** Keeps lightweight clock/health updates independent of transcript reads. */
export class RecordingRefresh {
  private value: MeetingStatus;
  private generation = 0;
  private disposed = false;
  private loading: Promise<MeetingStatus> | null = null;

  constructor(initial: MeetingStatus) { this.value = initial; }

  dispose() { this.disposed = true; ++this.generation; }

  async refresh(runtime: MeetingRuntimeStatus, load: () => Promise<MeetingStatus>): Promise<MeetingStatus | null> {
    if (this.disposed || runtime.session_id !== this.value.session_id
      || (runtime.meeting_uid && this.value.meeting_uid ? runtime.meeting_uid !== this.value.meeting_uid : runtime.id !== this.value.id)) return null;
    const generation = ++this.generation;
    const currentRevision = this.value.transcript_revision;
    if (currentRevision !== undefined && runtime.transcript_revision <= currentRevision) {
      const preview = runtime.transcript_revision === currentRevision
        ? currentPreview(this.value.preview, runtime.preview, currentRevision) : this.value.preview;
      this.value = { ...this.value, ...runtime, preview, transcript_revision: currentRevision };
      return this.value;
    }
    if (!this.loading) {
      const loading = Promise.resolve().then(load);
      this.loading = loading;
      void loading.finally(() => { if (this.loading === loading) this.loading = null; }).catch(() => undefined);
    }
    const snapshot = await this.loading;
    if (this.disposed || generation !== this.generation || snapshot.session_id !== runtime.session_id
      || (snapshot.meeting_uid && runtime.meeting_uid ? snapshot.meeting_uid !== runtime.meeting_uid : snapshot.id !== runtime.id)) return null;
    const transcriptRevision = snapshot.transcript_revision ?? runtime.transcript_revision;
    const preview = runtime.transcript_revision === transcriptRevision
      ? currentPreview(snapshot.preview, runtime.preview, transcriptRevision) : snapshot.preview;
    this.value = { ...snapshot, ...runtime, transcript: snapshot.transcript, notes: snapshot.notes,
      preview, transcript_revision: transcriptRevision };
    return this.value;
  }
}
