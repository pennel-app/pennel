import type { ShortcutBinding, ShortcutRow } from './api';
import { domShortcutKey, type ShortcutLayout } from './shortcut-layout.ts';

export interface CaptureStarted { native: boolean; generation: number; reason: string | null }
export interface CaptureEvent extends ShortcutBinding { action: string; generation: number; keycaps: string[]; text: string; done: boolean; cancelled: boolean }
export interface CaptureState { phase: 'idle' | 'starting' | 'listening' | 'error'; action: string | null; keycaps: string[]; error?: unknown }
interface CaptureClient {
  start(action: string): Promise<CaptureStarted>;
  stop(): Promise<unknown>;
  listen(handler: (event: CaptureEvent) => void): Promise<() => void>;
  changed(state: CaptureState): void;
  binding(binding: ShortcutBinding): void | Promise<void>;
  layout?(): ShortcutLayout | null;
}
let owner: ShortcutCapture | null = null;
export function shortcutCaptureActive(): boolean { return owner !== null; }
export async function cancelShortcutCapture(): Promise<void> { await owner?.cancel(); }
/** Owns the native session until even a delayed activation has been stopped. */
export class ShortcutCapture {
  private client: CaptureClient;
  private current: { action: string; scope: ShortcutRow['scope']; valid: boolean; commitAllowed: boolean; finishing: boolean; started?: CaptureStarted; events: CaptureEvent[]; unlisten?: () => void } | null = null;
  private activation: Promise<void> = Promise.resolve();
  private completion: Promise<void> = Promise.resolve();
  constructor(client: CaptureClient) { this.client = client; }
  async start(action: string, scope: ShortcutRow['scope']): Promise<void> {
    if (owner) return;
    const session = { action, scope, valid: true, commitAllowed: true, finishing: false, events: [] } as NonNullable<ShortcutCapture['current']>;
    this.current = session; owner = this;
    this.client.changed({ phase: 'starting', action, keycaps: [] });
    this.activation = (async () => {
      try {
        session.unlisten = await this.client.listen(event => {
          if (!session.valid || this.current !== session || event.action !== action) return;
          if (!session.started) session.events.push(event); else this.receive(event);
        });
        if (!session.valid) return;
        session.started = await this.client.start(action);
        if (!session.valid) { await this.client.stop(); return; }
        if (!session.started.native && scope !== 'application') throw new Error('native_capture_unavailable');
        this.client.changed({ phase: 'listening', action, keycaps: [] });
        for (const event of session.events) this.receive(event);
        session.events = [];
      } catch (error) {
        if (session.valid) {
          session.valid = false;
          await this.client.stop().catch(() => undefined);
          this.release(session);
          this.client.changed({ phase: 'error', action, keycaps: [], error });
        }
      } finally {
        if (!session.valid && !session.finishing) this.release(session);
      }
    })();
    await this.activation;
  }
  private release(session: NonNullable<ShortcutCapture['current']>) {
    session.unlisten?.(); session.unlisten = undefined;
    if (this.current === session) { this.current = null; if (owner === this) owner = null; }
  }
  private receive(event: CaptureEvent) {
    const session = this.current;
    if (!session?.valid || event.generation !== session.started?.generation || event.action !== session.action) return;
    if (!event.done) { this.client.changed({ phase: 'listening', action: session.action, keycaps: event.keycaps }); return; }
    const binding = event.cancelled ? null : { mods: [...event.mods], code: event.code, key: event.key };
    this.finish(session, binding);
  }
  private finish(session: NonNullable<ShortcutCapture['current']>, binding: ShortcutBinding | null, error?: unknown) {
    session.valid = false; session.finishing = true;
    this.completion = this.activation.then(async () => {
      await this.client.stop();
      const current = this.current === session;
      this.release(session);
      if (current) this.client.changed(error ? { phase: 'error', action: session.action, keycaps: [], error } : { phase: 'idle', action: null, keycaps: [] });
      if (binding && current && session.commitAllowed) await this.client.binding(binding);
    }).catch(error => {
      const current = this.current === session;
      this.release(session);
      if (current) this.client.changed({ phase: 'error', action: session.action, keycaps: [], error });
    });
  }
  keydown(event: Pick<KeyboardEvent, 'key' | 'code'> & Partial<Pick<KeyboardEvent, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'repeat' | 'isComposing'>>): boolean {
    const session = this.current;
    if (!session) return false;
    if (!session.valid || session.started?.native !== false || session.scope !== 'application') return true;
    if (event.isComposing || event.repeat || ['Fn','Shift','Control','Alt','Meta','Dead','Unidentified'].includes(event.key)) return false;
    if (event.key === 'Escape' && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) { void this.cancel(); return true; }
    if (!event.code) return false;
    const key = domShortcutKey(event, this.client.layout?.() ?? null);
    if (key === null) { this.finish(session, null, new Error('dom_layout_unavailable')); return true; }
    this.receive({ action: session.action, generation: session.started.generation, mods: [event.ctrlKey && 'ctrl', event.altKey && 'alt', event.shiftKey && 'shift', event.metaKey && 'meta'].filter(Boolean) as string[], code: event.code, key, keycaps: [], text: '', done: true, cancelled: false });
    return true;
  }
  async cancel(): Promise<void> {
    const session = this.current;
    if (!session) return;
    const started = !!session.started;
    session.commitAllowed = false; session.valid = false;
    await this.activation;
    if (started) await this.client.stop();
    const current = this.current === session;
    this.release(session);
    if (current) this.client.changed({ phase: 'idle', action: null, keycaps: [] });
  }
  async settled(): Promise<void> { await this.activation; await this.completion; }
}
