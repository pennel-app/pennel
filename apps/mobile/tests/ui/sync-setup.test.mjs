import assert from 'node:assert/strict';
import test from 'node:test';
import { syncSetupStage, syncStatusKey, linkIsPending, requiresRecovery, restorableLink, recoveryDisplayFor } from '../../src/sync-setup.ts';
import { syncCopy } from '../../src/sync-copy.ts';
import { localizedFailure } from '../../src/errors.ts';

const cloud = (patch = {}) => ({ signed_in: true, sync_enabled: false, syncing: false, status: 'off', recovery_confirmed: true, ...patch });
const setup = (phase, patch = {}) => ({ phase, sync_enabled: false, generation: 1, can_approve_devices: false, uploaded: 0, downloaded: 0, conflicts: 0, last_success_ms: null, ...patch });

test('a paused library never claims it is waiting for an automatic first sync', () => {
  assert.equal(syncStatusKey(cloud()), 'mobile.sync.paused');
  assert.equal(syncSetupStage(cloud(), setup('paused')), 'resume');
  for (const phase of ['new_library', 'locked', 'ready']) assert.equal(syncStatusKey(cloud(), setup(phase)), 'mobile.sync.setup_needed');
});

test('an existing locked library exposes recovery even when sync is off', () => {
  assert.equal(syncSetupStage(cloud(), setup('locked')), 'unlock');
  assert.equal(requiresRecovery({ code: 'vault_locked' }), true);
  assert.equal(requiresRecovery({ code: 'network' }), false);
});

test('backup acknowledgment remains distinct from a usable local key', () => {
  assert.equal(syncSetupStage(cloud({ recovery_confirmed: false }), setup('ready')), 'backup');
  assert.equal(syncSetupStage(cloud(), setup('ready')), 'backup');
  assert.equal(syncSetupStage(cloud({ recovery_confirmed: false, sync_enabled: true }), setup('active')), 'backup');
  assert.equal(syncSetupStage(cloud(), setup('new_library')), 'create');
});

test('authentication and the first successful sync are separate stages', () => {
  assert.equal(syncSetupStage(cloud({ signed_in: false }), null), 'sign_in');
  assert.equal(syncStatusKey(cloud({ sync_enabled: true, status: 'pending' })), 'account.sync.first_pending');
  assert.equal(syncStatusKey(cloud({ sync_enabled: true, status: 'synced' })), 'account.sync.synced');
  assert.equal(syncStatusKey(cloud({ sync_enabled: true, status: 'offline' })), 'account.sync.offline');
  assert.equal(syncStatusKey(cloud({ syncing: true, sync_enabled: true })), 'account.sync.busy');
});

test('pairing polling and navigation guards finish at every terminal phase', () => {
  for (const phase of ['waiting_peer', 'confirming', 'awaiting_approval', 'waiting_approval', 'provisioning']) assert.equal(linkIsPending(phase), true, phase);
  for (const phase of ['idle', 'complete', 'cancelled', 'expired', 'failed']) assert.equal(linkIsPending(phase), false, phase);
});

test('a new Add device visit ignores historical terminal sessions and other roles', () => {
  const account = { signed_in: true, account: { email: 'fixture@example.test' } };
  const old = { phase: 'complete', role: 'new_device', request_id: 'old' };
  assert.equal(restorableLink(old, 'trusted_device', account), null);
  assert.equal(restorableLink(old, 'new_device', account), null);
  assert.equal(restorableLink({ ...old, phase: 'waiting_approval' }, 'trusted_device', account), null);
  const pending = { ...old, phase: 'awaiting_approval', role: 'trusted_device' };
  assert.equal(restorableLink(pending, 'trusted_device', account), pending);
  assert.equal(restorableLink(old, 'new_device', account, 'old'), old);
  assert.equal(restorableLink(old, 'new_device', account, 'other'), null);
  assert.equal(restorableLink(old, 'trusted_device', account, 'old'), null);
});

test('recovery display and confirmation are hidden after signout, account change or rotation', () => {
  const account = { signed_in: true, account_epoch: 'account-a', recovery_display_token: 'shown-a' };
  const display = { recovery_code: 'fixture', confirmation_token: 'shown-a', account_epoch: 'account-a' };
  assert.equal(recoveryDisplayFor(account, display), display);
  assert.equal(recoveryDisplayFor({ ...account, signed_in: false }, display), null);
  assert.equal(recoveryDisplayFor({ ...account, account_epoch: 'account-b' }, display), null);
  assert.equal(recoveryDisplayFor({ ...account, recovery_display_token: 'shown-b' }, display), null);
  assert.equal(recoveryDisplayFor({ ...account, recovery_display_token: null }, display), null);
  assert.equal(recoveryDisplayFor(account, { ...display, recovery_code: null, confirmation_token: null }), null);
  assert.equal(recoveryDisplayFor(account, null), null);
});

test('a fresh connection visit never restores a different account or signed-out trusted request', () => {
  const accountA = { signed_in: true, account: { email: 'account-a@example.test' } };
  const accountB = { signed_in: true, account: { email: 'account-b@example.test' } };
  const signedOut = { signed_in: false, account: null };
  const trusted = { phase: 'awaiting_approval', role: 'trusted_device', request_id: 'old-a', account_email: 'account-a@example.test' };
  assert.equal(restorableLink(trusted, 'trusted_device', accountA), trusted);
  assert.equal(restorableLink(trusted, 'trusted_device', accountB), null);
  assert.equal(restorableLink(trusted, 'trusted_device', { signed_in: true, account: null }), null);
  assert.equal(restorableLink(trusted, 'trusted_device', signedOut), null);
  assert.equal(restorableLink({ ...trusted, account_email: null }, 'trusted_device', signedOut), null);
  const requesting = { ...trusted, role: 'new_device', phase: 'waiting_approval' };
  assert.equal(restorableLink(requesting, 'new_device', accountA), requesting);
  assert.equal(restorableLink(requesting, 'new_device', accountB), null);
  assert.equal(restorableLink(requesting, 'new_device', signedOut), requesting);
  const completed = { ...requesting, phase: 'complete' };
  assert.equal(restorableLink(completed, 'new_device', accountA, 'old-a'), completed);
  assert.equal(restorableLink(completed, 'new_device', accountB, 'old-a'), null);
});

test('sync setup copy is complete in both languages and errors describe the recovery path', () => {
  assert.deepEqual(Object.keys(syncCopy.en).sort(), Object.keys(syncCopy.nl).sort());
  const fields = (value) => [...value.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
  for (const key of Object.keys(syncCopy.en)) assert.deepEqual(fields(syncCopy.en[key]), fields(syncCopy.nl[key]), key);
  for (const code of ['camera_denied', 'scanner_unavailable', 'recovery_unconfirmed', 'pairing_code', 'pairing_invalid', 'pairing_expired', 'pairing_restart', 'device_limit', 'generation']) {
    const en = localizedFailure('en', { code, message: 'private diagnostic' });
    const nl = localizedFailure('nl', { code, message: 'private diagnostic' });
    assert.notEqual(en, nl, code);
    assert.equal(en.includes('private diagnostic'), false);
    assert.notEqual(en, localizedFailure('en', { code: 'unknown', message: '' }), code);
  }
});

test('device connection errors describe service fallback, durable retry and a fresh verified connection', () => {
  const text = (code, language = 'en') => localizedFailure(language, { code, message: 'private server diagnostic' });
  for (const code of ['pairing_unavailable', 'pairing_storage', 'pairing_busy', 'pairing_unknown', 'pairing_claimed', 'pairing_used', 'pairing_account', 'pairing_existing', 'pairing_changed', 'pairing_transcript', 'pairing_message', 'pairing_phase', 'pairing_approval', 'pairing_pending', 'pairing_retry']) {
    assert.notEqual(text(code), text('unknown'), code);
    assert.notEqual(text(code), text(code, 'nl'), code);
    assert.equal(text(code).includes('private server diagnostic'), false, code);
  }
  assert.match(text('pairing_unavailable'), /email.*recovery code/i);
  assert.match(text('pairing_storage'), /Unlock this device.*pending connection open/i);
  assert.doesNotMatch(text('pairing_storage'), /new (connection|code)|start again/i);
  for (const code of ['pairing_claimed', 'pairing_used', 'pairing_unknown']) assert.match(text(code), /new connection.*fresh code/i);
  assert.match(text('pairing_account'), /Sign out.*intended account/i);
  assert.match(text('pairing_existing'), /already signed in.*unlock this library/i);
  for (const code of ['pairing_changed', 'pairing_transcript', 'pairing_message', 'pairing_phase', 'pairing_approval']) assert.match(text(code), /verified.*new connection/i);
});
