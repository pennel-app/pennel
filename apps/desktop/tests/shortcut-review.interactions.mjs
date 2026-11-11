const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let i=0;i<150;i++){if(await check())return;await new Promise(r=>setTimeout(r,20));}throw Error(message);}
const settle=()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
const fixture=()=>window.__PENNEL_FIXTURE__;
const row=id=>fixture().info().shortcuts.find(row=>row.id===id);
async function begin(id){const button=document.querySelector(`[data-shortcut-record="${id}"]`);assert(button&&!button.disabled,'Recorder unavailable.');button.focus();button.click();await until(()=>fixture().shortcutCapture()?.action===id,'Capture did not start.');await settle();return fixture().shortcutCapture();}
function key(event){const input=new KeyboardEvent('keydown',{bubbles:true,cancelable:true,...event});window.dispatchEvent(input);return input;}
async function back(){document.querySelector('[data-action="back-to-work"]').click();await until(()=>document.querySelector('.library-sidebar:not([hidden])'),'Did not return to work.');}
async function settings(){document.querySelector('.library-settings').click();await until(()=>document.querySelector('.shortcuts-settings'),'Did not return to Shortcuts.');}
export async function checkLayoutMappedCaptureAndDispatch(){
 const {refreshKeyboardLayout}=await import('/src/shortcut-layout.ts');
 await refreshKeyboardLayout({getLayoutMap:async()=>new Map([['Digit1','1'],['KeyQ','a']])});
 for(const [input,base] of [[{key:'!',code:'Digit1',ctrlKey:true,shiftKey:true},'1'],[{key:'æ',code:'KeyQ',ctrlKey:true,altKey:true},'a']]){
  const capture=await begin('find');assert(capture.native===false,'Expected the explicit DOM fallback.');key(input);
  await until(()=>row('find').binding?.key===base,'DOM capture did not save its layout-derived base key.');
  assert(row('find').binding.code===input.code,'DOM capture replaced physical code/key separation.');
  await back();const search=document.querySelector('.library-search input');search.blur();
  document.querySelector('[data-command="toggle_sidebar"]').click();
  const command=key(input);await until(()=>document.activeElement===search,'The saved canonical binding did not dispatch Search.');assert(command.defaultPrevented,'Search did not consume its command.');
  await settings();
 }
 const saved=JSON.stringify(row('find').binding);await begin('find');key({key:'a',code:'KeyQ',ctrlKey:true});
 await until(()=>!document.querySelector('[data-shortcut-recorder="find"] .shortcut-conflict').hidden,'Reserved editing keys were not rejected.');
 assert(document.querySelector('[data-shortcut-replace="find"]').hidden&&JSON.stringify(row('find').binding)===saved,'Reserved keys offered replacement or changed the saved binding.');document.querySelector('[data-shortcut-cancel="find"]').click();
 return {shiftedDigitBaseKey:true,nonUSAltBaseKey:true,centralWriterAndDispatchAgree:true};
}
export async function checkUnavailableLayoutCapture(){
 const {refreshKeyboardLayout}=await import('/src/shortcut-layout.ts');const {t}=await import('/src/i18n.ts');
 await refreshKeyboardLayout(null);const original=JSON.stringify(row('find').binding);
 const recorder=document.querySelector('[data-shortcut-recorder="find"]');
 assert(recorder.querySelector('.shortcut-unavailable').textContent===t('shortcuts.reason.layout'),'Saved ambiguous shortcut lacks a truthful localized limitation.');
 const before=fixture().calls.filter(call=>call==='set_shortcut').length;
 const capture=await begin('find');
 if(capture.native) fixture().emit('shortcut-capture',{...capture,mods:['ctrl','shift'],code:'Digit1',key:'1',keycaps:['Ctrl','Shift','1'],text:'Ctrl+Shift+1',done:true,cancelled:false});
 else key({key:'!',code:'Digit1',ctrlKey:true,shiftKey:true});
 await until(()=>!fixture().shortcutCapture(),'Unavailable layout left capture active.');
 await until(()=>recorder.querySelector('.shortcut-status').textContent===t('shortcuts.reason.layout'),'Modified capture did not explain the missing base-key mapping.');
 assert(JSON.stringify(row('find').binding)===original,'Unavailable map altered the saved shortcut.');assert(fixture().calls.filter(call=>call==='set_shortcut').length===before,'An ambiguous candidate reached the central writer.');
 await back();const search=document.querySelector('.library-search input');search.blur();const command=key({key:'!',code:'Digit1',ctrlKey:true,shiftKey:true});await settle();
 assert(!command.defaultPrevented&&document.activeElement!==search,'Unavailable-map dispatch claimed a shifted printable shortcut.');
 await settings();const next=await begin('find');
 if(next.native)fixture().emit('shortcut-capture',{...next,mods:[],code:'F5',key:'F5',keycaps:['F5'],text:'F5',done:true,cancelled:false});else key({key:'F5',code:'F5'});
 await until(()=>row('find').binding?.key==='F5','A function-key replacement should remain available without a layout map.');
 return {unavailableMapExplained:true,savedBindingPreserved:true,ambiguousWriterBlocked:true,ambiguousDispatchBlocked:true,functionKeyRecovery:true,native:capture.native};
}

export async function checkX11CapabilityProjection(){
 const {localizeBackendText}=await import('/src/i18n.ts');const {api}=await import('/src/api.ts');
 for(const action of ['record','open_pennel']){
  const recorder=document.querySelector(`[data-shortcut-recorder="${action}"]`);
  assert(row(action).capability.reason==='x11_keyed_shortcuts_unavailable'&&!row(action).capability.available,'X11 keyed global capability was not projected.');
  assert(recorder.querySelector('[data-shortcut-record]').disabled,'Unavailable keyed global recorder remained enabled.');
  assert(recorder.querySelector('.shortcut-unavailable').textContent===localizeBackendText('x11_keyed_shortcuts_unavailable'),'X11 reason was not localized.');
 }
 assert(fixture().info().capabilities.record_shortcut.status==='unsupported','Legacy recording toggle did not receive the X11 gap.');
 for(const action of ['dictate','cancel','find'])assert(!document.querySelector(`[data-shortcut-record="${action}"]`).disabled,'Safe modifier-only or application control was disabled.');
 let rejected=false;try{await api.shortcutCaptureStart('record');}catch(error){rejected=String(error).includes('x11_keyed_shortcuts_unavailable');}assert(rejected&&!fixture().shortcutCapture(),'Unsupported X11 capture activated a session.');
 const original=JSON.stringify(row('dictate').binding),capture=await begin('dictate');assert(capture.native,'X11 modifier observer did not remain native.');
 const input=key({key:'a',code:'KeyA',ctrlKey:true});assert(input.defaultPrevented&&fixture().shortcutCapture(),'Native observation allowed a DOM edit/default or constructed a second binding.');
 fixture().emit('shortcut-capture',{...capture,mods:[],code:null,key:null,keycaps:[],text:'',done:true,cancelled:true});
 await until(()=>!fixture().shortcutCapture(),'Native observer cancellation did not stop.');assert(JSON.stringify(row('dictate').binding)===original,'Native DOM suppression changed the binding.');
 return {keyedGlobalsUnavailable:true,localizedReason:true,safeModifiersRetained:true,domDefaultsConsumed:true};
}
