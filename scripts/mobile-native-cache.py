#!/usr/bin/env python3
"""Validate and retain complete iPhoneOS release inputs for Xcode Cloud."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import uuid
import xml.etree.ElementTree as ET


APPLE_TEAM = "B538Q2K76C"
APP_STORE_TEAM = "4dc89f53-aed3-450a-9094-b1251bf1dbe3"
BUNDLE_ID = "app.pennel.ios.preview"
APP_GROUP = "group.app.pennel.preview"
EXPECTED_TARGETS = {"Pennel": BUNDLE_ID, "PennelKeyboard": BUNDLE_ID + ".keyboard",
                    "ExpoWidgetsTarget": BUNDLE_ID + ".widgets"}
ARTIFACTS = Path("apps/mobile/artifacts/ios")
SNAPSHOT = Path("apps/mobile/ios/pennel-testflight.json")
SNAPSHOT_ORIGIN = Path("build/testflight/cloud-snapshot-origin.json")
GENERATED_SEAL = Path("build/testflight/cloud-generated-seal.json")
POD_MUTABLE_FILES = {"apps/mobile/ios/Pennel.xcodeproj/project.pbxproj",
                     "apps/mobile/ios/Pennel.xcworkspace/contents.xcworkspacedata"}
BUILD_MUTABLE_FILES = {"apps/mobile/ios/Pennel.xcodeproj/project.pbxproj",
                       *(f"apps/mobile/ios/{name}/Info.plist" for name in EXPECTED_TARGETS)}
NATIVE_SCRIPTS = ("scripts/build-mobile-core-ios.sh", "scripts/build-keyboard-core-ios.sh",
                  "scripts/with-toolchain.sh", "scripts/mobile-native-cache.py")
HEX = re.compile(r"^[0-9a-f]{64}$")
SHA = re.compile(r"^[0-9a-f]{40}$")


def command(args, root=None):
    return subprocess.check_output(args, cwd=root, text=True, stderr=subprocess.PIPE).strip()


def digest(file):
    result = hashlib.sha256()
    with file.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_json(file, value):
    file.parent.mkdir(parents=True, exist_ok=True)
    temporary = file.with_name(file.name + ".tmp-" + uuid.uuid4().hex)
    temporary.write_bytes(json_bytes(value))
    os.replace(temporary, file)


def safe_relative(value):
    if not isinstance(value, str) or not value or "\\" in value or "\0" in value:
        raise ValueError("Unsafe relative artifact path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in value.split("/")):
        raise ValueError("Unsafe relative artifact path")
    return Path(*path.parts)


def no_symlink_path(file):
    for parent in (file, *file.parents):
        if parent.is_symlink():
            raise ValueError("Native release paths must not contain symbolic links")


def regular_files(directory):
    no_symlink_path(directory)
    if not directory.is_dir():
        raise ValueError("Native artifact directory is missing")
    files = {}
    for current, directories, names in os.walk(directory, followlinks=False):
        for name in directories + names:
            file = Path(current) / name
            mode = file.lstat().st_mode
            if stat.S_ISLNK(mode):
                raise ValueError("Native release bundles must not contain symbolic links")
            if not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
                raise ValueError("Native release bundles require regular files and directories")
            if stat.S_ISREG(mode):
                files[file.relative_to(directory).as_posix()] = {"sha256": digest(file), "bytes": file.stat().st_size}
    return dict(sorted(files.items()))


def toolchain_identity(root):
    return {
        "rustc": command(["rustc", "-vV"], root),
        "cargo": command(["cargo", "-Vv"], root),
        "xcode": command(["xcodebuild", "-version"], root),
        "iphoneos_sdk": command(["xcrun", "--sdk", "iphoneos", "--show-sdk-version"], root),
        "iphoneos_sdk_build": command(["xcrun", "--sdk", "iphoneos", "--show-sdk-build-version"], root),
    }


def fingerprint(root, toolchain=None, environment=None):
    root = Path(root)
    inputs = {}
    for source in (root / "core", root / ".cargo"):
        if not source.is_dir():
            continue
        no_symlink_path(source)
        for current, directories, files in os.walk(source, followlinks=False):
            directories[:] = sorted(name for name in directories if name not in ("target", ".git", "__pycache__"))
            for name in directories + files:
                file = Path(current) / name
                if file.is_symlink():
                    raise ValueError("Native inputs must not contain symbolic links")
            for name in sorted(files):
                if name == ".DS_Store":
                    continue
                file = Path(current) / name
                if not stat.S_ISREG(file.lstat().st_mode):
                    raise ValueError("Native inputs require regular files")
                inputs[file.relative_to(root).as_posix()] = digest(file)
    for relative in NATIVE_SCRIPTS:
        file = root / relative
        no_symlink_path(file)
        if not file.is_file():
            raise ValueError("Native preparation input is missing: " + relative)
        inputs[relative] = digest(file)
    lock = (root / "core/Cargo.lock").read_text()
    uniffi = re.search(r'\[\[package\]\]\s+name = "uniffi"\s+version = "([^"]+)"', lock)
    if not uniffi:
        raise ValueError("The locked UniFFI version is missing")
    env = os.environ if environment is None else environment
    flags = {name: value for name, value in env.items() if name in (
        "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTFLAGS", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET",
    ) or name.startswith(("CARGO_PROFILE_RELEASE_", "CARGO_TARGET_AARCH64_APPLE_IOS_"))}
    context = {
        "schema": 1, "inputs": dict(sorted(inputs.items())),
        "toolchain": toolchain if toolchain is not None else toolchain_identity(root),
        "uniffi": uniffi.group(1), "variant": "preview", "deployment_target": "26.0",
        "targets": ["aarch64-apple-ios"], "core_features": [], "keyboard_features": ["ffi"],
        "profile": "release", "effective_flags_sha256": hashlib.sha256(json_bytes(flags)).hexdigest(),
    }
    return {"key": hashlib.sha256(json_bytes(context)).hexdigest(), "context": context}


def validate_context(value):
    if not isinstance(value, dict) or not HEX.fullmatch(value.get("key", "")):
        raise ValueError("Invalid native cache key")
    if hashlib.sha256(json_bytes(value.get("context"))).hexdigest() != value["key"]:
        raise ValueError("Native cache context does not match its key")


def architectures(file):
    return command(["xcrun", "lipo", "-archs", str(file)]).split()


def validate_bundle(directory, inspector=None):
    files = regular_files(directory)
    allowed = {"PennelCoreFFI.xcframework", "PennelTypingFFI.xcframework", "PennelCore", "manifest.json"}
    if not files or any(PurePosixPath(name).parts[0] not in allowed for name in files):
        raise ValueError("Native artifact bundle contains unexpected files")
    inspector = inspector or architectures
    symbol_paths = []
    for name, library, header in (
        ("PennelCoreFFI", "libpennel_core.a", "pennel_coreFFI.h"),
        ("PennelTypingFFI", "libpennel_typing.a", "PennelTypingFFI.h"),
    ):
        framework = directory / (name + ".xcframework")
        with (framework / "Info.plist").open("rb") as file:
            description = plistlib.load(file)
        slices = description.get("AvailableLibraries", [])
        if len(slices) != 1:
            raise ValueError("The archive cache requires one iPhoneOS device slice")
        item = slices[0]
        if (item.get("SupportedPlatform") != "ios" or item.get("SupportedPlatformVariant")
                or item.get("SupportedArchitectures") != ["arm64"]):
            raise ValueError("The archive cache requires an arm64 iPhoneOS device slice")
        slice_root = framework / safe_relative(item.get("LibraryIdentifier"))
        library_file = slice_root / safe_relative(item.get("LibraryPath"))
        if library_file.name != library:
            raise ValueError("Unexpected native static library")
        with library_file.open("rb") as file:
            if file.read(8) != b"!<arch>\n":
                raise ValueError("Native static archives must contain their objects")
        if inspector(library_file) != ["arm64"]:
            raise ValueError("Native library architecture must be arm64")
        headers = slice_root / safe_relative(item.get("HeadersPath"))
        for expected in (header, "module.modulemap"):
            if not (headers / expected).is_file() or not (headers / expected).stat().st_size:
                raise ValueError("Matching native headers/modulemap are missing")
        for field in ("DebugSymbolsPath", "BitcodeSymbolMapsPath"):
            if field in item:
                symbols = slice_root / safe_relative(item[field])
                if not symbols.exists() or not any(symbols.rglob("*")):
                    raise ValueError("Referenced native symbol dependencies are missing")
                symbol_paths.append(symbols.relative_to(directory).as_posix())
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest.get("schema") != 1 or manifest.get("targets") != ["aarch64-apple-ios"] or manifest.get("features") != []:
        raise ValueError("Native artifact mode or features do not match an archive")
    if manifest.get("bindings_sha256") != digest(directory / "PennelCore/pennel_core.swift"):
        raise ValueError("Native bindings do not match the artifact manifest")
    return files, symbol_paths


def validate_entry(entry, context, inspector=None):
    no_symlink_path(entry)
    if not entry.is_dir() or sorted(path.name for path in entry.iterdir()) != ["artifacts", "cache-manifest.json"]:
        raise ValueError("Incomplete native cache entry")
    manifest_file = entry / "cache-manifest.json"
    if manifest_file.is_symlink() or not manifest_file.is_file():
        raise ValueError("Native cache manifest must be a regular file")
    manifest = json.loads(manifest_file.read_text())
    if manifest.get("schema") != 1 or manifest.get("key") != context["key"] or manifest.get("context") != context["context"]:
        raise ValueError("Native cache key or toolchain mismatch")
    if not SHA.fullmatch(manifest.get("producer_source_sha", "")):
        raise ValueError("Native cache producer provenance is missing")
    declared = manifest.get("files")
    if not isinstance(declared, dict):
        raise ValueError("Native cache checksums are missing")
    for relative, properties in declared.items():
        safe_relative(relative)
        if (not isinstance(properties, dict) or not HEX.fullmatch(properties.get("sha256", ""))
                or not isinstance(properties.get("bytes"), int) or properties["bytes"] < 0):
            raise ValueError("Invalid native file checksum")
    actual, symbols = validate_bundle(entry / "artifacts", inspector)
    if actual != declared or symbols != manifest.get("symbol_paths"):
        raise ValueError("Native cache files or symbols do not match their checksums")
    return manifest


def restore(root, cache_root, context, inspector=None):
    validate_context(context)
    entry = Path(cache_root) / context["key"]
    try:
        manifest = validate_entry(entry, context, inspector)
    except (OSError, ValueError, KeyError, TypeError, plistlib.InvalidFileException, subprocess.SubprocessError):
        return {"hit": False, "key": context["key"], "reason": "missing-or-invalid"}
    destination = Path(root) / ARTIFACTS
    no_symlink_path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_dir() and regular_files(destination) == manifest["files"]:
        return {"hit": True, "key": context["key"], "bytes": sum(file["bytes"] for file in manifest["files"].values())}
    staging = Path(tempfile.mkdtemp(prefix=".native-restore-", dir=destination.parent))
    retained = None
    try:
        shutil.copytree(entry / "artifacts", staging, dirs_exist_ok=True)
        files, unused = validate_bundle(staging, inspector)
        if files != manifest["files"]:
            raise ValueError("Native release inputs changed while restoring")
        if destination.exists():
            retained = destination.with_name(destination.name + ".retained-" + uuid.uuid4().hex)
            destination.rename(retained)
        try:
            staging.rename(destination)
        except OSError:
            if retained is not None:
                retained.rename(destination)
            raise
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return {"hit": True, "key": context["key"], "bytes": sum(file["bytes"] for file in manifest["files"].values())}


def store(root, cache_root, context, source_sha, inspector=None):
    validate_context(context)
    if not SHA.fullmatch(source_sha):
        raise ValueError("A complete native producer source SHA is required")
    artifacts = Path(root) / ARTIFACTS
    files, symbols = validate_bundle(artifacts, inspector)
    cache_root = Path(cache_root)
    no_symlink_path(cache_root)
    cache_root.mkdir(parents=True, exist_ok=True)
    entry = cache_root / context["key"]
    no_symlink_path(entry)
    if entry.exists():
        try:
            validate_entry(entry, context, inspector)
            return {"stored": False, "key": context["key"], "reason": "existing-valid-entry"}
        except (OSError, ValueError, KeyError, TypeError, plistlib.InvalidFileException, subprocess.SubprocessError):
            pass
    staging = Path(tempfile.mkdtemp(prefix=".native-store-", dir=cache_root))
    try:
        shutil.copytree(artifacts, staging / "artifacts")
        write_json(staging / "cache-manifest.json", {
            "schema": 1, **context, "producer_source_sha": source_sha,
            "files": files, "symbol_paths": symbols,
            "retention": "Retain release inputs, referenced symbols, and consuming archives/dSYMs; never debug-cache evictable.",
        })
        validate_entry(staging, context, inspector)
        if entry.exists():
            entry.rename(entry.with_name(entry.name + ".retained-" + uuid.uuid4().hex))
        staging.rename(entry)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return {"stored": True, "key": context["key"], "bytes": sum(file["bytes"] for file in files.values())}


def validate_cloud_environment(environment):
    expected = {"CI_XCODE_CLOUD": "TRUE", "CI_TEAM_ID": APP_STORE_TEAM, "CI_BUNDLE_ID": BUNDLE_ID,
                "CI_XCODE_SCHEME": "Pennel", "CI_XCODEBUILD_ACTION": "archive",
                "PENNEL_APPLE_DECLARATIONS_CONFIRMED": "1"}
    for name, value in expected.items():
        if environment.get(name) != value:
            raise ValueError("Unexpected Xcode Cloud archive environment: " + name)
    if environment.get("CI_BRANCH", "codex/ios-testflight") != "codex/ios-testflight":
        raise ValueError("Xcode Cloud must build the selected TestFlight snapshot branch")


def validate_snapshot(root, pristine=True, allow_pod_changes=False):
    root = Path(root)
    manifest_file = root / SNAPSHOT
    no_symlink_path(manifest_file)
    manifest = json.loads(manifest_file.read_text())
    if (manifest.get("schema") != 1 or manifest.get("variant") != "preview"
            or manifest.get("apple_team_id") != APPLE_TEAM or manifest.get("bundle_identifier") != BUNDLE_ID
            or not SHA.fullmatch(manifest.get("source_sha", ""))
            or not SHA.fullmatch(manifest.get("source_tree_sha", ""))):
        raise ValueError("Invalid Preview snapshot identity")
    validation = manifest.get("validation", {})
    checks = validation.get("checks", [])
    if (validation.get("schema") != 1 or validation.get("source_sha") != manifest["source_sha"]
            or validation.get("status") != "success" or not checks
            or any(check.get("status") != "success" or not check.get("name") for check in checks)):
        raise ValueError("The exact selected source has no successful validation receipt")
    try:
        tree = command(["git", "rev-parse", manifest["source_sha"] + "^{tree}"], root)
    except subprocess.CalledProcessError as error:
        raise ValueError("The selected source commit must be available in the Cloud clone") from error
    if tree != manifest["source_tree_sha"]:
        raise ValueError("Selected source tree provenance does not match")
    try:
        command(["git", "diff", "--quiet", manifest["source_sha"], "--", ".", ":(exclude)apps/mobile/ios"], root)
    except subprocess.CalledProcessError as error:
        raise ValueError("The snapshot changes source files outside the generated iOS project") from error
    seal = None
    if not pristine and not allow_pod_changes and (root / GENERATED_SEAL).exists():
        seal = validated_seal(root, manifest)
    for section in ("inputs", "generated_files"):
        entries = manifest.get(section)
        if not isinstance(entries, dict) or not entries:
            raise ValueError("Snapshot input/generated checksums are missing")
        if section == "generated_files" and seal is not None:
            entries = seal["generated_files"]
        for relative, expected in entries.items():
            file = root / safe_relative(relative)
            no_symlink_path(file)
            if not HEX.fullmatch(expected):
                raise ValueError("Invalid snapshot checksum")
            if section == "generated_files" and allow_pod_changes and relative in POD_MUTABLE_FILES:
                continue
            if not file.is_file() or digest(file) != expected:
                if section == "generated_files" and seal is not None:
                    raise ValueError("A sealed generated file changed: " + relative)
                raise ValueError("Snapshot file identity does not match: " + relative)
    if seal is not None and pod_integration_files(root) != seal["pod_files"]:
        raise ValueError("A sealed Pod integration file changed")
    return manifest


def attest_snapshot(root):
    root = Path(root)
    manifest = validate_snapshot(root)
    app_info = (root / "apps/mobile/ios/Pennel/Info.plist").read_bytes()
    source_version = plistlib.loads(app_info).get("CFBundleShortVersionString")
    if not isinstance(source_version, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){1,2}", source_version):
        raise ValueError("The original source marketing version must be explicit")
    origin = {"schema": 1, "snapshot_sha256": digest(root / SNAPSHOT),
              "source_sha": manifest["source_sha"], "source_tree_sha": manifest["source_tree_sha"],
              "source_version": source_version, "app_info_b64": base64.b64encode(app_info).decode("ascii")}
    write_json(root / SNAPSHOT_ORIGIN, origin)
    return manifest


def validated_origin(root, manifest):
    file = root / SNAPSHOT_ORIGIN
    no_symlink_path(file)
    if not file.is_file():
        raise ValueError("The pristine source attestation is required before installing Pods")
    origin = json.loads(file.read_text())
    if (origin.get("schema") != 1 or origin.get("snapshot_sha256") != digest(root / SNAPSHOT)
            or origin.get("source_sha") != manifest["source_sha"]
            or origin.get("source_tree_sha") != manifest["source_tree_sha"]):
        raise ValueError("The pristine source attestation does not match the snapshot")
    app_info = base64.b64decode(origin.get("app_info_b64", ""), validate=True)
    if hashlib.sha256(app_info).hexdigest() != manifest["generated_files"].get("apps/mobile/ios/Pennel/Info.plist"):
        raise ValueError("The original app Info.plist is not attested by the snapshot")
    version = plistlib.loads(app_info).get("CFBundleShortVersionString")
    if version != origin.get("source_version") or not isinstance(version, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){1,2}", version):
        raise ValueError("The original source marketing version is invalid")
    return origin


def pod_integration_files(root):
    canonical_lock = root / "apps/mobile/Podfile.lock"
    installed_lock = root / "apps/mobile/ios/Pods/Manifest.lock"
    generated_lock = root / "apps/mobile/ios/Podfile.lock"
    for file in (canonical_lock, installed_lock, generated_lock):
        no_symlink_path(file)
    if not canonical_lock.is_file() or not installed_lock.is_file() or not generated_lock.is_file():
        raise ValueError("The frozen CocoaPods installation must be complete before sealing")
    if not (digest(canonical_lock) == digest(installed_lock) == digest(generated_lock)):
        raise ValueError("The frozen CocoaPods lock identity changed")
    files = {installed_lock.relative_to(root).as_posix(): digest(installed_lock)}
    for relative in ("apps/mobile/ios/Pods/Pods.xcodeproj", "apps/mobile/ios/Pods/Target Support Files"):
        directory = root / relative
        for name, value in regular_files(directory).items():
            files[relative + "/" + name] = value["sha256"]
    return dict(sorted(files.items()))


def validated_seal(root, manifest):
    file = root / GENERATED_SEAL
    no_symlink_path(file)
    if not file.is_file():
        raise ValueError("The validated post-pod seal is required before archiving")
    seal = json.loads(file.read_text())
    origin = validated_origin(root, manifest)
    if (seal.get("schema") != 1 or seal.get("snapshot_sha256") != digest(root / SNAPSHOT)
            or seal.get("origin_sha256") != digest(root / SNAPSHOT_ORIGIN)
            or seal.get("source_sha") != manifest["source_sha"]
            or seal.get("source_version") != origin["source_version"]
            or seal.get("phase") not in ("pods-installed", "build-prepared")
            or not isinstance(seal.get("generated_files"), dict)
            or set(seal["generated_files"]) != set(manifest["generated_files"])
            or not isinstance(seal.get("pod_files"), dict) or not seal["pod_files"]):
        raise ValueError("The post-pod seal does not match the source snapshot")
    return seal


def seal_pods(root):
    root = Path(root)
    if (root / GENERATED_SEAL).exists():
        manifest = validate_snapshot(root, pristine=False)
        seal = validated_seal(root, manifest)
        if seal["phase"] != "pods-installed":
            raise ValueError("This snapshot is already prepared for an archive")
        return seal
    manifest = validate_snapshot(root, pristine=False, allow_pod_changes=True)
    origin = validated_origin(root, manifest)
    validate_project(json.loads(command(["node", "-e", PROJECT_SCRIPT, str(root), "inspect"], root)))
    seal = {"schema": 1, "snapshot_sha256": digest(root / SNAPSHOT),
            "origin_sha256": digest(root / SNAPSHOT_ORIGIN), "source_sha": manifest["source_sha"],
            "source_version": origin["source_version"], "phase": "pods-installed",
            "generated_files": {relative: digest(root / safe_relative(relative)) for relative in manifest["generated_files"]},
            "pod_files": pod_integration_files(root)}
    write_json(root / GENERATED_SEAL, seal)
    return seal


def unquote(value):
    return str(value or "").strip('"')


def reference(value):
    return unquote(value.get("value")) if isinstance(value, dict) else unquote(value)


def validate_project(objects):
    found = set()
    for key, target in objects.get("PBXNativeTarget", {}).items():
        if key.endswith("_comment") or not isinstance(target, dict):
            continue
        name = unquote(target.get("name"))
        if name not in EXPECTED_TARGETS:
            continue
        if name in found:
            raise ValueError("Duplicate Preview archive target")
        found.add(name)
        configurations = objects["XCConfigurationList"][reference(target.get("buildConfigurationList"))]["buildConfigurations"]
        if not configurations:
            raise ValueError("Preview archive target has no build configuration")
        for configuration in configurations:
            settings = objects["XCBuildConfiguration"][reference(configuration)]["buildSettings"]
            for setting, expected in (("PRODUCT_BUNDLE_IDENTIFIER", EXPECTED_TARGETS[name]),
                                      ("DEVELOPMENT_TEAM", APPLE_TEAM), ("CODE_SIGN_STYLE", "Automatic")):
                if unquote(settings.get(setting)) != expected:
                    raise ValueError("Unexpected Preview archive target setting: " + name + "/" + setting)
    if found != set(EXPECTED_TARGETS):
        raise ValueError("The app, keyboard and widget archive targets are required")


PROJECT_SCRIPT = r'''
const path = require('node:path');
const fs = require('node:fs');
const root = process.argv[1];
const project = require(path.join(root, 'apps/mobile/node_modules/xcode')).project(path.join(root, 'apps/mobile/ios/Pennel.xcodeproj/project.pbxproj'));
project.parseSync();
if (process.argv[2] === 'inspect') {
  process.stdout.write(JSON.stringify(project.hash.project.objects));
} else {
  const versions = JSON.parse(process.argv[2]);
  for (const entry of Object.values(project.hash.project.objects.XCBuildConfiguration)) {
    const settings = entry && entry.buildSettings;
    if (settings && Object.keys(versions.bundles).includes(String(settings.PRODUCT_BUNDLE_IDENTIFIER || '').replace(/^"|"$/g, ''))) {
      settings.CURRENT_PROJECT_VERSION = versions.build;
      settings.MARKETING_VERSION = versions.version;
    }
  }
  fs.writeFileSync(project.filepath, project.writeSync());
}
'''


def prepare_build(root, build_number):
    root = Path(root)
    if not re.fullmatch(r"[1-9][0-9]*", build_number):
        raise ValueError("CI_BUILD_NUMBER must be a positive integer")
    manifest = validate_snapshot(root, pristine=False)
    seal = validated_seal(root, manifest)
    if seal["phase"] == "build-prepared" and seal.get("build_number") != build_number:
        raise ValueError("This snapshot is already prepared for another Cloud build number")
    objects = json.loads(command(["node", "-e", PROJECT_SCRIPT, str(root), "inspect"], root))
    validate_project(objects)
    ios = root / "apps/mobile/ios"
    workspace = ET.parse(ios / "Pennel.xcworkspace/contents.xcworkspacedata").getroot()
    locations = [entry.get("location") for entry in workspace.findall(".//FileRef")]
    if sorted(locations) != ["group:Pennel.xcodeproj", "group:Pods/Pods.xcodeproj"]:
        raise ValueError("The Preview workspace must contain only its app and Pods projects")
    scheme = ET.parse(ios / "Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme").getroot()
    archive = scheme.find("ArchiveAction")
    if archive is None or archive.get("buildConfiguration") != "Release":
        raise ValueError("The shared scheme must archive Release")
    if not any(entry.get("buildForArchiving") == "YES" and entry.find("BuildableReference") is not None
               and entry.find("BuildableReference").get("BlueprintName") == "Pennel"
               for entry in scheme.findall("./BuildAction/BuildActionEntries/BuildActionEntry")):
        raise ValueError("The app must be enabled for archiving")
    plists = {}
    for name, bundle in EXPECTED_TARGETS.items():
        file = ios / name / "Info.plist"
        no_symlink_path(file)
        data = plistlib.loads(file.read_bytes())
        if data.get("CFBundleIdentifier", "$(PRODUCT_BUNDLE_IDENTIFIER)") not in (bundle, "$(PRODUCT_BUNDLE_IDENTIFIER)"):
            raise ValueError("Unexpected Preview Info.plist identifier")
        if data.get("PennelAppGroupIdentifier") != APP_GROUP or data.get("PennelKeychainService") != BUNDLE_ID:
            raise ValueError("Unexpected Preview App Group or credential namespace")
        entitlement_file = ios / name / (name + ".entitlements")
        no_symlink_path(entitlement_file)
        entitlements = plistlib.loads(entitlement_file.read_bytes())
        if entitlements.get("com.apple.security.application-groups") != [APP_GROUP]:
            raise ValueError("Unexpected Preview App Group entitlement")
        plists[file] = data
    version = seal["source_version"]
    if plists[ios / "Pennel/Info.plist"].get("CFBundleShortVersionString") != version:
        raise ValueError("The app marketing version differs from the original source attestation")
    if seal["phase"] == "build-prepared":
        return {"build_number": build_number, "version": version, "team": APPLE_TEAM, "variant": "preview"}
    command(["node", "-e", PROJECT_SCRIPT, str(root), json.dumps({
        "build": build_number, "version": version, "bundles": {value: True for value in EXPECTED_TARGETS.values()},
    })], root)
    for file, data in plists.items():
        data["CFBundleVersion"] = build_number
        data["CFBundleShortVersionString"] = version
        file.write_bytes(plistlib.dumps(data, sort_keys=False))
    updated = json.loads(command(["node", "-e", PROJECT_SCRIPT, str(root), "inspect"], root))
    validate_project(updated)

    def without_versions(value):
        value = json.loads(json.dumps(value))
        for entry in value["XCBuildConfiguration"].values():
            settings = entry.get("buildSettings") if isinstance(entry, dict) else None
            if settings and unquote(settings.get("PRODUCT_BUNDLE_IDENTIFIER")) in EXPECTED_TARGETS.values():
                settings.pop("CURRENT_PROJECT_VERSION", None)
                settings.pop("MARKETING_VERSION", None)
        return value

    if without_versions(updated) != without_versions(objects):
        raise ValueError("Archive preparation changed project settings beyond the attested versions")
    generated = {relative: digest(root / safe_relative(relative)) for relative in seal["generated_files"]}
    if any(value != seal["generated_files"][relative] and relative not in BUILD_MUTABLE_FILES
           for relative, value in generated.items()):
        raise ValueError("Archive preparation changed an immutable generated file")
    if pod_integration_files(root) != seal["pod_files"]:
        raise ValueError("A sealed Pod integration file changed during archive preparation")
    write_json(root / GENERATED_SEAL, {**seal, "phase": "build-prepared", "build_number": build_number,
                                     "generated_files": generated})
    return {"build_number": build_number, "version": version, "team": APPLE_TEAM, "variant": "preview"}


def record_timing(root, phase, elapsed, exit_code, cache_hit=None):
    if not re.fullmatch(r"[a-z][a-z0-9-]{0,60}", phase) or elapsed < 0:
        raise ValueError("Invalid Cloud timing phase")
    file = root / "build/testflight/cloud-timings.json"
    receipt = json.loads(file.read_text()) if file.exists() else {"schema": 1, "phases": []}
    manifest_file = root / SNAPSHOT
    if manifest_file.is_file():
        receipt["source_sha"] = json.loads(manifest_file.read_text())["source_sha"]
    receipt["phases"].append({"name": phase, "seconds": elapsed, "exit_code": exit_code})
    if cache_hit is not None:
        receipt["native_cache_hit"] = cache_hit
    build_number = os.environ.get("CI_BUILD_NUMBER", "")
    if re.fullmatch(r"[1-9][0-9]*", build_number):
        receipt["build_number"] = int(build_number)
    write_json(file, receipt)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("fingerprint", "restore", "store", "validate-environment", "validate-snapshot", "seal-pods", "prepare-build", "record"))
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--cache-root", type=Path)
    parser.add_argument("--build-number", default=os.environ.get("CI_BUILD_NUMBER", ""))
    parser.add_argument("--phase")
    parser.add_argument("--elapsed", type=float, default=0)
    parser.add_argument("--exit-code", type=int, default=0)
    parser.add_argument("--cache-hit", choices=("true", "false"))
    args = parser.parse_args()
    root = args.root.resolve()
    try:
        if args.action == "validate-environment":
            validate_cloud_environment(os.environ)
        elif args.action == "validate-snapshot":
            validate_cloud_environment(os.environ)
            result = attest_snapshot(root)
            print(json.dumps({"source_sha": result["source_sha"], "valid": True}))
        elif args.action == "seal-pods":
            validate_cloud_environment(os.environ)
            result = seal_pods(root)
            print(json.dumps({"source_sha": result["source_sha"], "sealed": True}))
        elif args.action == "prepare-build":
            validate_cloud_environment(os.environ)
            print(json.dumps(prepare_build(root, args.build_number)))
        elif args.action == "record":
            record_timing(root, args.phase or "unknown", args.elapsed, args.exit_code,
                          None if args.cache_hit is None else args.cache_hit == "true")
        else:
            context = fingerprint(root)
            if args.action == "fingerprint":
                print(json.dumps(context, sort_keys=True))
                return 0
            cache_root = args.cache_root
            if cache_root is None:
                derived = os.environ.get("CI_DERIVED_DATA_PATH")
                if not derived or not Path(derived).is_absolute():
                    raise ValueError("An absolute Cloud DerivedData path is required")
                cache_root = Path(derived) / "pennel-native"
            if not cache_root.is_absolute():
                raise ValueError("Native cache root must be absolute")
            if args.action == "restore":
                result = restore(root, cache_root, context)
                print(json.dumps(result, sort_keys=True))
                return 0 if result["hit"] else 3
            source = validate_snapshot(root, pristine=False)["source_sha"]
            print(json.dumps(store(root, cache_root, context, source), sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, TypeError, plistlib.InvalidFileException, subprocess.SubprocessError) as error:
        print("Native release validation failed: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
