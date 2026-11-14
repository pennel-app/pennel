import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("stage_decider_runtime", ROOT / "scripts/stage-decider-runtime.py")
stage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stage)


class DeciderRuntimeStagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.patch = mock.patch.object(stage, "ROOT", self.root)
        self.patch.start()
        self.resources, self.marker, self.cache = stage.paths()
        self.resources.mkdir(parents=True)
        (self.resources / "README.md").write_text("Owned source placeholder\n")
        self.bundle = self.root / "owned-source-bundle"
        self.bundle.mkdir()
        self.inputs = {"owned_source": "fixed"}
        self.signature = "a" * 64
        (self.bundle / "libpennel_decider.dylib").write_bytes(b"owned-original-runtime")
        for name in ["LICENSE.llama.cpp", "LICENSE.ggml"]:
            (self.bundle / name).write_text("Owned license fixture")
        self.refresh(self.bundle)
        self.prepare = SimpleNamespace(read_lock=lambda: {}, build_identity=lambda _: (self.signature, self.inputs),
                                       check_binary=lambda *args: {}, DEFAULT_SOURCE=self.root / "source", DEFAULT_BUILD_ROOT=self.root / "build")
        self.identity = {"identity": "B" * 40, "adhoc": False, "team": "B538Q2K76C"}

    def tearDown(self):
        self.patch.stop()
        self.temporary.cleanup()

    def refresh(self, bundle):
        binary = bundle / "libpennel_decider.dylib"
        (bundle / "manifest.json").write_text(json.dumps({"build_signature": self.signature, "source_inputs": self.inputs,
            "component": {"file": binary.name, "sha256": stage.digest(binary), "bytes": binary.stat().st_size}}))

    def prepare_command(self, arguments, *_args, **_kwargs):
        self.assertIn("--refresh-manifest", arguments)
        self.refresh(Path(arguments[arguments.index("--output") + 1]))

    def sign(self, binary, *_args):
        self.assertNotEqual(binary.resolve(), (self.bundle / binary.name).resolve())
        binary.write_bytes(binary.read_bytes() + b"-owned-signature")

    def invoke(self, **kwargs):
        with mock.patch.object(stage, "module", return_value=self.prepare), mock.patch.object(stage, "selected_identity", return_value=self.identity), \
             mock.patch.object(stage, "sign_copy", side_effect=self.sign), mock.patch.object(stage, "prepare_command", side_effect=self.prepare_command), \
             mock.patch.object(stage.sys, "platform", "darwin"):
            return stage.stage_runtime(stage.TARGET, prepared_runtime=self.bundle, environment={}, **kwargs)

    def test_signing_changes_only_fresh_copy_and_manifest_is_refreshed_afterward(self):
        before = {path.name: path.read_bytes() for path in self.bundle.iterdir()}
        result = self.invoke()
        self.assertTrue(result["source_unchanged"])
        self.assertEqual({path.name: path.read_bytes() for path in self.bundle.iterdir()}, before)
        self.assertEqual((self.resources / "libpennel_decider.dylib").read_bytes(), before["libpennel_decider.dylib"] + b"-owned-signature")
        manifest = json.loads((self.resources / "manifest.json").read_text())
        self.assertEqual(manifest["component"]["sha256"], stage.digest(self.resources / "libpennel_decider.dylib"))
        self.assertEqual(stage.validate_stage(self.resources, self.marker)["manifest.json"], stage.digest(self.resources / "manifest.json"))

    def test_copied_app_receives_exact_signed_bytes_and_stale_copy_is_refused(self):
        self.invoke()
        app = self.root / "Pennel.app"
        destination = app / "Contents/Resources/decider-runtime"
        shutil.copytree(self.resources, destination)
        with mock.patch.object(stage, "module", return_value=self.prepare):
            self.assertTrue(stage.verify_bundle(app)["copied_bytes_match_stage"])
            (destination / "libpennel_decider.dylib").write_bytes(b"old-runtime")
            with self.assertRaisesRegex(ValueError, "manifest"):
                stage.verify_bundle(app)

    def test_unsupported_target_keeps_only_placeholder_and_retains_previous_owned_copy(self):
        self.invoke()
        expected = (self.resources / "libpennel_decider.dylib").read_bytes()
        with mock.patch.object(stage, "selected_identity", side_effect=AssertionError("Unsupported targets must not sign")):
            result = stage.stage_runtime("x86_64-pc-windows-msvc", environment={})
        self.assertFalse(result["staged"])
        self.assertEqual({path.name for path in self.resources.iterdir()}, {"README.md"})
        retained = list((self.cache / "retained").glob("*/libpennel_decider.dylib"))
        self.assertEqual(len(retained), 1)
        self.assertEqual(retained[0].read_bytes(), expected)

    def test_unknown_stage_files_and_changed_owned_files_are_preserved_without_tool_actions(self):
        unknown = self.resources / "unrelated-work.txt"
        unknown.write_text("Retain this work")
        with mock.patch.object(stage, "selected_identity", side_effect=AssertionError("Must validate before signing/build")), self.assertRaisesRegex(ValueError, "Unknown files"):
            stage.stage_runtime(stage.TARGET, build=True, environment={})
        self.assertEqual(unknown.read_text(), "Retain this work")
        unknown.unlink()
        self.invoke()
        binary = self.resources / "libpennel_decider.dylib"
        binary.write_bytes(b"owned manual edit")
        with self.assertRaisesRegex(ValueError, "changed"):
            stage.stage_runtime("x86_64-unknown-linux-gnu", environment={})
        self.assertEqual(binary.read_bytes(), b"owned manual edit")

    def test_stage_and_source_symlinks_cannot_redirect_signing(self):
        (self.bundle / "libpennel_decider.dylib").unlink()
        outside = self.root / "outside.dylib"
        outside.write_bytes(b"Preserved unrelated library")
        (self.bundle / "libpennel_decider.dylib").symlink_to(outside)
        with mock.patch.object(stage, "module", return_value=self.prepare), mock.patch.object(stage, "selected_identity", return_value=self.identity), \
             mock.patch.object(stage, "sign_copy", side_effect=AssertionError("No symlink signing")), mock.patch.object(stage.sys, "platform", "darwin"), self.assertRaisesRegex(ValueError, "regular files"):
            stage.stage_runtime(stage.TARGET, prepared_runtime=self.bundle, environment={})
        self.assertEqual(outside.read_bytes(), b"Preserved unrelated library")
        self.resources.rename(self.resources.with_name("retained-placeholder"))
        self.resources.symlink_to(self.resources.with_name("retained-placeholder"), target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "regular owned"):
            stage.validate_stage(self.resources, self.marker)

    def test_an_unowned_binary_is_not_deleted_even_on_unsupported_targets(self):
        binary = self.resources / "libpennel_decider.dylib"
        binary.write_bytes(b"Unknown but retained")
        with self.assertRaisesRegex(ValueError, "Unowned"):
            stage.stage_runtime("x86_64-unknown-linux-gnu", environment={})
        self.assertEqual(binary.read_bytes(), b"Unknown but retained")

    def test_local_adhoc_and_missing_identity_are_refused(self):
        for environment in [{}, {"APPLE_SIGNING_IDENTITY": "-"}, {"CI": "true", "APPLE_SIGNING_IDENTITY": "-"}]:
            with self.assertRaises(ValueError):
                stage.selected_identity(environment)
        self.assertTrue(stage.selected_identity({"CI": "true", "GITHUB_ACTIONS": "true", "APPLE_SIGNING_IDENTITY": "-"})["adhoc"])

    def test_another_team_cannot_sign_a_packaging_copy(self):
        module = importlib.util.spec_from_file_location("owned_mac_signing", ROOT / "scripts/mac_signing.py")
        signing = importlib.util.module_from_spec(module)
        module.loader.exec_module(signing)
        work = "Developer ID Application: Work Developer (OTHERTEAM01)"
        fake = SimpleNamespace(command=lambda _: '  1) ' + "B" * 40 + ' "' + work + '"\n', identities=signing.identities,
                               choose_identity=signing.choose_identity, certificate_for_identity=mock.Mock(side_effect=AssertionError("No work certificate read")))
        with mock.patch.object(stage, "module", return_value=fake), self.assertRaises(ValueError):
            stage.selected_identity({"APPLE_SIGNING_IDENTITY": work})
        fake.certificate_for_identity.assert_not_called()
        with mock.patch.object(stage, "tool", side_effect=["", "TeamIdentifier=OTHERTEAM01\n"]), self.assertRaisesRegex(ValueError, "personal Apple team"):
            stage.verify_signature(self.bundle / "libpennel_decider.dylib", self.identity, {})

    def test_default_validation_does_not_build_fetch_or_sign(self):
        with mock.patch("sys.argv", ["stage-decider-runtime.py"]), mock.patch.object(stage, "stage_runtime", side_effect=AssertionError("No stage action")), \
             mock.patch.object(stage, "tool", side_effect=AssertionError("No tool action")):
            stage.main()

    def test_tauri_hook_is_before_bundle_and_maps_the_staged_directory(self):
        config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())
        self.assertEqual(config["build"]["beforeBuildCommand"], "npm run build")
        self.assertEqual(config["build"]["beforeBundleCommand"], "node ../../scripts/stage-decider-runtime.mjs")
        self.assertEqual(config["bundle"]["resources"]["resources/decider-runtime"], "decider-runtime")
        hook = (ROOT / "scripts/stage-decider-runtime.mjs").read_text()
        self.assertIn("TAURI_ENV_TARGET_TRIPLE", hook)
        self.assertIn('args.push("--fetch-source")', hook)

    def test_model_worker_dispatch_precedes_the_gui_and_single_instance(self):
        main = (ROOT / "apps/desktop/src-tauri/src/main.rs").read_text().split("fn main() {", 1)[1]
        self.assertLess(main.index("managed_local_decider_worker_main"), main.index("tauri::Builder"))
        setup = (ROOT / "apps/desktop/src-tauri/src/main.rs").read_text().split("fn setup(", 1)[1].split("fn main() {", 1)[0]
        self.assertIn('resources.join("decider-runtime")', setup)
        self.assertIn("set_managed_local_decider_runtime", setup)


if __name__ == "__main__":
    unittest.main()
