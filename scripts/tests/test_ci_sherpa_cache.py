import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


HELPER = Path(__file__).resolve().parents[1] / "ci-sherpa-cache.sh"


class SherpaCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / "target"

    def write(self, relative, value=b"retained"):
        path = self.target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value)
        return path

    def repair(self, *options):
        subprocess.run(["bash", str(HELPER), *options, str(self.target)], check=True, capture_output=True)

    def test_explicit_release_repairs_only_run_metadata_and_preserves_release_products(self):
        stale = [self.write("release/build/sherpa-onnx-sys-release/output"),
                 self.write("release/.fingerprint/sherpa-onnx-sys-release/run-build-script-build-script-build")]
        retained = [self.write("release/build/sherpa-onnx-sys-release/out/native.a"),
                    self.write("release/deps/libretained.rmeta"),
                    self.write("release/retained.dSYM/Contents/Resources/DWARF/retained"),
                    self.write("release/pennel"),
                    self.write("debug/build/sherpa-onnx-sys-debug/output"),
                    self.write("custom/build/sherpa-onnx-sys-custom/output"),
                    self.write("sherpa-onnx-prebuilt/linux/lib/libsherpa-onnx-c-api.a")]
        before = {path: path.read_bytes() for path in retained}
        self.repair("--profile", "release")
        self.assertTrue(all(not path.exists() for path in stale))
        self.assertEqual({path: path.read_bytes() for path in retained}, before)

    def test_explicit_target_release_repairs_new_layout_and_preserves_other_scopes(self):
        prefix = "x86_64-unknown-linux-gnu/release/build/sherpa-onnx-sys/0123456789abcdef"
        stale = [self.write(f"{prefix}/run/stdout"),
                 self.write(f"{prefix}/fingerprint/run-build-script-build-script-build.json")]
        retained = [self.write(f"{prefix}/out/native.a"),
                    self.write(f"{prefix}/out/libretained.rmeta"),
                    self.write("x86_64-unknown-linux-gnu/release/pennel"),
                    self.write("x86_64-unknown-linux-gnu/debug/build/sherpa-onnx-sys-other/output"),
                    self.write("release/build/sherpa-onnx-sys-other/output"),
                    self.write("debug/build/sherpa-onnx-sys-other/output"),
                    self.write("custom/build/sherpa-onnx-sys-other/output"),
                    self.write("aarch64-apple-darwin/release/build/sherpa-onnx-sys-other/output"),
                    self.write("sherpa-onnx-prebuilt/linux/lib/libsherpa-onnx-c-api.a")]
        before = {path: path.read_bytes() for path in retained}
        self.repair("--profile", "release", "--target", "x86_64-unknown-linux-gnu")
        self.assertTrue(all(not path.exists() for path in stale))
        self.assertEqual({path: path.read_bytes() for path in retained}, before)

    def test_unapproved_profiles_and_unsafe_targets_are_refused_before_invalidation(self):
        marker = self.write("debug/build/sherpa-onnx-sys-retained/output")
        for options in [("--profile", "custom"), ("--profile", "../release"),
                        ("--target", "../../release"), ("--target", "-bad"),
                        ("--target", "x86_64-unknown-linux-gnu/../../release"),
                        ("--unknown", "value"), ("--profile",)]:
            result = subprocess.run(["bash", str(HELPER), *options, str(self.target)], capture_output=True)
            self.assertNotEqual(result.returncode, 0, options)
            self.assertEqual(marker.read_bytes(), b"retained")

    def test_symlinked_selected_target_is_refused_and_external_products_remain(self):
        outside = self.root / "external-target"
        path = outside / "release/build/sherpa-onnx-sys-test/output"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"outside")
        self.target.mkdir()
        (self.target / "x86_64-unknown-linux-gnu").symlink_to(outside, target_is_directory=True)
        result = subprocess.run(["bash", str(HELPER), "--profile", "release", "--target", "x86_64-unknown-linux-gnu", str(self.target)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(path.read_bytes(), b"outside")

    def test_invalidates_only_sherpa_metadata_and_keeps_complete_native_download(self):
        stale = [
            self.write("debug/build/sherpa-onnx-sys-first/output"),
            self.write("debug/.fingerprint/sherpa-onnx-sys-second/run-build-script-build-script-build.json"),
        ]
        retained = [
            self.write("debug/build/unrelated-crate/output"),
            self.write("debug/.fingerprint/unrelated-crate/lib.json"),
            self.write("debug/deps/libsibling.a"),
            self.write("debug/build/sherpa-onnx-sys-first/out/unknown-native-object.o"),
            self.write("debug/build/sherpa-onnx-sys-first/build-script-build"),
            self.write("release/pennel-server"),
            self.write("release/deps/librelease.a"),
            self.write("release/build/sherpa-onnx-sys-release/output"),
            self.write("release/.fingerprint/sherpa-onnx-sys-release/run-build-script-build-script-build.json"),
            self.write("custom/build/sherpa-onnx-sys-custom/output"),
            self.write("custom/.fingerprint/sherpa-onnx-sys-custom/run-build-script-build-script-build"),
            self.write("x86_64-unknown-linux-gnu/debug/build/sherpa-onnx-sys-cross/output"),
            self.write("sherpa-onnx-prebuilt/linux/lib/libsherpa-onnx-c-api.a"),
            self.write("sherpa-onnx-prebuilt/linux/lib/libonnxruntime.a"),
            self.write("sherpa-onnx-prebuilt/download.tar.bz2"),
        ]
        before = {path: path.read_bytes() for path in retained}
        self.repair()
        self.assertTrue(all(not path.exists() for path in stale))
        self.assertEqual({path: path.read_bytes() for path in retained}, before)

    def test_missing_native_download_is_refreshed_without_touching_siblings(self):
        self.write("sherpa-onnx-prebuilt/linux/lib/unrelated-placeholder")
        sibling = self.write("other-native/libother.a")
        self.repair()
        self.assertFalse((self.target / "sherpa-onnx-prebuilt").exists())
        self.assertEqual(sibling.read_bytes(), b"retained")

    def test_incomplete_native_products_and_archives_are_refused_and_preserved(self):
        for name in ["libonnxruntime.a", "download.tar.bz2", "retained.rmeta", "saved.dSYM/Contents/Resources/DWARF/native"]:
            with self.subTest(name=name):
                product = self.write("sherpa-onnx-prebuilt/" + name)
                metadata = self.write("debug/build/sherpa-onnx-sys-first/output")
                self.assert_refused()
                self.assertEqual(product.read_bytes(), b"retained")
                self.assertEqual(metadata.read_bytes(), b"retained")
                product.unlink()

    @unittest.skipUnless(os.name == "posix", "This fixture requires POSIX execution-mode bits")
    def test_extensionless_native_executables_with_owner_permissions_are_refused(self):
        for mode in (0o700, 0o750):
            with self.subTest(mode=oct(mode)):
                native = self.write("sherpa-onnx-prebuilt/bin/retained-native", b"compiled-native")
                native.chmod(mode)
                metadata = self.write("debug/build/sherpa-onnx-sys-first/output")
                compiled = self.write("debug/build/sherpa-onnx-sys-first/out/compiled.rmeta")
                self.assert_refused()
                self.assertEqual(native.read_bytes(), b"compiled-native")
                self.assertEqual(native.stat().st_mode & 0o777, mode)
                self.assertEqual(metadata.read_bytes(), b"retained")
                self.assertEqual(compiled.read_bytes(), b"retained")
                native.unlink()

    def test_new_layout_invalidates_run_metadata_only_and_preserves_unit_outputs(self):
        run = "debug/build/sherpa-onnx-sys/0123456789abcdef"
        compile_unit = "debug/build/sherpa-onnx-sys/fedcba9876543210"
        stale = [self.write(f"{run}/run/{name}") for name in ("stdout", "stderr", "root-output", "invoked.timestamp")]
        stale += [self.write(f"{run}/fingerprint/{name}") for name in
                  ("run-build-script-build-script-build", "run-build-script-build-script-build.json")]
        retained = [
            self.write(f"{run}/out/unknown-native-object.o"),
            self.write(f"{run}/out/retained.dSYM/Contents/Resources/DWARF/sherpa"),
            self.write(f"{compile_unit}/out/build_script_build"),
            self.write(f"{compile_unit}/fingerprint/build-script-build-script-build"),
            self.write("debug/build/unrelated-crate/0123456789abcdef/run/stdout"),
            self.write("release/build/sherpa-onnx-sys/0123456789abcdef/run/stdout"),
            self.write("custom/build/sherpa-onnx-sys/0123456789abcdef/run/stdout"),
            self.write("x86_64-unknown-linux-gnu/debug/build/sherpa-onnx-sys/0123456789abcdef/run/stdout"),
            self.write("sherpa-onnx-prebuilt/linux/lib/libsherpa-onnx-c-api.a"),
        ]
        before = {path: path.read_bytes() for path in retained}
        self.repair()
        self.assertTrue(all(not path.exists() for path in stale))
        self.assertEqual({path: path.read_bytes() for path in retained}, before)

    def test_new_layout_unknown_unit_refuses_before_legacy_metadata_is_invalidated(self):
        outside_layout = self.write("debug/build/sherpa-onnx-sys/unknown-layout/run/stdout")
        legacy = self.write("debug/build/sherpa-onnx-sys-legacy/output")
        self.assert_refused()
        self.assertEqual(outside_layout.read_bytes(), b"retained")
        self.assertEqual(legacy.read_bytes(), b"retained")

    def test_new_layout_symlinked_run_directory_is_refused_before_invalidation(self):
        outside = self.root / "outside-run"
        outside.mkdir()
        marker = outside / "stdout"
        marker.write_bytes(b"outside")
        run = self.target / "debug/build/sherpa-onnx-sys/0123456789abcdef/run"
        run.parent.mkdir(parents=True)
        run.symlink_to(outside, target_is_directory=True)
        legacy = self.write("debug/build/sherpa-onnx-sys-legacy/output")
        self.assert_refused()
        self.assertTrue(run.is_symlink())
        self.assertEqual(marker.read_bytes(), b"outside")
        self.assertEqual(legacy.read_bytes(), b"retained")

    def test_new_layout_symlinked_unit_or_fingerprint_is_refused(self):
        for relative in ("debug/build/sherpa-onnx-sys/0123456789abcdef",
                         "debug/build/sherpa-onnx-sys/fedcba9876543210/fingerprint"):
            with self.subTest(relative=relative):
                outside = self.root / ("outside-" + str(len(list(self.root.iterdir()))))
                outside.mkdir()
                marker = outside / "run-build-script-build-script-build"
                marker.write_bytes(b"outside")
                link = self.target / relative
                link.parent.mkdir(parents=True, exist_ok=True)
                link.symlink_to(outside, target_is_directory=True)
                self.assert_refused()
                self.assertTrue(link.is_symlink())
                self.assertEqual(marker.read_bytes(), b"outside")
                link.unlink()

    def test_missing_target_is_a_noop(self):
        self.repair()
        self.assertFalse(self.target.exists())

    def assert_refused(self):
        result = subprocess.run(["bash", str(HELPER), str(self.target)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)

    def test_symlinked_target_is_refused_and_preserved(self):
        outside = self.root / "outside"
        outside.mkdir()
        marker = outside / "retained"
        marker.write_bytes(b"outside")
        self.target.symlink_to(outside, target_is_directory=True)
        self.assert_refused()
        self.assertTrue(self.target.is_symlink())
        self.assertEqual(marker.read_bytes(), b"outside")

    def test_symlinked_package_is_refused_before_any_invalidation(self):
        outside = self.root / "outside"
        outside.mkdir()
        (outside / "output").write_bytes(b"outside")
        package = self.target / "debug/build/sherpa-onnx-sys-link"
        package.parent.mkdir(parents=True)
        package.symlink_to(outside, target_is_directory=True)
        retained = self.write("debug/.fingerprint/sherpa-onnx-sys-local/run-build-script-build-script-build")
        self.assert_refused()
        self.assertTrue(package.is_symlink())
        self.assertEqual((outside / "output").read_bytes(), b"outside")
        self.assertEqual(retained.read_bytes(), b"retained")

    def test_symlinked_metadata_file_is_refused_and_preserved(self):
        outside = self.root / "outside-metadata"
        outside.write_bytes(b"outside")
        metadata = self.target / "debug/build/sherpa-onnx-sys-first/output"
        metadata.parent.mkdir(parents=True)
        metadata.symlink_to(outside)
        self.assert_refused()
        self.assertTrue(metadata.is_symlink())
        self.assertEqual(outside.read_bytes(), b"outside")

    def test_symlinked_incomplete_native_download_is_refused(self):
        outside = self.root / "outside-native"
        outside.mkdir()
        (outside / "unpacked-object.o").write_bytes(b"outside")
        link = self.target / "sherpa-onnx-prebuilt/lib"
        link.parent.mkdir(parents=True)
        link.symlink_to(outside, target_is_directory=True)
        self.assert_refused()
        self.assertTrue(link.is_symlink())
        self.assertEqual((outside / "unpacked-object.o").read_bytes(), b"outside")

    @unittest.skipUnless(shutil.which("cargo"), "cargo is needed for the offline build-script fixture")
    def test_warm_cargo_build_reruns_build_script_after_metadata_repair(self):
        self.warm_cargo_fixture()

    @unittest.skipUnless(shutil.which("cargo"), "cargo is needed for the offline build-script fixture")
    def test_warm_release_cargo_build_reruns_without_deleting_products(self):
        self.warm_cargo_fixture(profile="release")

    @unittest.skipUnless(shutil.which("cargo") and shutil.which("rustc"), "Rust is needed for the explicit-target fixture")
    def test_warm_explicit_target_release_build_reruns_without_deleting_products(self):
        info = subprocess.check_output(["rustc", "-vV"], text=True)
        host = next(line.split(": ", 1)[1] for line in info.splitlines() if line.startswith("host: "))
        self.warm_cargo_fixture(profile="release", target=host)

    def warm_cargo_fixture(self, profile="debug", target=None):
        project = self.root / "fixture"
        (project / "src").mkdir(parents=True)
        (project / "Cargo.toml").write_text('[package]\nname="sherpa-onnx-sys"\nversion="0.0.0"\nedition="2021"\n')
        (project / "src/lib.rs").write_text("pub fn fixture() {}\n")
        (project / "build.rs").write_text('''
use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-env-changed=FIXTURE_RUNS");
    let marker = PathBuf::from(env::var("FIXTURE_RUNS").unwrap());
    let runs = fs::read_to_string(&marker).unwrap_or_default().parse::<u32>().unwrap_or(0);
    fs::write(marker, (runs + 1).to_string()).unwrap();
}
''')
        marker = self.root / "build-script-runs"
        environment = {name: value for name, value in os.environ.items()
                       if not any(part in name.upper() for part in ("TOKEN", "PASSWORD", "SECRET", "PRIVATE_KEY", "CREDENTIAL", "API_KEY"))}
        environment.update({"CARGO_TARGET_DIR": str(self.target), "CARGO_BUILD_BUILD_DIR": str(self.target),
                            "CARGO_INCREMENTAL": "0", "FIXTURE_RUNS": str(marker)})

        def build():
            arguments = ["cargo", "build" if profile == "release" else "check", "--offline", "--quiet"]
            if profile == "release":
                arguments.append("--release")
            if target:
                arguments += ["--target", target]
            subprocess.run(arguments, cwd=project, env=environment, check=True, capture_output=True)

        native = self.write("sherpa-onnx-prebuilt/linux/lib/libsherpa-onnx-c-api.a")
        build()
        build()
        self.assertEqual(marker.read_text(), "1", "the warm build must reuse its cached build script")
        metadata = list(self.target.rglob("run-build-script-build-script-build"))
        self.assertTrue(metadata, "the fixture must locate actual Cargo run fingerprints in its owned intermediate directory")
        products = {path: path.read_bytes() for path in self.target.rglob("*") if path.is_file() and path.suffix in (".rmeta", ".rlib")}
        self.assertTrue(products)
        options = ["--profile", profile]
        if target:
            options += ["--target", target]
        self.repair(*options)
        self.assertTrue(all(not path.exists() for path in metadata))
        self.assertEqual({path: path.read_bytes() for path in products}, products)
        build()
        self.assertEqual(marker.read_text(), "2", "the repair must make Cargo execute the build script again")
        self.assertEqual(native.read_bytes(), b"retained")

    def test_packaging_callers_select_the_planned_release_and_same_owned_build_root(self):
        repository = HELPER.parents[1]
        for name in ["desktop.yml", "desktop-candidate.yml"]:
            content = (repository / ".github/workflows" / name).read_text()
            self.assertIn("CARGO_BUILD_BUILD_DIR: ${{ github.workspace }}/apps/desktop/src-tauri/target", content)
            self.assertIn('--profile release --target "$PENNEL_CACHE_BUILD_TARGET" apps/desktop/src-tauri/target', content)
            self.assertIn("PENNEL_CACHE_BUILD_TARGET: ${{ matrix.target }}", content)
            self.assertIn('\"$py\" -B -m unittest discover -s scripts/tests -p test_ci_sherpa_cache.py', content)


if __name__ == "__main__":
    unittest.main()
