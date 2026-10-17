#!/bin/bash
# 在 macOS 上对**整个 crate**做 Windows 目标的 Rust 类型检查。
#
# 实测「同一缺陷、两道都抓」：把 `smtp.rs::hostname` 的 `#[cfg(windows)]` 改成
# `#[cfg(any())]`（函数在所有平台消失）后——片段门禁报 E0425（生成的 main 显式引用
# 它，引用不到）；本脚本报 E0425，但来自 **bin 目标** 的真实调用方。注意
# `--lib-only` 抓不到它（只剩 dead_code 警告），所以别拿 --lib-only 当等价物。
# 而本脚本的另一半价值片段门禁给不了：本仓整 crate `--tests` 检查一次找出 9 个片段
# 门禁看不见的真实错误——`webhook.rs` 的令牌测试无条件使用 Unix 权限 API 与
# `libc::umask`，`forecast.rs` 的日期夹具用 Windows 不提供的 `libc::mktime`。
# 那些错误在本机（macOS）永远看不见，因为测试在 Unix 上编得过。
#
# 为什么需要绕两步（都不是我们的代码问题）：
#   1. `rusqlite` 挂着 `bundled`，要编 SQLite 的 C 源码，而本机没有 Windows SDK
#      头文件，死在 `fatal error: 'stdlib.h' file not found`。
#      解法：`LIBSQLITE3_SYS_USE_PKG_CONFIG=1` 让 libsqlite3-sys 走 linked 分支，
#      小 crate 复制预生成绑定，不编 C、不探测本机库（见下面的禁用变量）。
#   2. Tauri 的 Windows 资源编译要 `llvm-rc`（或 `windres`），本机没有。
#      解法：在**临时副本**里把 tauri-build 的资源分支关掉。
#
# ── 铁律：这里通过 ≠ Windows 构建成功 ────────────────────────────────────
# 它证明的是「排除 Windows 资源编译后，这些 Rust 源码能在 Windows 目标下
# 通过类型检查」。它**不**证明：资源编译、最终链接、打包、Windows 上的
# `cargo test` 实际执行、或程序能在 Windows 上跑起来。
# 输出里必须把这句打出来，不许只报一个 ✓ 就完事。
#
# 用法：
#   scripts/check-windows-crate.sh             # 默认范围 + 测试（--all-targets）
#   scripts/check-windows-crate.sh --lib-only  # 只查默认范围
#
# 主仓只读：全部写入落在临时目录；任何一步失败都是非零退出。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="x86_64-pc-windows-msvc"
TARGETS="--all-targets"
if [[ "${1:-}" == "--lib-only" ]]; then
  TARGETS=""
elif [[ $# -gt 0 ]]; then
  echo "✗ 未知参数: $*（只接受 --lib-only）" >&2
  exit 1
fi

WORK="$(mktemp -d)"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# 依赖编译是这里最贵的一步。缓存目录留在仓内 `.build/`（已忽略、不交付），
# 于是只有本仓自己的代码每次重编：首次分钟级，之后秒级。
# ⚠️ 代价：缓存约 **2.0G**（2026-10-16 实测）。不需要时删 `.build/windows-typecheck-target`
# 即可，下一次会自动重建。覆盖它只是为了隔离实验，正常调用不要动。
CACHE="${WINDOWS_TYPECHECK_CACHE:-$REPO_ROOT/.build/windows-typecheck-target}"
mkdir -p "$CACHE"

echo "-- 快照（只读复制 tracked + 未忽略的未跟踪文件，主仓不改动）"
python3 - "$REPO_ROOT" "$WORK/source" <<'PY'
import os
import shutil
import subprocess
import sys
from pathlib import Path

root, destination_root = Path(sys.argv[1]), Path(sys.argv[2])
listing = subprocess.run(
    ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
    cwd=root, check=True, capture_output=True).stdout
destination_root.mkdir(parents=True)
copied = 0
for raw in sorted(set(listing.split(b'\0')) - {b''}):
    name = os.fsdecode(raw)
    source = root / name
    if not source.exists() and not source.is_symlink():
        continue
    destination = destination_root / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        destination.symlink_to(os.readlink(source))
    elif source.is_file():
        shutil.copy2(source, destination)
    copied += 1
if copied < 100:
    sys.exit(f'快照只复制了 {copied} 个文件，采集器可能失效')
print(f'   {copied} 个文件')
PY

echo "-- 临时副本里关闭 tauri-build 的 Windows 资源分支"
python3 - "$WORK" <<'PY'
import shutil
import subprocess
import sys
from pathlib import Path

work = Path(sys.argv[1])
# 版本必须与主 crate 的锁定一致，否则 patch 对不上任何依赖。
lock = (work / 'source/app/src-tauri/Cargo.lock').read_text(encoding='utf-8')
needle = '\nname = "tauri-build"\nversion = "'
if lock.count(needle) != 1:
    sys.exit('Cargo.lock 里 tauri-build 条目不唯一，无法确定版本')
version = lock.split(needle, 1)[1].split('"', 1)[0]

registry = Path.home() / '.cargo/registry/src'
candidates = sorted(registry.glob(f'*/tauri-build-{version}'))
if len(candidates) != 1:
    sys.exit(f'本地 cargo 缓存里 tauri-build-{version} 命中 {len(candidates)} 个，'
             '无法确定要复制哪一个')
source = candidates[0]

vendor = work / 'source/vendor/tauri-build-windows-typecheck'
shutil.copytree(source, vendor)

lib = vendor / 'src/lib.rs'
text = lib.read_text(encoding='utf-8')
# 两处 Windows 条件：第一处把固定 WebView2 运行时追加进资源清单（要保留），
# 第二处才是资源编译。锚点用「后面紧跟 tauri_winres 导入」的那一处，避免误改。
anchor = 'if target_triple.contains("windows") {\n    use semver::Version;\n    use tauri_winres::'
if text.count(anchor) != 1:
    sys.exit(f'资源分支锚点命中 {text.count(anchor)} 次（应为 1），'
             f'tauri-build-{version} 的结构变了，别盲改')
lib.write_text(text.replace(
    anchor,
    'if false && target_triple.contains("windows") {\n    use semver::Version;\n    use tauri_winres::',
    1), encoding='utf-8')

manifest = work / 'source/app/src-tauri/Cargo.toml'
text = manifest.read_text(encoding='utf-8')
if 'tauri-build' in text.split('[patch.crates-io]', 1)[1]:
    sys.exit('临时 Cargo.toml 已有 tauri-build patch，锚点假设不成立')
manifest.write_text(text.rstrip('\n') + '\ntauri-build = { path = "../../vendor/tauri-build-windows-typecheck" }\n',
                    encoding='utf-8')
# lock 里该包的 registry 来源必须去掉，否则 path patch 不生效。
subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1',
                '--manifest-path', str(manifest)],
               cwd=work / 'source', check=True, capture_output=True)
print(f'   tauri-build {version} 已复制并关闭资源分支')
PY

echo "-- Windows 目标类型检查: cargo check ${TARGETS:-(default)}"
LOG="$WORK/check.log"
# shellcheck disable=SC2086
if (cd "$WORK/source" && env \
      LIBSQLITE3_SYS_USE_PKG_CONFIG=1 \
      SQLITE3_NO_PKG_CONFIG=1 \
      VCPKGRS_NO_SQLITE3=1 \
      SQLITE3_STATIC=0 \
      PKG_CONFIG=/usr/bin/false \
      CARGO_TARGET_DIR="$CACHE" \
      CARGO_TERM_COLOR=never \
      cargo check --offline --target "$TARGET" $TARGETS \
        --manifest-path app/src-tauri/Cargo.toml) >"$LOG" 2>&1; then
  errors=0
  echo "  ✓ 无 Rust 错误（$(grep -c '^warning' "$LOG" || true) 条 warning 行）"
else
  echo "  ✗ 类型检查失败：" >&2
  grep -n -A6 '^error' "$LOG" | head -60 >&2
  errors=1
fi

cat >&2 <<'EOF'

注意：以上结果**不是** Windows 构建成功。本次在临时快照里关闭了 tauri-build 的
Windows 资源分支（本机没有 llvm-rc/windres），并且只做类型检查。
未覆盖：资源编译、最终链接、打包、Windows 上 `cargo test` 的实际执行、实机运行。
权威判据仍是 .github/workflows/build.yml 的 windows job。
EOF

if [[ "$errors" -ne 0 ]]; then
  exit 1
fi
echo "✓ 整 crate Windows 类型检查通过（排除资源编译；不是标准构建成功）"
