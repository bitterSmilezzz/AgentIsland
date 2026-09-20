import Foundation

// MARK: - CLI JSON 序列化模型

public struct CLIAgentStatusDTO: Codable, Sendable {
    public let id: String
    public let name: String
    public let status: String
    public let pid: Int32?
    public let cpuPercent: Double
    public let memoryBytes: UInt64
    public let memoryFormatted: String
    public let activeSessions: Int
    public let isHung: Bool
    public let tokens24h: Int
    public let cost24h: Double
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
        self.tokens24h = snapshot.tokenUsage?.tokens24h ?? 0
        self.cost24h = snapshot.tokenUsage?.cost24h ?? 0
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
    public let killedPids: [Int32]
    public let freedMemoryBytes: UInt64
    public let freedMemoryFormatted: String
    public let dryRun: Bool

    public init(
        success: Bool,
        killedPids: [Int32],
        freedMemoryBytes: UInt64,
        freedMemoryFormatted: String,
        dryRun: Bool
    ) {
        self.success = success
        self.killedPids = killedPids
        self.freedMemoryBytes = freedMemoryBytes
        self.freedMemoryFormatted = freedMemoryFormatted
        self.dryRun = dryRun
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
    /// observed / blindSessionSource / noLocalData / notInstalled
    public let observability: String
    public let evidence: [String]

    public init(from snapshot: AgentSnapshot) {
        self.id = snapshot.id
        self.name = snapshot.profile.name
        self.status = snapshot.level.rawValue
        self.processRunning = snapshot.processRunning
        self.installed = snapshot.installed
        self.activeSessions = snapshot.activeSessions
        self.healthScore = AgentHealthEvaluator.evaluate(snapshot: snapshot).score
        let verdict = AgentObservability.evaluate(snapshot: snapshot)
        self.observability = verdict.code.rawValue
        self.evidence = verdict.evidence
    }
}
