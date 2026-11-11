export type PostprocessStage = "summary" | "suggestions" | "postprocess";
export type PostprocessPhase = "started" | "finished" | "failed" | "skipped" | "cancelled";
export interface MeetingPostprocessEvent { id: string; meeting_uid: string; operation_id: string; stage: PostprocessStage; state: PostprocessPhase }

/** Presentation progress never grants consent or starts work. */
export class MeetingPostprocessState {
  private readonly current = new Map<string, MeetingPostprocessEvent>();
  private readonly listeners = new Set<() => void>();
  subscribe(listener: () => void): () => void { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  private changed() { this.listeners.forEach(listener => listener()); }
  accept(value: unknown): boolean {
    if (!value || typeof value !== "object") return false;
    const event = value as Partial<MeetingPostprocessEvent>;
    if (![event.id, event.meeting_uid, event.operation_id].every(item => typeof item === "string" && item.length > 0)
      || !["summary", "suggestions", "postprocess"].includes(event.stage ?? "") || !["started", "finished", "failed", "skipped", "cancelled"].includes(event.state ?? "")) return false;
    const accepted = event as MeetingPostprocessEvent, key = `${accepted.meeting_uid}:${accepted.stage}`, previous = this.current.get(key);
    if (accepted.state !== "started" && previous?.operation_id !== accepted.operation_id) return false;
    if (previous?.operation_id === accepted.operation_id && previous.state !== "started") return false;
    this.current.set(key, { id: accepted.id, meeting_uid: accepted.meeting_uid, operation_id: accepted.operation_id, stage: accepted.stage, state: accepted.state });
    this.changed(); return true;
  }
  get(uid: string, stage: PostprocessStage): MeetingPostprocessEvent | null { return this.current.get(`${uid}:${stage}`) ?? null; }
  clearTerminal(uid: string, stage: PostprocessStage): void {
    const key = `${uid}:${stage}`;
    if (this.current.get(key)?.state !== "started" && this.current.delete(key)) this.changed();
  }
}
export const meetingPostprocess = new MeetingPostprocessState();
