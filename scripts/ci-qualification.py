#!/usr/bin/env python3
"""Capture PR cohort evidence before rerunning, then assess the CI timing gates.

Previous-attempt artifacts may no longer be listed after a rerun. Archive each
complete observation first; this collector never substitutes newer receipts.
"""

import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import selectors
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile


WORKFLOWS = {
    'docs': ('ci.yml', 'desktop.yml'),
    'desktop': ('ci.yml', 'desktop.yml'),
    'mobile': ('ci.yml', 'desktop.yml', 'mobile.yml'),
    'core': ('ci.yml', 'desktop.yml', 'mobile.yml', 'decision-evaluation.yml'),
}
TARGETS = {'docs': 300, 'desktop': 1200, 'mobile': 3600, 'core': 3600}
PHASES = {
    'serial': ('typecheck-and-tests', 'native-preparation', 'release-simulator',
               'release-device', 'debug-runtime-and-keyboard'),
    'release': ('typecheck-and-tests', 'native-preparation', 'release-simulator', 'release-device'),
    'runtime': ('typecheck-and-tests', 'native-preparation', 'debug-runtime-and-keyboard'),
}
SHA = re.compile(r'[0-9a-f]{40}')
IDENTIFIER = re.compile(r'[A-Za-z0-9][A-Za-z0-9._-]{0,63}')
LOG_LIMIT = 262144
ARCHIVE_LIMIT = 4 * 1024 * 1024
READ_ATTEMPTS = 3
READ_TIMEOUT = 25


class ReadError(ValueError):
    """An observer failure, distinct from a hosted job's execution result."""


def read_failure(stderr):
    text = stderr.decode('utf-8', errors='replace') if isinstance(stderr, bytes) else str(stderr)
    matched = re.search(r'\bHTTP\s+([0-9]{3})\b', text, re.I)
    status = int(matched[1]) if matched else None
    if status is not None:
        return 'http-error', status, status == 429 or status >= 500
    transient = re.search(
        r'tls handshake timeout|i/o timeout|context deadline exceeded|timed out|'
        r'could not resolve host|temporary failure in name resolution|dial tcp|'
        r'connection reset|unexpected eof|\beof\b|connection refused|'
        r'network is unreachable|connection aborted|failed to connect|'
        r'timeout awaiting response headers|server misbehaving|http2.*stream error', text, re.I)
    return ('network-error', None, True) if transient else ('cli-error', None, False)


def utc(value):
    if not isinstance(value, str):
        raise ValueError('Missing timestamp')
    try:
        result = datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError:
        raise ValueError('Invalid timestamp') from None
    if result.tzinfo is None or result.utcoffset() != timedelta(0):
        raise ValueError('Timestamps must include UTC')
    return result


def integer(value):
    if isinstance(value, bool) or not re.fullmatch(r'[1-9][0-9]*', str(value)):
        raise ValueError('Invalid positive integer')
    return int(value)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)


def read_json_archive(payload, filename):
    if len(payload) > ARCHIVE_LIMIT:
        raise ValueError('Timing artifact archive exceeds its size limit')
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        members = archive.infolist()
        if len(members) != 1 or members[0].filename != filename:
            raise ValueError('Timing artifact has an unexpected root or members')
        member = members[0]
        kind = (member.external_attr >> 16) & 0o170000
        if (member.is_dir() or member.file_size > ARCHIVE_LIMIT
                or kind not in (0, 0o100000)):
            raise ValueError('Timing artifact must be one bounded regular JSON file')
        value = json.loads(archive.read(member))
        if not isinstance(value, dict):
            raise ValueError('Timing artifact JSON must be an object')
        return value


def log_identity(prefix):
    text = re.sub(r'\x1b\[[0-9;]*m', '', prefix)
    text = text.removeprefix('\ufeff')
    text = re.sub(r'(?m)^\d{4}-\d\d-\d\dT\S+\s+', '', text)
    image_group = re.search(r'##\[group\]Runner Image[ \t]*\r?\n(.*?)##\[endgroup\]', text, re.S)
    image_text = image_group[1] if image_group else ''
    def match(pattern, source=text):
        found = re.search(pattern, source, re.M)
        return found[1].strip() if found else None
    full_match = re.search(r'full match:\s*(true|false)', text, re.I)
    keys = re.findall(r'v0-rust-[A-Za-z0-9_.-]+', text)
    return {
        'image': match(r'^\s*Image:\s*(.+)$', image_text),
        'image_version': match(r'^\s*Version:\s*(.+)$', image_text),
        'rustc': match(r'^\s*(rustc \d+\.\d+\.\d+[^\n]*)$'),
        'node': match(r'^\s*node:\s*(v?[0-9][^\n]*)$'),
        'npm': match(r'^\s*npm:\s*([0-9][^\n]*)$'),
        'python': match(r'Successfully set up (CPython \([^\n]+\))'),
        'rust_key': keys[-1] if keys else None,
        'node_cache_keys': sorted(set(re.findall(r'node-cache-[A-Za-z0-9_.-]+', text))),
        'node_cache_restored': bool(re.search(r'(?:Cache restored from key:|Cache hit for:)\s*node-cache-', text)),
        'rust_exact': full_match[1].lower() == 'true' if full_match else (
            False if re.search(r'(?:No cache found|Cache not found)', text, re.I) and keys else None),
    }


class GitHub:
    def __init__(self, repository):
        if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
            raise ValueError('Invalid repository')
        self.prefix = 'repos/' + repository + '/'
        self.read_trace = []

    def request(self, endpoint, binary=False):
        safe_endpoint = endpoint.split('?')[0]
        for attempt in range(1, READ_ATTEMPTS + 1):
            try:
                arguments = ['gh', 'api', '--method', 'GET', self.prefix + endpoint]
                if binary:
                    arguments.append('--allow-escape-sequences')
                result = subprocess.run(arguments,
                                        capture_output=True, timeout=READ_TIMEOUT)
                if not result.returncode:
                    try:
                        value = result.stdout if binary else json.loads(result.stdout)
                    except (json.JSONDecodeError, UnicodeDecodeError):
                        kind, status, transient = 'invalid-json', None, False
                    else:
                        if attempt > 1:
                            self.read_trace.append({'kind': 'metadata-read', 'endpoint': safe_endpoint,
                                                    'attempt': attempt, 'status': 'recovered',
                                                    'recorded_at': datetime.now(timezone.utc).isoformat()})
                        return value
                else:
                    kind, status, transient = read_failure(result.stderr)
            except subprocess.TimeoutExpired:
                kind, status, transient = 'timeout', None, True
            retrying = transient and attempt < READ_ATTEMPTS
            # Keep only classified diagnostics: gh error text may include credentials.
            self.read_trace.append({'kind': 'metadata-read', 'endpoint': safe_endpoint,
                                    'attempt': attempt, 'status': 'error', 'error_kind': kind,
                                    'http_status': status, 'retrying': retrying,
                                    'recorded_at': datetime.now(timezone.utc).isoformat()})
            if not retrying:
                raise ReadError(f'Observer GitHub GET failed: {safe_endpoint} ({kind}, attempts={attempt})')
            time.sleep(min(attempt, 2))

    def pages(self, endpoint, field):
        values = []
        for page in range(1, 11):
            separator = '&' if '?' in endpoint else '?'
            data = self.request(f'{endpoint}{separator}per_page=100&page={page}')
            batch = data[field]
            values.extend(batch)
            if len(batch) < 100:
                return values
        raise ValueError('Evidence exceeds bounded pagination')

    def job_identity(self, job):
        if not job.get('runner_name') or job.get('status') != 'completed':
            return {}
        endpoint = f"actions/jobs/{job['id']}/logs"
        for attempt in range(1, READ_ATTEMPTS + 1):
            prefix = bytearray()
            timed_out = False
            with tempfile.TemporaryFile() as diagnostics:
                process = subprocess.Popen(['gh', 'api', '--method', 'GET', self.prefix + endpoint,
                                            '--allow-escape-sequences'],
                                           stdout=subprocess.PIPE, stderr=diagnostics)
                try:
                    with selectors.DefaultSelector() as selector:
                        selector.register(process.stdout, selectors.EVENT_READ)
                        deadline = time.monotonic() + READ_TIMEOUT
                        while len(prefix) < LOG_LIMIT:
                            remaining = deadline - time.monotonic()
                            if remaining <= 0 or not selector.select(remaining):
                                timed_out = True
                                break
                            chunk = os.read(process.stdout.fileno(), min(65536, LOG_LIMIT - len(prefix)))
                            if not chunk:
                                break
                            prefix.extend(chunk)
                finally:
                    process.stdout.close()
                    if process.poll() is None:
                        process.terminate()
                    try:
                        returncode = process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        returncode = process.wait()
                diagnostics.seek(0)
                kind, status, transient = read_failure(diagnostics.read(8192))
            bounded = len(prefix) == LOG_LIMIT
            retrying = (not bounded and (timed_out or (not prefix and (returncode == 0 or transient)))
                        and attempt < READ_ATTEMPTS)
            self.read_trace.append({'kind': 'log-prefix-read',
                                    'endpoint': endpoint, 'attempt': attempt,
                                    'status': 'bounded' if bounded else 'timeout' if timed_out else (
                                        'empty' if not prefix else 'recovered' if attempt > 1 else 'complete'),
                                    'bytes': len(prefix), 'retrying': retrying,
                                    'error_kind': 'timeout' if timed_out else kind if not prefix and returncode else None,
                                    'http_status': status if not prefix else None,
                                    'recorded_at': datetime.now(timezone.utc).isoformat()})
            if retrying:
                time.sleep(min(attempt, 2))
                continue
            if timed_out or not prefix:
                return {}
            return log_identity(prefix.decode('utf-8', errors='replace'))


def run_spec(value):
    match = re.fullmatch(r'([A-Za-z0-9_.-]+\.yml)=([1-9][0-9]*)(?::([1-9][0-9]*))?', value)
    if not match:
        raise ValueError('Run selection must be workflow.yml=RUN_ID[:ATTEMPT]')
    return match[1], int(match[2]), int(match[3]) if match[3] else None


def capture(args, api=None):
    api = api or GitHub(args.repository)
    destination = Path(args.output)
    destination.mkdir(parents=True, exist_ok=True)
    if any((destination / name).exists() for name in ('observation.json', 'observer-read-error.json')):
        raise ValueError('Use a fresh output directory; existing observations are preserved')
    requested = utc(args.requested_at)
    if requested > datetime.now(timezone.utc):
        raise ValueError('Observation request cannot be in the future')
    if not SHA.fullmatch(args.head_sha) or not IDENTIFIER.fullmatch(args.cohort):
        raise ValueError('Invalid frozen head or cohort identity')
    integer(args.pr)
    pair = args.pair or args.cohort
    if not IDENTIFIER.fullmatch(pair):
        raise ValueError('Invalid matched pair identity')
    def preserve_read_error(error):
        failure = {'schema': 1, 'status': 'observer-read-error', 'repository': args.repository,
                   'pr': args.pr, 'head_sha': args.head_sha, 'cohort': args.cohort,
                   'requested_at': args.requested_at, 'error': str(error),
                   'api_read_trace': list(getattr(api, 'read_trace', []))}
        (destination / 'observer-read-error.json').write_text(json.dumps(failure, indent=2, sort_keys=True) + '\n')
    try:
        pr = api.request(f'pulls/{args.pr}')
    except ReadError as error:
        preserve_read_error(error)
        raise
    if (pr['head']['sha'] != args.head_sha or pr['head']['repo']['full_name'] != args.repository
            or pr['base']['repo']['full_name'] != args.repository):
        raise ValueError('PR is not the requested immutable same-repository source')
    observation = {
        'schema': 1, 'repository': args.repository, 'pr': args.pr, 'category': args.category,
        'head_sha': args.head_sha, 'merge_sha': pr.get('merge_commit_sha'),
        'head_branch': pr['head']['ref'], 'base_sha': pr['base']['sha'],
        'mode': args.mode, 'cohort': args.cohort, 'pair': pair, 'cache_state': args.cache_state,
        'requested_at': args.requested_at, 'captured_at': None,
        'metric': 'replay-request-to-last-physical-job-complete',
        'push_to_dispatch': 'unknown', 'runs': [], 'capture_errors': [],
    }
    selected = {}
    if args.run:
        for entry in args.run:
            workflow, run_id, attempt = run_spec(entry)
            if workflow in selected:
                raise ValueError('Duplicate workflow selection')
            selected[workflow] = (run_id, attempt)
    else:
        try:
            runs = api.pages(f'actions/runs?event=pull_request&head_sha={args.head_sha}', 'workflow_runs')
        except ReadError as error:
            preserve_read_error(error)
            raise
        for run in runs:
            if (run.get('head_branch') != pr['head']['ref']
                    or args.pr not in [item['number'] for item in run.get('pull_requests', [])]):
                continue
            workflow = Path(run['path'].split('@')[0]).name
            candidate = (integer(run['id']), integer(run.get('run_attempt', 1)))
            if candidate > selected.get(workflow, (0, 0)):
                selected[workflow] = candidate
    for workflow, (run_id, wanted_attempt) in selected.items():
        try:
            latest = api.request(f'actions/runs/{run_id}')
            attempt = wanted_attempt or integer(latest['run_attempt'])
            raw = api.request(f'actions/runs/{run_id}/attempts/{attempt}')
            run = {key: raw.get(key) for key in (
                'id', 'run_attempt', 'event', 'path', 'status', 'conclusion', 'head_sha',
                'head_branch', 'created_at', 'run_started_at', 'updated_at', 'html_url', 'pull_requests')}
            run.update(workflow=workflow, jobs=[], artifacts={})
            observation['runs'].append(run)
            jobs = api.pages(f'actions/runs/{run_id}/attempts/{attempt}/jobs', 'jobs')
            for raw_job in jobs:
                job = {key: raw_job.get(key) for key in (
                    'id', 'name', 'run_attempt', 'status', 'conclusion', 'started_at',
                    'completed_at', 'runner_name', 'runner_group_name', 'labels', 'html_url', 'steps')}
                job['identity'] = api.job_identity(raw_job)
                run['jobs'].append(job)
            # Reruns may remove earlier attempts from this listing. Missing
            # expected artifacts remain missing; never substitute another attempt.
            artifacts = api.pages(f'actions/runs/{run_id}/artifacts', 'artifacts')
            allowed = {f'ci-mode-{attempt}': 'mode.json'}
            if workflow == 'mobile.yml':
                allowed.update({f'mobile-ci-{lane}-{args.mode}-attempt-{attempt}': 'mobile.json'
                                for lane in PHASES})
                allowed[f'mobile-validation-attempt-{attempt}'] = 'mobile-validation.json'
            for artifact in artifacts:
                name = artifact['name']
                if name not in allowed:
                    continue
                if name in run['artifacts']:
                    raise ValueError('Duplicate expected artifact')
                if artifact.get('expired') or artifact.get('size_in_bytes', 0) > ARCHIVE_LIMIT:
                    raise ValueError('Timing artifact is expired or oversized')
                payload = api.request(f"actions/artifacts/{integer(artifact['id'])}/zip", binary=True)
                document = read_json_archive(payload, allowed[name])
                (destination / f"{run_id}-{attempt}-{integer(artifact['id'])}.zip").write_bytes(payload)
                run['artifacts'][name] = {
                    'id': artifact['id'], 'created_at': artifact.get('created_at'),
                    'sha256': hashlib.sha256(payload).hexdigest(), 'document': document,
                }
        except (KeyError, ValueError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
            observation['capture_errors'].append(f'{workflow}: {error}')
    observation['captured_at'] = datetime.now(timezone.utc).isoformat()
    observation['api_read_trace'] = list(getattr(api, 'read_trace', []))
    validated = validate_observation(observation)
    observation['assessment'] = validated
    (destination / 'observation.json').write_text(json.dumps(observation, indent=2, sort_keys=True) + '\n')
    return observation


def validate_observation(observation):
    errors = list(observation.get('capture_errors', []))
    identity = {'jobs': {}, 'mobile': None}
    last_completed = None
    physical = []
    intervals = []
    try:
        if observation.get('schema') != 1 or observation['category'] not in WORKFLOWS:
            raise ValueError('Unknown observation schema/category')
        integer(observation['pr'])
        if not IDENTIFIER.fullmatch(observation['cohort']) or not IDENTIFIER.fullmatch(observation['pair']):
            raise ValueError('Invalid observation cohort/pair')
        if (not SHA.fullmatch(observation['head_sha']) or not SHA.fullmatch(observation['merge_sha'])
                or observation['mode'] not in ('serial', 'candidate')
                or observation['cache_state'] not in ('warm', 'cold')):
            raise ValueError('Invalid observation source/mode/cache state')
        requested, captured = utc(observation['requested_at']), utc(observation['captured_at'])
        if requested > captured or captured > datetime.now(timezone.utc) + timedelta(seconds=1):
            raise ValueError('Observation timestamps are in the future or reversed')
        runs = observation['runs']
        complete = bool(runs) and all(run.get('status') == 'completed' for run in runs)
        paths = [run['workflow'] for run in runs]
        if len(set(paths)) != len(paths) or set(paths) != set(WORKFLOWS[observation['category']]):
            errors.append('Incomplete or unexpected workflow set')
        for run in runs:
            workflow = run['workflow']
            attempt, run_id = integer(run['run_attempt']), integer(run['id'])
            if (run.get('event') != 'pull_request' or run.get('head_sha') != observation['head_sha']
                    or run.get('head_branch') != observation['head_branch']
                    or Path(run.get('path', '').split('@')[0]).name != workflow
                    or observation['pr'] not in [item['number'] for item in run.get('pull_requests') or []]):
                errors.append(workflow + ': PR/source association mismatch')
            if run.get('status') != 'completed' or run.get('conclusion') != 'success':
                errors.append(workflow + ': workflow did not complete successfully')
            artifacts = run.get('artifacts', {})
            for artifact in artifacts.values():
                try:
                    if (utc(artifact.get('created_at')) < requested - timedelta(seconds=1)
                            or utc(artifact.get('created_at')) > captured + timedelta(seconds=1)):
                        raise ValueError('Artifact is stale or from the future')
                    integer(artifact.get('id'))
                except ValueError as error:
                    errors.append(workflow + ': ' + str(error))
            if workflow in WORKFLOWS['core']:
                mode = artifact_document(artifacts, f'ci-mode-{attempt}')
                expected = {'schema': 1, 'status': 'success', 'event': 'pull_request',
                            'repository': observation['repository'], 'pr': observation['pr'],
                            'head_repository': observation['repository'], 'head_sha': observation['head_sha'],
                            'merge_sha': observation['merge_sha'], 'run_id': run_id, 'run_attempt': attempt,
                            'ci_mode': observation['mode'], 'qualification_id': observation['cohort'],
                            'dependency_cache': observation['cache_state']}
                if any(mode.get(k) != v for k, v in expected.items()):
                    errors.append(workflow + ': actual mode receipt mismatch or missing')
            executed = []
            for job in run.get('jobs', []):
                if job.get('conclusion') == 'skipped':
                    continue
                if job.get('status') != 'completed':
                    complete = False
                try:
                    if not job.get('runner_name') or not job.get('steps'):
                        intervals.append({'workflow': workflow, 'job': job.get('name'),
                                          'physical': False, 'execution_seconds': None,
                                          'conclusion': job.get('conclusion')})
                        raise ValueError('Job never acquired an observed physical runner')
                    started, completed = utc(job.get('started_at')), utc(job.get('completed_at'))
                    if (started < requested - timedelta(seconds=1) or completed < started
                            or completed > captured + timedelta(seconds=1)):
                        raise ValueError('Carried-forward or invalid physical job timestamps')
                    last_completed = max(last_completed or completed, completed)
                    physical.append(integer(job['id']))
                    intervals.append({'workflow': workflow, 'job': job['name'],
                                      'physical': True,
                                      'request_to_start_seconds': (started - requested).total_seconds(),
                                      'execution_seconds': (completed - started).total_seconds()})
                except ValueError as error:
                    errors.append(workflow + ': ' + str(error))
                if (job.get('status') != 'completed' or job.get('conclusion') != 'success'
                        or not job.get('runner_name') or not job.get('steps')):
                    errors.append(workflow + ': selected job did not physically succeed: ' + str(job.get('name')))
                meta = job.get('identity', {})
                if not meta.get('image') or not meta.get('image_version'):
                    errors.append(workflow + ': missing observed runner image: ' + str(job.get('name')))
                steps = job.get('steps') or []
                rust_steps = [step for step in steps if 'rust-cache' in step['name'] or 'Restore mobile Rust' in step['name']]
                rust = bool(rust_steps)
                node = any('setup-node' in step['name'] for step in steps)
                if rust and not meta.get('rustc'):
                    errors.append(workflow + ': missing observed Rust compiler identity')
                if rust and observation['cache_state'] == 'warm' and (
                        not meta.get('rust_key') or meta.get('rust_exact') is not True):
                    errors.append(workflow + ': missing/mismatched exact Rust restore evidence')
                if rust and observation['cache_state'] == 'cold' and (
                        any(step.get('conclusion') != 'skipped' for step in rust_steps) or meta.get('rust_key')):
                    errors.append(workflow + ': controlled cold Rust cache was not bypassed')
                if node and (not meta.get('node') or not meta.get('npm')):
                    errors.append(workflow + ': missing observed Node/npm identity')
                if node and observation['cache_state'] == 'warm' and not meta.get('node_cache_restored'):
                    errors.append(workflow + ': missing observed npm dependency restore')
                if node and observation['cache_state'] == 'cold' and meta.get('node_cache_keys'):
                    errors.append(workflow + ': controlled cold npm cache was not bypassed')
                role = 'mobile-native' if workflow == 'mobile.yml' and rust else job['name']
                fingerprint = {k: meta.get(k) for k in ('image', 'image_version', 'rustc', 'node', 'npm', 'python', 'rust_key', 'node_cache_keys')}
                key = workflow + ':' + role
                previous = identity['jobs'].get(key)
                if previous is not None and previous != fingerprint:
                    errors.append(workflow + ': isolated native lanes have differing identities')
                identity['jobs'][key] = fingerprint
                executed.append(job)
            if not executed:
                errors.append(workflow + ': no physical jobs observed')
            errors.extend(coverage_errors(observation['category'], observation['mode'], workflow, run.get('jobs', [])))
            if workflow == 'mobile.yml':
                native, mobile_errors = mobile_identity(observation, run)
                errors.extend(mobile_errors)
                identity['mobile'] = native
        if len(physical) != len(set(physical)):
            errors.append('Duplicate physical job evidence')
        elapsed = (last_completed - requested).total_seconds() if complete and last_completed else None
        lower_bound = elapsed if complete else (captured - requested).total_seconds()
        identity.update(repository=observation['repository'], head_sha=observation['head_sha'],
                        merge_sha=observation['merge_sha'], base_sha=observation['base_sha'])
        return {'valid': not errors, 'errors': errors, 'elapsed_seconds': elapsed,
                'elapsed_lower_bound_seconds': lower_bound,
                'target_miss': (elapsed > TARGETS[observation['category']]) if elapsed is not None else (
                    True if lower_bound is not None and lower_bound > TARGETS[observation['category']] else None),
                'matching_identity': identity, 'physical_job_ids': physical, 'intervals': intervals}
    except (KeyError, TypeError, ValueError, AttributeError) as error:
        return {'valid': False, 'errors': errors + ['Malformed observation: ' + str(error)],
                'elapsed_seconds': None, 'elapsed_lower_bound_seconds': None,
                'target_miss': None, 'matching_identity': None,
                'physical_job_ids': physical, 'intervals': intervals}


def artifact_document(artifacts, name):
    value = artifacts.get(name, {}).get('document', {})
    return value if isinstance(value, dict) else {}


def coverage_errors(category, mode, workflow, jobs):
    errors = []
    gate = {'ci.yml': 'required', 'mobile.yml': 'Complete mobile validation'}.get(workflow)
    if gate and not any(j.get('name') == gate and j.get('conclusion') == 'success' for j in jobs):
        errors.append(workflow + ': final required result did not succeed')
    if workflow == 'desktop.yml':
        apps = [j for j in jobs if j.get('name', '').startswith('app (')]
        selected = category in ('desktop', 'core')
        if selected and (len(apps) != 3 or any(j.get('conclusion') != 'success' for j in apps)):
            errors.append('Desktop: all three installer OS jobs must succeed')
        if not selected and any(j.get('conclusion') != 'skipped' for j in apps):
            errors.append('Desktop: observed installer scope differs from requested category')
        for job in apps if selected else []:
            steps = {s['name']: s.get('conclusion') for s in job.get('steps') or []}
            names = ('Frontend tests', 'Rust format', 'Installers with verified non-Mac x64 CPU baseline')
            names += ('Rust lint', 'Rust tests', 'Frontend') if mode == 'serial' else (
                'Rust lint after packaging', 'Rust tests after packaging')
            if any(steps.get(name) != 'success' for name in names):
                errors.append('Desktop: expected frontend/backend/packaging checks missing')
            if mode == 'candidate' and steps.get('Frontend') != 'skipped':
                errors.append('Desktop: duplicate frontend build was not skipped')
    if category == 'core':
        if workflow == 'ci.yml' and not any(j.get('name') == 'build-and-test'
                                            and j.get('conclusion') == 'success' for j in jobs):
            errors.append('Core: macOS core validation missing')
        if workflow == 'desktop.yml' and len([j for j in jobs if j.get('name', '').startswith('core (')
                                             and j.get('conclusion') == 'success']) != 2:
            errors.append('Core: Windows/Linux core validation missing')
        if workflow == 'decision-evaluation.yml' and len([j for j in jobs if j.get('name', '').startswith('offline (')
                                                          and j.get('conclusion') == 'success']) != 3:
            errors.append('Core: three-OS evaluation validation missing')
        required_steps = {
            'ci.yml': ('Rust format', 'Rust lint', 'Rust tests', 'Local keyboard policy',
                       'Rust lint without local models', 'Rust tests without local models', 'Speaker pipeline'),
            'desktop.yml': ('Rust lint', 'Rust tests', 'Local keyboard policy',
                            'Rust lint and tests without local speaker recognition', 'Speaker pipeline'),
            'decision-evaluation.yml': ('Build evaluation-only bridge without native models',
                                        'Evaluation Rust checks', 'Offline corpus, metrics and mocked transports',
                                        'Real deterministic baselines and reproducible report'),
        }.get(workflow, ())
        selected_jobs = [j for j in jobs if (workflow == 'ci.yml' and j.get('name') == 'build-and-test')
                         or (workflow == 'desktop.yml' and j.get('name', '').startswith('core ('))
                         or (workflow == 'decision-evaluation.yml' and j.get('name', '').startswith('offline ('))]
        for job in selected_jobs:
            steps = {s['name']: s.get('conclusion') for s in job.get('steps') or []}
            if any(steps.get(name) != 'success' for name in required_steps):
                errors.append('Core: required validation steps missing: ' + job['name'])
    elif workflow == 'ci.yml':
        for job in jobs:
            if (job.get('name') == 'build-and-test'
                    or (category in ('docs', 'mobile') and job.get('name') == 'platform')):
                if job.get('conclusion') != 'skipped':
                    errors.append('Core: observed native scope differs from requested category')
    return errors


def mobile_identity(observation, run):
    errors = []
    attempt, run_id = integer(run['run_attempt']), integer(run['id'])
    lanes = ('serial',) if observation['mode'] == 'serial' else ('release', 'runtime')
    identities = []
    documents = []
    for lane in lanes:
        doc = artifact_document(run['artifacts'], f"mobile-ci-{lane}-{observation['mode']}-attempt-{attempt}")
        documents.append(doc)
        validation, provenance, metadata = doc.get('validation', {}), doc.get('provenance', {}), doc.get('metadata', {})
        expected = {'ci_mode': observation['mode'], 'lane': lane, 'qualification_id': observation['cohort'],
                    'dependency_cache': observation['cache_state'],
                    'repository': observation['repository'], 'event_name': 'pull_request',
                    'source_sha': observation['merge_sha'], 'head_sha': observation['head_sha'],
                    'job_id': 'ios-native-tests' if lane == 'runtime' else 'ios-generated-build',
                    'expected_phases': list(PHASES[lane])}
        if (doc.get('schema') != 2 or any(validation.get(k) != v for k, v in expected.items())
                or str(provenance.get('run_id')) != str(run_id)
                or str(provenance.get('run_attempt')) != str(attempt)
                or metadata.get('revision') != observation['merge_sha']
                or doc.get('cache', {}).get('mode') != 'off' or doc.get('validation_errors') != []):
            errors.append('Mobile: lane source/mode/provenance/validation mismatch: ' + lane)
        phases = doc.get('phases') or []
        if ([p.get('name') for p in phases] != list(PHASES[lane])
                or any(p.get('exit_code') != 0 or isinstance(p.get('exit_code'), bool)
                       or not isinstance(p.get('seconds'), (int, float)) or isinstance(p.get('seconds'), bool)
                       or not math.isfinite(p['seconds']) or p['seconds'] < 0 for p in phases)):
            errors.append('Mobile: incomplete or failed native phases: ' + lane)
        rust_cache = doc.get('rust_cache', {})
        if observation['cache_state'] == 'warm' and rust_cache.get('exact_hit') is not True:
            errors.append('Mobile: exact dependency cache restore not proven: ' + lane)
        if observation['cache_state'] == 'cold' and (
                rust_cache.get('bypassed') is not True or rust_cache.get('exact_hit') is not None
                or rust_cache.get('bypass_scope') != ['rust', 'npm']):
            errors.append('Mobile: controlled dependency cache bypass not proven: ' + lane)
        fields = ('xcode', 'device_sdk', 'simulator_sdk', 'rustc', 'architecture', 'variant', 'dependency_sha256')
        if any(not metadata.get(key) for key in fields) or not provenance.get('image_version'):
            errors.append('Mobile: missing observed native input identity: ' + lane)
        identities.append({**{key: metadata.get(key) for key in fields},
                           'image_os': provenance.get('image_os'), 'image_version': provenance.get('image_version')})
    if identities and any(i != identities[0] for i in identities[1:]):
        errors.append('Mobile: native lane fingerprints differ')
    aggregate = artifact_document(run['artifacts'], f'mobile-validation-attempt-{attempt}')
    expected = {'schema': 1, 'status': 'success', 'ci_mode': observation['mode'],
                'dependency_cache': observation['cache_state'],
                'qualification_id': observation['cohort'], 'repository': observation['repository'],
                'event_name': 'pull_request', 'source_sha': observation['merge_sha'],
                'head_sha': observation['head_sha'], 'expected_lanes': list(lanes), 'errors': []}
    if (any(aggregate.get(k) != v for k, v in expected.items())
            or str(aggregate.get('run_id')) != str(run_id)
            or str(aggregate.get('run_attempt')) != str(attempt)
            or not isinstance(aggregate.get('receipts'), list)
            or sorted(canonical(r) for r in aggregate['receipts']) != sorted(canonical(r) for r in documents)):
        errors.append('Mobile: complete aggregate receipt missing or mismatched')
    return identities[0] if identities else None, errors


def assess(observations):
    evaluated = []
    seen = set()
    for observation in observations:
        result = validate_observation(observation)
        physical = tuple(sorted(result['physical_job_ids']))
        if physical and any(job in seen for job in physical):
            result['valid'] = False
            result['errors'].append('Physical jobs reused across observations')
        seen.update(physical)
        evaluated.append({'observation': observation, 'result': result})
    categories = {}
    for category in WORKFLOWS:
        gates = {}
        for cache_state in ('warm', 'cold'):
            relevant = [r for r in evaluated if r['observation'].get('category') == category
                        and r['observation'].get('cache_state') == cache_state]
            pairs = {}
            for row in relevant:
                if not row['result']['valid']:
                    continue
                observation = row['observation']
                pairs.setdefault(observation.get('pair'), {}).setdefault(observation.get('mode'), []).append(row)
            matched, rejected = [], []
            blocking_pairs = False
            for pair, modes in pairs.items():
                if set(modes) != {'serial', 'candidate'} or any(len(v) != 1 for v in modes.values()):
                    rejected.append({'pair': pair, 'reason': 'Missing or duplicate mode observations'})
                    blocking_pairs = True
                    continue
                serial, candidate = modes['serial'][0], modes['candidate'][0]
                if canonical(serial['result']['matching_identity']) != canonical(candidate['result']['matching_identity']):
                    rejected.append({'pair': pair, 'reason': 'Source/image/toolchain/cache identities differ'})
                    continue
                matched.append({'pair': pair, 'serial_seconds': serial['result']['elapsed_seconds'],
                                'candidate_seconds': candidate['result']['elapsed_seconds']})
            controls = [p['serial_seconds'] for p in matched]
            candidates = [p['candidate_seconds'] for p in matched]
            control_median = statistics.median(controls) if controls else None
            candidate_median = statistics.median(candidates) if candidates else None
            timing_pass = (candidate_median is not None and candidate_median <= TARGETS[category]) if cache_state == 'warm' else (
                control_median is not None and control_median > 0 and candidate_median <= control_median * 1.1)
            gates[cache_state] = {'qualified': len(matched) >= 5 and timing_pass
                                  and not blocking_pairs and all(r['result']['valid'] for r in relevant),
                                  'matched_pairs': matched, 'rejected_pairs': rejected,
                                  'control_median_seconds': control_median,
                                  'candidate_median_seconds': candidate_median,
                                  'required_pairs': 5, 'target_seconds': TARGETS[category] if cache_state == 'warm' else None,
                                  'max_regression_ratio': 1.1 if cache_state == 'cold' else None,
                                  'observations': [{'cohort': r['observation'].get('cohort'),
                                                    'mode': r['observation'].get('mode'), **r['result']} for r in relevant]}
        categories[category] = gates
    return {'schema': 1, 'rollout_qualified': all(g['qualified'] for c in categories.values() for g in c.values()),
            'metric': 'replay-request-to-last-physical-job-complete', 'push_to_dispatch': 'unknown',
            'categories': categories, 'observation_count': len(evaluated),
            'limits': 'No p95, maximum, guaranteed dispatch deadline or native-CAS adoption is established.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    collect = commands.add_parser('capture', help='Archive attempt evidence before rerunning workflows',
                                  description='Archive complete attempt evidence before another rerun; '
                                  'previous-attempt artifacts may disappear and are never substituted.')
    collect.add_argument('--repository', required=True)
    collect.add_argument('--pr', type=int, required=True)
    collect.add_argument('--category', choices=WORKFLOWS, required=True)
    collect.add_argument('--head-sha', required=True)
    collect.add_argument('--mode', choices=('serial', 'candidate'), required=True)
    collect.add_argument('--cohort', required=True)
    collect.add_argument('--pair')
    collect.add_argument('--cache-state', choices=('warm', 'cold'), default='warm')
    collect.add_argument('--requested-at', required=True)
    collect.add_argument('--run', action='append', help='Pin workflow.yml=RUN_ID:ATTEMPT; archive before rerunning it')
    collect.add_argument('--output', required=True)
    report = commands.add_parser('assess')
    report.add_argument('observations', nargs='+')
    report.add_argument('--output', required=True)
    args = parser.parse_args()
    if args.command == 'capture':
        value = capture(args)
        print(json.dumps({'observation': str(Path(args.output) / 'observation.json'), **value['assessment']}, indent=2))
        return 0 if value['assessment']['valid'] else 2
    observations = []
    for name in args.observations:
        path = Path(name)
        if path.is_dir():
            path /= 'observation.json'
        observations.append(json.loads(path.read_text()))
    value = assess(observations)
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'report': str(output), 'rollout_qualified': value['rollout_qualified'],
                      'observation_count': value['observation_count']}, indent=2))
    return 0 if value['rollout_qualified'] else 2


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (KeyError, ValueError, TypeError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
