#!/usr/bin/env python3
"""复盘账本共享逻辑（AUTO-02）。

pipeline、selftest 与 CLI 都用这里的实现，避免三份逻辑各自漂移。

设计要点（写入口径与 ADR 0033「写入前校验并原子发布」一致）：

- 缺陷键：纯 ASCII 规则**沿用历史算法**，键字节不变；中文/Unicode 规则走 Unicode
  归一化（NFKC + casefold，保留各语言字母数字）。凡归一化会被截断、或含非 ASCII
  字符而无法与历史键安全共存时，追加规则文本的 bounded SHA-256 后缀——同一文本
  恒得同一键（跨日期聚合仍成立），不同文本极难碰撞。
- 晋升阈值按**不同日期**（真实日历日）计数：同一天写一百条也凑不满三次；每个计入
  的日期必须绑定非空证据，候选会把「日期 → 证据」原样带出。
- seen_date 必须是真实日历日（2026-99-99 拒），recorded_at 必须是 UTC 时刻。
- 坏账本一律 `LessonError` + 固定 code：坏 JSON、非 object、空行、超长行、超上限、
  未知/缺失字段、未来 schema。**保留原字节、不跳行、不静默清空、不覆盖重写**。
- append 幂等 by (key, seen_date, evidence)：同身份同内容 = no-op；同身份不同内容 =
  拒绝（`duplicate_conflict`）。跨进程 flock + O_NOFOLLOW 打开 + 同目录独占临时文件
  + fsync + os.replace，落盘权限 0600。
"""

from __future__ import annotations

import datetime as dt
import errno
import hashlib
import json
import os
import re
import tempfile
import time
import unicodedata
from pathlib import Path

try:
    import fcntl
except ImportError:  # pragma: no cover - Windows
    fcntl = None  # type: ignore[assignment]

try:
    import msvcrt
except ImportError:  # pragma: no cover - POSIX
    msvcrt = None  # type: ignore[assignment]

# ---- 明示上限（无隐藏阈值） -------------------------------------------------
SCHEMA_VERSION = 1
MAX_LEDGER_BYTES = 1 << 20  # 整份账本 1 MiB
MAX_LINE_BYTES = 16 << 10  # 单行（含换行前）；容纳 rule+why+evidence 三个上限字段
KEY_MAX = 80  # 键长度（字符）
RULE_MAX = 1000  # rule 字段字符数
WHY_MAX = 1000  # why 字段字符数
EVIDENCE_MAX = 200  # evidence 字段字符数
HASH_SUFFIX_CHARS = 8  # 碰撞后缀（sha256 十六进制前缀）长度
LOCK_TIMEOUT_SECONDS = 10.0

SCHEMA_FIELDS = ("v", "key", "rule", "why", "evidence", "seen_date", "recorded_at")
REQUIRED_FIELDS = ("v", "key", "rule", "seen_date", "recorded_at")

# 键：首字符为任意语言的字母数字，其后允许字母数字 / 下划线 / 点 / 连字符。
KEY_RE = re.compile(r"^[^\W_][\w.-]{%d,%d}$" % (2, KEY_MAX), re.UNICODE)
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
UTC_RE = re.compile(r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")
_CONTROL_RE = re.compile(r"[\x00-\x1f\x7f]")


class LessonError(ValueError):
    """账本错误。message 只含固定 code（与行号），**不回显**原规则或凭据。"""

    def __init__(self, code: str, *, line: int | None = None):
        self.code = code
        self.line = line
        super().__init__(code if line is None else f"{code} at line {line}")


# ---------------------------------------------------------------- 归一化


def _legacy_key(text: str) -> str:
    """历史算法（纯 ASCII 规则保持字节兼容）。"""
    key = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return re.sub(r"-{2,}", "-", key)


def _bounded_hash(body: str) -> str:
    """碰撞后缀：哈希**归一化后的键体**。

    「同一条规则的不同书写」（大小写、全角/半角、空白与标点差异）得到同一键，
    与历史 ASCII 行为一致；而键体只在截断处相同、尾部不同的两条规则后缀不同，
    截断碰撞由此消歧。
    """
    payload = body[:MAX_LINE_BYTES].encode("utf-8", "surrogatepass")
    return hashlib.sha256(payload).hexdigest()[:HASH_SUFFIX_CHARS]


def _unicode_body(text: str) -> str:
    """保留各语言字母数字，其余（空白/标点/符号）折成单个 '-'。"""
    folded = unicodedata.normalize("NFKC", text).casefold()
    out: list[str] = []
    for ch in folded:
        if ch.isalnum():
            out.append(ch)
        elif out and out[-1] != "-":
            out.append("-")
    return re.sub(r"-{2,}", "-", "".join(out).strip("-"))


def normalize_key(rule: str) -> str:
    """规则文本 -> 稳定键。同文本恒同键；不可归一化时返回空串（由调用方拒绝）。"""
    text = unicodedata.normalize("NFKC", (rule or "").strip())
    if not text:
        return ""
    legacy = _legacy_key(text)
    if text.isascii() and legacy and len(legacy) <= KEY_MAX:
        return legacy  # 既有 ASCII 键保持字节不变
    body = _unicode_body(text)
    if not body:
        return ""  # 全是标点/符号/emoji：没有可用键语义，交调用方拒绝而不是编一个
    stem = body[: KEY_MAX - 1 - HASH_SUFFIX_CHARS]
    return f"{stem}-{_bounded_hash(body)}"


# ---------------------------------------------------------------- 字段校验


def real_date(value: str) -> str:
    """严格真实日历日（2026-99-99 / 2026-02-30 拒）。"""
    if not isinstance(value, str) or not DATE_RE.match(value):
        raise LessonError("invalid_seen_date")
    try:
        dt.date.fromisoformat(value)
    except ValueError:
        raise LessonError("invalid_seen_date") from None
    return value


def utc_instant(value: str) -> str:
    """严格 UTC 时刻（带 Z 或 +00:00；其它时区偏移拒）。"""
    if not isinstance(value, str) or not UTC_RE.match(value):
        raise LessonError("invalid_recorded_at")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00").replace(" ", "T"))
    except ValueError:
        raise LessonError("invalid_recorded_at") from None
    if parsed.tzinfo is None or parsed.utcoffset() != dt.timedelta(0):
        raise LessonError("invalid_recorded_at")
    return value


def _bounded_text(value: object, limit: int, code: str) -> str:
    if not isinstance(value, str) or _CONTROL_RE.search(value) or len(value) > limit:
        raise LessonError(code)
    return value


def _entry_codes(entry: dict) -> list[str]:
    """单条记录的固定 code 列表（不回显任何字段值）。"""
    codes: list[str] = []
    if not isinstance(entry, dict):
        return ["not_object"]
    if set(entry) - set(SCHEMA_FIELDS):
        codes.append("unknown_field")
    for field in REQUIRED_FIELDS:
        if field not in entry:
            codes.append("missing_field")
    if entry.get("v") != SCHEMA_VERSION:
        codes.append("invalid_schema")
    key = entry.get("key")
    if not isinstance(key, str) or not KEY_RE.match(key):
        codes.append("invalid_key")
    rule = entry.get("rule", "")
    if not isinstance(rule, str) or not rule.strip():
        codes.append("empty_rule")
    for field, limit, code in (
        ("rule", RULE_MAX, "invalid_rule"),
        ("why", WHY_MAX, "invalid_why"),
        ("evidence", EVIDENCE_MAX, "invalid_evidence"),
    ):
        value = entry.get(field, "")
        if not isinstance(value, str) or _CONTROL_RE.search(value) or len(value) > limit:
            codes.append(code)
    try:
        real_date(entry.get("seen_date", ""))
    except LessonError:
        codes.append("invalid_seen_date")
    try:
        utc_instant(entry.get("recorded_at", ""))
    except LessonError:
        codes.append("invalid_recorded_at")
    if isinstance(key, str) and key and isinstance(rule, str) and normalize_key(rule) != key:
        codes.append("key_rule_mismatch")
    return codes


# ---------------------------------------------------------------- 构造


def make_entry(rule: str, why: str, evidence: str, seen_date: str, recorded_at: str) -> dict:
    if not isinstance(rule, str) or not rule.strip():
        raise LessonError("empty_rule")
    rule_text = _bounded_text(rule, RULE_MAX, "invalid_rule")
    why_text = _bounded_text(why if why is not None else "", WHY_MAX, "invalid_why")
    evidence_text = _bounded_text(evidence if evidence is not None else "", EVIDENCE_MAX, "invalid_evidence")
    key = normalize_key(rule_text)
    if not key or not KEY_RE.match(key):
        raise LessonError("rule_not_keyable")
    return {
        "v": SCHEMA_VERSION,
        "key": key,
        "rule": rule_text,
        "why": why_text,
        "evidence": evidence_text,
        "seen_date": real_date(seen_date),
        "recorded_at": utc_instant(recorded_at),
    }


def entry_identity(entry: dict) -> tuple[str, str, str]:
    return (
        str(entry.get("key", "")),
        str(entry.get("seen_date", "")),
        str(entry.get("evidence", "")),
    )


# ---------------------------------------------------------------- 读（严格）


def _read_bytes(path: Path) -> bytes:
    """O_NOFOLLOW 读原字节；符号链接与超上限明确拒绝。"""
    if path.is_symlink():
        raise LessonError("ledger_symlink_refused")
    if not path.exists():
        return b""
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as exc:
        if exc.errno in (errno.ELOOP, errno.EMLINK):
            raise LessonError("ledger_symlink_refused") from None
        raise LessonError("ledger_unreadable") from None
    with os.fdopen(fd, "rb") as handle:
        if os.fstat(handle.fileno()).st_size > MAX_LEDGER_BYTES:
            raise LessonError("ledger_too_large")
        data = handle.read(MAX_LEDGER_BYTES + 1)
    if len(data) > MAX_LEDGER_BYTES:
        raise LessonError("ledger_too_large")
    return data


def _parse_bytes(data: bytes) -> list[tuple[int, str, dict]]:
    """严格解析：坏行即 LessonError（行号 + 固定 code），不跳行、不丢内容。"""
    if not data:
        return []
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise LessonError("ledger_not_utf8") from None
    raw_lines = text.split("\n")
    if raw_lines and raw_lines[-1] == "":
        raw_lines.pop()  # 唯一的容忍：结尾换行
    rows: list[tuple[int, str, dict]] = []
    for index, raw in enumerate(raw_lines, start=1):
        if not raw.strip():
            raise LessonError("ledger_blank_line", line=index)
        if len(raw.encode("utf-8")) > MAX_LINE_BYTES:
            raise LessonError("ledger_line_too_long", line=index)
        try:
            entry = json.loads(raw)
        except json.JSONDecodeError:
            raise LessonError("ledger_json_invalid", line=index) from None
        if not isinstance(entry, dict):
            raise LessonError("ledger_not_object", line=index)
        if set(entry) - set(SCHEMA_FIELDS):
            raise LessonError("ledger_unknown_field", line=index)
        for field in REQUIRED_FIELDS:
            if field not in entry:
                raise LessonError("ledger_missing_field", line=index)
        version = entry.get("v")
        if not isinstance(version, int):
            raise LessonError("ledger_invalid_schema", line=index)
        if version > SCHEMA_VERSION:
            raise LessonError("ledger_schema_unsupported", line=index)
        if version != SCHEMA_VERSION:
            raise LessonError("ledger_invalid_schema", line=index)
        rows.append((index, raw, entry))
    return rows


def load_entries(path: Path) -> list[dict]:
    """读账本条目。坏文件**报错**而不是当成空账本（旧版静默吞掉坏 JSON 会假绿）。"""
    return [entry for _, _, entry in _parse_bytes(_read_bytes(Path(path)))]


def parse_ledger_bytes(data: bytes) -> list[dict]:
    return [entry for _, _, entry in _parse_bytes(data)]


# ---------------------------------------------------------------- 锁 + 原子写


class _LedgerLock:
    """跨进程独占锁（flock / msvcrt.locking），锁文件 0600 且 O_NOFOLLOW。"""

    def __init__(self, ledger: Path):
        self.path = ledger.with_name(ledger.name + ".lock")
        self.handle = None

    def __enter__(self) -> "_LedgerLock":
        path = self.path
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_symlink():
            raise LessonError("lock_symlink_refused")
        try:
            fd = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
        except OSError as exc:
            if exc.errno in (errno.ELOOP, errno.EMLINK):
                raise LessonError("lock_symlink_refused") from None
            raise LessonError("lock_unavailable") from None
        os.fchmod(fd, 0o600)
        deadline = time.monotonic() + LOCK_TIMEOUT_SECONDS
        while True:
            try:
                if fcntl is not None:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                elif msvcrt is not None:  # pragma: no cover - Windows
                    msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
                else:  # pragma: no cover - 无锁原语
                    raise LessonError("locking_unsupported")
                break
            except OSError:
                if time.monotonic() >= deadline:
                    os.close(fd)
                    raise LessonError("ledger_lock_timeout") from None
                time.sleep(0.02)
        self.handle = fd
        return self

    def __exit__(self, *exc_info) -> None:
        if self.handle is None:
            return
        try:
            if fcntl is not None:
                fcntl.flock(self.handle, fcntl.LOCK_UN)
            elif msvcrt is not None:  # pragma: no cover - Windows
                os.lseek(self.handle, 0, os.SEEK_SET)
                msvcrt.locking(self.handle, msvcrt.LK_UNLCK, 1)
        finally:
            os.close(self.handle)
            self.handle = None


def _atomic_write(path: Path, data: bytes) -> None:
    """同目录独占临时文件 + fsync + os.replace：要么旧内容完整，要么新内容完整。"""
    directory = path.parent
    directory.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(directory), prefix=f".{path.name}.", suffix=".tmp")
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, path)  # rename 不跟随符号链接：替换链接本身
        try:
            dir_fd = os.open(directory, os.O_RDONLY)
            try:
                os.fsync(dir_fd)
            finally:
                os.close(dir_fd)
        except OSError:
            pass
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


# ---------------------------------------------------------------- 追加（幂等）


def _same_content(left: dict, right: dict) -> bool:
    """幂等内容一致性：比较 key/rule/why/evidence/seen_date，**不比较 recorded_at**。

    重试同一条记录会带新的 recorded_at，那不该算冲突；真正的内容差异（改了 rule 或
    why）必须拒绝而不是覆盖。
    """
    for field in ("key", "rule", "why", "evidence", "seen_date"):
        if left.get(field, "") != right.get(field, ""):
            return False
    return True


def append_entry(path: Path, entry: dict) -> str:
    """幂等追加。返回 "appended" / "duplicate"；同身份不同内容抛 LessonError。"""
    path = Path(path)
    candidate = dict(entry)
    codes = _entry_codes(candidate)
    if codes:
        raise LessonError(codes[0])
    line = json.dumps(candidate, ensure_ascii=False)
    if len(line.encode("utf-8")) > MAX_LINE_BYTES:
        raise LessonError("entry_line_too_long")
    with _LedgerLock(path):
        data = _read_bytes(path)
        rows = _parse_bytes(data)
        for index, _, existing in rows:
            for code in _entry_codes(existing):
                raise LessonError(code if code != "unknown_field" else "ledger_invalid", line=index)
        identity = entry_identity(candidate)
        for _, _, existing in rows:
            if entry_identity(existing) != identity:
                continue
            if _same_content(existing, candidate):
                return "duplicate"
            raise LessonError("duplicate_conflict")
        payload = data
        if payload and not payload.endswith(b"\n"):
            payload += b"\n"
        payload += line.encode("utf-8") + b"\n"
        if len(payload) > MAX_LEDGER_BYTES:
            raise LessonError("ledger_full")
        _atomic_write(path, payload)
    return "appended"


# ---------------------------------------------------------------- 撤销


def revoke_key(path: Path, key: str, reason: str = "") -> int:
    """按 key 撤销条目（人工撤销候选）。明确的文件操作：只重写该文件，不动权限模型。"""
    path = Path(path)
    key = (key or "").strip()
    if not KEY_RE.match(key):
        raise LessonError("invalid_key")
    with _LedgerLock(path):
        data = _read_bytes(path)
        rows = _parse_bytes(data)
        for index, _, existing in rows:
            for code in _entry_codes(existing):
                raise LessonError(code if code != "unknown_field" else "ledger_invalid", line=index)
        kept: list[str] = []
        removed = 0
        for _, raw, existing in rows:
            if existing.get("key") == key:
                removed += 1
            else:
                kept.append(raw)
        if not removed:
            raise LessonError("key_not_found")
        _atomic_write(path, b"".join(raw.encode("utf-8") + b"\n" for raw in kept))
    return removed


# ---------------------------------------------------------------- 晋升 + 校验


def promotion_candidates(entries: list[dict], threshold: int = 3) -> list[dict]:
    """同一 key 在 ≥threshold 个**不同真实日期**、且各日期都有非空证据时才建议升级。"""
    if not isinstance(threshold, int) or threshold < 2:
        threshold = 2  # 同日永远不能凑数
    dates: dict[str, dict[str, set[str]]] = {}
    rules: dict[str, str] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        key = entry.get("key")
        if not isinstance(key, str) or not KEY_RE.match(key):
            continue
        try:
            date = real_date(entry.get("seen_date", ""))
        except LessonError:
            continue
        evidence = entry.get("evidence", "")
        if not isinstance(evidence, str) or not evidence.strip():
            continue
        dates.setdefault(key, {}).setdefault(date, set()).add(evidence)
        rules.setdefault(key, entry.get("rule", ""))
    candidates: list[dict] = []
    for key, per_date in dates.items():
        if len(per_date) < threshold:
            continue
        candidates.append(
            {
                "key": key,
                "rule": rules[key],
                "distinct_dates": sorted(per_date),
                "distinct_dates_count": len(per_date),
                "evidences": {day: sorted(per_date[day]) for day in sorted(per_date)},
            }
        )
    return candidates


def validate(entries: list[dict]) -> list[str]:
    """返回问题列表（固定 code + 行号，不回显规则与凭据）；空列表表示账本可用。"""
    problems: list[str] = []
    seen: dict[tuple[str, str, str], dict] = {}
    for index, entry in enumerate(entries, start=1):
        for code in _entry_codes(entry):
            problems.append(f"line {index}: {code}")
        if not isinstance(entry, dict):
            continue
        identity = entry_identity(entry)
        previous = seen.get(identity)
        if previous is None:
            seen[identity] = entry
        elif previous == entry:
            problems.append(f"line {index}: duplicate_entry")
        else:
            problems.append(f"line {index}: conflicting_entry")
    return problems
