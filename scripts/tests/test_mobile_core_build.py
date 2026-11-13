"""Exercise mobile artifact preparation without an installed native toolchain."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "build-mobile-core-ios.sh"
TOOL = '''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path(os.environ["PENNEL_BUILD_TEST_ROOT"])
with pathlib.Path(os.environ["PENNEL_BUILD_TEST_LOG"]).open("a") as log:
    log.write(json.dumps([name, *args]) + "\\n")
def mutate_registry(phase):
    if os.environ.get("PENNEL_BUILD_TEST_REGISTRY_MUTATION") == phase:
        (root / "assets/qualifications/meeting-assistance.json").write_text('{"schema_version":1,"adopted_records":[{"fixture":"changed during build"}]}')
    if os.environ.get("PENNEL_BUILD_TEST_SUPPORT_MUTATION") == phase:
        (root / "assets/qualifications/meeting-support-cloud-v1.json").write_text('{"schema_version":1,"runtime_policy":"changed-during-build"}')
if name == "cargo" and args[0] == "build":
    target = args[args.index("--target") + 1]
    keyboard = pathlib.Path(args[args.index("--manifest-path") + 1]).parent.name == "keyboard"
    output = (root / "core/keyboard/target" if keyboard else root / "core/target") / target / "release"
    output.mkdir(parents=True, exist_ok=True)
    stem = "libpennel_typing" if keyboard else "libpennel_core"
    contents = target.encode()
    if not keyboard:
        contents += (root / "assets/qualifications/meeting-assistance.json").read_bytes()
        if os.environ.get("PENNEL_BUILD_TEST_MISSING_SUPPORT_EMBEDDING") not in ("1", target):
            contents += (root / "assets/qualifications/meeting-support-cloud-v1.json").read_bytes()
    (output / (stem + ".a")).write_bytes(contents)
    (output / (stem + ".dylib")).write_bytes(contents)
    mutate_registry(("keyboard-" if keyboard else "core-") + target)
elif name == "cargo" and args[0] == "run":
    assert pathlib.Path(args[args.index("--library") + 1]).is_file()
    output = pathlib.Path(args[args.index("--out-dir") + 1])
    output.mkdir(parents=True, exist_ok=True)
    for file in ("pennel_core.swift", "pennel_coreFFI.h", "pennel_coreFFI.modulemap"):
        (output / file).write_text("matching " + file)
    mutate_registry("bindings")
elif name == "xcodebuild":
    for index, value in enumerate(args):
        if value == "-library": assert pathlib.Path(args[index + 1]).is_file()
        if value == "-headers": assert pathlib.Path(args[index + 1]).is_dir()
    output = pathlib.Path(args[args.index("-output") + 1])
    output.mkdir(parents=True)
    (output / "Info.plist").write_text("matched device and simulator slices")
    if output.name == "PennelCoreFFI.xcframework": mutate_registry("framework")
elif name == "git":
    mutate_registry("manifest")
    print("a" * 40)
elif name == "node":
    assert (root / "apps/mobile/artifacts/ios/manifest.json").is_file()
    assert not (root / "apps/ios").exists()
    assert not (root / "apps/macos").exists()
    mutate_registry("native-sources")
else:
    raise SystemExit("Unexpected native build command: " + name)
'''


class MobileCoreBuildTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for script in (SCRIPT, SCRIPT.with_name("build-keyboard-core-ios.sh")):
            shutil.copy2(script, scripts / script.name)
        self.core = self.root / "core"
        self.core.mkdir()
        self.source = self.core / "Cargo.toml"
        self.source.write_text("[package]\nname = 'fixture'\n")
        self.keyboard = self.core / "keyboard"
        (self.keyboard / "include").mkdir(parents=True)
        self.keyboard_source = self.keyboard / "Cargo.toml"
        self.keyboard_source.write_text("[package]\nname = 'pennel-keyboard'\n")
        (self.keyboard / "include/PennelTypingFFI.h").write_text("fixture ABI header\n")
        self.registry = self.root / "assets/qualifications/meeting-assistance.json"
        self.registry.parent.mkdir(parents=True)
        self.registry.write_text('{"schema_version":1,"adopted_records":[]}\n')
        self.support = self.registry.with_name("meeting-support-cloud-v1.json")
        self.support.write_text('{"schema_version":1,"runtime_policy":"fixture-v1"}\n')
        tools = self.root / "tools"
        tools.mkdir()
        for name in ("cargo", "xcodebuild", "git", "node"):
            executable = tools / name
            executable.write_text(TOOL)
            executable.chmod(0o755)
        self.log = self.root / "commands.jsonl"
        self.env = {**os.environ, "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "PENNEL_BUILD_TEST_ROOT": str(self.root), "PENNEL_BUILD_TEST_LOG": str(self.log)}
        self.env.pop("PENNEL_IOS_ARCHIVE_ONLY", None)
        self.artifacts = self.root / "apps/mobile/artifacts/ios"

    def prepare(self, archive_only=None, script=SCRIPT.name):
        env = dict(self.env)
        if archive_only is not None:
            env["PENNEL_IOS_ARCHIVE_ONLY"] = archive_only
        return subprocess.run(["bash", str(self.root / "scripts" / script)], env=env,
                              capture_output=True, text=True)

    def commands(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def assert_build_targets(self, commands, targets):
        builds = [args for name, *args in commands if name == "cargo" and args[0] == "build"]
        self.assertEqual(len(builds), len(targets) * 2)
        for crate in (self.core, self.keyboard):
            crate_builds = [args for args in builds if Path(args[args.index("--manifest-path") + 1]).parent == crate]
            self.assertEqual([args[args.index("--target") + 1] for args in crate_builds], targets)
            self.assertTrue(all(("--no-default-features" in args) == (crate == self.core) for args in crate_builds))
        frameworks = [args for name, *args in commands if name == "xcodebuild"]
        self.assertEqual(len(frameworks), 2)
        for args in frameworks:
            libraries = [args[index + 1] for index, value in enumerate(args) if value == "-library"]
            self.assertEqual(len(libraries), len(targets))
            self.assertEqual([Path(library).parts[-3] for library in libraries], targets)

    def test_preparation_owns_matched_artifacts_without_legacy_trees(self):
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.artifacts / "PennelCoreFFI.xcframework/Info.plist").is_file())
        self.assertTrue((self.artifacts / "PennelTypingFFI.xcframework/Info.plist").is_file())
        bindings = self.artifacts / "PennelCore/pennel_core.swift"
        manifest = json.loads((self.artifacts / "manifest.json").read_text())
        self.assertEqual(manifest["revision"], "a" * 40)
        self.assertEqual(manifest["features"], [])
        self.assertEqual(manifest["targets"], ["aarch64-apple-ios", "aarch64-apple-ios-sim"])
        self.assertEqual(manifest["bindings_sha256"], hashlib.sha256(bindings.read_bytes()).hexdigest())
        digest = hashlib.sha256(b"core/Cargo.toml" + self.source.read_bytes()
                                + b"core/keyboard/Cargo.toml" + self.keyboard_source.read_bytes()).hexdigest()
        self.assertEqual(manifest["core_sha256"], digest)
        self.assertEqual(manifest["qualification_registry_sha256"],
                         hashlib.sha256(self.registry.read_bytes()).hexdigest())
        self.assertEqual(manifest["qualification_registry_adopted_records"], 0)
        self.assertEqual(manifest["support_calibration_sha256"],
                         hashlib.sha256(self.support.read_bytes()).hexdigest())
        commands = self.commands()
        self.assert_build_targets(commands, ["aarch64-apple-ios", "aarch64-apple-ios-sim"])
        generator = next(args for name, *args in commands if name == "cargo" and args[0] == "run")
        self.assertIn("--no-default-features", generator)
        self.assertEqual(generator[generator.index("--library") + 1], str(self.core / "target/aarch64-apple-ios-sim/release/libpennel_core.dylib"))
        self.assertEqual(commands[-1], ["node", str(self.root / "apps/mobile/plugins/native-sources.cjs")])
        self.assertFalse((self.root / "apps/ios").exists())
        self.assertFalse((self.root / "apps/macos").exists())

    def test_explicit_zero_retains_device_and_simulator_preparation(self):
        result = self.prepare("0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_build_targets(self.commands(), ["aarch64-apple-ios", "aarch64-apple-ios-sim"])

    def test_registry_provenance_changes_without_redefining_core_or_binding_hashes(self):
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        before = json.loads((self.artifacts / "manifest.json").read_text())
        before_commands = len(self.commands())
        self.registry.write_text('{"schema_version":1,"adopted_records":[{"fixture":"adopted"}]}\n')
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        after = json.loads((self.artifacts / "manifest.json").read_text())
        self.assertEqual(after["core_sha256"], before["core_sha256"])
        self.assertEqual(after["bindings_sha256"], before["bindings_sha256"])
        self.assertNotEqual(after["qualification_registry_sha256"], before["qualification_registry_sha256"])
        self.assertEqual(after["qualification_registry_adopted_records"], 1)
        self.assert_build_targets(self.commands()[before_commands:], ["aarch64-apple-ios", "aarch64-apple-ios-sim"])

    def test_missing_or_invalid_registry_never_emits_a_generation_manifest(self):
        for body in (None, '{"adopted_records":"not a list"}', '[]'):
            with self.subTest(body=body):
                if body is None:
                    self.registry.unlink()
                else:
                    self.registry.write_text(body)
                result = self.prepare()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.artifacts / "manifest.json").exists())
                self.assertEqual(self.commands(), [])

    def test_support_asset_provenance_changes_without_redefining_core_or_binding_hashes(self):
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        before = json.loads((self.artifacts / "manifest.json").read_text())
        self.support.write_text('{"schema_version":1,"runtime_policy":"updated-fixture"}\n')
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        after = json.loads((self.artifacts / "manifest.json").read_text())
        self.assertEqual(after["core_sha256"], before["core_sha256"])
        self.assertEqual(after["bindings_sha256"], before["bindings_sha256"])
        self.assertEqual(after["qualification_registry_sha256"], before["qualification_registry_sha256"])
        self.assertNotEqual(after["support_calibration_sha256"], before["support_calibration_sha256"])
        self.assertEqual(after["support_calibration_sha256"], hashlib.sha256(self.support.read_bytes()).hexdigest())

    def test_missing_or_invalid_support_asset_fails_before_native_commands(self):
        for body in (None, '[]', '{}', '{"schema_version":2,"runtime_policy":"fixture"}'):
            with self.subTest(body=body):
                if body is None:
                    self.support.unlink()
                else:
                    self.support.write_text(body)
                result = self.prepare()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.artifacts / "manifest.json").exists())
                self.assertEqual(self.commands(), [])

    def test_missing_support_bytes_in_compiled_library_fails_before_bindings(self):
        for target in ('aarch64-apple-ios', 'aarch64-apple-ios-sim'):
            with self.subTest(target=target):
                self.env['PENNEL_BUILD_TEST_MISSING_SUPPORT_EMBEDDING'] = target
                result = self.prepare()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('support calibration asset is missing from', result.stderr.lower())
                self.assertIn(target + '/release/libpennel_core.a', result.stderr)
                self.assertFalse((self.artifacts / "manifest.json").exists())
                self.assertFalse(any(name == 'cargo' and args[0] == 'run' for name, *args in self.commands()))

    def test_registry_changes_during_preparation_never_label_old_outputs_with_new_registry(self):
        pinned = self.registry.read_bytes()
        pinned_hash = hashlib.sha256(pinned).hexdigest()
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        for phase in ('keyboard-aarch64-apple-ios', 'core-aarch64-apple-ios',
                      'core-aarch64-apple-ios-sim', 'bindings', 'framework', 'manifest', 'native-sources'):
            with self.subTest(phase=phase):
                self.registry.write_bytes(pinned)
                self.env['PENNEL_BUILD_TEST_REGISTRY_MUTATION'] = phase
                result = self.prepare()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('qualification registry changed', result.stderr.lower())
                manifest_file = self.artifacts / 'manifest.json'
                if manifest_file.exists():
                    manifest = json.loads(manifest_file.read_text())
                    self.assertEqual(manifest['qualification_registry_sha256'], pinned_hash)
                    self.assertEqual(manifest['qualification_registry_adopted_records'], 0)
                    self.assertNotEqual(manifest['qualification_registry_sha256'],
                                        hashlib.sha256(self.registry.read_bytes()).hexdigest())

    def test_support_changes_during_preparation_never_mislabel_existing_outputs(self):
        pinned = self.support.read_bytes()
        pinned_hash = hashlib.sha256(pinned).hexdigest()
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        for phase in ('keyboard-aarch64-apple-ios', 'core-aarch64-apple-ios',
                      'core-aarch64-apple-ios-sim', 'bindings', 'framework', 'manifest', 'native-sources'):
            with self.subTest(phase=phase):
                self.support.write_bytes(pinned)
                self.env['PENNEL_BUILD_TEST_SUPPORT_MUTATION'] = phase
                result = self.prepare()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('support calibration asset changed', result.stderr.lower())
                manifest_file = self.artifacts / 'manifest.json'
                if manifest_file.exists():
                    manifest = json.loads(manifest_file.read_text())
                    self.assertEqual(manifest['support_calibration_sha256'], pinned_hash)
                    self.assertNotEqual(manifest['support_calibration_sha256'],
                                        hashlib.sha256(self.support.read_bytes()).hexdigest())

    def test_archive_preparation_builds_only_device_libraries_and_matching_bindings(self):
        result = self.prepare("1")
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.commands()
        self.assert_build_targets(commands, ["aarch64-apple-ios"])
        generator = next(args for name, *args in commands if name == "cargo" and args[0] == "run")
        self.assertEqual(generator[generator.index("--library") + 1], str(self.core / "target/aarch64-apple-ios/release/libpennel_core.dylib"))
        self.assertNotIn("aarch64-apple-ios-sim", json.dumps(commands))
        manifest = json.loads((self.artifacts / "manifest.json").read_text())
        self.assertEqual(manifest["targets"], ["aarch64-apple-ios"])
        self.assertEqual(manifest["features"], [])
        self.assertFalse((self.core / "target/aarch64-apple-ios-sim").exists())
        self.assertFalse((self.keyboard / "target/aarch64-apple-ios-sim").exists())

    def test_invalid_archive_mode_fails_before_native_commands(self):
        for script in (SCRIPT.name, "build-keyboard-core-ios.sh"):
            for value in ("yes", "2", ""):
                with self.subTest(script=script, value=value):
                    result = self.prepare(value, script)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("PENNEL_IOS_ARCHIVE_ONLY", result.stderr)
                    self.assertEqual(self.commands(), [])
                    self.assertFalse(self.artifacts.exists())


if __name__ == "__main__":
    unittest.main()
