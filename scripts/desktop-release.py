#!/usr/bin/env python3
"""Build signed desktop candidates locally; never publish or alter tracked versions."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / "apps" / "desktop"
UPDATES_URL = "https://pennel.app/updates/latest.json"


def mac_signing():
    spec = importlib.util.spec_from_file_location("pennel_mac_signing", ROOT / "scripts/mac_signing.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def release_config(version, platform, public_key, build_number=None, previous_build=None, signing_identity=None, sign_command=None, certificate_thumbprint=None, *, homebrew=False):
    if not re.fullmatch(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("Version must be a semantic version such as 0.2.0 or 0.2.0-beta.1")
    if homebrew and platform != "darwin":
        raise ValueError("The Homebrew distribution path is macOS-only")
    if not homebrew and (not public_key or not public_key.strip()):
        raise ValueError("TAURI_UPDATER_PUBLIC_KEY is required; existing keys are never generated or replaced")
    config = {
        "version": version,
        "bundle": {"createUpdaterArtifacts": not homebrew},
    }
    if not homebrew:
        config["plugins"] = {"updater": {"pubkey": public_key.strip(), "endpoints": [UPDATES_URL]}}
    if platform == "darwin":
        if homebrew and build_number is None and "-" in version:
            raise ValueError("Prerelease Mac candidates require explicit increasing numeric native build numbers")
        if (not homebrew or build_number is not None or previous_build is not None) and (
            not isinstance(build_number, int) or not isinstance(previous_build, int) or previous_build < 0 or build_number <= previous_build
        ):
            raise ValueError("Mac build number must be greater than the last distributed native or unified build number")
        if not signing_identity or not (signing_identity.startswith("Developer ID Application:") or re.fullmatch(r"[A-Fa-f0-9]{40}", signing_identity)):
            raise ValueError("APPLE_SIGNING_IDENTITY must name an existing Developer ID Application certificate")
        config["bundle"]["macOS"] = {"bundleVersion": str(build_number) if build_number is not None else version, "signingIdentity": signing_identity}
    elif platform == "win32":
        if sign_command:
            if "%1" not in sign_command:
                raise ValueError("PENNEL_WINDOWS_SIGN_COMMAND must contain %1 for the file to sign")
            config["bundle"]["windows"] = {"signCommand": sign_command}
        elif certificate_thumbprint and re.fullmatch(r"[A-Fa-f0-9]{40}", certificate_thumbprint):
            config["bundle"]["windows"] = {
                "certificateThumbprint": certificate_thumbprint,
                "digestAlgorithm": "sha256", "timestampUrl": "https://timestamp.digicert.com", "tsp": True,
            }
        else:
            raise ValueError("Provide an existing PENNEL_WINDOWS_CERTIFICATE_THUMBPRINT or PENNEL_WINDOWS_SIGN_COMMAND")
    elif platform == "linux":
        pass
    else:
        raise ValueError("Candidates target macOS arm64, Windows x64 and Linux x64")
    return config


def run(*args, cwd=ROOT, env=None):
    subprocess.run([str(arg) for arg in args], cwd=cwd, env=env, check=True)


def portable_target(platform, target):
    return platform in ("win32", "linux") and target.startswith("x86_64-")


# x86-64-v3 without AVX-512: every Intel Core since Haswell and AMD since Zen. The conservative option
# keeps any ggml SIMD flag not listed here off; core/src/cpu_support.rs refuses local models at run
# time on processors without these features (e.g. Pentium Gold/Celeron) instead of crashing.
X86_64_BASELINE = {
    "GGML_NATIVE": "OFF",
    "GGML_SSE42": "ON",
    "GGML_AVX": "ON",
    "GGML_AVX2": "ON",
    "GGML_FMA": "ON",
    "GGML_F16C": "ON",
    "GGML_BMI2": "ON",
    "GGML_AVX_VNNI": "OFF",
    "GGML_AVX512": "OFF",
    "GGML_AVX512_VBMI": "OFF",
    "GGML_AVX512_VNNI": "OFF",
    "GGML_AVX512_BF16": "OFF",
    "GGML_AMX_TILE": "OFF",
    "GGML_AMX_INT8": "OFF",
    "GGML_AMX_BF16": "OFF",
}
# Rust names of the features the runtime check requires for each enabled flag.
X86_64_RUNTIME_FEATURES = {"GGML_SSE42": "sse4.2", "GGML_AVX": "avx", "GGML_AVX2": "avx2", "GGML_FMA": "fma", "GGML_F16C": "f16c", "GGML_BMI2": "bmi2"}
PORTABLE_CMAKE_FLAGS = " ".join(["-DTRANSCRIBE_X86_CONSERVATIVE=ON"] + [f"-D{name}={value}" for name, value in X86_64_BASELINE.items()])


def portable_environment(platform, target, environment):
    result = dict(environment)
    if portable_target(platform, target):
        # Appended last so earlier host-tuned or user values cannot change the shipped baseline.
        result["TRANSCRIBE_CMAKE_ARGS"] = (result.get("TRANSCRIBE_CMAKE_ARGS", "").rstrip() + " " + PORTABLE_CMAKE_FLAGS).lstrip()
        # The dependency applies CMAKE_ARGS afterwards; preserve it while keeping the same baseline last.
        if result.get("CMAKE_ARGS", "").strip():
            result["CMAKE_ARGS"] = result["CMAKE_ARGS"].rstrip() + " " + PORTABLE_CMAKE_FLAGS
    return result


def verify_portable_caches(target_dir, target):
    caches = list((target_dir / target / "release" / "build").glob("transcribe-cpp-sys-*/out/**/CMakeCache.txt"))
    if not caches:
        raise ValueError("Cannot verify the distributable native CPU baseline: transcribe CMake cache is missing")
    enabled = ("ON", "TRUE", "1", "YES")
    for cache in caches:
        values = {}
        for line in cache.read_text(encoding="utf-8").splitlines():
            entry, separator, value = line.partition("=")
            if separator and not entry.startswith(("#", "//")):
                values[entry.partition(":")[0]] = value.strip().upper()
        if values.get("TRANSCRIBE_X86_CONSERVATIVE") not in enabled:
            raise ValueError("Native distribution build did not enable TRANSCRIBE_X86_CONSERVATIVE")
        wrong = []
        for name, expected in X86_64_BASELINE.items():
            if name not in values:
                if expected == "ON" or name == "GGML_NATIVE":
                    wrong.append(f"{name} missing")
            elif (values[name] in enabled) != (expected == "ON"):
                wrong.append(f"{name}={values[name]}")
        wrong += [f"{name}={value}" for name, value in values.items()
                  if re.fullmatch(r"GGML_(?:NATIVE|SSE.*|AVX.*|BMI.*|FMA.*|F16C|AMX.*)", name)
                  and name not in X86_64_BASELINE and value in enabled]
        if wrong:
            raise ValueError("Native distribution CPU baseline is not x86-64-v3: " + ", ".join(wrong))
    return {"x86_64_baseline": "x86-64-v3 (no AVX-512)", "cmake_caches_checked": len(caches)}


def build_distributable(npx, target, bundles, config_path, environment, platform):
    env = portable_environment(platform, target, environment)
    target_dir = None
    if portable_target(platform, target):
        cargo = shutil.which("cargo", path=env.get("PATH"))
        if not cargo:
            raise ValueError("Cargo is required to prepare the portable native dependency build")
        manifest = DESKTOP / "src-tauri" / "Cargo.toml"
        metadata = json.loads(subprocess.check_output([
            cargo, "metadata", "--manifest-path", str(manifest), "--format-version", "1", "--no-deps", "--locked",
        ], cwd=DESKTOP, env=env, text=True))
        target_dir = Path(metadata["target_directory"])
        # Restored host-tuned caches must not survive into a distributable build.
        run(cargo, "clean", "--manifest-path", manifest, "--package", "transcribe-cpp-sys", "--release", "--target", target, cwd=DESKTOP, env=env)
    arguments = [npx, "tauri", "build", "--target", target, "--bundles", bundles]
    if config_path is not None:
        arguments.extend(["--config", config_path])
    run(*arguments, cwd=DESKTOP, env=env)
    return verify_portable_caches(target_dir, target) if target_dir is not None else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--build-number", type=int)
    parser.add_argument("--previous-build-number", type=int)
    parser.add_argument("--prepare-only", action="store_true", help="Validate inputs and write the generated release configuration only")
    parser.add_argument("--legacy-sparkle", action="store_true", help="Also prepare the archive/appcast understood by existing native Mac installations")
    parser.add_argument("--homebrew", action="store_true", help="Build a notarized Mac DMG independently of Windows and Tauri updater credentials")
    args = parser.parse_args()
    platform = sys.platform
    try:
        env = os.environ.copy()
        signing = mac_signing() if platform == "darwin" else None
        plan = None
        if signing:
            # Profiles and entitlements enter the bundle before its first signature.
            plan = signing.prepare("production", ROOT / "build/mac-signing/production", env)
            signing.notary_arguments(env)
        config = release_config(
            args.version, platform, os.environ.get("TAURI_UPDATER_PUBLIC_KEY"),
            args.build_number, args.previous_build_number,
            plan["identity"] if plan else os.environ.get("APPLE_SIGNING_IDENTITY") or os.environ.get("PENNEL_SIGN_IDENTITY"),
            os.environ.get("PENNEL_WINDOWS_SIGN_COMMAND"),
            os.environ.get("PENNEL_WINDOWS_CERTIFICATE_THUMBPRINT"),
            homebrew=args.homebrew,
        )
        if plan:
            overlay = json.loads(Path(plan["config"]).read_text())
            config["identifier"] = overlay["identifier"]
            config["bundle"]["macOS"].update(overlay["bundle"]["macOS"])
            # The updater archive must contain the final stapled app; create it below.
            config["bundle"]["createUpdaterArtifacts"] = False
        target = {"darwin": "aarch64-apple-darwin", "win32": "x86_64-pc-windows-msvc", "linux": "x86_64-unknown-linux-gnu"}[platform]
        output = ROOT / "build" / "desktop-release" / args.version / target
        output.mkdir(parents=True, exist_ok=True)
        config_path = output / "tauri.release.conf.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        if args.prepare_only:
            print(f"Release configuration prepared at {config_path}; no build, signing, or publication performed")
            return
        if subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip():
            raise ValueError("Signed candidates require a clean checkout so the source revision identifies exactly what was built")
        if not args.homebrew and not os.environ.get("TAURI_SIGNING_PRIVATE_KEY"):
            raise ValueError("TAURI_SIGNING_PRIVATE_KEY must reference the existing updater signing key")
        if platform == "darwin":
            build_env = signing.build_environment(env, plan["identity"])
        else:
            build_env = env
        npx = shutil.which("npx.cmd" if platform == "win32" else "npx")
        if not npx:
            raise ValueError("Node.js/npm must be available to run the Tauri CLI")
        bundles = {"darwin": "app", "win32": "nsis,msi", "linux": "deb,appimage"}[platform]
        native_baseline = build_distributable(npx, target, bundles, config_path, build_env, platform)
        bundle_dir = DESKTOP / "src-tauri" / "target" / target / "release" / "bundle"
        if platform == "darwin":
            app = bundle_dir / "macos" / "Pennel.app"
            run(sys.executable, ROOT / "scripts" / "verify-desktop-bundle.py", app, "--require-developer-id", "--require-private-keychain")
            dmg = bundle_dir / "dmg" / f"Pennel_{args.version}_aarch64.dmg"
            mac_receipt = signing.finish_distribution(app, dmg, plan, env)
            mac_receipt["revision"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
            (output / "mac-notarization.json").write_text(json.dumps(mac_receipt, indent=2) + "\n")
            if not args.homebrew:
                archive = bundle_dir / "macos" / "Pennel.app.tar.gz"
                run("tar", "-czf", archive, "-C", app.parent, app.name)
                sign_detached(npx, archive, args.version, env)
            if args.legacy_sparkle:
                prepare_sparkle(app, args.version, output)
        elif platform == "win32":
            verify_windows_signature(DESKTOP / "src-tauri" / "target" / target / "release" / "pennel.exe")
        else:
            # Tauri creates the AppImage updater signature. Debian downloads use the same existing key.
            for artifact in (bundle_dir / "deb").glob("*.deb"):
                sign_detached(npx, artifact, args.version, env)
        for extension in ("*.dmg", "*.exe", "*.msi", "*.app.tar.gz", "*.AppImage", "*.deb", "*.sig"):
            for artifact in bundle_dir.rglob(extension):
                if platform == "darwin" and artifact not in ({dmg} if args.homebrew else {dmg, archive, archive.with_name(archive.name + ".sig")}):
                    continue
                if platform == "win32" and artifact.suffix in (".exe", ".msi"):
                    verify_windows_signature(artifact)
                shutil.copy2(artifact, output / artifact.name)
        receipt = {
            "version": args.version, "target": target,
            "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "build_number": args.build_number if platform == "darwin" else None,
            "signed_candidate": True,
            "published": False,
            "hardware_acceptance": "pending",
        }
        if native_baseline is not None:
            receipt["native_cpu_baseline"] = native_baseline
        updater_folder, updater_pattern = {
            "darwin": ("macos", "*.app.tar.gz"), "win32": ("nsis", "*.exe"), "linux": ("appimage", "*.AppImage"),
        }[platform]
        if not args.homebrew:
            updater_artifacts = list((bundle_dir / updater_folder).glob(updater_pattern))
            if len(updater_artifacts) != 1 or not updater_artifacts[0].with_name(updater_artifacts[0].name + ".sig").is_file():
                raise ValueError("Expected exactly one signed updater artifact for the candidate")
            receipt["updater_artifact"] = updater_artifacts[0].name
            receipt["sha256"] = hashlib.sha256(updater_artifacts[0].read_bytes()).hexdigest()
        if plan:
            receipt["mac_notarization"] = "mac-notarization.json"
        receipt["installer_artifacts"] = {
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in output.iterdir()
            if path.suffix in (".dmg", ".exe", ".msi", ".AppImage", ".deb")
        }
        if platform == "linux":
            receipt["signature_scheme"] = "minisign"
            receipt["desktop_acceptance"] = {"x11": "pending", "wayland": "pending"}
        (output / "candidate.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        print(f"Signed candidate files: {output}. Hardware acceptance and publication remain separate steps.")
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Desktop candidate failed: {error}\n")


def verify_windows_signature(path):
    env = os.environ.copy()
    env["PENNEL_VERIFY_SIGN_FILE"] = str(path)
    command = "$s=Get-AuthenticodeSignature -LiteralPath $env:PENNEL_VERIFY_SIGN_FILE; if ($s.Status -ne 'Valid') { Write-Error 'Invalid Authenticode signature'; exit 1 }"
    run("powershell", "-NoProfile", "-NonInteractive", "-Command", command, env=env)


def sign_detached(npx, artifact, version, env):
    signing_env = env.copy()
    key = signing_env.get("TAURI_SIGNING_PRIVATE_KEY", "")
    try:
        key_is_path = Path(key).is_file()
    except OSError:
        key_is_path = False
    if key_is_path:
        signing_env["TAURI_SIGNING_PRIVATE_KEY_PATH"] = key
        signing_env.pop("TAURI_SIGNING_PRIVATE_KEY", None)
    run(npx, "tauri", "signer", "sign", "--app-version", version, artifact, cwd=DESKTOP, env=signing_env)


def prepare_sparkle(app, version, output):
    directory = os.environ.get("PENNEL_SPARKLE_BIN")
    if not directory:
        raise ValueError("PENNEL_SPARKLE_BIN is required for the legacy upgrade; point it to existing Sparkle tools")
    generator = Path(directory) / "generate_appcast"
    if not generator.is_file():
        raise ValueError("PENNEL_SPARKLE_BIN must contain the existing Sparkle generate_appcast tool")
    archive = output / f"Pennel-{version}.zip"
    run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive)
    updates = output / "legacy-sparkle"
    updates.mkdir(exist_ok=True)
    shutil.copy2(archive, updates / archive.name)
    prefix = os.environ.get("PENNEL_RELEASE_DOWNLOAD_BASE", f"https://github.com/pennel-app/pennel/releases/download/v{version}/")
    if not prefix.startswith("https://"):
        raise ValueError("PENNEL_RELEASE_DOWNLOAD_BASE must use HTTPS")
    run(generator, "--account", "pennel", "--download-url-prefix", prefix.rstrip("/") + "/", "--link", "https://pennel.app", "-o", output / "appcast.xml", updates)


if __name__ == "__main__":
    main()
