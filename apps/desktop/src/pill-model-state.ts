import type { ModelChoice } from "./api";

export interface ConfiguredDictationModel { choice: ModelChoice; label: string }
export interface PillModelPresentation {
  session_id: string;
  generation: number;
  model: ConfiguredDictationModel;
  available_models: number;
  enabled: boolean;
  switched: boolean;
}

/** Owns model metadata and its brief acknowledgement independently of recognition text. */
export class PillModelState {
  private session = "";
  private current: PillModelPresentation | null = null;
  private confirmedUntil = 0;

  begin(session: string) {
    if (session === this.session) return;
    this.clear(); this.session = session;
  }

  clear() { this.session = ""; this.current = null; this.confirmedUntil = 0; }
  clearConfirmation() { this.confirmedUntil = 0; }

  receive(next: PillModelPresentation, now: number): boolean {
    if (!this.session || next.session_id !== this.session || !Number.isSafeInteger(next.generation) || next.generation < 0
      || next.generation < (this.current?.generation ?? 0) || !next.model.choice.provider_id || !next.model.choice.model || !next.model.label) return false;
    const changed = next.generation !== this.current?.generation;
    this.current = next;
    if (changed && next.switched) this.confirmedUntil = now + 900;
    return true;
  }

  get chip(): ConfiguredDictationModel | null {
    return this.current?.enabled && this.current.available_models > 1 ? this.current.model : null;
  }

  confirmationRemaining(now: number): number { return Math.max(0, this.confirmedUntil - now); }

  confirmation(now: number): string | null {
    return now < this.confirmedUntil ? this.current?.model.label ?? null : null;
  }
}
