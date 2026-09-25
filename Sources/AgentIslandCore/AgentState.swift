import Foundation

// MARK: - App 侧状态的读端点（11 号票）
//
// 症状：自报记录活在 **App 进程**的引擎里，而 `agentisland status` 是另起进程做一次性采样，
// 它的登记表永远是空的。实测（v0.0.134 发版后）：App 侧 `bound:true` 的一条自报，
// CLI 给的是 `status=idle provenance=inferred`。脚本作者据此会得出「这个 Agent 没自报过」——
// 而真相是「这个进程看不到 App 里的自报」。**「没看到」不等于「没有」**，这条口径本仓写在第一句。
//
// 这一票补的是读通路：`GET /state`（同一枚令牌）+ `agentisland state`。
// 刻意**不与 `status` 合并**：两套数据源拼在一行里，就会有两行互相矛盾而看不出来。
// 连不上时 CLI 必须**明说**这是独立采样，而不是静默给一个 `inferred`。

/// 一个 Agent 在 App 那一侧的当前视图。
///
/// **安全边界**：这一格里刻意**不带** `currentAction` / `detail` / `message` / 会话 id——
/// 那些是从别家 Agent 的会话正文里来的（命令、路径、提示语）。读端点要回答的是
/// 「它说自己是什么状态、这句话还有没有效」，不是「它在干什么活」。
/// 有一条用例拿哨兵值钉这件事，别把这些字段顺手加回来。
public struct CLIAgentStateEntryDTO: Codable, Equatable {
    public let id: String
    public let name: String
    /// App 这一拍对外给的状态（`ActivityLevel.rawValue`）
    public let status: String
    /// 这一拍的状态是谁说的（`selfReported` / `observed` / `inferred` / `conflict`）
    public let provenance: String?
    /// 冲突那句原文（不冲突时省略键——与 `CLIAgentStatusDTO` 既有约定一致）
    public let conflictStatement: String?
    public let processRunning: Bool
    /// 可信窗口内那条自报的状态；没有可信自报时为 nil
    public let selfReportedState: String?
    /// 那条自报还剩多少秒（TTL 剩余）。负数不会出现：过期就不再是「可信」
    public let selfReportExpiresInSeconds: Double?

    public init(from snapshot: AgentSnapshot, now: Date) {
        self.id = snapshot.profile.id
        self.name = snapshot.profile.name
        self.status = snapshot.level.rawValue
        self.provenance = snapshot.provenance?.rawValue
        self.conflictStatement = snapshot.conflictStatement
        self.processRunning = snapshot.processRunning
        self.selfReportedState = snapshot.selfReport?.state.rawValue
        self.selfReportExpiresInSeconds = snapshot.selfReport.map {
            max(0, $0.expiresAt.timeIntervalSince(now))
        }
    }
}

/// `GET /state` 的响应体。
public struct CLIAgentStateDTO: Codable, Equatable {
    /// 这句话是谁说的。脚本要靠它区分「App 的实时状态」与「CLI 自己采的样」——
    /// 少了这一格，两个来源的数据在 JSON 里长得一模一样
    public let generatedBy: String
    public let capturedAt: TimeInterval
    public let agents: [CLIAgentStateEntryDTO]

    public init(snapshots: [AgentSnapshot], now: Date) {
        self.generatedBy = "app"
        self.capturedAt = now.timeIntervalSince1970
        self.agents = snapshots.map { CLIAgentStateEntryDTO(from: $0, now: now) }
    }

    public init(generatedBy: String, capturedAt: TimeInterval, agents: [CLIAgentStateEntryDTO]) {
        self.generatedBy = generatedBy
        self.capturedAt = capturedAt
        self.agents = agents
    }
}

/// `GET /state` 那三种回脸的形状。**状态码与句子都在这里**：服务端只负责发，
/// 它自己写一个 403/503 就会出现「改了 Core 的口径、服务端还在发旧脸」的那天。
public enum AgentStateEndpoint {
    public static var deniedReply: LocalEventHTTP.Outcome {
        .reply(status: SelfReportWire.deniedStatus, body: LocalEventHTTP.json(
            CLINotifyResultDTO(success: false,
                               message: "没有有效令牌：这份实时状态不对外可读")))
    }
    public static var noEngineReply: LocalEventHTTP.Outcome {
        .reply(status: SelfReportWire.noEngineStatus, body: LocalEventHTTP.json(
            CLINotifyResultDTO(success: false, message: "引擎尚未就绪，本次没有状态可读")))
    }
    public static func body(snapshots: [AgentSnapshot], now: Date) -> String {
        LocalEventHTTP.json(CLIAgentStateDTO(snapshots: snapshots, now: now))
    }
}

/// `state --json` 在失败时给的那一格。`reason` 是给脚本 switch 的枚举，
/// `message` 是给人看的那句——两者分开，是因为脚本不该去匹配中文。
public struct CLIAgentStateFailureDTO: Codable, Equatable {
    public let success: Bool
    public let reason: String
    public let message: String
}

/// 客户端怎么解读一次 `/state` 请求。**结论与措辞都在这里**，CLI 不许自己拼句子：
/// 「连不上」和「App 拒了你」是两件事，混成一句「读不到」就是把两种原因折成一个布尔。
public enum AgentStateClient {
    public enum Outcome: Equatable {
        /// 拿到了 App 的实时状态
        case appState(CLIAgentStateDTO)
        /// 连不上（App 没在跑、端口没监听、超时）
        case unreachable
        /// 端口在听，但令牌不对或没给（403）
        case denied
        /// 405：这一路根本没这个端点（老版本 App 在跑）
        case unsupported
        /// 503：App 在跑、端点也在，但它的引擎还没就绪 —— 与「解不出」是两件事：
        /// 前者等一等就有，后者要升级
        case engineNotReady
        /// 200 但正文解不出来（版本漂移）
        case malformed
    }

    public static func classify(status: Int?, data: Data?, transportError: Bool) -> Outcome {
        if transportError || status == nil { return .unreachable }
        switch status! {
        case 200:
            guard let data, let decoded = try? JSONDecoder().decode(CLIAgentStateDTO.self, from: data)
            else { return .malformed }
            return .appState(decoded)
        case 403: return .denied
        case 404, 405: return .unsupported
        case 503: return .engineNotReady
        default: return .malformed
        }
    }

    /// `--json` 失败时给的那一格：`reason` 是给脚本 switch 的，`message` 是给人的
    /// （与 `/session` 的响应同一套分工）
    public static func failureDTO(for outcome: Outcome) -> CLIAgentStateFailureDTO {
        let reason: String
        switch outcome {
        case .appState: reason = "ok"
        case .unreachable: reason = "unreachable"
        case .denied: reason = "noToken"
        case .unsupported: reason = "unsupported"
        case .engineNotReady: reason = "engineNotReady"
        case .malformed: reason = "malformed"
        }
        return CLIAgentStateFailureDTO(success: false, reason: reason,
                                       message: headline(for: outcome) ?? "读到了状态")
    }

    /// `state` 的参数解析。**放在 Core 是因为 CLI 那一侧链接不到测试**：
    /// 「`--port` 是最后一个参数」这种形状的错误如果留在 `StateCommand` 里，
    /// 就会静默改用默认端口去查一个错的端点，而调用方以为自己指定过了。
    public static func parseArgs(_ args: [String]) -> (json: Bool, port: UInt16, error: String?) {
        var port: UInt16 = SelfReportWire.defaultPort
        var isJson = false
        var i = 0
        while i < args.count {
            let arg = args[i]
            if arg == "--json" {
                isJson = true
                i += 1
                continue
            }
            if arg == "--port" || arg == "-p" {
                guard i + 1 < args.count else {
                    return (isJson, port, "错误：\(arg) 需要一个端口号，但它后面没有值")
                }
                let raw = args[i + 1]
                guard let parsed = UInt16(raw), parsed > 0 else {
                    return (isJson, port, "错误：--port 需要一个 1-65535 的数字，实得「\(raw)」")
                }
                port = parsed
                i += 2
                continue
            }
            i += 1
        }
        return (isJson, port, nil)
    }

    /// 状态与来源的说法都从这里出：CLI 自己拼一份，就会有一份只改一半的措辞。
    /// 状态那一格**复用** `ActivityLevel.label`（同一个词，不另造）
    public static func statusLabel(_ raw: String) -> String {
        ActivityLevel(rawValue: raw)?.label ?? raw
    }

    public static func provenanceLabel(_ raw: String) -> String {
        AgentProvenance(rawValue: raw)?.explainText ?? raw
    }

    /// 给人看的那一句。`nil` = 拿到了数据，不需要解释为什么。
    public static func headline(for outcome: Outcome) -> String? {
        switch outcome {
        case .appState: return nil
        case .unreachable:
            // 这句话不许承诺「下面这份」：`state` 失败时什么都不打印，直接退出码 1。
            // 承诺一份不会出现的输出，和把「没看到」讲成「没有」是同一类错
            return "连不上本机灵动岛（App 没在跑，或端口没监听）：这次没有状态可读。"
                + "本 CLI 自己采的样在 `agentisland status`——那条命令读得到进程与 CPU，"
                + "但**读不到 App 进程里的自报记录**。"
        case .engineNotReady:
            return "灵动岛在跑，但它的引擎还没就绪（503）：等一两秒再读，这次没有状态可读。"
        case .denied:
            return "灵动岛拒了这次读取：没带令牌或令牌不对（`GET /state` 与 `/session` 用同一枚）。"
        case .unsupported:
            return "这个端点在跑着的灵动岛里不存在——多半是 App 版本比 CLI 旧。重启到新版本后重试。"
        case .malformed:
            return "读到了回脸但解不出来：CLI 与 App 的 `/state` 形状不一致，请把两边升到同一版。"
        }
    }
}
