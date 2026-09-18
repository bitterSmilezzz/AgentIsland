import Foundation

// MARK: - 智能体健康度与稳定性诊断评估器 (v0.0.73)

public enum HealthGrade: String, CaseIterable, Equatable {
    case healthy = "健康"
    case attention = "需留意"
    case warning = "异常"
    case critical = "危急"

    public var icon: String {
        switch self {
        case .healthy: return "checkmark.shield.fill"
        case .attention: return "info.circle.fill"
        case .warning: return "exclamationmark.triangle.fill"
        case .critical: return "exclamationmark.octagon.fill"
        }
    }
}

public struct AgentHealthReport: Equatable {
    public let score: Int
    public let grade: HealthGrade
    public let summary: String
    public let issues: [String]
    public let suggestion: String

    public init(score: Int, grade: HealthGrade, summary: String, issues: [String], suggestion: String) {
        self.score = min(max(score, 0), 100)
        self.grade = grade
        self.summary = summary
        self.issues = issues
        self.suggestion = suggestion
    }
}

public enum AgentHealthEvaluator {

    /// 针对快照评估健康度评分（100分制纯函数，零外部副作用）
    public static func evaluate(snapshot: AgentSnapshot) -> AgentHealthReport {
        guard snapshot.processRunning else {
            return AgentHealthReport(
                score: 100,
                grade: .healthy,
                summary: "未运行（待机休眠）",
                issues: [],
                suggestion: "智能体未启动或在会话间隙休眠，无系统资源占用"
            )
        }

        var deduction = 0
        var issues: [String] = []

        // 1. 卡死 / 死锁检测（最严重）
        if snapshot.isHung {
            deduction += 50
            issues.append("疑似线程死锁或主循环卡死（长时间高负荷无响应）")
        }

        // 2. CPU 激增与死循环倾向
        if snapshot.cpuPercent >= 80 {
            deduction += 25
            issues.append("CPU 持续占用高达 \(String(format: "%.1f", snapshot.cpuPercent))%")
        } else if snapshot.cpuPercent >= 50 {
            deduction += 10
            issues.append("CPU 负荷较高 (\(String(format: "%.1f", snapshot.cpuPercent))%)")
        }

        // 3. 内存驻留集 (RSS) 溢出与泄露倾向
        let twoGB: UInt64 = 2 * 1024 * 1024 * 1024
        let oneAndHalfGB: UInt64 = UInt64(1536) * 1024 * 1024
        if snapshot.memoryBytes >= twoGB {
            deduction += 30
            issues.append("物理内存严重过高 (\(snapshot.memoryText) ≥ 2.0GB)")
        } else if snapshot.memoryBytes >= oneAndHalfGB {
            deduction += 15
            issues.append("物理内存占用偏大 (\(snapshot.memoryText) ≥ 1.5GB)")
        }

        let score = max(0, 100 - deduction)
        let grade: HealthGrade = {
            if score >= 85 { return .healthy }
            if score >= 70 { return .attention }
            if score >= 50 { return .warning }
            return .critical
        }()

        let summary: String = {
            switch grade {
            case .healthy: return "运行平稳正常"
            case .attention: return "资源占用略高"
            case .warning: return "负载异常，建议关注"
            case .critical: return "严重异常，需紧急介入"
            }
        }()

        let suggestion: String = {
            if snapshot.isHung {
                return "检测到死锁，建议点击「终止逃生舱」重置智能体进程"
            }
            if snapshot.memoryBytes >= twoGB {
                return "长会话存在内存泄露隐患，建议在新会话中重新开始"
            }
            if snapshot.cpuPercent >= 80 {
                return "任务可能陷入重度计算或死循环，请检查终端日志"
            }
            if grade == .attention {
                return "进程资源使用正常，可继续观测执行进展"
            }
            return "会话心跳活跃，内存与 CPU 分布均衡"
        }()

        return AgentHealthReport(
            score: score,
            grade: grade,
            summary: summary,
            issues: issues,
            suggestion: suggestion
        )
    }
}
