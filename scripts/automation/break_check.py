#!/usr/bin/env python3
"""Explicit, content-bound Break execution in a caller-owned disposable cwd.

Public API: run_break_check(check_id, registry, cwd, *, source_fingerprint,
fingerprint, timeout=30, max_output_bytes=65536). `fingerprint(cwd)` must use the
same scope as source_fingerprint and is checked before and after execution.
The caller owns isolation/authorization: this is process containment, not an OS
sandbox. Only the selected registry discriminating_by.command is executed, with
shell=False and a minimal environment; argv/env overrides are not accepted.

Native stdout is exactly JSON v1 {schema_version:1, experiments:[{id:stable_id,
baseline_rejected:true, mutant_false_green:true}]}; every row must be true, IDs
unique, 1..64 rows, no extra fields, duplicate keys, NaN or trailing output.
Exit 0 is mandatory. There is no nonzero-exit-as-success rejection protocol.
Only automation.artifact's exact existing --ablation-experiments declaration may
use the legacy {experiments:[{ablation,baseline_code,mutant_code,
mutant_false_green}]} adapter: source_content_comparison -> source_drift and
artifact_content_comparison -> artifact_changed, mutant_code=ok, literal true.
automation.snapshot currently has no structured outlet: declared_not_executed.

Evidence contains fixed codes and validated fields, never argv/stdout/stderr.
The combined stdout/stderr budget is at most 65536 bytes; only stdout is kept in
memory for parsing, stderr is discarded. Timeout kills the owned POSIX group;
ps verifies no live group members remain (zombies are already stopped). Detached
children that deliberately create a new session require an OS sandbox and are
outside this trusted-registry protocol. On unsupported hosts execution is blocked.
SHA256(argv) uses UTF-8 JSON with ensure_ascii=True and compact separators;
counterexample_command_sha256 binds the declaration, executed_command_sha256
also binds the selected Python interpreter when a version floor is declared.
CLI accepts --isolated (optionally with --cwd naming the project root; defaults to
the process cwd) to freeze vetted current Git-selected bytes in a temporary
independent repository, or explicit --cwd/--source-fingerprint for caller-owned
isolation. Exit codes: passed=0, failed=1, blocked/not_run=3, bad arguments=2.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True

MAX_OUTPUT_BYTES = 65536
MAX_EXPERIMENTS = 64
HEX = re.compile(r"[0-9a-f]{64}\Z")
ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}\Z")
ARTIFACT_COMMAND = ["python3", "scripts/test-automation-artifact.py", "--ablation-experiments"]
LEGACY_BASELINES = {"source_content_comparison": "source_drift",
                    "artifact_content_comparison": "artifact_changed"}


class BreakError(ValueError):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


def command_sha256(command):
    """Hash declared argv, preserving argument boundaries; never publish argv."""
    return hashlib.sha256(json.dumps(command, ensure_ascii=True,
        separators=(",", ":"), allow_nan=False).encode("utf-8")).hexdigest()


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise BreakError("duplicate_key")
        result[key] = value
    return result


def _decode(data):
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=_unique,
            parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    except BreakError:
        raise
    except (ValueError, UnicodeError, RecursionError):
        raise BreakError("invalid_json") from None


def validate_experiments(data: bytes, *, artifact_legacy=False):
    """Validate a bounded stdout buffer and return normalized v1 experiment rows."""
    if not isinstance(data, bytes) or len(data) > MAX_OUTPUT_BYTES:
        raise BreakError("output_limit")
    value = _decode(data)
    legacy = artifact_legacy and isinstance(value, dict) and set(value) == {"experiments"}
    if not legacy and (not isinstance(value, dict)
            or set(value) != {"schema_version", "experiments"}
            or type(value.get("schema_version")) is not int or value["schema_version"] != 1):
        raise BreakError("invalid_schema")
    rows = value["experiments"]
    if not isinstance(rows, list) or not 1 <= len(rows) <= MAX_EXPERIMENTS:
        raise BreakError("invalid_experiments")
    experiments, ids = [], set()
    for row in rows:
        keys = ({"ablation", "baseline_code", "mutant_code", "mutant_false_green"}
                if legacy else {"id", "baseline_rejected", "mutant_false_green"})
        if not isinstance(row, dict) or set(row) != keys:
            raise BreakError("invalid_experiment")
        identity = row["ablation" if legacy else "id"]
        if not isinstance(identity, str) or not ID.fullmatch(identity):
            raise BreakError("invalid_experiment")
        if identity in ids:
            raise BreakError("duplicate_experiment")
        ids.add(identity)
        if legacy:
            if (identity not in LEGACY_BASELINES or row["baseline_code"] != LEGACY_BASELINES[identity]
                    or row["mutant_code"] != "ok" or row["mutant_false_green"] is not True):
                raise BreakError("experiment_not_discriminating")
        elif row["baseline_rejected"] is not True or row["mutant_false_green"] is not True:
            raise BreakError("experiment_not_discriminating")
        experiments.append({"id": identity, "baseline_rejected": True, "mutant_false_green": True})
    return experiments


def _kill_group(proc):
    try:
        os.killpg(proc.pid, signal.SIGKILL)
        return True
    except ProcessLookupError:
        return True
    except OSError:
        # Permission/sandbox refusal must never be reported as a successful reclaim.
        return False


def _group_members(pgid):
    """Live (non-zombie) members of a process group, read via ps: no signal permission."""
    try:
        proc = subprocess.run(["ps", "-A", "-o", "pgid=,pid=,stat="], capture_output=True,
                              text=True, timeout=5)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    members = []
    for line in proc.stdout.splitlines():
        fields = line.split()
        if len(fields) != 3 or not fields[0].isdigit() or not fields[1].isdigit():
            continue
        if int(fields[0]) == pgid and not fields[2].startswith("Z"):
            members.append(int(fields[1]))
    return members


def _verify_group_absent(proc):
    deadline = time.monotonic() + 0.75
    while True:
        members = _group_members(proc.pid)
        if members == []:
            return True
        if members is None:
            return False
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.05)


def _execute(command, cwd, timeout, limit, process):
    env = _minimal_env()
    try:
        proc = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    except OSError:
        raise BreakError("command_unavailable") from None
    process["started"] = True
    stdout = bytearray()
    total = 0
    code = None
    deadline = time.monotonic() + timeout
    selector = selectors.DefaultSelector()
    try:
        for stream in (proc.stdout, proc.stderr):
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ)
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                process["timed_out"] = True
                code = "timeout"
                break
            if proc.poll() is not None:
                process["completed"] = True
                members = _group_members(proc.pid)
                if members:
                    process["descendants_detected"] = True
                    code = "descendants_left_running"
                    break
                if members is None:
                    # Unreadable process table: pipes are the only remaining proof.
                    process["group_inspection_unavailable"] = True
                if not selector.get_map():
                    break
            for key, _ in selector.select(min(remaining, 0.05)):
                chunk = os.read(key.fileobj.fileno(), 8192)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                total += len(chunk)
                if total > limit:
                    process["output_limited"] = True
                    code = "output_limit"
                    break
                if key.fileobj is proc.stdout:
                    stdout.extend(chunk)
            if code:
                break
    finally:
        # SIGKILL applies even after a successful parent exit; inherited pipes must
        # never let a child survive or keep this runner waiting indefinitely.
        killed = _kill_group(proc)
        try:
            proc.wait(timeout=1)
        except subprocess.TimeoutExpired:
            killed = False
        process["returncode"] = proc.returncode
        process["group_kill_sent"] = killed
        absent = _verify_group_absent(proc) if killed else False
        process["group_absent_verified"] = absent
        process["group_reclaimed"] = killed and absent
        process["output_bytes_observed"] = total
        selector.close()
        proc.stdout.close()
        proc.stderr.close()
    if not process["group_reclaimed"]:
        raise BreakError("cleanup_failed")
    if code:
        raise BreakError(code)
    if proc.returncode != 0:
        raise BreakError("nonzero_exit")
    return bytes(stdout)


def _envelope(check_id):
    """Fixed result shape shared by direct and isolated execution envelopes."""
    return {"schema_version": 1, "ok": False, "status": "failed", "code": "invalid_argument",
            "check_id": check_id if isinstance(check_id, str) and ID.fullmatch(check_id) else None,
            "source_fingerprint": None, "fingerprint_before": None, "fingerprint_after": None,
            "counterexample_command_sha256": None, "executed_command_sha256": None,
            "timeout_seconds": None, "max_output_bytes": None, "experiments": [],
            "process": {"started": False, "completed": False, "returncode": None,
                "timed_out": False, "output_limited": False, "descendants_detected": False,
                "group_reclaimed": False, "group_kill_sent": False, "group_absent_verified": False,
                "group_inspection_unavailable": False, "output_bytes_observed": 0}}


def run_break_check(check_id, registry, cwd, *, source_fingerprint, fingerprint,
                    timeout=30, max_output_bytes=MAX_OUTPUT_BYTES):
    """Run one explicitly selected declaration. Failures are fixed-code JSON data."""
    result = _envelope(check_id)
    result["source_fingerprint"] = (source_fingerprint if isinstance(source_fingerprint, str)
                                    and HEX.fullmatch(source_fingerprint) else None)
    started = time.monotonic()
    bound = False
    try:
        if (result["check_id"] is None or result["source_fingerprint"] is None
                or not callable(fingerprint) or type(timeout) not in (int, float)
                or not math.isfinite(timeout) or not 0 < timeout <= 86400
                or type(max_output_bytes) is not int or not 1 <= max_output_bytes <= MAX_OUTPUT_BYTES):
            raise BreakError("invalid_argument")
        result["timeout_seconds"] = timeout
        result["max_output_bytes"] = max_output_bytes
        cwd = Path(cwd).resolve(strict=True)
        if not cwd.is_dir():
            raise BreakError("invalid_cwd")
        if (not isinstance(registry, dict) or type(registry.get("schema_version")) is not int
                or registry["schema_version"] != 1 or not isinstance(registry.get("checks"), dict)):
            raise BreakError("invalid_registry")
        entry = registry["checks"].get(check_id)
        if not isinstance(entry, dict):
            raise BreakError("unknown_check")
        counter = entry.get("discriminating_by")
        if counter is None:
            result["status"] = "not_run"
            raise BreakError("not_declared")
        if not isinstance(counter, dict) or set(counter) != {"command", "reason"}:
            raise BreakError("invalid_declaration")
        command = counter["command"]
        if (not isinstance(command, list) or not 1 <= len(command) <= 128
                or any(not isinstance(p, str) or not p.strip() or len(p) > 4096 or "\0" in p for p in command)
                or not isinstance(counter["reason"], str) or not counter["reason"].strip()
                or len(counter["reason"]) > 400 or "\0" in counter["reason"]):
            raise BreakError("invalid_declaration")
        result["counterexample_command_sha256"] = command_sha256(command)
        if (entry.get("requires_live_service") or entry.get("requires_spending")
                or entry.get("requires_env") or entry.get("command_env")
                or any("{env:" in p for p in command)):
            result["status"] = "blocked"
            raise BreakError("external_authorization_required")
        host = "windows" if sys.platform == "win32" else sys.platform
        if os.name != "posix" or ("host_os" in entry and host not in entry["host_os"]):
            result["status"] = "blocked"
            raise BreakError("unsupported_host")
        try:
            before = fingerprint(cwd)
        except Exception:
            raise BreakError("fingerprint_failed") from None
        if not isinstance(before, str) or not HEX.fullmatch(before):
            raise BreakError("fingerprint_failed")
        result["fingerprint_before"] = before
        if before != source_fingerprint:
            raise BreakError("source_fingerprint_mismatch")
        bound = True
        actual = list(command)
        if actual[0] in ("python", "python3") and "requires_python" in entry:
            floor = entry["requires_python"]
            if not isinstance(floor, str) or not re.fullmatch(r"\d+\.\d+", floor):
                raise BreakError("invalid_declaration")
            if sys.version_info[:2] < tuple(map(int, floor.split("."))):
                result["status"] = "blocked"
                raise BreakError("python_unavailable")
            actual[0] = sys.executable
        result["executed_command_sha256"] = command_sha256(actual)
        data = _execute(actual, cwd, timeout, max_output_bytes, result["process"])
        result["experiments"] = validate_experiments(data,
            artifact_legacy=check_id == "automation.artifact" and command == ARTIFACT_COMMAND)
        result.update(ok=True, status="passed", code="ok")
    except BreakError as exc:
        result["code"] = exc.code
    except (OSError, ValueError, TypeError, KeyError, OverflowError):
        result["code"] = "invalid_argument"
    finally:
        if bound:
            try:
                after = fingerprint(cwd)
                if not isinstance(after, str) or not HEX.fullmatch(after):
                    raise ValueError()
                result["fingerprint_after"] = after
                if after != source_fingerprint:
                    result.update(ok=False, status="failed", code="source_changed", experiments=[])
            except Exception:
                result.update(ok=False, status="failed", code="fingerprint_failed", experiments=[])
        result["duration_seconds"] = round(time.monotonic() - started, 3)
    return result


def _minimal_env():
    # Never inherit Python import injection, Git redirection, or credentials.
    return {"PATH": os.pathsep.join([str(Path(sys.executable).parent), os.defpath]),
            "PYTHONDONTWRITEBYTECODE": "1", "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_OPTIONAL_LOCKS": "0"}


def _git_read(cwd, *args):
    try:
        proc = subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull, *args],
            cwd=cwd, env=_minimal_env(), stdin=subprocess.DEVNULL,
            capture_output=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        raise BreakError("snapshot_git_failed") from None
    if proc.returncode:
        raise BreakError("snapshot_git_failed")
    return proc.stdout


def _selected_names(root):
    return tuple(sorted(set(os.fsdecode(_git_read(root, "ls-files", "-z", "--cached",
        "--others", "--exclude-standard")).split("\0")) - {""}))


def _entries_fingerprint(entries):
    digest = hashlib.sha256()
    kinds = {"120000": b"symlink", "100755": b"executable", "100644": b"file"}
    for entry in sorted(entries, key=lambda item: item.name):
        digest.update(os.fsencode(entry.name) + b"\0" + kinds[entry.git_mode] + b"\0"
                      + hashlib.sha256(entry.data).digest() + b"\0")
    return digest.hexdigest()


def _copy_snapshot(entries, root):
    # Write only the already vetted bytes; never reopen a source file.
    for entry in entries:
        target = root / entry.name
        if not target.parent.resolve().is_relative_to(root):
            raise BreakError("snapshot_source_symlink_escape")
        if entry.git_mode == "120000":
            link = os.fsdecode(entry.data)
            if not (target.parent / link).resolve().is_relative_to(root):
                raise BreakError("snapshot_source_symlink_escape")
        target.parent.mkdir(parents=True, exist_ok=True)
        if entry.git_mode == "120000":
            target.symlink_to(link)
        else:
            with target.open("xb") as handle:
                handle.write(entry.data)
            target.chmod(0o755 if entry.git_mode == "100755" else 0o644)
    # Recheck complete link chains after all entries have been materialized.
    if any(not (root / entry.name).resolve().is_relative_to(root) for entry in entries):
        raise BreakError("snapshot_source_symlink_escape")


def run_isolated_check(check_id, source_root, *, registry_path=None, timeout=30,
                       max_output_bytes=MAX_OUTPUT_BYTES):
    """Freeze current selected bytes before any Git write; all execution is in temp."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from snapshot_safety import prepare_snapshot, SnapshotSafetyError
    result = _envelope(check_id)
    isolation = {"source_root": None, "execution_root": None,
                 "source_fingerprint_before": None, "source_fingerprint_after_snapshot": None,
                 "source_fingerprint_after": None, "snapshot_fingerprint": None,
                 "temporary_directory_removed": False}
    temporary = None
    frozen_digest = None
    root = None
    try:
        root = Path(source_root).resolve(strict=True)
        isolation["source_root"] = str(root)
        # Refuse subdirectories/foreign Git contexts; --cwd means the project root.
        if Path(os.fsdecode(_git_read(root, "rev-parse", "--show-toplevel")).strip()).resolve() != root:
            raise BreakError("invalid_cwd")
        names = _selected_names(root)
        entries = prepare_snapshot(root, names)
        frozen_digest = _entries_fingerprint(entries)
        isolation["source_fingerprint_before"] = frozen_digest
        registry_path = Path(registry_path) if registry_path is not None else \
            root / "scripts/automation/registry.json"
        try:
            registry_path = registry_path.resolve(strict=True)
            registry_name = registry_path.relative_to(root).as_posix()
        except (OSError, ValueError):
            raise BreakError("invalid_registry") from None
        registry_entry = next((entry for entry in entries if entry.name == registry_name), None)
        if (registry_entry is None or registry_entry.git_mode == "120000"
                or len(registry_entry.data) > 1024 * 1024):
            raise BreakError("invalid_registry")
        registry = _decode(registry_entry.data)
        after = _entries_fingerprint(prepare_snapshot(root, _selected_names(root)))
        isolation["source_fingerprint_after_snapshot"] = after
        if after != frozen_digest:
            raise BreakError("snapshot_source_drift")
        # Resolve the default temp base (e.g. /var -> /private/var on macOS).
        temporary = tempfile.TemporaryDirectory(prefix="automation-break-",
            dir=str(Path(tempfile.gettempdir()).resolve(strict=True)))
        execution_root = Path(temporary.name).resolve(strict=True)
        isolation["execution_root"] = str(execution_root)
        _copy_snapshot(entries, execution_root)
        _git_read(execution_root, "init", "-q", "--template=")
        # No add/index/commit: include all frozen paths, even tracked ignored files.
        # New nonignored files are included too, so source markers cannot go unseen.
        def fingerprint(cwd):
            selected = set(names) | set(_selected_names(cwd))
            return _entries_fingerprint(prepare_snapshot(cwd, selected))
        copied_digest = fingerprint(execution_root)
        isolation["snapshot_fingerprint"] = copied_digest
        if copied_digest != frozen_digest:
            raise BreakError("snapshot_copy_mismatch")
        result = run_break_check(check_id, registry, execution_root,
            source_fingerprint=copied_digest, fingerprint=fingerprint,
            timeout=timeout, max_output_bytes=max_output_bytes)
    except SnapshotSafetyError as exc:
        result.update(ok=False, status="failed", code=str(exc), experiments=[])
    except BreakError as exc:
        result.update(ok=False, status="failed", code=exc.code, experiments=[])
    except Exception:
        result.update(ok=False, status="failed", code="snapshot_failed", experiments=[])
    finally:
        if frozen_digest is not None:
            try:
                after = _entries_fingerprint(prepare_snapshot(root, _selected_names(root)))
                isolation["source_fingerprint_after"] = after
                if after != frozen_digest:
                    result.update(ok=False, status="failed", code="source_changed", experiments=[])
            except Exception:
                result.update(ok=False, status="failed", code="fingerprint_failed", experiments=[])
        if temporary is not None:
            try:
                temporary.cleanup()
                if Path(temporary.name).exists():
                    raise OSError()
                isolation["temporary_directory_removed"] = True
            except Exception:
                result.update(ok=False, status="failed", code="temporary_cleanup_failed", experiments=[])
        result["isolated"] = True
        result["isolation"] = isolation
    return result


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        raise BreakError("invalid_argument")


def main(argv=None):
    try:
        parser = _Parser(description=__doc__)
        parser.add_argument("--registry", type=Path)
        parser.add_argument("--check-id", required=True)
        parser.add_argument("--cwd", type=Path)
        parser.add_argument("--isolated", action="store_true")
        parser.add_argument("--source-fingerprint")
        parser.add_argument("--timeout", type=float, default=30)
        parser.add_argument("--max-output-bytes", type=int, default=MAX_OUTPUT_BYTES)
        args = parser.parse_args(argv)
        if (not ID.fullmatch(args.check_id) or not math.isfinite(args.timeout)
                or not 0 < args.timeout <= 86400
                or not 1 <= args.max_output_bytes <= MAX_OUTPUT_BYTES
                or (args.isolated and args.source_fingerprint is not None)
                or (not args.isolated and (args.cwd is None or args.source_fingerprint is None))
                or (args.source_fingerprint is not None and not HEX.fullmatch(args.source_fingerprint))):
            raise BreakError("invalid_argument")
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        if args.isolated:
            result = run_isolated_check(args.check_id, args.cwd or Path.cwd(),
                registry_path=args.registry, timeout=args.timeout, max_output_bytes=args.max_output_bytes)
        else:
            registry_path = args.registry or Path(__file__).with_name("registry.json")
            with registry_path.open("rb") as handle:
                raw = handle.read(1024 * 1024 + 1)
            if len(raw) > 1024 * 1024:
                raise BreakError("invalid_registry")
            registry = _decode(raw)
            import pipeline
            result = run_break_check(args.check_id, registry, args.cwd,
                source_fingerprint=args.source_fingerprint, fingerprint=pipeline.worktree_fingerprint_safe,
                timeout=args.timeout, max_output_bytes=args.max_output_bytes)
    except BreakError as exc:
        result = {"schema_version": 1, "ok": False, "status": "failed", "code": exc.code}
    except (OSError, ValueError, TypeError, ImportError):
        result = {"schema_version": 1, "ok": False, "status": "failed", "code": "invalid_argument"}
    print(json.dumps(result, sort_keys=True, allow_nan=False))
    if result["ok"]:
        return 0
    if result["code"] == "invalid_argument":
        return 2
    return 3 if result["status"] in ("blocked", "not_run") else 1


if __name__ == "__main__":
    raise SystemExit(main())
