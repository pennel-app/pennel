import assert from 'node:assert/strict';
import test from 'node:test';
import { clock, meetingGroups, recognitionActions } from '../../src/format.ts';
import { NoteSession } from '../../src/note-session.ts';
import { rememberSource, resolveSource } from '../../src/source-navigation.ts';

test('notes preserve exact text and reject an edit based on a stale version', async () => {
  const writes = [];
  const session = new NoteSession('base', async (notes, expected) => { writes.push({ notes, expected }); return true; });
  const draft = 'My note\n\n- follow up\n';
  session.edit(draft); assert.equal(await session.save(), true);
  assert.deepEqual(writes, [{ notes: draft, expected: 'base' }]);
  session.edit('My pending changes'); session.adoptRemote('Changed on the Mac');
  assert.equal(await session.save(), false);
  assert.equal(writes.length, 1);
  assert.equal(session.getSnapshot().draft, 'My pending changes');
  assert.equal(session.getSnapshot().conflict, true);
});

test('notes typed during an in-flight save are queued with the newly saved baseline', async () => {
  let finish;
  const first = new Promise((resolve) => { finish = resolve; });
  const writes = [];
  const session = new NoteSession('base', async (notes, expected) => { writes.push({ notes, expected }); return writes.length === 1 ? first : true; });
  session.edit('first edit'); const save = session.save();
  session.edit('latest edit'); finish(true); assert.equal(await save, true);
  assert.deepEqual(writes, [{ notes: 'first edit', expected: 'base' }, { notes: 'latest edit', expected: 'first edit' }]);
  assert.equal(session.getSnapshot().dirty, false);
  assert.equal(session.getSnapshot().baseline, 'latest edit');
});

test('failed note saving retains the draft and can be retried explicitly', async () => {
  let available = false;
  const session = new NoteSession('base', async () => available);
  session.edit('saved later'); assert.equal(await session.save(), false);
  assert.equal(session.getSnapshot().draft, 'saved later');
  assert.equal(session.getSnapshot().failed, true);
  available = true; assert.equal(await session.save(), true);
  assert.equal(session.getSnapshot().failed, false); assert.equal(session.getSnapshot().dirty, false);
});

test('unknown server outcome requires confirmation and finalization cannot be cancelled', () => {
  assert.deepEqual(recognitionActions({ stage: 'response_unknown', recoverable: true }), { cancel: true, resume: true, confirmRetry: true });
  assert.deepEqual(recognitionActions({ stage: 'interrupted', recoverable: true }), { cancel: true, resume: true, confirmRetry: false });
  assert.deepEqual(recognitionActions({ stage: 'applying', recoverable: false }), { cancel: false, resume: false, confirmRetry: false });
  assert.deepEqual(recognitionActions({ stage: 'cancelled', recoverable: false }), { cancel: false, resume: false, confirmRetry: false });
});

test('source navigation validates the cited passage without putting its text in a route token', () => {
  const passage = { meeting_id: 'one', title: 'Owned fixture', kind: 'transcript', text: 'A verified decision.', start_ms: 800 };
  const token = rememberSource(passage, 100);
  assert.equal(token.includes(passage.text), false);
  const detail = { info: { id: 'one' }, notes: '', summary: null, transcript: [{ text: 'A verified decision.', start_ms: 800 }] };
  assert.equal(resolveSource(token, detail, 200).changed, false);
  assert.equal(resolveSource(token, { ...detail, transcript: [{ text: 'An edited decision.' }] }, 200).changed, true);
  assert.equal(resolveSource(token, { ...detail, info: { id: 'different' } }, 200).changed, true);
  assert.equal(resolveSource(token, detail, 31 * 60 * 1000).changed, true);
});

test('meeting groups use local day boundaries and stay newest first', () => {
  const now = new Date(2026, 9, 27, 12);
  const meeting = (id, date) => ({ id, title: id, start_iso: date.toISOString(), duration_min: 5, has_summary: false });
  const groups = meetingGroups([meeting('older', new Date(2026, 9, 1)), meeting('morning', new Date(2026, 9, 27, 9)), meeting('yesterday', new Date(2026, 9, 26, 18)), meeting('afternoon', new Date(2026, 9, 27, 13))], now);
  assert.deepEqual(groups.map((group) => [group.key, group.meetings.map((meeting) => meeting.id)]), [['today', ['afternoon', 'morning']], ['week', ['yesterday']], ['earlier', ['older']]]);
  assert.equal(clock(-5), '0:00'); assert.equal(clock(3723), '62:03');
});

test('fresh core transcript citations include stable named and own-speaker prefixes', () => {
  const detail = { info: { id: 'native-fixture' }, speakers: [{ voice: 12, name: 'Chris' }], transcript: [
    { start_ms: 0, end_ms: 5000, speaker: 'others', voice: 12, failed: false, text: 'A verified decision.' },
    { start_ms: 5000, end_ms: 9000, speaker: 'me', voice: null, failed: false, text: 'I will follow up.' },
  ] };
  const passage = { meeting_id: detail.info.id, title: 'Owned fixture', kind: 'transcript', start_ms: 0, text: 'Chris: A verified decision.\nIk: I will follow up.' };
  const token = rememberSource(passage, 100);
  assert.equal(resolveSource(token, detail, 200).changed, false);
  assert.equal(resolveSource(token, detail, 200).transcriptIndex, 0);
  assert.equal(resolveSource(token, { ...detail, speakers: [{ voice: 12, name: 'Changed name' }] }, 200).changed, true);
});

test('unknown multi-digit speaker prefixes and failed chunks match the core source contract', () => {
  const detail = { info: { id: 'native-fixture' }, speakers: [], transcript: [
    { start_ms: 0, end_ms: 1000, speaker: 'others', voice: 12, failed: false, text: 'First decision.' },
    { start_ms: 1000, end_ms: 2000, speaker: 'others', voice: null, failed: true, text: 'Failed chunk.' },
    { start_ms: 2000, end_ms: 3000, speaker: 'others', voice: null, failed: false, text: 'Next step.' },
  ] };
  const token = rememberSource({ meeting_id: detail.info.id, kind: 'transcript', start_ms: 0, text: 'Anderen 12: First decision.\nAnderen: Next step.' }, 100);
  assert.equal(resolveSource(token, detail, 200).changed, false);
  const later = rememberSource({ meeting_id: detail.info.id, kind: 'transcript', start_ms: 2000, text: 'Anderen: Next step.' }, 100);
  assert.equal(resolveSource(later, detail, 200).transcriptIndex, 2);
});

test('a changed cited turn cannot match identical words from a later turn', () => {
  const detail = { info: { id: 'native-fixture' }, speakers: [], transcript: [
    { start_ms: 0, end_ms: 1000, speaker: 'others', voice: null, failed: false, text: 'Changed original.' },
    { start_ms: 1000, end_ms: 2000, speaker: 'others', voice: null, failed: false, text: 'Repeated decision.' },
  ] };
  const token = rememberSource({ meeting_id: detail.info.id, kind: 'transcript', start_ms: 0, text: 'Anderen: Repeated decision.' }, 100);
  assert.equal(resolveSource(token, detail, 200).changed, true);
});

test('a long utterance citation can begin after the core split its first labelled chunk', () => {
  const detail = { info: { id: 'native-fixture' }, speakers: [], transcript: [
    { start_ms: 5000, end_ms: 10000, speaker: 'others', voice: null, failed: false, text: 'First part. Later unlabelled source chunk.' },
  ] };
  const token = rememberSource({ meeting_id: detail.info.id, kind: 'transcript', start_ms: 5000, text: 'Later unlabelled source chunk.' }, 100);
  assert.equal(resolveSource(token, detail, 200).changed, false);
});

test('separate core passages may start on different tracks at the same timestamp', () => {
  const detail = { info: { id: 'native-fixture' }, speakers: [], transcript: [
    { start_ms: 0, end_ms: 5000, speaker: 'me', voice: null, failed: false, text: 'x'.repeat(596) },
    { start_ms: 0, end_ms: 5000, speaker: 'others', voice: 12, failed: false, text: 'A fresh second track.' },
  ] };
  const token = rememberSource({ meeting_id: detail.info.id, kind: 'transcript', start_ms: 0, text: 'Anderen 12: A fresh second track.' }, 100);
  assert.equal(resolveSource(token, detail, 200).changed, false);
  assert.equal(resolveSource(token, detail, 200).transcriptIndex, 1);
});

test('durable mobile sources survive speaker changes and meeting folder renames', () => {
  const segment = { utterance_id: 'u-one', source_id: 'microphone', source_fingerprint: 'a'.repeat(64), start_ms: 500,
    end_ms: 1200, speaker: 'others', voice: 2, failed: false, text: 'A verified decision.' };
  const detail = { info: { id: 'renamed-folder' }, meeting_uid: 'stable-meeting', metadata_status: 'valid', speakers: [{ voice: 2, name: 'New name' }], transcript: [segment] };
  const passage = { meeting_id: 'old-folder', meeting_uid: 'stable-meeting', title: 'Owned fixture', kind: 'transcript', start_ms: 500,
    text: 'Earlier name: A verified decision.', utterance_ids: ['u-one'], utterance_fingerprints: ['a'.repeat(64)] };
  const token = rememberSource(passage, 100);
  assert.equal(resolveSource(token, detail, 200).changed, false);
  assert.equal(resolveSource(token, { ...detail, transcript: [{ ...segment, speaker: 'me', voice: null }] }, 200).transcriptIndex, 0);
  for (const change of [{ meeting_uid: 'another-meeting' }, { metadata_status: 'stale' }, { metadata_status: 'unsupported' },
    { transcript: [{ ...segment, source_fingerprint: 'b'.repeat(64) }] }, { transcript: [{ ...segment, utterance_id: 'replacement' }] },
    { transcript: [segment, segment] }]) {
    assert.equal(resolveSource(token, { ...detail, ...change }, 200).changed, true);
  }
});

test('a durable source never falls back to identical words on a different utterance', () => {
  const detail = { info: { id: 'same-folder' }, meeting_uid: 'stable-meeting', metadata_status: 'valid', speakers: [], transcript: [
    { utterance_id: 'different-utterance', source_fingerprint: 'a'.repeat(64), start_ms: 500, end_ms: 1200, speaker: 'others', voice: null, failed: false, text: 'Repeated words.' },
  ] };
  const passage = { meeting_id: detail.info.id, meeting_uid: detail.meeting_uid, kind: 'transcript', start_ms: 500, text: 'Anderen: Repeated words.',
    utterance_ids: ['missing-utterance'], utterance_fingerprints: ['a'.repeat(64)] };
  const resolved = resolveSource(rememberSource(passage, 100), detail, 200);
  assert.equal(resolved.changed, true);
  assert.equal(resolved.passage.text, passage.text);
});

test('remembering a durable citation freezes its source identity arrays and quoted text', () => {
  const segment = { utterance_id: 'original', source_fingerprint: 'a'.repeat(64), start_ms: 0, end_ms: 1000, text: 'Original words.' };
  const detail = { info: { id: 'meeting' }, meeting_uid: 'uid', metadata_status: 'valid', transcript: [segment] };
  const passage = { meeting_id: 'meeting', meeting_uid: 'uid', kind: 'transcript', start_ms: 0, text: 'Frozen original words.',
    utterance_ids: ['original'], utterance_fingerprints: ['a'.repeat(64)] };
  const token = rememberSource(passage, 100);
  passage.text = 'Later words'; passage.utterance_ids[0] = 'later'; passage.utterance_fingerprints[0] = 'b'.repeat(64);
  const resolved = resolveSource(token, detail, 200);
  assert.equal(resolved.changed, false);
  assert.equal(resolved.passage.text, 'Frozen original words.');
});
