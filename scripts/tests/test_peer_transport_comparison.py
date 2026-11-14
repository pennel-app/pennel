"""Exercise the comparison controller at its process and evidence boundaries."""

import hashlib
import http.client
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import textwrap
import threading
from types import SimpleNamespace
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "peer-transport-comparison.py"


def load_module():
    spec = importlib.util.spec_from_file_location("peer_transport_comparison", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.module = load_module()
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = self.root / "fixture.bin"
        self.fixture.write_bytes(b"original fixture")
        self.digest = hashlib.sha256(self.fixture.read_bytes()).hexdigest()

    def report(self, **updates):
        result = {"complete": True, "bytes_sent": 0, "bytes_received": self.fixture.stat().st_size,
                  "blocks_transferred": 1, "total_bytes": self.fixture.stat().st_size,
                  "sha256": self.digest}
        result.update(updates)
        return result

    def test_completion_requires_both_an_honest_receipt_and_the_actual_destination(self):
        destination = self.root / "target.bin"
        with self.assertRaises(self.module.QualificationError):
            self.module.validate_transfer(self.report(), self.fixture, destination)
        destination.write_bytes(b"different bytes")
        with self.assertRaises(self.module.QualificationError):
            self.module.validate_transfer(self.report(), self.fixture, destination)
        destination.write_bytes(self.fixture.read_bytes())
        self.module.validate_transfer(self.report(), self.fixture, destination)

    def test_malformed_counts_or_false_zero_repeat_never_become_success(self):
        destination = self.root / "target.bin"
        destination.write_bytes(self.fixture.read_bytes())
        for field, value in [("bytes_received", True), ("total_bytes", -1), ("sha256", "0" * 64),
                             ("complete", 1), ("blocks_transferred", 1.5)]:
            with self.subTest(field=field), self.assertRaises(self.module.QualificationError):
                self.module.validate_transfer(self.report(**{field: value}), self.fixture, destination)
        with self.assertRaises(self.module.QualificationError):
            self.module.validate_transfer(self.report(), self.fixture, destination, expected_bytes=0)

    def test_private_artifacts_refuse_overwrite_and_never_print_capabilities(self):
        output = self.root / "private.json"
        self.module.write_private_json(output, {"ticket": "secret-capability"})
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
        with self.assertRaises(FileExistsError):
            self.module.write_private_json(output, {"ticket": "other"})
        self.assertEqual(json.loads(output.read_text())["ticket"], "secret-capability")
        public = self.module.public_summary({"schema_version": 1, "complete": False,
            "candidate": "quinn", "route": "forced_relay", "private_config": str(output),
            "ticket": "secret-capability", "peer_id": "f" * 64,
            "results": [{"receipt": {"ticket": "nested-secret"}, "private_key": "key-secret"}]})
        self.assertNotIn("secret-capability", json.dumps(public))
        self.assertNotIn(str(output), json.dumps(public))
        self.assertNotIn("peer_id", public)
        self.assertNotIn("nested-secret", json.dumps(public))
        self.assertNotIn("key-secret", json.dumps(public))

    def test_fixture_is_the_retained_original_size_and_hash_is_deterministic(self):
        first = self.module.make_fixtures(self.root / "a")
        second = self.module.make_fixtures(self.root / "b")
        self.assertEqual(first["binary"].stat().st_size, 8_388_633)
        self.assertEqual(self.module.sha256(first["binary"]), self.module.sha256(second["binary"]))
        self.assertEqual(first["empty"].stat().st_size, 0)
        self.assertEqual(first["one_byte"].stat().st_size, 1)

    def test_device_fixture_size_is_explicit_and_bounded(self):
        fixtures = self.module.make_fixtures(self.root / "larger", mib=2)
        self.assertEqual(fixtures["binary"].stat().st_size, 2 * 1024 * 1024 + len(self.module.TAIL))
        for size in (0, 257, True, 1.5):
            with self.subTest(size=size), self.assertRaises(self.module.QualificationError):
                self.module.make_fixtures(self.root / "invalid", mib=size)

    def capture_payload(self, phase="active"):
        return {"candidate": "quinn", "route": "forced_relay", "phase": phase,
                "transport_ongoing": phase != "baseline",
                "measurements": [{"start_ms": 2.5, "stop_ms": 1.5, "samples": 4000,
                                  "callback_after_stop": False} for _ in range(10)]}

    def capture_controller(self):
        controller = self.module.DeviceController.__new__(self.module.DeviceController)
        controller.args = SimpleNamespace(candidate="quinn", route="forced_relay")
        controller.root = self.root
        controller.lock = threading.RLock()
        controller.capture_results = {}
        controller.result = None
        return controller

    def test_capture_receipt_is_independent_of_returned_transfer_and_never_qualifies_candidate(self):
        controller = self.capture_controller()
        for phase in ("baseline", "active", "stalled"):
            receipt = controller.save_capture_result(self.capture_payload(phase))
            self.assertTrue(receipt["saved"])
            self.assertFalse(receipt["qualification_complete"])
        saved = json.loads((self.root / "private-capture-result.json").read_text())
        self.assertTrue(saved["capture_phases_complete"])
        self.assertFalse(saved["qualification_complete"])
        self.assertEqual(set(saved["phases"]), {"baseline", "active", "stalled"})
        self.assertIsNone(controller.result)
        self.assertFalse(hasattr(controller, "return_verified"))

    def test_capture_rejects_wrong_scope_bad_counts_and_post_stop_callbacks(self):
        for mutate in (
            lambda value: value.update(candidate="native"),
            lambda value: value.update(route="direct"),
            lambda value: value.update(phase="unknown"),
            lambda value: value.update(transport_ongoing=False),
            lambda value: value["measurements"].pop(),
            lambda value: value["measurements"][0].update(start_ms=float("inf")),
            lambda value: value["measurements"][0].update(stop_ms=-1),
            lambda value: value["measurements"][0].update(samples=True),
            lambda value: value["measurements"][0].update(samples=0),
            lambda value: value["measurements"][0].update(callback_after_stop=True),
        ):
            controller = self.capture_controller()
            payload = self.capture_payload()
            mutate(payload)
            with self.subTest(payload=payload), self.assertRaises(self.module.QualificationError):
                controller.save_capture_result(payload)
        self.assertFalse((self.root / "private-capture-result.json").exists())

    def test_capture_phase_is_idempotent_and_conflicting_receipt_preserves_original(self):
        controller = self.capture_controller()
        payload = self.capture_payload()
        first = controller.save_capture_result(payload)
        self.assertEqual(controller.save_capture_result(payload), first)
        changed = self.capture_payload()
        changed["measurements"][0]["samples"] += 1
        with self.assertRaises(self.module.QualificationError):
            controller.save_capture_result(changed)
        saved = json.loads((self.root / "capture-active-result.json").read_text())
        self.assertEqual(saved["result"], payload)

    def test_capture_aggregate_write_failure_is_retryable_without_false_acknowledgement(self):
        controller = self.capture_controller()
        payload = self.capture_payload()
        writer = self.module.write_private_json
        def fail_aggregate(path, value, **kwargs):
            if Path(path).name == "private-capture-result.json":
                raise OSError("synthetic disk-full")
            return writer(path, value, **kwargs)
        with mock.patch.object(self.module, "write_private_json", side_effect=fail_aggregate):
            with self.assertRaises(OSError):
                controller.save_capture_result(payload)
        self.assertEqual(controller.capture_results, {})
        self.assertFalse((self.root / "private-capture-result.json").exists())
        self.assertTrue(controller.save_capture_result(payload)["saved"])
        self.assertEqual(controller.capture_results["active"], payload)

    def test_verified_staging_counts_only_exact_original_blocks(self):
        source = self.root / "blocks.bin"
        source.write_bytes(b"abcdefghij")
        stage = self.root / "stage"
        stage.mkdir()
        (stage / "block-00000000.part").write_bytes(b"abcd")
        (stage / "block-00000001.part").write_bytes(b"corrupt")
        (stage / "block-00000002.part").write_bytes(b"ij")
        self.assertEqual(self.module.verified_staging_bytes(stage, source, 4), 6)

    def test_relay_pause_requires_an_owned_prepared_session(self):
        controller = self.capture_controller()
        controller.peer = "a" * 64
        controller.fixture = {"candidate": "quinn"}
        controller.relay = None
        with self.assertRaises(self.module.QualificationError):
            controller.set_relay_paused(True)
        class Relay:
            calls = []
            def set_paused(self, value):
                self.calls.append(value)
                return {"paused": value}
        controller.relay = Relay()
        controller.fixture = None
        with self.assertRaises(self.module.QualificationError):
            controller.set_relay_paused(True)
        self.assertEqual(controller.relay.calls, [])
        controller.fixture = {"candidate": "quinn"}
        self.assertEqual(controller.set_relay_paused(True), {"paused": True})
        self.assertEqual(controller.set_relay_paused(False), {"paused": False})
        self.assertEqual(controller.relay.calls, [True, False])

    def test_repository_evidence_must_be_ignored_before_any_private_data_is_created(self):
        output = SCRIPT.parents[1] / "unignored-peer-test-evidence"
        self.assertFalse(output.exists())
        with self.assertRaises(self.module.QualificationError):
            self.module.new_directory(output)
        self.assertFalse(output.exists())

    def test_controller_rejects_boolean_counts_wrong_route_and_nonfinite_time(self):
        state = self.module.DeviceEvidence(self.fixture, "quinn", "forced_relay", 4)
        valid = {"candidate": "quinn", "route": "forced_relay", "complete": True,
                 "first_bytes": 4, "resumed_bytes": self.fixture.stat().st_size - 4,
                 "unchanged_bytes": 0, "returned_bytes": self.fixture.stat().st_size,
                 "sha256": self.digest, "elapsed_seconds": 1.0}
        for update in ({"first_bytes": True}, {"route": "direct"}, {"elapsed_seconds": float("nan")},
                       {"complete": 1}, {"returned_bytes": 0}, {"extra": "unexpected"}):
            with self.subTest(update=update), self.assertRaises(self.module.QualificationError):
                state.validate({**valid, **update})
        state.validate(valid)

    def device_payload(self):
        return {"candidate": "quinn", "route": "forced_relay", "complete": True,
                "first_bytes": 4, "resumed_bytes": self.fixture.stat().st_size - 4,
                "unchanged_bytes": 0, "returned_bytes": self.fixture.stat().st_size,
                "sha256": self.digest, "elapsed_seconds": 1.0}

    def app_metrics(self):
        return {"scope": "owned_app_process", "method": "mach_task_basic_info+getrusage",
                "idle_rss_bytes": 1000, "peak_rss_bytes": 1500, "cpu_seconds": 0.25,
                "sample_count": 10, "elapsed_seconds": 1.5}

    def test_owned_app_metrics_are_optional_but_strictly_validated_when_present(self):
        state = self.module.DeviceEvidence(self.fixture, "quinn", "forced_relay", 4)
        state.validate(self.device_payload())
        state.validate({**self.device_payload(), "metrics": self.app_metrics()})
        for update in ({"scope": "all_processes"}, {"method": "arbitrary"}, {"idle_rss_bytes": 0},
                       {"peak_rss_bytes": 999}, {"cpu_seconds": float("nan")}, {"cpu_seconds": -1},
                       {"sample_count": True}, {"sample_count": 0}, {"elapsed_seconds": 0},
                       {"elapsed_seconds": float("inf")}, {"token": "private-secret"}):
            with self.subTest(update=update), self.assertRaises(self.module.QualificationError):
                state.validate({**self.device_payload(), "metrics": {**self.app_metrics(), **update}})

    def test_device_metrics_persist_privately_and_have_a_sanitized_public_projection(self):
        controller = self.module.DeviceController.__new__(self.module.DeviceController)
        controller.args = SimpleNamespace(candidate="quinn", route="forced_relay", block_size=4)
        controller.source = self.fixture
        controller.root = self.root
        controller.lock = threading.RLock()
        controller.result = None
        controller.return_verified = True
        (self.root / "returned.bin").write_bytes(self.fixture.read_bytes())
        payload = {**self.device_payload(), "metrics": self.app_metrics()}
        self.assertTrue(controller.save_result(payload)["saved"])
        private = json.loads((self.root / "private-device-result.json").read_text())
        compatibility = json.loads((self.root / "device-result.json").read_text())
        public = json.loads((self.root / "public-device-result.json").read_text())
        self.assertEqual(private, payload)
        self.assertEqual(compatibility, payload)
        self.assertEqual(public["metrics"], self.app_metrics())
        self.assertFalse(public["qualification_complete"])
        self.assertEqual(self.module.public_summary({"metrics": {**self.app_metrics(), "token": "private-secret"}}), {"metrics": {}})

    def test_metrics_numbers_cannot_use_booleans_or_nonfinite_values(self):
        metrics = self.app_metrics()
        for field in ("idle_rss_bytes", "peak_rss_bytes", "cpu_seconds", "sample_count", "elapsed_seconds"):
            with self.subTest(field=field), self.assertRaises(self.module.QualificationError):
                self.module.validate_device_metrics({**metrics, field: True})


class ProcessTests(unittest.TestCase):
    def setUp(self):
        self.module = load_module()
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def binary(self, body):
        path = self.root / ("probe-" + str(len(list(self.root.iterdir()))))
        path.write_text("#!" + sys.executable + "\n" + textwrap.dedent(body))
        path.chmod(0o700)
        return path

    def test_failed_or_stalled_startup_reaps_the_owned_process(self):
        for body in ("import sys; print('{\"ok\":false}', flush=True); sys.stdin.read()",
                     "import sys,time; sys.stdout.write('{'); sys.stdout.flush(); time.sleep(30)"):
            with self.subTest(body=body):
                child = self.module.JsonProcess([str(self.binary(body))], self.root / (str(len(list(self.root.iterdir()))) + ".log"))
                try:
                    with self.assertRaises(self.module.QualificationError):
                        child.read(timeout=0.15, require_ok=True)
                finally:
                    child.close(graceful=False)
                self.assertIsNotNone(child.process.poll())

    def test_partial_lines_are_bounded_by_the_deadline(self):
        binary = self.binary("import sys,time; sys.stdout.write('{'); sys.stdout.flush(); time.sleep(30)")
        child = self.module.JsonProcess([str(binary)], self.root / "stderr.log")
        try:
            with self.assertRaises(self.module.QualificationError):
                child.read(timeout=0.1)
        finally:
            child.close(graceful=False)

    def test_probe_identity_and_route_are_checked_at_startup(self):
        binary = self.binary('''
            import json,sys
            print(json.dumps({"ok":True,"event":"ready","result":{"identity_id":"a"*64,"candidate":"native","route":"direct","address_json":"{}"}}),flush=True)
            for line in sys.stdin:
                print('{"ok":true,"result":null}',flush=True)
                if json.loads(line).get('command')=='close': break
        ''')
        probe = self.module.Probe(binary, "quinn", self.root / "identity.json", "peer-probe-test", self.root / "stderr.log", start=False)
        try:
            with self.assertRaises(self.module.QualificationError):
                probe.ready()
        finally:
            probe.close()
        self.assertIsNotNone(probe.process.poll())

    def test_only_allowlisted_probe_errors_are_exposed_to_evidence(self):
        binary = self.binary('''
            import sys
            for line in sys.stdin:
                print('{"ok":false,"error":"private-secret-capability"}',flush=True)
        ''')
        child = self.module.JsonProcess([str(binary)], self.root / "stderr.log")
        try:
            with self.assertRaises(self.module.ProbeOperationError) as error:
                child.call("receive")
            self.assertEqual(error.exception.code, "unknown_probe_error")
            self.assertNotIn("private-secret", str(error.exception))
        finally:
            child.close(graceful=False)

    def test_relay_pause_requires_the_exact_admin_acknowledgement(self):
        binary = self.binary('''
            import json,sys
            for line in sys.stdin:
                value=json.loads(line)
                if value['command']=='shutdown': break
                print('{"event":"paused","result":{"paused":"true"}}',flush=True)
        ''')
        relay = self.module.Relay.__new__(self.module.Relay)
        self.module.JsonProcess.__init__(relay, [str(binary)], self.root / "stderr.log")
        try:
            with self.assertRaises(self.module.QualificationError):
                relay.set_paused(True)
        finally:
            relay.close()

    def test_host_controller_exercises_restarts_and_retains_unmet_device_gates(self):
        binary = self.binary('''
            import hashlib,json,pathlib,sys
            args=sys.argv[1:]
            identity_file=pathlib.Path(args[args.index('--identity-file')+1])
            identity=hashlib.sha256(str(identity_file).encode()).hexdigest()
            identity_file.write_text(json.dumps({'account':identity}))
            identity_file.chmod(0o600)
            offer_file=identity_file.with_suffix('.offer')
            address={'identity_id':identity,'host':'127.0.0.1','port':1,'route':'direct','offer':str(offer_file)}
            approved=set()
            print(json.dumps({'ok':True,'event':'ready','result':{'identity_id':identity,'address_json':json.dumps(address),'candidate':'quinn','route':'direct'}}),flush=True)
            for line in sys.stdin:
                command=json.loads(line); name=command['command']; result=True
                if name=='address': result=json.dumps(address)
                elif name=='approve': approved.add(command['peer_id'])
                elif name=='revoke': approved.discard(command['peer_id'])
                elif name=='offer':
                    source=pathlib.Path(command['path']); data=source.read_bytes()
                    result={'sha256':hashlib.sha256(data).hexdigest(),'size':len(data)}
                    offer_file.write_text(json.dumps({'path':str(source),'block_size':command['block_size']}))
                elif name=='receive':
                    remote=json.loads(command['address_json'])
                    if remote['identity_id'] not in approved:
                        print('{"ok":false,"error":"unauthorized"}',flush=True); continue
                    offer=json.loads(pathlib.Path(remote['offer']).read_text()); data=pathlib.Path(offer['path']).read_bytes()
                    stage=pathlib.Path(command['staging']); stage.mkdir(exist_ok=True)
                    target=pathlib.Path(command['target']); size=offer['block_size']; count=sent=0
                    if not target.exists():
                        for index,offset in enumerate(range(0,len(data),size)):
                            part=stage/f'block-{index:08}.part'; block=data[offset:offset+size]
                            if part.exists() and part.read_bytes()==block: continue
                            if command.get('max_blocks') is not None and count>=command['max_blocks']: break
                            part.write_bytes(block); sent+=len(block); count+=1
                        complete=all((stage/f'block-{index:08}.part').exists() and (stage/f'block-{index:08}.part').read_bytes()==data[offset:offset+size] for index,offset in enumerate(range(0,len(data),size)))
                        if complete: target.write_bytes(data)
                    else: complete=True
                    result={'complete':complete,'bytes_sent':0,'bytes_received':sent,'blocks_transferred':count,'total_bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
                print(json.dumps({'ok':True,'result':result}),flush=True)
                if name=='close': break
        ''')
        args = SimpleNamespace(output=self.root / "run", candidates=["quinn"], routes=["direct"],
            quinn_binary=binary, native_binary=None, relay_binary=None, run_id="controller-test",
            block_sizes=[128 * 1024], repeats=1)
        result = self.module.run_host(args)
        self.assertTrue(result["host_cases_complete"])
        self.assertFalse(result["qualification_complete"])
        self.assertIn("physical_device_acceptance", result["gaps"])
        cases = result["results"][0]["results"]
        self.assertEqual(len(cases), 14)
        self.assertEqual({row["case"] for row in cases}, {"transfer", "process_restart", "corrupt_durable_block", "revoked_peer"})
        self.assertTrue(all(row["complete"] for row in cases))
        for case in ("process_restart", "corrupt_durable_block", "revoked_peer"):
            self.assertEqual({row["direction"] for row in cases if row["case"] == case}, {"a_to_b", "b_to_a"})
        public = json.loads((args.output / "public-summary.json").read_text())
        self.assertFalse(public["qualification_complete"])
        self.assertNotIn("identity-a.json", json.dumps(public))


class HTTPTests(unittest.TestCase):
    def setUp(self):
        self.module = load_module()
        class Controller:
            token = "private-capability"
            result = None
            calls = []
            def prepare(self, peer):
                self.calls.append(peer)
                return {"candidate": "quinn"}
            def save_capture_result(self, value):
                self.calls.append(("capture", value))
                return {"saved": True, "qualification_complete": False}
            def set_relay_paused(self, value):
                self.calls.append(("pause", value))
                return {"paused": value}
        self.controller = Controller()
        self.server = self.module.BoundedHTTPServer(("127.0.0.1", 0), self.module.Handler)
        self.server.controller = self.controller
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.cleanup)

    def cleanup(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def request(self, path, body, authorization=None, extra_headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=2)
        self.addCleanup(connection.close)
        headers = {"Content-Type": "application/json"}
        if authorization is not None:
            headers["Authorization"] = authorization
        headers.update(extra_headers or {})
        connection.request("POST", path, body, headers)
        response = connection.getresponse()
        result = json.loads(response.read())
        return response, result

    def test_unauthorized_requests_cannot_prepare_or_reveal_state(self):
        response, result = self.request("/prepare", '{"peer_id":"attacker"}', "Bearer wrong")
        self.assertEqual(response.status, 403)
        self.assertEqual(result, {"error": "unauthorized"})
        self.assertEqual(self.controller.calls, [])
        self.assertEqual(response.getheader("Cache-Control"), "no-store")

    def test_oversized_or_chunked_control_requests_are_rejected_before_state_mutation(self):
        for body, headers in (("x" * (self.module.MAX_CONTROL + 1), {}),
                              ('{"peer_id":"attacker"}', {"Transfer-Encoding": "chunked"})):
            response, result = self.request("/prepare", body, "Bearer private-capability", headers)
            self.assertEqual(response.status, 413)
            self.assertEqual(self.controller.calls, [])

    def test_authenticated_control_has_no_path_or_file_read_endpoint(self):
        response, result = self.request("/read", '{"path":"/private-file"}', "Bearer private-capability")
        self.assertEqual(response.status, 404)
        self.assertEqual(self.controller.calls, [])

    def test_capture_endpoint_dispatches_independently_from_transfer_completion(self):
        payload = {"candidate": "quinn", "route": "forced_relay", "phase": "stalled"}
        response, result = self.request("/capture-result", json.dumps(payload), "Bearer private-capability")
        self.assertEqual(response.status, 200)
        self.assertEqual(self.controller.calls, [("capture", payload)])
        self.assertTrue(result["saved"])
        self.assertFalse(result["qualification_complete"])
        self.assertIsNone(self.controller.result)

    def test_pause_and_resume_are_authenticated_fixed_empty_body_controls(self):
        response, result = self.request("/pause", "{}", "Bearer wrong")
        self.assertEqual(response.status, 403)
        self.assertEqual(self.controller.calls, [])
        response, result = self.request("/pause", '{"command":"shutdown"}', "Bearer private-capability")
        self.assertEqual(response.status, 400)
        self.assertEqual(self.controller.calls, [])
        for path, paused in (("/pause", True), ("/resume", False)):
            response, result = self.request(path, "{}", "Bearer private-capability")
            self.assertEqual(response.status, 200)
            self.assertEqual(result, {"paused": paused})
        self.assertEqual(self.controller.calls, [("pause", True), ("pause", False)])


if __name__ == "__main__":
    unittest.main()
