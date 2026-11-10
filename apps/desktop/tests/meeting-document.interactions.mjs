import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
const assert = (condition, message) => { if (!condition) throw Error(message); };
async function until(check, message) { for (let attempt=0;attempt<200;attempt++) { if (await check()) return; await new Promise(resolve=>setTimeout(resolve,25)); } throw Error(message); }
const settle = () => new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
export async function checkMeetingDocument() {
  const fixture = window.__PENNEL_FIXTURE__;
  const { api } = await import("/src/api.ts"), { t } = await import("/src/i18n.ts");
  await until(()=>document.querySelector('.live-workspace[data-document-layout="continuous"] .meeting-summary-rendered') && !document.querySelector('.meeting-summary-editor').disabled,"The finished document did not load.");
  const root=document.querySelector('.live-workspace'), doc=root.querySelector('.meeting-document'), notes=root.querySelector('.live-notes');
  assert(!root.querySelector('.live-summary-pane').hidden && !root.querySelector('.live-notes-pane').hidden,"Summary and notes are still alternative tabs.");
  assert(doc.children[0].dataset.livePane==='summary' && doc.children[1].dataset.livePane==='notes',"The shared document lost Summary-before-notes order.");
  assert(root.querySelector('.meeting-writing-tabs').hidden && root.querySelector('.meeting-notes-heading').textContent.includes(t('meetings.notes_private_share')),"Finished review lost its private-notes label or retained writing tabs.");
  assert(root.querySelector('.meeting-header-date').textContent && root.querySelectorAll('.meeting-person-chip').length>0,"Finished metadata or people chips are missing.");
  assert(root.querySelector('.meeting-export-action').classList.contains('primary') && root.querySelector('.meeting-export-action').textContent===t('meetings.share'),"Share is not the primary finished-header action.");
  assert(root.querySelector('.meeting-transcript-provenance').hidden,"A reopened meeting inferred transcript provenance from current setup.");
  selectMeetingPane(root,'notes');
  notes.value='Synthetic private document draft.\n'+Array.from({length:18},(_,i)=>`Retained note line ${i}`).join('\n'); notes.dispatchEvent(new Event('input',{bubbles:true})); notes.focus(); notes.setSelectionRange(10,19,'backward');
  const selection=[notes.selectionStart,notes.selectionEnd,notes.selectionDirection];
  fixture.emit('settings-changed',null); await settle();
  assert(root.querySelector('.live-notes')===notes && document.activeElement===notes && selection.join() === [notes.selectionStart,notes.selectionEnd,notes.selectionDirection].join(),"A refresh replaced the writer or lost selection/focus.");
  await import('/src/views/meetings.ts').then(module=>module.flushMeetingDrafts());
  assert((await api.meetingDetail(root.dataset.meetingId)).notes===notes.value,"The single document lost durable notes autosave.");
  selectMeetingPane(root,'summary'); doc.scrollTop=Math.min(100,doc.scrollHeight-doc.clientHeight); const scroll=doc.scrollTop;
  root.querySelector('.meeting-export-action').click(); await until(()=>document.querySelector('.meeting-export-dialog[open] .meeting-export-preview:not([hidden])'),"Share did not open its export dialog.");
  document.querySelector('.meeting-export-dialog .meeting-export-cancel').click(); await settle();
  assert(!doc.hidden && doc.scrollTop===scroll && root.querySelector('.live-notes')===notes,"Returning from Share lost the document position or writer.");
  if(fixture.options.document_enlarged) {
    const sizes=[...root.querySelectorAll('*')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);
    for(const[node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}
    await settle();
  }
  assert(doc.clientHeight>40 && document.documentElement.scrollWidth<=innerWidth,"The document or header overflows the viewport.");
  doc.scrollTop=0; await settle();
  return {singleDocument:true,privateNotes:true,header:true,retainedWriter:true,autosave:true,shareReturn:true,syntheticOnly:true};
}
