import Foundation
import AgentIslandCore
import Darwin

// MARK: - CLI 实时交互监控看板 (TopCommand - v0.0.78)
// 类似 htop / top 的轻量级 ANSI 实时监控看板，每秒动态刷新 Agent 运行态与 Token 消耗。

public enum TopCommand {
    @MainActor
    public static func run(args: [String]) async {
        let showAll = args.contains("--all") || args.contains("-a")
        var intervalSeconds: Double = 1.5

        var i = 0
        while i < args.count {
            let arg = args[i]
            if (arg == "--interval" || arg == "-i") && i + 1 < args.count {
                if let sec = Double(args[i + 1]), sec >= 0.5 && sec <= 60.0 {
                    intervalSeconds = sec
                }
                i += 2
                continue
            }
            i += 1
        }

        // 初始化采样引擎
        let installedApps = InstalledAppsCache()
        installedApps.refresh()
        let registry = AgentRegistry.fullRegistry(
            installedCLIs: installedApps.installedCLIs(),
            installedBundles: installedApps.installedBundleIDs()
        )
        let engine = ActivityEngine(profiles: registry, installedApps: installedApps)

        // 终端 Raw 模式设置（非阻塞单字符读取）
        var origTermios = termios()
        let isTTY = isatty(STDIN_FILENO) == 1
        if isTTY {
            tcgetattr(STDIN_FILENO, &origTermios)
            var raw = origTermios
            raw.c_lflag &= ~tcflag_t(ECHO | ICANON)
            // VMIN = 0, VTIME = 1 (0.1 秒读超时，用于非阻塞轮询)
            withUnsafeMutableBytes(of: &raw.c_cc) { ptr in
                ptr[Int(VMIN)] = 0
                ptr[Int(VTIME)] = 1
            }
            tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw)
        }

        // 退出时恢复终端模式与光标
        defer {
            if isTTY {
                tcsetattr(STDIN_FILENO, TCSAFLUSH, &origTermios)
                print("\u{001B}[?25h", terminator: "") // 显示光标
                fflush(stdout)
            }
        }

        if isTTY {
            print("\u{001B}[?25l", terminator: "") // 隐藏光标
            fflush(stdout)
        }

        var lastCleanMessage: String? = nil
        var cleanMessageUntil = Date.distantPast
        let dateFormatter = DateFormatter()
        dateFormatter.dateFormat = "HH:mm:ss"

        var shouldExit = false

        while !shouldExit {
            let now = Date()
            let snapshots = engine.sample()

            var displayed = snapshots
            if !showAll {
                displayed = snapshots.filter { s in
                    s.level != .offline || s.processRunning || (s.lastActivityAgo != nil && s.lastActivityAgo! < 3600)
                }
                if displayed.isEmpty {
                    displayed = snapshots
                }
            }

            // 统计指标
            let workingCount = snapshots.filter { $0.level == .working }.count
            let totalCPU = snapshots.reduce(0.0) { $0 + $1.cpuPercent }
            let totalTokens24h = snapshots.reduce(0) { $0 + ($1.tokenUsage?.tokens24h ?? 0) }
            let totalCost24h = snapshots.reduce(0.0) { $0 + ($1.tokenUsage?.cost24h ?? 0.0) }

            // 清屏并将光标置顶
            var output = "\u{001B}[H\u{001B}[2J"

            // 顶部状态栏
            let timeStr = dateFormatter.string(from: now)
            output += CLIColor.bold("🏝️  AgentIsland Top Monitor") + "  " + CLIColor.dim("[\(timeStr)]")
            output += "  " + CLIColor.cyan("刷新: \(String(format: "%.1f", intervalSeconds))s")
            output += "  " + CLIColor.dim("[q]退出 [r]刷新 [c]一键清理") + "\n"

            let cpuStr = String(format: "%.1f%%", totalCPU)
            let tokenStr = TokenUsage.compact(totalTokens24h)
            let costStr = totalCost24h > 0 ? TokenUsage.cost(totalCost24h) : "$0.00"
            output += "  活跃: \(CLIColor.green("\(workingCount)")) / \(snapshots.count) 项"
            output += "   总 CPU: \(CLIColor.cyan(cpuStr))"
            output += "   24h Tokens: \(CLIColor.yellow(tokenStr))"
            output += "   24h 费用: \(CLIColor.white(costStr))\n"

            if now < cleanMessageUntil, let msg = lastCleanMessage {
                output += "  " + CLIColor.green("✓ \(msg)") + "\n"
            }
            output += "\n"

            // 表格渲染
            var table = CLITable(columns: [
                CLITable.Column("状态", minWidth: 8),
                CLITable.Column("智能体", minWidth: 16),
                CLITable.Column("PID", minWidth: 8, alignRight: true),
                CLITable.Column("CPU", minWidth: 8, alignRight: true),
                CLITable.Column("内存", minWidth: 8, alignRight: true),
                CLITable.Column("会话", minWidth: 6, alignRight: true),
                CLITable.Column("24h 用量", minWidth: 10, alignRight: true),
                CLITable.Column("24h 费用", minWidth: 9, alignRight: true),
                CLITable.Column("最近活动", minWidth: 12)
            ])

            for s in displayed {
                let statusIcon: String
                switch s.level {
                case .working:
                    statusIcon = CLIColor.green("🟢 工作中")
                case .attention:
                    statusIcon = CLIColor.yellow("⏳ 待确认")
                case .idle:
                    statusIcon = CLIColor.yellow("🟡 待机")
                case .completed:
                    statusIcon = CLIColor.cyan("✨ 已完成")
                case .offline:
                    statusIcon = CLIColor.dim("⚪️ 离线")
                }

                let pidStr = s.pid != nil ? "\(s.pid!)" : "-"
                let cpuStr = s.processRunning ? String(format: "%.1f%%", s.cpuPercent) : "-"
                let memStr = s.processRunning ? s.memoryText : "-"
                let tokenStr = (s.tokenUsage?.tokens24h ?? 0) > 0 ? TokenUsage.compact(s.tokenUsage!.tokens24h) : "0"
                let costStr = (s.tokenUsage?.cost24h ?? 0) > 0 ? TokenUsage.cost(s.tokenUsage!.cost24h) : "$0.00"
                let actStr = s.lastActivityText.isEmpty ? "无记录" : s.lastActivityText

                table.addRow([
                    statusIcon,
                    s.profile.name,
                    pidStr,
                    cpuStr,
                    memStr,
                    "\(s.activeSessions)",
                    tokenStr,
                    costStr,
                    actStr
                ])
            }

            output += table.render()
            print(output, terminator: "")
            fflush(stdout)

            // 等待下一次循环或响应键盘输入
            let loopStart = Date()
            while Date().timeIntervalSince(loopStart) < intervalSeconds {
                if isTTY {
                    var ch: UInt8 = 0
                    let n = read(STDIN_FILENO, &ch, 1)
                    if n > 0 {
                        if ch == UInt8(ascii: "q") || ch == UInt8(ascii: "Q") || ch == 3 || ch == 27 {
                            shouldExit = true
                            break
                        } else if ch == UInt8(ascii: "r") || ch == UInt8(ascii: "R") {
                            // 立即刷新
                            break
                        } else if ch == UInt8(ascii: "c") || ch == UInt8(ascii: "C") {
                            // 快速清理
                            let cleaner = AgentCleaner(processMonitor: ProcessProvider())
                            let anomalies = cleaner.scanAnomalies(profiles: AgentRegistry.builtin)
                            let toKill = anomalies.filter { $0.batchCleanable }
                            for a in toKill {
                                kill(a.pid, SIGTERM)
                            }
                            lastCleanMessage = "已清理 \(toKill.count) 个异常进程"
                            cleanMessageUntil = Date().addingTimeInterval(3.0)
                            break
                        }
                    }
                }
                try? await Task.sleep(nanoseconds: 100_000_000) // 0.1s
            }
        }

        // 退出时输出换行
        print("\n" + CLIColor.dim("已退出 AgentIsland 动态监控。"))
    }
}
