import Foundation

// MARK: - 活动等级

public enum ActivityLevel: String, Codable, Equatable, Comparable {
    case offline
    case idle
    case completed
    case working
    case attention

    public var label: String {
        switch self {
        case .offline: return "离线"
        case .idle: return "待机"
        case .completed: return "已完成"
        case .working: return "工作中"
        case .attention: return "待确认"
        }
    }

    public static func < (lhs: ActivityLevel, rhs: ActivityLevel) -> Bool {
        order(lhs) < order(rhs)
    }

    private static func order(_ level: ActivityLevel) -> Int {
        switch level {
        case .offline: return 0
        case .idle: return 1
        case .completed: return 2
        case .working: return 3
        case .attention: return 4
        }
    }
}

// MARK: - 贴边停靠模式

public enum DockEdge: String, Codable, CaseIterable, Identifiable, Sendable {
    case right
    case top
    case bottom
    case left

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .right: return "右侧边栏"
        case .top: return "顶部灵动岛"
        case .bottom: return "底部停靠条"
        case .left: return "左侧边栏"
        }
    }

    /// 水平边缘（顶部/底部）沿 X 轴保存位置；垂直边缘沿 Y 轴保存位置。
    public var isHorizontal: Bool {
        self == .top || self == .bottom
    }
}

// MARK: - Agent 会话解析方言

/// 会话记录的存放格式。解析器按**格式**分派，而不是按 agent id 分派：
/// 格式数量远少于 Agent 数量（Cline 与 Roo Code 同源、WorkBuddy 两版同 schema），
/// 新增一个复用既有格式的 Agent 时只需登记档案，不必再改解析器里的 id 梯子。
public enum AgentSessionDialect: String, Codable {
    /// 通用 JSONL/文本尾部（FileMonitor 定位的活动文件 + 有界尾读）
    case genericTail
    /// Antigravity：`brain/<session>/.system_generated/logs/transcript.jsonl` + 同名 tasks 目录
    case antigravityBrain
    /// DSH：`session_projcache/sessions/<id>.json` 投影缓存
    case dshProjection
    /// Cline / Roo Code：`tasks/<taskId>/ui_messages.json`
    case clineTasks
    /// Qoder：`~/.qoder/projects/<项目 slug>/<会话 uuid>.jsonl`（Anthropic 兼容逐行）
    case qoderTranscript
}

/// 只读会话库：部分桌面 Agent 把会话只写进 SQLite，FileMonitor 的「最新文件」只能定位到
/// 二进制库本身，需要按已知 schema 做索引命中的末条查询。
///
/// 位置随档案声明：此前同一批 `.sqlite` 路径在解析器、动作探测、日志流、Token 统计里各
/// 硬编码一份，注册表只记目录不记库文件，已经漂移（`dimcode.sqlite` 不在注册表里）。
public struct AgentSessionDatabase: Codable, Equatable {
    public enum Schema: String, Codable {
        /// dimcode：末 32 条会话按 tool call id 配对确认请求与结果
        case dimTasks
        /// 通用状态索引：`SELECT id, <status>, updated_at … ORDER BY updated_at DESC LIMIT 1`
        case statusIndex
        /// OpenCode：message/part 双表
        case openCode
    }

    public let path: String
    public let schema: Schema
    /// 仅 `statusIndex` 需要：完整查询语句（各产品列名与软删条件不同）
    public let statusSQL: String?

    public init(path: String, schema: Schema, statusSQL: String? = nil) {
        self.path = path
        self.schema = schema
        self.statusSQL = statusSQL
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
    /// 可执行路径排除子串：命中即不算本 Agent。
    /// 用于宿主应用内嵌同名二进制的场景——ChatGPT 桌面版把 Codex 打包进
    /// /Applications/ChatGPT.app/Contents/Resources/codex，basename 与独立 codex CLI
    /// 相同，仅靠 pathContains 白名单无法区分，会把同一份 ChatGPT 数成两个 Agent。
    public let pathExcludes: [String]
    /// 宿主应用 bundle id：这些 App 已安装且本 profile 自身未独立安装时，
    /// 说明该 Agent 是宿主的内嵌组件而非独立产品（如 ChatGPT 桌面版内嵌 Codex，
    /// 二者共用 ~/.codex 会话目录），不再单独成行，避免同一份程序数成两个 Agent。
    public let hostBundleIDs: [String]
    /// 本 Agent 的 CPU 工作判定**下限**（nil 表示无下限）。
    ///
    /// 实际阈值 = `max(cpuWorkingThreshold ?? 0, EngineConfig.cpuThreshold)`
    /// 语义说明：
    /// - 桌面类（Electron/多进程）Agent 即使空闲，渲染与 IPC 也有 4%~15% 抖动，
    ///   必须有一个下限把它们和「真在工作」区分开，否则空闲会被误判成 working
    ///   （此为已修复的回归，见 CHANGELOG「ChatGPT 桌面版空闲误判工作态」）
    /// - 使用「下限」而非「覆盖」：用户把设置页阈值调**高**时对所有 Agent 生效；
    ///   调**低**时不会突破该 Agent 的保护下限（避免重新引入上面的误报）
    /// - 此前这段判断按 id 硬编码在引擎里，导致设置项对 13/17 个内置 Agent 完全无效
    public let cpuWorkingThreshold: Double?
    /// 本 Agent 的 Token 激增告警下限（nil 表示无下限，完全遵循全局设置）。
    ///
    /// 实际阈值 = `max(tokenAlertFloor ?? 0, EngineConfig.tokenAlertThreshold)`
    /// 语义说明：
    /// - 针对 WorkBuddy 等多专家团/多 Agent 协同体系，单轮任务并发消耗较大，
    ///   专属下限可避免日常多专家协同被误判为 Token 异常激增或死循环。
    /// - 使用「下限」而非「覆盖」：用户把设置页阈值调高时对所有 Agent 生效；
    ///   调低时不会突破该 Agent 的保护下限。
    public let tokenAlertFloor: Int?
    public let sessionDirs: [String]      // 会话目录（后台扫描）
    /// Token 明细根目录：这些目录树下的 JSONL 记录承载本 Agent 的 token 用量（空表示无）。
    ///
    /// 与 `sessionDirs` 分开声明是有原因的：两者常在不同子树里——WorkBuddy 的工作信号
    /// 在 `tasks/`，token 明细却在 `projects/`。此前用量侧把这些路径又抄了一份字面量，
    /// 档案换目录或改名后只有采集的一半生效（见 CHANGELOG「会话数据库位置与查询改为档案数据」一条），故一律收进档案。
    public let tokenRoots: [String]

    /// 终端看板（`agentisland status` / `top`）与摘要里用的展示符号。
    /// 曾经是 CLI 里一张 19 分支的 id→emoji 表，其中 4 个 id（dimagent / vibe /
    /// ima.copilot / egobrowser）在注册表里并不存在——档案改名后梯子不会报错，只会静默
    /// 退化成默认符号，所以展示数据随档案声明。
    public let emoji: String
    /// 会话记录的解析方言（见 `AgentSessionDialect`）
    public let sessionDialect: AgentSessionDialect
    /// 只读会话库（nil 表示该 Agent 的会话不落 SQLite）
    public let sessionDatabase: AgentSessionDatabase?
    public let defaultEnabled: Bool
    public let category: AgentCategory
    /// 是否为用户自定义（来自设置界面）
    public let isCustom: Bool

    public enum AgentCategory: String, Codable {
        case assistant   // 桌面/CLI 助手
        case codeEditor  // 代码编辑器
        case other
    }

    /// 该档案是否登记了任何**本地明细源**（只读会话库或 token 目录）。
    /// 「没登记」与「登记了但本轮读不到」是两种结论：前者是设计如此，后者才可能是故障。
    public var hasLocalDetailSource: Bool {
        sessionDatabase != nil || !tokenRoots.isEmpty
    }

    /// 自定义条目 id 推导（设置表单校验与落库共用，规则只此一处）
    public static func makeCustomID(_ processName: String) -> String {
        "custom-\(processName.lowercased().replacingOccurrences(of: " ", with: "-"))"
    }

    public init(id: String, name: String, icon: String,
                bundleIDs: [String], processNames: [String], pathContains: [String] = [],
                pathExcludes: [String] = [],
                hostBundleIDs: [String] = [],
                cpuWorkingThreshold: Double? = nil,
                tokenAlertFloor: Int? = nil,
                sessionDirs: [String],
                tokenRoots: [String] = [],
                defaultEnabled: Bool = true, category: AgentCategory = .assistant,
                isCustom: Bool = false,
                emoji: String = "🤖",
                sessionDialect: AgentSessionDialect = .genericTail,
                sessionDatabase: AgentSessionDatabase? = nil) {
        self.id = id
        self.name = name
        self.icon = icon
        self.bundleIDs = bundleIDs
        self.processNames = processNames
        self.pathContains = pathContains
        self.pathExcludes = pathExcludes
        self.hostBundleIDs = hostBundleIDs
        self.cpuWorkingThreshold = cpuWorkingThreshold
        self.tokenAlertFloor = tokenAlertFloor
        self.sessionDirs = sessionDirs
        self.tokenRoots = tokenRoots
        self.emoji = emoji
        self.sessionDialect = sessionDialect
        self.sessionDatabase = sessionDatabase
        self.defaultEnabled = defaultEnabled
        self.category = category
        self.isCustom = isCustom
    }

    // MARK: Codable（手写以兼容历史存档）
    // 合成的解码器要求新增的非可选字段在 JSON 中必须存在，会让升级前保存的
    // 自定义 Agent 整条解码失败（表现为用户配置凭空消失）。这里逐字段
    // decodeIfPresent + 默认值，新旧存档都能读。

    private enum CodingKeys: String, CodingKey {
        case id, name, icon, bundleIDs, processNames, pathContains, pathExcludes
        case hostBundleIDs, cpuWorkingThreshold, tokenAlertFloor, sessionDirs, tokenRoots
        case defaultEnabled, category, isCustom
        case emoji, sessionDialect, sessionDatabase
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        icon = try c.decode(String.self, forKey: .icon)
        bundleIDs = try c.decodeIfPresent([String].self, forKey: .bundleIDs) ?? []
        processNames = try c.decodeIfPresent([String].self, forKey: .processNames) ?? []
        pathContains = try c.decodeIfPresent([String].self, forKey: .pathContains) ?? []
        pathExcludes = try c.decodeIfPresent([String].self, forKey: .pathExcludes) ?? []
        hostBundleIDs = try c.decodeIfPresent([String].self, forKey: .hostBundleIDs) ?? []
        // 阈值字段来自可被手改的存档：`1e999` 会被解成 Infinity，
        // 而 `max(cpu, inf)` 恒为 inf——症状是「阈值设了却永远不触发」，
        // 看起来像没生效而不是像坏了。这里统一钳回可用区间（NaN 也一并落到 nil）
        // 每个字段各自 `try?`：`1e999` 这种数在 JSONDecoder 眼里是「解不出来」而不是
        // 「解成 Infinity」，用 `try` 会让整条档案解码失败并被上层当损坏元素丢弃
        var cpuThreshold: Double?
        if let raw = (try? c.decodeIfPresent(Double.self, forKey: .cpuWorkingThreshold)) ?? nil,
           raw.isFinite {
            cpuThreshold = min(max(raw, 0), 1_000)
        }
        cpuWorkingThreshold = cpuThreshold
        var tokenFloor: Int?
        if let raw = (try? c.decodeIfPresent(Int.self, forKey: .tokenAlertFloor)) ?? nil, raw >= 0 {
            tokenFloor = min(raw, 100_000_000)
        }
        tokenAlertFloor = tokenFloor
        sessionDirs = try c.decodeIfPresent([String].self, forKey: .sessionDirs) ?? []
        tokenRoots = try c.decodeIfPresent([String].self, forKey: .tokenRoots) ?? []
        emoji = try c.decodeIfPresent(String.self, forKey: .emoji) ?? "🤖"
        sessionDialect = try c.decodeIfPresent(AgentSessionDialect.self, forKey: .sessionDialect) ?? .genericTail
        sessionDatabase = try c.decodeIfPresent(AgentSessionDatabase.self, forKey: .sessionDatabase)
        defaultEnabled = try c.decodeIfPresent(Bool.self, forKey: .defaultEnabled) ?? true
        category = try c.decodeIfPresent(AgentCategory.self, forKey: .category) ?? .assistant
        isCustom = try c.decodeIfPresent(Bool.self, forKey: .isCustom) ?? false
    }
}

// MARK: - 内存显示文案（唯一实现）

/// 字节数 → 紧凑易读文本（如 "128M"、"1.4G"、"<1M"）。
///
/// 此前这段逻辑散落三处（`AgentSnapshot` / `AgentAnomaly` / `CleanResult`），
/// 其中只有 `AgentSnapshot` 处理了 `<1MB` 的情况，另两处 `Int(mb)` 截断会把
/// 小额内存显示成「0M」（易被误读为无占用）。收敛到此处后三处口径一致。
public enum MemoryFormat {
    public static func text(_ bytes: UInt64) -> String {
        guard bytes > 0 else { return "—" }
        let mb = Double(bytes) / (1024 * 1024)
        if mb >= 1024 {
            return String(format: "%.1fG", mb / 1024.0)
        } else if mb < 1 {
            return "<1M"
        } else {
            return "\(Int(mb))M"
        }
    }
}

// MARK: - 时钟文案格式化（唯一实现）

/// 定宽 UI 里的时钟列/轴标签（`HH:mm:ss`、`HH:mm`）必须锁死数字口径。
///
/// `DateFormatter` 默认跟随系统区域设置：换成 12 小时制 locale 会多出「上午/下午」，
/// 部分 locale 还会改写数字字形与分隔符——同一份数据在用户机器上就会撑破固定宽度的
/// 列（`DetailViews` / `LiveLogStreamView` 的原注释记录过这个坑）。口径只在这里声明一次。
public enum TimeFormat {
    /// 固定数字口径的 locale：不跟随系统区域设置
    public static let posixLocale = Locale(identifier: "en_US_POSIX")

    /// 构造一个 POSIX 口径的 DateFormatter。
    /// 调用方务必 `static` 缓存（DateFormatter 创建昂贵，行 body 每次重算都 new 一个是回归点）
    public static func formatter(_ dateFormat: String) -> DateFormatter {
        let f = DateFormatter()
        f.dateFormat = dateFormat
        f.locale = posixLocale
        return f
    }

    /// 共享实例：`HH:mm:ss`（事件流 / 日志流的时钟列）
    public static let clock: DateFormatter = formatter("HH:mm:ss")

    /// 共享实例：`HH:mm`（时间轴刻度与趋势图悬浮提示）
    public static let hourAndMinute: DateFormatter = formatter("HH:mm")
}

// MARK: - 后台在途任务与子智能体模型

public struct AgentBackgroundTask: Equatable, Codable, Sendable {
    public let id: String
    public let action: String
    public let startTime: Date?

    public init(id: String, action: String, startTime: Date? = nil) {
        self.id = id
        self.action = action
        self.startTime = startTime
    }
}

public struct AgentSubagentInfo: Equatable, Codable, Sendable {
    public let conversationId: String
    public let role: String
    public let model: String?
    public let state: String?

    public init(conversationId: String, role: String, model: String? = nil, state: String? = nil) {
        self.conversationId = conversationId
        self.role = role
        self.model = model
        self.state = state
    }
}

public struct AgentTokenBreakdown: Equatable, Codable, Sendable {
    public let promptTokens: Int
    public let completionTokens: Int
    public let cacheReadTokens: Int
    public let cacheWriteTokens: Int
    public let reasoningTokens: Int
    public let totalTokens: Int

    public init(promptTokens: Int = 0, completionTokens: Int = 0, cacheReadTokens: Int = 0, cacheWriteTokens: Int = 0, reasoningTokens: Int = 0, totalTokens: Int = 0) {
        self.promptTokens = promptTokens
        self.completionTokens = completionTokens
        self.cacheReadTokens = cacheReadTokens
        self.cacheWriteTokens = cacheWriteTokens
        self.reasoningTokens = reasoningTokens
        self.totalTokens = totalTokens > 0 ? totalTokens : (promptTokens + completionTokens + cacheReadTokens + cacheWriteTokens + reasoningTokens)
    }
}

public struct SessionActiveContext: Equatable, Sendable {
    public var backgroundTasks: [AgentBackgroundTask]
    public var subagents: [AgentSubagentInfo]
    public var tokenBreakdown: AgentTokenBreakdown?

    public init(backgroundTasks: [AgentBackgroundTask] = [], subagents: [AgentSubagentInfo] = [], tokenBreakdown: AgentTokenBreakdown? = nil) {
        self.backgroundTasks = backgroundTasks
        self.subagents = subagents
        self.tokenBreakdown = tokenBreakdown
    }
}

// MARK: - 实时快照（引擎输出）

public struct AgentSnapshot: Identifiable, Equatable {
    public let profile: AgentProfile
    public let level: ActivityLevel
    public let processRunning: Bool
    /// CPU 利用率的**两态**：有值是「本拍确有差分窗口」测出来的利用率，`nil` 是根本没测。
    /// CPU% 是两次采样之间的进程时间差分量，所以第一拍（没有前一拍）结构性地测不出来，
    /// 只按 bundle 命中的占位条目同样给不出。把它们印成 `0.0%` 等于向用户宣布
    /// 「这个进程没在烧 CPU」，而事实是「这一拍还没来得及知道」——与死锁维度同一类谎报。
    public let cpuPercent: Double?
    public let installed: Bool                 // 检测到安装（bundle/CLI 存在）
    public let activeSessions: Int             // 会话目录下的活跃会话数（子目录数）
    public let lastActivityAgo: TimeInterval?  // 距最近一次文件活动的时间（nil=从未）
    public let lastActivityText: String
    public let tokenUsage: TokenUsage?         // token 用量（数据源缺失时为 nil）
    public let pid: Int32?                     // 匹配到的进程 PID（若运行中）
    public let currentAction: String?          // 实时动作透传（执行的命令/修改的文件/思考等）
    public let memoryBytes: UInt64             // 物理内存占用（RSS 字节数）
    /// 死锁/僵死判定的**三态**结果：`true` 持续高负载已达阈值；`false` 观测窗口够长、
    /// 判过且清白；`nil` 本轮结构上判不出（对该进程的连续观测不足 `runawayDurationThreshold`，
    /// 一次性 CLI 必然如此）。
    /// 默认 nil 是有意的：漏传只能得到「没测」，不能得到一个假的「没有」。
    public let isHung: Bool?
    public let backgroundTasks: [AgentBackgroundTask] // 正在运行的后台命令/任务
    public let subagents: [AgentSubagentInfo]   // 正在运行/关联的子智能体列表
    public let tokenBreakdown: AgentTokenBreakdown?   // Token 细分耗损指标
    /// 会话探测最近一次失败的原因（nil 表示源可读或本轮未探测）。
    /// 有值时 level 的「待机」不可信——见 SessionProbeHealth。
    public let sessionProbeHealth: SessionProbeHealth?

    public var id: String { profile.id }

    /// 紧凑易读的内存显示文案（如 "128M"、"1.4G"）
    public var memoryText: String {
        MemoryFormat.text(memoryBytes)
    }

    public init(profile: AgentProfile, level: ActivityLevel, processRunning: Bool,
                cpuPercent: Double?, installed: Bool, activeSessions: Int,
                lastActivityAgo: TimeInterval?, lastActivityText: String,
                tokenUsage: TokenUsage? = nil, pid: Int32? = nil, currentAction: String? = nil,
                memoryBytes: UInt64 = 0, isHung: Bool? = nil,
                backgroundTasks: [AgentBackgroundTask] = [],
                subagents: [AgentSubagentInfo] = [],
                tokenBreakdown: AgentTokenBreakdown? = nil,
                sessionProbeHealth: SessionProbeHealth? = nil) {
        self.profile = profile
        self.level = level
        self.processRunning = processRunning
        self.cpuPercent = cpuPercent
        self.installed = installed
        self.activeSessions = activeSessions
        self.lastActivityAgo = lastActivityAgo
        self.lastActivityText = lastActivityText
        self.tokenUsage = tokenUsage
        self.pid = pid
        self.currentAction = currentAction
        self.memoryBytes = memoryBytes
        self.isHung = isHung
        self.backgroundTasks = backgroundTasks
        self.subagents = subagents
        self.tokenBreakdown = tokenBreakdown
        self.sessionProbeHealth = sessionProbeHealth
    }
}

// MARK: - 任务事件（生命周期关键节点）

public struct AgentTaskEvent: Identifiable, Equatable {
    public let id: UUID
    public let agentId: String
    public let agentName: String
    public let eventType: EventType
    public let duration: TimeInterval
    public let timestamp: Date
    public let pid: Int32?
    public let message: String?
    public let detail: String?
    /// 由外部投递（`agentisland://notify` 深链、`/notify` HTTP 端点）而非引擎自己判定。
    /// 岛内必须看得出来：否则任意本地进程（npm postinstall、cron、被截写的日志）
    /// 都能伪造一条与真实「等待你确认」逐字节相同的横幅与系统通知
    public let externallyDelivered: Bool

    public enum EventType: String, Equatable {
        case completed  // 任务执行完毕
        case attention  // 需要关注/等待确认
        case costSpike  // 消耗突增/死循环熔断告警
    }

    public init(id: UUID = UUID(), agentId: String, agentName: String, eventType: EventType, duration: TimeInterval, timestamp: Date = Date(), pid: Int32? = nil, message: String? = nil, detail: String? = nil, externallyDelivered: Bool = false) {
        self.id = id
        self.agentId = agentId
        self.agentName = agentName
        self.eventType = eventType
        self.duration = duration
        self.timestamp = timestamp
        self.pid = pid
        self.message = message
        self.detail = detail
        self.externallyDelivered = externallyDelivered
    }

    /// 「N分M秒」时长文案（唯一实现）：summaryText 兜底、引擎完成事件、系统通知共用。
    /// 四舍五入到秒——此前三处分别截断/四舍五入，同一事件可能显示 59秒 或 1分0秒
    public static func durationText(_ duration: TimeInterval) -> String {
        let seconds = max(0, Int(duration.rounded()))
        return seconds >= 60 ? "\(seconds / 60)分\(seconds % 60)秒" : "\(seconds)秒"
    }

    public var summaryText: String {
        if let message, !message.isEmpty {
            return message
        }
        switch eventType {
        case .completed:
            return "\(agentName) 任务完成 (\(Self.durationText(duration)))"
        case .attention:
            return "\(agentName) 等待确认操作"
        case .costSpike:
            return "⚠️ \(agentName) 资源/Token 消耗突增"
        }
    }

    public var copyableDiagnosticText: String {
        var lines = [
            "[\(eventType.rawValue.uppercased())] \(agentName) (ID: \(agentId))",
            "时间: \(timestamp)",
            "摘要: \(summaryText)"
        ]
        if let pid { lines.append("PID: \(pid)") }
        if let detail, !detail.isEmpty { lines.append("详情: \(detail)") }
        return lines.joined(separator: "\n")
    }
}

// MARK: - 会话语义信号

/// Agent 已停下来等待用户操作。fingerprint 是日志中的 request/call id（缺失时由
/// 检查器稳定生成），供采样引擎保证同一个请求只提醒一次。
public struct AgentAttentionRequest: Equatable {
    public let fingerprint: String
    public let message: String

    public init(fingerprint: String, message: String) {
        self.fingerprint = fingerprint
        self.message = message
    }
}

/// 会话日志给出的强语义信号。它优先于 CPU/mtime 这类活动近似值：
/// 明确等待用户时不是「工作中」，明确 task_complete 后也不应继续被写入窗口拖住。
public enum AgentSessionSignal: Equatable {
    case attention(AgentAttentionRequest)
    case completed(fingerprint: String)
    case active(fingerprint: String, action: String?)

    public var attentionRequest: AgentAttentionRequest? {
        guard case let .attention(request) = self else { return nil }
        return request
    }

    public var isActive: Bool {
        if case .active = self { return true }
        return false
    }

    public var fingerprint: String? {
        switch self {
        case let .attention(req): return req.fingerprint
        case let .completed(fp): return fp
        case let .active(fp, _): return fp
        }
    }

    public var actionText: String? {
        if case let .active(_, act) = self { return act }
        return nil
    }
}

/// 会话探测**自身**失败的原因。与 `AgentSessionSignal == nil` 是两件事：
/// 「没有信号」可能是真的空闲，也可能是根本没读到源——后者绝不能渲染成「待机」，
/// 否则第三方 App 改了 schema 时智能体永久失明，而用户看到的却是「空闲」且毫无证据。
/// 与 token 子系统里 `TokenSourceUsage.isAvailable`（false 表示没有可读取的明细，
/// 而不是用量为 0）是同一条诚实性规则，见 CONTEXT.md。
public enum SessionProbeFailure: String, Equatable {
    /// 会话库存在但打不开（无权限 / 被占用 / open_v2 返回非 OK）
    case unreadableDB
    /// 会话库能打开，但档案登记的查询 prepare 失败——典型形态：对方改了 schema
    case prepareFailed
    /// 会话文件超过单次读取上限，本轮信号直接放弃
    case oversizedFile
    /// 会话文件在，但读不出来（权限拒绝 / 是目录 / 读取抛错）——SQLite 侧早有
    /// `.unreadableDB` 的对应物，文件侧此前一条都不报，于是「读不到」被呈现成「闲着」
    case unreadableFile
    /// 会话文件读出来了，但不是解析器认识的形状（改版成 `{"messages":[…]}`、
    /// 顶层类型漂移等）。与「文件里确实没有待确认事项」是两件事，不得混为一谈
    case undecodableFile
    /// 查询能 prepare，但 `sqlite3_step` 返回了既不是 ROW 也不是 DONE 的码
    /// （BUSY / LOCKED / CORRUPT…）。prepareFailed 抓的是「结构变了」，抓不到这一类：
    /// 对方**正在写库**的那一拍最容易撞上 BUSY，而撞上之后按「库里没行」处理，
    /// 正在等确认的卡片就变成了「待机」——恰好在最需要提醒的那一刻失灵。
    case stepFailed

    public var label: String {
        switch self {
        case .unreadableDB: return "会话数据库无法打开"
        case .prepareFailed: return "会话数据库结构已变更（查询无法执行）"
        case .oversizedFile: return "会话文件超出单次读取上限"
        case .unreadableFile: return "会话文件无法读取"
        case .undecodableFile: return "会话文件格式与解析器不匹配"
        case .stepFailed: return "会话数据库查询被中断（占用或损坏）"
        }
    }
}

/// 最近一次会话探测失败的证据（原因 + 涉及路径）。
///
/// 这是「最近已知状态」而不是事件流：每一拍探测覆盖一次，失败后不做任何节律上报，
/// 成功探测则清回 nil。路径要带上——用户据此才能判断是自家没跑过还是会话库搬家了。
public struct SessionProbeHealth: Equatable {
    public let failure: SessionProbeFailure
    public let path: String
    /// 这条「读不到」是哪一拍观测到的。它是最近值而不是事件，必须有保质期：
    /// 源恢复之后若长时间没有新写入（探测被跳过），旧故障会一直挂着，
    /// 于是「读不到」反过来伪装成「坏了」——同样是 CONTEXT.md 反对的谎报。
    public let observedAt: Date

    public init(failure: SessionProbeFailure, path: String, observedAt: Date = Date()) {
        self.failure = failure
        self.path = path
        self.observedAt = observedAt
    }

    /// 由引擎按采样时钟盖章（而不是构造点取 `Date()`）：合成时间的测试才能稳定判定保质期。
    public func observed(at date: Date) -> SessionProbeHealth {
        SessionProbeHealth(failure: failure, path: path, observedAt: date)
    }

    /// 详情卡与「复制当前状态诊断快照」里的单行文案：先说结论，再说这条结论的边界
    public var diagnosticText: String {
        "会话源不可读：\(failure.label)（\(path)）——此后的「待机」只代表没有读到信号，不代表智能体真的空闲"
    }
}

/// 一轮会话探测的完整产出：强语义信号 + 该 Agent 本轮的活跃上下文 + 探测失败原因。
///
/// 上下文与探测健康都必须随信号一起返回，不能存放在按 agent id 索引的全局缓存里：
/// 读取方（快照装配）拿的是「上一个被解析的 Agent」的上下文，会让 Claude、Codex 等
/// 任意 Agent 的卡片串到 Antigravity 的子任务与 Token 细分，且 Agent 退出后残留永不失效。
public struct AgentSessionProbe: Equatable {
    public var signal: AgentSessionSignal?
    public var context: SessionActiveContext
    /// 本轮探测为什么没读成（nil = 探测本身没问题，见 SessionProbeHealth）
    public var health: SessionProbeHealth?

    public init(signal: AgentSessionSignal? = nil, context: SessionActiveContext = SessionActiveContext(),
                health: SessionProbeHealth? = nil) {
        self.signal = signal
        self.context = context
        self.health = health
    }
}

/// 系统通知与点击回调之间的稳定路由载荷。只保存 Agent ID，不把提示正文或会话内容
/// 写进 userInfo，避免通知数据库持久化敏感任务文本。
public enum AgentNotificationRoute {
    public static let agentIdKey = "agentisland.agent-id"

    public static func userInfo(agentId: String) -> [String: String] {
        [agentIdKey: agentId]
    }

    public static func agentId(from userInfo: [AnyHashable: Any]) -> String? {
        guard let value = userInfo[agentIdKey] as? String, !value.isEmpty else { return nil }
        return value
    }

    public static func agentId(from userInfo: [String: String]) -> String? {
        guard let value = userInfo[agentIdKey], !value.isEmpty else { return nil }
        return value
    }
}

// MARK: - 引擎参数

public struct EngineConfig: Equatable {
    public var sampleInterval: TimeInterval = 2.0      // 有活动时采样间隔
    public var idleSampleInterval: TimeInterval = 5.0  // 全闲置时降频采样间隔（5s：反应速度与节能平衡）
    public var workingWindow: TimeInterval = 60.0      // 该窗口内有文件写入 → working（双信号之一）
    public var cpuThreshold: Double = 6.0              // 进程 CPU% 超过 → working（6%：过滤 Electron/Chromium 空闲微抖动，保证真工作才触发）
    public var activeSessionWindow: TimeInterval = 600.0 // 活跃会话计数窗口（10 分钟）
    public var minWorkingHold: TimeInterval = 10.0    // 滞回：working 信号消失后保持最短时长（防抖动）
    public var tokenAlertEnabled: Bool = true          // 是否开启 Token 突增告警
    public var tokenAlertThreshold: Int = 200_000      // 单分钟内 Token 增量阈值（默认 200k）
    public var runawayCpuAlert: Bool = true            // 是否开启持续高负荷死循环告警
    public var runawayCpuThreshold: Double = 70.0      // 持续死循环/高负载判定阈值 (70% CPU)
    public var runawayDurationThreshold: TimeInterval = 300 // 持续高负载时长阈值 (5 分钟)

    public init(sampleInterval: TimeInterval = 2.0,
                idleSampleInterval: TimeInterval = 5.0,
                workingWindow: TimeInterval = 60.0,
                cpuThreshold: Double = 6.0,
                activeSessionWindow: TimeInterval = 600.0,
                minWorkingHold: TimeInterval = 10.0,
                tokenAlertEnabled: Bool = true,
                tokenAlertThreshold: Int = 200_000,
                runawayCpuAlert: Bool = true,
                runawayCpuThreshold: Double = 70.0,
                runawayDurationThreshold: TimeInterval = 300) {
        self.sampleInterval = sampleInterval
        self.idleSampleInterval = idleSampleInterval
        self.workingWindow = workingWindow
        self.cpuThreshold = cpuThreshold
        self.activeSessionWindow = activeSessionWindow
        self.minWorkingHold = minWorkingHold
        self.tokenAlertEnabled = tokenAlertEnabled
        self.tokenAlertThreshold = tokenAlertThreshold
        self.runawayCpuAlert = runawayCpuAlert
        self.runawayCpuThreshold = runawayCpuThreshold
        self.runawayDurationThreshold = runawayDurationThreshold
    }

    /// cpuThreshold 合法区间（slider range / 钳制 / 归一化唯一来源）
    public static let cpuThresholdRange: ClosedRange<Double> = 1.0...50.0
    /// 采样间隔合法区间。下限 0.5s：小于此值会退化成忙循环——实测把间隔写成 0
    /// 会让引擎每秒采样 2895 次（timer 立即重排 + 空转），CPU 直接跑满。
    /// 上限 600s：再长就失去「监控」意义。
    public static let sampleIntervalRange: ClosedRange<Double> = 0.5...600.0
    /// 其余字段的合法区间（与设置页滑杆 range 对齐；滑杆是 UI 输入口径，这里是自愈口径）。
    /// minWorkingHold / runawayCpuThreshold / runawayDurationThreshold / tokenAlertThreshold
    /// 当前不暴露在设置页，也一并设防——配置值只能来自滑杆或默认值，其余都是脏数据。
    public static let workingWindowRange: ClosedRange<Double> = 10.0...300.0
    public static let activeSessionWindowRange: ClosedRange<Double> = 60.0...3600.0
    public static let minWorkingHoldRange: ClosedRange<Double> = 1.0...300.0
    public static let runawayCpuThresholdRange: ClosedRange<Double> = 10.0...100.0
    public static let runawayDurationThresholdRange: ClosedRange<Double> = 30.0...3600.0
    public static let tokenAlertThresholdRange: ClosedRange<Int> = 1_000...10_000_000

    /// 归一化：脏持久化值自愈的唯一入口。
    /// - 全部 Double 字段：NaN 先归位默认值再钳制（min/max 对 NaN 透传，直接钳会漏）
    /// - cpuThreshold / 采样间隔 / 工作窗口 / 活跃会话窗口 / 滞回 / 死循环阈值 / 告警阈值：
    ///   各自钳入合法区间（workingWindow/activeSessionWindow 为负会让对应信号通道整体失效）
    /// - sample ≤ idle 钳平（否则「闲置降频」逻辑反转）
    /// 设置表单、启动读取、配置热更新三处共用，钳制规则只此一处。
    public func normalized() -> EngineConfig {
        var c = self
        let base = EngineConfig()
        if c.sampleInterval.isNaN { c.sampleInterval = base.sampleInterval }
        if c.idleSampleInterval.isNaN { c.idleSampleInterval = base.idleSampleInterval }
        if c.workingWindow.isNaN { c.workingWindow = base.workingWindow }
        if c.cpuThreshold.isNaN { c.cpuThreshold = base.cpuThreshold }
        if c.activeSessionWindow.isNaN { c.activeSessionWindow = base.activeSessionWindow }
        if c.minWorkingHold.isNaN { c.minWorkingHold = base.minWorkingHold }
        if c.runawayCpuThreshold.isNaN { c.runawayCpuThreshold = base.runawayCpuThreshold }
        if c.runawayDurationThreshold.isNaN { c.runawayDurationThreshold = base.runawayDurationThreshold }

        func clamp(_ v: Double, _ r: ClosedRange<Double>) -> Double {
            min(max(v, r.lowerBound), r.upperBound)
        }
        c.cpuThreshold = clamp(c.cpuThreshold, Self.cpuThresholdRange)
        c.sampleInterval = clamp(c.sampleInterval, Self.sampleIntervalRange)
        c.idleSampleInterval = clamp(c.idleSampleInterval, Self.sampleIntervalRange)
        c.workingWindow = clamp(c.workingWindow, Self.workingWindowRange)
        c.activeSessionWindow = clamp(c.activeSessionWindow, Self.activeSessionWindowRange)
        c.minWorkingHold = clamp(c.minWorkingHold, Self.minWorkingHoldRange)
        c.runawayCpuThreshold = clamp(c.runawayCpuThreshold, Self.runawayCpuThresholdRange)
        c.runawayDurationThreshold = clamp(c.runawayDurationThreshold, Self.runawayDurationThresholdRange)
        c.tokenAlertThreshold = min(max(c.tokenAlertThreshold, Self.tokenAlertThresholdRange.lowerBound),
                                    Self.tokenAlertThresholdRange.upperBound)
        if c.sampleInterval > c.idleSampleInterval {
            c.sampleInterval = c.idleSampleInterval
        }
        return c
    }

    /// 从持久化读取（SettingKey 键，缺项回落默认）并归一化——启动即自愈脏值。
    ///
    /// 注意：minWorkingHold / runawayCpuThreshold / runawayDurationThreshold 三个字段
    /// **刻意不持久化**（无对应 SettingKey，设置页也不暴露）——它们是行为调优常量，
    /// 持久化会复现 0.0.17 修过的「设置实效」类缺陷（设置页加了滑块却读不回来）。
    /// 未来要暴露它们时，必须同步补 SettingKey + 本函数读取分支 + 设置页滑杆三者。
    public static func load(from defaults: UserDefaults) -> EngineConfig {
        let base = EngineConfig()
        return EngineConfig(
            sampleInterval: defaults.object(forKey: SettingKey.sampleInterval) as? Double ?? base.sampleInterval,
            idleSampleInterval: defaults.object(forKey: SettingKey.idleSampleInterval) as? Double ?? base.idleSampleInterval,
            workingWindow: defaults.object(forKey: SettingKey.workingWindow) as? Double ?? base.workingWindow,
            cpuThreshold: defaults.object(forKey: SettingKey.cpuThreshold) as? Double ?? base.cpuThreshold,
            activeSessionWindow: defaults.object(forKey: SettingKey.activeSessionWindow) as? Double ?? base.activeSessionWindow,
            tokenAlertEnabled: defaults.object(forKey: SettingKey.tokenAlertEnabled) as? Bool ?? base.tokenAlertEnabled,
            tokenAlertThreshold: defaults.object(forKey: SettingKey.tokenAlertThreshold) as? Int ?? base.tokenAlertThreshold,
            runawayCpuAlert: defaults.object(forKey: SettingKey.runawayCpuAlert) as? Bool ?? base.runawayCpuAlert
        ).normalized()
    }
}
