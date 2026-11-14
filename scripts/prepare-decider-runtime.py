#!/usr/bin/env python3
"""Build Pennel's pinned, text-only native Decider runtime without model acquisition.

Source fetch, compilation and manifest refresh are separate explicit actions. A
refresh after platform signing updates distribution hashes, never the compiled
source identity. This command does not load a model or execute inference.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
NATIVE = ROOT / "core/native/decider"
LOCK = NATIVE / "runtime-lock.json"
DEFAULT_SOURCE = ROOT / "build/review-cloud-local/runtime-source/llama.cpp"
DEFAULT_BUILD_ROOT = ROOT / "build/review-cloud-local/runtime-build"


def digest(value):
    return hashlib.sha256(value).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def run(*arguments, cwd=None, capture=False, env=None):
    return subprocess.run(arguments, cwd=cwd, env=env, check=True,
                          text=True, stdout=subprocess.PIPE if capture else None).stdout


def read_lock(path=LOCK):
    value = json.loads(path.read_text())
    if (value.get("schema_version") != 1 or value.get("architecture") != "arm64"
            or value.get("minimum_macos") != "14.4"
            or value.get("abi") != "pennel-managed-decider-cabi-v1"
            or value["source"]["repository"] != "https://github.com/ggml-org/llama.cpp.git"
            or not re.fullmatch("[0-9a-f]{40}", value["source"]["revision"])
            or not re.fullmatch("[0-9a-f]{40}", value["source"]["ggml_tree"])):
        raise ValueError("Unsupported Decider runtime lock")
    return value


def verify_source(source, lock):
    expected = lock["source"]
    if run("git", "rev-parse", "HEAD", cwd=source, capture=True).strip() != expected["revision"]:
        raise ValueError("Native source revision changed")
    if run("git", "status", "--porcelain", "--untracked-files=all", cwd=source, capture=True).strip():
        raise ValueError("Native source is dirty")
    remote = run("git", "remote", "get-url", "origin", cwd=source, capture=True).strip()
    if remote != expected["repository"]:
        raise ValueError("Unexpected native source origin")
    tree = run("git", "rev-parse", "HEAD:ggml", cwd=source, capture=True).strip()
    if tree != expected["ggml_tree"] or not (source / "ggml/CMakeLists.txt").is_file():
        raise ValueError("Bundled GGML source changed or is absent")


def fetch_source(source, lock):
    source.mkdir(parents=True, exist_ok=True)
    if not (source / ".git").is_dir():
        if any(source.iterdir()):
            raise ValueError("Refusing to overwrite an existing source directory")
        run("git", "init", "-q", str(source))
        run("git", "remote", "add", "origin", lock["source"]["repository"], cwd=source)
    run("git", "fetch", "--depth=1", "origin", lock["source"]["revision"], cwd=source)
    if run("git", "status", "--porcelain", cwd=source, capture=True).strip():
        raise ValueError("Refusing to replace dirty native source")
    run("git", "checkout", "-q", "--detach", lock["source"]["revision"], cwd=source)
    verify_source(source, lock)


def build_identity(lock):
    # These bytes are compiled into od_build_signature. Runtime/SDK/binary
    # provenance remains separately reported; filenames are never identity.
    inputs = {"lock": lock,
              "native_source_sha256": digest((NATIVE / "owned_decider.cpp").read_bytes()),
              "build_definition_sha256": digest((NATIVE / "CMakeLists.txt").read_bytes()),
              "exported_abi_sha256": digest((NATIVE / "exports.txt").read_bytes())}
    return digest(canonical(inputs)), inputs


def check_binary(binary, lock):
    architectures = run("lipo", "-archs", str(binary), capture=True).strip()
    if architectures != "arm64":
        raise ValueError("The native runtime has an unexpected architecture")
    build = run("vtool", "-show-build", str(binary), capture=True)
    if not re.search(r"\bminos\s+14\.4\b", build):
        raise ValueError("The native runtime was not built for macOS14.4")
    dependencies = run("otool", "-L", str(binary), capture=True).splitlines()[1:]
    paths = [row.strip().split(" (", 1)[0] for row in dependencies]
    if any(not (path.startswith(("/usr/lib/", "/System/Library/")) or path == "@rpath/libpennel_decider.dylib")
           for path in paths):
        raise ValueError("A machine-specific native dependency escaped static linking")
    return {"architectures": architectures, "minimum_macos": lock["minimum_macos"],
            "dependencies": paths}


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False) as temporary:
        json.dump(value, temporary, indent=2)
        temporary.write("\n")
        temporary_path = Path(temporary.name)
    temporary_path.replace(path)


def refresh_manifest(output, lock, signature, inputs):
    binary = output / "libpennel_decider.dylib"
    binary_metadata = check_binary(binary, lock)
    manifest = {"schema_version": 1, "runtime_id": lock["runtime_id"], "abi": lock["abi"],
                "build_signature": signature, "source_inputs": inputs,
                "component": {"file": binary.name, "sha256": digest(binary.read_bytes()),
                              "bytes": binary.stat().st_size},
                "platform": "macos-aarch64", **binary_metadata,
                "model_downloaded": False, "model_loaded": False, "inference_executed": False,
                "compatibility": "deployment-target build and dependency inspection; not old-OS device acceptance"}
    write_json(output / "manifest.json", manifest)
    return manifest


def build(source, build_root, output, lock, jobs):
    if platform.system() != "Darwin" or platform.machine() not in ("arm64", "aarch64"):
        raise ValueError("This runtime build currently requires Apple Silicon macOS")
    verify_source(source, lock)
    signature, inputs = build_identity(lock)
    build_dir = build_root / signature[:20]
    build_dir.mkdir(parents=True, exist_ok=True)
    cmake = [str(ROOT / "scripts/with-toolchain.sh"), "cmake"]
    flags = {**lock["cmake_options"],
             "CMAKE_OSX_DEPLOYMENT_TARGET": lock["minimum_macos"],
             "CMAKE_OSX_ARCHITECTURES": lock["architecture"],
             "PENNEL_LLAMA_SOURCE_DIR": str(source),
             "PENNEL_DECIDER_BUILD_SIGNATURE": signature}
    environment = os.environ.copy()
    for variable in ("CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "CMAKE_ARGS"):
        environment.pop(variable, None)
    environment["MACOSX_DEPLOYMENT_TARGET"] = lock["minimum_macos"]
    run(*cmake, "-S", str(NATIVE), "-B", str(build_dir),
        *[f"-D{key}={value}" for key, value in sorted(flags.items())], env=environment)
    run(*cmake, "--build", str(build_dir), "--target", "pennel_decider", "--parallel", str(jobs), env=environment)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(build_dir / "libpennel_decider.dylib", output / "libpennel_decider.dylib")
    shutil.copy2(source / "LICENSE", output / "LICENSE.llama.cpp")
    # This pinned repository's root MIT notice names the ggml authors and
    # covers its bundled ggml subtree; that subtree has no separate license file.
    shutil.copy2(source / "LICENSE", output / "LICENSE.ggml")
    manifest = refresh_manifest(output, lock, signature, inputs)
    write_json(build_dir / "provenance.json", {
        "build_signature": signature, "inputs": inputs,
        "compiler": run("xcrun", "clang++", "--version", capture=True).splitlines()[0],
        "sdk_version": run("xcrun", "--show-sdk-version", capture=True).strip(),
        "source_head": run("git", "rev-parse", "HEAD", cwd=source, capture=True).strip(),
        "binary_sha256": manifest["component"]["sha256"],
        "inference_executed": False})
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--build-root", type=Path, default=DEFAULT_BUILD_ROOT)
    parser.add_argument("--output", type=Path, default=ROOT / "build/review-cloud-local/runtime-bundle")
    parser.add_argument("--fetch-source", action="store_true")
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--refresh-manifest", action="store_true", help="Rehash after platform signing; never edits a binary")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if not 1 <= args.jobs <= 16 or args.build and args.refresh_manifest:
        parser.error("Choose valid bounded build actions")
    lock = read_lock()
    source = args.source_dir.resolve()
    if args.fetch_source:
        fetch_source(source, lock)
    signature, inputs = build_identity(lock)
    if args.build:
        manifest = build(source, args.build_root.resolve(), args.output.resolve(), lock, args.jobs)
        print(json.dumps({"built": True, "build_signature": signature, "component": manifest["component"],
                          "model_loads": 0, "inference_requests": 0}))
    elif args.refresh_manifest:
        previous = json.loads((args.output / "manifest.json").read_text())
        if previous["build_signature"] != signature:
            raise ValueError("The runtime source identity changed; rebuild rather than relabel")
        manifest = refresh_manifest(args.output, lock, signature, inputs)
        print(json.dumps({"refreshed": True, "component": manifest["component"]}))
    else:
        print(json.dumps({"lock_valid": True, "build_signature": signature,
                          "source_fetch_requested": args.fetch_source,
                          "model_loads": 0, "inference_requests": 0}))


if __name__ == "__main__":
    main()
