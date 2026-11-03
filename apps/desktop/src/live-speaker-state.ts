import type { LiveSpeakerAttributionStatus } from "./api";
import type { TranslationKey } from "./i18n";

export class LiveSpeakerControl {
  readonly sessionId: string;
  value: LiveSpeakerAttributionStatus | null;
  error: string | null = null;
  private request: number | null = null;
  private sequence = 0;
  private stopped = false;

  constructor(sessionId: string, initial?: LiveSpeakerAttributionStatus | null) {
    this.sessionId = sessionId;
    this.value = initial ?? null;
  }

  get pending() { return this.request !== null; }
  get checked() { return !this.stopped && this.value?.enabled === true; }

  canChange(recording: boolean): boolean {
    return recording && !this.stopped && !this.pending && !!this.value
      && (this.value.enabled || this.value.available && !["busy", "stopped"].includes(this.value.state));
  }

  observe(sessionId: string, value?: LiveSpeakerAttributionStatus | null): boolean {
    if (this.stopped || sessionId !== this.sessionId || !value || this.value && value.revision < this.value.revision) return false;
    this.value = value;
    return true;
  }

  begin(recording: boolean): number | null {
    if (!this.canChange(recording)) return null;
    this.error = null;
    return this.request = ++this.sequence;
  }

  complete(request: number | null, value: LiveSpeakerAttributionStatus): boolean {
    if (request === null || request !== this.request || this.stopped) return false;
    this.request = null;
    this.observe(this.sessionId, value);
    return true;
  }

  fail(request: number | null, error: string): boolean {
    if (request === null || request !== this.request || this.stopped) return false;
    this.request = null;
    this.error = error;
    return true;
  }

  stop() { this.stopped = true; this.request = null; this.error = null; }
}

export function liveSpeakerStatusKey(status: LiveSpeakerAttributionStatus | null): TranslationKey {
  const reasons: Record<NonNullable<LiveSpeakerAttributionStatus["reason_code"]>, TranslationKey> = {
    local_required: "meetings.live_speakers_local_required", models_missing: "meetings.live_speakers_models_missing",
    unsupported: "meetings.live_speakers_unsupported", configuration_changed: "meetings.live_speakers_configuration_changed",
    foreground_work: "meetings.live_speakers_foreground_work", busy: "meetings.live_speakers_busy", paused: "meetings.live_speakers_paused",
    failed: "meetings.live_speakers_failed", stopped: "meetings.live_speakers_stopped",
  };
  if (status?.reason_code && reasons[status.reason_code]) return reasons[status.reason_code];
  return status && ["off", "warming", "running", "busy", "paused", "unavailable", "failed", "stopped"].includes(status.state)
    ? `meetings.live_speakers_${status.state}` : "meetings.live_speakers_unavailable";
}
