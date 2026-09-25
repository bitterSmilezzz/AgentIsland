import Foundation
import AgentIslandCore

// MARK: - CLI state 子命令 (v0.0.135)
// 读**App 进程里**的实时状态（含自报与冲突）。与 `status` 的区别是这条命令的全部意义：
// `status` 是另起进程独立采样，它的自报登记表永远是空的（见 11 号票）。

public enum StateCommand {
    public static func run(args: [String]) async {
        if args.contains("--help") || args.contains("-h") {
            printHelp()
            return
        }
        // 解析在 Core（`AgentStateClient.parseArgs`）——那里有测试。
        // 以前「`--port` 后面没有值」会静默用默认端口去查一个错的端点
        let parsedArgs = AgentStateClient.parseArgs(args)
        if let usage = parsedArgs.error {
            FileHandle.standardError.write((usage + "\n").data(using: .utf8)!)
            exit(2)
        }
        let isJson = parsedArgs.json
        let port = parsedArgs.port

        let outcome = await fetch(port: port)
        switch outcome {
        case let .appState(state):
            if isJson {
                // 编码失败不是「打印一个空对象」：那会让调用方以为自己读到了一份空状态
                guard let data = try? JSONEncoder().encode(state),
                      let text = String(data: data, encoding: .utf8) else {
                    print(AgentStateClient.headline(for: .malformed) ?? "编码失败")
                    exit(1)
                }
                print(text)
            } else {
                render(state)
            }
        default:
            let headline = AgentStateClient.headline(for: outcome) ?? "读取失败"
            if isJson {
                guard let data = try? JSONEncoder().encode(
                        AgentStateClient.failureDTO(for: outcome)),
                      let text = String(data: data, encoding: .utf8) else {
                    print(headline)
                    exit(1)
                }
                print(text)
            } else {
                print(headline)
            }
            // 读不到就是没答上这个问题：退出码 1，别给调用方一个 0 让它以为拿到了空状态
            exit(1)
        }
    }

    private static func fetch(port: UInt16) async -> AgentStateClient.Outcome {
        var components = URLComponents()
        components.scheme = "http"
        components.host = "127.0.0.1"
        components.port = Int(port)
        components.path = "/state"
        guard let url = components.url else { return .unreachable }
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.timeoutInterval = 2.5
        // 令牌只读不生成：CLI 没有权利替用户创建一把新的
        if let token = SelfReportTokenStore().read() {
            request.setValue(token, forHTTPHeaderField: SelfReportTokenStore.headerName)
        }
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            // 2.5s 超时那类「连上了但没人答」也归 unreachable：对调用方来说两者是同一件事
            return AgentStateClient.classify(status: (response as? HTTPURLResponse)?.statusCode,
                                             data: data, transportError: false)
        } catch {
            return AgentStateClient.classify(status: nil, data: nil, transportError: true)
        }
    }

    private static func render(_ state: CLIAgentStateDTO) {
        let ago = max(0, Date().timeIntervalSince1970 - state.capturedAt)
        print("App 侧实时状态（\(state.agents.count) 个档案，采样于 \(Int(ago)) 秒前）")
        for agent in state.agents {
            var line = "  \(agent.name)  \(AgentStateClient.statusLabel(agent.status))"
            if let provenance = agent.provenance {
                line += "  来源：\(AgentStateClient.provenanceLabel(provenance))"
            }
            if let left = agent.selfReportExpiresInSeconds {
                line += "（自报还剩 \(Int(left)) 秒）"
            }
            print(line)
            if let conflict = agent.conflictStatement {
                print("      ↳ \(conflict)")
            }
        }
    }

    private static func printHelp() {
        print("""
        agentisland state — 读 App 进程里的实时状态（含自报来源与冲突双显）

          agentisland state [--json] [--port <port>]

        与 `status` 的区别：`status` 是另起进程独立采样，它看不到 App 里的自报记录；
        这条命令走本机回环的 `GET /state`，与 `/session` 用同一枚令牌
        （`~/Library/Application Support/AgentIsland/report.token`）。
        连不上、被拒、端点不存在都会**明说**并以退出码 1 结束——不会给你一个看起来正常的空表。
        """)
    }
}
