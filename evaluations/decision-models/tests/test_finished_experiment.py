import copy
from decimal import Decimal
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import subprocess

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished
import finished_corpus_v2 as corpus
import finished_experiment as experiment
from runners import SpendLedger
from test_finished_qualification_v2 import BatchedBridge, ACTUAL, CONFIG


class Bridge(BatchedBridge):
    def __call__(self, request):
        if request['op'] == 'finished_meeting_replay':
            request = copy.deepcopy(request)
            for row in request['responses']:
                parsed = json.loads(row['body'])
                row['body'] = json.dumps({key: answer['choice'] for key, answer in parsed['answers'].items()})
        value = super().__call__(request)
        if request['op'] == 'finished_meeting_export':
            for batch in value['batches']:
                body = json.loads(batch['wire_body']);body.update(model=CONFIG['model'],
                    provider={'only': ['perplexity'], 'allow_fallbacks': False, 'data_collection': 'deny', 'zdr': True})
                batch['wire_body'] = json.dumps(body, sort_keys=True)
                batch['wire_body_sha256'] = finished.digest(body)
            value['freeze_sha256'] = finished.digest({k: v for k, v in value.items() if k != 'freeze_sha256'})
        return value


class ExecutionTests(unittest.TestCase):
    def setUp(self):
        self.bridge = Bridge()
        self.fixture = corpus.fixture('development', 0, 0, 16, '-resource-panel')[0]
        self.exported = self.bridge({'op': 'finished_meeting_export', 'fixture': self.fixture, 'configuration': CONFIG, 'expected_actual': ACTUAL})
        self.clock = [0.0]
        self.calls = []
        self.ledger = SpendLedger()
        self.ledger.spent = Decimal('0.00001416')
        self.directory = tempfile.TemporaryDirectory()
        self.output = Path(self.directory.name) / 'run.json'
    def tearDown(self): self.directory.cleanup()
    def transport(self, endpoint, body, timeout, credential):
        self.calls.append((endpoint, body, timeout));self.clock[0] += .2
        questions = json.loads(body)['questions']
        return 200, json.dumps({'model': ACTUAL['model'], 'provider': ACTUAL['provider'], 'usage': {'cost': .00001},
            'answers': {key: {'type': 'choice', 'choice': 'none', 'probabilities': {'none': 1, 'confirmed': 0}} for key in questions}}).encode()
    def run_job(self, **kwargs):
        return experiment.run_job(self.exported, 'f' * 64, self.bridge, 'not-a-real-key', self.ledger, self.output,
            transport=kwargs.pop('transport', self.transport), clock=lambda: self.clock[0], execute=True, **kwargs)

    def test_actual_wire_bytes_are_sent_once_and_existing_spend_is_preserved(self):
        run = self.run_job()
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.calls[0][1], self.exported['batches'][0]['wire_body'].encode())
        self.assertEqual(run['measurements']['executed_requests'], 2)
        self.assertEqual(run['status'], 'ok')
        self.assertEqual(len(run['responses']), 2)
        self.assertEqual(self.ledger.spent, Decimal('0.00003416'))
        self.assertEqual(json.loads(self.output.read_text())['responses'], run['responses'])
        self.assertNotIn('not-a-real-key', self.output.read_text())

    def test_absolute_job_deadline_prevents_a_second_request_but_keeps_consumed_reply(self):
        def late(*args):
            status, body = self.transport(*args);self.clock[0] = 91;return status, body
        run = self.run_job(transport=late)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(run['reason'], 'job_deadline_exceeded')
        self.assertEqual(len(run['responses']), 1)
        self.assertEqual(self.ledger.spent, Decimal('0.00002416'))
        self.assertEqual(len(run['requests']), 1)

    def test_transport_failure_is_not_retried_and_unknown_charge_blocks_future_work(self):
        def fail(*args):
            self.calls.append(args)
            raise finished.TransportFailure('transport_failed', None, b'partial response')
        run = self.run_job(transport=fail)
        self.assertEqual(len(self.calls), 1)
        self.assertTrue(self.ledger.unresolved)
        self.assertEqual(run['requests'][0]['raw_response'], 'partial response')
        self.assertEqual(run['status'], 'blocked')
        self.assertIn('raw_response', json.loads(self.output.read_text())['requests'][0])

    def test_foreign_or_changed_source_never_sends_content(self):
        self.exported['fixture']['utterances'][0]['text'] = 'Not the committed invented corpus.'
        with self.assertRaisesRegex(ValueError, 'approved_invented'):
            self.run_job()
        self.assertEqual(self.calls, [])

    def test_isolated_client_reuses_only_its_process_and_is_reaped(self):
        script = "import sys,json,os\nfor line in sys.stdin:\n json.loads(line);print(json.dumps({'status':'ok','reason':None,'pid':os.getpid()}),flush=True)"
        value = {'export': {'configuration': {'timeout_secs': 1}}, 'global_deadline': time.monotonic() + 5}
        with experiment.JobClient([sys.executable, '-c', script]) as client:
            first = client.call(value);second = client.call(value)
            self.assertEqual(first['pid'], second['pid'])
            self.assertEqual(first['pid'], client.process.pid)
        self.assertIsNotNone(client.process.poll())
        with experiment.JobClient([sys.executable, '-c', script]) as fresh:
            self.assertNotEqual(fresh.call(value)['pid'], first['pid'])

    def test_isolated_client_deadline_kills_the_owned_process(self):
        script = "import sys,time\nsys.stdin.readline();time.sleep(10)"
        value = {'export': {'configuration': {'timeout_secs': .1}}, 'global_deadline': time.monotonic() + .3}
        with experiment.JobClient([sys.executable, '-c', script], grace_seconds=0) as client:
            with self.assertRaisesRegex(ValueError, 'client_deadline_exceeded'):
                client.call(value)
            self.assertIsNotNone(client.process.poll())

    def test_native_smoke_allowance_includes_conservative_exposure(self):
        token = self.ledger.reserve('1.79999', {'request_sha256': 'a'*64})
        self.ledger.resolve_upper_bound(token, {'request_sha256': 'a'*64}, {'reason': 'synthetic interruption'})
        run = self.run_job()
        self.assertEqual(run['reason'], 'reserved_native_smoke_budget_unavailable')
        self.assertEqual(self.calls, [])
        self.assertEqual(self.ledger.spent, Decimal('0.00001416'))

    def test_credential_lookup_is_bounded_and_global_expiry_sends_nothing(self):
        with patch.object(experiment.subprocess, 'run', side_effect=subprocess.TimeoutExpired(['security'], 15)) as call:
            with self.assertRaisesRegex(ValueError, 'evaluation_credential_timeout'):
                experiment.credential(deadline=100, clock=lambda: 0)
            self.assertEqual(call.call_args.kwargs['timeout'], 15)
        with patch.object(experiment.subprocess, 'run') as call:
            with self.assertRaisesRegex(ValueError, 'evaluation_budget_exceeded'):
                experiment.credential(deadline=5, clock=lambda: 5)
            call.assert_not_called()

    def test_price_evidence_must_name_the_exact_model_and_upstream(self):
        price = Path(self.directory.name) / 'price.json'
        value = {'data': {'id': CONFIG['model'], 'endpoints': [{'provider_name': 'Perplexity', 'tag': 'perplexity',
            'model_id': CONFIG['model'], 'name': 'Perplexity | ' + ACTUAL['model'],
            'pricing': {'prompt': '.00000002', 'completion': '0'}}]}}
        price.write_text(json.dumps(value))
        self.assertEqual(experiment._prices(price), (Decimal('.00000002'), Decimal(0)))
        value['data']['id'] = 'perplexity/pplx-decider-flash-v1.2'
        price.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'exact_model_endpoint'):
            experiment._prices(price)

    def test_execution_is_explicit_and_reservation_bound_is_conservative(self):
        with self.assertRaisesRegex(ValueError, 'execution_not_enabled'):
            experiment.run_job(self.exported, 'f' * 64, self.bridge, 'not-a-real-key', self.ledger, self.output,
                               transport=self.transport)
        bound = experiment.reservation_bound(self.exported['batches'][0]['wire_body'], Decimal('.00000002'))
        self.assertGreater(bound, Decimal(0))
        self.assertLess(bound, Decimal('.10'))
        self.assertEqual(self.calls, [])


if __name__ == '__main__': unittest.main()
