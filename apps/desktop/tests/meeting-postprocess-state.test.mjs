import assert from "node:assert/strict";
import test from "node:test";
import { MeetingPostprocessState } from "../src/meeting-postprocess-state.ts";
const event=(uid,operation,state,stage="summary")=>({id:`meeting-${uid}`,meeting_uid:uid,operation_id:operation,stage,state});
test("postprocess progress belongs to its meeting, stage and current operation",()=>{
 const progress=new MeetingPostprocessState();
 assert.equal(progress.accept(event("A","one","started")),true);
 assert.equal(progress.accept(event("B","other","started")),true);
 assert.equal(progress.accept(event("A","two","started")),true);
 assert.equal(progress.accept(event("A","one","finished")),false);
 assert.equal(progress.get("A","summary").operation_id,"two");
 progress.accept(event("A","suggestion","started","suggestions"));
 assert.equal(progress.get("A","summary").state,"started");
 assert.equal(progress.accept(event("A","two","finished")),true);
 assert.equal(progress.get("A","suggestions").state,"started");
});
test("clearing terminal progress never erases an active operation",()=>{
 const progress=new MeetingPostprocessState();progress.accept(event("A","one","started"));progress.clearTerminal("A","summary");
 assert.equal(progress.get("A","summary").state,"started");
 progress.accept(event("A","one","failed"));progress.clearTerminal("A","summary");assert.equal(progress.get("A","summary"),null);
});
test("malformed or unowned completion metadata cannot create a processing state",()=>{
 const progress=new MeetingPostprocessState();
 for(const value of [null,{},event("A","one","finished"),{...event("A","one","started"),stage:"transcript text"},{...event("A","one","started"),state:"provider reply"}])assert.equal(progress.accept(value),false);
 assert.equal(progress.get("A","summary"),null);
});
