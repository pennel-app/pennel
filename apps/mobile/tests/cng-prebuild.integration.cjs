const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const xcode = require('xcode');
const plist = require('@expo/plist').default;
const { targetsNamed, configurations } = require('../plugins/with-pennel.cjs')._test;

const workspace = path.resolve(__dirname, '..');
const objects = (section = {}) => Object.entries(section).filter(([key]) => !key.endsWith('_comment'));
const unquote = (value) => String(value ?? '').replace(/^"|"$/g, '');

function introduceStaleProjectSettings() {
  const file = path.join(workspace, 'ios/Pennel.xcodeproj/project.pbxproj');
  const project = xcode.project(file);
  project.parseSync();
  const graph = project.hash.project.objects;
  for (const name of ['PennelKeyboard', 'PennelKeyboardTests', 'PennelRuntimeTests']) {
    const [id] = targetsNamed(project, name)[0];
    for (const configuration of configurations(project, id)) {
      configuration.buildSettings.PRODUCT_BUNDLE_IDENTIFIER = '"app.pennel.stale"';
    }
  }
  const native = objects(graph.PBXGroup).find(([, group]) => unquote(group.name) === 'PennelNative')[1];
  for (const [source, targets] of [
    ['../../ios/PennelKeyboard/Sources/KeyboardRoot.swift', ['PennelKeyboard']],
    ['../../ios/PennelKeyboard/Model/KeyboardModel.swift', ['PennelKeyboard', 'PennelKeyboardTests']],
    ['../../ios/PennelKeyboardTests/KeyboardModelTests.swift', ['PennelKeyboardTests']],
  ]) {
    const reference = project.generateUuid();
    graph.PBXFileReference[reference] = { isa: 'PBXFileReference', lastKnownFileType: 'sourcecode.swift', path: JSON.stringify(source), sourceTree: 'SOURCE_ROOT' };
    graph.PBXFileReference[`${reference}_comment`] = path.basename(source);
    native.children.push({ value: reference, comment: path.basename(source) });
    for (const name of targets) {
      const [, target] = targetsNamed(project, name)[0];
      const phase = target.buildPhases.map(({ value }) => graph.PBXSourcesBuildPhase[value]).find(Boolean);
      const build = project.generateUuid();
      graph.PBXBuildFile[build] = { isa: 'PBXBuildFile', fileRef: reference };
      graph.PBXBuildFile[`${build}_comment`] = `${path.basename(source)} in Sources`;
      phase.files.push({ value: build, comment: `${path.basename(source)} in Sources` });
    }
  }
  fs.writeFileSync(file, project.writeSync());
}

function snapshot() {
  const project = xcode.project(path.join(workspace, 'ios/Pennel.xcodeproj/project.pbxproj'));
  project.parseSync();
  const graph = project.hash.project.objects;
  const expected = ['Pennel', 'ExpoWidgetsTarget', 'PennelKeyboard', 'PennelKeyboardTests', 'PennelRuntimeTests'];
  const targets = expected.map((name) => {
    const rows = targetsNamed(project, name);
    assert.equal(rows.length, 1, `unique target ${name}`);
    const [id, target] = rows[0];
    const phases = target.buildPhases.flatMap((entry) => {
      for (const kind of ['PBXSourcesBuildPhase', 'PBXResourcesBuildPhase', 'PBXFrameworksBuildPhase', 'PBXCopyFilesBuildPhase']) {
        const phase = graph[kind]?.[entry.value];
        if (phase) return [{ kind, files: (phase.files ?? []).map((file) => {
          const build = graph.PBXBuildFile[file.value];
          const reference = graph.PBXFileReference[build?.fileRef];
          return unquote(reference?.path ?? file.comment);
        }).sort() }];
      }
      return [];
    });
    const settings = configurations(project, id).map(({ name, buildSettings }) => ({
      name,
      deployment: buildSettings.IPHONEOS_DEPLOYMENT_TARGET,
      bundle: buildSettings.PRODUCT_BUNDLE_IDENTIFIER,
      module: buildSettings.PRODUCT_MODULE_NAME,
      flags: buildSettings.SWIFT_ACTIVE_COMPILATION_CONDITIONS,
      entitlement: buildSettings.CODE_SIGN_ENTITLEMENTS,
    }));
    for (const setting of settings) assert.equal(setting.deployment, '26.0');
    if (name !== 'Pennel') for (const setting of settings) assert.doesNotMatch(setting.flags ?? '', /PENNEL_APP/);
    return { name, phases, settings };
  });
  const groups = objects(graph.PBXGroup).filter(([, group]) => unquote(group.name ?? group.path) === 'ExpoWidgetsTarget');
  assert.equal(groups.length, 1, 'unique SDK widget source group');
  const keyboard = targets.find(({ name }) => name === 'PennelKeyboard');
  assert(keyboard.phases.some(({ files }) => files.some((file) => file.endsWith('KeyboardViewController.swift'))));
  const runtime = targets.find(({ name }) => name === 'PennelRuntimeTests');
  for (const file of fs.readdirSync(path.join(workspace, 'native/ios/Tests')).filter((file) => file.endsWith('.swift'))) {
    assert(runtime.phases.some(({ kind, files }) => kind === 'PBXSourcesBuildPhase' && files.some((source) => source.endsWith(`/native/ios/Tests/${file}`))), `hosted runtime test membership for ${file}`);
  }
  for (const target of targets) for (const phase of target.phases) {
    for (const file of phase.files) assert.doesNotMatch(file, /^\.\.\/\.\.\/ios\//, 'native compile ownership does not reach a retired application');
  }
  const widget = targets.find(({ name }) => name === 'ExpoWidgetsTarget');
  assert(widget.phases.some(({ files }) => files.some((file) => file.endsWith('PennelIntents.swift'))));
  assert(widget.phases.some(({ files }) => files.some((file) => file.endsWith('RecordingLiveActivity.swift'))));
  const widgetSource = fs.readFileSync(path.join(workspace, 'ios/ExpoWidgetsTarget/index.swift'), 'utf8');
  assert.match(widgetSource, /SessionLiveActivity\(\)/);
  assert.match(widgetSource, /RecordingLiveActivity\(\)/);
  const keyboardInfo = plist.parse(fs.readFileSync(path.join(workspace, 'ios/PennelKeyboard/Info.plist'), 'utf8'));
  assert.equal(keyboardInfo.PennelAppGroupIdentifier, 'group.app.pennel.preview');
  assert.equal(keyboardInfo.NSExtension.NSExtensionAttributes.RequestsOpenAccess, true);
  const podfile = fs.readFileSync(path.join(workspace, 'ios/Podfile'), 'utf8').trim();
  const entitlements = plist.parse(fs.readFileSync(path.join(workspace, 'ios/PennelKeyboard/PennelKeyboard.entitlements'), 'utf8'));
  const scheme = fs.readFileSync(path.join(workspace, 'ios/Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme'), 'utf8');
  const appDelegate = fs.readFileSync(path.join(workspace, 'ios/Pennel/AppDelegate.swift'), 'utf8');
  assert.match(appDelegate, /class AppDelegate: ExpoAppDelegate, @preconcurrency ExpoReactNativeFactoryProvider/);
  const testables = [...scheme.matchAll(/<TestableReference\b[\s\S]*?<\/TestableReference>/g)].map(([reference]) => reference.match(/BlueprintName\s*=\s*"([^"]+)"/)?.[1]);
  assert.deepEqual(testables, ['PennelKeyboardTests', 'PennelRuntimeTests']);
  return { targets, podfile, keyboardInfo, entitlements, widgetSource, testables, appDelegate };
}

test('clean and repeated non-clean Prebuild retain equivalent Pennel target ownership', { timeout: 180_000 }, () => {
  const run = (script) => execFileSync('npm', ['run', script], { cwd: workspace, stdio: 'pipe', timeout: 60_000, env: { ...process.env, CI: '1', PENNEL_VARIANT: 'preview' } });
  run('prebuild:clean');
  const initial = snapshot();
  introduceStaleProjectSettings();
  run('prebuild');
  assert.deepEqual(snapshot(), initial);
  run('prebuild');
  assert.deepEqual(snapshot(), initial);
  run('prebuild:clean');
  assert.deepEqual(snapshot(), initial);
});
