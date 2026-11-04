import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'ci-mobile-status.py'
PHASES = {
    'serial': ['typecheck-and-tests', 'native-preparation', 'release-simulator',
               'release-device', 'debug-runtime-and-keyboard'],
    'release': ['typecheck-and-tests', 'native-preparation', 'release-simulator', 'release-device'],
    'runtime': ['typecheck-and-tests', 'native-preparation', 'debug-runtime-and-keyboard'],
}


class MobileStatusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('ci_mobile_status', SCRIPT)
        cls.status = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.status)

    def needs(self, mode='serial', release='success', runtime=None):
        return {
            'changes': {'result': 'success', 'outputs': {
                'ci_mode': mode, 'dependency_cache': 'warm', 'qualification_id': 'cohort-1',
            }},
            'ios-generated-build': {'result': release},
            'ios-native-tests': {'result': runtime or ('success' if mode == 'candidate' else 'skipped')},
        }

    def receipt(self, lane='serial', mode='serial'):
        return {
            'schema': 2,
            'metadata': {'revision': 'a' * 40},
            'cache': {'mode': 'off'},
            'provenance': {'run_id': '101', 'run_attempt': '2'},
            'validation': {
                'ci_mode': mode, 'dependency_cache': 'warm', 'lane': lane, 'qualification_id': 'cohort-1',
                'repository': 'pennel-app/pennel', 'event_name': 'pull_request',
                'source_sha': 'a' * 40, 'head_sha': 'b' * 40,
                'job_id': 'ios-native-tests' if lane == 'runtime' else 'ios-generated-build',
                'expected_phases': PHASES[lane],
            },
            'phases': [{'name': phase, 'seconds': 1.0, 'exit_code': 0} for phase in PHASES[lane]],
        }

    def check(self, needs=None, receipts=None, **changes):
        options = {
            'event_name': 'pull_request', 'repository': 'pennel-app/pennel',
            'source_sha': 'a' * 40, 'head_sha': 'b' * 40,
            'run_id': '101', 'run_attempt': '2', 'cache_mode': 'off', 'repetitions': 1,
        } | changes
        return self.status.failures(needs or self.needs(), receipts if receipts is not None else [self.receipt()], **options)

    def test_serial_requires_the_original_complete_pipeline_and_intentional_skip(self):
        self.assertEqual(self.check(), [])
        self.assertTrue(self.check(self.needs(runtime='success')))

    def test_candidate_requires_both_independent_lanes(self):
        receipts = [self.receipt('release', 'candidate'), self.receipt('runtime', 'candidate')]
        self.assertEqual(self.check(self.needs('candidate'), receipts), [])
        for job in ('ios-generated-build', 'ios-native-tests'):
            for result in ('failure', 'cancelled', 'skipped', 'pending'):
                with self.subTest(job=job, result=result):
                    needs = self.needs('candidate')
                    needs[job]['result'] = result
                    self.assertTrue(self.check(needs, receipts))

    def test_missing_invalid_or_failed_mode_selection_never_passes(self):
        for mode in ('', 'split', None):
            with self.subTest(mode=mode):
                self.assertTrue(self.check(self.needs(mode)))
        for result in ('failure', 'cancelled', 'skipped'):
            needs = self.needs()
            needs['changes']['result'] = result
            self.assertTrue(self.check(needs))
        needs = self.needs()
        del needs['changes']['outputs']['ci_mode']
        self.assertTrue(self.check(needs))

    def test_missing_duplicate_or_unexpected_lane_receipts_never_pass(self):
        release = self.receipt('release', 'candidate')
        runtime = self.receipt('runtime', 'candidate')
        for receipts in ([], [release], [release, release], [release, runtime, self.receipt()]):
            with self.subTest(count=len(receipts)):
                self.assertTrue(self.check(self.needs('candidate'), receipts))

    def test_source_attempt_run_head_and_mode_provenance_are_bound_to_the_gate(self):
        changes = [('validation', 'source_sha', 'c' * 40),
                   ('validation', 'head_sha', 'c' * 40),
                   ('validation', 'repository', 'someone/else'),
                   ('validation', 'event_name', 'push'),
                   ('validation', 'qualification_id', 'older-cohort'),
                   ('validation', 'ci_mode', 'candidate'),
                   ('validation', 'dependency_cache', 'cold'),
                   ('validation', 'job_id', 'ios-native-tests'),
                   ('metadata', 'revision', 'c' * 40),
                   ('provenance', 'run_id', '100'),
                   ('provenance', 'run_attempt', '1')]
        for section, field, value in changes:
            with self.subTest(section=section, field=field):
                receipt = self.receipt()
                receipt[section][field] = value
                self.assertTrue(self.check(receipts=[receipt]))

    def test_all_required_phases_must_execute_successfully_exactly_once(self):
        for change in ('missing', 'duplicate', 'failed', 'invalid-duration', 'unexpected', 'wrong-manifest'):
            with self.subTest(change=change):
                receipt = self.receipt()
                if change == 'missing':
                    receipt['phases'].pop()
                elif change == 'duplicate':
                    receipt['phases'].append(receipt['phases'][0].copy())
                elif change == 'failed':
                    receipt['phases'][-1]['exit_code'] = 1
                elif change == 'invalid-duration':
                    receipt['phases'][-1]['seconds'] = float('nan')
                elif change == 'unexpected':
                    receipt['phases'].append({'name': 'other-phase', 'seconds': 1, 'exit_code': 0})
                else:
                    receipt['validation']['expected_phases'] = PHASES['release']
                self.assertTrue(self.check(receipts=[receipt]))

    def test_ordinary_runs_cannot_enable_compiler_caching(self):
        receipt = self.receipt()
        receipt['cache']['mode'] = 'warm'
        self.assertTrue(self.check(receipts=[receipt]))

    def test_malformed_receipt_sections_fail_closed(self):
        for section in ('metadata', 'provenance', 'validation', 'cache', 'phases'):
            for invalid in (None, 'invalid', []):
                with self.subTest(section=section, invalid=invalid):
                    receipt = self.receipt()
                    receipt[section] = invalid
                    self.assertTrue(self.check(receipts=[receipt]))

    def test_cold_dependency_bypass_is_explicit_and_not_fabricated_as_a_hit_or_miss(self):
        needs = self.needs()
        needs['changes']['outputs']['dependency_cache'] = 'cold'
        receipt = self.receipt()
        receipt['validation']['dependency_cache'] = 'cold'
        receipt['rust_cache'] = {'bypassed': True, 'exact_hit': None, 'bypass_scope': ['rust', 'npm']}
        self.assertEqual(self.check(needs, [receipt]), [])
        for field, value in [('exact_hit', True), ('exact_hit', False), ('bypassed', False), ('bypass_scope', ['rust'])]:
            with self.subTest(field=field, value=value):
                invalid = receipt | {'rust_cache': receipt['rust_cache'] | {field: value}}
                self.assertTrue(self.check(needs, [invalid]))

    def legacy_receipt(self, repetition='1', cache_mode='off'):
        receipt = self.receipt()
        receipt['schema'] = 1
        del receipt['validation']
        receipt['repetition'] = repetition
        receipt['cache'] = {'mode': cache_mode, 'exact_hit': cache_mode == 'warm',
                            'save_allowed': True, 'archive_bytes': 100}
        if cache_mode in ('cold', 'warm'):
            receipt['phases'].append({'name': 'cache-pack' if cache_mode == 'cold' else 'cache-unpack',
                                      'seconds': 1, 'exit_code': 0})
        return receipt

    def test_manual_dispatch_keeps_all_original_repetitions_and_native_phases(self):
        receipts = [self.legacy_receipt(str(i), 'warm') for i in (1, 2, 3)]
        self.assertEqual(self.check(receipts=receipts, event_name='workflow_dispatch', cache_mode='warm', repetitions=3), [])
        self.assertTrue(self.check(receipts=receipts[:2], event_name='workflow_dispatch', cache_mode='warm', repetitions=3))
        self.assertTrue(self.check(self.needs('candidate'), receipts, event_name='workflow_dispatch', cache_mode='warm', repetitions=3))
        receipts[0]['phases'].pop(4)
        self.assertTrue(self.check(receipts=receipts, event_name='workflow_dispatch', cache_mode='warm', repetitions=3))

    def test_manual_cold_and_warm_preserve_snapshot_acceptance(self):
        self.assertEqual(self.check(receipts=[self.legacy_receipt(cache_mode='cold')],
                                    event_name='workflow_dispatch', cache_mode='cold'), [])
        receipt = self.legacy_receipt(cache_mode='warm')
        receipt['cache']['exact_hit'] = False
        self.assertTrue(self.check(receipts=[receipt], event_name='workflow_dispatch', cache_mode='warm'))
        receipt = self.legacy_receipt(cache_mode='cold')
        receipt['cache']['save_allowed'] = False
        self.assertTrue(self.check(receipts=[receipt], event_name='workflow_dispatch', cache_mode='cold'))

    def test_manual_retry_can_retain_original_successful_siblings_without_recounting_them(self):
        receipts = [self.legacy_receipt(str(i), 'warm') for i in (1, 2, 3)]
        receipts[0]['provenance']['run_attempt'] = '1'
        self.assertEqual(self.check(receipts=receipts, event_name='workflow_dispatch', cache_mode='warm', repetitions=3), [])
        receipts[0]['provenance']['run_attempt'] = '3'
        self.assertTrue(self.check(receipts=receipts, event_name='workflow_dispatch', cache_mode='warm', repetitions=3))


if __name__ == '__main__':
    unittest.main()
