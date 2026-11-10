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
