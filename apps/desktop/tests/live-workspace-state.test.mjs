import assert from "node:assert/strict";
import test from "node:test";
import { LivePaneState, formatSelection, LiveStatusEpoch, segmentEvidenceValue } from "../src/live-workspace-state.ts";

test("the deliberately selected pane survives wide and narrow transitions", () => {
  const panes = new LivePaneState();
  assert.equal(panes.selected, "notes");
  assert.equal(panes.companion, "transcript");
  assert.equal(panes.companionOpen, false);
  panes.resize(1000);
  assert.equal(panes.shown("notes"), true);
  assert.equal(panes.shown("transcript"), false);
  panes.select("questions"); panes.resize(900);
  assert.equal(panes.selected, "questions");
  panes.select("notes"); panes.resize(799);
  assert.equal(panes.selected, "notes");
  assert.equal(panes.companion, "questions");
  panes.resize(800);
  assert.equal(panes.wide, true);
  assert.equal(panes.companion, "questions");
  assert.equal(panes.companionOpen, true);
});

test("closing a companion survives resize without losing the selected writing surface", () => {
  const panes = new LivePaneState();
  panes.select("summary"); panes.select("questions"); panes.resize(1000);
  assert.equal(panes.shown("summary"), true);
  assert.equal(panes.shown("questions"), true);
  panes.closeCompanion(); panes.resize(700); panes.resize(1000);
  assert.equal(panes.primary, "summary");
  assert.equal(panes.selected, "summary");
  assert.equal(panes.companion, "questions");
  assert.equal(panes.shown("questions"), false);
  assert.equal(panes.shown("summary"), true);
});

test("a narrow companion is the same retained pane and returns to the selected notes or summary", () => {
  const panes = new LivePaneState();
  panes.select("summary"); panes.toggleCompanion("transcript"); panes.resize(700);
  assert.equal(panes.shown("summary"), false);
  assert.equal(panes.shown("transcript"), true);
  panes.resize(1000);
  assert.equal(panes.shown("summary"), true);
  assert.equal(panes.shown("transcript"), true);
  panes.resize(700); panes.toggleCompanion("transcript");
  assert.equal(panes.selected, "summary");
  assert.equal(panes.shown("summary"), true);
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
  panes.resize(700); assert.equal(panes.shown("summary"), false); assert.equal(panes.shown("questions"), true);
  panes.closeCompanion(); assert.equal(panes.shown("summary"), true); assert.equal(panes.shown("notes"), true);
});
