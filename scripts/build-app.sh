#!/bin/bash
# 打包 AgentIsland.app（无 Xcode 环境：swift build + 手工 .app 结构 + ad-hoc 签名）
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="AgentIsland"
BUILD_DIR=".build/release"
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
            echo "!! 注意：系统 macOS $system_major 但 SDK 是 $sdk_major。webview 可能静默失效（见本脚本注释）" >&2
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

# 版本一致性校验：README 开头的「本文档描述 vX.Y.Z」必须同步（漂移即拒绝打包）。
# 校验的是「文档有没有跟着这版更新」，不是版本号写在哪——README 只留这一处版本，
# 逐版记录归 CHANGELOG（此前 README 也记一遍版本史，结果长出了 5 组重复小节和 0.0.35 的过期下载链接）
README_VERSION=$(grep -m1 -oE '本文档描述 \*\*v[0-9]+\.[0-9]+\.[0-9]+\*\*' README.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
if [[ "$README_VERSION" != "$VERSION" ]]; then
    echo "✗ 版本漂移：CHANGELOG=$VERSION 但 README 功能版本=${README_VERSION:-缺失}。先同步 README 再发布。" >&2
    exit 1
fi

# 版本单一来源校验：代码里的 AppVersion.string 必须与 CHANGELOG 首条一致。
# CLI 横幅、导出的 Raycast 清单与设置页都读它——曾经应用已到 v0.0.84 而 CLI 仍打印 v0.0.80。
CODE_VERSION=$(grep -m1 -oE 'public static let string = "[0-9]+\.[0-9]+\.[0-9]+"' Sources/AgentIslandCore/AppVersion.swift | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
if [[ "$CODE_VERSION" != "$VERSION" ]]; then
    echo "✗ 版本漂移：CHANGELOG=$VERSION 但 AppVersion.string=${CODE_VERSION:-缺失}。先改 AppVersion.swift 再发布。" >&2
    exit 1
fi

# 测试门禁：打包前全量测试（SKIP_TESTS=1 跳过，仅供快速冒烟）
if [[ "${SKIP_TESTS:-0}" != "1" ]]; then
    echo "==> 测试门禁（SKIP_TESTS=1 可跳过）"
    swift build --build-tests
    .build/debug/AgentIslandTestsRunner
fi

echo "==> 生成图标"
ICON_DIR="/tmp/agentisland-icon.iconset"
rm -rf "$ICON_DIR"
swift scripts/make-icon.swift "$ICON_DIR" >/dev/null
iconutil -c icns "$ICON_DIR" -o "$ICON_DIR/AppIcon.icns"

# Swift 版：降级为**回退产物**（本机保留，不进发布包）。
#
# v0.0.233 起用户拍板把交付物整个换成 Rust 端。理由不是「新写的更好」，
# 而是**口径**：在换之前，每次发版用户装到机器上、每天打开的都是 Swift 那个
# 二进制，Rust 端只以 CLI 的身份搭车——「迁移已完成到哪」这件事，
# 从用户视角根本看不出来。
#
# 仍然构建它，是为了**留一条回退路**：一个不留退路的切换不是切换，是砸东西。
# 出问题就 `open dist/AgentIsland-Swift.app`，一秒钟退回去。
if [[ "${SKIP_SWIFT:-0}" != "1" ]]; then
    # **前置检查**：`@State` / `@Binding` 这些 SwiftUI 属性包装器是**宏**，
    # 由 `SwiftUIMacros` 插件实现，而它是 **Xcode 闭源提供的**——
    # CommandLineTools 的 host/plugins 里只有 libObservationMacros 与 libSwiftMacros。
    # 没有它，整棵 SwiftUI 视图层编不出来，而且报错长得极不像这件事：
    # 先报「SwiftUIMacros.StateMacro could not be found」，再连锁出几十条
    # 「cannot find '$state' in scope」「self is immutable」「类型检查超时」，
    # 最后把一整条 4000 字符的 swift-frontend 命令行糊在脸上。
    # 那些 `self is immutable` **不是**代码写错了——先排掉这一层再去看代码。
    PLUGIN_DIR="/Library/Developer/CommandLineTools/usr/lib/swift/host/plugins"
    if ! ls "$PLUGIN_DIR" 2>/dev/null | grep -q SwiftUIMacros; then
        echo "✗ 这台机器的 Swift 工具链里没有 SwiftUIMacros 插件，Swift 版编不出来。" >&2
        echo "  xcode-select -p ⇒ $(xcode-select -p 2>/dev/null || echo '(未设置)')" >&2
        echo "  该插件由 Xcode 提供，只装了 CommandLineTools 时没有它；" >&2
        echo "  于是所有用 @State/@Binding 的 SwiftUI 视图都编不出来。" >&2
        echo >&2
        echo "  三选一：" >&2
        echo "    ① 装 Xcode（xcode-select -s /Applications/Xcode.app）后重跑；" >&2
        echo "    ② SKIP_SWIFT=1 跳过——代价是**没有 dist/${APP_NAME}-Swift.app 这条回退路**；" >&2
        echo "    ③ 确认不再需要回退路，然后把这一步从脚本里删掉。" >&2
        exit 1
    fi
    echo "==> 构建 Swift 版（回退产物：dist/${APP_NAME}-Swift.app；SKIP_SWIFT=1 可跳过）"
    swift build -c release --product AgentIsland
    swift build -c release --product AgentIslandCLI
    SWIFT_DIR="dist/${APP_NAME}-Swift.app"
    rm -rf "$SWIFT_DIR"
    mkdir -p "$SWIFT_DIR/Contents/MacOS" "$SWIFT_DIR/Contents/Resources" "$SWIFT_DIR/Contents/Helpers"
    cp "$BUILD_DIR/$APP_NAME" "$SWIFT_DIR/Contents/MacOS/"
    cp "$ICON_DIR/AppIcon.icns" "$SWIFT_DIR/Contents/Resources/"
    cp "$BUILD_DIR/AgentIslandCLI" "$SWIFT_DIR/Contents/Helpers/agentisland"
    chmod +x "$SWIFT_DIR/Contents/Helpers/agentisland"
    cat > "$SWIFT_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>AgentIsland</string>
    <key>CFBundleDisplayName</key><string>AgentIsland</string>
    <key>CFBundleIdentifier</key><string>com.agentisland.app</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>AgentIsland</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>© 2026 AgentIsland</string>
    <key>LSUIElement</key><true/>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key><string>com.agentisland.url</string>
            <key>CFBundleURLSchemes</key><array><string>agentisland</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST
    codesign --force --deep --sign - "$SWIFT_DIR"
    echo "==> Swift 版已就位（回退用）：$SWIFT_DIR"
fi

echo "==> 构建 Rust/Tauri 端（**主交付物**）"
( cd app/src-tauri && cargo tauri build --bundles app )
RUST_SRC="app/src-tauri/target/release/bundle/macos/${APP_NAME}.app"
if [[ ! -d "$RUST_SRC" ]]; then
    # 静默跳过 = 「构建失败但看起来成功」，那正是 views.js 潜伏 29 个版本的同款坑
    echo "!! Rust 端 .app 未产出：$RUST_SRC 不存在" >&2
    exit 1
fi

# CLI 工具：两边同名同位置，外部接入方（脚本 / Raycast）不用改路径
swift build -c release --product AgentIslandCLI
cp "$BUILD_DIR/AgentIslandCLI" "dist/agentisland"
chmod +x "dist/agentisland"
mkdir -p "$RUST_SRC/Contents/Helpers"
cp "$BUILD_DIR/AgentIslandCLI" "$RUST_SRC/Contents/Helpers/agentisland"
chmod +x "$RUST_SRC/Contents/Helpers/agentisland"

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

echo "==> 签名（ad-hoc）"
codesign --force --sign - "dist/agentisland"
codesign --force --deep --sign - "$APP_DIR"

# ⚠️ 变量后面紧跟中文全角括号会被 bash 当成变量名的一部分
# （`$APP_DIR（` ⇒ 报 `APP_DIR…: unbound variable`）。这个坑踩了两次，
# 所以**所有变量与中文之间一律加花括号或空格**。
echo "==> 完成: $(pwd)/${APP_DIR}  —— Rust/Tauri 端；回退用 dist/${APP_NAME}-Swift.app"
