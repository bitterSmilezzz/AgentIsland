#!/usr/bin/env python3
"""Render the static Tauri v2 updater manifest from an existing signed macOS bundle.

Why this exists: manually copying release URLs and signatures can publish a manifest
for a missing or wrong bundle. Require the archive and its sibling signature first,
read version/architecture from the archive to reject stale or mislabeled artifacts,
and make --check read-only so a release gate can detect drift.

The generated markers live in the optional notes string: HTML comments outside the
JSON would break Tauri's parser. This prepares release metadata; uploading the archive,
.sig and latest.json to the matching GitHub release remains the publisher's job.
Official contract: https://v2.tauri.app/plugin/updater/#static-json-file
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import plistlib
import re
import struct
import sys
import tarfile
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[2]
BEGIN = "<!-- generated:update-manifest:begin -->"
END = "<!-- generated:update-manifest:end -->"
RELEASES = "https://github.com/bitterSmilezzz/AgentIsland/releases/download"
CPU_ARCH = {0x01000007: "x86_64", 0x0100000C: "aarch64"}


def bundle_arches(archive: Path, version: str) -> list[str]:
    with tarfile.open(archive, "r:gz") as bundle:
        infos = [m for m in bundle.getmembers()
                 if m.isfile() and m.name.endswith(".app/Contents/Info.plist")]
        if len(infos) != 1:
            raise ValueError("archive must contain exactly one app Info.plist")
        info_file = bundle.extractfile(infos[0])
        assert info_file is not None
        info = plistlib.loads(info_file.read())
        if info.get("CFBundleShortVersionString") != version:
            raise ValueError("archive version does not match tauri.conf.json")
        executable = info.get("CFBundleExecutable", "")
        if not executable or "/" in executable:
            raise ValueError("invalid CFBundleExecutable in archive")
        binary = bundle.extractfile(infos[0].name.removesuffix("Info.plist") + "MacOS/" + executable)
        if binary is None:
            raise ValueError("archive executable missing")
        header = binary.read(8)
        magic = header[:4]
        if magic in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf"):
            endian = "<" if magic == b"\xcf\xfa\xed\xfe" else ">"
            cpus = [struct.unpack(endian + "I", header[4:])[0]]
        elif magic in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
            count = struct.unpack(">I", header[4:])[0]
            if not 1 <= count <= 16:
                raise ValueError("invalid universal Mach-O architecture count")
            width = 32 if magic == b"\xca\xfe\xba\xbf" else 20
            cpus = [struct.unpack(">I", binary.read(width)[:4])[0] for _ in range(count)]
        else:
            raise ValueError("archive executable is not a supported macOS Mach-O")
        if any(cpu not in CPU_ARCH for cpu in cpus):
            raise ValueError("unsupported macOS architecture in archive")
        return sorted({CPU_ARCH[cpu] for cpu in cpus})


NOTES_TEMPLATE = "AgentIsland {version}（{date}）。安装包已做 ad-hoc 签名；未公证，首次打开需右键 → 打开。"


class NoSignedBundle(Exception):
    """没有可用的签名更新包——**正常状态**，不是缺陷。

    干净克隆、只跑过普通构建的机器上都不会有 `.app.tar.gz`。
    这种情况下 `--check` 应当安静通过（无物可校验），
    而 `--write` 必须失败（没有产物却写出清单 = 伪造发布信息）。
    """


def render(root: Path) -> str:
    config = json.loads((root / "app/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
    version = config["version"]
    if not isinstance(version, str) or not re.fullmatch(
        r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version
    ):
        raise ValueError("tauri.conf.json version must be a SemVer string")
    directory = root / "app/src-tauri/target/release/bundle/macos"
    archives = sorted(directory.glob("*.app.tar.gz"))
    if not archives:
        raise NoSignedBundle(f"no .app.tar.gz in {directory}")
    if len(archives) != 1:
        raise ValueError(f"expected exactly one .app.tar.gz in {directory}, found {len(archives)}")
    archive = archives[0]
    if not archive.is_file() or archive.stat().st_size == 0:
        raise ValueError("updater archive missing or empty")
    signature_path = archive.with_name(archive.name + ".sig")
    signature = signature_path.read_text(encoding="utf-8").strip()
    if not signature:
        raise ValueError("updater .sig is empty")
    arches = bundle_arches(archive, version)
    pub_date = datetime.fromtimestamp(archive.stat().st_mtime, timezone.utc).isoformat().replace("+00:00", "Z")
    payload = {
        "version": version,
        # `notes` 是 Tauri 展示给用户的更新说明，**不是给生成器看的**。
        # 初版把它写成 `<!-- generated:… -->` 块标记，那是把两个用途混在一起：
        # 一旦界面把 notes 渲染出来，用户就会看见 HTML 注释。
        # 清单是纯 JSON、没有块标记的必要（`--check` 比对整份文件即可）。
        "notes": NOTES_TEMPLATE,
        "pub_date": pub_date,
        "platforms": {
            "darwin-" + arch: {
                "signature": signature,
                "url": f"{RELEASES}/v{quote(version, safe='')}/{quote(archive.name, safe='')}",
            } for arch in arches
        },
    }
    payload["notes"] = NOTES_TEMPLATE.format(version=version, date=pub_date[:10])
    return json.dumps(payload, ensure_ascii=False, indent=2) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    parser.add_argument("--root", type=Path, default=ROOT, help="repository root (isolated fixture support)")
    args = parser.parse_args()
    output = args.root / "dist/latest.json"
    try:
        expected = render(args.root)
    except NoSignedBundle as absent:
        # 「本机还没构建签名更新包」是**正常状态**，不是缺陷。
        #
        # 初版把两种情况都判成失败，于是 `verify-framework.sh` 在任何没跑过
        # 带签名构建的干净克隆上恒红（实测 test-framework-verifier.py 20 项里 5 项红）——
        # 夹具环境没有 .app.tar.gz，于是每个用例都挂在这一条上。
        #
        # `--write` 仍然必须失败：没有产物时**编一份清单**就是伪造发布信息。
        if args.write:
            print(f"updater manifest: {absent}", file=sys.stderr)
            return 1
        print("updater manifest: no signed bundle present; nothing to check (not a drift)")
        return 0
    try:
        if args.write:
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(expected, encoding="utf-8")
            print("updater manifest regenerated: dist/latest.json")
            return 0
        if not output.is_file() or output.read_text(encoding="utf-8") != expected:
            raise ValueError("updater manifest missing or drifted; run render-update-manifest.py --write")
        print("updater manifest matches signed macOS bundle")
        return 0
    except (OSError, ValueError, KeyError, tarfile.TarError, struct.error, plistlib.InvalidFileException) as error:
        print(f"updater manifest: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
