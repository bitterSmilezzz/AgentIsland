import Foundation

// MARK: - CLI JSON 序列化模型

public struct CLIAgentStatusDTO: Codable, Sendable {
    public let id: String
    public let name: String
    public let status: String
    public let pid: Int32?
    /// nil = 本拍没有 CPU 差分窗口（一次性 `status` / `report` 的第一拍必然如此）。
    /// 与 `0.0` 分开写：后者是「测了，确实空闲」，前者是「还没来得及知道」
    public let cpuPercent: Double?
    public let memoryBytes: UInt64
    public let memoryFormatted: String
    public let activeSessions: Int
    /// 三态：`true` 判定卡死、`false` 观测窗口够长且清白、`null` 本轮判不出。
    /// 一次性 `status` / `report` 结构性地给不出那段观测窗口，所以它们输出 null 而不是 false
    /// ——与 `tokens24h` 用 nil 表示「没取」同一条口径（CONTEXT.md：源缺失不得当作零活动）
    public let isHung: Bool?
    /// nil = 本轮没有去取用量（一次性进程默认不付同步刷新的开销）；
    /// 0 = 取到了、确实是 0。两者混成一谈就违反 CONTEXT.md 的「源缺失不得当作零活动」
    public let tokens24h: Int?
    public let cost24h: Double?
    public let lastActivityAgoSeconds: TimeInterval?
    public let lastActivityText: String?
    /// 稳定性评分（`AgentHealthEvaluator`）：只看卡死/CPU/内存，与下面的可观测性结论是两件事
    public let healthScore: Int
    public let healthGrade: String
    /// 「这条状态结论可信吗」的口径（见 AgentObservability）。此前失明证据只在 Markdown 里，
    /// 脚本与 Raycast 读到的 JSON 会把「读不到」当成「闲着」
    public let observability: String
    public let observabilityEvidence: [String]

    public init(from snapshot: AgentSnapshot) {
        self.id = snapshot.id
        self.name = snapshot.profile.name
        self.status = snapshot.level.rawValue
        self.pid = snapshot.pid
        self.cpuPercent = snapshot.cpuPercent
        self.memoryBytes = snapshot.memoryBytes
        self.memoryFormatted = MemoryFormat.text(snapshot.memoryBytes)
        self.activeSessions = snapshot.activeSessions
        self.isHung = snapshot.isHung
        self.tokens24h = snapshot.tokenUsage?.tokens24h
        self.cost24h = snapshot.tokenUsage?.cost24h
        self.lastActivityAgoSeconds = snapshot.lastActivityAgo
        self.lastActivityText = snapshot.lastActivityText
        let health = AgentHealthEvaluator.evaluate(snapshot: snapshot)
        self.healthScore = health.score
        self.healthGrade = health.grade.rawValue
        let verdict = AgentObservability.evaluate(snapshot: snapshot)
        self.observability = verdict.code.rawValue
        self.observabilityEvidence = verdict.evidence
    }
}

public struct CLITokenReportDTO: Codable, Sendable {
    public let tokens24h: Int
    public let cost24h: Double
    public let tokensTotal: Int
    public let costTotal: Double
    public let projectedMonthEndTokens: Int
    public let projectedMonthEndCost: Double
    public let budgetExhaustionDay: Int?
    public let forecastSummary: String
    public let dailyBudget: Int?
    public let budgetRatio: Double?
    public let budgetStatus: String?
    public let agents: [String: CLIAgentTokenDTO]

    public init(
        tokens24h: Int,
        cost24h: Double,
        tokensTotal: Int,
        costTotal: Double,
        projectedMonthEndTokens: Int,
        projectedMonthEndCost: Double,
        budgetExhaustionDay: Int?,
        forecastSummary: String,
        dailyBudget: Int? = nil,
        budgetRatio: Double? = nil,
        budgetStatus: String? = nil,
        agents: [String: CLIAgentTokenDTO]
    ) {
        self.tokens24h = tokens24h
        self.cost24h = cost24h
        self.tokensTotal = tokensTotal
        self.costTotal = costTotal
        self.projectedMonthEndTokens = projectedMonthEndTokens
        self.projectedMonthEndCost = projectedMonthEndCost
        self.budgetExhaustionDay = budgetExhaustionDay
        self.forecastSummary = forecastSummary
        self.dailyBudget = dailyBudget
        self.budgetRatio = budgetRatio
        self.budgetStatus = budgetStatus
        self.agents = agents
    }
}

public struct CLINotifyRequestDTO: Codable, Sendable {
    public let agent: String
    public let type: String
    public let message: String
    public let detail: String?

    public init(agent: String, type: String, message: String, detail: String? = nil) {
        self.agent = agent
        self.type = type
        self.message = message
        self.detail = detail
    }
}

public struct CLINotifyResultDTO: Codable, Sendable {
    public let success: Bool
    public let eventId: String?
    public let message: String

    public init(success: Bool, eventId: String? = nil, message: String) {
        self.success = success
        self.eventId = eventId
        self.message = message
    }
}

/// `/session` 的响应（spec 第 3 节的契约形状）。
/// 三态不能压成一个 Bool：「没令牌」「pid 对不上」「没收到」是三种完全不同的下一步。
/// `message` 只在被拒时带**为什么**——`reason` 是给脚本读的枚举，人读的是这句话；
/// 缺一个就会出现「得回查代码才知道错在哪」。`expiresAt` 让接入方不必同步自己的时钟
/// 就知道还剩多久（服务端已经把 ttl 钳进 [15,600]，回给它实际生效的那个值）。
/// `/session` 响应里 `reason` 那一格的**全部**合法取值。它是要给脚本 `switch` 的枚举，
/// 所以这里一律不填人话（人话在 `message`）——上一轮同一格混装过 `"noToken"` 与「没有这条记录」，
/// 于是 `reason == "noToken"` 之外还得写字符串相等。
/// 超出 spec §3 那三个值的部分都写在注释里，03 号票的契约测试按这张表写。
public enum SelfReportReason: String, Codable, Sendable, CaseIterable {
    /// 正文不是 JSON 对象，或缺 agent/session，或 pid 给了却读不出
    case malformed
    /// 注册表里没有这个档案（绝不自动建档）。只在**带令牌**时才回，否则与 `malformed`
    /// 收敛成同一个 `noToken`，免得 `/session` 成为「这台机器装了哪些 Agent」的免费枚举口
    case unknownAgent
    /// state/hook_event_name 不在该档案已知的取值里
    case unknownState
    case pidMismatch
    /// 没有令牌或令牌不对：不采信，申报落回 `externallyDelivered` 那条无鉴权通道
    case noToken
    /// 档案已经不在了（与 pid 无关，所以不复用 `pidMismatch`）
    case profileGone
    /// DELETE 指向的 (agent, session) 没有记录
    case notFound
    /// `/session` 只认 POST 与 DELETE
    case badMethod
    /// 引擎没就绪：这是**服务端**的状态，不该报成调用方的请求有问题（HTTP 503）
    case noEngine
}

public struct CLISessionResultDTO: Codable, Sendable {
    public let bound: Bool
    public let reason: SelfReportReason?
    public let message: String?
    public let expiresAt: TimeInterval?

    public init(bound: Bool, reason: SelfReportReason? = nil, message: String? = nil,
                expiresAt: TimeInterval? = nil) {
        self.bound = bound
        self.reason = reason
        self.message = message
        self.expiresAt = expiresAt
    }
}

/// `DELETE /session` 的响应。撤销与声明是两件事：它不写入状态，只把之前声明的收回去。
/// 与 bind 共用一个 DTO 会让 `bound:false` 同时表示「没绑上」和「已撤销」。
public struct CLISessionRevokeDTO: Codable, Sendable {
    public let revoked: Bool
    public let reason: SelfReportReason?
    public let message: String?

    public init(revoked: Bool, reason: SelfReportReason? = nil, message: String? = nil) {
        self.revoked = revoked
        self.reason = reason
        self.message = message
    }
}

public struct CLIAgentTokenDTO: Codable, Sendable {
    public let tokens24h: Int
    public let cost24h: Double
    public let tokensTotal: Int
    public let costTotal: Double

    public init(tokens24h: Int, cost24h: Double, tokensTotal: Int, costTotal: Double) {
        self.tokens24h = tokens24h
        self.cost24h = cost24h
        self.tokensTotal = tokensTotal
        self.costTotal = costTotal
    }
}

public struct CLIAnomalyDTO: Codable, Sendable {
    public let id: String
    public let pid: Int32
    public let ppid: Int32
    public let agentName: String
    public let type: String
    public let cpuPercent: Double
    public let memoryBytes: UInt64
    public let memoryFormatted: String
    public let reason: String
    public let batchCleanable: Bool

    public init(from anomaly: AgentAnomaly) {
        self.id = anomaly.id
        self.pid = anomaly.pid
        self.ppid = anomaly.ppid
        self.agentName = anomaly.agentName
        self.type = anomaly.anomalyType.rawValue
        self.cpuPercent = anomaly.cpuPercent
        self.memoryBytes = anomaly.memoryBytes
        self.memoryFormatted = anomaly.memoryText
        self.reason = anomaly.reason
        self.batchCleanable = anomaly.batchCleanable
    }
}

public struct CLICleanResultDTO: Codable, Sendable {
    public let success: Bool
    /// 复核确认已退出的 pid（不是「发过信号」的 pid）
    public let killedPids: [Int32]
    public let freedMemoryBytes: UInt64
    public let freedMemoryFormatted: String
    public let dryRun: Bool
    /// 发出信号但复核仍在运行的 pid：忽略终止信号的进程。脚本据此才知道还要人工处理
    public let unconfirmedPids: [Int32]

    public init(
        success: Bool,
        killedPids: [Int32],
        freedMemoryBytes: UInt64,
        freedMemoryFormatted: String,
        dryRun: Bool,
        unconfirmedPids: [Int32] = []
    ) {
        self.success = success
        self.killedPids = killedPids
        self.freedMemoryBytes = freedMemoryBytes
        self.freedMemoryFormatted = freedMemoryFormatted
        self.dryRun = dryRun
        self.unconfirmedPids = unconfirmedPids
    }
}

/// `agentisland doctor` 的结构化输出（见 AgentObservability）
public struct CLIAgentDoctorDTO: Codable, Sendable {
    public let id: String
    public let name: String
    public let status: String
    public let processRunning: Bool
    public let installed: Bool
    public let activeSessions: Int
    public let healthScore: Int
    /// 只有分数不够：一次性 doctor 的 100 分里死锁那一维是「没测」，
    /// 评级（观测不全）才是让脚本分辨得出「清白」与「没查」的那一位
    public let healthGrade: String
    /// observed / blindSessionSource / noLocalData / sourceNotWired / notInstalled
    public let observability: String
    public let evidence: [String]

    public init(from snapshot: AgentSnapshot) {
        self.id = snapshot.id
        self.name = snapshot.profile.name
        self.status = snapshot.level.rawValue
        self.processRunning = snapshot.processRunning
        self.installed = snapshot.installed
        self.activeSessions = snapshot.activeSessions
        let health = AgentHealthEvaluator.evaluate(snapshot: snapshot)
        self.healthScore = health.score
        self.healthGrade = health.grade.rawValue
        let verdict = AgentObservability.evaluate(snapshot: snapshot)
        self.observability = verdict.code.rawValue
        self.evidence = verdict.evidence
    }
}
