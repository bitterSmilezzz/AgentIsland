import Foundation

// MARK: - 活动等级

public enum ActivityLevel: String, Codable, Equatable, Comparable {
    case offline
    case idle
    case working

    public static func < (lhs: ActivityLevel, rhs: ActivityLevel) -> Bool {
        order(lhs) < order(rhs)
    }

    private static func order(_ level: ActivityLevel) -> Int {
        switch level {
        case .offline: return 0
        case .idle: return 1
        case .working: return 2
        }
    }
}

// MARK: - Agent 定义（注册表条目）

public struct AgentProfile: Identifiable, Codable, Equatable {
    public let id: String                 // 稳定 ID，如 "dim"
    public let name: String               // 显示名
    public let icon: String               // SF Symbol
    public let bundleIDs: [String]        // GUI App bundle id（NSWorkspace 匹配）
    public let processNames: [String]     // 进程名前缀（GUI/CLI，ps 匹配，大小写不敏感）
    public let pathContains: [String]     // 可执行路径子串（Electron 应用区分用）
    public let sessionDirs: [String]      // 会话目录（后台扫描）
    public let defaultEnabled: Bool
    public let category: AgentCategory
    /// 是否为用户自定义（来自设置界面）
    public let isCustom: Bool

    public enum AgentCategory: String, Codable {
        case assistant   // 桌面/CLI 助手
        case codeEditor  // 代码编辑器
        case other
    }

    public init(id: String, name: String, icon: String,
                bundleIDs: [String], processNames: [String], pathContains: [String] = [],
                sessionDirs: [String],
                defaultEnabled: Bool = true, category: AgentCategory = .assistant,
                isCustom: Bool = false) {
        self.id = id
        self.name = name
        self.icon = icon
        self.bundleIDs = bundleIDs
        self.processNames = processNames
        self.pathContains = pathContains
        self.sessionDirs = sessionDirs
        self.defaultEnabled = defaultEnabled
        self.category = category
        self.isCustom = isCustom
    }
}

// MARK: - 实时快照（引擎输出）

public struct AgentSnapshot: Identifiable, Equatable {
    public let profile: AgentProfile
    public let level: ActivityLevel
    public let processRunning: Bool
    public let cpuPercent: Double
    public let installed: Bool                 // 检测到安装（bundle/CLI 存在）
    public let activeSessions: Int             // 会话目录下的活跃会话数（子目录数）
    public let lastActivityAgo: TimeInterval?  // 距最近一次文件活动的时间（nil=从未）
    public let lastActivityText: String
    public let tokenUsage: TokenUsage?         // token 用量（数据源缺失时为 nil）

    public var id: String { profile.id }

    public init(profile: AgentProfile, level: ActivityLevel, processRunning: Bool,
                cpuPercent: Double, installed: Bool, activeSessions: Int,
                lastActivityAgo: TimeInterval?, lastActivityText: String,
                tokenUsage: TokenUsage? = nil) {
        self.profile = profile
        self.level = level
        self.processRunning = processRunning
        self.cpuPercent = cpuPercent
        self.installed = installed
        self.activeSessions = activeSessions
        self.lastActivityAgo = lastActivityAgo
        self.lastActivityText = lastActivityText
        self.tokenUsage = tokenUsage
    }
}

// MARK: - 引擎参数

public struct EngineConfig: Equatable {
    public var sampleInterval: TimeInterval = 2.0      // 有活动时采样间隔
    public var idleSampleInterval: TimeInterval = 15.0 // 全闲置时降频采样间隔
    public var workingWindow: TimeInterval = 60.0      // 该窗口内有文件写入 → working（双信号之一）
    public var cpuThreshold: Double = 1.0              // 进程 CPU% 超过 → working（双信号之二，ps 平均值偏低故取 1%）
    public var activeSessionWindow: TimeInterval = 600.0 // 活跃会话计数窗口（10 分钟）
    public var collapseDelay: TimeInterval = 0.5      // 鼠标离开后自动收起延迟
    public var minWorkingHold: TimeInterval = 10.0    // 滞回：working 信号消失后保持最短时长（防抖动）

    public init(sampleInterval: TimeInterval = 2.0,
                idleSampleInterval: TimeInterval = 15.0,
                workingWindow: TimeInterval = 60.0,
                cpuThreshold: Double = 1.0,
                activeSessionWindow: TimeInterval = 600.0,
                collapseDelay: TimeInterval = 3.0,
                minWorkingHold: TimeInterval = 10.0) {
        self.sampleInterval = sampleInterval
        self.idleSampleInterval = idleSampleInterval
        self.workingWindow = workingWindow
        self.cpuThreshold = cpuThreshold
        self.activeSessionWindow = activeSessionWindow
        self.collapseDelay = collapseDelay
        self.minWorkingHold = minWorkingHold
    }
}
