import Foundation
import AgentIslandCore

@main
struct AgentIslandCLI {
    static let version = AppVersion.string

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

        case "top", "watch":
            let subArgs = Array(rawArgs.dropFirst())
            await TopCommand.run(args: subArgs)

        case "tokens", "token", "stats":
            let subArgs = Array(rawArgs.dropFirst())
            await TokensCommand.run(args: subArgs)

        case "doctor", "diagnose":
            let subArgs = Array(rawArgs.dropFirst())
            await DoctorCommand.run(args: subArgs)

        case "check", "anomalies", "anomaly":
            let subArgs = Array(rawArgs.dropFirst())
            await CheckCommand.run(args: subArgs)

        case "clean", "kill-hung":
            let subArgs = Array(rawArgs.dropFirst())
            await CleanCommand.run(args: subArgs)

        case "selftest":
            let subArgs = Array(rawArgs.dropFirst())
            await SelftestCommand.run(args: subArgs)

        case "open":
            let subArgs = Array(rawArgs.dropFirst())
            await OpenCommand.run(args: subArgs)

        case "notify", "event", "alert":
            let subArgs = Array(rawArgs.dropFirst())
            await NotifyCommand.run(args: subArgs)

        case "report", "export":
            let subArgs = Array(rawArgs.dropFirst())
            await ReportCommand.run(args: subArgs)

        case "raycast":
            let subArgs = ["--format", "raycast"]
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
  \(CLIColor.cyan("status"))     查看所有 AI 智能体当前运行态快照 (默认，支持 -w 动态监控)
  \(CLIColor.cyan("top"))        类似 htop 的交互式全屏动态监控看板 (或 watch)
  \(CLIColor.cyan("tokens"))     查看 24h Token 用量明细、成本分析与月末预测
  \(CLIColor.cyan("doctor"))    一次性实况自查：这个 Agent 是真闲着，还是我根本没看到它
  \(CLIColor.cyan("check"))      排查长期死锁、孤儿后台与高内存泄漏异常
  \(CLIColor.cyan("clean"))      一键清理释放异常智能体占用的系统资源
  \(CLIColor.cyan("selftest"))  核心逻辑自检（假数据断言，验证构建本身；不读真实机器状态）
  \(CLIColor.cyan("open"))       通过深度链接呼出/联动桌面灵动岛
  \(CLIColor.cyan("notify"))     主动向灵动岛投递智能体完成、待确认或告警事件
  \(CLIColor.cyan("report"))     生成 Markdown / CSV / JSON 运维审计报告
  \(CLIColor.cyan("raycast"))    导出 Raycast Extension 命令清单配置

\(CLIColor.bold("常用选项:"))
  \(CLIColor.yellow("--json"))          以标准 JSON 结构化输出（供脚本/Raycast 调用）
  \(CLIColor.yellow("--all, -a"))       包含全部支持的 Agent（状态包含离线项）
  \(CLIColor.yellow("--usage"))         同步取回每 Agent 的 Token 用量（慢约 5s，否则该列显示 —）
  \(CLIColor.yellow("--agent <id>"))    只诊断指定智能体 (配合 doctor，可用 id 或名称)
  \(CLIColor.yellow("--quiet, -q"))     去掉进度提示，只输出结果 (配合 doctor)
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
  $ agentisland doctor
  $ agentisland doctor --json --all
  $ agentisland selftest
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
