import json
import hashlib
import os
from pathlib import Path
import re
import runpy
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / '.github/workflows/desktop.yml'
INSTALLERS = 'Installers with verified non-Mac x64 CPU baseline'
LINT = 'cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings'
TESTS = 'cargo test --manifest-path src-tauri/Cargo.toml'


def app_job():
    return WORKFLOW.read_text().split('\n  app:\n', 1)[1].split('\n  packaging-tools:\n', 1)[0]


def app_steps(mode, dependency_cache='warm', runner_environment='github-hosted'):
    steps = []
    for block in re.split(r'(?=^      - )', app_job(), flags=re.MULTILINE):
        if not block.startswith('      - '):
            continue
        condition = re.search(r'^        if: (.+)$', block, flags=re.MULTILINE)
        if condition and 'needs.changes.outputs.ci_mode' in condition[1]:
            if condition[1] == "needs.changes.outputs.ci_mode == 'candidate'":
                selected = mode == 'candidate'
            elif condition[1] == "needs.changes.outputs.ci_mode != 'candidate'":
                selected = mode != 'candidate'
            else:
                raise AssertionError('Unrecognised desktop qualification condition')
            if not selected:
                continue
        if condition and 'needs.changes.outputs.dependency_cache' in condition[1]:
            expected = "runner.environment == 'github-hosted' && needs.changes.outputs.dependency_cache != 'cold'"
            if condition[1] != expected:
                raise AssertionError('Unrecognised desktop dependency-cache condition')
            if runner_environment != 'github-hosted' or dependency_cache == 'cold':
                continue
        label = re.search(r'^      - (?:name|uses|run): (.+)$', block, flags=re.MULTILINE)
        command = re.search(r'^        run: (.+)$', block, flags=re.MULTILINE)
        steps.append((label[1], command[1] if command else None, block))
    return steps


class DesktopBuildTests(unittest.TestCase):
    def test_serial_and_missing_modes_preserve_original_build_order(self):
        for mode in ('serial', '', 'unknown'):
            with self.subTest(mode=mode):
                labels = [step[0] for step in app_steps(mode)]
                self.assertLess(labels.index('Frontend'), labels.index('Frontend tests'))
                self.assertLess(labels.index('Frontend tests'), labels.index('Rust format'))
                self.assertLess(labels.index('Rust format'), labels.index('Rust lint'))
                self.assertLess(labels.index('Rust lint'), labels.index('Rust tests'))
                self.assertLess(labels.index('Rust tests'), labels.index(INSTALLERS))
                self.assertNotIn('Rust lint after packaging', labels)
                self.assertNotIn('Rust tests after packaging', labels)

    def test_candidate_uses_only_the_canonical_tauri_frontend_build(self):
        steps = app_steps('candidate')
        labels = [step[0] for step in steps]
        commands = [step[1] for step in steps]
        self.assertNotIn('Frontend', labels)
        self.assertNotIn('npm run build', commands)
        config = json.loads((ROOT / 'apps/desktop/src-tauri/tauri.conf.json').read_text())
        self.assertEqual(config['build']['beforeBuildCommand'], 'npm run build')
        self.assertEqual(labels.count(INSTALLERS), 1)
        self.assertLess(labels.index('Frontend tests'), labels.index(INSTALLERS))
        self.assertLess(labels.index('Rust format'), labels.index(INSTALLERS))
        self.assertLess(labels.index(INSTALLERS), labels.index('Rust lint after packaging'))
        self.assertLess(labels.index('Rust lint after packaging'), labels.index('Rust tests after packaging'))

    def test_both_modes_keep_each_frontend_and_backend_check_once(self):
        for mode in ('serial', 'candidate'):
            with self.subTest(mode=mode):
                commands = [step[1] for step in app_steps(mode)]
                self.assertEqual(commands.count('npm test'), 1)
                self.assertEqual(commands.count('cargo fmt --manifest-path src-tauri/Cargo.toml -- --check'), 1)
                self.assertEqual(commands.count(LINT), 1)
                self.assertEqual(commands.count(TESTS), 1)
                self.assertNotIn('continue-on-error:', app_job())

    def test_validation_failures_cannot_upload_or_accept_a_package(self):
        for mode in ('serial', 'candidate'):
            with self.subTest(mode=mode):
                steps = app_steps(mode)
                labels = [step[0] for step in steps]
                commands = [step[1] for step in steps]
                inspection = labels.index('Inspect packaged Mac app without opening it')
                upload = labels.index('actions/upload-artifact@v5')
                self.assertLess(commands.index(LINT), inspection)
                self.assertLess(commands.index(TESTS), inspection)
                self.assertLess(inspection, upload)
                self.assertNotIn('always()', steps[inspection][2])
                self.assertIsNone(re.search(r'^        if:', steps[upload][2], flags=re.MULTILINE))
                self.assertNotIn('continue-on-error:', steps[upload][2])

    def test_mac_default_and_manual_all_targets_and_portability_validation_remain(self):
        job = app_job()
        self.assertIn('matrix: ${{ fromJSON(needs.changes.outputs.desktop_matrix) }}', job)
        resolve = runpy.run_path(str(ROOT / 'scripts/ci-platforms.py'))['resolve_platforms']
        mac = resolve('push', 'all')['desktop_matrix']['include']
        full = resolve('workflow_dispatch', 'all')['desktop_matrix']['include']
        self.assertEqual([row['target'] for row in mac], ['aarch64-apple-darwin'])
        self.assertEqual([row['target'] for row in full], [
            'aarch64-apple-darwin', 'x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu',
        ])
        self.assertEqual([row['bundles'] for row in full], ['app,dmg', 'nsis,msi', 'deb,appimage'])
        self.assertIn('release.build_distributable(', job)
        self.assertIn('needs: [changes, version]', job)
        self.assertIn('fail-fast: false', job)

    def test_foreign_core_is_manual_all_only(self):
        workflow = WORKFLOW.read_text()
        core = workflow.split('\n  core:\n', 1)[1].split('\n  app:\n', 1)[0]
        self.assertIn("if: needs.changes.outputs.core == 'true' && needs.changes.outputs.cross_platform == 'true'", core)
        self.assertIn("platforms: ${{ inputs.platforms || 'macos' }}", workflow)
        self.assertRegex(workflow, r'platforms:\n(?:.*\n){0,5}\s+default: macos')

    def test_evaluation_and_signed_candidates_have_explicit_manual_all_paths(self):
        evaluation = (ROOT / '.github/workflows/decision-evaluation.yml').read_text()
        candidate = (ROOT / '.github/workflows/desktop-candidate.yml').read_text()
        self.assertIn('workflow_dispatch:', evaluation)
        self.assertIn('matrix: ${{ fromJSON(needs.changes.outputs.evaluation_matrix) }}', evaluation)
        self.assertIn("platforms: ${{ inputs.platforms || 'macos' }}", evaluation)
        self.assertIn('matrix: ${{ fromJSON(needs.platforms.outputs.desktop_matrix) }}', candidate)
        self.assertIn('needs: platforms', candidate)
        self.assertIn('PENNEL_CI_PLATFORM_SCOPE: ${{ inputs.platforms }}', candidate)
        self.assertNotIn('gh release create', candidate)
        for text in (evaluation, candidate):
            self.assertRegex(text, r'platforms:\n(?:.*\n){0,5}\s+default: macos')

    def test_cold_cache_bypass_keeps_every_build_and_test(self):
        for mode in ('serial', 'candidate'):
            with self.subTest(mode=mode):
                warm = app_steps(mode)
                cold = app_steps(mode, dependency_cache='cold')
                self.assertIn('Swatinem/rust-cache@v2', [step[0] for step in warm])
                self.assertNotIn('Swatinem/rust-cache@v2', [step[0] for step in cold])
                self.assertEqual([step[1] for step in warm if step[1]], [step[1] for step in cold if step[1]])
                self.assertIn('actions/setup-node@v5', [step[0] for step in cold])
                self.assertIn('npm ci', [step[0] for step in cold])
                self.assertEqual([step[0] for step in cold].count(INSTALLERS), 1)

    def test_both_rust_cache_actions_keep_identity_and_existing_save_rules(self):
        workflow = WORKFLOW.read_text()
        cache_blocks = re.findall(
            r'^      - uses: Swatinem/rust-cache@v2\n.*?(?=^      - |\Z)',
            workflow, flags=re.MULTILINE | re.DOTALL,
        )
        self.assertEqual(len(cache_blocks), 2)
        for block in cache_blocks:
            self.assertIn("if: runner.environment == 'github-hosted' && needs.changes.outputs.dependency_cache != 'cold'", block)
            self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' || (github.event_name == 'workflow_dispatch' && inputs.cache_probe && runner.os == 'Linux') }}", block)
            self.assertIn('key: sherpa-prebuilt', block)
            self.assertIn('cache-bin: "false"', block)
        self.assertIn('workspaces: core\n', cache_blocks[0])
        self.assertIn('workspaces: apps/desktop/src-tauri\n', cache_blocks[1])

    def test_dependency_cold_disables_npm_cache_without_skipping_node_setup(self):
        node = next(step[2] for step in app_steps('candidate', dependency_cache='cold') if step[0] == 'actions/setup-node@v5')
        self.assertIn("cache: ${{ runner.environment == 'github-hosted' && needs.changes.outputs.dependency_cache != 'cold' && 'npm' || '' }}", node)
        self.assertIn("package-manager-cache: ${{ needs.changes.outputs.dependency_cache != 'cold' }}", node)
        self.assertNotRegex(node, r'\n        if:')
        for mode in ('serial', 'candidate'):
            self.assertNotIn('Swatinem/rust-cache@v2', [
                step[0] for step in app_steps(mode, runner_environment='self-hosted')
            ])

    def test_release_and_cleanup_gates_remain_failure_safe(self):
        workflow = WORKFLOW.read_text()
        release = workflow.split('\n  release:\n', 1)[1]
        self.assertIn('needs: [changes, version, app, packaging-tools]', release)
        self.assertIn("if: needs.version.outputs.release == 'true' && needs.changes.outputs.platform_scope == 'macos'", release)
        self.assertNotIn('always()', release)
        self.assertNotIn('continue-on-error:', release)
        publish = release.split('name: Publish the GitHub release', 1)[1].split('name: Update the Homebrew cask', 1)[0]
        self.assertIn('find dist -type f -name "Pennel_${VERSION}_aarch64.dmg"', publish)
        self.assertIn('${#files[@]}" -ne 1', publish)
        for foreign in ('.exe', '.msi', '.deb', '.AppImage'):
            self.assertNotIn(foreign, publish)
        self.assertIn("cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}", workflow)
        self.assertIn("if: always() && runner.os == 'macOS' && env.PENNEL_SIGNING_KEYCHAIN != ''", workflow)
        self.assertIn("if: always() && runner.os == 'Linux'", workflow)

    def test_publisher_only_sends_one_current_mac_dmg_and_checksums(self):
        workflow = WORKFLOW.read_text()
        block = workflow.split('      - name: Publish the GitHub release\n', 1)[1]
        block = block.split('      - name: Update the Homebrew cask\n', 1)[0]
        script = textwrap.dedent(block.split('        run: |\n', 1)[1])
        current = 'Pennel_1.2.3_aarch64.dmg'
        foreign = ['Pennel_1.2.3_x64-setup.exe', 'Pennel_1.2.3_x64.msi',
                   'Pennel_1.2.3_amd64.deb', 'Pennel_1.2.3_amd64.AppImage']
        scenarios = {
            'mac-with-foreign-assets': (['one/' + current] + foreign, True),
            'missing-mac': (foreign, False),
            'ambiguous-mac': (['one/' + current, 'two/' + current] + foreign, False),
            'stale-mac': (['Pennel_1.2.2_aarch64.dmg'] + foreign, False),
        }
        for name, (files, succeeds) in scenarios.items():
            with self.subTest(scenario=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'dist').mkdir()
                tools = root / 'tools'
                tools.mkdir()
                gh = tools / 'gh'
                gh.write_text('#!/usr/bin/env python3\nimport json,os,pathlib,sys\n'
                              'pathlib.Path(os.environ["GH_ARGS_RECEIPT"]).write_text(json.dumps(sys.argv[1:]))\n')
                sha = tools / 'sha256sum'
                sha.write_text('#!/usr/bin/env python3\nimport hashlib,pathlib,sys\n'
                               'for name in sys.argv[1:]: print(hashlib.sha256(pathlib.Path(name).read_bytes()).hexdigest()+"  "+name)\n')
                gh.chmod(0o700)
                sha.chmod(0o700)
                for filename in files:
                    target = root / 'dist' / filename
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(('synthetic ' + filename).encode())
                receipt = root / 'gh-arguments.json'
                env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'],
                           VERSION='1.2.3', GITHUB_SHA='a' * 40, GH_TOKEN='synthetic',
                           GH_ARGS_RECEIPT=str(receipt))
                result = subprocess.run(['bash', '-c', script], cwd=root, env=env,
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode == 0, succeeds, result.stderr + result.stdout)
                if succeeds:
                    arguments = json.loads(receipt.read_text())
                    self.assertEqual(arguments[:3], ['release', 'create', 'v1.2.3'])
                    self.assertEqual(arguments[3:arguments.index('--target')],
                                     ['dist/one/' + current, 'SHA256SUMS.txt'])
                    expected = hashlib.sha256(('synthetic one/' + current).encode()).hexdigest()
                    self.assertEqual((root / 'SHA256SUMS.txt').read_text().split(), [expected, current])
                else:
                    self.assertFalse(receipt.exists(), 'Invalid assets reached publication')


if __name__ == '__main__':
    unittest.main()
