const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const xcode = require('xcode');
const { serviceFiles } = require('../plugins/native-sources.cjs');

const root = path.resolve(__dirname, '..');
const support = path.join(root, 'ios/Pods/Target Support Files');
const unquote = (value) => String(value ?? '').replace(/^"|"$/g, '');
const realEntries = (section = {}) => Object.entries(section).filter(([id]) => !id.endsWith('_comment'));

function graph() {
  const file = path.join(root, 'ios/Pods/Pods.xcodeproj/project.pbxproj');
  assert(fs.existsSync(file), 'Run pod install after Prebuild before checking linked native ownership');
  const project = xcode.project(file);
  project.parseSync();
  return project.hash.project.objects;
}

function dependencies(objects, rootName) {
  const targets = { ...objects.PBXNativeTarget, ...objects.PBXAggregateTarget };
  const root = realEntries(targets).find(([, target]) => unquote(target.name) === rootName);
  assert(root, `pod aggregate target ${rootName} exists`);
  const visited = new Set();
  const names = new Set();
  function visit(id) {
    if (visited.has(id)) return;
    visited.add(id);
    const target = targets[id];
    if (!target) return;
    names.add(unquote(target.name));
    for (const entry of target.dependencies ?? []) {
      const dependency = objects.PBXTargetDependency[entry.value];
      const targetId = dependency?.target ?? objects.PBXContainerItemProxy?.[dependency?.targetProxy]?.remoteGlobalIDString;
      if (targetId) visit(targetId);
    }
  }
  visit(root[0]);
  return names;
}

function provider(name) {
  const folder = path.join(support, name);
  const files = fs.existsSync(folder) ? fs.readdirSync(folder).filter((file) => file.endsWith('.swift') && file.includes('ExpoModulesProvider')) : [];
  return files.map((file) => fs.readFileSync(path.join(folder, file), 'utf8')).join('\n');
}

function target(objects, name) {
  const found = realEntries(objects.PBXNativeTarget).find(([, item]) => unquote(item.name) === name);
  assert(found, `native pod target ${name} exists`);
  return found[1];
}

function compileSources(objects, name) {
  return target(objects, name).buildPhases.flatMap((entry) => {
    const phase = objects.PBXSourcesBuildPhase?.[entry.value];
    return (phase?.files ?? []).map((file) => {
      const build = objects.PBXBuildFile[file.value];
      return unquote(objects.PBXFileReference[build.fileRef]?.path ?? file.comment);
    });
  });
}

test('keyboard actual pod graph links only lightweight Pennel helpers', () => {
  const names = dependencies(graph(), 'Pods-PennelKeyboard');
  assert(names.has('PennelMobileKit'));
  assert(names.has('PennelTyping'));
  for (const name of names) assert.doesNotMatch(name, /PennelMobile(?!Kit)|PennelCore|Expo|React|hermes|^RN|Yoga/);
  assert.equal(provider('Pods-PennelKeyboard'), '');
});

test('widget graph, linker configuration and module registry exclude the product engine', () => {
  const names = dependencies(graph(), 'Pods-ExpoWidgetsTarget');
  assert(names.has('ExpoWidgets'));
  assert(names.has('PennelMobileKit'));
  for (const name of names) assert.doesNotMatch(name, /PennelMobile(?!Kit)|PennelCore/);
  for (const mode of ['debug', 'release']) {
    const config = fs.readFileSync(path.join(support, `Pods-ExpoWidgetsTarget/Pods-ExpoWidgetsTarget.${mode}.xcconfig`), 'utf8');
    const linkerFlags = config.split('\n').filter((line) => /^(OTHER_LDFLAGS|OTHER_SWIFT_FLAGS)\s*=/.test(line)).join('\n');
    assert.doesNotMatch(linkerFlags, /PennelMobile(?!Kit)|pennel_core|PennelCoreFFI/);
  }
  const registry = provider('Pods-ExpoWidgetsTarget');
  assert(registry.length > 0, 'generated widget module provider exists');
  assert.match(registry, /ExpoWidgets/);
  assert.doesNotMatch(registry, /PennelMobile|PennelNativeRuntime|PennelAppDelegateSubscriber/);
});

test('only the app pod graph and provider register Pennel runtime and lifecycle', () => {
  const names = dependencies(graph(), 'Pods-Pennel');
  assert(names.has('PennelMobile'));
  assert(names.has('PennelMobileKit'));
  const registry = provider('Pods-Pennel');
  assert.match(registry, /PennelMobileModule/);
  assert.match(registry, /PennelAppDelegateSubscriber/);
});

test('actual Pennel compile sources include all canonical services and generated Rust bindings', () => {
  const objects = graph();
  const sources = compileSources(objects, 'PennelMobile');
  for (const file of [...serviceFiles, 'pennel_core.swift', 'PennelMobileModule.swift', 'PennelNativeRuntime.swift']) {
    assert(sources.some((source) => source.endsWith(file)), `Swift compile membership for ${file}`);
  }
  const staged = path.join(root, 'modules/pennel-mobile/generated/ios/Services/AppModel.swift');
  assert(fs.lstatSync(staged).isSymbolicLink(), 'reused source stays linked to canonical authority');
  assert.equal(fs.realpathSync(staged), path.join(root, 'native/ios/Services/AppModel.swift'));
  for (const mode of ['debug', 'release']) {
    const config = fs.readFileSync(path.join(support, `Pods-Pennel/Pods-Pennel.${mode}.xcconfig`), 'utf8');
    const linker = config.split('\n').filter((line) => line.startsWith('OTHER_LDFLAGS =')).join('\n');
    assert.match(linker, /pennel_core|PennelCoreFFI/, 'Rust binary is linked by the app');
  }
});

test('MobileKit really compiles shared ActivityKit attributes from one canonical source', () => {
  const sources = compileSources(graph(), 'PennelMobileKit');
  assert(sources.some((source) => source.endsWith('PennelActivityAttributes.swift')));
  const source = path.join(root, 'native/ios/MobileKit/Sources/PennelMobileKit/PennelActivityAttributes.swift');
  assert(fs.lstatSync(source).isFile());
  assert(!fs.lstatSync(source).isSymbolicLink());
  assert(!fs.existsSync(path.join(root, 'native/ios/MobileKit/Generated')));
});

test('owned Swift remains strict 6 while undeclared SDK pods use stock Swift 5', () => {
  const objects = graph();
  for (const [name, expected] of [['PennelMobile', '6.0'], ['PennelMobileKit', '6.0'], ['ExpoSplashScreen', '5.0'], ['ExpoWidgets', '5.9']]) {
    const native = target(objects, name);
    const configs = objects.XCConfigurationList[native.buildConfigurationList].buildConfigurations;
    for (const entry of configs) assert.equal(unquote(objects.XCBuildConfiguration[entry.value].buildSettings.SWIFT_VERSION), expected, name);
  }
});
