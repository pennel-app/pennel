import assert from 'node:assert/strict';
import test from 'node:test';
import { participantGroups, participantHint, noteSelectionBytes, samePeopleSelection, canAcknowledgePeopleNotes, speakerLinkUtteranceIds, PeopleLoadEpoch } from '../src/people-state.ts';

const person = (id, kind, name, extra = {}) => ({ id, origin: kind, display_name: name, source_status: 'snapshot', ...extra });

test('participant search preserves duplicate identities and places meeting people before chosen contacts', () => {
  const people = [person('contact-noor', 'contact', 'Noor', { email: 'private@example.test' }), person('attendee-one', 'attendee', 'Noor'), person('attendee-two', 'attendee', 'Noor'), person('manual', 'manual', 'Guest')];
  assert.deepEqual(participantGroups(people, '').map(group => [group.kind, group.people.map(item => item.id)]), [['meeting', ['attendee-one', 'attendee-two', 'manual']], ['contacts', ['contact-noor']]]);
  assert.deepEqual(participantGroups(people, 'private@')[0].people.map(item => item.id), ['contact-noor']);
  assert.equal(people[0].display_name, 'Noor');
});

test('mention selection converts UTF-16 selection to exact UTF-8 bytes without changing Markdown', () => {
  const text = '# 🖋️\r\n**Noor** and `Noor`\r\n';
  const start = text.indexOf('Noor');
  const selection = noteSelectionBytes(text, start, start + 4);
  assert.deepEqual(selection, { start_byte: 13, end_byte: 17 });
  assert.equal(text, '# 🖋️\r\n**Noor** and `Noor`\r\n');
  assert.equal(noteSelectionBytes('🖋', 1, 2), null);
});

test('an asynchronous mention selection cannot replace changed text, selection or composition', () => {
  const before = { text: 'Ordinary Noor', revision: 4, selection_start: 9, selection_end: 13, selection_revision: 2, composing: false };
  assert.equal(samePeopleSelection(before, { ...before }), true);
  for (const changed of [{ text: 'Ordinary Els' }, { revision: 5 }, { selection_start: 0 }, { selection_revision: 3 }, { composing: true }]) assert.equal(samePeopleSelection(before, { ...before, ...changed }), false);
});

test('a durable mention is acknowledged after selection changes only in its captured draft', () => {
  const before = { text: 'Meeting notes', revision: 4, selection_start: 0, selection_end: 0, selection_revision: 2, composing: false };
  const saved = '@Noor Meeting notes';
  assert.equal(canAcknowledgePeopleNotes(before, { ...before, selection_start: 3, selection_end: 7, selection_revision: 8 }, saved, 'owner-one', 'owner-one'), true);
  assert.equal(canAcknowledgePeopleNotes(before, { ...before, text: saved }, saved, 'owner-one', 'owner-one'), true);
  assert.equal(canAcknowledgePeopleNotes(before, { ...before }, saved, 'owner-one', 'owner-two'), false);
  assert.equal(canAcknowledgePeopleNotes(before, { ...before }, saved, null, null), false);
  assert.equal(canAcknowledgePeopleNotes(before, { ...before, text: 'Newer user text' }, saved, 'owner-one', 'owner-one'), false);
  assert.equal(canAcknowledgePeopleNotes(before, { ...before, composing: true }, saved, 'owner-one', 'owner-one'), false);
});

test('a late picker load cannot apply after closing or changing meeting identity', () => {
  const epoch = new PeopleLoadEpoch();
  const first = epoch.begin('uid-one', 'session-one');
  assert.equal(epoch.accepts(first, 'uid-one', 'session-one'), true);
  epoch.begin('uid-two', 'session-two');
  assert.equal(epoch.accepts(first, 'uid-one', 'session-one'), false);
  const second = epoch.begin('uid-two', 'session-two');
  epoch.invalidate();
  assert.equal(epoch.accepts(second, 'uid-two', 'session-two'), false);
});

test('speaker links capture stable turns without grouping unnamed or overlapping speakers', () => {
  const first = { utterance_id: 'one', speaker: 'others', voice: 2 };
  const second = { ...first, utterance_id: 'two' };
  const unknown = { utterance_id: 'three', speaker: 'me', voice: null };
  const overlap = { ...first, utterance_id: 'four', attribution: { kind: 'overlap', voices: [2, 3] } };
  const turns = [first, second, unknown, { ...unknown, utterance_id: 'five' }, overlap];
  assert.deepEqual(speakerLinkUtteranceIds(first, turns), ['one', 'two']);
  assert.deepEqual(speakerLinkUtteranceIds(unknown, turns), ['three']);
  assert.deepEqual(speakerLinkUtteranceIds(overlap, turns), []);
  assert.deepEqual(speakerLinkUtteranceIds({ ...first, utterance_id: null }, turns), []);
});

test('duplicate names get separate hints and calendar status requires a unique source record', () => {
  const one = person('one', 'attendee', 'Alex', { source: { source_id: 'source', account_id: null, record_id: 'one', revision: 'v1' } });
  const two = person('two', 'attendee', 'Alex', { source: { ...one.source, record_id: 'two' } });
  const calendar = { source: 'eventkit', source_id: 'source', account_id: null, participants: [{ source_id: 'one', is_self: true, response: 'declined' }] };
  assert.deepEqual(participantHint(one, [one, two], calendar), { ordinal: 1, is_self: true, response: 'declined' });
  assert.deepEqual(participantHint(two, [one, two], calendar), { ordinal: 2, is_self: false, response: null });
  assert.deepEqual(participantHint({ ...one, source_status: 'unavailable' }, [one, two], calendar), { ordinal: 1, is_self: false, response: null });
  assert.deepEqual(participantHint(one, [one, two], { ...calendar, participants: [...calendar.participants, ...calendar.participants] }), { ordinal: 1, is_self: false, response: null });
});
