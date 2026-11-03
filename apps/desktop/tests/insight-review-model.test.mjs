import assert from "node:assert/strict";
import test from "node:test";
import { InsightReviewModel } from "../../../assets/contracts/insight-review-model.ts";

const item = { id: "item", revision: 1, text: "Original quote", edited_text: null, edit_claim: null, review_state: "proposed", sources: [], source_history: [], source_state: "current" };
const snapshot = (revision = 1, items = [item], status = "ready") => ({ meeting_uid: "meeting", revision, status, items, insertion_receipts: [] });
function deferred() { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; }
function service(changes = {}) {
  return { read: async () => snapshot(), availability: async (_id, policy) => ({ available: false, reason_code: policy === "unknown" ? "unknown_language" : "unqualified" }),
    start: async () => { throw Error("unqualified work must not run"); }, ...changes };
}

test("missing qualification or explicit language prevents analysis without affecting reviewed content", async () => {
  const model = new InsightReviewModel("folder", "meeting", service());
  await model.load(); await model.choosePolicy("unknown");
  assert.equal(await model.analyze(), false);
  await model.choosePolicy("nl"); assert.equal(await model.analyze(), false);
  assert.equal(model.items()[0].text, "Original quote");
});

test("overlapping metadata invalidations do not discard the only owned snapshot", async () => {
  const waiting = deferred(); let reads = 0;
  const model = new InsightReviewModel("folder", "meeting", service({ read: async () => { reads++; return reads === 1 ? waiting.promise : snapshot(2); } }));
  const first = model.load(); const recent = model.load(); waiting.resolve(snapshot());
  await Promise.all([first, recent]);
  assert.ok(model.getSnapshot().snapshot);
  assert.equal(model.getSnapshot().snapshot.meeting_uid, "meeting");
});

test("editing is protected before opening and typing during save survives the acknowledgement", async () => {
  const claimed = { ...item, revision: 2, edit_claim: "claim" };
  const waiting = deferred();
  const model = new InsightReviewModel("folder", "meeting", service({ claim: async () => snapshot(2, [claimed]), review: async () => waiting.promise }));
  await model.load(); await model.beginEdit("item"); model.edit("item", "First edit");
  const saving = model.saveEdit("item"); model.edit("item", "Typed during save");
  waiting.resolve(snapshot(3, [{ ...claimed, revision: 3, edit_claim: null, edited_text: "First edit" }]));
  await saving;
  assert.equal(model.getSnapshot().edits.get("item").text, "Typed during save");
  assert.equal(model.dirty, true);
  model.apply(snapshot(4, [], "unsupported"));
  assert.equal(model.items().length, 1);
  assert.equal(await model.saveEdit("item"), false);
  assert.equal(model.getSnapshot().edits.get("item").text, "Typed during save");
});

test("logical cancellation does not admit replacement while the physical worker is running", async () => {
  const cancelled = { job_id: "job", meeting_uid: "meeting", revision: 2, state: "cancelled", physical_running: true };
  const model = new InsightReviewModel("folder", "meeting", service({ availability: async () => ({ available: true }),
    start: async () => ({ ...cancelled, revision: 1, state: "running" }), cancel: async () => cancelled,
    job: async () => ({ ...cancelled, revision: 3, physical_running: false }) }));
  await model.load(); await model.choosePolicy("en"); assert.equal(await model.analyze(), true);
  await model.cancel(); assert.equal(model.canAnalyze, false);
  await model.poll(); assert.equal(model.canAnalyze, true);
});
