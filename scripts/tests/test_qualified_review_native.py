import contextlib
from decimal import Decimal
import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("qualified_review_native", ROOT / "scripts/verify-qualified-review-native.py")
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class QualifiedReviewNativeTests(unittest.TestCase):
    def test_execution_is_explicit_before_platform_or_credential_access(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as stopped:
            native.main(["--window-helper", "missing", "--credential-bridge", "missing", "--output", "missing"])
        self.assertEqual(stopped.exception.code, 2)

    def test_fixture_contains_only_the_fixed_synthetic_development_panel(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary)
            identifier, metadata = native.write_fixture(profile)
            meeting = profile / "Meetings" / identifier
            spec = native.corpus.experiment()
            expected = next(value for value in spec["fixtures"] if value["id"] == spec["resource_fixture_id"])
            lines = (meeting / "transcript.md").read_text().splitlines()
            self.assertEqual(len(lines), 16)
            self.assertEqual([line.split(":** ", 1)[1] for line in lines], [row["text"] for row in expected["utterances"]])
            self.assertEqual((meeting / "notes.md").read_text(), native.NOTES)
            self.assertIn("pennel_stt_language: nl", (meeting / "meeting.md").read_text())
            self.assertEqual(metadata["id"], spec["resource_fixture_id"])
            self.assertFalse(metadata["quality_credit"])

    def test_billing_requires_a_stopped_fully_reported_finite_cost(self):
        job = {"billing_complete": True, "physical_running": False, "reported_billed_cost_usd": 0.0004}
        self.assertEqual(native.billed_cost(job), Decimal("0.0004"))
        for update in ({"billing_complete": False}, {"physical_running": True},
                       {"reported_billed_cost_usd": None}, {"reported_billed_cost_usd": True},
                       {"reported_billed_cost_usd": -1}, {"reported_billed_cost_usd": float("nan")},
                       {"reported_billed_cost_usd": float("inf")}):
            with self.subTest(update=update):
                self.assertIsNone(native.billed_cost(job | update))

    def test_capture_process_detection_covers_other_worktree_bundles(self):
        self.assertTrue(native.capture_is_running(
            "123 /another/worktree/Pennel Capture Preview.app/Contents/MacOS/pennel\n"))
        self.assertFalse(native.capture_is_running(
            "123 /Applications/Pennel Preview.app/Contents/MacOS/pennel\n456 /bin/zsh\n"))

    def test_credential_read_timeout_is_bounded_and_sanitized(self):
        timeout = subprocess.TimeoutExpired(["security"], 15, output=b"owned-fake-secret")
        with mock.patch.object(native.subprocess, "run", side_effect=timeout) as command:
            with self.assertRaisesRegex(ValueError, "within its bound") as error:
                native.evaluation_credential()
        self.assertNotIn("owned-fake-secret", str(error.exception))
        self.assertEqual(command.call_args.kwargs["timeout"], 15)

    def test_capture_credential_timeout_is_bounded_and_sanitized(self):
        timeout = subprocess.TimeoutExpired(["bridge"], 15, output=b"owned-fake-secret")
        with mock.patch.object(native.subprocess, "run", side_effect=timeout) as command:
            with self.assertRaisesRegex(ValueError, "within its bound") as error:
                native.capture_credential(Path("bridge"), "set", "review96-owned-test", Path("Capture.app"), io.StringIO(), b"owned-fake-secret")
        self.assertNotIn("owned-fake-secret", str(error.exception))
        self.assertEqual(command.call_args.kwargs["timeout"], 15)
        self.assertNotIn(b"owned-fake-secret", command.call_args.args[0])

    @unittest.skipUnless(shutil.which("node"), "Node is required to parse the native checkpoint JavaScript")
    def test_every_native_checkpoint_is_valid_javascript_after_expansion(self):
        values = {"account": "review96-owned-test", "appearance": "dark", "locale": "en", "model": native.MODEL,
                  "meeting_id": "owned", "notes": native.NOTES,
                  "expected": {"qualification_id": "real-record", "item_count": 15, "accepted_id": "owned-item"}}
        scripts = [native.expand(getattr(native, name), **values)
                   for name in ("OBSERVE", "SETUP", "PRESET", "OPEN", "START", "CHECK", "ACCEPT", "REOPEN")]
        check = "const scripts=JSON.parse(require('node:fs').readFileSync(0,'utf8'));for(const body of scripts)new Function('return(async()=>{'+body+'})()');"
        result = subprocess.run(["node", "-e", check], input=json.dumps(scripts), capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
