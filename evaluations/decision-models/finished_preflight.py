"""Identity-first execution of unchanged production bytes on two explicitly allowed routes."""

import base64
from decimal import Decimal, InvalidOperation
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
import time
import urllib.error
import urllib.request

import finished

HOSTED_ENDPOINT = 'https://openrouter.ai/api/alpha/decisions'
HOSTED_MODEL = 'perplexity/pplx-decider-v1.2'
MAX_BYTES = finished.MAX_RESPONSE_BYTES


def _request_worker():
    value = json.load(sys.stdin)
    if value.get('endpoint') != HOSTED_ENDPOINT:
        raise ValueError('unsupported_hosted_endpoint')
    timeout = value['timeout']
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not 0 < timeout <= 30:
        raise ValueError('invalid_transport_timeout')
    request = urllib.request.Request(HOSTED_ENDPOINT, data=base64.b64decode(value['body'], validate=True),
                                    headers=value['headers'], method='POST')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), finished._NoRedirect())
    try:
        response = opener.open(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        sys.stdout.buffer.write(json.dumps({'http_status': response.code}).encode() + b'\n')
        sys.stdout.buffer.flush()
        remaining = MAX_BYTES + 1
        while remaining > 0:
            chunk = response.read1(min(65536, remaining))
            if not chunk: break
            sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()
            remaining -= len(chunk)


def hosted_transport(endpoint, body, timeout, credential):
    if endpoint != HOSTED_ENDPOINT or not credential:
        raise finished.FinishedError('explicit_hosted_route_and_credential_required')
    process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '_request'],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    value = {'endpoint': endpoint, 'body': base64.b64encode(body).decode(), 'timeout': min(30, timeout),
             'headers': {'Content-Type': 'application/json', 'Authorization': 'Bearer ' + credential}}
    try:
        output, _ = process.communicate(json.dumps(value).encode(), timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        output, _ = process.communicate()
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


def run(frozen, bridge, *, execute=False, credential=None, ledger=None, transport=None,
        clock=time.monotonic, max_seconds=7200, started_at=None):
    if not execute:
        raise finished.FinishedError('execution_not_enabled')
    if isinstance(max_seconds, bool) or not isinstance(max_seconds, (int, float)) or not 0 < max_seconds <= 7200:
        raise finished.FinishedError('evaluation_budget_exceeds_two_hours')
    now = clock()
    if started_at is not None and (isinstance(started_at, bool) or not isinstance(started_at, (int, float))
                                  or not math.isfinite(started_at) or started_at > now):
        raise finished.FinishedError('invalid_evaluation_start')
    global_deadline = (now if started_at is None else started_at) + max_seconds
    try:
        finished.verify_freeze(frozen, bridge, deadline=global_deadline, clock=clock)
    except Exception:
        if clock() >= global_deadline:
            return {'schema_version': 1, 'freeze_sha256': frozen.get('freeze_sha256'), 'status': 'blocked',
                'reason': 'evaluation_budget_exceeded', 'qualification': False, 'production_admission': False,
                'evidence_class': 'recorded_inference', 'physical_requests': 0, 'requests': [], 'fixtures': []}
        raise
    exports = frozen['exports']
    if any(item['fixture']['language'] != 'nl' or item['fixture']['split'] != 'development' for item in exports):
        raise finished.FinishedError('invented_dutch_development_only')
    config = exports[0]['configuration']
    hosted = config.get('protocol') == 'open_router_decisions'
    if hosted:
        if (config.get('endpoint') != HOSTED_ENDPOINT or config.get('model') != HOSTED_MODEL
                or config.get('expected_model') != 'perplexity/pplx-decider-v1.2-20261016'
                or config.get('upstream') != 'perplexity' or config.get('timeout_secs') != 30
                or config.get('max_input_bytes') != 16000 or ledger is None or not credential):
            raise finished.FinishedError('explicit_pinned_hosted_preflight_required')
        if ledger.limit > 2:
            raise finished.FinishedError('hosted_budget_exceeds_usd_2')
    else:
        finished._local_configuration(config)
    if any(item['configuration'] != config for item in exports):
        raise finished.FinishedError('mixed_candidate_routes')
    if hosted:
        for exported in exports:
            for window in exported['windows']:
                provider = json.loads(window['wire_body']).get('provider', {})
                if provider.get('only') != ['perplexity'] or provider.get('allow_fallbacks') is not False or provider.get('data_collection') != 'deny' or provider.get('zdr') is not True:
                    raise finished.FinishedError('production_route_privacy_contract_changed')
    report = {'schema_version': 1, 'freeze_sha256': frozen['freeze_sha256'], 'configuration': config,
        'status': 'blocked', 'reason': None, 'qualification': False, 'production_admission': False,
        'evidence_class': 'recorded_inference', 'physical_requests': 0, 'requests': [], 'fixtures': []}
    for exported in exports:
        deadline = min(global_deadline, clock() + config['timeout_secs'])
        responses = []
        for index, window in enumerate(exported['windows']):
            remaining = deadline - clock()
            if remaining <= 0:
                report['reason'] = finished._deadline_reason(clock(), deadline, global_deadline)
                return report
            body = window['wire_body'].encode()
            try:
                reservation = ledger.reserve('0.10') if hosted else None
            except Exception:
                report.update(reason='unresolved_or_exhausted_hosted_budget', spend_ledger=ledger.snapshot())
                return report
            entry = {'fixture_id': exported['fixture']['id'], 'window_index': index,
                     'request_sha256': hashlib.sha256(body).hexdigest(), 'http_status': None}
            report['requests'].append(entry)
            report['physical_requests'] += 1
            started = clock()
            try:
                if transport is not None:
                    status, raw = transport(config['endpoint'], body, remaining)
                elif hosted:
                    status, raw = hosted_transport(config['endpoint'], body, remaining, credential)
                else:
                    status, raw = finished.local_transport(config['endpoint'], body, remaining, deadline=deadline)
                raw_bytes = finished._response_entry(entry, status, raw, complete=True)
                if len(raw_bytes) > MAX_BYTES or entry.get('raw_response') is None:
                    raise finished.FinishedError('invalid_or_oversized_response')
            except Exception as error:
                if isinstance(error, finished.TransportFailure):
                    finished._response_entry(entry, error.http_status, error.raw_response, complete=False)
                entry['elapsed_ms'] = (clock() - started) * 1000
                if hosted:
                    ledger.settle(reservation, None, {'request_sha256': entry['request_sha256'], 'status': 'transport_failed'})
                    report['spend_ledger'] = ledger.snapshot()
                report['reason'] = str(error) if isinstance(error, finished.FinishedError) else 'transport_failed'
                return report
            entry['elapsed_ms'] = (clock() - started) * 1000
            parsed = None
            try: parsed = json.loads(entry['raw_response'])
            except ValueError: pass
            if hosted:
                usage = parsed.get('usage') if isinstance(parsed, dict) else None
                billed = usage.get('cost') if isinstance(usage, dict) else None
                try:
                    amount = Decimal(str(billed))
                    if isinstance(billed, bool) or not amount.is_finite() or amount < 0: billed = None
                except (ValueError, InvalidOperation): billed = None
                ledger.settle(reservation, billed, {'request_sha256': entry['request_sha256'], 'http_status': status})
                report['spend_ledger'] = ledger.snapshot()
            if clock() >= deadline:
                report['reason'] = finished._deadline_reason(clock(), deadline, global_deadline)
                return report
            if not 200 <= status < 300:
                report['reason'] = 'provider_failed'
                return report
            responses.append({'window_index': index, 'http_status': status, 'body': entry['raw_response']})
            try:
                replay = finished._bridge_call(bridge, {'op': 'finished_meeting_replay', 'export': exported, 'responses': responses}, deadline=deadline, clock=clock)
            except Exception as error:
                report['reason'] = (finished._deadline_reason(clock(), deadline, global_deadline)
                    or (str(error) if isinstance(error, finished.FinishedError) else 'core_bridge_failed'))
                return report
            report['fixtures'].append({'id': exported['fixture']['id'], 'window_index': index, 'replay': replay})
            if replay.get('status') != 'ok':
                report['reason'] = replay.get('status', 'invalid_replay')
                return report
            if hosted and ledger.unresolved:
                report['reason'] = 'unresolved_charge'
                return report
    report.update(status='compatible', reason=None)
    return report


if __name__ == '__main__' and sys.argv[1:] == ['_request']:
    try: _request_worker()
    except Exception: raise SystemExit(2) from None
