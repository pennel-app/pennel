import copy
from decimal import Decimal
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from runners import SpendLedger, EvaluationError

META = {'request_sha256': 'a' * 64, 'fixture_id': 'interrupted-invented-calibration', 'batch_index': 1}
EVIDENCE = {'reason': 'coordinator_interruption', 'counter_observation_sha256': 'b' * 64,
            'raw_billed_cost': 'unreported', 'no_aggregate_attribution': True}


class ConservativeBudgetTests(unittest.TestCase):
    def test_full_reservation_moves_once_without_inventing_known_billing(self):
        writes = []
        ledger = SpendLedger(checkpoint=writes.append)
        token = ledger.reserve('.10', META)
        before = ledger.snapshot()
        ledger.resolve_upper_bound(token, META, EVIDENCE)
        state = ledger.snapshot()
        self.assertEqual(state['known_billed_usd'], '0')
        self.assertEqual(state['conservative_upper_bound_usd'], '0.10')
        self.assertEqual(state['total_exposure_usd'], '0.10')
        self.assertEqual(state['pending_reservations'], {})
        self.assertIsNone(state['attempts'][-1]['billed_cost_usd'])
        self.assertEqual(state['attempts'][-1]['reservation_id'], token)
        self.assertEqual(state['attempts'][-1]['metadata'], META)
        self.assertEqual(state['conservative_upper_bounds'][token]['evidence'], EVIDENCE)
        self.assertEqual(len(writes), 2)
        ledger.resolve_upper_bound(token, META, EVIDENCE)
        self.assertEqual(ledger.snapshot(), state)
        self.assertEqual(len(writes), 2)
        self.assertEqual(before['reserved_usd'], '0.10')
        with self.assertRaises(EvaluationError):
            ledger.resolve_upper_bound(token, {**META, 'request_sha256': 'c'*64}, EVIDENCE)

    def test_capacity_and_restart_include_bounds_and_pending(self):
        ledger = SpendLedger('.20')
        token = ledger.reserve('.15', META)
        ledger.resolve_upper_bound(token, META, EVIDENCE)
        self.assertEqual(ledger.total_exposure, Decimal('.15'))
        with self.assertRaises(EvaluationError): ledger.reserve('.06')
        restored = SpendLedger.from_dict(ledger.snapshot())
        self.assertEqual(restored.total_exposure, Decimal('.15'))
        second = restored.reserve('.05')
        self.assertNotEqual(second, token)
        restored.settle(second, '.04')
        self.assertEqual(restored.snapshot()['known_billed_usd'], '0.04')
        self.assertEqual(restored.total_exposure, Decimal('.19'))
        with self.assertRaises(EvaluationError): restored.reserve('.02')

    def test_other_pending_and_overbound_failures_are_not_cleared(self):
        ledger = SpendLedger()
        first, second = ledger.reserve('.10', META), ledger.reserve('.10', {**META, 'batch_index': 2})
        ledger.settle(first, None, META)
        ledger.settle(second, None, {**META, 'batch_index': 2})
        restored = SpendLedger.from_dict(ledger.snapshot())
        restored.resolve_upper_bound(first, META, EVIDENCE)
        self.assertTrue(restored.unresolved)
        with self.assertRaises(EvaluationError): restored.reserve('.01')
        restored.resolve_upper_bound(second, {**META, 'batch_index': 2}, EVIDENCE)
        self.assertFalse(restored.unresolved)
        third = restored.reserve('.01')
        restored.settle(third, '.02')
        self.assertTrue(restored.unresolved)
        reloaded = SpendLedger.from_dict(restored.snapshot())
        self.assertTrue(reloaded.unresolved)
        with self.assertRaises(EvaluationError): reloaded.reserve('.01')

    def test_restore_rejects_dropped_negative_discounted_or_inconsistent_coverage(self):
        ledger = SpendLedger();token = ledger.reserve('.10', META);ledger.resolve_upper_bound(token, META, EVIDENCE)
        original = ledger.snapshot()
        for alter in [lambda d: d.update(conservative_upper_bound_usd='0'),
                      lambda d: d['conservative_upper_bounds'].pop(token),
                      lambda d: d['conservative_upper_bounds'][token].update(upper_bound_usd='-.10'),
                      lambda d: d['conservative_upper_bounds'][token].update(upper_bound_usd='.09'),
                      lambda d: d.update(total_exposure_usd='0'),
                      lambda d: d['attempts'][-1].update(billed_cost_usd='0')]:
            value = copy.deepcopy(original);alter(value)
            with self.assertRaises(EvaluationError): SpendLedger.from_dict(value)

    def test_failed_checkpoint_keeps_entire_previous_exposure_in_memory(self):
        def checkpoint(state):
            if state.get('conservative_upper_bounds'): raise OSError('synthetic durable write unavailable')
        ledger = SpendLedger(checkpoint=checkpoint);token = ledger.reserve('.10', META)
        before = ledger.snapshot()
        with self.assertRaises(OSError): ledger.resolve_upper_bound(token, META, EVIDENCE)
        self.assertEqual(ledger.snapshot(), before)
        self.assertEqual(ledger.total_exposure, Decimal('.10'))

    def test_unexplained_manual_stop_or_dropped_unknown_identity_is_not_cleared(self):
        ledger = SpendLedger();ledger.unresolved = True
        restored = SpendLedger.from_dict(ledger.snapshot())
        self.assertTrue(restored.unresolved)
        with self.assertRaises(EvaluationError): restored.reserve('.01')
        state = SpendLedger().snapshot();state['unknown_reservation_tokens'] = ['missing-token']
        with self.assertRaises(EvaluationError): SpendLedger.from_dict(state)
        stopped = SpendLedger();known = stopped.reserve('.01');pending = stopped.reserve('.10', META)
        stopped.settle(known, '.02')
        restored = SpendLedger.from_dict(stopped.snapshot())
        restored.resolve_upper_bound(pending, META, EVIDENCE)
        self.assertTrue(restored.unresolved)
        with self.assertRaises(EvaluationError): restored.reserve('.01')

    def test_legacy_ledgers_default_to_zero_bound_and_still_fail_closed(self):
        state = {'limit_usd': '2.00', 'known_billed_usd': '.01', 'reserved_usd': '0',
                 'pending_reservations': {}, 'unresolved_charge': False, 'attempts': []}
        ledger = SpendLedger.from_dict(state)
        self.assertEqual(ledger.snapshot()['conservative_upper_bound_usd'], '0')
        self.assertEqual(ledger.total_exposure, Decimal('.01'))
        state.update(unresolved_charge=True)
        stopped = SpendLedger.from_dict(state)
        self.assertTrue(stopped.unresolved)
        with self.assertRaises(EvaluationError): stopped.reserve('.01')


if __name__ == '__main__': unittest.main()
