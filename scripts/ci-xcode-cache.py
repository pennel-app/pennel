#!/usr/bin/env python3
"""Bounded, opt-in mobile compiler-cache experiments and content-free timings."""

import argparse
from datetime import datetime
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import platform
import re
import subprocess
import sys
import tarfile
import tempfile
import time
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / 'build/ci-performance/mobile.json'
CACHE_DIRECTORY = ROOT / 'build/xcode-compilation-cache'
CACHE_ARCHIVE = ROOT / 'build/xcode-compilation-cache.tar.gz'
CACHE_ALLOWANCE_BYTES = 10_000_000_000
CACHE_RESERVE_BYTES = 100_000_000
MOBILE_PHASES = {
    'serial': ('typecheck-and-tests', 'native-preparation', 'release-simulator',
               'release-device', 'debug-runtime-and-keyboard'),
    'release': ('typecheck-and-tests', 'native-preparation', 'release-simulator', 'release-device'),
    'runtime': ('typecheck-and-tests', 'native-preparation', 'debug-runtime-and-keyboard'),
}


def validate_inputs(mode, repetitions, generation):
    if mode not in ('off', 'cold', 'warm'):
        raise ValueError('Cache mode must be off, cold or warm')
    if repetitions not in (1, 3):
        raise ValueError('Benchmark repetitions must be 1 or 3')
    if mode == 'cold' and repetitions != 1:
        raise ValueError('A cold run needs exactly one cache producer')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,31}', generation):
        raise ValueError('Cache generation must be 1–32 letters, digits, dots, hyphens or underscores')


def cache_identity(metadata, generation):
    context = {key: value for key, value in metadata.items() if key != 'revision'}
    digest = hashlib.sha256(json.dumps(context, sort_keys=True).encode()).hexdigest()[:24]
    prefix = f'pennel-xcode-cas-v2-{digest}-{generation}-'
    return {'key': prefix + metadata['revision'], 'restore_prefix': prefix}


def digest_files(root, paths):
    digest = hashlib.sha256()
    for path in sorted(set(paths)):
        digest.update(str(path.relative_to(root)).encode())
        digest.update(b'\0')
        digest.update(path.read_bytes())
        digest.update(b'\0')
    return digest.hexdigest()


def validate_restore(mode, hit):
    if mode == 'warm' and hit != 'true':
        raise ValueError('Warm benchmark requires the exact cold seed for this SHA, Xcode, SDK and dependency graph')
    if mode == 'cold' and hit == 'true':
        raise ValueError('This immutable seed already exists; use a new cache_generation for another cold experiment')


def validate_snapshot_mode(action, mode):
    if (action, mode) not in (('pack', 'cold'), ('unpack', 'warm')):
        raise ValueError('Only cold experiments pack a seed, and only warm experiments unpack one')


def fits_budget(current_bytes, candidate_bytes):
    if current_bytes < 0 or candidate_bytes < 0:
        raise ValueError('Cache sizes cannot be negative')
    return current_bytes + candidate_bytes + CACHE_RESERVE_BYTES <= CACHE_ALLOWANCE_BYTES


def xcode_arguments(arguments, mode, directory):
    if mode == 'off':
        return list(arguments)
    if mode not in ('cold', 'warm') or not directory.is_absolute():
        raise ValueError('Enabled compilation caching requires a valid mode and absolute CAS directory')
    return [*arguments,
            'COMPILATION_CACHE_ENABLE_CACHING=YES',
            'COMPILATION_CACHE_ENABLE_DIAGNOSTIC_REMARKS=YES',
            f'COMPILATION_CACHE_CAS_PATH={directory}']


def read_report():
    return json.loads(REPORT.read_text()) if REPORT.exists() else {'schema': 1, 'phases': []}


def provenance():
    return {
        'run_id': os.environ.get('GITHUB_RUN_ID'),
        'run_attempt': os.environ.get('GITHUB_RUN_ATTEMPT'),
        'ref': os.environ.get('GITHUB_REF'),
        'runner_name': os.environ.get('RUNNER_NAME'),
        'runner_os': os.environ.get('RUNNER_OS'),
        'runner_arch': os.environ.get('RUNNER_ARCH'),
        'image_os': os.environ.get('ImageOS'),
        'image_version': os.environ.get('ImageVersion'),
    }


def validation_context(environment):
    lane = environment.get('PENNEL_CI_LANE', '')
    if not lane:
        return None
    mode = environment.get('PENNEL_CI_MODE')
    if lane not in MOBILE_PHASES or (mode, lane) not in (
            ('serial', 'serial'), ('candidate', 'release'), ('candidate', 'runtime')):
        raise ValueError('Invalid ordinary mobile validation mode or lane')
    if environment.get('GITHUB_EVENT_NAME') not in ('pull_request', 'push'):
        raise ValueError('Ordinary mobile lanes cannot replace manual compiler-cache experiments')
    dependency_cache = environment.get('PENNEL_DEPENDENCY_CACHE', 'warm')
    if dependency_cache not in ('warm', 'cold'):
        raise ValueError('Invalid ordinary dependency-cache policy')
    return {
        'ci_mode': mode,
        'dependency_cache': dependency_cache,
        'lane': lane,
        'qualification_id': environment.get('PENNEL_QUALIFICATION_ID', ''),
        'repository': environment.get('GITHUB_REPOSITORY'),
        'event_name': environment.get('GITHUB_EVENT_NAME'),
        'source_sha': environment.get('GITHUB_SHA'),
        'head_sha': environment.get('PENNEL_CI_HEAD_SHA'),
        'job_id': environment.get('GITHUB_JOB'),
        'expected_phases': list(MOBILE_PHASES[lane]),
    }


def phase_errors(phases, expected):
    if not isinstance(phases, list):
        return ['Missing mobile phase observations']
    names = []
    errors = []
    for phase in phases:
        if not isinstance(phase, dict) or not isinstance(phase.get('name'), str):
            errors.append('Invalid mobile phase observation')
            continue
        name = phase['name']
        names.append(name)
        if phase.get('exit_code') != 0 or isinstance(phase.get('exit_code'), bool):
            errors.append(f'{name}: validation did not succeed')
        seconds = phase.get('seconds')
        if (not isinstance(seconds, (int, float)) or isinstance(seconds, bool)
                or not math.isfinite(seconds) or seconds < 0):
            errors.append(f'{name}: invalid duration')
    for name in expected:
        if names.count(name) != 1:
            errors.append(f'{name}: expected one completed observation')
    if any(name not in expected for name in names):
        errors.append('Unexpected mobile phase observation')
    return errors


def write_report(data):
    data['provenance'] = provenance()
    validation = validation_context(os.environ)
    if validation is not None:
        data.update(schema=2, validation=validation)
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(data, indent=2) + '\n')


def output(values):
    target = os.environ.get('GITHUB_OUTPUT')
    if target:
        with open(target, 'a') as file:
            for key, value in values.items():
                file.write(f'{key}={value}\n')


def command(*arguments):
    return subprocess.check_output(arguments, cwd=ROOT, text=True).strip()


def github_json(path):
    repository = os.environ['GITHUB_REPOSITORY']
    token = os.environ['GH_TOKEN']
    request = Request(f'https://api.github.com/repos/{repository}/{path}', headers={
        'Authorization': f'Bearer {token}', 'Accept': 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
    })
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def directory_bytes(directory):
    return sum(path.stat().st_size for path in directory.rglob('*') if path.is_file() and not path.is_symlink())


def require_empty_cas(directory):
    if directory.is_symlink() or directory.exists() and (not directory.is_dir() or any(directory.iterdir())):
        raise ValueError('A comparable benchmark requires an empty local CAS directory')


def pack_snapshot(directory, archive):
    if not directory.is_dir() or directory.is_symlink():
        raise ValueError('Cannot pack an empty or missing CAS directory')
    paths = list(directory.rglob('*'))
    if any(path.is_symlink() or not (path.is_file() or path.is_dir()) for path in paths):
        raise ValueError('A CAS snapshot may contain only regular files and directories')
    size = sum(path.stat().st_size for path in paths if path.is_file())
    if not size:
        raise ValueError('Cannot pack an empty CAS directory')
    if archive.resolve().is_relative_to(directory.resolve()):
        raise ValueError('The compressed snapshot must be outside the CAS directory')
    archive.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=archive.parent, prefix='.xcode-cas-', delete=False) as temporary:
        staging = Path(temporary.name)
    try:
        with tarfile.open(staging, 'w:gz', compresslevel=1, dereference=True) as file:
            file.add(directory, arcname='cas')
        staging.replace(archive)
    finally:
        staging.unlink(missing_ok=True)
    return size


def unpack_snapshot(archive, directory):
    require_empty_cas(directory)
    if not archive.is_file() or archive.is_symlink():
        raise ValueError('Warm benchmark requires a regular compressed seed archive')
    directory.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, 'r:gz') as file:
        members = file.getmembers()
        names = set()
        roots = 0
        size = 0
        for member in members:
            path = PurePosixPath(member.name)
            if not path.parts or path.parts[0] != 'cas' or path.is_absolute() or '..' in path.parts or '\\' in member.name:
                raise ValueError('Snapshot contains an unexpected CAS root or path traversal')
            if not (member.isfile() or member.isdir()):
                raise ValueError('A CAS snapshot may contain only regular files and directories')
            if str(path) in names:
                raise ValueError('Snapshot contains duplicate CAS paths')
            names.add(str(path))
            if path == PurePosixPath('cas'):
                roots += int(member.isdir())
            if member.isfile():
                size += member.size
        if roots != 1 or not size:
            raise ValueError('Snapshot must contain one nonempty CAS directory')
        with tempfile.TemporaryDirectory(dir=directory.parent, prefix='.xcode-cas-restore-') as staging:
            file.extractall(staging, members=members, filter='data')
            restored = Path(staging) / 'cas'
            if directory.exists():
                directory.rmdir()
            restored.replace(directory)
    return size


def snapshot(action, mode):
    validate_snapshot_mode(action, mode)
    started = time.monotonic()
    exit_code = 1
    try:
        operation = pack_snapshot if action == 'pack' else unpack_snapshot
        size = operation(CACHE_DIRECTORY, CACHE_ARCHIVE) if action == 'pack' else operation(CACHE_ARCHIVE, CACHE_DIRECTORY)
        data = read_report()
        data['cache'].update(archive_bytes=CACHE_ARCHIVE.stat().st_size, uncompressed_bytes=size)
        if action == 'unpack':
            data['cache']['restored_uncompressed_bytes'] = size
        write_report(data)
        exit_code = 0
    finally:
        data = read_report()
        data['phases'].append({'name': f'cache-{action}', 'seconds': round(time.monotonic() - started, 3), 'exit_code': exit_code})
        write_report(data)


def prepare(args):
    validate_inputs(args.mode, args.repetitions, args.generation)
    paths = command('git', 'ls-files', '-z', '--', 'apps/mobile/package-lock.json',
                    'apps/mobile/app.config.*', 'apps/mobile/plugins', 'apps/mobile/modules',
                    'core/Cargo.lock', 'scripts/build-mobile-ios.sh', 'Makefile').split('\0')
    files = [ROOT / path for path in paths if path]
    files += [ROOT / 'apps/mobile/ios/Podfile.lock', ROOT / 'apps/mobile/ios/Podfile.properties.json']
    metadata = {
        'revision': command('git', 'rev-parse', 'HEAD'),
        'xcode': command('xcodebuild', '-version'),
        'device_sdk': command('xcrun', '--sdk', 'iphoneos', '--show-sdk-version'),
        'simulator_sdk': command('xcrun', '--sdk', 'iphonesimulator', '--show-sdk-version'),
        'rustc': command('rustc', '-Vv'),
        'architecture': platform.machine(),
        'variant': os.environ.get('PENNEL_VARIANT', 'preview'),
        'dependency_sha256': digest_files(ROOT, files),
    }
    if metadata['architecture'] != 'arm64' or metadata['variant'] != 'preview':
        raise ValueError('The pilot is qualified only for ARM64 preview builds')
    if args.mode != 'off':
        require_empty_cas(CACHE_DIRECTORY)
    identity = cache_identity(metadata, args.generation)
    data = read_report()
    data.update(metadata=metadata, cache=data.get('cache', {}) | identity | {'mode': args.mode}, repetition=args.repetition)
    write_report(data)
    output(identity | {'directory': str(CACHE_DIRECTORY), 'archive': str(CACHE_ARCHIVE)})


def timed(args):
    arguments = args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments
    if not arguments:
        raise ValueError('A timed phase needs a command')
    started = time.monotonic()
    result = subprocess.run(arguments, cwd=ROOT)
    data = read_report()
    data['phases'].append({'name': args.phase, 'seconds': round(time.monotonic() - started, 3), 'exit_code': result.returncode})
    write_report(data)
    return result.returncode


def budget():
    data = read_report()
    size = CACHE_ARCHIVE.stat().st_size
    current = github_json('actions/cache/usage')['active_caches_size_in_bytes']
    fits = size > 0 and fits_budget(current, size)
    data['cache'].update(archive_bytes=size, repository_bytes_before_save=current,
                         allowance_bytes=CACHE_ALLOWANCE_BYTES, reserve_bytes=CACHE_RESERVE_BYTES,
                         save_allowed=fits)
    write_report(data)
    output({'save': str(fits).lower()})
    if not fits:
        raise ValueError(f'Cold seed was not saved: compressed archive {size} bytes + repository {current} bytes must fit the existing cache allowance with headroom')


def preflight():
    current = github_json('actions/cache/usage')['active_caches_size_in_bytes']
    data = read_report()
    data['cache'] = {'mode': 'cold', 'repository_bytes_preflight': current,
                     'allowance_bytes': CACHE_ALLOWANCE_BYTES, 'reserve_bytes': CACHE_RESERVE_BYTES}
    write_report(data)
    if not fits_budget(current, 0):
        raise ValueError(f'Cold experiment cannot seed a cache: repository usage {current} bytes already leaves insufficient headroom; cache usage may lag recent deletions')


def report(args):
    data = read_report()
    cache = data.setdefault('cache', {'mode': os.environ.get('PENNEL_XCODE_CACHE_MODE', 'off')})
    cache['exact_hit'] = args.hit == 'true'
    cache['uncompressed_bytes'] = directory_bytes(CACHE_DIRECTORY)
    cache['archive_bytes'] = CACHE_ARCHIVE.stat().st_size if CACHE_ARCHIVE.is_file() else 0
    validation = validation_context(os.environ)
    if validation is not None:
        exact_hit = os.environ.get('PENNEL_RUST_CACHE_EXACT_HIT', '')
        data['rust_cache'] = {
            'exact_hit': {'true': True, 'false': False}.get(exact_hit),
            'shared_key': 'expo-mobile-lean-ios-generated-build',
            'bypassed': validation['dependency_cache'] == 'cold',
            'bypass_scope': ['rust', 'npm'] if validation['dependency_cache'] == 'cold' else [],
        }
    if (cache['mode'] != 'off' or validation is not None) and os.environ.get('GH_TOKEN'):
        try:
            if cache['mode'] != 'off':
                entries = github_json('actions/caches?per_page=100')['actions_caches']
                matches = [entry for entry in entries if entry['key'] == cache.get('key') and entry['ref'] == os.environ.get('GITHUB_REF')]
                if matches:
                    cache['stored_bytes'] = matches[0]['size_in_bytes']
            jobs = github_json(f"actions/runs/{os.environ['GITHUB_RUN_ID']}/jobs?per_page=100")['jobs']
            name = (os.environ.get('PENNEL_CI_JOB_NAME') if validation is not None else None)
            name = name or f"iOS generated build ({data.get('repetition', '1')})"
            for job in jobs:
                if job['name'] != name:
                    continue
                for step in job['steps']:
                    if step['name'] in ('Restore Xcode compilation cache', 'Save Xcode compilation cache') and step.get('completed_at'):
                        seconds = (datetime.fromisoformat(step['completed_at'].replace('Z', '+00:00')) - datetime.fromisoformat(step['started_at'].replace('Z', '+00:00'))).total_seconds()
                        cache['restore_seconds' if step['name'].startswith('Restore') else 'save_seconds'] = seconds
                    elif (validation is not None and step['name'] == 'Restore mobile Rust dependencies'
                          and step.get('conclusion') == 'success' and step.get('started_at') and step.get('completed_at')):
                        data['rust_cache']['restore_seconds'] = (datetime.fromisoformat(step['completed_at'].replace('Z', '+00:00')) - datetime.fromisoformat(step['started_at'].replace('Z', '+00:00'))).total_seconds()
        except (OSError, KeyError, ValueError) as error:
            cache['metrics_error'] = str(error)
    errors = []
    if validation is not None:
        errors = phase_errors(data.get('phases'), validation['expected_phases'])
        if data.get('metadata', {}).get('revision') != validation['source_sha']:
            errors.append('Missing or mismatched prepared source identity')
        if cache['mode'] != 'off':
            errors.append('Ordinary mobile validation cannot enable compiler caching')
        if validation['dependency_cache'] == 'cold' and data['rust_cache']['exact_hit'] is not None:
            errors.append('Cold dependency qualification unexpectedly ran a Rust cache restore')
        data['validation_errors'] = errors
    write_report(data)
    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a') as file:
            file.write(f"### Mobile CI timing — cache {cache['mode']}\n\n")
            file.write('| Phase | Seconds | Exit code |\n| --- | ---: | ---: |\n')
            for phase in data['phases']:
                file.write(f"| {phase['name']} | {phase['seconds']} | {phase['exit_code']} |\n")
            file.write(f"\nExact seed hit: {cache['exact_hit']}. CAS bytes: {cache['uncompressed_bytes']}. Compressed archive bytes: {cache['archive_bytes']}.\n")
    print(json.dumps(data, indent=2))
    return int(bool(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('validate', 'prepare'):
        entry = commands.add_parser(name)
        entry.add_argument('--mode', choices=('off', 'cold', 'warm'), required=True)
        entry.add_argument('--repetitions', type=int, default=1)
        entry.add_argument('--generation', default='pilot-1')
        if name == 'prepare':
            entry.add_argument('--repetition', default='1')
    entry = commands.add_parser('verify-restore')
    entry.add_argument('--mode', required=True)
    entry.add_argument('--hit', default='')
    entry = commands.add_parser('time')
    entry.add_argument('--phase', required=True)
    entry.add_argument('arguments', nargs=argparse.REMAINDER)
    for name in ('pack', 'unpack'):
        entry = commands.add_parser(name)
        entry.add_argument('--mode', choices=('off', 'cold', 'warm'), required=True)
    commands.add_parser('budget')
    commands.add_parser('preflight')
    entry = commands.add_parser('report')
    entry.add_argument('--hit', default='')
    entry = commands.add_parser('xcodebuild')
    entry.add_argument('arguments', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    try:
        if args.command == 'validate':
            validate_inputs(args.mode, args.repetitions, args.generation)
        elif args.command == 'prepare':
            prepare(args)
        elif args.command == 'verify-restore':
            validate_restore(args.mode, args.hit)
        elif args.command == 'time':
            return timed(args)
        elif args.command in ('pack', 'unpack'):
            snapshot(args.command, args.mode)
        elif args.command == 'budget':
            budget()
        elif args.command == 'preflight':
            preflight()
        elif args.command == 'report':
            return report(args)
        elif args.command == 'xcodebuild':
            mode = os.environ.get('PENNEL_XCODE_CACHE_MODE', 'off')
            directory = Path(os.environ.get('PENNEL_XCODE_CACHE_DIR', str(CACHE_DIRECTORY)))
            arguments = xcode_arguments(args.arguments, mode, directory)
            os.execvp('xcodebuild', ['xcodebuild', *arguments])
    except (ValueError, OSError, EOFError, tarfile.TarError, KeyError, subprocess.CalledProcessError) as error:
        print(f'::error::{error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
