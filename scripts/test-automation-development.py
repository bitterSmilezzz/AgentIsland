#!/usr/bin/env python3
"""Development-loop meta-tests: real temporary git fixtures, synthetic checks, mock agent only.

Run: python3 scripts/test-automation-development.py
No native builds, credentials, network, real agents or shared-repo writes.
The pipeline under test is loaded from scripts/automation/pipeline.py and driven in a
subprocess so the real CLI path (argparse -> run_pipeline) is exercised.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
AUTOMATION = SCRIPTS / "automation"
PIPELINE_PATH = AUTOMATION / "pipeline.py"
PY = sys.executable or "python3"
HOST = "windows" if sys.platform == "win32" else sys.platform

spec = importlib.util.spec_from_file_location("automation_pipeline_development", PIPELINE_PATH)
pipeline = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = pipeline
spec.loader.exec_module(pipeline)

# Re-root the real CLI inside the fixture instead of re-implementing the pipeline entry.
BOOTSTRAP = """
import sys
from pathlib import Path
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import pipeline
root = Path(sys.argv[2])
pipeline.REPO = root
pipeline.REGISTRY = root / 'scripts/automation/registry.json'
pipeline.EVIDENCE_ROOT = root / '.scratch/runs'
pipeline.WORKSPACE_ROOT = root / '.scratch/workspace'
pipeline.LESSON_LEDGER = root / '.scratch/lessons.jsonl'
# Inject author edits only into the disposable mock workspace. CLI, planning,
# checks, review and candidate publication still run through production code.
import json
import os
edit_path = os.environ.get('TEST_DEVELOPMENT_PLAN_EDIT')
if edit_path:
    assert edit_path in ('docs/需求.md', 'scripts/automation/registry.json')
    original_edit = pipeline.mock_develop_edit
    def edit_plan(wt, round_no):
        original_edit(wt, round_no)
        target = wt / edit_path
        if edit_path == 'docs/需求.md':
            target.write_text('# Changed requirements: require a newly planned check\\n', encoding='utf-8')
        else:
            data = json.loads(target.read_text(encoding='utf-8'))
            data['checks']['new-required'] = {
                'command': [sys.executable, '-c', 'raise SystemExit(1)'],
                'host_os': [pipeline_host]}
            data['requirements']['AUTO-01']['checks'].append('new-required')
            target.write_text(json.dumps(data), encoding='utf-8')
    pipeline_host = 'windows' if sys.platform == 'win32' else sys.platform
    pipeline.mock_develop_edit = edit_plan
# Patch the transport itself: a real author edits inside run_agent, before it
# returns, rather than inside the pipeline's later mock_develop_edit helper.
if os.environ.get('TEST_DEVELOPMENT_AGENT_EDIT'):
    import agent_driver
    original_run_agent = agent_driver.run_agent
    author_turn = 0
    def edit_in_agent(**kwargs):
        global author_turn
        assert kwargs['driver'] == 'mock', 'fixture must never invoke a real model'
        if not kwargs['prompt'].startswith('Read-only review of '):
            author_turn += 1
            wt = kwargs['cwd']
            events = os.environ.get('TEST_DEVELOPMENT_EVENTS')
            if events:
                with Path(events).open('a') as stream:
                    stream.write(json.dumps({'kind': 'author', 'cwd': str(wt),
                                             'source': (wt / 'source.txt').read_text()}) + '\\n')
            if os.environ['TEST_DEVELOPMENT_AGENT_EDIT'] == 'raise':
                raise RuntimeError('synthetic author transport failure')
            if os.environ['TEST_DEVELOPMENT_AGENT_EDIT'] == 'edit':
                (wt / 'source.txt').write_text(f'author round {author_turn}\\n')
        return original_run_agent(**kwargs)
    agent_driver.run_agent = edit_in_agent
if os.environ.get('TEST_DEVELOPMENT_REGISTRY_AFTER_LOAD'):
    from unittest.mock import patch
    original_load = pipeline.load_registry
    registry_changed = False
    def load_then_change_registry():
        global registry_changed
        loaded = original_load()
        if not registry_changed:
            # Change disk bytes without changing the object returned to the caller.
            changed = json.loads(pipeline.REGISTRY.read_text(encoding='utf-8'))
            changed['checks']['new-required'] = {
                'command': [sys.executable, '-c', 'raise SystemExit(1)'],
                'host_os': ['windows' if sys.platform == 'win32' else sys.platform]}
            changed['requirements']['AUTO-01']['checks'].append('new-required')
            pipeline.REGISTRY.write_text(json.dumps(changed), encoding='utf-8')
            registry_changed = True
        return loaded
    # The patch lives only for this disposable CLI subprocess.
    patch.object(pipeline, 'load_registry', side_effect=load_then_change_registry).start()
sys.argv = ['pipeline', *sys.argv[3:]]
raise SystemExit(pipeline.main())
"""


def check_entry(code="pass", **extra):
    return {"command": [PY, "-c", code], "host_os": [HOST], **extra}


def registry(checks=None):
    checks = checks or {"ok": check_entry()}
    return {"schema_version": 1, "gates": {"L0": {"required": True}},
            "requirements": {"AUTO-01": {"checks": list(checks), "gates": ["L0"]}},
            "checks": checks, "profiles": {"test": list(checks), "note": "synthetic"},
            "agents": {"default": "mock", "available": ["mock", "agy"]}}


class DevelopmentTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="automation-development-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.write(".gitignore", ".scratch/\ntarget/\n__pycache__/\n")
        self.write("source.txt", "baseline\n")
        self.write("docs/需求.md", "# Synthetic requirements\nAUTO-01: baseline check\n")
        self.data = registry()
        self.save()
        self.git("init", "-q")
        self.git("add", "--", ".gitignore", "source.txt", "docs/需求.md",
                 "scripts/automation/registry.json")
        self.git("-c", "user.name=Dev Test", "-c", "user.email=dev@example.invalid", "commit", "-qm", "fixture")

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def git(self, *args, check=True):
        return subprocess.run(["git", *args], cwd=self.root, text=True, capture_output=True, check=check)

    def save(self):
        self.write("scripts/automation/registry.json", json.dumps(self.data))

    # 预算必须够；这里断言的是**流水线行为**，不是它跑多快。固定 120 秒在负载 80+
    # 的机器上会被真实跑穿（实测一次 1329 秒的整轮里多个 launch 撞到上限），
    # 表现为「测试红但没有任何产品差异」——把调度延迟记成缺陷比没有门禁更糟。
    LAUNCH_TIMEOUT = 900

    def launch(self, *args, env=None):
        self.save()
        command = [PY, "-c", BOOTSTRAP, str(AUTOMATION), str(self.root), *args]
        return subprocess.run(command, capture_output=True, text=True,
                              cwd=self.root, env={**os.environ, **(env or {})},
                              timeout=self.LAUNCH_TIMEOUT)

    def develop(self, *args, env=None):
        proc = self.launch(*args, env=env)
        summary = json.loads(proc.stdout)
        evidence = Path(summary["evidence"])
        return proc, json.loads((evidence / "run.json").read_text()), evidence

    def assert_rejected(self, scenario, reason, *args):
        # Each intent is a distinct request: reusing one prompt inside the same
        # fixture is exactly the duplicate submission the runner must pause.
        proc, record, evidence = self.develop("--scenario", "edit", "--review-scenario", scenario,
                                               "--prompt", f"synthetic request for {scenario}", *args)
        self.assertNotEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(reason, [s["detail"] for s in record["steps"]])
        self.assertFalse((evidence / "candidate.json").exists())
        self.assertTrue(Path(record["workspace"]["path"]).exists())
        return record

    def test_edit_revise_retest_new_review_candidate_and_main_unchanged(self):
        self.write("source.txt", "unstaged current bytes\n")
        self.write("staged.txt", "staged bytes\n")
        self.git("add", "--", "staged.txt")
        self.write("untracked.py", "CURRENT_UNTRACKED = True\n")
        self.write("deleted.txt", "delete me")
        self.git("add", "--", "deleted.txt")
        (self.root / "deleted.txt").unlink()
        index_path = self.root / ".git/index"
        index_before = index_path.read_bytes()
        checks = {
            "current": check_entry(
                "from pathlib import Path; assert Path('source.txt').read_text() == 'unstaged current bytes\\n'; "
                "assert Path('staged.txt').read_text() == 'staged bytes\\n'; "
                "assert Path('untracked.py').exists(); assert not Path('deleted.txt').exists()"),
            "edit": check_entry(
                "from pathlib import Path; assert Path('automation-mock-change.txt').read_text()"
                ".startswith('mock development round ')")}
        self.data = registry(checks)
        proc, record, evidence = self.develop("--scenario", "edit", "--review-scenario", "revise-once")
        self.assertEqual(proc.returncode, 0, proc.stderr + json.dumps(record))
        rounds = record["rounds"]
        self.assertEqual(len(rounds), 2)
        self.assertEqual([r["review"]["verdict"] for r in rounds], ["revise", "pass"])
        self.assertNotEqual(rounds[0]["input_digest"], rounds[1]["input_digest"])
        self.assertNotEqual(rounds[0]["snapshot_commit"], rounds[1]["snapshot_commit"])
        for r in rounds:
            self.assertEqual(len(r["checks"]), 2)
            self.assertTrue(all(c["status"] == "passed" for c in r["checks"]))
            self.assertEqual(r["input_digest"], r["checks_digest"])
            self.assertEqual(r["input_digest"], r["review"]["input_digest"])
            self.assertEqual(r["review_fingerprint_before"], r["review_fingerprint_after"])
            self.assertNotEqual(record["workspace"]["path"], r["review_workspace"]["path"])
            self.assertFalse(Path(r["review_workspace"]["path"]).exists())
        manifest = json.loads((evidence / "candidate.json").read_text())
        self.assertEqual(manifest["input_digest"], rounds[-1]["input_digest"])
        self.assertEqual(manifest["review_kind"], "deterministic-mock")
        self.assertEqual(manifest["publication"], "none")
        self.assertIn("automation-mock-change.txt", (evidence / "candidate.patch").read_text())
        wt = Path(record["workspace"]["path"])
        self.assertTrue(wt.exists())
        self.assertEqual(record["workspace"]["cleanup_policy"], "retained")
        self.assertEqual((wt / "automation-mock-change.txt").read_text(), "mock development round 2\n")
        self.assertEqual(index_path.read_bytes(), index_before)
        self.assertEqual((self.root / "source.txt").read_bytes(), b"unstaged current bytes\n")
        self.assertEqual((self.root / "untracked.py").read_bytes(), b"CURRENT_UNTRACKED = True\n")
        self.assertFalse((self.root / "automation-mock-change.txt").exists())

    def assert_no_valid_candidate(self, record, evidence):
        # An explicit invalid manifest is allowed; an ordinary local manifest
        # remaining after failure must never look publishable to a consumer.
        manifest_path = evidence / "candidate.json"
        candidates = [("run.candidate", record.get("candidate"))]
        if manifest_path.exists():
            candidates.append(("candidate.json", json.loads(manifest_path.read_text())))
        for location, candidate in candidates:
            with self.subTest(candidate=location):
                if candidate is None:
                    continue
                self.assertIsInstance(candidate, dict)
                invalid = (candidate.get("valid") is False
                           or candidate.get("status") in ("invalid", "revoked")
                           or candidate.get("kind") in ("invalid", "revoked", "invalid-candidate"))
                self.assertTrue(invalid, f"{location} still exposes a valid candidate")

    def test_invalid_lesson_failure_revokes_local_candidate(self):
        proc, record, evidence = self.develop("--scenario", "edit", "--lesson", "!!!")
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(record["outcome"], "failed")
        self.assertIn("lesson_invalid", [s["detail"] for s in record["steps"]])
        self.assert_no_valid_candidate(record, evidence)

    def assert_plan_change_requires_replanning(self, changed_path):
        marker = self.root / ".scratch/obsolete-plan-check-ran"
        self.save()
        origin_before = (self.root / changed_path).read_bytes()
        # An absolute marker observes timing across the disposable check copies.
        # Read the plan from check cwd: unchanged bytes prove before-edit execution;
        # changed bytes would prove an obsolete check ran after the author edit.
        if changed_path == "scripts/automation/registry.json":
            # The registry contains the check itself; compare its planned semantic
            # contents rather than embedding those same bytes recursively.
            planned = ("'new-required' not in json.loads("
                       "Path('scripts/automation/registry.json').read_text())['checks']")
        else:
            planned = "Path(" + repr(changed_path) + ").read_bytes() == " + repr(origin_before)
        self.data = registry({"obsolete-plan": check_entry(
            "from pathlib import Path; import json; "
            "phase = 'before edit' if " + planned + " else 'after edit'; "
            "marker = Path(" + repr(str(marker)) + "); "
            "marker.write_text((marker.read_text() if marker.exists() else '') + phase + '\\n')")})
        self.save()
        origin_before = (self.root / changed_path).read_bytes()
        proc, record, evidence = self.develop(
            "--scenario", "edit", env={"TEST_DEVELOPMENT_PLAN_EDIT": changed_path})
        # Independent subtests expose all consequences of executing a stale plan.
        with self.subTest(contract="must report replanning reason"):
            self.assertIn("requirement_plan_changed", [s["detail"] for s in record["steps"]])
        with self.subTest(contract="must pause for replanning"):
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn(record["outcome"], ("failed", "blocked"))
        with self.subTest(contract="old checks execute only before edit"):
            self.assertTrue(marker.exists(), "planned diagnostic baseline never executed")
            self.assertEqual(marker.read_text().splitlines(), ["before edit"],
                             "obsolete checks executed after author changed the plan")
            self.assertFalse(any(c["status"] == "passed" for c in record["checks"]),
                             "obsolete plan was certified after author changed plan inputs")
            self.assertFalse(record["rounds"][0]["red_check"]["authoritative"])
        with self.subTest(contract="no review under obsolete plan"):
            self.assertFalse(any(r.get("review") for r in record["rounds"]))
        self.assert_no_valid_candidate(record, evidence)
        # The mutation belongs to the isolated author workspace, never origin.
        self.assertEqual((self.root / changed_path).read_bytes(), origin_before)

    def test_requirement_document_edit_pauses_before_checks_and_candidate(self):
        self.assert_plan_change_requires_replanning("docs/需求.md")

    def test_registry_edit_pauses_before_checks_and_candidate(self):
        self.assert_plan_change_requires_replanning("scripts/automation/registry.json")

    def test_registry_changed_after_load_cannot_certify_new_snapshot_with_old_checks(self):
        proc, record, evidence = self.develop(
            "--scenario", "edit", env={"TEST_DEVELOPMENT_REGISTRY_AFTER_LOAD": "1"})
        changed = json.loads((self.root / "scripts/automation/registry.json").read_text())
        # Prove the fixture really installed a new, required failing check on disk.
        self.assertIn("new-required", changed["requirements"]["AUTO-01"]["checks"])
        self.assertIn("raise SystemExit(1)", changed["checks"]["new-required"]["command"])
        with self.subTest(contract="stale loaded registry must fail"):
            self.assertEqual(proc.returncode, 1, proc.stderr)
            self.assertEqual(record["outcome"], "failed")
            self.assertEqual(record["returncode"], 1)
        with self.subTest(contract="must identify changed plan source"):
            details = [s["detail"] for s in record["steps"]]
            self.assertTrue(any(reason in details for reason in
                                ("requirement_plan_changed", "registry_source_changed")), details)
        with self.subTest(contract="old registry cannot publish a candidate"):
            self.assert_no_valid_candidate(record, evidence)
            self.assertFalse((evidence / "candidate.json").exists())

    def assert_safe_source_binding(self, payload, expected, trigger):
        # Container naming is intentionally unspecified. The digest fields must
        # coexist in one binding object, rather than be borrowed from a nested
        # candidate/round (which would conceal a missing run-level binding).
        def objects(value):
            if isinstance(value, dict):
                yield value
                for key, child in value.items():
                    if key not in ("rounds", "candidate"):
                        yield from objects(child)
        def matches(item):
            for key, value in expected.items():
                if key == "input_sha256":
                    # Both names identify the trigger's external JSON snapshot,
                    # distinct from the source string and workspace fingerprint.
                    if not any(item.get(alias) == value for alias in ("input_sha256", "snapshot_sha256")):
                        return False
                elif item.get(key) != value:
                    return False
            return True
        bindings = [item for item in objects(payload) if matches(item)]
        self.assertTrue(bindings, "missing safe source/prompt/snapshot digest binding")
        self.assertTrue(any(item.get("trigger") == trigger or item.get("type") == trigger
                            or payload.get("trigger") == trigger for item in bindings),
                        "source binding is missing trigger type")

    def assert_source_evidence(self, source, prompt, private_markers, input_digest=None):
        trigger = "manual-github"
        proc, record, evidence = self.develop(
            "--scenario", "edit", "--review-scenario", "revise-once",
            "--trigger", trigger, "--source", source, "--prompt", prompt)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(len(record["rounds"]), 2)
        expected = {"source_sha256": hashlib.sha256(source.encode("utf-8")).hexdigest(),
                    "prompt_sha256": hashlib.sha256(prompt.encode("utf-8")).hexdigest()}
        if input_digest is not None:
            expected["input_sha256"] = input_digest
        manifest = json.loads((evidence / "candidate.json").read_text())
        surfaces = [("run", record), ("candidate.json", manifest),
                    ("run.candidate", record["candidate"])]
        surfaces.extend((f"round.{r['round']}", r) for r in record["rounds"])
        for label, payload in surfaces:
            with self.subTest(binding=label):
                self.assert_safe_source_binding(payload, expected, trigger)
        # Scan every persisted evidence file, not just selected JSON keys. The
        # synthetic markers stand in for private notes/body; no real secrets.
        outputs = [("stdout/stderr", (proc.stdout + proc.stderr).encode("utf-8"))]
        outputs.extend((str(p.relative_to(evidence)), p.read_bytes())
                       for p in evidence.rglob("*") if p.is_file())
        for label, content in outputs:
            with self.subTest(privacy=label):
                self.assertTrue(all(marker.encode("utf-8") not in content for marker in private_markers),
                                "synthetic private source/prompt text was persisted or reflected")

    def test_manual_private_input_retains_only_digest_binding_at_every_level(self):
        source = "synthetic-private-operator-note-甲"
        prompt = "synthetic-private-manual-request-乙\nquoted requirements only"
        self.assert_source_evidence(source, prompt,
                                    ("synthetic-private-operator-note-", "synthetic-private-manual-request-"))

    def test_github_snapshot_digest_survives_every_round_and_candidate(self):
        # Use the actual trigger's offline snapshot encoder; gh/network are never
        # invoked. This proves its source format is recognized by the real CLI.
        trigger_spec = importlib.util.spec_from_file_location("development_trigger", AUTOMATION / "trigger.py")
        trigger_module = importlib.util.module_from_spec(trigger_spec)
        trigger_spec.loader.exec_module(trigger_module)
        payload = {"title": "Synthetic issue", "body": "synthetic-private-issue-body-丙",
                   "url": "https://github.com/example/project/issues/123"}
        source, prompt = trigger_module.issue_material(payload)
        snapshot = json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        digest = hashlib.sha256(snapshot.encode("utf-8")).hexdigest()
        self.assert_source_evidence(source, prompt, ("synthetic-private-issue-body-", payload["url"]), digest)

    def test_reject_preserves_dirty_without_candidate(self):
        self.assert_rejected("reject", "review_rejected")

    def test_stale_review_rejected(self):
        self.assert_rejected("stale", "review_binding_stale")

    def test_review_modification_rejected_and_retained(self):
        record = self.assert_rejected("mutate", "review_source_modified")
        review = record["rounds"][0]
        self.assertNotEqual(review["review_fingerprint_before"], review["review_fingerprint_after"])
        self.assertTrue(Path(review["review_workspace"]["path"]).exists())

    def test_red_baseline_precedes_author_transport_edits_in_every_round(self):
        events = self.root / ".scratch/baseline-events.jsonl"
        self.data = registry({"current": check_entry(
            "from pathlib import Path; import json; "
            "source = Path('source.txt').read_text(); "
            "event = {'kind': 'check', 'cwd': str(Path.cwd()), 'source': source}; "
            "stream = Path(" + repr(str(events)) + ").open('a'); "
            "stream.write(json.dumps(event) + '\\n'); stream.close(); "
            "assert source.startswith('author round ')")})
        proc, record, _ = self.develop(
            "--scenario", "edit", "--review-scenario", "revise-once",
            env={"TEST_DEVELOPMENT_AGENT_EDIT": "edit", "TEST_DEVELOPMENT_EVENTS": str(events)})
        self.assertEqual(proc.returncode, 0, json.dumps(record))
        observed = [json.loads(line) for line in events.read_text().splitlines()]
        self.assertEqual([item["kind"] for item in observed],
                         ["check", "author", "check", "check", "author", "check"])
        self.assertEqual([item["source"] for item in observed],
                         ["baseline\n", "baseline\n", "author round 1\n",
                          "author round 1\n", "author round 1\n", "author round 2\n"])
        wt = record["workspace"]["path"]
        for index, round_record in enumerate(record["rounds"]):
            red = round_record["red_check"]
            self.assertEqual(red["statuses"], {"failed": 1} if index == 0 else {"passed": 1})
            self.assertFalse(red["authoritative"])
            self.assertNotEqual(red["input_digest"], round_record["input_digest"])
            self.assertNotEqual(observed[index * 3]["cwd"], wt)
            self.assertEqual(observed[index * 3 + 1]["cwd"], wt)
            self.assertEqual(round_record["checks"][0]["status"], "passed")

    def test_red_baseline_runs_for_noop_and_failed_author_turns(self):
        for scenario in ("ok", "auth", "timeout"):
            with self.subTest(scenario=scenario):
                proc, record, evidence = self.develop(
                    "--scenario", scenario, "--prompt", "baseline " + scenario)
                self.assertEqual(proc.returncode, 0 if scenario == "ok" else 1, json.dumps(record))
                red = record["rounds"][0]["red_check"]
                self.assertEqual(red["statuses"], {"passed": 1})
                self.assertFalse(red["authoritative"])
                self.assertEqual(red["input_digest"], record["rounds"][0]["input_digest"])
                # Moving the baseline earlier must not drop the author record.
                for name in ("agent-result-1.json", "agent-result.json"):
                    self.assertTrue((evidence / name).is_file(), name)
                    self.assertEqual(json.loads((evidence / name).read_text())["role"], "author")
                self.assertEqual(record["checks"][0]["status"], "passed" if scenario == "ok" else "blocked")

    def test_red_baseline_survives_author_transport_exception(self):
        proc, record, _ = self.develop(env={"TEST_DEVELOPMENT_AGENT_EDIT": "raise"})
        self.assertEqual(proc.returncode, 1, json.dumps(record))
        self.assertEqual(record["rounds"][0]["red_check"]["statuses"], {"passed": 1})
        self.assertFalse(record["rounds"][0]["red_check"]["authoritative"])
        self.assertEqual(record["checks"][0]["reason"], "agent_failed")
        self.assertIn("agent_failed", [s["detail"] for s in record["steps"]])
        self.assertEqual(record["rounds"][0]["red_check"]["purpose"], "pre_edit_baseline")

    def test_red_check_writes_stay_in_copy_and_never_reach_author_input(self):
        events = self.root / ".scratch/author-events.jsonl"
        self.data = registry({"writes": check_entry(
            "from pathlib import Path; "
            "assert Path('source.txt').read_text() == 'baseline\\n'; "
            "Path('source.txt').write_text('check copy mutation\\n') "
            "if Path.cwd().name.startswith('workspace-red-') else None")})
        proc, record, _ = self.develop(
            env={"TEST_DEVELOPMENT_AGENT_EDIT": "observe", "TEST_DEVELOPMENT_EVENTS": str(events)})
        self.assertNotEqual(proc.returncode, 0, json.dumps(record))
        red = record["rounds"][0]["red_check"]
        self.assertEqual(red["statuses"], {"passed": 1})
        baseline_wt = Path(record["rounds"][0]["red_check_workspace"]["path"])
        self.assertEqual((baseline_wt / "source.txt").read_text(), "check copy mutation\n")
        observed = json.loads(events.read_text())
        self.assertEqual(observed["source"], "baseline\n")
        self.assertNotEqual(observed["cwd"], str(baseline_wt))
        self.assertEqual((self.root / "source.txt").read_text(), "baseline\n")
        self.assertFalse(any(c["status"] == "failed" for c in record["checks"]))

    def test_red_check_detects_absolute_author_mutation_before_author_call(self):
        events = self.root / ".scratch/author-events.jsonl"
        self.data = registry({"escape": check_entry(
            "from pathlib import Path; "
            "roots = Path(" + repr(str(self.root / ".scratch")) + ").glob('workspace-*'); "
            "author = next(path for path in roots if not path.name.startswith('workspace-red-')); "
            "(author / 'source.txt').write_text('escaped baseline write\\n')")})
        proc, record, evidence = self.develop(
            env={"TEST_DEVELOPMENT_AGENT_EDIT": "observe", "TEST_DEVELOPMENT_EVENTS": str(events)})
        self.assertEqual(proc.returncode, 1, json.dumps(record))
        self.assertIn("red_check_source_drift", [step["detail"] for step in record["steps"]])
        self.assertFalse(events.exists(), "author ran after baseline corrupted its input")
        self.assertFalse(record["checks"], "baseline checks leaked into final checks")
        self.assertFalse((evidence / "candidate.json").exists())
        self.assertEqual((self.root / "source.txt").read_text(), "baseline\n")

    def test_red_check_records_a_pre_edit_baseline_for_every_round(self):
        # The baseline is what distinguishes "checks ran" from "checks never ran".
        # It must exist before the edit and must be excluded from the round's
        # own result, which is the only thing that may certify the candidate.
        checks = {
            "current": check_entry("from pathlib import Path; "
                                   "assert Path('automation-mock-change.txt').exists()"),
        }
        self.data = registry(checks)
        proc, record, evidence = self.develop("--scenario", "edit", "--review-scenario", "revise-once")
        self.assertEqual(proc.returncode, 0, json.dumps(record))
        self.assertEqual(len(record["rounds"]), 2)
        for index, round_record in enumerate(record["rounds"], start=1):
            with self.subTest(round=index):
                red = round_record["red_check"]
                self.assertEqual(red["purpose"], "pre_edit_baseline")
                self.assertEqual(red["scope"], "disposable-copy")
                self.assertFalse(red["authoritative"])
                expected = {"failed": 1} if index == 1 else {"passed": 1}
                self.assertEqual(red["statuses"], expected)
                self.assertNotEqual(red["input_digest"], round_record["input_digest"])
        # The baseline is never part of what certifies the candidate.
        self.assertNotIn("red_check", json.loads((evidence / "candidate.json").read_text()))
        # The candidate is certified by post-edit results, never by the baseline.
        manifest = json.loads((evidence / "candidate.json").read_text())
        self.assertEqual(manifest["input_digest"], record["rounds"][-1]["input_digest"])

    def test_identical_input_on_unchanged_source_pauses_instead_of_rerunning(self):
        shared = ("--prompt", "one deliberate request")
        proc, record, evidence = self.develop(*shared)
        self.assertEqual(proc.returncode, 0, json.dumps(record))
        first = json.loads((evidence / "candidate.json").read_text())["run_id"]
        proc, record, evidence = self.develop(*shared)
        self.assertEqual(proc.returncode, 3)
        self.assertIn(f"input_duplicate_paused:{first}",
                      [s["detail"] for s in record["steps"]])
        self.assertNotIn("workspace", record)
        self.assertFalse((evidence / "candidate.json").exists())
        pause = json.loads((evidence / "pause.json").read_text())
        self.assertEqual(pause["run_id"], record["run_id"])
        self.assertFalse(pause["notification"]["delivered"])

    def test_max_rounds_and_timeout_rejected(self):
        record = self.assert_rejected("revise-always", "revision_limit_exhausted", "--max-rounds", "2")
        self.assertEqual(len(record["rounds"]), 2)
        self.assert_rejected("timeout", "review_timeout")

    def test_development_failure_uses_single_opt_in_pause_finalizer(self):
        import io
        for notify in (False, True):
            with self.subTest(notify=notify):
                args = pipeline.build_parser().parse_args(
                    ["--author-identity", "same", "--reviewer-identity", "same"]
                    + (["--notify-failure"] if notify else []))
                with patch.object(pipeline, "REPO", self.root), \
                        patch.object(pipeline, "EVIDENCE_ROOT", self.root / ".scratch/runs"), \
                        patch.object(pipeline.pause, "deliver", return_value={
                            "channel": "fixture", "delivered": False, "detail": "channel_unavailable",
                            "log": {"written": False, "detail": "log_write_failed"}}) as delivery, \
                        patch.object(pipeline.pause, "pause_record", wraps=pipeline.pause.pause_record) as write_pause, \
                        patch("sys.stdout", io.StringIO()):
                    rc = pipeline._run_development(args, self.data)
                self.assertEqual(rc, 1)
                self.assertEqual(write_pause.call_count, 1)
                self.assertEqual(delivery.call_count, int(notify))
                directory = write_pause.call_args.args[0]
                receipt = json.loads((directory / "pause.json").read_text())
                self.assertEqual(receipt["stage"], "development")
                self.assertEqual(receipt["notification"]["detail"],
                                 "channel_unavailable" if notify else "not_requested")

    def test_same_identity_rejected_without_workspace(self):
        proc, record, evidence = self.develop("--author-identity", "same", "--reviewer-identity", "same")
        self.assertEqual(proc.returncode, 1)
        self.assertNotIn("workspace", record)
        self.assertFalse((evidence / "candidate.json").exists())

    def test_arbitrary_preauth_never_grants_real_driver(self):
        for flag in ("--driver", "--review-driver"):
            proc, record, evidence = self.develop(flag, "agy", "--preauth", "arbitrary-synthetic-string")
            self.assertEqual(proc.returncode, 3)
            self.assertNotIn("workspace", record)
            self.assertFalse((evidence / "agent-result.json").exists())
            self.assertIn("trusted_authorization_policy_unconfigured",
                          [s["detail"] for s in record["steps"]])
            self.assertNotIn("arbitrary-synthetic-string", (evidence / "run.json").read_text())

    def test_check_failure_blocks_review_and_candidate(self):
        self.data = registry({"fail": check_entry("raise SystemExit(1)")})
        proc, record, evidence = self.develop("--scenario", "edit")
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(record["rounds"][0]["review_status"], "blocked_by_checks")
        self.assertFalse((evidence / "candidate.json").exists())

    def test_check_source_drift_rejected(self):
        self.data = registry({"drift": check_entry(
            "from pathlib import Path; Path('source.txt').write_text('changed')")})
        proc, record, evidence = self.develop("--scenario", "edit")
        self.assertEqual(proc.returncode, 1)
        self.assertIn("source_drift", [s["detail"] for s in record["steps"]])
        self.assertFalse((evidence / "candidate.json").exists())

    def test_origin_drift_rejected_before_candidate(self):
        code = "from pathlib import Path; Path(" + repr(str(self.root / "source.txt")) + ").write_text('concurrent origin edit')"
        self.data = registry({"origin": check_entry(code)})
        proc, record, evidence = self.develop("--scenario", "edit")
        self.assertEqual(proc.returncode, 1)
        self.assertIn("origin_source_drift", [s["detail"] for s in record["steps"]])
        self.assertFalse((evidence / "agent-result.json").exists(),
                         "author ran after baseline changed the origin source")
        self.assertFalse(record["checks"], "diagnostic baseline leaked into final checks")
        self.assertFalse((evidence / "candidate.json").exists())

    def test_unsafe_name_and_symlink_escape_rejected_without_reading(self):
        self.write(".env", "synthetic excluded material")
        proc, record, _ = self.develop()
        self.assertEqual(proc.returncode, 1)
        self.assertNotIn("workspace", record)
        (self.root / ".env").unlink()
        (self.root / "escape").symlink_to("/tmp")
        proc, record, _ = self.develop()
        self.assertEqual(proc.returncode, 1)
        self.assertNotIn("workspace", record)

    def test_snapshot_bypasses_filters_and_checkout_hooks(self):
        marker = self.root / ".scratch/filter-ran"
        self.git("config", "filter.fixture.smudge", "touch " + str(marker))
        self.git("config", "filter.fixture.clean", "touch " + str(marker))
        self.write(".gitattributes", "source.txt filter=fixture\n")
        hook = self.write(".git/hooks/post-checkout", "#!/bin/sh\ntouch " + str(marker) + "\n")
        hook.chmod(0o755)
        self.data = registry({"current": check_entry(
            "from pathlib import Path; assert Path('source.txt').read_text() == 'baseline\\n'")})
        proc, record, _ = self.develop("--scenario", "edit")
        self.assertEqual(proc.returncode, 0, json.dumps(record))
        self.assertFalse(marker.exists())

    def test_review_schema_strict_and_late_run_rejected(self):
        payload = {"schema_version": 1, "verdict": "pass", "input_digest": "digest", "run_id": "run",
                   "round": 2, "reviewer": "reviewer", "reason": "synthetic"}
        self.assertEqual(pipeline.validate_review(payload, "digest", "run", 2, "author", "reviewer"), "pass")
        for changed in ({"status": "OK"}, {**payload, "run_id": "older"}, {**payload, "round": 1},
                        {**payload, "verdict": "OK"}, {**payload, "unexpected": True},
                        {**payload, "schema_version": True}, {**payload, "round": True}):
            with self.assertRaises(pipeline.PipelineError):
                pipeline.validate_review(changed, "digest", "run", 2, "author", "reviewer")

    def test_candidate_evidence_mismatch_and_patch_limit_fail_closed(self):
        self.write("huge.txt", "x" * (2 * 1024 * 1024 + 100))
        base = self.git("rev-parse", "HEAD").stdout.strip()
        revision = pipeline.snapshot_commit(self.root, base)
        digest = pipeline.worktree_fingerprint_safe(self.root)
        evidence = self.root / ".scratch/direct-candidate"
        evidence.mkdir(parents=True)
        record = {"run_id": "direct", "workspace": {"base_head": base}, "author": "author",
                  "reviewer": "reviewer", "requirements": ["AUTO-01"]}
        round_record = {"round": 1, "checks": [{"status": "passed"}], "checks_digest": digest,
                        "review": {"verdict": "pass", "input_digest": digest}}
        with self.assertRaisesRegex(pipeline.PipelineError, "too_large"):
            pipeline.local_candidate(evidence, record, self.root, base, revision, digest, round_record)
        self.assertFalse((evidence / "candidate.patch").exists())
        self.assertFalse((evidence / "candidate.json").exists())
        round_record["checks_digest"] = "stale"
        with self.assertRaisesRegex(pipeline.PipelineError, "evidence_mismatch"):
            pipeline.local_candidate(evidence, record, self.root, base, revision, digest, round_record)

    def test_two_ablation_controls_prove_protection_rejects_wrong_results(self):
        payload = {"schema_version": 1, "verdict": "pass", "input_digest": "old", "run_id": "run",
                   "round": 1, "reviewer": "reviewer", "reason": "synthetic"}

        def review_guard():
            return pipeline.validate_review(payload, "current", "run", 1, "author", "reviewer")

        with self.assertRaises(pipeline.PipelineError):
            review_guard()
        with patch.object(pipeline, "validate_review", return_value="pass"):
            self.assertEqual(review_guard(), "pass")

        def fingerprint_guard():
            return pipeline.require_same_digest("original", "tampered", "review_source_modified")

        with self.assertRaises(pipeline.PipelineError):
            fingerprint_guard()
        with patch.object(pipeline, "require_same_digest", return_value=None):
            self.assertIsNone(fingerprint_guard())


if __name__ == "__main__":
    unittest.main(verbosity=2)
