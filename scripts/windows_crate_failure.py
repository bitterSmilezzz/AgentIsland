#!/usr/bin/env python3
"""Classify one known host prerequisite failure from the Windows crate check.

This never turns arbitrary compiler failures into environment blocks. It recognizes
only ring's Windows MSVC build failing because the target CRT's assert.h is absent.
"""
from __future__ import annotations

from pathlib import Path
import re
import sys

WINDOWS_SDK_HEADERS_UNAVAILABLE = "windows_sdk_headers_unavailable"
WINDOWS_CRATE_TYPECHECK_FAILED = "windows_crate_typecheck_failed"

CUSTOM_BUILD_FAILURE = re.compile(
    r"^error: failed to run custom build command for `(?P<package>[^`]+)`$"
)
ERROR_LINE = re.compile(r"^error(?::|\[)")
MISSING_ASSERT_HEADER = re.compile(r"fatal error: 'assert\.h' file not found")
COMPILER_ERROR_DIAGNOSTIC = re.compile(r"(?:^|:\s)(?:fatal\s+)?error(?:\[[A-Za-z0-9]+\])?:")
RING_WARNING_OWNER = re.compile(r"^warning: ring@[0-9][^:\s]*:")
RING_PATH_OWNER = re.compile(r"(?:^|[\\/])ring-[0-9][^\\/\s]*[\\/]")
COMPILER_FAILURE = re.compile(r"fatal error:|\b[0-9]+ errors? generated\.")
C_COMPILER_ERROR_COUNT = re.compile(r"\b([0-9]+) errors? generated\.")
CARGO_COMPILER_SUMMARY = re.compile(r"^\s*cargo:warning=[0-9]+ errors? generated\.$")


def classify(log: str) -> str:
    lines = log.splitlines()
    build_failures = [
        match for line in lines if (match := CUSTOM_BUILD_FAILURE.fullmatch(line))
    ]
    if len(build_failures) != 1 or not build_failures[0]["package"].startswith("ring v"):
        return WINDOWS_CRATE_TYPECHECK_FAILED

    root_failure = build_failures[0].group(0)
    if any(ERROR_LINE.match(line) and line != root_failure for line in lines):
        return WINDOWS_CRATE_TYPECHECK_FAILED

    def owned_by_ring(line: str) -> bool:
        return bool(RING_WARNING_OWNER.search(line) or RING_PATH_OWNER.search(line))

    ring_header_is_missing = any(
        MISSING_ASSERT_HEADER.search(line) and owned_by_ring(line)
        for line in lines
    )
    if any(
        COMPILER_ERROR_DIAGNOSTIC.search(line)
        and line != root_failure
        and not (MISSING_ASSERT_HEADER.search(line) and owned_by_ring(line))
        for line in lines
    ):
        return WINDOWS_CRATE_TYPECHECK_FAILED

    error_counts = [
        int(match.group(1))
        for line in lines
        if (match := C_COMPILER_ERROR_COUNT.search(line))
    ]
    if any(count != 1 for count in error_counts):
        return WINDOWS_CRATE_TYPECHECK_FAILED

    # A mixed log must stay a failure even when ring also reports this host's
    # missing CRT header. Only ring-owned compiler diagnostics can justify the
    # environment block; a second crate's diagnostic is independent evidence.
    # Cargo also replays the failing build script's generic "N errors generated"
    # summary without a crate name. Accept that summary only after ring's own
    # missing-header diagnostic and its sole build-failure root are established.
    if any(
        COMPILER_FAILURE.search(line)
        and not owned_by_ring(line)
        and not (ring_header_is_missing and CARGO_COMPILER_SUMMARY.fullmatch(line))
        for line in lines
    ):
        return WINDOWS_CRATE_TYPECHECK_FAILED

    if ring_header_is_missing:
        return WINDOWS_SDK_HEADERS_UNAVAILABLE
    return WINDOWS_CRATE_TYPECHECK_FAILED


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: windows_crate_failure.py <cargo-log>", file=sys.stderr)
        return 2
    try:
        log = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
    except OSError:
        print("cannot read Windows crate check log", file=sys.stderr)
        return 2
    print(classify(log))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
