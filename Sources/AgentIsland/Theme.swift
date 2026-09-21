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

// MARK: - 基色阶（Tailwind steps）
//
// 这些整数此前被逐处抄写在 17 个视图文件里（单是 slate-200 一条描边色就 45 处），
// 于是「同一个语义」在岛内有了好几套深浅：改一次色板要改 149 处，漏改的地方只能靠
// 眼睛发现。这里收成唯一来源，`Theme` 的动态令牌浅色半边也引用同一批整数，
// 两轨不可能再各说各话。约定：视图里只准引用 `Ramp.xxx`，不准再出现这些字面量
// （由「语义色单点」与「债务棘轮」两条测试钉住）。
enum Ramp {
    static let slate50Hex: UInt32   = 0xf8fafc
    static let slate100Hex: UInt32  = 0xf1f5f9
    static let slate200Hex: UInt32  = 0xe2e8f0
    static let slate300Hex: UInt32  = 0xcbd5e1
    static let slate400Hex: UInt32  = 0x94a3b8
    static let slate500Hex: UInt32  = 0x64748b
    static let slate600Hex: UInt32  = 0x475569
    static let slate700Hex: UInt32  = 0x334155
    static let emerald50Hex: UInt32 = 0xecfdf5
    static let emerald200Hex: UInt32 = 0xa7f3d0
    static let emerald700Hex: UInt32 = 0x047857
    static let amber50Hex: UInt32   = 0xfffbeb
    static let amber200Hex: UInt32  = 0xfde68a
    static let amber700Hex: UInt32  = 0xb45309
    static let red50Hex: UInt32     = 0xfef2f2
    static let red200Hex: UInt32    = 0xfecaca
    static let red700Hex: UInt32    = 0xb91c1c
    static let green50Hex: UInt32   = 0xf0fdf4
    // 浅色下的强调色加深档：v0.0.96 的可读性收口专用（8–10pt 小字号压在浅底上，
    // 原值只有 3.25–4.38:1，低于 WCAG AA 的 4.5:1；数值由 sRGB 相对亮度实算）
    static let sky800Hex: UInt32    = 0x075985
    static let amber900Hex: UInt32  = 0x78350f

    // 浅色底（白瓷）上的深压强调色：状态色与环形水位色在浅色下必须是同一支，
    // 原先 Theme 与 Palette 各写一次，改一支漏一支。
    static let inkGreenHex: UInt32  = 0x157f3c
    static let inkAmberHex: UInt32  = 0x8f6a00

    // 深色外观的荧光强调：同一支荧光在「状态色」「sydedock 色标」「细条辉光」里必须是同一个值，
    // 原先各写各的（0xffd60a 三处、0xff3b30 两处），改一支就漏两支。
    static let neonAmberHex: UInt32 = 0xffd60a
    static let neonGreenHex: UInt32 = 0x30d158
    static let neonRedHex: UInt32   = 0xff3b30

    static let slate50   = Color(hex: slate50Hex)
    static let slate100  = Color(hex: slate100Hex)
    static let slate200  = Color(hex: slate200Hex)
    static let slate300  = Color(hex: slate300Hex)
    static let slate400  = Color(hex: slate400Hex)
    static let slate500  = Color(hex: slate500Hex)
    static let slate600  = Color(hex: slate600Hex)
    static let slate700  = Color(hex: slate700Hex)
    static let emerald50  = Color(hex: emerald50Hex)
    static let emerald200 = Color(hex: emerald200Hex)
    static let emerald700 = Color(hex: emerald700Hex)
    static let amber50   = Color(hex: amber50Hex)
    static let amber200  = Color(hex: amber200Hex)
    static let amber700  = Color(hex: amber700Hex)
    static let red50     = Color(hex: red50Hex)
    static let red200    = Color(hex: red200Hex)
    static let red700    = Color(hex: red700Hex)
    static let green50   = Color(hex: green50Hex)
    static let sky800    = Color(hex: sky800Hex)
    static let amber900  = Color(hex: amber900Hex)
}

enum Theme {
    // Brand & Accent (Apple Action Blue)
    static let actionBlue = Color(hex: 0x0066cc)
    static let focusBlue = Color(hex: 0x0071e3)

    // 常规界面色板（设置窗口等，动态：深色系统下自动转暗）
    static let canvas = Color(dynamicLight: 0xf5f5f7, dark: 0x151517)
    static let parchment = Color(dynamicLight: 0xffffff, dark: 0x1e1e20)
    static let hairline = Color(dynamicLight: Ramp.slate200Hex, dark: 0x3a3a3c)

    // 灵动岛主体（动态：深色=黑曜石质感，浅色=白瓷工控）
    static let tile1 = Color(dynamicLight: Ramp.slate100Hex, dark: 0x141418)

    // Sydedock 专属工业触觉设计令牌（动态透光毛玻璃与原生 macOS HUD 质感）
    static let obsidianBase = Color(dynamicLight: Ramp.slate50Hex, dark: 0x0c0d14)
    static let obsidianCard = Color(dynamicLight: 0xffffff, dark: 0x161722)
    static let obsidianPill = Color(dynamic: NSColor(hex: Ramp.slate100Hex, alpha: 0.95), dark: NSColor(hex: 0xffffff, alpha: 0.09))
    static let obsidianHairline = Color(dynamic: NSColor(hex: Ramp.slate200Hex, alpha: 0.90), dark: NSColor(hex: 0xffffff, alpha: 0.14))
    static let obsidianCardFill = Color(dynamic: NSColor(hex: 0xffffff, alpha: 0.94), dark: NSColor(hex: 0xffffff, alpha: 0.07))
    static let obsidianCardHoverFill = Color(dynamic: NSColor(hex: 0xffffff, alpha: 1.0), dark: NSColor(hex: 0xffffff, alpha: 0.13))
    static let obsidianCardBorder = Color(dynamic: NSColor(hex: Ramp.slate200Hex, alpha: 0.85), dark: NSColor(hex: 0xffffff, alpha: 0.14))
    static let obsidianCardBorderHover = Color(dynamic: NSColor(hex: Ramp.slate300Hex, alpha: 0.95), dark: NSColor(hex: 0xffffff, alpha: 0.28))

    // Sydedock 荧光强调色（深色模式绚丽高亮，浅色模式加深保障可读对比度）
    static let sydedockCyan = Color(dynamicLight: Ramp.sky800Hex, dark: 0x00e1ff)
    static let sydedockBlue = Color(dynamicLight: 0x1d4ed8, dark: 0x38bdf8)
    static let sydedockAmber = Color(dynamicLight: Ramp.amber900Hex, dark: Ramp.neonAmberHex)
    static let sydedockEmerald = Color(dynamicLight: 0x15803d, dark: Ramp.neonGreenHex)

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
    static let dangerRed = Color(dynamicLight: 0xd32f2f, dark: Ramp.neonRedHex)
    static let warningOrange = Color(dynamicLight: Ramp.amber700Hex, dark: 0xff9500)

    // 文本（动态：深色模式纯净象牙白与清透柔银，浅色模式墨水黑与工控灰，HIG 对比度 ≥4.5:1 / 7:1）
    static let ink = Color(dynamicLight: 0x0f172a, dark: 0xf8fafc)
    static let inkMuted80 = Color(dynamicLight: Ramp.slate700Hex, dark: 0xcfd4dc)
    static let inkMuted48 = Color(dynamicLight: Ramp.slate600Hex, dark: Ramp.slate400Hex)
    static let onDark = Color(dynamicLight: 0x0f172a, dark: 0xffffff)          // 主文字：深色纯白 100%，浅色黑蓝
    static let onDarkMuted = Color(dynamicLight: Ramp.slate700Hex, dark: Ramp.slate200Hex)     // 次要文字：清透软银色
    static let onDarkFaint = Color(dynamicLight: Ramp.slate600Hex, dark: Ramp.slate400Hex)     // 弱化信息：清晰可辨浅石板灰

    // 悬停/按压蒙层（浅色模式纯白磨砂浮层，深色模式纯白透光浮层）
    static let hoverFill = Color(dynamic: NSColor(hex: 0xffffff, alpha: 0.96), dark: NSColor(hex: 0xffffff, alpha: 0.12))
    static let chipFill = Color(dynamic: NSColor(hex: Ramp.slate100Hex, alpha: 0.90), dark: NSColor(hex: 0xffffff, alpha: 0.08))
    static let cardFill = Color(dynamic: NSColor(hex: 0xffffff, alpha: 0.94), dark: NSColor(hex: 0xffffff, alpha: 0.07))

    // Status（浅色模式加深保证对比度）
    static let statusWorking = Color(dynamicLight: Ramp.inkGreenHex, dark: Ramp.neonGreenHex)
    static let statusIdle = Color(dynamicLight: Ramp.inkAmberHex, dark: Ramp.neonAmberHex)
    static let statusOffline = Color(dynamicLight: 0x5c5c61, dark: 0x8e8e93)

    /// docked 细条辉光：CALayer 要 NSColor，且两套外观共用同一支荧光，故不走动态令牌
    static let glowAlert = NSColor(hex: Ramp.neonRedHex)
    static let glowWorking = NSColor(hex: 0x10b981)
    static let glowIdle = NSColor(hex: Ramp.neonAmberHex)

    // 面板视觉（细条/蒙层/阴影；岛专属令牌）
    /// docked 细条填充：浅色微透白冰底，深色暗黑冷炭毛玻璃底
    static func dockedSliverFill(working: Bool, alert: Bool = false) -> Color {
        if alert {
            return Color(dynamic: NSColor(hex: 0xd32f2f, alpha: 0.85),
                         dark: NSColor(hex: 0x3d0a0a, alpha: 0.85))
        }
        return Color(dynamic: NSColor(hex: 0xffffff, alpha: working ? 0.95 : 0.90),
                     dark: NSColor(hex: 0x08080c, alpha: working ? 0.82 : 0.70))
    }
    /// docked 细条描边（浅色精致石板微透/深色白微透）
    static func dockedSliverStroke(working: Bool, alert: Bool = false) -> Color {
        if alert {
            return dangerRed.opacity(0.85)
        } else if working {
            return sydedockEmerald.opacity(0.85)
        }
        return Color(dynamic: NSColor(hex: Ramp.slate300Hex, alpha: 0.80), dark: NSColor(hex: 0xffffff, alpha: 0.25))
    }
    /// 玻璃卡蒙层不透明度（GlassCardBackground）
    static let glassOverlayOpacity: Double = 0.35
    /// 玻璃卡 1px 晶莹微反光边缘
    static let glassSpecularBorder = Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.15)
    // 面板 AppKit 阴影已停用（`IslandPanel.setupPanel` 里 `panel.hasShadow = false`；
    // updateChrome 只裁圆角，别去那儿找）：窗口与玻璃卡同尺寸，阴影无处落地，
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

// MARK: - 活动等级色阶（岛内唯一的「状态 → 颜色」来源）

extension ActivityLevel {
    /// 不区分外观的基础色（深色模式与所有需要单值的场合）
    var color: Color {
        switch self {
        case .working, .completed: return Theme.statusWorking
        case .attention: return Theme.warningOrange
        case .idle: return Theme.statusIdle
        case .offline: return Theme.statusOffline
        }
    }

    // 岛内药丸/徽标在浅色（白瓷底）下必须整组压深才够 HIG 对比度，深色则直接用 `color`
    // 的半透染色。这三段梯子原先在 AgentIslandApp / AgentRowView / AgentHoverTooltip 里
    // 各抄了一份完全相同的副本（45 处字面量），改色板只能三处对齐着改。
    // 透明度与各卡暗色染色系数（0.12 / 0.14 / 0.18）留给调用点——那是组件自身的决定。

    /// 浅色下的文字基色（对比度按 sRGB 实算，压在各自 lightFill 上：
    /// working/attention 5.21 与 4.84；idle 6.92；offline 4.55。全部 ≥ AA 正文 4.5）
    var lightText: Color {
        switch self {
        case .working, .completed: return Ramp.emerald700
        case .attention: return Ramp.amber700
        case .idle: return Ramp.slate600
        case .offline: return Ramp.slate500
        }
    }

    /// 浅色下的底色基色
    var lightFill: Color {
        switch self {
        case .working, .completed: return Ramp.emerald50
        case .attention: return Ramp.amber50
        case .idle: return Ramp.slate100
        // 离线底再退浅一档，配合上面 slate500 的文字：4.55:1（slate100 底只有 4.34）
        case .offline: return Ramp.slate50
        }
    }

    /// 浅色下的描边基色
    var lightBorder: Color {
        switch self {
        case .working, .completed: return Ramp.emerald200
        case .attention: return Ramp.amber200
        case .idle, .offline: return Ramp.slate200
        }
    }
}
