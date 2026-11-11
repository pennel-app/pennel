import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished
import finished_qualification as qualification
from test_finished import FakeBridge, configuration, fixture


KINDS = ('action', 'decision', 'open_question', 'changed_commitment')
ACTUAL = {'model': 'fixture', 'revision': 'fixture-revision', 'provider': 'fixture-provider',
          'artifact_sha256': 'a' * 64}


class Bridge(FakeBridge):
    def __call__(self, request):
        value = super().__call__(request)
        if request['op'] == 'finished_meeting_export':
            value['windows'][0]['sources'] = [{'source': {'meeting_uid': 'meeting', 'utterance_id': request['fixture']['id'],
                'source_fingerprint': 'a' * 64, 'quote': request['fixture']['utterances'][0]['text'],
                'start_byte': 0, 'end_byte': len(request['fixture']['utterances'][0]['text'].encode()),
                'start_ms': 0, 'end_ms': 1000}}]
            value['qualification_request']['grouping'] = 'production-fixture'
            value['qualification_request']['resource_profile_sha256'] = '9' * 64
            value['freeze_sha256'] = finished.digest({k: v for k, v in value.items() if k != 'freeze_sha256'})
        else:
            for row, response in zip(value['windows'], request['responses']):
                expected = json.loads(response['body'])
                row['result'] = {'answers': [{'id': key, 'choice': expected[key], 'abstained': False,
                    'probabilities': [{'option_id': label, 'probability': float(label == expected[key])}
                                      for label in ['none', 'confirmed']]} for key in KINDS]}
                source = request['export']['windows'][row['window_index']]['sources'][0]['source']
                row['candidates'] = [{'kind': key, 'commitment': 'confirmed', 'text': source['quote'],
                    'owner': None, 'due': None, 'sources': [source]} for key in KINDS if expected[key] == 'confirmed']
        return value


def experiment(bridge):
    fixtures, answers, gold = [], {}, {}
    for split in ['development', 'calibration', 'heldout']:
        for index in range(12):
            item = fixture('nl', split, f'{split}-{index}')
            item['id'] = f'{split}-{index}'
            item['utterances'][0]['text'] = f'Fictieve bron {split} {index}.'
            fixtures.append(item)
            labels = {key: 'confirmed' if index % 2 else 'none' for key in KINDS}
            answers[item['id']] = [labels]
            gold[item['id']] = [{'window_index': 0, 'kind': key, 'commitment': 'confirmed',
                'text': item['utterances'][0]['text'], 'owner': None, 'due': None}
                for key, label in labels.items() if label != 'none']
    return qualification.freeze(fixtures, configuration(), bridge, ACTUAL, answers, gold)


def predictions(frozen, split, evidence='offline_fixture'):
    result = []
    for exported in frozen['production']['exports']:
        if exported['fixture']['split'] != split:
            continue
        identifier = exported['fixture']['id']
        result.append({'fixture_id': identifier, 'freeze_sha256': frozen['freeze_sha256'],
            'export_sha256': exported['freeze_sha256'], 'evidence_class': evidence,
            'request_sha256s': [hashlib.sha256(w['wire_body'].encode()).hexdigest() for w in exported['windows']],
            'responses': [{'window_index': 0, 'http_status': 200,
                'body': json.dumps(frozen['production']['expected_answers'][identifier][0])}],
            'measurements': {}})
    return result


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.bridge = Bridge('ok')
        self.frozen = experiment(self.bridge)

    def calibrated(self):
        return qualification.calibrate(self.frozen, predictions(self.frozen, 'calibration'), self.bridge)

    def test_heldout_export_requires_calibration_and_calibration_refuses_heldout(self):
        with self.assertRaisesRegex(ValueError, 'calibration_before_heldout'):
            qualification.requests(self.frozen, 'heldout', self.bridge)
        with self.assertRaisesRegex(ValueError, 'calibration_only'):
            qualification.calibrate(self.frozen, predictions(self.frozen, 'heldout'), self.bridge)
        ready = self.calibrated()
        self.assertTrue(qualification.requests(ready, 'heldout', self.bridge))

    def test_raw_production_body_and_prompt_identity_are_frozen(self):
        ready = self.calibrated()
        rows = qualification.requests(ready, 'heldout', self.bridge)
        self.assertEqual(rows[0]['wire_body'], ready['production']['exports'][24]['windows'][0]['wire_body'])
        changed = copy.deepcopy(ready)
        changed['gold_items']['heldout-1'][0]['owner'] = 'invented'
        with self.assertRaises(ValueError):
            qualification.requests(changed, 'heldout', self.bridge)

    def test_perfect_offline_fixture_cannot_become_a_qualified_route(self):
        ready = self.calibrated()
        report = qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge)
        self.assertEqual(report['raw_component_metrics']['macro_f1'], 1)
        self.assertEqual(report['complete_item_metrics']['precision'], 1)
        self.assertFalse(report['qualification'])
        self.assertIn('original_job_deadline_measurements_unavailable', report['gates'])
        self.assertFalse(report['production_admission'])
        self.assertIn('actual_inference_unmeasured', report['gates'])
        self.assertIn('resource_measurements_unavailable', report['gates'])
        self.assertEqual(report['candidate_records'], [])

    def test_missing_failed_windows_stay_in_denominators(self):
        ready = self.calibrated()
        rows = predictions(ready, 'heldout')
        rows[1]['responses'] = []
        rows[1]['request_sha256s'] = []
        report = qualification.evaluate(ready, rows, self.bridge)
        self.assertEqual(report['raw_component_metrics']['count'], 48)
        self.assertLess(report['complete_item_metrics']['recall'], 1)
        self.assertIn('production_jobs_incomplete', report['gates'])

    def test_repeated_or_unlinked_predictions_and_invalid_resources_are_rejected(self):
        ready = self.calibrated()
        original = predictions(ready, 'heldout')
        for mutate in [lambda rows: rows.append(rows[0]),
                       lambda rows: rows[0].update(export_sha256='b' * 64),
                       lambda rows: rows[0]['measurements'].update(cpu_time_ms=float('nan')),
                       lambda rows: rows[0]['measurements'].update(latency_ms=30001),
                       lambda rows: rows[0]['request_sha256s'].__setitem__(0, 'c' * 64)]:
            rows = copy.deepcopy(original)
            mutate(rows)
            with self.assertRaises(ValueError):
                qualification.evaluate(ready, rows, self.bridge)

    def test_incomplete_calibration_job_is_retained_as_a_gate(self):
        rows = predictions(self.frozen, 'calibration')
        rows[0]['responses'] = []
        rows[0]['request_sha256s'] = []
        ready = qualification.calibrate(self.frozen, rows, self.bridge)
        report = qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge)
        self.assertIn('calibration_jobs_incomplete', report['gates'])

    def test_resource_panel_is_fixed_source_bound_and_separate_from_quality(self):
        ready = self.calibrated()
        run = predictions(ready, 'development', 'recorded_inference')[0]
        run['measurements'] = {'whole_job_elapsed_ms': 20, 'executed_requests': 1}
        resource = {'freeze_sha256': ready['freeze_sha256'], 'resource_profile_sha256': '9' * 64,
            'scope': 'owned_client_and_local_model', 'cpu_time_ms': 100, 'peak_rss_bytes': 200,
            'samples': [{'id': f'{phase}-{index}', 'phase': phase, 'run': copy.deepcopy(run)}
                        for phase, count in [('cold', 5), ('warm', 30)] for index in range(count)]}
        report = qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge, resource)
        self.assertTrue(report['resources']['complete'])
        self.assertEqual(report['resources']['warm_p95_ms'], 20)
        self.assertEqual(report['raw_component_metrics']['count'], 48)
        self.assertFalse(report['qualification'])
        self.assertIn('original_job_deadline_measurements_unavailable', report['gates'])
        resource['samples'].pop()
        report = qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge, resource)
        self.assertIn('resource_measurements_unavailable', report['gates'])
        resource['samples'][0]['run']['request_sha256s'] = ['changed']
        with self.assertRaisesRegex(ValueError, 'unlinked_prediction'):
            qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge, resource)

    def test_independent_timing_cannot_replace_original_recorded_job_deadlines(self):
        calibration_rows = predictions(self.frozen, 'calibration', 'recorded_inference')
        for row in calibration_rows: row['measurements']['whole_job_elapsed_ms'] = 1
        ready = qualification.calibrate(self.frozen, calibration_rows, self.bridge)
        recorded = predictions(ready, 'heldout', 'recorded_inference')
        timing_run = predictions(ready, 'development', 'recorded_inference')[0]
        timing_run['measurements'] = {'whole_job_elapsed_ms': 20, 'executed_requests': 1}
        resource = {'freeze_sha256': ready['freeze_sha256'], 'resource_profile_sha256': '9' * 64,
            'scope': 'owned_client_and_local_model', 'cpu_time_ms': 100, 'peak_rss_bytes': 200,
            'samples': [{'id': f'{phase}-{index}', 'phase': phase, 'run': copy.deepcopy(timing_run)}
                        for phase, count in [('cold', 5), ('warm', 30)] for index in range(count)]}
        report = qualification.evaluate(ready, recorded, self.bridge, resource)
        self.assertFalse(report['qualification'])
        self.assertFalse(report['original_job_deadlines']['heldout_verified'])
        self.assertIn('original_job_deadline_measurements_unavailable', report['gates'])
        self.assertEqual(report['candidate_records'], [])

    def test_foreign_source_and_invented_owner_never_count_as_complete_items(self):
        ready = self.calibrated()
        base = self.bridge.__class__.__call__
        class Broken(Bridge):
            def __call__(self, request):
                value = base(self, request)
                if request['op'] == 'finished_meeting_replay':
                    for row in value['windows']:
                        for candidate in row['candidates']:
                            candidate['owner'] = {'text': 'invented', 'source_index': 0, 'start_byte': 0, 'end_byte': 8}
                            candidate['sources'][0] = {**candidate['sources'][0], 'source_fingerprint': 'f' * 64}
                return value
        report = qualification.evaluate(ready, predictions(ready, 'heldout'), Broken('ok'))
        self.assertEqual(report['complete_item_metrics']['correct_count'], 0)
        self.assertIn('complete_item_quality_failed', report['gates'])
        self.assertIn('unsupported_owner_due_state_or_source', report['gates'])


if __name__ == '__main__':
    unittest.main()
