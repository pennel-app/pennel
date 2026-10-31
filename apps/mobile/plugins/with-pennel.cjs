const fs = require('node:fs');
const path = require('node:path');
const plist = require('@expo/plist').default;
const { withBaseMod, withInfoPlist, withPodfileProperties } = require('expo/config-plugins');
const { prepareNativeSources } = require('./native-sources.cjs');

const keyboardName = 'PennelKeyboard';
const widgetName = 'ExpoWidgetsTarget';
const keyboardTestsName = 'PennelKeyboardTests';
const runtimeTestsName = 'PennelRuntimeTests';
const unquote = (value) => String(value ?? '').replace(/^"|"$/g, '');
const quote = (value) => JSON.stringify(value);
const realEntries = (section = {}) => Object.entries(section).filter(([key]) => !key.endsWith('_comment'));

function afterExistingMod(config, mod, action, before) {
  return withBaseMod(config, {
    platform: 'ios', mod, isProvider: false,
    async action({ modRequest: { nextMod, ...modRequest }, ...current }) {
      const prepared = before ? await before({ ...current, modRequest }) : { ...current, modRequest };
      const generated = await nextMod(prepared);
      return action(generated);
    },
  });
}

function writePlist(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, plist.build(value));
}

function patchPodfile(content) {
  const declaration = "  pod 'PennelMobileKit', :path => '../native/ios/MobileKit'";
  content = content.replace(/^\s*pod 'PennelMobileKit', :path => '[^']*'\r?\n/gm, '');
  content = content.replace(/\n  # BEGIN PENNEL RUNTIME TESTS[\s\S]*?  # END PENNEL RUNTIME TESTS\n?/g, '\n');
  for (const target of ['Pennel', widgetName]) {
    const targetStart = new RegExp(`(target ['\"]${target}['\"] do\\n)`);
    if (!targetStart.test(content)) throw new Error(`Pennel CNG requires the ${target} target before pod configuration`);
    content = content.replace(targetStart, `$1${declaration}\n`);
  }
  content = content.replace(/(target 'Pennel' do\n)/, `$1  # BEGIN PENNEL RUNTIME TESTS\n  target '${runtimeTestsName}' do\n    inherit! :search_paths\n  end\n  # END PENNEL RUNTIME TESTS\n`);
  const helpers = "require_relative '../plugins/pod_helpers'";
  if (!content.includes(helpers)) content = `${helpers}\n${content}`;
  if (!content.includes('pennel_post_install(installer)')) content = content.replace(/(post_install do \|installer\|\n)/, '$1    pennel_post_install(installer)\n');
  content = content.replace(/\n# BEGIN PENNEL KEYBOARD PODS[\s\S]*?# END PENNEL KEYBOARD PODS\n?/g, '\n');
  const typing = "  pod 'PennelTyping', :path => '../native/ios/Typing'";
  content += `\n# BEGIN PENNEL KEYBOARD PODS\ntarget '${keyboardName}' do\n${declaration}\n${typing}\nend\ntarget '${keyboardTestsName}' do\n${declaration}\n${typing}\nend\n# END PENNEL KEYBOARD PODS\n`;
  const lines = content.split('\n');
  return `${lines.filter((line, index) => line !== declaration || lines[index - 1] !== declaration).join('\n').replace(/\n{3,}/g, '\n\n').trimEnd()}\n`;
}

function variantInfo(options) {
  return {
    PennelAppGroupIdentifier: options.groupIdentifier,
    PennelHandoffNamespace: options.production ? 'app.pennel.handoff' : `${options.bundleIdentifier}.handoff`,
    PennelKeychainService: options.bundleIdentifier,
    PennelURLScheme: options.scheme,
    PennelSyncTaskIdentifier: `${options.bundleIdentifier}.sync`,
    PennelFinishTaskIdentifier: `${options.bundleIdentifier}.finish`,
  };
}

function patchAppDelegateSwift(contents) {
  const conformance = /(\bclass\s+AppDelegate\s*:\s*ExpoAppDelegate\s*,\s*)(?:@preconcurrency\s+)?ExpoReactNativeFactoryProvider\b/;
  if (!conformance.test(contents)) throw new Error('Pennel expects the SDK 58 Swift AppDelegate factory-provider conformance');
  // SDK 58's protocol is nonisolated while its UIKit AppDelegate is main-actor isolated.
  return contents.replace(conformance, '$1@preconcurrency ExpoReactNativeFactoryProvider');
}

function prepareNativeFiles(config, options) {
  const ios = config.modRequest.platformProjectRoot;
  const native = path.join(ios, 'PennelNative');
  prepareNativeSources(config.modRequest.projectRoot);
  fs.mkdirSync(native, { recursive: true });
  const keyboardInfo = plist.parse(fs.readFileSync(path.join(config.modRequest.projectRoot, 'native/ios/Keyboard/Info.plist'), 'utf8'));
  Object.assign(keyboardInfo, variantInfo(options), {
    CFBundleDisplayName: options.production ? 'Pennel' : 'Pennel Preview',
    CFBundleDevelopmentRegion: 'en',
    UIAppFonts: ['Figtree-Regular.ttf', 'Figtree-Medium.ttf', 'Figtree-SemiBold.ttf'],
  });
  keyboardInfo.NSExtension.NSExtensionAttributes.IsASCIICapable = true;
  keyboardInfo.NSExtension.NSExtensionAttributes.PrimaryLanguage = 'en';
  writePlist(path.join(ios, keyboardName, 'Info.plist'), keyboardInfo);
  writePlist(path.join(ios, keyboardName, `${keyboardName}.entitlements`), {
    'com.apple.security.application-groups': [options.groupIdentifier],
  });
  const widgetInfoPath = path.join(ios, widgetName, 'Info.plist');
  if (!fs.existsSync(widgetInfoPath)) throw new Error('Run the expo-widgets generator before Pennel native configuration');
  const widgetInfo = plist.parse(fs.readFileSync(widgetInfoPath, 'utf8'));
  Object.assign(widgetInfo, variantInfo(options), { UIAppFonts: keyboardInfo.UIAppFonts });
  writePlist(widgetInfoPath, widgetInfo);
  const podsPath = path.join(ios, 'Podfile');
  fs.writeFileSync(podsPath, patchPodfile(fs.readFileSync(podsPath, 'utf8')));
  const propertiesPath = path.join(ios, 'Podfile.properties.json');
  const properties = JSON.parse(fs.readFileSync(propertiesPath, 'utf8'));
  properties['ios.deploymentTarget'] = '26.0';
  fs.writeFileSync(propertiesPath, `${JSON.stringify(properties, null, 2)}\n`);
  return config;
}

function targetsNamed(project, name) {
  return realEntries(project.pbxNativeTargetSection()).filter(([, target]) => unquote(target.name) === name);
}

function removeLegacyKeyboardSources(project) {
  const objects = project.hash.project.objects;
  const legacy = /^\.\.\/\.\.\/ios\/(?:PennelKeyboard\/(?:Sources|Model)\/|PennelKeyboardTests\/).*\.swift$/;
  const references = new Set(realEntries(objects.PBXFileReference)
    .filter(([, file]) => unquote(file.sourceTree) === 'SOURCE_ROOT' && legacy.test(unquote(file.path)))
    .map(([id]) => id));
  if (!references.size) return;
  const builds = new Set(realEntries(objects.PBXBuildFile).filter(([, file]) => references.has(unquote(file.fileRef))).map(([id]) => id));
  for (const entries of Object.values(objects)) {
    for (const [, entry] of realEntries(entries)) {
      if (entry.files) entry.files = entry.files.filter((file) => !builds.has(file.value));
      if (entry.children) entry.children = entry.children.filter((child) => !references.has(child.value));
    }
  }
  for (const id of builds) {
    delete objects.PBXBuildFile[id];
    delete objects.PBXBuildFile[`${id}_comment`];
  }
  for (const id of references) {
    delete objects.PBXFileReference[id];
    delete objects.PBXFileReference[`${id}_comment`];
  }
}

function removeTargets(project, name) {
  const objects = project.hash.project.objects;
  const removed = new Set(targetsNamed(project, name).map(([id]) => id));
  if (!removed.size) return;
  const removedRefs = new Set();
  const removedGroups = new Set();
  const remove = (section, id) => { if (objects[section]) { delete objects[section][id]; delete objects[section][`${id}_comment`]; } };
  for (const [id, group] of realEntries(objects.PBXGroup)) {
    if (unquote(group.name) === name || unquote(group.path) === name) {
      removedGroups.add(id);
      for (const child of group.children ?? []) removedRefs.add(child.value);
    }
  }
  for (const id of removed) {
    const target = objects.PBXNativeTarget[id];
    removedRefs.add(target.productReference);
    for (const phase of target.buildPhases ?? []) {
      for (const section of ['PBXSourcesBuildPhase', 'PBXResourcesBuildPhase', 'PBXFrameworksBuildPhase', 'PBXShellScriptBuildPhase', 'PBXCopyFilesBuildPhase']) {
        if (objects[section]?.[phase.value]) {
          for (const file of objects[section][phase.value].files ?? []) remove('PBXBuildFile', file.value);
          remove(section, phase.value);
        }
      }
    }
    const list = objects.XCConfigurationList?.[target.buildConfigurationList];
    for (const entry of list?.buildConfigurations ?? []) remove('XCBuildConfiguration', entry.value);
    remove('XCConfigurationList', target.buildConfigurationList);
    remove('PBXNativeTarget', id);
  }
  const removedDependencies = new Set();
  for (const [id, dep] of realEntries(objects.PBXTargetDependency)) if (removed.has(dep.target)) { removedDependencies.add(id); remove('PBXTargetDependency', id); }
  for (const [id, proxy] of realEntries(objects.PBXContainerItemProxy)) if (removed.has(proxy.remoteGlobalIDString)) remove('PBXContainerItemProxy', id);
  const removedBuildFiles = new Set();
  for (const [id, file] of realEntries(objects.PBXBuildFile)) if (removedRefs.has(file.fileRef)) { removedBuildFiles.add(id); remove('PBXBuildFile', id); }
  for (const [section, entries] of Object.entries(objects)) {
    for (const [, entry] of realEntries(entries)) {
      if (section === 'PBXProject') {
        entry.targets = entry.targets?.filter((target) => !removed.has(target.value));
        for (const id of removed) if (entry.attributes?.TargetAttributes) delete entry.attributes.TargetAttributes[id];
      }
      if (section === 'PBXNativeTarget') entry.dependencies = entry.dependencies?.filter((dep) => !removedDependencies.has(dep.value));
      if (entry.files) entry.files = entry.files.filter((file) => !removedBuildFiles.has(file.value));
      if (entry.children) entry.children = entry.children.filter((file) => !removedRefs.has(file.value) && !removedGroups.has(file.value));
    }
  }
  for (const id of removedRefs) remove('PBXFileReference', id);
  for (const id of removedGroups) remove('PBXGroup', id);
}

function ensureGroup(project) {
  const section = project.hash.project.objects.PBXGroup;
  let entry = realEntries(section).find(([, group]) => unquote(group.name) === 'PennelNative');
  if (entry) return entry[0];
  const group = project.addPbxGroup([], 'PennelNative');
  const root = project.getPBXGroupByKey(project.getFirstProject().firstProject.mainGroup);
  if (!root.children.some((child) => child.value === group.uuid)) root.children.push({ value: group.uuid, comment: 'PennelNative' });
  return group.uuid;
}

function fileReference(project, relativePath, type) {
  const objects = project.hash.project.objects;
  objects.PBXFileReference ??= {};
  const existing = realEntries(objects.PBXFileReference).find(([, file]) => unquote(file.path) === relativePath);
  if (existing) return existing[0];
  const id = project.generateUuid();
  objects.PBXFileReference[id] = { isa: 'PBXFileReference', lastKnownFileType: type, path: quote(relativePath), sourceTree: 'SOURCE_ROOT' };
  objects.PBXFileReference[`${id}_comment`] = path.basename(relativePath);
  project.getPBXGroupByKey(ensureGroup(project)).children.push({ value: id, comment: path.basename(relativePath) });
  return id;
}

function addFileToPhase(project, targetId, relativePath, phaseType, type) {
  const objects = project.hash.project.objects;
  const target = objects.PBXNativeTarget[targetId];
  let phase = target.buildPhases?.map((entry) => objects[phaseType]?.[entry.value]).find(Boolean);
  if (!phase) {
    project.addBuildPhase([], phaseType, phaseType === 'PBXSourcesBuildPhase' ? 'Sources' : 'Resources', targetId);
    phase = target.buildPhases.map((entry) => objects[phaseType]?.[entry.value]).find(Boolean);
  }
  const ref = fileReference(project, relativePath, type);
  objects.PBXBuildFile ??= {};
  if (phase.files.some((entry) => objects.PBXBuildFile[entry.value]?.fileRef === ref)) return;
  const id = project.generateUuid();
  const comment = `${path.basename(relativePath)} in ${phaseType === 'PBXSourcesBuildPhase' ? 'Sources' : 'Resources'}`;
  objects.PBXBuildFile[id] = { isa: 'PBXBuildFile', fileRef: ref, fileRef_comment: path.basename(relativePath) };
  objects.PBXBuildFile[`${id}_comment`] = comment;
  phase.files.push({ value: id, comment });
}

function configurations(project, targetId) {
  const objects = project.hash.project.objects;
  const list = objects.XCConfigurationList[objects.PBXNativeTarget[targetId].buildConfigurationList];
  return list.buildConfigurations.map((entry) => objects.XCBuildConfiguration[entry.value]);
}

function swiftFiles(directory) {
  if (!fs.existsSync(directory)) return [];
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(directory, entry.name);
    return entry.isDirectory() ? swiftFiles(full) : entry.name.endsWith('.swift') ? [full] : [];
  }).sort();
}

function configureProject(config, options) {
  const intentProbe = keyboardFeasibilityEnabled(options);
  const project = config.modResults;
  removeLegacyKeyboardSources(project);
  const ios = config.modRequest.platformProjectRoot;
  const workspace = config.modRequest.projectRoot;
  const root = path.resolve(workspace, '../..');
  const app = realEntries(project.pbxNativeTargetSection()).find(([, target]) => unquote(target.productType) === 'com.apple.product-type.application');
  if (!app) throw new Error('Pennel app target was not generated');
  const widget = targetsNamed(project, widgetName)[0];
  if (!widget) throw new Error('Expo Widgets target must be generated before Pennel adds native integrations');
  const keyboard = targetsNamed(project, keyboardName)[0] ?? (() => {
    const added = project.addTarget(keyboardName, 'app_extension', keyboardName, `${options.bundleIdentifier}.keyboard`);
    return [added.uuid, added.pbxNativeTarget];
  })();
  const test = targetsNamed(project, keyboardTestsName)[0] ?? (() => {
    const added = project.addTarget(keyboardTestsName, 'unit_test_bundle', keyboardTestsName, `${options.bundleIdentifier}.keyboard-tests`);
    // Unit tests are built by the scheme, never embedded or made an app dependency.
    const objects = project.hash.project.objects;
    const deps = realEntries(objects.PBXTargetDependency).filter(([, dep]) => dep.target === added.uuid).map(([id]) => id);
    app[1].dependencies = app[1].dependencies.filter((entry) => !deps.includes(entry.value));
    return [added.uuid, added.pbxNativeTarget];
  })();
  const runtimeTest = targetsNamed(project, runtimeTestsName)[0] ?? (() => {
    const added = project.addTarget(runtimeTestsName, 'unit_test_bundle', runtimeTestsName, `${options.bundleIdentifier}.runtime-tests`);
    const objects = project.hash.project.objects;
    const deps = realEntries(objects.PBXTargetDependency).filter(([, dep]) => dep.target === added.uuid).map(([id]) => id);
    app[1].dependencies = app[1].dependencies.filter((entry) => !deps.includes(entry.value));
    project.addTargetDependency(added.uuid, [app[0]]);
    return [added.uuid, added.pbxNativeTarget];
  })();
  const relative = (full) => path.relative(ios, full).split(path.sep).join('/');
  const intent = path.join(workspace, 'native/ios/Shared/PennelIntents.swift');
  if (!fs.existsSync(intent)) throw new Error('Pennel native App Intent source is missing outside the generated project');
  for (const target of [app, widget]) {
    addFileToPhase(project, target[0], relative(intent), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  }
  const probe = path.join(workspace, 'native/ios/Keyboard/Feasibility/KeyboardRecordingIntentProbe.swift');
  for (const target of [app, keyboard]) addFileToPhase(project, target[0], relative(probe), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  const appIntents = path.join(workspace, 'native/ios/App');
  for (const file of swiftFiles(appIntents)) addFileToPhase(project, app[0], relative(file), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  const keyboardFiles = [...swiftFiles(path.join(workspace, 'native/ios/Keyboard/Sources')), ...swiftFiles(path.join(workspace, 'native/ios/Keyboard/Model'))];
  for (const file of keyboardFiles) addFileToPhase(project, keyboard[0], relative(file), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  const testedKeyboardSources = ['KeyboardTypingModel.swift', 'KeyboardLetterGrid.swift', 'KeyboardSystemKeys.swift']
    .map((name) => path.join(workspace, 'native/ios/Keyboard/Sources', name)).filter((file) => fs.existsSync(file));
  for (const file of [...swiftFiles(path.join(workspace, 'native/ios/Keyboard/Model')), ...testedKeyboardSources, ...swiftFiles(path.join(workspace, 'native/ios/KeyboardTests'))]) addFileToPhase(project, test[0], relative(file), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  for (const file of swiftFiles(path.join(workspace, 'native/ios/Tests'))) addFileToPhase(project, runtimeTest[0], relative(file), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  for (const file of swiftFiles(path.join(workspace, 'native/ios/Widgets'))) addFileToPhase(project, widget[0], relative(file), 'PBXSourcesBuildPhase', 'sourcecode.swift');
  for (const target of [keyboard, widget]) {
    for (const name of ['Figtree-Regular.ttf', 'Figtree-Medium.ttf', 'Figtree-SemiBold.ttf']) addFileToPhase(project, target[0], relative(path.join(root, 'assets/brand/fonts', name)), 'PBXResourcesBuildPhase', 'file');
  }
  for (const [id] of [app, widget, keyboard, test, runtimeTest]) {
    for (const configuration of configurations(project, id)) {
      const settings = configuration.buildSettings;
      settings.IPHONEOS_DEPLOYMENT_TARGET = '26.0';
      settings.SWIFT_VERSION = '6.0';
      const probeFlags = unquote(settings.SWIFT_ACTIVE_COMPILATION_CONDITIONS || '$(inherited)')
        .split(/\s+/).filter((flag) => flag !== 'PENNEL_KEYBOARD_FEASIBILITY');
      if (intentProbe && (id === app[0] || id === keyboard[0])) probeFlags.push('PENNEL_KEYBOARD_FEASIBILITY');
      settings.SWIFT_ACTIVE_COMPILATION_CONDITIONS = quote(probeFlags.join(' '));
      if (id === app[0]) {
        const flags = unquote(settings.SWIFT_ACTIVE_COMPILATION_CONDITIONS || '$(inherited)').split(/\s+/);
        if (!flags.includes('PENNEL_APP')) flags.push('PENNEL_APP');
        settings.SWIFT_ACTIVE_COMPILATION_CONDITIONS = quote(flags.join(' '));
      } else if (id === keyboard[0]) {
        Object.assign(settings, {
          APPLICATION_EXTENSION_API_ONLY: 'YES',
          CODE_SIGN_ENTITLEMENTS: quote(`${keyboardName}/${keyboardName}.entitlements`),
          INFOPLIST_FILE: quote(`${keyboardName}/Info.plist`),
          PRODUCT_MODULE_NAME: keyboardName,
          CURRENT_PROJECT_VERSION: quote(config.ios?.buildNumber ?? '1'),
          MARKETING_VERSION: quote(config.version ?? '0.1.0'),
          TARGETED_DEVICE_FAMILY: quote('1,2'),
          GENERATE_INFOPLIST_FILE: 'NO',
        });
      } else if (id === test[0] || id === runtimeTest[0]) {
        Object.assign(settings, { GENERATE_INFOPLIST_FILE: 'YES', ENABLE_TESTING_SEARCH_PATHS: 'YES', CODE_SIGNING_ALLOWED: 'NO', PRODUCT_MODULE_NAME: id === test[0] ? keyboardTestsName : runtimeTestsName, TARGETED_DEVICE_FAMILY: quote('1,2') });
        if (id === runtimeTest[0]) {
          settings.TEST_HOST = quote('$(BUILT_PRODUCTS_DIR)/Pennel.app/Pennel');
          settings.BUNDLE_LOADER = quote('$(TEST_HOST)');
        }
        delete settings.INFOPLIST_FILE;
      }
    }
  }
  const widgetIndex = path.join(ios, widgetName, 'index.swift');
  let source = fs.readFileSync(widgetIndex, 'utf8');
  if (!source.includes('SessionLiveActivity()')) source = source.replace('WidgetLiveActivity()', 'WidgetLiveActivity()\n    SessionLiveActivity()\n    RecordingLiveActivity()\n    DictateControl()\n    DictationSessionControl()\n    RecordingControl()');
  fs.writeFileSync(widgetIndex, source);
  configureScheme(ios, [[keyboardTestsName, test[0]], [runtimeTestsName, runtimeTest[0]]], new Set(realEntries(project.pbxNativeTargetSection()).map(([id]) => id)));
  return config;
}

function keyboardFeasibilityEnabled(options, environment = process.env) {
  const enabled = environment.PENNEL_KEYBOARD_FEASIBILITY === '1';
  if (enabled && options.production) throw new Error('Keyboard process-routing probes are restricted to Pennel Preview');
  return enabled;
}

function configureScheme(ios, testTargets, targetIds) {
  const schemePath = path.join(ios, 'Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme');
  if (!fs.existsSync(schemePath)) return;
  let scheme = fs.readFileSync(schemePath, 'utf8');
  scheme = scheme.replace(/<TestableReference\b[\s\S]*?<\/TestableReference>/g, (reference) => {
    const id = reference.match(/BlueprintIdentifier\s*=\s*"([^"]+)"/)?.[1];
    return targetIds.has(id) ? reference : '';
  });
  for (const [name, testId] of testTargets) if (!scheme.includes(`BlueprintName="${name}"`)) {
    const testable = `<TestableReference skipped="NO"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="${testId}" BuildableName="${name}.xctest" BlueprintName="${name}" ReferencedContainer="container:Pennel.xcodeproj"/></TestableReference>`;
    scheme = scheme.replace(/<Testables\s*\/>/, `<Testables>${testable}</Testables>`).replace('</Testables>', `${scheme.includes('<Testables/>') || scheme.includes('<Testables />') ? '' : testable}</Testables>`);
    fs.writeFileSync(schemePath, scheme);
  }
}

function withPennel(config, options) {
  config = withInfoPlist(config, (mod) => {
    Object.assign(mod.modResults, variantInfo(options));
    return mod;
  });
  config = withPodfileProperties(config, (mod) => {
    mod.modResults['ios.deploymentTarget'] = '26.0';
    return mod;
  });
  config = afterExistingMod(config, 'appDelegate', (mod) => {
    if (mod.modResults.language !== 'swift') throw new Error('Pennel requires the SDK 58 Swift application entry point');
    mod.modResults.contents = patchAppDelegateSwift(mod.modResults.contents);
    return mod;
  });
  config = afterExistingMod(config, 'dangerous', (mod) => prepareNativeFiles(mod, options));
  config = afterExistingMod(config, 'xcodeproj', (mod) => configureProject(mod, options), (mod) => {
    // Expo Widgets recreates its target on non-clean Prebuild; remove the old graph first.
    removeTargets(mod.modResults, widgetName);
    return mod;
  });
  return config;
}

module.exports = withPennel;
module.exports._test = { afterExistingMod, patchPodfile, variantInfo, patchAppDelegateSwift, removeLegacyKeyboardSources, removeTargets, targetsNamed, configureProject, configurations, keyboardFeasibilityEnabled };
