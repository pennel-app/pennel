import type { ShortcutBinding } from './api';

export interface ShortcutLayout { get(code: string): string | undefined }
export type DomShortcutEvent = Pick<KeyboardEvent, 'key'> & Partial<Pick<KeyboardEvent, 'code' | 'shiftKey' | 'altKey' | 'ctrlKey'>>;
interface LayoutProvider {
  getLayoutMap(): Promise<ShortcutLayout>;
  addEventListener?: EventTarget['addEventListener'];
  removeEventListener?: EventTarget['removeEventListener'];
}
let layout: ShortcutLayout | null = null;
let revision = 0;
const listeners = new Set<() => void>();
export function keyboardLayout(): ShortcutLayout | null { return layout; }
export function onShortcutLayoutChange(listener: () => void): () => void { listeners.add(listener); return () => listeners.delete(listener); }
function publish(next: ShortcutLayout | null) { layout = next; listeners.forEach(listener => listener()); }
function browserKeyboard(): LayoutProvider | null {
  if (typeof navigator === 'undefined') return null;
  const keyboard = (navigator as Navigator & { keyboard?: Partial<LayoutProvider> }).keyboard;
  return typeof keyboard?.getLayoutMap === 'function' ? keyboard as LayoutProvider : null;
}
export async function refreshKeyboardLayout(keyboard: LayoutProvider | null = browserKeyboard()): Promise<void> {
  const mine = ++revision;
  publish(null);
  let next: ShortcutLayout | null = null;
  try { next = await keyboard?.getLayoutMap() ?? null; } catch { /* Missing or denied layout access stays explicitly unavailable. */ }
  if (mine === revision) publish(next);
}
export function watchKeyboardLayout(): () => void {
  const keyboard = browserKeyboard();
  const refresh = () => { void refreshKeyboardLayout(keyboard); };
  keyboard?.addEventListener?.('layoutchange', refresh);
  window.addEventListener('focus', refresh);
  refresh();
  return () => { revision++; keyboard?.removeEventListener?.('layoutchange', refresh); window.removeEventListener('focus', refresh); };
}
function printable(key: string): boolean { return [...key].length === 1 && !/[\u0000-\u001f\u007f]/.test(key); }
/** The layout supplies base characters; physical code names never imply a US character. */
export function domShortcutKey(event: DomShortcutEvent, map: ShortcutLayout | null): string | null {
  // Space remains a named key when Alt turns its text into a non-breaking space.
  if (event.code === 'Space' || event.key === ' ' || event.key === 'Space' || event.key === 'Spacebar') return 'Space';
  if (!printable(event.key) || !event.shiftKey && !event.altKey) return event.key;
  const base = event.code ? map?.get(event.code) : undefined;
  return base && printable(base) ? base === ' ' ? 'Space' : base : null;
}
export function domBindingAvailable(binding: ShortcutBinding, map: ShortcutLayout | null): boolean {
  if (!binding.key || !printable(binding.key) || binding.key === ' '
      || !binding.mods.some(mod => ['shift','alt'].includes(mod.replace(/^(left|right)_/, '')))) return true;
  const base = binding.code ? map?.get(binding.code) : undefined;
  return !!base && printable(base);
}
