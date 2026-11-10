import test from 'node:test';
import assert from 'node:assert/strict';
import { PillModelState } from '../src/pill-model-state.ts';

const model = (provider_id = 'apple', name = 'Apple dictation') => ({ choice: { provider_id, model: 'dictation' }, label: name });
const event = (generation = 0, extra = {}) => ({ session_id: 'capture', generation, model: model(), available_models: 2, enabled: true, switched: false, ...extra });

test('model metadata belongs to its current capture and presentation generation', () => {
  const state = new PillModelState();
  state.begin('capture');
  assert.equal(state.receive(event(), 0), true);
  assert.equal(state.chip?.label, 'Apple dictation');
  assert.equal(state.receive(event(1, { model: model('batch', 'Speech on my server'), switched: true }), 10), true);
  assert.equal(state.receive(event(0, { model: model('obsolete', 'Old model') }), 20), false);
  assert.equal(state.receive(event(99, { session_id: 'previous' }), 20), false);
  assert.equal(state.chip?.label, 'Speech on my server');
  state.begin('next-capture');
  assert.equal(state.chip, null);
  assert.equal(state.receive(event(100), 30), false);
});

test('confirmation lasts 900 ms and duplicates cannot extend it', () => {
  const state = new PillModelState();
  state.begin('capture');
  state.receive(event(), 0);
  assert.equal(state.confirmation(0), null);
  state.receive(event(1, { switched: true }), 100);
  assert.equal(state.confirmation(999), 'Apple dictation');
  state.receive(event(1, { switched: true }), 999);
  assert.equal(state.confirmation(1000), null);
  state.receive(event(2, { model: model('second', 'Second'), switched: true }), 1100);
  state.clear();
  assert.equal(state.confirmation(1101), null);
  assert.equal(state.chip, null);
});

test('the real toggle and available model count control the chip', () => {
  const state = new PillModelState();
  state.begin('capture');
  state.receive(event(0, { enabled: false }), 0);
  assert.equal(state.chip, null);
  state.receive(event(0, { available_models: 1 }), 1);
  assert.equal(state.chip, null);
  state.receive(event(), 2);
  assert.equal(state.chip?.label, 'Apple dictation');
  state.receive(event(1, { switched: true }), 3);
  state.clearConfirmation();
  assert.equal(state.confirmation(4), null);
  assert.equal(state.chip?.label, 'Apple dictation');
});

test('malformed generations and empty model identity never replace presentation', () => {
  const state = new PillModelState(); state.begin('capture');
  for (const value of [event(-1), event(NaN), event(1.1), event(0, { model: model('', 'Missing identity') })]) assert.equal(state.receive(value, 0), false);
  assert.equal(state.chip, null);
});
