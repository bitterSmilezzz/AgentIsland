import Foundation
import AgentIslandCore

public enum CleanCommand {
    @MainActor
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")
        let isDryRun = args.contains("--dry-run") || args.contains("-n")
        let isForce = args.contains("--force") || args.contains("-f")
        // 孤儿要单独、显式地请进来。`--force` 原先的含义是「连孤儿一起杀」，
        // 而孤儿的判定本身分不清「用户刻意后台化的智能体」与「launchd 托管的常驻服务」
        // （ppid 都是 1）——一个批量开关不该跨过这道分不清的界线
        let includeOrphans = args.contains("--include-orphans")

        // 档案集与 check/status/report 同口径：自定义与自动发现的 Agent 一样会留下
        // 孤儿与死锁进程。此前这里只看 AgentRegistry.builtin，于是 check 列出某个
        // cli-* Agent 的异常并提示「运行 agentisland clean 可一键终止」，
        // 而 clean（含 --dry-run）看不见它并回答「未发现需要清理的异常进程」
        let profiles = LiveSampler.context().registry
        let cleaner = AgentCleaner(processMonitor: ProcessProvider())
        let anomalies = cleaner.scanAnomalies(profiles: profiles)

        let batch = anomalies.filter(\.batchCleanable)
        let orphans = anomalies.filter { !$0.batchCleanable }
        var targets = batch
        var declinedOrphans: [AgentAnomaly] = []

        if includeOrphans {
            guard !isJson else {
                // --json 是脚本消费的：脚本没有「逐条确认」这件事，
                // 让它静默杀掉孤儿等于把安全闸门藏进输出格式里
                CLIExit.fail("--include-orphans 不能与 --json 同用：孤儿只支持人工逐条确认",
                             code: CLIExit.badUsage)
            }
            guard isatty(FileHandle.standardInput.fileDescriptor) == 1 else {
                CLIExit.fail("标准输入不是终端：孤儿进程需要逐条人工确认，无法在非交互环境清理",
                             code: CLIExit.badUsage)
            }
            for orphan in orphans {
                print("  ⚠️ [PID \(orphan.pid)] \(orphan.agentName) 孤儿：\(orphan.reason)")
                print("     杀掉可能带走一个正在跑的任务。确认终止? [y/N] ", terminator: "")
                if readLine()?.trimmingCharacters(in: .whitespaces).lowercased() == "y" {
                    targets.append(orphan)
                } else {
                    declinedOrphans.append(orphan)
                }
            }
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
                print("\n" + CLIColor.green("✨ 未发现需要清理的异常进程。"))
                // 有异常但都被闸门挡住时，必须说清「为什么看起来一切正常」：
                // 只报「没有异常」会让用户以为孤儿不存在
                if !orphans.isEmpty {
                    print(CLIColor.dim("  （\(orphans.count) 个孤儿进程未列入清理范围：孤儿只支持逐条人工确认，"
                                        + "需要时加 --include-orphans）") + "\n")
                } else {
                    print("")
                }
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
                print("\n" + CLIColor.cyan("预计共终止 \(targets.count) 个进程，释放 \(MemoryFormat.text(totalMem)) 内存。"))
                noteSkipped(orphans: orphans, declined: declinedOrphans, force: isForce)
            }
            return
        }

        // 批量项也要先列出来再动手。此前 `clean` 一个键都不问就直接杀进程，
        // 而同仓的 `top [c]` 早就改成「先列目标再按 y」——两个入口一个问一个不问，
        // 用户记住的那个恰好是危险的那个。
        // `--force` 与 `--json` 都视为「我要非交互执行」的显式意图（后者天然给不出回答）
        let nonInteractive = isForce || isJson
        if !targets.isEmpty, !isDryRun, !nonInteractive {
            print("\n" + CLIColor.bold("即将终止以下 \(targets.count) 个进程"))
            print(CLIColor.dim("──────────────────────────────────────────"))
            for t in targets {
                print("  - [PID \(t.pid)] \(t.agentName) (\(t.anomalyType.rawValue)): \(t.reason)")
            }
            print(CLIColor.yellow("\n确认执行? [y/N] "), terminator: "")
            guard readLine()?.trimmingCharacters(in: .whitespaces).lowercased() == "y" else {
                print(CLIColor.dim("已取消，未终止任何进程。"))
                noteSkipped(orphans: orphans, declined: declinedOrphans, force: false)
                return
            }
        } else if !targets.isEmpty, !isDryRun, isJson {
            // 静默执行可以，但「为什么没问」要说得清：--json 按定义是机器在跑
            AppLog.warn("clean --json 非交互执行：跳过确认直接终止 \(targets.count) 个进程")
        }

        // 真实清理
        let result = cleaner.clean(anomalies: targets)
        let signaled = targets.filter { result.terminatedPids.contains($0.pid) }
        // 复核：`clean` 只知道「信号发出去了」。忽略 SIGTERM/SIGKILL 的死锁进程还在跑，
        // 当场报「已终止 / 已释放内存」等于让脚本与人都以为异常已清零。CLI 本来就是阻塞式
        // 工具，这里直接等同一个复核窗口（与灵动岛横幅同一口径，见 TerminationRecheck）。
        var verdict = CleanVerification(confirmedPids: [], stillRunningPids: [],
                                        reclaimedMemoryBytes: 0)
        if !signaled.isEmpty {
            Thread.sleep(forTimeInterval: TerminationRecheck.delay)
            verdict = cleaner.verifyTermination(of: signaled)
        }

        if isJson {
            let res = CLICleanResultDTO(
                success: !verdict.confirmedPids.isEmpty,
                killedPids: verdict.confirmedPids,
                freedMemoryBytes: verdict.reclaimedMemoryBytes,
                freedMemoryFormatted: verdict.reclaimedMemoryText,
                dryRun: false,
                unconfirmedPids: verdict.stillRunningPids
            )
            printJSON(res)
        } else {
            guard !verdict.confirmedPids.isEmpty else {
                // 一个都没杀掉（身份不符 / EPERM / 已退出）不能报「清理完成」
                let why = signaled.isEmpty
                    ? "\(targets.count) 个候选全部跳过或失败"
                    : "向 \(signaled.count) 个进程发出信号，复核后仍在运行"
                CLIExit.fail("未能终止任何目标进程（\(why)）")
            }
            print("\n" + CLIColor.bold("🧹 智能体异常进程清理复核"))
            print(CLIColor.dim("──────────────────────────────────────────"))
            print("  已确认退出: " + CLIColor.green("\(verdict.confirmedPids.count) 个"))
            if !verdict.stillRunningPids.isEmpty {
                print("  仍在运行: " + CLIColor.yellow(
                    "\(verdict.stillRunningPids.count) 个（PID \(verdict.stillRunningPids.map(String.init).joined(separator: ", "))"
                    + "，忽略终止信号，需手动处理）"))
            }
            print("  回收内存: " + CLIColor.green(verdict.reclaimedMemoryText)
                    + CLIColor.dim("（只统计确认退出的进程）"))
            noteSkipped(orphans: orphans, declined: declinedOrphans, force: isForce)
            print("")
        }
    }

    /// 把「这次没动的那些」说出来。清理类工具的谎报形态不是多杀，
    /// 而是杀完批量项就报完成，让人以为异常已经清零
    private static func noteSkipped(orphans: [AgentAnomaly], declined: [AgentAnomaly], force: Bool) {
        let declinedPIDs = Set(declined.map(\.pid))
        let untouched = orphans.filter { !declinedPIDs.contains($0.pid) }
        if !declined.isEmpty {
            print(CLIColor.dim("  你选择保留 \(declined.count) 个孤儿进程（未终止）。"))
        }
        if !untouched.isEmpty {
            print(CLIColor.dim("  另有 \(untouched.count) 个孤儿进程未处理：加 --include-orphans 逐条确认。"))
        }
        if force {
            print(CLIColor.dim("  （--force 只跳过批量项的确认；它不再包含孤儿进程。）"))
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
