import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'ci-changes.py'


class ChangesTests(unittest.TestCase):
    def classify(self, *paths):
        spec = importlib.util.spec_from_file_location('ci_changes', SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return {key for key, enabled in module.classify(paths).items() if enabled}

    def test_non_pr_runs_everything(self):
        output = subprocess.check_output(['python3', str(SCRIPT)], text=True)
        self.assertEqual(set(output.splitlines()), {
            'core=true', 'platform=true', 'app=true', 'packaging=true',
        })

    def test_empty_diff_skips_builds(self):
        self.assertEqual(self.classify(), set())

    def test_docs_do_not_build(self):
        self.assertEqual(self.classify('README.md', 'docs/desktop.md', 'AGENTS.md'), set())

    def test_frontend_only_builds_desktop(self):
        self.assertEqual(self.classify('apps/desktop/src/main.ts'), {'app'})

    def test_bundle_config_runs_packaging_checks(self):
        self.assertEqual(self.classify('apps/desktop/src-tauri/tauri.conf.json'),
                         {'app', 'packaging'})

    def test_core_builds_all_consumers(self):
        self.assertEqual(self.classify('core/src/lib.rs'), {'core', 'app'})

    def test_speaker_changes_build_all_consumers(self):
        self.assertEqual(self.classify('core/speakers/Cargo.lock'), {'core', 'app'})

    def test_platform_library_builds_swift_tests_and_desktop(self):
        self.assertEqual(self.classify('apps/macos-platform/Package.swift'), {'platform', 'app'})

    def test_mobile_inputs_use_their_own_workflows(self):
        for path in (
            'apps/mobile/app/index.tsx',
            'apps/mobile/package-lock.json',
            'apps/mobile/plugins/with-pennel.cjs',
            'apps/mobile/modules/pennel-mobile/ios/PennelMobile.swift',
            'apps/mobile/native/ios/Services/Meetings/MeetingRecorder.swift',
            'apps/mobile/native/ios/MobileKit/Package.swift',
            'apps/mobile/native/ios/Keyboard/Sources/KeyboardViewController.swift',
            'apps/mobile/native/ios/KeyboardTests/KeyboardTests.swift',
        ):
            with self.subTest(path=path):
                self.assertEqual(self.classify(path), set())

    def test_mobile_mixed_changes_preserve_other_consumers(self):
        self.assertEqual(self.classify('apps/mobile/app/index.tsx', 'core/src/lib.rs'),
                         {'core', 'app'})
        self.assertEqual(self.classify('apps/mobile/native/ios/MobileKit/Package.swift',
                                       'apps/desktop/src/main.ts'), {'app'})
        self.assertEqual(self.classify('apps/mobile/app/index.tsx', 'new-build-config.toml'),
                         {'core', 'platform', 'app', 'packaging'})

    def test_shared_assets_build_desktop(self):
        self.assertEqual(self.classify('assets/brand/tokens.json'), {'app'})

    def test_translation_catalogs_validate_core_and_desktop(self):
        self.assertEqual(self.classify('assets/locales/onboarding.en.json'),
                         {'core', 'app'})

    def test_qualification_registry_validates_core_desktop_and_mobile(self):
        self.assertEqual(self.classify('assets/qualifications/meeting-assistance.json'),
                         {'core', 'app'})
        workflow = SCRIPT.parents[1] / '.github/workflows/mobile.yml'
        self.assertIn('      - "assets/qualifications/**"', workflow.read_text())

    def test_build_infrastructure_runs_everything(self):
        for path in ('Makefile', '.cargo/config.toml', '.github/workflows/ci.yml',
                     'scripts/generate-theme.py', 'scripts/tests/test_ci_changes.py'):
            with self.subTest(path=path):
                self.assertEqual(self.classify(path), {'core', 'platform', 'app', 'packaging'})

    def test_mixed_changes_union_jobs(self):
        self.assertEqual(self.classify('README.md', 'apps/desktop/package-lock.json',
                                       'core/src/lib.rs'), {'core', 'app'})

    def test_unknown_inputs_fail_open(self):
        self.assertEqual(self.classify('new-build-config.toml'),
                         {'core', 'platform', 'app', 'packaging'})

    def test_git_diff_handles_deletions_renames_and_branch_base(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], text=True).strip()
            git('init', '-q')
            git('config', 'user.email', 'ci@example.com')
            git('config', 'user.name', 'CI test')
            (repo / 'core').mkdir()
            (repo / 'core/input.rs').write_text('input')
            git('add', '.')
            git('commit', '-qm', 'base')
            base = git('rev-parse', 'HEAD')
            git('checkout', '-qb', 'topic')
            git('mv', 'core/input.rs', 'README.md')
            git('commit', '-qm', 'rename code to docs')
            head = git('rev-parse', 'HEAD')
            git('checkout', '-qb', 'base-advanced', base)
            (repo / 'unrelated.toml').write_text('base-only change')
            git('add', '.')
            git('commit', '-qm', 'advance base')
            advanced_base = git('rev-parse', 'HEAD')
            output = subprocess.check_output([
                'python3', str(SCRIPT), '--base', advanced_base, '--head', head,
            ], cwd=repo, text=True)
            self.assertEqual(set(output.splitlines()),
                             {'core=true', 'app=true', 'platform=false', 'packaging=false'})

    def test_moving_desktop_code_to_mobile_still_checks_deleted_desktop_input(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], text=True).strip()
            git('init', '-q')
            git('config', 'user.email', 'ci@example.com')
            git('config', 'user.name', 'CI test')
            (repo / 'apps/desktop/src').mkdir(parents=True)
            (repo / 'apps/desktop/src/input.ts').write_text('input')
            git('add', '.')
            git('commit', '-qm', 'base')
            base = git('rev-parse', 'HEAD')
            (repo / 'apps/mobile/app').mkdir(parents=True)
            git('mv', 'apps/desktop/src/input.ts', 'apps/mobile/app/input.ts')
            git('commit', '-qm', 'move to mobile')
            output = subprocess.check_output([
                'python3', str(SCRIPT), '--base', base, '--head', git('rev-parse', 'HEAD'),
            ], cwd=repo, text=True)
            self.assertEqual(set(output.splitlines()),
                             {'core=false', 'app=true', 'platform=false', 'packaging=false'})

    def test_mobile_only_git_diff_handles_deletions_and_renames(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], text=True).strip()
            git('init', '-q')
            git('config', 'user.email', 'ci@example.com')
            git('config', 'user.name', 'CI test')
            (repo / 'apps/mobile/app').mkdir(parents=True)
            (repo / 'apps/mobile/native/ios').mkdir(parents=True)
            (repo / 'apps/mobile/app/input.ts').write_text('input')
            (repo / 'apps/mobile/native/ios/input.swift').write_text('input')
            git('add', '.')
            git('commit', '-qm', 'base')
            base = git('rev-parse', 'HEAD')
            git('mv', 'apps/mobile/app/input.ts', 'apps/mobile/app/renamed.ts')
            git('rm', 'apps/mobile/native/ios/input.swift')
            git('commit', '-qm', 'mobile rename and iOS deletion')
            output = subprocess.check_output([
                'python3', str(SCRIPT), '--base', base, '--head', git('rev-parse', 'HEAD'),
            ], cwd=repo, text=True)
            self.assertEqual(set(output.splitlines()),
                             {'core=false', 'app=false', 'platform=false', 'packaging=false'})


if __name__ == '__main__':
    unittest.main()
