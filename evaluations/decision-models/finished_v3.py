"""Frozen production v3 exports with actual state/support context coverage."""

from collections import defaultdict
from pathlib import Path
import time

import finished

PROTOCOL = Path(__file__).with_name('protocol-finished-production-v3.json')
PROTOCOL_ID = 'production-finished-meeting-v3'
RUNTIME_POLICY = 'raw_supported_passage_decisions_v3'


def _deadline(deadline, clock):
    if deadline is not None and clock() >= deadline:
        raise finished.FinishedError('evaluation_budget_exceeded')


def context_coverage(exports, expected_answers):
    """Use actual exported templates/slots/sizes, never inferred fixture lengths."""
    observed, required, details = defaultdict(list), defaultdict(set), {}
    for exported in exports:
        fixture = exported['fixture'];split = fixture['split']
        for index, window in enumerate(exported['windows']):
            batch = exported['batches'][window['batch_index']]
            for question in window['request']['questions']:
                if question['kind'] != 'choice': continue
                signature = finished.digest({'question':question,'physical_prompt_context_sha256':window['prompt_context_sha256']})
                required[signature].add(split)
                details[signature] = {'question_id':question['id'],'batch_size':len(batch['window_indices']),
                    'slot':window['batch_slot'],'prompt_sha256':signature}
                if split == 'calibration':
                    observed[signature].append((expected_answers[fixture['id']][index][question['id']],fixture['family_id']))
    return [details[key] | {'required_splits':sorted(required[key]),'observations':len(observed[key]),
        'classes':len({label for label,_ in observed[key]}),'families':len({family for _,family in observed[key]})}
        for key in sorted(required)]


def freeze(fixtures, configuration, bridge, expected_actual=None, expected_answers=None, *, deadline=None, clock=time.monotonic):
    _deadline(deadline,clock)
    if not isinstance(fixtures,list) or not fixtures:
        raise finished.FinishedError('fixtures_required')
    families, ids, exports = {},set(),[]
    for fixture in fixtures:
        _deadline(deadline,clock)
        if not isinstance(fixture,dict) or fixture.get('language') != 'nl':
            raise finished.FinishedError('invalid_fixture_language')
        split,family,identifier = fixture.get('split'),fixture.get('family_id'),fixture.get('id')
        if split not in ('development','calibration','heldout') or not isinstance(family,str) or not family:
            raise finished.FinishedError('invalid_fixture_split')
        if not isinstance(identifier,str) or not identifier or identifier in ids:
            raise finished.FinishedError('duplicate_or_missing_fixture_id')
        ids.add(identifier)
        if family in families and families[family] != split:
            raise finished.FinishedError('family_split_leakage')
        families[family]=split
        exported=finished._export_fixture(bridge,fixture,configuration,expected_actual,deadline=deadline,clock=clock)
        if (exported.get('schema_version') != 3 or exported.get('protocol_id') != PROTOCOL_ID
                or exported.get('runtime_policy') != RUNTIME_POLICY):
            raise finished.FinishedError('supported_production_contract_required')
        exports.append(exported)
    finished._validate_expected_answers(exports,expected_answers)
    if expected_answers is None:raise finished.FinishedError('state_and_support_judgments_required')
    coverage=context_coverage(exports,expected_answers)
    if not coverage or any(row['observations']<10 or row['classes']<2 or row['families']<2 for row in coverage):
        raise finished.FinishedError('unsupported_actual_prompt_context')
    value={'schema_version':3,'protocol_id':PROTOCOL_ID,'runtime_policy':RUNTIME_POLICY,
        'protocol_sha256':finished.file_hash(PROTOCOL),'runner_sha256':finished.file_hash(__file__),
        'helper_sha256':finished.file_hash(finished.__file__),'bridge_sha256':bridge.artifact_sha256,
        'exports':exports,'expected_answers':expected_answers,'calibration_context_coverage':coverage,
        'split_sha256':finished.digest(sorted(families.items())),'production_admission':False}
    value['freeze_sha256']=finished.digest(value)
    return value


def verify_freeze(frozen,bridge,*,deadline=None,clock=time.monotonic):
    _deadline(deadline,clock)
    if (not isinstance(frozen,dict) or frozen.get('schema_version')!=3 or frozen.get('protocol_id')!=PROTOCOL_ID
            or frozen.get('runtime_policy')!=RUNTIME_POLICY or frozen.get('protocol_sha256')!=finished.file_hash(PROTOCOL)
            or frozen.get('runner_sha256')!=finished.file_hash(__file__)
            or frozen.get('helper_sha256')!=finished.file_hash(finished.__file__)
            or frozen.get('bridge_sha256')!=bridge.artifact_sha256
            or frozen.get('freeze_sha256')!=finished.digest({k:v for k,v in frozen.items() if k!='freeze_sha256'})
            or frozen.get('production_admission') is not False):
        raise finished.FinishedError('production_freeze_changed')
    exports=frozen.get('exports')
    if not isinstance(exports,list) or not exports:raise finished.FinishedError('production_freeze_changed')
    rebuilt=freeze([value['fixture'] for value in exports],exports[0]['configuration'],bridge,
        exports[0].get('expected_actual'),frozen.get('expected_answers'),deadline=deadline,clock=clock)
    _deadline(deadline,clock)
    if rebuilt!=frozen:raise finished.FinishedError('production_freeze_changed')


def requests(exported):
    batches=exported.get('batches')
    if not isinstance(batches,list) or not batches:raise finished.FinishedError('supported_production_contract_required')
    for index,batch in enumerate(batches):
        if batch.get('index')!=index or not isinstance(batch.get('wire_body'),str):
            raise finished.FinishedError('invalid_physical_batch_map')
        yield batch
