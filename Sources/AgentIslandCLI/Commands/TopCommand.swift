import Foundation
import AgentIslandCore
import Darwin

// MARK: - CLI 实时交互监控看板 (TopCommand - v0.0.78)
// 类似 htop / top 的轻量级 ANSI 实时监控看板，每秒动态刷新 Agent 运行态与 Token 消耗。

/// 阻塞读一个键（确认提示用；与看板主循环同一 raw 模式，不额外改终端状态）
func readKeyBlocking() -> UInt8 {
    var ch: UInt8 = 0
    _ = read(STDIN_FILENO, &ch, 1)
    return ch
}

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

        // 初始化采样引擎：走与 status/report/doctor 同一个一次性采样器。
        // 此前 top 自造一套：既不受启停集约束（实测 `活跃: 0 / 27 项`，而 status 是 10 项），
        // 也从不开用量轮询——一次性进程不 start()，tokenUsage 恒 nil，
        // 于是每行都印 `0`，把「没去取数」呈现成「没有消耗」（v0.0.90 在 status 修过的同一类）
        let engine = LiveSampler.makeEngine(restrictToEnabled: !showAll, refreshUsage: true)

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
                var statusIcon: String
                switch s.level {
                case .working:
                    if !s.backgroundTasks.isEmpty {
                        statusIcon = CLIColor.green("🟢 后台(\(s.backgroundTasks.count))")
                    } else if !s.subagents.isEmpty {
                        statusIcon = CLIColor.green("🟢 子任务(\(s.subagents.count))")
                    } else {
                        statusIcon = CLIColor.green("🟢 工作中")
                    }
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
                // nil 是「本轮没取到用量」，0 是「取到了、确实是 0」——两者不能同形
                let tokenStr = s.tokenUsage.map { TokenUsage.compact($0.tokens24h) } ?? "—"
                let costStr = s.tokenUsage.map { TokenUsage.cost($0.cost24h).isEmpty ? "—" : TokenUsage.cost($0.cost24h) } ?? "—"
                var actStr = s.lastActivityText.isEmpty ? "无记录" : s.lastActivityText
                if let act = s.currentAction, !act.isEmpty, s.level == .working {
                    actStr = act
                }

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
                            // 快速清理：先列目标等一次显式确认，再走与 CLI/App 同一条
                            // ProcessTerminator 路径（带身份复核）。
                            // 此前是单键裸发 SIGTERM：绕过复核、不杀进程树、
                            // 且无论 kill 成败都印「已清理 N 个」
                            let cleaner = AgentCleaner(processMonitor: ProcessProvider())
                            let anomalies = cleaner.scanAnomalies(profiles: engine.allProfiles)
                                .filter { $0.batchCleanable }
                            if anomalies.isEmpty {
                                lastCleanMessage = "没有可批量清理的异常进程"
                            } else {
                                print("\n\(CLIColor.yellow("将终止以下 \(anomalies.count) 个进程："))")
                                for a in anomalies {
                                    print("  pid \(a.pid)  \(a.agentName)  \(a.reason)")
                                }
                                print(CLIColor.dim("按 y 确认，其它键取消: "), terminator: "")
                                fflush(stdout)
                                let confirmed = readKeyBlocking() == UInt8(ascii: "y")
                                if confirmed {
                                    let result = cleaner.clean(anomalies: anomalies)
                                    lastCleanMessage = "已终止 \(result.terminatedPids.count)/\(anomalies.count) 个"
                                } else {
                                    lastCleanMessage = "已取消"
                                }
                            }
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
