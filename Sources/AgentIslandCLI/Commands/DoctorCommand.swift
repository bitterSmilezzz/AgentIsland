import Foundation
import AgentIslandCore

/// `agentisland doctor` —— 一次性实况自查：回答「这个 Agent 显示待机，是真闲着，
/// 还是我根本没看到它」。结论与依据都来自引擎本轮真实算出来的东西
/// （会话探测健康、进程/安装判定、活跃会话数），不新造第二套判断逻辑。
public enum DoctorCommand {

    @MainActor
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")
        let showAll = args.contains("--all") || args.contains("-a")
        let quiet = args.contains("--quiet") || args.contains("-q")

        var onlyAgent: String?
        if let idx = args.firstIndex(of: "--agent"), args.indices.contains(idx + 1) {
            onlyAgent = args[idx + 1]
        }

        // 双采：doctor 是排障入口，宁可多等 1.5s 也要拿到真实 CPU 利用率
        let engine = LiveSampler.makeEngine(restrictToEnabled: !showAll, refreshUsage: true)
        // 进度提示走 stderr：`--json` 的 stdout 必须是可被 jq 直接吃的纯 JSON
        if !quiet && !isJson {
            print(CLIColor.dim("  ⏳ 采集 CPU 基线…（1.5s）"))
        }
        var snaps = LiveSampler.twoBeatSample(engine)
        if let onlyAgent {
            snaps = snaps.filter { $0.id == onlyAgent || $0.profile.name.lowercased() == onlyAgent.lowercased() }
            if snaps.isEmpty {
                FileHandle.standardError.write((CLIColor.red("未找到智能体: \(onlyAgent)") + "\n").data(using: .utf8)!)
                exit(1)
            }
        }

        if isJson {
            let dtos = snaps.map { CLIAgentDoctorDTO(from: $0) }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(dtos), let str = String(data: data, encoding: .utf8) {
                print(str)
            }
            return
        }

        let verdicts = snaps.map { ($0, AgentObservability.evaluate(snapshot: $0)) }
        let blind = verdicts.filter { $0.1.code == .blindSessionSource }
        let noData = verdicts.filter { $0.1.code == .noLocalData }
        let absent = verdicts.filter { $0.1.code == .notInstalled }
        let unwired = verdicts.filter { $0.1.code == .sourceNotWired }

        print("\n" + CLIColor.bold("🩺 AgentIsland 可观测性自查"))
        print(CLIColor.dim("──────────────────────────────────────────"))
        // 四类计数相加必须等于总行数，否则读者无法核对
        print("  共 \(verdicts.count) 项：" + CLIColor.green("结论可信 \(verdicts.filter { $0.1.isTrustworthy }.count)")
              + CLIColor.dim("   ·   待机不可读 ") + CLIColor.red("\(blind.count)")
              + CLIColor.dim("   ·   无本地明细 ") + CLIColor.yellow("\(noData.count)")
              + CLIColor.dim("   ·   未接入明细源 ") + CLIColor.dim("\(unwired.count)")
              + CLIColor.dim("   ·   未安装 ") + CLIColor.dim("\(absent.count)"))

        var table = CLITable(columns: [
            CLITable.Column("智能体", minWidth: 16),
            CLITable.Column("状态", minWidth: 8),
            CLITable.Column("结论", minWidth: 14),
            CLITable.Column("依据", minWidth: 24)
        ])
        for (snap, verdict) in verdicts {
            let conclusion: String
            switch verdict.code {
            case .observed: conclusion = CLIColor.green(verdict.summary)
            case .blindSessionSource: conclusion = CLIColor.red(verdict.summary)
            case .noLocalData: conclusion = CLIColor.yellow(verdict.summary)
            case .sourceNotWired: conclusion = CLIColor.dim(verdict.summary)
            case .notInstalled: conclusion = CLIColor.dim(verdict.summary)
            }
            table.addRow([
                "\(snap.profile.emoji) \(snap.profile.name)",
                snap.level.label,
                conclusion,
                verdict.evidence.first ?? "—"
            ])
        }
        print("")
        print(table.render())

        if !blind.isEmpty {
            print("\n" + CLIColor.bold("需要处理"))
            for (snap, _) in blind {
                print("  • \(snap.profile.name)：会话源读不到，因此它的「待机」与用量都不作数。"
                      + CLIColor.dim("核对档案登记的库路径是否仍存在，或对方是否改了表结构。"))
            }
        }
        if !noData.isEmpty {
            print("  " + CLIColor.dim("• \(noData.count) 个在跑的 Agent 登记了明细源却读不到数据："
                                     + "可能未启用自动发现、会话写在别处，或一次性采样没等到异步刷新。"))
        }
        if !unwired.isEmpty {
            print("  " + CLIColor.dim("• \(unwired.count) 个未接入本地明细源（档案没有会话库与 token 目录）："
                                     + "它们的「待机」只代表进程与文件信号，属正常形态。"))
        }
        print("")
    }
}
