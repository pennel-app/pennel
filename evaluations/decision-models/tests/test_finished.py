import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished


def fixture(language="en", split="development", family="cedar"):
    return {"id": "cedar_" + language, "family_id": family, "split": split,
            "language": language, "utterances": [
                {"start_ms": 0, "end_ms": 1000, "speaker": "others",
                 "text": "Invented Cedar planning: Noor will prepare the demo."}]}


def configuration():
    return {"protocol": "local_decider", "endpoint": "http://127.0.0.1:8080/v1/decisions",
            "model": "pplx-decider-flash-v1.2", "expected_model": "pplx-decider-flash-v1.2",
            "upstream": None, "max_input_bytes": 16000, "timeout_secs": 30}


class FakeBridge:
    artifact_sha256 = "b" * 64

    def __init__(self, status="identity_unavailable"):
        self.status = status
        self.calls = []
        self.changed = False

    def __call__(self, request):
        self.calls.append(copy.deepcopy(request))
        if request["op"] == "finished_meeting_export":
            value = {"schema_version": 1, "protocol_id": "production-finished-meeting-v1",
                     "fixture": request["fixture"], "configuration": request["configuration"],
                     "expected_actual": request.get("expected_actual"),
                     "qualification_request": {"task": "meeting_suggestions_v1", "language_policy": request["fixture"]["language"]},
                     "source_sha256": {"production": ("c" if self.changed else "a") * 64},
                     "production_admission": False, "windows": [{"index": 0,
                         "request":{"questions":[{"id":key,"kind":"choice","options":[{"id":"none"},{"id":"confirmed"}]} for key in ["action","decision","open_question","changed_commitment"]]},
                         "wire_body": '{"model":"fixture","state":"exact raw body","questions":{"action":{},"decision":{},"open_question":{},"changed_commitment":{}}}'}]}
            value["freeze_sha256"] = finished.digest(value)
            return value
        if request["op"] == "finished_meeting_replay":
            return {"status": self.status, "production_admission": False,
                    "complete": len(request["responses"]) == len(request["export"]["windows"]),
                    "windows": [{"window_index": r["window_index"], "status": self.status,
                                 "observed_identity": {"actual_model": "fixture"}, "candidates": []}
                                for r in request["responses"]]}
        raise AssertionError(request["op"])

    def call_before(self, request, deadline):
        return self(request)


class FinishedTests(unittest.TestCase):
    def test_freeze_uses_bridge_exports_and_keeps_language_families_together(self):
        bridge = FakeBridge()
        frozen = finished.freeze([fixture("en"), fixture("nl"), fixture("mixed")], configuration(), bridge)
        self.assertEqual(len(frozen["exports"]), 3)
        self.assertEqual({x["fixture"]["language"] for x in frozen["exports"]}, {"en", "nl", "mixed"})
        self.assertTrue(all(x["op"] == "finished_meeting_export" for x in bridge.calls))
        self.assertEqual(frozen["bridge_sha256"], bridge.artifact_sha256)
        with self.assertRaisesRegex(finished.FinishedError, "family_split_leakage"):
            finished.freeze([fixture("en"), fixture("nl", "heldout")], configuration(), bridge)

    def test_first_request_is_exported_bytes_and_missing_identity_stops_pilot(self):
        bridge = FakeBridge()
        frozen = finished.freeze([fixture("en"), fixture("nl")], configuration(), bridge)
        seen = []
        def transport(endpoint, body, timeout):
            seen.append((endpoint, body, timeout))
            return 200, '{"model":"fixture","answers":{}}'
        report = finished.run_local_pilot(frozen, bridge, transport=transport, execute=True)
        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0][1], frozen["exports"][0]["windows"][0]["wire_body"].encode())
        self.assertLessEqual(seen[0][2], 30)
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(report["reason"], "identity_unavailable")
        self.assertEqual(report["physical_requests"], 1)
        self.assertFalse(report["production_admission"])

    def test_changed_source_or_bridge_cannot_send_any_request(self):
        for changed in ("source", "binary", "freeze"):
            bridge = FakeBridge("ok")
            frozen = finished.freeze([fixture()], configuration(), bridge)
            if changed == "source": bridge.changed = True
            if changed == "binary": bridge.artifact_sha256 = "d" * 64
            if changed == "freeze": frozen["exports"][0]["windows"][0]["wire_body"] += " "
            with self.assertRaises(finished.FinishedError):
                finished.run_local_pilot(frozen, bridge, transport=lambda *_: self.fail("network called"), execute=True)

    def test_network_execution_is_explicit_loopback_only_and_development_only(self):
        bridge = FakeBridge()
        frozen = finished.freeze([fixture()], configuration(), bridge)
        with self.assertRaisesRegex(finished.FinishedError, "local_execution_not_enabled"):
            finished.run_local_pilot(frozen, bridge)
        for endpoint in ("https://hosted.example/v1/decisions", "http://localhost.evil/v1/decisions", "http://secret@127.0.0.1:8080/v1/decisions", "http://127.0.0.1:8080/v1/decisions?key=x"):
            config = configuration(); config["endpoint"] = endpoint
            bad = finished.freeze([fixture()], config, bridge)
            with self.assertRaisesRegex(finished.FinishedError, "loopback"):
                finished.run_local_pilot(bad, bridge, execute=True)
        heldout = finished.freeze([fixture(split="heldout")], configuration(), bridge)
        with self.assertRaisesRegex(finished.FinishedError, "development_only"):
            finished.run_local_pilot(heldout, bridge, execute=True)

    def test_one_absolute_job_deadline_covers_sequential_windows(self):
        clock = [0.0]
        bridge = FakeBridge("ok")
        # The fake exporter must reproduce the same three-window contract.
        original = bridge.__class__.__call__
        class ThreeBridge(FakeBridge):
            def __call__(self, request):
                value = original(self, request)
                if request["op"] == "finished_meeting_export":
                    value["windows"] *= 3
                    value["freeze_sha256"] = finished.digest({k:v for k,v in value.items() if k != "freeze_sha256"})
                return value
        bridge = ThreeBridge("ok")
        frozen = finished.freeze([fixture()], configuration(), bridge)
        timeouts = []
        def transport(endpoint, body, timeout):
            timeouts.append(timeout); clock[0] += 16
            return 200, '{}'
        report = finished.run_local_pilot(frozen, bridge, transport=transport, execute=True, clock=lambda:clock[0])
        self.assertEqual(len(timeouts), 2)
        self.assertLessEqual(timeouts[1], 14)
        self.assertEqual(report["reason"], "job_deadline_exceeded")

    def test_caps_refuse_extension_and_provider_failures_never_retry(self):
        bridge = FakeBridge("ok")
        frozen = finished.freeze([fixture()], configuration(), bridge)
        for limit in (0, 1801):
            with self.assertRaisesRegex(finished.FinishedError, "evaluation_budget"):
                finished.run_local_pilot(frozen, bridge, execute=True, max_seconds=limit)
        calls=[]
        report=finished.run_local_pilot(frozen, bridge, execute=True,
            transport=lambda *args:(calls.append(args) or (503, 'private provider body')))
        self.assertEqual(len(calls),1)
        self.assertEqual(report["reason"],"provider_failed")
        self.assertEqual(report["requests"][0]["http_status"],503)
        self.assertNotIn('private provider body',str({k:v for k,v in report["requests"][0].items() if k!='raw_response'}))

    def test_validation_and_preparation_count_against_their_absolute_budgets(self):
        clock = [0.0]
        class SlowBridge(FakeBridge):
            slow = False
            def __call__(self, request):
                if self.slow and request["op"] == "finished_meeting_export": clock[0] += 950
                return super().__call__(request)
        bridge=SlowBridge("ok")
        frozen=finished.freeze([fixture()],configuration(),bridge)
        bridge.slow=True
        report=finished.run_local_pilot(frozen,bridge,execute=True,clock=lambda:clock[0],
            transport=lambda *_:self.fail('late validation sent a request'))
        self.assertEqual(report["reason"],"evaluation_budget_exceeded")
        self.assertEqual(report["physical_requests"],0)
        self.assertGreaterEqual(report["elapsed_ms"],1800000)

    def test_final_raw_replay_cannot_turn_a_late_response_into_compatibility(self):
        clock=[0.0]
        class SlowReplay(FakeBridge):
            def __call__(self,request):
                if request["op"]=="finished_meeting_replay":clock[0]+=31
                return super().__call__(request)
        bridge=SlowReplay("ok")
        frozen=finished.freeze([fixture()],configuration(),bridge)
        report=finished.run_local_pilot(frozen,bridge,execute=True,clock=lambda:clock[0],
            transport=lambda *_:(200,'{}'))
        self.assertEqual(report["status"],"blocked")
        self.assertEqual(report["reason"],"job_deadline_exceeded")
        self.assertEqual(report["physical_requests"],1)
        self.assertIsNone(report["fixtures"][0]["replay"])
        self.assertEqual(report["requests"][0]["raw_response"],'{}')

    def test_expected_labels_are_frozen_and_missing_replies_reduce_raw_coverage(self):
        bridge=FakeBridge("invalid_response")
        labels={"cedar_en":[{key:"none" for key in ["action","decision","open_question","changed_commitment"]}]}
        frozen=finished.freeze([fixture()],configuration(),bridge,expected_answers=labels)
        report=finished.score_responses(frozen,[[]],bridge)
        self.assertEqual(report["by_language"]["en"]["failed_or_missing_windows"],1)
        self.assertEqual(report["by_language"]["en"]["raw_component_metrics"]["count"],4)
        self.assertEqual(report["by_language"]["en"]["raw_component_metrics"]["coverage"],0)
        self.assertIsNone(report["by_language"]["en"]["complete_item_metrics"])
        self.assertFalse(report["qualification"])
        with self.assertRaisesRegex(finished.FinishedError,"expected_answers_not_frozen"):
            finished.score_responses(finished.freeze([fixture()],configuration(),bridge),[[]],bridge)
        invalid=copy.deepcopy(labels);invalid["cedar_en"][0]["action"]="invented_label"
        with self.assertRaisesRegex(finished.FinishedError,"expected_answer_contract_changed"):
            finished.freeze([fixture()],configuration(),bridge,expected_answers=invalid)

    def test_slow_drip_http_is_terminated_at_absolute_deadline_with_partial_body(self):
        stop=threading.Event()
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args):pass
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length',0)))
                self.send_response(200);self.send_header('Content-Length','1000');self.end_headers()
                while not stop.is_set():
                    try:self.wfile.write(b'x');self.wfile.flush()
                    except (BrokenPipeError,ConnectionResetError):break
                    stop.wait(.03)
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=lambda:server.serve_forever(poll_interval=.01),daemon=True);thread.start()
        timer=threading.Timer(1.5,stop.set);timer.start()
        started=time.monotonic()
        try:
            with self.assertRaises(finished.FinishedError) as caught:
                finished.local_transport(f'http://127.0.0.1:{server.server_port}/v1/decisions',b'{}',.5)
            self.assertLess(time.monotonic()-started,1.2)
            self.assertEqual(str(caught.exception),'transport_deadline_exceeded')
            self.assertEqual(caught.exception.http_status,200)
            self.assertGreater(len(caught.exception.raw_response),0)
        finally:
            stop.set();timer.cancel();server.shutdown();server.server_close();thread.join(timeout=1)

    def test_real_bridge_subprocess_uses_only_remaining_deadline(self):
        with tempfile.TemporaryDirectory(prefix='pennel-finished-deadline-') as directory:
            script=Path(directory)/'bridge'
            script.write_text('#!'+sys.executable+'\nimport time\ntime.sleep(2)\nprint("{}")\n')
            script.chmod(0o700)
            bridge=finished.CoreBridge(script)
            started=time.monotonic()
            with self.assertRaisesRegex(finished.FinishedError,'core_bridge_timeout'):
                bridge.call_before({'op':'fixture'},started+.15)
            self.assertLess(time.monotonic()-started,.65)

    def test_next_fixture_preparation_failure_preserves_prior_physical_ledger(self):
        for kind in ('changed','failure'):
            class MutatingBridge(FakeBridge):
                after_request=False
                def __call__(self,request):
                    if self.after_request and request['op']=='finished_meeting_export' and request['fixture']['language']=='nl':
                        if kind=='failure':raise finished.FinishedError('core_bridge_failed')
                        self.changed=True
                    return super().__call__(request)
            bridge=MutatingBridge('ok')
            frozen=finished.freeze([fixture('en'),fixture('nl')],configuration(),bridge)
            def transport(*args):
                bridge.after_request=True
                return 200,'{}'
            report=finished.run_local_pilot(frozen,bridge,execute=True,transport=transport)
            self.assertEqual(report['status'],'blocked')
            self.assertEqual(report['physical_requests'],1)
            self.assertEqual(report['requests'][0]['raw_response'],'{}')
            self.assertEqual(report['reason'],'core_bridge_failed' if kind=='failure' else 'production_export_changed')

    def test_pilot_passes_one_absolute_deadline_to_repreparation_and_near_deadline_replay(self):
        clock=[0.0]
        class ObservedBridge(FakeBridge):
            deadlines=[]
            def call_before(self,request,deadline):
                self.deadlines.append((request['op'],deadline))
                return self(request)
        bridge=ObservedBridge('ok');frozen=finished.freeze([fixture()],configuration(),bridge)
        def transport(*args):
            clock[0]=29.9
            return 200,'{}'
        report=finished.run_local_pilot(frozen,bridge,execute=True,transport=transport,clock=lambda:clock[0])
        self.assertEqual(report['status'],'compatible')
        self.assertEqual(bridge.deadlines[-2:],[('finished_meeting_export',30),('finished_meeting_replay',30)])
        self.assertTrue(all(deadline==1800 for _,deadline in bridge.deadlines[:-2]))


if __name__ == '__main__':
    unittest.main()
