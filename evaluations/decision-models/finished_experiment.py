"""Explicit hosted execution of frozen invented Dutch production batches.

The cumulative ledger is reserved before transmission. This module never changes
model/provider routes, retries uncertain requests, or adopts qualification records.
"""

from __future__ import annotations

import argparse
import base64
from contextlib import contextmanager
from decimal import Decimal, InvalidOperation, ROUND_CEILING
import fcntl
import hashlib
import json
import os
from pathlib import Path
import resource
import selectors
import re
import subprocess
import sys
import stat
import time
import urllib.error
import urllib.request

import finished
import finished_corpus_v2 as corpus
import finished_corpus_v3 as corpus_v3
import finished_qualification as qualification
import runners

ROOT = Path(__file__).resolve().parents[2]
ENDPOINT = 'https://openrouter.ai/api/alpha/decisions'
MODEL = 'perplexity/pplx-decider-v1.2'
ACTUAL_MODEL = 'perplexity/pplx-decider-v1.2-20261016'
DEFAULT_LEDGER = ROOT / 'build/review-qualification-96/20261111/spend-ledger.json'
MAX_SECONDS = 7200
MAX_BYTES = finished.MAX_RESPONSE_BYTES
NATIVE_SMOKE_ALLOWANCE = Decimal('.20')


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.writing')
    with temporary.open('w') as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2, allow_nan=False)
        handle.write('\n');handle.flush();os.fsync(handle.fileno())
    temporary.replace(path)
    directory = os.open(path.parent, os.O_RDONLY)
    try: os.fsync(directory)
    finally: os.close(directory)


@contextmanager
def locked_ledger(path):
    path = Path(path)
    with path.with_name(path.name + '.lock').open('a') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        ledger = load_ledger(path)
        yield ledger


def load_ledger(path):
    path = Path(path)
    if not path.is_file(): raise ValueError('existing_cumulative_ledger_required')
    return runners.SpendLedger.from_dict(json.loads(path.read_text()), checkpoint=lambda state: write_json(path, state))


def _route(config, actual):
    if (config.get('protocol') != 'open_router_decisions' or config.get('endpoint') != ENDPOINT
            or config.get('model') != MODEL or config.get('expected_model') != ACTUAL_MODEL
            or config.get('upstream') != 'perplexity' or not 1 <= config.get('max_input_bytes', 0) <= 65536
            or not 1 <= config.get('timeout_secs', 0) <= 90
            or actual.get('identity_evidence') != 'provider_reported_version'
            or actual.get('model') != ACTUAL_MODEL or actual.get('provider') != 'Perplexity'
            or actual.get('revision') not in ('', None) or actual.get('artifact_sha256') not in ('', None)):
        raise ValueError('explicit_approved_provider_version_route_required')


def _canonical_fixture(value):
    return {key: value[key] for key in ('id', 'family_id', 'split', 'language')} | {'utterances': [
        {key: row.get(key, False if key == 'failed' else None) for key in ('start_ms', 'end_ms', 'speaker', 'text', 'failed', 'voice')}
        for row in value['utterances']]}


def _approved_fixture(fixture, version=2):
    if version not in (2,3):raise ValueError('approved_invented_corpus_required')
    module = corpus_v3 if version==3 else corpus
    approved = {item['id']: item for item in module.experiment()['fixtures']}
    if fixture.get('id') not in approved or _canonical_fixture(fixture) != _canonical_fixture(approved[fixture['id']]):
        raise ValueError('approved_invented_corpus_required')


def _body(body, configuration):
    value = json.loads(body, object_pairs_hook=runners._unique_json)
    provider = value.get('provider', {})
    if (value.get('model') != configuration['model'] or provider.get('only') != ['perplexity']
            or provider.get('allow_fallbacks') is not False or provider.get('data_collection') != 'deny'
            or provider.get('zdr') is not True or len(body.encode()) > configuration['max_input_bytes']
            or not isinstance(value.get('questions'), dict) or not value['questions']):
        raise ValueError('production_route_privacy_or_input_bound_changed')
    return value


def reservation_bound(body, prompt_price, completion_price=Decimal(0)):
    """Byte-per-token upper bound, including repeated state for every question.

    This deliberately remains conservative even if the provider amortizes shared
    state. The actual selected upstream's published price supplies the reservation.
    """
    value = json.loads(body)
    count = len(value['questions'])
    state_bytes = len(json.dumps(value.get('state'), ensure_ascii=False).encode())
    question_bytes = len(json.dumps(value['questions'], ensure_ascii=False).encode())
    tokens = count * state_bytes + question_bytes + count * 2048
    amount = Decimal(tokens) * Decimal(prompt_price) + Decimal(MAX_BYTES) * Decimal(completion_price)
    return max(Decimal('.00000001'), amount.quantize(Decimal('.00000001'), rounding=ROUND_CEILING))


def _http_worker():
    value = json.load(sys.stdin)
    timeout = value['timeout']
    if value['endpoint'] != ENDPOINT or isinstance(timeout, bool) or not 0 < timeout <= 90:
        raise ValueError('invalid_transport_bound')
    request = urllib.request.Request(ENDPOINT, data=base64.b64decode(value['body'], validate=True),
                                     headers=value['headers'], method='POST')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), finished._NoRedirect())
    try: response = opener.open(request, timeout=timeout)
    except urllib.error.HTTPError as error: response = error
    with response:
        sys.stdout.buffer.write(json.dumps({'http_status': response.code}).encode() + b'\n')
        sys.stdout.buffer.flush()
        remaining = MAX_BYTES + 1
        while remaining:
            part = response.read1(min(65536, remaining))
            if not part: break
            sys.stdout.buffer.write(part);sys.stdout.buffer.flush();remaining -= len(part)


def hosted_transport(endpoint, body, timeout, credential):
    if endpoint != ENDPOINT or not credential: raise ValueError('explicit_route_credential_required')
    process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '_http'], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    value = {'endpoint': endpoint, 'body': base64.b64encode(body).decode(), 'timeout': min(90, timeout),
             'headers': {'Authorization': 'Bearer ' + credential, 'Content-Type': 'application/json'}}
    try: output, _ = process.communicate(json.dumps(value).encode(), timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill();output, _ = process.communicate()
        header, _, raw = output.partition(b'\n')
        try: status = json.loads(header)['http_status']
        except (ValueError, KeyError): status = None
        raise finished.TransportFailure('transport_deadline_exceeded', status, raw) from None
    header, _, raw = output.partition(b'\n')
    try: status = json.loads(header)['http_status']
    except (ValueError, KeyError): raise finished.TransportFailure('transport_failed', None, raw) from None
    if process.returncode or len(raw) > MAX_BYTES:
        raise finished.TransportFailure('transport_failed_or_oversized', status, raw)
    return status, raw


def _usage():
    own, children = resource.getrusage(resource.RUSAGE_SELF), resource.getrusage(resource.RUSAGE_CHILDREN)
    factor = 1 if sys.platform == 'darwin' else 1024
    return {'cpu_ms': (own.ru_utime + own.ru_stime + children.ru_utime + children.ru_stime) * 1000,
            'peak_rss_bytes': max(own.ru_maxrss, children.ru_maxrss) * factor,
            'client_peak_rss_bytes': own.ru_maxrss * factor,
            'largest_child_peak_rss_bytes': children.ru_maxrss * factor}


def _cost(raw):
    try:
        value = json.loads(raw, object_pairs_hook=runners._unique_json)
        billed = value.get('usage', {}).get('cost')
        amount = Decimal(str(billed))
        return amount if not isinstance(billed, bool) and amount.is_finite() and amount >= 0 else None
    except (ValueError, InvalidOperation, AttributeError): return None


def run_job(exported, freeze_sha256, bridge, credential, ledger, output, *, execute=False,
            transport=None, clock=time.monotonic, global_deadline=None, prompt_price=Decimal('.00000002'),
            completion_price=Decimal(0)):
    if not execute: raise ValueError('execution_not_enabled')
    _approved_fixture(exported['fixture'],exported.get('schema_version',2));_route(exported['configuration'], exported['expected_actual'])
    batches = qualification.physical_requests(exported)
    for batch in batches: _body(batch['wire_body'], exported['configuration'])
    output = Path(output)
    if output.exists(): raise ValueError('preserve_existing_execution_artifact')
    if ledger.limit > Decimal(2): raise ValueError('budget_exceeds_usd_2')
    started, initial_usage = clock(), _usage()
    deadline = started + exported['configuration']['timeout_secs']
    if global_deadline is not None: deadline = min(deadline, global_deadline)
    run = {'fixture_id': exported['fixture']['id'], 'split': exported['fixture']['split'],
        'freeze_sha256': freeze_sha256, 'export_sha256': exported['freeze_sha256'],
        'evidence_class': 'recorded_inference', 'request_sha256s': [], 'responses': [], 'requests': [],
        'status': 'blocked', 'reason': None, 'measurements': {},
        'transport_lifecycle': 'new_https_connection_and_transport_process_per_physical_request',
        'remote_server_cpu_rss': 'unavailable', 'remote_server_warmth': 'unknown'}
    def finish(reason=None):
        final_usage = _usage()
        run['reason'] = reason
        run['status'] = 'ok' if reason is None else 'blocked'
        run['measurements'] = {'whole_job_elapsed_ms': (clock() - started) * 1000,
            'cpu_time_ms': max(0, final_usage['cpu_ms'] - initial_usage['cpu_ms']),
            'peak_rss_bytes': final_usage['peak_rss_bytes'],
            'rss_measurement': 'maximum_individual_process_lifetime_high_water_mark',
            'client_peak_rss_bytes': final_usage['client_peak_rss_bytes'],
            'largest_child_peak_rss_bytes': final_usage['largest_child_peak_rss_bytes'],
            'aggregate_process_tree_rss_bytes': None, 'executed_requests': len(run['requests']),
            'input_bytes': sum(row['input_bytes'] for row in run['requests'])}
        run['spend_ledger'] = ledger.snapshot();write_json(output, run)
        return run
    def expired():
        if global_deadline is not None and clock() >= global_deadline: return 'evaluation_budget_exceeded'
        return 'job_deadline_exceeded' if clock() >= deadline else None
    try:
        finished._verify_export(exported, bridge, deadline=deadline, clock=clock)
    except Exception as error:
        return finish(expired() or (str(error) if isinstance(error, finished.FinishedError) else 'core_bridge_failed'))
    for batch in batches:
        if expired(): return finish(expired())
        body = batch['wire_body'].encode()
        request_sha = hashlib.sha256(body).hexdigest()
        try:
            bound = reservation_bound(batch['wire_body'], prompt_price, completion_price)
            if ledger.total_exposure + bound + NATIVE_SMOKE_ALLOWANCE > ledger.limit:
                return finish('reserved_native_smoke_budget_unavailable')
            token = ledger.reserve(bound, {'fixture_id': run['fixture_id'], 'batch_index': batch['index'], 'request_sha256': request_sha})
        except runners.EvaluationError as error: return finish(str(error))
        entry = {'batch_index': batch['index'], 'reservation_id': token, 'request_sha256': request_sha, 'input_bytes': len(body),
            'http_status': None, 'raw_response': None, 'elapsed_ms': None}
        run['requests'].append(entry);write_json(output, run)
        request_started = clock()
        try:
            status, raw = (transport or hosted_transport)(exported['configuration']['endpoint'], body, deadline - clock(), credential)
            finished._response_entry(entry, status, raw, complete=True)
            if len(raw) > MAX_BYTES or entry.get('raw_response') is None:
                raise finished.TransportFailure('invalid_or_oversized_response', status, raw)
        except Exception as error:
            if isinstance(error, finished.TransportFailure):
                finished._response_entry(entry, error.http_status, error.raw_response, complete=False)
            entry['elapsed_ms'] = (clock() - request_started) * 1000
            ledger.settle(token, None, {'fixture_id': run['fixture_id'], 'batch_index': batch['index'],
                'request_sha256': request_sha, 'status': 'transport_failed'})
            return finish(expired() or (str(error) if isinstance(error, finished.FinishedError) else 'transport_failed'))
        entry['elapsed_ms'] = (clock() - request_started) * 1000
        ledger.settle(token, _cost(entry['raw_response']), {'fixture_id': run['fixture_id'],
            'batch_index': batch['index'], 'request_sha256': request_sha, 'http_status': status})
        run['request_sha256s'].append(request_sha)
        run['responses'].append({'batch_index': batch['index'], 'http_status': status, 'body': entry['raw_response']})
        write_json(output, run)
        if expired(): return finish(expired())
        if ledger.unresolved: return finish('unresolved_charge')
        if not 200 <= status < 300: return finish('provider_failed')
        try:
            parsed = json.loads(entry['raw_response'], object_pairs_hook=runners._unique_json)
            if parsed.get('model') != ACTUAL_MODEL or parsed.get('provider') != 'Perplexity':
                return finish('identity_changed')
            if set(parsed.get('answers', {})) != set(json.loads(batch['wire_body'])['questions']):
                return finish('invalid_answer_ids')
        except (ValueError, TypeError): return finish('invalid_response')
    try:
        replay = finished._bridge_call(bridge, {'op': 'finished_meeting_replay', 'export': exported, 'responses': run['responses']},
                                       deadline=deadline, clock=clock)
        run['replay_summary'] = {key: replay.get(key) for key in ('status', 'complete', 'schema_version', 'protocol_id')}
    except Exception as error:
        return finish(expired() or (str(error) if isinstance(error, finished.FinishedError) else 'core_bridge_failed'))
    if expired(): return finish(expired())
    if replay.get('status') != 'ok' or not replay.get('complete'): return finish(replay.get('status', 'invalid_replay'))
    return finish()


def _anonymous_credential_pipe(fd, information):
    if os.name != 'posix' or not stat.S_ISFIFO(information.st_mode): return False
    if sys.platform.startswith('linux'):
        try: target = os.readlink('/proc/self/fd/' + str(fd))
        except OSError: return False
        return target == 'pipe:[' + str(information.st_ino) + ']'
    if sys.platform == 'darwin' or sys.platform.startswith(('freebsd', 'openbsd', 'netbsd', 'dragonfly')):
        return information.st_nlink == 0
    return False


def credential_from_fd(fd, *, deadline=None, clock=time.monotonic):
    if isinstance(fd, bool) or not isinstance(fd, int) or fd < 3:
        raise ValueError('invalid_credential_fd')
    bounded = clock() + 15
    if deadline is not None: bounded = min(bounded, deadline)
    data = bytearray()
    try:
        information = os.fstat(fd)
        if not _anonymous_credential_pipe(fd, information):
            raise ValueError('anonymous_credential_pipe_required')
        os.set_blocking(fd, False)
        with selectors.DefaultSelector() as selector:
            selector.register(fd, selectors.EVENT_READ)
            while True:
                remaining = bounded-clock()
                if remaining <= 0 or not selector.select(remaining):
                    raise ValueError('credential_pipe_timeout')
                try: part = os.read(fd, min(4097-len(data), 4096))
                except BlockingIOError: continue
                if not part: break
                data.extend(part)
                if len(data) > 4096: raise ValueError('credential_pipe_too_large')
        try: secret = data.decode('utf-8').rstrip('\r\n')
        except UnicodeDecodeError: raise ValueError('invalid_credential_pipe_encoding') from None
        if not secret: raise ValueError('credential_pipe_empty')
        if any(ord(char) < 33 or ord(char) > 126 for char in secret):
            raise ValueError('invalid_credential_pipe_value')
        return secret
    except OSError:
        raise ValueError('credential_pipe_unavailable') from None
    finally:
        for index in range(len(data)): data[index] = 0
        try: os.close(fd)
        except OSError: pass


def credential(credential_fd=None, *, deadline=None, clock=time.monotonic):
    if credential_fd is not None: return credential_from_fd(credential_fd, deadline=deadline, clock=clock)
    timeout = 15 if deadline is None else min(15, deadline-clock())
    if timeout <= 0: raise ValueError('evaluation_budget_exceeded')
    try:
        result = subprocess.run(['security', 'find-generic-password', '-s', 'dev.pennel.evaluation.openrouter',
                                 '-a', 'OPENROUTER_API_KEY', '-w'], capture_output=True, check=False, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise ValueError('evaluation_credential_timeout') from None
    if result.returncode or not result.stdout.strip(): raise ValueError('evaluation_credential_unavailable')
    return result.stdout.decode().rstrip('\r\n')


def _verify_committed_lock(commit, lock, frozen):
    if not commit or not lock: raise ValueError('committed_freeze_required')
    path = Path(lock).resolve().relative_to(ROOT)
    raw = subprocess.check_output(['git', 'show', commit + ':' + str(path)], cwd=ROOT)
    value = json.loads(raw)
    if value.get('freeze_sha256') != frozen['freeze_sha256']:
        raise ValueError('committed_freeze_mismatch')


def _prices(path):
    value = json.loads(Path(path).read_text())
    data = value.get('data', value)
    if data.get('id') != MODEL: raise ValueError('exact_model_endpoint_price_evidence_required')
    matches = [row for row in data.get('endpoints', [])
               if row.get('provider_name') == 'Perplexity' and row.get('tag') == 'perplexity']
    if len(matches) != 1: raise ValueError('exact_upstream_price_evidence_required')
    if matches[0].get('model_id') != MODEL or ACTUAL_MODEL not in matches[0].get('name', ''):
        raise ValueError('exact_returned_model_price_evidence_required')
    price = matches[0]['pricing']
    prompt, completion = Decimal(price['prompt']), Decimal(price['completion'])
    if not prompt.is_finite() or not completion.is_finite() or prompt < 0 or completion < 0:
        raise ValueError('invalid_exact_upstream_price')
    return prompt, completion


def _client_worker():
    """Retained job client; its memory excludes the corpus-supervisor freeze."""
    bridge = None
    for line in sys.stdin:
        value = json.loads(line)
        if bridge is None:
            bridge = finished.CoreBridge(value['bridge'])
        if bridge.artifact_sha256 != value['bridge_sha256']:
            raise ValueError('bridge_binary_changed')
        ledger = load_ledger(value['ledger'])
        result = run_job(value['export'], value['freeze_sha256'], bridge, value['credential'], ledger, value['output'],
                         execute=True, global_deadline=value['global_deadline'], prompt_price=Decimal(value['prompt_price']),
                         completion_price=Decimal(value['completion_price']))
        print(json.dumps({'status': result['status'], 'reason': result['reason']}), flush=True)


class JobClient:
    def __init__(self, command=None, grace_seconds=10):
        self.grace_seconds = grace_seconds
        self.started = time.monotonic()
        self.process = subprocess.Popen(command or [sys.executable, str(Path(__file__).resolve()), '_client'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
    def call(self, value):
        deadline = min(value['global_deadline'], time.monotonic() + value['export']['configuration']['timeout_secs'] + self.grace_seconds)
        self.process.stdin.write(json.dumps(value, ensure_ascii=False) + '\n');self.process.stdin.flush()
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            if not selector.select(max(0, deadline-time.monotonic())):
                self.process.kill();self.process.wait();raise ValueError('client_deadline_exceeded')
        line = self.process.stdout.readline()
        if not line: raise ValueError('isolated_client_failed')
        result = json.loads(line)
        if result.get('status') not in ('ok', 'blocked'): raise ValueError('invalid_client_response')
        return result
    def close(self):
        if not self.process.stdin.closed:
            try: self.process.stdin.close()
            except BrokenPipeError: pass
        if self.process.poll() is None:
            try: self.process.wait(timeout=5)
            except subprocess.TimeoutExpired: self.process.kill();self.process.wait()
        self.process.stdout.close()
    def __enter__(self): return self
    def __exit__(self, *_): self.close()


def _client_value(exported, frozen, bridge, args, secret, target, global_deadline, prices):
    return {'export': exported, 'freeze_sha256': frozen['freeze_sha256'], 'bridge': str(Path(args.bridge).resolve()),
        'bridge_sha256': bridge.artifact_sha256, 'ledger': str(Path(args.ledger).resolve()), 'credential': secret,
        'output': str(target.resolve()), 'global_deadline': global_deadline,
        'prompt_price': str(prices[0]), 'completion_price': str(prices[1])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('development', 'calibration', 'heldout', 'resources'))
    parser.add_argument('--freeze', required=True);parser.add_argument('--bridge', required=True)
    parser.add_argument('--output', required=True);parser.add_argument('--pricing', required=True)
    parser.add_argument('--ledger', default=str(DEFAULT_LEDGER));parser.add_argument('--execute', action='store_true')
    parser.add_argument('--credential-fd', type=int, help='Inherited anonymous pipe only; secret never belongs in argv/environment/files')
    parser.add_argument('--fixture-id', action='append');parser.add_argument('--freeze-commit');parser.add_argument('--freeze-lock')
    args = parser.parse_args()
    if not args.execute: raise ValueError('execution_not_enabled')
    output = Path(args.output)
    if output.exists(): raise ValueError('preserve_existing_execution_artifact')
    output.mkdir(parents=True)
    started = time.monotonic();global_deadline = started + MAX_SECONDS
    secret = credential(args.credential_fd, deadline=global_deadline) if args.credential_fd is not None else None
    frozen = json.loads(Path(args.freeze).read_text());bridge = finished.CoreBridge(args.bridge)
    qualification.verify_freeze(frozen, bridge, deadline=global_deadline, clock=time.monotonic)
    if not qualification._batched(frozen): raise ValueError('batched_provider_version_freeze_required')
    if args.phase != 'development': _verify_committed_lock(args.freeze_commit, args.freeze_lock, frozen)
    if args.phase in ('heldout', 'resources') and frozen['stage'] != 'heldout_ready':
        raise ValueError('calibration_before_heldout_required')
    for exported in frozen['production']['exports']:
        _approved_fixture(exported['fixture'],exported.get('schema_version',2));_route(exported['configuration'], exported['expected_actual'])
    if time.monotonic() >= global_deadline: raise ValueError('evaluation_budget_exceeded')
    prices = _prices(args.pricing)
    if time.monotonic() >= global_deadline: raise ValueError('evaluation_budget_exceeded')
    if secret is None: secret = credential(deadline=global_deadline)
    runs = []
    with locked_ledger(args.ledger) as ledger:
        if args.phase == 'resources':
            exported = next(row for row in frozen['production']['exports'] if row['fixture']['id'] == frozen['resource_fixture_id'])
            full_panel_bound = sum(reservation_bound(batch['wire_body'], *prices) for batch in qualification.physical_requests(exported)) * 35
            if ledger.total_exposure + full_panel_bound + NATIVE_SMOKE_ALLOWANCE > ledger.limit or ledger.unresolved:
                raise ValueError('fixed_resource_panel_exceeds_reconciled_budget')
            samples = []
            for phase, count in [('cold', 5), ('warm', 30)]:
                retained = JobClient() if phase == 'warm' else None
                try:
                    for index in range(count):
                        identifier = f'{phase}-{index}';target = output / (identifier + '.json')
                        value = _client_value(exported, frozen, bridge, args, secret, target, global_deadline, prices)
                        if retained is None:
                            with JobClient() as fresh:
                                fresh.call(value)
                                lifecycle_ms = (time.monotonic()-fresh.started)*1000
                        else:
                            call_started = time.monotonic();retained.call(value)
                            lifecycle_ms = (time.monotonic()-call_started)*1000
                        if not target.is_file(): raise ValueError('client_result_missing')
                        run = json.loads(target.read_text());run['client_lifecycle_elapsed_ms'] = lifecycle_ms
                        run['client_process_kind'] = 'fresh' if phase == 'cold' else 'retained'
                        write_json(target, run)
                        ledger = load_ledger(args.ledger)
                        samples.append({'id': identifier, 'phase': phase, 'run': run})
                        if run['status'] != 'ok': raise ValueError('resource_job_failed_' + str(run['reason']))
                        print(json.dumps({'phase': 'resources', 'sample': identifier, 'requests': run['measurements']['executed_requests'], 'status': run['status']}), flush=True)
                finally:
                    if retained is not None: retained.close()
            resource_report = {'freeze_sha256': frozen['freeze_sha256'], 'resource_profile_sha256': exported['qualification_request']['resource_profile_sha256'],
                'scope': 'owned_client', 'cpu_time_ms': sum(row['run']['measurements']['cpu_time_ms'] for row in samples),
                'peak_rss_bytes': max(row['run']['measurements']['peak_rss_bytes'] for row in samples), 'samples': samples,
                'lifecycle': {'cold': 'five fresh evaluation-client processes; new HTTPS transport per physical request',
                              'warm': 'thirty repeated complete jobs in one retained evaluation-client process; new HTTPS transport per physical request',
                              'server_cold_warm': 'unknown', 'warm_process_only': True,
                              'client_scope': 'isolated job client process plus waited HTTP and production Rust children',
                              'corpus_supervisor_memory_excluded': True,
                              'rss_measurement': 'maximum individual-process lifetime high-water mark across client and waited children; not aggregate process-tree RSS',
                              'aggregate_process_tree_rss': 'unmeasured'}}
            write_json(output / 'resources.json', resource_report)
        else:
            selected = [row for row in frozen['production']['exports'] if row['fixture']['split'] == args.phase]
            if args.fixture_id:
                if args.phase != 'development': raise ValueError('partial_quality_execution_forbidden')
                selected = [row for row in selected if row['fixture']['id'] in args.fixture_id]
                if len(selected) != len(args.fixture_id): raise ValueError('unknown_development_fixture')
            with JobClient() as client:
                for exported in selected:
                    target = output / (exported['fixture']['id'] + '.json')
                    client.call(_client_value(exported, frozen, bridge, args, secret, target, global_deadline, prices))
                    run = json.loads(target.read_text());ledger = load_ledger(args.ledger)
                    runs.append(run);write_json(output / 'predictions.json', runs)
                    print(json.dumps({'phase': args.phase, 'fixture_id': run['fixture_id'], 'requests': run['measurements']['executed_requests'],
                                      'elapsed_ms': round(run['measurements']['whole_job_elapsed_ms'],2), 'status': run['status'], 'reason': run['reason']}), flush=True)
                    if run['status'] != 'ok': raise ValueError('production_job_failed_' + str(run['reason']))
        write_json(output / 'phase-receipt.json', {'phase': args.phase, 'elapsed_ms': (time.monotonic()-started)*1000,
            'freeze_sha256': frozen['freeze_sha256'], 'spend_ledger': ledger.snapshot(), 'automatic_retries': False, 'fallbacks': False,
            'pricing_evidence_sha256': finished.file_hash(args.pricing), 'prompt_token_price_usd': str(prices[0]),
            'completion_token_price_usd': str(prices[1]), 'native_smoke_allowance_usd': str(NATIVE_SMOKE_ALLOWANCE),
            'credential_delivery': 'inherited_anonymous_pipe' if args.credential_fd is not None else 'bounded_os_keychain',
            'client_scope': 'isolated bounded job client plus waited HTTP/production Rust children; corpus supervisor separate'})


if __name__ == '__main__':
    try:
        if sys.argv[1:] == ['_http']: _http_worker()
        elif sys.argv[1:] == ['_client']: _client_worker()
        else: main()
    except Exception as error:
        # Never format transport exceptions, credential-bearing inputs, or arbitrary model content.
        message = str(error)
        code = message if re.fullmatch(r'[a-z][a-z0-9_]{1,150}', message) else 'experiment_failed'
        print(json.dumps({'error': code}), file=sys.stderr)
        raise SystemExit(2) from None
