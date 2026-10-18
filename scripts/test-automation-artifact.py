#!/usr/bin/env python3
"""Build-receipt meta tests. Synthetic inputs only; no git/build/app/user data.

Run: python3 scripts/test-automation-artifact.py
Two real ablations (--ablation-experiments) remove source/artifact comparison in
an in-memory mutant and show the exact otherwise rejected fixture turns green.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
# artifact.py needs stdlib tomllib. Fail with an actionable message instead of a bare
# ImportError traceback, so a wrong interpreter is never mistaken for a broken check.
if sys.version_info < (3, 11):
    print(f"Python >= 3.11 required (tomllib); running {sys.version_info.major}.{sys.version_info.minor}",
          file=sys.stderr)
    sys.exit(3)
HERE = Path(__file__).resolve().parent
MODULE = HERE / "automation/artifact.py"
spec = importlib.util.spec_from_file_location("artifact_meta", MODULE)
artifact = importlib.util.module_from_spec(spec)
spec.loader.exec_module(artifact)
PY = sys.executable


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="artifact-meta-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.write("app/src-tauri/Cargo.toml", '[package]\nname="agentisland"\nversion="1.2.3"\n[patch.crates-io]\ntao={path="../../vendor/tao"}\n')
        self.write("app/src-tauri/Cargo.lock", "version = 4\n")
        self.write("app/src-tauri/build.rs", "fn main() {}\n")
        self.write("app/src-tauri/src/main.rs", "fn main() {}\n")
        self.write("app/src-tauri/src/engine.rs", "const VALUE: u8 = 1;\n")
        self.write("app/src-tauri/tauri.conf.json", json.dumps({"build": {"frontendDist": "../ui"},
                   "bundle": {"icon": ["icons/test.png"], "macOS": {"infoPlist": "Info.plist"}}}))
        self.write("app/src-tauri/Info.plist", "<plist/>")
        self.write("app/src-tauri/icons/test.png", "fake icon")
        self.write("app/src-tauri/capabilities/default.json", "{}")
        self.write("app/src-tauri/permissions/default.toml", "[default]\n")
        self.write("app/ui/index.html", "<p>Synthetic</p>")
        self.write("app/ui/js/view.js", "const value=1;")
        self.write("app/ui/assets/image.svg", "<svg/>")
        self.write("vendor/tao/Cargo.toml", '[package]\nname="tao"\nversion="0.1.0"\n')
        self.write("vendor/tao/src/lib.rs", "pub fn synthetic() {}\n")
        # Copy the real common.sh: a stale hard-coded copy silently diverges from the
        # product and makes the shell path fail for the wrong reason.
        self.write("scripts/common.sh", (HERE / "common.sh").read_text())
        self.write("scripts/heavy-build-lock.sh", (HERE / "heavy-build-lock.sh").read_text())
        self.write("scripts/automation/heavy_build_lock.py",
                   (HERE / "automation/heavy_build_lock.py").read_text())
        self.write("scripts/check-version.py", "pass\n")
        self.write("scripts/build-app.sh", (HERE / "build-app.sh").read_text())
        self.write("scripts/automation/artifact.py", MODULE.read_text())
        self.write("CHANGELOG.md", "## [1.2.3]\n")

    def write(self, name, value, mode=None):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value, encoding="utf-8")
        if mode is not None:
            path.chmod(mode)
        return path

    def artifacts(self):
        for path in artifact.BINARIES:
            self.write(path, "#!/bin/sh\nexit 0\n", 0o755)
        self.write("dist/AgentIsland.app/Contents/Info.plist", "<plist>synthetic</plist>")
        self.write("dist/AgentIsland.app/Contents/Resources/icon.png", "synthetic")
        self.write("dist/AgentIsland.app/Contents/_CodeSignature/CodeResources", "synthetic signature")

    def build(self):
        start = artifact.begin(self.root)
        self.artifacts()
        result = artifact.finish(self.root, start["build_id"])
        self.assertTrue(result["ok"], result)
        return result

    def assert_code(self, code, **kwargs):
        self.assertEqual(artifact.verify(self.root, **kwargs), {"ok": False, "code": code})

    def test_valid_new_dist_and_private_receipt_and_manifest(self):
        digest = artifact.source_digest(self.root)
        self.assertFalse((self.root / "dist").exists())
        result = self.build()
        self.assertEqual(result["source_manifest_sha256"], digest)
        receipt = self.root / artifact.RECEIPT
        self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
        names = {e["path"] for e in json.loads(receipt.read_text())["source_manifest"]["files"]}
        self.assertIn("app/src-tauri/src/engine.rs", names)
        self.assertIn("vendor/tao/src/lib.rs", names)
        self.assertNotIn(artifact.RECEIPT, names)
        self.assertTrue(artifact.verify(self.root, expected_sourcehash=digest,
                                      expected_receipt_sha256=result["receipt_sha256"])["ok"])

    def test_nonmain_content_change_with_identical_mtime_and_size(self):
        self.build()
        path = self.root / "app/src-tauri/src/engine.rs"
        info = path.stat()
        path.write_text(path.read_text().replace("1", "2"))
        os.utime(path, ns=(info.st_atime_ns, info.st_mtime_ns))
        self.assertEqual(path.stat().st_mtime_ns, info.st_mtime_ns)
        self.assertEqual(path.stat().st_size, info.st_size)
        self.assert_code("source_drift")

    def test_mtime_alone_is_not_an_identity_change(self):
        self.build()
        path = self.root / "app/src-tauri/src/main.rs"
        os.utime(path, (1, 1))
        self.assertTrue(artifact.verify(self.root)["ok"])

    def test_all_build_input_categories_change_digest(self):
        paths = ["app/src-tauri/Cargo.lock", "app/src-tauri/build.rs", "app/src-tauri/Info.plist",
                 "app/src-tauri/src/engine.rs", "app/src-tauri/icons/test.png",
                 "app/src-tauri/capabilities/default.json", "app/src-tauri/permissions/default.toml",
                 "app/ui/js/view.js", "app/ui/assets/image.svg", "vendor/tao/src/lib.rs",
                 "scripts/build-app.sh", "scripts/heavy-build-lock.sh",
                 "scripts/automation/heavy_build_lock.py"]
        for name in paths:
            with self.subTest(name=name):
                before = artifact.source_digest(self.root)
                path = self.root / name
                path.write_text(path.read_text() + "\n// change\n")
                self.assertNotEqual(before, artifact.source_digest(self.root))

    def test_added_and_removed_nonmain_source(self):
        self.build()
        path = self.write("app/src-tauri/src/new.rs", "// synthetic")
        self.assert_code("source_drift")
        path.unlink()
        (self.root / "app/src-tauri/src/engine.rs").unlink()
        self.assert_code("source_drift")

    def test_output_pruning_and_unrelated_vendor_not_read(self):
        digest = artifact.source_digest(self.root)
        for name in ["app/src-tauri/target/source.rs", "app/ui/node_modules/lib.js", "dist/receipt.json",
                     ".scratch/source.rs", "app/ui/dist/out.js", "vendor/unused/auth.json"]:
            self.write(name, "SYNTHETIC_PRIVATE_SENTINEL")
        self.assertEqual(digest, artifact.source_digest(self.root))

    def test_generated_schema_only_exclusion_and_bundle_no_pruning(self):
        digest = artifact.source_digest(self.root)
        self.write("app/src-tauri/gen/schemas/desktop-schema.json", "generated")
        self.assertEqual(digest, artifact.source_digest(self.root))
        self.write("app/ui/gen/source.js", "actual source")
        self.assertNotEqual(digest, artifact.source_digest(self.root))
        self.build()
        self.write("dist/AgentIsland.app/Contents/Resources/target/injected", "extra artifact")
        self.assert_code("artifact_changed")

    def test_platform_overlay_external_input_is_rejected(self):
        self.write("app/src-tauri/tauri.macos.conf.json", '{"bundle":{"resources":["../../../outside"]}}')
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.source_digest(self.root)
        self.assertEqual(caught.exception.code, "unsupported_input")

    def test_local_cargo_configuration_content_and_redirection(self):
        before = artifact.source_digest(self.root)
        self.write(".cargo/config.toml", '[build]\nrustflags=["--cfg", "synthetic"]')
        self.assertNotEqual(before, artifact.source_digest(self.root))
        self.write(".cargo/config.toml", '[build]\ntarget-dir="../../../outside"')
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.source_digest(self.root)
        self.assertEqual(caught.exception.code, "unsupported_input")

    def test_recursive_declared_vendor_dependency(self):
        self.write("vendor/tao/Cargo.toml", '[dependencies]\nextra={path="../extra"}\n')
        self.write("vendor/extra/Cargo.toml", '[package]\nname="extra"\nversion="0.1.0"')
        path = self.write("vendor/extra/src/lib.rs", "// first")
        before = artifact.source_digest(self.root)
        path.write_text("// other")
        self.assertNotEqual(before, artifact.source_digest(self.root))

    def test_all_delivered_artifacts_are_bound(self):
        for name in [*artifact.BINARIES, "dist/AgentIsland.app/Contents/Info.plist",
                     "dist/AgentIsland.app/Contents/Resources/icon.png",
                     "dist/AgentIsland.app/Contents/_CodeSignature/CodeResources"]:
            with self.subTest(name=name):
                self.build()
                path = self.root / name
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                self.assert_code("artifact_changed")
                path.write_bytes(original)

    def test_missing_and_nonexecutable_binary_and_helper(self):
        for name in artifact.BINARIES:
            with self.subTest(name=name):
                self.build()
                path = self.root / name
                path.chmod(0o644)
                self.assert_code("artifact_unreadable")
                path.unlink()
                self.assert_code("artifact_missing")

    def test_missing_source_is_never_silently_skipped(self):
        self.build()
        (self.root / "app/src-tauri/src/main.rs").unlink()
        self.assert_code("source_missing")

    def test_missing_receipt_never_certifies_old_artifacts(self):
        self.artifacts()
        self.assert_code("receipt_missing")

    def test_pending_failed_attempt_blocks_previous_good_receipt(self):
        self.build()
        original = (self.root / artifact.RECEIPT).read_bytes()
        start = artifact.begin(self.root)
        self.assert_code("build_in_progress")
        artifact.fail(self.root, start["build_id"])
        self.assert_code("build_in_progress")
        self.assertEqual((self.root / artifact.RECEIPT).read_bytes(), original)
        self.build()
        self.assertTrue(artifact.verify(self.root)["ok"])

    def test_drift_between_begin_and_finish_preserves_original(self):
        self.build()
        original = (self.root / artifact.RECEIPT).read_bytes()
        start = artifact.begin(self.root)
        self.write("app/ui/js/view.js", "new source during build")
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.finish(self.root, start["build_id"])
        self.assertEqual(caught.exception.code, "source_drift")
        self.assertEqual((self.root / artifact.RECEIPT).read_bytes(), original)
        self.assert_code("build_in_progress")

    def test_broken_future_duplicate_and_edited_receipts_preserved(self):
        self.build()
        path = self.root / artifact.RECEIPT
        good = path.read_bytes()
        value = json.loads(good)
        future = dict(value, schema_version=99)
        changed = dict(value, version="9.9.9")
        samples = [(b'{"schema":', "receipt_invalid"),
                   (b'{"schema":1,"schema":2}', "receipt_invalid"),
                   (json.dumps(future).encode(), "receipt_schema"),
                   (json.dumps(changed).encode(), "receipt_integrity"),
                   (b'[]', "receipt_invalid"),
                   (json.dumps(dict(value, schema_version=True)).encode(), "receipt_schema")]
        for data, code in samples:
            with self.subTest(code=code, data=data[:20]):
                path.write_bytes(data)
                self.assert_code(code)
                with self.assertRaises(artifact.ArtifactError):
                    artifact.begin(self.root)
                self.assertEqual(path.read_bytes(), data)
        path.write_bytes(good)

    def test_sealed_manifest_and_artifact_changes_still_rejected(self):
        self.build()
        path = self.root / artifact.RECEIPT
        value = json.loads(path.read_bytes())
        value.pop("receipt_sha256")
        value["source_manifest"]["files"] = []
        value["source_manifest_sha256"] = artifact._sha(value["source_manifest"])
        path.write_bytes(artifact._canonical(artifact._seal(value)))
        self.assert_code("receipt_invalid")
        path.unlink()  # Explicit synthetic recovery; production preserves the bad original.
        self.build()
        value = json.loads(path.read_bytes())
        value.pop("receipt_sha256")
        value["artifacts"] = []
        path.write_bytes(artifact._canonical(artifact._seal(value)))
        self.assert_code("receipt_invalid")

    def test_resealed_receipt_other_hashes_still_fail_content_comparison(self):
        # A writer who can rewrite the receipt re-seals it; only re-reading the declared
        # bytes (not trusting the receipt) can reject that, so this pins both comparisons.
        self.build()
        path = self.root / artifact.RECEIPT
        value = json.loads(path.read_bytes())
        value.pop("receipt_sha256")
        entry = next(e for e in value["source_manifest"]["files"] if e["path"].endswith("engine.rs"))
        entry["sha256"] = "0" * 64
        value["source_manifest_sha256"] = artifact._sha(value["source_manifest"])
        path.write_bytes(artifact._canonical(artifact._seal(value)))
        self.assert_code("source_drift")
        self.build()
        value = json.loads(path.read_bytes())
        value.pop("receipt_sha256")
        next(e for e in value["artifacts"] if e["path"].endswith("Contents/Info.plist"))["sha256"] = "0" * 64
        path.write_bytes(artifact._canonical(artifact._seal(value)))
        self.assert_code("artifact_changed")

    def test_pinned_independent_receipt_hash(self):
        result = self.build()
        self.assert_code("receipt_integrity", expected_receipt_sha256="0" * 64)
        self.assertTrue(artifact.verify(self.root, expected_receipt_sha256=result["receipt_sha256"])["ok"])

    def test_symlinks_receipt_source_binary_and_parent_refused(self):
        for name in [artifact.RECEIPT, "app/src-tauri/src/engine.rs", *artifact.BINARIES]:
            with self.subTest(name=name):
                self.build()
                path = self.root / name
                saved = path.read_bytes()
                target = self.write("outside-synthetic", saved.decode())
                path.unlink()
                path.symlink_to(target)
                self.assert_code("unsafe_path")
                path.unlink()
                path.write_bytes(saved)
                path.chmod(0o600 if name == artifact.RECEIPT else 0o755)
        shutil.rmtree(self.root / "app/ui")
        (self.root / "app/ui").symlink_to(self.root / "vendor/tao", target_is_directory=True)
        self.assert_code("unsafe_path")

    def test_symlink_dist_does_not_write_external_receipt(self):
        outside = self.root / "external"
        outside.mkdir()
        (self.root / "dist").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.begin(self.root)
        self.assertEqual(caught.exception.code, "unsafe_path")
        self.assertEqual(list(outside.iterdir()), [])

    def test_credential_named_input_is_rejected_without_read(self):
        path = self.write("app/ui/.env", "SYNTHETIC_PRIVATE_SENTINEL")
        real = artifact._read
        def read(root, name, *args):
            self.assertNotEqual(name, "app/ui/.env")
            return real(root, name, *args)
        with patch.object(artifact, "_read", side_effect=read):
            with self.assertRaises(artifact.ArtifactError) as caught:
                artifact.source_digest(self.root)
        self.assertEqual(caught.exception.code, "unsafe_path")
        self.assertTrue(path.exists())

    def test_external_dependency_and_external_ui_fail_closed(self):
        self.write("app/src-tauri/Cargo.toml", '[package]\nversion="1.2.3"\n[dependencies]\nprivate={path="../../../external"}')
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.source_digest(self.root)
        self.assertEqual(caught.exception.code, "unsupported_input")
        self.write("app/src-tauri/Cargo.toml", '[package]\nversion="1.2.3"')
        self.write("app/src-tauri/tauri.conf.json", '{"build":{"frontendDist":"../../external"}}')
        with self.assertRaises(artifact.ArtifactError):
            artifact.source_digest(self.root)

    def test_profile_platform_expected_hash_mismatch(self):
        result = self.build()
        self.assert_code("profile_mismatch", profile="debug")
        self.assert_code("source_drift", expected_sourcehash="0" * 64)
        self.assert_code("invalid_argument", expected_sourcehash="malformed")
        with patch.object(artifact, "_platform", return_value=("linux", "synthetic")):
            self.assert_code("platform_mismatch")
        self.assertTrue(artifact.verify(self.root, expected_sourcehash=result["source_manifest_sha256"])["ok"])

    def test_concurrent_build_and_wrong_token_rejected(self):
        start = artifact.begin(self.root)
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.begin(self.root)
        self.assertEqual(caught.exception.code, "build_in_progress")
        self.artifacts()
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.finish(self.root, "0" * 32)
        self.assertEqual(caught.exception.code, "build_token_mismatch")
        artifact.fail(self.root, start["build_id"])
        with self.assertRaises(artifact.ArtifactError) as caught:
            artifact.finish(self.root, start["build_id"])
        self.assertEqual(caught.exception.code, "build_not_active")

    def test_readonly_status_fixed_json_and_nonzero(self):
        before = list(self.root.iterdir())
        cmd = [PY, str(MODULE), "status", "--root", str(self.root)]
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=5)
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(json.loads(proc.stdout), {"ok": False, "code": "receipt_missing"})
        self.assertEqual(proc.stderr, "")
        self.assertEqual(before, list(self.root.iterdir()))
        proc = subprocess.run([PY, str(MODULE), "bad-command"], capture_output=True, text=True, timeout=5)
        self.assertEqual(json.loads(proc.stdout), {"ok": False, "code": "invalid_argument"})
        self.assertNotIn(str(self.root), proc.stdout + proc.stderr)

    def fake_build(self, mode):
        # Execute the real packaging shell using synthetic command shims; never cargo.
        shim = self.write("shims/cargo", """#!/bin/sh
# The packaging shell must pass its lock descriptor through to command children.
"$ARTIFACT_FIXTURE_PY" "$ARTIFACT_FIXTURE_ROOT/scripts/automation/heavy_build_lock.py" --root "$ARTIFACT_FIXTURE_ROOT" -- true 2>/dev/null
[ "$?" = "2" ] || exit 97
[ "$1" = "tauri" ] || exit 0
[ "$ARTIFACT_FIXTURE_MODE" != "failure" ] || exit 42
mkdir -p target/release/bundle/macos/AgentIsland.app/Contents/MacOS
printf '#!/bin/sh\\nexit 0\\n' > target/release/agentisland
chmod +x target/release/agentisland
cp target/release/agentisland target/release/bundle/macos/AgentIsland.app/Contents/MacOS/agentisland
if [ "$ARTIFACT_FIXTURE_MODE" = "drift" ]; then
  printf '// drift\\n' >> src/engine.rs
fi
# --bundles dmg 时还要产出 DMG 文件：build-app.sh 随后会 `hdiutil attach` 做签名自校验。
# 桩件不造它，构建会在「DMG 未产出」处失败——那是**真实的失败**（发版确实没包可发）。
mkdir -p app/src-tauri/target/release/bundle/dmg
printf 'synthetic dmg payload\\n' > "app/src-tauri/target/release/bundle/dmg/AgentIsland_0.0.324_aarch64.dmg"
exit 0
""", 0o755)
        # `cargo tauri build --bundles dmg` 的桩：造一个**签名完整**的 DMG。
        # build-app.sh 出包后会挂载它做 `codesign --verify --strict --deep` 自校验，
        # 桩件不造 DMG 的话那一步会失败（真实构建不受影响，只有夹具受影响）。
        self.write("shims/hdiutil", """#!/bin/sh
# 桩：只回答自校验脚本会问的两件事——挂载点与「校验通过」。
if [ "$1" = "attach" ]; then
  # 桩件里没有真挂载，但自校验要看到「挂载点 + app 目录」才可能通过；
  # codesign 本身也是桩（exit 0），所以造出目录形状即可。
  # 挂载点放在**夹具自己的临时目录**里：不需要 /Volumes 权限，也不会污染宿主机。
  # build-app.sh 读 hdiutil 输出的第三列（挂载路径），所以这里给什么路径都行。
  mkdir -p "$ARTIFACT_FIXTURE_ROOT/fixture-mount/AgentIsland.app"
  printf '/dev/disk9s1\tApple_HFS\t%s\n' "$ARTIFACT_FIXTURE_ROOT/fixture-mount"
  exit 0
fi
if [ "$1" = "detach" ]; then
  rm -rf "$ARTIFACT_FIXTURE_ROOT/fixture-mount" 2>/dev/null || true
  exit 0
fi
exit 0
""", 0o755)
        self.write("shims/codesign", "#!/bin/sh\nexit 0\n", 0o755)
        return subprocess.run(["/bin/bash", str(self.root / "scripts/build-app.sh"), "1.2.3"],
                              cwd=self.root, env={**os.environ, "PATH": str(shim.parent) + os.pathsep + os.environ["PATH"],
                              "SKIP_TESTS": "1", "ARTIFACT_FIXTURE_MODE": mode,
                              "ARTIFACT_FIXTURE_PY": PY, "ARTIFACT_FIXTURE_ROOT": str(self.root)}, 
                              capture_output=True, text=True, timeout=10)

    def test_shell_success_creates_receipt_after_signature_checks(self):
        proc = self.fake_build("success")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue(artifact.verify(self.root)["ok"])
        script = (self.root / "scripts/build-app.sh").read_text()
        self.assertLess(script.index("artifact.py begin"), script.index("cargo test --locked"))
        self.assertLess(script.index("codesign --verify --deep"), script.index("artifact.py finish"))

    def test_shell_busy_rejects_before_begin_and_preserves_receipt_state(self):
        self.build()
        before = (self.root / artifact.RECEIPT).read_bytes()
        holder = subprocess.Popen(
            [PY, str(self.root / 'scripts/automation/heavy_build_lock.py'),
             '--root', str(self.root), '--', PY, '-c',
             "print('READY', flush=True); input()"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            import select
            self.assertTrue(select.select([holder.stdout], [], [], 5)[0])
            self.assertEqual(holder.stdout.readline().strip(), 'READY')
            proc = self.fake_build('success')
            self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
            self.assertIn('heavy_build_busy', proc.stderr)
            self.assertEqual((self.root / artifact.RECEIPT).read_bytes(), before)
            self.assertFalse((self.root / artifact.STATE).exists())
        finally:
            holder.stdin.close()
            try:
                holder.wait(timeout=5)
            except subprocess.TimeoutExpired:
                holder.kill()
                holder.wait(timeout=5)
            holder.stdout.close()
            holder.stderr.close()

    def test_shell_begin_rejection_releases_kernel_lock_without_clearing_active(self):
        artifact.begin(self.root)
        before = (self.root / artifact.STATE).read_bytes()
        for _ in range(2):
            proc = self.fake_build('success')
            self.assertEqual(proc.returncode, 3, proc.stdout + proc.stderr)
            self.assertIn('build_in_progress', proc.stdout + proc.stderr)
            self.assertNotIn('heavy_build_busy', proc.stderr)
            self.assertEqual((self.root / artifact.STATE).read_bytes(), before)

    def test_shell_source_drift_is_nonzero_and_old_receipt_blocked(self):
        self.build()
        before = (self.root / artifact.RECEIPT).read_bytes()
        proc = self.fake_build("drift")
        self.assertNotEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn('"code": "source_drift"', proc.stdout)
        self.assertEqual((self.root / artifact.RECEIPT).read_bytes(), before)
        self.assert_code("build_in_progress")

    def test_shell_compile_failure_cannot_reuse_old_green_receipt(self):
        self.build()
        before = (self.root / artifact.RECEIPT).read_bytes()
        proc = self.fake_build("failure")
        self.assertEqual(proc.returncode, 42, proc.stdout + proc.stderr)
        self.assertEqual((self.root / artifact.RECEIPT).read_bytes(), before)
        self.assert_code("build_in_progress")


def ablation_experiments():
    results = []
    for category in ("source", "artifact"):
        fixture = ArtifactTests()
        fixture.setUp()
        try:
            fixture.build()
            if category == "source":
                path = fixture.root / "app/src-tauri/src/engine.rs"
                info = path.stat()
                path.write_text(path.read_text().replace("1", "2"))
                os.utime(path, ns=(info.st_atime_ns, info.st_mtime_ns))
            else:
                fixture.write("dist/AgentIsland.app/Contents/Resources/icon.png", "tampered synthetic")
            baseline = artifact.verify(fixture.root)
            receipt = json.loads((fixture.root / artifact.RECEIPT).read_bytes())
            # Actual execution of a mutant with that single comparison bypassed.
            if category == "source":
                with patch.object(artifact, "source_manifest", return_value=receipt["source_manifest"]):
                    mutant = artifact.verify(fixture.root)
            else:
                with patch.object(artifact, "_artifacts", return_value=receipt["artifacts"]):
                    mutant = artifact.verify(fixture.root)
            assert not baseline["ok"] and mutant["ok"], (baseline, mutant)
            results.append({"ablation": category + "_content_comparison", "baseline_code": baseline["code"],
                            "mutant_code": mutant["code"], "mutant_false_green": mutant["ok"]})
        finally:
            fixture.doCleanups()
    print(json.dumps({"experiments": results}, sort_keys=True))


if __name__ == "__main__":
    if sys.argv[1:] == ["--ablation-experiments"]:
        ablation_experiments()
    else:
        unittest.main(verbosity=2)
