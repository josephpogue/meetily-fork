"""Signing scripts must not treat an untrusted identity as usable.

`security` is stubbed on PATH; the real keychain is never touched.
"""
import os
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
UNTRUSTED = '  2) 4831CFF4A1B2C3D4E5F60718293A4B5C6D7E8F90 "meetily-fork local" (CSSMERR_TP_NOT_TRUSTED)'
TRUSTED = '  2) 4831CFF4A1B2C3D4E5F60718293A4B5C6D7E8F90 "meetily-fork local"'


def stub_env(tmp, identity_line):
    """PATH with a fake `security`; `trusted.flag` appears once add-trusted-cert runs."""
    bin_dir = Path(tmp) / "bin"
    bin_dir.mkdir()
    flag = Path(tmp) / "trusted.flag"
    stub = bin_dir / "security"
    stub.write_text(textwrap.dedent(f"""\
        #!/bin/bash
        case "$1" in
          find-identity)
            if [ -e "{flag}" ]; then echo '{TRUSTED}'; else echo '{identity_line}'; fi ;;
          find-certificate) echo "-----BEGIN CERTIFICATE-----"; exit 0 ;;
          add-trusted-cert) touch "{flag}" ;;
        esac
        """))
    stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", MEETILY_KEYCHAIN="/nonexistent.keychain-db")
    return env, flag


def run(script, env, *args):
    return subprocess.run(["bash", str(SCRIPTS / script), *args], env=env, capture_output=True, text=True, timeout=30)


class SigningScripts(unittest.TestCase):
    def setUp(self):
        self._d = tempfile.TemporaryDirectory()
        self.addCleanup(self._d.cleanup)
        self.tmp = self._d.name

    def test_untrusted_identity_is_trusted_not_skipped(self):
        env, flag = stub_env(self.tmp, UNTRUSTED)
        r = run("setup-mac-signing.sh", env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(flag.exists(), "trust step never ran")
        self.assertNotIn("Nothing to do", r.stdout)
        self.assertIn("password window", r.stdout)

    def test_trusted_identity_is_left_alone(self):
        env, flag = stub_env(self.tmp, TRUSTED)
        r = run("setup-mac-signing.sh", env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("Nothing to do", r.stdout)
        self.assertFalse(flag.exists())

    def test_install_refuses_untrusted_identity(self):
        env, _ = stub_env(self.tmp, UNTRUSTED)
        r = run("install-mac.sh", env, "--build")
        self.assertEqual(r.returncode, 1)
        self.assertIn("No trusted signing identity", r.stderr)


if __name__ == "__main__":
    unittest.main()
