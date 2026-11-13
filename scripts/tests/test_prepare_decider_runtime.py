import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("prepare_decider_runtime", ROOT / "scripts/prepare-decider-runtime.py")
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


class ManagedDeciderRuntimePreparationTests(unittest.TestCase):
    def test_default_validation_never_fetches_builds_or_loads_models(self):
        with mock.patch("sys.argv", ["prepare-decider-runtime.py"]), mock.patch.object(runtime, "run", side_effect=AssertionError("No tool action")):
            with contextlib.redirect_stdout(io.StringIO()) as output:
                runtime.main()
        value = json.loads(output.getvalue())
        self.assertTrue(value["lock_valid"])
        self.assertEqual(value["model_loads"], 0)
        self.assertEqual(value["inference_requests"], 0)

    def test_unpinned_or_changed_runtime_source_is_refused(self):
        lock = runtime.read_lock()
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            (source / "ggml").mkdir()
            (source / "ggml/CMakeLists.txt").write_text("owned fixture")
            for output in ["wrong-revision", lock["source"]["revision"]]:
                def command(*args, **kwargs):
                    if args[:3] == ("git", "rev-parse", "HEAD"):
                        return output
                    if args[:3] == ("git", "status", "--porcelain"):
                        return "?? injected.cpp\n"
                    self.fail("Source verification must stop before any write")
                with mock.patch.object(runtime, "run", side_effect=command), self.assertRaises(ValueError):
                    runtime.verify_source(source, lock)

    def test_changed_build_identity_cannot_be_relabelled_after_signing(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            (output / "manifest.json").write_text(json.dumps({"build_signature": "old-source"}))
            binary = output / "libpennel_decider.dylib"
            binary.write_bytes(b"owned unchanged binary")
            with mock.patch("sys.argv", ["prepare-decider-runtime.py", "--refresh-manifest", "--output", directory]):
                with mock.patch.object(runtime, "build_identity", return_value=("new-source", {})):
                    with self.assertRaisesRegex(ValueError, "rebuild rather than relabel"):
                        runtime.main()
            self.assertEqual(binary.read_bytes(), b"owned unchanged binary")

    def test_distribution_dependency_check_rejects_machine_specific_or_extra_rpath(self):
        lock = runtime.read_lock()
        for dependency in ["/opt/homebrew/opt/ggml/lib/libggml.dylib", "@rpath/libpennel_decider_external.dylib"]:
            def command(*args, **kwargs):
                if args[0] == "lipo": return "arm64\n"
                if args[0] == "vtool": return "minos 14.4\n"
                return "owned:\n\t" + dependency + " (compatibility version 0.0.0)\n"
            with mock.patch.object(runtime, "run", side_effect=command), self.assertRaises(ValueError):
                runtime.check_binary(Path("owned.dylib"), lock)


if __name__ == "__main__":
    unittest.main()
