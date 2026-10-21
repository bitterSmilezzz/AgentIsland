#!/usr/bin/env python3
"""AgentIsland 自动开发流水线（第一版）。

目标（对应需求 AUTO-01/AUTO-02，测试框架 L8）：
把「触发 → 隔离工作区 → Agent 执行 → 门禁 → 独立审查 → 结果回执」变成一条可复跑、
可审计、失败不谎报的链路。第一版只打通本机手动触发，X 监听留作后续适配。

硬约束（违反即失败，不放宽）：
- 源码回归必须先于构建；任何真实 Agent 调用只能在隔离工作区里发生。
- 门禁「未执行 / 缺依赖」不得记为通过；独立项失败后继续汇总，依赖失败下游 blocked。
- 禁止自动 push / merge / 发版 / 发短信 / 访问凭据；候选产物只落本地证据目录。
- 复盘账本（LESSONS）按缺陷键 + 日期聚合，重复写日志不能凑三次。
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import signal
import stat
import subprocess
import sys
import threading
import tempfile
import time
import uuid
import xml.etree.ElementTree as ET
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parent.parent.parent
REGISTRY = REPO / "scripts/automation/registry.json"
EVIDENCE_ROOT = REPO / ".scratch/automation-runs"


class PipelineError(RuntimeError):
    pass


def utcnow() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def today() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d")


ENV_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")
ENV_ARG = re.compile(r"\{env:([A-Za-z_][A-Za-z0-9_]*)\}")


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise PipelineError("registry contains duplicate JSON key")
        result[key] = value
    return result


def _string_list(value: Any) -> bool:
    return (isinstance(value, list) and bool(value)
            and all(isinstance(v, str) and bool(v.strip()) and "\0" not in v for v in value))


def _self_referential(command: list[str]) -> bool:
    path = (REPO / "scripts/automation/pipeline.py").resolve()
    return any((REPO / part).resolve() == path for part in command)


def validate_registry(data: Any) -> None:
    required = {"schema_version", "gates", "requirements", "checks", "agents", "profiles"}
    if not isinstance(data, dict) or required - data.keys():
        raise PipelineError("registry missing required objects")
    if type(data["schema_version"]) is not int or data["schema_version"] != 1:
        raise PipelineError("unsupported registry schema_version")
    for key in ("gates", "requirements", "checks", "agents", "profiles"):
        if not isinstance(data[key], dict) or not data[key]:
            raise PipelineError(f"registry {key} must be a nonempty object")
    agents = data["agents"]
    if not _string_list(agents.get("available")) or agents.get("default") not in agents["available"]:
        raise PipelineError("invalid agents schema")
    for cid, entry in data["checks"].items():
        if not cid.strip() or not isinstance(entry, dict) or not _string_list(entry.get("command")):
            raise PipelineError("check command must be a nonempty argv list")
        if "depends_on" in entry:
            dependencies = entry["depends_on"]
            if (not isinstance(dependencies, list)
                    or (dependencies and not _string_list(dependencies))
                    or len(set(dependencies)) != len(dependencies)):
                raise PipelineError(f"invalid depends_on schema: {cid}")
            if any(dependency not in data["checks"] for dependency in dependencies):
                raise PipelineError(f"unknown check dependency: {cid}")
        host = entry.get("host_os")
        if not _string_list(host) or not set(host) <= {"darwin", "linux", "windows"}:
            raise PipelineError("invalid check host_os")
        if "requires_binary" in entry and (not isinstance(entry["requires_binary"], str)
                or not entry["requires_binary"].strip() or "\0" in entry["requires_binary"]):
            raise PipelineError("invalid requires_binary")
        if "binary_newer_than" in entry and not _string_list(entry["binary_newer_than"]):
            raise PipelineError("invalid binary_newer_than")
        if "requires_python" in entry:
            value = entry["requires_python"]
            if not isinstance(value, str) or not re.fullmatch(r"\d+\.\d+", value):
                raise PipelineError("invalid requires_python")
        if "timeout_seconds" in entry:
            # A protocol that legitimately runs longer than the profile default
            # must be able to say so. Without this, a 105-minute soak can only
            # ever report "timeout", which looks like a broken check instead of
            # a mismatched budget.
            value = entry["timeout_seconds"]
            if type(value) is not int or not 60 <= value <= 86400:
                raise PipelineError(f"invalid timeout_seconds: {cid}")
        if "blocked_returncodes" in entry:
            codes = entry["blocked_returncodes"]
            if (not isinstance(codes, list) or not codes
                    or any(type(c) is not int or c <= 0 for c in codes)):
                raise PipelineError("invalid blocked_returncodes")
            if 3 in codes:
                raise PipelineError("blocked_returncodes must not redeclare exit code 3")
        for field in ("requires_env", "command_env"):
            if field not in entry:
                continue
            value = entry[field]
            valid = isinstance(value, dict) and bool(value) and all(
                ENV_NAME.fullmatch(k) and isinstance(v, str) and bool(v) and "\0" not in v
                for k, v in value.items())
            if field == "requires_env" and isinstance(value, list):
                valid = _string_list(value) and all(ENV_NAME.fullmatch(k) for k in value)
            if not valid:
                raise PipelineError(f"invalid {field} schema")
        if "discriminating_by" in entry:
            # A declared counterexample is only meaningful if it really exists and
            # really exercises the same production path as the check it guards.
            # A stale or self-referential declaration would let a check claim
            # "proven able to fail" while nothing ever proves it.
            counter = entry["discriminating_by"]
            if not isinstance(counter, dict) or set(counter) != {"command", "reason"}:
                raise PipelineError(f"invalid discriminating_by schema: {cid}")
            if not _string_list(counter["command"]):
                raise PipelineError(f"invalid discriminating_by command: {cid}")
            reason = counter["reason"]
            if (not isinstance(reason, str) or not reason.strip() or len(reason) > 400
                    or "\0" in reason):
                raise PipelineError(f"invalid discriminating_by reason: {cid}")
            if _self_referential(counter["command"]):
                raise PipelineError(f"self-referential counterexample: {cid}")
        for part in entry["command"]:
            if "{env:" in ENV_ARG.sub("", part):
                raise PipelineError("invalid environment argv placeholder")
    # Validate the entire graph, including checks absent from every profile.
    ordered_checks(data, list(data["checks"]))
    for rid, entry in data["requirements"].items():
        if not rid.strip() or not isinstance(entry, dict) or not isinstance(entry.get("checks"), list):
            raise PipelineError("requirement checks must be a list")
        if entry['checks'] and not _string_list(entry['checks']):
            raise PipelineError("invalid requirement checks")
        if not entry['checks'] and entry.get('status') != 'planned':
            raise PipelineError("empty requirement checks require explicit planned status")
        if any(cid not in data["checks"] for cid in entry["checks"]):
            raise PipelineError("unknown requirement check reference")
        # 逐需求的 `gates` 已删除（BUG-001）：它是矩阵「层」列的重复副本，实测 36 项里 26 项漂移。
        # 这里**明确拒绝**而不是忽略——静默接受等于给「再长回来一份副本」留了门。
        if "gates" in entry:
            raise PipelineError(
                "per-requirement gates were removed; the matrix 层 column is the single statement")
    count = 0
    for name, ids in data["profiles"].items():
        if name == "note":
            if not isinstance(ids, str):
                raise PipelineError("profiles.note must be text")
            continue
        count += 1
        if not name.strip() or not _string_list(ids) or len(set(ids)) != len(ids):
            raise PipelineError("profile must contain a nonempty unique check list")
        if any(cid not in data["checks"] for cid in ids):
            raise PipelineError("unknown profile check reference")
        ordered_checks(data, ids)
        if any(_self_referential(data["checks"][cid]["command"]) for cid in ids):
            raise PipelineError("self-referential profile check")
    if not count:
        raise PipelineError("registry must define a profile")


def load_registry() -> dict[str, Any]:
    try:
        data = json.loads(REGISTRY.read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, ValueError) as exc:
        raise PipelineError("registry unreadable or invalid JSON") from exc
    validate_registry(data)
    return data


@dataclass
class CheckResult:
    check_id: str
    status: str  # passed | failed | blocked | not_run
    returncode: int | None = None
    duration_seconds: float = 0.0
    stdout_tail: str = ""
    stderr_tail: str = ""
    reason: str = ""


@dataclass
class StepResult:
    name: str
    status: str
    detail: str = ""


def git(*args: str, cwd: Path, check: bool = True) -> subprocess.CompletedProcess[str]:
    # Even `status`/`worktree remove` can execute clean filters. Disable configured
    # filters for infrastructure operations without reading their command values.
    keys = subprocess.run(["git", "config", "--name-only", "--get-regexp", r"^filter\..*\.(clean|smudge|process|required)$"],
                          cwd=str(cwd), capture_output=True, text=True).stdout.splitlines()
    options = ["-c", "core.hooksPath=/dev/null"]
    for key in keys:
        options.extend(["-c", key + ("=false" if key.endswith(".required") else "=")])
    proc = subprocess.run(["git", *options, *args], cwd=str(cwd), capture_output=True, text=True)
    if check and proc.returncode != 0:
        raise PipelineError("git command failed")
    return proc


def worktree_fingerprint(cwd: Path, *, skip_missing: bool = False) -> str:
    """Hash tracked and nonignored untracked content; never publish file content."""
    digest = hashlib.sha256()
    names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard", cwd=cwd).stdout
    for name in sorted(set(names.split("\0")) - {""}):
        path = cwd / name
        try:
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode):
                value = os.fsencode(os.readlink(path))
                kind = b"symlink"
                file_hash = hashlib.sha256(value).digest()
            elif stat.S_ISREG(mode):
                kind = b"executable" if mode & 0o111 else b"file"
                content = hashlib.sha256()
                with path.open("rb") as handle:
                    for chunk in iter(lambda: handle.read(65536), b""):
                        content.update(chunk)
                file_hash = content.digest()
            else:
                raise PipelineError("unsupported source file type")
        except FileNotFoundError:
            if skip_missing:
                continue
            kind, file_hash = b"missing", b""
        except OSError as exc:
            raise PipelineError("source fingerprint unreadable") from exc
        digest.update(os.fsencode(name) + b"\0" + kind + b"\0" + file_hash + b"\0")
    return digest.hexdigest()


def resolve_checks(registry: dict[str, Any], requirement_ids: list[str]) -> list[str]:
    """把需求 ID 展开为去重的检查项列表；未登记需求直接失败。

    自引用防护：如果一个检查项的命令就是本流水线自身，会在运行时形成无限递归
    （AUTO-01 → automation.selfcheck → pipeline → AUTO-01）。这里直接拒绝。
    """
    checks: list[str] = []
    unknown: list[str] = []
    unexecutable: list[str] = []
    self_path = str((REPO / "scripts/automation/pipeline.py").resolve())
    for rid in requirement_ids:
        entry = registry["requirements"].get(rid)
        if entry is None:
            unknown.append(rid)
            continue
        # 已登记但暂无检查项的需求必须**记账**，不能静默跳过：把它从选择里悄悄去掉，
        # 读者就分不清「执行过且覆盖」与「被忽略」。带 planned 标记的是已知未实现，
        # 也不该被选进来充数（selection 用途是「这次要验这些」）。
        if not entry.get("checks"):
            unexecutable.append(f"{rid} ({entry.get('status', 'unknown')})")
            continue
        for cid in entry["checks"]:
            if cid not in registry["checks"]:
                unknown.append(f"{rid}:{cid}")
                continue
            command = registry["checks"][cid].get("command", [])
            if any(str((REPO / part).resolve()) == self_path for part in command if isinstance(part, str)):
                unknown.append(f"{rid}:{cid} (self-referential)")
                continue
            if cid not in checks:
                checks.append(cid)
    if unknown:
        raise PipelineError("unknown requirement/check or self-referential command")
    if unexecutable:
        raise PipelineError(
            "selected requirements have no executable checks: " + ", ".join(unexecutable)
        )
    if not checks:
        raise PipelineError("requirements resolve to no checks")
    checks = ordered_checks(registry, checks, include_dependencies=True)
    if any(_self_referential(registry["checks"][cid]["command"]) for cid in checks):
        raise PipelineError("self-referential requirement check dependency")
    return checks


def _kill_process_group(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def _drain(stream: Any) -> None:
    # Keep at most 4096 bytes per stream in memory; deliberately discard all output.
    tail = bytearray()
    try:
        for chunk in iter(lambda: stream.read(65536), b""):
            tail[:] = (tail + chunk)[-4096:]
    finally:
        stream.close()


def missing_external(entry: dict[str, Any], cwd: Path | None = None) -> str:
    cwd = cwd or REPO
    host = "windows" if sys.platform == "win32" else sys.platform
    if host not in entry["host_os"]:
        return "host_os_mismatch"
    # POSIX process groups provide the required descendant cleanup guarantee.
    if os.name != "posix":
        return "process_group_cleanup_unsupported"
    binary = entry.get("requires_binary")
    if binary:
        path = cwd / binary
        if not path.is_file() or not os.access(path, os.X_OK):
            return "required_binary_unavailable"
        # A stale artifact silently passing is worse than a missing one: it certifies
        # behaviour that was never rebuilt. The binary must be newer than every source
        # it claims to contain.
        newer_than = entry.get("binary_newer_than", [])
        if newer_than:
            try:
                built = path.stat().st_mtime
                for name in newer_than:
                    source = cwd / name
                    if source.exists() and source.stat().st_mtime > built:
                        return f"required_binary_stale:{name}"
            except OSError:
                return "required_binary_unverifiable"
    required = entry.get("requires_env", [])
    if isinstance(required, dict):
        if any((not os.environ.get(key)) if value == '*' else os.environ.get(key) != value for key, value in required.items()):
            return "required_env_mismatch"
    elif any(not os.environ.get(key) for key in required):
        return "required_env_missing"
    # A stated interpreter floor must not be satisfied by whatever `python3` happens to
    # resolve to: /usr/bin/python3 is 3.9 on macOS while CI pins 3.12, so the same gate
    # would pass in CI and traceback locally, and the local failure would look like a
    # defect in the code under test.
    floor = entry.get("requires_python")
    if floor:
        env = {**os.environ, **entry.get("command_env", {})}
        if _python_satisfying(floor, cwd, env) is None:
            return f"required_python_missing:{floor}"
    return ""


_VERSIONED_PYTHON = re.compile(r"python3\.(\d+)")


def _python_version(executable: str) -> str | None:
    try:
        result = subprocess.run(
            [executable, "-c", "import sys;print('%d.%d' % sys.version_info[:2])"],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            timeout=10, text=True,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    match = re.fullmatch(r"(\d+\.\d+)", result.stdout.strip())
    return match.group(1) if result.returncode == 0 and match else None


def _python_satisfying(floor: str, cwd: Path, env: dict[str, str]) -> str | None:
    """Find an interpreter meeting the floor instead of trusting PATH order."""
    needed = tuple(int(part) for part in floor.split("."))
    search_path = os.pathsep.join(str((cwd / part).absolute()) for part in
                                  env.get("PATH", os.defpath).split(os.pathsep))
    candidates: list[str] = []
    for directory in search_path.split(os.pathsep):
        if not directory or not os.path.isdir(directory):
            continue
        try:
            names = sorted(os.listdir(directory))
        except OSError:
            continue
        for name in names:
            if name == "python3":
                candidates.append(os.path.join(directory, name))
                continue
            match = _VERSIONED_PYTHON.fullmatch(name)
            if match and tuple(int(p) for p in name.removeprefix("python").split(".")) >= needed:
                candidates.append(os.path.join(directory, name))
    seen: set[str] = set()
    for candidate in candidates:
        resolved = os.path.realpath(candidate)
        if resolved in seen:
            continue
        seen.add(resolved)
        if not (os.path.isfile(candidate) and os.access(candidate, os.X_OK)):
            continue
        version = _python_version(candidate)
        if version and tuple(int(part) for part in version.split(".")) >= needed:
            return candidate
    return None


def run_check(check_id: str, registry: dict[str, Any], cwd: Path, timeout: int) -> CheckResult:
    if timeout <= 0:
        raise PipelineError("check timeout must be positive")
    entry = registry["checks"][check_id]
    # A declared longer budget is a property of the protocol, so it may extend
    # the profile default but never silently shorten it.
    declared = entry.get("timeout_seconds")
    if isinstance(declared, int) and declared > timeout:
        timeout = declared
    started = time.monotonic()
    missing = missing_external(entry, cwd)
    if missing:
        return CheckResult(check_id, "blocked", reason=missing)
    env = {**os.environ, **entry.get("command_env", {})}
    if any(not env.get(match.group(1)) for part in entry["command"] for match in ENV_ARG.finditer(part)):
        return CheckResult(check_id, "blocked", reason="argv_env_missing")
    command = [ENV_ARG.sub(lambda match: env[match.group(1)], part) for part in entry["command"]]
    executable = command[0]
    # A stated interpreter floor must also govern which interpreter actually runs the
    # check, otherwise the gate is satisfied by a `python3` that cannot even import it.
    floor = entry.get("requires_python")
    if floor and executable in ("python", "python3"):
        interpreter = _python_satisfying(floor, cwd, env)
        if interpreter is None:
            return CheckResult(check_id, "blocked", reason=f"required_python_missing:{floor}")
        command[0] = interpreter
        executable = interpreter
    if os.path.dirname(executable):
        path = cwd / executable
        available = path.is_file() and os.access(path, os.X_OK)
        command[0] = str(path.absolute())
    else:
        # Relative PATH components must also be interpreted from the check cwd.
        search_path = os.pathsep.join(str((cwd / part).absolute()) for part in
                                      env.get("PATH", os.defpath).split(os.pathsep))
        resolved = shutil.which(executable, path=search_path)
        available = resolved is not None
        if resolved:
            command[0] = resolved
    if not available:
        return CheckResult(check_id, "blocked", reason="command_unavailable")
    try:
        proc = subprocess.Popen(command, cwd=str(cwd), env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    except (FileNotFoundError, PermissionError):
        return CheckResult(check_id, "blocked", reason="command_unavailable")
    except OSError:
        return CheckResult(check_id, "failed", reason="command_start_failed")
    readers = [threading.Thread(target=_drain, args=(stream,), daemon=True)
               for stream in (proc.stdout, proc.stderr)]
    reason = ""
    try:
        for reader in readers:
            reader.start()
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        reason = "timeout"
    finally:
        # Also reclaim descendants left behind by an exited parent.
        _kill_process_group(proc)
        proc.wait()
        for reader in readers:
            if reader.ident is not None:
                reader.join(timeout=1)
    # Exit code 3 is this pipeline's own "could not run" contract (dry run, missing
    # environment). Folding it into "failed" would report an absent artifact as if the
    # code under test were broken, which is exactly the confusion this gate exists to
    # prevent. A check may also declare which of its codes mean "not executable".
    if proc.returncode == 0 and not reason:
        status = "passed"
    elif not reason and proc.returncode == 3:
        status = "blocked"
    elif not reason and proc.returncode in entry.get("blocked_returncodes", []):
        status = "blocked"
    else:
        status = "failed"
    return CheckResult(check_id, status, proc.returncode, round(time.monotonic() - started, 3),
                       reason=reason or ("" if status == "passed" else f"returncode={proc.returncode}"))


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


# ---------------------------------------------------------------- lessons

sys.path.insert(0, str(Path(__file__).resolve().parent))

import evidence  # noqa: E402  (same directory; used for run provenance)
import pause  # noqa: E402  (same directory; failure records and duplicate input)

from lesson_ledger import (  # noqa: E402
    LessonError,
    append_entry,
    load_entries,
    make_entry,
    promotion_candidates,
)

LESSON_LEDGER = REPO / ".scratch/automation-lessons.jsonl"

# 每轮在此路径旁创建唯一 worktree；仍兼容 selftest 的 AUTOMATION_WORKSPACE 覆盖。
WORKSPACE_ROOT = Path(os.environ.get("AUTOMATION_WORKSPACE") or (REPO / ".scratch/automation-workspace"))


def add_lesson(rule: str, why: str, evidence: str, date: str | None = None) -> str:
    """登记一条复盘；键非法/日期非法直接抛错，不静默丢弃。"""
    entry = make_entry(
        rule=rule,
        why=why,
        evidence=evidence,
        seen_date=date or today(),
        recorded_at=utcnow(),
    )
    append_entry(LESSON_LEDGER, entry)
    return entry["key"]


def load_lessons() -> list[dict]:
    return load_entries(LESSON_LEDGER)


# ---------------------------------------------------------------- pipeline

# Only paths successfully created by this process may be removed.
_CREATED_WORKSPACES: set[Path] = set()


# Snapshot objects are local only: no ref, main index, hooks, filters or commit is changed.
# Credential-shaped source names fail closed before reading or storing their bytes.
SENSITIVE_NAME = re.compile(r"(^|/)(\.env(?:\..*)?|credentials(?:\..*)?|auth\.json|id_rsa|id_ed25519|.*\.(?:pem|key|p12|pfx))$", re.I)


def snapshot_commit(cwd: Path, parent: str) -> str:
    before = worktree_fingerprint_safe(cwd)
    names = sorted(set(git("ls-files", "-z", "--cached", "--others", "--exclude-standard", cwd=cwd).stdout.split("\0")) - {""})
    from snapshot_safety import prepare_snapshot, SnapshotSafetyError
    try:
        snapshot_entries = prepare_snapshot(cwd, names)
    except SnapshotSafetyError as exc:
        raise PipelineError(str(exc)) from None
    with tempfile.TemporaryDirectory(prefix="automation-index-") as temp:
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        env.update(GIT_INDEX_FILE=str(Path(temp) / "index"), GIT_AUTHOR_NAME="Automation Snapshot",
                   GIT_AUTHOR_EMAIL="snapshot@example.invalid", GIT_COMMITTER_NAME="Automation Snapshot",
                   GIT_COMMITTER_EMAIL="snapshot@example.invalid", GIT_CONFIG_NOSYSTEM="1")
        def command(*args: str, data: bytes | None = None) -> bytes:
            proc = subprocess.run(["git", *args], cwd=cwd, env=env, input=data, capture_output=True)
            if proc.returncode:
                raise PipelineError("snapshot_git_failed")
            return proc.stdout.strip()
        command("read-tree", "--empty")
        entries = bytearray()
        for entry in snapshot_entries:
            blob = command("hash-object", "-w", "--no-filters", "--stdin", data=entry.data)
            entries.extend(entry.git_mode.encode() + b" " + blob + b"\t" + os.fsencode(entry.name) + b"\0")
        command("update-index", "-z", "--index-info", data=bytes(entries))
        tree = command("write-tree").decode()
        commit = command("commit-tree", tree, "-p", parent, data=b"Local automation source snapshot\n").decode()
    if worktree_fingerprint_safe(cwd) != before:
        raise PipelineError("snapshot_source_drift")
    return commit


def worktree_fingerprint_safe(cwd: Path) -> str:
    names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard", cwd=cwd).stdout
    for name in set(names.split("\0")) - {""}:
        if SENSITIVE_NAME.search(name):
            raise PipelineError("unsafe_source_name")
        path = cwd / name
        if not path.resolve().is_relative_to(cwd.resolve()):
            raise PipelineError("source_path_escape")
        if path.is_symlink():
            if not path.resolve().is_relative_to(cwd.resolve()):
                raise PipelineError("source_symlink_escape")
        elif path.exists() and (not path.is_file() or path.stat().st_size > 8 * 1024 * 1024):
            raise PipelineError("unsupported_source_file")
    return worktree_fingerprint(cwd, skip_missing=True)


def create_workspace(cwd: Path, evidence: Path, keep: bool,
                     revision: str | None = None) -> tuple[Path, dict[str, Any]]:
    """Unique worktree of current source, or an explicitly bound immutable revision."""
    base = git("rev-parse", "HEAD", cwd=cwd).stdout.strip()
    revision = revision or snapshot_commit(cwd, base)
    wt = WORKSPACE_ROOT.parent / f"{WORKSPACE_ROOT.name}-{evidence.name}-{uuid.uuid4().hex[:8]}"
    if wt.exists():
        raise PipelineError("workspace path already exists")
    wt.parent.mkdir(parents=True, exist_ok=True)
    add = git("-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", "--no-checkout",
              str(wt), revision, cwd=cwd, check=False)
    if add.returncode:
        raise PipelineError("workspace creation failed")
    _CREATED_WORKSPACES.add(wt)
    # Populate only this worktree's index. Raw blobs preserve actual bytes and avoid
    # smudge filters, autocrlf, checkout hooks and attributes supplied by the source.
    git("read-tree", revision, cwd=wt)
    entries = git("ls-tree", "-rz", revision, cwd=wt).stdout.split("\0")
    for entry in entries:
        if not entry:
            continue
        metadata, name = entry.split("\t", 1)
        mode, kind, oid = metadata.split(" ")
        if kind != "blob" or mode not in ("100644", "100755", "120000"):
            raise PipelineError("unsupported_snapshot_entry")
        target = wt / name
        if not target.parent.resolve().is_relative_to(wt.resolve()):
            raise PipelineError("snapshot_path_escape")
        target.parent.mkdir(parents=True, exist_ok=True)
        blob = subprocess.run(["git", "cat-file", "blob", oid], cwd=wt, capture_output=True, check=True).stdout
        if mode == "120000":
            target.symlink_to(os.fsdecode(blob))
        else:
            target.write_bytes(blob)
            target.chmod(0o755 if mode == "100755" else 0o644)
    return wt, {"name": wt.name, "path": str(wt), "created_at": utcnow(),
                "base_head": base, "snapshot_commit": revision,
                "source_mode": "current-source-snapshot", "reused": False}


def cleanup_workspace(cwd: Path, wt: Path, keep: bool) -> StepResult:
    if wt not in _CREATED_WORKSPACES:
        return StepResult("workspace.cleanup", "failed", "workspace_not_owned")
    status = git("status", "--porcelain", "--untracked-files=all", cwd=wt, check=False)
    if status.returncode:
        return StepResult("workspace.cleanup", "failed", "workspace_status_failed")
    if keep:
        # Intentional retention is a successful policy decision, required by selftest.
        return StepResult("workspace.cleanup", "passed", "workspace_kept_by_request")
    if status.stdout.strip():
        return StepResult("workspace.cleanup", "blocked", "dirty_workspace_preserved")
    remove = git("worktree", "remove", str(wt), cwd=cwd, check=False)
    if remove.returncode:
        return StepResult("workspace.cleanup", "failed", "workspace_remove_failed")
    _CREATED_WORKSPACES.discard(wt)
    return StepResult("workspace.cleanup", "passed", "workspace_removed")


def _new_run(mode: str) -> tuple[Path, dict[str, Any]]:
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:12]
    run_dir = EVIDENCE_ROOT / run_id
    run_dir.mkdir(parents=True, exist_ok=False)
    return run_dir, {"run_id": run_id, "mode": mode, "started_at": utcnow(),
                     "checks": [], "steps": []}


def _failure_state(record: dict[str, Any], name: str, code: str) -> None:
    record["steps"].append(asdict(StepResult(name, "failed", code)))
    record.update(outcome="failed", returncode=1, finished_at=utcnow())
    record["failed_checks"] = [c["check_id"] for c in record["checks"] if c["status"] != "passed"]
    record["failed_steps"] = [s.get("name") for s in record["steps"] if s["status"] != "passed"]
    record["failed_or_blocked"] = record["failed_checks"] + record["failed_steps"]
    if record.get("candidate"):
        invalid = record.pop("candidate")
        invalid.update(valid=False, invalidated_by=code)
        record["invalid_candidate"] = invalid


def _persist_failure(run_dir: Path, record: dict[str, Any], code: str) -> None:
    # Independent attempts: a failed manifest removal or report rewrite must not
    # suppress the durable fixed-code diagnostic in the other writable location.
    diagnostic = {"run_id": record["run_id"], "outcome": "failed", "returncode": 1,
                  "detail": code, "candidate_valid": False}
    try:
        (run_dir / "candidate.json").unlink(missing_ok=True)
    except OSError:
        diagnostic["candidate_removal"] = "failed"
    persisted = False
    for path, payload in ((run_dir / "run.json", record),
                          (run_dir / "pause-failure.json", diagnostic)):
        try:
            pause.atomic_json(path, payload)
            persisted = True
        except Exception:
            pass
    if not persisted:
        try:
            pause.atomic_json(EVIDENCE_ROOT / (record["run_id"] + ".failure.json"), diagnostic)
        except Exception:
            print(json.dumps({"outcome": "failed", "reason": "failure_diagnostic_unwritable"}),
                  file=sys.stderr)


def _finish_run(run_dir: Path, record: dict[str, Any], config_error: bool = False,
                registry: dict[str, Any] | None = None, *, notify_failure: bool = False) -> int:
    try:
        rc = _write_run_reports(run_dir, record, config_error, registry)
    except Exception:
        _failure_state(record, "report.write", "report_write_failed")
        _persist_failure(run_dir, record, "report_write_failed")
        rc = 1
    if rc != 0:
        stage = next((str(s.get("name", "")).split(".")[0] for s in record["steps"]
                      if s.get("status") in ("failed", "blocked")),
                     "profile" if record.get("mode") == "profile" else "run")
        if stage not in pause.STAGES:
            stage = "run"
        try:
            record["pause"] = pause.pause_record(
                run_dir, record, stage=stage, log_path=EVIDENCE_ROOT / "notify.log",
                notify=notify_failure, detail=record.get("duplicate_of", ""))
            pause.atomic_json(run_dir / "run.json", record)
        except Exception:
            _failure_state(record, "pause.record", "pause_record_failed")
            _persist_failure(run_dir, record, "pause_record_failed")
            rc = 1
    summary = {**record.get("summary", {}), "run_id": record["run_id"],
               "outcome": record["outcome"], "evidence": str(run_dir),
               "failed_or_blocked": record.get("failed_or_blocked", [])}
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return rc


def _write_run_reports(run_dir: Path, record: dict[str, Any], config_error: bool = False,
                       registry: dict[str, Any] | None = None) -> int:
    items = record["checks"] + record["steps"]
    failed = [item for item in items if item["status"] == "failed"]
    blocked = [item for item in items if item["status"] != "passed" and item["status"] != "failed"]
    outcome = "failed" if failed or config_error else "blocked" if blocked or not items else "passed"
    rc = 2 if config_error else {"passed": 0, "failed": 1, "blocked": 3}[outcome]
    record.update(outcome=outcome, returncode=rc, finished_at=utcnow())
    # Provenance and per-requirement execution mapping belong IN the report, not in a
    # separate document: a reader must be able to tell "these mapped checks ran" from
    # "the requirement is fully verified" (it never is, on mapped checks alone).
    if "provenance" not in record:
        record["provenance"] = evidence.provenance(REPO)
    # A manifest is usable only after every blocking finalizer has succeeded.
    # Preserve diagnostics, but revoke any candidate from a failed/blocked run.
    if outcome != "passed" and record.get("candidate"):
        invalid = record.pop("candidate")
        invalid.update(valid=False, invalidated_by=outcome)
        record["invalid_candidate"] = invalid
        write_json(run_dir / "candidate-invalid.json", invalid)
        (run_dir / "candidate.json").unlink(missing_ok=True)
    if registry is not None:
        record["requirement_results"] = evidence.requirement_results(registry, record["checks"])
    record["summary"] = {status: sum(c["status"] == status for c in record["checks"])
                         for status in ("passed", "failed", "blocked")}
    record["summary"]["total"] = len(record["checks"])
    # Steps fail the run too (source drift, cleanup), so both lists belong in run.json:
    # a reader must not have to reconstruct them from the items array.
    record["failed_checks"] = [c["check_id"] for c in record["checks"] if c["status"] != "passed"]
    record["failed_steps"] = [s.get("name") for s in record["steps"] if s["status"] != "passed"]
    record["failed_or_blocked"] = record["failed_checks"] + record["failed_steps"]
    # Persist a non-successful pending record before auxiliary reports. Even if the
    # evidence directory becomes unwritable later, it must never certify success.
    pending = dict(record)
    pending.pop("candidate", None)
    pending.update(outcome="blocked", returncode=3, publication_state="pending_reports")
    write_json(run_dir / "run.json", pending)
    suite = ET.Element("testsuite", name="automation", tests=str(len(items)),
                       failures=str(len(failed)), skipped=str(len(blocked)))
    for item in items:
        case = ET.SubElement(suite, "testcase", name=item.get("check_id", item.get("name", "run")),
                             time=str(item.get("duration_seconds", 0)))
        if item["status"] != "passed":
            ET.SubElement(case, "failure" if item["status"] == "failed" else "skipped",
                          message=item.get("reason", item.get("detail", "")))
    ET.ElementTree(suite).write(run_dir / "junit.xml", encoding="utf-8", xml_declaration=True)
    lines = [f"# Automation run {record['run_id']}", "", f"Outcome: {outcome}",
             f"Exit code: {rc}", "", "| Check / step | Status | Reason |", "| --- | --- | --- |"]
    lines.extend(f"| {item.get('check_id', item.get('name'))} | {item['status']} | "
                 f"{item.get('reason', item.get('detail', ''))} |" for item in items)
    (run_dir / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    write_json(run_dir / "run.json", record)
    if outcome == "passed" and record.get("candidate"):
        write_json(run_dir / "candidate.json", record["candidate"])
    return rc


def ordered_checks(registry: dict[str, Any], ids: list[str], *,
                   include_dependencies: bool = False) -> list[str]:
    """Stable dependency-first order; strict selections must already be closed."""
    selected = set(ids)
    ordered: list[str] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(cid: str) -> None:
        if cid not in registry["checks"]:
            raise PipelineError(f"unknown check dependency: {cid}")
        if cid in visiting:
            raise PipelineError(f"check dependency cycle: {cid}")
        if cid in visited:
            return
        visiting.add(cid)
        for dependency in registry["checks"][cid].get("depends_on", []):
            if not include_dependencies and dependency not in selected:
                raise PipelineError(f"check selection missing dependency: {cid} depends_on {dependency}")
            visit(dependency)
        visiting.remove(cid)
        visited.add(cid)
        ordered.append(cid)

    for cid in ids:
        visit(cid)
    return ordered


def _status_counts(checks: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for item in checks:
        counts[item["status"]] = counts.get(item["status"], 0) + 1
    return counts


def _execute_checks(record: dict[str, Any], ids: list[str], registry: dict[str, Any],
                    cwd: Path, timeout: int) -> None:
    ids = ordered_checks(registry, ids)
    results: dict[str, CheckResult] = {}
    for index, cid in enumerate(ids):
        unmet = [f"{dependency}={results[dependency].status}"
                 for dependency in registry["checks"][cid].get("depends_on", [])
                 if results[dependency].status != "passed"]
        try:
            result = (CheckResult(cid, "blocked", reason="dependency_not_passed:" + ",".join(unmet))
                      if unmet else run_check(cid, registry, cwd, timeout))
        except KeyboardInterrupt:
            record["checks"].append(asdict(CheckResult(cid, "failed", reason="interrupted")))
            record["checks"].extend(asdict(CheckResult(rest, "blocked", reason="interrupted"))
                                    for rest in ids[index + 1:])
            raise
        except Exception:
            result = CheckResult(cid, "failed", reason="runner_error")
        results[cid] = result
        record["checks"].append(asdict(result))


def _verify_fingerprint(record: dict[str, Any], cwd: Path) -> None:
    try:
        after = worktree_fingerprint_safe(cwd) if record.get("mode") == "pipeline" else worktree_fingerprint(cwd)
        record["fingerprint_after"] = after
        unchanged = after == record.get("fingerprint_before")
        record["steps"].append(asdict(StepResult("source.integrity", "passed" if unchanged else "failed",
                                                "" if unchanged else "source_drift")))
    except Exception:
        record["steps"].append(asdict(StepResult("source.integrity", "failed", "fingerprint_failed")))


def run_profile(args: argparse.Namespace, registry: dict[str, Any]) -> int:
    """Run only registered checks, never an agent; blocked evidence returns 3."""
    run_dir, record = _new_run("profile")
    record["profile"] = args.profile
    config_error = False
    try:
        validate_registry(registry)
        if args.profile == "note" or args.profile not in registry["profiles"]:
            raise PipelineError("unknown profile")
        selected = list(registry["profiles"][args.profile])
        # Which requirements this run can speak about, and which it cannot. A profile
        # runs checks, not requirements: the mapping must be explicit so a green run
        # is never read as "these requirements are complete".
        record["selected_checks"] = selected
        record["requirements"] = sorted(
            rid for rid, entry in registry["requirements"].items()
            if any(cid in selected for cid in entry.get("checks", []))
        )
        record["cleanup"] = {"scope": "none", "detail": "profile mode creates no workspace"}
        record["review"] = {"performed": False, "reason": "profile mode runs checks only"}
        record["fingerprint_before"] = worktree_fingerprint(REPO)
        record["fingerprint"] = record["fingerprint_before"]
        require_current_registry(registry, REPO)
        _execute_checks(record, selected, registry, REPO, args.check_timeout)
    except PipelineError:
        config_error = True
        record["steps"].append(asdict(StepResult("configuration", "failed", "configuration_error")))
    except KeyboardInterrupt:
        record["steps"].append(asdict(StepResult("run", "failed", "interrupted")))
    except Exception:
        record["steps"].append(asdict(StepResult("run", "failed", "runner_error")))
    finally:
        if "fingerprint_before" in record:
            _verify_fingerprint(record, REPO)
        rc = _finish_run(run_dir, record, config_error, registry,
                         notify_failure=args.notify_failure)
    return rc


def run_pipeline(args: argparse.Namespace) -> int:
    if min(args.agent_timeout, args.check_timeout, args.review_timeout) <= 0 or not 1 <= args.max_rounds <= 10:
        raise PipelineError("timeouts must be positive and max-rounds must be 1..10")
    if not args.author_identity.strip() or not args.reviewer_identity.strip():
        raise PipelineError("identities must be nonempty")
    try:
        registry = load_registry()
    except PipelineError:
        if args.list_profiles or args.validate_registry:
            raise  # Read-only CLI must not create evidence directories.
        run_dir, record = _new_run("configuration")
        try:
            record["steps"].append(asdict(StepResult("configuration", "failed", "configuration_error")))
        finally:
            rc = _finish_run(run_dir, record, config_error=True,
                             notify_failure=args.notify_failure)
        return rc
    if args.list_profiles or args.validate_registry:
        payload = {"valid": True}
        if args.list_profiles:
            payload["profiles"] = {k: v for k, v in registry["profiles"].items() if k != "note"}
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        return 0

    # --profile 优先于 --requirements：profile 是 registry 里预先定义的一组检查项集合。
    # 用 profile 时不需要 Agent 轮次（它是测试编排，不是自动开发），
    # 因此跳过 workspace 与 driver，直接执行检查并汇总。
    if args.profile:
        return run_profile(args, registry)

    return _run_development(args, registry)


def validate_review(payload: Any, digest: str, run_id: str, round_no: int,
                    author: str, reviewer: str) -> str:
    fields = {"schema_version", "verdict", "input_digest", "run_id", "round", "reviewer", "reason"}
    if (not isinstance(payload, dict) or set(payload) != fields
            or type(payload["schema_version"]) is not int or payload["schema_version"] != 1
            or type(payload["round"]) is not int):
        raise PipelineError("review_schema_invalid")
    if author == reviewer or payload["reviewer"] != reviewer:
        raise PipelineError("review_identity_invalid")
    if (payload["input_digest"] != digest or payload["run_id"] != run_id
            or payload["round"] != round_no):
        raise PipelineError("review_binding_stale")
    if payload["verdict"] not in ("pass", "revise", "reject") or not isinstance(payload["reason"], str):
        raise PipelineError("review_verdict_invalid")
    return payload["verdict"]


def require_same_digest(expected: str, actual: str, reason: str) -> None:
    if actual != expected:
        raise PipelineError(reason)


def mock_develop_edit(wt: Path, round_no: int) -> None:
    # Fixed fixture operation; no user-controlled code, command or filesystem path.
    target = wt / "automation-mock-change.txt"
    if target.is_symlink():
        raise PipelineError("mock_edit_symlink")
    target.write_text(f"mock development round {round_no}\n", encoding="utf-8")


def mock_review(wt: Path, scenario: str, digest: str, run_id: str,
                round_no: int, reviewer: str) -> dict[str, Any]:
    verdict = "revise" if scenario == "revise-always" or scenario == "revise-once" and round_no == 1 else "pass"
    if scenario == "reject":
        verdict = "reject"
    if scenario == "timeout":
        raise PipelineError("review_timeout")
    if scenario == "mutate":
        # Deliberately violate the read-only contract in a disposable negative fixture.
        target = wt / "automation-review-violation.txt"
        wt.chmod(0o755)
        target.write_text("unauthorized reviewer edit\n", encoding="utf-8")
    return {"schema_version": 1, "verdict": verdict, "input_digest": "stale" if scenario == "stale" else digest,
            "run_id": run_id, "round": round_no, "reviewer": reviewer,
            "reason": "deterministic mock review (not an independent human or model review)"}


def readonly_source(wt: Path, readonly: bool) -> None:
    # Permission restriction plus before/after fingerprints; not an OS security sandbox.
    names = git("ls-files", "-z", cwd=wt).stdout.split("\0")
    directories = {wt}
    for name in names:
        if not name:
            continue
        path = wt / name
        if not path.is_symlink() and path.is_file():
            path.chmod(path.stat().st_mode & ~0o222 if readonly else path.stat().st_mode | 0o200)
        directories.update(p for p in path.parents if p == wt or wt in p.parents)
    for path in sorted(directories, key=lambda p: len(p.parts), reverse=readonly):
        path.chmod(0o555 if readonly else 0o755)


def local_candidate(run_dir: Path, record: dict[str, Any], wt: Path,
                    base: str, revision: str, digest: str, round_record: dict[str, Any]) -> None:
    require_same_digest(digest, worktree_fingerprint_safe(wt), "candidate_source_drift")
    if (not round_record["checks"] or any(c["status"] != "passed" for c in round_record["checks"])
            or round_record["checks_digest"] != digest or round_record["review"]["verdict"] != "pass"
            or round_record["review"]["input_digest"] != digest):
        raise PipelineError("candidate_evidence_mismatch")
    # Write bounded patch before publishing a manifest. Never merge, push or release.
    patch_path = run_dir / "candidate.patch"
    with patch_path.open("wb") as handle:
        proc = subprocess.Popen(["git", "diff", "--binary", "--no-ext-diff", "--no-textconv", base, revision, "--"],
                                cwd=wt, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        total = 0
        try:
            while chunk := proc.stdout.read(65536):
                total += len(chunk)
                if total > 2 * 1024 * 1024:
                    raise PipelineError("candidate_patch_unavailable_or_too_large")
                handle.write(chunk)
            if proc.wait(timeout=30):
                raise PipelineError("candidate_patch_unavailable_or_too_large")
        except BaseException:
            proc.kill()
            proc.wait()
            patch_path.unlink(missing_ok=True)
            raise
        finally:
            proc.stdout.close()
    changes = git("diff", "--name-status", "-z", base, revision, "--", cwd=wt).stdout
    require_same_digest(digest, worktree_fingerprint_safe(wt), "candidate_source_drift")
    manifest = {"schema_version": 1, "kind": "local-only-candidate", "run_id": record["run_id"],
                "round": round_record["round"], "input_digest": digest, "checks_digest": digest,
                "review_digest": round_record["review"]["input_digest"],
                "base_head": record["workspace"]["base_head"], "source_snapshot": base,
                "candidate_commit": revision, "changes_name_status": changes.split("\0")[:-1],
                "patch_bytes": patch_path.stat().st_size,
                "patch_sha256": hashlib.sha256(patch_path.read_bytes()).hexdigest(),
                "author": record["author"], "reviewer": record["reviewer"],
                "review_kind": "deterministic-mock", "workspace_policy": "retained",
                "requirements": record["requirements"], "publication": "none",
                "input_binding": record["input_binding"], "baselines": round_record["baselines"]}
    # Keep the proposal private to this in-memory run until finalizers succeed.
    record["candidate"] = manifest


def require_current_registry(registry: dict[str, Any], root: Path) -> None:
    try:
        actual = json.loads((root / REGISTRY.relative_to(REPO)).read_text(encoding="utf-8"),
                            object_pairs_hook=_unique_object)
    except (OSError, ValueError, PipelineError):
        raise PipelineError("registry_source_changed") from None
    if actual != registry:
        raise PipelineError("registry_source_changed")


def plan_baselines(root: Path) -> dict[str, Any]:
    baselines = evidence.baseline_digests(root)
    baselines["selected_registry"] = evidence.file_digest(root / REGISTRY.relative_to(REPO))
    return baselines


def development_rounds(args: argparse.Namespace, registry: dict[str, Any], check_ids: list[str],
                       run_dir: Path, record: dict[str, Any], wt: Path, meta: dict[str, Any],
                       origin_digest: str, origin_head: str) -> None:
    from agent_driver import run_agent
    previous_digest = None
    for round_no in range(1, args.max_rounds + 1):
        round_record: dict[str, Any] = {"round": round_no, "author": args.author_identity,
                                      "reviewer": args.reviewer_identity, "checks": []}
        round_record["input_binding"] = record["input_binding"]
        round_record["baselines"] = plan_baselines(wt)
        if round_record["baselines"] != record["plan_baselines"]:
            raise PipelineError("requirement_plan_changed")
        record["rounds"].append(round_record)
        prompt = args.prompt if round_no == 1 else "Revise based on structured review: " + json.dumps(record["rounds"][-2]["review"])
        # Capture every round before the author transport can edit, including
        # failed/no-op turns. Green baselines (e.g. after a revision) stay green.
        # Checks run in a disposable snapshot so their writes cannot become part
        # of the author's input or the candidate's certifying checks.
        baseline_digest = worktree_fingerprint_safe(wt)
        baseline_revision = snapshot_commit(wt, meta["snapshot_commit"])
        baseline_wt, baseline_meta = create_workspace(
            REPO, run_dir / f"red-{round_no}", False, baseline_revision)
        try:
            baseline_record: dict[str, Any] = {"checks": [], "steps": []}
            _execute_checks(baseline_record, check_ids, registry, baseline_wt, args.check_timeout)
            round_record["red_check"] = {
                "purpose": "pre_edit_baseline",
                "input_digest": baseline_digest,
                "scope": "disposable-copy",
                "checks": baseline_record["checks"],
                "statuses": _status_counts(baseline_record["checks"]),
                "authoritative": False,
                "note": "Baseline is diagnostic only: it runs before the author call, in a "
                        "disposable copy, and never certifies the round's result.",
            }
        finally:
            baseline_cleanup = cleanup_workspace(REPO, baseline_wt, False)
            round_record["red_check_workspace"] = baseline_meta
            if baseline_cleanup.status != "passed":
                record["steps"].append(asdict(StepResult(
                    f"red.cleanup.{round_no}", baseline_cleanup.status, baseline_cleanup.detail)))
        require_same_digest(baseline_digest, worktree_fingerprint_safe(wt), "red_check_source_drift")
        require_same_digest(origin_digest, worktree_fingerprint_safe(REPO), "origin_source_drift")
        try:
            result = run_agent(prompt=prompt, cwd=wt, driver=args.driver, timeout=args.agent_timeout,
                               scenario="ok" if args.scenario == "edit" else args.scenario)
            result_meta = {"status": result.status, "returncode": result.returncode,
                           "duration_seconds": result.duration_seconds, "identity": args.author_identity,
                           "role": "author", "round": round_no, "driver": args.driver}
            write_json(run_dir / f"agent-result-{round_no}.json", result_meta)
            write_json(run_dir / "agent-result.json", result_meta)
            agent_ok = result.status == "OK"
        except Exception:
            agent_ok = False
        if agent_ok and (args.scenario == "edit" or round_no > 1):
            mock_develop_edit(wt, round_no)
        record["steps"].append(asdict(StepResult(f"agent.turn.{round_no}", "passed" if agent_ok else "failed",
                                                "" if agent_ok else "agent_failed")))
        digest = worktree_fingerprint_safe(wt)
        round_record["input_digest"] = digest
        record["fingerprint_before"] = record["fingerprint"] = digest
        if not agent_ok:
            record["checks"].extend(asdict(CheckResult(cid, "blocked", reason="agent_failed")) for cid in check_ids)
            return
        if previous_digest == digest:
            raise PipelineError("revision_did_not_change_source")
        previous_digest = digest
        if plan_baselines(wt) != record["plan_baselines"]:
            raise PipelineError("requirement_plan_changed")
        start = len(record["checks"])
        _execute_checks(record, check_ids, registry, wt, args.check_timeout)
        round_record["checks"] = record["checks"][start:]
        round_record["checks_digest"] = digest
        require_same_digest(digest, worktree_fingerprint_safe(wt), "source_drift")
        if any(c["status"] != "passed" for c in round_record["checks"]):
            round_record["review_status"] = "blocked_by_checks"
            return
        revision = snapshot_commit(wt, meta["snapshot_commit"])
        round_record["snapshot_commit"] = revision
        review_wt, review_meta = create_workspace(REPO, run_dir / f"review-{round_no}", False, revision)
        round_record["review_workspace"] = review_meta
        try:
            require_same_digest(digest, worktree_fingerprint_safe(review_wt), "review_snapshot_mismatch")
            readonly_source(review_wt, True)
            review_result = run_agent(prompt=f"Read-only review of {digest}", cwd=review_wt,
                                      driver=args.review_driver, timeout=args.review_timeout, scenario="ok")
            round_record["review_transport"] = {"status": review_result.status, "driver": args.review_driver,
                                                "identity": args.reviewer_identity, "role": "reviewer"}
            if review_result.status != "OK":
                raise PipelineError("review_driver_failed")
            payload = mock_review(review_wt, args.review_scenario, digest, record["run_id"], round_no, args.reviewer_identity)
            round_record["review"] = payload
            write_json(run_dir / f"review-{round_no}.json", payload)
            after = worktree_fingerprint_safe(review_wt)
            round_record["review_fingerprint_before"] = digest
            round_record["review_fingerprint_after"] = after
            require_same_digest(digest, after, "review_source_modified")
            verdict = validate_review(payload, digest, record["run_id"], round_no,
                                      args.author_identity, args.reviewer_identity)
        finally:
            readonly_source(review_wt, False)
            cleanup = cleanup_workspace(REPO, review_wt, False)
            round_record["review_cleanup"] = asdict(cleanup)
            record["steps"].append(asdict(StepResult(f"review.cleanup.{round_no}", cleanup.status, cleanup.detail)))
        if cleanup.status != "passed":
            raise PipelineError("review_cleanup_incomplete")
        record["steps"].append(asdict(StepResult(f"review.round.{round_no}", "passed", verdict)))
        require_same_digest(digest, worktree_fingerprint_safe(wt), "development_source_drift")
        if verdict == "reject":
            raise PipelineError("review_rejected")
        if verdict == "revise":
            if round_no == args.max_rounds:
                raise PipelineError("revision_limit_exhausted")
            continue
        require_same_digest(origin_digest, worktree_fingerprint_safe(REPO), "origin_source_drift")
        if git("rev-parse", "HEAD", cwd=REPO).stdout.strip() != origin_head:
            raise PipelineError("origin_head_drift")
        local_candidate(run_dir, record, wt, meta["snapshot_commit"], revision, digest, round_record)
        record["steps"].append(asdict(StepResult("candidate.local", "passed")))
        return


def _run_development(args: argparse.Namespace, registry: dict[str, Any]) -> int:
    run_dir, record = _new_run("pipeline")
    wt: Path | None = None
    config_error = False
    duplicate_of: str | None = None
    try:
        record.update(driver=args.driver, review_driver=args.review_driver, requirements=args.requirements,
                      author=args.author_identity, reviewer=args.reviewer_identity,
                      review_kind="deterministic-mock", rounds=[],
                      input_binding=evidence.input_binding(args.trigger, args.source, args.prompt))
        if args.driver not in registry["agents"]["available"] or args.review_driver not in registry["agents"]["available"]:
            config_error = True
            raise PipelineError("unknown_driver")
        try:
            check_ids = resolve_checks(registry, args.requirements)
        except PipelineError:
            config_error = True
            raise
        # Caller text is not authority; no trusted policy verifier exists yet.
        # Never persist preauth, consult credentials or invoke real drivers here.
        authorized = args.driver == "mock" and args.review_driver == "mock"
        if not authorized:
            record["steps"].append(asdict(StepResult("preauth", "blocked", "trusted_authorization_policy_unconfigured")))
        elif args.author_identity == args.reviewer_identity:
            raise PipelineError("review_identity_invalid")
        else:
            require_current_registry(registry, REPO)
            origin_digest = source_digest = worktree_fingerprint_safe(REPO)
            # Duplicate input is only meaningful once the request is otherwise
            # valid: an unauthorized driver or a bad argument must still report
            # its own reason instead of being hidden behind "already submitted".
            try:
                duplicate_of = pause.duplicate_input(EVIDENCE_ROOT, record["input_binding"],
                                                     record["run_id"], source_digest)
            except pause.PauseError as exc:
                config_error = True
                raise PipelineError(f"input_history_{exc.code}") from None
            if duplicate_of:
                record["steps"].append(asdict(StepResult("input.duplicate", "blocked",
                                                        f"input_duplicate_paused:{duplicate_of}")))
            else:
                origin_head = git("rev-parse", "HEAD", cwd=REPO).stdout.strip()
                record["origin"] = {"input_digest": origin_digest, "head": origin_head}
                wt, meta = create_workspace(REPO, run_dir, args.keep_workspace)
                record["workspace"] = meta
                require_current_registry(registry, wt)
                record["provenance"] = evidence.provenance(wt)
                record["plan_baselines"] = plan_baselines(wt)
                record["steps"].append(asdict(StepResult("workspace.create", "passed")))
                development_rounds(args, registry, check_ids, run_dir, record, wt, meta, origin_digest, origin_head)
                if args.lesson:
                    try:
                        add_lesson(rule=args.lesson, why=args.lesson_why or "", evidence=record["run_id"])
                        record["steps"].append(asdict(StepResult("lesson.record", "passed")))
                        promo = promotion_candidates(load_lessons())
                        if promo:
                            write_json(run_dir / "promotion-candidates.json", promo)
                    except LessonError:
                        record["steps"].append(asdict(StepResult("lesson.record", "failed", "lesson_invalid")))
    except PipelineError as exc:
        record["steps"].append(asdict(StepResult("configuration" if config_error else "development", "failed", str(exc))))
    except KeyboardInterrupt:
        record["steps"].append(asdict(StepResult("run", "failed", "interrupted")))
    except Exception:
        record["steps"].append(asdict(StepResult("run", "failed", "runner_error")))
    finally:
        if wt is not None:
            if "fingerprint_before" in record:
                _verify_fingerprint(record, wt)
            try:
                retained = bool(record.get("candidate"))
                cleanup = cleanup_workspace(REPO, wt, args.keep_workspace or retained)
                if retained:
                    cleanup.detail = "candidate_workspace_retained"
                    record["workspace"]["cleanup_policy"] = "retained"
                record["steps"].append(asdict(cleanup))
            except Exception:
                record["steps"].append(asdict(StepResult("workspace.cleanup", "failed", "cleanup_error")))
        if duplicate_of:
            record["duplicate_of"] = duplicate_of
        rc = _finish_run(run_dir, record, config_error, registry,
                         notify_failure=args.notify_failure)
    return rc

def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="AgentIsland automation pipeline (local, manual trigger).")
    parser.add_argument("--trigger", default="manual-github", help="manual-github | x-mention (reserved)")
    parser.add_argument("--source", default="", help="free text: issue URL / x post id / operator note")
    parser.add_argument("--driver", default="mock", help="registry agents.available")
    parser.add_argument("--prompt", default="No-op: dry run.", help="agent prompt")
    parser.add_argument("--scenario", default=None, help="mock scenario: ok|auth|timeout|edit")
    parser.add_argument("--review-driver", default="mock", help="independent review driver; real drivers blocked pending trusted policy")
    parser.add_argument("--author-identity", default="mock-author")
    parser.add_argument("--reviewer-identity", default="mock-reviewer")
    parser.add_argument("--review-scenario", default="pass", choices=["pass", "revise-once", "revise-always", "reject", "stale", "mutate", "timeout"])
    parser.add_argument("--review-timeout", type=int, default=900)
    parser.add_argument("--max-rounds", type=int, default=3)
    parser.add_argument("--requirements", nargs="*", default=["AUTO-01"], help="requirement ids from registry")
    parser.add_argument(
        "--profile",
        default=None,
        help="run a registry-defined check profile (fast|ci|native); skips agent turn and workspace",
    )
    parser.add_argument("--list-profiles", action="store_true", help="validate and list profiles without running")
    parser.add_argument("--validate-registry", action="store_true", help="validate registry without running")
    parser.add_argument("--agent-timeout", type=int, default=900)
    parser.add_argument("--check-timeout", type=int, default=900,
                        help="default per-check seconds; effective budget is max(this value, "
                             "the check's registry timeout_seconds), including explicit overrides")
    parser.add_argument("--preauth", default=None, help="legacy authorization hint; never grants execution authority")
    parser.add_argument("--keep-workspace", action="store_true")
    parser.add_argument("--notify-failure", action="store_true",
                        help="send a host notification when a run fails or pauses; "
                             "the pause record is always written regardless")
    parser.add_argument("--lesson", default=None, help="record a lesson rule text")
    parser.add_argument("--lesson-why", default="")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        return run_pipeline(args)
    except PipelineError as exc:
        print(json.dumps({"outcome": "failed", "reason": str(exc)}, ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
