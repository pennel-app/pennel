const assert = (condition, message) => { if (!condition) throw Error(message); };
async function until(check, message) { for(let i=0;i<150;i++){if(await check()) return;await new Promise(r=>setTimeout(r,30));}throw Error(message); }
const settle = () => new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
export async function checkDictationSettings() {
  assert(window.__PENNEL_FIXTURE__, 'Use the owned fixture.');
  const root=document.querySelector('.dictation-task-settings');
  assert(root, 'Dictation Settings missing.');
  assert([...root.querySelectorAll('[data-dictation-group]')].map(x=>x.dataset.dictationGroup).join(',')==='processing,during,words,permissions','Dictation groups do not follow the approved order.');
  assert(!document.querySelector('[data-settings-category="dictionary"]'),'Dictionary still duplicates a category.');
  assert(!root.querySelector('[data-settings-detail="styles"]'),'Styles is still hidden in a disclosure.');
  const change=root.querySelector('[data-setup-task="dictation_stt"]');change.focus();
  window.__PENNEL_FIXTURE__.emit('settings-changed',null);await settle();await new Promise(r=>setTimeout(r,100));
  assert(change.isConnected && document.activeElement===change,'Information refresh replaced the focused setup action.');
  const {api}=await import('/src/api.ts');
  const mode=root.querySelector('[data-preference="result"] .choice');
  mode.querySelector('[data-value="clean"]').click();
  await until(()=>window.__PENNEL_FIXTURE__.info().settings.mode==='clean','Result did not save.');await settle();
  assert(mode.isConnected && mode.value==='clean','Saving Result replaced its control.');
  root.querySelector('[data-open-dictation="dictionary"]').click();
  await until(()=>document.querySelector('.dictionary-settings'),'Dictionary row did not open the complete page.');
  await checkDictationSubpageDraft('dictionary');
  document.querySelector('[data-dictation-settings-back]').click();
  await until(()=>document.querySelector('.dictation-task-settings [data-open-dictation="styles"]'),'Dictionary Back did not return to Dictation.');
  document.querySelector('[data-open-dictation="styles"]').click();
  await until(()=>document.querySelector('.style-settings'),'Styles row did not open the complete page.');
  await checkDictationSubpageDraft('styles');
  document.querySelector('[data-dictation-settings-back]').click();
  await until(()=>document.querySelector('.dictation-task-settings'),'Styles Back did not return.');
  const permissions=document.querySelector('[data-dictation-group="permissions"]');
  assert(permissions.querySelector('[data-permission-kind="microphone"]'),'Microphone permission row missing.');
  assert(permissions.querySelector('[data-permission-kind="accessibility"]'),'Accessibility permission row missing.');
  assert((await api.info()).settings.mode==='clean','Navigation changed saved Result.');
}
export async function checkDictationSubpageDraft(kind) {
  const root=document.querySelector(kind==='dictionary'?'.dictionary-settings':'.style-settings');
  const field=root?.querySelector('input[type=text]');assert(field,'Full subpage field missing.');
  assert(document.querySelector('[data-settings-category="dictation"][aria-current="page"]'),'Subpage did not select Dictation category.');
  assert(document.querySelector('[data-dictation-settings-back]'),'Subpage has no Back.');
  const details=field.closest('details');if(details)details.open=true;
  field.value='Unfinished draft term';field.focus();field.setSelectionRange(3,10);
  const {api}=await import('/src/api.ts');const info=await api.info();
  await api.saveSettings({...info.settings,appearance:info.settings.appearance==='dark'?'light':'dark',providers:info.settings.providers.map(provider=>provider.id===info.settings.dictation_stt.provider_id?{...provider,name:'Synthetic local service'}:provider)});
  await new Promise(r=>setTimeout(r,100));await settle();
  assert(field.isConnected && document.activeElement===field && field.value==='Unfinished draft term' && field.selectionStart===3 && field.selectionEnd===10,'Live information/theme refresh lost the subpage draft or selection.');
  window.__PENNEL_FIXTURE__.emit('models-changed',null);await settle();
  assert(field.isConnected && field.value==='Unfinished draft term','Model refresh discarded the draft.');
}

export async function checkDictationPermissionRepair() {
  const row=document.querySelector('[data-permission-kind="microphone"]'), button=row?.querySelector('button');
  assert(button && !button.hidden && !button.disabled,'Missing microphone recovery action.');
  button.focus();button.click();
  await until(()=>row.querySelector('.permission-status.success'),'Granting access did not refresh the real permission state.');
  assert(row.isConnected,'Permission repair replaced its row.');
  assert(window.__PENNEL_FIXTURE__.calls.includes('request_permission'),'Permission repair did not call the backend permission operation.');
  row.scrollIntoView({block:'center'});
}
