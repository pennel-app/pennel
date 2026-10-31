const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { prepareNativeSources, serviceFiles } = require('../plugins/native-sources.cjs');

function fixture(context) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'pennel-native-sources-')));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const workspace = path.join(root, 'apps/mobile');
  for (const file of serviceFiles) {
    const source = path.join(workspace, 'native/ios/Services', file);
    fs.mkdirSync(path.dirname(source), { recursive: true });
    fs.writeFileSync(source, `canonical ${file}\n`);
  }
  const bindings = path.join(workspace, 'artifacts/ios/PennelCore/pennel_core.swift');
  fs.mkdirSync(path.dirname(bindings), { recursive: true });
  fs.writeFileSync(bindings, 'matched bindings\n');
  fs.mkdirSync(path.join(workspace, 'artifacts/ios/PennelCoreFFI.xcframework'), { recursive: true });
  fs.mkdirSync(path.join(workspace, 'artifacts/ios/PennelTypingFFI.xcframework'), { recursive: true });
  return { root, workspace };
}

test('native preparation links mobile-owned services and artifacts without creating legacy trees', (context) => {
  const { root, workspace } = fixture(context);
  const first = prepareNativeSources(workspace);
  fs.writeFileSync(path.join(first, 'stale.swift'), 'stale\n');
  assert.equal(prepareNativeSources(workspace), first);
  assert(!fs.existsSync(path.join(first, 'stale.swift')));
  for (const file of serviceFiles) {
    const staged = path.join(first, 'Services', file);
    assert(fs.lstatSync(staged).isSymbolicLink());
    assert.equal(fs.realpathSync(staged), path.join(workspace, 'native/ios/Services', file));
  }
  assert.equal(fs.realpathSync(path.join(first, 'Bindings/pennel_core.swift')), path.join(workspace, 'artifacts/ios/PennelCore/pennel_core.swift'));
  assert.equal(fs.realpathSync(path.join(first, 'PennelCoreFFI.xcframework')), path.join(workspace, 'artifacts/ios/PennelCoreFFI.xcframework'));
  assert.equal(fs.realpathSync(path.join(workspace, 'native/ios/Typing/Generated/PennelTypingFFI.xcframework')), path.join(workspace, 'artifacts/ios/PennelTypingFFI.xcframework'));
  assert(!fs.existsSync(path.join(root, 'apps/ios')));
  assert(!fs.existsSync(path.join(root, 'apps/macos')));
});

test('missing canonical native services fail preparation instead of silently dropping compile membership', (context) => {
  const { workspace } = fixture(context);
  fs.unlinkSync(path.join(workspace, 'native/ios/Services', serviceFiles[0]));
  assert.throws(() => prepareNativeSources(workspace), /Required native service source is missing: AppModel\.swift/);
});
