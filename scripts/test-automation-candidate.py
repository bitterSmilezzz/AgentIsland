#!/usr/bin/env python3
"""Consumer-side candidate verification meta-tests: read-only, synthetic fixtures only.

Run: python3 scripts/test-automation-candidate.py
The candidate under test is produced by the REAL pipeline (mock driver, mock review)
inside a disposable temporary git repository, then copied and mutated per case.
No product build, native gate, service, credential, network or real agent is used.
The CLI under test never applies, merges, pushes, or starts a model: it only reads.
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
CLI = AUTOMATION / "candidate.py"
PY = sys.executable or "python3"
HOST = "windows" if sys.platform == "win32" else sys.platform

# Re-root the real pipeline CLI inside the fixture: argparse -> run_pipeline ->
# development_rounds -> local_candidate all run through production code.
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
    root = Path(tempfile.mkdtemp(prefix="automation-candidate-root-"))

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


def produce_candidate(root: Path) -> Path:
    """Run the real mock pipeline once and return its evidence directory."""
    proc = subprocess.run([PY, "-c", BOOTSTRAP, str(AUTOMATION), str(root), "--scenario", "edit"],
                          capture_output=True, text=True, cwd=root,
                          env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}, timeout=300)
    if proc.returncode != 0:
        raise RuntimeError(f"fixture pipeline failed rc={proc.returncode}: {proc.stderr[-400:]}")
    summary = json.loads(proc.stdout)
    evidence = Path(summary["evidence"])
    manifest = json.loads((evidence / "candidate.json").read_text(encoding="utf-8"))
    if manifest["kind"] != "local-only-candidate" or manifest["review_kind"] != "deterministic-mock":
        raise RuntimeError("fixture did not produce a mock local candidate")
    return evidence


class CandidateVerifyTests(unittest.TestCase):
    """One real pipeline run per class; every case mutates an isolated copy."""

    @classmethod
    def setUpClass(cls):
        cls.fixture_root = build_fixture_root()
        cls.fixture_evidence = produce_candidate(cls.fixture_root)

    @classmethod
    def tearDownClass(cls):
        subprocess.run(["git", "-C", str(cls.fixture_root), "worktree", "prune"],
                       capture_output=True, text=True)
        shutil.rmtree(cls.fixture_root, ignore_errors=True)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="automation-candidate-")
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        self.run_dir = self.work / "runs" / self.fixture_evidence.name
        shutil.copytree(self.fixture_evidence, self.run_dir)

    def restore(self):
        """Reset to the pristine produced candidate so mutations never accumulate."""
        shutil.rmtree(self.run_dir)
        shutil.copytree(self.fixture_evidence, self.run_dir)

    # ---------------------------------------------------------------- helpers

    def clone_root(self) -> Path:
        """A private, mutable copy of the repository that produced the candidate."""
        destination = self.work / "repo"
        proc = subprocess.run(["git", "clone", "--local", "--quiet", str(self.fixture_root),
                               str(destination)], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return destination

    def cli(self, *args, root=None):
        command = [PY, str(CLI), *args]
        if root is not None:
            command += ["--root", str(root)]
        return subprocess.run(command, capture_output=True, text=True, timeout=60)

    def verify(self, *, run_dir=None, root=None):
        proc = self.cli("verify", "--run-dir", str(run_dir or self.run_dir), root=root)
        self.assertEqual(proc.stderr, "", proc.stderr)
        payload = json.loads(proc.stdout)
        # Output is a fixed vocabulary: no path, no file body, no operator text.
        # A rejected run claims nothing beyond its code: scope and review_kind are
        # claims about evidence that was not accepted.
        expected = ({"ok", "code", "scope", "review_kind", "independent_review", "run_id"}
                    if payload.get("ok") else {"ok", "code"})
        self.assertEqual(sorted(payload), sorted(expected), payload)
        self.assertNotIn(str(self.work), proc.stdout)
        self.assertNotIn(str(self.fixture_root), proc.stdout)
        return proc.returncode, payload

    def assert_verified(self, root=None, scope="evidence-only"):
        rc, payload = self.verify(root=root)
        self.assertEqual(rc, 0, payload)
        self.assertEqual(payload, {"ok": True, "code": "ok", "scope": scope,
                                   "review_kind": "deterministic-mock",
                                   "independent_review": False,
                                   "run_id": json.loads((self.run_dir / "candidate.json").read_text(
                                       encoding="utf-8"))["run_id"]})
        return payload

    def assert_rejected(self, code, rc=1, *, root=None, run_dir=None):
        actual_rc, payload = self.verify(root=root, run_dir=run_dir)
        self.assertEqual(actual_rc, rc, payload)
        self.assertEqual(payload["ok"], False)
        self.assertEqual(payload["code"], code, payload)
        return payload

    def manifest(self):
        return json.loads((self.run_dir / "candidate.json").read_text(encoding="utf-8"))

    def record(self):
        return json.loads((self.run_dir / "run.json").read_text(encoding="utf-8"))

    def rewrite(self, manifest=None, record=None, raw=None):
        """Persist a manifest/run edit, like a same-user rewrite of the evidence."""
        if manifest is not None:
            self.rewrite_json(self.run_dir / "candidate.json", manifest)
        if record is not None:
            self.rewrite_json(self.run_dir / "run.json", record)
        if raw is not None:
            for name, payload in raw.items():
                (self.run_dir / name).write_text(payload, encoding="utf-8")

    def rewrite_json(self, path, payload):
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def consistent_rewrite(self, mutate):
        """Edit candidate.json AND the run record's embedded candidate together.

        This is the honest threat model: a same-user writer who updates every
        digest consistently. Internal consistency alone cannot catch that, so the
        fixture also proves which checks still do.
        """
        manifest = self.manifest()
        mutate(manifest)
        record = self.record()
        record["candidate"] = manifest
        self.rewrite(manifest=manifest, record=record)
        return manifest

    def tamper_patch(self, mutate):
        """Rewrite the patch AND its recorded size/digest AND the run record."""
        patch = self.run_dir / "candidate.patch"
        patch.write_bytes(mutate(patch.read_bytes()))
        data = patch.read_bytes()
        self.consistent_rewrite(lambda manifest: manifest.update(
            patch_bytes=len(data), patch_sha256=hashlib.sha256(data).hexdigest()))

    # ---------------------------------------------------------------- cases

    def test_valid_evidence_only_scope_and_mock_review_not_attested(self):
        self.assert_verified(scope="evidence-only")

    def test_valid_with_root_verifies_git_objects_and_bytes(self):
        self.assert_verified(root=self.clone_root(), scope="evidence+git")

    def test_missing_run_dir_is_blocked_not_a_failure(self):
        self.assert_rejected("run_dir_missing", rc=3, run_dir=self.work / "absent")

    def test_invalid_arguments_exit_two(self):
        # A bad flag still answers with the fixed code vocabulary on stdout, so a
        # consumer parses one shape either way. No usage text: it would echo paths.
        for args in (["verify"], ["verify", "--run-dir"], ["inspect", "--run-dir", str(self.run_dir)],
                     ["verify", "--run-dir", str(self.run_dir), "--unknown", "1"]):
            with self.subTest(args=args):
                proc = self.cli(*args)
                self.assertEqual(proc.returncode, 2, proc.stdout + proc.stderr)
                self.assertEqual(json.loads(proc.stdout), {"ok": False, "code": "invalid_argument"})
                self.assertEqual(proc.stderr, "")
                self.assertNotIn(str(self.work), proc.stdout)

    def test_root_without_repository_is_blocked(self):
        self.assert_rejected("root_not_git_repository", rc=3, root=self.work)
        self.assert_rejected("root_missing", rc=3, root=self.work / "absent")

    def test_dot_and_relative_paths_are_accepted_but_dotdot_is_not(self):
        # "." and relative paths are ordinary ways to call this; ".." is refused
        # rather than silently normalized to somewhere the caller did not name.
        root = self.clone_root()
        proc = subprocess.run([PY, str(CLI), "verify", "--run-dir", str(self.run_dir),
                               "--root", "."], cwd=root, capture_output=True, text=True)
        self.assertEqual(json.loads(proc.stdout)["ok"], True, proc.stdout)
        proc = subprocess.run([PY, str(CLI), "verify", "--run-dir",
                               os.path.relpath(self.run_dir, self.work), "--root", str(root)],
                              cwd=self.work, capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        for value in ("..", str(root / ".." / root.name), Path(".") / ".." / root.name):
            with self.subTest(value=value):
                rc, payload = self.verify(root=value)
                self.assertEqual((rc, payload["code"]), (2, "invalid_argument"))

    def test_failed_run_cannot_publish_a_candidate(self):
        record = self.record()
        record.update(outcome="failed", returncode=1, failed_or_blocked=["ok"])
        record.pop("candidate")
        self.rewrite(record=record)
        self.assert_rejected("run_not_passed")

    def test_revoked_manifest_beside_candidate_is_inconsistent(self):
        invalid = dict(self.manifest(), valid=False, invalidated_by="failed")
        self.rewrite_json(self.run_dir / "candidate-invalid.json", invalid)
        self.assert_rejected("revoked_candidate_present")

    def test_candidate_json_must_equal_run_record_candidate(self):
        manifest = self.manifest()
        manifest["requirements"] = ["AUTO-01", "AUTO-02"]
        self.rewrite(manifest=manifest)
        self.assert_rejected("candidate_run_mismatch")

    def test_foreign_run_record_is_rejected_by_identity(self):
        record = self.record()
        record["run_id"] = "20200101T000000Z-000000000000"
        record["candidate"] = dict(record["candidate"], run_id=record["run_id"])
        self.rewrite(record=record)
        self.assert_rejected("run_binding_mismatch")

    def test_round_must_be_the_final_round(self):
        self.consistent_rewrite(lambda manifest: manifest.update(round=2))
        self.assert_rejected("round_binding_mismatch")

    def test_stale_review_digest_is_rejected(self):
        self.consistent_rewrite(lambda manifest: manifest.update(review_digest="0" * 64))
        self.assert_rejected("review_binding_stale")

    def test_non_pass_verdict_is_rejected(self):
        record = self.record()
        record["rounds"][-1]["review"]["verdict"] = "revise"
        self.rewrite(record=record)
        self.assert_rejected("review_verdict_invalid")

    def test_same_author_and_reviewer_identity_is_rejected(self):
        self.consistent_rewrite(lambda manifest: manifest.update(author="mock-reviewer"))
        record = self.record()
        record.update(author="mock-reviewer")
        record["rounds"][-1]["author"] = "mock-reviewer"
        self.rewrite(record=record)
        self.assert_rejected("identity_conflict")

    def test_baseline_binding_to_the_planned_run_is_required(self):
        # The manifest and the run's plan_baselines are one binding: rewriting both
        # is internally consistent, disagreeing with the run is not.
        self.consistent_rewrite(lambda manifest: manifest["baselines"].update(requirements="f" * 64))
        record = self.record()
        record["plan_baselines"] = dict(record["plan_baselines"], requirements="e" * 64)
        self.rewrite(record=record)
        self.assert_rejected("baseline_binding_mismatch")
        self.restore()
        self.consistent_rewrite(lambda manifest: manifest["baselines"].update(requirements="f" * 64))
        record = self.record()
        record["plan_baselines"] = dict(record["plan_baselines"], requirements="f" * 64)
        self.rewrite(record=record)
        self.assert_verified(scope="evidence-only")

    def test_input_binding_must_match_the_run(self):
        self.consistent_rewrite(lambda manifest: manifest["input_binding"].update(prompt_sha256="a" * 64))
        self.assert_rejected("input_binding_mismatch")

    def test_tampered_patch_bytes_are_rejected(self):
        patch = self.run_dir / "candidate.patch"
        patch.write_bytes(patch.read_bytes() + b"+injected line\n")
        self.assert_rejected("patch_digest_mismatch")

    def test_consistent_patch_and_manifest_rewrite_needs_root_to_detect(self):
        # Honest limit: evidence-only scope checks internal consistency, so a rewrite
        # of patch + digests + run record is self-consistent. The git diff bytes are
        # the independent anchor, and only --root can compare against them.
        self.tamper_patch(lambda data: b"+++ synthetic forged diff\n")
        self.assert_verified(scope="evidence-only")
        self.assert_rejected("patch_git_mismatch", root=self.clone_root())

    def test_missing_git_objects_are_rejected_with_root(self):
        empty = self.work / "empty-repo"
        empty.mkdir()
        subprocess.run(["git", "init", "-q", str(empty)], check=True, capture_output=True)
        self.assert_rejected("candidate_commit_missing", root=empty)

    def test_root_baseline_drift_is_rejected(self):
        root = self.clone_root()
        (root / "docs/需求.md").write_text("# Changed after the run\n", encoding="utf-8")
        self.assert_rejected("source_baseline_drift", root=root)

    def test_symlinked_evidence_is_rejected(self):
        patch = self.run_dir / "candidate.patch"
        payload = patch.read_bytes()
        patch.unlink()
        patch.symlink_to(self.work / "elsewhere.patch")
        (self.work / "elsewhere.patch").write_bytes(payload)
        self.assert_rejected("evidence_symlink_rejected")
        self.restore()
        record = self.run_dir / "run.json"
        payload = record.read_bytes()
        record.unlink()
        record.symlink_to(self.work / "elsewhere.json")
        (self.work / "elsewhere.json").write_bytes(payload)
        self.assert_rejected("evidence_symlink_rejected")
        self.restore()
        manifest = self.run_dir / "candidate.json"
        payload = manifest.read_bytes()
        manifest.unlink()
        manifest.symlink_to(self.work / "elsewhere-candidate.json")
        (self.work / "elsewhere-candidate.json").write_bytes(payload)
        self.assert_rejected("evidence_symlink_rejected")

    def test_unknown_schema_and_extra_fields_are_rejected(self):
        self.consistent_rewrite(lambda manifest: manifest.update(schema_version=2))
        self.assert_rejected("candidate_schema_unsupported")
        self.restore()
        self.consistent_rewrite(lambda manifest: manifest.update(kind="signed-candidate"))
        self.assert_rejected("candidate_kind_unknown")
        self.restore()
        # An added field could otherwise ride along as an unattested claim, e.g. a
        # self-declared signature this tool cannot and does not verify.
        self.consistent_rewrite(lambda manifest: manifest.update(
            signature="not-a-real-cryptographic-attestation"))
        self.assert_rejected("candidate_schema_unsupported")

    def test_truncated_and_duplicate_key_json_are_rejected(self):
        raw = (self.run_dir / "candidate.json").read_text(encoding="utf-8")
        (self.run_dir / "candidate.json").write_text(raw[: len(raw) // 2], encoding="utf-8")
        self.assert_rejected("evidence_truncated")
        # A duplicate key is a rewrite hiding behind two values for one field; the
        # last-one-wins default must not be accepted here.
        self.restore()
        manifest = json.dumps(self.manifest(), ensure_ascii=False)
        duplicate = '{"schema_version": 1, "schema_version": 1, "kind": "local-only-candidate"' \
            + manifest[manifest.index(', "kind"') + len(', "kind": "local-only-candidate"'):]
        self.assertEqual(json.loads(duplicate)["schema_version"], 1)  # valid, one value hidden
        self.rewrite(raw={"candidate.json": duplicate})
        self.assert_rejected("evidence_duplicate_key")

    def test_oversized_patch_is_rejected_before_hashing(self):
        (self.run_dir / "candidate.patch").write_bytes(b"x" * (2 * 1024 * 1024 + 1))
        self.assert_rejected("evidence_oversize")

    def test_verification_is_read_only(self):
        root = self.clone_root()
        before = {p.name: p.read_bytes() for p in sorted(self.run_dir.iterdir())}
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True,
                              text=True).stdout
        self.assert_verified(root=root, scope="evidence+git")
        self.assertEqual({p.name: p.read_bytes() for p in sorted(self.run_dir.iterdir())}, before)
        self.assertEqual(subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True,
                                        text=True).stdout, head)
        status = subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True,
                                text=True).stdout
        self.assertEqual(status, "")


if __name__ == "__main__":
    unittest.main(verbosity=2)
