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

export async function checkPillPreferences() {
  assert(window.__PENNEL_FIXTURE__, 'Use the owned synthetic fixture.');
  const { api } = await import('/src/api.ts');
  const { t } = await import('/src/i18n.ts');
  const click = (name, value) => { const button = document.querySelector(`[data-preference="${name}"] [data-value="${value}"]`); assert(button, `Missing ${name} ${value}`); button.click(); };
  await until(() => document.querySelector('[data-pill-preview]'), 'The settings preview did not render.');
  assert(window.__PENNEL_FIXTURE__.calls.filter(command => command === 'dictation_state').length === 1, 'The shortcut recorder must make exactly one initial activity read.');
  const callsBeforePreferences = window.__PENNEL_FIXTURE__.calls.length;
  document.querySelector('[data-dictation-group="pill"]').scrollIntoView({ block: 'start' });
  click('pill-colour', 'light'); click('pill-size', 'large');
  await until(async () => { const info = await api.info(); return info.settings.pill_colour === 'light' && info.settings.pill_size === 'large'; }, 'Colour and size choices did not persist.');
  const sample = document.querySelector('.pill-sample');
  assert(getComputedStyle(sample).backgroundColor === 'rgb(252, 251, 248)' && getComputedStyle(sample).fontFamily.includes('Figtree'), 'The preview lost its selected Light variant or fixed Figtree font.');
  const position = document.querySelector('[data-preference="pill-position"] .choice'); position.scrollIntoView({ block: 'center' }); position.click();
  const cursorOption = () => [...document.querySelectorAll('[role="option"]')].find(node => node.textContent.includes(t('dictation.pill.position.cursor')));
  await until(cursorOption, 'Position native-menu fallback did not open.');
  cursorOption().click();
  await until(async () => (await api.info()).settings.pill_position === 'near_cursor', 'The position choice did not persist.');
  click('pill-colour', 'system');
  await until(() => document.querySelector('[data-pill-preview]').dataset.pillColour === 'system', 'Match system did not update the preview.');
  const expected = matchMedia('(prefers-color-scheme: dark)').matches ? 'rgb(42, 39, 35)' : 'rgb(252, 251, 248)';
  assert(getComputedStyle(sample).backgroundColor === expected, 'Match system did not follow the operating-system appearance.');
  document.querySelector('[data-show-pill]').click();
  await until(() => window.__PENNEL_FIXTURE__.calls.includes('show_pill_preview'), 'Show the pill did not call the display-only backend action.');
  const calls = window.__PENNEL_FIXTURE__.calls;
  assert(!calls.some(command => /toggle_dictation|copy_|append_dictation/.test(command)) && !calls.slice(callsBeforePreferences).includes('dictation_state'), 'Pill preferences caused capture or text/clipboard actions.');
  document.querySelector('[data-dictation-group="pill"]').scrollIntoView({ block: 'start' });
  return { preferencesSaved: true, systemAppearance: true, fixedFigtree: true, noCaptureOrClipboard: true, syntheticOnly: true };
}

export async function checkPillPreferenceBounds({ enlarged = false, wayland = false } = {}) {
  await until(() => document.querySelector('[data-dictation-group="pill"]'), 'Missing pill preferences.');
  if (enlarged) {
    const values = [...document.querySelectorAll('#app *')].filter(node => node instanceof HTMLElement).map(node => [node, parseFloat(getComputedStyle(node).fontSize)]);
    for (const [node, size] of values) { node.style.fontSize = `${size * 2}px`; node.style.lineHeight = '1.4'; }
  }
  const group = document.querySelector('[data-dictation-group="pill"]');
  for (const control of group.querySelectorAll('button,input')) {
    if (control.hidden || control.closest('[hidden]')) continue;
    control.scrollIntoView({ block: 'center' });
    const rect = control.getBoundingClientRect();
    assert(rect.left >= 0 && rect.right <= innerWidth + 1 && rect.height > 0, 'A pill preference control is clipped horizontally.');
  }
  if (wayland) {
    assert(document.querySelector('[data-preference="pill-position"] .choice').disabled, 'Unsupported Wayland position remained enabled.');
    assert(!document.querySelector('[data-preference="pill-position"] .help').hidden, 'Wayland position explanation is hidden.');
    const reason = document.querySelector('[data-model-switch-reason]').textContent;
    assert(reason.trim() && !reason.includes('wayland_global_shortcuts'), 'The model switch displayed a raw platform reason code.');
  }
  group.scrollIntoView({ block: 'start' });
  return { reachableControls: true, enlarged, unsupportedPositionExplained: wayland, syntheticOnly: true };
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

export async function checkPillOppositeSystemPreview() {
  const fixture = window.__PENNEL_FIXTURE__;
  assert(fixture && fixture.options.app_appearance_opposite_system, 'Use the declared forced-app/opposite-system fixture.');
  const { api } = await import('/src/api.ts');
  await until(() => document.querySelector('[data-pill-preview][data-pill-colour="system"]'), 'Match Pennel preview did not load.');
  assert(fixture.calls.filter(command => command === 'dictation_state').length === 1, 'The shortcut recorder must make exactly one initial activity read.');
  const callsBeforeAppearance = fixture.calls.length;
  const preview = document.querySelector('[data-pill-preview]'), sample = document.querySelector('.pill-sample');
  const forced = fixture.options.appearance === 'dark' ? 'light' : 'dark';
  const expected = forced === 'dark' ? 'rgb(42, 39, 35)' : 'rgb(252, 251, 248)';
  await until(() => preview.dataset.pillAppearance === forced && getComputedStyle(sample).backgroundColor === expected, 'The preview ignored Pennel appearance in favour of the opposite OS media query.');
  const settings = (await api.info()).settings;
  await api.saveSettings({ ...settings, appearance: 'system' });
  const systemColour = matchMedia('(prefers-color-scheme: dark)').matches ? 'rgb(42, 39, 35)' : 'rgb(252, 251, 248)';
  await until(() => preview.dataset.pillAppearance === 'system' && getComputedStyle(sample).backgroundColor === systemColour, 'System appearance did not restore the OS-following preview.');
  await api.saveSettings({ ...settings, appearance: forced });
  await until(() => preview.dataset.pillAppearance === forced && getComputedStyle(sample).backgroundColor === expected, 'The following preview did not return to the explicit Pennel override.');
  assert(!fixture.calls.some(command => /toggle_dictation|copy_|append_dictation/.test(command)) && !fixture.calls.slice(callsBeforeAppearance).includes('dictation_state'), 'Appearance resolution caused capture/text side effects.');
  document.querySelector('[data-dictation-group="pill"]').scrollIntoView({ block: 'start' });
  return { forcedAppOppositeSystem: true, globalAppearanceHierarchy: true, noNativeThemeOverride: true, syntheticOnly: true };
}
