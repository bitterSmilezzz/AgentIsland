import Foundation

// MARK: - 可观测性结论（「这条待机可信吗」）
//
// 此前判断「一个 Agent 的状态结论有多可信」的证据散在四处、且互不相识：
// · `AgentHealthEvaluator` 只看 isHung/CPU/内存，对**根本没在被监控**的 Agent 直接给 100 分「健康」
// · `SessionProbeHealth` 说得出「会话库读不到」，但只在详情页一个 9pt 标签与 Markdown 报告里
// · 安装判定（`installed`）与进程判定（`processRunning`）各自为真，合起来才是「离线」的含义
// · token 明细源是否被发现，又在分析页用另一套措辞
// 这里把它们合成**一条结论 + 依据**，供 CLI `doctor`、CSV/JSON 报表与应用内共用，
// 口径只有一处。刻意只做纯函数（输入只有快照），因此可表驱动测试。
public enum AgentObservability {

    public enum Code: String, Equatable {
        /// 结论可信：要么读到了会话信号，要么明确判定为离线/未安装
        case observed
        /// 进程在跑，但会话源读不到 —— 此时的「待机」只代表没读到信号
        case blindSessionSource
        /// 进程在跑、档案登记了明细源，但本轮没读到任何会话与用量
        case noLocalData
        /// 档案根本没登记本地明细源：读不到是设计如此，不是故障
        case sourceNotWired
        /// 未安装且进程不在：不该期待任何状态
        case notInstalled
    }

    public struct Verdict: Equatable {
        public let code: Code
        /// 人话依据，按贡献顺序排列（第一条即结论的直接支撑）
        public let evidence: [String]

        public var isTrustworthy: Bool { code == .observed }

        /// 一行式结论（CLI 与报表共用）
        public var summary: String {
            switch code {
            case .observed: return "结论可信"
            case .blindSessionSource: return "待机不可信：会话源读不到"
            case .noLocalData: return "无本地明细：读不到会话与用量"
            case .sourceNotWired: return "未接入明细源"
            case .notInstalled: return "未安装：不该期待状态"
            }
        }
    }

    public static func evaluate(snapshot: AgentSnapshot) -> Verdict {
        var evidence: [String] = []

        if !snapshot.processRunning {
            if !snapshot.installed {
                evidence.append("PATH 与 /Applications 均未发现该智能体")
                return Verdict(code: .notInstalled, evidence: evidence)
            }
            // 已安装但进程不在：「离线」本身就是可信结论（进程存在性是直接证据）
            return Verdict(code: .observed, evidence: ["进程不在，离线判定来自进程表本身"])
        }

        // 进程在跑 —— 此时状态全靠会话信号，读不到就不能把「待机」当结论。
        // 先问「本轮到底读到过什么」：待确认 / 已完成 / 工作中这些等级本身就是
        // 会话强语义的产物——源确实是通的（实测 Antigravity 处于「待确认」却因
        // 活跃会话数为 0 被判成「无本地明细」，自相矛盾）。
        switch snapshot.level {
        case .attention, .completed, .working:
            evidence.append("本轮读到了会话强语义（\(snapshot.level.label)）")
            return Verdict(code: .observed, evidence: evidence)
        case .idle, .offline:
            break
        }

        if let health = snapshot.sessionProbeHealth {
            evidence.append(health.diagnosticText)
            return Verdict(code: .blindSessionSource, evidence: evidence)
        }
        let hasUsage = (snapshot.tokenUsage?.tokensTotal ?? 0) > 0
        if snapshot.activeSessions == 0 && !hasUsage {
            if !snapshot.profile.hasLocalDetailSource {
                evidence.append("档案未登记本地明细源（会话库与 token 目录都没有），"
                                + "读不到明细是设计如此，不代表它没在工作")
                return Verdict(code: .sourceNotWired, evidence: evidence)
            }
            // 措辞必须留有余地：一次性采样（CLI doctor/status）不等待 token 监控的异步刷新，
            // 用量为空很可能只是没赶上，把「没赶上」说成「没有」就是新的谎报
            evidence.append("会话目录在判定窗口内没有活动会话，本轮也没取到用量账本"
                            + "（一次性采样不等待异步刷新，未必代表真的为零）")
            return Verdict(code: .noLocalData, evidence: evidence)
        }
        evidence.append("活跃会话 \(snapshot.activeSessions) 个")
        return Verdict(code: .observed, evidence: evidence)
    }
}
