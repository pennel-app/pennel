"""Opt-in managed-local qualification through the shipped Rust Engine and native worker.

No HTTP, credentials, downloads, provider fallbacks, retries or registry writes.
The Rust Engine and loaded worker remain alive across warm original jobs.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

import finished
import finished_corpus_managed_v1 as corpus
import finished_experiment as common
import finished_qualification as qualification
import runners
from evaluation_io import jsonl_exchange, stop_owned_group

MODEL = 'pennel-decider-flash-v1.2-q8_0'
ARTIFACT = 'ffb27ade8976765b0e6943a0b7f5f3fd0f7042d45ad90256a76d608bcd3942c2'
ENDPOINT = 'pennel-local://decider/decisions'


def route(exported):
    config = exported['configuration']
    if (config != {'protocol': 'local_decider', 'endpoint': ENDPOINT, 'model': MODEL,
            'expected_model': 'sha256:'+ARTIFACT, 'upstream': None, 'max_input_bytes': 65536, 'timeout_secs': 90}
            or not qualification._valid_actual(config, exported.get('expected_actual'))
            or exported.get('runtime_policy') != 'raw_supported_passage_decisions_v3'):
        raise ValueError('frozen_managed_local_route_required')
    return config


def authored_spec(path, frozen):
    raw = Path(path).read_bytes()
    spec = json.loads(raw)
    canonical = corpus.experiment()
    if any(spec.get(key) != canonical[key] for key in ('fixtures', 'expected_answers', 'gold_items')):
        raise ValueError('canonical_fresh_managed_corpus_required')
    if frozen.get('resource_fixture_id') != canonical['resource_fixture_id']:
        raise ValueError('canonical_managed_resource_panel_required')
    by_id = {fixture['id']: fixture for fixture in spec['fixtures']}
    exports = frozen['production']['exports']
    if len(by_id) != len(exports) or spec['expected_answers'] != frozen['production']['expected_answers']:
        raise ValueError('authored_local_corpus_changed')
    for exported in exports:
        identifier = exported['fixture']['id']
        if not identifier.startswith('nl-managed-v1-') or by_id.get(identifier) != exported['fixture']:
            raise ValueError('fresh_authored_local_fixture_required')
        if qualification._gold(exported, spec['gold_items'][identifier]) != frozen['gold_items'][identifier]:
            raise ValueError('authored_local_judgment_changed')
    old_held = {row['text'].strip() for fixture in common.corpus_v3.experiment()['fixtures']
                if fixture['split'] == 'heldout' for row in fixture['utterances']}
    if any(row['text'].strip() in old_held for fixture in spec['fixtures']
           if fixture['split'] in ('calibration', 'heldout') for row in fixture['utterances']):
        raise ValueError('old_heldout_text_is_not_fresh_local_evidence')
    return hashlib.sha256(raw).hexdigest()


class LocalClient:
    def __init__(self, binary, bundle, worker, model):
        self.created_at = time.monotonic()
        self.executed_jobs = 0
        self.binary = str(Path(binary).resolve(strict=True))
        self.sha256 = finished.file_hash(self.binary)
        environment = {key: value for key, value in os.environ.items()
                       if key in ('PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL')}
        self.process = subprocess.Popen([self.binary, '--execute-local-review',
            '--runtime-bundle', str(Path(bundle).resolve(strict=True)),
            '--worker', str(Path(worker).resolve(strict=True)), '--model', str(Path(model).resolve(strict=True))],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=environment, start_new_session=True)
        self.buffer = bytearray()
        self.resources = None
        self.group_stopped = False

    def stop(self):
        if not self.group_stopped:
            self.group_stopped = True
            stop_owned_group(self.process)

    def call(self, value, deadline):
        try:
            if finished.file_hash(self.binary) != self.sha256:
                raise ValueError('bridge_binary_changed')
            result = jsonl_exchange(self.process, value, self.buffer, deadline,
                prefix='local_client', max_response_bytes=finished.MAX_RESPONSE_BYTES*8)
            if not isinstance(result, dict):
                raise ValueError('local_client_invalid_output')
            if 'error' in result:
                raise ValueError(result['error'])
            return result
        except BaseException:
            self.stop()
            raise

    def finish(self, deadline):
        value = self.call({'op': 'finished_local_review_finish'}, deadline)
        if value.get('native_worker_unloaded_and_reaped') is not True or value.get('finished') is not True:
            raise ValueError('local_worker_reap_unconfirmed')
        self.resources = value.get('resources')
        return value

    def close(self):
        if not self.process.stdin.closed:
            try: self.process.stdin.close()
            except BrokenPipeError: pass
        try: self.process.wait(timeout=5)
        except subprocess.TimeoutExpired: self.stop()
        finally: self.stop()
        self.process.stdout.close()

    def __enter__(self): return self
    def __exit__(self, *_): self.close()


def validate_attempt(exported, batch, entry):
    attempts = entry.get('attempts')
    if not isinstance(attempts, list) or len(attempts) != 1:
        raise ValueError('exactly_one_actual_Engine_attempt_required')
    attempt = attempts[0]
    body = attempt.get('request_body')
    if (not isinstance(body, str) or json.loads(body) != batch['request']
            or attempt.get('request') != batch['request']
            or hashlib.sha256(body.encode()).hexdigest() != attempt.get('request_sha256')
            or attempt.get('request_capture') != 'validated_Engine_request_before_managed_local_scheduler'
            or attempt.get('response_capture') != 'validated_production_DecisionResult'
            or attempt.get('raw_worker_io_available') is not False):
        raise ValueError('actual_managed_local_request_changed')
    result = attempt.get('result')
    if not isinstance(result, dict) or result != entry.get('outcome', {}).get('result'):
        raise ValueError('production_local_result_missing')
    identity = result.get('identity', {})
    expected = exported['expected_actual']
    if (identity.get('protocol') != 'local_decider' or identity.get('endpoint') != ENDPOINT
            or identity.get('requested_model') != MODEL or identity.get('requested_upstream') is not None
            or identity.get('actual_model') != expected['model'] or identity.get('actual_revision') != expected['revision']
            or identity.get('actual_provider') != expected['provider']):
        raise ValueError('actual_local_serving_identity_changed')
    try: identifier = uuid.UUID(identity['request_id'])
    except (ValueError, TypeError, KeyError): raise ValueError('actual_local_request_id_missing') from None
    if identifier.int == 0 or str(identifier) != identity['request_id']:
        raise ValueError('actual_local_request_id_missing')
    if result.get('usage', {}).get('billed_cost_usd') != 0:
        raise ValueError('managed_local_zero_cost_required')
    native = attempt.get('native_execution', {})
    if (type(native.get('native_decode_calls')) is not int or native['native_decode_calls'] < 1
            or native.get('independent_question_sequences') != len(batch['request']['questions'])
            or native.get('expanded_input_tokens') != result.get('usage', {}).get('input_tokens')):
        raise ValueError('actual_local_native_execution_missing')
    return attempt


def run_job(exported, freeze_sha256, bridge, client, output, *, execute=False, global_deadline=None):
    if not execute: raise ValueError('execution_not_enabled')
    config = route(exported)
    output = Path(output)
    if output.exists(): raise ValueError('preserve_existing_execution_artifact')
    started = getattr(client, 'created_at', time.monotonic()) if getattr(client, 'executed_jobs', 0) == 0 else time.monotonic()
    client.executed_jobs = getattr(client, 'executed_jobs', 0)+1
    deadline = started+config['timeout_secs']
    if global_deadline is not None: deadline = min(deadline, global_deadline)
    run = {'fixture_id': exported['fixture']['id'], 'split': exported['fixture']['split'],
        'freeze_sha256': freeze_sha256, 'export_sha256': exported['freeze_sha256'],
        'evidence_class': 'recorded_inference', 'requests': [], 'request_sha256s': [],
        'actual_Engine_request_sha256s': [], 'responses': [], 'status': 'blocked', 'reason': None,
        'response_capture': 'validated_production_DecisionResult', 'raw_worker_io_available': False,
        'transport_lifecycle': 'retained_Rust_Engine_and_loaded_owned_native_worker_across_original_jobs',
        'provider_cost_usd': '0', 'spend_ledger': runners.SpendLedger().snapshot()}
    reason = None
    run_id = str(uuid.uuid4())
    try:
        finished._verify_export(exported, bridge, deadline=deadline)
        for batch in qualification.physical_requests(exported):
            value = {'op': 'finished_local_review_batch', 'run_id': run_id, 'batch_index': batch['index']}
            value.update({'export': exported} if batch['index'] == 0 else {'export_sha256': exported['freeze_sha256']})
            entry = client.call(value, deadline)
            run['requests'].append(entry)
            common.write_json(output, run)
            if entry.get('outcome', {}).get('status') != 'ok':
                reason = entry.get('outcome', {}).get('status', 'local_request_failed')
                break
            attempt = validate_attempt(exported, batch, entry)
            run['request_sha256s'].append(hashlib.sha256(batch['wire_body'].encode()).hexdigest())
            run['actual_Engine_request_sha256s'].append(attempt['request_sha256'])
            # This is a native production result, with no invented HTTP status/body.
            run['responses'].append({'batch_index': batch['index'], 'production_result': attempt['result']})
            if entry['outcome'].get('replay', {}).get('status') != 'ok':
                reason = entry['outcome'].get('replay', {}).get('status', 'invalid_replay')
                break
        if reason is None and time.monotonic() >= deadline: reason = 'job_deadline_exceeded'
    except Exception:
        reason = 'local_execution_failed'
    run.update(status='ok' if reason is None else 'blocked', reason=reason)
    attempts = [attempt for entry in run['requests'] for attempt in entry.get('attempts', [])]
    native = [attempt.get('native_execution', {}) for attempt in attempts]
    complete_native = all(type(value.get('native_decode_calls')) is int for value in native)
    run['measurements'] = {'whole_job_elapsed_ms': (time.monotonic()-started)*1000,
        'executed_requests': len(attempts),
        'executed_native_decode_calls': sum(value['native_decode_calls'] for value in native) if complete_native else None,
        'input_bytes': sum(len(value.get('request_body', '').encode()) for value in attempts),
        'cpu_rss_scope': 'phase_finish_after_retained_native_worker_unload_and_reap'}
    common.write_json(output, run)
    return run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('preflight', 'development', 'calibration', 'heldout', 'resources'))
    for name in ('bridge', 'runtime-bundle', 'worker', 'model', 'output'):
        parser.add_argument('--'+name, required=True)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--deadline-unix', type=float, required=True)
    parser.add_argument('--freeze'); parser.add_argument('--authored-spec')
    parser.add_argument('--fixture-id'); parser.add_argument('--freeze-lock'); parser.add_argument('--freeze-commit')
    args = parser.parse_args()
    if not args.execute: raise ValueError('execution_not_enabled')
    remaining = args.deadline_unix-time.time()
    if not 0 < remaining <= 7200: raise ValueError('bounded_existing_local_deadline_required')
    deadline = time.monotonic()+remaining
    output = Path(args.output)
    if output.exists(): raise ValueError('preserve_existing_execution_artifact')
    output.mkdir(parents=True)
    bridge = finished.CoreBridge(args.bridge)
    factory = lambda: LocalClient(args.bridge, args.runtime_bundle, args.worker, args.model)
    if args.phase == 'preflight':
        with factory() as client:
            result = client.call({'op': 'local_review_preflight'}, min(deadline, time.monotonic()+30))
            result['finish'] = client.finish(min(deadline, time.monotonic()+15))
        common.write_json(output/'preflight.json', result)
        return
    frozen = json.loads(Path(args.freeze).read_text())
    qualification.verify_freeze(frozen, bridge)
    if not qualification._local(frozen): raise ValueError('local_qualification_freeze_required')
    spec_sha = authored_spec(args.authored_spec, frozen)
    if args.phase != 'development':
        common._verify_committed_lock(args.freeze_commit, args.freeze_lock, frozen)
        relative = Path(args.freeze_lock).resolve().relative_to(common.ROOT)
        lock = json.loads(subprocess.check_output(['git', 'show', args.freeze_commit+':'+str(relative)], cwd=common.ROOT))
        if lock.get('authored_spec_sha256') != spec_sha: raise ValueError('committed_authored_local_corpus_required')
    if args.phase in ('heldout', 'resources') and frozen['stage'] != 'heldout_ready':
        raise ValueError('calibration_before_heldout_required')
    exports = frozen['production']['exports']
    for exported in exports: route(exported)
    if args.phase == 'development':
        selected = [value for value in exports if value['fixture']['id'] == args.fixture_id and value['fixture']['split'] == 'development']
        if len(selected) != 1: raise ValueError('explicit_frozen_development_fixture_required')
        jobs = [(None, None, selected[0])]
    elif args.phase == 'resources':
        panel = next(value for value in exports if value['fixture']['id'] == frozen['resource_fixture_id'])
        jobs = [(f'cold-{index}', 'cold', panel) for index in range(5)]+[(f'warm-{index}', 'warm', panel) for index in range(30)]
    else: jobs = [(None, None, value) for value in exports if value['fixture']['split'] == args.phase]
    runs, samples, resource_receipts = [], [], []
    retained = None
    try:
        for key, phase, exported in jobs:
            client = factory() if phase == 'cold' else retained
            if client is None: retained = client = factory()
            run = run_job(exported, frozen['freeze_sha256'], bridge, client,
                output/(str(key or exported['fixture']['id'])+'.json'), execute=True, global_deadline=deadline)
            runs.append(run)
            if key: samples.append({'id': key, 'phase': phase, 'run': run})
            if phase == 'cold':
                try: resource_receipts.append(client.finish(min(deadline, time.monotonic()+15)))
                finally: client.close()
            if run['status'] != 'ok': break
    finally:
        if retained:
            try: resource_receipts.append(retained.finish(min(deadline, time.monotonic()+15)))
            finally: retained.close()
    common.write_json(output/'native-resource-receipts.json', resource_receipts)
    if args.phase == 'resources':
        measurements = [value['resources'] for value in resource_receipts if isinstance(value.get('resources'), dict)]
        value = {'freeze_sha256': frozen['freeze_sha256'],
            'resource_profile_sha256': jobs[0][2]['qualification_request']['resource_profile_sha256'],
            'scope': 'owned_client_and_local_model', 'samples': samples,
            'cpu_time_ms': sum(value['cpu_time_ms'] for value in measurements),
            'peak_rss_bytes': max((value['peak_rss_bytes'] for value in measurements), default=0),
            'native_receipts': resource_receipts}
        common.write_json(output/'resources.json', value)
    else: common.write_json(output/'predictions.json', runs)
    complete = len(runs) == len(jobs) and all(value['status'] == 'ok' for value in runs)
    common.write_json(output/'phase-receipt.json', {'phase': args.phase, 'complete': complete,
        'scheduled_jobs': len(jobs), 'executed_jobs': len(runs), 'freeze_sha256': frozen['freeze_sha256'],
        'authored_spec_sha256': spec_sha, 'deadline_unix': args.deadline_unix,
        'automatic_retries': False, 'fallbacks': False, 'provider_cost_usd': '0', 'production_admission': False})
    print(json.dumps({'phase': args.phase, 'complete': complete, 'executed_jobs': len(runs), 'scheduled_jobs': len(jobs)}))
    if not complete: raise SystemExit(1)


if __name__ == '__main__': main()
