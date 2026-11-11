#!/usr/bin/env python3
"""Select PR checks from the complete merge-base diff; non-PR runs stay full."""

import argparse
from pathlib import PurePosixPath
import subprocess


JOBS = ('core', 'platform', 'app', 'packaging')


def classify(paths):
    selected = set()
    for path in paths:
        if path.startswith('core/'):
            selected.update(('core', 'app'))
        elif path.startswith('apps/desktop/'):
            selected.add('app')
            if path.startswith('apps/desktop/src-tauri/tauri.'):
                selected.add('packaging')
        elif path.startswith('apps/macos-platform/'):
            selected.update(('platform', 'app'))
        elif path.startswith('apps/mobile/'):
            # Their own workflows validate native/mobile inputs; desktop does not consume them.
            continue
        elif path.startswith(('assets/locales/', 'assets/qualifications/')):
            selected.update(('core', 'app'))
        elif path.startswith('assets/'):
            selected.add('app')
        elif path.startswith(('.github/workflows/', '.cargo/', 'scripts/')) or path == 'Makefile':
            selected.update(JOBS)
        elif path.startswith('docs/') or PurePosixPath(path).suffix == '.md' or path in (
            'LICENSE', '.gitignore',
        ):
            continue
        else:
            # New build inputs must not silently escape validation.
            selected.update(JOBS)
    return {job: job in selected for job in JOBS}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base')
    parser.add_argument('--head')
    args = parser.parse_args()
    if bool(args.base) != bool(args.head):
        parser.error('--base and --head must be supplied together')
    if args.base:
        # Disable rename detection so moving code to docs still checks the deleted code.
        # NUL delimiters handle whitespace/newlines; git failure fails the detection job.
        diff = subprocess.check_output([
            'git', 'diff', '--name-only', '--no-renames', '-z',
            f'{args.base}...{args.head}', '--',
        ])
        selected = classify(diff.decode('utf-8', errors='surrogateescape').split('\0')[:-1])
    else:
        selected = dict.fromkeys(JOBS, True)
    for job, enabled in selected.items():
        print(f'{job}={str(enabled).lower()}')


if __name__ == '__main__':
    main()
