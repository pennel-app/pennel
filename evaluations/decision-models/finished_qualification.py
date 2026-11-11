"""Offline, production-bound calibration/item/resource qualification. Never sends requests."""

import argparse
from collections import Counter
import copy
import hashlib
import json
import math
import time
from decimal import Decimal, InvalidOperation
from pathlib import Path

import calibration
import finished
import finished_v2
import metrics
import runners

PROTOCOL = Path(__file__).with_name('protocol-finished-qualification-v1.json')
KINDS = ('action', 'decision', 'open_question', 'changed_commitment')
SOURCES = ('finished_qualification.py', 'finished.py', 'finished_preflight.py', 'calibration.py', 'metrics.py')
V2_SOURCES = SOURCES + ('finished_v2.py', 'finished_experiment.py', 'finished_corpus_v2.py', 'runners.py')
PROTOCOL_V2 = PROTOCOL.with_name('protocol-finished-qualification-v2.json')
RUNTIME_POLICY = 'raw_closed_set_passage_decisions_v2'


def _seal(value):
    value['freeze_sha256'] = finished.digest({key: item for key, item in value.items() if key != 'freeze_sha256'})
    return value


def _base(frozen):
    value = {key: copy.deepcopy(item) for key, item in frozen.items()
             if key not in ('calibration', 'calibration_runs', 'calibration_runs_sha256', 'base_freeze_sha256', 'freeze_sha256')}
    value['stage'] = 'calibration'
    return _seal(value)


def _source_hashes(v2=False):
    return {name: finished.file_hash(Path(__file__).with_name(name)) for name in (V2_SOURCES if v2 else SOURCES)}


def _v2(frozen):
    return frozen.get('schema_version') == 2 and frozen.get('protocol_id') == 'production-finished-qualification-v2'


def _protocol(frozen):
    return PROTOCOL_V2 if _v2(frozen) else PROTOCOL


def _contract(frozen):
    return finished_v2 if _v2(frozen) else finished


def physical_requests(exported):
    return exported['batches'] if exported.get('schema_version') == 2 else exported['windows']


def _valid_actual(configuration, actual):
    if not isinstance(actual, dict): return False
    if actual.get('identity_evidence') == 'provider_reported_version':
        model = 'perplexity/pplx-decider-v1.2-20261016'
        return (configuration.get('protocol') == 'open_router_decisions'
            and configuration.get('endpoint') == 'https://openrouter.ai/api/alpha/decisions'
            and configuration.get('model') in ('perplexity/pplx-decider-v1.2', model)
            and configuration.get('expected_model') == model and configuration.get('upstream') == 'perplexity'
            and actual.get('model') == model and actual.get('provider') == 'Perplexity'
            and actual.get('revision') in ('', None) and actual.get('artifact_sha256') in ('', None))
    if actual.get('identity_evidence', 'serving_revision_and_artifact') != 'serving_revision_and_artifact':
        return False
    return (all(actual.get(key) for key in ('model', 'revision', 'provider'))
        and isinstance(actual.get('artifact_sha256'), str) and len(actual['artifact_sha256']) == 64
        and all(char in '0123456789abcdef' for char in actual['artifact_sha256']))


def _projection(item):
    return {key: item.get(key) for key in ('kind', 'commitment', 'text', 'owner', 'due', 'sources')}


def _gold(exported, rows):
    if not isinstance(rows, list):
        raise ValueError('complete_item_judgments_required')
    result = []
    for item in rows:
        if (not isinstance(item, dict) or set(item) != {'window_index', 'kind', 'commitment', 'text', 'owner', 'due'}
                or item['kind'] not in KINDS or item['commitment'] not in ('tentative', 'confirmed', 'retracted', 'unresolved', 'changed', 'unknown')
                or isinstance(item['window_index'], bool) or not isinstance(item['window_index'], int)
                or not 0 <= item['window_index'] < len(exported['windows'])):
            raise ValueError('invalid_complete_item_judgment')
        window = exported['windows'][item['window_index']]
        sources = [entry['source'] for entry in window['sources']]
        if not sources or item['text'] != sources[0]['quote']:
            # v1 qualifies the two selected passage routes. Exact-span extraction needs
            # its own authored source-bound judgments rather than an inferred match.
            raise ValueError('judgment_must_match_exact_production_passage')
        for key in ('owner', 'due'):
            detail = item[key]
            if detail is not None:
                if not isinstance(detail, dict) or set(detail) != {'text', 'source_index', 'start_byte', 'end_byte'}:
                    raise ValueError('invalid_supported_detail_judgment')
                index, start, end = (detail[name] for name in ('source_index', 'start_byte', 'end_byte'))
                if any(isinstance(value, bool) or not isinstance(value, int) for value in (index, start, end)) or not 0 <= index < len(sources):
                    raise ValueError('invalid_supported_detail_judgment')
                source = sources[index]['quote'].encode()
                if not 0 <= start < end <= len(source) or source[start:end] != detail['text'].encode():
                    raise ValueError('unsupported_detail_judgment')
        result.append({**_projection(item), 'sources': sources, 'window_index': item['window_index']})
    keys = [finished.digest(item) for item in result]
    if len(keys) != len(set(keys)):
        raise ValueError('duplicate_complete_item_judgment')
    return result


def freeze(fixtures, configuration, bridge, expected_actual, expected_answers, gold_items):
    if not _valid_actual(configuration, expected_actual):
        raise ValueError('verified_actual_identity_required')
    v2 = expected_actual.get('identity_evidence') == 'provider_reported_version'
    if any(item.get('language') != 'nl' for item in fixtures):
        raise ValueError('initial_qualification_is_dutch_only')
    production = (finished_v2 if v2 else finished).freeze(fixtures, configuration, bridge, expected_actual, expected_answers)
    ids = {item['fixture']['id'] for item in production['exports']}
    if not isinstance(gold_items, dict) or set(gold_items) != ids:
        raise ValueError('complete_item_judgments_required')
    gold = {item['fixture']['id']: _gold(item, gold_items[item['fixture']['id']]) for item in production['exports']}
    splits = {item['fixture']['split'] for item in production['exports']}
    if splits != {'development', 'calibration', 'heldout'}:
        raise ValueError('all_disjoint_splits_required')
    value = {'schema_version': 2 if v2 else 1, 'protocol_id': f'production-finished-qualification-v{2 if v2 else 1}',
        'protocol_sha256': finished.file_hash(PROTOCOL_V2 if v2 else PROTOCOL), 'source_sha256': _source_hashes(v2),
        'production': production, 'gold_items': gold, 'stage': 'calibration', 'production_admission': False}
    if v2:
        value.update(runtime_policy=RUNTIME_POLICY, resource_fixture_id=next(item['id'] for item in fixtures if item['split'] == 'development'))
    return _seal(value)


def _deadline(deadline, clock):
    if deadline is not None and clock() >= deadline:
        raise finished.FinishedError('evaluation_budget_exceeded')


def verify_freeze(frozen, bridge, *, deadline=None, clock=time.monotonic):
    _deadline(deadline, clock)
    if ((frozen.get('schema_version'), frozen.get('protocol_id')) not in ((1, 'production-finished-qualification-v1'), (2, 'production-finished-qualification-v2'))
            or frozen.get('protocol_sha256') != finished.file_hash(_protocol(frozen)) or frozen.get('source_sha256') != _source_hashes(_v2(frozen))
            or frozen.get('freeze_sha256') != finished.digest({key: item for key, item in frozen.items() if key != 'freeze_sha256'})
            or frozen.get('production_admission') is not False or frozen.get('stage') not in ('calibration', 'heldout_ready')):
        raise ValueError('qualification_freeze_changed')
    if _v2(frozen) and frozen.get('runtime_policy') != RUNTIME_POLICY:
        raise ValueError('runtime_selection_policy_changed')
    _contract(frozen).verify_freeze(frozen['production'], bridge, deadline=deadline, clock=clock)
    if frozen['stage'] == 'heldout_ready':
        base = _base(frozen)
        if base['freeze_sha256'] != frozen['base_freeze_sha256'] or finished.digest(frozen['calibration_runs']) != frozen['calibration_runs_sha256']:
            raise ValueError('unlinked_calibration')
        rows, _, _ = _runs(base, frozen['calibration_runs'], bridge, 'calibration', deadline=deadline, clock=clock)
        expected = calibration.fit_predictions(rows, run_manifest_hash=base['freeze_sha256'])
        _deadline(deadline, clock)
        if expected != frozen['calibration']:
            raise ValueError('calibration_artifact_changed')


def requests(frozen, split, bridge):
    verify_freeze(frozen, bridge)
    if split not in ('development', 'calibration', 'heldout'):
        raise ValueError('unknown_split')
    if split == 'heldout' and frozen['stage'] != 'heldout_ready':
        raise ValueError('calibration_before_heldout_required')
    return [{'fixture_id': exported['fixture']['id'],
             ('batch_index' if _v2(frozen) else 'window_index'): batch['index'],
             'freeze_sha256': frozen['freeze_sha256'], 'export_sha256': exported['freeze_sha256'],
             'wire_body': batch['wire_body'], 'request_sha256': hashlib.sha256(batch['wire_body'].encode()).hexdigest(),
             'endpoint': exported['configuration']['endpoint'], 'whole_job_timeout_secs': exported['configuration']['timeout_secs']}
            for exported in frozen['production']['exports'] if exported['fixture']['split'] == split
            for batch in physical_requests(exported)]


def _number(value):
    return not isinstance(value, bool) and isinstance(value, (int, float)) and math.isfinite(value) and value >= 0


def _run(frozen, exported, run, bridge, *, deadline=None, clock=time.monotonic):
    _deadline(deadline, clock)
    responses = run.get('responses')
    hashes = [hashlib.sha256(batch['wire_body'].encode()).hexdigest() for batch in physical_requests(exported)]
    if (run.get('freeze_sha256') != frozen['freeze_sha256'] or run.get('export_sha256') != exported['freeze_sha256']
            or run.get('evidence_class') not in ('recorded_inference', 'offline_fixture')
            or not isinstance(responses, list) or run.get('request_sha256s') != hashes[:len(responses)]):
        raise ValueError('unlinked_prediction_or_changed_request')
    for key, value in run.get('measurements', {}).items():
        if key in ('latency_ms', 'whole_job_elapsed_ms', 'cpu_time_ms', 'peak_rss_bytes', 'executed_requests', 'input_bytes') and not _number(value):
            raise ValueError('invalid_resource_measurement')
    replay = finished._bridge_call(bridge, {'op': 'finished_meeting_replay', 'export': exported, 'responses': responses}, deadline=deadline, clock=clock)
    elapsed = run.get('measurements', {}).get('whole_job_elapsed_ms', run.get('measurements', {}).get('latency_ms'))
    if (replay.get('complete') is True and elapsed is not None
            and elapsed > exported['configuration']['timeout_secs'] * 1000):
        raise ValueError('successful_job_exceeds_whole_job_deadline')
    return replay


def _execution_reconciled(run):
    if (run.get('evidence_class') != 'recorded_inference' or run.get('status') != 'ok'
            or run.get('reason') is not None):
        return False
    ledger = run.get('spend_ledger')
    if (not isinstance(ledger, dict) or ledger.get('unresolved_charge') is not False
            or ledger.get('pending_reservations') != {}):
        return False
    try:
        restored = runners.SpendLedger.from_dict(ledger)
        return not restored.unresolved and 0 <= restored.total_exposure <= restored.limit <= 2 and not restored.pending
    except (runners.EvaluationError, KeyError, InvalidOperation, ValueError, TypeError):
        return False


def _runs(frozen, predictions, bridge, split, *, deadline=None, clock=time.monotonic):
    _deadline(deadline, clock)
    exports = {item['fixture']['id']: item for item in frozen['production']['exports'] if item['fixture']['split'] == split}
    if not isinstance(predictions, list):
        raise ValueError('predictions_required')
    observed = {}
    for run in predictions:
        identifier = run.get('fixture_id')
        if identifier not in exports or identifier in observed:
            raise ValueError('duplicate_unknown_or_wrong_split_prediction')
        observed[identifier] = (run, _run(frozen, exports[identifier], run, bridge, deadline=deadline, clock=clock))
    rows, jobs = [], {}
    for identifier, exported in exports.items():
        run, replay = observed.get(identifier, ({}, {}))
        complete = replay.get('complete') is True and replay.get('status') == 'ok'
        elapsed = run.get('measurements', {}).get('whole_job_elapsed_ms')
        deadline_verified = _number(elapsed) and 0 < elapsed <= exported['configuration']['timeout_secs'] * 1000
        jobs[identifier] = {'run': run, 'replay': replay, 'complete': complete, 'deadline_verified': deadline_verified, 'execution_reconciled': _execution_reconciled(run)}
        windows = {window['window_index']: window for window in replay.get('windows', [])}
        identity = exported['expected_actual']
        for index, window in enumerate(exported['windows']):
            actual = windows.get(index, {})
            answers = {answer['id']: answer for answer in actual.get('result', {}).get('answers', [])}
            for question in window['request']['questions']:
                if question['kind'] != 'choice':
                    continue
                key = question['id']
                answer = answers.get(key, {})
                valid = actual.get('status') == 'ok' and answer.get('abstained') is False
                rows.append({'id': f'{identifier}:{index}:{key}', 'fixture_id': identifier, 'window_index': index,
                    'task': 'events', 'subtask': key, 'component': key, 'split': split, 'language': 'nl',
                    'group_id': exported['fixture']['family_id'], 'runner_id': 'production-finished',
                    'expected': frozen['production']['expected_answers'][identifier][index][key],
                    'prediction': answer.get('choice') if valid else None,
                    'probabilities': {value['option_id']: value['probability'] for value in answer.get('probabilities', [])} if valid else None,
                    'score_type': 'uncalibrated_typed_probability', 'prompt_hash': finished.digest({'question': question, 'physical_prompt_context_sha256': window['prompt_context_sha256']}) if _v2(frozen) else finished.digest(question),
                    'label_order': [option['id'] for option in question['options']],
                    'grouping': exported['qualification_request']['grouping'],
                    'identity': {'transport': exported['configuration']['protocol'], **identity},
                    'run_manifest_hash': frozen.get('base_freeze_sha256', frozen['freeze_sha256'])})
    return rows, jobs, len(exports)


def calibrate(frozen, predictions, bridge):
    verify_freeze(frozen, bridge)
    if frozen['stage'] != 'calibration':
        raise ValueError('calibration_already_frozen')
    allowed = {item['fixture']['id'] for item in frozen['production']['exports'] if item['fixture']['split'] == 'calibration'}
    if any(row.get('fixture_id') not in allowed for row in predictions):
        raise ValueError('calibration_only_predictions_required')
    rows, _, _ = _runs(frozen, predictions, bridge, 'calibration')
    result = copy.deepcopy(frozen)
    result.update(stage='heldout_ready', base_freeze_sha256=frozen['freeze_sha256'],
        calibration=calibration.fit_predictions(rows, run_manifest_hash=frozen['freeze_sha256']),
        calibration_runs=copy.deepcopy(predictions), calibration_runs_sha256=finished.digest(predictions))
    return _seal(result)


def _calibrated(rows, artifact):
    result = []
    for row in rows:
        value = {**row, 'prediction': None, 'accepted': False}
        if row['probabilities']:
            context = calibration.prediction_context(row)
            fit = artifact['fits'].get(metrics._context_hash(context))
            if fit and fit['status'] == 'fitted':
                distribution = metrics.apply_calibration(row['probabilities'], fit['fit'], context)
                chosen = max(sorted(distribution), key=distribution.get)
                threshold = fit['fit']['threshold']['threshold']
                value.update(prediction=chosen, accepted=threshold is not None and distribution[chosen] >= threshold,
                             probabilities=distribution, score_type='calibrated_probabilities')
        result.append(value)
    return result


def _items(frozen, jobs, rows):
    counts = Counter(expected_count=0, proposed_count=0, correct_count=0, duplicates=0, unsupported=0)
    complete_rows = []
    for identifier, job in jobs.items():
        gold = frozen['gold_items'][identifier]
        counts['expected_count'] += len(gold)
        # Production commits nothing unless every window finishes. Raw proposals
        # cannot gain quality credit by selectively dropping a failed window.
        candidates = [(window['window_index'], item) for window in job['replay'].get('windows', [])
                      for item in window.get('candidates', [])] if job['complete'] else []
        counts['proposed_count'] += len(candidates)
        available = [dict(item) for item in gold]
        correct = Counter()
        seen = set()
        for index, item in candidates:
            key = finished.digest({'window_index': index, **_projection(item)})
            if key in seen:
                counts['duplicates'] += 1
            seen.add(key)
            match = next((expected for expected in available if expected == {'window_index': index, **_projection(item)}), None)
            if match is not None:
                available.remove(match)
                counts['correct_count'] += 1
                correct[index, item['kind']] += 1
            else:
                counts['unsupported'] += 1
        for row in rows:
            if row['fixture_id'] != identifier:
                continue
            value = dict(row)
            required = sum(item['window_index'] == row['window_index'] and item['kind'] == row['component'] for item in gold)
            if required and correct[row['window_index'], row['component']] != required:
                value.update(prediction=None, accepted=False)
            complete_rows.append(value)
    counts.update(missed=counts['expected_count'] - counts['correct_count'],
                  false_positives=counts['proposed_count'] - counts['correct_count'])
    return {**dict(counts), 'precision': counts['correct_count'] / counts['proposed_count'] if counts['proposed_count'] else None,
            'recall': counts['correct_count'] / counts['expected_count'] if counts['expected_count'] else None,
            'classification': metrics.classification_metrics(complete_rows)}


def _resources(frozen, resources, bridge):
    missing = {'complete': False, 'cold_samples': 0, 'warm_samples': 0, 'warm_p95_ms': None,
               'peak_rss_bytes': None, 'cpu_time_ms': None, 'executed_requests': None,
               'remote_server_cpu_rss': 'unavailable'}
    if resources is None:
        return missing
    exported = next(item for item in frozen['production']['exports'] if item['fixture']['id'] == frozen.get('resource_fixture_id')) if _v2(frozen) else next(item for item in frozen['production']['exports'] if item['fixture']['split'] == 'development')
    profile = exported['qualification_request'].get('resource_profile_sha256')
    if resources.get('freeze_sha256') != frozen['freeze_sha256'] or resources.get('resource_profile_sha256') != profile:
        raise ValueError('unlinked_runtime_resource_evidence')
    required = {f'cold-{index}': 'cold' for index in range(5)} | {f'warm-{index}': 'warm' for index in range(30)}
    seen, samples = set(), []
    for sample in resources.get('samples', []):
        key = sample.get('id')
        if key in seen or key not in required or sample.get('phase') != required[key]:
            raise ValueError('resource_panel_changed')
        seen.add(key)
        run = sample['run']
        if run.get('fixture_id') != exported['fixture']['id']:
            raise ValueError('resource_panel_changed')
        replay = _run(frozen, exported, run, bridge)
        measure = run.get('measurements', {})
        if (run.get('evidence_class') != 'recorded_inference' or replay.get('status') != 'ok'
                or not replay.get('complete') or not _execution_reconciled(run) or not _number(measure.get('whole_job_elapsed_ms'))
                or not measure.get('whole_job_elapsed_ms')
                or measure.get('executed_requests') != len(physical_requests(exported))):
            return missing
        samples.append((sample['phase'], measure))
    cpu, rss = resources.get('cpu_time_ms'), resources.get('peak_rss_bytes')
    scope = 'owned_client_and_local_model' if exported['configuration']['protocol'] == 'local_decider' else 'owned_client'
    if seen != set(required) or resources.get('scope') != scope or not _number(cpu) or not _number(rss) or not cpu or not rss:
        return missing
    warm = [measure['whole_job_elapsed_ms'] for phase, measure in samples if phase == 'warm']
    return {**missing, 'complete': True, 'scope': scope, 'cold_samples': 5, 'warm_samples': 30,
            'warm_p95_ms': metrics.percentile(warm, 95), 'peak_rss_bytes': rss, 'cpu_time_ms': cpu,
            'executed_requests': sum(measure['executed_requests'] for _, measure in samples)}


def evaluate(frozen, predictions, bridge, resources=None):
    verify_freeze(frozen, bridge)
    if frozen['stage'] != 'heldout_ready':
        raise ValueError('calibration_before_heldout_required')
    rows, jobs, scheduled = _runs(frozen, predictions, bridge, 'heldout')
    _, calibration_jobs, calibration_scheduled = _runs(_base(frozen), frozen['calibration_runs'], bridge, 'calibration')
    raw = metrics.classification_metrics(rows)
    calibrated = metrics.classification_metrics(_calibrated(rows, frozen['calibration']))
    items, runtime = _items(frozen, jobs, rows), _resources(frozen, resources, bridge)
    calibration_actual = bool(frozen['calibration_runs']) and all(row['evidence_class'] == 'recorded_inference' for row in frozen['calibration_runs'])
    actual = len(predictions) == scheduled and all(row['evidence_class'] == 'recorded_inference' for row in predictions)
    complete = all(job['complete'] for job in jobs.values())
    fits = list(frozen['calibration']['fits'].values())
    gates = []
    if not actual or not calibration_actual: gates.append('actual_inference_unmeasured')
    if not complete: gates.append('production_jobs_incomplete')
    if (not all(job['execution_reconciled'] for job in jobs.values())
            or not all(job['execution_reconciled'] for job in calibration_jobs.values())):
        gates.append('original_execution_failed_or_unreconciled')
    if len(frozen['calibration_runs']) != calibration_scheduled or not all(job['complete'] for job in calibration_jobs.values()):
        gates.append('calibration_jobs_incomplete')
    if not all(job['deadline_verified'] for job in jobs.values()) or not all(job['deadline_verified'] for job in calibration_jobs.values()):
        gates.append('original_job_deadline_measurements_unavailable')
    if not fits or any(fit['status'] != 'fitted' for fit in fits): gates.append('uncalibrated_contexts')
    if raw['macro_f1'] is None or raw['macro_f1'] < .90 or calibrated['macro_f1'] is None or calibrated['macro_f1'] < .90:
        gates.append('component_quality_failed')
    if items['classification']['macro_f1'] is None or items['classification']['macro_f1'] < .90:
        gates.append('complete_item_quality_failed')
    if items['precision'] is None or items['precision'] < .95: gates.append('accepted_item_precision_failed')
    if items['duplicates'] or items['unsupported']: gates.append('unsupported_owner_due_state_or_source')
    if not runtime['complete']: gates.append('resource_measurements_unavailable')
    configured = frozen['production']['exports'][0]['configuration']
    if runtime['warm_p95_ms'] is not None and runtime['warm_p95_ms'] > (5000 if configured['protocol'] == 'local_decider' else configured['timeout_secs'] * 1000):
        gates.append('warm_latency_budget_failed')
    report = {'schema_version': 1, 'protocol_id': frozen['protocol_id'], 'freeze_sha256': frozen['freeze_sha256'],
        'qualification': not gates, 'production_admission': False, 'use_case': 'manual_finished_suggestions', 'language': 'nl',
        'raw_component_metrics': raw, 'calibrated_component_metrics': calibrated, 'complete_item_metrics': items,
        'resources': runtime, 'gates': gates, 'candidate_records': [],
        'original_job_deadlines': {'calibration_verified': all(job['deadline_verified'] for job in calibration_jobs.values()),
                                   'heldout_verified': all(job['deadline_verified'] for job in jobs.values())},
        'natural_asr_acceptance': False, 'human_acceptance': False, 'automatic_analysis_accepted': False}
    if _v2(frozen):
        report.update(runtime_policy=RUNTIME_POLICY, runtime_calibration_applied=False, calibration_role='diagnostic_evidence_only', remote_weights_independently_verified=False)
    if not gates:
        exported = frozen['production']['exports'][0]
        context = {**exported['qualification_request'], 'actual': exported['expected_actual'],
                   'calibration_sha256': finished.digest(frozen['calibration'])}
        report['candidate_records'] = [{'schema_version': 1, 'qualification_id': 'finished-nl-' + frozen['freeze_sha256'][:20],
            'context': context,
            'evidence': {'mode': 'recorded_inference', 'protocol_sha256': frozen['protocol_sha256'],
                'corpus_sha256': finished.digest(frozen['production']['exports']), 'split_sha256': frozen['production']['split_sha256'],
                'report_sha256': finished.digest(report), 'prefix_isolation': True, 'heldout_disjoint': True,
                'source_judgments_complete': True, 'failures_counted': True, 'resource_measurements_complete': True,
                'foreground_impact_accepted': False, 'background_resource_accepted': False},
            'metrics': {'macro_f1': min(raw['macro_f1'], calibrated['macro_f1'], items['classification']['macro_f1']),
                'accepted_item_precision': items['precision'], 'correct_owner_and_final_status': True,
                'selective_precision': None, 'coverage': raw['coverage'], 'warm_p95_ms': runtime['warm_p95_ms'],
                'peak_rss_bytes': runtime['peak_rss_bytes'], 'cpu_time_ms': runtime['cpu_time_ms'],
                'calibration_observations': min(fit['sample_count'] for fit in fits),
                'calibration_classes': min(fit['observed_class_count'] for fit in fits),
                'calibration_meeting_groups': min(fit['meeting_group_count'] for fit in fits), 'heldout_observations': len(rows)}}]
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('freeze', 'requests', 'calibrate', 'evaluate'))
    parser.add_argument('--bridge', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--input', required=True)
    parser.add_argument('--predictions')
    parser.add_argument('--resources')
    parser.add_argument('--split', choices=('development', 'calibration', 'heldout'))
    args = parser.parse_args()
    bridge = finished.CoreBridge(args.bridge)
    value = json.loads(Path(args.input).read_text())
    if args.operation == 'freeze':
        result = freeze(value['fixtures'], value['configuration'], bridge, value['expected_actual'], value['expected_answers'], value['gold_items'])
    elif args.operation == 'requests': result = requests(value, args.split, bridge)
    else:
        predictions = json.loads(Path(args.predictions).read_text())
        result = calibrate(value, predictions, bridge) if args.operation == 'calibrate' else evaluate(value, predictions, bridge,
            json.loads(Path(args.resources).read_text()) if args.resources else None)
    output = Path(args.output)
    if output.exists(): raise ValueError('preserve_existing_qualification_artifact')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False) + '\n')
    print(json.dumps({'operation': args.operation, 'qualification': result.get('qualification', False) if isinstance(result, dict) else False,
                      'production_admission': False}))


if __name__ == '__main__':
    main()
