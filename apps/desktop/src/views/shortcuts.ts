import { api, listen, type PillState } from '../api';
import { bindLocalization, l, t, type TranslationKey } from '../i18n.ts';
import { controlButton, errorText, h, keycaps, section } from '../dom';
import type { Context } from '../main';
import { shortcutRecorder } from '../shortcut-recorder';
import { commandKeycaps, shortcutName, shortcutReason } from '../shortcut-presentation';
import { cancelShortcutCapture } from '../shortcut-capture-state';

export function shortcutsView(ctx: Context): HTMLElement {
  let disposed = false, saving = false, listening = false, saved = false;
  let failure: unknown = null;
  const status = h('p', { class: 'help shortcuts-reset-status', role: 'status', 'aria-live': 'polite', hidden: true });
  const reset = controlButton({ class: 'btn', 'data-shortcuts-reset-all': true, onclick: async () => {
    if (saving) return;
    saving = true; saved = false; failure = null; refresh();
    try {
      await cancelShortcutCapture();
      await ctx.mutate(async () => {
        try { const info = await api.resetAllShortcuts(); saved = true; return info; }
        catch (error) { failure = error; return ctx.info; }
      });
      if (saved && !disposed) root.querySelectorAll('[data-shortcut-recorder]').forEach(recorder => recorder.dispatchEvent(new Event('shortcut-reset')));
    } catch (error) { failure = error; }
    finally { saving = false; if (!disposed) refresh(); }
  } }, l('shortcuts.reset_all'));
  const descriptions: Record<string, TranslationKey> = {
    dictate: 'shortcuts.help.dictate', cancel: 'shortcuts.help.cancel', next_model: 'shortcuts.help.next_model',
    record: 'shortcuts.help.record', open_pennel: 'shortcuts.help.open_pennel', new_recording: 'shortcuts.help.new_recording', find: 'shortcuts.help.find',
  };
  const row = (action: string) => h('div', { class: 'shortcut-settings-row', 'data-shortcut-action': action },
    h('div', { class: 'shortcut-settings-copy' }, h('h3', {}, l(`shortcuts.action.${action}` as TranslationKey)), descriptions[action] ? h('p', { class: 'help' }, l(descriptions[action])) : null), shortcutRecorder(ctx, action));
  const command = h('span', { class: 'shortcut-fixed-binding' });
  const derived = h('div', { class: 'shortcut-settings-row', 'data-shortcut-action': 'command' },
    h('div', { class: 'shortcut-settings-copy' }, h('h3', {}, l('shortcuts.action.command')), h('p', { class: 'help' }, l('shortcuts.following'))), command);
  const fixed = (action: 'close' | 'quit') => {
    const keys = ctx.info.platform === 'macos' ? ['⌘', action === 'close' ? 'W' : 'Q'] : action === 'close' ? ['Ctrl', 'W'] : [];
    return h('div', { class: 'shortcut-settings-row shortcut-system-row', 'data-shortcut-fixed': action },
      h('div', { class: 'shortcut-settings-copy' }, h('h3', {}, l(action === 'close' ? 'shortcuts.close' : 'shortcuts.quit')), h('p', { class: 'help' }, l('shortcuts.fixed'))),
      h('span', { class: 'shortcut-fixed-binding' }, keys.length ? keycaps(keys) : l('shortcuts.system')));
  };
  const root = h('div', { class: 'shortcuts-settings' },
    h('div', { class: 'shortcuts-intro' }, h('p', { class: 'help' }, l('shortcuts.intro')), reset), status,
    section(l('shortcuts.global'), null, row('dictate'), derived, row('cancel'), row('next_model'), row('record'), row('open_pennel')),
    section(l('shortcuts.application'), null, ...['new_recording','find','dictation','meetings','history','toggle_sidebar','settings'].map(row), fixed('close'), fixed('quit')));
  function refresh() {
    if (disposed) return;
    const keys = commandKeycaps(ctx.info);
    command.replaceChildren(keys.length ? keycaps(keys) : t('shortcuts.none'));
    command.setAttribute('aria-label', `${shortcutName('command')}: ${keys.length ? keys.join(' ') : t('shortcuts.none')}`);
    reset.disabled = saving || listening || !ctx.info.shortcuts?.some(row => row.changed) && !Object.keys(ctx.info.settings.shortcuts ?? {}).length;
    const reason = failure && typeof failure === 'object' && 'reason' in failure ? shortcutReason(String(failure.reason)) : failure ? errorText(failure) : null;
    status.textContent = reason ?? (listening ? t('shortcuts.reason.input_active') : saved ? t('shortcuts.reset_done') : '');
    status.classList.toggle('error', !!failure); status.hidden = !status.textContent;
  }
  let activityEvent = false;
  const activity = (pill: PillState) => { if (!disposed) { listening = ['listening','hands_free','command','no_sound'].includes(pill.state); refresh(); } };
  void listen<PillState>('dictation-state', event => { activityEvent = true; activity(event.payload); }).then(unlisten => { if (disposed) unlisten(); else ctx.onDispose(unlisten); }).catch(error => { failure = error; refresh(); });
  void api.dictationState().then(pill => { if (!activityEvent) activity(pill); }).catch(error => { failure = error; refresh(); });
  ctx.onDispose(() => { disposed = true; }); ctx.onInfoChange(refresh); bindLocalization(root, refresh); refresh();
  return root;
}
