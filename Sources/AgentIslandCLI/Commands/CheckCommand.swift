import Foundation
import AgentIslandCore

public enum CheckCommand {
    @MainActor
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")

        // 档案集与 status/report/doctor 同口径：自定义与自动发现的 Agent 一样会留下
        // 孤儿与死锁进程，只看内置集等于对它们免疫
        let ctx = LiveSampler.context()
        // 孤儿佐证集要有活动证据才算得出来：单看进程表分不清「launchd 托管的常驻服务」
        // 与「终端关掉的遗孤」（ppid 都是 1），只有会话目录近 10 分钟无写入才敢报孤儿。
        // 此前这里只传 profiles，两道闸门走默认空集 → 活进程被报成孤儿、死锁一条扫不出。
        // restrictToEnabled=false：扫描覆盖完整注册表，佐证集也就必须覆盖它，
        // 否则被关掉监控的档案拿不到活动证据，会被成批误报成孤儿。
        let engine = LiveSampler.makeEngine(from: ctx, restrictToEnabled: false)
        let gates = AnomalyScanGates(snapshots: engine.sample(), sustainedObservation: false)
        let anomalies = AgentCleaner.warmedForOneShot()
            .scanAnomalies(profiles: ctx.registry, gates: gates)
        let hungNote = gates.canJudgeHung ? nil : AnomalyScanGates.hungNotEvaluatedNote(config: engine.config)

        if isJson {
            let dtos = anomalies.map { CLIAnomalyDTO(from: $0) }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(dtos), let str = String(data: data, encoding: .utf8) {
                print(str)
            }
            // stdout 要保持可被 jq 直接吃下，所以「死锁这一维本轮没查」走 stderr，
            // 而不是塞进 JSON 里改形状：空数组冒充的是「一切正常」，脚本读不出缺了一维
            if let hungNote { FileHandle.standardError.write((hungNote + "\n").data(using: .utf8)!) }
            return
        }

        print("\n" + CLIColor.bold("🔍 AgentIsland 智能体异常排查"))
        print(CLIColor.dim("──────────────────────────────────────────"))

        if anomalies.isEmpty {
            print(CLIColor.green("  ✨ 未检测到孤儿或内存溢出进程。"))
        } else {
            print(CLIColor.yellow("  ⚠️ 检测到 \(anomalies.count) 个潜在异常进程：\n"))
        }
        // 「没测」不能播报成「没有」：死锁是时间性判定，一次性扫描结构性地覆盖不到
        if let hungNote { print(CLIColor.dim("  " + hungNote) + "\n") }
        guard !anomalies.isEmpty else { return }

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
