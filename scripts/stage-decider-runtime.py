#!/usr/bin/env python3
"""Prepare a signed packaging copy of the pinned Decider runtime, never model weights.

The ordinary command only validates the staging directory. --stage is explicit;
the Tauri before-bundle hook supplies it after the caller authorizes packaging.
"""
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
import tempfile
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
TARGET = "aarch64-apple-darwin"
OWNER = "pennel-decider-runtime-stage-v1"
GENERATED = {"libpennel_decider.dylib", "manifest.json", "LICENSE.llama.cpp", "LICENSE.ggml"}


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def paths():
    resources = ROOT / "apps/desktop/src-tauri/resources"
    return resources / "decider-runtime", resources / ".decider-runtime-stage-v1.json", ROOT / "build/decider-runtime-staging"


def write_json(path, value):
    with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False) as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        temporary = Path(stream.name)
    temporary.replace(path)


def validate_stage(stage, marker):
    if stage.is_symlink() or stage.parent.is_symlink() or marker.is_symlink() or not stage.is_dir():
        raise ValueError("Runtime staging requires its regular owned resource directory")
    entries = {path.name for path in stage.iterdir()}
    if "README.md" not in entries or entries - GENERATED - {"README.md"}:
        raise ValueError("Unknown files in runtime staging are preserved; inspect before packaging")
    for path in stage.iterdir():
        if path.is_symlink() or not path.is_file():
            raise ValueError("Runtime staging contains an unsupported path")
    generated = entries - {"README.md"}
    if not marker.exists():
        if generated:
            raise ValueError("Unowned generated runtime resources are preserved")
        return {}
    if not marker.is_file():
        raise ValueError("Runtime staging ownership is unavailable")
    record = json.loads(marker.read_text())
    if record.get("schema_version") != 1 or record.get("owner") != OWNER or not isinstance(record.get("files"), dict):
        raise ValueError("Runtime staging ownership is unknown")
    if set(record["files"]) != generated:
        raise ValueError("Runtime staging changed outside its owned preparation")
    for name, sha in record["files"].items():
        if name not in GENERATED or not re.fullmatch("[0-9a-f]{64}", sha) or digest(stage / name) != sha:
            raise ValueError("Runtime staging content changed; preserve it for inspection")
    return record["files"]


def verify_runtime(bundle, prepare):
    if bundle.is_symlink() or not bundle.is_dir() or {path.name for path in bundle.iterdir()} != GENERATED:
        raise ValueError("The prepared runtime bundle has unknown or missing files")
    for path in bundle.iterdir():
        if path.is_symlink() or not path.is_file():
            raise ValueError("The prepared runtime must contain regular files")
    manifest = json.loads((bundle / "manifest.json").read_text())
    signature, inputs = prepare.build_identity(prepare.read_lock())
    component = manifest.get("component", {})
    if (manifest.get("build_signature") != signature or manifest.get("source_inputs") != inputs
            or component.get("file") != "libpennel_decider.dylib"
            or component.get("sha256") != digest(bundle / component["file"])
            or component.get("bytes") != (bundle / component["file"]).stat().st_size):
        raise ValueError("The prepared runtime differs from its pinned source or distribution manifest")
    prepare.check_binary(bundle / component["file"], prepare.read_lock())
    return manifest


def selected_identity(environment):
    requested = environment.get("APPLE_SIGNING_IDENTITY") or environment.get("PENNEL_SIGN_IDENTITY")
    if not requested:
        raise ValueError("Select the app's signing identity before staging its native runtime")
    if requested == "-":
        isolated_ci = environment.get("CI") == "true" and (environment.get("GITHUB_ACTIONS") == "true" or environment.get("PENNEL_ALLOW_ADHOC_PREVIEW") == "1")
        if not isolated_ci:
            raise ValueError("Ad-hoc runtime signing is restricted to the existing isolated CI policy")
        return {"identity": "-", "adhoc": True, "team": None}
    signing = module("pennel_runtime_mac_signing", "mac_signing.py")
    available = signing.identities(signing.command(["security", "find-identity", "-v", "-p", "codesigning"]))
    identity, _ = signing.choose_identity(available, requested, preview=True)
    developer_id = signing.validate_certificate(signing.certificate_for_identity(identity), preview=True)
    return {"identity": identity, "adhoc": False, "team": signing.PERSONAL_TEAM, "developer_id": developer_id}


def tool(arguments, environment):
    result = subprocess.run([str(value) for value in arguments], env=environment, capture_output=True, text=True)
    if result.returncode:
        raise ValueError(f"Native runtime {Path(arguments[0]).name} verification failed")
    return result.stdout + result.stderr


def verify_signature(binary, identity, environment):
    tool(["codesign", "--verify", "--strict", binary], environment)
    metadata = tool(["codesign", "-d", "--verbose=4", binary], environment)
    if identity["adhoc"]:
        if "Signature=adhoc" not in metadata:
            raise ValueError("The CI runtime signature does not match its selected policy")
    elif "TeamIdentifier=B538Q2K76C" not in metadata:
        raise ValueError("The runtime must be signed by the personal Apple team")


def sign_copy(binary, identity, environment):
    arguments = ["codesign", "--force", "--sign", identity["identity"], "--options", "runtime"]
    if not identity["adhoc"]:
        arguments.append("--timestamp")
    tool([*arguments, binary], environment)
    verify_signature(binary, identity, environment)


def prepare_command(arguments, environment, *, guarded=False):
    command = [sys.executable, str(ROOT / "scripts/prepare-decider-runtime.py"), *map(str, arguments)]
    guard = Path.home() / ".local/bin/worktree-storage"
    if guarded and guard.is_file():
        command = [str(guard), "run", "--", *command]
    subprocess.run(command, env=environment, check=True)


def replace_owned(stage, marker, cache, previous, fresh=None):
    # Retain every replaced generated byte. Never mutate or sign a potentially mapped source inode.
    if previous:
        retained = cache / "retained" / uuid.uuid4().hex
        retained.mkdir(parents=True)
        for name in previous:
            shutil.copy2(stage / name, retained / name)
        shutil.copy2(marker, retained / "stage-owner.json")
    for name in GENERATED:
        if fresh is not None:
            (fresh / name).replace(stage / name)
        elif name in previous:
            (stage / name).unlink()
    files = {name: digest(stage / name) for name in GENERATED} if fresh is not None else {}
    write_json(marker, {"schema_version": 1, "owner": OWNER, "files": files})


def stage_runtime(target, *, source_dir=None, build_root=None, prepared_runtime=None, fetch=False, build=False, environment=None):
    environment = dict(os.environ if environment is None else environment)
    stage, marker, cache = paths()
    previous = validate_stage(stage, marker)
    if target != TARGET:
        cache.mkdir(parents=True, exist_ok=True)
        replace_owned(stage, marker, cache, previous)
        return {"staged": False, "reason": "unsupported_target", "target": target, "model_loads": 0, "inference_requests": 0}
    if sys.platform != "darwin":
        raise ValueError("The Apple Silicon runtime must be packaged on macOS")
    identity = selected_identity(environment)
    prepare = module("pennel_runtime_prepare", "prepare-decider-runtime.py")
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="packaging-", dir=cache) as temporary:
        temporary = Path(temporary)
        if prepared_runtime is not None:
            source_bundle = Path(prepared_runtime)
        else:
            if not build:
                raise ValueError("An explicit pinned native build or prepared runtime is required")
            source_bundle = temporary / "compiled"
            arguments = ["--build", "--output", source_bundle, "--source-dir", source_dir or prepare.DEFAULT_SOURCE,
                         "--build-root", build_root or prepare.DEFAULT_BUILD_ROOT]
            if fetch:
                arguments.append("--fetch-source")
            prepare_command(arguments, environment, guarded=True)
        verify_runtime(source_bundle, prepare)
        source_hashes = {name: digest(source_bundle / name) for name in GENERATED}
        fresh = temporary / "signed-copy"
        fresh.mkdir()
        for name in GENERATED:
            shutil.copy2(source_bundle / name, fresh / name)
        sign_copy(fresh / "libpennel_decider.dylib", identity, environment)
        prepare_command(["--refresh-manifest", "--output", fresh], environment)
        signed_manifest = verify_runtime(fresh, prepare)
        if {name: digest(source_bundle / name) for name in GENERATED} != source_hashes:
            raise ValueError("The source runtime changed while its packaging copy was prepared")
        replace_owned(stage, marker, cache, previous, fresh)
    return {"staged": True, "target": target, "component": signed_manifest["component"],
            "build_signature": signed_manifest["build_signature"], "signing": identity,
            "source_unchanged": True, "model_loads": 0, "inference_requests": 0}


def verify_bundle(app):
    prepare = module("pennel_runtime_prepare", "prepare-decider-runtime.py")
    bundle = Path(app) / "Contents/Resources/decider-runtime"
    # README is a shipped placeholder, outside the generated runtime's manifest.
    files = {path.name for path in bundle.iterdir() if path.name != "README.md"}
    if files != GENERATED:
        raise ValueError("The copied app runtime has missing or unknown components")
    manifest = json.loads((bundle / "manifest.json").read_text())
    binary = bundle / "libpennel_decider.dylib"
    signature, inputs = prepare.build_identity(prepare.read_lock())
    if (manifest.get("build_signature") != signature or manifest.get("source_inputs") != inputs
            or binary.is_symlink() or not binary.is_file() or manifest["component"]["sha256"] != digest(binary)
            or manifest["component"]["bytes"] != binary.stat().st_size):
        raise ValueError("The copied app runtime no longer matches its signed manifest")
    stage, _, _ = paths()
    if digest(stage / "libpennel_decider.dylib") != digest(binary) or digest(stage / "manifest.json") != digest(bundle / "manifest.json"):
        raise ValueError("The bundler changed the signed staged runtime")
    return {"copied_bytes_match_stage": True, "component": manifest["component"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", action="store_true")
    parser.add_argument("--target", default=os.environ.get("TAURI_ENV_TARGET_TRIPLE", ""))
    parser.add_argument("--source-dir", type=Path)
    parser.add_argument("--build-root", type=Path)
    parser.add_argument("--prepared-runtime", type=Path, default=os.environ.get("PENNEL_DECIDER_PREPARED_RUNTIME"))
    parser.add_argument("--fetch-source", action="store_true")
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--verify-bundle", type=Path)
    args = parser.parse_args()
    if args.verify_bundle:
        value = verify_bundle(args.verify_bundle)
    elif args.stage:
        if not re.fullmatch("[A-Za-z0-9][A-Za-z0-9_-]{1,99}", args.target):
            parser.error("An explicit desktop target triple is required")
        value = stage_runtime(args.target, source_dir=args.source_dir, build_root=args.build_root,
                              prepared_runtime=args.prepared_runtime, fetch=args.fetch_source, build=args.build)
    else:
        stage, marker, _ = paths()
        value = {"stage_valid": True, "generated_files": len(validate_stage(stage, marker)), "actions_executed": False}
    print(json.dumps(value))


if __name__ == "__main__":
    main()
