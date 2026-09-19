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

        let installedApps = InstalledAppsCache()
        installedApps.refresh()

        let engine = ActivityEngine(
            profiles: AgentRegistry.builtin,
            installedApps: installedApps
        )

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
            let cpuText = s.processRunning ? String(format: "%.1f%%", s.cpuPercent) : CLIColor.dim("-")
            let memText = s.processRunning ? MemoryFormat.text(s.memoryBytes) : CLIColor.dim("-")
            let sessionsText = s.activeSessions > 0 ? "\(s.activeSessions)" : CLIColor.dim("0")
            let tokens24h = s.tokenUsage?.tokens24h ?? 0
            let cost24h = s.tokenUsage?.cost24h ?? 0
            let tokensText = tokens24h > 0 ? TokenUsage.compact(tokens24h) : CLIColor.dim("0")
            let costText = cost24h > 0 ? TokenUsage.cost(cost24h) : CLIColor.dim("$0.00")

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
