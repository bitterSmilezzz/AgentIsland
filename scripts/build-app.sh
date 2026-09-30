#!/bin/bash
# 打包 Rust/Tauri 应用与 Rust CLI，使用系统 SDK 和 ad-hoc 签名。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"
cd "$SCRIPT_DIR/.."

APP_DIR="dist/$APP_NAME.app"

# SDK 选择：**用机器上最新的那个**，不要钉死某个版本。
#
# 这条是被一个静默到极点的坑逼出来的（v0.0.235）：原来写死
# `MacOSX26.5.sdk`，而这台机器跑 macOS 27.0。拿 26.5 的 SDK 去链 WKWebView、
# 跑在 27.0 上 ⇒ **webview 根本不发起导航**，界面一片空白，而：
#   · 窗口建出来了、尺寸位置都对、`visible=true`
#   · 资源确实嵌进去了（index.html 1840B，MIME 正确）
#   · 命令全部注册了、WebView 内容进程也起来了
#   · **唯一征兆是 `on_page_load` 一次都不触发**
# 换成 `MacOSX27.0.sdk` 重新构建，**同一份代码，页面立刻加载完成**。
#
# 所以这条不是「保险起见」：**SDK 与运行时的 macOS 版本错配，会让 webview 静默失效**，
# 而症状看起来像「前端代码写错了」。钉死版本等于把这个坑永久焊死。
SDKROOT_CANDIDATES=(
    /Library/Developer/CommandLineTools/SDKs/MacOSX27.0.sdk
    /Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
    /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
    /Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk
    /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk
)
if [[ -z "${SDKROOT:-}" ]]; then
    for candidate in "${SDKROOT_CANDIDATES[@]}"; do
        if [[ -d "$candidate" ]]; then
            export SDKROOT="$candidate"
            break
        fi
    done
fi
if [[ -n "${SDKROOT:-}" ]]; then
    echo "==> 使用 SDK: $SDKROOT"
    # 钉住一个**旧于运行时**的 SDK 是上面那个坑的成因，而它不会报任何错。
    # 所以这里明说：拿到的 SDK 比系统还旧时把话说出来，而不是默默继续。
    if command -v sw_vers >/dev/null 2>&1; then
        system_major=$(sw_vers -productVersion | cut -d. -f1)
        sdk_major=$(basename "$SDKROOT" | sed -E 's/MacOSX([0-9]+).*/\1/')
        if [[ "$sdk_major" =~ ^[0-9]+$ && "$system_major" =~ ^[0-9]+$ && "$sdk_major" -lt "$system_major" ]]; then
            echo "!! 注意：系统 macOS ${system_major} 但 SDK 是 ${sdk_major}。webview 可能静默失效（见本脚本注释）" >&2
        fi
    fi
fi

# 版本号：$1 优先，否则从 CHANGELOG 首条版本抽取（单一事实源，替代手工双处硬编码）
VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
    VERSION=$(grep -m1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
fi
if [[ -z "$VERSION" ]]; then
    echo "✗ 无法解析版本号：CHANGELOG 首条缺少 '## [x.y.z]' 标题" >&2
    exit 1
fi

python3 scripts/check-version.py "$VERSION"
if [[ "${SKIP_TESTS:-0}" != "1" ]]; then
    cargo test --locked --manifest-path app/src-tauri/Cargo.toml
fi
mkdir -p dist

echo "==> 构建 Rust/Tauri 端（**主交付物**）"
( cd app/src-tauri && cargo tauri build --bundles app )
RUST_SRC="app/src-tauri/target/release/bundle/macos/${APP_NAME}.app"
if [[ ! -d "$RUST_SRC" ]]; then
    # 静默跳过 = 「构建失败但看起来成功」，那正是 views.js 潜伏 29 个版本的同款坑
    echo "!! Rust 端 .app 未产出：$RUST_SRC 不存在" >&2
    exit 1
fi

# 同一 Rust 二进制按 CLI 子命令分派；打包路径保持稳定。
cp app/src-tauri/target/release/agentisland dist/agentisland
chmod +x dist/agentisland
mkdir -p "$RUST_SRC/Contents/Helpers"
cp dist/agentisland "$RUST_SRC/Contents/Helpers/agentisland"

# 主名归 Rust 版。旧的主名此刻是 Swift 那个 bundle，先移开再放。
#
# ⚠️ `mv` 到一个**已存在且非空**的目录会失败（`Directory not empty`）——
# 第二次以后每次跑都撞这个。这里先清掉上一次的备份，否则「换主名」这件事
# 只在第一次能成功，之后一直静默失败。
PREVIOUS_APP="dist/.previous-${APP_NAME}.app"
if [[ -d "$PREVIOUS_APP" ]]; then
    rm -rf "$PREVIOUS_APP"
fi
if [[ -d "$APP_DIR" ]]; then
    mv "$APP_DIR" "$PREVIOUS_APP"
fi
cp -R "$RUST_SRC" "$APP_DIR"

# 旧名称是同一 Rust 应用的过期副本，留在可见目录会被再次打开。
# 移入隐藏归档，保留恢复能力；旧端产物不再用于当前开发。
for legacy in Rust Swift; do
    if [[ -d "dist/${APP_NAME}-${legacy}.app" ]]; then
        LEGACY_ARCHIVE=$(mktemp -d "dist/.retired-${APP_NAME}-${legacy}.XXXXXX")
        mv "dist/${APP_NAME}-${legacy}.app" "$LEGACY_ARCHIVE/"
        echo "==> 旧应用已归档：${LEGACY_ARCHIVE}"
    fi
done

echo "==> 签名（ad-hoc）"
codesign --force --sign - "dist/agentisland"
codesign --force --deep --sign - "$APP_DIR"

# ⚠️ 变量后面紧跟中文全角括号会被 bash 当成变量名的一部分
# （`$APP_DIR（` ⇒ 报 `APP_DIR…: unbound variable`）。这个坑踩了两次，
# 所以**所有变量与中文之间一律加花括号或空格**。
echo "==> 完成: $(pwd)/${APP_DIR}  —— Rust/Tauri 端；CLI 同为 Rust"
