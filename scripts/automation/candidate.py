#!/usr/bin/env python3
"""Read-only verification of a published local candidate manifest.

`pipeline.py` writes `candidate.json` + `candidate.patch` into an evidence
directory, but nothing on the consumer side ever checked them, so a stale or
rewritten file was indistinguishable from a fresh one. This CLI answers one
question — "does this run still prove what it claims?" — and only reads.

Scope, reported in every reply instead of being implied:
- `evidence-only` (default, no `--root`): run.json / candidate.json / candidate.patch
  cross-consistency, run identity and outcome, round/checks/review digest binding,
  baseline and input binding, patch digest and size, on-disk review bytes.
- `evidence+git` (with `--root <repo>`): additionally read-only git checks that the
  snapshot and candidate commits exist, that the candidate descends from the
  snapshot, that the patch bytes equal `git diff base_head..candidate_commit`, and
  that the run's recorded plan baselines still match the repository's own files.

This is integrity checking over an unsigned local evidence trail, NOT
authentication. SHA256 digests detect stale or accidentally edited bytes; they do
not prove who wrote a file. A writer with access to the whole run directory can
rewrite every digest consistently — only the git object identity and the
repository's own bytes sit outside such a rewrite, and those are not a signature
either. Nothing here applies, merges, pushes, publishes or starts a model, and
`review_kind` is always reported so a deterministic mock review is never presented
as an independent one.

Exit codes: 0 verified, 1 corrupt or inconsistent evidence, 2 invalid arguments,
3 nothing to verify (missing run dir/evidence, unusable --root). Output is one JSON
object with fixed keys and fixed codes only: no path, file body or operator text.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from evidence import BASELINES  # noqa: E402  (plan baseline paths, single source)

SCHEMA_VERSION = 1
MAX_JSON_BYTES = 1024 * 1024
MAX_PATCH_BYTES = 2 * 1024 * 1024
MAX_ITEMS = 4096
HEX = set("0123456789abcdef")
DIGITS = set("0123456789")
RUN_ID_CHARS = set("0123456789TZ-abcdef")

# code -> exit status. "nothing to verify" is 3; every inconsistency is 1.
CODES = {
    "ok": 0,
    "invalid_argument": 2,
    # not verifiable at all
    "run_dir_missing": 3,
    "evidence_missing": 3,
    "root_missing": 3,
    "root_not_git_repository": 3,
    "git_unavailable": 3,
    # present but not trustworthy
    "evidence_unreadable": 1,
    "evidence_symlink_rejected": 1,
    "evidence_oversize": 1,
    "evidence_truncated": 1,
    "evidence_duplicate_key": 1,
    "evidence_schema_invalid": 1,
    "candidate_schema_unsupported": 1,
    "candidate_kind_unknown": 1,
    "run_schema_invalid": 1,
    "run_binding_mismatch": 1,
    "run_not_passed": 1,
    "revoked_candidate_present": 1,
    "candidate_run_mismatch": 1,
    "round_binding_mismatch": 1,
    "review_binding_stale": 1,
    "review_verdict_invalid": 1,
    "identity_conflict": 1,
    "baseline_binding_mismatch": 1,
    "input_binding_mismatch": 1,
    "patch_digest_mismatch": 1,
    "patch_git_mismatch": 1,
    "source_baseline_drift": 1,
    "candidate_commit_missing": 1,
    "source_snapshot_missing": 1,
}
OK_CODE = "ok"

# Exactly the fields pipeline.local_candidate() writes. An added field cannot ride
# along as an unattested claim, and a removed field is a format change, not a pass.
CANDIDATE_FIELDS = {
    "schema_version", "kind", "run_id", "round", "input_digest", "checks_digest",
    "review_digest", "base_head", "source_snapshot", "candidate_commit",
    "changes_name_status", "patch_bytes", "patch_sha256", "author", "reviewer",
    "review_kind", "workspace_policy", "requirements", "publication", "input_binding",
    "baselines",
}
PLAN_BASELINE_KEYS = {"requirements", "matrix", "registry", "selected_registry"}
BINDING_KEYS = {"trigger", "source_sha256", "prompt_sha256"}
MOCK_REVIEW_KIND = "deterministic-mock"


class VerifyError(RuntimeError):
    def __init__(self, code: str):
        assert code in CODES, code
        self.code = code
        super().__init__(code)


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        # No usage text: an echoed path would leak a private path into the reply.
        raise VerifyError("invalid_argument")


# ---------------------------------------------------------------- primitives


def _unique_object(pairs):
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise VerifyError("evidence_duplicate_key")
        result[key] = value
    return result


def _reject_constant(value):
    # NaN / Infinity have no canonical JSON form, so they can never be compared.
    raise ValueError("non-finite JSON number")


def _canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                      allow_nan=False)


def _is_hex(value: Any, length: int) -> bool:
    return isinstance(value, str) and len(value) == length and not set(value) - HEX


def _is_identity(value: Any) -> bool:
    return (isinstance(value, str) and bool(value.strip()) and "\0" not in value
            and len(value) <= 256)


def _is_run_id(value: Any) -> bool:
    # Pipeline format: YYYYMMDDThhmmssZ-<12 hex>, e.g. 20261015T124147Z-ee7fcfa4b0a9.
    # Only the shape is checked. The id carries no authority of its own; it is
    # matched against run.json, review-N.json and the git objects instead.
    if not isinstance(value, str) or len(value) != 29:
        return False
    if value[8] != "T" or value[15] != "Z" or value[16] != "-":
        return False
    if set(value) - RUN_ID_CHARS:
        return False
    numbers = (value[9:11], value[11:13], value[13:15])
    if any(set(part) - DIGITS for part in numbers):
        return False
    hour, minute, second = (int(part) for part in numbers)
    return hour < 24 and minute < 60 and second < 60


def _no_dot_dot(directory: Any) -> Path:
    """Accept '.' and relative paths, refuse any '..' component outright.

    A caller-supplied directory is never resolved through '..': the tool reads
    only the directory it was handed, so an escape attempt is a caller error,
    not something to silently normalize.
    """
    if not isinstance(directory, (str, os.PathLike)) or isinstance(directory, bytes):
        raise VerifyError("invalid_argument")
    if "\0" in os.fsdecode(directory):
        raise VerifyError("invalid_argument")
    path = Path(directory)
    if any(part == ".." for part in path.parts):
        raise VerifyError("invalid_argument")
    return path


def _existing_dir(directory: Path, missing: str, *, allow_symlink: bool = False) -> Path:
    try:
        info = directory.lstat()
    except OSError:
        raise VerifyError(missing) from None
    if stat.S_ISLNK(info.st_mode) and not allow_symlink:
        raise VerifyError("evidence_symlink_rejected")
    if not stat.S_ISDIR(info.st_mode):
        raise VerifyError(missing)
    return directory


def _read_file(path: Path, limit: int, missing: str) -> bytes:
    """Regular-file bytes only: no symlink traversal, bounded size, writer guard."""
    if path.is_symlink():
        raise VerifyError("evidence_symlink_rejected")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    try:
        fd = os.open(path, flags)
    except FileNotFoundError:
        raise VerifyError(missing) from None
    except OSError:
        raise VerifyError("evidence_unreadable") from None
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise VerifyError("evidence_oversize")
        with os.fdopen(fd, "rb", closefd=False) as handle:
            data = handle.read(limit + 1)
    except VerifyError:
        raise
    except OSError:
        raise VerifyError("evidence_unreadable") from None
    finally:
        os.close(fd)
    if len(data) > limit:
        raise VerifyError("evidence_oversize")
    if len(data) != before.st_size:
        # Read raced a rewrite or a sparse/short read: never treat it as complete.
        raise VerifyError("evidence_truncated")
    fields = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_mode)
    try:
        if fields(path.lstat()) != fields(before):
            raise VerifyError("evidence_unreadable")
    except OSError:
        raise VerifyError("evidence_unreadable") from None
    return data


def _load_json(path: Path, missing: str) -> Any:
    data = _read_file(path, MAX_JSON_BYTES, missing)
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise VerifyError("evidence_schema_invalid") from None
    try:
        return json.loads(text, object_pairs_hook=_unique_object, parse_constant=_reject_constant)
    except VerifyError:
        raise
    except (ValueError, RecursionError):
        raise VerifyError("evidence_truncated") from None


# ---------------------------------------------------------------- git (read-only)


def _git(root: Path, *args: str) -> subprocess.CompletedProcess:
    # No hooks, no external diff/textconv, no credentials prompt, no optional locks:
    # infrastructure reads only. git's own stderr is discarded, never returned.
    options = ["-c", "core.hooksPath=/dev/null", "-c", "diff.external=",
               "-c", "diff.noprefix=false", "-c", "core.fsmonitor="]
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_TERMINAL_PROMPT="0", GIT_CONFIG_NOSYSTEM="1", GIT_OPTIONAL_LOCKS="0",
               LC_ALL="C")
    return subprocess.run(["git", *options, *args], cwd=str(root), env=env, check=False,
                          stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, timeout=60)


def _git_ok(root: Path, *args: str) -> bool:
    try:
        return _git(root, *args).returncode == 0
    except (OSError, subprocess.SubprocessError):
        raise VerifyError("git_unavailable") from None


def _commit_present(root: Path, name: Any, missing: str) -> str:
    if not _is_hex(name, 40):
        raise VerifyError("candidate_schema_unsupported")
    if _git_ok(root, "cat-file", "-e", f"{name}^{{commit}}"):
        return name
    raise VerifyError(missing)


# ---------------------------------------------------------------- verification


def _check_schema(manifest: Any, patch: bytes) -> None:
    if not isinstance(manifest, dict):
        raise VerifyError("evidence_schema_invalid")
    if type(manifest.get("schema_version")) is not int or manifest["schema_version"] != SCHEMA_VERSION:
        raise VerifyError("candidate_schema_unsupported")
    if set(manifest) != CANDIDATE_FIELDS:
        raise VerifyError("candidate_schema_unsupported")
    if manifest["kind"] != "local-only-candidate":
        raise VerifyError("candidate_kind_unknown")
    if manifest["publication"] != "none" or manifest["workspace_policy"] != "retained":
        raise VerifyError("candidate_schema_unsupported")
    if type(manifest["round"]) is not int or manifest["round"] < 1:
        raise VerifyError("candidate_schema_unsupported")
    for key in ("input_digest", "checks_digest", "review_digest", "patch_sha256"):
        if not _is_hex(manifest[key], 64):
            raise VerifyError("candidate_schema_unsupported")
    for key in ("base_head", "source_snapshot", "candidate_commit"):
        if not _is_hex(manifest[key], 40):
            raise VerifyError("candidate_schema_unsupported")
    if not _is_run_id(manifest["run_id"]):
        raise VerifyError("candidate_schema_unsupported")
    for key in ("author", "reviewer"):
        if not _is_identity(manifest[key]):
            raise VerifyError("candidate_schema_unsupported")
    if not _is_identity(manifest["review_kind"]):
        raise VerifyError("candidate_schema_unsupported")
    changes = manifest["changes_name_status"]
    if (not isinstance(changes, list) or len(changes) > MAX_ITEMS
            or any(not isinstance(item, str) or not item or "\0" in item or len(item) > 4096
                   for item in changes)):
        raise VerifyError("candidate_schema_unsupported")
    if (not isinstance(manifest["requirements"], list) or not manifest["requirements"]
            or len(manifest["requirements"]) > MAX_ITEMS
            or any(not _is_identity(item) for item in manifest["requirements"])):
        raise VerifyError("candidate_schema_unsupported")
    if type(manifest["patch_bytes"]) is not int or not 0 <= manifest["patch_bytes"] <= MAX_PATCH_BYTES:
        raise VerifyError("candidate_schema_unsupported")
    if len(patch) > MAX_PATCH_BYTES:
        raise VerifyError("evidence_oversize")
    if manifest["patch_bytes"] != len(patch):
        raise VerifyError("patch_digest_mismatch")
    if manifest["patch_sha256"] != hashlib.sha256(patch).hexdigest():
        raise VerifyError("patch_digest_mismatch")
    if manifest["input_digest"] != manifest["checks_digest"]:
        raise VerifyError("round_binding_mismatch")


def _check_run(manifest: dict, record: Any, run_dir: Path) -> dict:
    if not isinstance(record, dict) or not _is_run_id(record.get("run_id")):
        raise VerifyError("run_schema_invalid")
    if record["run_id"] != manifest["run_id"]:
        raise VerifyError("run_binding_mismatch")
    if record.get("outcome") != "passed" or record.get("returncode") != 0:
        raise VerifyError("run_not_passed")
    if record.get("failed_or_blocked") or record.get("failed_checks") or record.get("failed_steps"):
        raise VerifyError("run_not_passed")
    embedded = record.get("candidate")
    if embedded is not None and _canonical(embedded) != _canonical(manifest):
        raise VerifyError("candidate_run_mismatch")
    if record.get("author") != manifest["author"] or record.get("reviewer") != manifest["reviewer"]:
        raise VerifyError("identity_conflict")
    if record.get("review_kind") != manifest["review_kind"]:
        raise VerifyError("review_binding_stale")
    if record.get("requirements") != manifest["requirements"]:
        raise VerifyError("input_binding_mismatch")
    if record.get("input_binding") != manifest["input_binding"]:
        raise VerifyError("input_binding_mismatch")
    # A revoked manifest from a failed publication must never sit beside a candidate.
    revoked = run_dir / "candidate-invalid.json"
    if revoked.is_file() or revoked.is_symlink():
        raise VerifyError("revoked_candidate_present")

    rounds = record.get("rounds")
    if not isinstance(rounds, list) or not rounds:
        raise VerifyError("run_schema_invalid")
    finals = [item for item in rounds
              if isinstance(item, dict) and item.get("round") == manifest["round"]]
    if len(finals) != 1 or manifest["round"] != max(
            item["round"] for item in rounds if isinstance(item, dict)
            and type(item.get("round")) is int):
        raise VerifyError("round_binding_mismatch")
    final = finals[0]
    if final.get("author") != manifest["author"] or final.get("reviewer") != manifest["reviewer"]:
        raise VerifyError("identity_conflict")
    if manifest["author"] == manifest["reviewer"]:
        raise VerifyError("identity_conflict")
    checks = final.get("checks")
    if (not isinstance(checks, list) or not checks
            or any(not isinstance(item, dict) or item.get("status") != "passed" for item in checks)):
        raise VerifyError("run_not_passed")
    if final.get("checks_digest") != manifest["checks_digest"]:
        raise VerifyError("round_binding_mismatch")
    if final.get("input_digest") != manifest["input_digest"]:
        raise VerifyError("round_binding_mismatch")
    if final.get("review_fingerprint_before") != final.get("review_fingerprint_after"):
        raise VerifyError("review_binding_stale")
    if final.get("review_fingerprint_after") != manifest["review_digest"]:
        raise VerifyError("review_binding_stale")
    review = final.get("review")
    if not isinstance(review, dict):
        raise VerifyError("review_binding_stale")
    if review.get("verdict") != "pass":
        raise VerifyError("review_verdict_invalid")
    if (review.get("input_digest") != manifest["review_digest"]
            or review.get("run_id") != manifest["run_id"]
            or review.get("round") != manifest["round"]
            or review.get("reviewer") != manifest["reviewer"]):
        raise VerifyError("review_binding_stale")
    # The review file the run wrote must equal the review the record claims.
    reviewed = run_dir / f"review-{manifest['round']}.json"
    if reviewed.is_file():
        if _canonical(_load_json(reviewed, "evidence_missing")) != _canonical(review):
            raise VerifyError("review_binding_stale")
    elif reviewed.is_symlink():
        raise VerifyError("evidence_symlink_rejected")

    baselines = manifest["baselines"]
    if not isinstance(baselines, dict) or set(baselines) != PLAN_BASELINE_KEYS:
        raise VerifyError("baseline_binding_mismatch")
    for key, value in baselines.items():
        if value is not None and not _is_hex(value, 64):
            raise VerifyError("baseline_binding_mismatch")
    if not _is_hex(baselines["selected_registry"], 64):
        raise VerifyError("baseline_binding_mismatch")
    planned = record.get("plan_baselines")
    if isinstance(planned, dict) and planned != baselines:
        raise VerifyError("baseline_binding_mismatch")
    binding = manifest["input_binding"]
    if not isinstance(binding, dict) or not BINDING_KEYS <= set(binding):
        raise VerifyError("input_binding_mismatch")
    if not all(_is_hex(binding[key], 64) for key in ("source_sha256", "prompt_sha256")):
        raise VerifyError("input_binding_mismatch")
    return final


def _check_repository(manifest: dict, patch: bytes, root: Path) -> None:
    _commit_present(root, manifest["candidate_commit"], "candidate_commit_missing")
    _commit_present(root, manifest["source_snapshot"], "source_snapshot_missing")
    if not _git_ok(root, "merge-base", "--is-ancestor", manifest["source_snapshot"],
                    manifest["candidate_commit"]):
        raise VerifyError("source_snapshot_missing")
    diff = _git(root, "diff", "--binary", "--no-ext-diff", "--no-textconv",
                manifest["base_head"], manifest["candidate_commit"], "--")
    if diff.returncode:
        raise VerifyError("candidate_commit_missing")
    if diff.stdout != patch:
        raise VerifyError("patch_git_mismatch")
    for key, name in BASELINES.items():
        expected = manifest["baselines"][key]
        if expected is None:
            continue
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise VerifyError("source_baseline_drift")
        digest = hashlib.sha256()
        try:
            with path.open("rb") as handle:
                for chunk in iter(lambda: handle.read(65536), b""):
                    digest.update(chunk)
        except OSError:
            raise VerifyError("source_baseline_drift") from None
        if digest.hexdigest() != expected:
            raise VerifyError("source_baseline_drift")


def verify(run_dir: Any, root: Any = None) -> dict[str, Any]:
    directory = _existing_dir(_no_dot_dot(run_dir).absolute(), "run_dir_missing")
    repo = None
    if root is not None:
        candidate_root = _no_dot_dot(root)
        if not candidate_root.is_absolute():
            candidate_root = Path.cwd() / candidate_root
        repo = _existing_dir(candidate_root, "root_missing")
        repo = Path(os.path.realpath(repo))
        if not _git_ok(repo, "rev-parse", "--git-dir"):
            raise VerifyError("root_not_git_repository")
    manifest = _load_json(directory / "candidate.json", "evidence_missing")
    record = _load_json(directory / "run.json", "evidence_missing")
    patch = _read_file(directory / "candidate.patch", MAX_PATCH_BYTES, "evidence_missing")
    _check_schema(manifest, patch)
    _check_run(manifest, record, directory)
    if repo is not None:
        _check_repository(manifest, patch, repo)
    return {
        "ok": True,
        "code": OK_CODE,
        "scope": "evidence+git" if repo is not None else "evidence-only",
        "review_kind": manifest["review_kind"],
        # A deterministic mock review is reproducible output, not a second opinion.
        "independent_review": manifest["review_kind"] != MOCK_REVIEW_KIND,
        "run_id": manifest["run_id"],
    }


def main(argv: list[str] | None = None) -> int:
    try:
        parser = _Parser(prog="candidate.py",
                         description="Read-only verification of a local candidate manifest")
        parser.add_argument("command", choices=("verify",))
        parser.add_argument("--run-dir", required=True,
                            help="pipeline evidence directory holding candidate.json")
        parser.add_argument("--root", default=None,
                            help="repository root; enables the git scope")
        args = parser.parse_args(argv)
        result = verify(args.run_dir, args.root)
    except VerifyError as exc:
        result = {"ok": False, "code": exc.code}
    except SystemExit as exc:  # argparse's own exit (--help, bad flag)
        if exc.code in (0, None):
            return 0
        result = {"ok": False, "code": "invalid_argument"}
    except RecursionError:
        result = {"ok": False, "code": "evidence_schema_invalid"}
    except (OSError, ValueError, TypeError, KeyError, AttributeError, MemoryError,
            subprocess.SubprocessError):
        result = {"ok": False, "code": "evidence_unreadable"}
    order = ("ok", "code", "scope", "review_kind", "independent_review", "run_id")
    print(json.dumps({key: result[key] for key in order if key in result}, sort_keys=True))
    return CODES[result["code"]]


if __name__ == "__main__":
    raise SystemExit(main())
