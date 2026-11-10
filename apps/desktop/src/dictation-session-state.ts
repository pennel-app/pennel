import type { DictationSessionSnapshot } from "./api";

export class HomePracticeDraft {
  value = "";
  private pending = false;
  private previous = "";
  private completed = false;
  private sessionId = "";
  accept(snapshot: DictationSessionSnapshot): void {
    if (snapshot.session_id !== this.sessionId) {
      this.sessionId = snapshot.session_id;
      this.pending = false; this.completed = false; this.previous = "";
    }
    if (snapshot.phase === "listening") {
      if (!this.pending) this.previous = snapshot.text;
      this.pending = true; this.completed = false;
    } else if (snapshot.phase === "done" && !this.completed || this.pending && snapshot.phase !== "working") {
      this.pending = false; this.completed = true;
      if (snapshot.text.trim() && (snapshot.phase === "done" || snapshot.phase === "error" && snapshot.text !== this.previous)) {
        this.value += `${this.value && !/\s$/.test(this.value) ? " " : ""}${snapshot.text}`;
      }
    }
  }
}

/** The Pad presents its own navigation recovery without leaving the current view. */
export class PadNavigationBlocked extends Error {
  constructor(message = "Finish or cancel dictation before switching sections.") { super(message); this.name = "PadNavigationBlocked"; }
}

export class DictationSessionState {
  snapshot: DictationSessionSnapshot | null = null;
  private closed = true;

  bind(snapshot: DictationSessionSnapshot): void {
    this.closed = false;
    this.snapshot = snapshot;
  }

  accept(snapshot: DictationSessionSnapshot): boolean {
    if (this.closed || !this.snapshot || this.snapshot.session_id !== snapshot.session_id || this.snapshot.kind !== snapshot.kind) return false;
    this.snapshot = snapshot;
    return true;
  }

  close(): string | null {
    const id = this.snapshot?.session_id ?? null;
    this.closed = true;
    this.snapshot = null;
    return id;
  }

  get busy(): boolean {
    return this.snapshot?.phase === "listening" || this.snapshot?.phase === "working";
  }
}

export class PadDraftState {
  value = "";
  loaded = false;
  dirty = false;

  edit(value: string): void { this.value = value; this.dirty = true; }

  acknowledge(value: string): void { if (this.value === value) this.dirty = false; }

  hydrate(value: string): void { if (!this.dirty) this.value = value; this.loaded = true; }

  accept(snapshot: Pick<DictationSessionSnapshot, "phase" | "text">): boolean {
    if (this.dirty || snapshot.phase === "listening" || snapshot.phase === "working") return false;
    this.value = snapshot.text;
    this.loaded = true;
    if (snapshot.phase === "error") this.dirty = true;
    return true;
  }
}

export interface DictationSessionDriver {
  open(kind: DictationSessionSnapshot["kind"]): Promise<DictationSessionSnapshot>;
  get(id: string): Promise<DictationSessionSnapshot>;
  toggle(id: string): Promise<void>;
  cancel(id: string): Promise<void>;
  close(id: string): Promise<void>;
  leave?(id: string): Promise<void>;
  subscribe(listener: (snapshot: DictationSessionSnapshot) => void): Promise<() => void>;
}

export class OwnedSessionLifecycle {
  readonly state = new DictationSessionState();
  private generation = 0;
  private unlisten: (() => void) | null = null;
  private opening: Promise<void> | null = null;
  private eventRevision = 0;
  private readonly kind: DictationSessionSnapshot["kind"];
  private readonly update: (snapshot: DictationSessionSnapshot) => void;
  private readonly driver: DictationSessionDriver;

  constructor(kind: DictationSessionSnapshot["kind"], update: (snapshot: DictationSessionSnapshot) => void, driver: DictationSessionDriver) {
    this.kind = kind; this.update = update; this.driver = driver;
  }

  async open(): Promise<void> {
    if (this.state.snapshot) return;
    if (this.opening) { await this.opening; if (!this.state.snapshot) await this.open(); return; }
    const generation = ++this.generation;
    this.opening = (async () => {
      const pending = new Map<string, DictationSessionSnapshot>();
      const unlisten = await this.driver.subscribe((snapshot) => {
        if (generation !== this.generation) return;
        if (!this.state.snapshot) { pending.set(snapshot.session_id, snapshot); return; }
        if (this.state.accept(snapshot)) { this.eventRevision += 1; this.update(snapshot); }
      });
      if (generation !== this.generation) { unlisten(); return; }
      this.unlisten = unlisten;
      const snapshot = await this.driver.open(this.kind);
      if (generation !== this.generation) { await this.driver.close(snapshot.session_id); return; }
      this.state.bind(snapshot); this.update(snapshot);
      const received = pending.get(snapshot.session_id);
      if (received && this.state.accept(received)) this.update(received);
    })().catch((error: unknown) => { this.unlisten?.(); this.unlisten = null; throw error; }).finally(() => { this.opening = null; });
    return this.opening;
  }

  async toggle(): Promise<void> {
    await this.open();
    const snapshot = this.state.snapshot;
    if (!snapshot || snapshot.phase === "working") return;
    await this.driver.toggle(snapshot.session_id); await this.refresh();
  }

  async refresh(): Promise<void> {
    const snapshot = this.state.snapshot;
    if (!snapshot) return;
    const revision = this.eventRevision;
    const result = await this.driver.get(snapshot.session_id);
    if (revision === this.eventRevision && this.state.accept(result)) this.update(result);
  }

  invalidateRefresh(): void { this.eventRevision += 1; }

  acceptLocalSavedText(text: string): void {
    this.invalidateRefresh();
    const snapshot = this.state.snapshot;
    if (!snapshot || snapshot.kind !== "pad" || this.state.busy) return;
    const next = { ...snapshot, text, phase: snapshot.phase === "error" ? "idle" as const : snapshot.phase, error: snapshot.phase === "error" ? null : snapshot.error };
    if (this.state.accept(next)) this.update(next);
  }

  async cancel(): Promise<void> {
    const snapshot = this.state.snapshot;
    if (snapshot) { await this.driver.cancel(snapshot.session_id); await this.refresh(); }
  }

  async close(): Promise<void> {
    ++this.generation;
    this.unlisten?.(); this.unlisten = null;
    const id = this.state.close();
    if (id) await this.driver.close(id);
  }

  async leave(): Promise<boolean> {
    if (this.opening) await this.opening;
    if (this.state.busy) return false;
    const id = this.state.snapshot?.session_id;
    if (id) {
      try {
        if (this.driver.leave) await this.driver.leave(id);
        else await this.driver.close(id);
      }
      catch (error) {
        await this.refresh().catch(() => undefined);
        if (this.state.busy) return false;
        throw error;
      }
    }
    ++this.generation;
    this.unlisten?.(); this.unlisten = null;
    this.state.close();
    return true;
  }
}
