import { selectMeetingPane } from "./meeting-workspace-test-controls.mjs";
const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let attempt=0;attempt<240;attempt++){if(await check())return;await new Promise(resolve=>setTimeout(resolve,25));}throw Error(message);}
const shown=node=>!!node?.getClientRects().length&&!node.closest('[hidden]');
async function owned(){
 await until(()=>document.querySelector('.summary-consent[data-mode="question"],.summary-consent[data-mode="quiet"]')&&!document.querySelector('.meeting-summary-editor').disabled,'The durable summary choice did not load.');
 const root=document.querySelector('.live-workspace');selectMeetingPane(root,'summary');
 return{root,card:root.querySelector('.summary-consent'),fixture:window.__PENNEL_FIXTURE__,api:(await import('/src/api.ts')).api};
}
const modelCalls=fixture=>fixture.calls.filter(command=>['summarize_meeting','generate_meeting_summary','test_provider_task','test_provider_task_choice'].includes(command));
export async function checkSummaryConsentCreate(){
 const{root,card,fixture,api}=await owned(),automatic=!!fixture.options.consent_automatic;
 assert(!modelCalls(fixture).length,'Rendering the summary question contacted a provider.');
 const info=await api.info(),before=await api.meetingSummary(root.dataset.meetingId);
 assert(card.querySelector('.summary-consent-card').textContent.includes(info.settings.llm.model),'The consent question omits its actual model destination.');
 const checkbox=card.querySelector('[data-summary-consent-automatic]');assert(!checkbox.checked,'The checkbox granted consent before an explicit choice.');if(automatic)checkbox.click();
 card.querySelector('[data-summary-consent="create"]').click();
 await until(()=>!root.querySelector('.summary-placeholders').hidden,'The actual summary request did not show working placeholders.');
 await until(()=>root.querySelector('.meeting-summary-surface').dataset.summaryState==='saved'&&!root.querySelector('.summary-placeholders').getClientRects().length,'The created summary did not return to saved reading mode.');
 const saved=await api.meetingSummary(root.dataset.meetingId);
 assert(saved.markdown.trim()&&!before.markdown.trim()&&root.querySelector('.meeting-summary-rendered').textContent.trim(),'The empty meeting did not receive its explicitly created summary.');
 assert((await api.info()).settings.summary_automation===(automatic?'automatic':'manual'),'Create did not persist exactly the displayed checkbox choice.');
 assert(fixture.calls.filter(command=>command==='summarize_meeting').length===1&&!fixture.calls.includes('generate_meeting_summary'),'Empty creation did not use one guarded direct path.');
 assert((await api.info()).settings.llm.provider_id===info.settings.llm.provider_id,'Consent silently selected a different provider.');
 if(fixture.options.reduced_motion)assert([...root.querySelectorAll('.meeting-summary-rendered .markdown > *')].every(node=>node.getAnimations().every(animation=>animation.effect.getKeyframes().every(frame=>!frame.transform||frame.transform==='none'))),'Reduced motion retained generated-section movement.');
 return{explicitPolicy:true,actualDestination:true,guardedEmptyCreate:true,realWorkingPlaceholders:true,syntheticOnly:true};
}
export async function dismissSummaryConsent(){
 const{root,card,fixture,api}=await owned();
 const notes=root.querySelector('.live-notes');selectMeetingPane(root,'notes');notes.applyUserChange('Synthetic private notes retained after Not now.');notes.setSelectionRange(3,13,'backward');selectMeetingPane(root,'summary');
 if(!fixture.options.configured_text_provider){assert(card.querySelector('[data-summary-consent="create"]').disabled&&shown(card.querySelector('.summary-consent-setup')),'Missing setup is not explained and repairable.');card.querySelector('.summary-consent-setup').click();await until(()=>document.querySelector('.task-setup-dialog [data-setup-cancel]'),'Summary setup did not open.');document.querySelector('.task-setup-dialog [data-setup-cancel]').click();await until(()=>!document.querySelector('.task-setup-dialog'),'Summary setup did not cancel back to the meeting.');}
 card.querySelector('[data-summary-consent="not_now"]').click();await until(()=>card.dataset.mode==='quiet','Not now did not reveal its durable quiet action.');
 assert((await api.info()).settings.summary_automation==='unanswered'&&await api.meetingSummaryPromptDismissed(root.dataset.meetingId),'Not now changed global consent or failed to persist per meeting.');
 assert(notes.value.includes('Synthetic private notes')&&notes.selectionStart===3&&notes.selectionEnd===13,'Not now lost the writer or its selection.');
 assert(!modelCalls(fixture).length&&!fixture.calls.includes('save_settings'),'Not now or setup cancellation started inference or changed policy.');
 return{durablePerMeeting:true,globalUnanswered:true,noProviderOnRenderOrDismiss:true,setupReachable:true,writerRetained:true,syntheticOnly:true};
}
export async function checkDismissedSummaryReopened(){
 const{root,card,fixture,api}=await owned();assert(card.dataset.mode==='quiet','Reopening forgot this meeting’s Not now choice.');
 assert((await api.info()).settings.summary_automation==='unanswered'&&await api.meetingSummaryPromptDismissed(root.dataset.meetingId),'Reopening changed global consent.');
 assert(!modelCalls(fixture).length&&!fixture.calls.includes('save_settings'),'Reopening a dismissed card contacted a provider or saved policy.');
 return{reopenedDurableChoice:true,noAutomaticAction:true,syntheticOnly:true};
}
export async function checkQuietSummaryCreate(){
 await dismissSummaryConsent();const{root,card,fixture,api}=await owned();
 card.querySelector('[data-summary-consent="quiet_create"]').click();await until(async()=>!!(await api.meetingSummary(root.dataset.meetingId)).markdown.trim(),'The quiet Create action did not generate once.');
 assert((await api.info()).settings.summary_automation==='unanswered'&&await api.meetingSummaryPromptDismissed(root.dataset.meetingId),'Quiet Create rewrote consent or the durable dismissal.');
 assert(fixture.calls.filter(command=>command==='summarize_meeting').length===1&&!fixture.calls.includes('save_settings'),'Quiet Create did not remain an explicit one-off action.');
 return{oneOffCreate:true,unansweredUnchanged:true,dismissalRetained:true,syntheticOnly:true};
}
export async function checkSummaryConsentCandidate(){
 const{root,card,fixture,api}=await owned();const before=await api.meetingSummary(root.dataset.meetingId);
 card.querySelector('[data-summary-consent="create"]').click();await until(()=>!root.querySelector('.meeting-summary-candidate').hidden,'A replacement was not offered for review.');
 assert((await api.meetingSummary(root.dataset.meetingId)).markdown===before.markdown&&!fixture.calls.includes('summarize_meeting'),'Consent overwrote the existing summary directly.');
 assert(fixture.calls.filter(command=>command==='generate_meeting_summary').length===1,'The existing summary did not use the guarded candidate route.');
 return{candidatePreservesSavedBody:true,explicitReviewRetained:true,syntheticOnly:true};
}
export async function checkConsentLateSummaryRead(){
 const{root,card,fixture}=await owned(),client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),id=root.dataset.meetingId;
 let reachedModel=false,releaseModel,releaseRead,holdRead=false,held=false;
 client.invoke=async(command,args)=>{
  if(command==='summarize_meeting'){reachedModel=true;await new Promise(resolve=>{releaseModel=resolve;});}
  const value=await invoke(command,args);
  if(command==='meeting_summary'&&holdRead&&!held&&!value.markdown.trim()){held=true;await new Promise(resolve=>{releaseRead=resolve;});}
  return value;
 };
 try{
  card.querySelector('[data-summary-consent="create"]').click();await until(()=>reachedModel,'The direct creation request did not reach its admission.');
  holdRead=true;fixture.emit('meeting-updated',id);await until(()=>releaseRead,'The old empty summary read was not held.');releaseModel();
  await until(async()=>!!(await invoke('meeting_summary',{id})).markdown.trim(),'The native-like create did not commit before releasing the old read.');
  releaseRead();await until(()=>root.querySelector('.meeting-summary-surface').dataset.summaryState!=='generating','The creation action did not settle.');
  assert(root.querySelector('.meeting-summary-editor').value.trim()&&root.querySelector('.meeting-summary-rendered').textContent.includes('Besluiten'),'A late pre-write read hid the acknowledged created summary.');
  return{preWriteReadDiscarded:true,createdBodyRetained:true,syntheticOnly:true};
 }finally{releaseModel?.();releaseRead?.();client.invoke=invoke;}
}

export async function checkSummaryConsentFailure() {
 const{root,card,fixture,api}=await owned(),client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),metadata=fixture.options.consent_failure==='metadata';let reject=true;
 client.invoke=async(command,args)=>{if(reject&&command===(metadata?'dismiss_meeting_summary_prompt':'save_settings'))throw Error(metadata?'Synthetic metadata save unavailable.':'Synthetic policy save unavailable.');return invoke(command,args);};
 try{
  const action=card.querySelector(`[data-summary-consent="${metadata?'not_now':'create'}"]`);action.focus();action.click();
  await until(()=>card.querySelector('.error').textContent.includes(metadata?'metadata save':'policy save'),'The choice failure lost its actual reason.');
  assert(!action.disabled&&(await api.info()).settings.summary_automation==='unanswered'&&!modelCalls(fixture).length,'A failed choice disabled retry, granted policy or started inference.');
  assert(!await api.meetingSummaryPromptDismissed(root.dataset.meetingId),'Failed metadata was presented as durable dismissal.');
  reject=false;action.click();
  if(metadata)await until(()=>card.dataset.mode==='quiet','Metadata retry did not persist the quiet choice.');
  else await until(async()=>!!(await api.meetingSummary(root.dataset.meetingId)).markdown.trim(),'The policy retry did not create the summary.');
  return{actualFailureVisible:true,retryReachable:true,noInferenceBeforePolicyAck:true,syntheticOnly:true};
 }finally{client.invoke=invoke;}
}
export async function checkSummaryConsentEnlarged() {
 const{root,card,fixture}=await owned(),note=root.querySelector('.live-notes');
 const sizes=[...root.querySelectorAll('*')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);
 for(const[node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}
 for(const selector of ['[data-summary-consent="create"]','[data-summary-consent="not_now"]']){
  const button=card.querySelector(selector);button.scrollIntoView({block:'center'});await new Promise(resolve=>requestAnimationFrame(resolve));const bounds=button.getBoundingClientRect();
  assert(bounds.left>=0&&bounds.right<=innerWidth&&bounds.top>=0&&bounds.bottom<=innerHeight,'An enlarged consent action is clipped.');
 }
 assert(root.querySelector('.live-notes')===note&&document.documentElement.scrollWidth<=innerWidth&&!modelCalls(fixture).length,'Enlarged layout replaced the writer, overflowed or started inference.');
 return{enlargedActionsReachable:true,writerRetained:true,readOnlyRender:true,syntheticOnly:true};
}
export async function showSummaryProcessing() {
 const root=document.querySelector('.live-workspace'),fixture=window.__PENNEL_FIXTURE__,api=(await import('/src/api.ts')).api;
 await until(()=>root&&!root.querySelector('.meeting-summary-editor').disabled,'The summary view did not load.');
 const uid=(await api.meetingSummary(root.dataset.meetingId)).meeting_uid;fixture.emit('meeting-postprocess',{id:root.dataset.meetingId,meeting_uid:uid,operation_id:'progress-current',stage:'summary',state:'started'});
 await until(()=>!root.querySelector('.summary-placeholders').hidden,'Native-like in-progress metadata did not show placeholders.');
 fixture.emit('meeting-postprocess',{id:root.dataset.meetingId,meeting_uid:uid,operation_id:'progress-old',stage:'summary',state:'finished'});
 assert(!root.querySelector('.summary-placeholders').hidden&&!modelCalls(fixture).length,'Stale completion hid progress or presentation started inference.');
 return{metadataOnlyProgress:true,staleCompletionRejected:true,noProviderOnRender:true,syntheticOnly:true};
}

export async function checkLatePromptReadAfterDismissal() {
 const{root,fixture,api}=await owned(),client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),id=root.dataset.meetingId;
 const{meetingSummaryConsent}=await import('/src/meeting-summary-consent.ts');
 // A stable production owner isolates the native acknowledgement from later workspace notifications.
 const context={info:await api.info(),onInfoChange(){},onDispose(){},openSetup(){},save(){throw Error('Unintended policy write.');},mutate(){throw Error('Unintended policy mutation.');}};
 const view=meetingSummaryConsent(context,{id:()=>id,finished:()=>true,working:()=>false,canGenerate:()=>true,canDismiss:()=>true,blockedReason:()=>'',focusTarget:root.querySelector('.meeting-summary-surface'),generate:async()=>{throw Error('Unintended inference.');},dismiss:async()=>{const before=await api.meetingSummary(id);await api.dismissMeetingSummaryPrompt(id,before.revision,before.source_revision);}});
 root.querySelector('.summary-consent').replaceWith(view.root);view.bind(context);await until(()=>view.root.dataset.mode==='question','The production consent owner did not prime its loaded question.');
 let releaseRead,held=false;
 client.invoke=async(command,args)=>{const value=await invoke(command,args);if(command==='meeting_summary_prompt_dismissed'&&!held){held=true;await new Promise(resolve=>{releaseRead=resolve;});if(fixture.options.prompt_late_error)throw Error('Synthetic superseded prompt read error.');}return value;};
 try{
  view.bind(context);await until(()=>releaseRead,'The old false prompt read was not held.');
  view.root.querySelector('[data-summary-consent="not_now"]').click();await until(()=>view.root.dataset.mode==='quiet','Dismissal B did not acknowledge before releasing read A.');
  releaseRead();await new Promise(resolve=>setTimeout(resolve,100));
  assert(view.root.dataset.mode==='quiet'&&view.root.querySelector('.error').hidden,'An older prompt read or its error replaced the durable dismissal acknowledgement.');
  assert(await api.meetingSummaryPromptDismissed(id)&&(await api.info()).settings.summary_automation==='unanswered','The durable dismissal or global unanswered consent changed.');
  assert(!modelCalls(fixture).length&&!fixture.calls.includes('save_settings'),'A stale prompt read started a provider or changed policy.');
  return{latePromptValueOrErrorRejected:true,durableQuietChoiceRetained:true,noInferenceOrPolicy:true,syntheticOnly:true};
 }finally{releaseRead?.();client.invoke=invoke;}
}
