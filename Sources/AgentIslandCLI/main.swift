import Foundation
import AgentIslandCore

@main
struct AgentIslandCLI {
    static let version = "0.0.76"

    @MainActor
    static func main() async {
        let rawArgs = Array(CommandLine.arguments.dropFirst())

        if rawArgs.isEmpty {
            await StatusCommand.run(args: [])
            return
        }

        let first = rawArgs[0]

        switch first {
        case "--version", "-v", "version":
            print("agentisland v\(version)")

        case "--help", "-h", "help":
            printHelp()

        case "status", "ls", "ps":
            let subArgs = Array(rawArgs.dropFirst())
            await StatusCommand.run(args: subArgs)

        case "tokens", "token", "stats":
            let subArgs = Array(rawArgs.dropFirst())
            await TokensCommand.run(args: subArgs)

        case "check", "anomalies", "anomaly":
            let subArgs = Array(rawArgs.dropFirst())
            await CheckCommand.run(args: subArgs)

        case "clean", "kill-hung":
            let subArgs = Array(rawArgs.dropFirst())
            await CleanCommand.run(args: subArgs)

        case "open":
            let subArgs = Array(rawArgs.dropFirst())
            await OpenCommand.run(args: subArgs)

        case "report", "export":
            let subArgs = Array(rawArgs.dropFirst())
            await ReportCommand.run(args: subArgs)

        default:
            if first.starts(with: "-") {
                // 如果传入以 - 开头的参数（例如 --json, --all），视作 status 的参数
                await StatusCommand.run(args: rawArgs)
            } else {
                print(CLIColor.red("未知子命令: \(first)"))
                print("运行 " + CLIColor.cyan("agentisland --help") + " 查看支持的命令列表。")
                exit(1)
            }
        }
    }

    private static func printHelp() {
        let text = """

\(CLIColor.bold("🏝️  AgentIsland CLI")) \(CLIColor.dim("v\(version)"))
macOS 智能体运行态监控与运维终端工具

\(CLIColor.bold("用法:"))
  agentisland [command] [options]

\(CLIColor.bold("核心命令:"))
  \(CLIColor.cyan("status"))     查看所有 AI 智能体当前运行态快照 (默认)
  \(CLIColor.cyan("tokens"))     查看 24h Token 用量明细、成本分析与月末预测
  \(CLIColor.cyan("check"))      排查长期死锁、孤儿后台与高内存泄漏异常
  \(CLIColor.cyan("clean"))      一键清理释放异常智能体占用的系统资源
  \(CLIColor.cyan("open"))       通过深度链接呼出/联动桌面灵动岛
  \(CLIColor.cyan("report"))     生成 Markdown 运维审计报告

\(CLIColor.bold("常用选项:"))
  \(CLIColor.yellow("--json"))          以标准 JSON 结构化输出（供脚本/Raycast 调用）
  \(CLIColor.yellow("--all, -a"))       包含全部支持的 Agent（状态包含离线项）
  \(CLIColor.yellow("--dry-run, -n"))   模拟清理，仅预览不实际终止进程 (配合 clean)
  \(CLIColor.yellow("--force, -f"))     强行清理所有异常，包含孤儿后台 (配合 clean)
  \(CLIColor.yellow("--copy, -c"))      将生成的审计报告直接复制至系统剪贴板 (配合 report)
  \(CLIColor.yellow("--output, -o"))    将生成的审计报告写入文件 (配合 report)
  \(CLIColor.yellow("--version, -v"))   查看版本号
  \(CLIColor.yellow("--help, -h"))      查看此帮助指引

\(CLIColor.bold("联动示例:"))
  \(CLIColor.dim("# 查看当前活跃 Agent 与 Token 消耗"))
  $ agentisland
  $ agentisland status --json

  \(CLIColor.dim("# 排查并安全清理卡顿僵死的 Agent"))
  $ agentisland check
  $ agentisland clean --dry-run
  $ agentisland clean

  \(CLIColor.dim("# 呼出灵动岛或直达特定页面"))
  $ agentisland open
  $ agentisland open analytics
  $ agentisland open claude

"""
        print(text)
    }
}
