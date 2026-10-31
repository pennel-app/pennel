const assert = require('node:assert/strict');
const test = require('node:test');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { configureTestFlightSigning } = require('../plugins/with-pennel.cjs')._test;

test('TestFlight signing uses the personal team and automatic distribution for Preview', () => {
  const settings = {};
  configureTestFlightSigning(settings, { testFlight: true, production: false });
  assert.equal(settings.DEVELOPMENT_TEAM, '"B538Q2K76C"');
  assert.equal(settings.CODE_SIGN_STYLE, 'Automatic');
});

test('ordinary generated builds retain their existing signing settings', () => {
  const settings = { DEVELOPMENT_TEAM: 'existing', CODE_SIGN_STYLE: 'Manual' };
  configureTestFlightSigning(settings, { production: false });
  assert.deepEqual(settings, { DEVELOPMENT_TEAM: 'existing', CODE_SIGN_STYLE: 'Manual' });
});

test('Preview distribution cannot select production identities', () => {
  assert.throws(() => configureTestFlightSigning({}, { testFlight: true, production: true }), /Preview/);
});

function appConfig(environment) {
  return JSON.parse(execFileSync(process.execPath, ['-e', "const {getConfig}=require('expo/config');process.stdout.write(JSON.stringify(getConfig(process.cwd()).exp.ios));"], {
    cwd: path.resolve(__dirname, '..'), encoding: 'utf8',
    env: { ...process.env, PENNEL_VARIANT: 'preview', PENNEL_TESTFLIGHT: '0', ...environment },
  }));
}

test('the owner-confirmed internal Preview carries its export documentation determination', () => {
  const config = appConfig({ PENNEL_TESTFLIGHT: '1' });
  assert.equal(config.bundleIdentifier, 'app.pennel.ios.preview');
  assert.equal(config.infoPlist.ITSAppUsesNonExemptEncryption, false);
});

test('ordinary Preview and production builds carry no inferred export determination', () => {
  for (const variant of ['preview', 'production']) {
    assert.equal(appConfig({ PENNEL_VARIANT: variant }).infoPlist.ITSAppUsesNonExemptEncryption, undefined);
  }
});
