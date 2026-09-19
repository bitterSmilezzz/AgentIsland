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
    public static let hideDockedSliver = "hideDockedSliver"
    public static let dailyTokenBudget = "dailyTokenBudget"
    public static let budgetAlertEnabled = "budgetAlertEnabled"
    public static let compactView = "compactView"
    public static let globalHotKeyEnabled = "globalHotKeyEnabled"
    public static let menuBarBadgeMode = "menuBarBadgeMode"
    public static let screenFollowMode = "screenFollowMode"
    public static let completionSoundOption = "completionSoundOption"
    public static let alertSoundOption = "alertSoundOption"
    public static let hapticFeedbackEnabled = "hapticFeedbackEnabled"
    public static let batterySaverEnabled = "batterySaverEnabled"
    public static let autoAnomaliesAlertEnabled = "autoAnomaliesAlertEnabled"
    public static let predictiveAnalyticsEnabled = "predictiveAnalyticsEnabled"
    public static let localEventServerEnabled = "localEventServerEnabled"
    public static let localEventServerPort = "localEventServerPort"
}

/// 屏幕自适应与跟随模式 (v0.0.74)
public enum ScreenFollowMode: String, CaseIterable, Identifiable {
    case followMouse = "followMouse"
    case mainScreen = "mainScreen"
    case builtInScreen = "builtInScreen"
    case externalScreen = "externalScreen"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .followMouse: return "跟随鼠标所在屏幕"
        case .mainScreen: return "固定主屏幕 (带菜单栏)"
        case .builtInScreen: return "优先内置屏幕 (MacBook)"
        case .externalScreen: return "优先外接显示器"
        }
    }
}

/// 任务完成提示音选项 (v0.0.74)
public enum CompletionSoundOption: String, CaseIterable, Identifiable {
    case glass = "Glass"
    case pop = "Pop"
    case ping = "Ping"
    case blow = "Blow"
    case mute = "mute"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .glass: return "Glass (清脆水滴)"
        case .pop: return "Pop (轻快微泡)"
        case .ping: return "Ping (清脆叮咚)"
        case .blow: return "Blow (低调柔和)"
        case .mute: return "静音 (无声音)"
        }
    }

    public var systemSoundName: String? {
        self == .mute ? nil : rawValue
    }
}

/// 熔断与严重异常告警音效选项 (v0.0.74)
public enum AlertSoundOption: String, CaseIterable, Identifiable {
    case sosumi = "Sosumi"
    case basso = "Basso"
    case funk = "Funk"
    case mute = "mute"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .sosumi: return "Sosumi (经典敲击)"
        case .basso: return "Basso (低沉警示)"
        case .funk: return "Funk (强烈警报)"
        case .mute: return "静音 (无声音)"
        }
    }

    public var systemSoundName: String? {
        self == .mute ? nil : rawValue
    }
}

/// 菜单栏图标附加徽标模式 (v0.0.73)
public enum MenuBarBadgeMode: String, CaseIterable, Identifiable {
    case iconOnly = "iconOnly"
    case activeCount = "activeCount"
    case tokenUsage = "tokenUsage"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .iconOnly: return "仅图标（极简）"
        case .activeCount: return "活跃任务数 (例: ⚡️ 2)"
        case .tokenUsage: return "今日 Token (例: 120k)"
        }
    }
}

/// UserDefaults 读取的钳制区间（脏持久化值自愈；与设置页滑杆 range 对齐）。
/// 引擎配置字段的区间在 EngineConfig（normalized 唯一入口），这里只收 UI 侧散点。
public enum SettingLimits {
    /// 自动收起延迟：下限 0.2s 防鼠标掠过即收、上限 5s 防脏值让面板久驻
    public static let collapseDelayRange: ClosedRange<Double> = 0.2...5.0
}

/// 启停集合持久化：key/编解码/空数组语义单点持有。
/// 「无记录」与「空数组」是两个状态——空集合是用户主动全关，照常存取；
/// 提供 resolvedEnabled(registry:defaults:) 实现版本升级与历史配置向前兼容自愈。
public enum EnabledAgentStore {

    /// 引入版本前已知的所有内置 Agent ID（用于旧版本无 knownAgents 记录时的平滑迁移基线）
    public static let legacyKnownAgentIDs: Set<String> = [
        "dim", "claude", "codex", "cursor", "trae", "copilot", "workbuddy", "opencode", "hermes", "continue"
    ]

    /// 存档读取三态（R21）：键不存在 ≠ 解码失败——损坏存档若当「无记录」处理，
    /// 会被首次安装分支覆写，用户全部启停选择静默丢失
    public enum LoadState {
        case none      // 键不存在（全新安装/首次运行）
        case ok        // 解码成功
        case corrupt   // 键存在但解码失败（损坏，不得覆写）
    }

    public static func loadDetailed(from defaults: UserDefaults = .standard) -> (state: LoadState, ids: Set<String>?) {
        guard defaults.object(forKey: SettingKey.enabledAgents) != nil else {
            return (.none, nil)
        }
        guard let data = defaults.data(forKey: SettingKey.enabledAgents),
              let saved = try? JSONDecoder().decode([String].self, from: data) else {
            AppLog.error("enabledAgents 存档解码失败（只读降级，不覆写原数据）")
            return (.corrupt, nil)
        }
        return (.ok, Set(saved))
    }

    /// 兼容旧签名：无记录/损坏均返回 nil
    public static func load(from defaults: UserDefaults = .standard) -> Set<String>? {
        loadDetailed(from: defaults).ids
    }

    /// 空集合是有意全关，照常写入（不得当作「清除记录」）
    public static func save(_ ids: Set<String>, to defaults: UserDefaults = .standard) {
        if let data = try? JSONEncoder().encode(Array(ids)) {
            defaults.set(data, forKey: SettingKey.enabledAgents)
        }
    }

    /// 读取已记录的已知 Agent ID 集合（nil = 无记录，表示需要从 legacy 迁移或全新安装）
    public static func loadKnownAgents(from defaults: UserDefaults = .standard) -> Set<String>? {
        guard defaults.object(forKey: SettingKey.knownAgents) != nil else { return nil }
        guard let data = defaults.data(forKey: SettingKey.knownAgents),
              let saved = try? JSONDecoder().decode([String].self, from: data) else {
            AppLog.error("knownAgents 存档解码失败（回退 legacy 基线）")
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
        let loaded = loadDetailed(from: defaults)
        guard var currentEnabled = loaded.ids else {
            if loaded.state == .corrupt {
                // 只读降级：按默认启用集运行但绝不写回——损坏存档保留可恢复原状
                //（写回 = 用户启停选择永久丢失）。恢复数据后重启即自愈
                AppLog.error("启停集存档损坏，本次按默认启用集只读运行（未写回）")
                return Set(registry.filter(\.defaultEnabled).map(\.id))
            }
            // 首次安装：初始化为默认开启集
            let defaultsEnabled = Set(registry.filter(\.defaultEnabled).map(\.id))
            save(defaultsEnabled, to: defaults)
            saveKnownAgents(allRegistryIDs, to: defaults)
            return defaultsEnabled
        }

        // 空集合表示用户主动全关，尊重用户选择。knownAgents 统一 union 口径：
        // 保留 registry 外的历史 known 项（与下方非空分支一致），避免口径漂移
        if currentEnabled.isEmpty {
            let known = loadKnownAgents(from: defaults) ?? legacyKnownAgentIDs
            saveKnownAgents(known.union(allRegistryIDs), to: defaults)
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
    case focus      // 专注免打扰（推荐）：普通完成静默；确认请求与 costSpike 告警仍提醒
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
        case .focus: return "普通完成静默更新；确认请求与熔断/死循环告警仍提醒"
        case .silent: return "不弹窗微窥、不响提示音，仅静默记录与展示"
        }
    }

    /// 贴边状态下是否应该触发微窥滑出预览卡片
    public func shouldPeek(for eventType: AgentTaskEvent.EventType) -> Bool {
        switch self {
        case .standard:
            return true
        case .focus:
            return eventType != .completed
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
            return eventType != .completed
        case .silent:
            return false
        }
    }
}
