const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let attempt=0;attempt<200;attempt++){if(await check())return;await new Promise(resolve=>setTimeout(resolve,25));}throw Error(message);}
const settle=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
async function owned(){await until(()=>document.querySelector('.meeting-companion')&&window.__PENNEL_DESKTOP_COMMAND_STATE__?.ready,'The companion workspace did not load.');return document.querySelector('.live-workspace');}
function press(root,pane){const button=root.querySelector(`[data-live-tab="${pane}"]`);button.focus({preventScroll:true});button.click();}
export async function checkMeetingCompanion(){
 const root=await owned(), fixture=window.__PENNEL_FIXTURE__, wide=innerWidth>=1100, companion=root.querySelector('.meeting-companion'), primary=root.querySelector('.meeting-primary-column'), note=root.querySelector('.live-notes');
 assert(root.classList.contains('live-wide')===wide&&companion.hidden===!wide,'The initial companion default does not follow viewport width.');
 if(wide)assert(Math.abs(companion.getBoundingClientRect().width-340)<2,'The default wide companion did not retain the prototype340px width.');
 assert(root.querySelectorAll('.meeting-companion-tabs').length===1,'The companion duplicated its segmented control.');
 assert(root.querySelector('.meeting-companion-tabs').parentElement===(wide?root.querySelector('.meeting-companion-header'):root.querySelector('.meeting-header-actions')),'The retained segmented control is in the wrong header.');
 const width=primary.getBoundingClientRect().width;
 note.value='Synthetic writer selection retained through companion actions.';note.dispatchEvent(new Event('input',{bubbles:true}));note.focus();note.setSelectionRange(10,19,'backward');
 const selection=[note.selectionStart,note.selectionEnd,note.selectionDirection].join();
 press(root,'questions');await settle();
 assert(!root.querySelector('.live-questions-pane').hidden&&root.querySelector('.live-notes')===note&&[note.selectionStart,note.selectionEnd,note.selectionDirection].join()===selection,'Opening Ask changed the writer or its selection.');
 if(!wide)assert(root.classList.contains('live-overlay')&&primary.inert&&Math.abs(primary.getBoundingClientRect().width-width)<1,'The narrow companion squeezed or exposed the covered document.');
 root.querySelector('.meeting-companion-close').click();await settle();
 assert(companion.hidden&&!primary.inert&&document.activeElement===note,'Closing the companion did not restore the writer focus.');
 press(root,'transcript');await settle();
 if(fixture.options.live_captions){fixture.workspace.preview('me',{text:'Synthetic provisional line',chunk_start_ms:21000,start_ms:21000,end_ms:24000});await until(()=>root.querySelector('.live-caption-copy').textContent.includes('provisional line'),'The provisional line did not arrive.');assert(root.querySelector('.live-captions').closest('.live-transcript')&&root.querySelector('.live-caption-interim').textContent==='Live','Provisional text is not labelled Live inside the transcript.');}
 if(!wide){root.querySelector('.meeting-companion').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));await settle();assert(companion.hidden&&document.activeElement===note,'Escape did not close the overlay and restore the writer.');press(root,'transcript');}
 if(fixture.options.companion_enlarged){const sizes=[...root.querySelectorAll('*')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);for(const[node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}await new Promise(resolve=>setTimeout(resolve,600));}
 await new Promise(resolve=>setTimeout(resolve,250));
 const bounds=companion.getBoundingClientRect(),close=root.querySelector('.meeting-companion-close').getBoundingClientRect();
 assert(bounds.left>=0&&bounds.right<=innerWidth+1&&close.right<=innerWidth&&close.bottom<=innerHeight&&document.documentElement.scrollWidth<=innerWidth,'The companion or close action is outside the viewport.');
 if(fixture.options.companion_enlarged){root.querySelector(".meeting-companion-close").click();await settle();assert(root.querySelector(".meeting-document").clientHeight>40,"Closing the enlarged companion left no document viewport.");press(root,"transcript");await new Promise(resolve=>setTimeout(resolve,250));}
 return{defaultByViewport:true,singleSegmentedControl:true,writerRetained:true,overlay:!wide,closeFocus:true,liveInline:!!fixture.options.live_captions,syntheticOnly:true};
}
export async function companionResizeStart(){const root=await owned(),note=root.querySelector('.live-notes');assert(!root.querySelector('.meeting-companion').hidden&&innerWidth>=1100,'Start the real viewport test wide.');note.value='Writer retained through actual viewport changes.';note.dispatchEvent(new Event('input',{bubbles:true}));note.focus();note.setSelectionRange(2,12,'backward');press(root,'questions');window.__PENNEL_COMPANION_RESIZE__={root,note,value:note.value};}
export async function companionResizeNarrow(){const{root,note,value}=window.__PENNEL_COMPANION_RESIZE__;await until(()=>root.classList.contains('live-overlay'),'The real narrow viewport did not open the chosen companion as an overlay.');assert(root.querySelector('.live-notes')===note&&note.value===value&&note.selectionStart===2&&note.selectionEnd===12,'Viewport resizing changed writer state.');root.querySelector('.meeting-companion-close').click();assert(!root.classList.contains('live-companion-open'),'The narrow companion did not close.');}
export async function companionResizeWide(){const{root,note}=window.__PENNEL_COMPANION_RESIZE__;await until(()=>root.classList.contains('live-wide'),'The real viewport did not return wide.');assert(root.querySelector('.meeting-companion').hidden&&root.querySelector('.live-notes')===note,'Resizing reopened an explicitly closed companion or replaced the writer.');press(root,'transcript');assert(!root.querySelector('.live-pane-separator').hidden,'The wide split divider is unavailable.');const divider=root.querySelector('.live-pane-separator'),ratio=Number(divider.getAttribute('aria-valuenow')),before=root.querySelector('.meeting-primary-column').getBoundingClientRect().width,step=(root.querySelector('.live-workspace-panes').clientWidth-28)*.05;divider.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true,cancelable:true}));await settle();assert(Number(divider.getAttribute('aria-valuenow'))===ratio-5,'The retained split lost keyboard resizing.');const moved=before-root.querySelector('.meeting-primary-column').getBoundingClientRect().width;assert(Math.abs(moved-step)<12,'The split percentage did not match its resized default geometry.');return{actualViewportChanges:true,explicitChoiceRetained:true,writerRetained:true,keyboardSplit:true,syntheticOnly:true};}

export async function prepareCompanionKeyboardReopen() {
  const root = await owned();
  const pane = window.__PENNEL_FIXTURE__.options.companion_reopen_pane ?? "transcript";
  press(root, pane);
  root.querySelector(".meeting-companion-close").click();
  await until(() => root.querySelector(".meeting-companion").hidden, "The companion did not close before keyboard reopening.");
  const tab = root.querySelector(`[data-live-tab="${pane}"]`);
  tab.focus();
  assert(document.activeElement === tab, "The closed segmented control could not receive keyboard focus.");
  window.__PENNEL_COMPANION_KEYBOARD__ = { root, tab };
}
export async function checkCompanionKeyboardReopen() {
  const { root, tab } = window.__PENNEL_COMPANION_KEYBOARD__;
  assert(!root.querySelector(".meeting-companion").hidden && tab.getAttribute("aria-selected") === "true", "Enter did not reopen the selected companion.");
  assert(document.activeElement === tab, "Reopening moved the selected tab into a hidden companion and lost keyboard focus.");
  delete window.__PENNEL_COMPANION_KEYBOARD__;
  return { realEnterReopens: true, selectedTabRetainsFocus: true, syntheticOnly: true };
}
