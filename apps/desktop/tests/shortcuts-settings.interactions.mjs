const assert=(condition,message)=>{if(!condition)throw Error(message);};
async function until(check,message){for(let i=0;i<160;i++){if(await check())return;await new Promise(r=>setTimeout(r,30));}throw Error(message);}
const settle=()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
const row=id=>window.__PENNEL_FIXTURE__.info().shortcuts.find(row=>row.id===id);
async function begin(id){document.querySelector(`[data-shortcut-record="${id}"]`).focus();document.querySelector(`[data-shortcut-record="${id}"]`).click();await until(()=>window.__PENNEL_FIXTURE__.shortcutCapture()?.action===id,'Capture did not start.');await settle();return window.__PENNEL_FIXTURE__.shortcutCapture();}
function emit(session,patch={}){window.__PENNEL_FIXTURE__.emit('shortcut-capture',{action:session.action,generation:session.generation,mods:window.__PENNEL_FIXTURE__.info().platform==='macos'?['alt','meta']:['ctrl','alt'],code:'KeyQ',key:'a',keycaps:window.__PENNEL_FIXTURE__.info().platform==='macos'?['⌥','⌘','A']:['Ctrl','Alt','A'],text:'A',done:true,cancelled:false,...patch});}
const conflict=id=>document.querySelector(`[data-shortcut-recorder="${id}"] .shortcut-conflict`);
export async function checkShortcutsSettings(){
 assert(document.querySelectorAll('[data-shortcut-action]').length===13,'Expected thirteen product shortcut rows.');
 assert(document.querySelectorAll('[data-shortcut-fixed]').length===2,'Fixed system rows missing.');
 assert(document.querySelector('[data-settings-category="shortcuts"][aria-current="page"]'),'Shortcuts category missing.');
 assert(document.querySelectorAll('[data-settings-category]').length===6,'Expected six Settings categories.');
 assert(!document.querySelector('[data-shortcut-action="command"] button'),'Derived Command became editable.');
 assert(document.querySelector('[data-shortcut-record="next_model"]').disabled,'Unconfigured next model shortcut is not disabled.');
 const recordDefault=JSON.stringify(row('record').binding),findDefault=JSON.stringify(row('find').binding);
 let session=await begin('find');emit(session);await until(()=>row('find').binding?.key==='a','Find binding not saved.');assert(document.activeElement===document.querySelector('[data-shortcut-record="find"]'),'Saving a shortcut lost recorder focus.');
 session=await begin('record');emit(session);await until(()=>!conflict('record').hidden,'Conflict not explained inline.');
 assert(JSON.stringify(row('record').binding)===recordDefault,'Conflict changed saved Record before approval.');
 document.querySelector('[data-shortcut-cancel="record"]').click();await settle();assert(JSON.stringify(row('record').binding)===recordDefault,'Cancel changed saved binding.');
 session=await begin('record');emit(session);await until(()=>!conflict('record').hidden,'Repeated conflict missing.');document.querySelector('[data-shortcut-replace="record"]').click();
 await until(()=>row('record').binding?.key==='a'&&row('find').binding===null,'Use it here did not transfer binding atomically.');
 assert(!document.querySelector('[data-shortcut-reset="find"]').hidden,'Cleared conflict target lacks Reset.');
 document.querySelector('[data-shortcut-reset="record"]').click();await until(()=>JSON.stringify(row('record').binding)===recordDefault,'Record reset failed.');
 document.querySelector('[data-shortcut-reset="find"]').click();await until(()=>JSON.stringify(row('find').binding)===findDefault,'Search reset failed.');
 session=await begin('find');emit(session,{mods:['meta'],code:'KeyQ',key:'q',keycaps:['⌘','Q']});await until(()=>!conflict('find').hidden,'Reserved system keys lack inline rejection.');assert(document.querySelector('[data-shortcut-replace="find"]').hidden,'Reserved binding offered replacement.');assert(JSON.stringify(row('find').binding)===findDefault,'Reserved key replaced current binding.');document.querySelector('[data-shortcut-cancel="find"]').click();
 session=await begin('find');emit(session,{mods:[],code:null,key:null,keycaps:[],cancelled:true});await until(()=>!window.__PENNEL_FIXTURE__.shortcutCapture(),'Escape did not end capture.');assert(JSON.stringify(row('find').binding)===findDefault,'Escape changed saved binding.');
 document.querySelector('[data-shortcut-clear="dictate"]').click();await until(()=>row('dictate').binding===null,'Clear Dictate failed.');
 const {t}=await import('/src/i18n.ts');assert(document.querySelector('[data-shortcut-action="command"] .shortcut-fixed-binding').textContent===t('shortcuts.none'),'Derived Command did not clear with Dictate.');
 const reset=document.querySelector('[data-shortcuts-reset-all]');assert(!reset.disabled,'Reset all is unavailable after changes.');reset.click();await until(()=>window.__PENNEL_FIXTURE__.info().shortcuts.every(row=>!row.changed),'Reset all did not restore every default.');
 session=await begin('find');document.querySelector('[data-settings-category="appearance"]').click();await until(()=>document.querySelector('[data-settings-category="appearance"][aria-current="page"]')&&!window.__PENNEL_FIXTURE__.shortcutCapture(),'Leaving the page left native capture active.');
 emit(session);await settle();assert(JSON.stringify(row('find').binding)===findDefault,'Disposed recorder saved late event.');
 document.querySelector('[data-settings-category="shortcuts"]').click();await until(()=>document.querySelector('.shortcuts-settings'),'Returning to Shortcuts failed.');
 await showShortcutConflict();document.querySelector('[data-shortcuts-reset-all]').click();await until(()=>window.__PENNEL_FIXTURE__.info().shortcuts.every(row=>!row.changed),'Reset all from conflict failed.');
 assert(conflict('record').hidden,'Reset all retained a stale replacement candidate.');
}
export async function showShortcutConflict(){
 let session=await begin('find');emit(session);await until(()=>row('find').binding?.key==='a','Find capture failed.');
 session=await begin('record');emit(session);await until(()=>!conflict('record').hidden,'Conflict alert missing.');
 document.querySelector('[data-shortcut-action="record"]').scrollIntoView({block:'center'});
}
export async function checkUnavailableShortcuts(){
 for(const action of ['dictate','cancel','record','open_pennel']){const recorder=document.querySelector(`[data-shortcut-recorder="${action}"]`);assert(recorder.querySelector('[data-shortcut-record]').disabled,`${action} unavailable recorder enabled`);assert(recorder.querySelector('.shortcut-unavailable').textContent.trim(),`${action} reason missing`);}
 await import('/tests/shortcut-recorder.interactions.mjs').then(m=>m.checkDomCapture());
}
