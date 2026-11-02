import assert from "node:assert/strict";
import test from "node:test";
import { LivePaneState, formatSelection, LiveRequestState, LiveStatusEpoch, segmentEvidenceValue } from "../src/live-workspace-state.ts";

test("the deliberately selected pane survives wide and narrow transitions", () => {
  const panes = new LivePaneState();
  assert.equal(panes.selected, "notes");
  assert.equal(panes.companion, "transcript");
  panes.select("questions"); panes.resize(900);
  assert.equal(panes.selected, "questions");
  panes.select("notes"); panes.resize(799);
  assert.equal(panes.selected, "notes");
  assert.equal(panes.companion, "questions");
  panes.resize(800);
  assert.equal(panes.wide, true);
  assert.equal(panes.companion, "questions");
});

test("format helpers preserve source text and the selected content", () => {
  assert.deepEqual(formatSelection("one two", 4, 7, "bold"), { value: "one **two**", start: 6, end: 9 });
  assert.deepEqual(formatSelection("one\ntwo", 1, 6, "task"), { value: "- [ ] one\n- [ ] two", start: 7, end: 18 });
});

test("cancelled or replaced requests cannot accept a late result", () => {
  const state = new LiveRequestState("meeting", "session");
  state.begin("first");
  assert.equal(state.accepts("meeting", "session", "first"), true);
  state.cancel(); state.begin("second");
  assert.equal(state.accepts("meeting", "session", "first"), false);
  assert.equal(state.accepts("other", "session", "second"), false);
  assert.equal(state.accepts("meeting", "next", "second"), false);
  assert.equal(state.accepts("meeting", "session", "second"), true);
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
