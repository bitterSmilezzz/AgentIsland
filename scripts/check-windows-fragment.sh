#!/bin/bash
# 在 macOS 上验证 Windows 专用代码能否编译。
#
# 为什么需要它：本仓的 Windows 缺口（`libc::getppid` / `SIGKILL` / `gethostname`、
# `std::os::unix::fs::PermissionsExt`、以及 placement 的四个工作区函数）**在本机构建里
# 根本不会被编译**——`#[cfg(windows)]` 那一支在 macOS 上零覆盖。后果是改错了也看不出来：
# 本仓实测过一次把 `GetCurrentProcessId` 写在 `unsafe` 块外（windows-sys 里它是
# unsafe fn），以及把 `GetComputerNameW` 的模块路径猜成
# `Win32_System::SystemInformation`（真实位置是 `Win32_System_WindowsProgramming`）。
# 两处都只有 Windows 编译器能发现。
#
# 为什么不能直接 `cargo check --target x86_64-pc-windows-msvc`：
# `rusqlite` 硬编码了 `bundled`，要编 SQLite 的 C 源码，而本机没有 Windows SDK 头文件，
# 会死在 `fatal error: 'stdlib.h' file not found`。那是**环境**问题，与我们的 Rust 代码无关，
# 但它会让整条交叉编译路径看起来「走不通」——本仓曾因此得出「Windows 代码无法本地验证」
# 的错误结论，并据此给过错误的停止建议。
#
# 办法：把待验证的片段提取到一个独立小 crate（只依赖 windows-sys），单独交叉编译。
# 真实符号、真实签名、真实 unsafe 规则，一样都不少。
#
# 用法：
#   scripts/check-windows-fragment.sh                 # 验证登记在册的全部片段
#   scripts/check-windows-fragment.sh --list          # 只看有哪些片段
#   scripts/check-windows-fragment.sh cleaner smtp    # 只验证指定片段
#
# 新增一个片段：在 FRAGMENTS 里加一行 `名字|文件|起始锚点`，起始锚点必须能唯一定位
# 到那个 `#[cfg(windows)]` / `#[cfg(not(unix))]` 属性所在的行。
#
# ── 铁律：这个脚本只能「真的编了」才说通过 ────────────────────────────────
# 提取失败、锚点找不到、cargo 没跑起来，都必须是非零退出。空跑通过比失败更危险——
# 它会让人以为 Windows 那一支已经验过了。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DIR="$REPO_ROOT/app/src-tauri/src"
PROBE_DIR="$(mktemp -d)"
TARGET="x86_64-pc-windows-msvc"

# 名字|文件|起始锚点（锚点需在文件中唯一）
FRAGMENTS=(
  "cleaner|cleaner.rs|#[cfg(windows)]
pub fn self_and_ancestors()"
  "webhook|webhook.rs|#[cfg(not(unix))]
fn write_new_token"
  "smtp|smtp.rs|#[cfg(windows)]
pub fn hostname()"
)

# 需要真实 tauri 的片段：这些函数的签名里带 `tauri::WebviewWindow` 等类型。
# ⚠️ 曾经这里用手写 Tauri stub 验——那只能证明语法与类型推导，
# **证明不了真实 API 签名匹配**，等于给出虚假的安全感。
# tauri 2.12.0 已在本地 cargo 缓存里（主 crate 的 Cargo.lock 锁的就是它），
# 配上那份 Cargo.lock 就能离线交叉编译。
#
# ⚠️⚠️ **必须把调用方一起提进来，并且让 `main` 真的引用它。**
#
# v0.0.325 实测过的失效模式：只提取 4 个工作区函数 + 一个空 `fn main() {}`，
# 然后把 `work_area_under_window` 重新加上 `#[cfg(not(windows))]`
# （也就是最初那个缺陷）——脚本**照样报编译通过**。
# 原因：被 `#[cfg]` 排除的函数只是从文件里消失，而没有任何代码引用它，
# 编译器毫无意见。
#
# 修法：保留调用方 `work_area_for_mode` 与 `named_monitor`，并让 `main`
# 显式引用全部登记函数，包括不在调用方路径上的 `work_area_at_logical`。
# 这样门控一加回去就报 `cannot find function \`work_area_under_window\``，
# 与 Windows CI 的真实错误逐字一致。已实测判别力：rc=101。
#
# 名字|文件|要提取的函数名（逗号分隔，第一个必须是调用方）
TAURI_FRAGMENTS=(
  "placement|placement.rs|work_area_for_mode,named_monitor,work_area_under_window,work_area_at_logical,physical_rect_to_logical,fallback_work_area"
)

# ⚠️ **这个脚本仍然有一个边界。** 它证明的是「这几个函数连同它们的调用方
# 能在 Windows 上编」，不是「整个 crate 能在 Windows 上编」。
# 跨模块的调用、以及 `cargo test` 在 Windows 上的结果，只有 Windows runner
# 知道（见 .github/workflows/build.yml 的 windows job，别删它）。

cleanup() { rm -rf "$PROBE_DIR"; }
trap cleanup EXIT

mkdir -p "$PROBE_DIR/src"
cat >"$PROBE_DIR/Cargo.toml" <<'EOF'
[package]
name = "winfragment"
version = "0.0.0"
edition = "2021"

[dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_System_Diagnostics_ToolHelp",
    "Win32_System_Threading",
    "Win32_System_WindowsProgramming",
] }
EOF

# 片段在原文件里能用 `super::*` / 同文件的 use 拿到的类型，这里必须显式补。
# 少一个导入，编译就红——那正是这个脚本要抓的「凭记忆写」类错误。
PRELUDE='#![allow(dead_code, unused_imports)]
use std::collections::{HashMap, HashSet};
'

# 从锚点所在行提取到与之花括号配对的右括号（含属性行本身）。
extract() {
  python3 - "$1" "$2" <<'PY'
import sys
from pathlib import Path

path, anchor = Path(sys.argv[1]), sys.argv[2]
src = path.read_text(encoding='utf-8')
if src.count(anchor) != 1:
    sys.exit(f"锚点不唯一或不存在（命中 {src.count(anchor)} 次）: {anchor!r}")
anchor_start = src.index(anchor)
start = src.rfind('\n', 0, anchor_start) + 1
while start > 0:
    prev_end = start - 1
    prev_start = src.rfind('\n', 0, prev_end) + 1
    prev = src[prev_start:prev_end].strip()
    if prev.startswith('///') or prev.startswith('//!'):
        start = prev_start
    elif prev.endswith(']'):
        block_start = prev_start
        balance = prev.count(']') - prev.count('[')
        while balance > 0 and block_start > 0:
            line_end = block_start - 1
            block_start = src.rfind('\n', 0, line_end) + 1
            line = src[block_start:line_end].strip()
            balance += line.count(']') - line.count('[')
        if balance != 0 or not src[block_start:prev_end].lstrip().startswith('#['):
            break
        start = block_start
    elif prev.endswith('*/'):
        block_start = src.rfind('/*', 0, prev_end)
        if block_start < 0 or src[block_start:block_start + 3] not in ('/**', '/*!'):
            break
        line_start = src.rfind('\n', 0, block_start) + 1
        if src[line_start:block_start].strip():
            break
        start = line_start
    else:
        break
i = src.index('{', anchor_start)
depth = 0
for j in range(i, len(src)):
    if src[j] == '{':
        depth += 1
    elif src[j] == '}':
        depth -= 1
        if depth == 0:
            print(src[start:j + 1])
            break
else:
    sys.exit("找不到配对的右括号——提取范围没闭合")
PY
}

# 按函数名提取多个函数（这些函数的签名里带 tauri 类型，需要真实依赖）。
#
# ⚠️ **必须把函数上方的属性行一起提出来。** 早先只从 `fn name(` 回溯到行首，
# 于是 `#[cfg(not(windows))]` 这类属性被丢掉——把门控加回去的破坏
# 脚本根本看不见（实测：加了门控仍然报"编译通过"）。
# 那正是这条脚本要防的事，所以这里向上扫到空行/非属性行为止。
extract_fns() {
  python3 - "$1" "$2" <<'PY'
import sys
from pathlib import Path

path, names = Path(sys.argv[1]), [n.strip() for n in sys.argv[2].split(',') if n.strip()]
src = path.read_text(encoding='utf-8')
out = []
for name in names:
    needle = f'fn {name}('
    if src.count(needle) != 1:
        sys.exit(f"函数 {name} 不唯一或不存在（命中 {src.count(needle)} 次）")
    idx = src.index(needle)
    # 向上吞掉连续的属性 / 文档注释行，直到遇到空行或普通代码行
    line_start = src.rindex('\n', 0, idx) + 1
    while True:
        prev_end = line_start - 1
        if prev_end <= 0:
            break
        prev_start = src.rindex('\n', 0, prev_end) + 1
        prev = src[prev_start:prev_end].strip()
        if prev.startswith('#[') or prev.startswith('///') or prev.startswith('//!'):
            line_start = prev_start
        else:
            break
    i = src.index('{', idx)
    depth = 0
    for j in range(i, len(src)):
        if src[j] == '{':
            depth += 1
        elif src[j] == '}':
            depth -= 1
            if depth == 0:
                out.append(src[line_start:j + 1])
                break
    else:
        sys.exit(f"函数 {name} 的花括号没闭合")
print('\n\n'.join(out))
PY
}

requested=("$@")

if [[ $# -eq 1 && "$1" == "--list" ]]; then
  echo "已登记的 Windows 片段："
  for entry in "${FRAGMENTS[@]}" "${TAURI_FRAGMENTS[@]}"; do
    name="${entry%%|*}"
    rest="${entry#*|}"
    file="${rest%%|*}"
    echo "  $name  ←  app/src-tauri/src/$file"
  done
  exit 0
fi

# Validate every name before invoking rustup or Cargo; a partial match is not success.
for r in "$@"; do
  known=0
  for entry in "${FRAGMENTS[@]}" "${TAURI_FRAGMENTS[@]}"; do
    if [[ "$r" == "${entry%%|*}" ]]; then known=1; break; fi
  done
  if [[ $known -eq 0 ]]; then
    echo "✗ 未知片段名称: ${r} (--list 必须单独使用)" >&2
    exit 1
  fi
done
[[ ${#requested[@]} -eq 0 ]] && requested=("__all__")

# target 没装就装；装不上是非零退出，不许静默跳过。
if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
  echo "安装 $TARGET ..."
  rustup target add "$TARGET" >/dev/null 2>&1 || {
    echo "✗ 装不上 ${TARGET} , 无法验证 Windows 片段" >&2
    exit 1
  }
fi

failed=0
checked=0
for entry in "${FRAGMENTS[@]}"; do
  name="${entry%%|*}"
  rest="${entry#*|}"
  file="${rest%%|*}"
  anchor="${rest#*|}"

  if [[ "$requested" != "__all__" ]]; then
    want=0
    for r in "${requested[@]}"; do
      if [[ "$r" == "$name" ]]; then want=1; fi
    done
    if [[ $want -eq 0 ]]; then continue; fi
  fi

  checked=$((checked + 1))
  echo "-- $name ($file)"

  if ! extract "$SRC_DIR/$file" "$anchor" >"$PROBE_DIR/src/lib.rs"; then
    echo "✗ $name: 提取失败" >&2
    failed=$((failed + 1))
    continue
  fi

  # 把前置导入拼到片段之前（片段在原文件里靠同文件的 use 拿这些类型）。
  printf '%s' "$PRELUDE" >"$PROBE_DIR/src/fragment.rs"
  cat "$PROBE_DIR/src/lib.rs" >>"$PROBE_DIR/src/fragment.rs"
  mv "$PROBE_DIR/src/fragment.rs" "$PROBE_DIR/src/lib.rs"

  # Reference the registered symbol without executing any system operations.
  if ! entry_fn="$(python3 - "$anchor" <<'PY'
import re
import sys
match = re.search(r'\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b', sys.argv[1])
if match is None:
    sys.exit("锚点中找不到函数名")
print(match.group(1))
PY
  )"; then
    echo "✗ $name: 入口函数名解析失败" >&2
    failed=$((failed + 1))
    continue
  fi
  printf '\n#[allow(dead_code)]\nfn main() { let _ = %s as usize; }\n' "$entry_fn" \
    >>"$PROBE_DIR/src/lib.rs"

  # --offline：windows-sys 已在本地缓存里（主 crate 就在用它）。
  if cargo check --offline --target "$TARGET" --manifest-path "$PROBE_DIR/Cargo.toml" \
      >"$PROBE_DIR/out.log" 2>&1; then
    echo "  ✓ Windows target 编译通过"
  else
    echo "  ✗ Windows target 编译失败：" >&2
    sed 's/^/    /' "$PROBE_DIR/out.log" >&2
    failed=$((failed + 1))
  fi
  rm -f "$PROBE_DIR/src/lib.rs"
done

# ── 需要真实 tauri 的片段 ────────────────────────────────────────────────
# 换一个 probe crate：依赖 tauri（版本与主 crate 的 Cargo.lock 一致），
# 并复用主 crate 的 Cargo.lock，这样整棵依赖树都能离线解析。
for entry in "${TAURI_FRAGMENTS[@]}"; do
  name="${entry%%|*}"
  rest="${entry#*|}"
  file="${rest%%|*}"
  fns="${rest#*|}"

  if [[ "$requested" != "__all__" ]]; then
    want=0
    for r in "${requested[@]}"; do
      if [[ "$r" == "$name" ]]; then want=1; fi
    done
    if [[ $want -eq 0 ]]; then continue; fi
  fi

  checked=$((checked + 1))
  echo "-- $name ($file, 真实 tauri)"

  mkdir -p "$PROBE_DIR/tauri/src"
  cat >"$PROBE_DIR/tauri/Cargo.toml" <<'EOF'
[workspace]

[package]
name = "winfragment-tauri"
version = "0.0.0"
edition = "2021"

[dependencies]
tauri = { version = "2", features = ["macos-private-api", "tray-icon", "image-png"] }
EOF

  # 没有锁文件就说明主 crate 的 Cargo.lock 不在预期位置——那是失败，不是跳过。
  if [[ ! -f "$REPO_ROOT/app/src-tauri/Cargo.lock" ]]; then
    echo "✗ $name: 找不到 app/src-tauri/Cargo.lock，无法锁定 tauri 版本" >&2
    failed=$((failed + 1))
    continue
  fi
  cp "$REPO_ROOT/app/src-tauri/Cargo.lock" "$PROBE_DIR/tauri/Cargo.lock"

  if ! extract_fns "$SRC_DIR/$file" "$fns" >"$PROBE_DIR/tauri/src/main.rs"; then
    echo "✗ $name: 提取失败" >&2
    failed=$((failed + 1))
    continue
  fi
  # All registered symbols must exist, including functions outside the caller's path.
  printf '\n#[allow(dead_code)]\nfn main() {\n' >>"$PROBE_DIR/tauri/src/main.rs"
  IFS=',' read -r -a entry_fns <<<"$fns"
  for entry_fn in "${entry_fns[@]}"; do
    printf '    let _ = %s as usize;\n' "$entry_fn" >>"$PROBE_DIR/tauri/src/main.rs"
  done
  printf '}\n' >>"$PROBE_DIR/tauri/src/main.rs"

  if cargo check --offline --target "$TARGET" --manifest-path "$PROBE_DIR/tauri/Cargo.toml" \
      >"$PROBE_DIR/tauri-out.log" 2>&1; then
    echo "  ✓ Windows target 编译通过（真实 tauri API）"
  else
    echo "  ✗ Windows target 编译失败：" >&2
    sed 's/^/    /' "$PROBE_DIR/tauri-out.log" >&2
    failed=$((failed + 1))
  fi
  rm -rf "$PROBE_DIR/tauri"
done

# 一个片段都没验到 = 调用方写错了名字，那也是失败。
if [[ $checked -eq 0 ]]; then
  echo "✗ 没有匹配到任何已登记片段: ${*}" >&2
  exit 1
fi

if [[ $failed -gt 0 ]]; then
  echo "✗ $failed / $checked 个片段编译失败" >&2
  exit 1
fi
echo "✓ $checked 个 Windows 片段全部通过交叉编译验证"
