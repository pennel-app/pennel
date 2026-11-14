"""Production exports for calibrated support abstention with raw state choices."""

from pathlib import Path
import time

import finished
import finished_v3
import finished_calibration_reuse as reuse

RUNTIME_POLICY = reuse.RUNTIME_POLICY
PROTOCOL_ID = finished_v3.PROTOCOL_ID
PROTOCOL = Path(__file__).with_name('protocol-finished-production-support-v1.json')
QUALIFICATION_PROTOCOL = Path(__file__).with_name('protocol-finished-qualification-cloud-support-v1.json')


def freeze(fixtures, configuration, bridge, expected_actual=None, expected_answers=None, *, deadline=None, clock=time.monotonic):
    finished_v3._deadline(deadline, clock)
    if not isinstance(fixtures, list) or not fixtures:
        raise finished.FinishedError('fixtures_required')
    families, identifiers, exports = {}, set(), []
    for fixture in fixtures:
        finished_v3._deadline(deadline, clock)
        split, family, identifier = fixture.get('split'), fixture.get('family_id'), fixture.get('id')
        if fixture.get('language') != 'nl':
            raise finished.FinishedError('invalid_fixture_language')
        if split not in ('development', 'calibration', 'heldout') or not isinstance(family, str) or not family:
            raise finished.FinishedError('invalid_fixture_split')
        if not isinstance(identifier, str) or not identifier or identifier in identifiers:
            raise finished.FinishedError('duplicate_or_missing_fixture_id')
        identifiers.add(identifier)
        if family in families and families[family] != split:
            raise finished.FinishedError('family_split_leakage')
        families[family] = split
        exported = finished._export_fixture(bridge, fixture, configuration, expected_actual, deadline=deadline, clock=clock)
        if (exported.get('schema_version') != 3 or exported.get('protocol_id') != PROTOCOL_ID
                or exported.get('runtime_policy') != RUNTIME_POLICY):
            raise finished.FinishedError('calibrated_support_production_contract_required')
        reuse.metadata(exported)
        exports.append(exported)
    finished._validate_expected_answers(exports, expected_answers)
    if expected_answers is None:
        raise finished.FinishedError('state_and_support_judgments_required')
    coverage = finished_v3.context_coverage(exports, expected_answers)
    if not coverage or any(row['observations'] < 10 or row['classes'] < 2 or row['families'] < 2 for row in coverage):
        raise finished.FinishedError('unsupported_actual_prompt_context')
    value = {'schema_version':3, 'protocol_id':PROTOCOL_ID, 'runtime_policy':RUNTIME_POLICY,
        'protocol_sha256':finished.file_hash(PROTOCOL), 'runner_sha256':finished.file_hash(__file__),
        'helper_sha256':finished.file_hash(finished.__file__), 'bridge_sha256':bridge.artifact_sha256,
        'exports':exports, 'expected_answers':expected_answers, 'calibration_context_coverage':coverage,
        'split_sha256':finished.digest(sorted(families.items())), 'production_admission':False}
    value['freeze_sha256'] = finished.digest(value)
    return value


def verify_freeze(frozen, bridge, *, deadline=None, clock=time.monotonic):
    finished_v3._deadline(deadline, clock)
    if (not isinstance(frozen, dict) or frozen.get('schema_version') != 3
            or frozen.get('protocol_id') != PROTOCOL_ID or frozen.get('runtime_policy') != RUNTIME_POLICY
            or frozen.get('protocol_sha256') != finished.file_hash(PROTOCOL)
            or frozen.get('runner_sha256') != finished.file_hash(__file__)
            or frozen.get('helper_sha256') != finished.file_hash(finished.__file__)
            or frozen.get('bridge_sha256') != bridge.artifact_sha256 or not reuse._sealed(frozen)
            or frozen.get('production_admission') is not False):
        raise finished.FinishedError('production_freeze_changed')
    exports = frozen.get('exports')
    if not isinstance(exports, list) or not exports:
        raise finished.FinishedError('production_freeze_changed')
    rebuilt = freeze([x['fixture'] for x in exports], exports[0]['configuration'], bridge,
        exports[0].get('expected_actual'), frozen.get('expected_answers'), deadline=deadline, clock=clock)
    if rebuilt != frozen:
        raise finished.FinishedError('production_freeze_changed')


requests = finished_v3.requests
