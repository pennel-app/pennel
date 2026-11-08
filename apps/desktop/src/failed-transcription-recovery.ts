import type { FailedTranscriptionRecoveryPlan } from "./api";

export interface FailedRecoverySnapshot {
  meeting_uid?: string;
  transcript: readonly { failed: boolean }[];
  failed_recovery?: FailedTranscriptionRecoveryPlan;
}

export interface FailedRecoveryControl {
  visible: boolean;
  incomplete: boolean;
  canRetry: boolean;
  failedCount: number | null;
  retryableCount: number | null;
  reasonCode: string | null;
}

const count = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

export function failedRecoveryControl(snapshot: FailedRecoverySnapshot | null, blocked = false, fresh = true): FailedRecoveryControl {
  const plan = snapshot?.failed_recovery;
  const failedRows = snapshot?.transcript.some(segment => segment.failed) ?? false;
  const known = fresh && !!plan && ["complete", "retryable", "unavailable"].includes(plan.status)
    && (plan.failed_count === null || count(plan.failed_count)) && (plan.retryable_count === null || count(plan.retryable_count))
    && !(count(plan.failed_count) && count(plan.retryable_count) && plan.retryable_count > plan.failed_count)
    && !(plan.failed_count === 0 && failedRows)
    && (plan.status !== "complete" || plan.failed_count === 0 && plan.retryable_count === 0)
    && (plan.status !== "retryable" || count(plan.failed_count) && plan.failed_count > 0 && count(plan.retryable_count) && plan.retryable_count > 0)
    && (plan.status !== "unavailable" || plan.retryable_count === null || plan.retryable_count === 0);
  const failedCount = known ? plan.failed_count : null;
  const retryableCount = known ? plan.retryable_count : null;
  const incomplete = failedRows || count(failedCount) && failedCount > 0;
  return {
    visible: incomplete || !!plan && (!known || plan.status !== "complete"), incomplete,
    canRetry: !blocked && known && plan.status === "retryable" && count(failedCount) && failedCount > 0 && count(retryableCount) && retryableCount > 0,
    failedCount, retryableCount, reasonCode: known ? plan.reason_code ?? (plan.status === "unavailable" ? "status_unavailable" : null) : "status_unavailable",
  };
}

interface FailedRecoveryHost<Detail extends FailedRecoverySnapshot> {
  scope(): string | null;
  id(): string;
  meetingUid: string;
  blocked(): boolean;
  flush(): Promise<void>;
  read(id: string): Promise<Detail>;
  retry(id: string): Promise<number>;
  accept(detail: Detail): void;
  changed(): void;
}

export class FailedRecoveryContextError extends Error {
  constructor() { super("meeting_changed"); }
}

export class FailedTranscriptionRecovery<Detail extends FailedRecoverySnapshot> {
  private snapshot: Detail | null = null;
  private fresh = true;
  private readonly host: FailedRecoveryHost<Detail>;
  busy = false;
  error: unknown = null;
  outcome: "complete" | "partial" | "none" | null = null;

  constructor(host: FailedRecoveryHost<Detail>) { this.host = host; }

  get control(): FailedRecoveryControl { return failedRecoveryControl(this.snapshot, this.busy || this.host.blocked(), this.fresh); }

  observe(detail: Detail): void {
    this.snapshot = detail; this.fresh = true;
    if (!this.busy) {
      const current = failedRecoveryControl(detail);
      if (this.outcome === "complete" && current.incomplete) this.outcome = null;
      if (current.failedCount === 0 && !current.incomplete) { this.outcome = null; this.error = null; }
    }
    this.host.changed();
  }

  private owns(scope: string, id?: string): boolean {
    return this.host.scope() === scope && (id === undefined || this.host.id() === id);
  }

  private accept(detail: Detail): boolean {
    if (detail.meeting_uid && detail.meeting_uid !== this.host.meetingUid) {
      this.fresh = false;
      this.error = new FailedRecoveryContextError();
      return false;
    }
    this.snapshot = detail; this.fresh = true;
    this.host.accept(detail);
    return true;
  }

  async retry(): Promise<boolean> {
    const scope = this.host.scope();
    if (!scope || !this.control.canRetry) return false;
    this.busy = true; this.error = null; this.outcome = null; this.host.changed();
    let attempted = false;
    try {
      await this.host.flush();
      if (!this.owns(scope) || this.host.blocked()) return false;
      const id = this.host.id();
      let before: Detail;
      try { before = await this.host.read(id); }
      catch (error) { if (this.owns(scope, id)) { this.fresh = false; this.error = error; } return false; }
      if (!this.owns(scope, id) || !this.accept(before) || !failedRecoveryControl(before, this.host.blocked()).canRetry) return false;
      let recovered: number | null = null;
      attempted = true;
      try { recovered = await this.host.retry(id); }
      catch (error) { if (this.owns(scope, id)) this.error = error; }
      if (!this.owns(scope, id)) return true;
      let after: Detail;
      try { after = await this.host.read(id); }
      catch (error) { if (this.owns(scope, id)) { this.fresh = false; this.error ??= error; } return true; }
      if (!this.owns(scope, id) || !this.accept(after)) return true;
      const previous = failedRecoveryControl(before), current = failedRecoveryControl(after);
      if (!this.error && count(recovered)) this.outcome = current.failedCount === 0 && !current.incomplete ? "complete" : recovered > 0 ? "partial" : "none";
      else if (count(previous.failedCount) && count(current.failedCount) && current.failedCount < previous.failedCount) this.outcome = "partial";
      return true;
    } catch (error) {
      if (this.owns(scope)) this.error = error;
      return attempted;
    } finally { this.busy = false; this.host.changed(); }
  }
}
