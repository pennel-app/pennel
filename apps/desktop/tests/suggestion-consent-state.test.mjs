import assert from 'node:assert/strict';
import test from 'node:test';
import { suggestionConsentPresentation, performSuggestionConsent } from '../src/suggestion-consent-state.ts';
const snapshot={meeting_uid:'uid',revision:1,status:'ready',items:[],insertion_receipts:[],resolved_language:'en'};
const availability={available:true,qualified:true,configured:true,reason_code:'',resolved_language:'en'};
const input={policy:'unanswered',snapshot,availability,dismissed:false,working:false,summaryWorking:false};
test('suggestion presentation follows its own consent and exact availability without authorizing work',()=>{
 for(const [patch,mode]of [[{},'consent'],[{dismissed:true},'quiet'],[{policy:'manual'},'manual'],[{policy:'automatic'},'not_run'],[{working:true},'running'],[{policy:'automatic',summaryWorking:true},'waiting_summary'],[{availability:{...availability,available:false,reason_code:'unqualified'}},'unavailable'],[{snapshot:null},'loading']])assert.equal(suggestionConsentPresentation({...input,...patch}).mode,mode);
 assert.equal(suggestionConsentPresentation({...input,availability:{...availability,available:false,reason_code:'foreground_impact_unaccepted'}}).showQuestion,false);
});
test('durable attempt and source states remain meaningful without discarding existing cards',()=>{
 const attempt={schema_version:1,meeting_uid:'uid',transcript_digest:'a'.repeat(64),language_policy:'en',admitted_at:'2026-11-10T00:00:00Z'};
 for(const [patch,mode,reason]of [[{automatic_attempt:{...attempt,outcome:'completed',items_found:0}},'done_empty',null],[{automatic_attempt:{...attempt,outcome:'failed',error_code:'deadline_exceeded'}},'failed','deadline_exceeded'],[{automatic_attempt:attempt},'failed','interrupted'],[{automatic_attempt_stale:true},'stale','source_changed'],[{automatic_attempt_reason_code:'attempt_unavailable'},'failed','attempt_unavailable']]){
  const result=suggestionConsentPresentation({...input,policy:'automatic',snapshot:{...snapshot,...patch}});assert.equal(result.mode,mode);assert.equal(result.reason,reason);
 }
 const existing={...snapshot,items:[{review_state:'proposed'}],automatic_attempt_stale:true};assert.equal(suggestionConsentPresentation({...input,snapshot:existing}).mode,'stale');assert.equal(existing.items.length,1);
});
test('current connection blockers take priority over historical analysis receipts without discarding cards',()=>{
 const histories=[{}, {automatic_attempt:{outcome:'completed',items_found:0}}, {automatic_attempt:{outcome:'failed',error_code:'deadline_exceeded'}}, {automatic_attempt_stale:true}, {automatic_attempt_reason_code:'attempt_unavailable'}, {items:[{review_state:'accepted'}]}];
 for(const policy of ['manual','automatic','unanswered'])for(const history of histories)for(const reason of ['unconfigured','unqualified']){
  const current={...snapshot,...history},before=structuredClone(current),result=suggestionConsentPresentation({...input,policy,snapshot:current,availability:{...availability,available:false,reason_code:reason}});
  assert.deepEqual(result,{mode:'unavailable',reason,showQuestion:false});assert.deepEqual(current,before);
 }
 assert.equal(suggestionConsentPresentation({...input,working:true,availability:{...availability,available:false,reason_code:'unconfigured'}}).mode,'running');
});
test('only an explicit question choice saves suggestion policy/fallback before starting',async()=>{
 const calls=[];const effects={savePolicy:async(policy,language)=>{calls.push(['policy',policy,language]);},start:async()=>{calls.push(['start']);return true;},dismiss:async()=>{calls.push(['dismiss']);}};
 await performSuggestionConsent('consent',true,'nl',true,effects);assert.deepEqual(calls,[['policy','automatic','nl'],['start']]);
 calls.length=0;await performSuggestionConsent('consent',false,'en',false,effects);assert.deepEqual(calls,[['policy','manual',undefined],['start']]);
 calls.length=0;await performSuggestionConsent('consent',false,'nl',true,effects);assert.deepEqual(calls,[['policy','manual',undefined],['start']]);
 calls.length=0;await performSuggestionConsent('not_now',false,'en',false,effects);assert.deepEqual(calls,[['dismiss']]);
 calls.length=0;await performSuggestionConsent('one_off',false,'en',false,effects);assert.deepEqual(calls,[['start']]);
});
test('failed suggestion consent never starts inference or grants the other policy',async()=>{
 let starts=0,dismissals=0;const before={summary:'automatic',suggestions:'unanswered'};
 await assert.rejects(performSuggestionConsent('consent',true,'en',true,{savePolicy:async()=>{throw Error('Synthetic save unavailable.');},start:async()=>{++starts;},dismiss:async()=>{++dismissals;}}),/save unavailable/);
 assert.equal(starts,0);assert.equal(dismissals,0);assert.deepEqual(before,{summary:'automatic',suggestions:'unanswered'});
});
