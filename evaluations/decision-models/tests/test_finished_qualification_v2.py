import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished
import finished_qualification as qualification
import finished_corpus_v2 as corpus

KINDS = corpus.KINDS
ACTUAL = {'model': 'perplexity/pplx-decider-v1.2-20261016', 'provider': 'Perplexity', 'revision': '',
          'artifact_sha256': '', 'identity_evidence': 'provider_reported_version'}
CONFIG = corpus.experiment()['configuration']


class BatchedBridge:
    artifact_sha256 = 'b' * 64
    def __call__(self, request):
        if request['op'] == 'finished_meeting_export':
            fixture = request['fixture']; windows, batches = [], []
            for index, row in enumerate(fixture['utterances']):
                source = {'meeting_uid': 'fixture-meeting', 'utterance_id': f'{fixture["id"]}-{index}',
                    'source_fingerprint': finished.digest(row), 'quote': row['text'], 'start_byte': 0,
                    'end_byte': len(row['text'].encode()), 'start_ms': row['start_ms'], 'end_ms': row['end_ms']}
                windows.append({'index': index, 'batch_index': index // 8, 'batch_slot': index % 8,
                    'prompt_context_sha256': finished.digest({'size': min(8, len(fixture['utterances']) - index // 8 * 8), 'slot': index % 8}),
                    'request': {'questions': [{'id': key, 'kind': 'choice', 'options': [{'id': 'none'}, {'id': 'confirmed'}]} for key in KINDS]},
                    'sources': [{'source': source}]})
            for start in range(0, len(windows), 8):
                ids = list(range(start, min(start + 8, len(windows))))
                mapping = [{'window_index': index, 'original_id': kind, 'batch_id': f'w{index-start}_{kind}'} for index in ids for kind in KINDS]
                body = json.dumps({'fixture': fixture['id'], 'window_indices': ids, 'questions': {x['batch_id']: {} for x in mapping}}, sort_keys=True)
                batches.append({'index': len(batches), 'window_indices': ids, 'answer_map': mapping, 'wire_body': body,
                                'wire_body_sha256': hashlib.sha256(body.encode()).hexdigest(), 'request': {'questions': mapping}})
            value = {'schema_version': 2, 'protocol_id': 'production-finished-meeting-v2', 'fixture': fixture,
                'configuration': request['configuration'], 'expected_actual': request['expected_actual'],
                'qualification_request': {'task': 'meeting_suggestions_v1', 'language_policy': 'nl',
                    'grouping': 'fixture-eight-window-batches', 'resource_profile_sha256': '9' * 64},
                'source_sha256': {'production': 'a' * 64}, 'production_admission': False, 'windows': windows, 'batches': batches}
            value['freeze_sha256'] = finished.digest(value)
            return value
        exported = request['export']; rows = []
        for response in request['responses']:
            batch = exported['batches'][response['batch_index']]
            labels = json.loads(response['body'])
            for index in batch['window_indices']:
                window = exported['windows'][index]; source = window['sources'][0]['source']
                answers, candidates = [], []
                for kind in KINDS:
                    label = labels[f'w{window["batch_slot"]}_{kind}']
                    answers.append({'id': kind, 'choice': label, 'abstained': False,
                        'probabilities': [{'option_id': option, 'probability': float(label == option)} for option in ['none', 'confirmed']]})
                    if label == 'confirmed': candidates.append({'kind': kind, 'commitment': label, 'text': source['quote'],
                        'owner': None, 'due': None, 'sources': [source]})
                rows.append({'window_index': index, 'status': 'ok', 'result': {'answers': answers}, 'candidates': candidates})
        return {'status': 'ok', 'complete': len(request['responses']) == len(exported['batches']), 'windows': rows,
                'production_admission': False, 'batches': []}

    def call_before(self, request, deadline): return self(request)


def experiment(bridge):
    fixtures, expected, gold = [], {}, {}
    for split in ['development', 'calibration', 'heldout']:
        for family in range(12):
            identifier = f'{split}-{family}'
            utterances = [{'start_ms': i * 1000, 'end_ms': i * 1000 + 900, 'speaker': 'others',
                           'text': f'Fictieve bron {split} familie {family} passage {i}.'} for i in range(16)]
            fixtures.append({'id': identifier, 'family_id': identifier, 'split': split, 'language': 'nl', 'utterances': utterances})
            expected[identifier] = [{key: 'confirmed' if (family+i) % 2 else 'none' for key in KINDS} for i in range(16)]
            gold[identifier] = [{'window_index': i, 'kind': key, 'commitment': label, 'text': utterances[i]['text'], 'owner': None, 'due': None}
                                for i, labels in enumerate(expected[identifier]) for key, label in labels.items() if label == 'confirmed']
    return qualification.freeze(fixtures, CONFIG, bridge, ACTUAL, expected, gold)


def predictions(frozen, split, evidence='offline_fixture'):
    result = []
    for exported in frozen['production']['exports']:
        if exported['fixture']['split'] != split: continue
        identifier = exported['fixture']['id']; responses = []
        for batch in exported['batches']:
            body = {row['batch_id']: frozen['production']['expected_answers'][identifier][row['window_index']][row['original_id']] for row in batch['answer_map']}
            responses.append({'batch_index': batch['index'], 'http_status': 200, 'body': json.dumps(body)})
        result.append({'fixture_id': identifier, 'freeze_sha256': frozen['freeze_sha256'], 'export_sha256': exported['freeze_sha256'],
            'evidence_class': evidence, 'status': 'ok', 'reason': None, 'spend_ledger': {'limit_usd': '2.00', 'known_billed_usd': '0.00001', 'reserved_usd': '0', 'pending_reservations': {}, 'unresolved_charge': False}, 'request_sha256s': [batch['wire_body_sha256'] for batch in exported['batches']],
            'responses': responses, 'measurements': {'whole_job_elapsed_ms': 100, 'executed_requests': len(responses)}})
    return result


class BatchedQualificationTests(unittest.TestCase):
    def setUp(self): self.bridge = BatchedBridge()

    def test_provider_reported_evidence_never_requires_fabricated_revision_or_weights(self):
        frozen = experiment(self.bridge)
        self.assertEqual(frozen['protocol_id'], 'production-finished-qualification-v2')
        self.assertEqual(frozen['production']['exports'][0]['expected_actual']['revision'], '')
        self.assertEqual(frozen['production']['exports'][0]['expected_actual']['artifact_sha256'], '')
        self.assertEqual(frozen['runtime_policy'], 'raw_closed_set_passage_decisions_v2')
        wrong = copy.deepcopy(ACTUAL); wrong.pop('identity_evidence')
        with self.assertRaisesRegex(ValueError, 'verified_actual_identity_required'):
            qualification.freeze([], CONFIG, self.bridge, wrong, {}, {})

    def test_requests_are_physical_batches_and_calibration_context_is_slot_bound(self):
        frozen = experiment(self.bridge)
        requests = qualification.requests(frozen, 'calibration', self.bridge)
        self.assertEqual(len(requests), 24)
        self.assertTrue(all(row['batch_index'] in [0, 1] for row in requests))
        rows, _, _ = qualification._runs(frozen, predictions(frozen, 'calibration'), self.bridge, 'calibration')
        hashes = {row['window_index']: row['prompt_hash'] for row in rows if row['fixture_id'] == 'calibration-0' and row['component'] == 'action'}
        self.assertNotEqual(hashes[0], hashes[1])
        self.assertEqual(hashes[0], hashes[8])

    def test_complete_resource_jobs_count_physical_requests_not_logical_windows(self):
        frozen = experiment(self.bridge)
        ready = qualification.calibrate(frozen, predictions(frozen, 'calibration'), self.bridge)
        run = predictions(ready, 'development', 'recorded_inference')[0]
        resources = {'freeze_sha256': ready['freeze_sha256'], 'resource_profile_sha256': '9' * 64,
            'scope': 'owned_client', 'cpu_time_ms': 100, 'peak_rss_bytes': 1000,
            'samples': [{'id': f'{phase}-{i}', 'phase': phase, 'run': copy.deepcopy(run)} for phase, count in [('cold', 5), ('warm', 30)] for i in range(count)]}
        report = qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge, resources)
        self.assertTrue(report['resources']['complete'])
        self.assertEqual(report['resources']['executed_requests'], 70)
        self.assertFalse(report['qualification'])
        self.assertFalse(report['runtime_calibration_applied'])
        self.assertEqual(report['candidate_records'], [])
        resources['samples'][0]['run']['measurements']['executed_requests'] = 16
        self.assertFalse(qualification.evaluate(ready, predictions(ready, 'heldout'), self.bridge, resources)['resources']['complete'])

    def test_unreconciled_or_failed_original_execution_cannot_emit_candidates(self):
        frozen = experiment(self.bridge)
        ready = qualification.calibrate(frozen, predictions(frozen, 'calibration', 'recorded_inference'), self.bridge)
        heldout = predictions(ready, 'heldout', 'recorded_inference')
        run = predictions(ready, 'development', 'recorded_inference')[0]
        resources = {'freeze_sha256': ready['freeze_sha256'], 'resource_profile_sha256': '9' * 64,
            'scope': 'owned_client', 'cpu_time_ms': 100, 'peak_rss_bytes': 1000,
            'samples': [{'id': f'{phase}-{i}', 'phase': phase, 'run': copy.deepcopy(run)}
                for phase, count in [('cold', 5), ('warm', 30)] for i in range(count)]}
        self.assertTrue(qualification.evaluate(ready, heldout, self.bridge, resources)['qualification'])
        for mutate in [lambda row: row.update(status='blocked', reason='unresolved_charge'),
                       lambda row: row['spend_ledger'].update(unresolved_charge=True),
                       lambda row: row['spend_ledger'].update(pending_reservations={'uncertain': '.10'}, reserved_usd='.10')]:
            changed = copy.deepcopy(heldout)
            mutate(changed[-1])
            report = qualification.evaluate(ready, changed, self.bridge, resources)
            self.assertFalse(report['qualification'])
            self.assertEqual(report['candidate_records'], [])
            self.assertIn('original_execution_failed_or_unreconciled', report['gates'])
            self.assertEqual(report['raw_component_metrics']['macro_f1'], 1)
        resources['samples'][-1]['run'].update(status='blocked', reason='unresolved_charge')
        self.assertFalse(qualification.evaluate(ready, heldout, self.bridge, resources)['resources']['complete'])

    def test_complete_freeze_reexports_obey_the_remaining_global_deadline(self):
        frozen = experiment(self.bridge)
        clock = [0.0]
        calls = []
        delegate = self.bridge
        class SlowBridge:
            artifact_sha256 = delegate.artifact_sha256
            def __call__(self, request): return delegate(request)
            def call_before(self, request, deadline):
                calls.append(deadline)
                clock[0] += .1
                return delegate(request)
        with self.assertRaisesRegex(ValueError, 'core_bridge_timeout|evaluation_budget_exceeded'):
            qualification.verify_freeze(frozen, SlowBridge(), deadline=.05, clock=lambda: clock[0])
        self.assertEqual(calls, [.05])


if __name__ == '__main__': unittest.main()
