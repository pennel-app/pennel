import importlib.util
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location('testflight_checks', Path(__file__).parents[1] / 'mobile-testflight-checks.py')
MODULE = importlib.util.module_from_spec(SPEC)


def run(identifier=1, sha='a' * 40, state='completed', conclusion='success', repository='pennel-app/pennel'):
    return {'id': identifier, 'head_sha': sha, 'event': 'workflow_dispatch', 'status': state, 'conclusion': conclusion,
            'head_repository': {'full_name': repository}, 'path': '.github/workflows/mobile.yml',
            'html_url': f'https://github.com/pennel-app/pennel/actions/runs/{identifier}'}


class ChecksTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        SPEC.loader.exec_module(MODULE)

    def test_exact_source_and_repository_are_required(self):
        candidates = [run(1), run(2, sha='b' * 40), run(3, repository='someone/fork')]
        self.assertEqual(MODULE.latest_run(candidates, 'pennel-app/pennel', 'a' * 40, 'mobile.yml')['id'], 1)

    def test_newer_failure_cannot_be_hidden_by_old_success(self):
        self.assertEqual(MODULE.latest_run([run(1), run(2, conclusion='failure')], 'pennel-app/pennel', 'a' * 40, 'mobile.yml')['conclusion'], 'failure')

    def test_wrong_workflow_cannot_satisfy_gate(self):
        candidate = run()
        candidate['path'] = '.github/workflows/other.yml'
        self.assertIsNone(MODULE.latest_run([candidate], 'pennel-app/pennel', 'a' * 40, 'mobile.yml'))

    def test_pr_merge_checks_cannot_attest_the_selected_source(self):
        candidate = run()
        candidate['event'] = 'pull_request'
        self.assertIsNone(MODULE.latest_run([candidate], 'pennel-app/pennel', 'a' * 40, 'mobile.yml'))

    def test_reuse_all_successes_without_mutation(self):
        class API:
            def runs(self, workflow, sha):
                candidate = run()
                candidate['path'] = '.github/workflows/' + workflow
                return [candidate]
            def post(self, *args):
                raise AssertionError('Already validated source must not trigger work')
        receipt = MODULE.ensure_checks(API(), 'pennel-app/pennel', 'a' * 40, timeout=1)
        self.assertEqual(receipt['status'], 'success')
        self.assertEqual(receipt['source_sha'], 'a' * 40)
        self.assertEqual(len(receipt['checks']), 3)

    def test_missing_checks_dispatch_immutable_source_tag(self):
        class API:
            def __init__(self):
                self.started = set()
                self.posts = []
            def runs(self, workflow, sha):
                if workflow not in self.started:
                    return []
                candidate = run()
                candidate['path'] = '.github/workflows/' + workflow
                return [candidate]
            def pin_source(self, sha):
                return 'pennel-testflight-validation/' + sha
            def post(self, endpoint, body=None):
                self.posts.append((endpoint, body))
                self.started.add(endpoint.split('/')[2])
        api = API()
        receipt = MODULE.ensure_checks(api, 'pennel-app/pennel', 'a' * 40, timeout=10, sleep=lambda _: None)
        self.assertEqual(receipt['status'], 'success')
        self.assertEqual(len(api.posts), 3)
        self.assertTrue(all(body['ref'] == 'pennel-testflight-validation/' + 'a' * 40 for _, body in api.posts))

    def test_repeated_failure_stops_without_receipt(self):
        class API:
            attempt = 1
            def runs(self, workflow, sha):
                candidate = run(conclusion='failure')
                candidate['run_attempt'] = self.attempt
                candidate['path'] = '.github/workflows/' + workflow
                return [candidate]
            def post(self, *args):
                self.attempt += 1
        with self.assertRaisesRegex(RuntimeError, 'failed'):
            MODULE.ensure_checks(API(), 'pennel-app/pennel', 'a' * 40, timeout=1, sleep=lambda _: None)

    def test_eventually_consistent_rerun_waits_for_new_attempt(self):
        class API:
            def __init__(self):
                self.observations = {}
                self.retries = 0
            def runs(self, workflow, sha):
                count = self.observations.get(workflow, 0) + 1
                self.observations[workflow] = count
                candidate = run(conclusion='success' if count >= 3 else 'failure')
                candidate['run_attempt'] = 2 if count >= 3 else 1
                candidate['path'] = '.github/workflows/' + workflow
                return [candidate]
            def post(self, *args):
                self.retries += 1
        api = API()
        receipt = MODULE.ensure_checks(api, 'pennel-app/pennel', 'a' * 40, timeout=1, sleep=lambda _: None)
        self.assertEqual(receipt['status'], 'success')
        self.assertEqual(api.retries, 3)

    def test_fork_source_and_generated_source_are_rejected(self):
        for value in ['https://github.com/a/b', 'codex/ios-testflight', 'refs/heads/codex/ios-testflight', '../bad']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.validate_ref(value)

    def test_transient_read_retries_without_losing_successful_checks(self):
        failure = SimpleNamespace(returncode=1, stdout='', stderr='gh: service unavailable (HTTP 503)')
        success = SimpleNamespace(returncode=0, stdout='{"ok": true}', stderr='')
        with patch.object(MODULE.subprocess, 'run', side_effect=[failure, success]) as command, patch.object(MODULE.time, 'sleep'):
            self.assertEqual(MODULE.GitHub('pennel-app/pennel').request('actions/workflows/ci.yml/runs'), {'ok': True})
            self.assertEqual(command.call_count, 2)

    def test_mutating_calls_and_permission_errors_are_not_blindly_retried(self):
        for method, status in [('POST', 503), ('GET', 403)]:
            failure = SimpleNamespace(returncode=1, stdout='', stderr=f'gh: refused (HTTP {status})')
            with self.subTest(method=method), patch.object(MODULE.subprocess, 'run', return_value=failure) as command:
                with self.assertRaisesRegex(RuntimeError, str(status)):
                    MODULE.GitHub('pennel-app/pennel').request('actions/workflows/ci.yml/dispatches', method, {})
                self.assertEqual(command.call_count, 1)


if __name__ == '__main__':
    unittest.main()
