import assert from "node:assert/strict";
import test from "node:test";
import { summaryConsentDecision, performSummaryConsent } from "../src/summary-consent-state.ts";
test("checkbox Create maps to explicit global policy while Not now and quiet Create leave it alone",()=>{
 assert.deepEqual(summaryConsentDecision("create",true),{policy:"automatic",generate:true,dismiss:false});
 assert.deepEqual(summaryConsentDecision("create",false),{policy:"manual",generate:true,dismiss:false});
 assert.deepEqual(summaryConsentDecision("not_now",false),{policy:null,generate:false,dismiss:true});
 assert.deepEqual(summaryConsentDecision("quiet_create",false),{policy:null,generate:true,dismiss:false});
});
test("a failed policy save cannot start inference or dismiss the meeting prompt",async()=>{
 const state={policy:"unanswered",summary:"",dismissed:false};
 await assert.rejects(performSummaryConsent("create",true,{savePolicy:async()=>{throw Error("disk unavailable");},generate:async()=>{state.summary="Generated";},dismiss:async()=>{state.dismissed=true;}}),/disk unavailable/);
 assert.deepEqual(state,{policy:"unanswered",summary:"",dismissed:false});
});
test("quiet Create and durable Not now preserve Unanswered independently",async()=>{
 const state={policy:"unanswered",summary:"",dismissed:false};const effects={savePolicy:async policy=>{state.policy=policy;},generate:async()=>{state.summary="Generated";},dismiss:async()=>{state.dismissed=true;}};
 await performSummaryConsent("not_now",false,effects);assert.deepEqual(state,{policy:"unanswered",summary:"",dismissed:true});
 await performSummaryConsent("quiet_create",false,effects);assert.deepEqual(state,{policy:"unanswered",summary:"Generated",dismissed:true});
});
