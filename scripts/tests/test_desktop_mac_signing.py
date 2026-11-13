import base64
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("mac_signing", SCRIPTS / "mac_signing.py")
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)
CERTIFICATE = b"synthetic public certificate"
FINGERPRINT = hashlib.sha1(CERTIFICATE).hexdigest().upper()
NAME = "Developer ID Application: Personal Developer (B538Q2K76C)"
PREFIX = "OLDPREFIX1"
NOW = datetime(2026, 11, 4, tzinfo=timezone.utc)


def profile(identifier="dev.pennel.Pennel", *, development=False):
    claims = {
        "com.apple.application-identifier": f"{PREFIX}.{identifier}",
        "com.apple.developer.team-identifier": signing.PERSONAL_TEAM,
        "keychain-access-groups": [f"{PREFIX}.*"],
    }
    return {"UUID": "synthetic-profile", "Platform": ["OSX"], "TeamIdentifier": [signing.PERSONAL_TEAM],
            "ApplicationIdentifierPrefix": [PREFIX], "ExpirationDate": datetime(2099, 1, 1),
            "DeveloperCertificates": [CERTIFICATE], "Entitlements": claims,
            **({"ProvisionedDevices": ["synthetic-test-Mac"]} if development else {"ProvisionsAllDevices": True})}


class MacSigningPolicyTests(unittest.TestCase):
    def test_missing_ad_hoc_work_and_ambiguous_identities_are_refused(self):
        work = ("F" * 40, "Developer ID Application: Work (WORKTEAM01)")
        available = [work, (FINGERPRINT, NAME)]
        self.assertEqual(signing.choose_identity(available), (FINGERPRINT, NAME))
        for request in ("-", work[0], "missing"):
            with self.subTest(request=request), self.assertRaises(ValueError):
                signing.choose_identity(available, request)
        with self.assertRaises(ValueError):
            signing.choose_identity([])
        with self.assertRaises(ValueError):
            signing.choose_identity(available + [("A" * 40, NAME)])

    def test_production_refuses_development_certificate_and_preview_stays_personal(self):
        dev = (FINGERPRINT, "Apple Development: Example Developer (CNIDENT001)")
        with self.assertRaises(ValueError):
            signing.choose_identity([dev])
        self.assertEqual(signing.choose_identity([dev], FINGERPRINT, preview=True), dev)
        with self.assertRaises(ValueError):
            signing.choose_identity([dev], preview=True)

    def test_profile_prefix_is_derived_and_generated_group_is_exact(self):
        attributes = signing.validate_profile(profile(), "dev.pennel.Pennel", CERTIFICATE, now=NOW)
        self.assertEqual(attributes["application_identifier"], f"{PREFIX}.dev.pennel.Pennel")
        self.assertEqual(attributes["keychain_access_group"], attributes["application_identifier"])
        preview = signing.validate_profile(profile("dev.pennel.Pennel.preview"), "dev.pennel.Pennel.preview", CERTIFICATE, now=NOW)
        self.assertNotEqual(attributes["keychain_access_group"], preview["keychain_access_group"])

    def test_profile_team_app_certificate_platform_group_and_expiry_are_required(self):
        mutations = [
            lambda p: p.update(TeamIdentifier=["WORKTEAM01"]),
            lambda p: p.update(ExpirationDate=datetime(2020, 1, 1)),
            lambda p: p.update(Platform=["iOS"]),
            lambda p: p.update(DeveloperCertificates=[b"another public certificate"]),
            lambda p: p.update(ApplicationIdentifierPrefix=[PREFIX, "B538Q2K76C"]),
            lambda p: p["Entitlements"].update({"com.apple.application-identifier": f"{PREFIX}.*"}),
            lambda p: p["Entitlements"].update({"com.apple.application-identifier": f"{PREFIX}.dev.pennel.Pennel.preview"}),
            lambda p: p["Entitlements"].update({"keychain-access-groups": [f"{PREFIX}.dev.pennel.Pennel.preview"]}),
            lambda p: p["Entitlements"].update({"com.apple.developer.team-identifier": "WORKTEAM01"}),
            lambda p: p["Entitlements"].update({"get-task-allow": True}),
        ]
        for mutate in mutations:
            value = profile()
            mutate(value)
            with self.subTest(profile=value), self.assertRaises(ValueError):
                signing.validate_profile(value, "dev.pennel.Pennel", CERTIFICATE, now=NOW)

    def test_development_profile_is_not_a_distributable_profile(self):
        value = profile("dev.pennel.Pennel.preview", development=True)
        with self.assertRaises(ValueError):
            signing.validate_profile(value, "dev.pennel.Pennel.preview", CERTIFICATE, now=NOW)
        attributes = signing.validate_profile(value, "dev.pennel.Pennel.preview", CERTIFICATE, developer_id=False, now=NOW)
        self.assertTrue(attributes["keychain_access_group"].endswith(".preview"))

    def test_preparation_requires_profile_before_looking_for_any_identity(self):
        with mock.patch.object(signing, "command") as command, self.assertRaisesRegex(ValueError, "PROVISIONING_PROFILE"):
            signing.prepare("production", "/unused", {})
        command.assert_not_called()

    def test_preparation_writes_profile_before_signing_overlay_and_never_adds_sandbox(self):
        pem = b"-----BEGIN CERTIFICATE-----\n" + base64.b64encode(CERTIFICATE) + b"\n-----END CERTIFICATE-----"
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            source = temporary / "source.provisionprofile"
            source.write_bytes(b"synthetic CMS profile")
            environment = {"PENNEL_MAC_PROVISIONING_PROFILE": str(source)}
            def execute(arguments, **kwargs):
                if arguments[1] == "find-identity":
                    return f'  1) {"F" * 40} "Developer ID Application: Another Personal Certificate (B538Q2K76C)"\n  2) {FINGERPRINT} "{NAME}"\n'
                if arguments[1] == "find-certificate":
                    return pem
                raise AssertionError("Preparation must not sign, build or contact the notary")
            with mock.patch.object(signing, "command", side_effect=execute), mock.patch.object(signing, "decode_profile", return_value=profile()), mock.patch.object(signing, "validate_certificate", return_value=True):
                result = signing.prepare("production", temporary / "prepared", environment)
            overlay = json.loads(Path(result["config"]).read_text())["bundle"]["macOS"]
            claims = plistlib.loads(Path(overlay["entitlements"]).read_bytes())
            embedded_source = Path(overlay["files"]["embedded.provisionprofile"])
            self.assertEqual(embedded_source.read_bytes(), source.read_bytes())
            self.assertEqual(claims["keychain-access-groups"], [f"{PREFIX}.dev.pennel.Pennel"])
            self.assertNotIn("com.apple.security.app-sandbox", claims)
            self.assertNotIn("com.apple.security.get-task-allow", claims)
            self.assertNotIn("*", json.dumps(claims))

    def test_invalid_cms_cannot_be_used_as_decoded_profile(self):
        def execute(arguments, **kwargs):
            if arguments[0] == "security":
                return b"system Apple roots"
            raise ValueError("CMS signature invalid")
        with mock.patch.object(signing, "command", side_effect=execute), self.assertRaisesRegex(ValueError, "CMS"):
            signing.decode_profile(Path("tampered.provisionprofile"))

    def test_certificate_requires_personal_subject_actual_apple_class_and_trusted_root(self):
        for subject, details in (("subject=OU=WORKTEAM01,CN=Developer", signing.DEVELOPER_ID_OID),
                                 (f"subject=OU={signing.PERSONAL_TEAM},CN=Developer", signing.DEVELOPMENT_OID)):
            with self.subTest(subject=subject, details=details), mock.patch.object(signing, "certificate_info", return_value=(subject, details)), self.assertRaises(ValueError):
                signing.validate_certificate(CERTIFICATE)
        fake_root = b"-----BEGIN CERTIFICATE-----\n" + base64.b64encode(b"untrusted root") + b"\n-----END CERTIFICATE-----"
        with mock.patch.object(signing, "certificate_info", return_value=(f"subject=OU={signing.PERSONAL_TEAM},CN=Developer", signing.DEVELOPER_ID_OID)), mock.patch.object(signing, "command", side_effect=[fake_root, b""]), self.assertRaisesRegex(ValueError, "system Apple root"):
            signing.validate_certificate(CERTIFICATE)

    def test_build_does_not_implicitly_notarize_or_reimport_a_different_identity(self):
        original = {key: "sensitive value" for key in (*signing.NOTARY_ENV, "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD")}
        original["PATH"] = "/existing/tools"
        result = signing.build_environment(original, FINGERPRINT)
        self.assertEqual(result, {"PATH": "/existing/tools", "APPLE_SIGNING_IDENTITY": FINGERPRINT})
        self.assertIn("APPLE_CERTIFICATE", original)

    def test_signed_application_must_claim_only_the_profile_authorized_private_group(self):
        with tempfile.TemporaryDirectory() as temporary:
            app = Path(temporary) / "Pennel.app"
            (app / "Contents").mkdir(parents=True)
            (app / "Contents/embedded.provisionprofile").write_bytes(b"synthetic profile")
            claims = {"com.apple.application-identifier": f"{PREFIX}.dev.pennel.Pennel",
                      "com.apple.developer.team-identifier": signing.PERSONAL_TEAM,
                      "keychain-access-groups": [f"{PREFIX}.dev.pennel.Pennel"]}
            metadata = f"Authority=Developer ID Application: Personal Developer\nTeamIdentifier={signing.PERSONAL_TEAM}\nCDHash={'a' * 40}\n"
            def execute(arguments, **kwargs):
                if "-R" in arguments:
                    self.assertTrue(arguments[arguments.index("-R") + 1].startswith("="), "codesign treats unprefixed requirements as filenames")
                if "--entitlements" in arguments:
                    return plistlib.dumps(claims)
                for argument in arguments:
                    if str(argument).startswith("--extract-certificates="):
                        Path(str(argument).partition("=")[2] + "0").write_bytes(CERTIFICATE)
                return ""
            with mock.patch.object(signing.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "", metadata)), mock.patch.object(signing, "command", side_effect=execute), mock.patch.object(signing, "decode_profile", return_value=profile()):
                verified = signing.inspect_signed_app(app)
                self.assertEqual(verified["keychain_access_group"], f"{PREFIX}.dev.pennel.Pennel")
                for groups in ([f"{PREFIX}.*"], [f"{PREFIX}.dev.pennel.Pennel.preview"],
                               [f"{PREFIX}.dev.pennel.Pennel", f"{PREFIX}.another-app"]):
                    claims["keychain-access-groups"] = groups
                    with self.subTest(groups=groups), self.assertRaisesRegex(ValueError, "signed Keychain entitlements"):
                        signing.inspect_signed_app(app)

    def test_notarization_does_not_need_windows_or_updater_credentials(self):
        environment = {"PENNEL_NOTARY_KEYCHAIN_PROFILE": "personal-notary"}
        self.assertEqual(signing.notary_arguments(environment), ["--keychain-profile", "personal-notary"])
        with self.assertRaises(ValueError):
            signing.notary_arguments({**environment, "APPLE_TEAM_ID": "WORKTEAM01"})
        with self.assertRaises(ValueError):
            signing.notary_arguments({})

    def test_notary_rejection_and_command_errors_never_expose_credentials(self):
        with mock.patch.object(signing, "command", return_value=json.dumps({"status": "Invalid"})), self.assertRaisesRegex(ValueError, "did not accept"):
            signing.notarize(Path("artifact.dmg"), {"PENNEL_NOTARY_KEYCHAIN_PROFILE": "personal-notary"})
        secret = "sensitive-notary-password"
        failed = subprocess.CompletedProcess([], 1, secret, secret)
        with mock.patch.object(signing.subprocess, "run", return_value=failed), self.assertRaises(ValueError) as raised:
            signing.command(["xcrun", "notarytool", "--password", secret])
        self.assertNotIn(secret, str(raised.exception))

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("codesign") and Path("/System/Applications/Calculator.app").is_dir(), "Requires the macOS signature inspection tool and a system signed application")
    def test_actual_codesign_extracts_public_certificate_to_the_requested_location(self):
        certificate = signing.signed_certificate(Path("/System/Applications/Calculator.app"))
        self.assertGreater(len(certificate), 128)
        self.assertEqual(certificate[0], 0x30)


class PreviewFirstBuildTests(unittest.TestCase):
    def test_first_tauri_build_uses_validated_overlay_and_sanitized_environment(self):
        plan = {"identity": FINGERPRINT, "config": "/prepared/tauri.signing.conf.json",
                "identifier": "dev.pennel.Pennel.preview", "developer_id": True}
        environment = {"APPLE_SIGNING_IDENTITY": FINGERPRINT, "APPLE_CERTIFICATE": "inherited-secret",
                       "APPLE_CERTIFICATE_PASSWORD": "inherited-secret", "APPLE_PASSWORD": "inherited-secret",
                       "PENNEL_SIGN_IDENTITY": "inherited-other-identity", "PENNEL_NOTARY_KEYCHAIN_PROFILE": "inherited-notary"}
        events = []
        def prepare(*arguments):
            events.append("validated")
            return plan
        def run(arguments, **kwargs):
            self.assertEqual(events, ["validated"])
            self.assertEqual(kwargs["env"], {"APPLE_SIGNING_IDENTITY": FINGERPRINT})
            self.assertEqual(arguments[-4:], ["--bundles", "app", "--config", plan["config"]])
            events.append("tauri")
        with mock.patch.object(signing, "prepare", side_effect=prepare), mock.patch.object(signing.subprocess, "run", side_effect=run) as build, mock.patch.object(signing, "inspect_signed_app") as inspect:
            self.assertEqual(signing.build_preview("/prepared", environment), plan)
        self.assertEqual(build.call_count, 1)
        inspect.assert_called_once()
        self.assertEqual(events, ["validated", "tauri"])

    def test_explicit_wrong_team_is_rejected_before_any_tauri_invocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "preview.provisionprofile"
            source.write_bytes(b"synthetic authenticated profile")
            environment = {"PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE": str(source),
                           "APPLE_SIGNING_IDENTITY": "Developer ID Application: Work Developer (WORKTEAM01)"}
            available = f'1) {FINGERPRINT} "{NAME}"\n2) {"F" * 40} "Developer ID Application: Work Developer (WORKTEAM01)"\n'
            with mock.patch.object(signing, "decode_profile", return_value=profile("dev.pennel.Pennel.preview")), mock.patch.object(signing, "command", return_value=available), mock.patch.object(signing.subprocess, "run") as build, self.assertRaises(ValueError):
                signing.build_preview(Path(temporary) / "prepared", environment)
            build.assert_not_called()

    def test_isolated_adhoc_ci_has_no_profile_claims_or_inherited_signing_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            environment = {"CI": "true", "PENNEL_ALLOW_ADHOC_PREVIEW": "1", "APPLE_SIGNING_IDENTITY": "inherited-other-identity",
                           "APPLE_CERTIFICATE": "inherited-secret", "APPLE_CERTIFICATE_PASSWORD": "inherited-secret",
                           "PENNEL_SIGN_IDENTITY": "inherited-other-identity"}
            with mock.patch.object(signing, "prepare") as prepare, mock.patch.object(signing.subprocess, "run") as build, mock.patch.object(signing, "inspect_signed_app") as inspect:
                plan = signing.build_preview(temporary, environment)
            prepare.assert_not_called()
            inspect.assert_not_called()
            self.assertEqual(build.call_count, 1)
            self.assertEqual(build.call_args.kwargs["env"], {"CI": "true", "PENNEL_ALLOW_ADHOC_PREVIEW": "1", "APPLE_SIGNING_IDENTITY": "-"})
            overlay = json.loads(Path(plan["config"]).read_text())
            self.assertEqual(overlay["productName"], "Pennel Preview")
            self.assertFalse(overlay["bundle"]["createUpdaterArtifacts"])
            self.assertEqual(overlay["bundle"]["macOS"]["files"], {})
            self.assertEqual(overlay["bundle"]["macOS"]["signingIdentity"], "-")

    def test_adhoc_outside_ci_is_rejected_before_preparing_or_building(self):
        with mock.patch.object(signing, "prepare") as prepare, mock.patch.object(signing.subprocess, "run") as build, self.assertRaises(ValueError):
            signing.build_preview("/unused", {"PENNEL_ALLOW_ADHOC_PREVIEW": "1"})
        prepare.assert_not_called()
        build.assert_not_called()

    def test_macos_entry_script_reaches_preparation_before_any_npx_invocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            programs = {
                "uname": "printf 'Darwin\\n'\n",
                "npm": "exit 0\n",
                "python3": "printf 'prepare\\n' >> \"$PENNEL_TEST_TRACE\"\nexit 42\n",
                "npx": "printf 'unvalidated npx\\n' >> \"$PENNEL_TEST_TRACE\"\nexit 43\n",
            }
            for name, body in programs.items():
                program = temporary / name
                program.write_text("#!/bin/sh\n" + body)
                program.chmod(0o755)
            trace = temporary / "trace"
            environment = {**os.environ, "PATH": str(temporary) + os.pathsep + os.environ["PATH"], "PENNEL_TEST_TRACE": str(trace)}
            result = subprocess.run(["bash", str(SCRIPTS / "build-desktop-preview.sh")], env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 42)
            self.assertEqual(trace.read_text(), "prepare\n")


class FinalDistributionTests(unittest.TestCase):
    def test_final_hash_is_after_both_staples_and_rejected_artifact_has_no_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            app = temporary / "Pennel.app"
            (app / "Contents").mkdir(parents=True)
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleShortVersionString": "0.2.0"}))
            dmg = temporary / "Pennel_0.2.0_aarch64.dmg"
            verified = {"profile_sha256": "a" * 64, "app_cdhash": "b" * 40}
            plan = {"identifier": "dev.pennel.Pennel", "profile_sha256": "a" * 64, "identity": FINGERPRINT}
            events = []
            def execute(arguments, **kwargs):
                events.append(tuple(str(arg) for arg in arguments))
                if "-R" in arguments:
                    self.assertTrue(arguments[arguments.index("-R") + 1].startswith("="), "codesign requires inline requirement syntax")
                if arguments[:2] == ["hdiutil", "create"]:
                    dmg.write_bytes(b"signed image before ticket")
                if arguments[:3] == ["xcrun", "stapler", "staple"] and Path(arguments[3]) == dmg:
                    dmg.write_bytes(dmg.read_bytes() + b" + final stapled ticket")
                return ""
            with mock.patch.object(signing, "command", side_effect=execute), mock.patch.object(signing, "inspect_signed_app", return_value=verified), mock.patch.object(signing, "notarize", side_effect=["app-submission", "dmg-submission"]) as notarize, mock.patch.object(signing, "quarantine_assessment") as quarantine:
                receipt = signing.finish_distribution(app, dmg, plan, {"PENNEL_NOTARY_KEYCHAIN_PROFILE": "personal-notary"})
            self.assertEqual(notarize.call_count, 2)
            self.assertEqual(receipt["dmg_sha256"], hashlib.sha256(b"signed image before ticket + final stapled ticket").hexdigest())
            self.assertLess(events.index(("xcrun", "stapler", "staple", str(app))), next(i for i, event in enumerate(events) if event[:2] == ("hdiutil", "create")))
            quarantine.assert_called_once_with(dmg, verified)
            with mock.patch.object(signing, "command", side_effect=execute), mock.patch.object(signing, "inspect_signed_app", return_value=verified), mock.patch.object(signing, "notarize", side_effect=ValueError("notary rejected")), self.assertRaisesRegex(ValueError, "notary rejected"):
                signing.finish_distribution(app, dmg, plan, {"PENNEL_NOTARY_KEYCHAIN_PROFILE": "personal-notary"})


if __name__ == "__main__":
    unittest.main()
