import argparse
import contextlib
import io
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import credential_session as session


class CredentialSessionTests(unittest.TestCase):
    def test_one_bounded_os_read_stays_in_memory(self):
        secret = b'owned-fake-credential'
        result = subprocess.CompletedProcess([], 0, stdout=secret+b'\n', stderr=b'')
        with mock.patch.object(session.subprocess, 'run', return_value=result) as lookup:
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(session.read_key(), secret)
        self.assertEqual(output.getvalue(), '')
        self.assertEqual(lookup.call_args.kwargs['timeout'], 60)
        self.assertEqual(lookup.call_args.kwargs['stdin'], subprocess.DEVNULL)
        self.assertEqual(lookup.call_args.args[0], ['security', 'find-generic-password', '-s',
            'dev.pennel.evaluation.openrouter', '-a', 'OPENROUTER_API_KEY', '-w'])
        self.assertNotIn(secret, lookup.call_args.args[0])

    def test_os_error_never_exposes_credential_buffers(self):
        failure = subprocess.TimeoutExpired(['security'], 60, output=b'owned-fake-credential')
        with mock.patch.object(session.subprocess, 'run', side_effect=failure):
            with self.assertRaisesRegex(ValueError, 'os_read_unavailable') as error:
                session.read_key()
        self.assertNotIn('owned-fake-credential', str(error.exception))

    def test_child_can_only_run_explicit_frozen_or_native_phases(self):
        args = argparse.Namespace(bridge='bridge', pricing='prices', ledger='ledger',
            window_helper='window', credential_bridge='credentials')
        value = {'phase':'calibration', 'output':'owned', 'freeze':'frozen',
            'freeze_lock':'lock', 'freeze_commit':'a'*40}
        with mock.patch.object(session, 'owned_output', return_value='owned-output'):
            command = session.command(value, args, 9)
            self.assertIn('--credential-fd', command)
            self.assertEqual(command[command.index('--credential-fd')+1], '9')
            self.assertIn('--freeze-commit', command)
            self.assertEqual(Path(command[1]), session.ROOT/'evaluations/decision-models/finished_experiment.py')
            for invalid in (value|{'phase':'development'}, value|{'extra':'unexpected'},
                            {'phase':'native','output':'owned','arbitrary_command':'false'}):
                with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                    session.command(invalid, args, 9)

    def test_output_must_be_new_and_inside_the_owned_evidence_scope(self):
        for invalid in ('/tmp/unowned-review', 'build/review-functional-96', '../unowned'):
            with self.subTest(path=invalid), self.assertRaises(ValueError):
                session.owned_output(invalid)


if __name__ == '__main__':
    unittest.main()
