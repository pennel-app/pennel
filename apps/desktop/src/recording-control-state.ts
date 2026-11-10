/** The toolbar, keyboard command and retry form share one admission gate. */
export class RecordingActionGate {
  pending = false;
  async run(work: () => Promise<void>): Promise<void> {
    if (this.pending) return;
    this.pending = true;
    try { await work(); } finally { this.pending = false; }
  }
}

/** A stopped session stays closed even if a runtime poll was already in flight. */
export class RecordingLevels {
  level = 0;
  private session: string | null = null;
  private readonly retired = new Set<string>();
  private readonly listeners = new Set<(level: number) => void>();
  subscribe(listener: (level: number) => void): () => void {
    this.listeners.add(listener); return () => { this.listeners.delete(listener); };
  }
  private set(level: number): void { this.level = level; this.listeners.forEach(listener => listener(level)); }
  observe(status: { session_id: string | null; recording: boolean; phase: string }): boolean {
    if (status.session_id && this.retired.has(status.session_id) && (this.session !== null || status.recording)) return false;
    const session = status.recording && status.phase === "recording" ? status.session_id : null;
    if (session !== this.session) {
      if (this.session) this.retired.add(this.session);
      this.session = session; this.set(0);
    }
    return true;
  }
  stop(session: string): void {
    this.retired.add(session);
    if (session !== this.session) return;
    this.session = null; this.set(0);
  }
  accept(event: { session_id: string; level: number }): boolean {
    if (!this.session || event.session_id !== this.session || !Number.isFinite(event.level)) return false;
    this.set(Math.max(0, Math.min(1, event.level))); return true;
  }
}

/** Coalesce toolbar/local Stop while allowing an explicit retry after an invoke error. */
export class RecordingStopGate {
  private completed: string | null = null;
  private readonly pending = new Map<string, Promise<void>>();
  run(session: string, work: () => Promise<void>): Promise<void> {
    if (this.completed === session) return Promise.resolve();
    const existing = this.pending.get(session); if (existing) return existing;
    const promise = work().then(() => { this.completed = session; }).finally(() => { this.pending.delete(session); });
    this.pending.set(session, promise); return promise;
  }
}

export const meetingLevels = new RecordingLevels();
