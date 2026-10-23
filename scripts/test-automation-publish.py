#!/usr/bin/env python3
"""Merge-gate and rollback meta-tests over the REAL pipeline's own candidate.

Run: python3 scripts/test-automation-publish.py
The candidate under test is produced by the real pipeline (mock driver, mock
review) inside a disposable temporary git repository — the same fixture family
as test-automation-candidate.py. No product build, network, service, credential
or real agent is used. The CLI under test acts only inside that disposable
clone; every refusal case asserts that the repository was left untouched.

Refusal-matrix note (门禁证明能拒错): the CI-binding cases below double as the
mutation control — a gate that accepted a pending or foreign-SHA report fails
`test_merge_refuses_pending_ci` / `test_merge_refuses_foreign_sha` immediately,
and a gate that merged twice fails `test_merge_refuses_second_merge`.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
AUTOMATION = HERE / "automation"
CLI = AUTOMATION / "publish.py"
PY = sys.executable or "python3"
HOST = "windows" if sys.platform == "win32" else sys.platform

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
sys.argv = ['pipeline', *sys.argv[3:]]
raise SystemExit(pipeline.main())
"""


def check_entry(code="pass", **extra):
    return {"command": [PY, "-c", code], "host_os": [HOST], **extra}


def registry():
    checks = {"ok": check_entry()}
    return {"schema_version": 1, "gates": {"L0": {"required": True}},
            "requirements": {"AUTO-01": {"checks": list(checks)}},
            "checks": checks, "profiles": {"test": list(checks), "note": "synthetic"},
            "agents": {"default": "mock", "available": ["mock", "agy"]}}


def build_fixture_root() -> Path:
    root = Path(tempfile.mkdtemp(prefix="automation-publish-root-"))

    def write(name, content):
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def git(*args):
        proc = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)
        if proc.returncode:
            raise RuntimeError("fixture git failed: " + proc.stderr)

    write(".gitignore", ".scratch/\ntarget/\n__pycache__/\n")
    write("source.txt", "baseline\n")
    write("docs/需求.md", "# Synthetic requirements\nAUTO-01: baseline check\n")
    write("docs/需求测试矩阵.md", "# Synthetic matrix\nAUTO-01 -> ok\n")
    write("scripts/automation/registry.json", json.dumps(registry()))
    git("init", "-q")
    git("add", "--", ".gitignore", "source.txt", "docs/需求.md", "docs/需求测试矩阵.md",
        "scripts/automation/registry.json")
    git("-c", "user.name=Candidate Test", "-c", "user.email=candidate@example.invalid",
        "commit", "-qm", "fixture")
    return root


def produce_candidate(root: Path) -> tuple[Path, str]:
    """Run the real mock pipeline once; return its evidence dir and base head."""
    proc = subprocess.run([PY, "-c", BOOTSTRAP, str(AUTOMATION), str(root), "--scenario", "edit"],
                          capture_output=True, text=True, cwd=root,
                          env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}, timeout=300)
    if proc.returncode != 0:
        raise RuntimeError(f"fixture pipeline failed rc={proc.returncode}: {proc.stderr[-400:]}")
    summary = json.loads(proc.stdout)
    evidence = Path(summary["evidence"])
    manifest = json.loads((evidence / "candidate.json").read_text(encoding="utf-8"))
    if manifest["kind"] != "local-only-candidate":
        raise RuntimeError("fixture did not produce a local candidate")
    return evidence, manifest["base_head"]


def ci_report(directory: Path, commit: str, state: str = "success") -> Path:
    path = directory / f"ci-{state}.json"
    path.write_text(json.dumps({
        "schema_version": 1, "commit": commit, "state": state,
        "provider": "github-actions", "url": "https://ci.invalid/run/1",
    }), encoding="utf-8")
    return path


class PublishGateTests(unittest.TestCase):
    """One real pipeline run per class; every case mutates an isolated clone."""

    @classmethod
    def setUpClass(cls):
        cls.fixture_root = build_fixture_root()
        cls.fixture_evidence, cls.base_head = produce_candidate(cls.fixture_root)
        cls.manifest = json.loads(
            (cls.fixture_evidence / "candidate.json").read_text(encoding="utf-8"))

    @classmethod
    def tearDownClass(cls):
        subprocess.run(["git", "-C", str(cls.fixture_root), "worktree", "prune"],
                       capture_output=True, text=True)
        shutil.rmtree(cls.fixture_root, ignore_errors=True)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="automation-publish-")
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        self.run_dir = self.work / "runs" / self.fixture_evidence.name
        shutil.copytree(self.fixture_evidence, self.run_dir)
        self.repo = self.clone_root()

    def clone_root(self) -> Path:
        destination = self.work / "repo"
        proc = subprocess.run(["git", "clone", "--local", "--quiet",
                               str(self.fixture_root), str(destination)],
                              capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        # The gate merges into whatever HEAD is; the fixture pins a branch at
        # the reviewed base so every case starts from the same state.
        proc = subprocess.run(["git", "checkout", "-q", "-B", "automation-merge",
                               self.base_head], cwd=destination,
                              capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        subprocess.run(["git", "config", "user.name", "Merge Test"], cwd=destination,
                       capture_output=True, text=True)
        subprocess.run(["git", "config", "user.email", "merge@example.invalid"],
                       cwd=destination, capture_output=True, text=True)
        return destination

    def git(self, *args):
        proc = subprocess.run(["git", *args], cwd=self.repo,
                              capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return proc.stdout

    def cli(self, *args):
        return subprocess.run([PY, str(CLI), *args], capture_output=True,
                              text=True, timeout=60)

    def merge(self, *, ci="success", root=None, run_dir=None, extra=()):
        report = None
        if ci is not None:
            report = ci_report(self.work, self.manifest["candidate_commit"], state=ci)
        return self.cli("merge", "--run-dir", str(run_dir or self.run_dir),
                        "--root", str(root or self.repo),
                        "--ci-json", str(report or self.work / "absent.json"), *extra)

    def payload(self, proc):
        self.assertEqual(proc.stderr, "", proc.stderr)
        parsed = json.loads(proc.stdout)
        self.assertNotIn(str(self.work), proc.stdout)
        self.assertNotIn(str(self.fixture_root), proc.stdout)
        return parsed

    def assert_refused(self, proc, code, rc=1):
        self.assertEqual(proc.returncode, rc, proc.stdout)
        payload = self.payload(proc)
        self.assertEqual(payload["ok"], False)
        self.assertEqual(payload["code"], code, payload)
        return payload

    def head(self):
        return self.git("rev-parse", "HEAD").strip()

    # ------------------------------------------------------------------ merge

    def test_merge_requires_the_full_chain_and_records_it(self):
        proc = self.merge()
        self.assertEqual(proc.returncode, 0, proc.stdout)
        payload = self.payload(proc)
        self.assertTrue(payload["ok"])
        self.assertNotEqual(payload["merged_head"], self.base_head)
        record = json.loads((self.run_dir / "publish.json").read_text(encoding="utf-8"))
        self.assertEqual(
            sorted(record),
            sorted(["schema_version", "kind", "run_id", "round", "candidate_commit",
                    "base_head", "pre_merge_head", "merged_head", "ci_report_sha256",
                    "ci_state", "ci_provider", "require_independent_review",
                    "review_kind", "independent_review", "publication"]))
        self.assertEqual(record["kind"], "merge")
        self.assertEqual(record["publication"], "merged-local")
        self.assertEqual(record["ci_state"], "success")
        self.assertEqual(record["pre_merge_head"], self.base_head)
        self.assertEqual(record["merged_head"], payload["merged_head"])
        report_bytes = (self.work / "ci-success.json").read_bytes()
        self.assertEqual(record["ci_report_sha256"],
                         hashlib.sha256(report_bytes).hexdigest())
        # The merge carried exactly the reviewed patch, nothing else.
        post = self.git("diff", "--binary", "--no-ext-diff", "--no-textconv",
                        self.base_head, "HEAD", "--")
        patch = (self.run_dir / "candidate.patch").read_bytes()
        self.assertEqual(post.encode(), patch)

    def test_merge_refuses_pending_ci(self):
        before = self.head()
        for state in ("pending", "failure", "missing"):
            with self.subTest(state=state):
                proc = self.merge(ci=state)
                payload = self.assert_refused(proc, "ci_state_refused")
                self.assertEqual(payload["ci_state"], state)
        self.assertEqual(self.head(), before, "被拒的合并不得动仓库")
        self.assertFalse((self.run_dir / "publish.json").exists())

    def test_merge_refuses_foreign_sha(self):
        before = self.head()
        report = self.work / "ci-foreign.json"
        report.write_text(json.dumps({
            "schema_version": 1, "commit": "f" * 40, "state": "success",
            "provider": "github-actions", "url": "https://ci.invalid/run/2",
        }), encoding="utf-8")
        proc = self.cli("merge", "--run-dir", str(self.run_dir), "--root", str(self.repo),
                        "--ci-json", str(report))
        self.assert_refused(proc, "ci_commit_mismatch")
        self.assertEqual(self.head(), before)

    def test_merge_refuses_broken_ci_report(self):
        before = self.head()
        missing = self.cli("merge", "--run-dir", str(self.run_dir), "--root", str(self.repo),
                           "--ci-json", str(self.work / "nope.json"))
        self.assert_refused(missing, "ci_report_missing", rc=3)
        broken = self.work / "ci-broken.json"
        broken.write_text(json.dumps({
            "schema_version": 1, "commit": self.manifest["candidate_commit"],
            "state": "green", "provider": "github-actions", "url": "https://x",
            "extra": "field",
        }), encoding="utf-8")
        proc = self.cli("merge", "--run-dir", str(self.run_dir), "--root", str(self.repo),
                        "--ci-json", str(broken))
        self.assert_refused(proc, "ci_report_schema_invalid")
        self.assertEqual(self.head(), before)

    def test_merge_refuses_drifted_head_and_leaves_it_alone(self):
        self.git("commit", "--allow-empty", "-qm", "drift after review")
        drifted = self.head()
        proc = self.merge()
        self.assert_refused(proc, "head_drifted")
        self.assertEqual(self.head(), drifted)
        self.assertFalse((self.run_dir / "publish.json").exists())

    def test_merge_refuses_dirty_worktree(self):
        (self.repo / "source.txt").write_text("dirty\n", encoding="utf-8")
        before = self.head()
        proc = self.merge()
        self.assert_refused(proc, "worktree_dirty")
        self.assertEqual(self.head(), before)

    def test_merge_refuses_second_merge(self):
        self.assertEqual(self.merge().returncode, 0)
        self.assert_refused(self.merge(), "already_merged")
        self.assertEqual(self.head(), json.loads(
            (self.run_dir / "publish.json").read_text(encoding="utf-8"))["merged_head"])

    def test_merge_can_require_independent_review(self):
        proc = self.merge(extra=("--require-independent-review",))
        self.assert_refused(proc, "independent_review_required")
        self.assertFalse((self.run_dir / "publish.json").exists())

    # --------------------------------------------------------------- rollback

    def test_rollback_restores_the_reviewed_base_exactly(self):
        self.assertEqual(self.merge().returncode, 0)
        record = json.loads((self.run_dir / "publish.json").read_text(encoding="utf-8"))
        proc = self.cli("rollback", "--run-dir", str(self.run_dir), "--root", str(self.repo))
        self.assertEqual(proc.returncode, 0, proc.stdout)
        payload = self.payload(proc)
        self.assertEqual(payload["restored_head"], self.base_head)
        self.assertEqual(self.head(), self.base_head)
        done = json.loads((self.run_dir / "rollback.json").read_text(encoding="utf-8"))
        self.assertEqual(done["kind"], "rollback")
        self.assertEqual(done["from_head"], record["merged_head"])
        self.assertEqual(done["restored_head"], self.base_head)
        # The tree is byte-identical to the reviewed base again.
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assert_refused(self.cli("rollback", "--run-dir", str(self.run_dir),
                                     "--root", str(self.repo)), "already_rolled_back")

    def test_rollback_refuses_without_a_merge(self):
        self.assert_refused(self.cli("rollback", "--run-dir", str(self.run_dir),
                                     "--root", str(self.repo)), "rollback_without_merge")

    def test_rollback_refuses_drifted_head(self):
        self.assertEqual(self.merge().returncode, 0)
        self.git("commit", "--allow-empty", "-qm", "work after merge")
        drifted = self.head()
        proc = self.cli("rollback", "--run-dir", str(self.run_dir), "--root", str(self.repo))
        self.assert_refused(proc, "rollback_head_mismatch")
        self.assertEqual(self.head(), drifted)
        self.assertFalse((self.run_dir / "rollback.json").exists())

    def test_rollback_refuses_a_tampered_merge_record(self):
        self.assertEqual(self.merge().returncode, 0)
        record = json.loads((self.run_dir / "publish.json").read_text(encoding="utf-8"))
        record.pop("merged_head")
        (self.run_dir / "publish.json").write_text(json.dumps(record), encoding="utf-8")
        proc = self.cli("rollback", "--run-dir", str(self.run_dir), "--root", str(self.repo))
        self.assert_refused(proc, "publish_record_invalid")
        self.assertFalse((self.run_dir / "rollback.json").exists())

    # ------------------------------------------------------------- boundaries

    def test_missing_run_dir_is_nothing_to_act_on(self):
        proc = self.merge(run_dir=self.work / "absent")
        self.assert_refused(proc, "run_dir_missing", rc=3)


if __name__ == "__main__":
    unittest.main()
