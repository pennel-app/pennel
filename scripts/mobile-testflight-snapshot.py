#!/usr/bin/env python3
"""Prepare an allowlisted, source-attested Expo project for Xcode Cloud."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
import xml.etree.ElementTree as ET


TEAM = "B538Q2K76C"
BUNDLE = "app.pennel.ios.preview"
GROUP = "group.app.pennel.preview"
IOS = Path("apps/mobile/ios")
MANIFEST = IOS / "pennel-testflight.json"
PATHS = Path("build/testflight/snapshot-paths.json")
SHA = re.compile(r"[0-9a-f]{40}\Z")
SECRET = re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----|\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{20,})")
PRIVATE_PATH = re.compile(r"(?:/Users/|/home/|/Volumes/|/private/|/tmp/|/var/folders/)")
APP_FILES = {
    "AppDelegate.swift", "SceneDelegate.swift", "Pennel-Bridging-Header.h", "Info.plist",
    "Pennel.entitlements", "SplashScreen.storyboard", "PrivacyInfo.xcprivacy", "Supporting/Expo.plist",
}
KEYBOARD_FILES = {"Info.plist", "PennelKeyboard.entitlements"}
WIDGET_FILES = {"index.swift", "Info.plist", "ExpoWidgetsTarget.entitlements", "ExpoWidgetsLayoutRegistry.config.json"}
FIXED_FILES = {
    "Podfile", "Podfile.properties.json", "Podfile.lock", ".xcode.env", "pennel-testflight.json",
    "Pennel.xcodeproj/project.pbxproj", "Pennel.xcodeproj/project.xcworkspace/contents.xcworkspacedata",
    "Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme",
    "Pennel.xcworkspace/contents.xcworkspacedata", "ci_scripts/ci_post_clone.sh",
    "ci_scripts/ci_pre_xcodebuild.sh", "ci_scripts/ci_post_xcodebuild.sh", "TestFlight/WhatToTest.en-US.txt",
}
REQUIRED_FILES = {
    "Pennel/AppDelegate.swift", "Pennel/Info.plist", "Pennel/Pennel.entitlements",
    "PennelKeyboard/Info.plist", "PennelKeyboard/PennelKeyboard.entitlements",
    "ExpoWidgetsTarget/index.swift", "ExpoWidgetsTarget/Info.plist",
    "ExpoWidgetsTarget/ExpoWidgetsTarget.entitlements", "ExpoWidgetsTarget/ExpoWidgetsLayoutRegistry.config.json",
    "Pennel.xcodeproj/project.pbxproj", "Podfile", "Podfile.properties.json",
}
INPUT_FILES = {
    "apps/mobile/package.json", "apps/mobile/package-lock.json", "apps/mobile/app.config.ts",
    "apps/mobile/Podfile.lock", "scripts/mobile-testflight-snapshot.py",
    "scripts/xcode-cloud-post-clone.sh", "scripts/xcode-cloud-pre-xcodebuild.sh", "scripts/xcode-cloud-post-xcodebuild.sh",
}
WORKSPACE = '''<?xml version="1.0" encoding="UTF-8"?>
<Workspace version="1.0">
   <FileRef location="group:Pennel.xcodeproj"/>
   <FileRef location="group:Pods/Pods.xcodeproj"/>
</Workspace>
'''


class SnapshotError(Exception):
    pass


def git(root, *arguments):
    result = subprocess.run(["git", "-C", str(root), *arguments], capture_output=True, text=True)
    if result.returncode:
        raise SnapshotError("Cannot establish the selected Git source revision")
    return result.stdout.strip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def regular(root, relative):
    path = root / relative
    for part in [path, *path.parents]:
        if part == root:
            break
        if part.is_symlink():
            raise SnapshotError(f"Symbolic links are not snapshot inputs: {relative}")
    if not path.is_file():
        raise SnapshotError(f"Missing required snapshot input: {relative}")
    return path


def checked_text(value, label, limit=None):
    if not isinstance(value, str) or any(ord(character) < 32 and character not in "\n\t" for character in value):
        raise SnapshotError(f"Invalid plain text in {label}")
    if limit is not None and len(value) > limit:
        raise SnapshotError(f"{label} exceeds {limit} characters")
    if SECRET.search(value):
        raise SnapshotError(f"Potential credential in {label}")
    return value


def validation_receipt(path, source):
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError):
        raise SnapshotError("Cannot read the source validation receipt") from None
    if not isinstance(data, dict) or data.get("schema") != 1 or data.get("source_sha") != source or data.get("status") != "success":
        raise SnapshotError("Validation must succeed for the exact selected source revision")
    checks = data.get("checks")
    if not isinstance(checks, list) or not checks:
        raise SnapshotError("Validation must identify successful source checks")
    selected = []
    for check in checks:
        if not isinstance(check, dict) or check.get("status") != "success":
            raise SnapshotError("Every selected source check must succeed")
        name = checked_text(check.get("name"), "check name", 200)
        if not re.fullmatch(r"[A-Za-z0-9 ()/_.:+-]+", name):
            raise SnapshotError("Invalid source check name")
        item = {"name": name, "status": "success"}
        if "url" in check:
            url = check["url"]
            if not isinstance(url, str) or not re.fullmatch(r"https://github\.com/pennel-app/pennel/actions/runs/[0-9]+(?:/[A-Za-z0-9_-]+)*", url):
                raise SnapshotError("Source check URLs must identify this repository's Actions runs")
            item["url"] = url
        selected.append(item)
    return {"schema": 1, "source_sha": source, "status": "success", "checks": selected}


def excluded(relative):
    parts = relative.parts
    return (parts[0] in {"Pods", "build", "PennelNative", ".xcode.env.local", ".gitignore", ".DS_Store"}
            or "xcuserdata" in parts or ".DS_Store" in parts
            or (parts[0] == "Pennel.xcworkspace" and "xcshareddata" in parts)
            or (parts[0] == "Pennel.xcodeproj" and "project.xcworkspace" in parts and "xcshareddata" in parts))


def allowed_file(relative):
    name = relative.as_posix()
    if name in FIXED_FILES:
        return True
    if len(relative.parts) > 1 and relative.parts[0] == "Pennel":
        tail = Path(*relative.parts[1:]).as_posix()
        if tail in APP_FILES:
            return True
        if re.fullmatch(r"Supporting/[A-Za-z0-9_-]+\.lproj/InfoPlist\.strings", tail):
            return True
        if tail.startswith("Images.xcassets/") and relative.suffix.lower() in {".json", ".png", ".jpg", ".jpeg", ".pdf"}:
            return all(re.fullmatch(r"[A-Za-z0-9_.@+-]+", part) and not part.startswith(".") for part in relative.parts[2:])
    return (len(relative.parts) == 2
            and ((relative.parts[0] == "PennelKeyboard" and relative.parts[1] in KEYBOARD_FILES)
                 or (relative.parts[0] == "ExpoWidgetsTarget" and relative.parts[1] in WIDGET_FILES)))


def allowed_directory(relative):
    name = relative.as_posix() + "/"
    if any(item.startswith(name) for item in FIXED_FILES | REQUIRED_FILES):
        return True
    return (name in {"Pennel/Supporting/", "Pennel/Images.xcassets/"}
            or re.fullmatch(r"Pennel/Supporting/[A-Za-z0-9_-]+\.lproj/", name) is not None
            or (name.startswith("Pennel/Images.xcassets/")
                and all(re.fullmatch(r"[A-Za-z0-9_.@+-]+", part) and not part.startswith(".") for part in relative.parts[2:])))


def generated_files(root):
    directory = root / IOS
    if directory.is_symlink() or not directory.is_dir():
        raise SnapshotError("Generate the preview iOS project before preparing a snapshot")
    selected = []
    for current, directories, files in os.walk(directory, topdown=True, followlinks=False):
        base = Path(current)
        for entry in list(directories):
            path = base / entry
            relative = path.relative_to(directory)
            if excluded(relative):
                directories.remove(entry)
                continue
            if path.is_symlink() or not allowed_directory(relative):
                raise SnapshotError(f"Unexpected generated iOS directory: {relative}")
        for entry in files:
            path = base / entry
            relative = path.relative_to(directory)
            if excluded(relative):
                continue
            if not allowed_file(relative):
                raise SnapshotError(f"Unexpected generated iOS file: {relative}")
            regular(root, IOS / relative)
            if path.suffix.lower() not in {".png", ".jpg", ".jpeg", ".pdf"}:
                try:
                    text = path.read_text()
                except (OSError, UnicodeError):
                    raise SnapshotError(f"Invalid generated text file: {relative}") from None
                checked_text(text, f"generated file {relative}")
                if PRIVATE_PATH.search(text) or str(root) in text:
                    raise SnapshotError(f"Nonportable path in generated file: {relative}")
            selected.append(IOS / relative)
    present = {path.relative_to(IOS).as_posix() for path in selected}
    if missing := REQUIRED_FILES - present:
        raise SnapshotError("Generated iOS files are missing: " + ", ".join(sorted(missing)))
    return sorted(selected)


def preview_identity(root):
    for directory, name in [("Pennel", "Pennel"), ("PennelKeyboard", "PennelKeyboard"),
                            ("ExpoWidgetsTarget", "ExpoWidgetsTarget")]:
        try:
            info = plistlib.loads((root / IOS / directory / "Info.plist").read_bytes())
            entitlements = plistlib.loads((root / IOS / directory / f"{name}.entitlements").read_bytes())
        except (OSError, ValueError, plistlib.InvalidFileException):
            raise SnapshotError(f"Invalid Preview identity files for {directory}") from None
        if info.get("PennelAppGroupIdentifier") != GROUP or entitlements.get("com.apple.security.application-groups") != [GROUP]:
            raise SnapshotError(f"Preview App Group is required for {directory}")
    project = (root / IOS / "Pennel.xcodeproj/project.pbxproj").read_text()
    identifiers = set(re.findall(r'PRODUCT_BUNDLE_IDENTIFIER\s*=\s*"?([A-Za-z0-9_.-]+)"?\s*;', project))
    shipping = {BUNDLE, BUNDLE + ".keyboard", BUNDLE + ".widgets"}
    if not shipping <= identifiers or not identifiers <= shipping | {BUNDLE + ".keyboard-tests", BUNDLE + ".runtime-tests"}:
        raise SnapshotError("The snapshot must contain only the Preview app and extension identities")
    teams = re.findall(r'DEVELOPMENT_TEAM\s*=\s*"?([A-Za-z0-9]+)"?\s*;', project)
    if not teams or set(teams) != {TEAM} or re.search(r'CODE_SIGN_STYLE\s*=\s*"?Manual"?\s*;', project):
        raise SnapshotError("Automatic signing with the personal Apple team is required")
    return project


def shared_scheme(root, project):
    identifiers = []
    for match in re.finditer(r"(?ms)^\s*([A-Fa-f0-9]{24}) /\* Pennel \*/ = \{(.*?)^\s*\};", project):
        if re.search(r"\bisa\s*=\s*PBXNativeTarget\s*;", match[2]) and re.search(r'\bname\s*=\s*"?Pennel"?\s*;', match[2]):
            identifiers.append(match[1])
    if len(identifiers) != 1:
        raise SnapshotError("Cannot identify the unique Pennel application target")
    identifier = identifiers[0]
    path = root / IOS / "Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme"
    if path.exists():
        try:
            contents = path.read_text()
            scheme = ET.fromstring(contents)
        except (OSError, UnicodeError, ET.ParseError):
            raise SnapshotError("Invalid shared Pennel scheme") from None
        entry = next((item for item in scheme.findall("BuildAction/BuildActionEntries/BuildActionEntry")
                      if (reference := item.find("BuildableReference")) is not None
                      and reference.get("BlueprintName") == "Pennel" and reference.get("BlueprintIdentifier") == identifier), None)
        if entry is None:
            raise SnapshotError("The shared scheme must build the Pennel application target")
        archive = scheme.find("ArchiveAction")
        if entry.get("buildForArchiving") == "YES" and archive is not None and archive.get("buildConfiguration") == "Release":
            return contents
        entry.set("buildForArchiving", "YES")
        if archive is None:
            archive = ET.SubElement(scheme, "ArchiveAction")
        archive.set("buildConfiguration", "Release")
        return ET.tostring(scheme, encoding="unicode", xml_declaration=True) + "\n"
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<Scheme version="1.3">
  <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
    <BuildActionEntries>
      <BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">
        <BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{identifier}" BuildableName="Pennel.app" BlueprintName="Pennel" ReferencedContainer="container:Pennel.xcodeproj"/>
      </BuildActionEntry>
    </BuildActionEntries>
  </BuildAction>
  <ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/>
</Scheme>
'''


def write_output(root, relative, contents, executable=False):
    path = root / relative
    for parent in [path, *path.parents]:
        if parent == root:
            break
        if parent.is_symlink():
            raise SnapshotError(f"Refusing a symbolic link output: {relative}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents if isinstance(contents, bytes) else contents.encode())
    path.chmod(0o755 if executable else 0o644)


def prepare(root, source, receipt, notes_file=None):
    root = root.resolve(strict=True)
    if not SHA.fullmatch(source) or git(root, "rev-parse", "HEAD") != source:
        raise SnapshotError("source-sha must identify the exact checked-out full commit")
    if Path(git(root, "rev-parse", "--show-toplevel")).resolve() != root:
        raise SnapshotError("root must be the source repository root")
    if git(root, "diff", "--name-only", source, "--", ".", ":(exclude)apps/mobile/ios"):
        raise SnapshotError("Tracked source files differ from the selected revision")
    validation = validation_receipt(receipt, source)
    try:
        notes = checked_text(notes_file.read_text(), "test instructions", 4000) if notes_file else ""
    except (OSError, UnicodeError):
        raise SnapshotError("Cannot read plain-text test instructions") from None
    inputs = set(INPUT_FILES)
    plugins = root / "apps/mobile/plugins"
    if not plugins.is_dir() or plugins.is_symlink():
        raise SnapshotError("The canonical Expo plugins are required")
    inputs.update(path.relative_to(root).as_posix() for path in plugins.rglob("*") if path.suffix in {".cjs", ".js", ".rb"})
    hashes = {}
    for relative in sorted(inputs):
        if git(root, "ls-files", "--error-unmatch", "--", relative) != relative:
            raise SnapshotError(f"Snapshot inputs must belong to the selected source: {relative}")
        hashes[relative] = digest(regular(root, relative))
    generated_files(root)
    project = preview_identity(root)
    scheme = shared_scheme(root, project)
    write_output(root, IOS / "Podfile.lock", (root / "apps/mobile/Podfile.lock").read_bytes())
    write_output(root, IOS / "Pennel.xcworkspace/contents.xcworkspacedata", WORKSPACE)
    write_output(root, IOS / "Pennel.xcodeproj/xcshareddata/xcschemes/Pennel.xcscheme", scheme)
    if not (root / IOS / ".xcode.env").exists():
        write_output(root, IOS / ".xcode.env", 'export NODE_BINARY=$(command -v node)\n')
    for hook, canonical in [("ci_post_clone.sh", "xcode-cloud-post-clone.sh"),
                            ("ci_pre_xcodebuild.sh", "xcode-cloud-pre-xcodebuild.sh"),
                            ("ci_post_xcodebuild.sh", "xcode-cloud-post-xcodebuild.sh")]:
        wrapper = ('#!/bin/sh\nset -eu\n'
                   'root="${CI_PRIMARY_REPOSITORY_PATH:-$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)}"\n'
                   f'exec /bin/bash "$root/scripts/{canonical}"\n')
        write_output(root, IOS / "ci_scripts" / hook, wrapper, executable=True)
    notes_path = root / IOS / "TestFlight/WhatToTest.en-US.txt"
    if notes:
        write_output(root, IOS / "TestFlight/WhatToTest.en-US.txt", notes)
    elif notes_path.exists():
        notes_path.unlink()
    selected = [path for path in generated_files(root) if path != MANIFEST]
    manifest = {
        "schema": 1, "variant": "preview", "apple_team_id": TEAM, "bundle_identifier": BUNDLE,
        "source_sha": source, "source_tree_sha": git(root, "rev-parse", f"{source}^{{tree}}"),
        "validation": validation, "inputs": hashes,
        "generated_files": {path.as_posix(): digest(root / path) for path in selected}, "notes": notes,
    }
    write_output(root, MANIFEST, json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    paths = sorted(path.as_posix() for path in [*selected, MANIFEST])
    write_output(root, PATHS, json.dumps(paths, indent=2) + "\n")
    return {"source_sha": source, "manifest": MANIFEST.as_posix(), "paths": PATHS.as_posix(), "file_count": len(paths)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--validation-receipt", type=Path, required=True)
    parser.add_argument("--notes-file", type=Path)
    args = parser.parse_args()
    try:
        result = prepare(args.root, args.source_sha, args.validation_receipt, args.notes_file)
    except (SnapshotError, OSError) as error:
        parser.error(str(error))
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
