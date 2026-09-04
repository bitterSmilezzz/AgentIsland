#!/bin/bash
# 打包 AgentIsland.app（无 Xcode 环境：swift build + 手工 .app 结构 + ad-hoc 签名）
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="AgentIsland"
BUILD_DIR=".build/release"
APP_DIR="dist/$APP_NAME.app"

echo "==> Release 构建（仅主产品）"
swift build -c release --product AgentIsland

echo "==> 生成图标"
ICON_DIR="/tmp/agentisland-icon.iconset"
rm -rf "$ICON_DIR"
swift scripts/make-icon.swift "$ICON_DIR" >/dev/null
iconutil -c icns "$ICON_DIR" -o "$ICON_DIR/AppIcon.icns"

echo "==> 组装 .app"
rm -rf "dist"
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
    <key>CFBundleVersion</key><string>1.2.0</string>
    <key>CFBundleShortVersionString</key><string>1.2.0</string>
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
