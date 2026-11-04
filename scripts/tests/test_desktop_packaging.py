import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import re
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent


def load_script(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


release = load_script("desktop-release")
updates = load_script("desktop-update-manifest")
bundle = load_script("verify-desktop-bundle")


class ReleaseConfigurationTests(unittest.TestCase):
    def test_unsigned_default_and_preview_never_emit_update_artifacts(self):
        for name in ("tauri.conf.json", "tauri.preview.conf.json"):
            config = json.loads((ROOT / "apps" / "desktop" / "src-tauri" / name).read_text())
            self.assertFalse(config["bundle"]["createUpdaterArtifacts"])

    def test_release_requires_existing_key(self):
        with self.assertRaisesRegex(ValueError, "PUBLIC_KEY"):
            release.release_config("0.2.0", "win32", "", sign_command="signtool %1")

    def test_version_cannot_escape_output_directory(self):
        for version in ("../0.2.0", "0.2.0/unsafe", "01.2.0", "0.2"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.release_config(version, "win32", "existing-public-key", sign_command="signtool %1")

    def test_mac_build_number_must_increase_and_use_developer_id(self):
        for next_build, previous in ((83, 83), (82, 83), (1, None)):
            with self.subTest(next_build=next_build), self.assertRaises(ValueError):
                release.release_config("0.2.0", "darwin", "key", next_build, previous, "Developer ID Application: Pennel")
        with self.assertRaises(ValueError):
            release.release_config("0.2.0", "darwin", "key", 84, 83, "Apple Development: Pennel")
        config = release.release_config("0.2.0", "darwin", "key", 84, 83, "Developer ID Application: Pennel")
        self.assertEqual(config["bundle"]["macOS"]["bundleVersion"], "84")
        self.assertTrue(config["bundle"]["createUpdaterArtifacts"])

    def test_mac_homebrew_distribution_is_independent_of_windows_and_updater_keys(self):
        config = release.release_config("0.2.0", "darwin", None, signing_identity="Developer ID Application: Personal Developer", homebrew=True)
        self.assertFalse(config["bundle"]["createUpdaterArtifacts"])
        self.assertNotIn("plugins", config)
        self.assertEqual(config["bundle"]["macOS"]["bundleVersion"], "0.2.0")
        with self.assertRaises(ValueError):
            release.release_config("0.2.0", "darwin", None, signing_identity="Apple Development: Personal Developer", homebrew=True)
        with self.assertRaises(ValueError):
            release.release_config("0.2.0", "win32", None, homebrew=True)

    def test_explicit_native_build_numbers_still_must_increase_for_homebrew_candidate(self):
        for next_build, previous in ((83, 83), (82, 83), (1, None)):
            with self.subTest(next_build=next_build), self.assertRaises(ValueError):
                release.release_config("0.2.0", "darwin", None, next_build, previous, "Developer ID Application: Personal Developer", homebrew=True)

    def test_windows_signing_cannot_be_silently_skipped(self):
        with self.assertRaises(ValueError):
            release.release_config("0.2.0", "win32", "key")
        with self.assertRaises(ValueError):
            release.release_config("0.2.0", "win32", "key", sign_command="signtool sign")
        config = release.release_config("0.2.0", "win32", "key", certificate_thumbprint="A" * 40)
        self.assertEqual(config["bundle"]["windows"]["digestAlgorithm"], "sha256")

    def test_linux_uses_detached_update_signatures_without_mac_or_windows_credentials(self):
        config = release.release_config("0.2.0", "linux", "existing-key")
        self.assertTrue(config["bundle"]["createUpdaterArtifacts"])
        self.assertNotIn("macOS", config["bundle"])
        self.assertNotIn("windows", config["bundle"])

    def test_linux_debian_dependencies_cover_the_platform_helpers(self):
        config = json.loads((ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json").read_text())
        dependencies = set(config["bundle"]["linux"]["deb"]["depends"])
        self.assertTrue({
            "gstreamer1.0-tools", "gstreamer1.0-plugins-base", "gstreamer1.0-plugins-good",
            "gstreamer1.0-libav", "pulseaudio-utils", "x11-utils", "wl-clipboard",
        }.issubset(dependencies))
        self.assertNotIn("gstreamer1.0-plugins-bad", dependencies)

    def test_detached_signer_never_receives_secret_key_on_command_line(self):
        key = "a-test-key-not-an-actual-credential"
        with mock.patch.object(release, "run") as run:
            release.sign_detached("npx", Path("Pennel.deb"), "0.2.0", {"TAURI_SIGNING_PRIVATE_KEY": key})
            self.assertNotIn(key, [str(value) for value in run.call_args.args])
            self.assertEqual(run.call_args.kwargs["env"]["TAURI_SIGNING_PRIVATE_KEY"], key)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "existing-test-key"
            path.write_text(key)
            with mock.patch.object(release, "run") as run:
                release.sign_detached("npx", Path("Pennel.deb"), "0.2.0", {"TAURI_SIGNING_PRIVATE_KEY": str(path)})
                signing_env = run.call_args.kwargs["env"]
                self.assertNotIn("TAURI_SIGNING_PRIVATE_KEY", signing_env)
                self.assertEqual(signing_env["TAURI_SIGNING_PRIVATE_KEY_PATH"], str(path))


class LegacySparkleTests(unittest.TestCase):
    def test_tool_directory_must_be_explicit(self):
        with mock.patch.dict("os.environ", {}, clear=True), mock.patch.object(release, "run") as run:
            with self.assertRaisesRegex(ValueError, "PENNEL_SPARKLE_BIN is required"):
                release.prepare_sparkle(Path("Pennel.app"), "0.2.0", Path("output"))
            run.assert_not_called()

    def test_missing_generator_fails_before_packaging(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.dict("os.environ", {"PENNEL_SPARKLE_BIN": temporary}, clear=True), mock.patch.object(release, "run") as run:
                with self.assertRaisesRegex(ValueError, "generate_appcast"):
                    release.prepare_sparkle(Path("Pennel.app"), "0.2.0", Path(temporary))
                run.assert_not_called()

    def test_external_generator_preserves_legacy_archive_and_account(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            tools = output / "existing Sparkle tools"
            tools.mkdir()
            generator = tools / "generate_appcast"
            generator.write_text("existing external tool")
            archive = output / "Pennel-0.2.0.zip"

            def package(*args, **kwargs):
                if args[0] == "ditto":
                    Path(args[-1]).write_bytes(b"candidate archive")

            with mock.patch.dict("os.environ", {"PENNEL_SPARKLE_BIN": str(tools)}, clear=True), mock.patch.object(release, "run", side_effect=package) as run:
                release.prepare_sparkle(Path("Pennel.app"), "0.2.0", output)
                self.assertEqual((output / "legacy-sparkle" / archive.name).read_bytes(), archive.read_bytes())
                self.assertEqual(run.call_args.args, (
                    generator, "--account", "pennel", "--download-url-prefix",
                    "https://github.com/pennel-app/pennel/releases/download/v0.2.0/",
                    "--link", "https://pennel.app", "-o", output / "appcast.xml", output / "legacy-sparkle",
                ))


class PortableNativeBuildTests(unittest.TestCase):
    def test_x86_64_v3_baseline_applies_only_to_non_mac_x64_distribution_targets(self):
        original = {"TRANSCRIBE_CMAKE_ARGS": '-DCUSTOM="a path"', "CMAKE_ARGS": "-DOTHER=ON", "PATH": "/existing/tools"}
        flags = release.PORTABLE_CMAKE_FLAGS
        for name in ("-DTRANSCRIBE_X86_CONSERVATIVE=ON", "-DGGML_NATIVE=OFF", "-DGGML_AVX2=ON", "-DGGML_FMA=ON", "-DGGML_F16C=ON", "-DGGML_BMI2=ON", "-DGGML_AVX512=OFF"):
            self.assertIn(name, flags.split())
        for platform, target in (("win32", "x86_64-pc-windows-msvc"), ("linux", "x86_64-unknown-linux-gnu")):
            with self.subTest(platform=platform):
                environment = release.portable_environment(platform, target, original)
                self.assertEqual(environment["TRANSCRIBE_CMAKE_ARGS"], original["TRANSCRIBE_CMAKE_ARGS"] + " " + flags)
                self.assertEqual(environment["CMAKE_ARGS"], original["CMAKE_ARGS"] + " " + flags)
                self.assertEqual(environment["PATH"], original["PATH"])
        for platform, target in (("darwin", "aarch64-apple-darwin"), ("darwin", "x86_64-apple-darwin"), ("linux", "aarch64-unknown-linux-gnu")):
            with self.subTest(platform=platform, target=target):
                self.assertEqual(release.portable_environment(platform, target, original), original)
        self.assertEqual(original["TRANSCRIBE_CMAKE_ARGS"], '-DCUSTOM="a path"')

    def test_earlier_host_or_simd_flags_cannot_override_the_distribution_baseline(self):
        environment = release.portable_environment("linux", "x86_64-unknown-linux-gnu", {
            "TRANSCRIBE_CMAKE_ARGS": "-DTRANSCRIBE_X86_CONSERVATIVE=OFF -DGGML_NATIVE=ON -DGGML_AVX512=ON",
            "CMAKE_ARGS": "-DGGML_AVX2=OFF",
        })
        for name in ("TRANSCRIBE_CMAKE_ARGS", "CMAKE_ARGS"):
            self.assertTrue(environment[name].endswith(" " + release.PORTABLE_CMAKE_FLAGS))
        environment = release.portable_environment("linux", "x86_64-unknown-linux-gnu", {})
        self.assertEqual(environment, {"TRANSCRIBE_CMAKE_ARGS": release.PORTABLE_CMAKE_FLAGS})

    def test_native_cache_must_prove_the_x86_64_v3_baseline(self):
        target = "x86_64-unknown-linux-gnu"
        with tempfile.TemporaryDirectory() as temporary:
            target_dir = Path(temporary)
            with self.assertRaises(ValueError):
                release.verify_portable_caches(target_dir, target)
            cache = target_dir / target / "release" / "build" / "transcribe-cpp-sys-test" / "out" / "build" / "CMakeCache.txt"
            cache.parent.mkdir(parents=True)
            safe = "TRANSCRIBE_X86_CONSERVATIVE:UNINITIALIZED=ON\n" + "".join(
                f"{name}:BOOL={value}\n" for name, value in release.X86_64_BASELINE.items()) + "GGML_LLAMAFILE:BOOL=ON\n"
            cache.write_text(safe)
            self.assertEqual(release.verify_portable_caches(target_dir, target)["cmake_caches_checked"], 1)
            unsafe_variants = (
                safe.replace("GGML_NATIVE:BOOL=OFF", "GGML_NATIVE:BOOL=ON"),
                safe.replace("GGML_AVX2:BOOL=ON", "GGML_AVX2:BOOL=OFF"),
                safe.replace("GGML_AVX512:BOOL=OFF", "GGML_AVX512:BOOL=ON"),
                safe.replace("GGML_BMI2:BOOL=ON\n", ""),
                safe + "GGML_AVX10:BOOL=ON\n",
                safe.replace("CONSERVATIVE:UNINITIALIZED=ON", "CONSERVATIVE:UNINITIALIZED=OFF"),
            )
            for unsafe in unsafe_variants:
                with self.subTest(unsafe=unsafe):
                    cache.write_text(unsafe)
                    with self.assertRaises(ValueError):
                        release.verify_portable_caches(target_dir, target)

    def test_runtime_processor_check_requires_exactly_the_enabled_simd_features(self):
        source = (Path(__file__).resolve().parents[2] / "core" / "src" / "cpu_support.rs").read_text(encoding="utf-8")
        declaration = re.search(r'#\[cfg\(all\(target_arch = "x86_64", not\(target_os = "macos"\)\)\)\]\s*const REQUIRED_X86_FEATURES: &\[&str\] = &\[([^\]]*)\]', source)
        self.assertIsNotNone(declaration)
        required = set(re.findall(r'"([^"]+)"', declaration.group(1)))
        enabled = {release.X86_64_RUNTIME_FEATURES[name] for name, value in release.X86_64_BASELINE.items() if value == "ON"}
        self.assertEqual(required, enabled)

    def test_distribution_build_cleans_only_native_dependency_for_selected_release_target(self):
        target = "x86_64-unknown-linux-gnu"
        environment = {"PATH": "/tools", "TRANSCRIBE_CMAKE_ARGS": "-DOTHER=ON"}
        with mock.patch.object(release.shutil, "which", return_value="/tools/cargo"), mock.patch.object(release.subprocess, "check_output", return_value='{"target_directory":"/tmp/target"}'), mock.patch.object(release, "run") as run, mock.patch.object(release, "verify_portable_caches", return_value={"x86_64_baseline": "x86-64-v3 (no AVX-512)", "cmake_caches_checked": 1}) as verify:
            result = release.build_distributable("/tools/npx", target, "deb,appimage", None, environment, "linux")
            self.assertIn("clean", run.call_args_list[0].args)
            self.assertIn("transcribe-cpp-sys", run.call_args_list[0].args)
            self.assertIn("--release", run.call_args_list[0].args)
            self.assertIn(target, run.call_args_list[0].args)
            self.assertEqual(run.call_args_list[1].args[:3], ("/tools/npx", "tauri", "build"))
            self.assertIn("-DOTHER=ON", run.call_args_list[1].kwargs["env"]["TRANSCRIBE_CMAKE_ARGS"])
            verify.assert_called_once_with(Path("/tmp/target"), target)
            self.assertEqual(result["x86_64_baseline"], "x86-64-v3 (no AVX-512)")

    def test_mac_distribution_build_keeps_existing_native_configuration(self):
        environment = {"TRANSCRIBE_CMAKE_ARGS": "-DMAC_OPTION=ON", "CMAKE_ARGS": "-DOTHER=ON"}
        with mock.patch.object(release, "run") as run, mock.patch.object(release.subprocess, "check_output") as metadata, mock.patch.object(release, "verify_portable_caches") as verify:
            self.assertIsNone(release.build_distributable("npx", "aarch64-apple-darwin", "app,dmg", None, environment, "darwin"))
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.kwargs["env"], environment)
            metadata.assert_not_called()
            verify.assert_not_called()


class ManifestGateTests(unittest.TestCase):
    def test_every_acceptance_gate_and_revision_are_required(self):
        receipt = {"revision": "candidate-sha", **{key: "passed" for key in updates.REQUIRED_GATES}}
        updates.accepted(receipt, "candidate-sha")
        with self.assertRaises(ValueError):
            updates.accepted(receipt, "another-sha")
        for key in updates.REQUIRED_GATES:
            with self.subTest(gate=key), self.assertRaises(ValueError):
                updates.accepted({**receipt, key: "pending"}, "candidate-sha")

    def test_acceptance_binds_exact_artifacts_and_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifacts = {key: Path(temporary) / key for key in updates.REQUIRED_TARGETS}
            for artifact in artifacts.values():
                artifact.write_bytes(b"accepted candidate")
            receipt = {"version": "0.2.0", "artifacts": {key: hashlib.sha256(path.read_bytes()).hexdigest() for key, path in artifacts.items()}}
            updates.verify_artifacts(receipt, "0.2.0", artifacts)
            with self.assertRaises(ValueError):
                updates.verify_artifacts(receipt, "0.3.0", artifacts)
            artifacts["windows-x86_64"].write_bytes(b"rebuilt after acceptance")
            with self.assertRaises(ValueError):
                updates.verify_artifacts(receipt, "0.2.0", artifacts)

    def test_manifest_requires_both_targets_and_signed_artifacts(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            mac = folder / "Pennel.app.tar.gz"
            windows = folder / "Pennel setup.exe"
            linux = folder / "Pennel.AppImage"
            for artifact in (mac, windows, linux):
                artifact.write_bytes(b"test fixture")
                artifact.with_name(artifact.name + ".sig").write_text(base64.b64encode(b"signature fixture").decode())
            artifacts = {"darwin-aarch64": mac, "windows-x86_64": windows, "linux-x86_64": linux}
            result = updates.manifest("0.2.0", "https://pennel.app/releases/0.2.0", artifacts, "Notes", "2026-10-24T12:00:00Z")
            self.assertEqual(result["version"], "0.2.0")
            self.assertTrue(result["platforms"]["windows-x86_64"]["url"].endswith("Pennel%20setup.exe"))
            with self.assertRaises(ValueError):
                updates.manifest("0.2.0", "http://pennel.app", artifacts, "", "")
            with self.assertRaises(ValueError):
                updates.manifest("0.2.0", "https://pennel.app", {"darwin-aarch64": mac}, "", "")
            windows.with_name(windows.name + ".sig").unlink()
            with self.assertRaises(OSError):
                updates.manifest("0.2.0", "https://pennel.app", artifacts, "", "")

    def test_missing_linux_acceptance_cannot_produce_a_production_feed(self):
        receipt = {"revision": "candidate-sha", **{key: "passed" for key in updates.REQUIRED_GATES}}
        for gate in ("linux_x11", "linux_wayland"):
            with self.subTest(gate=gate), self.assertRaises(ValueError):
                updates.accepted({key: value for key, value in receipt.items() if key != gate}, "candidate-sha")


class BundleMetadataTests(unittest.TestCase):
    def test_production_identity_permissions_and_executable_are_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            app = Path(temporary) / "Pennel.app"
            binary = app / "Contents" / "MacOS" / "pennel"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"not a Mach-O; metadata-only fixture")
            plist_path = app / "Contents" / "Info.plist"
            metadata = {
                "CFBundleIdentifier": "dev.pennel.Pennel", "CFBundleExecutable": "pennel",
                "LSMinimumSystemVersion": "14.4", "LSUIElement": True,
                **{key: "Permission explanation" for key in bundle.USAGE_KEYS},
            }
            plist_path.write_bytes(plistlib.dumps(metadata))
            self.assertEqual(bundle.inspect_bundle(app, "dev.pennel.Pennel")[1], binary)
            with self.assertRaises(ValueError):
                bundle.inspect_bundle(app, "dev.pennel.Pennel.preview")
            metadata["NSAudioCaptureUsageDescription"] = ""
            plist_path.write_bytes(plistlib.dumps(metadata))
            with self.assertRaises(ValueError):
                bundle.inspect_bundle(app, "dev.pennel.Pennel")

    def test_preview_identity_does_not_replace_native_product_identity(self):
        folder = ROOT / "apps" / "desktop" / "src-tauri"
        mac = json.loads((folder / "tauri.macos.conf.json").read_text())
        preview = json.loads((folder / "tauri.preview.conf.json").read_text())
        self.assertEqual(mac["identifier"], "dev.pennel.Pennel")
        self.assertNotEqual(preview["identifier"], mac["identifier"])
        self.assertEqual(mac["bundle"]["macOS"]["minimumSystemVersion"], "14.4")


if __name__ == "__main__":
    unittest.main()
