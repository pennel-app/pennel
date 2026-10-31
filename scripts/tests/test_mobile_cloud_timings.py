import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]


class CloudTimingsTests(unittest.TestCase):
    def test_archive_receipt_is_retained_in_downloadable_hook_logs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'scripts').mkdir()
            shutil.copy2(SCRIPTS / 'mobile-native-cache.py', root / 'scripts/mobile-native-cache.py')
            target = root / 'build/testflight'
            target.mkdir(parents=True)
            (target / 'archive-start.json').write_text(json.dumps({'started': time.time() - 5}))
            (target / 'cloud-timings.json').write_text(json.dumps({'schema': 1, 'source_sha': 'a' * 40, 'phases': []}))
            result = subprocess.run(['bash', str(SCRIPTS / 'xcode-cloud-post-xcodebuild.sh')], check=True, text=True, capture_output=True,
                                    env={**os.environ, 'CI_PRIMARY_REPOSITORY_PATH': str(root), 'CI_XCODEBUILD_EXIT_CODE': '0',
                                         'CI_COMMIT': 'b' * 40, 'CI_BUILD_URL': 'https://appstoreconnect.apple.com/build/example'})
            prefix = 'PENNEL_TESTFLIGHT_TIMINGS='
            self.assertTrue(result.stdout.startswith(prefix))
            receipt = json.loads(result.stdout[len(prefix):])
            self.assertEqual(receipt['source_sha'], 'a' * 40)
            self.assertEqual(receipt['snapshot_sha'], 'b' * 40)
            self.assertEqual(receipt['phases'][-1]['name'], 'native-archive')
            self.assertEqual(receipt['phases'][-1]['exit_code'], 0)
            self.assertGreaterEqual(receipt['phases'][-1]['seconds'], 5)


if __name__ == '__main__':
    unittest.main()
