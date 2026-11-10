import assert from "node:assert/strict";
import test from "node:test";
import { RecordingActionGate, RecordingLevels, RecordingStopGate } from "../src/recording-control-state.ts";

const active = session_id => ({ session_id, recording: true, phase: "recording" });
test("a recording start owns one pending action and releases it after failure", async () => {
  const gate = new RecordingActionGate();
  let release, starts = 0;
  const pending = gate.run(async () => { starts++; await new Promise(resolve => { release = resolve; }); });
  await gate.run(async () => { starts++; });
  assert.equal(starts, 1);
  assert.equal(gate.pending, true);
  release(); await pending;
  await assert.rejects(gate.run(async () => { throw Error("blocked"); }), /blocked/);
  assert.equal(gate.pending, false);
  await gate.run(async () => { starts++; });
  assert.equal(starts, 2);
});
test("only finite levels owned by the current capture can move the bars", () => {
  const levels = new RecordingLevels();
  levels.observe(active("first"));
  assert.equal(levels.accept({ session_id: "first", level: .7 }), true);
  assert.equal(levels.level, .7);
  assert.equal(levels.accept({ session_id: "old", level: 1 }), false);
  assert.equal(levels.accept({ session_id: "first", level: NaN }), false);
  assert.equal(levels.level, .7);
  levels.accept({ session_id: "first", level: 9 }); assert.equal(levels.level, 1);
  levels.accept({ session_id: "first", level: -2 }); assert.equal(levels.level, 0);
  levels.observe(active("second")); assert.equal(levels.level, 0);
  assert.equal(levels.accept({ session_id: "first", level: .9 }), false);
});
test("stopping clears the bars and a delayed recording poll cannot revive that session", () => {
  const levels = new RecordingLevels();
  levels.observe(active("first")); levels.accept({ session_id: "first", level: .7 });
  levels.stop("first"); assert.equal(levels.level, 0);
  levels.observe(active("first"));
  assert.equal(levels.accept({ session_id: "first", level: 1 }), false);
  levels.observe(active("second"));
  assert.equal(levels.accept({ session_id: "second", level: .5 }), true);
  levels.observe({ ...active("second"), recording: false, phase: "finalizing" });
  assert.equal(levels.level, 0);
  assert.equal(levels.accept({ session_id: "second", level: .9 }), false);
});
test("stop invokes capture exactly once per session and failed requests remain retryable", async () => {
  const gate = new RecordingStopGate(); let calls = 0, release;
  const pending = gate.run("first", async () => { calls++; await new Promise(resolve => { release = resolve; }); });
  const duplicate = gate.run("first", async () => { calls++; });
  assert.equal(calls, 1); release(); await Promise.all([pending, duplicate]);
  await gate.run("first", async () => { calls++; }); assert.equal(calls, 1);
  await assert.rejects(gate.run("second", async () => { throw Error("retry"); }), /retry/);
  await gate.run("second", async () => { calls++; }); assert.equal(calls, 2);
});

test("a delayed status from the stopped capture cannot close the next session's levels", () => {
  const levels = new RecordingLevels();
  levels.observe(active("first")); levels.stop("first");
  levels.observe(active("second")); levels.accept({ session_id: "second", level: .6 });
  levels.observe(active("first"));
  assert.equal(levels.level, .6);
  assert.equal(levels.accept({ session_id: "second", level: .8 }), true);
});

test("idle runtime without a session clears the last capture level", () => {
  const levels = new RecordingLevels();
  levels.observe(active("current")); levels.accept({ session_id: "current", level: .9 });
  levels.observe({ session_id: null, recording: false, phase: "idle" });
  assert.equal(levels.level, 0);
  assert.equal(levels.accept({ session_id: "current", level: 1 }), false);
});

test("retired capture identities remain rejected across multiple later recordings", () => {
  const levels = new RecordingLevels();
  levels.observe(active("A")); levels.stop("A");
  levels.observe(active("B")); levels.stop("B");
  levels.observe(active("C")); levels.accept({ session_id: "C", level: .6 });
  levels.observe(active("A"));
  assert.equal(levels.level, .6);
  assert.equal(levels.accept({ session_id: "A", level: 1 }), false);
  assert.equal(levels.accept({ session_id: "C", level: .8 }), true);
  levels.observe(active("B"));
  assert.equal(levels.accept({ session_id: "B", level: 1 }), false);
  assert.equal(levels.accept({ session_id: "C", level: .7 }), true);
});

for (const delayed of [{ session_id: null, recording: false, phase: "idle" }, { session_id: "C", recording: false, phase: "starting" }]) {
  test(`an ambiguous ${delayed.phase} status cannot permanently retire the still-live capture`, () => {
    const levels = new RecordingLevels();
    levels.observe(active("C")); levels.accept({ session_id: "C", level: .6 });
    levels.observe(delayed);
    assert.equal(levels.observe(active("C")), true);
    assert.equal(levels.accept({ session_id: "C", level: .8 }), true);
    assert.equal(levels.level, .8);
  });
}

test("a known replacement retires the previous capture even across an ambiguous snapshot", () => {
  const levels = new RecordingLevels();
  levels.observe(active("A"));
  levels.observe({ session_id: null, recording: false, phase: "idle" });
  levels.observe(active("B"));
  assert.equal(levels.observe(active("A")), false);
  assert.equal(levels.accept({ session_id: "B", level: .5 }), true);
});
