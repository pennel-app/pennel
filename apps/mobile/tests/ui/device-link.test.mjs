import assert from 'node:assert/strict';
import test from 'node:test';
import { LinkAvailabilityProbe, connectionFailureFor, localizedConnectionFailure } from '../../src/device-link.ts';

function deferred() {
  let resolve; let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('availability distinguishes disabled admission from a transient failed check and recovers', async () => {
  const answers = [{ available: false, reason_code: 'pairing_unavailable' }, new Error('private diagnostic'), { available: true, reason_code: null }];
  const probe = new LinkAvailabilityProbe(async () => {
    const answer = answers.shift();
    if (answer instanceof Error) throw answer;
    return answer;
  });
  await probe.refresh();
  assert.deepEqual(probe.getSnapshot(), { value: { available: false, reason_code: 'pairing_unavailable' }, error: null, checking: false });
  await probe.refresh();
  assert.equal(probe.getSnapshot().value, null);
  assert.equal(probe.getSnapshot().error.code, 'operation_failed');
  await probe.refresh();
  assert.deepEqual(probe.getSnapshot(), { value: { available: true, reason_code: null }, error: null, checking: false });
});

test('newer checks and account/screen cancellation discard stale availability and errors', async () => {
  const first = deferred(); const next = deferred(); const late = deferred();
  const answers = [first, next, late];
  const probe = new LinkAvailabilityProbe(() => answers.shift().promise);
  const oldRun = probe.refresh();
  const newRun = probe.refresh();
  next.resolve({ available: false, reason_code: 'pairing_incompatible' });
  await newRun;
  first.resolve({ available: true, reason_code: null });
  await oldRun;
  assert.equal(probe.getSnapshot().value.reason_code, 'pairing_incompatible');
  const cancelled = probe.refresh();
  probe.reset();
  late.reject({ code: 'network', message: 'stale private diagnostic' });
  await cancelled;
  assert.deepEqual(probe.getSnapshot(), { value: null, error: null, checking: false });
});

test('the latest action owns the single connection error and cached failures remain actionable', () => {
  const cached = { phase: 'failed', error_code: 'pairing_unavailable', error_message: 'private service text' };
  const latest = { code: 'network', message: 'private network diagnostic' };
  assert.equal(connectionFailureFor(cached, latest), latest);
  assert.deepEqual(connectionFailureFor(cached, null), { code: 'pairing_unavailable', message: '' });
  assert.equal(connectionFailureFor({ phase: 'waiting_peer', error_code: null, error_message: null }, null), null);
  assert.equal(connectionFailureFor(null, null), null);
  assert.equal(connectionFailureFor({ phase: 'failed', error_code: null, error_message: null }, null).code, 'operation_failed');
  assert.equal(connectionFailureFor({ phase: 'expired', error_code: null, error_message: null }, null).code, 'pairing_expired');
});

test('connection errors distinguish service access from provider keys and never show upstream bodies', () => {
  for (const language of ['en', 'nl']) {
    for (const code of ['pairing_unavailable', 'pairing_incompatible', 'http_403', 'http_502', 'network']) {
      const text = localizedConnectionFailure(language, { code, message: 'private upstream response' });
      assert.ok(!text.includes('private upstream response'));
      assert.notEqual(text, localizedConnectionFailure(language === 'en' ? 'nl' : 'en', { code, message: '' }));
    }
  }
  assert.match(localizedConnectionFailure('en', { code: 'pairing_incompatible', message: '' }), /update.*email.*recovery/i);
  assert.match(localizedConnectionFailure('en', { code: 'http_403', message: '' }), /service.*access/i);
  assert.doesNotMatch(localizedConnectionFailure('en', { code: 'http_403', message: '' }), /API key/i);
});
