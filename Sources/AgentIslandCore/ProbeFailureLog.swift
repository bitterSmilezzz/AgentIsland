import Foundation

// MARK: - 会话探测失败的日志出口
//
// 快照上的 `sessionProbeHealth` 只有 120s 保质期（见 ActivityEngine.probeHealthStaleness），
// 它回答「现在这一刻可信吗」；而运维真正要问的是「这个源是从什么时候开始读不到的」。
// 所以同一份失败原因另外走 AppLog：坏源不会每 2s 打一条（按 Agent+原因冷却），
// 恢复时补一条，时间线因此完整。
@MainActor
public enum ProbeFailureLog {
    /// 同一 (agent, 失败类型) 两次记录的最小间隔
    static let cooldown: TimeInterval = 600

    private static var lastLogged: [String: Date] = [:]

    private static func key(_ agentId: String, _ failure: SessionProbeFailure) -> String {
        "\(agentId)|\(failure.rawValue)"
    }

    /// 记录一次探测失败。返回是否真的落了一条（测试与调用方都需要这个确定性）
    @discardableResult
    public static func record(_ health: SessionProbeHealth, agentId: String, now: Date = Date()) -> Bool {
        let id = key(agentId, health.failure)
        if let last = lastLogged[id], now.timeIntervalSince(last) < cooldown {
            return false
        }
        lastLogged[id] = now
        AppLog.error("会话探测失败 [\(agentId)] \(health.diagnosticText)")
        return true
    }

    /// 之前记过故障、这一拍探测正常 → 记一条恢复并清掉冷却，下次再坏能立刻报
    public static func recordRecovery(agentId: String, now: Date = Date()) {
        let stale = lastLogged.keys.filter { $0.hasPrefix("\(agentId)|") }
        guard !stale.isEmpty else { return }
        for k in stale { lastLogged[k] = nil }
        AppLog.warn("会话源恢复可读 [\(agentId)]")
    }

    /// 测试隔离用
    static func resetForTesting() { lastLogged.removeAll() }
}
