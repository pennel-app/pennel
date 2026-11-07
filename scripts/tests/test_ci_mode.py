import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'ci-mode.py'
REPOSITORY = 'pennel-app/pennel'
HEAD = 'a' * 40
MERGE = 'b' * 40


def load_module():
    spec = importlib.util.spec_from_file_location('ci_mode', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def config(**updates):
    value = {'repository': REPOSITORY, 'pr': 155, 'head_sha': HEAD,
             'mode': 'candidate', 'cohort': 'warm-candidate-1'}
    value.update(updates)
    return json.dumps(value)


def event(**updates):
    value = {'number': 155, 'pull_request': {
        'number': 155,
        'head': {'sha': HEAD, 'repo': {'full_name': REPOSITORY}},
        'base': {'repo': {'full_name': REPOSITORY}},
    }}
    value.update(updates)
    return value


def environment(**updates):
    value = {'GITHUB_EVENT_NAME': 'pull_request', 'GITHUB_REPOSITORY': REPOSITORY,
             'GITHUB_SHA': MERGE, 'GITHUB_RUN_ID': '100', 'GITHUB_RUN_ATTEMPT': '2',
             'GITHUB_REF': 'refs/pull/155/merge',
             'PENNEL_CI_QUALIFICATION_CONFIG': '', 'PENNEL_CI_FORCE_SERIAL': ''}
    value.update(updates)
    return value


class ModeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load_module()

    def select(self, env=None, payload=None):
        return self.module.select_mode(env or environment(), payload or event())

    def test_same_repository_pr_uses_qualified_default_without_a_cohort(self):
        result = self.select()
        self.assertEqual(result['ci_mode'], 'candidate')
        self.assertEqual(result['qualification_id'], '')
        self.assertEqual(result['dependency_cache'], 'warm')
        self.assertEqual(result['status'], 'success')
        self.assertEqual(result['reason'], 'qualified-default')

    def test_main_push_uses_qualified_default_and_ignores_pr_only_configuration(self):
        for raw in ('', 'not-json', config(mode='serial', dependency_cache='cold')):
            with self.subTest(raw=raw):
                result = self.select(environment(GITHUB_EVENT_NAME='push',
                                                GITHUB_REF='refs/heads/main',
                                                PENNEL_CI_QUALIFICATION_CONFIG=raw))
                self.assertEqual(result['ci_mode'], 'candidate')
                self.assertEqual(result['dependency_cache'], 'warm')
                self.assertEqual(result['qualification_id'], '')
                self.assertEqual(result['reason'], 'qualified-default')

    def test_other_push_refs_cannot_select_candidate(self):
        for ref in (None, '', 'refs/heads/topic', 'refs/tags/main', 'refs/heads/main-extra'):
            with self.subTest(ref=ref):
                env = environment(GITHUB_EVENT_NAME='push', GITHUB_REF=ref,
                                  PENNEL_CI_QUALIFICATION_CONFIG=config())
                if ref is None:
                    env.pop('GITHUB_REF')
                result = self.select(env)
                self.assertEqual(result['ci_mode'], 'serial')
                self.assertEqual(result['dependency_cache'], 'warm')
                self.assertEqual(result['qualification_id'], '')

    def test_exact_same_repository_pr_and_head_can_select_candidate(self):
        result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config()))
        self.assertEqual(result['ci_mode'], 'candidate')
        self.assertEqual(result['qualification_id'], 'warm-candidate-1')
        self.assertEqual(result['head_sha'], HEAD)
        self.assertEqual(result['merge_sha'], MERGE)
        self.assertEqual(result['run_id'], 100)
        self.assertEqual(result['run_attempt'], 2)

    def test_exact_scope_can_select_serial_control(self):
        result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(mode='serial')))
        self.assertEqual(result['ci_mode'], 'serial')
        self.assertEqual(result['qualification_id'], 'warm-candidate-1')
        self.assertEqual(result['reason'], 'qualification')

    def test_exact_scope_can_disable_dependency_caches_for_either_mode(self):
        for mode in ('serial', 'candidate'):
            with self.subTest(mode=mode):
                result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(
                    mode=mode, dependency_cache='cold')))
                self.assertEqual(result['ci_mode'], mode)
                self.assertEqual(result['dependency_cache'], 'cold')

    def test_optional_warm_setting_preserves_default(self):
        result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(
            dependency_cache='warm')))
        self.assertEqual(result['dependency_cache'], 'warm')

    def test_cold_setting_cannot_escape_qualification_scope(self):
        for updates in ({'GITHUB_EVENT_NAME': 'push'},
                        {'GITHUB_EVENT_NAME': 'workflow_dispatch'},
                        {'PENNEL_CI_FORCE_SERIAL': 'true'}):
            with self.subTest(updates=updates):
                result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(
                    dependency_cache='cold'), **updates))
                self.assertEqual(result['dependency_cache'], 'warm')
        for updates in ({'pr': 156}, {'repository': 'another/repository'}):
            with self.subTest(updates=updates):
                result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(
                    dependency_cache='cold', **updates)))
                self.assertEqual(result['dependency_cache'], 'warm')

    def test_selected_scope_rejects_invalid_dependency_cache_setting(self):
        for value in ('Cold', 'cold\nci_mode=candidate', '', None, {}, True):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(
                    dependency_cache=value)))

    def test_manual_dispatch_and_privileged_events_stay_serial_even_on_main_ref(self):
        for name in ('workflow_dispatch', 'pull_request_target', 'merge_group', 'workflow_run'):
            with self.subTest(name=name):
                result = self.select(environment(GITHUB_EVENT_NAME=name,
                                                GITHUB_REF='refs/heads/main',
                                                PENNEL_CI_QUALIFICATION_CONFIG=config()))
                self.assertEqual(result['ci_mode'], 'serial')
                self.assertEqual(result['qualification_id'], '')

    def test_manual_dispatch_ignores_malformed_qualification_config(self):
        result = self.select(environment(GITHUB_EVENT_NAME='workflow_dispatch',
                                        PENNEL_CI_QUALIFICATION_CONFIG='not-json'))
        self.assertEqual(result['ci_mode'], 'serial')

    def test_force_serial_overrides_candidate_and_invalid_config(self):
        for name in ('pull_request', 'push'):
            for raw in ('', config(), 'not-json'):
                with self.subTest(name=name, raw=raw):
                    result = self.select(environment(GITHUB_EVENT_NAME=name,
                                                    GITHUB_REF='refs/heads/main',
                                                    PENNEL_CI_FORCE_SERIAL='true',
                                                    PENNEL_CI_QUALIFICATION_CONFIG=raw))
                    self.assertEqual(result['ci_mode'], 'serial')
                    self.assertEqual(result['qualification_id'], '')
                    self.assertEqual(result['dependency_cache'], 'warm')
                    self.assertEqual(result['reason'], 'force-serial')

    def test_force_serial_rejects_ambiguous_values(self):
        for raw in ('yes', '1', 'TRUE', 'false\nci_mode=candidate'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                self.select(environment(PENNEL_CI_FORCE_SERIAL=raw))

    def test_config_for_another_valid_scope_is_inert(self):
        for raw in (config(pr=156), config(repository='another/repository'),
                    json.dumps({'repository': REPOSITORY, 'pr': 156})):
            with self.subTest(raw=raw):
                result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=raw))
                self.assertEqual(result['ci_mode'], 'candidate')
                self.assertEqual(result['qualification_id'], '')
                self.assertEqual(result['dependency_cache'], 'warm')
                self.assertEqual(result['reason'], 'qualified-default')

    def test_fork_head_cannot_select_candidate(self):
        for head_repository in ({'full_name': 'someone/pennel'}, None):
            for raw in ('', config()):
                with self.subTest(head_repository=head_repository, raw=raw):
                    payload = event()
                    payload['pull_request']['head']['repo'] = head_repository
                    result = self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=raw), payload)
                    self.assertEqual(result['ci_mode'], 'serial')
                    self.assertEqual(result['dependency_cache'], 'warm')
                    self.assertEqual(result['qualification_id'], '')

    def test_wrong_base_repository_fails_closed(self):
        payload = event()
        payload['pull_request']['base']['repo']['full_name'] = 'another/repository'
        with self.assertRaises(ValueError):
            self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config()), payload)

    def test_stale_designated_pr_source_fails_closed(self):
        with self.assertRaises(ValueError):
            self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(head_sha='c' * 40)))

    def test_selected_scope_rejects_invalid_or_unknown_fields(self):
        invalid = [config(mode='split'), config(mode='candidate\nfoo=bar'),
                   config(cohort='../unsafe'), config(cohort=''),
                   config(cohort='x' * 65), config(cohort='a\nci_mode=candidate'),
                   config(head_sha=HEAD.upper()), config(head_sha='HEAD'),
                   config(extra='typo')]
        for raw in invalid:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=raw))

    def test_malformed_scope_fails_closed_without_echoing_raw_config(self):
        invalid = ['not-json', '[]', '{}', config(pr=True), config(pr='155'),
                   config(pr=0), config(repository='unsafe\nname'), 'null']
        for raw in invalid:
            with self.subTest(raw=raw), self.assertRaises(ValueError) as error:
                self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=raw))
            self.assertNotIn(raw, str(error.exception))

    def test_duplicate_config_keys_are_not_accepted(self):
        raw = config()[:-1] + ', "mode": "serial"}'
        with self.assertRaises(ValueError):
            self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=raw))

    def test_inconsistent_pr_number_fails_closed(self):
        payload = event(number=156)
        with self.assertRaises(ValueError):
            self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config()), payload)

    def test_inner_pr_number_must_be_an_exact_integer(self):
        for number in (155.0, True, '155', None):
            payload = event()
            payload['pull_request']['number'] = number
            with self.subTest(number=number), self.assertRaises(ValueError):
                self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config()), payload)

    def test_candidate_requires_complete_valid_github_identity(self):
        for updates in ({'GITHUB_SHA': ''}, {'GITHUB_SHA': 'refs/pull/155/merge'},
                        {'GITHUB_RUN_ID': '0'}, {'GITHUB_RUN_ATTEMPT': 'bad'}):
            with self.subTest(updates=updates), self.assertRaises(ValueError):
                self.select(environment(PENNEL_CI_QUALIFICATION_CONFIG=config(), **updates))

    def run_cli(self, directory, raw_config, **updates):
        root = Path(directory)
        payload = root / 'event.json'
        payload.write_text(json.dumps(event()))
        output = root / 'output.txt'
        summary = root / 'summary.md'
        receipt = root / 'receipt.json'
        env = {**os.environ, **environment(PENNEL_CI_QUALIFICATION_CONFIG=raw_config),
               'GITHUB_EVENT_PATH': str(payload), 'GITHUB_OUTPUT': str(output),
               'GITHUB_STEP_SUMMARY': str(summary), **updates}
        result = subprocess.run(['python3', '-B', str(SCRIPT), '--receipt', str(receipt)],
                                env=env, text=True, capture_output=True)
        return result, json.loads(receipt.read_text()), output, summary

    def test_cli_writes_safe_actual_mode_and_identity_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            result, receipt, output, summary = self.run_cli(directory, config())
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text().splitlines(),
                             ['ci_mode=candidate', 'qualification_id=warm-candidate-1',
                              'dependency_cache=warm'])
            self.assertEqual(receipt['ci_mode'], 'candidate')
            self.assertEqual(receipt['head_sha'], HEAD)
            self.assertEqual(receipt['merge_sha'], MERGE)
            self.assertIn('candidate', summary.read_text())
            self.assertNotIn('PENNEL_CI_QUALIFICATION_CONFIG', receipt)

    def test_cli_records_ordinary_qualified_default_without_benchmark_identity(self):
        for name, ref in (('pull_request', 'refs/pull/155/merge'), ('push', 'refs/heads/main')):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                result, receipt, output, _ = self.run_cli(
                    directory, '', GITHUB_EVENT_NAME=name, GITHUB_REF=ref)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(receipt['ci_mode'], 'candidate')
                self.assertEqual(receipt['qualification_id'], '')
                self.assertEqual(receipt['dependency_cache'], 'warm')
                self.assertEqual(receipt['reason'], 'qualified-default')
                self.assertEqual(output.read_text().splitlines(),
                                 ['ci_mode=candidate', 'qualification_id=', 'dependency_cache=warm'])

    def test_cli_records_scoped_cold_setting(self):
        with tempfile.TemporaryDirectory() as directory:
            result, receipt, output, summary = self.run_cli(
                directory, config(dependency_cache='cold'))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(receipt['dependency_cache'], 'cold')
            self.assertIn('dependency_cache=cold', output.read_text().splitlines())
            self.assertIn('cold', summary.read_text())

    def test_cli_errors_preserve_safe_failure_receipt_and_never_echo_config(self):
        raw = config(cohort='PRIVATE-TEXT\n::set-output name=ci_mode::candidate')
        with tempfile.TemporaryDirectory() as directory:
            result, receipt, output, summary = self.run_cli(directory, raw)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(receipt['status'], 'error')
            self.assertEqual(receipt['ci_mode'], 'serial')
            self.assertEqual(output.read_text().splitlines(),
                             ['ci_mode=serial', 'qualification_id=', 'dependency_cache=warm'])
            exposed = result.stdout + result.stderr + summary.read_text() + json.dumps(receipt)
            self.assertNotIn('PRIVATE-TEXT', exposed)
            self.assertNotIn('::set-output', exposed)


if __name__ == '__main__':
    unittest.main()
