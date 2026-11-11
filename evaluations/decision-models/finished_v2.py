"""Frozen batched production exports; historical v1 helpers remain unchanged."""

from pathlib import Path
import time

import finished

PROTOCOL = Path(__file__).with_name('protocol-finished-production-v2.json')
PROTOCOL_ID = 'production-finished-meeting-v2'


def freeze(fixtures, configuration, bridge, expected_actual=None, expected_answers=None, *, deadline=None, clock=time.monotonic):
    _deadline(deadline, clock)
    if not isinstance(fixtures, list) or not fixtures:
        raise finished.FinishedError('fixtures_required')
    families, ids, exports = {}, set(), []
    for fixture in fixtures:
        if not isinstance(fixture, dict) or fixture.get('language') not in ('en', 'nl', 'mixed'):
            raise finished.FinishedError('invalid_fixture_language')
        split, family, identifier = fixture.get('split'), fixture.get('family_id'), fixture.get('id')
        if split not in ('development', 'calibration', 'heldout') or not isinstance(family, str) or not family:
            raise finished.FinishedError('invalid_fixture_split')
        if not isinstance(identifier, str) or not identifier or identifier in ids:
            raise finished.FinishedError('duplicate_or_missing_fixture_id')
        ids.add(identifier)
        if family in families and families[family] != split:
            raise finished.FinishedError('family_split_leakage')
        families[family] = split
        exported = finished._export_fixture(bridge, fixture, configuration, expected_actual, deadline=deadline, clock=clock)
        if exported.get('schema_version') != 2 or exported.get('protocol_id') != PROTOCOL_ID:
            raise finished.FinishedError('batched_production_contract_required')
        exports.append(exported)
    finished._validate_expected_answers(exports, expected_answers)
    value = {'schema_version': 2, 'protocol_id': PROTOCOL_ID,
             'protocol_sha256': finished.file_hash(PROTOCOL), 'runner_sha256': finished.file_hash(__file__),
             'helper_sha256': finished.file_hash(finished.__file__), 'bridge_sha256': bridge.artifact_sha256,
             'exports': exports, 'expected_answers': expected_answers,
             'split_sha256': finished.digest(sorted(families.items())), 'production_admission': False}
    value['freeze_sha256'] = finished.digest(value)
    return value


def _deadline(deadline, clock):
    if deadline is not None and clock() >= deadline:
        raise finished.FinishedError('evaluation_budget_exceeded')


def verify_freeze(frozen, bridge, *, deadline=None, clock=time.monotonic):
    _deadline(deadline, clock)
    if (not isinstance(frozen, dict) or frozen.get('schema_version') != 2
            or frozen.get('protocol_id') != PROTOCOL_ID
            or frozen.get('protocol_sha256') != finished.file_hash(PROTOCOL)
            or frozen.get('runner_sha256') != finished.file_hash(__file__)
            or frozen.get('helper_sha256') != finished.file_hash(finished.__file__)
            or frozen.get('bridge_sha256') != bridge.artifact_sha256
            or frozen.get('freeze_sha256') != finished.digest({k: v for k, v in frozen.items() if k != 'freeze_sha256'})
            or frozen.get('production_admission') is not False):
        raise finished.FinishedError('production_freeze_changed')
    exports = frozen.get('exports')
    if not isinstance(exports, list) or not exports:
        raise finished.FinishedError('production_freeze_changed')
    rebuilt = freeze([value['fixture'] for value in exports], exports[0]['configuration'], bridge,
                     exports[0].get('expected_actual'), frozen.get('expected_answers'), deadline=deadline, clock=clock)
    _deadline(deadline, clock)
    if rebuilt != frozen:
        raise finished.FinishedError('production_freeze_changed')


def requests(exported):
    batches = exported.get('batches')
    if not isinstance(batches, list) or not batches:
        raise finished.FinishedError('batched_production_contract_required')
    for index, batch in enumerate(batches):
        if batch.get('index') != index or not isinstance(batch.get('wire_body'), str):
            raise finished.FinishedError('invalid_physical_batch_map')
        yield batch
