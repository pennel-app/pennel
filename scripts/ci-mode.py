#!/usr/bin/env python3
"""Select a benchmark-only CI treatment; ordinary validation stays serial."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys


REPOSITORY = re.compile(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+')
SHA = re.compile(r'[0-9a-f]{40}')
COHORT = re.compile(r'[A-Za-z0-9][A-Za-z0-9._-]{0,63}')
EVENT = re.compile(r'[a-z][a-z0-9_]{0,63}')
CONFIG_FIELDS = {'repository', 'pr', 'head_sha', 'mode', 'cohort'}
OPTIONAL_CONFIG_FIELDS = {'dependency_cache'}


def positive_integer(value):
    return isinstance(value, int) and not isinstance(value, bool) and 0 < value < 2 ** 63


def context(environ, payload):
    event = environ.get('GITHUB_EVENT_NAME', '')
    repository = environ.get('GITHUB_REPOSITORY', '')
    merge_sha = environ.get('GITHUB_SHA', '')
    if not EVENT.fullmatch(event) or not REPOSITORY.fullmatch(repository):
        raise ValueError('Invalid workflow event or repository identity')
    if not SHA.fullmatch(merge_sha):
        raise ValueError('Invalid workflow source identity')
    run_values = []
    for name in ('GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT'):
        value = environ.get(name, '')
        if not isinstance(value, str) or not value.isascii() or not value.isdecimal():
            raise ValueError('Invalid workflow run identity')
        number = int(value)
        if not positive_integer(number):
            raise ValueError('Invalid workflow run identity')
        run_values.append(number)
    result = {
        'schema': 1, 'status': 'success', 'event': event, 'repository': repository,
        'pr': None, 'head_repository': repository, 'head_sha': merge_sha,
        'merge_sha': merge_sha, 'run_id': run_values[0], 'run_attempt': run_values[1],
        'ci_mode': 'serial', 'qualification_id': '', 'dependency_cache': 'warm',
    }
    if event != 'pull_request':
        return result
    try:
        pr = payload['pull_request']
        number = payload['number']
        pr_number = pr['number']
        if (not positive_integer(number) or not positive_integer(pr_number)
                or pr_number != number):
            raise ValueError('Invalid pull request identity')
        head_sha = pr['head']['sha']
        if not isinstance(head_sha, str) or not SHA.fullmatch(head_sha):
            raise ValueError('Invalid pull request source identity')
        if pr['base']['repo']['full_name'] != repository:
            raise ValueError('Pull request base repository does not match workflow')
        head_repository = (pr['head'].get('repo') or {}).get('full_name')
        if head_repository is not None and (
                not isinstance(head_repository, str) or not REPOSITORY.fullmatch(head_repository)):
            raise ValueError('Invalid pull request head repository identity')
    except (KeyError, TypeError, AttributeError):
        raise ValueError('Incomplete pull request identity') from None
    result.update(pr=number, head_sha=head_sha, head_repository=head_repository)
    return result


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('Duplicate qualification configuration field')
        result[key] = value
    return result


def select_mode(environ, payload):
    result = context(environ, payload)
    force = environ.get('PENNEL_CI_FORCE_SERIAL', '')
    if force not in ('', 'false', 'true'):
        raise ValueError('Invalid force-serial setting')
    if force == 'true':
        return {**result, 'reason': 'force-serial'}
    if result['event'] != 'pull_request':
        return {**result, 'reason': 'non-pr'}
    if result['head_repository'] != result['repository']:
        return {**result, 'reason': 'foreign-head'}
    raw = environ.get('PENNEL_CI_QUALIFICATION_CONFIG', '')
    if not raw:
        return {**result, 'reason': 'unconfigured'}
    if len(raw) > 4096:
        raise ValueError('Qualification configuration is too large')
    try:
        config = json.loads(raw, object_pairs_hook=unique_object)
    except json.JSONDecodeError:
        raise ValueError('Invalid qualification configuration JSON') from None
    if not isinstance(config, dict):
        raise ValueError('Qualification configuration must be an object')
    repository = config.get('repository')
    number = config.get('pr')
    if (not isinstance(repository, str) or not REPOSITORY.fullmatch(repository)
            or not positive_integer(number)):
        raise ValueError('Invalid qualification scope')
    # A scoped experiment must not change or block unrelated PRs.
    if repository != result['repository'] or number != result['pr']:
        return {**result, 'reason': 'outside-scope'}
    if (not CONFIG_FIELDS.issubset(config)
            or set(config) - CONFIG_FIELDS - OPTIONAL_CONFIG_FIELDS):
        raise ValueError('Missing or unknown qualification configuration fields')
    head_sha = config['head_sha']
    if not isinstance(head_sha, str) or not SHA.fullmatch(head_sha):
        raise ValueError('Invalid qualification source identity')
    if head_sha != result['head_sha']:
        raise ValueError('Qualification source does not match the designated PR')
    if config['mode'] not in ('serial', 'candidate'):
        raise ValueError('Invalid qualification mode')
    cohort = config['cohort']
    if not isinstance(cohort, str) or not COHORT.fullmatch(cohort):
        raise ValueError('Invalid qualification cohort identity')
    dependency_cache = config.get('dependency_cache', 'warm')
    if dependency_cache not in ('warm', 'cold'):
        raise ValueError('Invalid qualification dependency-cache setting')
    return {**result, 'ci_mode': config['mode'], 'qualification_id': cohort,
            'dependency_cache': dependency_cache, 'reason': 'qualification'}


def failure_receipt(environ, payload, error):
    try:
        result = context(environ, payload)
    except ValueError:
        result = {'schema': 1, 'event': 'invalid', 'repository': None,
                  'pr': None, 'head_repository': None, 'head_sha': None,
                  'merge_sha': None, 'run_id': None, 'run_attempt': None}
    return {**result, 'status': 'error', 'reason': str(error),
            'ci_mode': 'serial', 'qualification_id': '', 'dependency_cache': 'warm'}


def write_result(result, receipt, environ):
    result['recorded_at'] = datetime.now(timezone.utc).isoformat()
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    if environ.get('GITHUB_OUTPUT'):
        with open(environ['GITHUB_OUTPUT'], 'a') as target:
            target.write('ci_mode=' + result['ci_mode'] + '\n')
            target.write('qualification_id=' + result['qualification_id'] + '\n')
            target.write('dependency_cache=' + result['dependency_cache'] + '\n')
    summary = ('### CI schedule selection\n\n'
               f"- Actual mode: `{result['ci_mode']}`\n"
               f"- Selection: `{result['reason']}`\n"
               f"- Status: `{result['status']}`\n"
               f"- Qualification cohort: `{result['qualification_id'] or 'none'}`\n"
               f"- Dependency-cache selection: `{result['dependency_cache']}`\n"
               f"- Head / merge: `{result['head_sha']}` / `{result['merge_sha']}`\n\n"
               'This records the observed selection; it does not prove that mutable '
               'configuration was reevaluated on a rerun.\n')
    if environ.get('GITHUB_STEP_SUMMARY'):
        with open(environ['GITHUB_STEP_SUMMARY'], 'a') as target:
            target.write(summary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipt', type=Path, default=Path('build/ci-performance/mode.json'))
    args = parser.parse_args()
    payload = {}
    try:
        event_path = os.environ.get('GITHUB_EVENT_PATH')
        if event_path:
            try:
                payload = json.loads(Path(event_path).read_text())
            except (OSError, json.JSONDecodeError):
                raise ValueError('Cannot read workflow event metadata') from None
        result = select_mode(os.environ, payload)
    except ValueError as error:
        result = failure_receipt(os.environ, payload, error)
    write_result(result, args.receipt, os.environ)
    print(f"CI schedule: {result['ci_mode']} ({result['reason']})")
    return int(result['status'] != 'success')


if __name__ == '__main__':
    sys.exit(main())
