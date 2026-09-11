#!/bin/bash
# 打包 AgentIsland.app（无 Xcode 环境：swift build + 手工 .app 结构 + ad-hoc 签名）
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="AgentIsland"
BUILD_DIR=".build/release"
APP_DIR="dist/$APP_NAME.app"

# 版本号：$1 优先，否则从 CHANGELOG 首条版本抽取（单一事实源，替代手工双处硬编码）
VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
    VERSION=$(grep -m1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
fi
if [[ -z "$VERSION" ]]; then
    echo "✗ 无法解析版本号：CHANGELOG 首条缺少 '## [x.y.z]' 标题" >&2
    exit 1
fi

# 版本一致性校验：README 功能版本必须同步（漂移即拒绝打包，杜绝三处各说各话）
README_VERSION=$(grep -m1 -oE '## 功能（v[0-9]+\.[0-9]+\.[0-9]+）' README.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
if [[ "$README_VERSION" != "$VERSION" ]]; then
    echo "✗ 版本漂移：CHANGELOG=$VERSION 但 README 功能版本=${README_VERSION:-缺失}。先同步 README 再发布。" >&2
    exit 1
fi

# 测试门禁：打包前全量测试（SKIP_TESTS=1 跳过，仅供快速冒烟）
if [[ "${SKIP_TESTS:-0}" != "1" ]]; then
    echo "==> 测试门禁（SKIP_TESTS=1 可跳过）"
    swift build --build-tests
    .build/debug/AgentIslandTestsRunner
fi

echo "==> Release 构建 v$VERSION（仅主产品）"
swift build -c release --product AgentIsland

echo "==> 生成图标"
ICON_DIR="/tmp/agentisland-icon.iconset"
rm -rf "$ICON_DIR"
swift scripts/make-icon.swift "$ICON_DIR" >/dev/null
iconutil -c icns "$ICON_DIR" -o "$ICON_DIR/AppIcon.icns"

echo "==> 组装 .app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BUILD_DIR/$APP_NAME" "$APP_DIR/Contents/MacOS/"
cp "$ICON_DIR/AppIcon.icns" "$APP_DIR/Contents/Resources/"

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
</dict>
</plist>
PLIST

echo "==> 签名（ad-hoc）"
codesign --force --deep --sign - "$APP_DIR"

echo "==> 完成: $(pwd)/$APP_DIR"
