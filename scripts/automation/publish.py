#!/usr/bin/env python3
"""Authorized local merge gate and rollback for a verified candidate.

AUTO-01's chain ends in "授权内合并/发布→回滚". `candidate.py` only verifies
evidence; nothing could act on it. This CLI is the explicit operator step:

- `merge --run-dir D --root R --ci-json F` merges the candidate commit into R
  after (a) full evidence+git verification via `candidate.verify`, (b) the
  target HEAD still equals the reviewed base, (c) the worktree is clean, and
  (d) a CI report **bound to exactly the candidate commit** shows `success`.
  A pending, failing, missing or foreign-SHA report never merges.
- `rollback --run-dir D --root R` restores R to the recorded pre-merge HEAD
  with the same refusal guards (clean tree, HEAD == recorded merge head).

Authorization boundary, stated rather than implied: this tool never starts by
itself, never pushes, never releases, and never decides *policy*. The
operator's explicit invocation IS the authorization; the gate enforces only
evidence shape, SHA binding and CI state. Review quality is reported
(`review_kind`, `independent_review`) and optionally required
(`--require-independent-review`); the default follows the current framework
policy, whose review driver is a deterministic mock by design.

The CI report is a **binding**, not a fetch: the gate cannot query CI offline,
and a fetched report would be operator-trustable all the same. What the gate
guarantees is that whatever report it accepted certifies *this exact commit*
and nothing else, and that its bytes are digested into the merge record.

Exit codes: 0 merged/rolled back, 1 refused (fixed code), 2 invalid arguments,
3 nothing to act on (missing evidence/repo). Output is one JSON object with
fixed keys and fixed codes only: no path, no file body, no operator text.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

import candidate  # noqa: E402  (same package; reuses its strict readers)
from candidate import VerifyError, _load_json, _read_file, _is_hex  # noqa: E402

SCHEMA_VERSION = 1
MERGE_FIELDS = {
    "schema_version", "kind", "run_id", "round", "candidate_commit", "base_head",
    "pre_merge_head", "merged_head", "ci_report_sha256", "ci_state", "ci_provider",
    "require_independent_review", "review_kind", "independent_review", "publication",
}
ROLLBACK_FIELDS = {
    "schema_version", "kind", "run_id", "candidate_commit", "from_head",
    "restored_head", "publication",
}
CI_FIELDS = {"schema_version", "commit", "state", "provider", "url"}
CI_STATES = frozenset(("success", "pending", "failure", "missing"))

# code -> exit status. "nothing to act on" is 3; every refusal is 1.
CODES = {
    "ok": 0,
    "invalid_argument": 2,
    "run_dir_missing": 3,
    "evidence_missing": 3,
    "root_missing": 3,
    "root_not_git_repository": 3,
    "git_unavailable": 3,
    "publish_record_unreadable": 1,
    "publish_record_invalid": 1,
    "ci_report_missing": 3,
    "ci_report_unreadable": 1,
    "ci_report_schema_invalid": 1,
    "ci_commit_mismatch": 1,
    "ci_state_refused": 1,
    "head_drifted": 1,
    "worktree_dirty": 1,
    "already_merged": 1,
    "already_rolled_back": 1,
    "merge_failed": 1,
    "merge_integrity_mismatch": 1,
    "rollback_without_merge": 1,
    "rollback_head_mismatch": 1,
    "independent_review_required": 1,
}
OK_CODE = "ok"


class PublishError(RuntimeError):
    """Fixed code only; never carries paths, file bodies or operator text."""

    def __init__(self, code: str):
        assert code in CODES, code
        self.code = code
        super().__init__(code)


def _git(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=root, capture_output=True,
                          stdin=subprocess.DEVNULL, timeout=60)


def _require_git(root: Path) -> None:
    if not candidate._git_ok(root, "rev-parse", "--git-dir"):
        raise VerifyError("root_not_git_repository")


def _head(root: Path, missing: str) -> str:
    proc = _git(root, "rev-parse", "HEAD")
    if proc.returncode:
        raise VerifyError(missing)
    return proc.stdout.decode().strip()


def _worktree_clean(root: Path) -> None:
    proc = _git(root, "status", "--porcelain")
    if proc.returncode or proc.stdout:
        raise PublishError("worktree_dirty")


def _check_ci_report(path: Any, candidate_commit: str) -> tuple[dict[str, Any], Any]:
    """Validate the CI report and bind it to exactly the candidate commit.

    Every non-success state and every foreign SHA is a refusal with its own
    code: "CI pending不得merge" is enforced here, not negotiated. Returns the
    report plus the exact bytes digested into the merge record.
    """
    try:
        # candidate.VerifyError only carries candidate codes; "evidence_missing"
        # is remapped to this gate's own vocabulary below.
        report = candidate._load_json(path, "evidence_missing")
        raw = _read_file(path, candidate.MAX_JSON_BYTES, "evidence_missing")
    except VerifyError as exc:
        if exc.code == "evidence_missing":
            raise PublishError("ci_report_missing") from None
        if exc.code in ("evidence_unreadable", "evidence_symlink_rejected",
                        "evidence_oversize"):
            raise PublishError("ci_report_unreadable") from None
        raise PublishError("ci_report_schema_invalid") from None
    if not isinstance(report, dict) or set(report) != CI_FIELDS:
        raise PublishError("ci_report_schema_invalid")
    if report["schema_version"] != SCHEMA_VERSION:
        raise PublishError("ci_report_schema_invalid")
    if not _is_hex(report["commit"], 40) or report["commit"] != candidate_commit:
        raise PublishError("ci_commit_mismatch")
    if report["state"] not in CI_STATES:
        raise PublishError("ci_report_schema_invalid")
    if report["state"] != "success":
        # The refused state itself is fixed vocabulary, safe to report.
        raise _RefusedState(report["state"])
    if not isinstance(report["provider"], str) or not 0 < len(report["provider"]) <= 60:
        raise PublishError("ci_report_schema_invalid")
    if not isinstance(report["url"], str) or not 0 < len(report["url"]) <= 300:
        raise PublishError("ci_report_schema_invalid")
    return report, raw


class _RefusedState(PublishError):
    """A CI report in a definite non-success state; the state rides along."""

    def __init__(self, state: str):
        super().__init__("ci_state_refused")
        self.state = state


def _exclusive_json(path: Path, payload: dict[str, Any], code: str) -> None:
    """Publish a record by exclusive creation; an existing record is a refusal."""
    body = json.dumps(payload, ensure_ascii=False, sort_keys=True, indent=1) + "\n"
    try:
        with path.open("x", encoding="utf-8") as handle:
            handle.write(body)
    except FileExistsError:
        raise PublishError(code) from None


def merge(run_dir: Any, root: Any, ci_report: Any,
          require_independent_review: bool = False) -> dict[str, Any]:
    directory = candidate._existing_dir(candidate._no_dot_dot(run_dir).absolute(),
                                        "run_dir_missing")
    target = candidate._existing_dir(candidate._no_dot_dot(root),
                                     "root_missing")
    target = Path(__import__("os").path.realpath(target))
    _require_git(target)
    try:
        report_path = candidate._no_dot_dot(ci_report)
    except VerifyError:
        raise PublishError("invalid_argument") from None
    # The candidate must prove itself first: digests, review binding, patch
    # bytes equal to the real git diff, commits present, baselines intact.
    verified = candidate.verify(str(directory), str(target))
    if verified["scope"] != "evidence+git":
        raise PublishError("invalid_argument")
    manifest = _load_json(directory / "candidate.json", "evidence_missing")
    if (directory / "publish.json").exists():
        raise PublishError("already_merged")
    if (directory / "rollback.json").exists():
        raise PublishError("already_rolled_back")
    if require_independent_review and not verified["independent_review"]:
        raise PublishError("independent_review_required")
    report, report_bytes = _check_ci_report(report_path, manifest["candidate_commit"])
    if _head(target, "root_missing") != manifest["base_head"]:
        raise PublishError("head_drifted")
    _worktree_clean(target)
    patch = _read_file(directory / "candidate.patch", candidate.MAX_PATCH_BYTES,
                       "evidence_missing")
    message = (f"automation: merge run {manifest['run_id']} round "
               f"{manifest['round']} candidate {manifest['candidate_commit'][:12]}")
    proc = _git(target, "merge", "--no-ff", "-m", message, manifest["candidate_commit"])
    if proc.returncode:
        raise PublishError("merge_failed")
    merged_head = _head(target, "root_missing")
    post = _git(target, "diff", "--binary", "--no-ext-diff", "--no-textconv",
                manifest["base_head"], merged_head, "--")
    if post.returncode or post.stdout != patch:
        raise PublishError("merge_integrity_mismatch")
    record = {
        "schema_version": SCHEMA_VERSION,
        "kind": "merge",
        "run_id": manifest["run_id"],
        "round": manifest["round"],
        "candidate_commit": manifest["candidate_commit"],
        "base_head": manifest["base_head"],
        "pre_merge_head": manifest["base_head"],
        "merged_head": merged_head,
        "ci_report_sha256": hashlib.sha256(report_bytes).hexdigest(),
        "ci_state": report["state"],
        "ci_provider": report["provider"],
        "require_independent_review": require_independent_review,
        "review_kind": manifest["review_kind"],
        "independent_review": verified["independent_review"],
        "publication": "merged-local",
    }
    assert set(record) == MERGE_FIELDS
    _exclusive_json(directory / "publish.json", record, "already_merged")
    return {"ok": True, "code": OK_CODE, "run_id": record["run_id"],
            "merged_head": merged_head, "review_kind": record["review_kind"],
            "independent_review": record["independent_review"]}


def rollback(run_dir: Any, root: Any) -> dict[str, Any]:
    directory = candidate._existing_dir(candidate._no_dot_dot(run_dir).absolute(),
                                        "run_dir_missing")
    target = candidate._existing_dir(candidate._no_dot_dot(root), "root_missing")
    target = Path(__import__("os").path.realpath(target))
    _require_git(target)
    if not (directory / "publish.json").is_file():
        raise PublishError("rollback_without_merge")
    record = _load_json(directory / "publish.json", "publish_record_unreadable")
    if not isinstance(record, dict) or set(record) != MERGE_FIELDS:
        raise PublishError("publish_record_invalid")
    if record.get("kind") != "merge" or record.get("schema_version") != SCHEMA_VERSION:
        raise PublishError("publish_record_invalid")
    if not _is_hex(record.get("candidate_commit"), 40):
        raise PublishError("publish_record_invalid")
    if (directory / "rollback.json").exists():
        raise PublishError("already_rolled_back")
    manifest = _load_json(directory / "candidate.json", "evidence_missing")
    if record["run_id"] != manifest["run_id"] or \
            record["candidate_commit"] != manifest["candidate_commit"]:
        raise PublishError("publish_record_invalid")
    _worktree_clean(target)
    if _head(target, "root_missing") != record["merged_head"]:
        raise PublishError("rollback_head_mismatch")
    if not candidate._git_ok(target, "rev-parse", "--verify", "--quiet",
                             record["pre_merge_head"] + "^{commit}"):
        raise PublishError("rollback_head_mismatch")
    proc = _git(target, "reset", "--hard", record["pre_merge_head"])
    if proc.returncode:
        raise PublishError("merge_failed")
    restored = _head(target, "root_missing")
    if restored != record["pre_merge_head"]:
        raise PublishError("merge_failed")
    rollback_record = {
        "schema_version": SCHEMA_VERSION,
        "kind": "rollback",
        "run_id": record["run_id"],
        "candidate_commit": record["candidate_commit"],
        "from_head": record["merged_head"],
        "restored_head": restored,
        "publication": "rolled-back",
    }
    assert set(rollback_record) == ROLLBACK_FIELDS
    _exclusive_json(directory / "rollback.json", rollback_record, "already_rolled_back")
    return {"ok": True, "code": OK_CODE, "run_id": record["run_id"],
            "restored_head": restored}


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        raise SystemExit(2)


def main(argv: list[str] | None = None) -> int:
    try:
        parser = _Parser(prog="publish.py",
                         description="Authorized local merge gate and rollback")
        sub = parser.add_subparsers(dest="command", required=True)
        merge_parser = sub.add_parser("merge")
        merge_parser.add_argument("--run-dir", required=True)
        merge_parser.add_argument("--root", required=True)
        merge_parser.add_argument("--ci-json", required=True)
        merge_parser.add_argument("--require-independent-review", action="store_true")
        rollback_parser = sub.add_parser("rollback")
        rollback_parser.add_argument("--run-dir", required=True)
        rollback_parser.add_argument("--root", required=True)
        args = parser.parse_args(argv)
        if args.command == "merge":
            result = merge(args.run_dir, args.root, args.ci_json,
                           args.require_independent_review)
        else:
            result = rollback(args.run_dir, args.root)
    except _RefusedState as exc:
        result = {"ok": False, "code": exc.code, "ci_state": exc.state}
    except PublishError as exc:
        result = {"ok": False, "code": exc.code}
    except VerifyError as exc:
        # Candidate verification refusals pass through with their own codes;
        # any candidate-internal code this gate does not re-declare (duplicate
        # keys, schema drift, oversize) still refuses with a fixed vocabulary
        # instead of leaking an unknown code or a traceback.
        code = exc.code if exc.code in CODES else "publish_record_unreadable"
        result = {"ok": False, "code": code}
    except SystemExit as exc:  # argparse's own exit (--help, bad flag)
        if exc.code in (0, None):
            return 0
        result = {"ok": False, "code": "invalid_argument"}
    except (OSError, ValueError, TypeError, KeyError, AttributeError, MemoryError,
            subprocess.SubprocessError):
        result = {"ok": False, "code": "publish_record_unreadable"}
    order = ("ok", "code", "ci_state", "run_id", "merged_head", "restored_head",
             "review_kind", "independent_review")
    print(json.dumps({key: result[key] for key in order if key in result},
                     sort_keys=True))
    return CODES[result["code"]]


if __name__ == "__main__":
    raise SystemExit(main())
