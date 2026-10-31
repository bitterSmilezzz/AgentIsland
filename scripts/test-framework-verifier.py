#!/usr/bin/env python3
"""Fast verifier regressions: real shell/common, isolated fake suite entry points.

Run: python3 scripts/test-framework-verifier.py
No Cargo build, actual gates, opt-in changes or shared-repository writes.
"""
from __future__ import annotations

from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
META = (
    "test-automation-runner", "test-test-registry", "test-automation-artifact",
    "test-automation-adapters", "test-automation-driver", "test-automation-trigger",
    "test-automation-lessons", "test-automation-evidence", "test-automation-development",
    "test-framework-verifier", "test-automation-snapshot", "test-automation-candidate", "test-automation-publish",
    "test-automation-pause",
    "test-automation-break",
)
STUB = r'''
tool=${0##*/}
root=${0%/bin/*}
case "$tool:${1:-}" in
  python*:-c|python*:-) exec "$REAL_PYTHON" "$@" ;;
esac
rc=0
case "$tool:${1:-}" in
  cargo:*) key=cargo; text='test result: ok. 42 passed; 0 failed' ;;
  bash:scripts/test-scan-secrets.sh) key=test-scan-secrets.sh; text='0 失败' ;;
  bash:scripts/scan-secrets.sh) key=scan-secrets.sh; text='扫描通过' ;;
  bash:scripts/check-windows-fragment.sh) key=check-windows-fragment.sh; text='✓ 3 个 Windows 片段全部通过交叉编译验证' ;;
  bash:scripts/check-windows-crate.sh) key=check-windows-crate.sh; text='✓ 整 crate Windows 类型检查通过（排除资源编译；不是标准构建成功）' ;;
  python*:scripts/automation/pipeline.py)
    key=$3
    if [ "$key" = ci ]; then text='{"outcome":"passed"}';
    else text='{"outcome":"blocked"}'; rc=3; fi ;;
  python*:scripts/*.py)
    key=${1##*/}; key=${key%.py}; text=OK
    [ -f "$root/$1" ] || exit 2 ;;
  *) printf 'unexpected fixture entry\n' >&2; exit 2 ;;
esac
source "$root/config.sh"
printf '%s\t%s\t%s\n' "$key" "$tool" "$*" >> "$root/calls.log"
printf '%s\n' "$text"
exit "$rc"
'''


class FrameworkVerifierTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="framework-verifier-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        scripts = self.root / "scripts"
        (scripts / "automation").mkdir(parents=True)
        for name in ("verify-framework.sh", "common.sh"):
            shutil.copy2(SCRIPTS / name, scripts / name)
        for name in META:
            (scripts / f"{name}.py").touch()
        # 生成器类脚本要**真的复制**，不能 touch 成空文件：`verify-framework.sh`
        # 会执行它们，空文件被 python 解释成语法错误（exit 2），
        # 于是每个夹具用例都挂在这一条上（实测 20 项里 5 项红）。
        # 空 touch 只适用于「只要文件存在、不需要被执行」的脚本。
        for name in ("automation/render-matrix.py", "automation/render-status.py",
                     "automation/render-update-manifest.py"):
            target = scripts / name
            source = SCRIPTS / name
            if source.is_file():
                shutil.copy2(source, target)
        for name in ("automation/selftest.py", "automation/pipeline.py",
                     "test-scan-secrets.sh", "scan-secrets.sh",
                     "check-windows-fragment.sh", "check-windows-crate.sh"):
            (scripts / name).touch()
        self.config = {}
        # PATH is deliberately closed: no host Python or real Cargo can leak in.
        for tool in ("dirname", "tail", "sed", "rg", "head", "tr"):
            target = shutil.which(tool)
            if target:
                (self.bin / tool).symlink_to(target)
        for tool in ("cargo", "bash", "python3", "python3.15", "python3.14"):
            entry = self.bin / tool
            entry.write_text(f"#!/bin/bash\nREAL_PYTHON={shlex.quote(sys.executable)}\n" + STUB)
            entry.chmod(0o755)

    def run_verifier(self):
        config = ["case \"$key\" in"]
        for key, (text, rc) in self.config.items():
            config.append(f"  {shlex.quote(key)}) text={shlex.quote(text)}; rc={rc} ;;")
        config.append("esac\n")
        (self.root / "config.sh").write_text("\n".join(config))
        log = self.root / "calls.log"
        log.unlink(missing_ok=True)
        env = {"PATH": str(self.bin), "HOME": str(self.root), "PYTHONDONTWRITEBYTECODE": "1"}
        result = subprocess.run(
            ["/bin/bash", str(self.root / "scripts/verify-framework.sh")],
            cwd=self.root.parent, env=env, text=True, capture_output=True, timeout=180,
        )
        self.output = result.stdout + result.stderr
        self.calls = []
        if log.exists():
            for line in log.read_text().splitlines():
                key, tool, args = line.split("\t", 2)
                self.calls.append({"key": key, "tool": tool, "args": args.split()})
        return result.returncode

    def assert_passes(self):
        self.assertEqual(self.run_verifier(), 0, self.output)
        self.assertIn("ALL VERIFIED", self.output)

    def assert_fails(self):
        self.assertNotEqual(self.run_verifier(), 0, self.output)
        self.assertNotIn("ALL VERIFIED", self.output)

    def test_every_automation_meta_suite_runs_in_the_full_verifier(self):
        # The verifier lists its suites by hand; a newly added suite that nobody
        # lists simply never runs, which reads as "everything passed".
        listed = (SCRIPTS / "verify-framework.sh").read_text(encoding="utf-8")
        suites = sorted(path.stem for path in SCRIPTS.glob("test-automation-*.py"))
        self.assertTrue(suites, "no automation meta suites found")
        for name in suites:
            with self.subTest(suite=name):
                self.assertIn(name, listed, f"verifier never runs {name}")
                self.assertIn(name, META, f"verifier stub does not know {name}")

    def test_blocked_profiles_are_accepted_but_identified_as_unverified(self):
        self.assert_passes()
        self.assertIn("unverified", self.output.lower())

    def test_success_text_cannot_mask_failed_command(self):
        for key, text in (
            ("cargo", "test result: ok. 42 passed; 0 failed"),
            ("test-scan-secrets.sh", "0 失败"),
            ("scan-secrets.sh", "扫描通过"),
            ("check-windows-fragment.sh", "✓ 3 个 Windows 片段全部通过交叉编译验证"),
            ("check-windows-crate.sh", "✓ 整 crate Windows 类型检查通过（排除资源编译；不是标准构建成功）"),
            ("ci", '{"outcome": "passed"}'),
            ("test-automation-runner", "OK"),
            ("selftest", "OK"),
        ):
            with self.subTest(key=key):
                self.config = {key: [text, 1]}
                self.assert_fails()

    def test_failure_does_not_skip_remaining_suites(self):
        self.config = {"cargo": ["test result: ok. 42 passed; 0 failed", 1]}
        self.assert_fails()
        self.assertTrue(set(META).issubset({call["key"] for call in self.calls}))
        self.assertIn("client", {call["key"] for call in self.calls})

    def test_ci_requires_valid_passed_json_and_zero_exit(self):
        for text, rc in (("not json", 0), ('{"outcome":"blocked"}', 0),
                         ('{"outcome":"failed"}', 0), ('{"outcome":"blocked"}', 3),
                         ('{"outcome":"passed"}', 3),
                         ('log says {"outcome":"passed"}', 0)):
            with self.subTest(text=text, rc=rc):
                self.config = {"ci": [text, rc]}
                self.assert_fails()

    def test_compact_and_pretty_ci_json_are_accepted(self):
        for text in ('{"outcome":"passed"}', '{\n  "outcome": "passed"\n}'):
            with self.subTest(text=text):
                self.config = {"ci": [text, 0]}
                self.assert_passes()

    def test_blocked_requires_exit_three(self):
        for profile in ("native", "package", "manual", "client"):
            for rc in (0, 1, 2):
                with self.subTest(profile=profile, rc=rc):
                    self.config = {profile: ['{"outcome":"blocked"}', rc]}
                    self.assert_fails()

    def test_profiles_can_really_pass_including_valid_package(self):
        self.config = {profile: ['{"outcome":"passed"}', 0]
                       for profile in ("native", "package", "manual", "client")}
        self.assert_passes()
        for call in self.calls:
            if call["key"] in ("ci", "native", "package", "manual", "client"):
                expected = ["scripts/automation/pipeline.py", "--profile", call["key"]]
                if call["key"] == "ci":
                    expected += ["--check-timeout", "1800"]
                self.assertEqual(call["args"], expected)

    def test_profile_failures_and_inconsistent_passes_are_rejected(self):
        for profile in ("native", "package", "manual", "client"):
            for text, rc in (('{"outcome":"failed"}', 1),
                             ('{"outcome":"passed"}', 3), ("not json", 3)):
                with self.subTest(profile=profile, text=text, rc=rc):
                    self.config = {profile: [text, rc]}
                    self.assert_fails()

    def test_development_suite_is_executed_and_failure_propagates(self):
        self.config = {"test-automation-development": ["OK", 1]}
        self.assert_fails()
        self.assertIn("test-automation-development", {call["key"] for call in self.calls})

    def test_python315_is_preferred_when_available(self):
        self.assert_passes()
        python_calls = [call for call in self.calls if call["tool"].startswith("python")]
        self.assertTrue(python_calls)
        self.assertTrue(all(call["tool"] == "python3.15" for call in python_calls))

    def test_python3_without_versioned_python_is_supported_everywhere(self):
        (self.bin / "python3.15").unlink()
        (self.bin / "python3.14").unlink()
        self.assert_passes()
        python_calls = [call for call in self.calls if call["tool"].startswith("python")]
        self.assertTrue(python_calls)
        self.assertTrue(all(call["tool"] == "python3" for call in python_calls))
        self.assertTrue(set(META).issubset({call["key"] for call in python_calls}))

    def test_unsupported_python_fails_before_running_suites(self):
        (self.bin / "python3.15").unlink()
        (self.bin / "python3.14").unlink()
        (self.bin / "python3").write_text("#!/bin/bash\nprintf '3.9\\n'\n")
        self.assert_fails()
        self.assertIn("3.11", self.output)
        self.assertEqual(self.calls, [])


class WindowsFragmentTests(unittest.TestCase):
    """Real extraction/dispatch, inspecting fake Cargo; not a Rust compile test."""

    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="windows-fragment-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copy2(SCRIPTS / "check-windows-fragment.sh", scripts)
        src = self.root / "app/src-tauri/src"
        src.mkdir(parents=True)
        self.smtp = src / "smtp.rs"
        self.smtp.write_text("#[cfg(windows)]\npub fn hostname() -> String { String::new() }\n")
        (self.root / "tmp").mkdir()
        for tool in ("dirname", "mktemp", "mkdir", "cat", "grep", "mv", "rm", "sed", "cp"):
            target = shutil.which(tool)
            self.assertIsNotNone(target, tool)
            (self.bin / tool).symlink_to(target)
        for name, body in {
            "python3": f'exec {shlex.quote(sys.executable)} "$@"',
            "cargo": f'exec {shlex.quote(sys.executable)} {shlex.quote(str(self.root / "fake_cargo.py"))} "$@"',
            "rustup": '[[ "$*" == "target list --installed" ]] || exit 97\nprintf "x86_64-pc-windows-msvc\\n"',
        }.items():
            path = self.bin / name
            path.write_text("#!/bin/bash\n" + body + "\n")
            path.chmod(0o755)
        (self.root / "fake_cargo.py").write_text(r'''
import os
from pathlib import Path
import re
import sys
root = Path(__file__).resolve().parent
args = sys.argv[1:]
with (root / "calls.log").open("a") as log:
    log.write(" ".join(args) + "\n")
assert args[0] == "check" and "--offline" in args
assert args[args.index("--target") + 1] == "x86_64-pc-windows-msvc"
manifest = Path(args[args.index("--manifest-path") + 1])
path = manifest.parent / "src/lib.rs"
if not path.exists():
    path = manifest.parent / "src/main.rs"
source = path.read_text()
(root / "generated.rs").write_text(source)
# Controlled fixtures only: this is not a Rust parser or a rustc substitute.
code = re.sub(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"', "", source, flags=re.S)
has_cfg = "#[cfg(any())]" in code
if os.environ["EXPECT_CFG"] == "1" and not has_cfg:
    sys.exit("extra cfg attribute lost")
symbol = os.environ["EXPECTED_SYMBOL"]
without_definition = re.sub(r"\bfn\s+" + re.escape(symbol) + r"\s*\(", "fn definition(", code)
if not re.search(r"\b" + re.escape(symbol) + r"\s*(?:\(|as\b|[,;)}])", without_definition):
    sys.exit("missing symbol reference")
if has_cfg:
    sys.exit("preserved cfg disables referenced hostname")
print("inspected generated source")
sys.exit(int(os.environ["CARGO_RC"]))
''')

    def run_fragment(self, *names, expect_cfg=False, cargo_rc=0, symbol="hostname"):
        log = self.root / "calls.log"
        generated = self.root / "generated.rs"
        log.unlink(missing_ok=True)
        generated.unlink(missing_ok=True)
        result = subprocess.run(
            ["/bin/bash", str(self.root / "scripts/check-windows-fragment.sh"), *names],
            cwd=self.root.parent, env={
                "PATH": str(self.bin), "HOME": str(self.root), "TMPDIR": str(self.root / "tmp"),
                "PYTHONDONTWRITEBYTECODE": "1", "EXPECT_CFG": "1" if expect_cfg else "0",
                "CARGO_RC": str(cargo_rc), "EXPECTED_SYMBOL": symbol,
            }, capture_output=True, text=True, timeout=180,
        )
        self.output = result.stdout + result.stderr
        self.calls = log.read_text().splitlines() if log.exists() else []
        self.generated = generated.read_text() if generated.exists() else ""
        return result.returncode

    def test_unknown_mixed_with_known_fails_before_cargo_starts(self):
        for names in (("smtp", "unknown"), ("unknown", "smtp"), ("--list", "unknown")):
            with self.subTest(names=names):
                self.assertNotEqual(self.run_fragment(*names), 0, self.output)
                self.assertEqual(self.calls, [], self.output)

    def test_cfg_any_is_preserved_and_hostname_is_referenced(self):
        self.smtp.write_text("#[cfg(any())]\n#[cfg(windows)]\n"
                             "pub fn hostname() -> String { String::new() }\n")
        self.assertNotEqual(self.run_fragment("smtp", expect_cfg=True), 0, self.output)
        self.assertEqual(len(self.calls), 1, self.output)
        self.assertIn("#[cfg(any())]", self.generated)
        self.assertIn("preserved cfg disables referenced hostname", self.output)

    def test_changed_anchor_fails_before_compilation(self):
        self.smtp.write_text("#[cfg(windows)]\n#[cfg(any())]\n"
                             "pub fn hostname() -> String { String::new() }\n")
        self.assertNotEqual(self.run_fragment("smtp"), 0, self.output)
        self.assertEqual(self.calls, [], self.output)
        self.assertIn("提取失败", self.output)

    def test_normal_fragment_succeeds_with_checked_symbol_reference(self):
        self.assertEqual(self.run_fragment("smtp"), 0, self.output)
        self.assertEqual(len(self.calls), 1, self.output)
        self.assertIn("pub fn hostname()", self.generated)

    def test_anchor_without_parameter_list_still_references_symbol(self):
        webhook = self.smtp.with_name("webhook.rs")
        webhook.write_text("#[cfg(not(unix))]\nfn write_new_token() -> Option<()> { None }\n")
        self.assertEqual(self.run_fragment("webhook", symbol="write_new_token"), 0, self.output)
        self.assertEqual(len(self.calls), 1, self.output)
        self.assertIn("let _ = write_new_token as usize", self.generated)

    def test_placement_references_functions_outside_the_entry_call_chain(self):
        names = ("work_area_for_mode", "named_monitor", "work_area_under_window",
                 "work_area_at_logical", "physical_rect_to_logical", "fallback_work_area")
        placement = self.smtp.with_name("placement.rs")
        placement.write_text("\n" + "\n".join(f"pub fn {name}() {{}}" for name in names))
        (self.root / "app/src-tauri/Cargo.lock").write_text("# fake Cargo does not resolve dependencies\n")
        self.assertEqual(self.run_fragment("placement", symbol="work_area_at_logical"), 0,
                         self.output)
        self.assertEqual(len(self.calls), 1, self.output)
        for name in names:
            self.assertIn(f"let _ = {name} as usize", self.generated)

    def test_cargo_failure_propagates_despite_successful_checks(self):
        self.assertNotEqual(self.run_fragment("smtp", cargo_rc=23), 0, self.output)
        self.assertEqual(len(self.calls), 1, self.output)
        self.assertIn("inspected generated source", self.output)
class WindowsCrateTests(unittest.TestCase):
    """Fail-fast behaviour of the whole-crate Windows type check, without cargo."""

    def test_unresolvable_tauri_build_version_fails_before_cargo(self):
        with tempfile.TemporaryDirectory(prefix="windows-crate-") as directory:
            root = Path(directory)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for tool in ("dirname", "mktemp", "mkdir", "rm", "git"):
                target = shutil.which(tool)
                self.assertIsNotNone(target, tool)
                (bin_dir / tool).symlink_to(target)
            (bin_dir / "python3").write_text(
                f'#!/bin/bash\nexec {shlex.quote(sys.executable)} "$@"\n')
            (bin_dir / "python3").chmod(0o755)
            scripts = root / "scripts"
            scripts.mkdir()
            shutil.copy2(SCRIPTS / "check-windows-crate.sh", scripts)
            # Enough files to clear the snapshot collector's own sanity floor.
            for index in range(120):
                (root / f"src/f{index}.rs").parent.mkdir(parents=True, exist_ok=True)
                (root / f"src/f{index}.rs").write_text("")
            (root / "app/src-tauri").mkdir(parents=True, exist_ok=True)
            (root / "app/src-tauri/Cargo.lock").write_text(
                '\nname = "tauri-build"\nversion = "9.9.9"\n')
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "add", "-A"], cwd=root, check=True,
                           stdout=subprocess.DEVNULL)
            result = subprocess.run(
                ["/bin/bash", str(scripts / "check-windows-crate.sh")],
                cwd=root, env={"PATH": str(bin_dir), "HOME": str(root),
                               "TMPDIR": str(root)},
                capture_output=True, text=True, timeout=300)
            output = result.stdout + result.stderr
            self.assertNotEqual(result.returncode, 0, output)
            self.assertIn("tauri-build-9.9.9", output)


if __name__ == "__main__":
    unittest.main()
