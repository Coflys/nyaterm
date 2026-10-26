from __future__ import annotations

import base64
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
import verify_updater_signatures


@unittest.skipUnless(shutil.which("minisign"), "minisign is required for signature integration tests")
class UpdaterSignatureTests(unittest.TestCase):
    def test_accepts_valid_signature_and_rejects_tampering(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            private = root / "test.key"
            public = root / "test.pub"
            artifact = root / "package.zip"
            artifact.write_bytes(b"test package")
            subprocess.run(["minisign", "-G", "-W", "-p", str(public), "-s", str(private)], check=True)
            subprocess.run(["minisign", "-Sm", str(artifact), "-s", str(private)], check=True)
            signature = root / "package.zip.sig"
            signature.write_bytes(base64.b64encode((root / "package.zip.minisig").read_bytes()))
            verify_updater_signatures.verify_file(artifact, public)
            artifact.write_bytes(b"tampered package")
            with self.assertRaises(subprocess.CalledProcessError):
                verify_updater_signatures.verify_file(artifact, public)
            signature.write_bytes(base64.b64encode(b"invalid signature"))
            with self.assertRaises(subprocess.CalledProcessError):
                verify_updater_signatures.verify_file(artifact, public)


if __name__ == "__main__":
    unittest.main()
