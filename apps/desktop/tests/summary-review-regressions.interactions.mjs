const assert=(value,message)=>{if(!value)throw Error(message);};
async function until(check,message){for(let i=0;i<200;i++){if(await check())return;await new Promise(r=>setTimeout(r,25));}throw Error(message);}
const visible=node=>!!node?.getClientRects().length&&!node.closest('[hidden]');
const settle=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
async function harness(run){
 assert(window.__PENNEL_FIXTURE__,'Use the synthetic owned fixture.');
 const {api}=await import('/src/api.ts'),{meetingSummaryEditor}=await import('/src/meeting-summary-editor.ts');
 const info=await api.info(),id=window.__PENNEL_FIXTURE__.options.meeting_id,cleanup=[];
 const ctx={info,openSetup(){},report(){},onDispose(fn){cleanup.push(fn);},onInfoChange(){}};
 const view=meetingSummaryEditor(ctx,{id:()=>id,finished:()=>true,flushNotes:async()=>{},template:()=>info.settings.default_template,sourceAvailable:()=>true});
 const container=document.createElement('div');container.dataset.summaryRegression='true';container.style.cssText='position:fixed;inset:20px;overflow:auto;background:var(--surface);z-index:100;padding:20px';container.append(view.root);document.body.append(container);
 try{await view.refresh();await run({api,id,view,root:view.root,container});}finally{cleanup.forEach(fn=>fn());container.remove();}
}
export async function checkStaleSummaryRead(){await harness(async({api,id,view,root})=>{
 const client=window.__PENNEL_PLATFORM__,original=client.invoke.bind(client);let latch=false,release,late;const writes=[];
 client.invoke=async(command,args={})=>{const shouldLatch=command==='meeting_summary'&&latch;if(shouldLatch)latch=false;const result=await original(command,args);if(command==='update_meeting_summary_action')writes.push({args,result});if(shouldLatch)await new Promise(resolve=>{release=resolve;});return result;};
 try{
   latch=true;late=view.refresh();await until(()=>release,'A summary read was not latched.');
   const checkbox=root.querySelector('.structured-action input'),owner=root.querySelector('.structured-action-owner');checkbox.focus();checkbox.click();
   await until(()=>writes.length===1&&!checkbox.disabled,'Action tick was not acknowledged during the older read.');
   owner.click();await until(()=>document.querySelector('dialog[open] input'),'Owner dialog missing.');const dialog=document.querySelector('dialog[open]');dialog.querySelector('input').value='Noor';dialog.querySelector('button[type=submit]').click();
   await until(()=>writes.length===2&&!owner.disabled,'Owner write was not acknowledged.');const acknowledged=writes[1].result;
   checkbox.focus();release();await late;await settle();
   assert(root.querySelector('.meeting-summary-recovery').hidden,'The pre-write read created a phantom remote conflict.');
   assert(checkbox.checked&&owner.textContent==='Noor'&&!checkbox.disabled,'The pre-write read rolled back acknowledged action state.');
   assert(document.activeElement===checkbox,'The pre-write read lost active action focus.');
   checkbox.click();await until(()=>writes.length===3&&!checkbox.disabled,'The acknowledged action was no longer usable.');
   assert(writes[2].args.expectedRevision===acknowledged.revision&&writes[2].args.expectedSourceRevision===acknowledged.source_revision,'The next write used the superseded read revision.');
   assert(writes[2].args.owner==='Noor','A later tick lost its acknowledged owner.');
 }finally{if(release)release();if(late)await late.catch(()=>{});client.invoke=original;}
});}
export async function checkProtectedRecordPresentation(){await harness(async({api,id,view,root,container})=>{
 const client=window.__PENNEL_PLATFORM__,original=client.invoke.bind(client),saved=await api.meetingSummary(id);let detached=true;
 client.invoke=async(command,args={})=>{const result=await original(command,args);return command==='meeting_summary'?{...result,record:detached?null:saved.record}:result;};
 try{
   const checkbox=root.querySelector('.structured-action input'),structured=root.querySelector('.meeting-summary-structured'),rendered=root.querySelector('.meeting-summary-rendered');
   container.scrollTop=180;const scroll=container.scrollTop;checkbox.focus({preventScroll:true});await view.refresh();
   assert(visible(structured)&&checkbox.isConnected&&document.activeElement===checkbox,'Same-body detachment hid the focused structured control.');
   assert(view.editor.value===saved.markdown&&Math.abs(container.scrollTop-scroll)<2,'Protected detachment changed canonical text or reading scroll.');
   assert(root.querySelector('.meeting-summary-recovery').hidden,'Metadata detachment created a body conflict.');
   root.querySelector('.meeting-summary-edit').focus({preventScroll:true});await until(()=>!visible(structured)&&visible(root.querySelector('.meeting-summary-markdown')),'Detached metadata did not apply after focus release.');
   const fallback=root.querySelector('.meeting-summary-markdown'),walker=document.createTreeWalker(fallback,NodeFilter.SHOW_TEXT);let node;
   while((node=walker.nextNode()))if(node.data.includes('team confirmed'))break;
   assert(node,'Canonical fallback text was not retained.');rendered.focus({preventScroll:true});const range=document.createRange();range.setStart(node,4);range.setEnd(node,22);const selection=getSelection();selection.removeAllRanges();selection.addRange(range);const selected=selection.toString(),position=container.scrollTop;
   detached=false;await view.refresh();await settle();
   assert(visible(fallback)&&selection.toString()===selected,'Same-body attachment removed the Markdown selection.');
   assert(document.activeElement===rendered&&view.editor.value===saved.markdown&&Math.abs(container.scrollTop-position)<2,'Protected attachment lost reading focus, text or scroll.');
   selection.removeAllRanges();root.querySelector('.meeting-summary-edit').focus({preventScroll:true});await until(()=>visible(structured)&&!visible(fallback),'Attached metadata did not apply after selection/focus release.');
   assert(view.editor.value===saved.markdown,'Presentation release changed the canonical body.');
 }finally{getSelection().removeAllRanges();client.invoke=original;}
});}
