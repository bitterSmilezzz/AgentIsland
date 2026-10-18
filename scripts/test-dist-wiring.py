#!/usr/bin/env python3
"""Static wiring gates for the release and permission surface.

Both cases below are defects that shipped once and were invisible locally:

1. ``capabilities/desktop.json`` granted ``updater:default`` to a window label
   (``main``) that this app never creates — the real shells are island / sidebar /
   workbench. Nothing failed; the permission would only have been refused the day
   a UI first called the updater.
2. ``release.sh`` uploaded the updater manifest as ``AgentIsland-<version>.latest.json``
   while the endpoint is ``releases/latest/download/latest.json``. Releases looked
   green and every installed client got a 404 on update checks.

These are text/shape assertions, not behavioural end-to-end proofs. They prove the
wiring is written down correctly, which is exactly the part that regressed.
"""
from __future__ import annotations

import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
TAURI_CONF = ROOT / "app/src-tauri/tauri.conf.json"
CAPABILITIES = ROOT / "app/src-tauri/capabilities"
RELEASE = ROOT / "scripts/release.sh"


def window_labels() -> set[str]:
    config = json.loads(TAURI_CONF.read_text(encoding="utf-8"))
    return {window["label"] for window in config["app"]["windows"]}


class CapabilityWiringTests(unittest.TestCase):
    def test_every_capability_targets_declared_window_labels(self):
        labels = window_labels()
        self.assertIn("workbench", labels, "fixture assumption: workbench shell exists")
        for path in sorted(CAPABILITIES.glob("*.json")):
            capability = json.loads(path.read_text(encoding="utf-8"))
            targets = set(capability.get("windows") or [])
            unknown = targets - labels
            self.assertFalse(
                unknown,
                f"{path.name} grants permissions to windows that do not exist: {sorted(unknown)}"
                f" (declared: {sorted(labels)})")

    def test_updater_permission_is_granted_to_a_real_window(self):
        labels = window_labels()
        granted = False
        for path in sorted(CAPABILITIES.glob("*.json")):
            capability = json.loads(path.read_text(encoding="utf-8"))
            if "updater:default" in capability.get("permissions", []):
                granted = granted or bool(set(capability.get("windows") or []) & labels)
        self.assertTrue(granted, "no capability grants updater:default to a real window")

    def test_capability_files_end_with_newline(self):
        for path in sorted(CAPABILITIES.glob("*.json")):
            self.assertTrue(
                path.read_bytes().endswith(b"\n"),
                f"{path.name} lacks a trailing newline")


class ReleaseWiringTests(unittest.TestCase):
    def setUp(self):
        self.source = RELEASE.read_text(encoding="utf-8")

    def test_updater_manifest_asset_is_uploaded_under_the_endpoint_name(self):
        config = json.loads(TAURI_CONF.read_text(encoding="utf-8"))
        # The gh invocation is written with line continuations, so match on the
        # whole (backslash-joined) source rather than a single line.
        flattened = re.sub(r"\\\n\s*", " ", self.source)
        for endpoint in config["plugins"]["updater"]["endpoints"]:
            name = endpoint.rsplit("/", 1)[-1]
            self.assertEqual(
                name, "latest.json",
                "endpoint must point at a stable asset name")
            self.assertRegex(
                flattened,
                r'gh release create [^\n]*"dist/latest\.json"',
                "release must upload dist/latest.json so the asset name matches the endpoint")
        # The versioned copy must not be what gets uploaded (GitHub names assets
        # after the file, so `AgentIsland-x.latest.json` would break the endpoint).
        create_calls = re.findall(r"gh release create[^\n]*", flattened)
        self.assertTrue(create_calls, "release create invocation not found")
        for call in create_calls:
            self.assertNotIn(".latest.json", call)

    def test_release_requires_the_updater_private_key(self):
        self.assertRegex(
            self.source,
            r"die \"发版需要 updater 私钥",
            "release must fail closed when the updater key is absent")

    def test_release_asserts_required_assets_instead_of_only_printing(self):
        self.assertIn("REQUIRED_ASSETS=", self.source)
        self.assertIn('latest.json")', self.source)
        self.assertRegex(self.source, r"die \"发布附件缺少")

    def test_build_script_disables_updater_artifacts_without_a_key(self):
        build = (ROOT / "scripts/build-app.sh").read_text(encoding="utf-8")
        self.assertIn('createUpdaterArtifacts\\":false', build.replace('"', '\\"'))
        self.assertRegex(build, r"UPDATER_BUILD_ARGS=\(\)")


if __name__ == "__main__":
    unittest.main()
