#!/usr/bin/env python3
"""Choose an existing stable identity for isolated Mac preview builds."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

PERSONAL_TEAM = "B538Q2K76C"


@dataclass(frozen=True)
class Identity:
    sha1: str
    name: str


@dataclass(frozen=True)
class Selection:
    identity: str
    sha1: str | None


def parse_identities(output):
    identities = []
    for line in output.splitlines():
        match = re.fullmatch(r'\s*\d+\)\s+([A-Fa-f0-9]{40})\s+"(.+)"\s*', line)
        if match:
            identity = Identity(match[1].upper(), match[2])
            if identity not in identities:
                identities.append(identity)
    return identities


def select_identity(identities, cached_sha=None, apple_override=None, pennel_override=None, *, allow_adhoc=False):
    personal = [identity for identity in identities
                if identity.name.startswith("Developer ID Application:")
                and identity.name.endswith(f"({PERSONAL_TEAM})")]
    explicit = apple_override or pennel_override
    if explicit:
        if explicit == "-" and allow_adhoc:
            return Selection("-", None)
        eligible = personal + [item for item in identities if item.name.startswith("Apple Development:")]
        matches = [item for item in eligible if explicit.upper() == item.sha1 or explicit == item.name]
        if len(matches) != 1:
            raise ValueError("Preview signing requires an existing personal-team certificate; other teams and unknown identities are refused")
        return Selection(matches[0].sha1, matches[0].sha1)
    if cached_sha:
        for identity in personal:
            if identity.sha1 == cached_sha.upper():
                return Selection(identity.sha1, identity.sha1)
    if personal:
        return Selection(personal[0].sha1, personal[0].sha1)
    if allow_adhoc:
        return Selection("-", None)
    raise ValueError("No personal-team preview certificate is available; isolated CI must explicitly opt into ad-hoc signing")


def validate_selection(selection):
    if selection.identity == "-":
        return
    spec = importlib.util.spec_from_file_location("pennel_mac_signing", Path(__file__).with_name("mac_signing.py"))
    signing = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(signing)
    signing.validate_certificate(signing.certificate_for_identity(selection.sha1), preview=True)


def cache_identity(path, sha1):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=path.name + ".", delete=False) as file:
            temporary = Path(file.name)
            file.write(sha1 + "\n")
        temporary.replace(path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=Path(__file__).resolve().parents[1] / "build/preview-signing-identity.sha1")
    parser.add_argument("--allow-adhoc-ci", action="store_true", help="Allow an unavailable credential store in explicitly isolated CI")
    args = parser.parse_args(argv)
    if args.allow_adhoc_ci and os.environ.get("CI") != "true":
        parser.error("Ad-hoc preview signing is available only in isolated CI")
    try:
        result = subprocess.run(["security", "find-identity", "-v", "-p", "codesigning"], capture_output=True, text=True, check=False)
        identities = parse_identities(result.stdout) if result.returncode == 0 else []
    except OSError:
        identities = []
    try:
        cached_sha = args.cache.read_text().strip()
    except OSError:
        cached_sha = None
    try:
        selection = select_identity(
            identities, cached_sha, os.environ.get("APPLE_SIGNING_IDENTITY"), os.environ.get("PENNEL_SIGN_IDENTITY"),
            allow_adhoc=args.allow_adhoc_ci,
        )
        validate_selection(selection)
    except ValueError as error:
        parser.exit(1, f"Preview signing failed: {error}\n")
    if selection.sha1:
        cache_identity(args.cache, selection.sha1)
    if selection.identity == "-":
        print("warning: isolated CI preview uses ad-hoc signing; its Data Protection Keychain is unavailable.", file=sys.stderr)
    print(selection.identity)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
