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
    public static let knownAgents = "knownAgents"
}

/// 启停集合持久化：key/编解码/空数组语义单点持有。
/// 「无记录」与「空数组」是两个状态——空集合是用户主动全关，照常存取；
/// 提供 resolvedEnabled(registry:defaults:) 实现版本升级与历史配置向前兼容自愈。
public enum EnabledAgentStore {

    /// 引入版本前已知的所有内置 Agent ID（用于旧版本无 knownAgents 记录时的平滑迁移基线）
    public static let legacyKnownAgentIDs: Set<String> = [
        "dim", "claude", "codex", "cursor", "trae", "copilot", "workbuddy", "opencode", "hermes", "continue"
    ]

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

    /// 读取已记录的已知 Agent ID 集合（nil = 无记录，表示需要从 legacy 迁移或全新安装）
    public static func loadKnownAgents(from defaults: UserDefaults = .standard) -> Set<String>? {
        guard let data = defaults.data(forKey: SettingKey.knownAgents),
              let saved = try? JSONDecoder().decode([String].self, from: data) else {
            return nil
        }
        return Set(saved)
    }

    /// 保存已知 Agent ID 集合
    public static func saveKnownAgents(_ ids: Set<String>, to defaults: UserDefaults = .standard) {
        if let data = try? JSONEncoder().encode(Array(ids)) {
            defaults.set(data, forKey: SettingKey.knownAgents)
        }
    }

    /// 把单个 ID 并入已知集合。
    /// 用于「用户刚创建的自定义 Agent」：登记后它不再被 resolvedEnabled 当作
    /// 「版本升级新增的默认启用项」而自动补回启用集（否则用户关掉它会被静默覆盖）。
    public static func markKnown(_ id: String, to defaults: UserDefaults = .standard) {
        var known = loadKnownAgents(from: defaults) ?? legacyKnownAgentIDs
        known.insert(id)
        saveKnownAgents(known, to: defaults)
    }

    /// 求解并自愈启停集合（解决版本升级新加内置/自动发现 Agent 被历史持久化集静默锁死的问题）
    /// - 首次运行（load 为 nil）：按 registry.filter(\.defaultEnabled) 全量初始化并固化
    /// - 用户全关（load 为 []）：严格尊重用户全关意图，保持空集不强行覆写
    /// - 存量迁移/版本升级：找出所有在已知集之外且 defaultEnabled 的新增 Agent，自动合并补齐并写回
    public static func resolvedEnabled(
        registry: [AgentProfile],
        defaults: UserDefaults = .standard
    ) -> Set<String> {
        let allRegistryIDs = Set(registry.map(\.id))
        guard var currentEnabled = load(from: defaults) else {
            // 首次安装：初始化为默认开启集
            let defaultsEnabled = Set(registry.filter(\.defaultEnabled).map(\.id))
            save(defaultsEnabled, to: defaults)
            saveKnownAgents(allRegistryIDs, to: defaults)
            return defaultsEnabled
        }

        // 空集合表示用户主动全关，尊重用户选择，只更新 knownAgents 避免后续重入触发
        if currentEnabled.isEmpty {
            saveKnownAgents(allRegistryIDs, to: defaults)
            return []
        }

        // 读取已知 ID 集；若为 nil 则使用旧版基线迁移
        let known = loadKnownAgents(from: defaults) ?? legacyKnownAgentIDs
        // 只把「内置/自动发现」的新条目自动补入启用集：用户自定义条目（isCustom）
        // 可能刚被用户主动关闭，若因不在 knownAgents 而重入，会静默覆盖用户意图。
        let newlyAddedProfiles = registry.filter {
            !known.contains($0.id) && $0.defaultEnabled && !$0.isCustom
        }

        if !newlyAddedProfiles.isEmpty {
            for profile in newlyAddedProfiles {
                currentEnabled.insert(profile.id)
            }
            save(currentEnabled, to: defaults)
        }

        // 将当前全量 ID 集合更新进 knownAgents
        let updatedKnown = known.union(allRegistryIDs)
        saveKnownAgents(updatedKnown, to: defaults)

        return currentEnabled
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

