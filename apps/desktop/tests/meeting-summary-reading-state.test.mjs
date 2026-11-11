import assert from 'node:assert/strict';
import test from 'node:test';
import { webcrypto } from 'node:crypto';
import { actionState, attachedSummaryRecord, laneSpans, summaryLength, summarySections } from '../src/meeting-summary-reading-state.ts';
const hash=async value=>Buffer.from(await webcrypto.subtle.digest('SHA-256',new TextEncoder().encode(value))).toString('hex');
const structure={labels:{summary:'Summary',decisions:'Decisions',actions:'Actions',open_questions:'Open',none:'None'},lead:'Literal <img src=x> text',topics:[],decisions:[],actions:[],open_questions:[]};
async function document(){const markdown='## Summary\n\nA real saved body';return{meeting_uid:'meeting-one',markdown,record:{schema_version:1,meeting_uid:'meeting-one',body_revision:await hash(markdown),provenance:{provider_id:'old-account',provider_kind:'groq',model:'actual-model',generated_at:'2026-11-10T10:00:00Z',template_id:'algemeen'},structure:structuredClone(structure)}};}
test('length defaults to Standard and Standard retains open questions without changing source content',()=>{
 assert.equal(summaryLength('detailed'),'detailed');assert.equal(summaryLength('brief'),'brief');assert.equal(summaryLength({length:'brief'}),'standard');assert.equal(summaryLength(null),'standard');
 assert.deepEqual(summarySections('brief'),{map:false,topics:false,open:false});assert.deepEqual(summarySections('standard'),{map:false,topics:true,open:true});assert.deepEqual(summarySections('detailed'),{map:true,topics:true,open:true});
});
test('only a known record for this exact meeting and canonical body attaches, including provenance-only fallback',async()=>{
 const saved=await document();assert.deepEqual(await attachedSummaryRecord(saved),saved.record);
 for(const change of [{schema_version:2},{meeting_uid:'foreign'},{body_revision:'0'.repeat(64)},{structure:{...structure,topics:'corrupt'}},{provenance:null}])assert.equal(await attachedSummaryRecord({...saved,record:{...saved.record,...change}}),null);
 assert.equal(await attachedSummaryRecord({...saved,markdown:saved.markdown+' edited'}),null);assert.equal(await attachedSummaryRecord({...saved,record:null}),null);
 assert.deepEqual(await attachedSummaryRecord({...saved,record:{...saved.record,structure:null}}),{...saved.record,structure:null});
});
test('ticking and assigning an owner preserve the other saved action-state field',()=>{
 const action={text:'Prepare draft',owner:'Generated owner',due:null,source:null,state:{done:true,owner:'Saved owner'}};
 assert.deepEqual(actionState(action,{done:false}),{done:false,owner:'Saved owner'});
 assert.deepEqual(actionState(action,{owner:'New owner'}),{done:true,owner:'New owner'});
 assert.deepEqual(actionState(action,{owner:null}),{done:true,owner:null});assert.deepEqual(action.state,{done:true,owner:'Saved owner'});
});
test('timeline geometry retains true gaps, never invents zero-width speech and bounds invalid data',()=>{
 const lane={key:'opaque',speaker:null,kind:'overlap',duration_ms:4000,spans:[{start_ms:1000,end_ms:3000},{start_ms:7000,end_ms:9000}]};
 assert.deepEqual(laneSpans(lane,10000),[{left:10,width:20},{left:70,width:20}]);
 assert.deepEqual(laneSpans({...lane,spans:[{start_ms:-1,end_ms:4},{start_ms:3,end_ms:3},{start_ms:Infinity,end_ms:9}]},10),[]);assert.deepEqual(laneSpans(lane,0),[]);
});
