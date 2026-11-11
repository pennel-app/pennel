"""Keep one OS-stored evaluation credential in RAM across explicit local phases.

Commands arrive on stdin as JSON. Each child receives the credential through a
new inherited anonymous pipe; phase arguments, files and logs never contain it.
The child retains all frozen-request, qualification and cumulative-budget checks.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
KEY_SERVICE = 'dev.pennel.evaluation.openrouter'
KEY_ACCOUNT = 'OPENROUTER_API_KEY'


def read_key():
    try:
        result = subprocess.run(['security', 'find-generic-password', '-s', KEY_SERVICE,
            '-a', KEY_ACCOUNT, '-w'], stdin=subprocess.DEVNULL, capture_output=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        raise ValueError('credential_session_os_read_unavailable') from None
    key = result.stdout.rstrip(b'\r\n')
    if result.returncode or not 0 < len(key) <= 4096 or any(value in key for value in (b'\r', b'\n', b'\0')):
        raise ValueError('credential_session_os_read_unavailable')
    return key


def owned_output(value):
    output = (ROOT / value).resolve()
    scope = (ROOT / 'build/review-functional-96').resolve()
    if not output.is_relative_to(scope) or output == scope or output.exists():
        raise ValueError('new_owned_phase_output_required')
    return str(output)


def command(value, args, descriptor):
    if not isinstance(value, dict):
        raise ValueError('phase_object_required')
    phase = value.get('phase')
    output = owned_output(value.get('output', ''))
    if phase in ('calibration', 'heldout', 'resources'):
        if set(value) != {'phase', 'output', 'freeze', 'freeze_lock', 'freeze_commit'}:
            raise ValueError('explicit_frozen_phase_required')
        return [sys.executable, str(ROOT / 'evaluations/decision-models/finished_experiment.py'), phase,
            '--freeze', value['freeze'], '--bridge', args.bridge, '--output', output,
            '--pricing', args.pricing, '--ledger', args.ledger,
            '--freeze-lock', value['freeze_lock'], '--freeze-commit', value['freeze_commit'],
            '--credential-fd', str(descriptor), '--execute']
    if phase == 'native':
        if set(value) - {'phase', 'output', 'locale', 'appearance'}:
            raise ValueError('explicit_owned_native_phase_required')
        return [sys.executable, str(ROOT / 'scripts/verify-qualified-review-native.py'),
            '--execute', '--output', output, '--ledger', args.ledger,
            '--window-helper', args.window_helper, '--credential-bridge', args.credential_bridge,
            '--credential-fd', str(descriptor), '--locale', value.get('locale', 'en'),
            '--appearance', value.get('appearance', 'dark')]
    raise ValueError('unsupported_explicit_phase')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bridge', required=True)
    parser.add_argument('--pricing', required=True)
    parser.add_argument('--ledger', required=True)
    parser.add_argument('--window-helper', required=True)
    parser.add_argument('--credential-bridge', required=True)
    args = parser.parse_args()
    key = read_key()
    print(json.dumps({'credential_ready_in_memory': True, 'model_requests': 0}), flush=True)
    try:
        for line in sys.stdin:
            if line.strip() == 'quit':
                break
            reader, writer = os.pipe()
            child = None
            try:
                value = json.loads(line)
                arguments = command(value, args, reader)
                child = subprocess.Popen(arguments, cwd=ROOT, pass_fds=(reader,), stdin=subprocess.DEVNULL)
                os.close(reader); reader = None
                with os.fdopen(writer, 'wb') as destination:
                    writer = None
                    destination.write(key)
                result = child.wait()
                print(json.dumps({'phase': value['phase'], 'exit_code': result}), flush=True)
            except (OSError, ValueError):
                # Neither failed command arguments nor credential buffers are printed.
                print(json.dumps({'phase_rejected': True}), flush=True)
                if child is not None:
                    child.wait()
            finally:
                if reader is not None: os.close(reader)
                if writer is not None: os.close(writer)
    finally:
        key = None
        print(json.dumps({'credential_session_closed': True}), flush=True)


if __name__ == '__main__':
    main()
