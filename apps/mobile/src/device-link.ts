import type { CloudLinkAvailability, CloudLinkView, InterfaceLanguage, NativeFailure } from '../modules/pennel-mobile/src/types.ts';
import { localizedFailure, nativeFailure } from './errors.ts';

interface AvailabilityState { value: CloudLinkAvailability | null; error: NativeFailure | null; checking: boolean }

export class LinkAvailabilityProbe {
  private state: AvailabilityState = { value: null, error: null, checking: false };
  private revision = 0;
  private listeners = new Set<() => void>();
  private readonly read: () => Promise<CloudLinkAvailability>;

  constructor(read: () => Promise<CloudLinkAvailability>) { this.read = read; }
  getSnapshot = (): AvailabilityState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(state: AvailabilityState) { this.state = state; for (const listener of this.listeners) listener(); }
  cancel = (): void => { this.revision += 1; };
  reset = (): void => { this.cancel(); this.update({ value: null, error: null, checking: false }); };

  refresh = async (): Promise<void> => {
    const revision = ++this.revision;
    this.update({ value: null, error: null, checking: true });
    try {
      const value = await this.read();
      if (revision === this.revision) this.update({ value, error: null, checking: false });
    } catch (error) {
      if (revision === this.revision) this.update({ value: null, error: nativeFailure(error), checking: false });
    }
  };
}

export function connectionFailureFor(view: Pick<CloudLinkView, 'phase' | 'error_code' | 'error_message'> | null, latest: NativeFailure | null): NativeFailure | null {
  if (latest) return latest;
  if (view?.phase === 'expired') return { code: view.error_code || 'pairing_expired', message: '' };
  return view && (view.phase === 'failed' || view.error_code || view.error_message)
    ? { code: view.error_code || 'operation_failed', message: '' } : null;
}

export function localizedConnectionFailure(language: InterfaceLanguage, error: NativeFailure): string {
  const code = error.code === 'http_403' ? 'pairing_service_access'
    : /^http_5\d\d$/.test(error.code) || ['http_408', 'network', 'offline', 'timeout'].includes(error.code) ? 'pairing_service_retry' : error.code;
  return localizedFailure(language, { ...error, code });
}
