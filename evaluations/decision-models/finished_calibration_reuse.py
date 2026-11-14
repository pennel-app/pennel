"""Verified reuse of the committed raw Cloud calibration, without new inference."""

import copy
import hashlib
import json
import math
from pathlib import Path
import subprocess

import finished
import calibration
import metrics

ROOT = Path(__file__).resolve().parents[2]
RUNTIME_POLICY = 'calibrated_support_raw_state_v1'
ASSET = ROOT/'assets/qualifications/meeting-support-cloud-v1.json'
SOURCE = 'build/review-cloud-local/cloud-qualification-v3-defaults/heldout-ready.json'
SOURCE_MANIFEST = '6a678edd97a67e2bae84734ffbffc72f4e938fdd2652c5059cf7d8193375ddd9'
FITTED_FREEZE = 'ebee7a9eceb6478390998984e84df25417c115abaf5c33fc767b71d1b98b63b7'
FITTED_LOCK_COMMIT = '0d4dcfa69c07b84769729e4dee97c42a83fe7436'
FITTED_LOCK = 'evaluations/decision-models/locks/review-pennel-cloud-nl-v1-2026-11-13-fitted.json'
FITTED_LOCK_SHA256 = 'ffb09545cbc373fedf571cae928f5d5d29ea2d1b7d98e85bebeb3699698590a0'
ARTIFACT_SHA256 = 'd7c5d03f73edfafcbeb15c8602196025ffa19ec033298282670eea469d8b51ee'
RUNS_SHA256 = 'd63ea44267fc215c801967268e94d6debd7ed9b8ee5b977a6a41d039a1726c35'
HISTORICAL_BRIDGE = 'build/review-cloud-local/production-review-eval-bridge-v2/pennel-eval'
HISTORICAL_BRIDGE_SHA256 = 'd38bdb9005c69a5f49fa4c364d6218974f8fb4243ac3c3f92ae4cdc239255800'


def _sealed(value):
    return value.get('freeze_sha256') == finished.digest({k:v for k,v in value.items() if k != 'freeze_sha256'})


def asset():
    raw = ASSET.read_bytes()
    value = json.loads(raw)
    provenance = {'source_manifest_sha256': SOURCE_MANIFEST, 'fitted_freeze_sha256': FITTED_FREEZE,
        'fitted_lock_commit': FITTED_LOCK_COMMIT, 'fitted_lock_sha256': FITTED_LOCK_SHA256,
        'calibration_artifact_sha256': ARTIFACT_SHA256, 'calibration_runs_sha256': RUNS_SHA256}
    if (value.get('runtime_policy') != RUNTIME_POLICY or value.get('schema_version') != 1
            or value.get('qualification') is not False or value.get('production_admission') is not False
            or value.get('provenance') != provenance
            or hashlib.sha256(value.get('calibration_json', '').encode()).hexdigest() != ARTIFACT_SHA256):
        raise ValueError('unlinked_runtime_support_calibration')
    calibration = json.loads(value['calibration_json'])
    if calibration.get('run_manifest_hash') != SOURCE_MANIFEST:
        raise ValueError('original_calibration_manifest_changed')
    return value, hashlib.sha256(raw).hexdigest(), calibration


def metadata(exported):
    value, sha, _ = asset()
    binding = value['binding']
    expected = {'runtime_policy': RUNTIME_POLICY, 'asset_sha256': sha,
        'artifact_sha256': ARTIFACT_SHA256, 'source_manifest_sha256': SOURCE_MANIFEST,
        'fitted_lock_sha256': FITTED_LOCK_SHA256, 'fitted_lock_commit': FITTED_LOCK_COMMIT,
        'fitted_freeze_sha256': FITTED_FREEZE, 'calibration_runs_sha256': RUNS_SHA256,
        'run_manifest_hash': SOURCE_MANIFEST, 'configured_identity': binding['configured'],
        'actual_identity': binding['actual']}
    request = exported['qualification_request']
    if (exported.get('support_calibration') != expected or exported['configuration'] != binding['configured']
            or exported['expected_actual'] != binding['actual']
            or request.get('prompt_sha256') != binding['input_prompt_sha256']
            or request.get('label_order_sha256') != binding['input_label_order_sha256']
            or request.get('grouping') != binding['grouping']
            or request.get('language_policy') != binding['language_policy']):
        raise ValueError('runtime_support_calibration_binding_changed')
    return expected


def input_projection(exported):
    # These are the only fields changed by the new local postprocessing. Everything
    # else, including unknown future fields, belongs to the equality check.
    value = {k:copy.deepcopy(v) for k,v in exported.items() if k not in
        ('freeze_sha256', 'source_sha256', 'runtime_policy', 'support_calibration')}
    for key in ('schema_sha256', 'preprocessing_sha256'):
        value.get('qualification_request', {}).pop(key, None)
    return value


def load_source(path=SOURCE):
    location = (ROOT/path).resolve()
    location.relative_to(ROOT)
    source = json.loads(location.read_text())
    raw_lock = subprocess.check_output(['git', 'show', FITTED_LOCK_COMMIT+':'+FITTED_LOCK], cwd=ROOT)
    lock = json.loads(raw_lock)
    if (hashlib.sha256(raw_lock).hexdigest() != FITTED_LOCK_SHA256
            or source.get('freeze_sha256') != FITTED_FREEZE or not _sealed(source)
            or source.get('base_freeze_sha256') != SOURCE_MANIFEST
            or source.get('runtime_policy') != 'raw_supported_passage_decisions_v3'
            or source.get('protocol_id') != 'production-finished-qualification-cloud-v1'
            or source.get('stage') != 'heldout_ready' or source.get('production_admission') is not False
            or source.get('source_sha256') != lock.get('python_source_sha256')
            or source['production'].get('freeze_sha256') != lock.get('production_freeze_sha256')
            or not _sealed(source['production'])
            or lock.get('freeze_sha256') != FITTED_FREEZE or lock.get('base_freeze_sha256') != SOURCE_MANIFEST
            or lock.get('calibration_artifact_sha256') != ARTIFACT_SHA256
            or lock.get('calibration_runs_sha256') != RUNS_SHA256
            or finished.digest(source.get('calibration')) != ARTIFACT_SHA256
            or finished.digest(source.get('calibration_runs')) != RUNS_SHA256
            or source['calibration'].get('run_manifest_hash') != SOURCE_MANIFEST):
        raise ValueError('committed_original_calibration_required')
    # Check the old base seal without reinterpreting it with today's source code.
    base = {k:v for k,v in source.items() if k not in ('calibration', 'calibration_runs',
        'calibration_runs_sha256', 'base_freeze_sha256', 'freeze_sha256')}
    base['stage'] = 'calibration'
    if finished.digest(base) != SOURCE_MANIFEST:
        raise ValueError('original_calibration_manifest_changed')
    return source


def equivalence(frozen, source):
    current = {x['fixture']['id']:x for x in frozen['production']['exports']
               if x['fixture']['split'] == 'calibration'}
    previous = {x['fixture']['id']:x for x in source['production']['exports']
                if x['fixture']['split'] == 'calibration'}
    if not current or set(current) != set(previous):
        raise ValueError('original_calibration_fixtures_required')
    projections = []
    for identifier, exported in current.items():
        old = previous[identifier]
        if (input_projection(exported) != input_projection(old)
                or frozen['production']['expected_answers'][identifier] != source['production']['expected_answers'][identifier]
                or frozen['gold_items'][identifier] != source['gold_items'][identifier]):
            raise ValueError('calibration_model_input_or_judgment_changed')
        projections.append({'fixture_id':identifier, 'input_sha256':finished.digest(input_projection(exported)),
            'answers_sha256':finished.digest(frozen['production']['expected_answers'][identifier]),
            'gold_sha256':finished.digest(frozen['gold_items'][identifier])})
    seen_ids = {x['fixture']['id'] for x in source['production']['exports']}
    seen_families = {x['fixture']['family_id'] for x in source['production']['exports']}
    seen_text = {row['text'].strip() for x in source['production']['exports'] for row in x['fixture']['utterances']}
    held = [x['fixture'] for x in frozen['production']['exports'] if x['fixture']['split'] == 'heldout']
    if any(x['id'] in seen_ids or x['family_id'] in seen_families
           or any(row['text'].strip() in seen_text for row in x['utterances']) for x in held):
        raise ValueError('fresh_heldout_after_runtime_change_required')
    return finished.digest(sorted(projections, key=lambda x:x['fixture_id']))


def proof(frozen, source, path=SOURCE):
    runs = source['calibration_runs']
    if len(runs) != 96 or sum(len(x.get('responses', [])) for x in runs) != 180:
        raise ValueError('original_calibration_execution_changed')
    return {'mode':'verified_recorded_calibration_reuse', 'source_artifact':str(path),
        'source_manifest_sha256':SOURCE_MANIFEST, 'fitted_freeze_sha256':FITTED_FREEZE,
        'fitted_lock_commit':FITTED_LOCK_COMMIT, 'fitted_lock_path':FITTED_LOCK,
        'fitted_lock_sha256':FITTED_LOCK_SHA256, 'artifact_sha256':ARTIFACT_SHA256,
        'calibration_runs_sha256':RUNS_SHA256, 'run_manifest_hash':SOURCE_MANIFEST,
        'model_input_equivalence_sha256':equivalence(frozen, source),
        'historical_bridge':HISTORICAL_BRIDGE, 'historical_bridge_sha256':HISTORICAL_BRIDGE_SHA256,
        'replay_policy':'actual_engine_typed_result_v1',
        'recorded_engine_results_sha256':finished.digest([{'fixture_id':run['fixture_id'],
            'batch_index':request['batch_index'], 'production_result':request['outcome']['result']}
            for run in runs for request in run['requests']]),
        'original_recorded_jobs':96, 'original_recorded_requests':180,
        'new_calibration_requests':0, 'counts_as_new_inference':False}


def historical_bridge(source):
    bridge = finished.CoreBridge(ROOT/HISTORICAL_BRIDGE)
    if bridge.artifact_sha256 != HISTORICAL_BRIDGE_SHA256 or source['production']['bridge_sha256'] != bridge.artifact_sha256:
        raise ValueError('original_calibration_bridge_changed')
    for name in ('calibration.py', 'metrics.py'):
        if finished.file_hash(Path(__file__).with_name(name)) != source['source_sha256'][name]:
            raise ValueError('original_calibration_fitting_code_changed')
    return bridge


def typed_responses(run, *, reused=False):
    requests = {x.get('batch_index'):x for x in run.get('requests', [])}
    result = []
    if len(requests) != len(run.get('requests', [])):
        raise ValueError('recorded_engine_result_required')
    for response in run['responses']:
        index = response.get('batch_index')
        outcome = requests.get(index, {}).get('outcome', {}).get('result')
        if (not isinstance(outcome, dict) or set(outcome) != {'answers', 'identity', 'usage'}
                or not reused and response.get('production_result') != outcome):
            raise ValueError('recorded_engine_result_required')
        result.append({**copy.deepcopy(response), 'production_result':copy.deepcopy(outcome)})
    return result


def verify_engine_replay(exported, responses, replay):
    physical = {x['batch_index']:x['production_result'] for x in responses}
    for window in replay.get('windows', []):
        if window.get('status') != 'ok': continue
        index = window['window_index']
        if not isinstance(index, int) or isinstance(index, bool) or not 0 <= index < len(exported['windows']):
            raise ValueError('actual_engine_replay_changed')
        batch = exported['batches'][exported['windows'][index]['batch_index']]
        original = physical.get(batch['index'])
        if original is None: raise ValueError('actual_engine_replay_changed')
        mappings = [x for x in batch['answer_map'] if x['window_index'] == index]
        expected = {k:copy.deepcopy(v) for k,v in original.items() if k != 'answers'}
        if len(batch['window_indices']) > 1:
            # Production deliberately keeps billing on the physical batch.
            expected['usage'] = {'input_tokens':None, 'output_tokens':None, 'billed_cost_usd':None}
        original_answers = {x['id']:x for x in original['answers']}
        expected['answers'] = [{**original_answers[x['batch_id']], 'id':x['original_id']} for x in mappings]
        if window.get('result') != expected:
            raise ValueError('actual_engine_replay_changed')


def serialization_delta(original_rows, engine_rows):
    before = {x['id']:x for x in original_rows}
    after = {x['id']:x for x in engine_rows}
    if set(before) != set(after): raise ValueError('calibration_raw_engine_context_changed')
    changed, maximum = 0, 0.0
    for identifier, row in before.items():
        actual = after[identifier]
        if ({k:v for k,v in row.items() if k != 'probabilities'}
                != {k:v for k,v in actual.items() if k != 'probabilities'}):
            raise ValueError('calibration_raw_engine_context_changed')
        first, second = row['probabilities'], actual['probabilities']
        if first is None or second is None:
            if first != second: raise ValueError('calibration_raw_engine_context_changed')
            continue
        if set(first) != set(second): raise ValueError('calibration_raw_engine_context_changed')
        for label, probability in first.items():
            delta = abs(probability-second[label])
            changed += delta != 0
            maximum = max(maximum, delta)
    # An additional fail-closed bound on serialization drift, not a quality
    # tolerance: each new raw value must already equal its recorded Engine value.
    if maximum > 1e-12: raise ValueError('calibration_serialization_delta_not_rounding')
    return {'rows':len(before), 'choices_states_abstentions_and_contexts_unchanged':True,
        'changed_probability_values':changed, 'max_probability_absolute_delta':maximum,
        'original_raw_replay_rows_sha256':finished.digest(original_rows),
        'recorded_engine_rows_sha256':finished.digest(engine_rows),
        'original_fit_and_thresholds_unchanged':True, 'counts_as_new_inference':False}


def verify_original_execution(source):
    # Validate the actual retained UUID-bearing request as well as its frozen
    # template. No provider, credentials or network operations are involved.
    import finished_cloud_experiment as cloud
    exports = {x['fixture']['id']:x for x in source['production']['exports']
               if x['fixture']['split'] == 'calibration'}
    seen, request_ids = set(), set()
    for run in source['calibration_runs']:
        identifier = run.get('fixture_id')
        if identifier not in exports or identifier in seen or run.get('evidence_class') != 'recorded_inference':
            raise ValueError('original_recorded_calibration_execution_required')
        seen.add(identifier)
        exported = exports[identifier]
        batches, requests, responses = exported['batches'], run.get('requests', []), run.get('responses', [])
        if len(requests) != len(batches) or len(responses) != len(batches):
            raise ValueError('original_recorded_calibration_execution_required')
        actual_hashes = []
        for batch, request, response in zip(batches, requests, responses):
            attempts = request.get('attempts', [])
            if (len(attempts) != 1 or request.get('batch_index') != batch['index']
                    or request.get('template_request_sha256') != batch['wire_body_sha256']
                    or request.get('outcome', {}).get('status') != 'ok'
                    or request.get('outcome', {}).get('replay', {}).get('status') != 'ok'):
                raise ValueError('original_recorded_calibration_execution_required')
            attempt = attempts[0]
            request_id = cloud.validate_attempt(exported, batch, attempt)
            if (request_id in request_ids or request.get('request_id') != request_id
                    or response.get('request_id') != request_id or attempt.get('http_status') != 200
                    or response.get('http_status') != 200 or response.get('batch_index') != batch['index']
                    or request.get('raw_response') != response.get('body')
                    or json.loads(response['body']) != attempt.get('response')):
                raise ValueError('original_recorded_calibration_execution_required')
            request_ids.add(request_id)
            actual_hashes.append(attempt['wire_body_sha256'])
        if run.get('actual_request_sha256s') != actual_hashes:
            raise ValueError('original_recorded_calibration_execution_required')
    if seen != set(exports):
        raise ValueError('original_recorded_calibration_execution_required')


def verify_gate_window(exported, window, replay, artifact):
    """Check Rust gate evidence against the immutable fit; never alter predictions."""
    decisions = replay.get('support_gate')
    required = {kind+'_support' for kind in ('action', 'decision', 'open_question', 'changed_commitment')}
    if (not isinstance(decisions, list) or len(decisions) != 4
            or {x.get('question_id') for x in decisions} != required):
        raise ValueError('runtime_support_gate_evidence_required')
    by_id = {x['question_id']:x for x in decisions}
    answers = {x['id']:x for x in replay.get('result', {}).get('answers', [])}
    accepted, supported = set(), set()
    for question in window['request']['questions']:
        key = question['id']
        if key not in required:
            continue
        answer = answers.get(key, {})
        row = {'task':'events', 'subtask':key, 'language':'nl', 'runner_id':'production-finished',
            'label_order':[option['id'] for option in question['options']],
            'prompt_hash':finished.digest({'question':question, 'physical_prompt_context_sha256':window['prompt_context_sha256']}),
            'grouping':exported['qualification_request']['grouping'],
            'identity':{'transport':exported['configuration']['protocol'], **exported['expected_actual']}}
        context = calibration.prediction_context(row)
        context_hash = metrics._context_hash(context)
        fitted = artifact.get('fits', {}).get(context_hash)
        if not fitted or fitted.get('status') != 'fitted':
            raise ValueError('uncalibrated_runtime_support_context')
        probabilities = {x['option_id']:x['probability'] for x in answer.get('probabilities', [])}
        distribution = metrics.apply_calibration(probabilities, fitted['fit'], context)
        chosen = max(sorted(distribution), key=distribution.get)
        threshold = fitted['fit']['threshold']['threshold']
        raw = answer.get('choice')
        reason = ('model_abstained' if answer.get('abstained') is not False else
            'raw_unclear' if raw == 'unclear' else 'choice_not_argmax' if raw != chosen else
            'accepted' if threshold is not None and distribution[chosen] >= threshold else 'below_frozen_threshold')
        decision = by_id[key]
        if (decision.get('context_sha256') != context_hash or decision.get('raw_choice') != raw
                or decision.get('calibrated_choice') != chosen or decision.get('threshold') != threshold
                or not isinstance(decision.get('calibrated_probability'), (int, float))
                or isinstance(decision.get('calibrated_probability'), bool)
                or not math.isclose(decision['calibrated_probability'], distribution[chosen], rel_tol=1e-10, abs_tol=1e-12)
                or decision.get('accepted') is not (reason == 'accepted') or decision.get('reason') != reason):
            raise ValueError('runtime_support_gate_evidence_changed')
        if reason == 'accepted':
            accepted.add(key.removesuffix('_support'))
            if raw == 'supported': supported.add(key.removesuffix('_support'))
    if (not set(replay.get('assessed_kinds', [])).issubset(accepted)
            or any(item.get('kind') not in supported for item in replay.get('candidates', []))):
        raise ValueError('runtime_support_gate_scope_changed')


def verified_source(frozen):
    recorded = frozen.get('calibration_reuse', {})
    source = load_source(recorded.get('source_artifact', SOURCE))
    if (recorded != proof(frozen, source, recorded.get('source_artifact', SOURCE))
            or frozen.get('calibration') != source['calibration']
            or frozen.get('calibration_runs') != source['calibration_runs']):
        raise ValueError('calibration_reuse_provenance_changed')
    verify_original_execution(source)
    return source
