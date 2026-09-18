import Foundation

/// 任务单次执行记录
public struct TaskDurationRecord: Codable, Equatable, Sendable {
    public let agentId: String
    public let duration: TimeInterval
    public let timestamp: Date

    public init(agentId: String, duration: TimeInterval, timestamp: Date) {
        self.agentId = agentId
        self.duration = duration
        self.timestamp = timestamp
    }
}

/// Agent 任务效能与工作时长统计指标
public struct AgentWorkStats: Equatable, Sendable {
    public let totalWorkTime: TimeInterval
    public let taskCount: Int
    public let averageDuration: TimeInterval
    public let maxDuration: TimeInterval

    public init(totalWorkTime: TimeInterval, taskCount: Int, averageDuration: TimeInterval, maxDuration: TimeInterval) {
        self.totalWorkTime = totalWorkTime
        self.taskCount = taskCount
        self.averageDuration = averageDuration
        self.maxDuration = maxDuration
    }

    public static let empty = AgentWorkStats(totalWorkTime: 0, taskCount: 0, averageDuration: 0, maxDuration: 0)

    /// 格式化总工作时长
    public var formattedTotalTime: String {
        guard totalWorkTime > 0 else { return "0秒" }
        let secs = Int(totalWorkTime.rounded())
        if secs < 60 {
            return "\(secs)秒"
        } else if secs < 3600 {
            let m = secs / 60
            let s = secs % 60
            return s > 0 ? "\(m)分\(s)秒" : "\(m)分钟"
        } else {
            let h = secs / 3600
            let m = (secs % 3600) / 60
            return m > 0 ? "\(h)小时\(m)分" : "\(h)小时"
        }
    }

    /// 格式化平均单任务时长
    public var formattedAverageDuration: String {
        guard averageDuration > 0 else { return "—" }
        let secs = Int(averageDuration.rounded())
        if secs < 60 {
            return "\(secs)秒/次"
        } else {
            let m = secs / 60
            let s = secs % 60
            return s > 0 ? "\(m)分\(s)秒/次" : "\(m)分/次"
        }
    }
}

/// 任务耗时与效率统计追踪器
public final class TaskDurationTracker: @unchecked Sendable {
    private let lock = NSLock()
    private var records: [String: [TaskDurationRecord]] = [:]
    private let maxRecordsPerAgent: Int

    public init(maxRecordsPerAgent: Int = 100) {
        self.maxRecordsPerAgent = maxRecordsPerAgent
    }

    /// 记录一次已完成任务
    public func record(agentId: String, duration: TimeInterval, timestamp: Date = Date()) {
        guard duration > 0 else { return }
        lock.lock()
        defer { lock.unlock() }

        var list = records[agentId] ?? []
        list.append(TaskDurationRecord(agentId: agentId, duration: duration, timestamp: timestamp))
        if list.count > maxRecordsPerAgent {
            list.removeFirst(list.count - maxRecordsPerAgent)
        }
        records[agentId] = list
    }

    /// 获取特定 Agent 过去 N 秒（默认 24 小时）的效能统计
    public func stats(for agentId: String, window: TimeInterval = 24 * 3600, now: Date = Date()) -> AgentWorkStats {
        lock.lock()
        let list = records[agentId] ?? []
        lock.unlock()

        let valid = list.filter { now.timeIntervalSince($0.timestamp) <= window }
        guard !valid.isEmpty else { return .empty }

        let total = valid.reduce(0.0) { $0 + $1.duration }
        let count = valid.count
        let avg = total / Double(count)
        let maxD = valid.map(\.duration).max() ?? 0

        return AgentWorkStats(
            totalWorkTime: total,
            taskCount: count,
            averageDuration: avg,
            maxDuration: maxD
        )
    }

    /// 清理已过期的记录（防止长期运行内存积压）
    public func prune(olderThan: Date) {
        lock.lock()
        defer { lock.unlock() }
        for (k, v) in records {
            records[k] = v.filter { $0.timestamp >= olderThan }
        }
    }
}
