"""Verify location-independent Expo lock checksums against CocoaPods' real loader."""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


HELPER = Path(__file__).resolve().parents[2] / "apps/mobile/plugins/pod_helpers.rb"
MARKER = "/__pennel_mobile_workspace__"
RUBY_CHECK = "gem 'cocoapods-core', '= 1.16.2'; require 'cocoapods-core'"
RUBY_RUN = RUBY_CHECK + r'''
require 'json'
helper, spec_path, memoize = ARGV
load helper
load helper
spec = Pod::Specification.from_file(Pathname.new(spec_path))
before = Marshal.dump(spec.attributes_hash)
first = spec.checksum
File.write(spec_path, File.read(spec_path) + "\n") if memoize == 'yes'
second = spec.checksum
undefined = Pod::Specification.new { |value| value.name = spec.name }
puts JSON.generate({
  checksum: first,
  second: second,
  encoding: first.encoding.name,
  subspec: spec.subspecs.first&.checksum,
  attributes_unchanged: before == Marshal.dump(spec.attributes_hash),
  undefined: undefined.checksum,
  overrides: Pod::Specification.ancestors.count { |ancestor| ancestor.name == 'PennelPortablePodspecChecksums' }
})
'''


class MobilePodChecksumTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ruby = shutil.which("ruby")
        if not cls.ruby:
            raise unittest.SkipTest("Ruby is unavailable; CocoaPods runs on macOS builders")
        result = subprocess.run([cls.ruby, "-e", RUBY_CHECK], capture_output=True, text=True)
        if result.returncode:
            raise unittest.SkipTest("CocoaPodsCore 1.16.2 is unavailable in this Ruby environment")

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def workspace(self, name):
        workspace = self.root / name / "apps/mobile"
        (workspace / "plugins").mkdir(parents=True)
        shutil.copy2(HELPER, workspace / "plugins/pod_helpers.rb")
        return workspace

    def specification(self, workspace, name, prefix=None, command="copy"):
        prefix = str(workspace) if prefix is None else prefix
        value = {
            "name": name,
            "version": "58.0.23",
            "summary": "Précompiled Expo module",
            "source": {"http": f"file://{prefix}/node_modules/expo-module/output.tar.gz"},
            "prepare_command": f'TARBALL="{prefix}/node_modules/expo-module/output.tar.gz"\n{command} "$TARBALL"',
            "script_phases": [{
                "name": "Copy bundled resources",
                "script": f'{command} "{prefix}/node_modules/expo-module/resource.bundle"',
                "input_files": [f"{prefix}/node_modules/expo-module/resource.bundle"],
            }],
            "subspecs": [{"name": "Internal", "source_files": "Internal.swift"}],
        }
        path = workspace / "ios/Pods/Local Podspecs" / f"{name}.podspec.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
        return path

    def checksum(self, workspace, path, memoize=False):
        result = subprocess.run([
            self.ruby, "-e", RUBY_RUN, str(workspace / "plugins/pod_helpers.rb"),
            str(path), "yes" if memoize else "no",
        ], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_both_evaluated_specs_have_identical_checksums_in_distinct_checkouts(self):
        left, right = self.workspace("local"), self.workspace("runner")
        for name in ("ExpoModulesCore", "ExpoWidgets"):
            with self.subTest(pod=name):
                left_path = self.specification(left, name)
                right_path = self.specification(right, name)
                left_result = self.checksum(left, left_path)
                right_result = self.checksum(right, right_path)
                self.assertEqual(left_result["checksum"], right_result["checksum"])
                expected = left_path.read_bytes().replace(str(left).encode(), MARKER.encode())
                self.assertEqual(left_result["checksum"], hashlib.sha1(expected).hexdigest())

    def test_real_script_changes_still_change_target_spec_checksums(self):
        left, right = self.workspace("before"), self.workspace("after")
        for name in ("ExpoModulesCore", "ExpoWidgets"):
            with self.subTest(pod=name):
                left_path = self.specification(left, name, command="copy")
                right_path = self.specification(right, name, command="changed-copy")
                self.assertNotEqual(self.checksum(left, left_path)["checksum"],
                                    self.checksum(right, right_path)["checksum"])

    def test_unrelated_pods_keep_cocoapods_exact_saved_file_checksum(self):
        left, right = self.workspace("local"), self.workspace("runner")
        sums = []
        for workspace in (left, right):
            path = self.specification(workspace, "AnotherPod")
            result = self.checksum(workspace, path)
            self.assertEqual(result["checksum"], hashlib.sha1(path.read_bytes()).hexdigest())
            sums.append(result["checksum"])
        self.assertNotEqual(*sums)

    def test_sibling_and_nested_nonprefix_paths_are_not_normalized(self):
        workspace = self.workspace("local")
        for prefix in (str(workspace) + "-backup", str(workspace) + " backup",
                       "/another" + str(workspace), "/another@" + str(workspace)):
            with self.subTest(prefix=prefix):
                path = self.specification(workspace, "ExpoWidgets", prefix=prefix)
                result = self.checksum(workspace, path)
                self.assertEqual(result["checksum"], hashlib.sha1(path.read_bytes()).hexdigest())

    def test_checksum_preserves_spec_bytes_attributes_root_and_repeat_load_behavior(self):
        workspace = self.workspace("local")
        path = self.specification(workspace, "ExpoWidgets")
        original = path.read_bytes()
        result = self.checksum(workspace, path)
        self.assertEqual(path.read_bytes(), original)
        self.assertTrue(result["attributes_unchanged"])
        self.assertEqual(result["checksum"], result["subspec"])
        self.assertEqual(result["encoding"], "UTF-8")
        self.assertIsNone(result["undefined"])
        self.assertEqual(result["overrides"], 1)
        result = self.checksum(workspace, path, memoize=True)
        self.assertEqual(result["checksum"], result["second"])

    def test_unevaluated_ruby_podspec_checksum_remains_unchanged(self):
        workspace = self.workspace("local")
        path = workspace / "ExpoWidgets.podspec"
        path.write_text("Pod::Spec.new do |spec|\n"
                        "  spec.name = 'ExpoWidgets'\n"
                        "  spec.version = '58.0.23'\n"
                        f"  spec.summary = '{workspace}/node_modules/resource'\n"
                        "end\n")
        result = self.checksum(workspace, path)
        self.assertEqual(result["checksum"], hashlib.sha1(path.read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
