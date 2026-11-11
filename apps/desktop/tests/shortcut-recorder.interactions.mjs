const assert=(condition,message)=>{if(!condition)throw Error(message);};
async function until(check,message){for(let i=0;i<150;i++){if(await check())return;await new Promise(r=>setTimeout(r,30));}throw Error(message);}
const settle=()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
const row=id=>window.__PENNEL_FIXTURE__.info().shortcuts.find(row=>row.id===id);
async function begin(id){document.querySelector(`[data-shortcut-record="${id}"]`).click();await until(()=>window.__PENNEL_FIXTURE__.shortcutCapture()?.action===id,'Capture did not start.');await settle();return window.__PENNEL_FIXTURE__.shortcutCapture();}
function emit(session,patch={}){window.__PENNEL_FIXTURE__.emit('shortcut-capture',{action:session.action,generation:session.generation,mods:['ctrl','alt'],code:'KeyQ',key:'a',keycaps:['⌃','⌥','A'],text:'⌃⌥A',done:true,cancelled:false,...patch});}
export async function checkDictationRecorder(){
 const initial=row('dictate').binding;
 window.__PENNEL_FIXTURE__.emit('dictation-state',{state:'hands_free'});await settle();
 assert(document.querySelector('[data-shortcut-record="dictate"]').disabled && document.querySelector('[data-shortcut-clear="dictate"]').disabled && document.querySelector('[data-shortcut-preset]').disabled,'Live dictation did not disable every shortcut writer.');
 window.__PENNEL_FIXTURE__.emit('dictation-state',{state:'hidden'});await settle();
 const record=document.querySelector('[data-shortcut-record="dictate"]');assert(record&&!record.disabled,'Enabled native recorder missing.');
 let session=await begin('dictate');
 emit(session,{generation:session.generation-1});await settle();assert(JSON.stringify(row('dictate').binding)===JSON.stringify(initial),'Stale capture saved.');
 emit(session,{done:false,keycaps:['fn'],mods:['fn'],code:null,key:null});await settle();assert(record.textContent.includes('fn'),'Native preview missing.');
 window.dispatchEvent(new KeyboardEvent('keydown',{key:'n',code:'KeyN',metaKey:true,bubbles:true,cancelable:true}));await settle();assert(!window.__PENNEL_FIXTURE__.calls.includes('start_recording'),'Application command ran during capture.');
 emit(session);await until(()=>row('dictate').binding?.key==='a','Completed native binding not saved.');assert(!window.__PENNEL_FIXTURE__.shortcutCapture(),'Capture not stopped before save.');
 session=await begin('dictate');window.dispatchEvent(new Event('blur'));await until(()=>!window.__PENNEL_FIXTURE__.shortcutCapture(),'Blur left capture active.');emit(session,{mods:['fn'],code:null,key:null,keycaps:['fn']});await settle();assert(row('dictate').binding.key==='a','Late callback after blur replaced binding.');window.dispatchEvent(new Event('focus'));
 session=await begin('dictate');emit(session,{mods:['shift'],code:null,key:null,keycaps:['⇧']});await until(()=>!document.querySelector('[data-shortcut-recorder="dictate"] .shortcut-conflict').hidden,'Invalid Shift chord lacks inline explanation.');assert(row('dictate').binding.key==='a','Rejected Shift replaced current binding.');document.querySelector('[data-shortcut-cancel="dictate"]').click();
 document.querySelector('[data-shortcut-clear="dictate"]').click();await until(()=>row('dictate').binding===null,'Clear did not use central writer.');assert(!document.querySelector('[data-shortcut-reset="dictate"]').hidden,'Cleared default lacks Reset.');
 document.querySelector('[data-shortcut-reset="dictate"]').click();await until(()=>JSON.stringify(row('dictate').binding)===JSON.stringify(initial),'Reset did not restore default.');
 document.querySelector('[data-shortcut-preset="right_ctrl"]').click();await until(()=>row('dictate').binding?.mods[0]==='right_ctrl','Preset did not use central writer.');
 const {api}=await import('/src/api.ts');await api.resetShortcut('dictate');await settle();
}
export async function checkClearedHome(){
 const {t}=await import('/src/i18n.ts');await until(()=>document.querySelector('.dictation-home')?.dataset.dictationReadiness==='ready','Home not ready.');
 assert(document.querySelector('.dictation-home-instruction').innerText.trim()===t('shortcuts.button'),'Cleared shortcut home still asks to hold a key.');
 assert(document.querySelector('.dictation-home-instruction kbd').hidden,'Cleared shortcut keycap still visible.');
}
export async function checkDomCapture(){
 const session=await begin('find');assert(session.native===false,'Fallback fixture claimed native.');
 window.dispatchEvent(new KeyboardEvent('keydown',{key:'a',code:'KeyQ',altKey:true,[window.__PENNEL_FIXTURE__.info().platform==='macos'?'metaKey':'ctrlKey']:true,bubbles:true,cancelable:true}));
 await until(()=>row('find').binding?.key==='a','Application-only DOM capture failed.');
 const global=document.querySelector('[data-shortcut-record="dictate"]');assert(global.disabled,'Unavailable global recorder remained enabled.');
}
