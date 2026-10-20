#!/usr/bin/env python3
"""GW-01: validate the real gateway auth probe and prove the gate rejects mutations.

Normal gate: --binary dist/agentisland (requires the real --gateway-auth-probe).
Debug only: --self-test uses an inline fixture; --receipt-file reads an existing receipt.
All mutations stay in memory. This script uses only the standard library, opens no
network connections, and writes no files. The child probe uses its own loopback peers.
Proxy evidence covers the allowed loopback target shape, not arbitrary remote hosts.
Refresh evidence covers RFC 6749 refresh_token exchange against a self-hosted loopback
peer; no real third-party gateway or real credential is exercised. Quota evidence covers
a user-declared endpoint/JSON path against the same kind of loopback peer: extraction is
verified, a mismatched path must not claim usage, and a plaintext remote endpoint must be
refused at save time. No upstream gateway schema is assumed.
Receipt values and untrusted diagnostic text are never echoed as credentials may leak
precisely when the product regresses. Top-level and nested objects use exact key sets.
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import subprocess
import sys
from typing import Any

PROBE_FLAG = "--gateway-auth-probe"
PROBE_TIMEOUT = 90

# Frozen receipt contract v4; pid is the sole variable leaf (any positive integer).
SYNTHETIC_FIXTURE: dict[str, Any] = {
    "schema_version": 1,
    "pid": 12345,
    "transport_policy": {
        "https_accepted": True,
        "loopback_http_accepted": True,
        "remote_plaintext": "insecure_transport",
        "unsupported_scheme": "invalid_address",
    },
    "auth_taxonomy": {
        "accepted": "readable",
        "rejected_401": "auth_failed",
        "forbidden_403": "permission_denied",
        "rate_limited_429": "rate_limited",
        "credential_missing": "credential_unavailable",
        "custom_header_scheme": "readable",
    },
    "rotation": {
        "before": "auth_failed",
        "after": "readable",
        "distinct_bearer_values": True,
        "probe_reads_source_each_time": True,
    },
    "refresh": {
        "before": "auth_failed",
        "after": "readable",
        "token_exchanges": 1,
        "read_attempts": 2,
        "retry_used_fresh_token": True,
        "failed_exchange": "refresh_failed",
        "plaintext_token_url": "insecure_transport",
        "missing_refresh_source": "credential_unavailable",
    },
    "quota_protocol": {
        "configured_usage_verified": True,
        "extracted_amount": 1000000.0,
        "extracted_unit": "Tokens",
        "mismatched_path_rejected": True,
        "plaintext_endpoint_blocked": True,
    },
    "proxy_isolation": {
        "env_proxy_configured": True,
        "target_peer_requests": 1,
        "proxy_peer_requests": 0,
        "request_succeeded": True,
    },
    "leakage": {
        "receipt_contains_secret": False,
        "status_reason_contains_secret": False,
    },
}


def verify(receipt: Any) -> list[str]:
    """Return every contract violation without modifying or printing the receipt."""
    violations: list[str] = []

    def check(actual: Any, expected: Any, path: tuple[str, ...]) -> None:
        label = ".".join(path) or "receipt"
        if isinstance(expected, dict):
            if type(actual) is not dict:
                violations.append(f"{label}: must be a JSON object")
                return
            for key in expected:
                if key not in actual:
                    violations.append(f"{label}.{key}: missing required field")
                else:
                    check(actual[key], expected[key], (*path, key))
            # Never print unknown keys: an unexpected key itself may contain a secret.
            if set(actual) - set(expected):
                violations.append(f"{label}: unexpected fields are forbidden")
        elif path == ("pid",):
            # bool is an int subclass; strict types prevent True from passing as 1.
            if type(actual) is not int or actual <= 0:
                violations.append("pid: must be a positive integer")
        elif type(actual) is not type(expected) or actual != expected:
            # Expected values are public contract constants; actual values stay private.
            violations.append(f"{label}: must be {expected!r} with exact type")

    check(receipt, SYNTHETIC_FIXTURE, ())
    return violations


def build_mutations(base_receipt: dict[str, Any]) -> list[tuple[str, Any]]:
    """Mutate every asserted leaf, then test object boundaries and strict scalar types."""
    mutations: list[tuple[str, Any]] = []

    def changed(path: tuple[str, ...], value: Any) -> dict[str, Any]:
        result = copy.deepcopy(base_receipt)
        parent = result
        for part in path[:-1]:
            parent = parent[part]
        parent[path[-1]] = value
        return result

    def walk(value: dict[str, Any], path: tuple[str, ...] = ()) -> None:
        for key, current in value.items():
            leaf_path = (*path, key)
            label = ".".join(leaf_path)
            if type(current) is dict:
                walk(current, leaf_path)
                continue
            if type(current) is bool:
                replacement = not current
            elif type(current) is str:
                # Valid domain enum substitutions catch incorrect taxonomy mappings.
                if current == "insecure_transport":
                    replacement = "invalid_address"
                elif current == "invalid_address":
                    replacement = "insecure_transport"
                else:
                    replacement = "auth_failed" if current == "readable" else "readable"
            elif leaf_path == ("pid",):
                # pid +100 is still a valid positive pid; test its actual boundary.
                replacement = 0
            else:
                replacement = current + 100
            mutations.append((f"changed {label}", changed(leaf_path, replacement)))
            # Deletion also proves each individual required leaf is checked.
            missing = copy.deepcopy(base_receipt)
            parent = missing
            for part in path:
                parent = parent[part]
            del parent[key]
            mutations.append((f"missing {label}", missing))
            if type(current) is bool:
                mutations.append((f"wrong type {label}", changed(leaf_path, int(current))))
            elif type(current) is int:
                mutations.append((f"wrong type {label}", changed(leaf_path, float(current))))

        # Prove exact key sets and object types at every nesting level, including root.
        extra = copy.deepcopy(base_receipt)
        parent = extra
        for part in path:
            parent = parent[part]
        parent["__unexpected_field__"] = "synthetic-extra"
        label = ".".join(path) or "receipt"
        mutations.append((f"extra field in {label}", extra))
        mutations.append((f"non-object {label}", changed(path, []) if path else []))
        if path:
            missing_section = copy.deepcopy(base_receipt)
            parent = missing_section
            for part in path[:-1]:
                parent = parent[part]
            del parent[path[-1]]
            mutations.append((f"missing section {label}", missing_section))

    walk(base_receipt)
    mutations.append(("zero target requests", changed(("proxy_isolation", "target_peer_requests"), 0)))
    mutations.append(("boolean pid", changed(("pid",), True)))
    mutations.append(("boolean schema version", changed(("schema_version",), True)))
    mutations.append(("boolean proxy request count", changed(("proxy_isolation", "proxy_peer_requests"), False)))
    return mutations


def parse_receipt(text: str) -> Any:
    """json.loads rejects extra log lines/objects; also reject duplicate JSON keys."""
    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON field")
            result[key] = value
        return result

    try:
        return json.loads(text, object_pairs_hook=unique_object)
    except (ValueError, RecursionError):
        # JSONDecodeError may include raw input: never print it or the source bytes.
        raise ValueError("receipt must be exactly one valid JSON object with unique fields") from None


def run_probe(binary: Path) -> Any:
    """Run the real binary, require success, and parse its complete stdout."""
    resolved = binary.resolve()
    if not resolved.is_file():
        raise ValueError("binary not found or not a regular file")
    try:
        proc = subprocess.run(
            [str(resolved), PROBE_FLAG], capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=PROBE_TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        raise ValueError(f"probe timed out after {PROBE_TIMEOUT}s") from None
    except OSError:
        raise ValueError("could not execute probe binary") from None
    if proc.returncode != 0:
        # The stderr fragment is wholly masked: arbitrary diagnostic text may contain
        # credentials. Preserve whitespace and the bounded fragment length only.
        snippet = "".join(c if c.isspace() else "*" for c in proc.stderr[:500]).strip()
        raise ValueError(
            f"probe exited with code {proc.returncode}; stderr (redacted): "
            f"{snippet or '(empty)'}"
        )
    return parse_receipt(proc.stdout)


def run_regressions(receipt: Any) -> int:
    violations = verify(receipt)
    if violations:
        raise ValueError("receipt violated contract:\n" + "\n".join(f"  - {v}" for v in violations))
    mutations = build_mutations(receipt)
    for index, (description, mutated) in enumerate(mutations, start=1):
        if not verify(mutated):
            raise ValueError(f"gate failed to reject mutation: {description}")
        print(f"ok {index} - mutation rejected: {description}")
    return len(mutations)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path, help="Required production input: real agentisland binary")
    source.add_argument("--receipt-file", type=Path, help="Debug only: read an existing JSON receipt")
    source.add_argument("--self-test", action="store_true", help="Debug only: verify the inline synthetic fixture")
    args = parser.parse_args()
    try:
        if args.self_test:
            receipt = copy.deepcopy(SYNTHETIC_FIXTURE)
            print("ok - synthetic fixture mode (no real binary exercised)")
        elif args.receipt_file:
            try:
                text = args.receipt_file.read_text(encoding="utf-8")
            except (OSError, UnicodeError):
                raise ValueError("could not read receipt file as UTF-8") from None
            receipt = parse_receipt(text)
            print("ok - receipt file debug mode (no real binary exercised)")
        else:
            receipt = run_probe(args.binary)
        count = run_regressions(receipt)
    except (ValueError, RecursionError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(f"PASS: {count} gateway auth regressions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
