import { playMotion } from "./motion";
import { api, listen, listenToWindow, type PillState, type ShortcutBinding, type ShortcutRow } from './api';
import { bindLocalization, l, t, type UiText } from './i18n.ts';
import { controlButton, errorText, h, keycaps } from './dom';
import { icon } from './icons';
import type { Context } from './main';
import { ShortcutCapture, type CaptureEvent, type CaptureState } from './shortcut-capture-state';
import { domBindingAvailable, keyboardLayout, onShortcutLayoutChange } from './shortcut-layout.ts';
import { shortcutName, shortcutReason, shortcutRow, shortcutText } from './shortcut-presentation';

interface ShortcutProblem { kind: string; reason: string; action?: string | null }
function problem(value: unknown): ShortcutProblem | null {
  return value !== null && typeof value === 'object' && 'kind' in value && 'reason' in value ? value as ShortcutProblem : null;
}
export function shortcutRecorder(ctx: Context, action: string, options: { presets?: { label: UiText; binding: ShortcutBinding }[]; recordLabel?: boolean } = {}): HTMLElement {
  let disposed = false, saving = false, listening = false, candidate: ShortcutBinding | null = null, issue: ShortcutProblem | null = null, failure: unknown = null, saved = false;
  let state: CaptureState = { phase: 'idle', action: null, keycaps: [] };
  const display = h('span', { class: 'shortcut-current' });
  const record = controlButton({ class: 'shortcut-record', 'data-shortcut-record': action, onclick: () => {
    if (state.phase === 'listening' || state.phase === 'starting') { void capture.cancel().catch(showError); return; }
    issue = null; failure = null; saved = false;
    const row = shortcutRow(ctx.info, action);
    if (row?.editable && canConfigure(row)) void capture.start(action, row.scope);
  } }, display);
  const clear = controlButton({ class: 'button-icon shortcut-clear', 'data-shortcut-clear': action, onclick: () => { void write(null); } }, icon('close'));
  const reset = controlButton({ class: 'button-icon shortcut-reset', 'data-shortcut-reset': action, onclick: () => { void write(undefined); } }, icon('rotate'));
  const status = h('p', { class: 'help shortcut-status', role: 'status', 'aria-live': 'polite', hidden: true });
  const reason = h('p', { class: 'help shortcut-unavailable', hidden: true });
  const alert = h('div', { class: 'shortcut-conflict', role: 'alert', hidden: true });
  const message = h('p');
  const replace = controlButton({ class: 'btn', 'data-shortcut-replace': action, onclick: () => { if (candidate) void write(candidate, true); } }, l('shortcuts.replace'));
  const cancel = controlButton({ class: 'btn', 'data-shortcut-cancel': action, onclick: () => { issue = null; candidate = null; failure = null; refresh(); record.focus(); } }, l('common.cancel'));
  alert.append(message, h('div', { class: 'row' }, replace, cancel));
  const presets = h('div', { class: 'shortcut-presets', 'aria-label': l('dictation.shortcut.title') }, ...(options.presets ?? []).map(preset => controlButton({ class: 'btn', 'data-shortcut-preset': preset.binding.mods.join(','), onclick: () => { void write(preset.binding); } }, preset.label)));
  const root = h('div', { class: 'shortcut-recorder', 'data-shortcut-recorder': action }, h('div', { class: 'shortcut-recorder-controls' }, record, clear, reset), options.presets ? presets : null, status, reason, alert);
  root.addEventListener('shortcut-reset', () => { issue = null; candidate = null; failure = null; saved = false; refresh(); });
  const showError = (error: unknown) => { if (!disposed) { failure = error; refresh(); } };
  const capture = new ShortcutCapture({ start: api.shortcutCaptureStart, stop: api.shortcutCaptureStop,
    listen: async handler => listenToWindow<CaptureEvent>('shortcut-capture', event => handler(event.payload)),
    changed: next => { if (!disposed) { state = next; if (next.phase === 'error') failure = next.error; refresh(); } },
    binding: binding => write(binding), layout: keyboardLayout,
  });
  async function write(binding: ShortcutBinding | null | undefined, replaceConflict = false) {
    if (saving || disposed) return;
    try { await capture.cancel(); } catch (error) { showError(error); return; }
    const row = shortcutRow(ctx.info, action);
    if (binding && row?.scope === 'application' && ctx.info.platform !== 'macos' && !domBindingAvailable(binding, keyboardLayout())) {
      showError(new Error('dom_layout_unavailable')); return;
    }
    saving = true; candidate = binding ?? null; issue = null; failure = null; saved = false; refresh();
    try {
      await ctx.mutate(async () => {
        try { const info = binding === undefined ? await api.resetShortcut(action) : await api.setShortcut(action, binding, replaceConflict); saved = true; return info; }
        catch (error) { const value = problem(error); if (value) { issue = value; return ctx.info; } throw error; }
      });
    } catch (error) { failure = error; }
    finally { saving = false; if (!disposed) refresh(); }
  }
  function canConfigure(row: ShortcutRow): boolean {
    return row.capability.available || row.capability.reason === 'dom_layout_unavailable'
      || action === 'next_model' && ['next_model_switch_disabled','next_model_shortcut_cleared'].includes(row.capability.reason ?? '');
  }
  let displayKey = '';
  function refresh() {
    if (disposed) return;
    const row = shortcutRow(ctx.info, action), recording = state.phase === 'starting' || state.phase === 'listening';
    const blocked = listening || !row?.editable || !canConfigure(row);
    const name = shortcutName(action);
    record.disabled = saving || blocked;
    record.setAttribute('aria-label', `${name}: ${recording ? t('shortcuts.press') : shortcutText(ctx.info, action)}. ${t('shortcuts.record')}`);
    record.setAttribute('aria-pressed', String(recording));
    const nextDisplay = JSON.stringify(recording ? [state.phase, state.keycaps, t('shortcuts.press')] : [row?.binding, row?.label.keycaps, t('shortcuts.none')]);
    if (displayKey !== nextDisplay) {
      const previousKeys = [...display.querySelectorAll('kbd')].map(key => key.textContent);
      displayKey = nextDisplay;
      display.replaceChildren(...recording ? state.keycaps.length ? [keycaps(state.keycaps)] : [t(state.phase === 'starting' ? 'shortcuts.starting' : 'shortcuts.press')] : row?.binding ? [keycaps(row.label.keycaps)] : [t('shortcuts.none')]);
      if (recording) display.querySelectorAll('kbd').forEach((key, index) => { if (previousKeys[index] !== key.textContent) playMotion(key, 'motion-feedback'); });
    }
    let label = record.querySelector<HTMLElement>('.shortcut-record-label');
    if (options.recordLabel && !recording) { if (!label) { label = h('span', { class: 'shortcut-record-label' }); record.append(label); } label.textContent = t('shortcuts.record'); } else label?.remove();
    clear.hidden = !row?.binding || recording; clear.disabled = saving || blocked; clear.setAttribute('aria-label', t('shortcuts.clear', { action: name }));
    reset.hidden = !row?.changed || recording; reset.disabled = saving || blocked; reset.setAttribute('aria-label', t('shortcuts.reset', { action: name }));
    presets.querySelectorAll<HTMLButtonElement>('button').forEach(button => {
      button.disabled = blocked || saving || recording;
      button.setAttribute('aria-pressed', String(!row?.binding?.code && row?.binding?.mods.join(',') === button.dataset.shortcutPreset));
    });
    status.textContent = failure ? captureError(failure) : recording ? t('shortcuts.cancel_hint') : saved ? t('shortcuts.saved') : '';
    status.classList.toggle('error', !!failure); status.hidden = !status.textContent;
    reason.textContent = listening ? t('shortcuts.reason.input_active') : shortcutReason(row?.capability.reason); reason.hidden = !reason.textContent;
    alert.hidden = !issue;
    message.textContent = issue ? issue.kind === 'conflict' ? t('shortcuts.conflict', { action: shortcutName(issue.action ?? '') }) : t('shortcuts.invalid', { reason: shortcutReason(issue.reason) }) : '';
    replace.hidden = issue?.kind !== 'conflict' || issue.action === 'command' || !candidate;
    replace.disabled = saving; cancel.disabled = saving;
  }
  function captureError(error: unknown): string {
    const text = errorText(error);
    if (text === 'dom_layout_unavailable') return shortcutReason(text);
    if (/Finish dictating/.test(text)) return t('shortcuts.reason.listening');
    if (/Focus|lost focus/.test(text)) return t('shortcuts.reason.focus');
    if (/Native shortcut capture|native_capture_unavailable/.test(text)) return t('shortcuts.reason.native_capture');
    return shortcutReason(text);
  }
  const keydown = (event: KeyboardEvent) => {
    if (capture.keydown(event)) { event.preventDefault(); event.stopImmediatePropagation(); }
  };
  window.addEventListener('keydown', keydown, true);
  const stop = () => capture.cancel().catch(showError);
  ctx.onFocusChange(focused => { if (!focused) void stop(); });
  ctx.onVisibilityChange(visible => { if (!visible) void stop(); });
  ctx.beforeLeave(stop);
  ctx.onDispose(() => { disposed = true; window.removeEventListener('keydown', keydown, true); void capture.cancel().catch(() => undefined); });
  let activityEvent = false;
  const activity = (pill: PillState) => { if (!disposed) { listening = ['listening', 'hands_free', 'command', 'no_sound'].includes(pill.state); refresh(); } };
  void listen<PillState>('dictation-state', event => { activityEvent = true; activity(event.payload); }).then(unlisten => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(showError);
  void api.dictationState().then(pill => { if (!activityEvent) activity(pill); }).catch(showError);
  ctx.onDispose(onShortcutLayoutChange(refresh));
  ctx.onInfoChange(refresh); bindLocalization(root, refresh); refresh();
  return root;
}
