const fs = require('node:fs');
const path = require('node:path');
const { resolveMobileVariant, targetBundleIdentifiers } = require('./mobile-variant.cjs');

const peerQAProfile = 'peer-transport';
const peerQAKeychainService = `${resolveMobileVariant({ PENNEL_VARIANT: 'preview' }).bundleIdentifier}.qa.${peerQAProfile}`;
const peerQAServiceFiles = ['PeerComparisonTransport.swift', 'PeerComparisonBridge.swift'];
const appEntitlements = 'Pennel/Pennel.entitlements';

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
  const enabled = peerTransportQAEnabled({}, environment);
  const manifest = JSON.parse(fs.readFileSync(path.join(workspace, 'artifacts/ios/manifest.json'), 'utf8'));
  validatePeerArtifacts(manifest, enabled);
  return enabled;
}

function readProject(file) {
  const project = require('xcode').project(file);
  project.parseSync();
  return project.hash.project.objects;
}

const unquote = (value) => String(value ?? '').replace(/^"|"$/g, '');

function checkTarget(graph, name, enabled, expectedSettings = {}) {
  const target = Object.values(graph.PBXNativeTarget).find((entry) => unquote(entry.name) === name);
  const configurations = (graph.XCConfigurationList[target?.buildConfigurationList]?.buildConfigurations ?? [])
    .map(({ value }) => graph.XCBuildConfiguration[value]);
  for (const mode of ['Debug', 'Release']) {
    const config = configurations.find((entry) => unquote(entry.name) === mode);
    if (!config) throw new Error(`Missing generated ${name} ${mode} configuration`);
    const settings = config.buildSettings;
    for (const [key, expected] of Object.entries(expectedSettings)) {
      if (unquote(settings[key]) !== expected) throw new Error(`The generated ${name} ${mode} ${key} does not match the selected variant`);
    }
    const flags = [settings.SWIFT_ACTIVE_COMPILATION_CONDITIONS].flat()
      .flatMap((value) => unquote(value).split(/\s+/));
    if (flags.includes('PENNEL_PEER_TRANSPORT_QA') !== enabled) {
      throw new Error(`The generated ${name} ${mode} build flags do not match the selected peer transport setting`);
    }
  }
  return target;
}

function validatePreparedBuild(workspace) {
  const enabled = validatePreparedArtifacts(workspace);
  const { bundleIdentifier, groupIdentifier } = resolveMobileVariant();
  const ios = path.join(workspace, 'ios');
  const readPlist = (file) => require('@expo/plist').default.parse(fs.readFileSync(path.join(ios, file), 'utf8'));
  const info = readPlist('Pennel/Info.plist');
  for (const [key, expected] of Object.entries({
    PennelQAProfile: enabled ? peerQAProfile : undefined,
    PennelKeychainService: enabled ? peerQAKeychainService : bundleIdentifier,
    PennelAppGroupIdentifier: groupIdentifier,
  })) {
    if (info[key] !== expected) throw new Error(`The generated ${key} does not match the selected Pennel variant`);
  }
  if (![bundleIdentifier, '$(PRODUCT_BUNDLE_IDENTIFIER)', '${PRODUCT_BUNDLE_IDENTIFIER}'].includes(info.CFBundleIdentifier)) {
    throw new Error('The generated app plist bundle identifier does not match the selected Pennel variant');
  }
  const groups = readPlist(appEntitlements)['com.apple.security.application-groups'];
  if (!Array.isArray(groups) || groups.length !== 1 || groups[0] !== groupIdentifier) {
    throw new Error('The generated app group entitlement does not match the selected Pennel variant');
  }
  const app = readProject(path.join(ios, 'Pennel.xcodeproj/project.pbxproj'));
  for (const [name, id] of Object.entries(targetBundleIdentifiers(bundleIdentifier))) {
    const settings = { PRODUCT_BUNDLE_IDENTIFIER: id };
    if (name === 'Pennel') settings.CODE_SIGN_ENTITLEMENTS = appEntitlements;
    checkTarget(app, name, enabled && (name === 'Pennel' || name === 'PennelRuntimeTests'), settings);
  }
  const pods = readProject(path.join(ios, 'Pods/Pods.xcodeproj/project.pbxproj'));
  const mobile = checkTarget(pods, 'PennelMobile', enabled);
  const sources = mobile.buildPhases.flatMap(({ value }) => pods.PBXSourcesBuildPhase[value]?.files ?? [])
    .map(({ value }) => path.basename(unquote(pods.PBXFileReference[pods.PBXBuildFile[value].fileRef].path)));
  if (!sources.includes('PennelMobileModule.swift') || peerQAServiceFiles.some((file) => sources.includes(file) !== enabled)) {
    throw new Error('The generated PennelMobile Pod sources do not match the selected peer transport setting');
  }
}

if (require.main === module) {
  try { validatePreparedBuild(path.resolve(process.argv[2])); }
  catch (error) {
    process.stderr.write(`${error.message}\nRun make mobile-prebuild with the selected Pennel variant and peer transport setting before building.\n`);
    process.exitCode = 1;
  }
}

module.exports = { peerTransportQAEnabled, peerQAProfile, peerQAKeychainService, peerQAServiceFiles, validatePeerArtifacts, validatePreparedArtifacts };
