#!/usr/bin/env python3
"""Runner meta-tests: only synthetic Python subprocesses and temporary git repositories.

Run: python3 scripts/test-automation-runner.py
No product builds, native checks, services, credentials, or real agents are used.
"""
from __future__ import annotations

import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

RUNNER = Path(__file__).resolve().parent / "automation/pipeline.py"
sys.dont_write_bytecode = True
sys.path.insert(0, str(RUNNER.parent))
spec = importlib.util.spec_from_file_location("automation_pipeline_meta", RUNNER)
pipeline = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = pipeline
spec.loader.exec_module(pipeline)
PY = sys.executable
HOST = "windows" if sys.platform == "win32" else sys.platform
BOOTSTRAP = """
import sys
from pathlib import Path
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import pipeline
root = Path(sys.argv[2])
pipeline.REPO = root
pipeline.REGISTRY = root / 'registry.json'
pipeline.EVIDENCE_ROOT = root / '.scratch/runs'
pipeline.WORKSPACE_ROOT = root / '.scratch/workspace'
pipeline.LESSON_LEDGER = root / '.scratch/lessons.jsonl'
sys.argv = ['pipeline', *sys.argv[3:]]
raise SystemExit(pipeline.main())
"""


def check_entry(code="pass", **extra):
    return {"command": [PY, "-c", code], "host_os": [HOST], **extra}


def registry(checks=None):
    checks = checks or {"ok": check_entry()}
    return {"schema_version": 1, "gates": {"L0": {"required": True}},
            "requirements": {"AUTO-01": {"checks": list(checks)}},
            "checks": checks, "profiles": {"test": list(checks), "note": "explanation"},
            "agents": {"default": "mock", "available": ["mock", "agy"]}}


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="automation-meta-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.write(".gitignore", ".scratch/\ntarget/\n__pycache__/\n")
        self.write("source.txt", "baseline\n")
        self.data = registry()
        self.save()
        self.git("init", "-q")
        self.git("add", "--", ".gitignore", "source.txt", "registry.json")
        self.git("-c", "user.name=Runner Test", "-c", "user.email=runner@example.invalid", "commit", "-qm", "fixture")

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, text=True, capture_output=True, check=True)

    def save(self):
        self.write("registry.json", json.dumps(self.data))

    def command(self, *args):
        return [PY, "-c", BOOTSTRAP, str(RUNNER.parent), str(self.root), *args]

    # 预算按**行为**给，不按快慢给：这条路径会起真实的嵌套流水线（git worktree、
    # 子进程、mock 检查），15 秒在负载高的机器上会被真实跑穿，表现为「测试红但产品没变」。
    LAUNCH_TIMEOUT = 300

    def launch(self, *args, env=None):
        self.save()
        return subprocess.run(self.command(*args), capture_output=True, text=True,
                              env={**os.environ, **(env or {})}, timeout=self.LAUNCH_TIMEOUT)

    def run_profile(self, checks=None, env=None, timeout=3):
        if checks:
            self.data = registry(checks)
        proc = self.launch("--profile", "test", "--check-timeout", str(timeout), env=env)
        summary = json.loads(proc.stdout)
        run_dir = Path(summary["evidence"])
        record = json.loads((run_dir / "run.json").read_text())
        return proc, record, run_dir

    def test_nonzero_profiles_always_persist_pause_without_notification(self):
        for rc in (1, 3):
            with self.subTest(rc=rc):
                proc, record, directory = self.run_profile({"failure": check_entry(f"raise SystemExit({rc})")})
                self.assertEqual(proc.returncode, rc)
                receipt = json.loads((directory / "pause.json").read_text())
                self.assertEqual(receipt["run_id"], record["run_id"])
                self.assertEqual(receipt["returncode"], rc)
                self.assertEqual(receipt["stage"], "profile")
                self.assertEqual(receipt["notification"]["detail"], "not_requested")

    def test_invalid_registry_persists_pause_but_readonly_does_not_create_evidence(self):
        self.data = {"invalid": True}
        for flag in ("--validate-registry", "--list-profiles"):
            proc = self.launch(flag)
            self.assertEqual(proc.returncode, 2)
            self.assertFalse((self.root / ".scratch").exists())
        proc = self.launch("--profile", "test")
        self.assertEqual(proc.returncode, 2)
        directory = Path(json.loads(proc.stdout)["evidence"])
        receipt = json.loads((directory / "pause.json").read_text())
        self.assertEqual(receipt["stage"], "configuration")
        self.assertEqual(receipt["returncode"], 2)
        self.assertEqual(receipt["notification"]["detail"], "not_requested")

    def test_pause_write_failure_is_durable_and_revokes_existing_candidate(self):
        directory = self.root / ".scratch/pause-failure"
        directory.mkdir(parents=True)
        candidate = {"kind": "local-only-candidate", "run_id": "pause-failure"}
        record = {"run_id": "pause-failure", "steps": [],
                  "checks": [{"check_id": "bad", "status": "failed"}], "candidate": candidate}
        (directory / "candidate.json").write_text(json.dumps(candidate))
        with patch.object(pipeline.pause, "pause_record", side_effect=OSError("private fixture body")), \
                patch("sys.stdout", io.StringIO()):
            rc = pipeline._finish_run(directory, record)
        self.assertNotEqual(rc, 0)
        persisted = json.loads((directory / "run.json").read_text())
        self.assertIn("pause.record", persisted["failed_or_blocked"])
        self.assertTrue(any(s.get("detail") == "pause_record_failed" for s in persisted["steps"]))
        self.assertFalse((directory / "candidate.json").exists())
        self.assertNotIn("private fixture body", json.dumps(persisted))
        self.assertTrue((directory / "pause-failure.json").is_file())

    def test_finish_run_handles_report_runtime_error_and_persists_pause(self):
        directory = self.root / ".scratch/runtime-report"
        directory.mkdir(parents=True)
        record = {"run_id": "runtime-report", "checks": [], "steps": [],
                  "candidate": {"kind": "local-only-candidate"}}
        with patch.object(pipeline.evidence, "provenance", side_effect=RuntimeError("private body")), \
                patch("sys.stdout", io.StringIO()):
            rc = pipeline._finish_run(directory, record)
        self.assertEqual(rc, 1)
        self.assertTrue((directory / "pause.json").is_file())
        self.assertNotIn("private body", (directory / "run.json").read_text())
        self.assertFalse((directory / "candidate.json").exists())

    def test_notification_is_explicit_for_profile_and_configuration(self):
        for mode in ("profile", "configuration"):
            for notify in (False, True):
                with self.subTest(mode=mode, notify=notify):
                    args = pipeline.build_parser().parse_args(
                        ["--profile", "test"] + (["--notify-failure"] if notify else []))
                    self.save()
                    with patch.object(pipeline, "REPO", self.root), \
                            patch.object(pipeline, "REGISTRY", self.root / "registry.json"), \
                            patch.object(pipeline, "EVIDENCE_ROOT", self.root / ".scratch/runs"), \
                            patch.object(pipeline, "load_registry", side_effect=(
                                pipeline.PipelineError("configuration_error") if mode == "configuration" else None),
                                return_value=self.data), \
                            patch.object(pipeline, "run_check", return_value=pipeline.CheckResult("ok", "failed", 1)), \
                            patch.object(pipeline.pause, "deliver", return_value={
                                "channel": "fixture", "delivered": False, "detail": "channel_failed"}) as delivery, \
                            patch("sys.stdout", io.StringIO()):
                        rc = pipeline.run_pipeline(args)
                    self.assertEqual(rc, 2 if mode == "configuration" else 1)
                    self.assertEqual(delivery.call_count, int(notify))

    def test_unwritable_run_dir_persists_fixed_diagnostic_in_evidence_root(self):
        directory = self.root / ".scratch/runs/blocked"
        directory.mkdir(parents=True)
        record = {"run_id": "blocked", "checks": [], "steps": []}
        real_atomic = pipeline.pause.atomic_json
        def write(path, payload):
            if path.parent == directory:
                raise OSError("private injected body")
            return real_atomic(path, payload)
        with patch.object(pipeline, "EVIDENCE_ROOT", directory.parent), \
                patch.object(pipeline, "_write_run_reports", side_effect=OSError("private injected body")), \
                patch.object(pipeline.pause, "atomic_json", side_effect=write), \
                patch("sys.stdout", io.StringIO()):
            rc = pipeline._finish_run(directory, record)
        self.assertEqual(rc, 1)
        diagnostic = json.loads((directory.parent / "blocked.failure.json").read_text())
        self.assertEqual(diagnostic["detail"], "pause_record_failed")
        self.assertFalse(diagnostic["candidate_valid"])
        self.assertNotIn("private injected body", json.dumps(diagnostic))

    def test_passed_reports_and_content_fingerprint(self):
        proc, record, run_dir = self.run_profile()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(record["outcome"], "passed")
        self.assertEqual(record["fingerprint_before"], record["fingerprint_after"])
        self.assertEqual(len(record["fingerprint"]), 64)
        suite = ET.parse(run_dir / "junit.xml").getroot()
        self.assertEqual(suite.attrib["failures"], "0")
        self.assertIn("Outcome: passed", (run_dir / "summary.md").read_text())

    def assert_report_write_failure_revokes_candidate(self, filename):
        run_dir = self.root / ".scratch" / ("report-failure-" + filename.replace(".", "-"))
        run_dir.mkdir(parents=True)
        candidate = {"kind": "local-only-candidate", "publication": "none", "run_id": "synthetic"}
        record = {"run_id": "synthetic", "checks": [{"check_id": "ok", "status": "passed"}],
                  "steps": [], "candidate": candidate, "provenance": {"fixture": "synthetic"}}
        # Include an existing manifest so a failed finalizer must actively revoke it.
        (run_dir / "candidate.json").write_text(json.dumps(candidate), encoding="utf-8")
        real_write_text = Path.write_text
        real_junit_write = ET.ElementTree.write
        failures = []
        stdout = io.StringIO()

        def write_text(path, *args, **kwargs):
            if path == run_dir / filename:
                failures.append(path.name)
                if filename == "candidate.json":
                    # Simulate a partial manifest too; it must be removed on failure.
                    real_write_text(path, '{"kind":', encoding="utf-8")
                raise OSError("synthetic report write failure")
            return real_write_text(path, *args, **kwargs)

        def write_junit(tree, path, *args, **kwargs):
            if Path(path) == run_dir / filename:
                failures.append(Path(path).name)
                raise OSError("synthetic JUnit write failure")
            return real_junit_write(tree, path, *args, **kwargs)

        raised = None
        rc = None
        with patch.object(Path, "write_text", new=write_text), \
                patch.object(ET.ElementTree, "write", new=write_junit), \
                patch("sys.stdout", stdout):
            try:
                rc = pipeline._finish_run(run_dir, record)
            except OSError as exc:
                raised = exc
        self.assertTrue(failures, "failure injection missed the requested report")
        persisted = json.loads((run_dir / "run.json").read_text())
        # Separate subtests expose the persisted false-success regression even when
        # the old implementation also leaks the OSError to its caller.
        with self.subTest(contract="report errors return failure without raising"):
            self.assertIsNone(raised, f"_finish_run leaked OSError: {raised}")
            self.assertEqual(rc, 1)
        with self.subTest(contract="run.json must not retain passed after write failure"):
            self.assertEqual(persisted["outcome"], "failed")
            self.assertEqual(persisted["returncode"], 1)
        for label, payload in (("memory", record), ("run.json", persisted)):
            with self.subTest(contract="candidate revoked", surface=label):
                active = payload.get("candidate")
                self.assertTrue(active is None or active.get("valid") is False,
                                f"{label} retains a valid candidate")
                if payload.get("invalid_candidate") is not None:
                    self.assertIs(payload["invalid_candidate"].get("valid"), False)
        with self.subTest(contract="no manifest remains after failed report"):
            self.assertFalse((run_dir / "candidate.json").exists())
        with self.subTest(contract="failed report always leaves pause receipt"):
            receipt = json.loads((run_dir / "pause.json").read_text())
            self.assertEqual(receipt["stage"], "report")
            self.assertEqual(receipt["returncode"], 1)
            self.assertEqual(receipt["notification"]["detail"], "not_requested")
        if raised is None:
            with self.subTest(contract="printed outcome matches failure"):
                self.assertEqual(json.loads(stdout.getvalue())["outcome"], "failed")

    def test_junit_write_failure_returns_failed_run_and_revokes_candidate(self):
        self.assert_report_write_failure_revokes_candidate("junit.xml")

    def test_summary_write_failure_returns_failed_run_and_revokes_candidate(self):
        self.assert_report_write_failure_revokes_candidate("summary.md")

    def test_candidate_manifest_write_failure_returns_failed_run_and_revokes_candidate(self):
        self.assert_report_write_failure_revokes_candidate("candidate.json")

    def test_real_nonzero_never_classified_by_stderr_or_code(self):
        # Codes 1/2/4/127 must be failures regardless of what the child printed:
        # a misleading "command not found" on stderr cannot turn a failure into a block.
        # (3 is deliberately excluded here - it has its own contract, see below.)
        checks = {str(rc): check_entry("import sys; print('command not found',file=sys.stderr); sys.exit(" + str(rc) + ")")
                  for rc in (1, 2, 4, 127)}
        checks["after"] = check_entry()
        proc, record, run_dir = self.run_profile(checks)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual([c["status"] for c in record["checks"]], ["failed"] * 4 + ["passed"])
        self.assertEqual(ET.parse(run_dir / "junit.xml").getroot().attrib["failures"], "4")

    def test_exit_three_is_a_not_executed_contract_not_a_failure(self):
        # The pipeline's own "could not run" code (dry run, missing environment) must
        # not be reported as a defect in the code under test.
        proc, record, run_dir = self.run_profile({"dry": check_entry("import sys; sys.exit(3)")})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["outcome"], "blocked")
        self.assertEqual(record["checks"][0]["status"], "blocked")
        self.assertEqual(record["checks"][0]["returncode"], 3)
        self.assertEqual(ET.parse(run_dir / "junit.xml").getroot().attrib["skipped"], "1")

    def test_dependency_failure_blocks_cli_without_running_its_marker(self):
        for rc, status in ((3, "blocked"), (1, "failed")):
            with self.subTest(returncode=rc):
                checks = {
                    "bundle.cli": check_entry(
                        "from pathlib import Path; Path('.scratch/cli-marker').touch()",
                        depends_on=["package.artifact"]),
                    "package.artifact": check_entry(f"import sys; sys.exit({rc})"),
                    "independent": check_entry(),
                }
                proc, record, run_dir = self.run_profile(checks)
                self.assertFalse((self.root / ".scratch/cli-marker").exists())
                self.assertEqual(proc.returncode, rc)
                self.assertEqual([c["check_id"] for c in record["checks"]],
                                 ["package.artifact", "bundle.cli", "independent"])
                self.assertEqual([c["status"] for c in record["checks"]],
                                 [status, "blocked", "passed"])
                cli = record["checks"][1]
                self.assertIsNone(cli["returncode"])
                self.assertIn("package.artifact", cli["reason"])
                self.assertIn(status, cli["reason"])
                skipped = ET.parse(run_dir / "junit.xml").getroot().findall("testcase/skipped")
                self.assertTrue(any("package.artifact" in item.attrib["message"] for item in skipped))

    def test_dependency_runs_first_and_once_for_reversed_shared_chain(self):
        checks = {
            "bundle.cli": check_entry(
                "from pathlib import Path; p=Path('.scratch/order'); "
                "assert p.read_text() == 'artifact\\nadapter\\n'; "
                "p.open('a').write('cli\\n')",
                depends_on=["adapter", "package.artifact"]),
            "adapter": check_entry(
                "from pathlib import Path; p=Path('.scratch/order'); "
                "assert p.read_text() == 'artifact\\n'; p.open('a').write('adapter\\n')",
                depends_on=["package.artifact"]),
            "package.artifact": check_entry(
                "from pathlib import Path; p=Path('.scratch/order'); "
                "assert not p.exists(); p.write_text('artifact\\n')"),
        }
        proc, record, _ = self.run_profile(checks)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual([c["check_id"] for c in record["checks"]],
                         ["package.artifact", "adapter", "bundle.cli"])
        self.assertEqual((self.root / ".scratch/order").read_text(), "artifact\nadapter\ncli\n")

    def test_declared_blocked_returncodes_are_honoured(self):
        entry = {**check_entry("import sys; sys.exit(75)"), "blocked_returncodes": [75]}
        proc, record, _ = self.run_profile({"tempfile-unavailable": entry})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["outcome"], "blocked")
        self.assertEqual(record["checks"][0]["status"], "blocked")

    def test_missing_command_is_blocked_and_junit_skipped(self):
        proc, record, run_dir = self.run_profile({"missing": {"command": ["missing-automation-fixture-81"], "host_os": [HOST]}})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["outcome"], "blocked")
        self.assertEqual(record["checks"][0]["reason"], "command_unavailable")
        suite = ET.parse(run_dir / "junit.xml").getroot()
        self.assertEqual(suite.attrib["skipped"], "1")
        self.assertIsNotNone(suite.find("testcase/skipped"))

    def test_host_os_checked_before_execution(self):
        other = "linux" if HOST != "linux" else "darwin"
        proc, record, _ = self.run_profile({"wrong": check_entry("raise AssertionError()", host_os=[other])})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["checks"][0]["reason"], "host_os_mismatch")

    def test_env_exact_zero_blocks_and_nonempty_list_compatible(self):
        checks = {"exact": check_entry(requires_env={"AUTOMATION_FIXTURE_OPT_IN": "1"}),
                  "legacy": check_entry(requires_env=["AUTOMATION_FIXTURE_OPT_IN"])}
        proc, record, _ = self.run_profile(checks, env={"AUTOMATION_FIXTURE_OPT_IN": "0"})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual([c["status"] for c in record["checks"]], ["blocked", "passed"])
        proc, record, _ = self.run_profile(checks, env={"AUTOMATION_FIXTURE_OPT_IN": "1"})
        self.assertEqual(proc.returncode, 0)
        proc, record, _ = self.run_profile(checks, env={"AUTOMATION_FIXTURE_OPT_IN": ""})
        self.assertEqual([c["status"] for c in record["checks"]], ["blocked", "blocked"])

    def test_command_env_and_argv_placeholder_are_literal(self):
        value = "fixture value; $(never-execute)"
        entry = check_entry("import os,sys; assert sys.argv[1] == os.environ['AUTOMATION_FIXTURE_ARG']")
        entry["command"].append("{env:AUTOMATION_FIXTURE_ARG}")
        entry["command_env"] = {"AUTOMATION_FIXTURE_ARG": value}
        proc, _, _ = self.run_profile({"argv": entry})
        self.assertEqual(proc.returncode, 0, proc.stderr)
        del entry["command_env"]
        proc, record, _ = self.run_profile({"argv": entry}, env={"AUTOMATION_FIXTURE_ARG": ""})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["checks"][0]["reason"], "argv_env_missing")

    def test_binary_and_relative_executable_resolve_from_cwd(self):
        exe = self.write("bin/fixture", "#!/bin/sh\nexit 0\n")
        exe.chmod(0o755)
        entry = {"command": ["./bin/fixture"], "requires_binary": "bin/fixture", "host_os": [HOST]}
        proc, _, _ = self.run_profile({"relative": entry})
        self.assertEqual(proc.returncode, 0)
        exe.chmod(0o644)
        proc, record, _ = self.run_profile({"relative": entry})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["checks"][0]["reason"], "required_binary_unavailable")

    def test_output_never_published_even_on_failure(self):
        sentinel = "SYNTHETIC_PRIVATE_OUTPUT_SENTINEL"
        code = "import sys; sys.stdout.write('x'*2000000); sys.stderr.write(" + repr(sentinel) + "); sys.exit(4)"
        proc, record, run_dir = self.run_profile({"noisy": check_entry(code), "after": check_entry()})
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(record["checks"][-1]["status"], "passed")
        for path in run_dir.iterdir():
            self.assertNotIn(sentinel, path.read_text())
        self.assertNotIn(sentinel, proc.stdout + proc.stderr)
        self.assertEqual(record["checks"][0]["stderr_tail"], "")

    def assert_pid_gone(self, pid):
        # 断言的是「最终被回收」，不是「3 秒内被回收」：高负载下回收线程可能
        # 很晚才被调度，把调度延迟判成产品缺陷是测量装置的抖动。
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            # On Linux a dead orphan can briefly remain as a zombie until init reaps it.
            ps = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True)
            if ps.stdout.lstrip().startswith("Z"):
                return
            time.sleep(0.02)
        self.fail("descendant survived cleanup")

    def descendant_code(self):
        child = "import os,time; from pathlib import Path; Path('.scratch/grandchild.pid').write_text(str(os.getpid())); time.sleep(30)"
        return "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c'," + repr(child) + "]); time.sleep(30)"

    @unittest.skipUnless(os.name == "posix", "POSIX process group guarantee")
    def test_timeout_reclaims_grandchild_and_continues(self):
        # 预算要够孙子进程真正起来：否则 pidfile 还没写就被 kill，
        # 「回收失败」与「夹具没来得及建立」无法区分。
        proc, record, _ = self.run_profile({"timeout": check_entry(self.descendant_code()), "after": check_entry()}, timeout=5)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(record["checks"][0]["reason"], "timeout")
        self.assertEqual(record["checks"][1]["status"], "passed")
        self.assert_pid_gone(int((self.root / ".scratch/grandchild.pid").read_text()))

    @unittest.skipUnless(os.name == "posix", "POSIX signals")
    def test_sigint_writes_run_and_reclaims_grandchild(self):
        self.data = registry({"interrupt": check_entry(self.descendant_code()), "after": check_entry()})
        self.save()
        proc = subprocess.Popen(self.command("--profile", "test"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: proc.kill() if proc.poll() is None else None)
        pidfile = self.root / ".scratch/grandchild.pid"
        deadline = time.monotonic() + 30
        while not pidfile.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(pidfile.exists())
        proc.send_signal(signal.SIGINT)
        out, err = proc.communicate(timeout=60)
        self.assertNotEqual(proc.returncode, 0, err)
        record = json.loads((Path(json.loads(out)["evidence"]) / "run.json").read_text())
        self.assertEqual(record["outcome"], "failed")
        self.assertEqual(record["checks"][-1]["status"], "blocked")
        self.assert_pid_gone(int(pidfile.read_text()))

    def test_duplicate_json_key_rejected(self):
        self.write("registry.json", '{"schema_version":1,"schema_version":1}')
        with patch.object(pipeline, "REGISTRY", self.root / "registry.json"):
            with self.assertRaises(pipeline.PipelineError):
                pipeline.load_registry()

    def test_registry_schema_and_reference_negative_cases(self):
        mutations = [
            lambda r: r["profiles"].update(test=[]),
            lambda r: r["profiles"].update(test=["unknown"]),
            lambda r: r["profiles"].update(test=["ok", "ok"]),
            lambda r: r["profiles"].update(note=[]),
            lambda r: r["requirements"]["AUTO-01"].update(checks=[]),
            lambda r: r["requirements"]["AUTO-01"].update(checks=["unknown"]),
            lambda r: r["requirements"]["AUTO-01"].update(gates=["unknown"]),
            lambda r: r["checks"]["ok"].update(command="python3"),
            lambda r: r["checks"]["ok"].update(command=[]),
            lambda r: r["checks"]["ok"].update(command=[" "]),
            lambda r: r["checks"]["ok"].update(host_os=[]),
            lambda r: r["checks"]["ok"].update(host_os=["mac"]),
            lambda r: r["checks"]["ok"].update(requires_env=[]),
            lambda r: r["checks"]["ok"].update(requires_env={}),
            lambda r: r["checks"]["ok"].update(requires_env={"bad name": "1"}),
            lambda r: r["checks"]["ok"].update(requires_env={"OK": 1}),
            lambda r: r["checks"]["ok"].update(command_env={"OK": ""}),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                data = registry()
                mutate(data)
                with self.assertRaises(pipeline.PipelineError):
                    pipeline.validate_registry(data)
        self.data["profiles"]["test"] = []
        proc = self.launch("--profile", "test")
        self.assertEqual(proc.returncode, 2)
        record_path = Path(json.loads(proc.stdout)["evidence"]) / "run.json"
        self.assertEqual(json.loads(record_path.read_text())["returncode"], 2)

    def test_dependency_schema_and_graph_rejected_before_any_child(self):
        mutations = [
            ("string", lambda r: r["checks"]["ok"].update(depends_on="artifact")),
            ("null", lambda r: r["checks"]["ok"].update(depends_on=None)),
            ("non-string", lambda r: r["checks"]["ok"].update(depends_on=[1])),
            ("blank", lambda r: r["checks"]["ok"].update(depends_on=[" "])),
            ("nul", lambda r: r["checks"]["ok"].update(depends_on=["artifact\0"])),
            ("duplicate", lambda r: r["checks"]["ok"].update(depends_on=["artifact", "artifact"])),
            ("unknown", lambda r: r["checks"]["ok"].update(depends_on=["unknown"])),
            ("self", lambda r: r["checks"]["ok"].update(depends_on=["ok"])),
            ("cycle", lambda r: r["checks"]["artifact"].update(depends_on=["ok"])),
        ]
        for name, mutate in mutations:
            with self.subTest(case=name):
                self.data = registry({
                    "ok": check_entry("from pathlib import Path; Path('.scratch/must-not-run').touch()",
                                      depends_on=["artifact"]),
                    "artifact": check_entry(),
                })
                mutate(self.data)
                proc = self.launch("--validate-registry")
                self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
                self.assertIn("depend", proc.stderr)
                self.assertFalse((self.root / ".scratch").exists())
                proc = self.launch("--profile", "test")
                self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
                record = json.loads((Path(json.loads(proc.stdout)["evidence"]) / "run.json").read_text())
                self.assertEqual(record["checks"], [])
                self.assertFalse((self.root / ".scratch/must-not-run").exists())
                shutil.rmtree(self.root / ".scratch")

    def test_profile_missing_transitive_dependency_is_rejected(self):
        self.data = registry({
            "cli": check_entry(depends_on=["adapter"]),
            "adapter": check_entry(depends_on=["artifact"]),
            "artifact": check_entry(depends_on=[]),
        })
        self.data["profiles"]["test"] = ["cli", "adapter"]
        proc = self.launch("--validate-registry")
        self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
        self.assertIn("artifact", proc.stderr)
        self.assertFalse((self.root / ".scratch").exists())
        # Validate every profile, including ones not selected for execution.
        self.data["profiles"]["test"].append("artifact")
        self.data["profiles"]["incomplete"] = ["cli"]
        proc = self.launch("--profile", "test")
        self.assertEqual(proc.returncode, 2)
        record = json.loads((Path(json.loads(proc.stdout)["evidence"]) / "run.json").read_text())
        self.assertEqual(record["checks"], [])
        del self.data["profiles"]["incomplete"]
        proc = self.launch("--validate-registry")
        self.assertEqual(proc.returncode, 0, proc.stderr)

    def test_config_reference_errors_fail_before_any_child(self):
        for mutate in (lambda r: r["profiles"].update(test=["unknown"]),
                       lambda r: r["requirements"]["AUTO-01"].update(checks=["unknown"])):
            self.data = registry({"ok": check_entry("raise AssertionError('must not run')")})
            mutate(self.data)
            proc = self.launch("--profile", "test")
            self.assertEqual(proc.returncode, 2)
            record = json.loads((Path(json.loads(proc.stdout)["evidence"]) / "run.json").read_text())
            self.assertEqual(record["checks"], [])

    def test_empty_resolve_and_self_reference_rejected(self):
        with self.assertRaises(pipeline.PipelineError):
            pipeline.resolve_checks(registry(), [])
        data = registry()
        data["checks"]["ok"]["command"] = [PY, "scripts/automation/pipeline.py", "--profile", "test"]
        with self.assertRaises(pipeline.PipelineError):
            pipeline.validate_registry(data)
        with self.assertRaises(pipeline.PipelineError):
            pipeline.resolve_checks(data, ["AUTO-01"])

    def test_requirement_selection_adds_transitive_dependencies_once(self):
        self.data = registry({
            "cli": check_entry(
                "from pathlib import Path; assert Path('.scratch/prerequisite').read_text() == 'ready'",
                depends_on=["adapter", "artifact"]),
            "adapter": check_entry(
                "from pathlib import Path; assert Path('.scratch/prerequisite').is_file()",
                depends_on=["artifact"]),
            "artifact": check_entry(
                "from pathlib import Path; p=Path('.scratch/prerequisite'); "
                "assert not p.exists(); p.parent.mkdir(exist_ok=True); p.write_text('ready')"),
        })
        self.data["requirements"]["AUTO-01"]["checks"] = ["cli"]
        proc = self.launch("--requirements", "AUTO-01", "AUTO-01")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        record = json.loads((Path(json.loads(proc.stdout)["evidence"]) / "run.json").read_text())
        self.assertEqual([c["check_id"] for c in record["checks"]], ["artifact", "adapter", "cli"])
        self.assertTrue(all(c["status"] == "passed" for c in record["checks"]))

    def test_self_referential_requirement_dependency_is_rejected(self):
        self.data = registry({
            "cli": check_entry(depends_on=["recursive"]),
            "recursive": {"command": [PY, "scripts/automation/pipeline.py", "--requirements", "AUTO-01"],
                          "host_os": [HOST]},
            "independent": check_entry(),
        })
        self.data["profiles"]["test"] = ["independent"]
        self.data["requirements"]["AUTO-01"]["checks"] = ["cli"]
        proc = self.launch("--requirements", "AUTO-01")
        self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
        record = json.loads((Path(json.loads(proc.stdout)["evidence"]) / "run.json").read_text())
        self.assertEqual(record["checks"], [])
        self.assertNotIn("workspace", record)

    def test_planned_requirements_are_never_silently_skipped(self):
        # An empty-checks requirement is honest in the registry, but selecting it must
        # fail loudly: silently dropping it would let "not executed" look like coverage.
        data = registry()
        data["requirements"]["PLANNED"] = {"checks": [], "status": "planned"}
        with self.assertRaises(pipeline.PipelineError) as caught:
            pipeline.resolve_checks(data, ["AUTO-01", "PLANNED"])
        self.assertIn("PLANNED", str(caught.exception))
        with self.assertRaises(pipeline.PipelineError):
            pipeline.resolve_checks(data, ["PLANNED"])
        # A requirement without executable checks and without a planned marker is a
        # configuration error, not a silently empty selection.
        del data["requirements"]["PLANNED"]["status"]
        with self.assertRaises(pipeline.PipelineError):
            pipeline.validate_registry(data)

    def test_a_stale_binary_is_blocked_not_passed(self):
        # An artifact older than the source it claims to contain must not certify
        # behaviour that was never rebuilt.
        self.write("source.py", "print('source')")
        binary = self.root / "dist" / "stub"
        binary.parent.mkdir(parents=True)
        binary.write_text("#!/bin/sh\n")
        binary.chmod(0o755)
        stale = binary.stat().st_mtime - 3600
        import os
        os.utime(binary, (stale, stale))
        proc, record, _ = self.run_profile({"stale": {**check_entry("raise AssertionError('must not run')"),
                                                      "requires_binary": "dist/stub",
                                                      "binary_newer_than": ["source.py"]}})
        self.assertEqual(proc.returncode, 3)
        self.assertEqual(record["outcome"], "blocked")
        self.assertEqual(record["checks"][0]["reason"], "required_binary_stale:source.py")

    def test_source_drift_tracked_and_untracked_refuses_pass(self):
        for name in ("source.txt", "new-source.py"):
            with self.subTest(name=name):
                code = "from pathlib import Path; Path(" + repr(name) + ").write_text('changed')"
                proc, record, _ = self.run_profile({"drift": check_entry(code)})
                self.assertEqual(proc.returncode, 1)
                self.assertEqual(record["outcome"], "failed")
                self.assertNotEqual(record["fingerprint_before"], record["fingerprint_after"])
                self.assertIn("source_drift", [s["detail"] for s in record["steps"]])

    def test_a_failing_step_is_never_hidden_behind_passing_checks(self):
        # Drift during the run fails a step while every check passes; the summary must
        # name the failing step instead of reporting an empty "nothing failed" list.
        proc, record, _ = self.run_profile({"drift": check_entry(
            "from pathlib import Path; Path('source.txt').write_text('changed during run')")})
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(record["outcome"], "failed")
        self.assertEqual(record["summary"]["failed"], 0)
        self.assertEqual(record["summary"]["passed"], 1)
        self.assertEqual(record["failed_steps"], ["source.integrity"])
        self.assertEqual(record["failed_checks"], [])
        self.assertEqual(record["failed_or_blocked"], ["source.integrity"])

    def test_fingerprint_changes_between_two_dirty_versions_and_ignores_outputs(self):
        self.write("source.txt", "first dirty")
        a = pipeline.worktree_fingerprint(self.root)
        self.write("source.txt", "second dirty")
        b = pipeline.worktree_fingerprint(self.root)
        self.assertNotEqual(a, b)
        self.write("target/build", "output")
        self.write(".scratch/evidence", "output")
        self.assertEqual(b, pipeline.worktree_fingerprint(self.root))
        self.write("untracked.py", "source")
        self.assertNotEqual(b, pipeline.worktree_fingerprint(self.root))

    def test_protocol_budget_reaches_the_real_subprocess_wait(self):
        real_wait = subprocess.Popen.wait
        for supplied, declared, expected in ((900, 7200, 7200), (8100, 7200, 8100),
                                             (60, 7200, 7200), (900, None, 900)):
            with self.subTest(supplied=supplied, declared=declared):
                data = registry({'ok': check_entry()})
                if declared is not None:
                    data['checks']['ok']['timeout_seconds'] = declared
                pipeline.validate_registry(data)
                calls = []
                def wait(proc, *args, **kwargs):
                    calls.append(kwargs.get('timeout'))
                    return real_wait(proc, *args, **kwargs)
                with patch.object(subprocess.Popen, 'wait', wait):
                    result = pipeline.run_check('ok', data, self.root, supplied)
                self.assertEqual(result.status, 'passed')
                self.assertEqual(calls[0], expected)
        for value in (True, False):
            data = registry({'ok': check_entry(timeout_seconds=value)})
            with self.assertRaises(pipeline.PipelineError):
                pipeline.validate_registry(data)

    def test_readonly_cli_and_positive_timeout(self):
        for flag in ("--list-profiles", "--validate-registry"):
            proc = self.launch(flag)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertFalse((self.root / ".scratch").exists())
        for flag in ("--agent-timeout", "--check-timeout"):
            for value in ("0", "-1"):
                proc = self.launch("--profile", "test", flag, value)
                self.assertEqual(proc.returncode, 2)

    def test_pipeline_mock_checks_worktree_and_keep_compatible(self):
        entry = check_entry("from pathlib import Path; assert Path.cwd().name.startswith('workspace-')")
        self.data = registry({"cwd": entry})
        for scenario, expected in (("ok", 0), ("auth", 1), ("timeout", 1)):
            with self.subTest(scenario=scenario):
                # Each scenario is a distinct request; reusing one prompt in the
                # same fixture is what duplicate-submission pausing must refuse.
                proc = self.launch("--scenario", scenario, "--keep-workspace",
                                   "--prompt", f"synthetic runner request {scenario}")
                self.assertEqual(proc.returncode, expected, proc.stderr)
                run_dir = Path(json.loads(proc.stdout)["evidence"])
                self.assertTrue((run_dir / "agent-result.json").is_file())
                record = json.loads((run_dir / "run.json").read_text())
                self.assertTrue(record["fingerprint"])
                self.assertTrue(Path(record["workspace"]["path"]).exists())
                if expected:
                    self.assertTrue(all(c["status"] == "blocked" for c in record["checks"]))
        proc = self.launch("--driver", "agy")
        self.assertEqual(proc.returncode, 3)
        run_dir = Path(json.loads(proc.stdout)["evidence"])
        self.assertFalse((run_dir / "agent-result.json").exists())
        record = json.loads((run_dir / "run.json").read_text())
        self.assertNotIn("workspace", record)
        proc = self.launch("--requirements", "unknown")
        self.assertEqual(proc.returncode, 2)

    def test_unique_workspace_preserves_existing_dirty_and_removes_only_clean_owned(self):
        old = self.root / ".scratch/workspace"
        self.git("worktree", "add", "--detach", str(old), "HEAD")
        (old / "source.txt").write_text("existing dirty")
        with patch.object(pipeline, "WORKSPACE_ROOT", old):
            wt, _ = pipeline.create_workspace(self.root, self.root / ".scratch/run-a", False)
            second, _ = pipeline.create_workspace(self.root, self.root / ".scratch/run-b", False)
            self.assertNotEqual(wt, second)
            self.assertNotEqual(wt, old)
            self.assertEqual((old / "source.txt").read_text(), "existing dirty")
            self.assertEqual(pipeline.cleanup_workspace(self.root, old, False).status, "failed")
            self.assertEqual(pipeline.cleanup_workspace(self.root, wt, False).status, "passed")
            self.assertFalse(wt.exists())
            (second / "source.txt").write_text("new dirty")
            self.assertEqual(pipeline.cleanup_workspace(self.root, second, False).status, "blocked")
            self.assertTrue(second.exists())
            self.assertEqual((second / "source.txt").read_text(), "new dirty")

    def test_existing_selftest_runs_with_synthetic_gates(self):
        # Execute the unchanged selftest contract without any native/cargo/real agent.
        automation = self.root / "scripts/automation"
        shutil.copytree(RUNNER.parent, automation, ignore=shutil.ignore_patterns("__pycache__"))
        (automation / "registry.json").write_text(json.dumps(self.data))
        self.save()
        self.git("add", "--", "scripts/automation")
        self.git("-c", "user.name=Runner Test", "-c", "user.email=runner@example.invalid", "commit", "-qm", "synthetic selftest")
        proc = subprocess.run([PY, str(automation / "selftest.py")], cwd=self.root,
                              env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
                              capture_output=True, text=True, timeout=300)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("ALL PASS", proc.stdout)

    def test_cleanup_status_changes_pipeline_outcome(self):
        self.data = registry({"dirty": check_entry("from pathlib import Path; Path('new-file').write_text('keep')")})
        proc = self.launch()
        self.assertEqual(proc.returncode, 1)  # source drift fails before dirty retention blocks
        record = json.loads((Path(json.loads(proc.stdout)["evidence"]) / "run.json").read_text())
        self.assertEqual(record["steps"][-1]["detail"], "dirty_workspace_preserved")
        self.assertTrue(Path(record["workspace"]["path"]).exists())
        self.data = registry({"missing": {"command": ["missing-automation-fixture-81"], "host_os": [HOST]}})
        proc = self.launch()
        self.assertEqual(proc.returncode, 3)

    def test_a_blocked_reason_is_distinguishable_from_a_failed_one(self):
        # rc=3 and rc=4 are both "not zero", but only one means "could not run".
        # The reason string must separate them, or a rebuild-only profile and a
        # genuinely broken check read identically in the summary.
        blocked = pipeline.run_check('ok', registry({'ok': check_entry("import sys; sys.exit(3)")}),
                                     self.root, 60)
        failed = pipeline.run_check('ok', registry({'ok': check_entry("import sys; sys.exit(4)")}),
                                    self.root, 60)
        self.assertEqual((blocked.status, blocked.reason), ('blocked', 'returncode=3'))
        self.assertEqual((failed.status, failed.reason), ('failed', 'returncode=4'))
        self.assertNotEqual(blocked.reason, failed.reason)

    def test_check_output_is_never_published_regardless_of_exit_code(self):
        # Existing contract, re-asserted next to the reason test above: a check's
        # transcript is never written into the evidence directory, on any exit path.
        # Readability of a reason must never be bought with the transcript itself.
        sentinel = "SYNTHETIC_PRIVATE_OUTPUT_SENTINEL"
        code = "import sys; sys.stdout.write('x'*500000); sys.stderr.write(" + repr(sentinel) + "); sys.exit(3)"
        proc, record, run_dir = self.run_profile({"noisy": check_entry(code), "after": check_entry()})
        self.assertEqual(proc.returncode, 3)
        for path in run_dir.iterdir():
            self.assertNotIn(sentinel, path.read_text())
        self.assertNotIn(sentinel, proc.stdout + proc.stderr)
        self.assertEqual(record["checks"][0]["stderr_tail"], "")
        self.assertEqual(record["checks"][0]["status"], "blocked")


if __name__ == "__main__":
    unittest.main(verbosity=2)
