#!/usr/bin/env swift
// 生成 AgentIsland 应用图标（灵动岛风格：深色玻璃胶囊 + 绿色活动脉冲）
// 用法: swift scripts/make-icon.swift <输出目录>
import AppKit

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "/tmp/agentisland-icon.iconset"
try? FileManager.default.createDirectory(atPath: outDir, withIntermediateDirectories: true)

let specs: [(Int, String)] = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512@2x.png"),
]

func drawIcon(size: CGFloat) -> NSImage {
    let image = NSImage(size: NSSize(width: size, height: size))
    image.lockFocus()
    defer { image.unlockFocus() }

    let s = size

    // 背景圆角方块（深色玻璃：从 #29292b 到 #121214）
    let bgPath = NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: s, height: s),
                              xRadius: s * 0.2237, yRadius: s * 0.2237)
    let gradient = NSGradient(colors: [
        NSColor(calibratedRed: 0.16, green: 0.17, blue: 0.18, alpha: 1),   // #29292b
        NSColor(calibratedRed: 0.07, green: 0.07, blue: 0.08, alpha: 1),   // #121214
    ])!
    gradient.draw(in: bgPath, angle: -70)

    // 灵动岛胶囊（居中拉长）
    let pillW = s * 0.62, pillH = s * 0.26
    let pillRect = NSRect(x: (s - pillW) / 2, y: (s - pillH) / 2, width: pillW, height: pillH)
    let pill = NSBezierPath(roundedRect: pillRect, xRadius: pillH / 2, yRadius: pillH / 2)
    NSColor(calibratedRed: 0.0, green: 0.0, blue: 0.0, alpha: 1).setFill()
    pill.fill()

    // 胶囊描边（微光）
    NSColor(calibratedRed: 1, green: 1, blue: 1, alpha: 0.10).setStroke()
    pill.lineWidth = s * 0.012
    pill.stroke()

    // 左侧活动点（绿色脉冲 #30d158）
    let dotR = pillH * 0.18
    let dotCenter = NSPoint(x: pillRect.minX + pillW * 0.24, y: pillRect.midY)
    NSColor(calibratedRed: 0.19, green: 0.82, blue: 0.35, alpha: 1).setFill()
    NSBezierPath(ovalIn: NSRect(x: dotCenter.x - dotR, y: dotCenter.y - dotR,
                                width: dotR * 2, height: dotR * 2)).fill()

    // 右侧文字条（模拟状态文本）
    let barW = pillW * 0.42, barH = pillH * 0.16
    let barRect = NSRect(x: pillRect.minX + pillW * 0.44,
                         y: pillRect.midY - barH / 2, width: barW, height: barH)
    let bar = NSBezierPath(roundedRect: barRect, xRadius: barH / 2, yRadius: barH / 2)
    NSColor(calibratedRed: 1, green: 1, blue: 1, alpha: 0.75).setFill()
    bar.fill()

    return image
}

for (px, filename) in specs {
    let img = drawIcon(size: CGFloat(px))
    guard let tiff = img.tiffRepresentation,
          let rep = NSBitmapImageRep(data: tiff),
          let png = rep.representation(using: .png, properties: [:]) else { continue }
    try? png.write(to: URL(fileURLWithPath: "\(outDir)/\(filename)"))
}
print("done")
