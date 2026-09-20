import Foundation
import AppKit
import AgentIslandCore

public enum OpenCommand {
    public static func run(args: [String]) async {
        let target = args.first ?? "toggle"
        let urlString: String

        switch target {
        case "toggle", "":
            urlString = "agentisland://toggle"
        case "expand":
            urlString = "agentisland://expand"
        case "collapse":
            urlString = "agentisland://collapse"
        case "analytics", "tokens":
            urlString = "agentisland://analytics"
        case "toolbox":
            urlString = "agentisland://toolbox"
        case "clean":
            urlString = "agentisland://clean"
        case "export":
            urlString = "agentisland://export"
        case "agent":
            let agentId = args.count > 1 ? args[1] : ""
            urlString = "agentisland://agent?id=\(agentId)"
        default:
            // 假设 target 为 agent ID
            urlString = "agentisland://agent?id=\(target)"
        }

        guard let url = URL(string: urlString) else {
            print(CLIColor.red("无效的 URL 目标: \(urlString)"))
            return
        }

        let success = NSWorkspace.shared.open(url)
        if success {
            print(CLIColor.green("已触发操作: \(urlString)"))
        } else {
            // 降级使用 /usr/bin/open
            let task = Process()
            task.executableURL = URL(fileURLWithPath: "/usr/bin/open")
            task.arguments = [urlString]
            do {
                try task.run()
                task.waitUntilExit()
                guard task.terminationStatus == 0 else {
                    CLIExit.fail("无法打开 URL \(urlString)：open 以 \(task.terminationStatus) 退出")
                }
                print(CLIColor.green("已触发操作: \(urlString)"))
            } catch {
                CLIExit.fail("无法打开 URL \(urlString): \(error.localizedDescription)")
            }
        }
    }
}
