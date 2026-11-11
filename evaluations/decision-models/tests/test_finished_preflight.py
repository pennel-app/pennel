from decimal import Decimal
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import finished
import finished_preflight
from runners import SpendLedger
from test_finished import FakeBridge, configuration, fixture


class HostedBridge(FakeBridge):
    def __call__(self, request):
        result = super().__call__(request)
        if request['op'] == 'finished_meeting_export':
            payload = json.loads(result['windows'][0]['wire_body'])
            payload['provider'] = {'only': ['perplexity'], 'allow_fallbacks': False, 'data_collection': 'deny', 'zdr': True}
            result['windows'][0]['wire_body'] = json.dumps(payload)
            result['freeze_sha256'] = finished.digest({key: value for key, value in result.items() if key != 'freeze_sha256'})
        return result


def hosted_config():
    return {**configuration(), 'protocol': 'open_router_decisions',
            'endpoint': finished_preflight.HOSTED_ENDPOINT, 'model': finished_preflight.HOSTED_MODEL,
            'expected_model': 'perplexity/pplx-decider-v1.2-20261016', 'upstream': 'perplexity'}


class PreflightTests(unittest.TestCase):
    def test_missing_identity_stops_local_before_later_windows_and_preserves_raw(self):
        bridge = FakeBridge()
        frozen = finished.freeze([fixture('nl'), {**fixture('nl'), 'id': 'second_nl'}], configuration(), bridge)
        seen = []
        report = finished_preflight.run(frozen, bridge, execute=True,
            transport=lambda endpoint, body, timeout: (seen.append(body) or (200, b'{"model":"fixture"}')))
        self.assertEqual(seen, [frozen['exports'][0]['windows'][0]['wire_body'].encode()])
        self.assertEqual(report['reason'], 'identity_unavailable')
        self.assertEqual(report['requests'][0]['raw_response'], '{"model":"fixture"}')
        self.assertFalse(report['qualification'])

    def test_hosted_uses_production_privacy_route_and_reconciles_exact_cost(self):
        bridge, ledger = HostedBridge(), SpendLedger('2.00')
        frozen = finished.freeze([fixture('nl')], hosted_config(), bridge)
        observed = []
        def transport(endpoint, body, timeout):
            observed.append((endpoint, body))
            return 200, b'{"usage":{"cost":0.00001416}}'
        report = finished_preflight.run(frozen, bridge, execute=True, credential='test-only', ledger=ledger, transport=transport)
        self.assertEqual(observed[0][1], frozen['exports'][0]['windows'][0]['wire_body'].encode())
        self.assertEqual(ledger.snapshot()['known_billed_usd'], '0.00001416')
        self.assertEqual(ledger.snapshot()['reserved_usd'], '0')
        self.assertEqual(report['reason'], 'identity_unavailable')
        self.assertNotIn('test-only', json.dumps(report))

    def test_execution_scope_route_drift_and_budget_are_rejected_before_transport(self):
        bridge = HostedBridge()
        frozen = finished.freeze([fixture('nl')], hosted_config(), bridge)
        def transport(*args): self.fail('a rejected setup sent a request')
        with self.assertRaisesRegex(ValueError, 'execution_not_enabled'):
            finished_preflight.run(frozen, bridge, transport=transport)
        for field, value in [('upstream', 'other'), ('expected_model', 'latest'), ('endpoint', 'https://example.invalid')]:
            config = hosted_config(); config[field] = value
            changed = finished.freeze([fixture('nl')], config, bridge)
            with self.assertRaisesRegex(ValueError, 'pinned_hosted'):
                finished_preflight.run(changed, bridge, execute=True, credential='test-only', ledger=SpendLedger(), transport=transport)
        ledger = SpendLedger()
        ledger.spent = Decimal('1.95')
        report = finished_preflight.run(frozen, bridge, execute=True, credential='test-only', ledger=ledger, transport=transport)
        self.assertEqual(report['reason'], 'unresolved_or_exhausted_hosted_budget')
        self.assertEqual(report['physical_requests'], 0)
        wrong_split = finished.freeze([fixture('nl', 'heldout')], hosted_config(), bridge)
        with self.assertRaisesRegex(ValueError, 'development_only'):
            finished_preflight.run(wrong_split, bridge, execute=True, credential='test-only', ledger=SpendLedger(), transport=transport)

    def test_provider_failure_has_no_retry_and_unknown_charge_stays_reserved(self):
        bridge, ledger, seen = HostedBridge(), SpendLedger(), []
        frozen = finished.freeze([fixture('nl')], hosted_config(), bridge)
        report = finished_preflight.run(frozen, bridge, execute=True, credential='test-only', ledger=ledger,
            transport=lambda *args: (seen.append(1) or (503, b'{"error":"unavailable"}')))
        self.assertEqual(seen, [1])
        self.assertEqual(report['reason'], 'provider_failed')
        self.assertTrue(ledger.unresolved)
        self.assertEqual(ledger.snapshot()['reserved_usd'], '0.10')

    def test_malformed_billed_cost_cannot_drop_the_consumed_reservation(self):
        for body in [b'{"usage":{"cost":"unreported"}}', b'{"usage":null}', b'{"usage":[]}']:
            bridge, ledger = HostedBridge(), SpendLedger()
            frozen = finished.freeze([fixture('nl')], hosted_config(), bridge)
            report = finished_preflight.run(frozen, bridge, execute=True, credential='test-only', ledger=ledger,
                transport=lambda *args: (200, body))
            self.assertEqual(report['physical_requests'], 1)
            self.assertTrue(ledger.unresolved)
            self.assertEqual(ledger.snapshot()['reserved_usd'], '0.10')

    def test_deadline_preserves_consumed_request_and_cannot_report_compatibility(self):
        clock, bridge = [0], FakeBridge('ok')
        frozen = finished.freeze([fixture('nl')], configuration(), bridge)
        def transport(*args):
            clock[0] = 31
            return 200, b'{}'
        report = finished_preflight.run(frozen, bridge, execute=True, transport=transport, clock=lambda: clock[0])
        self.assertEqual(report['reason'], 'job_deadline_exceeded')
        self.assertEqual(report['physical_requests'], 1)
        self.assertEqual(report['requests'][0]['raw_response'], '{}')

    def test_bridge_timeout_and_failure_preserve_consumed_response_and_settled_charge(self):
        for late in (True, False):
            clock = [0]
            class Broken(HostedBridge):
                def __call__(self, request):
                    if request['op'] == 'finished_meeting_replay':
                        if late: clock[0] = 31
                        raise finished.FinishedError('core_bridge_failed')
                    return super().__call__(request)
            bridge, ledger = Broken(), SpendLedger()
            frozen = finished.freeze([fixture('nl')], hosted_config(), bridge)
            report = finished_preflight.run(frozen, bridge, execute=True, credential='test-only', ledger=ledger,
                transport=lambda *args: (200, b'{"usage":{"cost":0.00001}}'), clock=lambda: clock[0])
            self.assertEqual(report['reason'], 'job_deadline_exceeded' if late else 'core_bridge_failed')
            self.assertEqual(report['physical_requests'], 1)
            self.assertEqual(report['requests'][0]['raw_response'], '{"usage":{"cost":0.00001}}')
            self.assertEqual(report['spend_ledger']['known_billed_usd'], '0.00001')

    def test_two_hour_global_budget_includes_preparation_and_cannot_be_extended(self):
        clock = [0]
        class Slow(FakeBridge):
            slow = False
            def __call__(self, request):
                if self.slow and request['op'] == 'finished_meeting_export': clock[0] += 3700
                return super().__call__(request)
        bridge = Slow('ok')
        frozen = finished.freeze([fixture('nl')], configuration(), bridge)
        with self.assertRaisesRegex(ValueError, 'two_hours'):
            finished_preflight.run(frozen, bridge, execute=True, max_seconds=7201)
        with self.assertRaisesRegex(ValueError, 'evaluation_start'):
            finished_preflight.run(frozen, bridge, execute=True, clock=lambda: 0, started_at=1)
        bridge.slow = True
        report = finished_preflight.run(frozen, bridge, execute=True, clock=lambda: clock[0],
            transport=lambda *args: self.fail('expired preparation sent a request'))
        self.assertEqual(report['reason'], 'evaluation_budget_exceeded')
        self.assertEqual(report['physical_requests'], 0)
        bridge = FakeBridge('ok')
        fixtures = [{**fixture('nl'), 'id': f'virtual-{index}'} for index in range(250)]
        many = finished.freeze(fixtures, configuration(), bridge)
        clock[0] = 0
        def bounded_transport(endpoint, body, timeout):
            clock[0] += min(29, timeout)
            return 200, b'{}'
        report = finished_preflight.run(many, bridge, execute=True, clock=lambda: clock[0], transport=bounded_transport)
        self.assertEqual(report['reason'], 'evaluation_budget_exceeded')
        self.assertLessEqual(clock[0], 7200)
        self.assertLess(report['physical_requests'], 250)


if __name__ == '__main__':
    unittest.main()
