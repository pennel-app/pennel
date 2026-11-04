import copy
from datetime import datetime, timedelta, timezone
import importlib.util
import io
import itertools
import json
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
import zipfile


SCRIPT = Path(__file__).resolve().parents[1] / 'ci-qualification.py'
SPEC = importlib.util.spec_from_file_location('ci_qualification', SCRIPT)
CI = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CI)
COUNTER = itertools.count(1000)
START = datetime(2020, 1, 1, tzinfo=timezone.utc)
HEAD, MERGE, BASE = 'a' * 40, 'b' * 40, 'c' * 40
REPO = 'pennel-app/pennel'


def stamp(seconds=0):
    return (START + timedelta(seconds=seconds)).isoformat()


def artifact(document):
    return {'id': next(COUNTER), 'created_at': stamp(20), 'document': document}


def job(name, seconds, steps=(), rust=False, node=False, cold=False, native=False):
    image = 'macos-26-arm64' if native or name == 'build-and-test' or 'macos-' in name else (
        'windows-2025' if 'windows-' in name else 'ubuntu-24.04')
    values = [{'name': step, 'conclusion': 'success'} for step in steps]
    if rust:
        values.append({'name': 'Run Swatinem/rust-cache@v2',
                       'conclusion': 'skipped' if cold else 'success'})
    if node:
        values.append({'name': 'Run actions/setup-node@v5', 'conclusion': 'success'})
    if not values:
        values = [{'name': 'Set up job', 'conclusion': 'success'}]
    return {'id': next(COUNTER), 'name': name, 'status': 'completed', 'conclusion': 'success',
            'started_at': stamp(1), 'completed_at': stamp(seconds), 'runner_name': 'hosted',
            'steps': values, 'identity': {
                'image': image, 'image_version': '20261026.0455.2',
                'rustc': 'rustc 1.99.1 (frozen)' if rust else None,
                'rust_key': 'v0-rust-frozen-env-deps' if rust and not cold else None,
                'rust_exact': True if rust and not cold else None,
                'node': 'v22.21.0' if node else None, 'npm': '10.9.4' if node else None,
                'python': None, 'node_cache_keys': ['node-cache-frozen'] if node and not cold else [],
                'node_cache_restored': node and not cold,
            }}


def observation(category='docs', mode='serial', cache='warm', pair='one', seconds=60):
    cohort = pair + '-' + mode
    obs = {'schema': 1, 'repository': REPO, 'pr': 155, 'category': category,
           'head_sha': HEAD, 'merge_sha': MERGE, 'base_sha': BASE, 'head_branch': 'codex/probe',
           'mode': mode, 'cohort': cohort, 'pair': pair, 'cache_state': cache,
           'requested_at': stamp(), 'captured_at': stamp(10000),
           'runs': [], 'capture_errors': []}
    cold = cache == 'cold'
    for workflow in CI.WORKFLOWS[category]:
        run_id = next(COUNTER)
        actual = {'schema': 1, 'status': 'success', 'event': 'pull_request',
                  'repository': REPO, 'pr': 155, 'head_repository': REPO,
                  'head_sha': HEAD, 'merge_sha': MERGE, 'run_id': run_id, 'run_attempt': 2,
                  'ci_mode': mode, 'qualification_id': cohort, 'dependency_cache': cache}
        run = {'workflow': workflow, 'id': run_id, 'run_attempt': 2,
               'event': 'pull_request', 'path': '.github/workflows/' + workflow,
               'status': 'completed', 'conclusion': 'success', 'head_sha': HEAD,
               'head_branch': 'codex/probe', 'pull_requests': [{'number': 155}],
               'created_at': stamp(-86400), 'jobs': [job('changes / changes', seconds)],
               'artifacts': {'ci-mode-2': artifact(actual)}}
        if workflow in ('ci.yml', 'mobile.yml'):
            run['jobs'].append(job('required', seconds))
        if workflow == 'ci.yml' and category == 'core':
            run['jobs'].append(job('build-and-test', seconds, (
                'Rust format', 'Rust lint', 'Rust tests', 'Local keyboard policy',
                'Rust lint without local models', 'Rust tests without local models',
                'Speaker pipeline'), rust=True, cold=cold))
        if workflow == 'desktop.yml' and category in ('desktop', 'core'):
            for os_name in ('macos-15', 'windows-latest', 'ubuntu-22.04'):
                steps = ['Frontend tests', 'Rust format', 'Installers with verified non-Mac x64 CPU baseline']
                steps += ['Frontend', 'Rust lint', 'Rust tests'] if mode == 'serial' else [
                    'Rust lint after packaging', 'Rust tests after packaging']
                app = job('app (' + os_name + ', target)', seconds, steps, rust=True, node=True, cold=cold)
                if mode == 'candidate':
                    app['steps'].append({'name': 'Frontend', 'conclusion': 'skipped'})
                run['jobs'].append(app)
            if category == 'core':
                for os_name in ('windows-latest', 'ubuntu-latest'):
                    run['jobs'].append(job('core (' + os_name + ')', seconds, (
                        'Rust lint', 'Rust tests', 'Local keyboard policy',
                        'Rust lint and tests without local speaker recognition',
                        'Speaker pipeline'), rust=True, cold=cold))
        if workflow == 'decision-evaluation.yml':
            for os_name in ('macos-latest', 'windows-latest', 'ubuntu-latest'):
                run['jobs'].append(job('offline (' + os_name + ')', seconds, (
                    'Build evaluation-only bridge without native models', 'Evaluation Rust checks',
                    'Offline corpus, metrics and mocked transports',
                    'Real deterministic baselines and reproducible report'), rust=True, cold=cold))
        if workflow == 'mobile.yml':
            lanes = ['serial'] if mode == 'serial' else ['release', 'runtime']
            receipts = []
            for lane in lanes:
                run['jobs'].append(job('native ' + lane, seconds, rust=True, node=True, cold=cold, native=True))
                receipt = {'schema': 2, 'cache': {'mode': 'off'}, 'validation_errors': [],
                           'rust_cache': {'exact_hit': None if cold else True, 'bypassed': cold,
                                          'bypass_scope': ['rust', 'npm'] if cold else []},
                           'validation': {
                               'ci_mode': mode, 'lane': lane, 'qualification_id': cohort,
                               'repository': REPO, 'event_name': 'pull_request',
                               'source_sha': MERGE, 'head_sha': HEAD, 'dependency_cache': cache,
                               'job_id': 'ios-native-tests' if lane == 'runtime' else 'ios-generated-build',
                               'expected_phases': list(CI.PHASES[lane])},
                           'metadata': {'revision': MERGE, 'xcode': 'Xcode 26.6',
                                        'device_sdk': '26.6', 'simulator_sdk': '26.6',
                                        'rustc': 'rustc 1.99.1', 'architecture': 'arm64',
                                        'variant': 'preview', 'dependency_sha256': 'd' * 64},
                           'provenance': {'run_id': str(run_id), 'run_attempt': '2',
                                          'image_os': 'macos26', 'image_version': '20261026.0455.2'},
                           'phases': [{'name': p, 'seconds': 1, 'exit_code': 0} for p in CI.PHASES[lane]]}
                receipts.append(receipt)
                run['artifacts'][f'mobile-ci-{lane}-{mode}-attempt-2'] = artifact(receipt)
            run['artifacts']['mobile-validation-attempt-2'] = artifact({
                'schema': 1, 'status': 'success', 'ci_mode': mode, 'dependency_cache': cache,
                'qualification_id': cohort, 'repository': REPO, 'event_name': 'pull_request',
                'source_sha': MERGE, 'head_sha': HEAD, 'run_id': run_id, 'run_attempt': 2,
                'expected_lanes': lanes, 'errors': [], 'receipts': receipts})
        obs['runs'].append(run)
    return obs


class QualificationTests(unittest.TestCase):
    def test_complete_scopes_and_both_modes_validate(self):
        for category in CI.WORKFLOWS:
            for mode in ('serial', 'candidate'):
                for cache in ('warm', 'cold'):
                    with self.subTest(category=category, mode=mode, cache=cache):
                        result = CI.validate_observation(observation(category, mode, cache))
                        self.assertTrue(result['valid'], result['errors'])

    def test_reruns_use_explicit_request_not_original_created_at(self):
        result = CI.validate_observation(observation(seconds=91))
        self.assertEqual(result['elapsed_seconds'], 91)

    def test_unknown_mode_or_source_receipt_never_passes(self):
        for field, value in [('ci_mode', 'serial'), ('head_sha', 'e' * 40),
                             ('qualification_id', 'another'), ('run_attempt', 1),
                             ('dependency_cache', 'cold'), ('status', 'error')]:
            obs = observation(mode='candidate')
            obs['runs'][0]['artifacts']['ci-mode-2']['document'][field] = value
            with self.subTest(field=field):
                self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_missing_workflows_wrong_pr_and_carried_forward_jobs_fail(self):
        for mutate in [lambda o: o['runs'].pop(),
                       lambda o: o['runs'][0].update(pull_requests=[{'number': 156}]),
                       lambda o: o['runs'][0]['jobs'][0].update(started_at=stamp(-1.1)),
                       lambda o: o['runs'][0]['jobs'][0].update(completed_at=stamp(10002))]:
            obs = observation()
            mutate(obs)
            self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_failed_abandoned_and_unstarted_jobs_stay_invalid(self):
        for change in ({'status': 'in_progress'}, {'conclusion': 'failure'},
                       {'conclusion': 'abandoned', 'runner_name': '', 'steps': []}):
            obs = observation()
            obs['runs'][0]['jobs'][0].update(change)
            self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_abandoned_jobs_are_not_physical_execution_and_partial_runs_have_no_final_time(self):
        obs = observation()
        failed = obs['runs'][0]['jobs'][0]
        failed.update(conclusion='abandoned', runner_name='', steps=[])
        result = CI.validate_observation(obs)
        self.assertNotIn(failed['id'], result['physical_job_ids'])
        self.assertIsNone(next(i for i in result['intervals'] if i['physical'] is False)['execution_seconds'])
        obs = observation()
        obs['runs'][0]['status'] = 'in_progress'
        result = CI.validate_observation(obs)
        self.assertIsNone(result['elapsed_seconds'])
        self.assertEqual(result['elapsed_lower_bound_seconds'], 10000)
        self.assertTrue(result['target_miss'])

    def test_runner_images_and_exact_warm_cache_need_observed_proof(self):
        for key, value in [('image_version', None), ('rust_exact', False),
                           ('rust_key', None), ('node_cache_restored', False)]:
            obs = observation('desktop')
            obs['runs'][1]['jobs'][1]['identity'][key] = value
            with self.subTest(key=key):
                self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_cold_is_a_controlled_bypass_not_an_unobserved_cache_miss(self):
        obs = observation('desktop', cache='cold')
        obs['runs'][1]['jobs'][1]['steps'][-2]['conclusion'] = 'success'
        self.assertFalse(CI.validate_observation(obs)['valid'])
        obs = observation('mobile', cache='cold')
        obs['runs'][-1]['artifacts']['mobile-ci-serial-serial-attempt-2']['document']['rust_cache']['bypassed'] = False
        self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_candidate_requires_literal_lane_artifact_names_and_all_native_phases(self):
        for mutation in ('missing', 'old-name', 'phase', 'aggregate'):
            obs = observation('mobile', 'candidate')
            artifacts = obs['runs'][-1]['artifacts']
            if mutation == 'missing':
                del artifacts['mobile-ci-runtime-candidate-attempt-2']
            elif mutation == 'old-name':
                artifacts['mobile-ci-runtime-off-attempt-2'] = artifacts.pop('mobile-ci-runtime-candidate-attempt-2')
            elif mutation == 'phase':
                artifacts['mobile-ci-runtime-candidate-attempt-2']['document']['phases'].pop()
            else:
                artifacts['mobile-validation-attempt-2']['document']['status'] = 'failure'
            with self.subTest(mutation=mutation):
                self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_core_cannot_omit_feature_or_platform_validation(self):
        obs = observation('core')
        obs['runs'][0]['jobs'][2]['steps'].pop(4)
        self.assertFalse(CI.validate_observation(obs)['valid'])
        obs = observation('core')
        obs['runs'][-1]['jobs'].pop()
        self.assertFalse(CI.validate_observation(obs)['valid'])

    def test_unexpected_workflow_time_is_counted_and_not_qualified(self):
        obs = observation(seconds=50)
        extra = copy.deepcopy(obs['runs'][0])
        extra['id'] = next(COUNTER)
        extra['workflow'] = 'extra.yml'
        extra['jobs'] = [job('extra', 400)]
        obs['runs'].append(extra)
        result = CI.validate_observation(obs)
        self.assertFalse(result['valid'])
        self.assertEqual(result['elapsed_seconds'], 400)
        self.assertTrue(result['target_miss'])

    def test_safe_archive_rejects_traversal_symlink_duplicate_wrong_root_and_size(self):
        for kind in ('traversal', 'symlink', 'duplicate', 'wrong-root', 'oversized'):
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, 'w') as archive:
                name = '../mode.json' if kind == 'traversal' else 'other/mode.json' if kind == 'wrong-root' else 'mode.json'
                info = zipfile.ZipInfo(name)
                if kind == 'symlink':
                    info.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(info, b'x' * (CI.ARCHIVE_LIMIT + 1) if kind == 'oversized' else b'{}')
                if kind == 'duplicate':
                    archive.writestr('extra.json', '{}')
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                CI.read_json_archive(buffer.getvalue(), 'mode.json')
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as archive:
            archive.writestr('mode.json', '{"status":"success"}')
        self.assertEqual(CI.read_json_archive(buffer.getvalue(), 'mode.json'), {'status': 'success'})

    def test_log_prefix_metadata_is_observed_and_bounded_to_image_group(self):
        prefix = ('2026-01-01T00:00:00Z ##[group]Runner Image\n'
                  '2026-01-01T00:00:00Z Image: macos-26-arm64\n'
                  '2026-01-01T00:00:00Z Version: 20261026.0455.2\n'
                  '2026-01-01T00:00:00Z ##[endgroup]\n'
                  'rustc 1.99.1 (frozen)\nnode: v22.21.0\nnpm: 10.9.4\n'
                  'Cache restored from key: v0-rust-frozen-env-deps\nfull match: true\n'
                  'Cache restored from key: node-cache-frozen\n')
        identity = CI.log_identity(prefix)
        self.assertEqual(identity['image_version'], '20261026.0455.2')
        self.assertTrue(identity['rust_exact'])
        self.assertTrue(identity['node_cache_restored'])
        self.assertIsNone(CI.log_identity('Version: an-unrelated-tool')['image_version'])

    def test_five_pairs_per_scope_and_cold_controls_are_required(self):
        observations = []
        for category in CI.WORKFLOWS:
            for cache in ('warm', 'cold'):
                for index in range(5):
                    for mode in ('serial', 'candidate'):
                        observations.append(observation(category, mode, cache, f'{category}-{cache}-{index}'))
        self.assertTrue(CI.assess(observations)['rollout_qualified'])
        self.assertFalse(CI.assess(observations[:-1])['rollout_qualified'])
        self.assertFalse(CI.assess([o for o in observations if o['cache_state'] == 'warm'])['rollout_qualified'])

    def test_mismatched_pairs_duplicates_and_cold_regression_fail(self):
        for kind in ('image', 'duplicate', 'regression', 'failed'):
            values = [observation('docs', mode, 'cold', str(i), seconds=100)
                      for i in range(5) for mode in ('serial', 'candidate')]
            if kind == 'image':
                values[-1]['runs'][0]['jobs'][0]['identity']['image_version'] = 'other'
            elif kind == 'duplicate':
                values.append(copy.deepcopy(values[-1]))
            elif kind == 'failed':
                extra = observation('docs', 'candidate', 'cold', 'failed')
                extra['runs'][0]['conclusion'] = 'failure'
                values.append(extra)
            else:
                for obs in values:
                    if obs['mode'] == 'candidate':
                        for run in obs['runs']:
                            run['jobs'][0]['completed_at'] = stamp(111)
            with self.subTest(kind=kind):
                self.assertFalse(CI.assess(values)['categories']['docs']['cold']['qualified'])

    def test_capture_is_read_only_and_preserves_existing_observations(self):
        fixture = observation()

        class FakeGitHub:
            def request(self, endpoint, binary=False):
                if endpoint == 'pulls/155':
                    return {'head': {'sha': HEAD, 'ref': 'codex/probe', 'repo': {'full_name': REPO}},
                            'base': {'sha': BASE, 'repo': {'full_name': REPO}}, 'merge_commit_sha': MERGE}
                for run in fixture['runs']:
                    if endpoint in (f"actions/runs/{run['id']}", f"actions/runs/{run['id']}/attempts/2"):
                        return run
                    for name, value in run['artifacts'].items():
                        if endpoint == f"actions/artifacts/{value['id']}/zip":
                            buffer = io.BytesIO()
                            with zipfile.ZipFile(buffer, 'w') as archive:
                                archive.writestr('mode.json', json.dumps(value['document']))
                            return buffer.getvalue()
                raise AssertionError(endpoint)

            def pages(self, endpoint, field):
                for run in fixture['runs']:
                    if endpoint == f"actions/runs/{run['id']}/attempts/2/jobs":
                        return run['jobs']
                    if endpoint == f"actions/runs/{run['id']}/artifacts":
                        return [{'name': name, **value} for name, value in run['artifacts'].items()]
                raise AssertionError(endpoint)

            def job_identity(self, job):
                return job['identity']

        with tempfile.TemporaryDirectory() as directory:
            args = SimpleNamespace(repository=REPO, pr=155, category='docs', head_sha=HEAD,
                                   mode='serial', cohort='one-serial', pair='one', cache_state='warm',
                                   requested_at=stamp(), output=directory,
                                   run=[f"{r['workflow']}={r['id']}:2" for r in fixture['runs']])
            result = CI.capture(args, FakeGitHub())
            self.assertTrue(result['assessment']['valid'], result['assessment']['errors'])
            self.assertEqual(result['assessment']['elapsed_seconds'], 60)
            saved = (Path(directory) / 'observation.json').read_bytes()
            with self.assertRaises(ValueError):
                CI.capture(args, FakeGitHub())
            self.assertEqual((Path(directory) / 'observation.json').read_bytes(), saved)


if __name__ == '__main__':
    unittest.main()
