import SwiftUI
import AppKit

// MARK: - Apple Design Tokens (from awesome-design-md/apple/DESIGN.md)

extension Color {
    init(hex: UInt32, alpha: Double = 1.0) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xFF) / 255.0,
                  green: Double((hex >> 8) & 0xFF) / 255.0,
                  blue: Double(hex & 0xFF) / 255.0,
                  opacity: alpha)
    }

    /// 动态色：跟随视图所在窗口的 effectiveAppearance 自动切换（面板强制浅/深色时同样生效）
    init(dynamic light: NSColor, dark: NSColor) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
        })
    }

    init(dynamicLight: UInt32, dark: UInt32) {
        self.init(dynamic: NSColor(hex: dynamicLight), dark: NSColor(hex: dark))
    }
}

extension NSColor {
    convenience init(hex: UInt32, alpha: Double = 1.0) {
        self.init(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255.0,
                  green: CGFloat((hex >> 8) & 0xFF) / 255.0,
                  blue: CGFloat(hex & 0xFF) / 255.0,
                  alpha: alpha)
    }
}

enum Theme {
    // Brand & Accent (Apple Action Blue)
    static let actionBlue = Color(hex: 0x0066cc)
    static let focusBlue = Color(hex: 0x0071e3)

    // 常规界面色板（设置窗口等，动态：深色系统下自动转暗）
    static let canvas = Color(dynamicLight: 0xffffff, dark: 0x151517)
    static let parchment = Color(dynamicLight: 0xf5f5f7, dark: 0x1e1e20)
    static let hairline = Color(dynamicLight: 0xe0e0e0, dark: 0x3a3a3c)

    // 灵动岛主体（动态：深色=黑曜石质感，浅色=白瓷工控）
    static let tile1 = Color(dynamicLight: 0xe9e9ec, dark: 0x141418)

    // Sydedock 专属工业触觉设计令牌（动态透光毛玻璃与原生 macOS HUD 质感）
    static let obsidianBase = Color(dynamicLight: 0xf5f5f7, dark: 0x0c0d14)
    static let obsidianCard = Color(dynamicLight: 0xffffff, dark: 0x161722)
    static let obsidianPill = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.05), dark: NSColor(hex: 0xffffff, alpha: 0.09))
    static let obsidianHairline = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.08), dark: NSColor(hex: 0xffffff, alpha: 0.14))
    static let obsidianCardFill = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.04), dark: NSColor(hex: 0xffffff, alpha: 0.07))
    static let obsidianCardHoverFill = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.08), dark: NSColor(hex: 0xffffff, alpha: 0.13))
    static let obsidianCardBorder = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.08), dark: NSColor(hex: 0xffffff, alpha: 0.14))
    static let obsidianCardBorderHover = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.16), dark: NSColor(hex: 0xffffff, alpha: 0.28))

    // Sydedock 荧光强调色（深色模式绚丽高亮，浅色模式加深保障可读对比度）
    static let sydedockCyan = Color(dynamicLight: 0x0284c7, dark: 0x00e1ff)
    static let sydedockBlue = Color(dynamicLight: 0x1d4ed8, dark: 0x38bdf8)
    static let sydedockAmber = Color(dynamicLight: 0xb45309, dark: 0xffd60a)
    static let sydedockEmerald = Color(dynamicLight: 0x15803d, dark: 0x30d158)

    /// 立体黑曜石药丸背景渐变（Sydedock Pill）
    static var pillGradient: LinearGradient {
        LinearGradient(
            colors: [
                Color(dynamicLight: 0xf0f0f3, dark: 0x202028),
                Color(dynamicLight: 0xe4e4e8, dark: 0x131318)
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }

    /// 高级工控状态轨底色渐变
    static var trackGradient: LinearGradient {
        LinearGradient(
            colors: [
                sydedockCyan,
                sydedockBlue
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
    }

    // Risk accents：浅色模式加深保证对比度
    static let dangerRed = Color(dynamicLight: 0xd32f2f, dark: 0xff3b30)
    static let warningOrange = Color(dynamicLight: 0xb45309, dark: 0xff9500)

    // 文本（动态：深色模式纯净象牙白与清透柔银，浅色模式墨水黑与工控灰，HIG 对比度 ≥4.5:1 / 7:1）
    static let ink = Color(dynamicLight: 0x0f172a, dark: 0xf8fafc)
    static let inkMuted80 = Color(dynamicLight: 0x334155, dark: 0xcfd4dc)
    static let inkMuted48 = Color(dynamicLight: 0x64748b, dark: 0x94a3b8)
    static let onDark = Color(dynamicLight: 0x0f172a, dark: 0xffffff)          // 主文字：深色纯白 100%，浅色黑蓝
    static let onDarkMuted = Color(dynamicLight: 0x334155, dark: 0xe2e8f0)     // 次要文字：清透软银色
    static let onDarkFaint = Color(dynamicLight: 0x64748b, dark: 0x94a3b8)     // 弱化信息：清晰可辨浅石板灰

    // 悬停/按压蒙层（浅色黑低透，深色白低透）
    static let hoverFill = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.08), dark: NSColor(hex: 0xffffff, alpha: 0.12))
    static let chipFill = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.05), dark: NSColor(hex: 0xffffff, alpha: 0.08))
    static let cardFill = Color(dynamic: NSColor(hex: 0x000000, alpha: 0.04), dark: NSColor(hex: 0xffffff, alpha: 0.07))

    // Status（浅色模式加深保证对比度）
    static let statusWorking = Color(dynamicLight: 0x157f3c, dark: 0x30d158)
    static let statusIdle = Color(dynamicLight: 0x8f6a00, dark: 0xffd60a)
    static let statusOffline = Color(dynamicLight: 0x5c5c61, dark: 0x8e8e93)

    // 面板视觉（细条/蒙层/阴影；岛专属令牌）
    /// docked 细条填充：浅色微透白冰底，深色暗黑冷炭毛玻璃底
    static func dockedSliverFill(working: Bool, alert: Bool = false) -> Color {
        if alert {
            return Color(dynamic: NSColor(hex: 0xd32f2f, alpha: 0.85),
                         dark: NSColor(hex: 0x3d0a0a, alpha: 0.85))
        }
        return Color(dynamic: NSColor(hex: 0xffffff, alpha: working ? 0.70 : 0.45),
                     dark: NSColor(hex: 0x08080c, alpha: working ? 0.82 : 0.70))
    }
    /// docked 细条描边（浅色黑微透/深色白微透）
    static func dockedSliverStroke(working: Bool, alert: Bool = false) -> Color {
        if alert {
            return dangerRed.opacity(0.85)
        } else if working {
            return sydedockEmerald.opacity(0.85)
        }
        return Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.25)
    }
    /// 玻璃卡蒙层不透明度（GlassCardBackground）
    static let glassOverlayOpacity: Double = 0.35
    /// 玻璃卡 1px 晶莹微反光边缘
    static let glassSpecularBorder = Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.15)
    // 面板 AppKit 阴影已停用（见 IslandPanelController.updateChrome）：窗口与玻璃卡同尺寸，阴影无处落地，
    // 只会向卡内渗入形成暗带。贴边观感由玻璃卡高光边缘与反向倒角承担。

    // Typography
    static func displayFont(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight, design: .default)
    }
    static func bodyFont(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .default)
    }
    static func monoFont(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }
    static func monoDigitFont(_ size: CGFloat, weight: Font.Weight = .medium) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }
    /// 徽标/标签统一字号（R24）：此前同一「Token 徽标」语义存在 8/9/10pt 三种，
    /// 且 <9pt 低于 HIG 最小可读字号。新入口强制 9pt，加权重载给徽标标题
    static func badgeFont(_ weight: Font.Weight = .medium) -> Font {
        .system(size: 9, weight: weight, design: .monospaced)
    }

    // Radii & spacing (8px base)
    static let radiusSm: CGFloat = 8
    static let radiusMd: CGFloat = 11
    static let radiusLg: CGFloat = 18
    /// 页面级水平边距：顶栏/主卡行/详情内容/会话内容/汇总栏统一（防再漂移）
    static let pageMargin: CGFloat = 14
}

import AgentIslandCore

// MARK: - 外观模式 SwiftUI 扩展

extension IslandAppearance {
    /// 对应的 SwiftUI 配色环境（nil 为跟随系统）
    public var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}
