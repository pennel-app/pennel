import copy
from decimal import Decimal
import hashlib
import io
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
import uuid
from unittest.mock import Mock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import finished_cloud_experiment as cloud
import evaluation_io
import finished_qualification as qualification
from runners import SpendLedger


CONFIG={'protocol':'pennel_cloud','endpoint':'https://api-staging.pennel.app/v1/decisions',
        'model':'pennel-review-v3','expected_model':'perplexity/pplx-decider-v1.2-20261016',
        'upstream':None,'max_input_bytes':65536,'timeout_secs':90}
ACTUAL={'identity_evidence':'provider_reported_version','model':CONFIG['expected_model'],
        'provider':'Perplexity','revision':'','artifact_sha256':''}


def exported():
    body={'model':CONFIG['model'],'expected_model':CONFIG['expected_model'],'state':'Invented source only.',
          'questions':{'action':{'type':'choice','instructions':'Classify invented source.','criteria':{'none':'Absent','confirmed':'Explicit'}}}}
    upstream=dict(body);upstream.pop('expected_model');upstream.update(model=cloud.common.MODEL,
        provider={'only':['perplexity'],'allow_fallbacks':False,'data_collection':'deny','zdr':True})
    wire=json.dumps(body)
    return {'schema_version':3,'protocol_id':'production-finished-meeting-v3','runtime_policy':'raw_supported_passage_decisions_v3',
        'fixture':{'id':'nl-cloud-development-owned','family_id':'owned','split':'development','language':'nl','utterances':[]},
        'configuration':CONFIG,'expected_actual':ACTUAL,'freeze_sha256':'a'*64,
        'batches':[{'index':0,'wire_body':wire,'managed_transport':{'canonical_upstream_contract':upstream}}]}


class Bridge:
    binary='owned-test-binary';artifact_sha256='b'*64
    def __call__(self,_):return exported()
    def call_before(self,_,deadline):return exported()


class Client:
    calls=[];fail=False
    def __init__(self,*_):self.sha256=Bridge.artifact_sha256
    def __enter__(self):return self
    def __exit__(self,*_):pass
    def call(self,value,deadline):
        self.calls.append(value)
        if self.fail:raise ValueError('owned-failure')
        body=json.loads(value['export']['batches'][0]['wire_body']);body['request_id']=str(uuid.uuid4())
        wire=json.dumps(body)
        response={'schema_version':1,'request_id':body['request_id'],'profile':cloud.PROFILE,
            'model':ACTUAL['model'],'provider':'Perplexity','usage':{'cost':.00001}}
        return {'outcome':{'status':'ok','replay':{'status':'ok'}},'attempts':[{'endpoint':CONFIG['endpoint'],
            'wire_body':wire,'wire_body_sha256':hashlib.sha256(wire.encode()).hexdigest(),
            'http_status':200,'response':response,'raw_http_bytes_available':False,
            'response_capture':'parsed_service_json_before_decision_decoder'}]}


class CloudExecutionTests(unittest.TestCase):
    def setUp(self):
        self.directory=tempfile.TemporaryDirectory();self.output=Path(self.directory.name)/'run.json'
        self.ledger=SpendLedger();self.ledger.spent=Decimal('.24420286');Client.calls=[];Client.fail=False
    def tearDown(self):self.directory.cleanup()
    def run_job(self,**kwargs):return cloud.run_job(exported(),'c'*64,Bridge(),b'owned-fake-session',self.ledger,
        self.output,execute=True,client_factory=Client,**kwargs)
    def test_actual_uuid_pin_capture_and_cost_are_kept_separate_from_template_hash(self):
        run=self.run_job()
        self.assertEqual(run['status'],'ok');self.assertEqual(len(Client.calls),1)
        self.assertNotEqual(run['request_sha256s'],run['actual_request_sha256s'])
        self.assertEqual(self.ledger.spent,Decimal('.24421286'))
        self.assertEqual(run['responses'][0]['request_id'],run['requests'][0]['request_id'])
        self.assertNotIn('owned-fake-session',self.output.read_text())
    def test_unknown_cost_is_not_retried_or_refunded(self):
        Client.fail=True;run=self.run_job()
        self.assertEqual(len(Client.calls),1);self.assertTrue(self.ledger.unresolved)
        self.assertEqual(run['reason'],'cloud_execution_failed')
    def test_existing_pending_cost_blocks_before_another_request(self):
        self.ledger.reserve('.1',{'owned':'pending'})
        run=self.run_job();self.assertEqual(run['reason'],'unreconciled_existing_charge')
        self.assertEqual(Client.calls,[])
    def test_unrecognized_route_or_pin_is_not_eligible(self):
        for change in ('endpoint','model','expected_model'):
            value=copy.deepcopy(exported());value['configuration'][change]='unreviewed'
            with self.assertRaises(ValueError):cloud.route(value)
    def test_cloud_protocol_keeps_every_original_quality_and_budget_gate(self):
        root=Path(cloud.__file__).parent
        direct=json.loads((root/'protocol-finished-qualification-v3.json').read_text())
        managed=json.loads((root/'protocol-finished-qualification-cloud-v1.json').read_text())
        self.assertEqual(direct['acceptance'],managed['acceptance'])
        self.assertEqual(direct['bounds'],managed['bounds'])
        self.assertTrue(qualification._valid_actual(CONFIG,ACTUAL))
        invalid=dict(CONFIG,endpoint='https://other.example/v1/decisions')
        self.assertFalse(qualification._valid_actual(invalid,ACTUAL))
    def test_internal_worker_still_requires_explicit_execution_before_session_access(self):
        with patch.object(sys,'argv',['actor','_client','--session-fd','3']):
            with patch.object(cloud,'session_from_fd',side_effect=AssertionError('No credential read')):
                with self.assertRaisesRegex(ValueError,'execution_not_enabled'):cloud.main()
    def test_explicit_disjoint_selections_preserve_frozen_order_without_claiming_a_complete_phase(self):
        jobs=list(range(96));combined=[]
        for start in range(0,96,24):
            selected,receipt=cloud.select_jobs('calibration',jobs,start,24)
            combined.extend(selected)
            self.assertFalse(receipt['entire_phase']);self.assertEqual(receipt['total_frozen_jobs'],96)
            self.assertEqual(receipt['start'],start);self.assertEqual(receipt['end_exclusive'],start+24)
        self.assertEqual(combined,jobs)
        self.assertTrue(cloud.select_jobs('heldout',jobs)[1]['entire_phase'])
    def test_resource_panels_and_invalid_selections_cannot_be_shortened(self):
        for phase,start,count in [('resources',0,5),('development',1,None),('calibration',-1,2),
                                  ('heldout',0,0),('heldout',90,7),('heldout',96,None),('heldout',True,2)]:
            with self.subTest(phase=phase,start=start,count=count):
                with self.assertRaisesRegex(ValueError,'invalid_explicit_job_selection'):
                    cloud.select_jobs(phase,list(range(96)),start,count)
    def test_secret_payload_is_piped_without_secret_arguments_or_inherited_secret_environment(self):
        script="import os,sys,json;fd=int(sys.argv[2]);v=os.read(fd,8192);os.close(fd);print(json.dumps({'bytes':len(v),'env_secret':'OPENROUTER_API_KEY' in os.environ,'arg_secret':any('owned-private-session' in a for a in sys.argv)}))"
        with patch.dict(cloud.os.environ,{'OPENROUTER_API_KEY':'owned-unrelated-environment-key'}):
            process=cloud._spawn([sys.executable,'-c',script],b'owned-private-session')
            process.stdin.close();value=json.loads(process.stdout.read());process.wait(timeout=5);process.stdout.close()
        self.assertEqual(value,{'bytes':21,'env_secret':False,'arg_secret':False})


class JsonlDeadlineTests(unittest.TestCase):
    def client(self,kind,script,*,pass_fds=()):
        client=object.__new__(kind)
        client.buffer=bytearray();client.grace_seconds=0
        client.binary=sys.executable;client.sha256=cloud.finished.file_hash(sys.executable)
        client.process=subprocess.Popen([sys.executable,'-u','-c',script],stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,start_new_session=True,pass_fds=pass_fds,
            text=kind is cloud.JobClient)
        def cleanup():
            client.process.poll()
            try:os.killpg(client.process.pid,signal.SIGKILL)
            except ProcessLookupError:pass
            client.process.wait()
            client.process.stdin.close();client.process.stdout.close()
        self.addCleanup(cleanup)
        return client
    def call(self,client,deadline,*,timeout=5,padding=''):
        value={'export':{'configuration':{'timeout_secs':timeout}},'global_deadline':deadline,'padding':padding}
        if isinstance(client,cloud.CloudClient):return client.call(value,deadline)
        return client.call(value)
    def test_group_cleanup_reaps_a_zombie_race_without_ignoring_live_permission_failure(self):
        process=SimpleNamespace(pid=123,poll=Mock(side_effect=[None,0]),wait=Mock())
        with patch.object(evaluation_io.os,'killpg',side_effect=[PermissionError(),ProcessLookupError()]) as kill:
            evaluation_io.stop_owned_group(process)
        self.assertEqual(kill.call_count,2);process.wait.assert_called_once_with()
        process=SimpleNamespace(pid=123,poll=Mock(return_value=None),wait=Mock())
        with patch.object(evaluation_io.os,'killpg',side_effect=PermissionError()):
            with self.assertRaises(PermissionError):evaluation_io.stop_owned_group(process)
        process.wait.assert_not_called()
    def test_blocked_request_pipe_obeys_deadline_and_reaps_child(self):
        script="import sys,time;time.sleep(.8);sys.stdin.buffer.readline();print('{\"status\":\"ok\"}',flush=True)"
        for kind in (cloud.CloudClient,cloud.JobClient):
            with self.subTest(kind=kind.__name__):
                client=self.client(kind,script);started=time.monotonic()
                with self.assertRaisesRegex(ValueError,'deadline_exceeded'):
                    self.call(client,started+.15,padding='x'*(2*1024*1024))
                self.assertLess(time.monotonic()-started,.65)
                self.assertIsNotNone(client.process.returncode)
    def test_partial_response_does_not_escape_global_deadline(self):
        script="import sys,time;sys.stdin.buffer.readline();sys.stdout.write('{\"status\":');sys.stdout.flush();time.sleep(.8);print('\"ok\"}',flush=True)"
        for kind in (cloud.CloudClient,cloud.JobClient):
            with self.subTest(kind=kind.__name__):
                client=self.client(kind,script);started=time.monotonic()
                with self.assertRaisesRegex(ValueError,'deadline_exceeded'):self.call(client,started+.15)
                self.assertLess(time.monotonic()-started,.65)
                self.assertIsNotNone(client.process.returncode)
    def test_partial_response_obeys_the_shorter_job_deadline(self):
        script="import sys,time;sys.stdin.buffer.readline();sys.stdout.write('{\"status\":');sys.stdout.flush();time.sleep(.8);print('\"ok\"}',flush=True)"
        client=self.client(cloud.JobClient,script);started=time.monotonic()
        with self.assertRaisesRegex(ValueError,'deadline_exceeded'):self.call(client,started+5,timeout=.15)
        self.assertLess(time.monotonic()-started,.65)
        self.assertIsNotNone(client.process.returncode)
    def test_valid_jsonl_keeps_one_worker_for_repeated_requests_and_unicode(self):
        script=("import json,os,sys\nfor line in sys.stdin:\n"
            " value=json.loads(line);print(json.dumps({'status':'ok','padding':value['padding'],'pid':os.getpid()}),flush=True)")
        for kind in (cloud.CloudClient,cloud.JobClient):
            with self.subTest(kind=kind.__name__):
                client=self.client(kind,script)
                for text in ('Vóór de vergadering','Actie voor één deelnemer'):
                    result=self.call(client,time.monotonic()+1,padding=text)
                    self.assertEqual(result['padding'],text);self.assertEqual(result['pid'],client.process.pid)
    def test_encode_cannot_submit_after_deadline(self):
        for kind in (cloud.CloudClient,cloud.JobClient):
            with self.subTest(kind=kind.__name__):
                clock=[0.0];encoded=json.dumps
                client=object.__new__(kind);client.grace_seconds=0;client.binary='owned';client.sha256='b'*64
                client.buffer=bytearray();client.process=SimpleNamespace(stdin=io.BytesIO() if kind is cloud.CloudClient else io.StringIO(),stdout=io.BytesIO(),
                    kill=lambda:None,wait=lambda:None,poll=lambda:None)
                def late_encode(*args,**kwargs):
                    result=encoded(*args,**kwargs);clock[0]=2.0;return result
                with patch.object(cloud.time,'monotonic',side_effect=lambda:clock[0]), \
                     patch.object(cloud.json,'dumps',side_effect=late_encode), \
                     patch.object(cloud.finished,'file_hash',return_value=client.sha256), \
                     patch.object(cloud,'stop_owned_group'):
                    with self.assertRaisesRegex(ValueError,'deadline_exceeded'):self.call(client,1.0)
                self.assertIn(client.process.stdin.getvalue(),(b'',''))
    def test_complete_response_decoded_after_deadline_is_rejected(self):
        script="import sys;sys.stdin.buffer.readline();print('{\"status\":\"ok\"}',flush=True)"
        for kind in (cloud.CloudClient,cloud.JobClient):
            with self.subTest(kind=kind.__name__):
                client=self.client(kind,script);clock=[0.0];monotonic=time.monotonic;decoded=json.loads
                def late_decode(*args,**kwargs):
                    result=decoded(*args,**kwargs);clock[0]=2.0;return result
                with patch.object(cloud.time,'monotonic',side_effect=lambda:monotonic()+clock[0]), \
                     patch.object(cloud.json,'loads',side_effect=late_decode):
                    with self.assertRaisesRegex(ValueError,'deadline_exceeded'):self.call(client,monotonic()+1)
                self.assertIsNotNone(client.process.returncode)
    def test_complete_frame_at_bound_succeeds_and_next_byte_is_rejected(self):
        limit=1024;empty=json.dumps({'status':'ok','padding':''})+'\n'
        for kind in (cloud.CloudClient,cloud.JobClient):
            for excess in (0,1):
                with self.subTest(kind=kind.__name__,excess=excess):
                    frame=json.dumps({'status':'ok','padding':'x'*(limit-len(empty)+excess)})+'\n'
                    script=f"import sys,time;sys.stdin.buffer.readline();sys.stdout.write({frame!r});sys.stdout.flush();time.sleep(.8)"
                    client=self.client(kind,script)
                    with patch.object(cloud.finished,'MAX_RESPONSE_BYTES',limit//8):
                        if excess:
                            with self.assertRaisesRegex(ValueError,'output_too_large'):self.call(client,time.monotonic()+1)
                            self.assertIsNotNone(client.process.returncode)
                        else:self.assertEqual(self.call(client,time.monotonic()+1)['status'],'ok')
    def test_outer_timeout_terminates_the_owned_descendant_group(self):
        reader,writer=os.pipe();self.addCleanup(os.close,reader)
        script=("import subprocess,sys,time;"
            f"subprocess.Popen([sys.executable,'-c','import time;time.sleep(5)'],pass_fds=({writer},));"
            "sys.stdin.buffer.readline();sys.stdout.write('{\"status\":');sys.stdout.flush();time.sleep(.8);print('\"ok\"}',flush=True)")
        try:client=self.client(cloud.JobClient,script,pass_fds=(writer,))
        finally:os.close(writer)
        with self.assertRaisesRegex(ValueError,'deadline_exceeded'):self.call(client,time.monotonic()+.15)
        self.assertIsNotNone(client.process.returncode)
        with selectors.DefaultSelector() as selector:
            selector.register(reader,selectors.EVENT_READ)
            self.assertTrue(selector.select(.5),'An owned descendant kept the pipe open after cancellation')
        self.assertEqual(os.read(reader,1),b'')
    def test_exited_supervisor_does_not_leave_its_owned_descendant_running(self):
        reader,writer=os.pipe();self.addCleanup(os.close,reader)
        script=("import subprocess,sys;"
            f"subprocess.Popen([sys.executable,'-c','import time;time.sleep(5)'],pass_fds=({writer},),stdout=subprocess.DEVNULL);"
            "sys.stdin.buffer.readline()")
        try:client=self.client(cloud.JobClient,script,pass_fds=(writer,))
        finally:os.close(writer)
        with self.assertRaises(ValueError):self.call(client,time.monotonic()+1)
        self.assertIsNotNone(client.process.returncode)
        with selectors.DefaultSelector() as selector:
            selector.register(reader,selectors.EVENT_READ)
            self.assertTrue(selector.select(.5),'An owned descendant survived its exited supervisor')
        self.assertEqual(os.read(reader,1),b'')
    def test_transport_deadline_keeps_pending_charge_without_retry(self):
        script="import sys,time;sys.stdin.buffer.readline();sys.stdout.write('{\"outcome\":');sys.stdout.flush();time.sleep(.8)"
        created=[];ledger=SpendLedger();ledger.spent=Decimal('.24420286')
        def factory(*_):
            client=self.client(cloud.CloudClient,script);client.sha256=Bridge.artifact_sha256
            created.append(client);return client
        with tempfile.TemporaryDirectory() as directory, \
             patch.dict(CONFIG,{'timeout_secs':.15}), \
             patch.object(cloud.finished,'file_hash',return_value=Bridge.artifact_sha256):
            output=Path(directory)/'run.json'
            run=cloud.run_job(exported(),'c'*64,Bridge(),b'owned-fake-session',ledger,output,
                execute=True,client_factory=factory)
            saved=json.loads(output.read_text())
        self.assertEqual(run['reason'],'cloud_execution_failed');self.assertEqual(saved['reason'],run['reason'])
        self.assertEqual(len(created),1);self.assertEqual(len(run['requests']),1)
        self.assertIsNotNone(created[0].process.returncode)
        self.assertTrue(ledger.unresolved);self.assertEqual(len(ledger.pending),1)
        self.assertEqual(ledger.spent,Decimal('.24420286'))
        self.assertEqual(ledger.total_exposure,Decimal('.34420286'))
        self.assertIsNone(ledger.attempts[0]['billed_cost_usd'])
        self.assertTrue(saved['spend_ledger']['unresolved_charge'])
        self.assertEqual(len(saved['spend_ledger']['pending_reservations']),1)
    def test_outer_timeout_preserves_durable_pending_reservation_and_blocks_later_jobs(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'ledger.json';ledger=SpendLedger();ledger.spent=Decimal('.24420286')
            cloud.common.write_json(path,ledger.snapshot())
            script=(f"import sys,time;sys.path.insert(0,{str(Path(cloud.__file__).parent)!r});"
                "import finished_experiment as common;sys.stdin.buffer.readline();"
                f"ledger=common.load_ledger({str(path)!r});ledger.reserve('.10',{{'owned':'interrupted'}});"
                "sys.stdout.write('{\"status\":');sys.stdout.flush();time.sleep(.8);print('\"ok\"}',flush=True)")
            client=self.client(cloud.JobClient,script)
            with self.assertRaisesRegex(ValueError,'deadline_exceeded'):self.call(client,time.monotonic()+.35)
            self.assertIsNotNone(client.process.returncode)
            retained=cloud.common.load_ledger(path)
            self.assertEqual(len(retained.pending),1);self.assertEqual(retained.attempts,[])
            self.assertEqual(retained.total_exposure,Decimal('.34420286'))
            factory=Mock(side_effect=AssertionError('No request after an interrupted charge'))
            run=cloud.run_job(exported(),'c'*64,Bridge(),b'owned-fake-session',retained,Path(directory)/'run.json',
                execute=True,client_factory=factory)
            self.assertEqual(run['reason'],'unreconciled_existing_charge');factory.assert_not_called()


if __name__=='__main__':unittest.main()
