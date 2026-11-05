import assert from 'node:assert/strict';
import test from 'node:test';
import { localizedFailure, nativeFailure, ownedFailureText } from '../../src/errors.ts';

test('provider billing failures use owned credit guidance without exposing response details', () => {
  for (const code of ['upstream_payment_required', 'http_402']) {
    const message = 'Bearer synthetic-private-key https://private.example/billing?token=secret';
    assert.equal(localizedFailure('en', { code, message }), 'The provider reports insufficient credit. Check your account.');
    assert.equal(localizedFailure('nl', { code, message }), 'De provider meldt dat er onvoldoende tegoed is. Controleer je account.');
    assert.equal(ownedFailureText('en', message, code), 'The provider reports insufficient credit. Check your account.');
  }
});
import { recognitionActions } from '../../src/format.ts';

test('native NSError envelopes retain diagnostic codes while English UI replaces Dutch backend literals', () => {
  const diagnostic = 'De opname is ondertussen gewijzigd.';
  const rejected = { code: 'ERR_NATIVE', message: `Call to function PennelMobile.invoke was rejected.\nCaused by: ${JSON.stringify({ code: 'speaker_stale', message: diagnostic })}` };
  const error = nativeFailure(rejected);
  assert.equal(error.code, 'speaker_stale'); assert.equal(error.message, diagnostic);
  assert.match(localizedFailure('en', error), /Start recognition again/);
  assert.equal(localizedFailure('en', error).includes(diagnostic), false);
  assert.match(localizedFailure('nl', error), /Start de herkenning opnieuw/);
});

test('every new recognition error has actionable EN/NL text independent of backend message', () => {
  for (const code of ['speaker_stale', 'speaker_checkpoint_version', 'speaker_checkpoint_invalid', 'speaker_retry_confirmation', 'speaker_cleanup', 'speaker_model', 'quota', 'auth', 'signed_out', 'storage', 'network', 'cancelled', 'interrupted', 'recognition']) {
    const original = { code, message: 'Backenddetails die niet in Engelse tekst thuishoren.' };
    const en = localizedFailure('en', original); const nl = localizedFailure('nl', original);
    assert.ok(en.length > 15, code); assert.ok(nl.length > 15, code);
    assert.notEqual(en, nl, code); assert.equal(en.includes('Backenddetails'), false, code);
    assert.equal(en.startsWith('mobile.errors.'), false, code);
    assert.deepEqual(original, { code, message: 'Backenddetails die niet in Engelse tekst thuishoren.' });
  }
});

test('structured errors can be recovered from NSError metadata without swallowing quoted braces', () => {
  const embedded = JSON.stringify({ code: 'stale_notes', message: 'De tekst {naam} is veranderd.' });
  const error = nativeFailure({ code: 'ERR_NATIVE', message: `Error Domain=PennelMobile Code=1 "${embedded}" UserInfo={NSLocalizedDescription=${embedded}}` });
  assert.equal(error.code, 'stale_notes');
  assert.equal(error.message, 'De tekst {naam} is veranderd.');
  assert.match(localizedFailure('en', error), /Reload/);
});

test('stale-note codes instruct reload instead of generic storage retry', () => {
  for (const code of ['stale_notes', 'notes_stale']) {
    assert.match(localizedFailure('en', { code, message: 'De notities zijn gewijzigd.' }), /Reload/);
    assert.match(localizedFailure('nl', { code, message: 'De notities zijn gewijzigd.' }), /Laad ze opnieuw/);
  }
});

test('unknown provider messages remain diagnostics and cannot become raw UI content', () => {
  const message = 'Potential provider content';
  const error = nativeFailure({ code: 'unrecognized_provider_code', message });
  assert.equal(error.message, message);
  assert.equal(localizedFailure('en', error).includes(message), false);
  assert.equal(localizedFailure('nl', error).includes(message), false);
});

test('saved interrupted work can be discarded and cleanup failures offer removal again', () => {
  assert.equal(recognitionActions({ stage: 'interrupted', recoverable: true, error: null }).cancel, true);
  assert.equal(recognitionActions({ stage: 'failed', recoverable: false, error: { code: 'speaker_cleanup' } }).cancel, true);
  assert.equal(recognitionActions({ stage: 'applying', recoverable: false, error: null }).cancel, false);
});

test('saved dictation and recording diagnostics relabel without changing the saved message', () => {
  for (const message of ['Geen spraak herkend. Probeer opnieuw.', 'No speech recognized. Try again.']) {
    assert.match(ownedFailureText('en', message), /No speech/);
    assert.match(ownedFailureText('nl', message), /Geen spraak/);
  }
  const saved = 'De opname is gestopt omdat iOS Pennel heeft afgesloten.';
  assert.match(ownedFailureText('en', saved), /iOS closed Pennel/);
  assert.match(ownedFailureText('nl', saved), /iOS Pennel heeft afgesloten/);
  assert.equal(saved, 'De opname is gestopt omdat iOS Pennel heeft afgesloten.');
});

test('stable failure codes select presentation independently of raw native diagnostics', () => {
  assert.match(ownedFailureText('en', 'Geen toegang tot de microfoon.', 'microphone_denied'), /Allow microphone/);
  assert.match(ownedFailureText('nl', 'Diagnostic in English', 'network'), /verbinding/);
  assert.match(ownedFailureText('en', 'Recording stopped.', 'recording_gaps'), /Timestamps preserve the gaps/);
  const content = 'Mijn notitie bevat privétekst.';
  assert.equal(ownedFailureText('en', content).includes(content), false);
  assert.equal(content, 'Mijn notitie bevat privétekst.');
});

test('provider key failures stay separate from Pennel Cloud sign-in failures', () => {
  for (const code of ['provider_auth', 'http_401', 'http_403']) {
    const english = localizedFailure('en', { code, message: 'Private provider details' });
    const dutch = localizedFailure('nl', { code, message: 'Private provider details' });
    assert.match(english, /provider.*API key/i);
    assert.match(dutch, /API-sleutel/);
    assert.equal(english.includes('Pennel Cloud'), false);
    assert.equal(dutch.includes('Pennel Cloud'), false);
  }
  assert.match(localizedFailure('en', { code: 'not_signed_in', message: '' }), /Sign in to Pennel Cloud/);
  assert.match(localizedFailure('en', { code: 'auth', message: '' }), /Sign in to Pennel Cloud/);
  assert.match(ownedFailureText('en', 'API-sleutel geweigerd'), /API key/);
});
