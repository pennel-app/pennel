const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let i=0;i<200;i++){if(await check())return;await new Promise(r=>setTimeout(r,25));}throw Error(message);}
const visible=node=>!!node?.getClientRects().length&&!node.closest('[hidden]');
async function owned(){assert(window.__PENNEL_FIXTURE__,'Use the owned synthetic fixture.');await until(()=>document.querySelector('.meeting-summary-editor:not(:disabled)'),'Summary did not load.');return document.querySelector('.live-workspace');}
export async function checkStructuredSummary(){
 const root=await owned(),surface=root.querySelector('.meeting-summary-surface'),reader=root.querySelector('.meeting-summary-structured');
 assert(visible(reader),'Attached structure did not replace the Markdown-only reader.');
 const {api}=await import('/src/api.ts'),client=window.__PENNEL_PLATFORM__,original=client.invoke.bind(client),calls=[];let generatedRecord;
 client.invoke=async(command,args={})=>{calls.push({command,args});const result=await original(command,args);if(command==='generate_meeting_summary')generatedRecord=result.record;return result;};
 try{
 const before=await api.meetingSummary(window.__PENNEL_FIXTURE__.options.meeting_id),body=before.markdown;
 const length=surface.querySelector('.meeting-summary-length');
 assert(length.value==='standard'&&visible(reader.querySelector('.structured-summary-open')),'Standard does not retain Still open by default.');
 const invokeCount=calls.length;length.querySelector('[data-value="brief"]').click();
 assert(reader.querySelector('.structured-summary-topics').hidden&&reader.querySelector('.structured-summary-open').hidden,'Brief filtering is incorrect.');
 length.querySelector('[data-value="detailed"]').click();
 await until(()=>reader.querySelectorAll('.summary-map-lane').length===3,'Detailed did not read actual speaker timeline lanes.');
 assert((await api.meetingSummary(window.__PENNEL_FIXTURE__.options.meeting_id)).markdown===body,'Length changed canonical Markdown.');
 assert(!calls.slice(invokeCount).some(call=>/generate|summarize|save_|accept_|update_meeting_summary/.test(call.command)),'Display length made a provider or body mutation call.');
 const doc=root.querySelector('.meeting-document');doc.scrollTop=120;const scroll=doc.scrollTop;
 const source=reader.querySelector('.summary-map-topics button');assert(!source.disabled,'Validated topic source is unavailable.');source.click();
 await until(()=>visible(root.querySelector('.live-source-back')),'Topic did not open its real transcript source.');
 assert(root.querySelector('.live-source-highlight'),'Topic did not highlight its source utterance.');root.querySelector('.live-source-back').click();
 await until(()=>document.activeElement===source,'Returning from the transcript lost the originating topic control.');
 await until(()=>source.tabIndex>=0,'Returning from source removed the topic from keyboard navigation.');
 assert(Math.abs(doc.scrollTop-scroll)<2,'Returning from transcript lost the summary reading position.');
 const action=reader.querySelector('.structured-action'),checkbox=action.querySelector('input'),owner=action.querySelector('.structured-action-owner');
 checkbox.focus();checkbox.click();
 await until(async()=>(await api.meetingSummary(window.__PENNEL_FIXTURE__.options.meeting_id)).record?.structure.actions[0].state.done,'Tick did not persist through the real action API.');
 await until(()=>!checkbox.disabled,'Action write did not finish.');
 assert(owner.textContent==='Sam'&&checkbox.isConnected&&document.activeElement===checkbox,'Tick lost its other owner field, identity or focus.');
 owner.click();await until(()=>document.querySelector('dialog[open] input'),'Owner editing did not open.');
 const dialog=document.querySelector('dialog[open]');dialog.querySelector('input').value='Noor';dialog.querySelector('button[type=submit]').click();
 await until(()=>owner.textContent==='Noor'&&!owner.disabled,'Owner did not persist.');
 assert(document.activeElement===owner,'Saving an owner lost its originating control focus.');
 const saved=await api.meetingSummary(window.__PENNEL_FIXTURE__.options.meeting_id);
 assert(saved.record.structure.actions[0].state.done&&saved.edited===before.edited,'Owner update lost the done field or changed generated/edit semantics.');
 const info=await api.info();await api.saveSettings({...info.settings,llm:{...info.settings.llm,model:'different-current-setting'}});
 await until(()=>root.querySelector('.summary-provenance-chip')?.textContent.includes('frozen-transcript-model'),'Validated transcript provenance is missing.');
 assert(root.querySelector('.meeting-summary-provenance').textContent.includes('frozen-summary-model')&&!root.querySelector('.meeting-summary-provenance').textContent.includes('different-current-setting'),'Provenance follows current settings rather than the generation record.');
 const {t}=await import('/src/i18n.ts');
 assert(!root.querySelector('.meeting-summary-provenance').textContent.includes(t('dictation.processing.local')),'Synced frozen provenance claims an unrecorded current-device identity.');
 surface.querySelector('.meeting-summary-generate').click();await until(()=>visible(surface.querySelector('.meeting-summary-candidate')),'Generated candidate was not staged.');
 surface.querySelector('.meeting-summary-accept').click();await until(()=>!visible(surface.querySelector('.meeting-summary-candidate')),'Candidate acceptance did not finish.');
 const acceptance=calls.findLast(call=>call.command==='accept_meeting_summary');
 assert(acceptance?.args.record&&JSON.stringify(acceptance.args.record)===JSON.stringify(generatedRecord),'Acceptance discarded candidate.record.');
 assert(visible(reader),'Accepted structured candidate fell back to plain Markdown.');
 const accepted=await api.meetingSummary(window.__PENNEL_FIXTURE__.options.meeting_id);
 assert(accepted.record.structure.actions[0].state.done&&accepted.record.structure.actions[0].state.owner==='Noor','Candidate round-trip lost the carried action state.');
 localStorage.setItem('pennel.summary.length.v1','brief');window.__PENNEL_FIXTURE__.emit('settings-changed',null);
 await until(()=>length.value==='brief','The retained reader ignored the shared per-device length preference.');
 localStorage.setItem('pennel.summary.length.v1','detailed');window.__PENNEL_FIXTURE__.emit('settings-changed',null);
 await until(()=>length.value==='detailed','The shared length preference did not restore.');
 assert(!calls.some(call=>['set_key','start_meeting','toggle_dictation','copy_text'].includes(call.command)),'Summary reading triggered an unrelated action.');
 }finally{client.invoke=original;}
}
export async function checkSummaryFallback(){const root=await owned();assert(!visible(root.querySelector('.meeting-summary-structured'))&&visible(root.querySelector('.meeting-summary-markdown .markdown')),'Missing/detached/corrupt structure did not preserve canonical Markdown.');assert(root.querySelector('.meeting-summary-length').hidden,'Fallback exposed a nonfunctional length control.');}
export async function checkSummarySourceRace(){
 const root=await owned();await until(()=>root.querySelector('.summary-map-topics button:not(:disabled)'),'Detailed topic not ready.');
 const source=root.querySelector('.summary-map-topics button'),digest=crypto.subtle.digest.bind(crypto.subtle);let release,paused=false,pauseNext=false;
 crypto.subtle.digest=async(...args)=>{if(pauseNext){pauseNext=false;paused=true;await new Promise(resolve=>{release=resolve;});}return digest(...args);};
 try{
   pauseNext=true;source.focus();source.click();await until(()=>paused,'Source fingerprint did not start.');
   root.querySelector('[data-live-tab="questions"]').click();release();await new Promise(r=>setTimeout(r,150));
   assert(root.querySelector('[data-live-tab="questions"]').getAttribute('aria-selected')==='true','Late source hashing replaced the newer Questions choice.');
   root.querySelector('.meeting-companion-close').click();
   const {api}=await import('/src/api.ts'),detail=await api.meetingDetail(window.__PENNEL_FIXTURE__.options.meeting_id);
   paused=false;pauseNext=true;source.focus();source.click();await until(()=>paused,'Second source fingerprint did not start.');
   detail.transcript[0].text='Changed actual synthetic transcript source.';window.__PENNEL_FIXTURE__.workspace.replaceTranscript(detail.transcript);
   await until(()=>root.querySelector('.live-transcript').textContent.includes('Changed actual synthetic transcript source.'),'Current transcript did not refresh.');
   release();await until(()=>visible(root.querySelector('.live-source-notice')),'Stale source has no visible notice in the originating summary.');
   assert(root.querySelector('.meeting-summary-surface').contains(root.querySelector('.live-source-notice')),'Stale summary source notice escaped into a hidden Questions panel.');
 }finally{if(release)release();crypto.subtle.digest=digest;}
}
export async function checkSummaryReducedMotion(){const root=await owned(),section=root.querySelector('.structured-summary-lead');assert(getComputedStyle(section).animationName==='structured-summary-fades','Reduced motion removed the fade or retained movement.');for(const animation of section.getAnimations())assert(animation.effect.getKeyframes().every(frame=>!frame.transform||frame.transform==='none'),'Reduced summary reveal still moves.');}
export async function checkTimelineRecovery(){const root=await owned();await until(()=>root.querySelector('.summary-map-status').textContent.includes(document.documentElement.lang==='nl'?'kon niet':'could not'),'Timeline read failure is not explained.');window.__PENNEL_FIXTURE_OPTIONS__.summary_timeline_error=false;root.querySelector('.structured-summary-map > button').click();await until(()=>root.querySelectorAll('.summary-map-lane').length===3,'Timeline retry did not recover.');assert(visible(root.querySelector('.meeting-summary-structured')),'Timeline failure discarded the valid structured body.');}
