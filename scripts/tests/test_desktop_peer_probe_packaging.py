"""Run the Mac QA build and signer in a disposable checkout with synthetic tools."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
FINGERPRINT = "A" * 40


@unittest.skipUnless(sys.platform == "darwin", "The host probe uses macOS bundle publication.")
class ProbePackagingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("build-peer-comparison.sh", "sign-peer-comparison.sh", "publish-peer-comparison-directory.py"):
            shutil.copyfile(SCRIPTS / name, scripts / name)
        self.tools = self.root / "tools-stub"
        self.tools.mkdir()
        (self.root / "core").mkdir()
        self.output = self.root / "build/peer-comparison"
        self.app = self.output / "Pennel Peer Comparison.app/Contents"
        self.env = {
            "PATH": os.pathsep.join([str(self.tools), str(Path(sys.executable).parent), os.defpath]),
            "QA_TEST_ROOT": str(self.root),
            "APPLE_SIGNING_IDENTITY": "Example display name",
            "PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE": "synthetic-profile",
        }
        self.stub("cargo", """
            from pathlib import Path
            if sys.argv[1] == "run":
                library = Path(arg("--library"))
                output = Path(arg("--out-dir"))
                assert library.read_text() == os.environ["GENERATION"]
                for name in ("pennel_coreFFI.h", "pennel_coreFFI.modulemap", "pennel_core.swift"):
                    (output / name).write_text(library.read_text())
            elif "core/Cargo.toml" in arg("--manifest-path"):
                target = Path(arg("--target-dir")) if "--target-dir" in sys.argv else root / "core/target"
                library = target / "release/libpennel_core.dylib"
                library.parent.mkdir(parents=True, exist_ok=True)
                library.write_text(os.environ["GENERATION"])
            elif os.environ.get("FAIL_RELAY"):
                sys.exit(1)
        """)
        self.stub("swiftc", """
            from pathlib import Path
            library = Path(arg("-L")) / "libpennel_core.dylib"
            binding = next(Path(item) for item in sys.argv if item.endswith("/pennel_core.swift"))
            assert library.read_text() == binding.read_text()
            Path(arg("-o")).write_text("linked-" + library.read_text())
        """)
        self.stub("install_name_tool", "pass")
        self.stub("otool", '''
            print("synthetic probe:\\n\\t@rpath/libpennel_core.dylib (compatibility version 0)")
        ''')
        self.stub("codesign", """
            import json
            if "--sign" in sys.argv:
                with (root / "signed.jsonl").open("a") as output:
                    print(json.dumps(arg("--sign")), file=output)
            if "--verify" in sys.argv and os.environ.get("FAIL_VERIFICATION"):
                sys.exit(1)
        """)
        (scripts / "mac_signing.py").write_text(textwrap.dedent(f"""
            from pathlib import Path
            import json, sys
            output = Path(sys.argv[sys.argv.index("--output") + 1])
            output.mkdir(parents=True, exist_ok=True)
            (output / "signing-plan.json").write_text(json.dumps({{"identity": "{FINGERPRINT}"}}))
            for name in ("embedded.provisionprofile", "Pennel.entitlements"):
                (output / name).write_text("synthetic")
        """))

    def stub(self, name, body):
        path = self.tools / name
        path.write_text(f"#!{sys.executable}\n" + textwrap.dedent("""
            from pathlib import Path
            import os, sys
            root = Path(os.environ["QA_TEST_ROOT"])
            def arg(name):
                return sys.argv[sys.argv.index(name) + 1]
        """) + textwrap.dedent(body))
        path.chmod(0o755)

    def run_script(self, name, *, check=True, **environment):
        return subprocess.run(["/bin/bash", self.root / "scripts" / f"{name}-peer-comparison.sh"],
                              env={**self.env, **environment}, capture_output=True, text=True, check=check)

    def assert_bundle(self, generation):
        self.assertEqual((self.app / "MacOS/peer-comparison-probe").read_text(), "linked-" + generation)
        self.assertEqual((self.app / "Frameworks/libpennel_core.dylib").read_text(), generation)

    def test_legacy_probe_requires_a_rebuild_even_if_the_ordinary_core_exists(self):
        legacy = self.output / "bin/peer-comparison-probe"
        legacy.parent.mkdir(parents=True)
        legacy.write_text("legacy")
        ordinary = self.root / "core/target/release/libpennel_core.dylib"
        ordinary.parent.mkdir(parents=True)
        ordinary.write_text("ordinary")
        result = self.run_script("sign", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("build-peer-comparison.sh", result.stderr)
        self.assertFalse((self.root / "signed.jsonl").exists())

    def test_only_complete_builds_can_replace_the_signable_pair(self):
        self.run_script("build", GENERATION="first")
        ordinary = self.root / "core/target/release/libpennel_core.dylib"
        ordinary.parent.mkdir(parents=True, exist_ok=True)
        ordinary.write_text("ordinary")
        self.run_script("sign")
        self.assert_bundle("first")
        self.assertNotEqual(self.run_script("build", GENERATION="failed", FAIL_RELAY="1", check=False).returncode, 0)
        self.run_script("sign")
        self.assert_bundle("first")
        self.run_script("build", GENERATION="second")
        self.assertNotEqual(self.run_script("sign", FAIL_VERIFICATION="1", check=False).returncode, 0)
        self.assert_bundle("first")
        self.run_script("sign")
        self.assert_bundle("second")
        self.assertEqual([json.loads(line) for line in (self.root / "signed.jsonl").read_text().splitlines()],
                         [FINGERPRINT] * 8)


if __name__ == "__main__":
    unittest.main()
