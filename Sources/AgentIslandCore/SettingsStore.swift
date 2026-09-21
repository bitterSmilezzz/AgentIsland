import Foundation

// MARK: - 设置持久化（键名与启停集合的唯一 owner）

/// 全部 UserDefaults 键名（UI/Core 共用；键名字面量不得散落在调用方）
// MARK: - 布尔设置的唯一读法

public enum SettingBool {
    /// `UserDefaults.bool(forKey:)` 在「键不存在」时给 **false**，而多个开关的 UI 默认是
    /// **true**——两边不一致时，功能对「从未进过设置页拨过这个开关」的用户永久失效。
    /// 实测：`budgetAlertEnabled` 的 UI 默认 true，引擎用 `bool()` 读，于是预算预警
    /// 与超额告警对新装用户一条都不发；声音开关同形（SoundEffectsManager 用 `?? true`
    /// 而 IslandView 用 `bool()`，同一个键两种真相）。
    public static func read(_ key: String, default fallback: Bool,
                            defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: key) as? Bool ?? fallback
    }
}

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
    /// 顶层损坏时的原件备份键（只由 saveCustomProfiles 写入，不参与注册表解码）
    public static let customAgentsCorruptBackup = "customAgents.corrupt-backup"
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

    /// 每日 token 预算区间。上限 10 亿/天已经远超任何真实用量（设置页最大档是 1000 万），
    /// 存在的理由不是「让人设大预算」而是**让脏值进不来算术**：
    /// 月度预估要算 `dailyBudget * 当月天数`，Int 溢出是 trap——
    /// 实测 `agentisland tokens --budget 9000000000000000000` 以 SIGTRAP(133) 退出且零输出
    public static let dailyTokenBudgetRange: ClosedRange<Int> = 0...1_000_000_000
}

/// 每日 token 预算的唯一读法与唯一解析入口。
///
/// 之前有三处 `UserDefaults.integer(forKey:)` 裸读（引擎、分析页两处、CLI），
/// 与设置页的 Picker 档位没有共同口径：任何一处都能把越界值喂进算术，
/// 而 `dailyBudget * totalDays` 溢出是崩溃而不是显示异常。
/// 收敛到这里之后，「读」与「解析命令行参数」共用同一个区间。
public enum DailyBudget {
    /// 从持久化里读并钳进区间。0 表示未设预算（各消费方都以 `> 0` 判断是否启用）
    public static func read(defaults: UserDefaults = .standard) -> Int {
        clamp(defaults.integer(forKey: SettingKey.dailyTokenBudget))
    }

    public static func clamp(_ value: Int) -> Int {
        min(max(value, SettingLimits.dailyTokenBudgetRange.lowerBound),
            SettingLimits.dailyTokenBudgetRange.upperBound)
    }

    /// 解析 `--budget` 的原始字符串（数字 + 可选 k/m 后缀）。
    /// 返回 nil 表示该拒绝并给用法错误；越界不拒绝而是钳制——
    /// 「1e308 这种值直接崩掉且不留诊断」正是这条入口原来的行为
    public static func parseArgument(_ text: String) -> Int? {
        var digits = text.lowercased()
        var multiplier = 1.0
        if digits.hasSuffix("m") {
            multiplier = 1_000_000
            digits = String(digits.dropLast())
        } else if digits.hasSuffix("k") {
            multiplier = 1_000
            digits = String(digits.dropLast())
        }
        let trimmed = digits.trimmingCharacters(in: .whitespacesAndNewlines)
        // 先按 Int 精确解析（避免 1e9 这类值在 Double 上丢精度），但倍数照样要乘：
        // 这一版最初写成「Int 成功就直接返回」，于是 500k 变成 500——测试抓到才补上
        if let exact = Int(trimmed) {
            guard exact >= 0 else { return nil }
            let (scaledInt, overflow) = exact.multipliedReportingOverflow(
                by: multiplier == 0 ? 1 : Int(multiplier))
            if !overflow { return clamp(scaledInt) }
            return SettingLimits.dailyTokenBudgetRange.upperBound
        }
        guard let asDouble = Double(trimmed), asDouble.isFinite, asDouble >= 0 else { return nil }
        let scaled = asDouble * multiplier
        guard scaled >= 0 else { return nil }
        // 连 Double(Int.max) 都不到说明它远超可用区间，钳到上限即可，不要再乘出 inf
        guard scaled < Double(Int.max) else { return SettingLimits.dailyTokenBudgetRange.upperBound }
        return clamp(Int(scaled))
    }
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
        quarantineCorruptArchive(in: defaults)
        if let data = try? JSONEncoder().encode(Array(ids)) {
            defaults.set(data, forKey: SettingKey.enabledAgents)
        }
    }

    /// 损坏存档的备份键。与自定义档案同一套做法（`customAgentsCorruptBackup`）：
    /// 覆盖前先把原字节抢救出来，用户还有手工恢复的机会。
    public static let corruptBackupKey = "enabledAgents.corruptBackup"

    /// 「解码失败 ⇒ 只读降级 ⇒ 不写回」这条保证此前只在读取侧成立：设置页把降级后的
    /// 默认集装进 @State，用户拨任何一个开关就整体写回，损坏存档被就地覆盖——
    /// 「保留可恢复原状」成了一句空话，而症状是「我的启停选择全没了」。
    /// 只在「键存在且解不开」时备份，且只备份最早那一份（反复保存不会把备份也冲掉）。
    static func quarantineCorruptArchive(in defaults: UserDefaults) {
        guard let original = defaults.object(forKey: SettingKey.enabledAgents) else { return }
        if let data = original as? Data,
           (try? JSONDecoder().decode([String].self, from: data)) != nil { return }
        guard defaults.object(forKey: corruptBackupKey) == nil else { return }
        defaults.set(original, forKey: corruptBackupKey)
        AppLog.error("enabledAgents 存档解码失败，原值已备份到 \(corruptBackupKey) 后再覆盖")
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
    /// - Parameter readOnly: 只求解不写回。CLI 与所有一次性工具都该传 true：
    ///   「读 → 算 → 整体写回」没有版本号，写的是**此刻的注册表快照**。
    ///   应用侧在同一窗口里改了某个 Agent 的开关，会被 CLI 的旧快照覆盖掉（反向同理），
    ///   而用户看到的症状是「我明明关掉了它，跑了一次 status 又回来了」。
    ///   这份配置的归属者是设置页，只有它能写
    public static func resolvedEnabled(
        registry: [AgentProfile],
        defaults: UserDefaults = .standard,
        readOnly: Bool = false
    ) -> Set<String> {
        func commit(_ write: () -> Void) {
            guard !readOnly else { return }
            write()
        }
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
            commit {
                save(defaultsEnabled, to: defaults)
                saveKnownAgents(allRegistryIDs, to: defaults)
            }
            return defaultsEnabled
        }

        // 空集合表示用户主动全关，尊重用户选择。knownAgents 统一 union 口径：
        // 保留 registry 外的历史 known 项（与下方非空分支一致），避免口径漂移
        if currentEnabled.isEmpty {
            let known = loadKnownAgents(from: defaults) ?? legacyKnownAgentIDs
            commit { saveKnownAgents(known.union(allRegistryIDs), to: defaults) }
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
            commit { save(currentEnabled, to: defaults) }
        }

        // 将当前全量 ID 集合更新进 knownAgents
        let updatedKnown = known.union(allRegistryIDs)
        commit { saveKnownAgents(updatedKnown, to: defaults) }

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

public extension SettingKey {
    /// 从一行源码里取出 `SettingKey.xxx` 的键名（结构断言用；不做语法解析，够用即可）
    static func name(in line: String) -> String? {
        guard let range = line.range(of: "SettingKey.") else { return nil }
        let tail = line[range.upperBound...]
        var out = ""
        for ch in tail {
            if ch.isLetter || ch.isNumber || ch == "_" { out.append(ch) } else { break }
        }
        return out.isEmpty ? nil : out
    }
}
