import Foundation
import AgentIslandCore

public enum CheckCommand {
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")

        let cleaner = AgentCleaner(processMonitor: ProcessProvider())
        let anomalies = cleaner.scanAnomalies(profiles: AgentRegistry.builtin)

        if isJson {
            let dtos = anomalies.map { CLIAnomalyDTO(from: $0) }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(dtos), let str = String(data: data, encoding: .utf8) {
                print(str)
            }
            return
        }

        print("\n" + CLIColor.bold("🔍 AgentIsland 智能体异常排查"))
        print(CLIColor.dim("──────────────────────────────────────────"))

        if anomalies.isEmpty {
            print(CLIColor.green("  ✨ 状态健康：未检测到任何死锁、孤儿或内存溢出进程。\n"))
            return
        }

        print(CLIColor.yellow("  ⚠️ 检测到 \(anomalies.count) 个潜在异常进程：\n"))

        var table = CLITable(columns: [
            CLITable.Column("异常类型", minWidth: 10),
            CLITable.Column("智能体", minWidth: 12),
            CLITable.Column("PID", minWidth: 7, alignRight: true),
            CLITable.Column("PPID", minWidth: 7, alignRight: true),
            CLITable.Column("CPU", minWidth: 7, alignRight: true),
            CLITable.Column("内存", minWidth: 8, alignRight: true),
            CLITable.Column("批量清理", minWidth: 8),
            CLITable.Column("诊断原因", minWidth: 20)
        ])

        for a in anomalies {
            let typeStr: String
            switch a.anomalyType {
            case .hung:
                typeStr = CLIColor.red("死锁/僵死")
            case .orphan:
                typeStr = CLIColor.yellow("孤儿进程")
            case .overweight:
                typeStr = CLIColor.magenta("内存泄漏")
            }

            let batchStr = a.batchCleanable ? CLIColor.green("可批量") : CLIColor.dim("建议核对")
            let cpuStr = String(format: "%.1f%%", a.cpuPercent)

            table.addRow([
                typeStr,
                a.agentName,
                "\(a.pid)",
                "\(a.ppid)",
                cpuStr,
                a.memoryText,
                batchStr,
                a.reason
            ])
        }

        print(table.render())
        print("\n" + CLIColor.dim("提示：运行 ") + CLIColor.cyan("agentisland clean") + CLIColor.dim(" 可一键自动终止并释放资源。") + "\n")
    }
}
