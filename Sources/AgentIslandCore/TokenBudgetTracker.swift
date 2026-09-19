import Foundation

// MARK: - Token 预算告警级别与状态跟踪

public enum TokenBudgetStatus: Equatable {
    case disabled
    case normal(used: Int, budget: Int, ratio: Double)
    case warning(used: Int, budget: Int, ratio: Double)
    case exceeded(used: Int, budget: Int, ratio: Double)

    public var isExceeded: Bool {
        if case .exceeded = self { return true }
        return false
    }

    public var isWarning: Bool {
        if case .warning = self { return true }
        return false
    }

    public var ratio: Double {
        switch self {
        case .disabled: return 0
        case .normal(_, _, let r), .warning(_, _, let r), .exceeded(_, _, let r): return r
        }
    }
}

/// 负责 Token 预算超额与预警判定的状态机（防止每次采样都重复弹窗/响铃）
public final class TokenBudgetTracker {
    private var lastNotifiedLevel: Int = 0 // 0: normal, 1: warning(80%), 2: exceeded(100%)
    private var lastCheckedDay: Int = -1

    public init() {}

    /// 评估当前用量并返回是否需要触发告警（仅在阈值升级跨越时触发一次）
    public func evaluate(used24h: Int, budget: Int, now: Date = Date()) -> (status: TokenBudgetStatus, alertMessage: String?) {
        let calendar = Calendar.current
        let currentDay = calendar.component(.day, from: now)
        if currentDay != lastCheckedDay {
            lastCheckedDay = currentDay
            lastNotifiedLevel = 0
        }

        guard budget > 0 else {
            lastNotifiedLevel = 0
            return (.disabled, nil)
        }

        let ratio = Double(used24h) / Double(budget)
        let status: TokenBudgetStatus
        var alertMessage: String? = nil

        if ratio >= 1.0 {
            status = .exceeded(used: used24h, budget: budget, ratio: ratio)
            if lastNotifiedLevel < 2 {
                lastNotifiedLevel = 2
                let usedStr = TokenUsage.compact(used24h)
                let budgetStr = TokenUsage.compact(budget)
                let pct = Int(ratio * 100)
                alertMessage = "Token 消费已达预算上限：\(pct)% (\(usedStr) / \(budgetStr))"
            }
        } else if ratio >= 0.8 {
            status = .warning(used: used24h, budget: budget, ratio: ratio)
            if lastNotifiedLevel < 1 {
                lastNotifiedLevel = 1
                let usedStr = TokenUsage.compact(used24h)
                let budgetStr = TokenUsage.compact(budget)
                let pct = Int(ratio * 100)
                alertMessage = "Token 消费已接近预算预警线：\(pct)% (\(usedStr) / \(budgetStr))"
            }
        } else {
            status = .normal(used: used24h, budget: budget, ratio: ratio)
            if ratio < 0.75 {
                lastNotifiedLevel = 0
            }
        }

        return (status, alertMessage)
    }
}
