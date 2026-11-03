import assert from 'node:assert/strict';
import test from 'node:test';
import { NoteSession } from '../../src/note-session.ts';
import { InsightReviewModel } from '../../../../assets/contracts/insight-review-model.ts';
import { insightSourceMatches } from '../../../../assets/contracts/meeting-insights.ts';

test('accepted insertion uses the ordinary note writer and queues later typing without repeating the receipt', async () => {
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const writes = [];
  const notes = new NoteSession('Private notes', async (text, expected, receipt) => { writes.push({ text, expected, receipt }); return writes.length === 1 ? blocked : true; });
  assert.equal(notes.stageInsightInsertion('Private notes\n\nAccepted item', 'attempt', 'Private notes'), true);
  const saving = notes.save(); notes.edit('Private notes\n\nAccepted item\nUser kept typing.'); release(true);
  assert.equal(await saving, true);
  assert.deepEqual(writes, [{ text: 'Private notes\n\nAccepted item', expected: 'Private notes', receipt: 'attempt' },
    { text: 'Private notes\n\nAccepted item\nUser kept typing.', expected: 'Private notes\n\nAccepted item', receipt: undefined }]);
  assert.equal(notes.getSnapshot().dirty, false);
});

test('failed insertion note saves keep exact editable text and retry the same attempt', async () => {
  let available = false; const receipts = [];
  const notes = new NoteSession('Before', async (_text, _expected, receipt) => { receipts.push(receipt); return available; });
  notes.stageInsightInsertion('Before\n\nAccepted item', 'stable-attempt', 'Before');
  assert.equal(await notes.save(), false); assert.equal(notes.getSnapshot().draft, 'Before\n\nAccepted item');
  available = true; assert.equal(await notes.save(), true); assert.deepEqual(receipts, ['stable-attempt', 'stable-attempt']);
});

test('a stale insertion cannot replace dirty mobile notes or an unrelated source', () => {
  const notes = new NoteSession('Before', async () => true); notes.edit('Changed');
  assert.equal(notes.stageInsightInsertion('Overwritten', 'attempt', 'Before'), false);
  const source = { meeting_uid: 'one', utterance_id: 'source', source_fingerprint: 'fingerprint', start_byte: 0, end_byte: 5, quote: 'Words', start_ms: 0, end_ms: 1000 };
  const row = { utterance_id: 'source', source_fingerprint: 'fingerprint', text: 'Words', start_ms: 0, end_ms: 1000, failed: false };
  assert.equal(insightSourceMatches(source, row, 'two'), false);
});

test('mobile finished review cannot send analysis without an explicit qualified language setup', async () => {
  let starts = 0;
  const model = new InsightReviewModel('folder', 'uid', { read: async () => ({ meeting_uid: 'uid', revision: 0, status: 'missing', items: [], insertion_receipts: [] }),
    availability: async () => ({ available: false, qualified: false, reason_code: 'unqualified' }), start: async () => { starts++; } });
  await model.load(); await model.choosePolicy('unknown'); assert.equal(await model.analyze(), false);
  await model.choosePolicy('mixed'); assert.equal(await model.analyze(), false); assert.equal(starts, 0);
});
