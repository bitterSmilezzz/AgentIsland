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
        self.agents = agents
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
