import assert from "node:assert/strict";
import test from "node:test";
import { LivePaneState, formatSelection, LiveStatusEpoch, segmentEvidenceValue } from "../src/live-workspace-state.ts";

test("wide windows default to Transcript while narrow windows start with the document", () => {
  const panes = new LivePaneState(); panes.resize(1099);
  assert.equal(panes.wide, false); assert.equal(panes.companionOpen, false);
  panes.resize(1100);
  assert.equal(panes.wide, true); assert.equal(panes.companionOpen, true);
  assert.equal(panes.companion, "transcript"); assert.equal(panes.shown("notes"), true);
  panes.resize(760); assert.equal(panes.companionOpen, false);
  panes.resize(1280); assert.equal(panes.companionOpen, true);
});

test("explicit companion choices survive resize and keep the document beneath an overlay", () => {
  const panes = new LivePaneState(); panes.setFinished(true); panes.resize(1280);
  panes.select("questions"); panes.resize(760);
  assert.equal(panes.companionOpen, true); assert.equal(panes.shown("questions"), true);
  assert.equal(panes.shown("notes"), true); assert.equal(panes.shown("summary"), true);
  panes.closeCompanion(); panes.resize(1400);
  assert.equal(panes.companionOpen, false); assert.equal(panes.companion, "questions");
  panes.toggleCompanion("transcript"); assert.equal(panes.companionOpen, true);
  panes.resize(700); panes.select("notes");
  assert.equal(panes.companionOpen, false); assert.equal(panes.shown("notes"), true);
});

test("closing a companion preserves the chosen writing section through wide and narrow layouts", () => {
  const panes = new LivePaneState(); panes.setFinished(true); panes.select("summary"); panes.resize(1280);
  panes.select("questions"); panes.closeCompanion(); panes.resize(760); panes.resize(1280);
  assert.equal(panes.primary, "summary"); assert.equal(panes.selected, "summary");
  assert.equal(panes.shown("questions"), false); assert.equal(panes.shown("summary"), true); assert.equal(panes.shown("notes"), true);
});

test("format helpers preserve source text and the selected content", () => {
  assert.deepEqual(formatSelection("one two", 4, 7, "bold"), { value: "one **two**", start: 6, end: 9 });
  assert.deepEqual(formatSelection("one\ntwo", 1, 6, "task"), { value: "- [ ] one\n- [ ] two", start: 7, end: 18 });
});

test("finishing or rebinding rejects already requested status and Stop results", async () => {
  const status = new LiveStatusEpoch();
  let resolve;
  const delayed = new Promise(done => { resolve = done; });
  const beforeFinish = status.capture();
  const accepted = delayed.then(() => status.accepts(beforeFinish));
  status.invalidate();
  resolve();
  assert.equal(await accepted, false);
  assert.equal(status.accepts(status.capture()), true);
  const beforeRebind = status.capture();
  status.invalidate();
  assert.equal(status.accepts(beforeRebind), false);
});

test("evidence identity survives persisted Markdown timestamps and whitespace", () => {
  const live = { start_ms: 1200, end_ms: 3500, speaker: "others", text: " First\n second\u0085third \t", voice: 2, failed: false };
  const stored = { ...live, end_ms: 1200, text: "First second third" };
  assert.deepEqual(segmentEvidenceValue(live), segmentEvidenceValue(stored));
  assert.notDeepEqual(segmentEvidenceValue(live), segmentEvidenceValue({ ...stored, text: "Different" }));
  assert.deepEqual(segmentEvidenceValue({ ...live, speaker: "me", voice: 10 }), [1200, "me", "First second third", false, null]);
});

test("durable source evidence excludes mutable speaker attribution but retains source timing and text", () => {
  const utterance = { utterance_id: "u-one", source_id: "microphone", start_ms: 1200, end_ms: 3500,
    speaker: "others", text: " First\n second\u0085third \t", voice: 2, failed: false };
  const value = segmentEvidenceValue(utterance);
  assert.deepEqual(value, ["u-one", "microphone", 1200, 3500, "First second third", false]);
  assert.deepEqual(value, segmentEvidenceValue({ ...utterance, speaker: "me", voice: null }));
  for (const change of [{ source_id: "system" }, { start_ms: 1400 }, { end_ms: 3600 }, { text: "Changed words" }, { utterance_id: "u-two" }]) {
    assert.notDeepEqual(value, segmentEvidenceValue({ ...utterance, ...change }));
  }
});

test("rich evidence uses Rust Unicode White_Space boundaries and preserves literal BOM characters", () => {
  const utterance = { utterance_id: "u-bom", source_id: "microphone", start_ms: 0, end_ms: 1000,
    speaker: "others", voice: null, failed: false, text: "\u00a0\ufeffalpha\u2003beta\n😀\ufeff\u00a0" };
  assert.deepEqual(segmentEvidenceValue(utterance), ["u-bom", "microphone", 0, 1000, "\ufeffalpha beta 😀\ufeff", false]);
});

test("a finished document keeps Summary and Your notes in one writing surface", () => {
  const panes = new LivePaneState(); panes.resize(1000); panes.setFinished(true);
  assert.equal(panes.shown("summary"), true); assert.equal(panes.shown("notes"), true);
  panes.select("summary"); assert.equal(panes.shown("notes"), true);
  panes.select("questions"); assert.equal(panes.shown("summary"), true); assert.equal(panes.shown("notes"), true);
  panes.resize(700); assert.equal(panes.shown("summary"), true); assert.equal(panes.shown("questions"), true);
  panes.closeCompanion(); assert.equal(panes.shown("summary"), true); assert.equal(panes.shown("notes"), true);
});

test("finished meetings default to Review while captures keep Transcript", () => {
  const panes = new LivePaneState(); panes.resize(1280);
  assert.equal(panes.companion, "transcript");
  panes.setFinished(true);
  assert.equal(panes.companion, "suggestions"); assert.equal(panes.companionOpen, true);
  panes.resize(760); assert.equal(panes.companion, "suggestions"); assert.equal(panes.companionOpen, false);
  panes.resize(1280); assert.equal(panes.companion, "suggestions"); assert.equal(panes.companionOpen, true);
});

test("finishing cannot replace an explicit companion or reopen a user's closed companion", () => {
  const transcript = new LivePaneState(); transcript.resize(1280); transcript.select("transcript"); transcript.setFinished(true);
  assert.equal(transcript.companion, "transcript"); assert.equal(transcript.companionOpen, true);
  const closed = new LivePaneState(); closed.resize(1280); closed.closeCompanion(); closed.setFinished(true); closed.resize(1400);
  assert.equal(closed.companionOpen, false);
  const questions = new LivePaneState(); questions.resize(760); questions.select("questions"); questions.setFinished(true);
  assert.equal(questions.companion, "questions"); assert.equal(questions.companionOpen, true);
});
