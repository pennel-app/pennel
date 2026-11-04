#!/usr/bin/env python3
"""Prepare personal-team Mac signing and verify the exact notarized distribution."""

import argparse
from datetime import datetime, timezone
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile
import uuid


ROOT = Path(__file__).resolve().parents[1]
PERSONAL_TEAM = "B538Q2K76C"
IDENTIFIERS = {"production": "dev.pennel.Pennel", "preview": "dev.pennel.Pennel.preview"}
DEVELOPER_ID_OID = "1.2.840.113635.100.6.1.13"
DEVELOPMENT_OID = "1.2.840.113635.100.6.1.2"
NOTARY_ENV = ("APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH")


def command(arguments, *, environment=None, binary=False):
    """Never put credentials, CMS payloads, or raw tool diagnostics in failures."""
    result = subprocess.run([str(arg) for arg in arguments], env=environment, capture_output=True, text=not binary)
    if result.returncode:
        raise ValueError(f"{Path(arguments[0]).name} {arguments[1] if len(arguments) > 1 else ''} failed (exit {result.returncode})")
    return result.stdout


def identities(output):
    return [(match[1].upper(), match[2]) for line in output.splitlines()
            if (match := re.fullmatch(r'\s*\d+\)\s+([A-Fa-f0-9]{40})\s+"(.+)"\s*', line))]


def choose_identity(available, requested=None, *, preview=False):
    candidates = [(sha, name) for sha, name in available
                  if name.startswith("Developer ID Application:") and name.endswith(f"({PERSONAL_TEAM})")]
    if requested:
        # Development CN parentheses identify the certificate, not its team. An
        # explicit selection is authorized by its DER OU and profile below.
        if preview:
            candidates += [(sha, name) for sha, name in available if name.startswith("Apple Development:")]
        candidates = [item for item in candidates if requested.upper() == item[0] or requested == item[1]]
    if len(candidates) != 1:
        raise ValueError("Select exactly one existing personal-team signing identity; ad-hoc and other teams are not allowed")
    return candidates[0]


def pem_certificates(data):
    return re.findall(rb"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", data, re.S)


def certificate_der(pem):
    import base64
    return base64.b64decode(re.sub(rb"-----[^\n]+-----|\s", b"", pem), validate=True)


def certificate_for_identity(sha):
    certificates = command(["security", "find-certificate", "-a", "-p"], binary=True)
    matches = [certificate_der(pem) for pem in pem_certificates(certificates)
               if hashlib.sha1(certificate_der(pem)).hexdigest().upper() == sha.upper()]
    if not matches:
        raise ValueError("Selected signing certificate is not available")
    return matches[0]


def certificate_info(der, directory):
    path = Path(directory) / "certificate.der"
    path.write_bytes(der)
    subject = command(["openssl", "x509", "-inform", "DER", "-in", path, "-noout", "-subject", "-nameopt", "RFC2253"])
    details = command(["openssl", "x509", "-inform", "DER", "-in", path, "-noout", "-text"])
    return subject, details


def validate_certificate(der, *, preview=False):
    with tempfile.TemporaryDirectory(prefix="pennel-certificate-") as temporary:
        subject, details = certificate_info(der, temporary)
        if not re.search(rf"(?:^|,)OU={PERSONAL_TEAM}(?:,|$)", subject.strip().removeprefix("subject=").strip()):
            raise ValueError("Signing certificate is not owned by the personal Apple team")
        developer_id = DEVELOPER_ID_OID in details
        if not developer_id and not (preview and DEVELOPMENT_OID in details):
            raise ValueError("Signing certificate has the wrong Apple certificate class")
        chain = command(["security", "verify-cert", "-p", "codeSign", "-P", "-c", Path(temporary) / "certificate.der"], binary=True)
        apple_roots = command(["security", "find-certificate", "-a", "-p", "-c", "Apple Root", "/System/Library/Keychains/SystemRootCertificates.keychain"], binary=True)
        trusted = {hashlib.sha256(certificate_der(pem)).digest() for pem in pem_certificates(apple_roots)}
        certificates = pem_certificates(chain)
        if not certificates or hashlib.sha256(certificate_der(certificates[-1])).digest() not in trusted:
            raise ValueError("Signing certificate does not chain to a system Apple root")
        return developer_id


def decode_profile(path):
    """Verify CMS against system Apple roots, then inspect the authenticated plist."""
    with tempfile.TemporaryDirectory(prefix="pennel-profile-") as temporary:
        temporary = Path(temporary)
        roots = temporary / "apple-roots.pem"
        roots.write_bytes(command(["security", "find-certificate", "-a", "-p", "-c", "Apple Root", "/System/Library/Keychains/SystemRootCertificates.keychain"], binary=True))
        decoded, signer = temporary / "profile.plist", temporary / "signer.pem"
        # Apple's provisioning-profile policy checks the profile's expiry, not the CMS
        # signer's current validity. Keep cryptographic verification and Apple anchors.
        command(["openssl", "cms", "-verify", "-binary", "-inform", "DER", "-in", path,
                 "-CAfile", roots, "-purpose", "any", "-no_check_time", "-out", decoded, "-signer", signer])
        subject = command(["openssl", "x509", "-in", signer, "-noout", "-subject", "-nameopt", "RFC2253"])
        if not re.search(r"(?:^|,)CN=(?:Mac OS X|Apple iPhone OS) Provisioning Profile Signing(?:,|$)", subject.strip().removeprefix("subject=").strip()):
            raise ValueError("Provisioning profile was not signed by Apple's profile-signing authority")
        profile = plistlib.loads(decoded.read_bytes())
        if not isinstance(profile, dict):
            raise ValueError("Provisioning profile payload is invalid")
        return profile


def validate_profile(profile, identifier, certificate, *, developer_id=True, now=None):
    if identifier not in IDENTIFIERS.values():
        raise ValueError("Only Pennel's canonical and isolated Preview identifiers are allowed")
    now = now or datetime.now(timezone.utc)
    expiry = profile.get("ExpirationDate")
    if not isinstance(expiry, datetime):
        raise ValueError("Provisioning profile has no expiry")
    if expiry.tzinfo is None:
        expiry = expiry.replace(tzinfo=timezone.utc)
    if expiry <= now:
        raise ValueError("Provisioning profile has expired")
    if profile.get("TeamIdentifier") != [PERSONAL_TEAM]:
        raise ValueError("Provisioning profile belongs to a different Apple team")
    if not set(profile.get("Platform", [])) & {"OSX", "macOS"}:
        raise ValueError("A macOS provisioning profile is required")
    if developer_id and (profile.get("ProvisionsAllDevices") is not True or profile.get("ProvisionedDevices")):
        raise ValueError("A Developer ID distribution profile is required")
    if not developer_id and not profile.get("ProvisionedDevices"):
        raise ValueError("A development profile must authorize the test Mac")
    prefixes = profile.get("ApplicationIdentifierPrefix", [])
    if len(prefixes) != 1 or not re.fullmatch(r"[A-Z0-9]{10}", prefixes[0]):
        raise ValueError("Provisioning profile has an ambiguous App ID prefix")
    app_id = f"{prefixes[0]}.{identifier}"
    claims = profile.get("Entitlements", {})
    if claims.get("com.apple.application-identifier") != app_id:
        raise ValueError("Provisioning profile must authorize the exact Pennel App ID")
    if claims.get("com.apple.developer.team-identifier") != PERSONAL_TEAM:
        raise ValueError("Provisioning profile has a different entitlement team")
    if developer_id and (claims.get("get-task-allow") or claims.get("com.apple.security.get-task-allow")):
        raise ValueError("Distribution profile must not allow debugging")
    if not any(isinstance(group, str) and fnmatch.fnmatchcase(app_id, group)
               for group in claims.get("keychain-access-groups", [])):
        raise ValueError("Provisioning profile does not authorize Pennel's private Keychain group")
    if certificate not in profile.get("DeveloperCertificates", []):
        raise ValueError("Provisioning profile does not authorize the selected signing certificate")
    return {"application_identifier": app_id, "keychain_access_group": app_id,
            "profile_uuid": profile.get("UUID"), "profile_expires": expiry.isoformat()}


def prepare(flavor, output, environment=None):
    environment = dict(os.environ if environment is None else environment)
    identifier = IDENTIFIERS[flavor]
    profile_variable = "PENNEL_MAC_PROVISIONING_PROFILE" if flavor == "production" else "PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE"
    profile_path = Path(environment.get(profile_variable, ""))
    if not environment.get(profile_variable) or not profile_path.is_file():
        raise ValueError(f"{profile_variable} must reference an existing personal-team macOS profile")
    profile = decode_profile(profile_path)
    authorized = {hashlib.sha1(certificate).hexdigest().upper()
                  for certificate in profile.get("DeveloperCertificates", []) if isinstance(certificate, bytes)}
    requested = environment.get("APPLE_SIGNING_IDENTITY") or environment.get("PENNEL_SIGN_IDENTITY")
    available = [identity for identity in identities(command(["security", "find-identity", "-v", "-p", "codesigning"]))
                 if identity[0] in authorized]
    sha, name = choose_identity(available, requested, preview=flavor == "preview")
    certificate = certificate_for_identity(sha)
    developer_id = validate_certificate(certificate, preview=flavor == "preview")
    attributes = validate_profile(profile, identifier, certificate, developer_id=developer_id)
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    profile_copy = output / "embedded.provisionprofile"
    shutil.copyfile(profile_path, profile_copy)
    profile_copy.chmod(0o600)
    entitlements = plistlib.loads((ROOT / "apps/desktop/src-tauri/Pennel.entitlements").read_bytes())
    entitlements.update({"com.apple.application-identifier": attributes["application_identifier"],
                         "com.apple.developer.team-identifier": PERSONAL_TEAM,
                         "keychain-access-groups": [attributes["keychain_access_group"]]})
    if entitlements.get("com.apple.security.app-sandbox") or entitlements.get("com.apple.security.get-task-allow"):
        raise ValueError("Pennel distribution must retain its unsandboxed, non-debug entitlement policy")
    entitlements_path = output / "Pennel.entitlements"
    entitlements_path.write_bytes(plistlib.dumps(entitlements))
    config = (json.loads((ROOT / "apps/desktop/src-tauri/tauri.preview.conf.json").read_text())
              if flavor == "preview" else {"identifier": identifier, "bundle": {}})
    config["bundle"]["macOS"] = {
        "signingIdentity": sha, "hardenedRuntime": True, "entitlements": str(entitlements_path),
        "files": {"embedded.provisionprofile": str(profile_copy)},
    }
    config_path = output / "tauri.signing.conf.json"
    config_path.write_text(json.dumps(config, indent=2) + "\n")
    plan = {"identifier": identifier, "team_identifier": PERSONAL_TEAM, "identity": sha, "identity_name": name,
            "developer_id": developer_id, "profile_sha256": hashlib.sha256(profile_copy.read_bytes()).hexdigest(),
            "config": str(config_path), **attributes}
    (output / "signing-plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    return plan


def release_requirement(identifier=None):
    if identifier is not None and identifier not in IDENTIFIERS.values():
        raise ValueError("Unexpected Pennel bundle identifier")
    return ((f'identifier "{identifier}" and ' if identifier else '') + 'anchor apple generic and '
            f'certificate leaf[field.{DEVELOPER_ID_OID}] exists and '
            f'certificate leaf[subject.OU] = "{PERSONAL_TEAM}"')


def signed_certificate(app):
    with tempfile.TemporaryDirectory(prefix="pennel-signed-cert-") as temporary:
        prefix = str(Path(temporary) / "cert")
        # This optional codesign argument must be attached; a separate prefix is
        # interpreted as another code object rather than an output location.
        command(["codesign", "-d", "--extract-certificates=" + prefix, app])
        return Path(prefix + "0").read_bytes()


def inspect_signed_app(app, *, identifier=IDENTIFIERS["production"], developer_id=True):
    app = Path(app)
    arguments = ["codesign", "--verify", "--deep", "--strict"]
    if developer_id:
        arguments.extend(["-R", "=" + release_requirement(identifier)])
    arguments.append(app)
    command(arguments)
    result = subprocess.run(["codesign", "-dv", "--verbose=4", str(app)], capture_output=True, text=True)
    if result.returncode:
        raise ValueError("Cannot inspect the app's signature")
    metadata = result.stdout + result.stderr
    if f"TeamIdentifier={PERSONAL_TEAM}" not in metadata:
        raise ValueError("Packaged app is not signed by the personal Apple team")
    if developer_id and "Authority=Developer ID Application:" not in metadata:
        raise ValueError("Packaged app requires a Developer ID Application signature")
    entitlements = plistlib.loads(command(["codesign", "-d", "--entitlements", ":-", app], binary=True))
    certificate = signed_certificate(app)
    profile_path = app / "Contents/embedded.provisionprofile"
    attributes = validate_profile(decode_profile(profile_path), identifier, certificate, developer_id=developer_id)
    expected = {"com.apple.application-identifier": attributes["application_identifier"],
                "com.apple.developer.team-identifier": PERSONAL_TEAM,
                "keychain-access-groups": [attributes["keychain_access_group"]]}
    if any(entitlements.get(key) != value for key, value in expected.items()):
        raise ValueError("Packaged app's signed Keychain entitlements do not match its profile")
    if entitlements.get("com.apple.security.app-sandbox") or entitlements.get("com.apple.security.get-task-allow") or entitlements.get("get-task-allow"):
        raise ValueError("Unexpected sandbox or debug entitlement in Pennel")
    cdhash = re.search(r"^CDHash=(\S+)$", metadata, re.M)
    if not cdhash:
        raise ValueError("Packaged app is missing its code directory hash")
    return {"bundle_identifier": identifier, "team_identifier": PERSONAL_TEAM, "developer_id": developer_id,
            "profile_sha256": hashlib.sha256(profile_path.read_bytes()).hexdigest(), "app_cdhash": cdhash[1], **attributes}


def notary_arguments(environment):
    if environment.get("APPLE_TEAM_ID") and environment["APPLE_TEAM_ID"] != PERSONAL_TEAM:
        raise ValueError("Notarization must use the personal Apple team")
    if environment.get("PENNEL_NOTARY_KEYCHAIN_PROFILE"):
        return ["--keychain-profile", environment["PENNEL_NOTARY_KEYCHAIN_PROFILE"]]
    if all(environment.get(key) for key in ("APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH")):
        if not Path(environment["APPLE_API_KEY_PATH"]).is_file():
            raise ValueError("Existing notarization API key file is missing")
        return ["--issuer", environment["APPLE_API_ISSUER"], "--key-id", environment["APPLE_API_KEY"], "--key", environment["APPLE_API_KEY_PATH"]]
    if all(environment.get(key) for key in ("APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID")):
        return ["--apple-id", environment["APPLE_ID"], "--password", environment["APPLE_PASSWORD"], "--team-id", PERSONAL_TEAM]
    raise ValueError("Existing personal-team notarization credentials are required")


def notarize(path, environment):
    result = json.loads(command(["xcrun", "notarytool", "submit", path, "--wait", "--output-format", "json", *notary_arguments(environment)], environment=environment))
    if result.get("status") != "Accepted" or not result.get("id"):
        raise ValueError("Apple did not accept the exact artifact for notarization")
    return result.get("id")


def build_environment(environment, identity):
    result = dict(environment)
    # We notarize and staple the final app and container explicitly, in order.
    for key in (*NOTARY_ENV, "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "PENNEL_SIGN_IDENTITY", "PENNEL_NOTARY_KEYCHAIN_PROFILE"):
        result.pop(key, None)
    result["APPLE_SIGNING_IDENTITY"] = identity
    return result


def build_preview(output, environment=None):
    environment = dict(os.environ if environment is None else environment)
    isolated_ci = environment.get("CI") == "true" and environment.get("PENNEL_ALLOW_ADHOC_PREVIEW") == "1"
    if environment.get("PENNEL_ALLOW_ADHOC_PREVIEW") == "1" and not isolated_ci:
        raise ValueError("Ad-hoc Preview signing is available only in explicitly isolated CI")
    if isolated_ci:
        # Never reuse a credential-capable overlay from an earlier local build.
        output = Path(output).resolve() / "isolated-ci"
        output.mkdir(parents=True, exist_ok=True)
        config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.preview.conf.json").read_text())
        config["bundle"]["macOS"] = {"signingIdentity": "-", "hardenedRuntime": True,
                                     "entitlements": str(ROOT / "apps/desktop/src-tauri/Pennel.entitlements"), "files": {}}
        config_path = output / "tauri.signing.conf.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n")
        plan = {"identity": "-", "identifier": IDENTIFIERS["preview"], "config": str(config_path), "developer_id": False}
    else:
        plan = prepare("preview", output, environment)
    arguments = [str(ROOT / "scripts/with-toolchain.sh"), "npx", "tauri", "build", "--bundles", "app", "--config", plan["config"]]
    guard = Path.home() / ".local/bin/worktree-storage"
    if guard.is_file():
        arguments = [str(guard), "run", "--", *arguments]
    subprocess.run(arguments, cwd=ROOT / "apps/desktop", env=build_environment(environment, plan["identity"]), check=True)
    if not isolated_ci:
        inspect_signed_app(ROOT / "apps/desktop/src-tauri/target/release/bundle/macos/Pennel Preview.app",
                           identifier=plan["identifier"], developer_id=plan["developer_id"])
    return plan


def quarantine_assessment(dmg, expected):
    with tempfile.TemporaryDirectory(prefix="pennel-quarantine-") as temporary:
        temporary = Path(temporary)
        copy = temporary / dmg.name
        shutil.copyfile(dmg, copy)
        quarantine = f"0083;{int(datetime.now(timezone.utc).timestamp()):x};PennelReleaseQA;{uuid.uuid4()}"
        command(["xattr", "-w", "com.apple.quarantine", quarantine, copy])
        command(["spctl", "--assess", "--type", "open", "--context", "context:primary-signature", copy])
        if not command(["xattr", "-p", "com.apple.quarantine", copy]).strip():
            raise ValueError("Gatekeeper check lost the quarantine attribute")
        mount = plistlib.loads(command(["hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen", "-plist", copy], binary=True))
        volumes = [Path(entry["mount-point"]) for entry in mount.get("system-entities", []) if entry.get("mount-point")]
        if len(volumes) != 1:
            for volume in volumes:
                command(["hdiutil", "detach", volume])
            raise ValueError("Cannot identify the exact DMG's mounted application")
        volume = volumes[0]
        try:
            source = volume / "Pennel.app"
            packaged = inspect_signed_app(source)
            if any(packaged[key] != expected[key] for key in ("app_cdhash", "profile_sha256", "keychain_access_group")):
                raise ValueError("The application inside the DMG differs from the verified app")
            installed = temporary / "Pennel.app"
            command(["ditto", source, installed])
            command(["xattr", "-w", "com.apple.quarantine", quarantine, installed])
            command(["spctl", "--assess", "--type", "execute", installed])
            if not command(["xattr", "-p", "com.apple.quarantine", installed]).strip():
                raise ValueError("App Gatekeeper check lost the quarantine attribute")
        finally:
            command(["hdiutil", "detach", volume])


def finish_distribution(app, dmg, plan, environment=None):
    environment = dict(os.environ if environment is None else environment)
    notary_arguments(environment)
    app, dmg = Path(app), Path(dmg)
    verified = inspect_signed_app(app)
    if plan["identifier"] != IDENTIFIERS["production"] or plan["profile_sha256"] != verified["profile_sha256"]:
        raise ValueError("Signing plan does not match the final canonical app")
    dmg.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pennel-distribution-", dir=dmg.parent) as temporary:
        temporary = Path(temporary)
        archive = temporary / "Pennel.zip"
        command(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive])
        app_submission = notarize(archive, environment)
        command(["xcrun", "stapler", "staple", app])
        command(["xcrun", "stapler", "validate", app])
        stage = temporary / "stage"
        stage.mkdir()
        command(["ditto", app, stage / "Pennel.app"])
        (stage / "Applications").symlink_to("/Applications")
        command(["hdiutil", "create", "-quiet", "-volname", "Pennel", "-srcfolder", stage, "-ov", "-format", "UDZO", dmg])
        command(["codesign", "--force", "--timestamp", "--sign", plan["identity"], dmg])
        dmg_submission = notarize(dmg, environment)
        command(["xcrun", "stapler", "staple", dmg])
    command(["codesign", "--verify", "--strict", "-R", "=" + release_requirement(), dmg])
    command(["xcrun", "stapler", "validate", dmg])
    command(["spctl", "--assess", "--type", "execute", app])
    quarantine_assessment(dmg, verified)
    plist = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    return {**verified, "version": plist["CFBundleShortVersionString"], "app_notary_submission": app_submission,
            "dmg_notary_submission": dmg_submission, "notarization": "accepted", "app_ticket": "passed", "dmg_ticket": "passed",
            "gatekeeper": "passed", "quarantine_assessment": "passed", "dmg_file": dmg.name,
            "dmg_sha256": hashlib.sha256(dmg.read_bytes()).hexdigest(), "app_launched": False, "runtime_acceptance": "pending"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--flavor", choices=IDENTIFIERS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build", action="store_true", help="Build and inspect a local app with the prepared signing configuration")
    args = parser.parse_args()
    try:
        plan = build_preview(args.output) if args.build and args.flavor == "preview" else prepare(args.flavor, args.output)
        if args.build and args.flavor == "production":
            arguments = [str(ROOT / "scripts/with-toolchain.sh"), "npx", "tauri", "build", "--config", plan["config"]]
            guard = Path.home() / ".local/bin/worktree-storage"
            if guard.is_file():
                arguments = [str(guard), "run", "--", *arguments]
            subprocess.run(arguments, cwd=ROOT / "apps/desktop", env=build_environment(os.environ, plan["identity"]), check=True)
            inspect_signed_app(ROOT / "apps/desktop/src-tauri/target/release/bundle/macos/Pennel.app")
        print(plan["config"])
    except (ValueError, OSError, plistlib.InvalidFileException, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Mac signing preparation failed: {error}\n")


if __name__ == "__main__":
    main()
