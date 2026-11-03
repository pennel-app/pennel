export class MeetingDraft {
  sessionId: string | null = null;
  value = "";
  dirty = false;
  acknowledgedValue = "";
  acknowledgedRevision = 0;
  error: string | null = null;
  insightReceipt: { text: string; attempt: string } | null = null;
  private pending = 0;
  private writes: Promise<unknown> = Promise.resolve();
  private listeners = new Set<() => void>();

  get status(): "saving" | "saved" | "unsaved" { return this.pending ? "saving" : this.dirty ? "unsaved" : "saved"; }
  subscribe(listener: () => void): () => void { this.listeners.add(listener); return () => this.listeners.delete(listener); }
  private changed() { for (const listener of this.listeners) listener(); }

  sync(sessionId: string | null, value: string): void {
    if (this.sessionId !== sessionId) {
      this.sessionId = sessionId;
      this.value = value;
      this.dirty = false;
      this.acknowledgedValue = value;
      this.acknowledgedRevision = 0;
      this.error = null;
    } else if (!this.dirty && !this.pending) {
      this.value = value;
      this.acknowledgedValue = value;
    }
    this.changed();
  }

  edit(value: string): void {
    if (this.insightReceipt && this.insightReceipt.text !== value) this.insightReceipt = null;
    this.value = value;
    this.dirty = true;
    this.changed();
  }

  stageInsightInsertion(value: string, attempt: string, expected: string): boolean {
    if (this.value !== expected || this.dirty || this.pending || this.sessionId === null) return false;
    this.insightReceipt = { text: value, attempt }; this.edit(value); return true;
  }

  acknowledge(sessionId: string | null, value: string): void {
    if (sessionId !== this.sessionId) return;
    this.acknowledgedValue = value;
    ++this.acknowledgedRevision;
    this.error = null;
    if (value === this.value) this.dirty = false;
    if (this.insightReceipt?.text === value) this.insightReceipt = null;
    this.changed();
  }

  persist(write: (value: string) => Promise<void>, target = this.value): Promise<void> {
    if (!this.dirty && target === this.acknowledgedValue) return this.writes.then(() => undefined);
    const sessionId = this.sessionId;
    ++this.pending;
    this.changed();
    const result = this.writes.then(async () => {
      try {
        await write(target);
        this.acknowledge(sessionId, target);
      } catch (error) {
        if (sessionId === this.sessionId) this.error = error instanceof Error ? error.message : String(error);
        throw error;
      } finally { --this.pending; this.changed(); }
    });
    this.writes = result.catch(() => undefined);
    return result;
  }
}
