import assert from 'node:assert/strict';
import test from 'node:test';
import { ownedNativeFailureText } from '../../src/errors.ts';

test('native conversation error envelopes retain localized owned storage reasons', () => {
  const error = 'Expo invoke rejected: {"code":"storage","message":"The library changed. Reopen the conversation to continue."}';
  assert.equal(ownedNativeFailureText('en', error), 'The library changed. Reopen the conversation to continue.');
  assert.equal(ownedNativeFailureText('nl', error), 'De bibliotheek is gewijzigd. Open het gesprek opnieuw om verder te gaan.');
});

test('unknown native provider text stays private while actionable error codes remain useful', () => {
  const error = 'Expo invoke rejected: {"code":"missing_api_key","message":"private provider sentinel"}';
  for (const language of ['en', 'nl']) {
    const text = ownedNativeFailureText(language, error);
    assert.equal(text.includes('private provider sentinel'), false);
    assert.match(text, /API/i);
  }
});
