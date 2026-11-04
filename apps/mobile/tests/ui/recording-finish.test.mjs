import assert from 'node:assert/strict';
import test from 'node:test';
import { NoteSession } from '../../src/note-session.ts';
import { RecordingIdentity, stopThenSave } from '../../src/recording-finish.ts';

const meeting = (extra = {}) => ({ id: 'original-meeting', phase: 'recording', title: 'Owned recording', notes: 'Saved notes', transcript: [{ text: 'Owned fixture', start_ms: 0 }], pending: 0, elapsed_secs: 12, error: null, ...extra });

test('Stop reaches native capture before a failing note save and leaves the draft recoverable', async () => {
  let recording = true;
  const order = [];
  const draft = new NoteSession('Saved notes', async () => { order.push('save'); throw { code: 'storage', message: 'No space' }; });
  draft.edit('My unsaved notes');
  const result = await stopThenSave(async () => { order.push('stop'); recording = false; return true; }, draft.save);
  assert.equal(recording, false);
  assert.deepEqual(order, ['stop', 'save']);
  assert.deepEqual(result, { stopped: true, saved: false });
  assert.equal(draft.getSnapshot().draft, 'My unsaved notes');
  assert.equal(draft.getSnapshot().dirty, true);
});

test('recording identity and notes stay pinned when native capture becomes idle or another meeting starts', () => {
  const identity = new RecordingIdentity();
  identity.observe(meeting());
  const stopped = identity.observe(meeting({ id: null, phase: 'idle', notes: '', transcript: [], elapsed_secs: 0 }));
  assert.equal(stopped.id, 'original-meeting'); assert.equal(stopped.notes, 'Saved notes'); assert.equal(stopped.phase, 'idle');
  const another = identity.observe(meeting({ id: 'new-meeting', notes: 'Unrelated notes' }));
  assert.equal(another.id, 'original-meeting'); assert.equal(another.notes, 'Saved notes'); assert.equal(another.phase, 'idle');
});

test('a pre-existing autosave failure cannot prevent a later Stop request', async () => {
  const draft = new NoteSession('base', async () => false); draft.edit('still here'); await draft.save();
  let stopped = 0;
  const result = await stopThenSave(async () => { stopped += 1; return true; }, draft.save);
  assert.equal(stopped, 1); assert.equal(result.saved, false); assert.equal(draft.getSnapshot().draft, 'still here');
});

test('finalization errors remain visible only for the recording that produced them', () => {
  const identity = new RecordingIdentity(); identity.observe(meeting());
  const failed = identity.observe(meeting({ id: null, phase: 'idle', retained_id: 'original-meeting', error: 'Storage failed' }));
  assert.equal(failed.error, 'Storage failed');
  const repaired = identity.observe(meeting({ id: null, phase: 'idle', last_finished_id: 'original-meeting', error: null }));
  assert.equal(repaired.error, null);
  const unrelated = identity.observe(meeting({ id: null, phase: 'idle', last_finished_id: 'different-meeting', error: 'Unrelated error' }));
  assert.equal(unrelated.error, null);
});

test('only a correlated Stop reply can retarget a renamed meeting without replacing its draft identity', () => {
  const identity = new RecordingIdentity(); identity.observe(meeting());
  assert.equal(identity.retarget('unrelated', 'wrong-target'), false);
  assert.equal(identity.retarget('original-meeting', 'renamed-finished-meeting'), true);
  assert.equal(identity.key, 'original-meeting');
  assert.equal(identity.observe(meeting({ id: null, phase: 'idle' })).id, 'renamed-finished-meeting');
});

test('successful Stop flushes notes before the screen can navigate, while a Stop failure is not reported as stopped', async () => {
  const draft = new NoteSession('base', async () => true); draft.edit('latest');
  assert.deepEqual(await stopThenSave(async () => true, draft.save), { stopped: true, saved: true });
  assert.equal(draft.getSnapshot().dirty, false);
  assert.deepEqual(await stopThenSave(async () => { throw new Error('native unavailable'); }, async () => true), { stopped: false, saved: true });
});

test('a reopened failed-save recording keeps its retained identity and native notes', () => {
  const identity = new RecordingIdentity();
  const retained = identity.observe(meeting({ id: null, retained_id: 'retained-recording', phase: 'idle', notes: 'Native retained notes', error: 'Save failed' }));
  assert.equal(retained.id, 'retained-recording'); assert.equal(identity.key, 'retained-recording');
  assert.equal(retained.notes, 'Native retained notes'); assert.equal(retained.error, 'Save failed');
});

test('Stop hands off the latest text, acknowledged source and any owned in-flight save without waiting', async () => {
  let finish;
  const waiting = new Promise(resolve => { finish = resolve; });
  const draft = new NoteSession('Acknowledged text', async () => waiting);
  draft.edit('Owned pending text'); const saving = draft.save();
  draft.edit('Last words before Stop');
  const handoff = draft.captureHandoff();
  assert.deepEqual(handoff, { notes: 'Last words before Stop', expected_notes: 'Acknowledged text', pending_notes: 'Owned pending text' });
  // Stop receives this value before the earlier storage request resolves.
  finish(false); assert.equal(await saving, false);
  assert.equal(draft.acknowledgeHandoff(handoff, handoff.notes), true);
  assert.equal(draft.getSnapshot().dirty, false); assert.equal(draft.getSnapshot().failed, false);
});

test('native handoff acknowledgment preserves notes typed after Stop and cannot bless unrelated remote edits', () => {
  const draft = new NoteSession('base', async () => true);
  draft.edit('Captured at Stop'); const handoff = draft.captureHandoff();
  draft.edit('Typed after Stop');
  assert.equal(draft.acknowledgeHandoff(handoff, 'Captured at Stop'), true);
  assert.equal(draft.getSnapshot().baseline, 'Captured at Stop');
  assert.equal(draft.getSnapshot().draft, 'Typed after Stop'); assert.equal(draft.getSnapshot().dirty, true);
  assert.equal(draft.acknowledgeHandoff(handoff, 'Changed elsewhere'), false);
  assert.equal(draft.getSnapshot().draft, 'Typed after Stop'); assert.equal(draft.getSnapshot().conflict, true);
});
