#!/usr/bin/env python3
"""Pause records for failed runs, and duplicate-input detection.

A failing run must leave a durable, reviewable record instead of only an exit
code, and re-submitting the same material must not silently re-run work. Nothing
here is sent anywhere by itself: notification is explicit, and every delivery
failure degrades to a log line plus a `delivered: false` record.
"""
from __future__ import annotations

import hashlib
import json
import os
import platform
import re
import stat
import uuid
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
MAX_NOTIFY_TITLE = 120
MAX_NOTIFY_BODY = 1000
MAX_LOG_BYTES = 1 << 20
NOTIFY_TIMEOUT = 20
HEX = re.compile(r"[0-9a-f]{64}\Z")
# Only these trigger kinds may be reported; unknown input is never persisted.
TRIGGERS = frozenset(("manual-github", "manual", "x-mention", "selftest", "unknown"))
STAGES = frozenset(("configuration", "development", "report", "profile", "agent", "run",
                    "input", "source", "review", "candidate", "workspace", "lesson",
                    "check", "dependency", "artifact", "ev", "digest", "preauth",
                    "cleanup", "snapshot", "bind", "publish", "patch"))


class PauseError(RuntimeError):
    """Fixed code only; never carries source text, prompts or file contents."""

    def __init__(self, code: str):
        assert re.fullmatch(r"[a-z0-9_]+", code)
        self.code = code
        super().__init__(code)


def _now() -> str:
    import datetime as dt
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def binding_key(binding: Any) -> str:
    """Stable identity of one input submission: digests only, never raw text."""
    if not isinstance(binding, dict):
        raise PauseError("binding_invalid")
    trigger = binding.get("trigger")
    parts = [binding.get("source_sha256"), binding.get("prompt_sha256")]
    if not isinstance(trigger, str) or trigger not in TRIGGERS:
        raise PauseError("binding_invalid")
    for value in parts:
        if not isinstance(value, str) or not HEX.match(value):
            raise PauseError("binding_invalid")
    snapshot = binding.get("snapshot_sha256")
    if snapshot is not None and (not isinstance(snapshot, str) or not HEX.match(snapshot)):
        raise PauseError("binding_invalid")
    payload = json.dumps([trigger, *parts, snapshot], separators=(",", ":"))
    return hashlib.sha256(payload.encode("ascii")).hexdigest()


def _load_run(path: Path) -> dict:
    """Strict read of one run record; unreadable evidence fails closed."""
    if path.is_symlink() or not path.is_file():
        raise PauseError("run_evidence_invalid")
    if path.stat().st_size > (4 << 20):
        raise PauseError("run_evidence_invalid")
    try:
        data = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=_no_duplicates)
    except (OSError, ValueError, RecursionError):
        raise PauseError("run_evidence_invalid") from None
    if not isinstance(data, dict):
        raise PauseError("run_evidence_invalid")
    return data


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def duplicate_input(evidence_root: Path, binding: Any, run_id: str,
                    source_digest: str | None = None) -> str | None:
    """Return the earlier run id when this exact input already ran on this source.

    Fail-closed: an unreadable run directory blocks the new run instead of
    letting a broken ledger read as "no duplicate". Re-running the same material
    against changed source is a legitimate retry, so identity also covers the
    source fingerprint when the caller knows it.
    """
    key = binding_key(binding)
    if source_digest is not None and not re.fullmatch(r"[0-9a-f]{64}", source_digest):
        raise PauseError("source_digest_invalid")
    if not evidence_root.is_dir():
        raise PauseError("evidence_root_unreadable")
    for directory in sorted(evidence_root.iterdir()):
        if not directory.is_dir() or directory.name == run_id:
            continue
        path = directory / "run.json"
        # A record that is unreadable, foreign or predating input binding simply
        # carries no duplicate identity. Only a present, well-formed binding
        # counts; failing closed here would block every run on any damaged file.
        if path.is_symlink() or not path.is_file():
            continue
        try:
            run = _load_run(path)
        except PauseError:
            continue
        recorded = run.get("input_binding")
        if not isinstance(recorded, dict):
            continue
        if binding_key(recorded) != key:
            continue
        # A request is only settled by a run that actually delivered a candidate.
        # Failed, blocked or candidate-less runs left work unfinished, so
        # re-submitting the same material is a legitimate retry - blocking it
        # would turn every failure into a permanently stuck request.
        if run.get("outcome") != "passed" or run.get("returncode") != 0:
            continue
        if not isinstance(run.get("candidate"), dict) and not (directory / "candidate.json").is_file():
            continue
        origin = run.get("origin")
        previous = origin.get("input_digest") if isinstance(origin, dict) else None
        if source_digest is not None and isinstance(previous, str) and previous != source_digest:
            continue  # Same request, different source state: a real retry.
        return run.get("run_id") or directory.name
    return None


def notify_command(title: str, body: str) -> list[str]:
    """Pick the host's notification channel; absence is a reported state."""
    text = title if platform.system() == "Darwin" else f"{title}\n{body}"
    if platform.system() == "Darwin":
        escaped = text.replace("\\", "\\\\").replace('"', '\\"')
        return ["osascript", "-e", f'display notification "{escaped}" with title "AgentIsland"']
    if platform.system() == "Linux":
        return ["notify-send", "--app-name=AgentIsland", title, body]
    if platform.system() == "Windows":
        return ["powershell", "-NoProfile", "-Command",
                f"[void][Windows.UI.Notifications.ToastNotificationManager]::"
                f"CreateToastNotifier('AgentIsland').Show("
                f"[Windows.UI.Notifications.ToastNotificationManager]::"
                f"GetTemplateContent(0))"]
    return []


def deliver(log_path: Path, title: str, body: str) -> dict:
    """Send only when explicitly enabled; always record the outcome."""
    result = {"channel": platform.system() or "unknown", "delivered": False, "detail": "not_requested"}
    if not title.strip() or not body.strip():
        raise PauseError("notify_text_invalid")
    if len(title) > MAX_NOTIFY_TITLE or len(body) > MAX_NOTIFY_BODY:
        raise PauseError("notify_text_too_long")
    command = notify_command(title, body)
    if not command:
        result["detail"] = "no_channel"
        result["log"] = _append_log(log_path, "no notification channel on this host")
        return result
    try:
        completed = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True,
                                   timeout=NOTIFY_TIMEOUT, check=False)
    except FileNotFoundError:
        result["detail"] = "channel_unavailable"
        result["log"] = _append_log(log_path, "notification channel unavailable")
        return result
    except subprocess.TimeoutExpired:
        result["detail"] = "channel_timeout"
        result["log"] = _append_log(log_path, "notification channel timed out")
        return result
    except OSError:
        result["detail"] = "channel_failed"
        result["log"] = _append_log(log_path, "notification channel failed")
        return result
    if completed.returncode != 0:
        result["detail"] = "channel_rejected"
        result["log"] = _append_log(log_path, "notification channel rejected")
        return result
    result["delivered"] = True
    result["detail"] = "delivered"
    return result


def _anchored() -> bool:
    """Windows lacks O_NOFOLLOW and dir_fd support; callers must fail loudly, not silently."""
    return (os.open in os.supports_dir_fd and hasattr(os, "O_DIRECTORY")
            and hasattr(os, "O_NOFOLLOW") and os.O_NOFOLLOW)


def _directory_fd(path: Path) -> int:
    """Walk anchored directory descriptors, rejecting symlinks at every level."""
    if not _anchored():
        raise PauseError("anchored_write_unsupported")
    absolute = Path(os.path.abspath(path))
    # macOS owns these two fixed aliases. Canonicalize only their verified
    # system mapping; arbitrary caller-controlled parent links remain refused.
    if sys.platform == "darwin" and len(absolute.parts) > 1:
        alias = absolute.parts[1]
        if alias in ("tmp", "var"):
            expected = Path("/private") / alias
            if Path("/" + alias).resolve() == expected:
                absolute = expected.joinpath(*absolute.parts[2:])
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    fd = os.open(absolute.anchor, flags)
    try:
        for part in absolute.parts[1:]:
            try:
                os.mkdir(part, 0o700, dir_fd=fd)
            except FileExistsError:
                pass
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def _atomic_json_unanchored(path: Path, payload: Any) -> None:
    """Weaker guarantee: exclusive create + atomic replace, no symlink refusal.

    Callers must record this limitation; it exists only so a failure still
    leaves a durable record where anchored writes are unavailable.
    """
    temporary = path.parent / f"{path.name}.new.{os.getpid()}.{uuid.uuid4().hex}"
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except (OSError, ValueError, TypeError):
        try:
            temporary.unlink()
        except OSError:
            pass
        raise PauseError("pause_write_failed") from None


def atomic_json(path: Path, payload: Any) -> None:
    """Publish through a private exclusive temporary file in an anchored directory.

    Where anchored descriptors are unavailable (non-POSIX), fall back to a
    plain exclusive create + replace and mark the record as weakly guaranteed:
    a failure record that cannot be written at all is worse than one that
    cannot prove it refused symlinked parents.
    """
    if not _anchored():
        _atomic_json_unanchored(path, payload)
        return
    directory = None
    temporary = f"{path.name}.new.{os.getpid()}.{uuid.uuid4().hex}"
    created = False
    try:
        directory = _directory_fd(path.parent)
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o600, dir_fd=directory)
        created = True
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path.name, src_dir_fd=directory, dst_dir_fd=directory)
        os.fsync(directory)
    except (OSError, ValueError, TypeError):
        raise PauseError("pause_write_failed") from None
    finally:
        if directory is not None:
            if created:
                try:
                    os.unlink(temporary, dir_fd=directory)
                except FileNotFoundError:
                    pass
            os.close(directory)


def _append_log(path: Path, line: str) -> dict:
    """Return a fixed-code receipt even when degradation logging fails."""
    if not _anchored():
        # No symlink refusal without anchored descriptors; report that plainly
        # rather than implying the stronger guarantee.
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
            with os.fdopen(fd, "ab") as handle:
                handle.write(f"{_now()} {line}\n".encode("utf-8"))
            return {"written": True, "detail": "log_written_unanchored"}
        except OSError:
            return {"written": False, "detail": "log_write_failed"}
    directory = None
    fd = None
    try:
        directory = _directory_fd(path.parent)
        fd = os.open(path.name, os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW
                     | os.O_NONBLOCK, 0o600, dir_fd=directory)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode):
            return {"written": False, "detail": "log_not_regular"}
        os.fchmod(fd, 0o600)
        data = f"{_now()} {line}\n".encode("utf-8")
        if info.st_size + len(data) > MAX_LOG_BYTES:
            return {"written": False, "detail": "log_size_limit"}
        with os.fdopen(fd, "ab") as handle:
            fd = None
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        return {"written": True, "detail": "log_written"}
    except (OSError, PauseError):
        # Logging may never break the pause record it is supposed to explain.
        return {"written": False, "detail": "log_write_failed"}
    finally:
        if fd is not None:
            os.close(fd)
        if directory is not None:
            os.close(directory)


def pause_record(run_dir: Path, record: dict, *, stage: str, log_path: Path,
                 notify: bool = False, detail: str = "") -> dict:
    """Write the durable pause record for a run that must not be forgotten."""
    if stage not in STAGES:
        raise PauseError("stage_invalid")
    summary = {
        "schema_version": SCHEMA_VERSION,
        "run_id": record.get("run_id"),
        "stage": stage,
        "outcome": record.get("outcome", "failed"),
        "returncode": record.get("returncode"),
        "failed_or_blocked": list(record.get("failed_or_blocked", [])),
        "detail": detail or stage,
        "input_binding": record.get("input_binding"),
        "recorded_at": _now(),
    }
    if record.get("evidence") and isinstance(record["evidence"], str):
        summary["evidence"] = Path(record["evidence"]).name
    summary["notify_log"] = log_path.name
    # Self-report the durability guarantee so a reader never assumes symlink
    # refusal that this host cannot provide.
    summary["write_guarantee"] = "anchored_dir_fd" if _anchored() else "unanchored_no_symlink_refusal"
    if notify:
        # Notification is a bounded summary, not the record. A long failure list
        # must never make the pause record unwritable or turn a blocked run into
        # a failed one: truncate the body and keep the full list in pause.json.
        title = f"自动化运行暂停 {summary['run_id']}"
        failed = summary["failed_or_blocked"]
        body = f"阶段 {stage}；未通过项 {len(failed)} 项：{', '.join(failed) or 'unknown'}"
        if len(body) > MAX_NOTIFY_BODY:
            kept, used = [], len(f"阶段 {stage}；未通过项 {len(failed)} 项：…")
            for name in failed:
                if used + len(name) + 2 > MAX_NOTIFY_BODY:
                    break
                kept.append(name)
                used += len(name) + 2
            body = (f"阶段 {stage}；未通过项 {len(failed)} 项（仅列前 {len(kept)} 项）："
                    + ", ".join(kept))
        try:
            summary["notification"] = deliver(log_path, title, body)
        except PauseError as exc:
            summary["notification"] = {"channel": platform.system() or "unknown",
                                       "delivered": False, "detail": f"notify_{exc.code}"}
    else:
        summary["notification"] = {"channel": platform.system() or "unknown",
                                   "delivered": False, "detail": "not_requested"}
    atomic_json(run_dir / "pause.json", summary)
    return summary


def wait_for_idle(path: Path, timeout: float = 2.0) -> None:  # pragma: no cover - helper
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        return


if __name__ == "__main__":  # pragma: no cover - module is imported by the runner
    sys.exit("pause.py is a library; the pipeline entry point remains pipeline.py")
