"""Release-native cache and Cloud identity regressions without a native build."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "mobile-native-cache.py"
spec = importlib.util.spec_from_file_location("mobile_native_cache", SCRIPT)
cache = importlib.util.module_from_spec(spec)


class NativeCacheTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec.loader.exec_module(cache)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve() / "checkout"
        self.root.mkdir()
        for relative, content in {
            "core/Cargo.toml": "[package]\nname='fixture'\n",
            "core/Cargo.lock": '[[package]]\nname = "uniffi"\nversion = "0.32.2"\n',
            "core/src/lib.rs": "pub fn fixture() {}\n",
            "core/keyboard/include/PennelTypingFFI.h": "void fixture(void);\n",
            "core/keyboard/include/module.modulemap": "module Fixture {}\n",
            "scripts/build-mobile-core-ios.sh": "fixture prep\n",
            "scripts/build-keyboard-core-ios.sh": "fixture typing prep\n",
            "scripts/with-toolchain.sh": "fixture toolchain\n",
            "scripts/mobile-native-cache.py": "fixture cache\n",
            ".cargo/config.toml": "[build]\n",
            "apps/mobile/app/index.tsx": "fixture UI\n",
        }.items():
            file = self.root / relative
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(content)
        self.toolchain = {"rustc": "rustc fixture", "cargo": "cargo fixture", "xcode": "Xcode fixture",
                          "iphoneos_sdk": "27.0", "iphoneos_sdk_build": "fixture"}
        self.context = cache.fingerprint(self.root, self.toolchain, {})
        self.artifacts = self.root / "apps/mobile/artifacts/ios"
        self.artifacts.mkdir(parents=True)
        for framework, library, header in (
            ("PennelCoreFFI", "libpennel_core.a", "pennel_coreFFI.h"),
            ("PennelTypingFFI", "libpennel_typing.a", "PennelTypingFFI.h"),
        ):
            directory = self.artifacts / (framework + ".xcframework")
            (directory / "ios-arm64/Headers").mkdir(parents=True)
            (directory / "ios-arm64" / library).write_bytes(b"!<arch>\nfixture object\n")
            (directory / "ios-arm64/Headers" / header).write_text("fixture header\n")
            (directory / "ios-arm64/Headers/module.modulemap").write_text("module Fixture {}\n")
            (directory / "Info.plist").write_bytes(plistlib.dumps({"AvailableLibraries": [{
                "LibraryIdentifier": "ios-arm64", "LibraryPath": library, "HeadersPath": "Headers",
                "SupportedPlatform": "ios", "SupportedArchitectures": ["arm64"],
            }]}))
        (self.artifacts / "PennelCore").mkdir()
        bindings = self.artifacts / "PennelCore/pennel_core.swift"
        bindings.write_text("fixture bindings\n")
        (self.artifacts / "manifest.json").write_text(json.dumps({
            "schema": 1, "revision": "a" * 40, "features": [], "targets": ["aarch64-apple-ios"],
            "bindings_sha256": hashlib.sha256(bindings.read_bytes()).hexdigest(),
        }))
        self.cache_root = self.root.parent / "derived-data/pennel-native"
        self.inspect = lambda file: ["arm64"]

    def store(self):
        return cache.store(self.root, self.cache_root, self.context, "a" * 40, self.inspect)

    def restore(self):
        return cache.restore(self.root, self.cache_root, self.context, self.inspect)

    def test_roundtrip_preserves_complete_bundle_and_older_artifacts(self):
        stored = self.store()
        self.assertTrue(stored["stored"])
        original = (self.artifacts / "PennelCore/pennel_core.swift").read_bytes()
        (self.artifacts / "old-evidence").write_text("keep this release evidence")
        restored = self.restore()
        self.assertTrue(restored["hit"])
        self.assertEqual((self.artifacts / "PennelCore/pennel_core.swift").read_bytes(), original)
        backups = list(self.artifacts.parent.glob("ios.retained-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual((backups[0] / "old-evidence").read_text(), "keep this release evidence")
        manifest = json.loads((self.cache_root / self.context["key"] / "cache-manifest.json").read_text())
        self.assertIn("PennelCore/pennel_core.swift", manifest["files"])
        self.assertIn("retain", manifest["retention"].lower())

    def test_missing_or_corrupt_cache_never_replaces_current_artifacts(self):
        self.assertFalse(self.restore()["hit"])
        self.store()
        file = self.cache_root / self.context["key"] / "artifacts/PennelCore/pennel_core.swift"
        file.write_text("tampered")
        before = (self.artifacts / "PennelCore/pennel_core.swift").read_bytes()
        self.assertFalse(self.restore()["hit"])
        self.assertEqual((self.artifacts / "PennelCore/pennel_core.swift").read_bytes(), before)

    def test_corrupt_immutable_entry_is_retained_when_replaced(self):
        self.store()
        entry = self.cache_root / self.context["key"]
        (entry / "cache-manifest.json").write_text("broken")
        result = self.store()
        self.assertTrue(result["stored"])
        retained = list(self.cache_root.glob(self.context["key"] + ".retained-*"))
        self.assertEqual(len(retained), 1)
        self.assertEqual((retained[0] / "cache-manifest.json").read_text(), "broken")

    def test_native_inputs_headers_flags_and_toolchain_change_keys_but_js_does_not(self):
        original = self.context["key"]
        (self.root / "apps/mobile/app/index.tsx").write_text("changed UI")
        self.assertEqual(cache.fingerprint(self.root, self.toolchain, {})["key"], original)
        header = self.root / "core/keyboard/include/PennelTypingFFI.h"
        header.write_text("changed ABI")
        self.assertNotEqual(cache.fingerprint(self.root, self.toolchain, {})["key"], original)
        self.assertNotEqual(cache.fingerprint(self.root, {**self.toolchain, "rustc": "other"}, {})["key"],
                            cache.fingerprint(self.root, self.toolchain, {})["key"])
        self.assertNotEqual(cache.fingerprint(self.root, self.toolchain, {"RUSTFLAGS": "-C opt-level=2"})["key"],
                            cache.fingerprint(self.root, self.toolchain, {})["key"])

    def test_intermediates_are_excluded_and_native_symlinks_are_refused(self):
        target = self.root / "core/target/temporary"
        target.parent.mkdir()
        target.write_text("compiler cache")
        self.assertEqual(cache.fingerprint(self.root, self.toolchain, {})["key"], self.context["key"])
        (self.root / "core/src/link.rs").symlink_to(self.root / "apps/mobile/app/index.tsx")
        with self.assertRaisesRegex(ValueError, "symbolic"):
            cache.fingerprint(self.root, self.toolchain, {})

    def test_cache_manifest_paths_and_unlisted_files_cannot_escape_validation(self):
        self.store()
        manifest_file = self.cache_root / self.context["key"] / "cache-manifest.json"
        manifest = json.loads(manifest_file.read_text())
        manifest["files"]["../../outside"] = {"sha256": "0" * 64, "bytes": 1}
        manifest_file.write_text(json.dumps(manifest))
        self.assertFalse(self.restore()["hit"])
        self.assertFalse((self.root.parent / "outside").exists())

    def test_key_mismatch_and_symlinked_cache_entries_are_misses(self):
        self.store()
        manifest_file = self.cache_root / self.context["key"] / "cache-manifest.json"
        manifest = json.loads(manifest_file.read_text())
        manifest["key"] = "0" * 64
        manifest_file.write_text(json.dumps(manifest))
        self.assertFalse(self.restore()["hit"])
        other = self.cache_root / "other"
        (self.cache_root / self.context["key"]).rename(other)
        (self.cache_root / self.context["key"]).symlink_to(other, target_is_directory=True)
        self.assertFalse(self.restore()["hit"])
        with self.assertRaisesRegex(ValueError, "symbolic"):
            self.store()

    def test_simulator_mixed_architecture_thin_archives_and_wrong_bindings_are_rejected(self):
        framework = self.artifacts / "PennelCoreFFI.xcframework/Info.plist"
        original = framework.read_bytes()
        data = plistlib.loads(original)
        data["AvailableLibraries"][0]["SupportedPlatformVariant"] = "simulator"
        framework.write_bytes(plistlib.dumps(data))
        with self.assertRaisesRegex(ValueError, "device"):
            self.store()
        framework.write_bytes(original)
        with self.assertRaisesRegex(ValueError, "arm64"):
            cache.store(self.root, self.cache_root, self.context, "a" * 40, lambda file: ["x86_64"])
        library = self.artifacts / "PennelCoreFFI.xcframework/ios-arm64/libpennel_core.a"
        library.write_bytes(b"!<thin>\nexternal objects")
        with self.assertRaisesRegex(ValueError, "archive"):
            self.store()
        library.write_bytes(b"!<arch>\nfixture object")
        (self.artifacts / "PennelCore/pennel_core.swift").write_text("other bindings")
        with self.assertRaisesRegex(ValueError, "bindings"):
            self.store()

    def test_incomplete_debug_symbol_reference_is_rejected(self):
        framework = self.artifacts / "PennelCoreFFI.xcframework/Info.plist"
        data = plistlib.loads(framework.read_bytes())
        data["AvailableLibraries"][0]["DebugSymbolsPath"] = "dSYMs"
        framework.write_bytes(plistlib.dumps(data))
        with self.assertRaisesRegex(ValueError, "symbol"):
            self.store()


class CloudIdentityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec.loader.exec_module(cache)

    def test_expected_target_settings_reject_production_team_and_manual_signing(self):
        objects = {"PBXNativeTarget": {}, "XCConfigurationList": {}, "XCBuildConfiguration": {}}
        for index, (target, bundle) in enumerate(cache.EXPECTED_TARGETS.items()):
            identifier = str(index)
            objects["PBXNativeTarget"][identifier] = {"name": target, "buildConfigurationList": "list" + identifier}
            objects["XCConfigurationList"]["list" + identifier] = {"buildConfigurations": [{"value": "config" + identifier}]}
            objects["XCBuildConfiguration"]["config" + identifier] = {"name": "Release", "buildSettings": {
                "PRODUCT_BUNDLE_IDENTIFIER": bundle, "DEVELOPMENT_TEAM": cache.APPLE_TEAM,
                "CODE_SIGN_STYLE": "Automatic", "CURRENT_PROJECT_VERSION": "1",
            }}
        cache.validate_project(objects)
        settings = objects["XCBuildConfiguration"]["config0"]["buildSettings"]
        for name, wrong in (("PRODUCT_BUNDLE_IDENTIFIER", "app.pennel.ios"),
                            ("DEVELOPMENT_TEAM", "OTHERTEAM"), ("CODE_SIGN_STYLE", "Manual")):
            with self.subTest(name=name):
                original = settings[name]
                settings[name] = wrong
                with self.assertRaises(ValueError):
                    cache.validate_project(objects)
                settings[name] = original

    def test_cloud_environment_rejects_nonarchive_other_team_and_other_branch(self):
        env = {"CI_XCODE_CLOUD": "TRUE", "CI_TEAM_ID": cache.APP_STORE_TEAM,
               "CI_BUNDLE_ID": cache.BUNDLE_ID, "CI_XCODE_SCHEME": "Pennel",
               "CI_XCODEBUILD_ACTION": "archive", "CI_BRANCH": "codex/ios-testflight",
               "PENNEL_APPLE_DECLARATIONS_CONFIRMED": "1"}
        cache.validate_cloud_environment(env)
        for name, wrong in (("CI_TEAM_ID", "OTHER"), ("CI_TEAM_ID", cache.APPLE_TEAM), ("CI_BUNDLE_ID", "app.pennel.ios"),
                            ("CI_XCODEBUILD_ACTION", "build"), ("CI_BRANCH", "main")):
            with self.subTest(name=name), self.assertRaises(ValueError):
                cache.validate_cloud_environment({**env, name: wrong})
        for value in (None, "", "0", "true"):
            without_confirmation = {key: current for key, current in env.items()
                                    if key != "PENNEL_APPLE_DECLARATIONS_CONFIRMED"}
            if value is not None:
                without_confirmation["PENNEL_APPLE_DECLARATIONS_CONFIRMED"] = value
            with self.subTest(confirmation=value), self.assertRaisesRegex(ValueError, "DECLARATIONS_CONFIRMED"):
                cache.validate_cloud_environment(without_confirmation)

    def test_snapshot_requires_exact_verified_source_and_unmodified_source_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "source.txt"
            source.write_text("verified source")
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "add", "source.txt"], check=True)
            env = {**os.environ, "GIT_AUTHOR_NAME": "Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
                   "GIT_COMMITTER_NAME": "Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid"}
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "verified source"], env=env, check=True)
            sha = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
            tree = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], text=True).strip()
            generated = root / "apps/mobile/ios/fixture.txt"
            generated.parent.mkdir(parents=True)
            generated.write_text("generated")
            manifest = {"schema": 1, "source_sha": sha, "source_tree_sha": tree, "variant": "preview",
                        "apple_team_id": cache.APPLE_TEAM, "bundle_identifier": cache.BUNDLE_ID,
                        "validation": {"schema": 1, "source_sha": sha, "status": "success",
                                       "checks": [{"name": "mobile", "status": "success"}]},
                        "inputs": {"source.txt": cache.digest(source)},
                        "generated_files": {"apps/mobile/ios/fixture.txt": cache.digest(generated)}}
            cache.write_json(root / cache.SNAPSHOT, manifest)
            self.assertEqual(cache.validate_snapshot(root)["source_sha"], sha)
            manifest["validation"]["source_sha"] = "b" * 40
            cache.write_json(root / cache.SNAPSHOT, manifest)
            with self.assertRaisesRegex(ValueError, "validation receipt"):
                cache.validate_snapshot(root)
            manifest["validation"]["source_sha"] = sha
            cache.write_json(root / cache.SNAPSHOT, manifest)
            source.write_text("unverified source")
            with self.assertRaisesRegex(ValueError, "source files"):
                cache.validate_snapshot(root)
            source.write_text("verified source")
            generated.write_text("tampered generated")
            with self.assertRaisesRegex(ValueError, "file identity"):
                cache.validate_snapshot(root)

    def test_untrusted_cloud_environment_stops_hooks_before_installing_tools(self):
        for script in ("xcode-cloud-post-clone.sh", "xcode-cloud-pre-xcodebuild.sh"):
            with self.subTest(script=script):
                env = {**os.environ, "CI_PRIMARY_REPOSITORY_PATH": str(SCRIPT.parents[1]),
                       "CI_XCODE_CLOUD": "TRUE", "CI_TEAM_ID": "OTHER"}
                result = subprocess.run(["bash", str(SCRIPT.with_name(script))], env=env,
                                        text=True, capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("CI_TEAM_ID", result.stderr)
                self.assertNotIn("rustup", result.stdout)

    def test_build_preparation_aligns_all_versions_and_rejects_namespace_changes_before_writing(self):
        xcode = SCRIPT.parents[1] / "apps/mobile/node_modules/xcode"
        if not xcode.is_dir():
            self.skipTest("The mobile workspace dependencies are required for project-parser integration")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "source.txt").write_text("verified source")
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "add", "source.txt"], check=True)
            env = {**os.environ, "GIT_AUTHOR_NAME": "Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
                   "GIT_COMMITTER_NAME": "Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid"}
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "verified source"], env=env, check=True)
            sha = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
            tree = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], text=True).strip()
            ios = root / "apps/mobile/ios"
            project = ios / "Pennel.xcodeproj"
            (project / "xcshareddata/xcschemes").mkdir(parents=True)
            sections = {"PBXNativeTarget": [], "XCConfigurationList": [], "XCBuildConfiguration": []}
            for index, (name, bundle) in enumerate(cache.EXPECTED_TARGETS.items()):
                target, configurations, configuration = [f"A{index + offset:023X}" for offset in (1, 10, 20)]
                sections["PBXNativeTarget"].append(f'{target} /* {name} */ = {{ isa = PBXNativeTarget; name = {name}; buildConfigurationList = {configurations}; }};')
                sections["XCConfigurationList"].append(f'{configurations} = {{ isa = XCConfigurationList; buildConfigurations = ({configuration},); }};')
                sections["XCBuildConfiguration"].append(f'{configuration} = {{ isa = XCBuildConfiguration; name = Release; buildSettings = {{ PRODUCT_BUNDLE_IDENTIFIER = {bundle}; DEVELOPMENT_TEAM = {cache.APPLE_TEAM}; CODE_SIGN_STYLE = Automatic; CURRENT_PROJECT_VERSION = 1; MARKETING_VERSION = 0.1.0; }}; }};')
                target_directory = ios / name
                target_directory.mkdir()
                (target_directory / "Info.plist").write_bytes(plistlib.dumps({
                    "CFBundleIdentifier": "$(PRODUCT_BUNDLE_IDENTIFIER)", "CFBundleVersion": "1",
                    "CFBundleShortVersionString": "0.1.0", "PennelAppGroupIdentifier": cache.APP_GROUP,
                    "PennelKeychainService": cache.BUNDLE_ID,
                    "PennelHandoffNamespace": cache.BUNDLE_ID + ".handoff",
                }))
                (target_directory / (name + ".entitlements")).write_bytes(plistlib.dumps({
                    "com.apple.security.application-groups": [cache.APP_GROUP],
                }))
            (project / "project.pbxproj").write_text("// !$*UTF8*$!\n{ archiveVersion = 1; classes = {}; objectVersion = 54; objects = {\n"
                                                   + "\n".join(f'/* Begin {name} section */\n' + "\n".join(entries)
                                                               + f'\n/* End {name} section */' for name, entries in sections.items())
                                                   + "\n}; rootObject = 000000000000000000000001; }\n")
            (project / "xcshareddata/xcschemes/Pennel.xcscheme").write_text(
                '<Scheme><BuildAction><BuildActionEntries><BuildActionEntry buildForArchiving="YES">'
                '<BuildableReference BlueprintName="Pennel"/></BuildActionEntry></BuildActionEntries></BuildAction>'
                '<ArchiveAction buildConfiguration="Release"/></Scheme>')
            workspace = ios / "Pennel.xcworkspace"
            workspace.mkdir()
            (workspace / "contents.xcworkspacedata").write_text(
                '<Workspace><FileRef location="group:Pennel.xcodeproj"/><FileRef location="group:Pods/Pods.xcodeproj"/></Workspace>')
            modules = root / "apps/mobile/node_modules"
            modules.mkdir()
            (modules / "xcode").symlink_to(xcode, target_is_directory=True)
            (root / "apps/mobile/Podfile.lock").write_text("COCOAPODS: 1.16.2\n")
            (ios / "Podfile.lock").write_text("COCOAPODS: 1.16.2\n")
            cache.write_json(root / cache.SNAPSHOT, {
                "schema": 1, "source_sha": sha, "source_tree_sha": tree, "variant": "preview",
                "apple_team_id": cache.APPLE_TEAM, "bundle_identifier": cache.BUNDLE_ID,
                "validation": {"schema": 1, "source_sha": sha, "status": "success",
                               "checks": [{"name": "mobile", "status": "success"}]},
                "inputs": {"source.txt": cache.digest(root / "source.txt")},
                "generated_files": {file.relative_to(root).as_posix(): cache.digest(file) for file in ios.rglob("*") if file.is_file()},
            })
            cache.attest_snapshot(root)
            app_info = ios / "Pennel/Info.plist"
            original_app_info = app_info.read_bytes()
            unverified_info = plistlib.loads(original_app_info)
            unverified_info["CFBundleShortVersionString"] = "9.9.9"
            app_info.write_bytes(plistlib.dumps(unverified_info))
            with self.assertRaisesRegex(ValueError, "file identity"):
                cache.validate_snapshot(root, pristine=False)
            app_info.write_bytes(original_app_info)
            with self.assertRaisesRegex(ValueError, "post-pod seal"):
                cache.prepare_build(root, "37")
            pods = ios / "Pods"
            (pods / "Pods.xcodeproj").mkdir(parents=True)
            (pods / "Pods.xcodeproj/project.pbxproj").write_text("installed Pods project")
            (pods / "Manifest.lock").write_text("COCOAPODS: 1.16.2\n")
            support = pods / "Target Support Files/Pods-Pennel"
            support.mkdir(parents=True)
            (support / "Pods-Pennel.release.xcconfig").write_text("validated CocoaPods integration")
            cache.seal_pods(root)
            self.assertEqual(cache.prepare_build(root, "37")["build_number"], "37")
            for name in cache.EXPECTED_TARGETS:
                info = plistlib.loads((ios / name / "Info.plist").read_bytes())
                self.assertEqual(info["CFBundleVersion"], "37")
                self.assertEqual(info["CFBundleShortVersionString"], "0.1.0")
            cache.prepare_build(root, "37")
            unchanged = (project / "project.pbxproj").read_bytes()
            for name, key, value in (("Pennel", "CFBundleShortVersionString", "9.9.9"),
                                     ("PennelKeyboard", "PennelHandoffNamespace", "app.pennel.ios.handoff")):
                with self.subTest(mutation=key):
                    file = ios / name / "Info.plist"
                    original = file.read_bytes()
                    info = plistlib.loads(original)
                    info[key] = value
                    file.write_bytes(plistlib.dumps(info))
                    with self.assertRaisesRegex(ValueError, "sealed generated"):
                        cache.prepare_build(root, "37")
                    self.assertEqual((project / "project.pbxproj").read_bytes(), unchanged)
                    self.assertEqual(plistlib.loads(file.read_bytes())[key], value)
                    file.write_bytes(original)
            (project / "project.pbxproj").write_bytes(unchanged.replace(b"MARKETING_VERSION = 0.1.0;",
                                                                       b"SWIFT_ACTIVE_COMPILATION_CONDITIONS = UNREVIEWED; MARKETING_VERSION = 0.1.0;"))
            with self.assertRaisesRegex(ValueError, "sealed generated"):
                cache.prepare_build(root, "37")
            (project / "project.pbxproj").write_bytes(unchanged)
            integration = support / "Pods-Pennel.release.xcconfig"
            original_integration = integration.read_bytes()
            integration.write_text("unverified compilation conditions")
            with self.assertRaisesRegex(ValueError, "sealed Pod"):
                cache.prepare_build(root, "37")
            integration.write_bytes(original_integration)
            with self.assertRaisesRegex(ValueError, "already prepared"):
                cache.prepare_build(root, "38")
            keyboard = ios / "PennelKeyboard/Info.plist"
            info = plistlib.loads(keyboard.read_bytes())
            info["PennelKeychainService"] = "app.pennel.ios"
            keyboard.write_bytes(plistlib.dumps(info))
            with self.assertRaisesRegex(ValueError, "sealed generated"):
                cache.prepare_build(root, "37")
            self.assertEqual((project / "project.pbxproj").read_bytes(), unchanged)


if __name__ == "__main__":
    unittest.main()
