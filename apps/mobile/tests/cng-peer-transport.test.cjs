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

function writeProject(root, relative, targets, sources, bundleIds) {
  const sections = {};
  function entry(kind, id, attributes) {
    (sections[kind] ??= []).push(`${id} = { isa = ${kind}; ${attributes} };`);
  }
  for (const [name, flags] of Object.entries(targets)) {
    entry('PBXNativeTarget', name, `name = ${name}; buildConfigurationList = ${name}Configs; buildPhases = (Sources /* Sources */,);`);
    entry('XCConfigurationList', `${name}Configs`, `buildConfigurations = (${name}Debug /* Debug */, ${name}Release /* Release */,);`);
    for (const mode of ['Debug', 'Release']) {
      const id = bundleIds?.[name]?.[mode];
      const identity = id ? `PRODUCT_BUNDLE_IDENTIFIER = ${id};` : '';
      const entitlements = name === 'Pennel' ? 'CODE_SIGN_ENTITLEMENTS = Pennel/Pennel.entitlements;' : '';
      entry('XCBuildConfiguration', name + mode, `name = ${mode}; buildSettings = { ${identity} ${entitlements} SWIFT_ACTIVE_COMPILATION_CONDITIONS = "$(inherited) ${flags[mode] ? 'PENNEL_PEER_TRANSPORT_QA' : ''}"; };`);
    }
  }
  const files = sources.map((file, index) => {
    entry('PBXFileReference', `File${index}`, `path = ${file};`);
    entry('PBXBuildFile', `Build${index}`, `fileRef = File${index};`);
    return `Build${index} /* ${file} in Sources */,`;
  });
  entry('PBXSourcesBuildPhase', 'Sources', `files = (${files.join(' ')});`);
  const body = Object.entries(sections).map(([kind, rows]) =>
    `/* Begin ${kind} section */\n${rows.join('\n')}\n/* End ${kind} section */`).join('\n');
  const project = path.join(root, relative);
  fs.mkdirSync(path.dirname(project), { recursive: true });
  fs.writeFileSync(project, `// !$*UTF8*$!\n{ objects = {\n${body}\n}; }\n`);
}

function writeBuildFixture(root, selected, overrides = {}) {
  const both = (enabled) => ({ Debug: enabled, Release: enabled });
  const production = overrides.variant === 'production';
  const bundle = production ? 'app.pennel.ios' : 'app.pennel.ios.preview';
  const group = production ? 'group.app.pennel' : 'group.app.pennel.preview';
  const { profile = selected, keychain = profile, app = both(selected), tests = both(selected),
    bundleIds = both(bundle), keychainService = keychain ? 'app.pennel.ios.preview.qa.peer-transport' : bundle,
    appGroup = group, entitlementGroups = [group],
    pod = both(selected), sources = selected ? peerQAServiceFiles : [] } = overrides;
  fs.mkdirSync(path.join(root, 'artifacts/ios'), { recursive: true });
  fs.writeFileSync(path.join(root, 'artifacts/ios/manifest.json'),
    JSON.stringify({ features: selected ? ['peer-transport-qa'] : [] }));
  fs.mkdirSync(path.join(root, 'ios/Pennel'), { recursive: true });
  const plist = require('@expo/plist').default;
  fs.writeFileSync(path.join(root, 'ios/Pennel/Info.plist'), plist.build({
    ...(profile ? { PennelQAProfile: 'peer-transport' } : {}),
    CFBundleIdentifier: '$(PRODUCT_BUNDLE_IDENTIFIER)', PennelKeychainService: keychainService, PennelAppGroupIdentifier: appGroup,
  }));
  fs.writeFileSync(path.join(root, 'ios/Pennel/Pennel.entitlements'), plist.build({ 'com.apple.security.application-groups': entitlementGroups }));
  const targets = { Pennel: app, PennelRuntimeTests: tests, PennelKeyboard: both(false), PennelKeyboardTests: both(false), ExpoWidgetsTarget: both(false) };
  writeProject(root, 'ios/Pennel.xcodeproj/project.pbxproj', targets, [], {
    Pennel: bundleIds, PennelKeyboard: both(`${bundle}.keyboard`), PennelKeyboardTests: both(`${bundle}.keyboard-tests`),
    PennelRuntimeTests: both(`${bundle}.runtime-tests`), ExpoWidgetsTarget: both(`${bundle}.widgets`), ...overrides.targetIds,
  });
  writeProject(root, 'ios/Pods/Pods.xcodeproj/project.pbxproj', { PennelMobile: pod }, ['PennelMobileModule.swift', ...sources]);
}

test('ordinary and QA builds check the existing generated app and Pods in both directions', (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-peer-projects-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const selected of [false, true]) {
    const environment = { ...process.env, PENNEL_PEER_TRANSPORT_QA: selected ? '1' : '0', PENNEL_VARIANT: 'preview', PENNEL_TESTFLIGHT: '0' };
    const check = () => spawnSync(process.execPath, [require.resolve('../plugins/peer-transport-qa.cjs'), root], { env: environment, encoding: 'utf8' });
    writeBuildFixture(root, selected);
    const matched = check();
    assert.equal(matched.status, 0, matched.stderr);
    for (const stale of [
      { profile: !selected }, { keychain: !selected },
      { bundleIds: { Debug: 'app.pennel.ios.preview', Release: 'app.pennel.ios' } },
      { targetIds: { PennelKeyboard: { Debug: 'app.pennel.ios.keyboard', Release: 'app.pennel.ios.keyboard' } } },
      { keychainService: 'unrelated' }, { appGroup: 'group.app.pennel' },
      { entitlementGroups: ['group.app.pennel.preview', 'group.app.pennel'] },
      { app: { Debug: selected, Release: !selected } }, { tests: { Debug: !selected, Release: selected } },
      { pod: { Debug: selected, Release: !selected } },
      { sources: selected ? peerQAServiceFiles.slice(1) : peerQAServiceFiles.slice(0, 1) },
    ]) {
      writeBuildFixture(root, selected, stale);
      const rejected = check();
      assert.notEqual(rejected.status, 0, JSON.stringify(stale));
      assert.match(rejected.stderr, /generated|Pods/i);
    }
  }
});

test('ordinary Preview, TestFlight and production require the generated identity selected by Expo', (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-generated-variant-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const cli = require.resolve('../plugins/peer-transport-qa.cjs');
  for (const variant of ['preview', 'production']) {
    const environment = { ...process.env, PENNEL_PEER_TRANSPORT_QA: '0', PENNEL_TESTFLIGHT: '0', PENNEL_VARIANT: variant };
    const check = () => spawnSync(process.execPath, [cli, root], { env: environment, encoding: 'utf8' });
    writeBuildFixture(root, false, { variant: variant === 'preview' ? 'production' : 'preview' });
    assert.notEqual(check().status, 0, `Existing project from the other variant accepted as ${variant}`);
    writeBuildFixture(root, false, { variant });
    const matching = check();
    assert.equal(matching.status, 0, matching.stderr);
    environment.PENNEL_TESTFLIGHT = '1';
    if (variant === 'preview') delete environment.PENNEL_VARIANT;
    assert.equal(check().status === 0, variant === 'preview', 'TestFlight defaults to Preview');
  }
});

test('Release build and hosted native tests stop before Xcode for a stale project', (context) => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-peer-build-'));
  context.after(() => fs.rmSync(repo, { recursive: true, force: true }));
  const workspace = path.join(repo, 'apps/mobile');
  const scripts = path.join(repo, 'scripts');
  fs.mkdirSync(path.join(workspace, 'ios/Pennel.xcworkspace'), { recursive: true });
  fs.mkdirSync(scripts);
  fs.symlinkSync(path.join(__dirname, '../plugins'), path.join(workspace, 'plugins'), 'dir');
  fs.copyFileSync(path.join(__dirname, '../../../Makefile'), path.join(repo, 'Makefile'));
  fs.copyFileSync(path.join(__dirname, '../../../scripts/build-mobile-ios.sh'), path.join(scripts, 'build-mobile-ios.sh'));
  fs.writeFileSync(path.join(scripts, 'ci-xcodebuild.sh'), 'echo "reached xcodebuild $1"\n');
  fs.writeFileSync(path.join(workspace, 'Podfile.lock'), 'COCOAPODS: 1.16.2\n');
  for (const selected of [false, true]) {
    for (const stale of [true, false]) {
      writeBuildFixture(workspace, selected, stale ? { pod: { Debug: !selected, Release: !selected } } : {});
      for (const [tool, args] of [['bash', ['scripts/build-mobile-ios.sh']], ['make', ['mobile-native-test']]]) {
        const result = spawnSync(tool, args, { cwd: repo, encoding: 'utf8', env: { ...process.env,
          PENNEL_PEER_TRANSPORT_QA: selected ? '1' : '0', PENNEL_VARIANT: 'preview', PENNEL_TESTFLIGHT: '0' } });
        assert.equal(result.status === 0, !stale, result.stderr);
        assert.equal(result.stdout.includes('reached xcodebuild'), !stale);
      }
    }
  }
});

test('local iOS run refreshes a stale or missing graph before launching Expo and reuses a matching graph', (context) => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-peer-run-'));
  context.after(() => fs.rmSync(repo, { recursive: true, force: true }));
  const workspace = path.join(repo, 'apps/mobile');
  const prepared = path.join(repo, 'fixture');
  const bin = path.join(repo, 'bin');
  const log = path.join(repo, 'commands');
  fs.mkdirSync(bin);
  fs.mkdirSync(path.join(workspace, 'artifacts/ios'), { recursive: true });
  fs.copyFileSync(path.join(__dirname, '../../../Makefile'), path.join(repo, 'Makefile'));
  fs.symlinkSync(path.join(__dirname, '../plugins'), path.join(workspace, 'plugins'), 'dir');
  fs.writeFileSync(path.join(workspace, 'Podfile.lock'), 'COCOAPODS: 1.16.2\n');
  for (const [tool, command] of Object.entries({
    npm: `if [ "$2" = prebuild ]; then
  mkdir -p ios/Pennel ios/Pennel.xcodeproj
  cp "$FIXTURE/ios/Pennel/Info.plist" ios/Pennel/
  cp "$FIXTURE/ios/Pennel/Pennel.entitlements" ios/Pennel/
  cp "$FIXTURE/ios/Pennel.xcodeproj/project.pbxproj" ios/Pennel.xcodeproj/
fi`,
    pod: `if [ "$PRESERVE_PODS" != 1 ]; then
  mkdir -p Pods/Pods.xcodeproj
  cp "$FIXTURE/ios/Pods/Pods.xcodeproj/project.pbxproj" Pods/Pods.xcodeproj/
fi`,
    expo: '',
  })) {
    fs.writeFileSync(path.join(bin, tool), `#!/bin/sh\nset -eu\necho "${tool} $*" >> "$COMMAND_LOG"\n${command}\n`, { mode: 0o755 });
  }
  function run(selected, variant = 'preview', preservePods = false) {
    writeBuildFixture(prepared, selected, { variant });
    fs.copyFileSync(path.join(prepared, 'artifacts/ios/manifest.json'), path.join(workspace, 'artifacts/ios/manifest.json'));
    fs.writeFileSync(log, '');
    const result = spawnSync('sh', ['-c', require('../package.json').scripts.ios + ' --device Fixture'], {
      cwd: workspace, encoding: 'utf8', env: { ...process.env, PATH: bin + path.delimiter + process.env.PATH,
        PENNEL_PEER_TRANSPORT_QA: selected ? '1' : '0', PENNEL_VARIANT: variant, PENNEL_TESTFLIGHT: '0',
        FIXTURE: prepared, COMMAND_LOG: log, PRESERVE_PODS: preservePods ? '1' : '0' },
    });
    return { result, commands: fs.readFileSync(log, 'utf8').trim().split('\n') };
  }
  for (const [enabled, refresh, variant] of [[false, true], [false, false], [false, true, 'production'], [false, true], [true, true], [false, true]]) {
    const { result, commands } = run(enabled, variant);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(commands.includes('npm run prebuild'), refresh);
    assert.equal(commands.includes('pod _1.16.2_ install --deployment'), refresh);
    assert.equal(commands.at(-1), 'expo run:ios --device Fixture');
  }
  const { result, commands } = run(true, 'preview', true);
  assert.notEqual(result.status, 0);
  assert(!commands.some((command) => command.startsWith('expo ')));
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
