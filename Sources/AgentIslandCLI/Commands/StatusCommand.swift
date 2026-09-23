import Foundation
import AgentIslandCore

public enum StatusCommand {
    @MainActor
    public static func run(args: [String]) async {
        if args.contains("--watch") || args.contains("-w") {
            await TopCommand.run(args: args)
            return
        }

        let isJson = args.contains("--json")
        let showAll = args.contains("--all") || args.contains("-a")
        // 同步取用量要解析全部会话索引（本机实测多花 5s），对脚本/Raycast 场景不划算：
        // 默认不取，用量列印「—」明说没取；`--usage` 才付这笔钱
        let wantUsage = args.contains("--usage")

        // 档案集走共享采样器的 fullRegistry 口径：此前 status 只遍历内置集，
        // 用户自定义与自动发现的 Agent 在 status / status --json 里根本不存在，
        // 而 report 能看到——同一份数据两个口径，脚本侧尤其容易被误导。
        // 单拍即可：本命令要快（Raycast / 脚本调用）。代价是 CPU 这一列根本没差分窗口，
        // 所以它印 `—` 而不是 0.0%；要真实利用率用 `doctor`（双采）或 `top`（持续观测）
        let engine = LiveSampler.makeEngine(restrictToEnabled: !showAll, refreshUsage: wantUsage)
        let snapshots = engine.sample()

        let filtered: [AgentSnapshot]
        if showAll {
            filtered = snapshots
        } else {
            filtered = snapshots.filter { s in
                s.installed || s.processRunning || s.level != .offline || !(s.tokenUsage?.isEmpty ?? true)
            }
        }

        if isJson {
            let dtos = filtered.map { CLIAgentStatusDTO(from: $0) }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(dtos), let str = String(data: data, encoding: .utf8) {
                print(str)
            }
            return
        }

        if filtered.isEmpty {
            print(CLIColor.dim("未检测到活跃或已安装的 Agent（使用 --all 查看全部支持的智能体）"))
            return
        }

        var table = CLITable(columns: [
            CLITable.Column("状态", minWidth: 8),
            CLITable.Column("智能体", minWidth: 14),
            CLITable.Column("PID", minWidth: 7, alignRight: true),
            CLITable.Column("CPU", minWidth: 7, alignRight: true),
            CLITable.Column("内存", minWidth: 8, alignRight: true),
            CLITable.Column("会话", minWidth: 5, alignRight: true),
            CLITable.Column("24h 用量", minWidth: 10, alignRight: true),
            CLITable.Column("24h 费用", minWidth: 8, alignRight: true),
            CLITable.Column("最近活动", minWidth: 16)
        ])

        for s in filtered {
            var statusText: String
            switch s.level {
            case .working:
                if !s.backgroundTasks.isEmpty {
                    statusText = CLIColor.green("🟢 后台(\(s.backgroundTasks.count))")
                } else if !s.subagents.isEmpty {
                    statusText = CLIColor.green("🟢 子任务(\(s.subagents.count))")
                } else {
                    statusText = CLIColor.green("🟢 工作中")
                }
            case .attention:
                statusText = CLIColor.yellow("⚠️ 需确认")
            case .completed:
                statusText = CLIColor.cyan("✅ 已完成")
            case .idle:
                statusText = CLIColor.yellow("🟡 待机")
            case .offline:
                statusText = CLIColor.dim("⚪️ 离线")
            }

            let icon = s.profile.emoji
            let nameText = "\(icon) \(s.profile.name)"
            let pidText = s.pid.map { "\($0)" } ?? CLIColor.dim("-")
            // 「没测」不印成 0.0%：CPU% 是差分量，本命令只采一拍，根本没有窗口。
            // 与用量列同一条口径（`—` = 没查，数字 = 查出来的值）
            let cpuText: String = {
                if !s.processRunning { return CLIColor.dim("-") }
                guard let cpu = s.cpuPercent else { return CLIColor.dim("—") }
                return String(format: "%.1f%%", cpu)
            }()
            let memText = s.processRunning ? MemoryFormat.text(s.memoryBytes) : CLIColor.dim("-")
            let sessionsText = s.activeSessions > 0 ? "\(s.activeSessions)" : CLIColor.dim("0")
            let tokensText: String
            let costText: String
            if let usage = s.tokenUsage {
                tokensText = usage.tokens24h > 0 ? TokenUsage.compact(usage.tokens24h) : CLIColor.dim("0")
                let costStr = TokenUsage.costText(usage.cost24h, zero: "$0.00")
                costText = usage.cost24h > 0 ? costStr : CLIColor.dim(costStr)
            } else {
                // 没取数就明说，别印 0 —— 用户无法分辨「没用」和「没查」
                tokensText = CLIColor.dim("—")
                costText = CLIColor.dim("—")
            }

            var activityText = ""
            if let act = s.currentAction, !act.isEmpty, s.level == .working {
                let agoStr = s.lastActivityAgo != nil ? formatAgoShort(s.lastActivityAgo!) : ""
                activityText = "\(act) \(CLIColor.dim(agoStr))".trimmingCharacters(in: .whitespaces)
            } else if let ago = s.lastActivityAgo {
                let agoStr = formatAgoShort(ago)
                let detail = s.lastActivityText
                activityText = "\(agoStr) \(detail)".trimmingCharacters(in: .whitespaces)
            } else {
                activityText = CLIColor.dim("无记录")
            }

            table.addRow([
                statusText,
                nameText,
                pidText,
                cpuText,
                memText,
                sessionsText,
                tokensText,
                costText,
                activityText
            ])
        }

        print("\n" + CLIColor.bold("🤖 AgentIsland 智能体运行态快照") + " " + CLIColor.dim("(\(filtered.count) 个条目)"))
        print(table.render())
        print("")
    }

    private static func formatAgoShort(_ seconds: TimeInterval) -> String {
        if seconds < 60 {
            return "\(Int(seconds))s前"
        } else if seconds < 3600 {
            return "\(Int(seconds / 60))m前"
        } else if seconds < 86400 {
            return "\(Int(seconds / 3600))h前"
        } else {
            return "\(Int(seconds / 86400))d前"
        }
    }
}
