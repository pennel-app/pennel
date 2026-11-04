import type { RuntimeSnapshot } from './domain.ts';

type MeetingSnapshot = RuntimeSnapshot['meeting'];
export interface RecordingStopIntent { id: string; previousFinishedId: string | null }

export function completedRecording(meeting: MeetingSnapshot | undefined, intent: RecordingStopIntent | null): string | null {
  const finished = meeting?.last_finished_id;
  if (!intent || !finished || meeting?.phase !== 'idle' || meeting.retained_id || meeting.error) return null;
  if (meeting.id && meeting.id !== intent.id) return null;
  return finished !== intent.previousFinishedId || finished === intent.id ? finished : null;
}

export function nativeOwnsRecordingDraft(meeting: MeetingSnapshot | undefined, id: string | null | undefined, text: string, handedOff?: string): boolean {
  if (!id || !meeting || meeting.id !== id && meeting.retained_id !== id || meeting.phase !== 'finishing' && !meeting.retained_id) return false;
  return meeting.pending_notes === text || handedOff === text || meeting.notes === text;
}

export class RecordingIdentity {
  private meeting: MeetingSnapshot | null = null;
  private originalId: string | null = null;
  get key(): string | null { return this.originalId; }
  observe(current: MeetingSnapshot | undefined): MeetingSnapshot | null {
    if (current?.retained_id && !current.id) current = { ...current, id: current.retained_id };
    if (!this.meeting && current?.id) { this.meeting = current; this.originalId = current.id; }
    else if (current?.id && current.id === this.meeting?.id) this.meeting = current;
    if (!this.meeting) return null;
    if (current?.id !== this.meeting.id) {
      if (current?.last_finished_id === this.meeting.id || current?.retained_id === this.meeting.id) this.meeting = { ...this.meeting, error: current.error, error_code: current.error_code };
      return { ...this.meeting, phase: 'idle', pending: 0 };
    }
    return this.meeting;
  }
  retarget(originalId: string, finishedId: string): boolean {
    if (!finishedId || this.meeting?.id !== originalId) return false;
    this.meeting = { ...this.meeting, id: finishedId, phase: 'idle' };
    return true;
  }
}

export async function stopThenSave(stop: () => Promise<boolean>, save: () => Promise<boolean>): Promise<{ stopped: boolean; saved: boolean }> {
  let stopped = false;
  let saved = false;
  try { stopped = await stop(); } catch { stopped = false; }
  try { saved = await save(); } catch { saved = false; }
  return { stopped, saved };
}
