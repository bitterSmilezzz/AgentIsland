#!/usr/bin/env python3
"""Content-bound local build receipts; no git, user data, network or subprocesses.

Scope v1: all regular files in app/src-tauri (including Rust modules, fixtures,
Cargo.lock/manifests/build.rs, Tauri configs, icons, capabilities/permissions and
bundle assets), app/ui, recursively declared in-repo Cargo path dependencies,
project-local .cargo config and rust-toolchain files, and the packaging scripts.
Generated/output directories below are pruned. Unreferenced vendor trees and
HOME Cargo caches/config/credentials are never read. A newly configured external
path dependency/frontend directory is rejected rather than silently omitted.

The SHA256 seal detects malformed/edited receipts, not a malicious same-user
writer capable of rewriting the receipt and seal together. Consumers can pin
receipt_sha256 and source_manifest_sha256 in independent run evidence.
Requires Python >= 3.11 (stdlib tomllib). Only release/macos packaging is enabled.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tempfile
import tomllib
import uuid

SCHEMA = "agentisland.build-receipt"
SCHEMA_VERSION = 1
SCOPE_VERSION = 1
EXCLUDED = {"target", "node_modules", "dist", ".scratch", ".git", "__pycache__"}
GENERATED = {"app/src-tauri/gen/schemas"}
IGNORED_FILES = {".DS_Store"}
PRIVATE_NAMES = {"credentials", "credentials.json", "credentials.toml", "auth.json", ".env"}
RECEIPT = "dist/build-receipt.json"
STATE = "dist/.build-in-progress.json"
LOCK = "dist/.artifact-lock"
REQUIRED = ("app/src-tauri/Cargo.toml", "app/src-tauri/Cargo.lock",
            "app/src-tauri/build.rs", "app/src-tauri/tauri.conf.json",
            "app/src-tauri/src/main.rs", "app/ui/index.html",
            "scripts/build-app.sh", "scripts/common.sh", "scripts/check-version.py",
            "scripts/heavy-build-lock.sh", "scripts/automation/heavy_build_lock.py",
            "scripts/automation/artifact.py")
BINARIES = ("dist/agentisland", "dist/AgentIsland.app/Contents/MacOS/agentisland",
            "dist/AgentIsland.app/Contents/Helpers/agentisland")
BUNDLE = "dist/AgentIsland.app"
HEX = re.compile(r"[0-9a-f]{64}\Z")
CODES = frozenset({"ok", "invalid_argument", "source_missing", "source_unreadable",
                   "unsafe_path", "unsupported_input", "source_changed", "source_drift",
                   "receipt_missing", "receipt_invalid", "receipt_schema", "receipt_integrity",
                   "platform_mismatch", "profile_mismatch", "version_mismatch",
                   "artifact_missing", "artifact_changed", "artifact_unreadable",
                   "build_in_progress", "build_state_invalid", "build_token_mismatch",
                   "build_not_active", "build_busy", "io_error"})


class ArtifactError(RuntimeError):
    def __init__(self, code: str):
        assert code in CODES
        self.code = code
        super().__init__(code)


def _canonical(value) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                      allow_nan=False).encode("ascii")


def _sha(value) -> str:
    return hashlib.sha256(_canonical(value)).hexdigest()


def _root(root) -> Path:
    path = Path(root).absolute()
    # Resolve only the explicitly supplied project root; never resolve child links.
    if not path.is_dir():
        raise ArtifactError("source_missing")
    return path.resolve()


def _safe(root: Path, name: str, missing: str) -> Path:
    rel = PurePosixPath(name)
    if rel.is_absolute() or not rel.parts or any(p in {"..", "."} for p in rel.parts):
        raise ArtifactError("unsafe_path")
    path = root
    for part in rel.parts:
        path = path / part
        try:
            info = path.lstat()
        except FileNotFoundError:
            raise ArtifactError(missing) from None
        if stat.S_ISLNK(info.st_mode):
            raise ArtifactError("unsafe_path")
    return path


def _read(root: Path, name: str, missing: str, unreadable: str) -> tuple[bytes, os.stat_result]:
    path = _safe(root, name, missing)
    try:
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
        fd = os.open(path, flags)
        with os.fdopen(fd, "rb") as handle:
            before = os.fstat(handle.fileno())
            if not stat.S_ISREG(before.st_mode):
                raise ArtifactError("unsafe_path")
            data = handle.read()
            after = os.fstat(handle.fileno())
        current = path.lstat()
        # Metadata is only a concurrent-read guard, never the freshness proof.
        fields = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_mode)
        if fields(before) != fields(after) or fields(after) != fields(current):
            raise ArtifactError("source_changed" if missing == "source_missing" else "artifact_changed")
        return data, after
    except OSError:
        raise ArtifactError(unreadable) from None


def _private(name: str) -> bool:
    lower = Path(name).name.lower()
    return (lower in PRIVATE_NAMES or lower.startswith(".env.")
            or lower.endswith((".pem", ".key", ".p12", ".pfx", ".mobileprovision")))


def _tree(root: Path, name: str, missing="source_missing") -> list[str]:
    path = _safe(root, name, missing)
    if not path.is_dir():
        raise ArtifactError("unsafe_path")
    result = []
    for child in sorted(path.iterdir()):
        rel = child.relative_to(root).as_posix()
        if missing == "source_missing" and (child.name in EXCLUDED or rel in GENERATED
                                             or child.name in IGNORED_FILES):
            continue
        info = child.lstat()
        if stat.S_ISLNK(info.st_mode) or _private(child.name):
            raise ArtifactError("unsafe_path")
        if stat.S_ISDIR(info.st_mode):
            result.extend(_tree(root, rel, missing))
        elif stat.S_ISREG(info.st_mode):
            result.append(rel)
        else:
            raise ArtifactError("unsafe_path")
    return result


def _toml(root: Path, name: str) -> dict:
    try:
        return tomllib.loads(_read(root, name, "source_missing", "source_unreadable")[0].decode("utf-8"))
    except (ValueError, UnicodeError):
        raise ArtifactError("unsupported_input") from None


def _path_dependencies(value: dict):
    for key, table in value.items():
        if key in {"dependencies", "build-dependencies", "dev-dependencies"} or key == "patch":
            for entry in table.values():
                if isinstance(entry, dict):
                    if isinstance(entry.get("path"), str):
                        yield entry["path"]
                    if key == "patch":
                        for patched in entry.values():
                            if isinstance(patched, dict) and isinstance(patched.get("path"), str):
                                yield patched["path"]
        elif isinstance(table, dict):
            yield from _path_dependencies(table)


def _local_path(root: Path, base: Path, value: str) -> str:
    # Normalize lexical '..' only; symlinks are checked by _safe before reading.
    path = Path(os.path.abspath(base / value))
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        raise ArtifactError("unsupported_input") from None


def source_manifest(root) -> dict:
    root = _root(root)
    for name in REQUIRED:
        _safe(root, name, "source_missing")
    names = set(REQUIRED)
    names.update(_tree(root, "app/src-tauri"))
    names.update(_tree(root, "app/ui"))
    # Read only Cargo's declared path dependencies, not every vendor or workspace file.
    manifests = ["app/src-tauri/Cargo.toml"]
    visited = set()
    while manifests:
        name = manifests.pop()
        if name in visited:
            continue
        visited.add(name)
        manifest = _toml(root, name)
        if manifest.get("workspace") or "workspace" in manifest.get("package", {}):
            raise ArtifactError("unsupported_input")
        for dependency in _path_dependencies(manifest):
            rel = _local_path(root, (root / name).parent, dependency)
            files = _tree(root, rel)
            if rel + "/Cargo.toml" not in files:
                raise ArtifactError("source_missing")
            names.update(files)
            manifests.append(rel + "/Cargo.toml")
    for parent in ("", "app/", "app/src-tauri/"):
        for suffix in (".cargo/config", ".cargo/config.toml", "rust-toolchain", "rust-toolchain.toml"):
            name = parent + suffix
            if (root / name).exists() or (root / name).is_symlink():
                _safe(root, name, "source_missing")
                if suffix in {".cargo/config", ".cargo/config.toml"}:
                    # These features can redirect compilation outside the declared tree.
                    cargo_config = _toml(root, name)
                    if (any(k in cargo_config for k in ("include", "paths", "patch", "source"))
                            or any(k in cargo_config.get("build", {}) for k in ("target", "target-dir"))):
                        raise ArtifactError("unsupported_input")
                names.add(name)
    # Platform overlays are hashed and checked too; no implicit external resources.
    configs = ["app/src-tauri/tauri.conf.json"]
    for name in names:
        base = Path(name).name
        if name.startswith("app/src-tauri/") and base.startswith("tauri."):
            if base.endswith((".json5", ".toml")):
                raise ArtifactError("unsupported_input")
            if base == "tauri.macos.conf.json":
                configs.append(name)
    # Currently packaged input locations are intentionally fixed; reject config changes
    # that would introduce unmanifested inputs, hooks or signing credentials.
    try:
        for config_name in configs:
            config = _json(root, config_name, "source_missing", "source_unreadable")
            build = config.get("build", {})
            if ((config_name == configs[0] and build.get("frontendDist") != "../ui")
                    or ("frontendDist" in build and build["frontendDist"] != "../ui")
                    or any(build.get(k) for k in ("beforeBuildCommand", "beforeBundleCommand", "beforeDevCommand"))):
                raise ArtifactError("unsupported_input")
            bundle = config.get("bundle", {})
            if bundle.get("externalBin") or bundle.get("macOS", {}).get("frameworks"):
                raise ArtifactError("unsupported_input")
            for section in (bundle, bundle.get("macOS", {})):
                for key in ("resources", "icon", "infoPlist", "entitlements"):
                    paths = section.get(key, []) or []
                    paths = [paths] if isinstance(paths, str) else list(paths)
                    for value in paths:
                        rel = _local_path(root, root / "app/src-tauri", value)
                        if (any(c in rel for c in "*?[") or not rel.startswith("app/src-tauri/")
                                or any(p in EXCLUDED for p in PurePosixPath(rel).parts)
                                or any(rel == p or rel.startswith(p + "/") for p in GENERATED)):
                            raise ArtifactError("unsupported_input")
                        path = _safe(root, rel, "source_missing")
                        if path.is_dir():
                            names.update(_tree(root, rel))
                        else:
                            names.add(rel)
    except (TypeError, AttributeError):
        raise ArtifactError("unsupported_input") from None
    entries = []
    for name in sorted(names):
        if _private(name):
            raise ArtifactError("unsafe_path")
        data, info = _read(root, name, "source_missing", "source_unreadable")
        entries.append({"path": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                        "executable": bool(info.st_mode & 0o111)})
    return {"scope_version": SCOPE_VERSION, "excluded_directories": sorted(EXCLUDED),
            "generated_directories": sorted(GENERATED), "files": entries}


def source_digest(root) -> str:
    return _sha(source_manifest(root))


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def _json(root, name, missing, invalid):
    data, info = _read(root, name, missing, invalid)
    if name in {RECEIPT, STATE} and os.name == "posix" and stat.S_IMODE(info.st_mode) != 0o600:
        raise ArtifactError("unsafe_path")
    try:
        return json.loads(data, object_pairs_hook=_unique,
                          parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    except (ValueError, UnicodeError, RecursionError):
        raise ArtifactError(invalid) from None


def _seal(payload: dict) -> dict:
    return {**payload, "receipt_sha256": _sha(payload)}


def _file_entries(entries, source: bool) -> bool:
    if not isinstance(entries, list) or not entries:
        return False
    paths = []
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path", "size", "sha256", "executable"}:
            return False
        name = entry["path"]
        if (not isinstance(name, str) or not name or "\\" in name or "\0" in name
                or PurePosixPath(name).is_absolute() or ".." in PurePosixPath(name).parts
                or type(entry["size"]) is not int or entry["size"] < 0
                or type(entry["executable"]) is not bool
                or not isinstance(entry["sha256"], str) or not HEX.fullmatch(entry["sha256"])):
            return False
        if not source and name != BINARIES[0] and not name.startswith(BUNDLE + "/"):
            return False
        paths.append(name)
    return paths == sorted(set(paths)) and set(REQUIRED if source else BINARIES) <= set(paths)


def _load(root: Path, name: str) -> dict:
    state = name == STATE
    payload = _json(root, name, "build_state_invalid" if state else "receipt_missing",
                    "build_state_invalid" if state else "receipt_invalid")
    invalid = "build_state_invalid" if state else "receipt_invalid"
    if not isinstance(payload, dict):
        raise ArtifactError(invalid)
    if (payload.get("schema") != (SCHEMA + ".pending" if state else SCHEMA)
            or type(payload.get("schema_version")) is not int
            or payload["schema_version"] != SCHEMA_VERSION):
        raise ArtifactError("build_state_invalid" if state else "receipt_schema")
    common = {"schema", "schema_version", "build_id", "version", "platform", "architecture",
              "profile", "source_manifest", "source_manifest_sha256", "receipt_sha256"}
    keys = common | ({"status"} if state else {"artifacts"})
    if set(payload) != keys:
        raise ArtifactError(invalid)
    seal = payload["receipt_sha256"]
    if not isinstance(seal, str) or not HEX.fullmatch(seal) or _sha({k: v for k, v in payload.items()
                                                                 if k != "receipt_sha256"}) != seal:
        raise ArtifactError("build_state_invalid" if state else "receipt_integrity")
    if (not isinstance(payload["source_manifest_sha256"], str)
            or not HEX.fullmatch(payload["source_manifest_sha256"])
            or _sha(payload["source_manifest"]) != payload["source_manifest_sha256"]
            or not isinstance(payload["build_id"], str)
            or not re.fullmatch(r"[0-9a-f]{32}", payload["build_id"])):
        raise ArtifactError(invalid)
    manifest = payload["source_manifest"]
    if (not isinstance(manifest, dict)
            or set(manifest) != {"scope_version", "excluded_directories", "generated_directories", "files"}
            or type(manifest["scope_version"]) is not int or manifest["scope_version"] != SCOPE_VERSION
            or manifest["excluded_directories"] != sorted(EXCLUDED)
            or manifest["generated_directories"] != sorted(GENERATED)
            or not _file_entries(manifest["files"], True)
            or any(not isinstance(payload[k], str) or not payload[k]
                   for k in ("version", "platform", "architecture", "profile"))
            or (not state and not _file_entries(payload["artifacts"], False))):
        raise ArtifactError(invalid)
    if state and payload["status"] not in {"active", "failed"}:
        raise ArtifactError(invalid)
    return payload


def _platform() -> tuple[str, str]:
    import platform
    return ("windows" if sys.platform == "win32" else sys.platform, platform.machine().lower())


def _identity(root: Path, payload: dict, profile: str, expected_sourcehash: str | None):
    if profile != "release" or payload["profile"] != profile:
        raise ArtifactError("profile_mismatch")
    host, arch = _platform()
    if payload["platform"] != host or payload["architecture"] != arch or host != "darwin":
        raise ArtifactError("platform_mismatch")
    if payload["version"] != _toml(root, "app/src-tauri/Cargo.toml").get("package", {}).get("version"):
        raise ArtifactError("version_mismatch")
    if expected_sourcehash is not None:
        if not HEX.fullmatch(expected_sourcehash):
            raise ArtifactError("invalid_argument")
        if payload["source_manifest_sha256"] != expected_sourcehash:
            raise ArtifactError("source_drift")


def _artifacts(root: Path) -> list[dict]:
    names = {BINARIES[0], *_tree(root, BUNDLE, "artifact_missing")}
    result = []
    for name in sorted(names):
        data, info = _read(root, name, "artifact_missing", "artifact_unreadable")
        executable = bool(info.st_mode & 0o111)
        if name in BINARIES and (not executable or not os.access(root / name, os.X_OK)):
            raise ArtifactError("artifact_unreadable")
        result.append({"path": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                       "executable": executable})
    if not set(BINARIES) <= names:
        raise ArtifactError("artifact_missing")
    # Each binary is bound by its own bytes. A real v0.0.323 bundle has a
    # differently-signed MacOS/agentisland than dist/agentisland, so equality is
    # deliberately not assumed; the shell copies from one source instead.
    return result


@contextmanager
def _lock(root: Path):
    dist = root / "dist"
    if not dist.exists() and not dist.is_symlink():
        dist.mkdir(mode=0o700, parents=True, exist_ok=True)
    _safe(root, "dist", "io_error")
    path = root / LOCK
    try:
        path.mkdir(mode=0o700)
    except FileExistsError:
        raise ArtifactError("build_busy") from None
    try:
        yield
    finally:
        path.rmdir()


def _atomic(root: Path, name: str, payload: dict):
    # Caller owns the short mutation lock; valid existing originals only may be replaced.
    if (root / name).exists() or (root / name).is_symlink():
        _load(root, name)
    data = _canonical(_seal(payload)) + b"\n"
    fd, temporary = tempfile.mkstemp(prefix=".artifact-", dir=root / "dist")
    try:
        with os.fdopen(fd, "wb") as handle:
            os.fchmod(handle.fileno(), 0o600)
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, root / name)
        if os.name == "posix":
            directory = os.open(root / "dist", os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def begin(root, profile="release", expected_sourcehash=None) -> dict:
    root = _root(root)
    with _lock(root):
        if (root / STATE).exists() or (root / STATE).is_symlink():
            if _load(root, STATE)["status"] == "active":
                raise ArtifactError("build_in_progress")
        # Preserve broken/future receipts byte for byte. A deliberate recovery must
        # move them aside; do not silently overwrite diagnostic evidence.
        if (root / RECEIPT).exists() or (root / RECEIPT).is_symlink():
            _load(root, RECEIPT)
        manifest = source_manifest(root)
        digest = _sha(manifest)
        if digest != source_digest(root):
            raise ArtifactError("source_changed")
        host, arch = _platform()
        payload = {"schema": SCHEMA + ".pending", "schema_version": SCHEMA_VERSION,
                   "build_id": uuid.uuid4().hex, "version": _toml(root, "app/src-tauri/Cargo.toml")["package"]["version"],
                   "platform": host, "architecture": arch, "profile": profile,
                   "source_manifest": manifest, "source_manifest_sha256": digest, "status": "active"}
        _identity(root, payload, profile, expected_sourcehash)
        _atomic(root, STATE, payload)
        return {"ok": True, "code": "ok", "build_id": payload["build_id"],
                "source_manifest_sha256": digest}


def finish(root, build_id: str, profile="release", expected_sourcehash=None) -> dict:
    root = _root(root)
    with _lock(root):
        pending = _load(root, STATE)
        if pending["build_id"] != build_id:
            raise ArtifactError("build_token_mismatch")
        if pending["status"] != "active":
            raise ArtifactError("build_not_active")
        _identity(root, pending, profile, expected_sourcehash)
        manifest = source_manifest(root)
        if manifest != pending["source_manifest"]:
            raise ArtifactError("source_drift")
        artifacts = _artifacts(root)
        if source_digest(root) != pending["source_manifest_sha256"]:
            raise ArtifactError("source_drift")
        payload = {k: v for k, v in pending.items() if k not in {"status", "receipt_sha256"}}
        payload.update(schema=SCHEMA, artifacts=artifacts)
        _atomic(root, RECEIPT, payload)
        (root / STATE).unlink()
    return verify(root, profile, expected_sourcehash)


def fail(root, build_id: str) -> dict:
    root = _root(root)
    with _lock(root):
        pending = _load(root, STATE)
        if pending["build_id"] != build_id:
            raise ArtifactError("build_token_mismatch")
        pending.pop("receipt_sha256")
        pending["status"] = "failed"
        _atomic(root, STATE, pending)
    return {"ok": True, "code": "ok"}


def verify(root, profile="release", expected_sourcehash=None, expected_receipt_sha256=None) -> dict:
    """Read-only, fail-closed status; fixed codes, no exceptions/paths/file content leaked."""
    try:
        root = _root(root)
        if (root / LOCK).exists() or (root / LOCK).is_symlink():
            raise ArtifactError("build_busy")
        if (root / STATE).exists() or (root / STATE).is_symlink():
            _load(root, STATE)
            raise ArtifactError("build_in_progress")
        payload = _load(root, RECEIPT)
        _identity(root, payload, profile, expected_sourcehash)
        if expected_receipt_sha256 is not None and payload["receipt_sha256"] != expected_receipt_sha256:
            raise ArtifactError("receipt_integrity")
        manifest = source_manifest(root)
        if manifest != payload["source_manifest"]:
            raise ArtifactError("source_drift")
        if _artifacts(root) != payload["artifacts"]:
            raise ArtifactError("artifact_changed")
        if source_digest(root) != payload["source_manifest_sha256"]:
            raise ArtifactError("source_drift")
        # Recheck publication identity to refuse a build started during verification.
        if ((root / STATE).exists() or (root / LOCK).exists()
                or _load(root, RECEIPT)["receipt_sha256"] != payload["receipt_sha256"]):
            raise ArtifactError("build_in_progress")
        return {"ok": True, "code": "ok", "schema_version": SCHEMA_VERSION,
                **{k: payload[k] for k in ("build_id", "version", "platform", "architecture", "profile",
                                           "source_manifest_sha256", "receipt_sha256")},
                "artifacts": payload["artifacts"]}
    except ArtifactError as exc:
        return {"ok": False, "code": exc.code}
    except (OSError, ValueError, KeyError, TypeError, AttributeError, RecursionError):
        return {"ok": False, "code": "io_error"}


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ArtifactError("invalid_argument")


def main(argv=None) -> int:
    try:
        parser = _Parser(description=__doc__)
        parser.add_argument("command", choices=("begin", "finish", "fail", "verify", "status", "sourcehash"))
        parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
        parser.add_argument("--profile", default="release")
        parser.add_argument("--sourcehash", default=None)
        parser.add_argument("--receipt-sha256", default=None)
        parser.add_argument("--build-id", default=None)
        args = parser.parse_args(argv)
        if args.command in {"finish", "fail"} and not args.build_id:
            raise ArtifactError("invalid_argument")
        if args.command == "begin":
            result = begin(args.root, args.profile, args.sourcehash)
        elif args.command == "finish":
            result = finish(args.root, args.build_id, args.profile, args.sourcehash)
        elif args.command == "fail":
            result = fail(args.root, args.build_id)
        elif args.command == "sourcehash":
            result = {"ok": True, "code": "ok", "source_manifest_sha256": source_digest(args.root)}
        else:
            result = verify(args.root, args.profile, args.sourcehash, args.receipt_sha256)
    except ArtifactError as exc:
        result = {"ok": False, "code": exc.code}
    except (OSError, ValueError, KeyError, TypeError, AttributeError, RecursionError):
        result = {"ok": False, "code": "io_error"}
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 3


if __name__ == "__main__":
    raise SystemExit(main())
