const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const plist = require('@expo/plist').default;
const withPennel = require('../plugins/with-pennel.cjs');
const { serviceFiles } = require('../plugins/native-sources.cjs');
const { afterExistingMod, patchPodfile, variantInfo, patchAppDelegateSwift, keyboardFeasibilityEnabled } = require('../plugins/with-pennel.cjs')._test;

test('Pennel mods finish after the already-registered Expo generators', async () => {
  const calls = [];
  const config = { mods: { ios: { dangerous: async (mod) => { calls.push('expo'); return mod; } } } };
  afterExistingMod(config, 'dangerous', (mod) => { calls.push('pennel'); return mod; });
  await config.mods.ios.dangerous({ modRequest: {} });
  assert.deepEqual(calls, ['expo', 'pennel']);
});

test('the stale widget graph is removed before SDK generation, native membership added after', async () => {
  const calls = [];
  const config = { mods: { ios: { xcodeproj: async (mod) => { calls.push('expo'); return mod; } } } };
  afterExistingMod(config, 'xcodeproj', (mod) => { calls.push('membership'); return mod; }, (mod) => { calls.push('prune'); return mod; });
  await config.mods.ios.xcodeproj({ modRequest: {} });
  assert.deepEqual(calls, ['prune', 'expo', 'membership']);
});

test('pod composition is idempotent and keyboard targets own only lightweight helpers', () => {
  const original = "target 'Pennel' do\n  use_expo_modules!\nend\n\ntarget \"ExpoWidgetsTarget\" do\n  use_expo_modules_widgets!\nend\n";
  const once = patchPodfile(original);
  assert.equal(patchPodfile(once), once);
  assert.match(once, /pod 'PennelMobileKit', :path => '\.\.\/native\/ios\/MobileKit'/);
  assert.doesNotMatch(once, /\.\.\/\.\.\/ios\//);
  for (const name of ['PennelKeyboard', 'PennelKeyboardTests']) {
    const block = once.match(new RegExp(`target '${name}' do\\n([\\s\\S]*?)\\nend`))[1];
    assert.match(block, /pod 'PennelMobileKit'/);
    assert.match(block, /pod 'PennelTyping'/);
    assert.doesNotMatch(block, /use_expo|use_react_native|PennelMobile'/);
  }
});

test('all shipping plists declare the RN architecture before Pods can rewrite sealed files', async (t) => {
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-plist-architecture-'));
  t.after(() => fs.rmSync(workspace, { recursive: true, force: true }));
  const ios = path.join(workspace, 'ios');
  for (const file of serviceFiles) {
    const destination = path.join(workspace, 'native/ios/Services', file);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.writeFileSync(destination, 'fixture');
  }
  const keyboard = path.join(workspace, 'native/ios/Keyboard/Info.plist');
  fs.mkdirSync(path.dirname(keyboard), { recursive: true });
  fs.copyFileSync(path.resolve(__dirname, '../native/ios/Keyboard/Info.plist'), keyboard);
  fs.mkdirSync(path.join(ios, 'ExpoWidgetsTarget'), { recursive: true });
  fs.writeFileSync(path.join(ios, 'ExpoWidgetsTarget/Info.plist'), plist.build({ RCTNewArchEnabled: false }));
  fs.writeFileSync(path.join(ios, 'Podfile'), "target 'Pennel' do\nend\n\ntarget 'ExpoWidgetsTarget' do\nend\n");
  fs.writeFileSync(path.join(ios, 'Podfile.properties.json'), '{}\n');
  const options = { bundleIdentifier: 'app.pennel.ios.preview', groupIdentifier: 'group.app.pennel.preview', scheme: 'pennel-preview', production: false };
  const config = withPennel({ mods: { ios: { infoPlist: async (mod) => mod, dangerous: async (mod) => mod } } }, options);
  const app = await config.mods.ios.infoPlist({ modRequest: {}, modResults: { RCTNewArchEnabled: false } });
  assert.equal(app.modResults.RCTNewArchEnabled, true);
  const request = { modRequest: { projectRoot: workspace, platformProjectRoot: ios } };
  await config.mods.ios.dangerous(request);
  const files = ['PennelKeyboard/Info.plist', 'ExpoWidgetsTarget/Info.plist'].map((file) => path.join(ios, file));
  const original = files.map((file) => fs.readFileSync(file, 'utf8'));
  for (const contents of original) assert.equal(plist.parse(contents).RCTNewArchEnabled, true);
  await config.mods.ios.dangerous(request);
  assert.deepEqual(files.map((file) => fs.readFileSync(file, 'utf8')), original);
});

test('keyboard routing probe is explicitly opt-in and cannot enter production', () => {
  assert.equal(keyboardFeasibilityEnabled({ production: false }, {}), false);
  assert.equal(keyboardFeasibilityEnabled({ production: false }, { PENNEL_KEYBOARD_FEASIBILITY: '1' }), true);
  assert.throws(() => keyboardFeasibilityEnabled({ production: true }, { PENNEL_KEYBOARD_FEASIBILITY: '1' }), /restricted to Pennel Preview/);
});

test('pod composition replaces legacy helper package paths during non-clean generation', () => {
  const previous = "target 'Pennel' do\n  pod 'PennelMobileKit', :path => '../../ios/PennelMobileKit'\nend\n\ntarget \"ExpoWidgetsTarget\" do\n  pod 'PennelMobileKit', :path => '../../ios/PennelMobileKit'\nend\n";
  const patched = patchPodfile(previous);
  assert.doesNotMatch(patched, /\.\.\/\.\.\/ios\//);
  assert.equal((patched.match(/pod 'PennelMobileKit'/g) ?? []).length, 4);
  assert.equal(patchPodfile(patched), patched);
});

test('preview group, handoff and Keychain identifiers cannot overlap production', () => {
  const preview = variantInfo({ bundleIdentifier: 'app.pennel.ios.preview', groupIdentifier: 'group.app.pennel.preview', scheme: 'pennel-preview', production: false });
  const production = variantInfo({ bundleIdentifier: 'app.pennel.ios', groupIdentifier: 'group.app.pennel', scheme: 'pennel', production: true });
  for (const key of Object.keys(preview)) assert.notEqual(preview[key], production[key], key);
});

test('SDK factory conformance compatibility is narrow and idempotent', () => {
  const source = '@main\nclass AppDelegate: ExpoAppDelegate, ExpoReactNativeFactoryProvider {\n  var window: UIWindow?\n}\n';
  const expected = source.replace(', ExpoReactNativeFactoryProvider', ', @preconcurrency ExpoReactNativeFactoryProvider');
  assert.equal(patchAppDelegateSwift(source), expected);
  assert.equal(patchAppDelegateSwift(expected), expected);
  assert.throws(() => patchAppDelegateSwift('class AppDelegate: ChangedSDKDelegate {}'), /SDK 58/);
});

test('native source and core artifacts are outside generated projects', () => {
  const root = path.resolve(__dirname, '..');
  const module = JSON.parse(fs.readFileSync(path.join(root, 'modules/pennel-mobile/expo-module.config.json')));
  assert.deepEqual(module.apple.modules, ['PennelMobileModule']);
  assert.deepEqual(module.apple.appDelegateSubscribers, ['PennelAppDelegateSubscriber']);
  assert.equal(module.apple.podspecPath, 'PennelMobile.podspec');
  assert.equal(module.apple.swiftModuleName, 'PennelMobile');
  const podspec = fs.readFileSync(path.join(root, 'modules/pennel-mobile/PennelMobile.podspec'), 'utf8');
  assert.match(podspec, /generated\/ios\/PennelCoreFFI\.xcframework/);
  assert.doesNotMatch(podspec, /Shared\/\*ActivityAttributes/);
});
