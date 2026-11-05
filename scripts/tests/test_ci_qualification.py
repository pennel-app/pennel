import copy
from datetime import datetime, timedelta, timezone
import importlib.util
import io
import itertools
import json
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch
import zipfile


SCRIPT = Path(__file__).resolve().parents[1] / 'ci-qualification.py'
SPEC = importlib.util.spec_from_file_location('ci_qualification', SCRIPT)
CI = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CI)
COUNTER = itertools.count(1000)
START = datetime(2020, 1, 1, tzinfo=timezone.utc)
HEAD, MERGE, BASE = 'a' * 40, 'b' * 40, 'c' * 40
REPO = 'pennel-app/pennel'
ROLLING_APPROVAL = 'https://github.com/pennel-app/pennel/issues/152#issuecomment-6664508244'


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
            run['jobs'].append(job('required' if workflow == 'ci.yml' else 'Complete mobile validation', seconds))
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

    def test_mobile_gate_uses_its_actual_api_display_name(self):
        obs = observation('mobile', 'candidate')
        gate = next(j for j in obs['runs'][-1]['jobs'] if j['name'] == 'Complete mobile validation')
        self.assertTrue(CI.validate_observation(obs)['valid'])
        gate['name'] = 'required'
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

    def test_live_startup_fixture_distinguishes_provisioner_and_image_with_bom(self):
        prefix = ('\ufeff2026-11-04T22:00:00.0000000Z ##[group]Runner Image Provisioner\r\n'
                  '2026-11-04T22:00:00.0000000Z Version: 20261028.624\r\n'
                  '2026-11-04T22:00:00.0000000Z ##[endgroup]\r\n'
                  '2026-11-04T22:00:00.0000000Z ##[group]Runner Image\r\n'
                  '2026-11-04T22:00:00.0000000Z Image: ubuntu-24.04\r\n'
                  '2026-11-04T22:00:00.0000000Z Version: 20261101.360.1\r\n'
                  '2026-11-04T22:00:00.0000000Z ##[endgroup]\r\n')
        identity = CI.log_identity(prefix)
        self.assertEqual(identity['image'], 'ubuntu-24.04')
        self.assertEqual(identity['image_version'], '20261101.360.1')
        provisioner_only = prefix.split('##[group]Runner Image\r\n')[0]
        self.assertIsNone(CI.log_identity(provisioner_only)['image'])
        self.assertIsNone(CI.log_identity(provisioner_only)['image_version'])

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

    def test_extra_valid_image_mismatch_does_not_block_five_comparable_pairs(self):
        for cache in ('warm', 'cold'):
            for matched_count in (4, 5):
                values = [observation('docs', mode, cache, str(i), seconds=100)
                          for i in range(matched_count) for mode in ('serial', 'candidate')]
                extra = [observation('docs', mode, cache, 'different-image', seconds=100)
                         for mode in ('serial', 'candidate')]
                extra[1]['runs'][0]['jobs'][0]['identity']['image_version'] = '20261102.360.1'
                self.assertTrue(all(CI.validate_observation(o)['valid'] for o in extra))
                gate = CI.assess(values + extra)['categories']['docs'][cache]
                with self.subTest(cache=cache, matched_count=matched_count):
                    self.assertEqual(gate['qualified'], matched_count >= 5)
                    self.assertEqual(len(gate['matched_pairs']), matched_count)
                    self.assertEqual(gate['rejected_pairs'], [{
                        'pair': 'different-image', 'reason': 'Source/image/toolchain/cache identities differ'}])
                    self.assertEqual(len(gate['observations']), 2 * matched_count + 2)

    def test_five_comparable_pairs_do_not_waive_duplicate_invalid_or_failed_evidence(self):
        for kind in ('missing-mode', 'duplicate-mode', 'reused-jobs', 'missing-metadata', 'failed-check'):
            values = [observation('docs', mode, 'warm', str(i), seconds=100)
                      for i in range(5) for mode in ('serial', 'candidate')]
            if kind == 'missing-mode':
                values.append(observation('docs', 'serial', 'warm', 'missing'))
            elif kind == 'duplicate-mode':
                values.append(observation('docs', 'serial', 'warm', '0'))
            elif kind == 'reused-jobs':
                values.append(copy.deepcopy(values[0]))
            else:
                extra = [observation('docs', mode, 'warm', 'invalid') for mode in ('serial', 'candidate')]
                if kind == 'missing-metadata':
                    extra[1]['runs'][0]['jobs'][0]['identity']['image_version'] = None
                else:
                    extra[1]['runs'][0]['conclusion'] = 'failure'
                values.extend(extra)
            with self.subTest(kind=kind):
                self.assertFalse(CI.assess(values)['categories']['docs']['warm']['qualified'])

    def test_all_comparable_pairs_and_timing_misses_remain_in_medians(self):
        for cache, fast, slow, expected_median in [('warm', 290, 400, 345), ('cold', 100, 130, 115)]:
            values = []
            for index in range(10):
                values.extend([observation('docs', 'serial', cache, str(index), seconds=100),
                               observation('docs', 'candidate', cache, str(index),
                                           seconds=fast if index < 5 else slow)])
            gate = CI.assess(values)['categories']['docs'][cache]
            with self.subTest(cache=cache):
                self.assertEqual(len(gate['matched_pairs']), 10)
                self.assertEqual(gate['candidate_median_seconds'], expected_median)
                self.assertFalse(gate['qualified'])
                if cache == 'warm':
                    self.assertEqual(sum(o['target_miss'] for o in gate['observations']), 5)

    def change_source(self, obs, index):
        head, merge, base = (f'{index + offset:040x}' for offset in (1, 11, 21))
        obs.update(head_sha=head, merge_sha=merge, base_sha=base)
        for run in obs['runs']:
            run['head_sha'] = head
            run['artifacts']['ci-mode-2']['document'].update(head_sha=head, merge_sha=merge)
        self.assertTrue(CI.validate_observation(obs)['valid'])

    def test_five_individually_matched_sources_cannot_pool_into_one_frozen_gate(self):
        values = []
        for index in range(5):
            for mode in ('serial', 'candidate'):
                obs = observation('docs', mode, 'warm', str(index), seconds=100)
                self.change_source(obs, index)
                values.append(obs)
        gate = CI.assess(values)['categories']['docs']['warm']
        self.assertFalse(gate['qualified'])
        self.assertEqual(gate['matched_pairs'], [])
        self.assertIsNone(gate['control_median_seconds'])
        self.assertIsNone(gate['candidate_median_seconds'])
        self.assertIsNotNone(gate['source_ambiguity'])
        self.assertEqual(len(gate['source_strata']), 5)
        self.assertTrue(all(s['matched_pair_count'] == 1 for s in gate['source_strata']))
        self.assertEqual(len(gate['observations']), 10)

    def test_multiple_matched_sources_do_not_select_the_fastest_qualified_wave(self):
        values = [observation('docs', mode, 'warm', str(index), seconds=100)
                  for index in range(5) for mode in ('serial', 'candidate')]
        for mode in ('serial', 'candidate'):
            obs = observation('docs', mode, 'warm', 'other-wave', seconds=400)
            self.change_source(obs, 99)
            values.append(obs)
        gate = CI.assess(values)['categories']['docs']['warm']
        self.assertFalse(gate['qualified'])
        self.assertEqual(gate['matched_pairs'], [])
        self.assertIsNone(gate['candidate_median_seconds'])
        self.assertEqual(sorted(s['matched_pair_count'] for s in gate['source_strata']), [1, 5])
        self.assertEqual(sorted(s['candidate_median_seconds'] for s in gate['source_strata']), [100, 400])
        self.assertEqual(sum(o['target_miss'] for o in gate['observations']), 2)

    def test_extra_valid_cross_source_noncomparable_pair_does_not_become_a_blocker(self):
        values = [observation('docs', mode, 'warm', str(index), seconds=100)
                  for index in range(5) for mode in ('serial', 'candidate')]
        values.append(observation('docs', 'serial', 'warm', 'noncomparable', seconds=100))
        extra = observation('docs', 'candidate', 'warm', 'noncomparable', seconds=100)
        self.change_source(extra, 99)
        values.append(extra)
        gate = CI.assess(values)['categories']['docs']['warm']
        self.assertTrue(gate['qualified'])
        self.assertEqual(len(gate['matched_pairs']), 5)
        self.assertIsNone(gate['source_ambiguity'])
        self.assertEqual(sorted(s['matched_pair_count'] for s in gate['source_strata']), [0, 5])
        self.assertEqual(gate['rejected_pairs'], [{
            'pair': 'noncomparable', 'reason': 'Source/image/toolchain/cache identities differ'}])
        self.assertEqual(len(gate['observations']), 12)

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
            with tempfile.TemporaryDirectory() as rolling_directory:
                rolling_args = copy.copy(args)
                rolling_args.output = rolling_directory
                rolling_args.image_policy = 'rolling-build'
                rolling_args.approval_reference = ROLLING_APPROVAL
                rolling = CI.capture(rolling_args, FakeGitHub())
                self.assertEqual(rolling['image_policy'], 'rolling-build')
                self.assertEqual(rolling['approval_reference'], ROLLING_APPROVAL)
                self.assertTrue(rolling['assessment']['valid'])
                self.assertEqual(rolling['strict_assessment']['image_policy'], 'exact')
            self.assertEqual((Path(directory) / 'observation.json').read_bytes(), saved)


class RollingImagePolicyTests(unittest.TestCase):
    def rolling(self, values):
        return CI.assess(values, image_policy='rolling-build', approval_reference=ROLLING_APPROVAL)

    def change_build_numbers(self, obs):
        for run in obs['runs']:
            for index, job in enumerate(run['jobs']):
                job['identity']['image_version'] = f'20261101.360.{index}'
            if run['workflow'] == 'mobile.yml':
                for artifact in run['artifacts'].values():
                    document = artifact['document']
                    if document.get('schema') != 2:
                        continue
                    lane = document['validation']['lane']
                    native = next(j for j in run['jobs'] if j['name'] == 'native ' + lane)
                    document['provenance']['image_version'] = native['identity']['image_version']

    def test_rolling_policy_ignores_only_build_numbers_at_all_matching_points(self):
        values = []
        for category in CI.WORKFLOWS:
            for cache in ('warm', 'cold'):
                for index in range(5):
                    for mode in ('serial', 'candidate'):
                        obs = observation(category, mode, cache, f'{category}-{cache}-{index}')
                        if mode == 'candidate':
                            self.change_build_numbers(obs)
                        values.append(obs)
        before = CI.canonical(values)
        report = self.rolling(values)
        self.assertTrue(report['rollout_qualified'])
        self.assertFalse(report['strict_assessment']['rollout_qualified'])
        self.assertEqual(report['image_policy'], 'rolling-build')
        self.assertEqual(report['approval_reference'], ROLLING_APPROVAL)
        self.assertEqual(report['strict_assessment']['image_policy'], 'exact')
        self.assertEqual(CI.canonical(values), before)
        for category, groups in report['categories'].items():
            for cache, gate in groups.items():
                self.assertEqual(len(gate['matched_pairs']), 5)
                strict = report['strict_assessment']['categories'][category][cache]
                self.assertEqual([o['elapsed_seconds'] for o in gate['observations']],
                                 [o['elapsed_seconds'] for o in strict['observations']])
        candidate = next(o for o in report['categories']['mobile']['warm']['observations']
                         if o['mode'] == 'candidate')
        versions = {i['lane']: i['image_version'] for i in candidate['observed_images']
                    if i['kind'] == 'receipt'}
        self.assertNotEqual(versions['release'], versions['runtime'])
        self.assertTrue(all(versions.values()))

    def test_default_exact_stays_strict_and_rolling_requires_an_explicit_reference(self):
        serial = observation('mobile', 'serial')
        candidate = observation('mobile', 'candidate')
        self.change_build_numbers(candidate)
        self.assertTrue(CI.validate_observation(candidate, image_policy='rolling-build')['valid'])
        self.assertFalse(CI.validate_observation(candidate)['valid'])
        strict = CI.assess([serial, candidate])
        self.assertEqual(strict['image_policy'], 'exact')
        self.assertNotIn('strict_assessment', strict)
        iterator_report = self.rolling(iter([serial, candidate]))
        self.assertEqual(iterator_report['observation_count'], 2)
        self.assertEqual(iterator_report['strict_assessment']['observation_count'], 2)
        for reference in (None, '', '  ', 'unsafe\nreference'):
            with self.subTest(reference=reference), self.assertRaises(ValueError):
                CI.assess([serial, candidate], image_policy='rolling-build', approval_reference=reference)
        with self.assertRaises(ValueError):
            CI.assess([], image_policy='unknown', approval_reference=ROLLING_APPROVAL)

    def test_assess_cli_makes_policy_and_approval_explicit(self):
        values = [observation('docs', mode, 'warm', str(index))
                  for index in range(5) for mode in ('serial', 'candidate')]
        for obs in values:
            if obs['mode'] == 'candidate':
                self.change_build_numbers(obs)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = []
            for index, obs in enumerate(values):
                path = root / f'observation-{index}.json'
                path.write_text(json.dumps(obs))
                inputs.append(str(path))
            for policy in ('exact', 'rolling-build'):
                output = root / f'{policy}.json'
                arguments = [sys.executable, '-B', str(SCRIPT), 'assess', *inputs, '--output', str(output)]
                if policy == 'rolling-build':
                    arguments += ['--image-policy', policy, '--approval-reference', ROLLING_APPROVAL]
                result = CI.subprocess.run(arguments, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2, result.stderr)
                report = json.loads(output.read_text())
                self.assertEqual(report['image_policy'], policy)
                self.assertEqual(report['categories']['docs']['warm']['qualified'], policy == 'rolling-build')

    def test_rolling_cli_without_reference_does_not_write_an_assessment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output = root / 'observation.json', root / 'report.json'
            source.write_text(json.dumps(observation()))
            result = CI.subprocess.run([sys.executable, '-B', str(SCRIPT), 'assess', str(source),
                                        '--output', str(output), '--image-policy', 'rolling-build'],
                                       capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            self.assertIn('explicit approval reference', result.stderr)
            self.assertFalse(output.exists())

    def test_rolling_does_not_normalize_family_architecture_toolchain_cache_or_missing_metadata(self):
        cases = ('image-family', 'architecture', 'rust', 'node', 'npm', 'python', 'xcode',
                 'sdk', 'variant', 'dependency', 'cache-key', 'cache-hit',
                 'missing-job-version', 'missing-receipt-version')
        for case in cases:
            values = [observation('mobile', mode, 'warm', str(index))
                      for index in range(5) for mode in ('serial', 'candidate')]
            for obs in values:
                if obs['mode'] == 'candidate':
                    self.change_build_numbers(obs)
            candidate = values[-1]
            run = candidate['runs'][-1]
            native = next(j for j in run['jobs'] if j['name'] == 'native release')
            if case in ('image-family', 'rust', 'node', 'npm', 'python', 'cache-key', 'cache-hit', 'missing-job-version'):
                key, value = {'image-family': ('image', 'macos-27-arm64'),
                              'rust': ('rustc', 'rustc 1.100.0'), 'node': ('node', 'v24.0.0'),
                              'npm': ('npm', '11.0.0'), 'python': ('python', 'CPython (3.11.18)'),
                              'cache-key': ('rust_key', 'v0-rust-other-inputs'),
                              'cache-hit': ('rust_exact', False),
                              'missing-job-version': ('image_version', None)}[case]
                native['identity'][key] = value
            else:
                for artifact in run['artifacts'].values():
                    document = artifact['document']
                    if document.get('schema') != 2:
                        continue
                    if case == 'missing-receipt-version':
                        document['provenance']['image_version'] = None
                    else:
                        key, value = {'architecture': ('architecture', 'x86_64'),
                                      'xcode': ('xcode', 'Xcode 27.0'), 'sdk': ('device_sdk', '27.0'),
                                      'variant': ('variant', 'production'),
                                      'dependency': ('dependency_sha256', 'e' * 64)}[case]
                        document['metadata'][key] = value
            gate = self.rolling(values)['categories']['mobile']['warm']
            with self.subTest(case=case):
                self.assertFalse(gate['qualified'])

    def test_rolling_preserves_source_strata_and_all_comparable_timing_misses(self):
        values = []
        source_helper = QualificationTests()
        for index in range(5):
            for mode in ('serial', 'candidate'):
                obs = observation('docs', mode, 'warm', str(index), seconds=100)
                source_helper.change_source(obs, index)
                if mode == 'candidate':
                    self.change_build_numbers(obs)
                values.append(obs)
        gate = self.rolling(values)['categories']['docs']['warm']
        self.assertFalse(gate['qualified'])
        self.assertEqual(len(gate['source_strata']), 5)
        self.assertEqual(gate['matched_pairs'], [])
        self.assertIsNone(gate['candidate_median_seconds'])
        values = []
        for index in range(10):
            serial = observation('docs', 'serial', 'warm', str(index), seconds=100)
            candidate = observation('docs', 'candidate', 'warm', str(index), seconds=290 if index < 5 else 400)
            self.change_build_numbers(candidate)
            values.extend([serial, candidate])
        gate = self.rolling(values)['categories']['docs']['warm']
        self.assertFalse(gate['qualified'])
        self.assertEqual(len(gate['matched_pairs']), 10)
        self.assertEqual(gate['candidate_median_seconds'], 345)
        self.assertEqual(sum(o['target_miss'] for o in gate['observations']), 5)


class ReadRobustnessTests(unittest.TestCase):
    def response(self, status=0, stdout=b'{"ok":true}', stderr=b''):
        return SimpleNamespace(returncode=status, stdout=stdout, stderr=stderr)

    def test_transient_timeout_and_tls_reads_retry_without_workflow_mutation(self):
        api = CI.GitHub(REPO)
        attempts = [CI.subprocess.TimeoutExpired('gh', 25),
                    self.response(1, stderr=b'net/http: TLS handshake timeout token=DO-NOT-STORE'),
                    self.response()]
        with patch.object(CI.subprocess, 'run', side_effect=attempts) as run, patch.object(CI.time, 'sleep'):
            self.assertEqual(api.request('actions/runs/1'), {'ok': True})
        self.assertEqual(run.call_count, 3)
        for call in run.call_args_list:
            self.assertEqual(call.args[0][2:4], ['--method', 'GET'])
            self.assertEqual(call.kwargs['timeout'], 25)
        self.assertEqual(api.read_trace[-1]['status'], 'recovered')
        self.assertNotIn('DO-NOT-STORE', json.dumps(api.read_trace))

    def test_rate_limit_and_server_errors_retry_but_permission_and_missing_do_not(self):
        api = CI.GitHub(REPO)
        responses = [self.response(1, stderr=b'HTTP 429'), self.response(1, stderr=b'(HTTP 503)'), self.response()]
        with patch.object(CI.subprocess, 'run', side_effect=responses) as run, patch.object(CI.time, 'sleep'):
            api.request('actions/runs/1')
        self.assertEqual(run.call_count, 3)
        for status in (401, 403, 404):
            with self.subTest(status=status), patch.object(CI.subprocess, 'run', return_value=self.response(
                    1, stderr=f'HTTP {status} TLS handshake timeout'.encode())) as run:
                with self.assertRaises(CI.ReadError):
                    CI.GitHub(REPO).request('actions/runs/1')
                self.assertEqual(run.call_count, 1)

    def test_exhausted_reads_preserve_observer_trace_without_inventing_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            args = SimpleNamespace(repository=REPO, pr=155, category='docs', head_sha=HEAD,
                                   mode='serial', cohort='one-serial', pair='one', cache_state='warm',
                                   requested_at=stamp(), output=directory, run=[])
            with patch.object(CI.subprocess, 'run', side_effect=CI.subprocess.TimeoutExpired('gh', 25)) as run, \
                    patch.object(CI.time, 'sleep'), self.assertRaises(CI.ReadError):
                CI.capture(args)
            self.assertEqual(run.call_count, 3)
            trace_file = Path(directory) / 'observer-read-error.json'
            trace = json.loads(trace_file.read_text())
            self.assertEqual(trace['status'], 'observer-read-error')
            self.assertEqual(len(trace['api_read_trace']), 3)
            self.assertFalse(trace['api_read_trace'][-1]['retrying'])
            self.assertFalse((Path(directory) / 'observation.json').exists())
            saved = trace_file.read_bytes()
            with self.assertRaises(ValueError):
                CI.capture(args)
            self.assertEqual(trace_file.read_bytes(), saved)

    def test_malformed_json_and_unclassified_cli_error_fail_once(self):
        for response in (self.response(stdout=b'not json'), self.response(1, stderr=b'Please authenticate TOKEN')):
            api = CI.GitHub(REPO)
            with self.subTest(response=response), patch.object(CI.subprocess, 'run', return_value=response) as run:
                with self.assertRaises(CI.ReadError):
                    api.request('actions/runs/1')
                self.assertEqual(run.call_count, 1)
                self.assertNotIn('TOKEN', json.dumps(api.read_trace))

    def test_binary_and_log_reads_allow_raw_capture_without_printing_terminal_data(self):
        api = CI.GitHub(REPO)
        binary = b'PK\x03\x04\x1b[31m'
        with patch.object(CI.subprocess, 'run', return_value=self.response(stdout=binary)) as run:
            self.assertEqual(api.request('actions/artifacts/1/zip', binary=True), binary)
        self.assertIn('--allow-escape-sequences', run.call_args.args[0])
        prefix = (b'\x1b[31m##[group]Runner Image\x1b[0m\n'
                  b'Image: macos-26-arm64\nVersion: 20261026.0455.2\n##[endgroup]\n')
        stream, process, selector = MagicMock(), MagicMock(), MagicMock()
        process.stdout = stream
        process.poll.return_value = 0
        process.wait.return_value = 0
        selector.__enter__.return_value = selector
        selector.select.return_value = [(stream, 1)]
        with patch.object(CI.subprocess, 'Popen', return_value=process) as popen, \
                patch.object(CI.selectors, 'DefaultSelector', return_value=selector), \
                patch.object(CI.os, 'read', side_effect=[prefix, b'']):
            identity = api.job_identity({'id': 1, 'runner_name': 'hosted', 'status': 'completed'})
        self.assertIn('--allow-escape-sequences', popen.call_args.args[0])
        self.assertEqual(identity['image'], 'macos-26-arm64')
        self.assertNotIn('\x1b', json.dumps(identity))

    def log_process(self, timeout=False, returncode=0):
        process, selector = MagicMock(), MagicMock()
        process.poll.return_value = returncode
        process.wait.return_value = returncode
        selector.__enter__.return_value = selector
        selector.select.return_value = [] if timeout else [(process.stdout, 1)]
        return process, selector

    def test_log_timeout_retries_same_read_then_recovers_observed_identity(self):
        api = CI.GitHub(REPO)
        first, first_selector = self.log_process(timeout=True)
        second, second_selector = self.log_process()
        prefix = b'##[group]Runner Image\nImage: ubuntu-24.04\nVersion: 20261101.360.1\n##[endgroup]\n'
        with patch.object(CI.subprocess, 'Popen', side_effect=[first, second]) as popen, \
                patch.object(CI.selectors, 'DefaultSelector', side_effect=[first_selector, second_selector]), \
                patch.object(CI.os, 'read', side_effect=[prefix, b'']), patch.object(CI.time, 'sleep'):
            identity = api.job_identity({'id': 123, 'runner_name': 'hosted', 'status': 'completed'})
        self.assertEqual(identity['image_version'], '20261101.360.1')
        self.assertEqual(popen.call_count, 2)
        self.assertEqual(popen.call_args_list[0].args, popen.call_args_list[1].args)
        self.assertLessEqual(first_selector.select.call_args.args[0], 25)
        self.assertEqual([r['status'] for r in api.read_trace], ['timeout', 'recovered'])
        self.assertEqual([r['retrying'] for r in api.read_trace], [True, False])

    def test_exhausted_log_timeouts_leave_unknown_identity_with_all_attempt_traces(self):
        api = CI.GitHub(REPO)
        attempts = [self.log_process(timeout=True) for _ in range(3)]
        with patch.object(CI.subprocess, 'Popen', side_effect=[a[0] for a in attempts]) as popen, \
                patch.object(CI.selectors, 'DefaultSelector', side_effect=[a[1] for a in attempts]), \
                patch.object(CI.time, 'sleep'):
            identity = api.job_identity({'id': 123, 'runner_name': 'hosted', 'status': 'completed'})
        self.assertEqual(identity, {})
        self.assertEqual(popen.call_count, 3)
        self.assertEqual([r['attempt'] for r in api.read_trace], [1, 2, 3])
        self.assertEqual([r['retrying'] for r in api.read_trace], [True, True, False])
        self.assertTrue(all(r['bytes'] == 0 and r['error_kind'] == 'timeout' for r in api.read_trace))

    def test_complete_missing_image_and_deliberately_bounded_logs_do_not_retry(self):
        for bounded in (False, True):
            api = CI.GitHub(REPO)
            process, selector = self.log_process()
            prefix = b'complete log with no runner image\n'
            with self.subTest(bounded=bounded), \
                    patch.object(CI.subprocess, 'Popen', return_value=process) as popen, \
                    patch.object(CI.selectors, 'DefaultSelector', return_value=selector), \
                    patch.object(CI.os, 'read', side_effect=[prefix, b'']), \
                    patch.object(CI, 'LOG_LIMIT', len(prefix) if bounded else 262144):
                identity = api.job_identity({'id': 123, 'runner_name': 'hosted', 'status': 'completed'})
            self.assertIsNone(identity['image'])
            self.assertEqual(popen.call_count, 1)
            self.assertEqual(api.read_trace[-1]['status'], 'bounded' if bounded else 'complete')
            self.assertFalse(api.read_trace[-1]['retrying'])

    def test_empty_log_http_transience_is_classified_without_preserving_diagnostics(self):
        for status, expected_attempts in ((None, 3), (503, 3), (404, 1), (403, 1)):
            api = CI.GitHub(REPO)
            process, selector = self.log_process(returncode=0 if status is None else 1)
            def start(*args, **kwargs):
                if status is not None:
                    kwargs['stderr'].write(f'HTTP {status} token=DO-NOT-STORE'.encode())
                return process
            with self.subTest(status=status), \
                    patch.object(CI.subprocess, 'Popen', side_effect=start) as popen, \
                    patch.object(CI.selectors, 'DefaultSelector', return_value=selector), \
                    patch.object(CI.os, 'read', return_value=b''), patch.object(CI.time, 'sleep'):
                identity = api.job_identity({'id': 123, 'runner_name': 'hosted', 'status': 'completed'})
            self.assertEqual(identity, {})
            self.assertEqual(popen.call_count, expected_attempts)
            self.assertNotIn('DO-NOT-STORE', json.dumps(api.read_trace))
            self.assertEqual(api.read_trace[-1]['http_status'], status)


if __name__ == '__main__':
    unittest.main()
