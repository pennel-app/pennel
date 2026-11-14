const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
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
  fs.writeFileSync(path.join(root, 'artifacts/ios/manifest.json'), JSON.stringify({ schema: 1, preparing: true }));
  assert.throws(() => validatePreparedArtifacts(root, {}), /matching/);
  assert.throws(() => validatePreparedArtifacts(root, { PENNEL_PEER_TRANSPORT_QA: '1' }), /matching/);
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
