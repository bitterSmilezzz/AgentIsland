#!/usr/bin/env python3
"""Break protocol tests: synthetic registries and real disposable subprocesses only."""
from __future__ import annotations

import hashlib
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
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("break_meta", HERE / "automation/break_check.py")
break_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(break_check)
GOOD = {"schema_version": 1, "experiments": [
    {"id": "content_comparison", "baseline_rejected": True, "mutant_false_green": True}]}

# A flaky failure inside `pipeline.py --profile ci` is invisible: the pipeline keeps no
# child transcript (an existing rule, and a good one), so a red row says only
# `returncode=1`. This suite already failed twice that way for reasons that could not be
# reproduced in 13 later runs. On failure the failing test *names* are written to
# `.scratch/` -- git-ignored, no child output, no source content -- so a recurrence can
# be diagnosed instead of guessed at. See `_RecordingResult` at the bottom of this file.
FLAKE_LOG = HERE.parent / ".scratch/automation-break-failures.jsonl"


class BreakTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="break-meta-",
            dir=str(Path(tempfile.gettempdir()).resolve()))
        self.addCleanup(temporary.cleanup)
        self.cwd = Path(temporary.name)
        self.source = self.cwd / "input.txt"
        self.source.write_text("synthetic source")
        self.fingerprint = lambda cwd: hashlib.sha256((cwd / "input.txt").read_bytes()).hexdigest()
        self.digest = self.fingerprint(self.cwd)

    def run_helper(self, body, **kwargs):
        timeout = kwargs.pop("timeout", 2)
        (self.cwd / "helper.py").write_text(body)
        registry = {"schema_version": 1, "checks": {"synthetic.check": {
            "command": [sys.executable, "helper.py", "--normal"],
            "requires_live_service": False, "requires_spending": False,
            "discriminating_by": {"command": [sys.executable, "helper.py", "--break"],
                                  "reason": "synthetic counterexample"}}}}
        return break_check.run_break_check("synthetic.check", registry, self.cwd,
            source_fingerprint=self.digest, fingerprint=self.fingerprint, timeout=timeout, **kwargs)

    def test_complete_success_is_bound_without_output_or_command_text(self):
        result = self.run_helper("import json\nprint(" + repr(json.dumps(GOOD)) + ")\n")
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["source_fingerprint"], self.digest)
        self.assertEqual(result["check_id"], "synthetic.check")
        self.assertEqual(result["process"]["returncode"], 0)
        self.assertTrue(result["process"]["group_reclaimed"])
        self.assertEqual(result["experiments"], GOOD["experiments"])
        self.assertRegex(result["counterexample_command_sha256"], "^[0-9a-f]{64}$")
        self.assertNotIn("stdout", result)
        self.assertNotIn("stderr", result)
        self.assertNotIn(str(self.cwd), json.dumps(result))

    def test_protocol_rejects_empty_nonjson_false_and_forged_experiments(self):
        samples = [({}, "invalid_schema"), (dict(GOOD, schema_version=True), "invalid_schema"),
                   ({"schema_version": 1, "experiments": []}, "invalid_experiments"),
                   ({"schema_version": 1, "experiments": GOOD["experiments"] * 65}, "invalid_experiments"),
                   ({"schema_version": 1, "experiments": GOOD["experiments"] * 2}, "duplicate_experiment"),
                   (dict(GOOD, extra=True), "invalid_schema")]
        for field in ("baseline_rejected", "mutant_false_green"):
            for bad in (False, 1, "true", None):
                samples.append(({"schema_version": 1, "experiments": [
                    dict(GOOD["experiments"][0], **{field: bad})]}, "experiment_not_discriminating"))
        samples.extend([({"schema_version": 1, "experiments": [{}]}, "invalid_experiment"),
                        ({"schema_version": 1, "experiments": [dict(GOOD["experiments"][0], id="../bad")]},
                         "invalid_experiment")])
        for value, code in samples:
            with self.subTest(value=value):
                result = self.run_helper("print(" + repr(json.dumps(value)) + ")")
                self.assertEqual(result["code"], code)
                self.assertFalse(result["ok"])
        for raw, code in [("", "invalid_json"), ("success PRIVATE_SENTINEL", "invalid_json"),
                          ('{"schema_version":1,"schema_version":1,"experiments":[]}', "duplicate_key"),
                          (json.dumps(GOOD) + " trailing", "invalid_json"),
                          ('{"schema_version":NaN,"experiments":[]}', "invalid_json"),
                          ('{"schema_version":1,"experiments":[{"id":"a","id":"b"}]}', "duplicate_key")]:
            with self.subTest(raw=raw):
                result = self.run_helper("print(" + repr(raw) + ")")
                self.assertEqual(result["code"], code)
                self.assertNotIn("PRIVATE_SENTINEL", json.dumps(result))

    def test_nonzero_even_with_complete_success_json_is_failure(self):
        for raw in (json.dumps(GOOD), "success PRIVATE_SENTINEL"):
            result = self.run_helper("import sys\nprint(" + repr(raw) + ")\nsys.exit(9)")
            self.assertEqual(result["code"], "nonzero_exit")
            self.assertEqual(result["process"]["returncode"], 9)
            self.assertFalse(result["ok"])

    def test_timeout_reclaims_group_and_suppresses_private_output(self):
        result = self.run_helper("import time\nprint('PRIVATE_SENTINEL', flush=True)\ntime.sleep(60)", timeout=0.2)
        self.assertEqual(result["code"], "timeout")
        self.assertTrue(result["process"]["timed_out"])
        self.assertTrue(result["process"]["group_reclaimed"])
        self.assertLess(result["duration_seconds"], 2)
        self.assertNotIn("PRIVATE_SENTINEL", json.dumps(result))

    def assert_child_stopped(self):
        pid = int((self.cwd / "child.pid").read_text())
        state = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True,
                               text=True, timeout=2).stdout.strip()
        self.assertTrue(not state or state.startswith("Z"), state)

    def test_success_parent_with_live_child_is_failure_and_child_is_killed(self):
        child = "import time; time.sleep(60)"
        body = ("import subprocess,sys\nfrom pathlib import Path\n"
                "p=subprocess.Popen([sys.executable,'-c'," + repr(child) + "],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
                "Path('child.pid').write_text(str(p.pid))\nprint(" + repr(json.dumps(GOOD)) + ")")
        result = self.run_helper(body)
        self.assertEqual(result["code"], "descendants_left_running")
        self.assertTrue(result["process"]["group_reclaimed"])
        self.assertTrue(result["process"]["group_absent_verified"])
        self.assert_child_stopped()

    def test_timeout_parent_and_pipe_inheriting_child_are_killed(self):
        body = ("import subprocess,sys,time\nfrom pathlib import Path\n"
                "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\n"
                "Path('child.pid').write_text(str(p.pid))\ntime.sleep(60)")
        result = self.run_helper(body, timeout=0.3)
        self.assertEqual(result["code"], "timeout")
        self.assert_child_stopped()

    def test_output_limit_counts_both_streams_and_stops_infinite_writer(self):
        for fd in (1, 2):
            result = self.run_helper("import os\nwhile True: os.write(" + str(fd) + ",b'x'*8192)",
                                     max_output_bytes=10000)
            self.assertEqual(result["code"], "output_limit")
            self.assertTrue(result["process"]["output_limited"])
            self.assertLessEqual(result["process"]["output_bytes_observed"], 18192)
            self.assertTrue(result["process"]["group_reclaimed"])

    def test_source_changed_rejects_success_and_initial_mismatch_prevents_execution(self):
        result = self.run_helper("from pathlib import Path\nPath('input.txt').write_text('changed')\nprint(" + repr(json.dumps(GOOD)) + ")")
        self.assertEqual(result["code"], "source_changed")
        self.assertEqual(result["experiments"], [])
        result = self.run_helper("raise RuntimeError('must not start')")
        self.assertEqual(result["code"], "source_fingerprint_mismatch")
        self.assertFalse(result["process"]["started"])

    def test_artifact_adapter_accepts_only_named_rejection_codes(self):
        legacy = {"experiments": [{"ablation": "source_content_comparison", "baseline_code": "source_drift",
                  "mutant_code": "ok", "mutant_false_green": True}]}
        self.assertEqual(break_check.validate_experiments(json.dumps(legacy).encode(), artifact_legacy=True),
                         [{"id": "source_content_comparison", "baseline_rejected": True, "mutant_false_green": True}])
        for field, value in (("baseline_code", "io_error"), ("baseline_code", "artifact_changed"),
                             ("baseline_code", "ok"), ("mutant_code", "source_drift"),
                             ("mutant_false_green", 1), ("ablation", "forged")):
            bad = {"experiments": [dict(legacy["experiments"][0], **{field: value})]}
            with self.assertRaises(break_check.BreakError):
                break_check.validate_experiments(json.dumps(bad).encode(), artifact_legacy=True)
        with self.assertRaises(break_check.BreakError):
            break_check.validate_experiments(json.dumps(legacy).encode())

    def test_snapshot_declaration_now_executes_while_external_authorization_is_blocked(self):
        # snapshot gained a structured outlet, so its declaration is executed like any
        # other; a missing counterexample script must surface as a real failure rather
        # than being reported as "declared but never run".
        registry = {"schema_version": 1, "checks": {"automation.snapshot": {
            "discriminating_by": {"command": [sys.executable, "must-not-run.py"], "reason": "snapshot"}}}}
        result = break_check.run_break_check("automation.snapshot", registry, self.cwd,
            source_fingerprint=self.digest, fingerprint=self.fingerprint)
        self.assertNotIn(result["code"], ("declared_not_executed",))
        self.assertFalse(result["ok"])
        self.assertTrue(result["process"]["started"],
                        "a declared counterexample is now really executed")
        for field, value in (("requires_spending", True), ("requires_live_service", True),
                             ("requires_env", ["PRIVATE_ENV"]), ("command_env", {"A": "B"})):
            registry["checks"]["synthetic.check"] = dict(registry["checks"]["automation.snapshot"], **{field: value})
            result = break_check.run_break_check("synthetic.check", registry, self.cwd,
                source_fingerprint=self.digest, fingerprint=self.fingerprint)
            self.assertEqual(result["code"], "external_authorization_required")
            self.assertFalse(result["process"]["started"])

    def test_cli_uses_registry_argv_only_and_emits_json(self):
        subprocess.run(["git", "init", "-q", str(self.cwd)], check=True, capture_output=True, timeout=5)
        (self.cwd / "helper.py").write_text("import sys\nassert sys.argv[1:]==['--break']\nprint(" + repr(json.dumps(GOOD)) + ")")
        registry = {"schema_version": 1, "checks": {"synthetic.check": {
            "command": [sys.executable, "missing-normal.py"],
            "discriminating_by": {"command": [sys.executable, "helper.py", "--break"], "reason": "synthetic"}},
            "unselected.check": {"discriminating_by": {"command": [sys.executable, "must-not-run.py"], "reason": "synthetic"}}}}
        path = self.cwd / "registry.json"
        path.write_text(json.dumps(registry))
        sys.path.insert(0, str(HERE / "automation"))
        import pipeline
        digest = pipeline.worktree_fingerprint_safe(self.cwd)
        command = [sys.executable, str(HERE / "automation/break_check.py"), "--registry", str(path),
                   "--check-id", "synthetic.check", "--cwd", str(self.cwd), "--source-fingerprint", digest]
        proc = subprocess.run(command, capture_output=True, text=True, timeout=5)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        result = json.loads(proc.stdout)
        self.assertTrue(result["ok"])
        self.assertEqual(result["counterexample_command_sha256"], break_check.command_sha256(
            registry["checks"]["synthetic.check"]["discriminating_by"]["command"]))
        self.assertEqual(proc.stderr, "")
        proc = subprocess.run(command + ["--command", "anything"], capture_output=True, text=True, timeout=5)
        self.assertEqual(proc.returncode, 2)
        self.assertEqual(json.loads(proc.stdout)["code"], "invalid_argument")
        self.assertEqual(proc.stderr, "")
        path.write_text('{"checks":{},"checks":{}}')
        proc = subprocess.run(command, capture_output=True, text=True, timeout=5)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(json.loads(proc.stdout)["code"], "duplicate_key")

    def test_effective_argv_hash_is_bound_and_environment_is_minimal(self):
        with patch.dict(os.environ, {"BREAK_SYNTHETIC_PRIVATE_ENV": "PRIVATE_SENTINEL",
                                     "PYTHONPATH": "PRIVATE_SENTINEL"}):
            result = self.run_helper("import os\nassert 'BREAK_SYNTHETIC_PRIVATE_ENV' not in os.environ\nassert 'PYTHONPATH' not in os.environ\nprint(" + repr(json.dumps(GOOD)) + ")")
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["executed_command_sha256"], result["counterexample_command_sha256"])

    def test_invalid_budget_and_fingerprint_error_never_start_helper(self):
        for timeout in (0, -1, float('nan'), float('inf'), True):
            result = self.run_helper("raise RuntimeError('must not run')", timeout=timeout)
            self.assertEqual(result["code"], "invalid_argument")
            self.assertFalse(result["process"]["started"])
        for limit in (0, 65537, True):
            result = self.run_helper("raise RuntimeError('must not run')", max_output_bytes=limit)
            self.assertEqual(result["code"], "invalid_argument")
        self.fingerprint = lambda cwd: (_ for _ in ()).throw(RuntimeError('PRIVATE_SENTINEL'))
        result = self.run_helper("raise RuntimeError('must not run')")
        self.assertEqual(result["code"], "fingerprint_failed")
        self.assertNotIn("PRIVATE_SENTINEL", json.dumps(result))

    def test_real_artifact_ablation_in_disposable_copy(self):
        registry = json.loads((HERE / "automation/registry.json").read_text())
        names = ("test-automation-artifact.py", "common.sh", "build-app.sh", "check-version.py",
                 "heavy-build-lock.sh", "automation/heavy_build_lock.py", "automation/artifact.py")
        def digest(cwd):
            return hashlib.sha256(b"".join((cwd / "scripts" / name).read_bytes() for name in names)).hexdigest()
        for name in names:
            target = self.cwd / "scripts" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(HERE / name, target)
        result = break_check.run_break_check("automation.artifact", registry, self.cwd,
            source_fingerprint=digest(self.cwd), fingerprint=digest, timeout=20)
        self.assertTrue(result["ok"], result)
        self.assertEqual([row["id"] for row in result["experiments"]],
                         ["source_content_comparison", "artifact_content_comparison"])

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.cwd, check=True,
                              capture_output=True, text=True, timeout=5)

    def isolated_fixture(self, extra=None):
        """Copy the real script dependencies into self.cwd and declare a synthetic registry."""
        for name in ("common.sh", "build-app.sh", "check-version.py", "test-automation-artifact.py",
                     "heavy-build-lock.sh"):
            (self.cwd / "scripts").mkdir(parents=True, exist_ok=True)
            shutil.copy2(HERE / name, self.cwd / "scripts" / name)
        (self.cwd / "scripts/automation").mkdir(parents=True, exist_ok=True)
        shutil.copy2(HERE / "automation/artifact.py", self.cwd / "scripts/automation/artifact.py")
        shutil.copy2(HERE / "automation/heavy_build_lock.py", self.cwd / "scripts/automation/heavy_build_lock.py")
        registry = {"schema_version": 1, "checks": {"automation.artifact": {
            "command": ["python3", "scripts/test-automation-artifact.py"],
            "host_os": ["darwin", "linux"], "requires_live_service": False, "requires_spending": False,
            "requires_python": "3.11",
            "discriminating_by": {
                "command": ["python3", "scripts/test-automation-artifact.py", "--ablation-experiments"],
                "reason": "Real ablations run inside the isolated fixture copy."}}}}
        registry["checks"].update(extra or {})
        (self.cwd / "scripts/automation/registry.json").write_text(json.dumps(registry))
        self.git("init", "-q")
        self.git("add", "-A", "scripts")
        (self.cwd / "DECLARED-UNTRACKED.txt").write_text("selected but never committed")
        return registry

    def source_state(self):
        sys.path.insert(0, str(HERE / "automation"))
        import snapshot_safety
        entries = snapshot_safety.prepare_snapshot(self.cwd, break_check._selected_names(self.cwd))
        return break_check._entries_fingerprint(entries), (self.cwd / ".git/index").read_bytes()

    def temp_entries(self):
        return {entry.name for entry in Path(tempfile.gettempdir()).resolve().glob("automation-break-*")}

    def run_isolated(self, *args, timeout=60):
        command = [sys.executable, str(HERE / "automation/break_check.py"),
                   "--check-id", args[0], "--isolated", "--timeout", str(timeout), *args[1:]]
        return subprocess.run(command, cwd=self.cwd, capture_output=True, text=True, timeout=180)

    @unittest.skipIf(sys.version_info < (3, 11), "artifact ablation needs tomllib")
    def test_isolated_cli_runs_real_ablation_and_binds_both_trees(self):
        self.isolated_fixture()
        before, index_before = self.source_state()
        leftovers = self.temp_entries()
        proc = self.run_isolated("automation.artifact")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(proc.stderr, "")
        result = json.loads(proc.stdout)
        self.assertTrue(result["ok"], result)
        self.assertEqual([row["id"] for row in result["experiments"]],
                         ["source_content_comparison", "artifact_content_comparison"])
        self.assertEqual(result["code"], "ok")
        isolation = result["isolation"]
        self.assertTrue(result["isolated"])
        self.assertEqual(isolation["source_root"], str(self.cwd.resolve()))
        self.assertTrue(Path(isolation["execution_root"]).is_relative_to(
            Path(tempfile.gettempdir()).resolve()))
        self.assertEqual(isolation["source_fingerprint_before"], before)
        self.assertEqual(isolation["snapshot_fingerprint"], result["source_fingerprint"])
        self.assertEqual(isolation["source_fingerprint_after_snapshot"], before)
        self.assertEqual(isolation["source_fingerprint_after"], before)
        self.assertTrue(isolation["temporary_directory_removed"])
        self.assertFalse(Path(isolation["execution_root"]).exists())
        self.assertEqual(self.temp_entries(), leftovers)
        self.assertEqual(self.source_state(), (before, index_before))
        self.assertFalse((self.cwd / "dist").exists())

    def test_isolated_refuses_private_file_and_symlink_escape_without_writing(self):
        self.isolated_fixture()
        index_before = (self.cwd / ".git/index").read_bytes()
        leftovers = self.temp_entries()
        (self.cwd / "leak.txt").write_text("credential " + "sk-" + "A" * 24)
        proc = self.run_isolated("automation.artifact")
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        result = json.loads(proc.stdout)
        self.assertEqual(result["code"], "snapshot_content_rejected")
        self.assertIsNone(result["isolation"]["execution_root"])
        self.assertNotIn("sk-", proc.stdout)
        self.assertEqual((self.cwd / ".git/index").read_bytes(), index_before)
        self.assertEqual(self.temp_entries(), leftovers)
        (self.cwd / "leak.txt").unlink()
        (self.cwd / "escape").symlink_to("/etc/hosts")
        proc = self.run_isolated("automation.artifact")
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertEqual(json.loads(proc.stdout)["code"], "snapshot_source_symlink_escape")
        self.assertEqual((self.cwd / ".git/index").read_bytes(), index_before)
        self.assertEqual(self.temp_entries(), leftovers)

    def test_isolated_writes_stay_in_temp_and_cleanup_failure_is_not_pass(self):
        self.isolated_fixture({"automation.marker": {"discriminating_by": {
            "command": [sys.executable, "marker.py"], "reason": "synthetic marker probe"}}})
        helper = ("from pathlib import Path\nPath('marker.txt').write_text('m')\n"
                  "@@print(" + repr(json.dumps(GOOD)) + ")\n")
        def write_helper(cleanup):
            return (self.cwd / "marker.py").write_text(helper.replace("@@", cleanup))
        write_helper("Path('marker.txt').unlink()\n")
        before = self.source_state()[0]
        result = break_check.run_isolated_check("automation.marker", self.cwd, timeout=20)
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["isolation"]["source_fingerprint_after"], before)
        self.assertFalse((self.cwd / "marker.txt").exists())
        write_helper("")
        before = self.source_state()[0]
        result = break_check.run_isolated_check("automation.marker", self.cwd, timeout=20)
        self.assertEqual(result["code"], "source_changed")
        self.assertEqual(result["isolation"]["source_fingerprint_after"], before)
        self.assertFalse((self.cwd / "marker.txt").exists())
        write_helper("Path('marker.txt').unlink()\n")
        before = self.source_state()[0]
        leaked = []

        class LeakyTemporaryDirectory:
            def __init__(self, prefix, dir):
                self.name = tempfile.mkdtemp(prefix=prefix, dir=dir)

            def cleanup(self):
                leaked.append(self.name)  # Simulated removal failure.

        try:
            with patch.object(tempfile, "TemporaryDirectory", LeakyTemporaryDirectory):
                result = break_check.run_isolated_check("automation.marker", self.cwd, timeout=20)
        finally:
            for path in leaked:
                shutil.rmtree(path, ignore_errors=True)
        self.assertFalse(result["ok"], result)
        self.assertEqual(result["code"], "temporary_cleanup_failed")
        self.assertFalse(result["isolation"]["temporary_directory_removed"])
        self.assertEqual(result["isolation"]["source_fingerprint_after"], before)
        self.assertEqual([str(Path(path).resolve()) for path in leaked],
                         [result["isolation"]["execution_root"]])

    def test_isolated_copy_failure_is_explicit_not_a_pass(self):
        self.isolated_fixture()
        leaked = []
        with patch.object(break_check, "_copy_snapshot", lambda entries, root: leaked.append(root)):
            result = break_check.run_isolated_check("automation.artifact", self.cwd, timeout=20)
        self.assertFalse(result["ok"], result)
        self.assertEqual(result["code"], "snapshot_copy_mismatch")
        self.assertEqual(result["experiments"], [])
        self.assertFalse(result["process"]["started"])
        self.assertFalse(leaked[0].exists())

    def test_isolated_source_marker_is_detected_and_leaves_other_files_intact(self):
        # 用**前后差集**而不是「全局必须为空」——本仓其它隔离测试（见上方几处
        # `leftovers`）都是这个写法，只有这里原来写成绝对空集。
        # 绝对空集会让这条断言看到**别的套件**的临时目录：ci profile 里
        # `automation.break-artifact` / `automation.break-snapshot` 都调
        # `break_check.py --isolated`，用的是同一个 `automation-break-` 前缀。
        # 只要它们的清理与本测试的断言有一瞬重叠，就会红成「本测试泄漏」——
        # 这是 `.scratch/automation-break-failures.jsonl` 里那条既有 flake 的机制。
        # 差集断言测的才是它真正想测的：**本测试**有没有泄漏。
        leftovers = self.temp_entries()
        self.isolated_fixture({"automation.marker": {"discriminating_by": {
            "command": [sys.executable, "marker.py", str(self.cwd / "SOURCE-MARKER.txt")],
            "reason": "escape probe"}}})
        others = {path: path.read_bytes() for path in (self.cwd / "scripts").rglob("*") if path.is_file()}
        index_before = (self.cwd / ".git/index").read_bytes()
        (self.cwd / "marker.py").write_text(
            "import sys\nfrom pathlib import Path\nPath(sys.argv[1]).write_text('escaped')\n"
            "print(" + repr(json.dumps(GOOD)) + ")\n")
        result = break_check.run_isolated_check("automation.marker", self.cwd, timeout=20)
        self.assertEqual(result["code"], "source_changed")
        self.assertEqual(result["experiments"], [])
        self.assertEqual((self.cwd / "SOURCE-MARKER.txt").read_text(), "escaped")
        self.assertEqual({path: path.read_bytes() for path in others}, others)
        self.assertEqual((self.cwd / ".git/index").read_bytes(), index_before)
        self.assertNotEqual(result["isolation"]["source_fingerprint_after"],
                            result["isolation"]["source_fingerprint_before"])
        (self.cwd / "SOURCE-MARKER.txt").unlink()
        self.assertEqual(self.temp_entries(), leftovers,
                         "本测试不得泄漏临时目录（差集：新增的才算泄漏）")

    def test_isolated_copy_includes_tracked_ignored_files(self):
        (self.cwd / ".gitignore").write_text("HIDDEN-TRACKED.txt\n")
        (self.cwd / "HIDDEN-TRACKED.txt").write_text("tracked despite ignore rules")
        self.isolated_fixture({"automation.hidden": {"discriminating_by": {
            "command": [sys.executable, "hidden.py"], "reason": "tracked ignored probe"}}})
        self.git("add", "-f", "HIDDEN-TRACKED.txt")
        (self.cwd / "hidden.py").write_text(
            "from pathlib import Path\n"
            "assert Path('HIDDEN-TRACKED.txt').read_text() == 'tracked despite ignore rules'\n"
            "print(" + repr(json.dumps(GOOD)) + ")\n")
        result = break_check.run_isolated_check("automation.hidden", self.cwd, timeout=20)
        self.assertTrue(result["ok"], result)

    def test_isolated_exit_codes_follow_failure_class(self):
        self.isolated_fixture({"automation.blocked": {"requires_live_service": True,
            "discriminating_by": {"command": [sys.executable, "never.py"], "reason": "synthetic"}}})
        self.assertEqual(self.run_isolated("automation.artifact", "--source-fingerprint",
                                           "0" * 64).returncode, 2)
        self.assertEqual(self.run_isolated("automation.artifact", "--timeout", "0").returncode, 2)
        self.assertEqual(self.run_isolated("automation.missing").returncode, 1)
        self.assertEqual(self.run_isolated("automation.blocked").returncode, 3)
        (self.cwd / "scripts/automation/registry.json").write_text("{}")
        self.assertEqual(self.run_isolated("automation.artifact").returncode, 1)
        self.assertEqual(self.run_isolated("automation.artifact", "--registry",
                                           "/etc/hosts").returncode, 1)
        nonrepo = Path(tempfile.mkdtemp(prefix="break-nonrepo-", dir=str(Path(tempfile.gettempdir()).resolve())))
        self.addCleanup(shutil.rmtree, nonrepo, True)
        proc = subprocess.run([sys.executable, str(HERE / "automation/break_check.py"),
                               "--check-id", "automation.artifact", "--isolated", "--timeout", "30"],
                              cwd=nonrepo, capture_output=True, text=True, timeout=60)
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertEqual(json.loads(proc.stdout)["code"], "snapshot_git_failed")
        registry = json.loads((HERE / "automation/registry.json").read_text())
        registry["checks"]["automation.artifact"]["requires_python"] = "99.0"
        (self.cwd / "scripts/automation/registry.json").write_text(json.dumps(registry))
        proc = self.run_isolated("automation.artifact")
        self.assertEqual(proc.returncode, 3, proc.stdout + proc.stderr)
        result = json.loads(proc.stdout)
        self.assertEqual(result["status"], "blocked")
        self.assertEqual(result["code"], "python_unavailable")
        self.assertFalse(result["process"]["started"])


class _RecordingResult(unittest.TextTestResult):
    """Writes the failing test names once, after the whole suite has run."""

    def stopTestRun(self) -> None:
        super().stopTestRun()
        names = [test.id() for test, _ in self.failures + self.errors]
        if not names:
            return
        try:
            FLAKE_LOG.parent.mkdir(parents=True, exist_ok=True)
            with FLAKE_LOG.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps({"failed": names}) + "\n")
        except OSError:
            # A gate that cannot write its own diagnostic must still fail loudly.
            pass


if __name__ == "__main__":
    program = unittest.main(verbosity=2, exit=False,
                            testRunner=unittest.TextTestRunner(resultclass=_RecordingResult))
    raise SystemExit(0 if program.result.wasSuccessful() else 1)
