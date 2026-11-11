const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let i=0;i<180;i++){if(await check())return;await new Promise(r=>setTimeout(r,25));}throw Error(message);}
const visible=node=>!!node?.getClientRects().length&&!node.closest('[hidden]');
export async function checkGoalSelection(){
 const root=document.querySelector('.onboarding-shell'), next=root.querySelector('.onboarding-next'), fixture=window.__PENNEL_FIXTURE__;
 assert(visible(next)&&!next.disabled,'Welcome needs an explicit Continue action.');
 const before=structuredClone(fixture.info().settings);
 const languagePicker=root.querySelector('.onboarding-top-controls .choice'),originalLanguage=document.documentElement.lang;
 for(const value of [originalLanguage==='en'?'nl':'en',before.ui_language??'system']) {
   const label=languagePicker.options.find(option=>option.value===value).label;languagePicker.click();
   [...document.querySelectorAll('.choice-list [role=option]')].find(option=>option.textContent===label).click();
   const language=value==='system'?fixture.info().system_interface_language??originalLanguage:value;
   await until(()=>fixture.info().settings.ui_language===value&&document.documentElement.lang===language,'Top language selection did not apply.');
   assert(languagePicker.isConnected,'Top language selection replaced its control.');
 }
 assert(JSON.stringify(fixture.info().settings.dictation_stt)===JSON.stringify(before.dictation_stt)&&fixture.info().settings.language===before.language,'Interface language changed speech configuration.');
 const meeting=root.querySelector('[data-onboarding-goal="meeting"]');assert(meeting,'Meeting goal card missing.');meeting.focus();meeting.click();
 await new Promise(r=>setTimeout(r,100));
 assert(root.dataset.step==='welcome'&&meeting.getAttribute('aria-pressed')==='true','Selecting a goal advanced before Continue.');
 assert(fixture.info().settings.onboarding.resume_step===before.onboarding.resume_step,'Goal selection persisted the next step too early.');
 assert(document.activeElement===meeting,'Selecting a goal replaced its focused card.');
 next.click();await until(()=>root.dataset.step==='engine'&&fixture.info().settings.onboarding.goal==='meeting','Continue lost the selected goal.');
 assert(JSON.stringify(fixture.info().settings.dictation_stt)===JSON.stringify(before.dictation_stt)&&fixture.info().settings.language===before.language,'Goal selection changed a speech setup.');
 assert(root.querySelector('.onboarding-top-controls .choice'),'Interface language is not available at the top.');
 const {api}=await import('/src/api.ts');const {recommendedSpeechModel}=await import('/src/dictation-home-state.ts');
 const recommended=recommendedSpeechModel(await api.localModels(fixture.info().settings.language,true));
 assert(root.querySelector('.engine-model-heading strong')?.textContent===recommended.name&&root.querySelector('.onboarding-recommended-badge'),'New first use did not lead with the compatible shared recommendation.');
}
export async function checkDownloadOutcome(kind){
 const root=document.querySelector('.onboarding-shell'),fixture=window.__PENNEL_FIXTURE__,next=root.querySelector('.onboarding-next'),id=fixture.info().settings.dictation_stt.model;
 if(kind==='resume') {
   await until(()=>visible(root.querySelector('.onboarding-cancel-download')),'Existing download did not attach.');
   root.querySelector('.onboarding-other-options').open=true;root.querySelector('.onboarding-local-options').open=true;
   const picker=root.querySelector('.engine-model-options .choice');picker.focus();
   assert(fixture.completeDownload(id),'No resumed download active.');
   await until(()=>!next.disabled,'Existing completed download did not become ready.');
   assert(picker.isConnected&&document.activeElement===picker&&picker.value===id,'Background completion replaced the focused saved-model picker.');
   assert(root.dataset.step==='engine','A background resumed download applied a setup without Continue.');
   next.click();await until(()=>root.dataset.step==='permissions','Continue did not apply the downloaded setup.');return;
 }
 next.click();await until(()=>visible(root.querySelector('.onboarding-cancel-download')),'Download did not start.');
 if(kind==='leave') {
   const {t}=await import('/src/i18n.ts');[...root.querySelectorAll('.onboarding-footer-left button')].find(button=>button.textContent===t('onboarding.back')).click();
   await until(()=>root.dataset.step==='welcome','Back did not leave speech setup.');
   assert(fixture.completeDownload(id),'Background explicit download was lost.');await new Promise(r=>setTimeout(r,180));
   assert(root.dataset.step==='welcome'&&fixture.info().settings.onboarding.resume_step==='welcome','Late download completion moved the user away from Welcome.');return;
 }
 assert(fixture.completeDownload(id,'Synthetic download failed.'),'No failing download active.');
 await until(()=>root.querySelector('.engine-error')?.textContent.includes('Synthetic download failed.')&&!next.disabled,'Download failure has no retry.');
 assert(root.dataset.step==='engine','Failed download advanced setup.');
 const attempts=fixture.calls.filter(command=>command==='download_model').length;await new Promise(r=>setTimeout(r,80));
 assert(fixture.calls.filter(command=>command==='download_model').length===attempts,'Download failure retried without an explicit action.');
 next.click();await until(()=>visible(root.querySelector('.onboarding-cancel-download')),'Download failure could not retry.');
 assert(fixture.completeDownload(id),'Retry did not own a new download.');await until(()=>root.dataset.step==='permissions','Successful retry did not continue.');
}
export async function checkDownloadContinuation(){
 const fixture=window.__PENNEL_FIXTURE__,root=document.querySelector('.onboarding-shell'),next=root.querySelector('.onboarding-next');
 const id=fixture.info().settings.dictation_stt.model;
 next.click();await until(()=>next.disabled&&visible(root.querySelector('.onboarding-cancel-download')),'Download did not expose footer progress and Cancel.');
 fixture.emit('download-progress',{id,downloaded:50,total:100});await until(()=>next.textContent.includes('50%'),'The footer does not show actual download progress.');
 assert(root.dataset.step==='engine','Downloading advanced before completion.');
 root.querySelector('.onboarding-cancel-download').click();await until(()=>!next.disabled&&!visible(root.querySelector('.onboarding-cancel-download')),'Cancel did not clear the pending continuation.');
 assert(root.dataset.step==='engine','Cancelled download advanced onboarding.');
 next.click();await until(()=>visible(root.querySelector('.onboarding-cancel-download')),'Download cannot retry after Cancel.');
 assert(fixture.completeDownload(id),'The synthetic actual download was not active.');
 await until(()=>root.dataset.step==='permissions','Actual completed download did not continue automatically.');
 assert(fixture.info().settings.onboarding.resume_step==='permissions','Completion did not persist the real next step.');
 assert(fixture.info().settings.onboarding.status!=='completed','Download incorrectly completed first use.');
 assert(!fixture.calls.some(command=>['toggle_dictation','copy_text','start_meeting'].includes(command)),'Speech setup started an unrelated capture or external delivery.');
}
export async function checkEngineKeyboard(){
 const root=document.querySelector('.onboarding-shell'),outer=root.querySelector('.onboarding-other-options'),local=root.querySelector('.onboarding-local-options');outer.open=true;
 const before=JSON.stringify(window.__PENNEL_FIXTURE__.info().settings);
 const cloud=root.querySelector('.onboarding-option-row');
 for(const [key,engine] of [['ArrowLeft','apple'],['Home','local']]) {
   local.open=false;cloud.click();cloud.focus();
   cloud.dispatchEvent(new KeyboardEvent('keydown',{key,bubbles:true,cancelable:true}));
   await until(()=>root.querySelector('.onboarding-engine-detail').dataset.engine===engine,'Keyboard engine selection did not update.');
   const selected=root.querySelector('.engine-choice[aria-pressed="true"]');
   await until(()=>document.activeElement===selected,'Keyboard engine selection left focus on the remote row.');
   assert(local.open&&visible(selected),'Keyboard engine selection focused a hidden local target.');
 }
 assert(JSON.stringify(window.__PENNEL_FIXTURE__.info().settings)===before,'Keyboard browsing applied a model/provider setup.');
}
