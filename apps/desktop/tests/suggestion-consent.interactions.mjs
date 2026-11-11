const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let attempt=0;attempt<300;attempt++){if(await check())return;await new Promise(resolve=>setTimeout(resolve,25));}throw Error(message);}
async function owned(){
 await until(()=>document.querySelector('.insight-review .suggestion-consent'),'Review consent did not mount.');
 const root=document.querySelector('.live-workspace'),review=root.querySelector('.insight-review'),card=review.querySelector('.suggestion-consent'),api=(await import('/src/api.ts')).api,fixture=window.__PENNEL_FIXTURE__;
 const tab=root.querySelector('[data-live-tab="suggestions"]');if(tab.getAttribute('aria-selected')!=='true')tab.click();
 await until(()=>card.dataset.mode!=='loading','Suggestion qualification/dismissal reads did not settle.');
 return{root,review,card,api,fixture};
}
async function chooseLanguage(review,key){
 const{t}=await import('/src/i18n.ts');review.querySelector('.insight-analysis-details').open=true;
 review.querySelector('.insight-analysis-details .insight-language .choice').click();await until(()=>[...document.querySelectorAll('[role="option"]')].some(node=>node.textContent===t(key)),'The explicit language menu did not open.');
 [...document.querySelectorAll('[role="option"]')].find(node=>node.textContent===t(key)).click();
}
const inference=fixture=>fixture.calls.filter(command=>['begin_insights_analysis','test_inference','set_key','download_model'].includes(command));
const policyMutations=fixture=>fixture.calls.filter(command=>['save_settings','set_meeting_suggestion_policy_checked'].includes(command));
export async function checkSuggestionChoice(){
 const{root,review,card,api,fixture}=await owned(),before=(await api.info()).settings;
 assert(!inference(fixture).length&&!policyMutations(fixture).length,'Reading suggestion options started inference, credentials or policy.');
 if(fixture.options.suggestion_choose_language){assert(card.dataset.mode==='unavailable','Unknown speech language silently selected a policy.');await chooseLanguage(review,'insights.language_nl');}
 await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="find"]').disabled,'The exact qualified synthetic setup did not enable the question.');
 assert(card.textContent.includes('decision.example.invalid')&&card.textContent.includes('owned-ui-fixture'),'The question omits its actual processing destination.');
 const checkbox=card.querySelector('[data-suggestion-consent-automatic]');assert(!checkbox.checked,'Automatic suggestions were granted before a choice.');if(fixture.options.suggestion_choose_automatic)checkbox.click();
 card.querySelector('[data-suggestion-consent="find"]').click();await until(()=>fixture.calls.includes('begin_insights_analysis'),'The acknowledged choice did not start the existing manual analysis path.');
 assert(fixture.calls.filter(command=>command==='set_meeting_suggestion_policy_checked').length===1&&!fixture.calls.includes('save_settings'),'The choice did not use one scoped policy write.');
 const saved=(await api.info()).settings;assert(saved.suggestion_automation===(fixture.options.suggestion_choose_automatic?'automatic':'manual')&&saved.summary_automation===before.summary_automation,'Suggestion consent rewrote summary policy or lost the checkbox mapping.');
 assert(saved.suggestion_language===(fixture.options.suggestion_choose_automatic&&fixture.options.suggestion_choose_language?'nl':before.suggestion_language),'One-off/manual language selection was persisted as an automatic fallback.');
 assert(root.querySelector('.live-notes'),'The choice lost the retained writer.');
 return{separateAcknowledgedConsent:true,exactDestination:true,explicitLanguageFallbackOnly:true,manualPathOnce:true,syntheticOnly:true};
}
export async function dismissSuggestionChoice(){
 const{root,card,api,fixture}=await owned();await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="not_now"]').disabled,'Not now did not become reachable.').catch(error=>{throw Error(error.message+JSON.stringify({mode:card.dataset.mode,block:card.querySelector('.suggestion-consent-block').textContent,state:root.querySelector('.meeting-summary-surface').dataset.summaryState,error:root.querySelector('.meeting-summary-error').textContent}));});
 const before=await api.meetingSummary(root.dataset.meetingId),reading=root.querySelector('.meeting-summary-rendered'),structured=reading.querySelector('.meeting-summary-structured');
 card.querySelector('[data-suggestion-consent="not_now"]').click();await until(()=>card.dataset.mode==='quiet','Not now did not acknowledge its durable quiet state.');
 const saved=await api.meetingSummary(root.dataset.meetingId),settings=(await api.info()).settings;
 assert(await api.meetingSuggestionPromptDismissed(root.dataset.meetingId)&&!await api.meetingSummaryPromptDismissed(root.dataset.meetingId),'The separate dismissal flags were conflated.');
 assert(settings.suggestion_automation==='unanswered'&&settings.summary_automation==='manual'&&!inference(fixture).length&&!policyMutations(fixture).length,'Dismissal changed consent or started inference.');
 assert(saved.markdown===before.markdown&&JSON.stringify(saved.record)===JSON.stringify(before.record)&&root.querySelector('.meeting-summary-rendered')===reading&&reading.querySelector('.meeting-summary-structured')===structured,'Suggestion metadata replaced the saved summary, record or retained reading DOM.');
 return{durableIndependentDismissal:true,summaryAndReadingRetained:true,noPolicyOrInference:true,syntheticOnly:true};
}
export async function checkSuggestionReopened(){
 const{root,card,api,fixture}=await owned();assert(card.dataset.mode==='quiet'&&await api.meetingSuggestionPromptDismissed(root.dataset.meetingId),'Reopening forgot suggestion Not now.');
 assert((await api.info()).settings.suggestion_automation==='unanswered'&&!inference(fixture).length&&!policyMutations(fixture).length,'Reopening the quiet choice performed work.');
 return{reopenedQuietChoice:true,noAutomaticAction:true,syntheticOnly:true};
}
export async function checkSuggestionQuietFind(){
 await dismissSuggestionChoice();const{card,api,fixture}=await owned();card.querySelector('[data-suggestion-consent="one_off"]').click();await until(()=>fixture.calls.includes('begin_insights_analysis'),'Quiet Find did not start once.');
 assert((await api.info()).settings.suggestion_automation==='unanswered'&&!policyMutations(fixture).length,'Quiet Find changed global policy.');
 return{oneOffAnalysis:true,unansweredRetained:true,syntheticOnly:true};
}
export async function checkSuggestionUnavailable(){
 const{review,card,fixture}=await owned();assert(card.dataset.mode==='unavailable'&&card.querySelector('.summary-consent-card').hidden,'An unqualified setup exposed the consent question.');
 assert(card.querySelector('.suggestion-consent-status').textContent.trim()&&!inference(fixture).length&&!policyMutations(fixture).length,'Unavailable setup lost its reason or caused work.');
 assert(review.querySelector('.insight-analysis-details').open&&!review.querySelector('.insight-connection-settings').hidden,'Unavailable connection setup is not reachable.');
 assert(review.querySelector('.insight-availability').hidden,'Unavailable Review repeated its primary reason in Analysis details.');
 assert(review.querySelector('.insight-empty').hidden&&review.querySelector('.insight-coverage').hidden,'Unavailable Review showed a competing empty state or warned about analysis that never ran.');
 if(!fixture.options.suggestion_configured)assert(review.querySelector('.insight-destination').hidden,'Missing connection details rendered Not configured placeholders.');
 return{exactQualificationGateRetained:true,oneReasonAndSetup:true,noPrematureCoverage:true,noInference:true,syntheticOnly:true};
}
export async function checkSuggestionFailure(){
 const{card,fixture,api}=await owned();await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="not_now"]').disabled,'The question is not ready.');
 const client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),metadata=fixture.options.suggestion_failure==='metadata';let reject=true;
 client.invoke=async(command,args)=>{if(reject&&command===(metadata?'dismiss_meeting_suggestion_prompt':'set_meeting_suggestion_policy_checked'))throw Error('Synthetic suggestion choice unavailable.');return invoke(command,args);};
 try{
  const action=card.querySelector(`[data-suggestion-consent="${metadata?'not_now':'find'}"]`);action.focus();action.click();await until(()=>card.querySelector('.suggestion-consent-error').textContent.includes('Synthetic suggestion choice unavailable'),'The actual failure was lost.');
  assert(!action.disabled&&(await api.info()).settings.suggestion_automation==='unanswered'&&!inference(fixture).length,'A failed choice disabled retry or started inference.');
  reject=false;action.click();await until(()=>metadata?card.dataset.mode==='quiet':fixture.calls.includes('begin_insights_analysis'),'The failed choice did not recover through its own action.');
  return{actualErrorVisible:true,retryReachable:true,noInferenceBeforeConsentAck:true,syntheticOnly:true};
 }finally{client.invoke=invoke;}
}
export async function checkSuggestionAttemptState(){
 const{review,card,fixture}=await owned(),expected=fixture.options.suggestion_expected_mode;
 assert(card.dataset.mode===expected&&card.querySelector('.suggestion-consent-status').textContent.trim(),'The automatic attempt state is not explained.');
 assert(!inference(fixture).length&&!policyMutations(fixture).length,'An attempt state retried or changed consent while rendering.');
 if(fixture.options.insights_seeded)assert(review.querySelectorAll('.insight-card').length,'Receipt damage hid existing review cards.');
 return{attemptStateExplained:true,existingCardsRetained:true,noAutomaticRetry:true,syntheticOnly:true};
}
export async function showSuggestionProgress(){
 const{root,review,card,api,fixture}=await owned(),uid=(await api.meetingSummary(root.dataset.meetingId)).meeting_uid;
 const emit=(stage,operation,state)=>fixture.emit('meeting-postprocess',{id:root.dataset.meetingId,meeting_uid:uid,operation_id:operation,stage,state});
 emit('postprocess','whole','started');await until(()=>card.dataset.mode==='waiting_summary','The pre-analysis processing span looked like a skipped run.');assert(review.querySelector('.insight-find').disabled,'Preparation still allowed competing Find work.');
 emit('suggestions','current','started');await until(()=>card.dataset.mode==='running','The actual suggestion stage did not show progress.');emit('suggestions','old','finished');
 assert(card.dataset.mode==='running'&&review.querySelector('.insight-find').disabled&&review.querySelector('.insight-cancel').hidden&&!inference(fixture).length,'Stale completion hid progress, a fake job got Cancel, or rendering started inference.');
 return{wholePreparationThenAnalysis:true,staleTerminalRejected:true,noFakeCancel:true,syntheticOnly:true};
}
export async function checkSuggestionSettings(){
 const fixture=window.__PENNEL_FIXTURE__,api=(await import('/src/api.ts')).api;
 await until(()=>document.querySelector('.meeting-suggestion-settings'),'The separate Settings control did not mount.');
 assert(!fixture.calls.includes('inference_availability')&&!inference(fixture).length,'Rendering suggestion Settings used a network-capable provider check.');
 const before=(await api.info()).settings.summary_automation;
 document.querySelector('button[data-suggestion-automation="automatic"]').click();await until(async()=>(await api.info()).settings.suggestion_automation==='automatic','Settings did not save explicit Automatic.');
 await until(()=>document.querySelector('.suggestion-settings-language .choice'),'Settings did not return its fallback control.');
 document.querySelector('.suggestion-settings-language .choice').click();const{t}=await import('/src/i18n.ts');await until(()=>[...document.querySelectorAll('[role="option"]')].some(node=>node.textContent===t('insights.language_nl')),'The fallback language menu did not open.');
 [...document.querySelectorAll('[role="option"]')].find(node=>node.textContent===t('insights.language_nl')).click();await until(async()=>(await api.info()).settings.suggestion_language==='nl','The explicit fallback was not saved.');
 await until(()=>document.querySelector('button[data-suggestion-automation="manual"]'),'Settings did not retain its policy control.');document.querySelector('button[data-suggestion-automation="manual"]').click();await until(async()=>(await api.info()).settings.suggestion_automation==='manual','Settings did not save explicit Manual.');
 await until(()=>document.querySelector('button[data-suggestion-automation="manual"]').getAttribute('aria-pressed')==='true','The acknowledged Manual choice was not reflected in Settings.');
 const selected=document.querySelector('button[data-suggestion-automation="manual"]'),other=document.querySelector('button[data-suggestion-automation="automatic"]');
 assert(getComputedStyle(selected).backgroundColor!==getComputedStyle(other).backgroundColor,'The saved suggestion policy has no visible selected state.');
 assert((await api.info()).settings.summary_automation===before&&!inference(fixture).length,'Suggestion Settings changed summary consent or started analysis.');
 return{separatePolicyAndFallback:true,summaryPolicyRetained:true,noProviderCheckOnRender:true,syntheticOnly:true};
}
export async function checkSuggestionLatePrompt(){
 const{root,card,api,fixture}=await owned(),id=root.dataset.meetingId,client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),snapshot=await api.insightsSnapshot(id),context={info:await api.info(),onInfoChange(){},onDispose(){},mutate(){throw Error('Unintended policy mutation.');}};
 const{meetingSuggestionConsent}=await import('/src/meeting-suggestion-consent.ts');
 let currentLanguage='en';
 const view=meetingSuggestionConsent(context,{id:()=>id,uid:snapshot.meeting_uid,snapshot:()=>snapshot,language:()=>currentLanguage,analysisBusy:()=>false,canAnalyze:()=>true,chooseLanguage:async()=>{},start:async()=>{throw Error('Unintended inference.');},dismiss:async()=>{const before=await api.meetingSummary(id);await api.dismissMeetingSuggestionPrompt(id,before.revision,before.source_revision);},canDismiss:()=>true,blockedReason:()=>'',reason:code=>code,changed(){},focusTarget:root.querySelector('.insight-review'),languageControlVisible:()=>false});
 card.replaceWith(view.root);view.bind(context);await until(()=>view.root.dataset.mode==='consent','The production suggestion owner did not prime its question.');
 if(fixture.options.suggestion_external_dismissal){const before=await api.meetingSummary(id);await api.dismissMeetingSuggestionPrompt(id,before.revision,before.source_revision);await new Promise(resolve=>setTimeout(resolve,3100));view.refresh();await until(()=>view.root.dataset.mode==='quiet','The periodic status refresh did not observe an external durable dismissal.');assert(!inference(fixture).length&&!policyMutations(fixture).length,'Refreshing durable status performed work.');return{externalDismissalObserved:true,readOnlyRefresh:true,syntheticOnly:true};}
 if(fixture.options.suggestion_read_retry){let reject=true;client.invoke=async(command,args)=>{if(reject&&command==='automatic_insights_availability'&&args.languagePolicy==='nl')throw Error('Synthetic status read unavailable.');return invoke(command,args);};try{currentLanguage='nl';view.refresh();await until(()=>view.root.querySelector('.suggestion-consent-error').textContent.includes('Synthetic status read unavailable.'),'The changed-language read did not report its failure.');const retry=view.root.querySelector(':scope > .btn');assert(!retry.hidden&&!retry.disabled,'A failed replacement status read has no reachable Retry.');reject=false;retry.click();await until(()=>view.root.dataset.mode==='consent'&&view.root.querySelector('.suggestion-consent-error').hidden,'Status Retry did not restore its qualified question.');assert(!inference(fixture).length&&!policyMutations(fixture).length,'Retrying a read performed work.');return{readFailureRetryReachable:true,queryRestoredWithoutWork:true,syntheticOnly:true};}finally{client.invoke=invoke;}}
 let release,held=false;
 client.invoke=async(command,args)=>{const value=await invoke(command,args);if(command==='meeting_suggestion_prompt_dismissed'&&!held){held=true;await new Promise(resolve=>{release=resolve;});if(fixture.options.suggestion_prompt_late_error)throw Error('Synthetic obsolete suggestion status error.');}return value;};
 try{
  view.bind(context);await until(()=>release,'Read A was not held at false.');view.root.querySelector('[data-suggestion-consent="not_now"]').click();await until(()=>view.root.dataset.mode==='quiet','Dismissal B was not acknowledged before A.');
  release();await new Promise(resolve=>setTimeout(resolve,100));assert(view.root.dataset.mode==='quiet'&&view.root.querySelector('.suggestion-consent-error').hidden,'A late suggestion status value/error replaced its durable acknowledgement.');
  assert(await api.meetingSuggestionPromptDismissed(id)&&(await api.info()).settings.suggestion_automation==='unanswered'&&!inference(fixture).length&&!policyMutations(fixture).length,'A stale status changed consent, persistence or inference.');
  return{lateValueAndErrorRejected:true,durableAcknowledgementRetained:true,noPolicyOrInference:true,syntheticOnly:true};
 }finally{release?.();client.invoke=invoke;}
}
export async function checkSuggestionEnlarged(){
 const{root,review,card,fixture}=await owned();await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="not_now"]').disabled,'The enlarged question did not become ready.');
 const writer=root.querySelector('.live-notes'),sizes=[...root.querySelectorAll('*')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);
 for(const[node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}
 for(const selector of ['[data-suggestion-consent="find"]','[data-suggestion-consent="not_now"]']){const button=card.querySelector(selector);button.scrollIntoView({block:'center'});await new Promise(resolve=>requestAnimationFrame(resolve));const bounds=button.getBoundingClientRect();assert(bounds.left>=0&&bounds.right<=innerWidth&&bounds.top>=0&&bounds.bottom<=innerHeight,'An enlarged suggestion consent action is clipped.');}
 assert(root.querySelector('.live-notes')===writer&&document.documentElement.scrollWidth<=innerWidth&&!inference(fixture).length,'Enlargement replaced writing, overflowed or started inference.');
 assert(review.querySelector('.insight-review-footer').getBoundingClientRect().bottom<=innerHeight,'Enlargement lost the pinned accepted export.');
 return{enlargedActionsReachable:true,retainedWriterAndFooter:true,noInference:true,syntheticOnly:true};
}
export async function checkSuggestionSettingsEnlarged(){
 await checkSuggestionSettings();const container=document.querySelector('.settings-pane'),sizes=[...container.querySelectorAll('*')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);
 for(const[node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}
 for(const selector of ['button[data-suggestion-automation="automatic"]','button[data-suggestion-automation="manual"]','.suggestion-settings-language .choice']){const control=document.querySelector(selector);control.scrollIntoView({block:'center'});await new Promise(resolve=>requestAnimationFrame(resolve));const bounds=control.getBoundingClientRect();assert(bounds.left>=0&&bounds.right<=innerWidth&&bounds.top>=0&&bounds.bottom<=innerHeight,'An enlarged suggestion Settings control is clipped.'+JSON.stringify({selector,left:bounds.left,right:bounds.right,top:bounds.top,bottom:bounds.bottom,width:innerWidth,height:innerHeight}));}
 assert(document.documentElement.scrollWidth<=innerWidth,'Suggestion Settings overflowed horizontally.');
 return{enlargedPolicyAndFallbackReachable:true,syntheticOnly:true};
}
export async function checkSuggestionFallbackChanged(){
 const{card,api,fixture}=await owned();assert(card.dataset.mode==='unavailable','The initial missing fallback was not explained.');
 const settings=structuredClone((await api.info()).settings);settings.suggestion_language='nl';await api.saveSettings(settings);
 await until(()=>{const current=document.querySelector('.suggestion-consent');return current.dataset.mode==='consent'&&!current.querySelector('[data-suggestion-consent="find"]').disabled;},'A newly saved fallback did not reach the open meeting’s manual analysis language.');
 assert(document.querySelector('.suggestion-consent-language').hidden,'The known saved fallback was still presented as an unanswered language choice.');
 assert((await api.info()).settings.suggestion_automation==='unanswered'&&!inference(fixture).length,'Observing the fallback granted consent or started analysis.');
 assert(policyMutations(fixture).length===1,'Observing the fallback saved an additional policy.');
 return{savedFallbackAdopted:true,noImplicitConsentOrInference:true,syntheticOnly:true};
}
export async function checkSuggestionCredentialFailure(){
 await checkSuggestionAttemptState();const{review,card,fixture}=await owned(),{t}=await import('/src/i18n.ts');
 assert(card.querySelector('.suggestion-consent-status').textContent.includes(t('insights.reason_missing_api_key')),'The durable missing-key reason was reduced to an unexplained failure.');
 assert(!review.querySelector('.insight-connection-settings').hidden&&!inference(fixture).length,'Credential repair is unreachable or rendering attempted inference.');
 return{durableCredentialReason:true,repairReachable:true,noAutomaticRetry:true,syntheticOnly:true};
}

export async function checkManualReviewResult(){
 const{review,card,fixture}=await owned(),{t}=await import('/src/i18n.ts'),find=review.querySelector('.insight-find');
 await until(()=>card.dataset.mode==='manual'&&!find.disabled,'The qualified manual Review action did not become ready.');
 assert(!inference(fixture).length&&review.querySelector('.insight-coverage').hidden,'Opening manual Review analysed content or showed premature coverage.');
 find.click();await until(()=>review.querySelector('.insight-progress').textContent.includes(t(fixture.options.insight_analysis_fail?'insights.job_failed':'insights.job_completed')),'The real manual job did not reach its terminal state.');
 assert(review.querySelector('.insight-availability').hidden,'The completed job repeated a connection-availability message.');
 const empty=review.querySelector('.insight-empty');
 if(fixture.options.insight_analysis_fail)assert(empty.hidden&&review.querySelector('.insight-coverage').hidden,'Failed analysis looked like completed empty analysis.');
 else assert(!empty.hidden&&empty.textContent===t('insights.none_found')&&!review.querySelector('.insight-coverage').hidden,'Completed empty analysis lost its result or coverage limitation.');
 assert(fixture.calls.filter(command=>command==='begin_insights_analysis').length===1,'Manual Review started more than one inference job.');
 return{terminalStateAccurate:true,coverageOnlyAfterCompletion:true,oneExplicitJob:true,syntheticOnly:true};
}

export async function checkReviewAvailabilityTransition(){
 const{review,card,fixture}=await owned(),{t}=await import('/src/i18n.ts');
 if(fixture.options.suggestion_automation==='manual')await checkManualReviewResult();
 else await until(()=>card.dataset.mode===fixture.options.suggestion_expected_mode,'The historical automatic result did not settle.');
 const client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),inferenceCount=inference(fixture).length,rows=[...review.querySelectorAll('.insight-card')];let reason='unconfigured';
 client.invoke=async(command,args)=>{const value=await invoke(command,args);return ['insights_availability','automatic_insights_availability'].includes(command)?{...value,configured:reason!=='unconfigured',qualified:false,available:false,reason_code:reason,qualification_id:null,...reason==='unconfigured'?{provider_id:null,model:null,endpoint:null}:{}}:value;};
 try{
  for(reason of ['unconfigured','unqualified']){
   review.querySelector('.insight-toolbar .text-button').click();
   await until(()=>card.dataset.mode==='unavailable'&&card.querySelector('.suggestion-consent-status').textContent===t(`insights.reason_${reason}`),'A changed connection was hidden by a historical analysis result.');
   assert(review.querySelector('.insight-find').disabled&&review.querySelector('.insight-progress').hidden&&review.querySelector('.insight-availability').hidden,'The current blocker competed with stale terminal progress or duplicate availability.');
   assert(review.querySelector('.insight-empty').hidden&&!review.querySelector('.insight-connection-settings').hidden,'Connection repair was hidden behind the previous empty result.');
   if(reason==='unconfigured')assert(review.querySelector('.insight-destination').hidden,'Removed connection details remained visible.');
   assert(rows.every(row=>row.isConnected)&&review.querySelectorAll('.insight-card').length===rows.length,'Changing availability discarded saved review cards.');
  }
  assert(inference(fixture).length===inferenceCount,'Refreshing current availability started another analysis.');
  return{currentConnectionReasonVisible:true,terminalProgressSuperseded:true,savedCardsRetained:true,noNewInference:true,syntheticOnly:true};
 }finally{client.invoke=invoke;}
}

export async function checkManualOnlySuggestions(referenceOnly=false){
 const{root,review,card,api,fixture}=await owned(),{t}=await import('/src/i18n.ts');
 if(referenceOnly)return{syntheticOnly:true,reference:true};
 const expected=fixture.options.suggestion_expected_mode??(fixture.options.suggestion_automation==='manual'?'manual':fixture.options.suggestion_automation==='automatic'?'not_run':'consent');
 await until(()=>card.dataset.mode===expected,'A qualified manual route was described as unavailable.');
 const checkbox=card.querySelector('[data-suggestion-consent-automatic]');
 assert(checkbox.disabled&&!checkbox.checked,'A manual-only route can grant automatic analysis.');
 assert(!inference(fixture).length&&!policyMutations(fixture).length,'Reading manual eligibility performed work.');
 if(expected==='consent')assert(review.querySelector('.insight-availability').hidden,'The manual consent card repeated a second readiness status.');
 if(expected==='consent'||expected==='not_run')assert(card.textContent.includes(t('insights.manual_only')),'The separate automatic restriction has no clear explanation.');
 const rows=[...review.querySelectorAll('.insight-card')],notes=root.querySelector('.live-notes').value,before=(await api.info()).settings.suggestion_automation;
 review.querySelector('.insight-analysis-details').open=true;
 const button=expected==='consent'?card.querySelector('[data-suggestion-consent="find"]'):review.querySelector('.insight-find');
 assert(!button.disabled&&!button.hidden&&button.getClientRects().length,'Manual Find is unreachable.');
 if(fixture.options.suggestion_forge_automatic){
  checkbox.checked=true;button.click();
  await until(()=>!card.querySelector('.suggestion-consent-error').hidden,'A forged automatic choice was not rejected.');
  assert(!inference(fixture).length&&!policyMutations(fixture).length&&(await api.info()).settings.suggestion_automation===before,'A forged disabled checkbox granted consent or sent content.');
  assert(checkbox.disabled&&!checkbox.checked,'The rejected automatic choice remained selected.');
 }
 if(fixture.options.suggestion_revoke_during_click){
  const client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client);let delivered=false;
  client.invoke=async(command,args)=>{const value=await invoke(command,args);if(command==='insights_availability')delivered=true;if(command==='app_info'&&delivered)return{...value,settings:{...value.settings,decision:{provider_id:'changed-route',protocol:'open_router_decisions',model:'changed-model',upstream:'other',expected_model:null,max_input_bytes:16000,timeout_secs:30}}};return value;};
  try{button.click();await until(()=>!card.querySelector('.suggestion-consent-error').hidden,'Changing the connection during a click was not reported.');assert(!inference(fixture).length&&!policyMutations(fixture).length,'A revoked connection saved consent or received content.');return{manualInitiallyAvailable:true,changedConnectionRejected:true,noInferenceOrPolicy:true,syntheticOnly:true};}finally{client.invoke=invoke;}
 }
 button.click();await until(()=>fixture.calls.includes('begin_insights_analysis'),'Qualified manual Find did not start.');
 const saved=(await api.info()).settings.suggestion_automation;
 assert(saved===(expected==='consent'?'manual':before),'Manual Find granted or rewrote automatic policy.');
 assert(fixture.calls.filter(command=>command==='begin_insights_analysis').length===1&&rows.every(row=>row.isConnected)&&root.querySelector('.live-notes').value===notes,'Manual Find duplicated work or discarded retained review/notes.');
 return{manualAvailable:true,automaticDisabled:true,freshManualAdmission:true,existingCardsAndNotesRetained:true,syntheticOnly:true};
}

export async function checkSuggestionContextRace(){
 const{root,card,api,fixture}=await owned(),client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client);
 const [phase,change]=fixture.options.suggestion_context_race.split('-'),before=await api.info(),snapshot=await api.insightsSnapshot(root.dataset.meetingId);
 const{meetingSuggestionConsent}=await import('/src/meeting-suggestion-consent.ts');
 let id=root.dataset.meetingId,chosen='nl',displayedPin='f'.repeat(64),release,entered=false,released=false,armed=false,infoReads=0,starts=0,policyWrites=0;
 const gate=new Promise(resolve=>{release=resolve;}),contexts=[];
 const available=()=>({configured:true,qualified:true,available:true,reason_code:'',provider_id:'owned-ui',model:'owned-ui-fixture',endpoint:'https://decision.example.invalid/v1',qualification_id:'owned-ui-only',configuration_fingerprint:displayedPin,resolved_language:chosen,language_policy:chosen,task:'meeting_suggestions_v1',use_case:'manual_finished_suggestions'});
 client.invoke=async(command,args)=>{
  if(command==='automatic_insights_availability'||command==='insights_availability')return available();
  if(command==='meeting_suggestion_prompt_dismissed')return false;
  if(['save_settings','set_meeting_suggestion_policy_checked'].includes(command))++policyWrites;
  const value=await invoke(command,args);
  if(command==='app_info'&&armed&&phase==='info'&&++infoReads===3){entered=true;await gate;released=true;}
  return value;
 };
 function context(){
  const disposals=[];const ctx={info:structuredClone(before),onDispose(callback){disposals.push(callback);},onInfoChange(){},async mutate(work){
   if(phase==='mutate'&&!entered){entered=true;await gate;try{return await work();}finally{released=true;}}
   return work();
  }};contexts.push(disposals);return ctx;
 }
 const ctx=context(),view=meetingSuggestionConsent(ctx,{id:()=>id,uid:snapshot.meeting_uid,snapshot:()=>snapshot,language:()=>chosen,analysisBusy:()=>false,canAnalyze:()=>true,
  chooseLanguage:async()=>{},manualAvailability:available,start:async()=>{++starts;},dismiss:async()=>{},canDismiss:()=>true,blockedReason:()=>'',reason:code=>code,changed(){},focusTarget:root.querySelector('.insight-review'),languageControlVisible:()=>false});
 card.replaceWith(view.root);view.bind(ctx);
 try{
  await until(()=>view.root.dataset.mode==='consent'&&!view.root.querySelector('[data-suggestion-consent-automatic]').disabled,'The direct consent owner did not become ready.');
  view.root.querySelector('[data-suggestion-consent-automatic]').checked=true;armed=true;view.root.querySelector('[data-suggestion-consent="find"]').click();
  await until(()=>entered,'The explicit choice did not reach its held boundary.');
  if(change==='dispose')contexts[0].forEach(callback=>callback());
  else if(change==='rebind'){id='replacement-owned-meeting';chosen='en';view.bind(context());}
  else if(change==='pin')displayedPin='b'.repeat(64);
  else chosen='en';
  release();await until(()=>released&&!view.busy,'The held choice did not settle after its local context changed.');
  await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  const after=await invoke('app_info',{});
  assert(policyWrites===0&&starts===0&&after.settings.suggestion_automation===before.settings.suggestion_automation&&after.settings.suggestion_language===before.settings.suggestion_language,
   'A disposed or replaced choice saved global consent/fallback or started analysis.');
  return{phase,change,staleContextSendsNothing:true,globalConsentPreserved:true,syntheticOnly:true};
 }finally{release();contexts.forEach(callbacks=>callbacks.forEach(callback=>callback()));client.invoke=invoke;view.root.replaceWith(card);}
}

export async function checkSuggestionAtomicPolicySave(){
 const{card,api,fixture}=await owned(),client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client);
 await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="find"]').disabled,'The manual choice did not become ready.');
 const before=await api.info(),nextLanguage=before.settings.ui_language==='nl'?'en':'nl',routeChange=fixture.options.suggestion_atomic_route_change;
 const nextSummary=before.settings.summary_automation==='automatic'?'manual':'automatic';
 let injected=false;
 client.invoke=async(command,args)=>{
  if(!injected&&['save_settings','set_meeting_suggestion_policy_checked'].includes(command)){
   injected=true;const latest=await invoke('app_info',{});
   const change=routeChange?{decision:{provider_id:'changed-route',protocol:'open_router_decisions',model:'changed-model',upstream:'other',expected_model:null,max_input_bytes:16000,timeout_secs:30}}:{ui_language:nextLanguage,summary_automation:nextSummary};
   await invoke('save_settings',{settings:{...latest.settings,...change}});
  }
  return invoke(command,args);
 };
 try{
  card.querySelector('[data-suggestion-consent="find"]').click();
  if(routeChange){
   await until(()=>!card.querySelector('.suggestion-consent-error').hidden,'The scoped save accepted a route changed at its native boundary.');
   const after=await invoke('app_info',{});
   assert(after.settings.decision.model==='changed-model'&&after.settings.suggestion_automation===before.settings.suggestion_automation&&after.settings.suggestion_language===before.settings.suggestion_language&&!fixture.calls.includes('begin_insights_analysis'),
    'A stale scoped save rewrote the new route, granted consent or started analysis.');
   await invoke('save_settings',{settings:{...after.settings,decision:before.settings.decision}});
   card.closest('.insight-review').querySelector('.insight-toolbar .text-button').click();
   await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="find"]').disabled,'Restoring the route left the failed choice stuck behind stale metadata.');
   card.querySelector('[data-suggestion-consent="find"]').click();await until(()=>fixture.calls.includes('begin_insights_analysis'),'An explicit retry could not use the restored route.');
   return{staleRouteRejectedAtPolicyWrite:true,staleAttemptSentNothing:true,explicitRetrySucceeded:true,syntheticOnly:true};
  }
  await until(()=>fixture.calls.includes('begin_insights_analysis'),'The acknowledged manual policy did not start analysis.');
  const after=await invoke('app_info',{});
  assert(injected&&after.settings.ui_language===nextLanguage&&after.settings.summary_automation===nextSummary,'Consent overwrote an unrelated setting changed at the save boundary.');
  assert(after.settings.suggestion_automation==='manual'&&fixture.calls.filter(command=>command==='set_meeting_suggestion_policy_checked').length===1&&fixture.calls.filter(command=>command==='save_settings').length===1,
   'Consent did not use one scoped policy write.');
  await invoke('save_settings',{settings:{...after.settings,ui_language:before.settings.ui_language}});
  return{concurrentAppearanceAndSummaryPreserved:true,oneAtomicConsentWrite:true,syntheticOnly:true};
 }finally{client.invoke=invoke;}
}
