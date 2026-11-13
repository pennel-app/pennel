import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
import uuid
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished_local_experiment as local
import finished_qualification as qualification


CONFIG = {'protocol': 'local_decider', 'endpoint': local.ENDPOINT, 'model': local.MODEL,
    'expected_model': 'sha256:'+local.ARTIFACT, 'upstream': None, 'max_input_bytes': 65536, 'timeout_secs': 90}
ACTUAL = {'model': CONFIG['expected_model'], 'artifact_sha256': local.ARTIFACT,
    'provider': 'Pennel local Decider', 'revision': 'a'*64, 'identity_evidence': 'serving_revision_and_artifact'}


def exported():
    request = {'state': 'Invented Dutch fixture', 'questions': [{'id': 'action', 'kind': 'choice',
        'instructions': 'Classify this invented fixture', 'options': [{'id': 'none', 'description': 'Absent'}]}]}
    wire = json.dumps(request)
    return {'schema_version': 3, 'protocol_id': 'production-finished-meeting-v3',
        'runtime_policy': 'raw_supported_passage_decisions_v3', 'freeze_sha256': 'b'*64,
        'fixture': {'id': 'nl-managed-v1-development-f00-v0', 'family_id': 'owned', 'split': 'development', 'language': 'nl', 'utterances': []},
        'configuration': copy.deepcopy(CONFIG), 'expected_actual': copy.deepcopy(ACTUAL),
        'batches': [{'index': 0, 'request': request, 'wire_body': wire}]}


class Bridge:
    binary = 'owned-test-bridge'
    artifact_sha256 = 'c'*64
    def __call__(self, _): return exported()
    def call_before(self, _, deadline): return exported()


class Client:
    def __init__(self): self.calls = []; self.fail = False; self.missing_native = False
    def call(self, value, deadline):
        self.calls.append(copy.deepcopy(value))
        if self.fail: return {'outcome': {'status': 'local_request_failed'}, 'attempts': []}
        request = value['export']['batches'][0]['request']
        body = json.dumps(request)
        result = {'identity': {'protocol': 'local_decider', 'endpoint': local.ENDPOINT,
            'requested_model': local.MODEL, 'requested_upstream': None, 'actual_model': ACTUAL['model'],
            'actual_revision': ACTUAL['revision'], 'actual_provider': ACTUAL['provider'],
            'request_id': str(uuid.uuid4())}, 'answers': [],
            'usage': {'billed_cost_usd': 0.0, 'input_tokens': 50, 'output_tokens': 0}}
        attempt = {'request': request, 'request_body': body, 'request_sha256': hashlib.sha256(body.encode()).hexdigest(),
            'request_capture': 'validated_Engine_request_before_managed_local_scheduler',
            'response_capture': 'validated_production_DecisionResult', 'raw_worker_io_available': False,
            'result': result, 'native_execution': {'native_decode_calls': 1,
                'independent_question_sequences': 1, 'expanded_input_tokens': 50}}
        if self.missing_native: attempt.pop('native_execution')
        return {'attempts': [attempt], 'outcome': {'status': 'ok', 'result': result, 'replay': {'status': 'ok'}}}


class ManagedLocalExecutionTests(unittest.TestCase):
    def setUp(self): self.directory = tempfile.TemporaryDirectory(); self.client = Client()
    def tearDown(self): self.directory.cleanup()
    def run_job(self, name='run', **kwargs):
        return local.run_job(exported(), 'd'*64, Bridge(), self.client,
            Path(self.directory.name)/(name+'.json'), execute=True, **kwargs)
    def test_actual_typed_engine_and_native_counts_are_recorded_without_fabricated_http(self):
        run = self.run_job()
        self.assertEqual(run['status'], 'ok')
        self.assertEqual(run['measurements']['executed_requests'], 1)
        self.assertEqual(run['measurements']['executed_native_decode_calls'], 1)
        self.assertEqual(set(run['responses'][0]), {'batch_index', 'production_result'})
        self.assertFalse(run['raw_worker_io_available'])
        self.assertEqual(run['spend_ledger']['known_billed_usd'], '0')
        self.assertTrue(qualification._execution_reconciled(run))
    def test_a_failed_attempt_is_not_retried(self):
        self.client.fail = True
        run = self.run_job()
        self.assertEqual(run['status'], 'blocked')
        self.assertEqual(len(self.client.calls), 1)
        self.assertEqual(run['responses'], [])
    def test_actual_native_execution_evidence_is_required(self):
        self.client.missing_native = True
        run = self.run_job()
        self.assertEqual(run['status'], 'blocked')
        self.assertEqual(len(self.client.calls), 1)
        self.assertIsNone(run['measurements']['executed_native_decode_calls'])
    def test_two_original_jobs_reuse_the_same_client_with_distinct_run_ids(self):
        self.run_job('first'); self.run_job('second')
        self.assertEqual(len(self.client.calls), 2)
        self.assertNotEqual(self.client.calls[0]['run_id'], self.client.calls[1]['run_id'])
    def test_existing_artifacts_and_unrecognized_routes_stop_before_another_attempt(self):
        self.run_job()
        with self.assertRaisesRegex(ValueError, 'preserve_existing'):
            self.run_job()
        for key in ('protocol', 'endpoint', 'model', 'expected_model', 'upstream'):
            value = exported(); value['configuration'][key] = 'unreviewed'
            with self.assertRaises(ValueError): local.route(value)
    def test_execution_flag_is_required_before_runtime_or_model_access(self):
        with patch.object(sys, 'argv', ['actor', 'preflight', '--bridge', 'unused', '--runtime-bundle', 'unused',
                '--worker', 'unused', '--model', 'unused', '--output', 'unused', '--deadline-unix', '1']):
            with patch.object(local, 'LocalClient', side_effect=AssertionError('No model access')):
                with self.assertRaisesRegex(ValueError, 'execution_not_enabled'): local.main()
    def test_local_source_freeze_includes_the_actual_runner(self):
        hashes = qualification._source_hashes(local=True)
        self.assertIn('finished_local_experiment.py', hashes)
        self.assertEqual(hashes['finished_local_experiment.py'], local.finished.file_hash(local.__file__))
        self.assertIn('evaluation_io.py', hashes)
        self.assertIn('evaluation_io.py', qualification._source_hashes(cloud=True))
    def test_local_deadline_failure_stops_the_whole_owned_group_once(self):
        client = local.LocalClient.__new__(local.LocalClient)
        client.process = MagicMock(); client.binary = 'owned-test'; client.sha256 = 'b'*64
        client.buffer = bytearray(); client.group_stopped = False
        with patch.object(local.finished, 'file_hash', return_value=client.sha256):
            with patch.object(local, 'jsonl_exchange', side_effect=ValueError('local_client_deadline_exceeded')):
                with patch.object(local, 'stop_owned_group') as stop:
                    with self.assertRaisesRegex(ValueError, 'deadline_exceeded'):
                        client.call({'owned': 'fixture'}, 1)
                    client.stop()
                    stop.assert_called_once_with(client.process)
    def test_an_exited_leader_does_not_skip_owned_descendant_cleanup(self):
        client = local.LocalClient.__new__(local.LocalClient)
        client.process = MagicMock(); client.process.poll.return_value = 0
        client.group_stopped = False
        with patch.object(local, 'stop_owned_group') as stop:
            client.stop()
            stop.assert_called_once_with(client.process)


if __name__ == '__main__': unittest.main()
