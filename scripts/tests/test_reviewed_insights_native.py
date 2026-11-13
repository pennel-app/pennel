import importlib.util
from pathlib import Path
from types import SimpleNamespace
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "reviewed_insights_native", SCRIPTS / "verify-reviewed-insights-native.py"
)
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class CompiledHelperTests(unittest.TestCase):
    def test_installed_storage_guard_wraps_the_compiler(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            guard = root / "home/.local/bin/worktree-storage"
            guard.parent.mkdir(parents=True)
            guard.write_text("synthetic storage guard")
            output = root / "output"
            output.mkdir()
            with mock.patch.object(native.Path, "home", return_value=root / "home"), mock.patch.object(
                native.subprocess, "run"
            ) as run:
                helper = native.compiled_helper(SimpleNamespace(window_helper=None), output)
            self.assertEqual(helper, output / "owned-window")
            self.assertEqual(
                run.call_args.args[0],
                [str(guard), "run", "--", "swiftc", "-O", str(output / "owned-window.swift"), "-o", str(helper)],
            )
            self.assertTrue(run.call_args.kwargs["check"])
            self.assertEqual(run.call_args.kwargs["timeout"], 240)
            self.assertEqual(run.call_args.kwargs["stderr"], subprocess.STDOUT)
            self.assertEqual(run.call_args.kwargs["stdout"].name, str(output / "window-helper-build.log"))

    def test_missing_storage_guard_runs_the_compiler_directly(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output"
            output.mkdir()
            with mock.patch.object(native.Path, "home", return_value=root / "home"), mock.patch.object(
                native.subprocess, "run"
            ) as run:
                helper = native.compiled_helper(SimpleNamespace(window_helper=None), output)
            self.assertEqual(
                run.call_args.args[0],
                ["swiftc", "-O", str(output / "owned-window.swift"), "-o", str(helper)],
            )

    def test_storage_guard_failure_is_not_retried_without_the_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            guard = root / "home/.local/bin/worktree-storage"
            guard.parent.mkdir(parents=True)
            guard.write_text("synthetic storage guard")
            output = root / "output"
            output.mkdir()
            with mock.patch.object(native.Path, "home", return_value=root / "home"), mock.patch.object(
                native.subprocess, "run", side_effect=subprocess.CalledProcessError(1, [str(guard)])
            ) as run, self.assertRaises(subprocess.CalledProcessError):
                native.compiled_helper(SimpleNamespace(window_helper=None), output)
            run.assert_called_once()
            self.assertEqual(run.call_args.args[0][0], str(guard))

    def test_supplied_helper_bypasses_compilation_and_storage_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            helper = root / "existing-window-helper"
            helper.write_text("synthetic prebuilt helper")
            output = root / "unused-output"
            with mock.patch.object(native.Path, "home") as home, mock.patch.object(native.subprocess, "run") as run:
                result = native.compiled_helper(SimpleNamespace(window_helper=helper), output)
            self.assertEqual(result, helper.resolve())
            home.assert_not_called()
            run.assert_not_called()
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
