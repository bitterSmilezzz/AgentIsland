#!/usr/bin/env python3
"""Bounded CLI adapters; only verified final answers enter AgentResult.response.

run_agent / AgentResult retain their existing API. Diagnostics are fixed codes;
stdout/stderr and prompt-bearing argv never become error evidence. CLI JSON is a
metadata projection; successful response text is available only to Python callers.
"""
from __future__ import annotations

import json
import math
import os
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any

MAX_OUTPUT_BYTES = 1024 * 1024  # Combined stdout + stderr, counted before decoding.
READ_BYTES = 16384


class DriverError(RuntimeError):
    """Invalid caller contract; messages must be fixed, never echo input."""


@dataclass
class AgentResult:
    driver: str
    status: str
    response: str = ""
    error: str = ""
    duration_seconds: float = 0.0
    conversation_id: str = ""
    usage: dict[str, int] = field(default_factory=dict)
    command: list[str] = field(default_factory=list)
    cwd: str = ""
    returncode: int | None = None

    def to_json(self) -> str:
        """Internal API serialization; successful response may contain private text."""
        return json.dumps(asdict(self), ensure_ascii=False, indent=2)


def _cleanup_group(proc: subprocess.Popen) -> None:
    """Kill the owned session even if its leader has already exited.

    Descendants may retain the pipes after the leader exits or ignore TERM. Always
    send KILL to the group after a bounded grace period and reap the direct child.
    No PID discovery or unrelated process matching is involved.
    """
    for sig in (signal.SIGTERM, signal.SIGKILL):
        # Reap an exited leader before signaling: macOS can return EPERM for a
        # group containing only an unreaped zombie. Still signal live descendants.
        proc.poll()
        try:
            os.killpg(proc.pid, sig)
        except ProcessLookupError:
            break
        if sig == signal.SIGTERM:
            time.sleep(0.1)
    proc.wait(timeout=2)


def _run(command: list[str], cwd: Path, timeout: int, stdin_text: str | None) -> tuple[int, str, str]:
    """Return (rc, bounded stdout, fixed code); never retain stderr.

    POSIX process groups provide the cleanup guarantee. Unsupported hosts fail
    closed before launching rather than silently killing only a parent.
    """
    if os.name != "posix":
        return 126, "", "process_groups_unavailable"
    proc = None
    output = bytearray()
    total = 0
    started = time.monotonic()
    try:
        with selectors.DefaultSelector() as selector:
            proc = subprocess.Popen(command, cwd=str(cwd),
                                    stdin=subprocess.PIPE if stdin_text is not None else subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    start_new_session=True)
            for pipe in (proc.stdout, proc.stderr):
                os.set_blocking(pipe.fileno(), False)
                selector.register(pipe, selectors.EVENT_READ)
            pending = memoryview(stdin_text.encode("utf-8")) if stdin_text is not None else None
            if pending is not None:
                os.set_blocking(proc.stdin.fileno(), False)
                selector.register(proc.stdin, selectors.EVENT_WRITE)
            while selector.get_map() or proc.poll() is None:
                remaining = timeout - (time.monotonic() - started)
                if remaining <= 0:
                    return 124, "", "timeout"
                for key, _ in selector.select(min(remaining, 0.05)):
                    pipe = key.fileobj
                    if pipe is proc.stdin:
                        try:
                            count = os.write(pipe.fileno(), pending[:READ_BYTES]) if pending else 0
                            pending = pending[count:]
                        except BrokenPipeError:
                            pending = None
                        if not pending:
                            selector.unregister(pipe)
                            pipe.close()
                        continue
                    try:
                        chunk = os.read(pipe.fileno(), READ_BYTES)
                    except BlockingIOError:
                        continue
                    if not chunk:
                        selector.unregister(pipe)
                        continue
                    total += len(chunk)
                    if total > MAX_OUTPUT_BYTES:
                        return 125, "", "output_limit"
                    if pipe is proc.stdout:
                        output.extend(chunk)
            rc = proc.wait()
            if rc != 0:
                return rc, "", "process_failed"
            try:
                return rc, output.decode("utf-8", errors="strict"), ""
            except UnicodeDecodeError:
                return rc, "", "invalid_encoding"
    except KeyboardInterrupt:
        return 130, "", "interrupted"
    except FileNotFoundError:
        return 127, "", "command_unavailable"
    except (OSError, ValueError):
        return 126, "", "execution_failed"
    finally:
        if proc is not None:
            # A second SIGINT must not interrupt descendant reclamation. Restore
            # the caller's mask afterward; a pending interrupt is then delivered.
            previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT})
            try:
                _cleanup_group(proc)
            finally:
                try:
                    for pipe in (proc.stdin, proc.stdout, proc.stderr):
                        if pipe is not None:
                            pipe.close()
                finally:
                    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)


def _pairs(pairs: list[tuple[str, Any]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate_key")
        result[key] = value
    return result


def _reject_constant(value: str) -> None:
    raise ValueError("invalid_constant")


def _finite_float(text: str) -> float:
    value = float(text)
    if not math.isfinite(value):
        raise ValueError("invalid_number")
    return value


def _json(text: str) -> dict:
    value = json.loads(text, object_pairs_hook=_pairs, parse_constant=_reject_constant,
                       parse_float=_finite_float)
    if not isinstance(value, dict):
        raise ValueError("invalid_object")
    return value


def _usage(value: Any) -> dict[str, int]:
    if not isinstance(value, dict) or any(type(v) is not int or v < 0 for v in value.values()):
        raise ValueError("invalid_usage")
    return value.copy()


def _text_fields(value: dict, names: tuple[str, ...]) -> None:
    for name in names:
        if name in value and not isinstance(value[name], str):
            raise ValueError("invalid_text")


def _nonnegative(value: Any) -> bool:
    return type(value) is int and value >= 0


def _agy_final(text: str) -> tuple[str, str, dict[str, int]]:
    payload = _json(text)
    _text_fields(payload, ("status", "response", "error", "conversation_id"))
    usage = _usage(payload["usage"]) if "usage" in payload else {}
    if payload.get("status") != "OK" or payload.get("error"):
        raise ValueError("failed_result")
    response = payload.get("response")
    if not isinstance(response, str) or not response.strip():
        raise ValueError("missing_final")
    return response, payload.get("conversation_id", ""), usage


def _hermes_final(text: str) -> tuple[str, str, dict[str, int]]:
    """Local hermes_cli/stream_json.py contract: one last type=result envelope.

    Known nonterminal events are schema-checked but never used as the answer.
    Unknown types, malformed lines, duplicate/late finals and invalid usage fail
    closed. Optional metadata accepts only documented field types.
    """
    final = None
    for line in text.splitlines():
        if not line.strip():
            continue
        event = _json(line)
        if final is not None:
            raise ValueError("late_event")
        _text_fields(event, ("type", "session_id", "error", "tool_call_id"))
        for name in ("timestamp", "duration_ms"):
            if name in event and not _nonnegative(event[name]):
                raise ValueError("invalid_metadata")
        for name in ("usage", "tokens"):
            if name in event:
                _usage(event[name])
        kind = event.get("type")
        if kind == "result":
            _text_fields(event, ("text",))
            if not _nonnegative(event.get("exit_code")) or event["exit_code"] != 0 or event.get("error"):
                raise ValueError("failed_result")
            if not isinstance(event.get("text"), str) or not event["text"].strip():
                raise ValueError("missing_final")
            # Hermes emits tokens (input/output/total/cache_read/cache_write), not usage.
            final = (event["text"], event.get("session_id", ""), _usage(event.get("tokens")))
        elif kind == "system":
            _text_fields(event, ("subtype", "model"))
            if event.get("subtype") != "init":
                raise ValueError("invalid_system")
        elif kind == "text":
            if not isinstance(event.get("text"), str):
                raise ValueError("invalid_delta")
        elif kind in ("tool_use", "tool_result"):
            if not isinstance(event.get("name"), str) or not event["name"]:
                raise ValueError("invalid_tool")
            if kind == "tool_use" and "input" in event and not isinstance(event["input"], dict):
                raise ValueError("invalid_tool_input")
            if kind == "tool_result":
                if not isinstance(event.get("output"), str) or type(event.get("is_error")) is not bool:
                    raise ValueError("invalid_tool_output")
                if not _nonnegative(event.get("duration_ms")):
                    raise ValueError("invalid_tool_duration")
        else:
            raise ValueError("unknown_event")
    if final is None:
        raise ValueError("missing_final")
    return final


def _invoke(name: str, command: list[str], cwd: Path, timeout: int, started: float) -> AgentResult:
    result = AgentResult(driver=name, status="ERROR", command=[name, "<redacted>"], cwd=str(cwd))
    try:
        rc, out, diagnostic = _run(command, cwd, timeout, None)
        result.returncode = rc
        if diagnostic or rc != 0:
            # Only internal fixed codes, never arbitrary diagnostic text.
            codes = {"timeout", "output_limit", "interrupted", "command_unavailable",
                     "execution_failed", "invalid_encoding", "process_groups_unavailable", "process_failed"}
            result.error = diagnostic if diagnostic in codes else "process_failed"
        else:
            response, conversation, usage = (_agy_final(out) if name == "agy" else _hermes_final(out))
            result.response, result.conversation_id, result.usage = response, conversation, usage
            result.status = "OK"
    except KeyboardInterrupt:
        result.error, result.returncode = "interrupted", 130
    except (ValueError, TypeError, RecursionError, OverflowError):
        result.error = "invalid_output"
    except (OSError, subprocess.SubprocessError):
        result.error = "execution_failed"
    finally:
        result.duration_seconds = round(time.monotonic() - started, 3)
    return result


# Explicit extension seam for deterministic scenarios; unknown names always reject.
MOCK_SCENARIOS = {
    "ok": ("OK", "mock response: acknowledged", "", 0, {"input_tokens": 3, "output_tokens": 5}),
    "auth": ("ERROR", "", "authentication_failed", 2, {"input_tokens": 0, "output_tokens": 0}),
    "timeout": ("ERROR", "", "timeout", 124, {}),
}


def run_mock(prompt: str, cwd: Path, timeout: int, scenario: str | None) -> AgentResult:
    started = time.monotonic()
    key = scenario or "ok"
    if key not in MOCK_SCENARIOS:
        raise DriverError("unknown_mock_scenario")
    status, response, error, rc, usage = MOCK_SCENARIOS[key]
    return AgentResult(driver="mock", status=status, response=response, error=error,
                       duration_seconds=round(time.monotonic() - started, 3),
                       conversation_id="mock-conv-0001", usage=usage.copy(),
                       command=["mock", "<redacted>"], cwd=str(cwd), returncode=rc)


def run_agy(prompt: str, cwd: Path, timeout: int, model: str | None = None) -> AgentResult:
    # Existing --print= transport retained: this installed binary's stdin contract
    # has not been established. Real argv contains prompt; result.command does not.
    command = ["agy", "--print=" + prompt, "--output-format", "json", "--mode", "accept-edits"]
    if model:
        command += ["--model", model]
    return _invoke("agy", command, cwd, timeout, time.monotonic())


def run_hermes(prompt: str, cwd: Path, timeout: int, model: str | None = None) -> AgentResult:
    """Exclusive 0600 query transport; do not implicitly authorize shell hooks.

    Local source also supports --query-file - (stdin). Keep the existing file
    transport for compatible installations; never reuse a caller-owned filename.
    """
    started = time.monotonic()
    result = AgentResult(driver="hermes", status="ERROR", command=["hermes", "<redacted>"], cwd=str(cwd))
    tmp = None
    try:
        # Preflight before prompt persistence. Popen remains the authoritative check.
        if not cwd.is_dir():
            result.error = "invalid_cwd"
            return result
        if shutil.which("hermes") is None:
            result.error, result.returncode = "command_unavailable", 127
            return result
        fd, path = tempfile.mkstemp(prefix=".agentisland-query-", suffix=".txt", dir=cwd)
        tmp = Path(path)
        with os.fdopen(fd, "w", encoding="utf-8") as query:
            # mkstemp uses O_EXCL and mode 0600 at creation, independent of umask.
            query.write(prompt)
        command = ["hermes", "chat", "--query-file", str(tmp), "--oneshot", "--format", "stream-json"]
        if model:
            command += ["-m", model]
        result = _invoke("hermes", command, cwd, timeout, started)
    except KeyboardInterrupt:
        result.error, result.returncode = "interrupted", 130
    except (OSError, ValueError, UnicodeError):
        result.error = "query_failed"
    finally:
        if tmp is not None:
            try:
                tmp.unlink()
            except FileNotFoundError:
                pass
            except OSError:
                result.status, result.error = "ERROR", "query_cleanup_failed"
                result.response, result.conversation_id, result.usage = "", "", {}
        result.duration_seconds = round(time.monotonic() - started, 3)
    return result


DRIVERS = {"mock": run_mock, "agy": run_agy, "hermes": run_hermes}


def run_agent(prompt: str, cwd: Path, driver: str = "mock", timeout: int = 900,
              scenario: str | None = None, model: str | None = None,
              argv_extra: list[str] | None = None) -> AgentResult:
    if driver not in DRIVERS:
        raise DriverError("unknown_driver")
    if argv_extra:
        raise DriverError("argv_extra_unsupported")
    if (type(timeout) not in (int, float) or timeout <= 0
            or (type(timeout) is float and not math.isfinite(timeout))):
        raise DriverError("invalid_timeout")
    if not isinstance(prompt, str) or not prompt.strip() or "\0" in prompt:
        raise DriverError("invalid_prompt")
    if model is not None and (not isinstance(model, str) or not model.strip() or "\0" in model):
        raise DriverError("invalid_model")
    if scenario is not None and not isinstance(scenario, str):
        raise DriverError("invalid_scenario")
    fn = DRIVERS[driver]
    if driver == "mock":
        return fn(prompt, cwd, timeout, scenario)
    return fn(prompt, cwd, timeout, model)


def main() -> int:
    import argparse

    # argparse normally echoes invalid input; never publish caller text on errors.
    class Parser(argparse.ArgumentParser):
        def error(self, message):
            raise DriverError("invalid_arguments")

    parser = Parser(description="Run one agent turn; print metadata only.")
    parser.add_argument("--driver", default="mock", choices=sorted(DRIVERS))
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--cwd", default=".")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--scenario", default=None)
    parser.add_argument("--model", default=None)
    try:
        args = parser.parse_args()
        result = run_agent(prompt=args.prompt, cwd=Path(args.cwd).resolve(), driver=args.driver,
                           timeout=args.timeout, scenario=args.scenario, model=args.model)
    except DriverError as exc:
        print(json.dumps({"status": "ERROR", "error": str(exc)}))
        return 2
    except (OSError, ValueError):
        print(json.dumps({"status": "ERROR", "error": "invalid_arguments"}))
        return 2
    print(json.dumps({"driver": result.driver, "status": result.status, "error": result.error,
                      "returncode": result.returncode, "duration_seconds": result.duration_seconds}))
    return 0 if result.status == "OK" else 1


if __name__ == "__main__":
    sys.exit(main())
