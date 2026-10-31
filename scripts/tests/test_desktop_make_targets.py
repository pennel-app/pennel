import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]
TEST_ARGS = ["test", "--manifest-path", "core/Cargo.toml"]
KEYBOARD_TEST_ARGS = ["test", "--manifest-path", "core/keyboard/Cargo.toml"]
RELEASE_ARGS = ["build", "--manifest-path", "core/Cargo.toml", "--release", "--lib"]


@unittest.skipIf(os.name == "nt", "Make targets use the project's POSIX shell")
class CoreMakeTargetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.workspace = Path(self.temporary.name)
        shutil.copy2(ROOT / "Makefile", self.workspace / "Makefile")
        scripts = self.workspace / "scripts"
        scripts.mkdir()
        shutil.copy2(ROOT / "scripts" / "with-toolchain.sh", scripts / "with-toolchain.sh")
        tools = self.workspace / "tools"
        tools.mkdir()
        cargo = tools / "cargo"
        cargo.write_text(textwrap.dedent("""\
            #!/usr/bin/env python3
            import json
            import os
            from pathlib import Path
            import sys

            args = sys.argv[1:]
            with Path(os.environ["PENNEL_TEST_CARGO_LOG"]).open("a") as output:
                output.write(json.dumps(args) + "\\n")
            if args[0] == "build" and "--release" in args:
                Path("release-built").write_text("release fixture")
            failure_manifest = os.environ.get("PENNEL_TEST_CARGO_FAILURE_MANIFEST")
            matches_manifest = not failure_manifest or (
                "--manifest-path" in args
                and args[args.index("--manifest-path") + 1] == failure_manifest
            )
            if args[0] == os.environ.get("PENNEL_TEST_CARGO_FAILURE") and matches_manifest:
                sys.exit(17)
            """))
        cargo.chmod(0o755)
        self.log = self.workspace / "cargo.jsonl"
        self.environment = dict(os.environ)
        for name in ("MAKEFLAGS", "MFLAGS", "MAKELEVEL"):
            self.environment.pop(name, None)
        self.environment.pop("PENNEL_TEST_CARGO_FAILURE", None)
        self.environment.pop("PENNEL_TEST_CARGO_FAILURE_MANIFEST", None)
        self.environment["PATH"] = str(tools) + os.pathsep + os.environ["PATH"]
        self.environment["PENNEL_TEST_CARGO_LOG"] = str(self.log)

    def run_make(self, target, *, failure=None, failure_manifest=None, parallel=False):
        if failure:
            self.environment["PENNEL_TEST_CARGO_FAILURE"] = failure
        if failure_manifest:
            self.environment["PENNEL_TEST_CARGO_FAILURE_MANIFEST"] = failure_manifest
        command = ["make", "--no-print-directory", "-f", str(ROOT / "Makefile")]
        if parallel:
            command.append("--jobs=2")
        command.append(target)
        result = subprocess.run(command, cwd=self.workspace, env=self.environment, text=True, capture_output=True)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def test_regular_tests_do_not_build_release_artifacts(self):
        result, calls = self.run_make("test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [TEST_ARGS, KEYBOARD_TEST_ARGS])
        self.assertFalse((self.workspace / "release-built").exists())

    def test_core_still_builds_the_explicit_release_library(self):
        result, calls = self.run_make("core")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [RELEASE_ARGS])
        self.assertTrue((self.workspace / "release-built").is_file())

    def test_combined_verification_orders_release_before_tests_even_with_parallel_make(self):
        result, calls = self.run_make("verify-core", parallel=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [RELEASE_ARGS, TEST_ARGS, KEYBOARD_TEST_ARGS])

    def test_combined_verification_stops_when_release_build_fails(self):
        result, calls = self.run_make("verify-core", failure="build", parallel=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [RELEASE_ARGS])

    def test_combined_verification_propagates_test_failure(self):
        result, calls = self.run_make("verify-core", failure="test")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [RELEASE_ARGS, TEST_ARGS])

    def test_regular_test_failure_never_falls_back_to_release_build(self):
        result, calls = self.run_make("test", failure="test")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [TEST_ARGS])
        self.assertFalse((self.workspace / "release-built").exists())

    def test_regular_tests_propagate_keyboard_policy_failure_without_release_build(self):
        result, calls = self.run_make("test", failure="test", failure_manifest="core/keyboard/Cargo.toml")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [TEST_ARGS, KEYBOARD_TEST_ARGS])
        self.assertFalse((self.workspace / "release-built").exists())

    def test_combined_verification_propagates_keyboard_policy_failure(self):
        result, calls = self.run_make("verify-core", failure="test", failure_manifest="core/keyboard/Cargo.toml", parallel=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [RELEASE_ARGS, TEST_ARGS, KEYBOARD_TEST_ARGS])


if __name__ == "__main__":
    unittest.main()
