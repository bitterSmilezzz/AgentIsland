import Foundation

// MARK: - 异常驻留与死锁持续守护状态机 (v0.0.75)

public final class AgentResilienceGuard {

    private struct AgentGuardState {
        var hungStartTime: Date?
        var lastAlertedHung: Date?
        var highMemStartTime: Date?
        var lastAlertedMem: Date?
    }

    private var states: [String: AgentGuardState] = [:]

    /// 判定持续死锁的告警门槛（秒，默认 180 秒 = 3 分钟）
    public let hungThreshold: TimeInterval
    /// 判定内存严重泄漏的告警门槛（秒，默认 300 秒 = 5 分钟）
    public let memoryThreshold: TimeInterval
    /// 同一类型告警的冷却间隔（秒，默认 600 秒 = 10 分钟）
    public let alertCooldown: TimeInterval

    public init(
        hungThreshold: TimeInterval = 180,
        memoryThreshold: TimeInterval = 300,
        alertCooldown: TimeInterval = 600
    ) {
        self.hungThreshold = hungThreshold
        self.memoryThreshold = memoryThreshold
        self.alertCooldown = alertCooldown
    }

    /// 评估快照列表，返回需要向用户发出的持久性严重警报事件
    public func evaluate(snapshots: [AgentSnapshot], now: Date = Date()) -> [AgentTaskEvent] {
        var events: [AgentTaskEvent] = []
        let twoGB: UInt64 = 2 * 1024 * 1024 * 1024

        for snap in snapshots {
            guard snap.processRunning else {
                states.removeValue(forKey: snap.id)
                continue
            }

            var st = states[snap.id] ?? AgentGuardState()

            // 1. 持续死锁检测
            if snap.isHung {
                let start = st.hungStartTime ?? now
                st.hungStartTime = start
                let elapsed = now.timeIntervalSince(start)

                if elapsed >= hungThreshold {
                    let shouldAlert: Bool = {
                        guard let last = st.lastAlertedHung else { return true }
                        return now.timeIntervalSince(last) >= alertCooldown
                    }()

                    if shouldAlert {
                        st.lastAlertedHung = now
                        let mins = Int(elapsed / 60)
                        events.append(AgentTaskEvent(
                            agentId: snap.id,
                            agentName: snap.profile.name,
                            eventType: .attention,
                            duration: elapsed,
                            timestamp: now,
                            pid: snap.pid,
                            message: "\(snap.profile.name) 疑似死锁已达 \(mins) 分钟，建议点击逃生舱重置"
                        ))
                    }
                }
            } else {
                st.hungStartTime = nil
                st.lastAlertedHung = nil
            }

            // 2. 持续物理内存超高检测
            if snap.memoryBytes >= twoGB {
                let start = st.highMemStartTime ?? now
                st.highMemStartTime = start
                let elapsed = now.timeIntervalSince(start)

                if elapsed >= memoryThreshold {
                    let shouldAlert: Bool = {
                        guard let last = st.lastAlertedMem else { return true }
                        return now.timeIntervalSince(last) >= alertCooldown
                    }()

                    if shouldAlert {
                        st.lastAlertedMem = now
                        events.append(AgentTaskEvent(
                            agentId: snap.id,
                            agentName: snap.profile.name,
                            eventType: .attention,
                            duration: elapsed,
                            timestamp: now,
                            pid: snap.pid,
                            message: "\(snap.profile.name) 内存长期占用过高 (\(snap.memoryText))，防爆保护建议重置"
                        ))
                    }
                }
            } else {
                st.highMemStartTime = nil
                st.lastAlertedMem = nil
            }

            states[snap.id] = st
        }

        return events
    }
}
