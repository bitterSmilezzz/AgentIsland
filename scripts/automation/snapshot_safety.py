"""Read and vet the entire snapshot before the first Git object write.

Use prepare_snapshot(cwd, exact_git_names), then hash ONLY each returned entry.data
with --no-filters --stdin. Never reopen source paths after this check. This module
writes no files, invokes no scanner script, and reports only fixed error codes.
It recognizes the project's finite scanner patterns, not arbitrary credentials.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import stat
from typing import Iterable


class SnapshotSafetyError(RuntimeError):
    """A fixed, non-sensitive failure code safe for pipeline evidence."""


@dataclass(frozen=True)
class SnapshotEntry:
    name: str = field(repr=False)
    git_mode: str
    data: bytes = field(repr=False)


MAX_FILE_BYTES = 8 * 1024 * 1024
SCANNER = "scripts/scan-secrets.sh"
BASELINE = "scripts/secrets-baseline.txt"
SENSITIVE_NAME = re.compile(
    r"(^|/)(\.env(?:\..*)?|credentials(?:\..*)?|auth\.json|id_rsa|id_ed25519|.*\.(?:pem|key|p12|pfx))$", re.I)
# These are pattern definitions, never credential values. The first four project
# rules also protect synthetic repositories that do not ship the project scanner.
_BUILTIN = (
    ("cred_prefix", rb"(sk|pk|rk)-[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{25,}|ya29\.[0-9A-Za-z_-]{20,}"),
    ("private_key", rb"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    ("secret_assign", rb'''(password|passwd|api[_-]?key|access[_-]?token|auth[_-]?token|secret)["']?[\t\r\v\f ]*[:=][\t\r\v\f ]*["'][^"']{4,}'''),
    ("url_token_param", rb"[?&](access[_-]?token|api[_-]?key|auth[_-]?token|token|key|secret)=[A-Za-z0-9_-]{12,}"),
)
_RULE_IDS = tuple(rule for rule, _ in _BUILTIN) + ("local_user_path", "username", "email", "phone")
_HARD = frozenset(("cred_prefix", "private_key"))


def _read_entry(root: Path, name: str) -> SnapshotEntry | None:
    """No symlinked parent traversal; regular-file bytes come from one open fd."""
    parts = PurePosixPath(name).parts
    if (not name or PurePosixPath(name).is_absolute() or ".." in parts
            or PurePosixPath(name).as_posix() != name or ".git" in (p.lower() for p in parts)
            or any(c in name for c in "\0\n\r\t|")):
        raise SnapshotSafetyError("snapshot_source_path_invalid")
    if SENSITIVE_NAME.search(name):
        raise SnapshotSafetyError("snapshot_source_name_rejected")
    # Names enter a Git tree as bytes too. Never put matching names in exceptions.
    encoded = os.fsencode(name)
    if any(re.search(pattern, encoded) for _, pattern in _BUILTIN):
        raise SnapshotSafetyError("snapshot_source_name_rejected")
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in parts[:-1]:
            try:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            except FileNotFoundError:
                return None  # A tracked directory may have been deleted.
            os.close(directory)
            directory = child
        try:
            mode = os.stat(parts[-1], dir_fd=directory, follow_symlinks=False).st_mode
        except FileNotFoundError:
            return None
        if stat.S_ISLNK(mode):
            target = os.readlink(parts[-1], dir_fd=directory)
            if not (root / name).parent.joinpath(target).resolve().is_relative_to(root):
                raise SnapshotSafetyError("snapshot_source_symlink_escape")
            return SnapshotEntry(name, "120000", os.fsencode(target))
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_FILE_BYTES:
                raise SnapshotSafetyError("snapshot_source_file_unsupported")
            chunks, size = [], 0
            while True:
                chunk = os.read(fd, min(65536, MAX_FILE_BYTES + 1 - size))
                if not chunk:
                    break
                chunks.append(chunk)
                size += len(chunk)
                if size > MAX_FILE_BYTES:
                    raise SnapshotSafetyError("snapshot_source_file_unsupported")
            return SnapshotEntry(name, "100755" if info.st_mode & 0o111 else "100644", b"".join(chunks))
        finally:
            os.close(fd)
    finally:
        os.close(directory)


def _array(source: str, name: str, multiline: bool = False) -> list[str]:
    # Read only known, literal declarations. Never source/eval/execute shell input.
    pattern = rf"^{name}=\(\n(.*?)^\)\s*$" if multiline else rf"^{name}=\(([^\n]*)\)$"
    matches = re.findall(pattern, source, re.M | (re.S if multiline else 0))
    if len(matches) != 1:
        raise SnapshotSafetyError("snapshot_scanner_rules_invalid")
    return shlex.split(matches[0], comments=False, posix=True)


def _project_rules(data: bytes) -> tuple[tuple[str, re.Pattern[bytes]], ...]:
    source = data.decode("utf-8")
    ids, patterns = _array(source, "RULE_IDS"), _array(source, "RULE_PAT", True)
    hard = _array(source, "HARD")
    if tuple(ids) != _RULE_IDS or set(hard) != _HARD or len(patterns) != len(ids):
        raise SnapshotSafetyError("snapshot_scanner_rules_invalid")
    username = os.environ.get("USER")
    if not username:
        import getpass
        username = getpass.getuser()
    if not username:
        raise SnapshotSafetyError("snapshot_scanner_identity_unavailable")
    compiled = []
    for rule, pattern in zip(ids, patterns):
        # Preserve the scanner's ERE meaning under LC_ALL=C, including USER as a
        # regex. All current static rules use the common ERE/Python subset.
        pattern = pattern.replace("$WHOAMI", username).replace("[[:space:]]", r"[\t\r\v\f ]")
        if re.search(r"\$[A-Za-z{(]", pattern) or "[[:" in pattern:
            raise SnapshotSafetyError("snapshot_scanner_rules_invalid")
        compiled.append((rule, re.compile(os.fsencode(pattern))))
    # A malformed or weakened project hard rule cannot disable the minimum guard.
    return tuple(compiled)


def _checksum(data: bytes) -> str:
    """POSIX cksum CRC, including the length suffix (not zlib.crc32)."""
    crc = 0
    for byte in data + _length_bytes(len(data)):
        crc ^= byte << 24
        for _ in range(8):
            crc = ((crc << 1) ^ (0x04C11DB7 if crc & 0x80000000 else 0)) & 0xFFFFFFFF
    return str(crc ^ 0xFFFFFFFF)


def _length_bytes(length: int) -> bytes:
    result = bytearray()
    while length:
        result.append(length & 255)
        length >>= 8
    return bytes(result)


def _baseline(data: bytes) -> frozenset[tuple[str, str, str]]:
    approved = set()
    for line in data.decode("utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split("|")
        if (len(fields) != 3 or fields[0] not in _RULE_IDS or fields[0] in _HARD
                or not fields[1] or not fields[2].isascii() or not fields[2].isdigit()):
            raise SnapshotSafetyError("snapshot_scanner_baseline_invalid")
        approved.add(tuple(fields))
    return frozenset(approved)


def prepare_snapshot(cwd: Path, names: Iterable[str], *,
                     require_project_scanner: bool = False) -> tuple[SnapshotEntry, ...]:
    """Freeze and scan the exact Git selection; callers must write returned bytes.

    Missing tracked files are deletions. Binary payloads and symlink target bytes
    are scanned too. Existing scanner/baseline read or parse failures fail closed.
    Missing scanner is allowed only for bare fixtures without either scanner asset;
    production callers can additionally pass require_project_scanner=True.
    Credential prefixes/private keys fail even under nosec/baseline; other project
    rules keep the scanner's per-line nosec and (rule, path, cksum) baseline model.
    No user-provided path, match, or underlying exception is returned in errors.
    """
    try:
        root = Path(cwd).resolve(strict=True)
        selected = tuple(sorted(set(names)))
        entries = tuple(entry for name in selected if (entry := _read_entry(root, name)) is not None)
        builtin = tuple((rule, re.compile(pattern)) for rule, pattern in _BUILTIN)
        # Hard patterns have no inline or baseline exceptions, including binary
        # payloads. Soft rules retain the project's baseline semantics below.
        for entry in entries:
            for rule, pattern in builtin:
                if rule in _HARD and pattern.search(entry.data):
                    raise SnapshotSafetyError("snapshot_content_rejected")
        by_name = {entry.name: entry for entry in entries}
        scanner = by_name.get(SCANNER) or _read_entry(root, SCANNER)
        baseline = by_name.get(BASELINE) or _read_entry(root, BASELINE)
        if scanner is None:
            if require_project_scanner or baseline is not None or SCANNER in selected or BASELINE in selected:
                raise SnapshotSafetyError("snapshot_scanner_missing")
            for entry in entries:
                if any(pattern.search(entry.data) for _, pattern in builtin):
                    raise SnapshotSafetyError("snapshot_content_rejected")
            return entries
        if scanner.git_mode == "120000" or baseline is None or baseline.git_mode == "120000":
            raise SnapshotSafetyError("snapshot_scanner_assets_invalid")
        rules, approved = _project_rules(scanner.data), _baseline(baseline.data)
        for entry in entries:
            for line in entry.data.split(b"\n"):
                for rule, pattern in rules:
                    if not pattern.search(line):
                        continue
                    if rule in _HARD:
                        raise SnapshotSafetyError("snapshot_content_rejected")
                    if b"nosec:" in line or (rule, entry.name, _checksum(line)) in approved:
                        continue
                    raise SnapshotSafetyError("snapshot_content_rejected")
        return entries
    except SnapshotSafetyError:
        raise
    except Exception:
        # OSError/regex/Unicode errors can embed source names, bytes or USER.
        raise SnapshotSafetyError("snapshot_safety_check_failed") from None
