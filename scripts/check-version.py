#!/usr/bin/env python3
"""Validate published versions against the Rust package version."""
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
cargo = (root / "app/src-tauri/Cargo.toml").read_text()
version = re.search(r'^version = "([^"\n]+)"', cargo, re.M).group(1)
lock = (root / "app/src-tauri/Cargo.lock").read_text()
values = {
    "Cargo.toml": version,
    "Cargo.lock": re.search(r'name = "agentisland"\nversion = "([^"\n]+)"', lock).group(1),
    "Tauri": json.loads((root / "app/src-tauri/tauri.conf.json").read_text())["version"],
    "CHANGELOG": re.search(r'^## \[([^]\n]+)\]', (root / "CHANGELOG.md").read_text(), re.M).group(1),
    "README": re.search(r'本文档描述 \*\*v([0-9.]+)', (root / "README.md").read_text()).group(1),
}
expected = sys.argv[1] if len(sys.argv) > 1 else version
if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", expected) or any(value != expected for value in values.values()):
    raise SystemExit(f"版本漂移：期望 {expected}，实际 {values}")
print(f"✓ Rust / Tauri / 文档版本一致：{expected}")
