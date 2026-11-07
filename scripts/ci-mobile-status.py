#!/usr/bin/env python3
"""Require the complete selected mobile pipeline and same-attempt phase evidence."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import sys


HELPER = Path(__file__).with_name('ci-xcode-cache.py')
SPEC = importlib.util.spec_from_file_location('ci_xcode_cache', HELPER)
CACHE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CACHE)


def observation_errors(receipt, source_sha, run_id, run_attempt, allow_prior_attempts=False):
    if not isinstance(receipt, dict):
        return ['Invalid mobile receipt']
    errors = []
    metadata = receipt.get('metadata', {})
    if not isinstance(metadata, dict) or metadata.get('revision') != source_sha:
        errors.append('Prepared mobile source does not match this run')
    provenance = receipt.get('provenance', {})
    if not isinstance(provenance, dict):
        return [*errors, 'Invalid mobile receipt provenance']
    if str(provenance.get('run_id')) != str(run_id):
        errors.append('Mobile receipt belongs to another run')
    observed_attempt = str(provenance.get('run_attempt'))
    current_attempt = str(run_attempt)
    if allow_prior_attempts:
        matched_attempt = (re.fullmatch(r'[1-9][0-9]*', observed_attempt)
                           and re.fullmatch(r'[1-9][0-9]*', current_attempt)
                           and int(observed_attempt) <= int(current_attempt))
    else:
        matched_attempt = observed_attempt == current_attempt
    if not matched_attempt:
        errors.append('Mobile receipt belongs to another attempt')
    return errors


def failures(needs, receipts, *, event_name, repository, source_sha, head_sha,
             run_id, run_attempt, cache_mode='off', repetitions=1):
    if not isinstance(needs, dict) or not isinstance(receipts, list):
        return ['Expected mobile jobs and receipt list']
    errors = []
    if (not isinstance(repository, str) or not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository)
            or not isinstance(source_sha, str) or not re.fullmatch(r'[0-9a-f]{40}', source_sha)
            or not isinstance(head_sha, str) or not re.fullmatch(r'[0-9a-f]{40}', head_sha)
            or not re.fullmatch(r'[1-9][0-9]*', str(run_id))
            or not re.fullmatch(r'[1-9][0-9]*', str(run_attempt))):
        errors.append('Missing or invalid current mobile provenance')
    changes = needs.get('changes', {})
    if not isinstance(changes, dict) or changes.get('result') != 'success':
        return [*errors, 'Mobile mode selection did not succeed']
    outputs = changes.get('outputs', {})
    if not isinstance(outputs, dict):
        return [*errors, 'Missing mobile mode outputs']
    mode = outputs.get('ci_mode')
    if mode not in ('serial', 'candidate'):
        return [*errors, 'Missing or invalid selected mobile mode']
    dependency_cache = outputs.get('dependency_cache')
    if dependency_cache not in ('warm', 'cold'):
        return [*errors, 'Missing or invalid dependency-cache policy']
    qualification_id = outputs.get('qualification_id', '')
    if not isinstance(qualification_id, str) or (qualification_id and not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}', qualification_id)):
        errors.append('Invalid mobile qualification identifier')
    manual = event_name == 'workflow_dispatch'
    if event_name not in ('workflow_dispatch', 'pull_request', 'push'):
        errors.append('Unexpected mobile event')
    if manual and mode != 'serial':
        errors.append('Manual compiler-cache experiments must remain serial')
    if manual and dependency_cache != 'warm':
        errors.append('Manual compiler-cache experiments cannot bypass dependency caches')
    selected_runtime = mode == 'candidate' and not manual
    required = {'ios-generated-build': 'success',
                'ios-native-tests': 'success' if selected_runtime else 'skipped'}
    for name, expected in required.items():
        job = needs.get(name, {})
        result = job.get('result') if isinstance(job, dict) else None
        if result != expected:
            errors.append(f'{name}: expected {expected}, got {result or "missing result"}')

    if manual:
        try:
            if not isinstance(repetitions, int) or isinstance(repetitions, bool):
                raise ValueError('Invalid repetition count')
            CACHE.validate_inputs(cache_mode, repetitions, 'pilot-1')
        except (ValueError, TypeError):
            return [*errors, 'Invalid legacy compiler-cache mode or repetitions']
        found = []
        for receipt in receipts:
            # Successful manual matrix siblings may be carried forward by a retry;
            # these remain their original observations, not fresh PR qualification.
            errors.extend(observation_errors(receipt, source_sha, run_id, run_attempt, allow_prior_attempts=True))
            if not isinstance(receipt, dict):
                continue
            if receipt.get('schema') != 1 or 'validation' in receipt:
                errors.append('Manual experiment receipt contract changed')
            repetition = str(receipt.get('repetition'))
            found.append(repetition)
            expected_phases = list(CACHE.MOBILE_PHASES['serial'])
            if cache_mode != 'off':
                expected_phases.append('cache-pack' if cache_mode == 'cold' else 'cache-unpack')
            errors.extend(CACHE.phase_errors(receipt.get('phases'), expected_phases))
            cache = receipt.get('cache', {})
            if not isinstance(cache, dict) or cache.get('mode') != cache_mode:
                errors.append('Manual receipt has another compiler-cache mode')
                continue
            if cache_mode == 'warm' and cache.get('exact_hit') is not True:
                errors.append('Warm manual receipt did not restore its exact seed')
            if cache_mode == 'cold' and (cache.get('save_allowed') is not True
                                          or not isinstance(cache.get('archive_bytes'), int)
                                          or isinstance(cache.get('archive_bytes'), bool)
                                          or cache['archive_bytes'] <= 0):
                errors.append('Cold manual receipt did not qualify seed storage')
        if sorted(found) != [str(i) for i in range(1, repetitions + 1)]:
            errors.append('Missing, duplicate or unexpected manual repetitions')
        return errors

    if cache_mode != 'off' or repetitions != 1 or isinstance(repetitions, bool):
        errors.append('Ordinary mobile runs cannot select a compiler-cache experiment')
    lanes = ('release', 'runtime') if selected_runtime else ('serial',)
    found = []
    for receipt in receipts:
        errors.extend(observation_errors(receipt, source_sha, run_id, run_attempt))
        if not isinstance(receipt, dict):
            continue
        validation = receipt.get('validation', {})
        if receipt.get('schema') != 2 or not isinstance(validation, dict):
            errors.append('Missing ordinary mobile validation contract')
            continue
        lane = validation.get('lane')
        found.append(lane)
        if lane not in lanes:
            errors.append('Unexpected ordinary mobile validation lane')
            continue
        expected = {
            'ci_mode': mode, 'dependency_cache': dependency_cache, 'qualification_id': qualification_id,
            'repository': repository, 'event_name': event_name,
            'source_sha': source_sha, 'head_sha': head_sha,
            'job_id': 'ios-native-tests' if lane == 'runtime' else 'ios-generated-build',
            'expected_phases': list(CACHE.MOBILE_PHASES[lane]),
        }
        for name, value in expected.items():
            if validation.get(name) != value:
                errors.append(f'{lane}: mismatched {name}')
        errors.extend(CACHE.phase_errors(receipt.get('phases'), expected['expected_phases']))
        cache = receipt.get('cache', {})
        if not isinstance(cache, dict) or cache.get('mode') != 'off':
            errors.append(f'{lane}: ordinary validation enabled compiler caching')
        if receipt.get('validation_errors'):
            errors.append(f'{lane}: timing receipt reported validation errors')
        rust_cache = receipt.get('rust_cache', {})
        if dependency_cache == 'cold':
            if (not isinstance(rust_cache, dict) or rust_cache.get('bypassed') is not True
                    or rust_cache.get('exact_hit') is not None or rust_cache.get('bypass_scope') != ['rust', 'npm']):
                errors.append(f'{lane}: controlled cold dependency bypass was not observed')
        elif isinstance(rust_cache, dict) and rust_cache.get('bypassed'):
            errors.append(f'{lane}: warm dependency validation unexpectedly bypassed caches')
    if sorted(str(lane) for lane in found) != sorted(lanes):
        errors.append('Missing, duplicate or unexpected ordinary mobile receipts')
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipts', type=Path, default=Path('build/ci-performance/receipts'))
    parser.add_argument('--output', type=Path, default=Path('build/ci-performance/mobile-validation.json'))
    args = parser.parse_args()
    receipts = []
    needs = {}
    repetitions = None
    try:
        needs = json.loads(os.environ['CI_NEEDS_JSON'])
        for path in sorted(args.receipts.rglob('mobile.json')):
            receipts.append(json.loads(path.read_text()))
        repetitions = int(os.environ.get('CACHE_REPETITIONS', '1'))
        errors = failures(
            needs, receipts, event_name=os.environ['GITHUB_EVENT_NAME'],
            repository=os.environ['GITHUB_REPOSITORY'], source_sha=os.environ['GITHUB_SHA'],
            head_sha=os.environ['PENNEL_CI_HEAD_SHA'], run_id=os.environ['GITHUB_RUN_ID'],
            run_attempt=os.environ['GITHUB_RUN_ATTEMPT'],
            cache_mode=os.environ.get('PENNEL_XCODE_CACHE_MODE', 'off'),
            repetitions=repetitions,
        )
    except (KeyError, ValueError, TypeError, AttributeError, OSError) as error:
        errors = [f'Invalid mobile validation evidence: {type(error).__name__}']
    changes = needs.get('changes', {}) if isinstance(needs, dict) else {}
    outputs = changes.get('outputs', {}) if isinstance(changes, dict) else {}
    outputs = outputs if isinstance(outputs, dict) else {}
    mode = outputs.get('ci_mode')
    mode = mode if mode in ('serial', 'candidate') else None
    dependency_cache = outputs.get('dependency_cache')
    dependency_cache = dependency_cache if dependency_cache in ('warm', 'cold') else None
    qualification_id = outputs.get('qualification_id', '')
    if not isinstance(qualification_id, str) or (qualification_id and not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}', qualification_id)):
        qualification_id = None
    if os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch':
        lanes = [f'legacy-{i}' for i in range(1, repetitions + 1)] if repetitions in (1, 3) else []
    else:
        lanes = ['release', 'runtime'] if mode == 'candidate' else ['serial'] if mode == 'serial' else []
    data = {'schema': 1, 'status': 'failure' if errors else 'success',
            'ci_mode': mode, 'dependency_cache': dependency_cache, 'qualification_id': qualification_id,
            'repository': os.environ.get('GITHUB_REPOSITORY'),
            'event_name': os.environ.get('GITHUB_EVENT_NAME'),
            'source_sha': os.environ.get('GITHUB_SHA'),
            'head_sha': os.environ.get('PENNEL_CI_HEAD_SHA'),
            'run_id': os.environ.get('GITHUB_RUN_ID'),
            'run_attempt': os.environ.get('GITHUB_RUN_ATTEMPT'),
            'expected_lanes': lanes, 'errors': errors, 'receipts': receipts}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2) + '\n')
    for error in errors:
        print(error, file=sys.stderr)
    if not errors:
        print('Every selected mobile lane and expected phase passed for this source and attempt')
    return int(bool(errors))


if __name__ == '__main__':
    sys.exit(main())
