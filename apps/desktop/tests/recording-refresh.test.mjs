import assert from "node:assert/strict";
import test from "node:test";
import { RecordingRefresh, segmentKeys, sameSegment } from "../src/recording-refresh.ts";

const segment = { start_ms: 100, end_ms: 300, speaker: "others", text: "First", voice: null, failed: false };
const snapshot = (revision = 1, changes = {}) => ({ session_id: "session", id: "meeting", busy: true, phase: "recording", transcript_revision: revision, notes: "notes", transcript: [segment], ...changes });
const metadata = (revision = 1, changes = {}) => {
  const { notes, transcript, ...runtime } = snapshot(revision, changes);
  return runtime;
};
function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { resolve, promise };
}

test("unchanged metadata retains the transcript and does not load its content", async () => {
  const initial = snapshot();
  const refresh = new RecordingRefresh(initial);
  const next = await refresh.refresh(metadata(1, { elapsed_secs: 5 }), () => { throw new Error("unexpected transcript read"); });
  assert.equal(next.transcript, initial.transcript);
  assert.equal(next.notes, "notes");
  assert.equal(next.elapsed_secs, 5);
  assert.equal(next.transcript_revision, 1);
});

test("overlapping revision reads share a request and only the latest metadata is delivered", async () => {
  const refresh = new RecordingRefresh(snapshot());
  const waiting = deferred();
  let loads = 0;
  const load = () => { loads++; return waiting.promise; };
  const old = refresh.refresh(metadata(2, { elapsed_secs: 2 }), load);
  const recent = refresh.refresh(metadata(3, { elapsed_secs: 3 }), load);
  waiting.resolve(snapshot(3, { transcript: [segment, { ...segment, start_ms: 500, text: "Second" }] }));
  assert.equal(await old, null);
  const next = await recent;
  assert.equal(loads, 1);
  assert.equal(next.elapsed_secs, 3);
  assert.equal(next.transcript.length, 2);
  await refresh.refresh(metadata(3), () => { throw new Error("already current"); });
});

test("a lagging full snapshot never acknowledges a revision it has not loaded", async () => {
  const refresh = new RecordingRefresh(snapshot());
  const next = await refresh.refresh(metadata(3), async () => snapshot(2));
  assert.equal(next.transcript_revision, 2);
  let loads = 0;
  await refresh.refresh(metadata(3), async () => { loads++; return snapshot(3); });
  assert.equal(loads, 1);
});

test("late snapshots from another session or a disposed view cannot replace content", async () => {
  const refresh = new RecordingRefresh(snapshot());
  assert.equal(await refresh.refresh(metadata(2), async () => snapshot(2, { session_id: "other" })), null);
  const waiting = deferred();
  const work = refresh.refresh(metadata(2), () => waiting.promise);
  refresh.dispose();
  waiting.resolve(snapshot(2));
  assert.equal(await work, null);
});

test("a failed transcript read can be retried without acknowledging its revision", async () => {
  const refresh = new RecordingRefresh(snapshot());
  await assert.rejects(refresh.refresh(metadata(2), async () => { throw new Error("read failed"); }), /read failed/);
  const next = await refresh.refresh(metadata(2), async () => snapshot(2));
  assert.equal(next.transcript_revision, 2);
});

test("row identity handles simultaneous tracks and duplicate timestamps without including mutable text", () => {
  const values = [segment, { ...segment, speaker: "me" }, { ...segment, text: "Duplicate" }];
  const keys = segmentKeys(values);
  assert.equal(new Set(keys).size, 3);
  assert.deepEqual(keys, segmentKeys(values.map(value => ({ ...value, text: value.text + " updated", end_ms: 900 }))));
  assert.equal(sameSegment(segment, { ...segment }), true);
  for (const change of [{ text: "New" }, { failed: true }, { voice: 2 }, { end_ms: 500 }, { speaker: "me" }]) {
    assert.equal(sameSegment(segment, { ...segment, ...change }), false);
  }
});

test("stable UID and session permit a folder rename without replacing the transcript", async () => {
  const initial = snapshot(1, { meeting_uid: "uid" });
  const refresh = new RecordingRefresh(initial);
  const renamed = await refresh.refresh(metadata(1, { meeting_uid: "uid", id: "renamed" }), async () => { throw Error("unexpected read"); });
  assert.equal(renamed.id, "renamed");
  assert.equal(renamed.transcript, initial.transcript);
  assert.equal(await refresh.refresh(metadata(2, { meeting_uid: "other", id: "renamed" }), async () => snapshot(2)), null);
});
