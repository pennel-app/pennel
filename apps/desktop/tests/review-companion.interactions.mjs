const assert=(value,message)=>{if(!value)throw Error(message);};
const visible=node=>{const closed=node?.closest("details:not([open])");return !!node?.getClientRects().length&&!node.closest("[hidden]")&&(!closed||closed.querySelector(":scope > summary")?.contains(node));};
async function until(check,message){for(let attempt=0;attempt<260;attempt++){if(await check())return;await new Promise(resolve=>setTimeout(resolve,25));}throw Error(message);}
export async function checkReviewCompanion(){
 const fixture=window.__PENNEL_FIXTURE__,root=document.querySelector('.live-workspace'),review=root.querySelector('.insight-review'),tab=root.querySelector('[data-live-tab="suggestions"]'),api=window.__PENNEL_PLATFORM__,id=root.dataset.meetingId;
 await until(()=>review?.querySelector('.insight-card'),'The review cards did not load.');
 const items=(await api.invoke('insights_snapshot',{id})).items,proposed=items.filter(item=>item.review_state==='proposed');
 assert(visible(tab),'Review is still hidden behind More.');
 if(innerWidth>=1100)assert(tab.getAttribute('aria-selected')==='true'&&visible(review),'A wide finished meeting did not default to Review.');
 else{assert(!visible(review),'A narrow finished meeting opened over its document by default.');tab.click();await until(()=>visible(review),'Review did not open over the document.');}
 assert(tab.querySelector('.review-count')?.textContent===String(proposed.length),'Review does not show the open proposal count.');
 tab.focus();tab.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));
 assert(document.activeElement===root.querySelector('[data-live-tab="questions"]')&&root.querySelector('[data-live-tab="questions"]').getAttribute('aria-selected')==='true','The segmented keyboard path skipped Ask.');
 document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true,cancelable:true}));
 assert(document.activeElement===tab&&tab.getAttribute('aria-selected')==='true','The segmented keyboard path skipped Review.');
 const item=review.querySelector(`[data-insight-id="${proposed[0].id}"]`),notes=fixture.workspace.notes(),details=item.querySelector('.insight-card-details');
 assert(details&&!details.open&&!visible(item.querySelector('.insight-processing-record')),'Detailed evidence did not start behind a disclosure.');
 item.querySelector('.insight-accept').focus();item.querySelector('.insight-accept').click();await until(()=>item.dataset.reviewState==='accepted','Acceptance did not produce its retained quiet card.');
 assert(visible(item.querySelector('.insight-undo'))&&tab.querySelector('.review-count').textContent===String(proposed.length-1),'Acceptance lost Undo or kept the old open count.');
 assert(document.activeElement===item.querySelector('.insight-undo'),'Acceptance lost keyboard focus instead of reaching Undo.');
 assert(fixture.workspace.notes()===notes,'Accept silently inserted into private notes.');
 item.querySelector('.insight-undo').click();await until(()=>item.dataset.reviewState==='proposed','Undo did not restore a proposal.');
 assert(document.activeElement===item.querySelector('.insight-accept'),'Undo lost keyboard focus instead of reaching Accept.');
 assert(tab.querySelector('.review-count').textContent===String(proposed.length),'Undo did not restore the open count.');
 item.querySelector('.insight-accept').click();await until(()=>item.dataset.reviewState==='accepted','Second acceptance did not persist.');
 details.open=true;assert(visible(item.querySelector('.insight-processing-record'))&&visible(item.querySelector('.insight-add-notes')),'The retained evidence/insertion actions are not reachable in Details.');
 details.open=false;
 const copy=review.querySelector('.insight-export'),footer=review.querySelector('.insight-review-footer');assert(footer?.contains(copy)&&!copy.disabled,'Copy accepted items is not pinned and reachable.');
 copy.click();await until(()=>fixture.calls.includes('copy_text'),'Copy accepted items did not use the existing export.');
 assert(!fixture.calls.includes('begin_insights_analysis')&&!fixture.calls.includes('save_settings'),'Presentation or review started inference or changed settings.');
 return{reviewDefaultAndCount:true,acceptUndoPersistent:true,detailsPreserved:true,pinnedAcceptedExport:true,syntheticOnly:true};
}
export async function checkAllReviewed(){
 await checkReviewCompanion();const root=document.querySelector('.live-workspace'),review=root.querySelector('.insight-review'),fixture=window.__PENNEL_FIXTURE__,api=window.__PENNEL_PLATFORM__,id=root.dataset.meetingId;
 const snapshot=await api.invoke('insights_snapshot',{id});
 for(const item of snapshot.items.filter(item=>item.review_state==='proposed')){review.querySelector(`[data-insight-id="${item.id}"] .insight-accept`).click();await until(async()=>(await api.invoke('insights_snapshot',{id})).items.find(value=>value.id===item.id).review_state==='accepted','The last proposal did not persist.');}
 const{t}=await import('/src/i18n.ts');assert(review.querySelector('.insight-review-intro').textContent===t('insights.all_reviewed'),'Zero open suggestions did not explain the completed review.');
 assert(root.querySelector('.review-count').hidden&&!review.querySelector('.insight-export').disabled,'Completed review kept an open badge or blocked copying.');
 assert(!fixture.calls.includes('begin_insights_analysis'),'Completing review started inference.');
 return{allReviewedMeaningful:true,acceptedItemsRetained:true,syntheticOnly:true};
}
export async function checkReviewEnlarged(){
 await checkReviewCompanion();const root=document.querySelector('.live-workspace'),review=root.querySelector('.insight-review'),note=root.querySelector('.live-notes');
 const sizes=[...root.querySelectorAll('*')].filter(node=>node instanceof HTMLElement).map(node=>[node,parseFloat(getComputedStyle(node).fontSize)]);
 for(const[node,size]of sizes){node.style.fontSize=`${size*2}px`;node.style.lineHeight='1.4';}
 const body=review.querySelector('.insight-review-body');body.scrollTop=body.scrollHeight;await new Promise(resolve=>requestAnimationFrame(resolve));
 const copy=review.querySelector('.insight-export'),bounds=copy.getBoundingClientRect();assert(bounds.left>=0&&bounds.right<=innerWidth&&bounds.top>=0&&bounds.bottom<=innerHeight,'Enlarged Copy accepted items is clipped or scrolls away.');
 assert(root.querySelector('.live-notes')===note&&document.documentElement.scrollWidth<=innerWidth,'Review enlargement replaced writing or overflowed horizontally.');
 return{enlargedPinnedCopyReachable:true,retainedWriter:true,syntheticOnly:true};
}
export async function checkEmptyReview(){
 const root=document.querySelector('.live-workspace');await until(()=>root?.querySelector('.insight-review .suggestion-consent')?.dataset.mode==='unavailable','Empty review did not load its availability.');
 const review=root.querySelector('.insight-review');if(innerWidth<1100)root.querySelector('[data-live-tab="suggestions"]').click();
 assert(review.querySelector('.insight-empty').hidden&&review.querySelector('.insight-analysis-details').open,'Unavailable review competed with a generic empty state or hid its setup controls.');
 assert(review.querySelector('.insight-find').disabled&&visible(review.querySelector('.suggestion-consent-status'))&&review.querySelector('.insight-availability').hidden&&!window.__PENNEL_FIXTURE__.calls.includes('begin_insights_analysis'),'Unqualified empty review permitted analysis or repeated its reason.');
 assert(review.querySelector('.insight-coverage').hidden,'Review described incomplete coverage before any analysis.');
 return{oneUnavailableReason:true,findAndSetupReachable:true,noPrematureCoverage:true,noInference:true,syntheticOnly:true};
}
