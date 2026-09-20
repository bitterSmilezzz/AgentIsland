import Foundation
import AgentIslandCore

public enum CleanCommand {
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")
        let isDryRun = args.contains("--dry-run") || args.contains("-n")
        let isForce = args.contains("--force") || args.contains("-f")

        let cleaner = AgentCleaner(processMonitor: ProcessProvider())
        let anomalies = cleaner.scanAnomalies(profiles: AgentRegistry.builtin)

        let targets: [AgentAnomaly]
        if isForce {
            targets = anomalies
        } else {
            targets = anomalies.filter(\.batchCleanable)
        }

        if targets.isEmpty {
            if isJson {
                let res = CLICleanResultDTO(
                    success: true,
                    killedPids: [],
                    freedMemoryBytes: 0,
                    freedMemoryFormatted: "0 MB",
                    dryRun: isDryRun
                )
                printJSON(res)
            } else {
                print("\n" + CLIColor.green("✨ 未发现需要清理的异常进程。") + "\n")
            }
            return
        }

        if isDryRun {
            let totalMem = targets.reduce(0) { $0 + $1.memoryBytes }
            if isJson {
                let res = CLICleanResultDTO(
                    success: true,
                    killedPids: targets.map(\.pid),
                    freedMemoryBytes: totalMem,
                    freedMemoryFormatted: MemoryFormat.text(totalMem),
                    dryRun: true
                )
                printJSON(res)
            } else {
                print("\n" + CLIColor.bold("🔍 [Dry Run] 模拟清理预览（未实际执行）"))
                print(CLIColor.dim("──────────────────────────────────────────"))
                for t in targets {
                    print("  - [PID \(t.pid)] \(t.agentName) (\(t.anomalyType.rawValue)): 预计释放 \(t.memoryText)")
                }
                print("\n" + CLIColor.cyan("预计共终止 \(targets.count) 个进程，释放 \(MemoryFormat.text(totalMem)) 内存。\n"))
            }
            return
        }

        // 真实清理
        let result = cleaner.clean(anomalies: targets)

        if isJson {
            let res = CLICleanResultDTO(
                success: !result.terminatedPids.isEmpty,
                killedPids: result.terminatedPids,
                freedMemoryBytes: result.reclaimedMemoryBytes,
                freedMemoryFormatted: result.reclaimedMemoryText,
                dryRun: false
            )
            printJSON(res)
        } else {
            guard !result.terminatedPids.isEmpty else {
                // 一个都没杀掉（身份不符 / EPERM / 已退出）不能报「清理完成」
                CLIExit.fail("未能终止任何目标进程（\(targets.count) 个候选全部跳过或失败）")
            }
            print("\n" + CLIColor.bold("🧹 智能体异常进程清理完成"))
            print(CLIColor.dim("──────────────────────────────────────────"))
            print("  已终止进程: " + CLIColor.green("\(result.terminatedPids.count) 个"))
            print("  已释放内存: " + CLIColor.green(result.reclaimedMemoryText))
            print("")
        }
    }

    private static func printJSON<T: Encodable>(_ value: T) {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        if let data = try? encoder.encode(value), let str = String(data: data, encoding: .utf8) {
            print(str)
        }
    }
}
