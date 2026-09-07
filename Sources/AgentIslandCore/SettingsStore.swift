import Foundation

// MARK: - 设置持久化（键名与启停集合的唯一 owner）

/// 全部 UserDefaults 键名（UI/Core 共用；键名字面量不得散落在调用方）
public enum SettingKey {
    public static let sampleInterval = "sampleInterval"
    public static let idleSampleInterval = "idleSampleInterval"
    public static let workingWindow = "workingWindow"
    public static let cpuThreshold = "cpuThreshold"
    public static let activeSessionWindow = "activeSessionWindow"
    public static let collapseDelay = "collapseDelay"
    public static let enabledAgents = "enabledAgents"
    public static let islandAppearance = "islandAppearance"
    public static let notificationPolicy = "notificationPolicy"
    public static let customAgents = "customAgents"
    public static let launchAtLogin = "launchAtLogin"
    public static let dockEdge = "dockEdge"
    public static let dockAnchorX = "dockAnchorX"
    public static let dockAnchorY = "dockAnchorY"
    public static let playCompletionSound = "playCompletionSound"
    public static let tokenAlertEnabled = "tokenAlertEnabled"
    public static let tokenAlertThreshold = "tokenAlertThreshold"
    public static let runawayCpuAlert = "runawayCpuAlert"
}

/// 启停集合持久化：key/编解码/空数组语义单点持有。
/// 「无记录」与「空数组」是两个状态——空集合是用户主动全关，照常存取；
/// 无记录时的回退策略（defaultEnabled 集 / 引擎当前集）由调用方决定，store 不掺和。
public enum EnabledAgentStore {

    /// nil = 无记录（键不存在或解码失败）
    public static func load(from defaults: UserDefaults = .standard) -> Set<String>? {
        guard let data = defaults.data(forKey: SettingKey.enabledAgents),
              let saved = try? JSONDecoder().decode([String].self, from: data) else {
            return nil
        }
        return Set(saved)
    }

    /// 空集合是有意全关，照常写入（不得当作「清除记录」）
    public static func save(_ ids: Set<String>, to defaults: UserDefaults = .standard) {
        if let data = try? JSONEncoder().encode(Array(ids)) {
            defaults.set(data, forKey: SettingKey.enabledAgents)
        }
    }
}

#if canImport(AppKit)
import AppKit
#endif

// MARK: - 外观主题定义（跟随系统 / 浅色模式 / 深色模式）

public enum IslandAppearance: String, CaseIterable, Identifiable, Sendable {
    case system
    case light
    case dark

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .system: return "跟随系统"
        case .light: return "浅色模式"
        case .dark: return "深色模式"
        }
    }

    public var shortLabel: String {
        switch self {
        case .system: return "跟随系统"
        case .light: return "浅色"
        case .dark: return "深色"
        }
    }

    public var icon: String {
        switch self {
        case .system: return "laptopcomputer"
        case .light: return "sun.max.fill"
        case .dark: return "moon.fill"
        }
    }

    #if canImport(AppKit)
    public var nsAppearance: NSAppearance? {
        switch self {
        case .system: return nil
        case .light: return NSAppearance(named: .aqua)
        case .dark: return NSAppearance(named: .darkAqua)
        }
    }
    #endif

    public func next() -> IslandAppearance {
        switch self {
        case .system: return .light
        case .light: return .dark
        case .dark: return .system
        }
    }
}

// MARK: - 通知策略分级（标准模式 / 专注免打扰 / 完全静默）

public enum NotificationPolicy: String, CaseIterable, Identifiable, Sendable {
    case standard   // 标准模式：全部事件均弹窗 Peek + 提示音
    case focus      // 专注免打扰（推荐）：普通完成静默；仅 costSpike（熔断/死循环告警）弹窗微窥并报警
    case silent     // 完全静默：全部事件绝不弹窗微窥，不播放声音

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .standard: return "标准模式"
        case .focus: return "专注免打扰"
        case .silent: return "完全静默"
        }
    }

    public var shortLabel: String {
        switch self {
        case .standard: return "标准"
        case .focus: return "专注"
        case .silent: return "静默"
        }
    }

    public var icon: String {
        switch self {
        case .standard: return "bell.fill"
        case .focus: return "bell.badge.slash.fill"
        case .silent: return "bell.slash.fill"
        }
    }

    public var detailDescription: String {
        switch self {
        case .standard: return "任务完成与告警均触发弹窗预览与声音提示"
        case .focus: return "普通完成静默更新；仅熔断/死循环告警弹窗并报警"
        case .silent: return "不弹窗微窥、不响提示音，仅静默记录与展示"
        }
    }

    /// 贴边状态下是否应该触发微窥滑出预览卡片
    public func shouldPeek(for eventType: AgentTaskEvent.EventType) -> Bool {
        switch self {
        case .standard:
            return true
        case .focus:
            return eventType == .costSpike
        case .silent:
            return false
        }
    }

    /// 是否应该播放提示音（受全局声音主开关 soundEnabled 与当前通知策略共同裁决）
    public func shouldPlaySound(for eventType: AgentTaskEvent.EventType, soundEnabled: Bool) -> Bool {
        guard soundEnabled else { return false }
        switch self {
        case .standard:
            return true
        case .focus:
            return eventType == .costSpike
        case .silent:
            return false
        }
    }
}

