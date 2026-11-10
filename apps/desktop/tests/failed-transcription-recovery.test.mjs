import assert from "node:assert/strict";
import test from "node:test";
import { FailedTranscriptionRecovery, failedRecoveryControl, transcriptDisplayText } from "../src/failed-transcription-recovery.ts";
import { setInterfaceLanguage } from "../src/i18n.ts";

test("retained partial speech is visible and stays explicitly incomplete", () => {
  setInterfaceLanguage("en");
  assert.equal(transcriptDisplayText({ failed: false, text: "A complete passage." }), "A complete passage.");
  assert.equal(transcriptDisplayText({ failed: true, text: "A recognized prefix" }), "A recognized prefix\nPartial transcription. Retry this passage to finish.");
  assert.ok(transcriptDisplayText({ failed: true, text: "" }).length);
  setInterfaceLanguage("nl");
  assert.match(transcriptDisplayText({ failed: true, text: "Een herkend begin" }), /^Een herkend begin\nOnvolledige transcriptie/);
});

const plan = (failed = 2, retryable = failed, reason_code = null) => ({ failed_count: failed, retryable_count: retryable,
  status: failed === 0 ? "complete" : retryable > 0 ? "retryable" : "unavailable", reason_code });
const detail = (failed = 2, recovery = plan(failed)) => ({ meeting_uid: "uid", transcript: Array.from({ length: 2 }, (_, index) => ({ failed: index < failed })), failed_recovery: recovery });
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };

function fixture(initial = detail()) {
  const state = { scope: "binding-1", id: "meeting", blocked: false, detail: initial, calls: [], accepted: [], flushError: null,
    retry: async () => { state.detail = detail(0); return 2; }, read: async () => state.detail, flush: async () => {} };
  const model = new FailedTranscriptionRecovery({ scope: () => state.scope, id: () => state.id, meetingUid: "uid", blocked: () => state.blocked,
    changed: () => {}, flush: async () => { state.calls.push("flush"); if (state.flushError) throw state.flushError; await state.flush(); },
    read: async id => { state.calls.push(`read:${id}`); return state.read(id); }, retry: async id => { state.calls.push(`retry:${id}`); return state.retry(id); },
    accept: value => state.accepted.push(value) });
  model.observe(initial);
  return { state, model };
}

test("reopened failures stay visible and only an authoritative eligible plan enables retry", () => {
  assert.deepEqual(failedRecoveryControl(detail()), { visible: true, incomplete: true, canRetry: true, failedCount: 2, retryableCount: 2, reasonCode: null });
  const oldBackend = { ...detail(), failed_recovery: undefined };
  assert.equal(failedRecoveryControl(oldBackend).canRetry, false);
  assert.equal(failedRecoveryControl(oldBackend).visible, true);
  assert.equal(failedRecoveryControl(oldBackend).failedCount, null);
  assert.equal(failedRecoveryControl(detail(2, plan(2, 0, "no_retained_audio"))).canRetry, false);
  assert.equal(failedRecoveryControl(detail(2, { failed_count: null, retryable_count: null, status: "unavailable", reason_code: "metadata_unavailable" })).failedCount, null);
  assert.equal(failedRecoveryControl(detail(0)).visible, false);
  for (const recovery of [plan(2, 3), { ...plan(), status: "future" }, { ...plan(), retryable_count: -1 }, { ...plan(), failed_count: null }, plan(0)]) {
    assert.equal(failedRecoveryControl(detail(2, recovery)).canRetry, false);
  }
});

test("explicit retry flushes notes, rechecks eligibility, retries once and refreshes completion", async () => {
  const { state, model } = fixture();
  assert.equal(await model.retry(), true);
  assert.deepEqual(state.calls, ["flush", "read:meeting", "retry:meeting", "read:meeting"]);
  assert.equal(model.outcome, "complete");
  assert.equal(model.control.canRetry, false);
});

test("save failure preserves the draft and prevents any recognition or preflight", async () => {
  const { state, model } = fixture(); const error = Error("Owned note save failure"); state.flushError = error;
  await model.retry();
  assert.deepEqual(state.calls, ["flush"]);
  assert.equal(model.error, error);
  assert.equal(model.outcome, null);
  assert.equal(model.busy, false);
});

test("eligibility is rechecked after notes are saved and disappearing audio cannot trigger retry", async () => {
  const { state, model } = fixture(); state.detail = detail(2, plan(2, 0, "no_retained_audio"));
  assert.equal(await model.retry(), false);
  assert.deepEqual(state.calls, ["flush", "read:meeting"]);
  assert.equal(model.control.canRetry, false);
});

test("provider error refreshes committed partial progress without reporting complete recovery", async () => {
  const { state, model } = fixture(); const error = Error("Owned speech unavailable");
  state.retry = async () => { state.detail = detail(1); throw error; };
  await model.retry();
  assert.equal(model.error, error);
  assert.equal(model.outcome, "partial");
  assert.equal(model.control.failedCount, 1);
  state.retry = async () => { state.detail = detail(0); return 1; };
  await model.retry();
  assert.equal(model.outcome, "complete");
  assert.equal(state.calls.filter(command => command.startsWith("retry:")).length, 2);
});

test("zero or a positive but partial native count is not complete recovery", async () => {
  const first = fixture(); first.state.retry = async () => 0;
  await first.model.retry(); assert.equal(first.model.outcome, "none"); assert.equal(first.model.control.failedCount, 2);
  const second = fixture(); second.state.retry = async () => { second.state.detail = detail(1, plan(1, 0, "unverified_provenance")); return 1; };
  await second.model.retry(); assert.equal(second.model.outcome, "partial"); assert.equal(second.model.control.canRetry, false);
});

test("a failed refresh never turns an old eligible plan into completion or a duplicate retry", async () => {
  const { state, model } = fixture(); let reads = 0;
  state.read = async () => { if (++reads > 1) throw Error("Owned refresh failure"); return state.detail; };
  await model.retry();
  assert.equal(model.outcome, null); assert.equal(model.control.canRetry, false); assert.equal(model.control.failedCount, null);
});

test("double clicks share the pending guard; navigation and a replaced identity reject late results", async () => {
  const { state, model } = fixture(); const wait = deferred(); state.retry = () => wait.promise;
  const first = model.retry(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(await model.retry(), false);
  assert.equal(state.calls.filter(command => command.startsWith("retry:")).length, 1);
  const acceptedBefore = state.accepted.length; state.scope = "binding-2"; wait.resolve(2); await first;
  assert.equal(state.accepted.length, acceptedBefore); assert.equal(model.outcome, null);
  const changed = fixture(); changed.state.detail = { ...detail(), meeting_uid: "replaced" };
  await changed.model.retry(); assert.equal(changed.state.calls.some(command => command.startsWith("retry:")), false);
});

test("navigation during note flush sends no retry, while an acknowledged rename uses the new ID", async () => {
  const first = fixture(); const wait = deferred(); first.state.flush = () => wait.promise;
  const request = first.model.retry(); first.state.scope = null; wait.resolve(); await request;
  assert.deepEqual(first.state.calls, ["flush"]);
  const renamed = fixture(); renamed.state.flush = async () => { renamed.state.id = "renamed"; };
  await renamed.model.retry(); assert.equal(renamed.state.calls.includes("retry:renamed"), true);
});

test("a later source refresh cannot leave stale completion or zero-progress copy over changed failures", async () => {
  const recovered = fixture(); await recovered.model.retry();
  assert.equal(recovered.model.outcome, "complete");
  recovered.model.observe(detail());
  assert.equal(recovered.model.outcome, null); assert.equal(recovered.model.control.incomplete, true);
  const zero = fixture(); zero.state.retry = async () => 0; await zero.model.retry();
  zero.model.observe(detail(0));
  assert.equal(zero.model.outcome, null); assert.equal(zero.model.control.visible, false);
});
