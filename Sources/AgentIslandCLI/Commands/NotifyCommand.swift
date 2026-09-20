import Foundation
import AgentIslandCore

// MARK: - CLI notify 子命令 (v0.0.77)
// 允许从终端、脚本、CI/CD 流程直接向 AgentIsland 发送完成、等待确认或告警事件。

public enum NotifyCommand {
    public static func run(args: [String]) async {
        if args.contains("--help") || args.contains("-h") {
            printHelp()
            return
        }

        let isJson = args.contains("--json")

        var agent = "system"
        var type = "completed"
        var message: String? = nil
        var detail: String? = nil
        var port: UInt16 = 41999

        var i = 0
        while i < args.count {
            let arg = args[i]
            if (arg == "--agent" || arg == "-a") && i + 1 < args.count {
                agent = args[i + 1]
                i += 2
                continue
            }
            if (arg == "--type" || arg == "-t" || arg == "--event") && i + 1 < args.count {
                type = args[i + 1]
                i += 2
                continue
            }
            if (arg == "--message" || arg == "-m" || arg == "--msg") && i + 1 < args.count {
                message = args[i + 1]
                i += 2
                continue
            }
            if (arg == "--detail" || arg == "-d") && i + 1 < args.count {
                detail = args[i + 1]
                i += 2
                continue
            }
            if arg == "--port" && i + 1 < args.count {
                if let p = UInt16(args[i + 1]) {
                    port = p
                }
                i += 2
                continue
            }
            if !arg.hasPrefix("-") && message == nil {
                message = arg
            }
            i += 1
        }

        guard let finalMessage = message, !finalMessage.isEmpty else {
            print(CLIColor.red("✗ 缺少消息内容。用法: agentisland notify --agent <name> --message <text>"))
            return
        }

        let reqDTO = CLINotifyRequestDTO(agent: agent, type: type, message: finalMessage, detail: detail)

        // 优先通过 localhost HTTP 接口直推 (毫秒级)
        let httpSuccess = await sendHTTP(dto: reqDTO, port: port)

        if httpSuccess {
            if isJson {
                let res = CLINotifyResultDTO(success: true, message: "Event delivered via HTTP")
                if let data = try? JSONEncoder().encode(res), let s = String(data: data, encoding: .utf8) {
                    print(s)
                }
            } else {
                let badge = type.lowercased() == "completed" ? CLIColor.green("✓ 完成") :
                           (type.lowercased() == "attention" ? CLIColor.yellow("⏳ 待确认") : CLIColor.red("🚨 告警"))
                print("\(badge) 已向 AgentIsland 投递事件: [\(CLIColor.cyan(agent))] \(finalMessage)")
            }
            return
        }

        // 回退至 URL Scheme 分发
        let escapedAgent = agent.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? agent
        let escapedType = type.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? type
        let escapedMsg = finalMessage.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? finalMessage
        var urlStr = "agentisland://notify?agent=\(escapedAgent)&type=\(escapedType)&message=\(escapedMsg)"
        if let d = detail, let escapedDetail = d.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) {
            urlStr += "&detail=\(escapedDetail)"
        }

        if URL(string: urlStr) != nil {
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/open")
            proc.arguments = [urlStr]
            try? proc.run()
            proc.waitUntilExit()

            if isJson {
                let res = CLINotifyResultDTO(success: true, message: "Event delivered via URL Scheme")
                if let data = try? JSONEncoder().encode(res), let s = String(data: data, encoding: .utf8) {
                    print(s)
                }
            } else {
                print(CLIColor.yellow("⚡️ 已通过 URL Scheme 投递事件: [\(agent)] \(finalMessage)"))
            }
        }
    }

    private static func sendHTTP(dto: CLINotifyRequestDTO, port: UInt16) async -> Bool {
        guard let url = URL(string: "http://127.0.0.1:\(port)/notify") else { return false }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 1.0
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONEncoder().encode(dto)

        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                return true
            }
        } catch {
            return false
        }
        return false
    }

    private static func printHelp() {
        print("""
        \(CLIColor.bold("用法:"))
          agentisland notify [选项] [消息]

        \(CLIColor.bold("选项:"))
          -a, --agent <name>     智能体标识符 (如 claude, antigravity, codex，默认: system)
          -t, --type <type>      事件类型 (completed, attention, costSpike，默认: completed)
          -m, --message <text>   主消息内容
          -d, --detail <text>    次级详情说明
              --port <port>      本地监听端口 (默认: 41999)
              --json             输出 JSON 格式结果
          -h, --help             显示帮助

        \(CLIColor.bold("示例:"))
          agentisland notify -a antigravity -m "编译与测试完成"
          agentisland notify -a cursor -t attention -m "等待代码审阅确认"
          curl -X POST http://127.0.0.1:41999/notify -d '{"agent":"ci","type":"completed","message":"构建通过"}'
        """)
    }
}
