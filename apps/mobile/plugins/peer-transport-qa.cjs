function peerTransportQAEnabled(options = {}, environment = process.env) {
  const flag = environment.PENNEL_PEER_TRANSPORT_QA ?? '0';
  if (flag !== '0' && flag !== '1') throw new Error('PENNEL_PEER_TRANSPORT_QA must be 0 or 1');
  const enabled = flag === '1';
  if (enabled && (options.production || options.testFlight || environment.PENNEL_VARIANT === 'production' || environment.PENNEL_TESTFLIGHT === '1')) {
    throw new Error('Peer transport qualification is restricted to local Pennel Preview builds');
  }
  return enabled;
}

function validatePeerArtifacts(manifest, enabled) {
  if (manifest.preparing === true || !Array.isArray(manifest.features) || manifest.features.includes('peer-transport-qa') !== enabled) {
    throw new Error('Prepare matching ordinary or peer-transport-qa core artifacts before generation');
  }
}

function validatePreparedArtifacts(workspace, environment = process.env) {
  const fs = require('node:fs');
  const path = require('node:path');
  const manifest = JSON.parse(fs.readFileSync(path.join(workspace, 'artifacts/ios/manifest.json'), 'utf8'));
  validatePeerArtifacts(manifest, peerTransportQAEnabled({}, environment));
}

if (require.main === module) {
  try { validatePreparedArtifacts(require('node:path').resolve(process.argv[2])); }
  catch { process.stderr.write('Prepare matching complete mobile artifacts before building.\n'); process.exit(1); }
}

module.exports = { peerTransportQAEnabled, validatePeerArtifacts, validatePreparedArtifacts };
