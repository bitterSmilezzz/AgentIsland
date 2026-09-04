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
    static let skyLinkBlue = Color(hex: 0x2997ff)

    // 常规界面色板（设置窗口等，动态：深色系统下自动转暗）
    static let canvas = Color(dynamicLight: 0xffffff, dark: 0x151517)
    static let parchment = Color(dynamicLight: 0xf5f5f7, dark: 0x1e1e20)
    static let pearl = Color(dynamicLight: 0xfafafc, dark: 0x232325)
    static let hairline = Color(dynamicLight: 0xe0e0e0, dark: 0x3a3a3c)
    static let dividerSoft = Color(dynamicLight: 0xf0f0f0, dark: 0x2c2c2e)

    // 灵动岛主体（动态：深色=黑玻璃，浅色=白玻璃）
    static let islandBody = Color(dynamicLight: 0xfafafa, dark: 0x1c1c1e)      // 岛身
    static let islandBodyDeep = Color(dynamicLight: 0xf0f0f2, dark: 0x121214)  // 岛底
    static let tile1 = Color(dynamicLight: 0xe9e9ec, dark: 0x272729)
    static let tile2 = Color(dynamicLight: 0xe2e2e6, dark: 0x2a2a2c)
    static let tile3 = Color(dynamicLight: 0xe5e5e8, dark: 0x252527)

    // Risk accents (system red/orange for danger semantics)
    static let dangerRed = Color(hex: 0xff3b30)
    static let warningOrange = Color(hex: 0xff9500)

    // 文本（动态：岛面上深色模式白字、浅色模式深字）
    static let ink = Color(dynamicLight: 0x1d1d1f, dark: 0xf5f5f7)
    static let inkMuted80 = Color(dynamicLight: 0x333333, dark: 0xbbbbbf)
    static let inkMuted48 = Color(dynamicLight: 0x7a7a7a, dark: 0x9a9aa0)
    static let onDark = Color(dynamicLight: 0x1d1d1f, dark: 0xffffff)          // 主文字
    static let onDarkMuted = Color(dynamicLight: 0x3f3f45, dark: 0xcccccc)     // 次要（浅色加深保对比）
    static let onDarkFaint = Color(dynamicLight: 0x6e6e73, dark: 0x8e8e93)     // 弱化（浅色≥4.5:1）

    // 悬停/按压蒙层（浅色下用黑低透明，深色下用白低透明）
    // hoverFill 需要明显强于 chipFill（≥2x），否则 hover 反馈肉眼不可辨
    static let hoverFill = Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.16)
    static let chipFill = Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.07)
    static let cardFill = Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.05)

    // Status（Apple system colors；浅色下加深保证对比 ≥4.5:1）
    static let statusWorking = Color(dynamicLight: 0x157f3c, dark: 0x30d158)
    static let statusIdle = Color(dynamicLight: 0x8f6a00, dark: 0xffd60a)
    static let statusOffline = Color(dynamicLight: 0x5c5c61, dark: 0x8e8e93)
    static let statusPending = Color(dynamicLight: 0x0a68c4, dark: 0x2997ff)

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

    // Radii & spacing (8px base)
    static let radiusSm: CGFloat = 8
    static let radiusMd: CGFloat = 11
    static let radiusLg: CGFloat = 18
    static let radiusPill: CGFloat = 9999
    static let spaceXs: CGFloat = 8
    static let spaceSm: CGFloat = 12
    static let spaceMd: CGFloat = 17
    static let spaceLg: CGFloat = 24
}

// MARK: - 外观模式管理（浅色/深色/跟随系统）

enum IslandAppearance: String, CaseIterable, Identifiable {
    case system
    case light
    case dark

    var id: String { rawValue }
    var label: String {
        switch self {
        case .system: return "跟随系统"
        case .light: return "浅色"
        case .dark: return "深色"
        }
    }
    /// 应用到悬浮面板的 NSAppearance（system = nil 跟随系统）
    var nsAppearance: NSAppearance? {
        switch self {
        case .system: return nil
        case .light: return NSAppearance(named: .aqua)
        case .dark: return NSAppearance(named: .darkAqua)
        }
    }
}
