"""Installer regression tests. Stub networking; never install into a real system directory."""

import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


@unittest.skipIf(os.name == "nt", "Unix installer")
class UnixInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="howdo-installer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.install_dir = self.root / "installed"
        self.install_dir.mkdir()
        self.destination = self.install_dir / "howdo"
        self.destination.write_bytes(b"existing binary")
        self.payload = b"#!/bin/sh\necho downloaded-test-binary\n"
        self.env = dict(
            os.environ,
            HOWDO_INSTALL_DIR=str(self.install_dir),
            PATH=str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            HOWDO_INSTALL_TEST_ROOT=str(self.root),
            TMPDIR=str(self.root),
        )
        target = ("aarch64" if platform.machine().lower() in ["arm64", "aarch64"] else "x86_64") + (
            "-apple-darwin" if sys.platform == "darwin" else "-unknown-linux-gnu"
        )
        self.asset = "howdo-" + target
        (self.root / "release.json").write_text(json.dumps({"tag_name": "v0.2.0"}))
        (self.root / "payload").write_bytes(self.payload)
        self.manifest = self.root / "manifest"
        self.manifest.write_text(f"{hashlib.sha256(self.payload).hexdigest()}  {self.asset}\n")
        curl = self.bin / "curl"
        curl.write_text(
            f"#!{sys.executable}\n"
            + """
import os, pathlib, shutil, sys
args = sys.argv[1:]
destination = args[args.index('-o') + 1]
url = args[-1]
root = pathlib.Path(os.environ['HOWDO_INSTALL_TEST_ROOT'])
source = 'release.json' if url.endswith('/releases/latest') else 'manifest' if url.endswith('/SHA256SUMS') else 'payload'
shutil.copyfile(root / source, destination)
"""
        )
        curl.chmod(0o755)

    def run_installer(self):
        return subprocess.run(
            ["sh", str(ROOT / "install.sh")],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_verified_install_is_atomic_and_cleans_temporary_files(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.destination.read_bytes(), self.payload)
        self.assertEqual(self.destination.stat().st_mode & 0o777, 0o755)
        self.assertEqual(list(self.root.glob("howdo-install.*")), [])
        self.assertEqual(list(self.install_dir.glob(".howdo.*")), [])

    def test_bad_missing_and_duplicate_checksums_preserve_existing_binary(self):
        correct = self.manifest.read_text()
        for manifest in ["0" * 64 + "  " + self.asset, "", correct + correct]:
            with self.subTest(manifest=manifest):
                self.manifest.write_text(manifest)
                result = self.run_installer()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.destination.read_bytes(), b"existing binary")
                self.assertEqual(list(self.root.glob("howdo-install.*")), [])

    def test_invalid_release_tag_does_not_install(self):
        (self.root / "release.json").write_text(json.dumps({"tag_name": "malformed/tag"}))
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertEqual(self.destination.read_bytes(), b"existing binary")

    def test_relative_install_directory_is_rejected(self):
        self.env["HOWDO_INSTALL_DIR"] = "relative"
        self.assertNotEqual(self.run_installer().returncode, 0)


@unittest.skipUnless(os.name == "nt", "Windows installer")
class WindowsInstallerTests(unittest.TestCase):
    def setUp(self):
        self.powershell = shutil.which("powershell") or shutil.which("pwsh")
        if not self.powershell:
            self.skipTest("PowerShell is unavailable")
        self.temp = tempfile.TemporaryDirectory(prefix="howdo-windows-installer-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.install_dir = self.root / "installed"
        self.install_dir.mkdir()
        self.destination = self.install_dir / "howdo.exe"
        self.destination.write_bytes(b"existing binary")
        self.payload = b"MZ fake binary for installer verification only"
        self.asset = "howdo-x86_64-pc-windows-msvc.exe"
        self.env = dict(
            os.environ,
            HOWDO_INSTALL_DIR=str(self.install_dir),
            HOWDO_NO_PATH_UPDATE="1",
            HOWDO_INSTALL_TEST_ROOT=str(self.root),
            HOWDO_INSTALL_TEST_SCRIPT=str(ROOT / "install.ps1"),
            PROCESSOR_ARCHITECTURE="AMD64",
            TEMP=str(self.root),
            TMP=str(self.root),
        )
        self.env.pop("PROCESSOR_ARCHITEW6432", None)
        self.metadata = {
            "tag_name": "v0.2.0",
            "assets": [
                {
                    "name": name,
                    "browser_download_url": f"https://github.com/GitAashishG/howdo/releases/download/v0.2.0/{name}",
                }
                for name in [self.asset, "SHA256SUMS"]
            ],
        }
        self.release_file = self.root / "release.json"
        self.release_file.write_text(json.dumps(self.metadata))
        (self.root / "payload").write_bytes(self.payload)
        self.manifest = self.root / "manifest"
        self.manifest.write_text(f"{hashlib.sha256(self.payload).hexdigest()}  {self.asset}\n")
        self.harness = self.root / "harness.ps1"
        self.harness.write_text("""
function Invoke-RestMethod {
    param($Uri, $Headers, $TimeoutSec)
    Get-Content -LiteralPath (Join-Path $env:HOWDO_INSTALL_TEST_ROOT 'release.json') -Raw | ConvertFrom-Json
}
function Invoke-WebRequest {
    param($Uri, $OutFile, $Headers, [switch]$UseBasicParsing, $TimeoutSec)
    $source = if ($Uri.EndsWith('/SHA256SUMS')) { 'manifest' } else { 'payload' }
    Copy-Item -LiteralPath (Join-Path $env:HOWDO_INSTALL_TEST_ROOT $source) -Destination $OutFile
}
try { & $env:HOWDO_INSTALL_TEST_SCRIPT } catch { Write-Error $_; exit 1 }
""")

    def run_installer(self):
        return subprocess.run(
            [self.powershell, "-NoProfile", "-NonInteractive", "-File", str(self.harness)],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=15,
        )

    def test_verified_install_and_cleanup(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.destination.read_bytes(), self.payload)
        self.assertEqual(list(self.root.glob("howdo-install-*")), [])
        self.assertEqual(list(self.install_dir.glob(".howdo-*")), [])

    def test_checksum_failures_preserve_existing_binary(self):
        correct = self.manifest.read_text()
        for manifest in ["0" * 64 + "  " + self.asset, "", correct + correct]:
            with self.subTest(manifest=manifest):
                self.manifest.write_text(manifest)
                result = self.run_installer()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.destination.read_bytes(), b"existing binary")
                self.assertEqual(list(self.root.glob("howdo-install-*")), [])

    def test_missing_assets_and_invalid_tag_do_not_install(self):
        for metadata in [
            {**self.metadata, "assets": []},
            {**self.metadata, "tag_name": "malformed/tag"},
        ]:
            with self.subTest(metadata=metadata):
                self.release_file.write_text(json.dumps(metadata))
                self.assertNotEqual(self.run_installer().returncode, 0)
                self.assertEqual(self.destination.read_bytes(), b"existing binary")

    def test_relative_install_directory_is_rejected(self):
        self.env["HOWDO_INSTALL_DIR"] = "relative"
        self.assertNotEqual(self.run_installer().returncode, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
