#!/usr/bin/env python3
"""Exercise the real updater generator with isolated, synthetic macOS bundles."""
import io
import json
import os
from pathlib import Path
import plistlib
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "scripts/automation/render-update-manifest.py"


class UpdateManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        config = self.root / "app/src-tauri/tauri.conf.json"
        config.parent.mkdir(parents=True)
        config.write_text(json.dumps({"version": "1.2.3"}), encoding="utf-8")
        self.archive = config.parent / "target/release/bundle/macos/AgentIsland.app.tar.gz"
        self.archive.parent.mkdir(parents=True)
        self.signature = self.archive.with_name(self.archive.name + ".sig")
        self.output = self.root / "dist/latest.json"
        self.make_archive()
        self.signature.write_text("synthetic-fixture-signature\n", encoding="utf-8")

    def make_archive(self, version="1.2.3", universal=False):
        info = plistlib.dumps({"CFBundleShortVersionString": version, "CFBundleExecutable": "agentisland"})
        binary = struct.pack("<II", 0xFEEDFACF, 0x0100000C)
        if universal:
            binary = struct.pack(">II", 0xCAFEBABE, 2)
            binary += struct.pack(">IIIII", 0x01000007, 0, 0, 0, 0)
            binary += struct.pack(">IIIII", 0x0100000C, 0, 0, 0, 0)
        with tarfile.open(self.archive, "w:gz") as bundle:
            for name, data in (("Info.plist", info), ("MacOS/agentisland", binary)):
                member = tarfile.TarInfo("AgentIsland.app/Contents/" + name)
                member.size = len(data)
                bundle.addfile(member, io.BytesIO(data))
        os.utime(self.archive, (1700000000, 1700000000))

    def run_generator(self, mode="--write"):
        return subprocess.run([sys.executable, str(GENERATOR), "--root", str(self.root), mode],
                              capture_output=True, text=True)

    def test_manifest_fields_and_check(self):
        result = self.run_generator()
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(manifest["version"], "1.2.3")
        self.assertEqual(manifest["pub_date"], "2023-11-14T22:13:20Z")
        self.assertEqual(manifest["platforms"], {"darwin-aarch64": {
            "signature": "synthetic-fixture-signature",
            "url": "https://github.com/bitterSmilezzz/AgentIsland/releases/download/v1.2.3/AgentIsland.app.tar.gz",
        }})
        # `notes` 是展示给用户的更新说明，**不得含 HTML 注释**——
        # 初版把它当成生成块标记，界面一渲染就会把注释露给用户。
        self.assertNotIn("<!--", manifest["notes"])
        self.assertNotIn("-->", manifest["notes"])
        self.assertIn("1.2.3", manifest["notes"])
        self.assertEqual(self.run_generator("--check").returncode, 0)
        before = self.output.read_bytes()
        self.assertEqual(self.run_generator().returncode, 0)
        self.assertEqual(self.output.read_bytes(), before)

    def test_missing_artifact_and_signature_fail_without_output(self):
        # 无产物时 **`--write` 必须失败**：没有 .app.tar.gz 却写出清单就是伪造发布信息。
        self.archive.unlink()
        result = self.run_generator()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no .app.tar.gz", result.stderr)
        self.assertFalse(self.output.exists())
        self.make_archive()
        self.signature.unlink()
        self.assertNotEqual(self.run_generator().returncode, 0)
        self.assertFalse(self.output.exists())

    def test_check_without_a_bundle_is_not_treated_as_drift(self):
        """「本机没构建签名包」与「清单漂移」是两件事，不能都判失败。

        初版把两者都判失败，于是 `verify-framework.sh` 在任何没跑过带签名构建的
        干净克隆上恒红（实测 test-framework-verifier.py 20 项里 5 项红）。
        """
        self.archive.unlink()
        if self.output.exists():
            self.output.unlink()
        checked = self.run_generator("--check")
        self.assertEqual(checked.returncode, 0,
                         "无产物时 --check 应通过（无物可校验），不该判成漂移")
        self.assertIn("nothing to check", checked.stdout)
        # 同一状态下 --write 仍然失败：不能凭空编一份清单。
        self.assertNotEqual(self.run_generator().returncode, 0)

    def test_check_rejects_drift_without_repair(self):
        self.assertEqual(self.run_generator().returncode, 0)
        self.output.write_text("{}\n", encoding="utf-8")
        result = self.run_generator("--check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("drifted", result.stderr)
        self.assertEqual(self.output.read_text(encoding="utf-8"), "{}\n")

    def test_missing_output_is_rejected(self):
        self.assertNotEqual(self.run_generator("--check").returncode, 0)
        self.assertFalse(self.output.exists())

    def test_stale_archive_and_ambiguous_archives_are_rejected(self):
        self.make_archive(version="1.2.2")
        result = self.run_generator()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("version does not match", result.stderr)
        self.archive.with_name("Other.app.tar.gz").write_bytes(b"fixture")
        self.assertNotEqual(self.run_generator().returncode, 0)

    def test_universal_bundle_advertises_both_actual_architectures(self):
        self.make_archive(universal=True)
        result = self.run_generator()
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(set(manifest["platforms"]), {"darwin-aarch64", "darwin-x86_64"})


if __name__ == "__main__":
    unittest.main(verbosity=2)
