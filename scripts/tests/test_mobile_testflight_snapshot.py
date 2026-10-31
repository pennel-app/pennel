"""Verify generated TestFlight snapshots without Expo, Xcode or network access."""

import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET


SCRIPT = Path(__file__).resolve().parents[1] / "mobile-testflight-snapshot.py"
TEAM = "B538Q2K76C"
GROUP = "group.app.pennel.preview"
APP = "app.pennel.ios.preview"


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.ios = self.root / "apps/mobile/ios"
        self.write(".gitignore", "apps/mobile/ios/\nbuild/\n")
        for relative in ["apps/mobile/package.json", "apps/mobile/package-lock.json",
                         "apps/mobile/app.config.ts", "apps/mobile/plugins/with-pennel.cjs",
                         "apps/mobile/plugins/pod_helpers.rb", "scripts/mobile-testflight-snapshot.py",
                         "scripts/xcode-cloud-post-clone.sh", "scripts/xcode-cloud-pre-xcodebuild.sh", "scripts/xcode-cloud-post-xcodebuild.sh"]:
            self.write(relative, "fixture source\n")
        self.write("apps/mobile/Podfile.lock", "PODS:\n  - Expo (58.0.19)\nCOCOAPODS: 1.17.0\n")
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Snapshot fixture")
        self.git("add", ".")
        self.git("commit", "-qm", "Create source fixture")
        self.sha = self.git("rev-parse", "HEAD").strip()
        self.receipt = self.root / "build/validation.json"
        self.validation = {"schema": 1, "source_sha": self.sha, "status": "success", "checks": [
            {"name": "iOS generated build (1)", "status": "success",
             "url": "https://github.com/pennel-app/pennel/actions/runs/123"},
        ]}
        self.write_receipt()
        self.prepare_generated()

    def write(self, relative, contents):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)
        return path

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True)

    def write_receipt(self):
        self.receipt.parent.mkdir(parents=True, exist_ok=True)
        self.receipt.write_text(json.dumps(self.validation))

    def prepare_generated(self):
        self.write("apps/mobile/ios/Pennel/AppDelegate.swift", "import Expo\n")
        self.write("apps/mobile/ios/ExpoWidgetsTarget/index.swift", "import SwiftUI\n")
        self.write("apps/mobile/ios/ExpoWidgetsTarget/ExpoWidgetsLayoutRegistry.config.json", "{}\n")
        for directory, name in [("Pennel", "Pennel"), ("PennelKeyboard", "PennelKeyboard"),
                                ("ExpoWidgetsTarget", "ExpoWidgetsTarget")]:
            base = self.ios / directory
            base.mkdir(parents=True, exist_ok=True)
            (base / "Info.plist").write_bytes(plistlib.dumps({"PennelAppGroupIdentifier": GROUP}))
            (base / f"{name}.entitlements").write_bytes(plistlib.dumps({
                "com.apple.security.application-groups": [GROUP]}))
        self.write("apps/mobile/ios/Pennel.xcodeproj/project.pbxproj", f"""// !$*UTF8*$!
{{
 objects = {{
  13B07F861A680F5B00A75B9A /* Pennel */ = {{
   isa = PBXNativeTarget;
   name = Pennel;
  }};
 }};
 DEVELOPMENT_TEAM = {TEAM};
 CODE_SIGN_STYLE = Automatic;
 PRODUCT_BUNDLE_IDENTIFIER = {APP};
 PRODUCT_BUNDLE_IDENTIFIER = {APP}.keyboard;
 PRODUCT_BUNDLE_IDENTIFIER = {APP}.widgets;
}}
""")
        self.write("apps/mobile/ios/Podfile", "platform :ios, '26.0'\n")
        self.write("apps/mobile/ios/Podfile.properties.json", '{"ios.deploymentTarget":"26.0"}\n')
        self.write("apps/mobile/ios/.xcode.env", 'export NODE_BINARY=$(command -v node)\n')

    def run_snapshot(self, *, success=True, source=None, notes=None):
        command = ["python3", str(SCRIPT), "--root", str(self.root), "--source-sha", source or self.sha,
                   "--validation-receipt", str(self.receipt)]
        if notes is not None:
            note_file = self.write("build/notes.txt", notes)
            command.extend(["--notes-file", str(note_file)])
        result = subprocess.run(command, capture_output=True, text=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout)
        return result

    def manifest(self):
        return json.loads((self.ios / "pennel-testflight.json").read_text())

    def paths(self):
        return json.loads((self.root / "build/testflight/snapshot-paths.json").read_text())

    def test_snapshot_records_exact_source_and_fixed_lock(self):
        self.run_snapshot(notes="Controleer dictatie en het toetsenbord.\n")
        manifest = self.manifest()
        self.assertEqual(manifest["schema"], 1)
        self.assertEqual(manifest["source_sha"], self.sha)
        self.assertEqual(manifest["source_tree_sha"], self.git("rev-parse", f"{self.sha}^{{tree}}").strip())
        self.assertEqual(manifest["variant"], "preview")
        self.assertEqual(manifest["apple_team_id"], TEAM)
        self.assertEqual(manifest["bundle_identifier"], APP)
        self.assertEqual(manifest["validation"], self.validation)
        canonical = self.root / "apps/mobile/Podfile.lock"
        self.assertEqual((self.ios / "Podfile.lock").read_bytes(), canonical.read_bytes())
        self.assertEqual(manifest["inputs"]["apps/mobile/Podfile.lock"], hashlib.sha256(canonical.read_bytes()).hexdigest())
        paths = self.paths()
        self.assertEqual(paths, sorted(paths))
        self.assertIn("apps/mobile/ios/pennel-testflight.json", paths)
        self.assertNotIn("apps/mobile/ios/pennel-testflight.json", manifest["generated_files"])
        self.assertEqual(set(paths), set(manifest["generated_files"]) | {"apps/mobile/ios/pennel-testflight.json"})
        for relative, digest in manifest["generated_files"].items():
            self.assertEqual(hashlib.sha256((self.root / relative).read_bytes()).hexdigest(), digest)
        self.assertEqual((self.ios / "TestFlight/WhatToTest.en-US.txt").read_text(), manifest["notes"])
        self.assertEqual(self.git("diff", "--name-only", self.sha), "")

    def test_repeated_snapshot_is_identical(self):
        self.run_snapshot(notes="Test the keyboard.\n")
        first = (self.ios / "pennel-testflight.json").read_bytes()
        first_paths = self.paths()
        self.run_snapshot(notes="Test the keyboard.\n")
        self.assertEqual((self.ios / "pennel-testflight.json").read_bytes(), first)
        self.assertEqual(self.paths(), first_paths)

    def test_known_local_and_dependency_paths_are_excluded_without_being_read(self):
        secret = "do-not-publish-local-settings"
        self.write("apps/mobile/ios/.xcode.env.local", secret)
        self.write("apps/mobile/ios/.gitignore", secret)
        self.write("apps/mobile/ios/Pennel.xcodeproj/xcuserdata/owner.xcuserdatad/state", secret)
        self.write("apps/mobile/ios/Pennel.xcworkspace/xcshareddata/IDEWorkspaceChecks.plist", secret)
        outside = self.root / "not-source"
        outside.mkdir()
        (outside / "key.p8").write_text(secret)
        (self.ios / "Pods").symlink_to(outside, target_is_directory=True)
        (self.ios / "PennelNative").symlink_to(outside, target_is_directory=True)
        self.run_snapshot()
        staged = self.paths()
        for path in staged:
            self.assertNotIn(secret, (self.root / path).read_bytes().decode(errors="ignore"))
        self.assertTrue((self.ios / ".xcode.env.local").exists())
        self.assertFalse(any("xcuserdata" in path or "/Pods/" in path or "PennelNative" in path for path in staged))

    def test_unknown_files_fail_before_manifest_creation(self):
        self.write("apps/mobile/ios/Pennel/key.p8", "private credential")
        self.run_snapshot(success=False)
        self.assertFalse((self.ios / "pennel-testflight.json").exists())

    def test_allowed_file_symlink_cannot_escape_the_snapshot(self):
        app = self.ios / "Pennel/AppDelegate.swift"
        app.unlink()
        app.symlink_to(self.root / "apps/mobile/package.json")
        self.run_snapshot(success=False)

    def test_asset_directory_symlink_is_rejected(self):
        assets = self.ios / "Pennel/Images.xcassets"
        assets.symlink_to(self.root / "apps/mobile", target_is_directory=True)
        self.run_snapshot(success=False)

    def test_absolute_personal_paths_are_rejected(self):
        self.write("apps/mobile/ios/.xcode.env", "export NODE_BINARY=/Users/fixture/node\n")
        result = self.run_snapshot(success=False)
        self.assertNotIn("/Users/fixture/node", result.stderr)

    def test_missing_lock_is_rejected(self):
        (self.root / "apps/mobile/Podfile.lock").unlink()
        self.run_snapshot(success=False)

    def test_receipt_must_match_source_and_succeed(self):
        for field, value in [("source_sha", "b" * 40), ("status", "pending"), ("checks", [])]:
            with self.subTest(field=field):
                original = self.validation[field]
                self.validation[field] = value
                self.write_receipt()
                self.run_snapshot(success=False)
                self.validation[field] = original

    def test_check_cannot_be_failed_or_reference_another_repository(self):
        self.validation["checks"][0]["status"] = "failure"
        self.write_receipt()
        self.run_snapshot(success=False)
        self.validation["checks"][0]["status"] = "success"
        self.validation["checks"][0]["url"] = "https://example.invalid/private-token"
        self.write_receipt()
        result = self.run_snapshot(success=False)
        self.assertNotIn("private-token", result.stderr)

    def test_receipt_metadata_is_not_copied_into_published_provenance(self):
        self.validation["credential"] = "sensitive-but-irrelevant"
        self.validation["checks"][0]["arguments"] = "sensitive-command-line"
        self.write_receipt()
        self.run_snapshot()
        data = (self.ios / "pennel-testflight.json").read_text()
        self.assertNotIn("sensitive-but-irrelevant", data)
        self.assertNotIn("sensitive-command-line", data)

    def test_only_full_existing_head_commit_is_accepted(self):
        for source in [self.sha[:12], "HEAD", "a" * 40, "../source"]:
            with self.subTest(source=source):
                self.run_snapshot(success=False, source=source)
        self.write("another.txt", "next source\n")
        self.git("add", "another.txt")
        self.git("commit", "-qm", "Advance source")
        self.run_snapshot(success=False, source=self.sha)

    def test_modified_tracked_inputs_are_rejected(self):
        self.write("apps/mobile/app.config.ts", "modified input\n")
        self.run_snapshot(success=False)

    def test_untracked_plugin_cannot_be_attested_as_selected_source(self):
        self.write("apps/mobile/plugins/local-only.cjs", "untracked generation behavior\n")
        self.run_snapshot(success=False)

    def test_workspace_and_wrappers_are_portable_and_shared_archive_is_available(self):
        self.run_snapshot()
        workspace = ET.fromstring((self.ios / "Pennel.xcworkspace/contents.xcworkspacedata").read_text())
        self.assertEqual([item.attrib["location"] for item in workspace.findall("FileRef")],
                         ["group:Pennel.xcodeproj", "group:Pods/Pods.xcodeproj"])
        scheme = ET.fromstring((self.ios / "Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme").read_text())
        self.assertEqual(scheme.find("ArchiveAction").attrib["buildConfiguration"], "Release")
        entry = scheme.find("BuildAction/BuildActionEntries/BuildActionEntry")
        self.assertEqual(entry.attrib["buildForArchiving"], "YES")
        self.assertEqual(entry.find("BuildableReference").attrib["BlueprintIdentifier"], "13B07F861A680F5B00A75B9A")
        for hook, canonical in [("ci_post_clone.sh", "xcode-cloud-post-clone.sh"),
                                ("ci_pre_xcodebuild.sh", "xcode-cloud-pre-xcodebuild.sh"),
                                ("ci_post_xcodebuild.sh", "xcode-cloud-post-xcodebuild.sh")]:
            wrapper = self.ios / "ci_scripts" / hook
            self.assertTrue(os.access(wrapper, os.X_OK))
            content = wrapper.read_text()
            self.assertIn("CI_PRIMARY_REPOSITORY_PATH", content)
            self.assertIn(f'"$root/scripts/{canonical}"', content)
            self.assertNotIn(str(self.root), content)
            subprocess.run(["bash", "-n", str(wrapper)], check=True)

    def test_production_group_or_foreign_team_is_rejected(self):
        entitlement = self.ios / "Pennel/Pennel.entitlements"
        entitlement.write_bytes(plistlib.dumps({"com.apple.security.application-groups": ["group.app.pennel"]}))
        self.run_snapshot(success=False)
        entitlement.write_bytes(plistlib.dumps({"com.apple.security.application-groups": [GROUP]}))
        project = self.ios / "Pennel.xcodeproj/project.pbxproj"
        project.write_text(project.read_text().replace(TEAM, "WRONGTEAM0"))
        self.run_snapshot(success=False)

    def test_notes_cannot_publish_secrets_or_control_characters(self):
        secret = "-----BEGIN PRIVATE KEY-----"
        result = self.run_snapshot(success=False, notes=secret)
        self.assertNotIn(secret, result.stderr)
        self.run_snapshot(success=False, notes="test\x00text")
        self.run_snapshot(success=False, notes="a" * 4001)

    def test_omitting_notes_removes_only_the_previous_generated_note(self):
        self.run_snapshot(notes="Old instructions")
        self.run_snapshot()
        self.assertFalse((self.ios / "TestFlight/WhatToTest.en-US.txt").exists())
        self.assertEqual(self.manifest()["notes"], "")


if __name__ == "__main__":
    unittest.main()
