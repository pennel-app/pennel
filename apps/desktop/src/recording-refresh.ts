import type { MeetingRuntimeStatus, MeetingStatus, Segment } from "./api";

export function segmentKeys(segments: Segment[]): string[] {
  const occurrences = new Map<string, number>();
  return segments.map((segment) => {
    const base = `${segment.speaker}:${segment.start_ms}`;
    const occurrence = occurrences.get(base) ?? 0;
    occurrences.set(base, occurrence + 1);
    return `${base}:${occurrence}`;
  });
}

export function sameSegment(left: Segment, right: Segment): boolean {
  return left.start_ms === right.start_ms && left.end_ms === right.end_ms && left.speaker === right.speaker
    && left.text === right.text && left.failed === right.failed && left.voice === right.voice;
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
      this.value = { ...this.value, ...runtime, transcript_revision: currentRevision };
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
    this.value = { ...snapshot, ...runtime, transcript: snapshot.transcript, notes: snapshot.notes,
      transcript_revision: snapshot.transcript_revision ?? runtime.transcript_revision };
    return this.value;
  }
}
