import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'ci-platforms.py'
DESKTOP_TARGETS = [
    {'os': 'macos-15', 'target': 'aarch64-apple-darwin', 'bundles': 'app,dmg'},
    {'os': 'windows-latest', 'target': 'x86_64-pc-windows-msvc', 'bundles': 'nsis,msi'},
    {'os': 'ubuntu-22.04', 'target': 'x86_64-unknown-linux-gnu', 'bundles': 'deb,appimage'},
]
EVALUATION_SYSTEMS = ['macos-latest', 'ubuntu-latest', 'windows-latest']
INVALID_SCOPE = 'Invalid CI platform scope: expected macos or all'


def load_module():
    spec = importlib.util.spec_from_file_location('ci_platforms', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def expected_result(all_platforms=False):
    return {
        'platform_scope': 'all' if all_platforms else 'macos',
        'cross_platform': all_platforms,
        'desktop_matrix': {'include': DESKTOP_TARGETS if all_platforms else DESKTOP_TARGETS[:1]},
        'evaluation_matrix': {'os': EVALUATION_SYSTEMS if all_platforms else EVALUATION_SYSTEMS[:1]},
    }


class PlatformTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = load_module()

    def test_default_and_empty_scope_select_macos_for_every_event(self):
        for event in ('pull_request', 'push', 'workflow_dispatch', 'unknown', '', None):
            with self.subTest(event=event):
                self.assertEqual(self.module.resolve_platforms(event), expected_result())
                self.assertEqual(self.module.resolve_platforms(event, ''), expected_result())

    def test_manual_all_retains_original_targets_bundles_and_order(self):
        self.assertEqual(self.module.resolve_platforms('workflow_dispatch', 'all'),
                         expected_result(all_platforms=True))

    def test_manual_macos_selects_only_macos(self):
        self.assertEqual(self.module.resolve_platforms('workflow_dispatch', 'macos'),
                         expected_result())

    def test_all_cannot_enable_foreign_platforms_on_automatic_or_unknown_events(self):
        for event in ('pull_request', 'push', 'schedule', 'pull_request_target',
                      'merge_group', 'workflow_call', 'workflow_run', 'unknown', '', None,
                      'workflow_dispatch\n', 'WORKFLOW_DISPATCH'):
            with self.subTest(event=event):
                self.assertEqual(self.module.resolve_platforms(event, 'all'), expected_result())

    def test_invalid_scope_is_rejected_without_echoing_input(self):
        for event in ('workflow_dispatch', 'pull_request'):
            for requested in ('windows', 'linux', 'ALL', 'all ', ' ',
                              'all\nsecret=value', False, 0, [], {}):
                with self.subTest(event=event, requested=requested):
                    with self.assertRaises(ValueError) as error:
                        self.module.resolve_platforms(event, requested)
                    self.assertEqual(str(error.exception), INVALID_SCOPE)

    def test_returned_matrices_cannot_change_later_selections(self):
        result = self.module.resolve_platforms('workflow_dispatch', 'all')
        result['desktop_matrix']['include'][0]['target'] = 'changed'
        result['desktop_matrix']['include'].clear()
        result['evaluation_matrix']['os'].clear()
        self.assertEqual(self.module.resolve_platforms('workflow_dispatch', 'all'),
                         expected_result(all_platforms=True))
        self.assertEqual(self.module.resolve_platforms('pull_request'), expected_result())

    def run_cli(self, updates=None, cwd=None):
        env = {**os.environ, 'GITHUB_EVENT_NAME': 'pull_request'}
        env.pop('PENNEL_CI_PLATFORM_SCOPE', None)
        for key, value in (updates or {}).items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = value
        return subprocess.run(['python3', '-B', str(SCRIPT)], env=env, cwd=cwd,
                              text=True, capture_output=True)

    def parse_output(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, '')
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 4)
        values = dict(line.split('=', 1) for line in lines)
        self.assertEqual(set(values), {'platform_scope', 'cross_platform',
                                       'desktop_matrix', 'evaluation_matrix'})
        self.assertIn(values['cross_platform'], ('true', 'false'))
        for key in ('desktop_matrix', 'evaluation_matrix'):
            parsed = json.loads(values[key])
            self.assertEqual(values[key], json.dumps(parsed, separators=(',', ':')))
            values[key] = parsed
        values['cross_platform'] = json.loads(values['cross_platform'])
        return values

    def test_cli_missing_or_empty_settings_use_macos(self):
        for updates in ({}, {'PENNEL_CI_PLATFORM_SCOPE': ''},
                        {'GITHUB_EVENT_NAME': None}, {'GITHUB_EVENT_NAME': ''}):
            with self.subTest(updates=updates):
                self.assertEqual(self.parse_output(self.run_cli(updates)), expected_result())

    def test_cli_manual_all_outputs_parseable_full_matrices(self):
        result = self.run_cli({'GITHUB_EVENT_NAME': 'workflow_dispatch',
                               'PENNEL_CI_PLATFORM_SCOPE': 'all'})
        self.assertEqual(self.parse_output(result), expected_result(all_platforms=True))

    def test_cli_nonmanual_all_stays_macos(self):
        result = self.run_cli({'PENNEL_CI_PLATFORM_SCOPE': 'all'})
        self.assertEqual(self.parse_output(result), expected_result())

    def test_cli_invalid_scope_fails_without_output_or_raw_environment_values(self):
        for event in ('workflow_dispatch', 'push'):
            result = self.run_cli({'GITHUB_EVENT_NAME': event,
                                   'PENNEL_CI_PLATFORM_SCOPE': 'all\nsecret=value',
                                   'UNRELATED_SECRET': 'private-sentinel'})
            with self.subTest(event=event):
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')
                self.assertEqual(result.stderr, INVALID_SCOPE + '\n')

    def test_cli_ignores_secret_environment_and_does_not_write_github_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_cli({'UNRELATED_SECRET': 'private-sentinel',
                                   'GITHUB_OUTPUT': str(root / 'output.txt'),
                                   'GITHUB_STEP_SUMMARY': str(root / 'summary.md'),
                                   'GITHUB_EVENT_PATH': str(root / 'unreadable-event.json')},
                                  cwd=root)
            self.assertEqual(self.parse_output(result), expected_result())
            self.assertNotIn('private-sentinel', result.stdout + result.stderr)
            self.assertEqual(list(root.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
