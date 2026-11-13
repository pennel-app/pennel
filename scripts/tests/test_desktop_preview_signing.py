import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "preview-signing-identity.py"
SPEC = importlib.util.spec_from_file_location("preview_signing_identity", SCRIPT)
signing = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = signing
SPEC.loader.exec_module(signing)

FIRST = "A" * 40
SECOND = "B" * 40
LOCAL = "C" * 40
WORK = "E" * 40
PERSONAL_TEAM = "B538Q2K76C"
SECURITY_OUTPUT = f'''  1) {WORK} "Developer ID Application: Work Developer (WORKTEAM01)"
  2) {FIRST} "Developer ID Application: First Developer ({PERSONAL_TEAM})"
  3) {SECOND} "Developer ID Application: Second Developer ({PERSONAL_TEAM})"
     2 valid identities found
'''


class PreviewSigningIdentityTests(unittest.TestCase):
    def setUp(self):
        self.identities = signing.parse_identities(SECURITY_OUTPUT)

    def test_first_personal_certificate_is_used_even_when_work_identity_appears_first(self):
        selection = signing.select_identity(self.identities)
        self.assertEqual(selection.identity, FIRST)
        self.assertEqual(selection.sha1, FIRST)

    def test_cached_certificate_survives_a_change_in_identity_order(self):
        selection = signing.select_identity(list(reversed(self.identities)), cached_sha=FIRST.lower())
        self.assertEqual(selection.identity, FIRST)

    def test_stale_cache_falls_back_to_a_current_valid_certificate(self):
        selection = signing.select_identity(self.identities, cached_sha="D" * 40)
        self.assertEqual(selection.identity, FIRST)

    def test_self_signed_and_work_certificates_cannot_replace_personal_identity(self):
        local = signing.Identity(LOCAL, "Pennel Local Dev")
        selection = signing.select_identity(self.identities + [local])
        self.assertEqual(selection.identity, FIRST)
        self.assertEqual(signing.select_identity(self.identities + [local], cached_sha=SECOND).identity, SECOND)
        self.assertEqual(signing.select_identity(self.identities, cached_sha=WORK).identity, FIRST)
        with self.assertRaises(ValueError):
            signing.select_identity(self.identities, apple_override=WORK)

    def test_apple_override_precedes_pennel_and_cache(self):
        selection = signing.select_identity(
            self.identities, cached_sha=SECOND,
            apple_override=f"Developer ID Application: First Developer ({PERSONAL_TEAM})", pennel_override=SECOND,
        )
        self.assertEqual(selection.identity, FIRST)
        self.assertEqual(selection.sha1, FIRST)

    def test_pennel_override_and_explicit_adhoc_are_preserved(self):
        self.assertEqual(signing.select_identity(self.identities, pennel_override=SECOND).identity, SECOND)
        with self.assertRaises(ValueError):
            signing.select_identity(self.identities, cached_sha=FIRST, apple_override="-")
        adhoc = signing.select_identity(self.identities, cached_sha=FIRST, apple_override="-", allow_adhoc=True)
        self.assertEqual(adhoc.identity, "-")
        self.assertIsNone(adhoc.sha1)

    def test_unknown_explicit_identity_is_not_replaced_silently(self):
        with self.assertRaises(ValueError):
            signing.select_identity(self.identities, apple_override="Requested release identity")

    def test_missing_certificate_requires_explicit_adhoc_ci_opt_in(self):
        with self.assertRaises(ValueError):
            signing.select_identity([])
        selection = signing.select_identity([], allow_adhoc=True)
        self.assertEqual(selection.identity, "-")
        self.assertIsNone(selection.sha1)

    def test_parser_rejects_invalid_or_unrecognized_identity_entries(self):
        output = SECURITY_OUTPUT + f'  3) {LOCAL} "Expired" (CSSMERR_TP_CERT_EXPIRED)\n  4) badhash "Invalid"\n'
        self.assertEqual([identity.sha1 for identity in signing.parse_identities(output)], [WORK, FIRST, SECOND])

    def test_cli_reads_valid_identities_and_atomically_persists_only_the_public_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory) / "preview-signing-identity.sha1"
            output = io.StringIO()
            with patch.dict(os.environ, {}, clear=True), patch.object(
                signing.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, SECURITY_OUTPUT, "")
            ) as security, patch.object(signing, "validate_selection") as validate, patch("sys.stdout", output):
                self.assertEqual(signing.main(["--cache", str(cache)]), 0)
            self.assertEqual(output.getvalue(), FIRST + "\n")
            self.assertEqual(cache.read_text(), FIRST + "\n")
            self.assertEqual(list(cache.parent.iterdir()), [cache])
            self.assertEqual(security.call_args.args[0], ["security", "find-identity", "-v", "-p", "codesigning"])
            validate.assert_called_once()

    def test_failed_security_lookup_reports_adhoc_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory) / "preview-signing-identity.sha1"
            output, errors = io.StringIO(), io.StringIO()
            with patch.dict(os.environ, {"CI": "true"}, clear=True), patch.object(
                signing.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "unavailable")
            ), patch("sys.stdout", output), patch("sys.stderr", errors):
                self.assertEqual(signing.main(["--cache", str(cache), "--allow-adhoc-ci"]), 0)
            self.assertEqual(output.getvalue(), "-\n")
            self.assertIn("ad-hoc", errors.getvalue())
            self.assertFalse(cache.exists())

    def test_adhoc_opt_in_is_refused_outside_ci(self):
        with patch.dict(os.environ, {}, clear=True), patch("sys.stderr", io.StringIO()), self.assertRaises(SystemExit):
            signing.main(["--allow-adhoc-ci"])

    def test_explicit_development_cn_is_validated_by_certificate_team_not_parentheses(self):
        development = signing.Identity(LOCAL, "Apple Development: Example Developer (CNIDENT001)")
        selected = signing.select_identity([development], apple_override=LOCAL)
        self.assertEqual(selected.sha1, LOCAL)
        with self.assertRaises(ValueError):
            signing.select_identity([development])
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory) / "identity.sha1"
            output = io.StringIO()
            with patch.dict(os.environ, {"APPLE_SIGNING_IDENTITY": LOCAL}, clear=True), patch.object(signing.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, f'1) {LOCAL} "{development.name}"\n', "")), patch.object(signing, "validate_selection", side_effect=ValueError("Signing certificate belongs to another team")), patch("sys.stdout", output), patch("sys.stderr", io.StringIO()), self.assertRaises(SystemExit):
                signing.main(["--cache", str(cache)])
            self.assertEqual(output.getvalue(), "")
            self.assertFalse(cache.exists())


if __name__ == "__main__":
    unittest.main()
