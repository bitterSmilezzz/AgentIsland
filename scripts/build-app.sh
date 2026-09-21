#!/bin/bash
# 打包 AgentIsland.app（无 Xcode 环境：swift build + 手工 .app 结构 + ad-hoc 签名）
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="AgentIsland"
BUILD_DIR=".build/release"
APP_DIR="dist/$APP_NAME.app"

if [[ -z "${SDKROOT:-}" && -d "/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk" ]]; then
    export SDKROOT="/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk"
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

echo "==> Release 构建 v${VERSION}（主产品与 CLI 工具）"
swift build -c release --product AgentIsland
swift build -c release --product AgentIslandCLI

echo "==> 生成图标"
ICON_DIR="/tmp/agentisland-icon.iconset"
rm -rf "$ICON_DIR"
swift scripts/make-icon.swift "$ICON_DIR" >/dev/null
iconutil -c icns "$ICON_DIR" -o "$ICON_DIR/AppIcon.icns"

echo "==> 组装 .app 与 CLI 工具"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources" "dist"
cp "$BUILD_DIR/$APP_NAME" "$APP_DIR/Contents/MacOS/"
cp "$ICON_DIR/AppIcon.icns" "$APP_DIR/Contents/Resources/"
cp "$BUILD_DIR/AgentIslandCLI" "dist/agentisland"
chmod +x "dist/agentisland"
mkdir -p "$APP_DIR/Contents/Helpers"
cp "$BUILD_DIR/AgentIslandCLI" "$APP_DIR/Contents/Helpers/agentisland"
chmod +x "$APP_DIR/Contents/Helpers/agentisland"

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
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

echo "==> 签名（ad-hoc）"
codesign --force --sign - "dist/agentisland"
codesign --force --deep --sign - "$APP_DIR"

echo "==> 完成: $(pwd)/$APP_DIR"
