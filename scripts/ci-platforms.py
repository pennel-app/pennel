#!/usr/bin/env python3
"""Select macOS-first CI, with full platforms available only by manual dispatch."""

import json
import os
import sys


DESKTOP_TARGETS = (
    {'os': 'macos-15', 'target': 'aarch64-apple-darwin', 'bundles': 'app,dmg'},
    {'os': 'windows-latest', 'target': 'x86_64-pc-windows-msvc', 'bundles': 'nsis,msi'},
    {'os': 'ubuntu-22.04', 'target': 'x86_64-unknown-linux-gnu', 'bundles': 'deb,appimage'},
)
EVALUATION_SYSTEMS = ('macos-latest', 'ubuntu-latest', 'windows-latest')


def resolve_platforms(event, requested='macos'):
    if requested == '':
        requested = 'macos'
    if requested not in ('macos', 'all'):
        raise ValueError('Invalid CI platform scope: expected macos or all')
    cross_platform = event == 'workflow_dispatch' and requested == 'all'
    targets = DESKTOP_TARGETS if cross_platform else DESKTOP_TARGETS[:1]
    systems = EVALUATION_SYSTEMS if cross_platform else EVALUATION_SYSTEMS[:1]
    return {
        'platform_scope': 'all' if cross_platform else 'macos',
        'cross_platform': cross_platform,
        'desktop_matrix': {'include': [dict(row) for row in targets]},
        'evaluation_matrix': {'os': list(systems)},
    }


def main():
    try:
        result = resolve_platforms(os.environ.get('GITHUB_EVENT_NAME', ''),
                                   os.environ.get('PENNEL_CI_PLATFORM_SCOPE', 'macos'))
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1
    print('platform_scope=' + result['platform_scope'])
    print('cross_platform=' + str(result['cross_platform']).lower())
    for key in ('desktop_matrix', 'evaluation_matrix'):
        print(key + '=' + json.dumps(result[key], separators=(',', ':')))
    return 0


if __name__ == '__main__':
    sys.exit(main())
