#!/usr/bin/env python3
"""Decide the next automatic desktop release and render its Homebrew cask."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
TAURI_CONFIG = ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
# Paths that change what ends up in an installer; docs alone never cut a release.
APP_PATHS = ("core", "apps/desktop", "apps/macos-platform", "assets", "scripts/desktop-release.py",
             "scripts/mac_signing.py", "scripts/verify-desktop-bundle.py", "scripts/desktop-auto-release.py",
             ".github/workflows/desktop.yml")
VERSION = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)")


def parse(version):
    match = VERSION.fullmatch(version)
    if not match:
        raise ValueError(f"Not a plain semantic version: {version!r}")
    return tuple(int(part) for part in match.groups())


def next_version(configured, latest_tag):
    """Patch-bump the newest tag; a higher version committed in tauri.conf.json wins."""
    if latest_tag is None:
        return configured
    major, minor, patch = parse(latest_tag.removeprefix("v"))
    bumped = (major, minor, patch + 1)
    return configured if parse(configured) > bumped else "{}.{}.{}".format(*bumped)


def latest_release_tag(tags):
    versions = [tag for tag in tags if tag.startswith("v") and VERSION.fullmatch(tag[1:])]
    return max(versions, key=lambda tag: parse(tag[1:]), default=None)


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True)


def app_changed_since(tag):
    if tag is None:
        return True
    return subprocess.run(["git", "diff", "--quiet", tag, "HEAD", "--", *APP_PATHS], cwd=ROOT).returncode != 0


def verify_mac_receipt(receipt, version, sha256, *, dmg_name=None):
    parse(version)
    if receipt.get("version") != version or receipt.get("dmg_sha256") != sha256:
        raise ValueError("Mac notarization receipt does not identify these exact release bytes")
    if dmg_name is not None and receipt.get("dmg_file") != dmg_name:
        raise ValueError("Mac notarization receipt names a different DMG")
    for key, expected in {"bundle_identifier": "dev.pennel.Pennel", "team_identifier": "B538Q2K76C",
                          "developer_id": True, "notarization": "accepted", "app_ticket": "passed",
                          "dmg_ticket": "passed", "gatekeeper": "passed", "quarantine_assessment": "passed"}.items():
        if receipt.get(key) != expected:
            raise ValueError(f"Mac distribution gate has not passed: {key}")
    group = receipt.get("keychain_access_group", "")
    if not re.fullmatch(r"[A-Z0-9]{10}\.dev\.pennel\.Pennel", group) or receipt.get("application_identifier") != group:
        raise ValueError("Mac distribution does not have its private canonical Keychain group")
    if not re.fullmatch(r"[0-9a-f]{64}", receipt.get("profile_sha256", "")):
        raise ValueError("Mac distribution receipt is missing its verified profile hash")
    try:
        expiry = datetime.fromisoformat(receipt["profile_expires"])
        if expiry.tzinfo is None or expiry <= datetime.now(timezone.utc):
            raise ValueError("Mac distribution's provisioning profile is expired or ambiguous")
    except (KeyError, TypeError) as error:
        raise ValueError("Mac distribution receipt is missing its profile expiry") from error


def render_cask(version, sha256, repository, *, notarization_receipt=None):
    if not re.fullmatch(r"[0-9a-f]{64}", sha256):
        raise ValueError("sha256 must be 64 lowercase hex characters")
    parse(version)
    quarantine = '''  # Legacy builds are not notarized; retain this only for an unpromoted legacy artifact.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Pennel.app"]
  end

'''
    if notarization_receipt is not None:
        verify_mac_receipt(notarization_receipt, version, sha256, dmg_name=f"Pennel_{version}_aarch64.dmg")
        quarantine = ""
    return f'''# typed: strict
# frozen_string_literal: true

cask "pennel" do
  version "{version}"
  sha256 "{sha256}"

  url "https://github.com/{repository}/releases/download/v#{{version}}/Pennel_#{{version}}_aarch64.dmg"
  name "Pennel"
  desc "Dictation and meeting notes"
  homepage "https://pennel.app/"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Pennel.app"

{quarantine}\
  # Meetings, history and models stay; zap only removes regenerable data.
  zap trash: [
    "~/Library/Caches/app.pennel.desktop",
    "~/Library/WebKit/app.pennel.desktop",
  ]
end
'''


def plan():
    configured = json.loads(TAURI_CONFIG.read_text(encoding="utf-8"))["version"]
    tag = latest_release_tag(git("tag", "--list", "v*").split())
    return {
        "release": app_changed_since(tag),
        "version": next_version(configured, tag),
        "previous": tag or "",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("plan", help="Print release, version and previous as KEY=value lines")
    cask = commands.add_parser("cask", help="Print the Homebrew cask for a published release")
    cask.add_argument("version")
    cask.add_argument("sha256")
    cask.add_argument("--repository", default="pennel-app/pennel")
    cask.add_argument("--notarization-receipt", type=Path)
    verify_mac = commands.add_parser("verify-mac", help="Gate publication on exact personal-team notarized DMG bytes")
    verify_mac.add_argument("version")
    verify_mac.add_argument("dmg", type=Path)
    verify_mac.add_argument("receipt", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "plan":
            for key, value in plan().items():
                print(f"{key}={str(value).lower() if isinstance(value, bool) else value}")
        elif args.command == "cask":
            receipt = json.loads(args.notarization_receipt.read_text()) if args.notarization_receipt else None
            sys.stdout.write(render_cask(args.version, args.sha256, args.repository, notarization_receipt=receipt))
        else:
            verify_mac_receipt(json.loads(args.receipt.read_text()), args.version,
                               hashlib.sha256(args.dmg.read_bytes()).hexdigest(), dmg_name=args.dmg.name)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error))


if __name__ == "__main__":
    main()
