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
export async function checkSuggestionChoice(){
 const{root,review,card,api,fixture}=await owned(),before=(await api.info()).settings;
 assert(!inference(fixture).length&&!fixture.calls.includes('save_settings'),'Reading suggestion options started inference, credentials or policy.');
 if(fixture.options.suggestion_choose_language){assert(card.dataset.mode==='unavailable','Unknown speech language silently selected a policy.');await chooseLanguage(review,'insights.language_nl');}
 await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="find"]').disabled,'The exact qualified synthetic setup did not enable the question.');
 assert(card.textContent.includes('decision.example.invalid')&&card.textContent.includes('owned-ui-fixture'),'The question omits its actual processing destination.');
 const checkbox=card.querySelector('[data-suggestion-consent-automatic]');assert(!checkbox.checked,'Automatic suggestions were granted before a choice.');if(fixture.options.suggestion_choose_automatic)checkbox.click();
 card.querySelector('[data-suggestion-consent="find"]').click();await until(()=>fixture.calls.includes('begin_insights_analysis'),'The acknowledged choice did not start the existing manual analysis path.');
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
 assert(settings.suggestion_automation==='unanswered'&&settings.summary_automation==='manual'&&!inference(fixture).length&&!fixture.calls.includes('save_settings'),'Dismissal changed consent or started inference.');
 assert(saved.markdown===before.markdown&&JSON.stringify(saved.record)===JSON.stringify(before.record)&&root.querySelector('.meeting-summary-rendered')===reading&&reading.querySelector('.meeting-summary-structured')===structured,'Suggestion metadata replaced the saved summary, record or retained reading DOM.');
 return{durableIndependentDismissal:true,summaryAndReadingRetained:true,noPolicyOrInference:true,syntheticOnly:true};
}
export async function checkSuggestionReopened(){
 const{root,card,api,fixture}=await owned();assert(card.dataset.mode==='quiet'&&await api.meetingSuggestionPromptDismissed(root.dataset.meetingId),'Reopening forgot suggestion Not now.');
 assert((await api.info()).settings.suggestion_automation==='unanswered'&&!inference(fixture).length&&!fixture.calls.includes('save_settings'),'Reopening the quiet choice performed work.');
 return{reopenedQuietChoice:true,noAutomaticAction:true,syntheticOnly:true};
}
export async function checkSuggestionQuietFind(){
 await dismissSuggestionChoice();const{card,api,fixture}=await owned();card.querySelector('[data-suggestion-consent="one_off"]').click();await until(()=>fixture.calls.includes('begin_insights_analysis'),'Quiet Find did not start once.');
 assert((await api.info()).settings.suggestion_automation==='unanswered'&&!fixture.calls.includes('save_settings'),'Quiet Find changed global policy.');
 return{oneOffAnalysis:true,unansweredRetained:true,syntheticOnly:true};
}
export async function checkSuggestionUnavailable(){
 const{review,card,fixture}=await owned();assert(card.dataset.mode==='unavailable'&&card.querySelector('.summary-consent-card').hidden,'An unqualified setup exposed the consent question.');
 assert(card.querySelector('.suggestion-consent-status').textContent.trim()&&!inference(fixture).length&&!fixture.calls.includes('save_settings'),'Unavailable setup lost its reason or caused work.');
 assert(review.querySelector('.insight-analysis-details').open&&!review.querySelector('.insight-connection-settings').hidden,'Unavailable connection setup is not reachable.');
 return{exactQualificationGateRetained:true,reasonAndSetup:true,noInference:true,syntheticOnly:true};
}
export async function checkSuggestionFailure(){
 const{card,fixture,api}=await owned();await until(()=>card.dataset.mode==='consent'&&!card.querySelector('[data-suggestion-consent="not_now"]').disabled,'The question is not ready.');
 const client=window.__PENNEL_PLATFORM__,invoke=client.invoke.bind(client),metadata=fixture.options.suggestion_failure==='metadata';let reject=true;
 client.invoke=async(command,args)=>{if(reject&&command===(metadata?'dismiss_meeting_suggestion_prompt':'save_settings'))throw Error('Synthetic suggestion choice unavailable.');return invoke(command,args);};
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
 assert(!inference(fixture).length&&!fixture.calls.includes('save_settings'),'An attempt state retried or changed consent while rendering.');
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
 if(fixture.options.suggestion_external_dismissal){const before=await api.meetingSummary(id);await api.dismissMeetingSuggestionPrompt(id,before.revision,before.source_revision);await new Promise(resolve=>setTimeout(resolve,3100));view.refresh();await until(()=>view.root.dataset.mode==='quiet','The periodic status refresh did not observe an external durable dismissal.');assert(!inference(fixture).length&&!fixture.calls.includes('save_settings'),'Refreshing durable status performed work.');return{externalDismissalObserved:true,readOnlyRefresh:true,syntheticOnly:true};}
 if(fixture.options.suggestion_read_retry){let reject=true;client.invoke=async(command,args)=>{if(reject&&command==='automatic_insights_availability'&&args.languagePolicy==='nl')throw Error('Synthetic status read unavailable.');return invoke(command,args);};try{currentLanguage='nl';view.refresh();await until(()=>view.root.querySelector('.suggestion-consent-error').textContent.includes('Synthetic status read unavailable.'),'The changed-language read did not report its failure.');const retry=view.root.querySelector(':scope > .btn');assert(!retry.hidden&&!retry.disabled,'A failed replacement status read has no reachable Retry.');reject=false;retry.click();await until(()=>view.root.dataset.mode==='consent'&&view.root.querySelector('.suggestion-consent-error').hidden,'Status Retry did not restore its qualified question.');assert(!inference(fixture).length&&!fixture.calls.includes('save_settings'),'Retrying a read performed work.');return{readFailureRetryReachable:true,queryRestoredWithoutWork:true,syntheticOnly:true};}finally{client.invoke=invoke;}}
 let release,held=false;
 client.invoke=async(command,args)=>{const value=await invoke(command,args);if(command==='meeting_suggestion_prompt_dismissed'&&!held){held=true;await new Promise(resolve=>{release=resolve;});if(fixture.options.suggestion_prompt_late_error)throw Error('Synthetic obsolete suggestion status error.');}return value;};
 try{
  view.bind(context);await until(()=>release,'Read A was not held at false.');view.root.querySelector('[data-suggestion-consent="not_now"]').click();await until(()=>view.root.dataset.mode==='quiet','Dismissal B was not acknowledged before A.');
  release();await new Promise(resolve=>setTimeout(resolve,100));assert(view.root.dataset.mode==='quiet'&&view.root.querySelector('.suggestion-consent-error').hidden,'A late suggestion status value/error replaced its durable acknowledgement.');
  assert(await api.meetingSuggestionPromptDismissed(id)&&(await api.info()).settings.suggestion_automation==='unanswered'&&!inference(fixture).length&&!fixture.calls.includes('save_settings'),'A stale status changed consent, persistence or inference.');
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
 assert(fixture.calls.filter(command=>command==='save_settings').length===1,'Observing the fallback saved an additional policy.');
 return{savedFallbackAdopted:true,noImplicitConsentOrInference:true,syntheticOnly:true};
}
export async function checkSuggestionCredentialFailure(){
 await checkSuggestionAttemptState();const{review,card,fixture}=await owned(),{t}=await import('/src/i18n.ts');
 assert(card.querySelector('.suggestion-consent-status').textContent.includes(t('insights.reason_missing_api_key')),'The durable missing-key reason was reduced to an unexplained failure.');
 assert(!review.querySelector('.insight-connection-settings').hidden&&!inference(fixture).length,'Credential repair is unreachable or rendering attempted inference.');
 return{durableCredentialReason:true,repairReachable:true,noAutomaticRetry:true,syntheticOnly:true};
}
