import assert from "node:assert/strict";
import test from "node:test";
import { MeetingDraft } from "../src/meeting-draft.ts";
import { SerialQueue } from "../src/async.ts";

test("opening a running meeting restores the stored notes", () => {
  const draft = new MeetingDraft();
  draft.sync("meeting-a", "Before navigation");
  assert.equal(draft.value, "Before navigation");
  assert.equal(draft.dirty, false);
});

test("polling cannot replace unsaved notes", () => {
  const draft = new MeetingDraft();
  draft.sync("meeting-a", "Saved");
  draft.edit("Still typing");
  draft.sync("meeting-a", "Saved");
  assert.equal(draft.value, "Still typing");
  assert.equal(draft.dirty, true);
});

test("a save that finishes after another edit does not mark that edit saved", () => {
  const draft = new MeetingDraft();
  draft.sync("meeting-a", "");
  draft.edit("First edit");
  draft.edit("Second edit");
  draft.acknowledge("meeting-a", "First edit");
  assert.equal(draft.dirty, true);
  draft.acknowledge("meeting-a", "Second edit");
  assert.equal(draft.dirty, false);
});

test("an old meeting's save cannot acknowledge the next meeting's draft", () => {
  const draft = new MeetingDraft();
  draft.sync("meeting-a", "Same text");
  draft.sync("meeting-b", "Same text");
  draft.edit("Same text");
  draft.acknowledge("meeting-a", "Same text");
  assert.equal(draft.sessionId, "meeting-b");
  assert.equal(draft.dirty, true);
});

test("failed persistence retains the unsaved draft for an exit retry", async () => {
  const draft = new MeetingDraft();
  draft.sync("meeting-a", "Before");
  draft.edit("Unsaved note");
  await assert.rejects(draft.persist(async () => { throw new Error("disk full"); }), /disk full/);
  assert.equal(draft.dirty, true);
  assert.equal(draft.value, "Unsaved note");
  let stored;
  await draft.persist(async (value) => { stored = value; });
  assert.equal(stored, "Unsaved note");
  assert.equal(draft.dirty, false);
});

test("queued persistence keeps the old session snapshot when the next meeting starts", async () => {
  const queue = new SerialQueue();
  let release;
  const waiting = new Promise((resolve) => { release = resolve; });
  queue.run(async () => { await waiting; });
  const draft = new MeetingDraft();
  draft.sync("meeting-a", "");
  draft.edit("First meeting note");
  const stored = [];
  const saving = draft.persist((value) => queue.run(async () => { stored.push(value); }));
  draft.sync("meeting-b", "");
  draft.edit("Second meeting note");
  release();
  await saving;
  assert.deepEqual(stored, ["First meeting note"]);
  assert.equal(draft.value, "Second meeting note");
  assert.equal(draft.dirty, true);
});

test("confirmed notes remain separate from a newer draft and failed storage", async () => {
  const draft = new MeetingDraft();
  draft.sync("session", "stored");
  draft.edit("sent");
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const pending = draft.persist(async () => blocked);
  assert.equal(draft.status, "saving");
  draft.edit("newer input");
  release();
  await pending;
  assert.equal(draft.acknowledgedValue, "sent");
  assert.equal(draft.value, "newer input");
  assert.equal(draft.status, "unsaved");
  await assert.rejects(draft.persist(async () => { throw Error("full"); }));
  assert.equal(draft.acknowledgedValue, "sent");
  assert.equal(draft.value, "newer input");
  assert.equal(draft.status, "unsaved");
});

test("draft writes are serialized without capturing text typed later", async () => {
  const draft = new MeetingDraft();
  draft.sync("session", "stored");
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const writes = [];
  draft.edit("first");
  const first = draft.persist(async value => { writes.push(value); await blocked; });
  draft.edit("second");
  const second = draft.persist(async value => { writes.push(value); });
  await Promise.resolve();
  assert.deepEqual(writes, ["first"]);
  release(); await Promise.all([first, second]);
  assert.deepEqual(writes, ["first", "second"]);
  assert.equal(draft.acknowledgedValue, "second");
  assert.equal(draft.status, "saved");
});
