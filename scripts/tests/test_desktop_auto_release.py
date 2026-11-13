import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("desktop_auto_release", SCRIPTS / "desktop-auto-release.py")
auto = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auto)


class NextVersionTests(unittest.TestCase):
    def test_first_release_uses_the_configured_version(self):
        self.assertEqual(auto.next_version("0.1.0", None), "0.1.0")

    def test_patch_bumps_the_latest_tag(self):
        self.assertEqual(auto.next_version("0.1.0", "v0.1.7"), "0.1.8")

    def test_a_higher_configured_version_wins(self):
        self.assertEqual(auto.next_version("0.2.0", "v0.1.7"), "0.2.0")

    def test_a_configured_version_equal_to_the_bump_is_used_once(self):
        self.assertEqual(auto.next_version("0.1.8", "v0.1.7"), "0.1.8")
        self.assertEqual(auto.next_version("0.1.8", "v0.1.8"), "0.1.9")

    def test_latest_tag_compares_numerically_and_ignores_other_tags(self):
        tags = ["v0.1.9", "v0.1.10", "v0.2.0-beta.1", "candidate", "v0.1.2"]
        self.assertEqual(auto.latest_release_tag(tags), "v0.1.10")
        self.assertIsNone(auto.latest_release_tag(["nightly"]))


class CaskTests(unittest.TestCase):
    SHA = "a" * 64

    def receipt(self):
        return {"version": "0.1.8", "dmg_sha256": self.SHA, "dmg_file": "Pennel_0.1.8_aarch64.dmg",
                "bundle_identifier": "dev.pennel.Pennel", "team_identifier": "B538Q2K76C", "developer_id": True,
                "notarization": "accepted", "app_ticket": "passed", "dmg_ticket": "passed", "gatekeeper": "passed",
                "quarantine_assessment": "passed", "application_identifier": "B538Q2K76C.dev.pennel.Pennel",
                "keychain_access_group": "B538Q2K76C.dev.pennel.Pennel", "profile_sha256": "b" * 64,
                "profile_expires": "2099-01-01T00:00:00+00:00"}

    def test_cask_points_at_the_release_dmg(self):
        cask = auto.render_cask("0.1.8", self.SHA, "pennel-app/pennel")
        self.assertIn('version "0.1.8"', cask)
        self.assertIn(f'sha256 "{self.SHA}"', cask)
        self.assertIn("https://github.com/pennel-app/pennel/releases/download/v#{version}/Pennel_#{version}_aarch64.dmg", cask)
        self.assertIn('app "Pennel.app"', cask)
        self.assertIn("depends_on arch: :arm64", cask)

    def test_zap_never_removes_meetings_or_models(self):
        cask = auto.render_cask("0.1.8", self.SHA, "pennel-app/pennel")
        self.assertNotIn("Application Support", cask)

    def test_only_an_exact_verified_notarized_promotion_removes_quarantine_workaround(self):
        legacy = auto.render_cask("0.1.8", self.SHA, "pennel-app/pennel")
        self.assertIn("com.apple.quarantine", legacy)
        promoted = auto.render_cask("0.1.8", self.SHA, "pennel-app/pennel", notarization_receipt=self.receipt())
        self.assertNotIn("com.apple.quarantine", promoted)
        self.assertNotIn("postflight_steps", promoted)
        for key in ("dmg_sha256", "team_identifier", "notarization", "app_ticket", "dmg_ticket", "gatekeeper", "quarantine_assessment", "profile_sha256"):
            receipt = self.receipt()
            receipt[key] = "unaccepted"
            with self.subTest(gate=key), self.assertRaises(ValueError):
                auto.render_cask("0.1.8", self.SHA, "pennel-app/pennel", notarization_receipt=receipt)

    def test_expired_preview_other_version_and_rebuilt_artifacts_cannot_be_promoted(self):
        for changes in ({"profile_expires": "2020-01-01T00:00:00+00:00"},
                        {"keychain_access_group": "B538Q2K76C.dev.pennel.Pennel.preview"},
                        {"version": "0.1.9"}, {"dmg_file": "another.dmg"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                auto.render_cask("0.1.8", self.SHA, "pennel-app/pennel", notarization_receipt={**self.receipt(), **changes})

    def test_rejects_malformed_inputs(self):
        with self.assertRaises(ValueError):
            auto.render_cask("0.1.8", "ABC", "pennel-app/pennel")
        with self.assertRaises(ValueError):
            auto.render_cask("0.1", self.SHA, "pennel-app/pennel")


class PackagingReleaseSelectionTests(unittest.TestCase):
    def test_signing_only_changes_require_new_distributed_artifacts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def git(*arguments):
                subprocess.run(["git", *arguments], cwd=root, check=True, capture_output=True)
            git("init", "-q")
            git("config", "user.email", "fixture@example.invalid")
            git("config", "user.name", "Test fixture")
            source = root / "scripts/mac_signing.py"
            source.parent.mkdir()
            source.write_text("previous signing implementation\n")
            git("add", ".")
            git("commit", "-qm", "fixture")
            git("tag", "v0.1.0")
            with mock.patch.object(auto, "ROOT", root):
                self.assertFalse(auto.app_changed_since("v0.1.0"))
                source.write_text("corrected signing implementation\n")
                git("add", ".")
                git("commit", "-qm", "signing fix")
                self.assertTrue(auto.app_changed_since("v0.1.0"))


if __name__ == "__main__":
    unittest.main()
