import assert from 'node:assert/strict';
import test from 'node:test';
import { hasRecordingToReturnTo, RecordingSheetReturn } from '../../src/recording-finish.ts';

test('an older meeting sheet keeps awareness of a different active or retained recording', () => {
  assert.equal(hasRecordingToReturnTo({ id: 'different-recording', phase: 'recording', retained_id: null }), true);
  assert.equal(hasRecordingToReturnTo({ id: 'different-recording', phase: 'finishing', retained_id: null }), true);
  assert.equal(hasRecordingToReturnTo({ id: null, phase: 'idle', retained_id: 'retained-recording' }), true);
  assert.equal(hasRecordingToReturnTo({ id: null, phase: 'idle', retained_id: null }), false);
});

test('Return dismisses the sheet first and navigates only after its native dismissal event', async () => {
  const controller = new RecordingSheetReturn();
  const events = [];
  const dismiss = () => events.push('hide sheet');
  controller.request(dismiss); controller.request(dismiss);
  await Promise.resolve();
  assert.deepEqual(events, ['hide sheet']);
  assert.equal(controller.didDismiss(() => events.push('close owner'), () => events.push('return to recording')), true);
  assert.deepEqual(events, ['hide sheet', 'close owner', 'return to recording']);
  assert.equal(controller.didDismiss(() => events.push('duplicate close'), () => events.push('duplicate route')), false);
  assert.equal(events.length, 3);
});

test('ordinary sheet closure does not navigate and the next presentation can return safely', () => {
  const controller = new RecordingSheetReturn();
  let returned = 0;
  assert.equal(controller.didDismiss(() => {}, () => returned++), false);
  controller.request(() => {});
  controller.didDismiss(() => {}, () => returned++);
  controller.request(() => {});
  controller.didDismiss(() => {}, () => returned++);
  assert.equal(returned, 2);
});

test('a recording that finishes during sheet dismissal is not reopened as an empty recorder', () => {
  const controller = new RecordingSheetReturn();
  let current = { id: 'active', phase: 'recording', retained_id: null };
  let closed = false; let returned = false;
  controller.request(() => {});
  current = { id: null, phase: 'idle', retained_id: null };
  controller.didDismiss(() => { closed = true; }, () => { if (hasRecordingToReturnTo(current)) returned = true; });
  assert.equal(closed, true); assert.equal(returned, false);
});
