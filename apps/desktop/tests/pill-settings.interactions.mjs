const pause = (ms = 20) => new Promise(resolve => setTimeout(resolve, ms));
const assert = (value, message) => { if (!value) throw new Error(message); };
async function until(check, message) { for (let i = 0; i < 100; i++) { if (await check()) return; await pause(); } throw new Error(message); }

export async function checkPillModelSettings() {
  assert(window.__PENNEL_FIXTURE__, 'Use the owned synthetic fixture.');
  const { api } = await import('/src/api.ts');
  await until(() => !document.querySelector('[data-speech-model-picker]')?.hidden && !document.querySelector('[data-speech-model-picker]')?.disabled, 'The configured speech model picker was unavailable.');
  const picker = document.querySelector('[data-speech-model-picker]'); picker.click();
  await until(() => [...document.querySelectorAll('[role^="menuitem"],[role="option"]')].some(node => node.textContent.includes('Apple dictation')), 'The model menu omitted a ready model.');
  [...document.querySelectorAll('[role^="menuitem"],[role="option"]')].find(node => node.textContent.includes('Apple dictation')).click();
  await until(async () => (await api.info()).settings.dictation_stt.provider_id === 'apple-speech', 'Selecting a ready model did not persist the actual choice.');
  const row = document.querySelector('[data-preference="model-switch"]'); row.scrollIntoView({ block: 'center' });
  const input = row.querySelector('input[role="switch"]');
  input.click();
  await until(async () => (await api.info()).settings.pill_model_switch_enabled === false, 'The real model-switch preference was not persisted.');
  assert(!window.__PENNEL_FIXTURE__.calls.some(command => /toggle_dictation|copy_dictation|copy_history/.test(command)), 'Model preferences caused a capture or clipboard action.');
  row.scrollIntoView({ block: 'center' });
  return { readyModelMenu: true, actualChoiceSaved: true, toggleSaved: true, noCaptureOrClipboard: true, syntheticOnly: true };
}

export async function checkPillModelUnavailableReasons() {
  assert(window.__PENNEL_FIXTURE__, 'Use the owned synthetic fixture.');
  const { api } = await import('/src/api.ts');
  const { t } = await import('/src/i18n.ts');
  const fixture = window.__PENNEL_FIXTURE__, client = window.__PENNEL_PLATFORM__, original = client.invoke;
  let reason = 'next_model_hook_unavailable';
  const binding = { mods: ['left_alt'], code: null, key: null };
  client.invoke = async (command, args) => {
    const value = await original(command, args);
    if (command !== 'app_info' && command !== 'save_settings') return value;
    return { ...value, shortcuts: [{ id: 'next_model', scope: 'listening', binding, default_binding: binding, label: { keycaps: ['⌥'], text: '⌥' }, editable: true, changed: false, capability: { available: false, reason } }] };
  };
  const status = () => document.querySelector('[data-model-switch-reason]') ?? document.querySelector('[data-preference="model-switch"] [role="status"]');
  try {
    for (const [code, key] of [
      ['next_model_hook_unavailable', 'backend.next_model_hook_unavailable'],
      ['accessibility_required', 'backend.shortcut_accessibility_required'],
      ['native_key_not_supported', 'backend.shortcut_native_key_not_supported'],
      ['wayland_global_shortcuts', 'backend.shortcut_wayland_global_shortcuts'],
    ]) {
      reason = code; fixture.emit('settings-changed', null);
      try { await until(() => status()?.textContent === t(key), `The existing unavailable binding lost its localized ${code} recovery.`); }
      catch (error) { throw new Error(`${error.message} Actual model-row recovery: ${status()?.textContent}`); }
    }
    reason = null; fixture.emit('settings-changed', null);
    await until(() => status()?.textContent === t('dictation.pill.switch_unavailable'), 'An existing binding without a reason was mislabeled as a missing shortcut.');
    reason = 'next_model_switch_disabled';
    const settings = (await api.info()).settings;
    await api.saveSettings({ ...settings, pill_model_switch_enabled: false });
    fixture.emit('settings-changed', null);
    await until(() => status()?.textContent === t('backend.next_model_switch_disabled'), 'The disabled switch incorrectly suggested recording a missing shortcut.');
    assert((await api.info()).settings.dictation_models.length >= 2, 'The fixture lost its two configured models.');
    document.querySelector('[data-dictation-group="pill"]').scrollIntoView({ block: 'center' });
    return { existingBindingReasons: true, disabledToggleReason: true, localizedStandardCodes: true, syntheticOnly: true };
  } finally { client.invoke = original; }
}
