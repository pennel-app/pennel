#!/usr/bin/env python3
"""Require successful repository checks for the selected immutable TestFlight source."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
from urllib.parse import quote


WORKFLOWS = ('ci.yml', 'desktop.yml', 'mobile.yml')


def validate_ref(value):
    if (not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', value)
            or '..' in value or value.rstrip('/') in ('codex/ios-testflight', 'refs/heads/codex/ios-testflight')):
        raise ValueError('Select a source commit, branch or tag from this repository, not the generated build branch')
    return value


def latest_run(runs, repository, sha, workflow):
    matches = [run for run in runs if run.get('head_sha') == sha
               and run.get('event') in ('push', 'workflow_dispatch')
               and run.get('head_repository', {}).get('full_name') == repository
               and run.get('path', '').split('@')[0] == '.github/workflows/' + workflow]
    return max(matches, key=lambda run: (int(run['id']), int(run.get('run_attempt', 1))), default=None)


class GitHub:
    def __init__(self, repository):
        if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
            raise ValueError('Invalid repository')
        self.prefix = 'repos/' + repository + '/'

    def request(self, endpoint, method='GET', body=None):
        command = ['gh', 'api', '--method', method, self.prefix + endpoint]
        payload = None
        if body is not None:
            command.extend(['--input', '-'])
            payload = json.dumps(body)
        for attempt in range(3):
            result = subprocess.run(command, input=payload, text=True, capture_output=True)
            if not result.returncode:
                return json.loads(result.stdout) if result.stdout.strip() else {}
            matched = re.search(r'\(HTTP ([0-9]{3})\)', result.stderr)
            status = int(matched[1]) if matched else None
            transient = status == 429 or (status is not None and status >= 500) or (status is None and result.returncode == 1)
            if method != 'GET' or not transient or attempt == 2:
                # Keep remote payloads and credential-bearing diagnostics out of receipts.
                detail = f'HTTP {status}' if status else f'exit {result.returncode}'
                raise RuntimeError(f'GitHub API {method} {endpoint.split("?")[0]} failed ({detail})')
            time.sleep(2 ** attempt)

    def post(self, endpoint, body=None):
        return self.request(endpoint, 'POST', body or {})

    def runs(self, workflow, sha):
        results = []
        for page in range(1, 11):
            response = self.request(f'actions/workflows/{workflow}/runs?head_sha={sha}&per_page=100&page={page}')
            items = response.get('workflow_runs', [])
            results.extend(items)
            if len(items) < 100:
                return results
        raise RuntimeError('Too many workflow runs to determine the latest validation safely')

    def pin_source(self, sha):
        tag = 'pennel-testflight-validation/' + sha
        try:
            self.post('git/refs', {'ref': 'refs/tags/' + tag, 'sha': sha})
        except RuntimeError:
            existing = self.request('git/ref/tags/' + tag)
            if existing.get('object', {}).get('sha') != sha:
                raise RuntimeError('The immutable validation tag points to another source') from None
        return tag


def ensure_checks(api, repository, sha, timeout=5400, sleep=time.sleep):
    if not re.fullmatch('[0-9a-f]{40}', sha):
        raise ValueError('An exact lowercase source SHA is required')
    deadline = time.monotonic() + timeout
    dispatched, retried = set(), {}
    pinned_ref = None
    while time.monotonic() < deadline:
        passed = []
        for workflow in WORKFLOWS:
            candidate = latest_run(api.runs(workflow, sha), repository, sha, workflow)
            if candidate and candidate['status'] == 'completed' and candidate.get('conclusion') == 'success':
                passed.append({'name': workflow, 'run_id': int(candidate['id']), 'status': 'success',
                               'url': candidate['html_url'], 'source_sha': sha})
            elif candidate and candidate['status'] == 'completed':
                if workflow in retried:
                    attempt = (int(candidate['id']), int(candidate.get('run_attempt', 1)))
                    if attempt <= retried[workflow]:
                        continue
                    raise RuntimeError(f'{workflow} failed for the selected source after one retry; no snapshot was published')
                api.post(f'actions/runs/{int(candidate["id"])}/rerun')
                retried[workflow] = (int(candidate['id']), int(candidate.get('run_attempt', 1)))
            elif not candidate and workflow not in dispatched:
                if pinned_ref is None:
                    pinned_ref = api.pin_source(sha)
                api.post(f'actions/workflows/{workflow}/dispatches', {'ref': pinned_ref})
                dispatched.add(workflow)
        if len(passed) == len(WORKFLOWS):
            return {'schema': 1, 'source_sha': sha, 'status': 'success', 'checks': passed}
        sleep(min(20, max(0, deadline - time.monotonic())))
    raise RuntimeError('Timed out waiting for exact-source checks; no snapshot was published')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['resolve', 'verify'])
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY', 'pennel-app/pennel'))
    parser.add_argument('--source-ref')
    parser.add_argument('--source-sha')
    parser.add_argument('--output', type=Path, default=Path('build/testflight/validation.json'))
    parser.add_argument('--timeout-seconds', type=int, default=5400)
    args = parser.parse_args()
    api = GitHub(args.repository)
    if args.action == 'resolve':
        ref = validate_ref(args.source_ref or '')
        sha = api.request('commits/' + quote(ref, safe=''))['sha']
        if not re.fullmatch('[0-9a-f]{40}', sha):
            raise ValueError('GitHub did not return a commit SHA')
        print(sha)
        if os.environ.get('GITHUB_OUTPUT'):
            with open(os.environ['GITHUB_OUTPUT'], 'a') as target:
                target.write('source_sha=' + sha + '\n')
    else:
        receipt = ensure_checks(api, args.repository, args.source_sha or '', args.timeout_seconds)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(receipt, indent=2) + '\n')
        print(json.dumps(receipt))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, KeyError) as error:
        raise SystemExit(str(error)) from None
