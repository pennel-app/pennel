const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { peerTransportQAEnabled, validatePeerArtifacts, validatePreparedArtifacts } = require('../plugins/peer-transport-qa.cjs');
const { prepareNativeSources, serviceFiles, peerQAServiceFiles } = require('../plugins/native-sources.cjs');

test('transport qualification is explicit and unavailable in shipping distributions', () => {
  assert.equal(peerTransportQAEnabled({}, {}), false);
  assert.equal(peerTransportQAEnabled({}, { PENNEL_PEER_TRANSPORT_QA: '1' }), true);
  assert.throws(() => peerTransportQAEnabled({ production: true }, { PENNEL_PEER_TRANSPORT_QA: '1' }), /Preview/);
  assert.throws(() => peerTransportQAEnabled({ testFlight: true }, { PENNEL_PEER_TRANSPORT_QA: '1' }), /Preview/);
  assert.throws(() => peerTransportQAEnabled({}, { PENNEL_PEER_TRANSPORT_QA: 'yes' }), /0 or 1/);
});

test('ordinary and QA generation reject artifacts prepared for the other feature graph', () => {
  validatePeerArtifacts({ features: [] }, false);
  validatePeerArtifacts({ features: ['peer-transport-qa'] }, true);
  assert.throws(() => validatePeerArtifacts({ features: [] }, true), /matching/);
  assert.throws(() => validatePeerArtifacts({ features: ['peer-transport-qa'] }, false), /matching/);
});

test('an interrupted artifact switch blocks either kind of native build', (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-peer-pending-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, 'artifacts/ios'), { recursive: true });
  fs.writeFileSync(path.join(root, 'artifacts/ios/manifest.json'), JSON.stringify({ schema: 1, preparing: true, features: [] }));
  assert.throws(() => validatePreparedArtifacts(root, {}), /matching/);
  assert.throws(() => validatePreparedArtifacts(root, { PENNEL_PEER_TRANSPORT_QA: '1' }), /matching/);
});

test('Pods rejects a pending manifest before evaluating either feature graph', (context) => {
  if (spawnSync('ruby', ['--version']).status !== 0) {
    context.skip('Ruby is required only for the iOS Pods boundary');
    return;
  }
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-peer-pending-pods-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const moduleRoot = path.join(root, 'modules/pennel-mobile');
  fs.mkdirSync(moduleRoot, { recursive: true });
  fs.mkdirSync(path.join(root, 'artifacts/ios'), { recursive: true });
  fs.copyFileSync(path.join(__dirname, '../modules/pennel-mobile/PennelMobile.podspec'),
    path.join(moduleRoot, 'PennelMobile.podspec'));
  fs.writeFileSync(path.join(moduleRoot, 'package.json'), JSON.stringify({ version: '0.1.0' }));
  fs.writeFileSync(path.join(root, 'artifacts/ios/manifest.json'),
    JSON.stringify({ schema: 1, preparing: true, features: [] }));
  for (const flag of ['0', '1']) {
    const result = spawnSync('ruby', [path.join(moduleRoot, 'PennelMobile.podspec')], {
      encoding: 'utf8', env: { ...process.env, PENNEL_PEER_TRANSPORT_QA: flag, PENNEL_VARIANT: 'preview', PENNEL_TESTFLIGHT: '0' },
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Core preparation is incomplete/);
    assert.doesNotMatch(result.stderr, /uninitialized constant Pod/);
  }
});

test('repeated preparation removes QA source membership when returning to ordinary Preview', (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-peer-membership-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const file of [...serviceFiles, ...peerQAServiceFiles]) {
    const source = path.join(root, 'native/ios/Services', file);
    fs.mkdirSync(path.dirname(source), { recursive: true });
    fs.writeFileSync(source, '// fixture\n');
  }
  for (const directory of ['PennelCoreFFI.xcframework', 'PennelTypingFFI.xcframework', 'PennelCore']) {
    fs.mkdirSync(path.join(root, 'artifacts/ios', directory), { recursive: true });
  }
  fs.writeFileSync(path.join(root, 'artifacts/ios/PennelCore/pennel_core.swift'), '// bindings\n');
  const qa = prepareNativeSources(root, { PENNEL_PEER_TRANSPORT_QA: '1' });
  for (const file of peerQAServiceFiles) assert(fs.existsSync(path.join(qa, 'Services', file)));
  const ordinary = prepareNativeSources(root, {});
  for (const file of peerQAServiceFiles) assert(!fs.existsSync(path.join(ordinary, 'Services', file)));
});
