import Foundation

// MARK: - 智能体健康度与稳定性诊断评估器 (v0.0.73)

public enum HealthGrade: String, CaseIterable, Equatable {
    case healthy = "健康"
    case partial = "观测不全"
    case attention = "需留意"
    case warning = "异常"
    case critical = "危急"

    public var icon: String {
        switch self {
        case .healthy: return "checkmark.shield.fill"
        case .partial: return "questionmark.circle.fill"
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

        // 1. 卡死 / 死锁检测（最严重）。三态里只有 .some(true) 扣分：
        // nil 是「本轮判不出」，给它扣分等于凭空造一个病症，给它过 100 分又等于宣布清白，
        // 所以扣分为 0、但在评级上作废「健康」（见下方 grade 降级）。
        switch snapshot.isHung {
        case .some(true):
            deduction += 50
            issues.append("疑似线程死锁或主循环卡死（长时间高负荷无响应）")
        case .some(false), .none:
            break
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
        var grade: HealthGrade = {
            if score >= 85 { return .healthy }
            if score >= 70 { return .attention }
            if score >= 50 { return .warning }
            return .critical
        }()

        // 「健康」是一条全清结论，而这一维判不出时它并不全清：一次性采样的每个入口
        // （`status` / `report` / `--probe`）都会拿到 nil，把「没测」播报成「没有卡死」。
        // 只在其余维度都干净时降级——CPU/内存已经扣分时那两级正在喊话，
        // 换成「观测不全」反而盖掉了真实的严重度。
        if snapshot.isHung == nil, grade == .healthy { grade = .partial }

        let summary: String = {
            switch grade {
            case .healthy: return "运行平稳正常"
            case .partial: return "已测维度无异常，死锁维度本轮未评估"
            case .attention: return "资源占用略高"
            case .warning: return "负载异常，建议关注"
            case .critical: return "严重异常，需紧急介入"
            }
        }()

        let suggestion: String = {
            if snapshot.isHung == true {
                return "检测到死锁，建议点击「终止逃生舱」重置智能体进程"
            }
            if snapshot.memoryBytes >= twoGB {
                return "长会话存在内存泄露隐患，建议在新会话中重新开始"
            }
            if snapshot.cpuPercent >= 80 {
                return "任务可能陷入重度计算或死循环，请检查终端日志"
            }
            // 措辞刻意不带阈值数字：那段时长只有一个来源（AnomalyScanGates.hungNotEvaluatedNote
            // 从配置里读），这里再写一遍就是同一结论两份措辞，漂移只是时间问题
            if snapshot.isHung == nil {
                return "本轮观测窗口不足以判定死锁/僵卡，此处不构成「没有卡死」的结论；"
                    + "持续观测请用灵动岛工作台或 `agentisland top`"
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
