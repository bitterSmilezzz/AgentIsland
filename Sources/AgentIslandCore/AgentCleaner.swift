import Foundation
import Darwin
import AppKit

// MARK: - 智能体进程与资源维护服务 (AgentCleaner · v1.7.7)

public struct AgentAnomaly: Identifiable, Equatable {
    public let id: String
    public let pid: Int32
    public let ppid: Int32
    public let agentName: String
    public let profileId: String
    public let commandPath: String
    public let cpuPercent: Double
    public let memoryBytes: UInt64
    public let anomalyType: AnomalyType
    public let reason: String
    /// 是否可进入「一键批量清理」。
    /// 死锁/内存超限有充分证据（引擎聚合判定 / 超过硬上限），可批量；
    /// 孤儿 (ppid==1) 无法与 launchd/LaunchAgent 托管的常驻服务区分——
    /// 用户刻意后台化的智能体也是 ppid==1，批量误杀等于静默丢任务，
    /// 因此孤儿只允许逐条手动清理（宁可漏杀，符合本模块定位）。
    public let batchCleanable: Bool

    /// 列表身份：同一 pid 可能被两个 profile 同时匹配（如 GUI 主程序与其 CLI 子工具同名），
    /// 只用「类型-pid」会在 ForEach 中产生重复 id（SwiftUI 未定义行为、条目互相顶替），
    /// 因此身份里必须带上 profileId。
    public static func identifier(profileId: String, type: AnomalyType, pid: Int32) -> String {
        "\(profileId)-\(type.rawValue)-\(pid)"
    }

    public enum AnomalyType: String, Equatable {
        case orphan     // 孤儿进程：终端已断开/父进程转为 launchd (ppid=1) 且非 GUI 主程序
        case hung       // 疑似死锁/僵死卡顿：持续异常高 CPU 且无响应
        case overweight // 内存异常泄漏/超限（> 2.0GB）
    }

    public var memoryText: String {
        MemoryFormat.text(memoryBytes)
    }

    public var commandBasename: String {
        let name = URL(fileURLWithPath: commandPath).lastPathComponent
        return name.isEmpty ? commandPath : name
    }

    public init(id: String, pid: Int32, ppid: Int32, agentName: String, profileId: String,
                commandPath: String, cpuPercent: Double, memoryBytes: UInt64,
                anomalyType: AnomalyType, reason: String, batchCleanable: Bool = true) {
        self.id = id
        self.pid = pid
        self.ppid = ppid
        self.agentName = agentName
        self.profileId = profileId
        self.commandPath = commandPath
        self.cpuPercent = cpuPercent
        self.memoryBytes = memoryBytes
        self.anomalyType = anomalyType
        self.reason = reason
        self.batchCleanable = batchCleanable
    }
}

public struct CleanResult: Equatable {
    public let terminatedCount: Int
    public let reclaimedMemoryBytes: UInt64
    /// **真正发出过信号**的 pid。此前 `clean --json` 上报的是「所有候选」，
    /// 一个都没杀掉时 killedPids 依然非空、success 依然 true，CI 据此认为孤儿已回收
    public let terminatedPids: [Int32]

    public init(terminatedCount: Int, reclaimedMemoryBytes: UInt64, terminatedPids: [Int32] = []) {
        self.terminatedCount = terminatedCount
        self.reclaimedMemoryBytes = reclaimedMemoryBytes
        self.terminatedPids = terminatedPids
    }

    public var reclaimedMemoryText: String {
        MemoryFormat.text(reclaimedMemoryBytes)
    }
}

/// 复核后的结论：与 `CleanResult` 分开，因为「发了信号」和「进程真没了」是两件事。
public struct CleanVerification: Equatable {
    public let confirmedPids: [Int32]
    public let stillRunningPids: [Int32]
    /// 只统计确认退出的那些——把仍在运行的进程内存也算进「回收」，等于报一个必然偏大的数
    public let reclaimedMemoryBytes: UInt64

    public init(confirmedPids: [Int32], stillRunningPids: [Int32], reclaimedMemoryBytes: UInt64) {
        self.confirmedPids = confirmedPids
        self.stillRunningPids = stillRunningPids
        self.reclaimedMemoryBytes = reclaimedMemoryBytes
    }

    public var reclaimedMemoryText: String { MemoryFormat.text(reclaimedMemoryBytes) }
    public var allGone: Bool { stillRunningPids.isEmpty }
}

/// 终止后等多久再复核。SIGTERM 有 300ms 优雅期 + SIGKILL 兜底，0.8s 让两者都落地。
public enum TerminationRecheck {
    public static let delay: TimeInterval = 0.8
}

/// 异常扫描的两道前置闸门：死锁集与孤儿佐证集。
///
/// 这两份集合的算法原先只写在灵动岛工作台里，CLI 的 `check` / `clean` / `top` 三个入口
/// 一处都没传（走参数默认值 = 空集），结果是死锁分支永不成立、孤儿佐证永不生效，
/// 而 `check` 照旧印「未检测到任何死锁」。UI 与 CLI 现在共用这一份实现。
public struct AnomalyScanGates {
    /// 孤儿佐证窗口：10 分钟内仍有会话写入的 profile 算「还活着」。
    /// LaunchAgent 托管的常驻服务与「终端关闭的遗孤」从 ppid 上分不开（都是 1），
    /// 但前者必有活动佐证——孤儿判定宁可漏报，不可误杀。
    public static let orphanEvidenceWindow: TimeInterval = 600

    public let hungAgentIDs: Set<String>
    public let recentlyActiveProfileIDs: Set<String>
    /// 本轮快照**有没有资格**判死锁。
    ///
    /// `isHung` 是「连续高负载超过阈值（默认 5 分钟）」的时间性判定，只有持续采样的引擎
    /// 攒得出那段时长；一次性进程哪怕双采也只覆盖 1.5 秒，此时 `isHung` 全 false 的含义是
    /// 「没测」而不是「没有」。调用方必须据此改口，不许把「没测」播报成「未检测到死锁」。
    public let canJudgeHung: Bool

    /// - Parameter sustainedObservation: 快照来自持续采样的引擎（灵动岛、`top`）时传
    ///   `true`；一次性采样（`check` / `clean`）传 `false`。
    public init(snapshots: [AgentSnapshot], sustainedObservation: Bool) {
        self.init(hungAgentIDs: sustainedObservation
                    ? Set(snapshots.filter(\.isHung).map { $0.profile.id }) : [],
                  recentlyActiveProfileIDs: AnomalyScanGates.recentlyActiveProfileIDs(in: snapshots),
                  canJudgeHung: sustainedObservation)
    }

    public init(hungAgentIDs: Set<String>,
                recentlyActiveProfileIDs: Set<String>,
                canJudgeHung: Bool = true) {
        self.hungAgentIDs = hungAgentIDs
        self.recentlyActiveProfileIDs = recentlyActiveProfileIDs
        self.canJudgeHung = canJudgeHung
    }

    /// 佐证集单独拆出来：测试与调用方都要能在「本轮不判死锁」时照样算活动佐证。
    public static func recentlyActiveProfileIDs(in snapshots: [AgentSnapshot],
                                                window: TimeInterval = orphanEvidenceWindow) -> Set<String> {
        Set(snapshots.compactMap { snap -> String? in
            guard let ago = snap.lastActivityAgo, ago < window else { return nil }
            return snap.profile.id
        })
    }

    /// 「本轮没判死锁」这句话的唯一措辞——`check` 与 `clean` 必须同口径，
    /// 否则用户在两个入口看到两份真相。阈值取自传入的配置，不写死。
    public static func hungNotEvaluatedNote(config: EngineConfig) -> String {
        let minutes = max(1, Int(config.runawayDurationThreshold / 60))
        return "死锁/僵死：本次未评估——判定要求 CPU 连续 \(Int(config.runawayCpuThreshold))% 以上"
            + "达 \(minutes) 分钟，一次性扫描覆盖不到这段时长。"
            + "持续观测请用灵动岛工作台或 `agentisland top`。"
    }
}

public final class AgentCleaner {
    private let processMonitor: ProcessProviding

    /// 一次性扫描专用：先把 CPU 差分基线热起来再交给扫描。
    ///
    /// CPU% 是两次 `snapshot()` 之间的差分量，冷启动第一拍恒 0——而死锁规则要求
    /// `cpuPercent > 10`，用没预热的提供者去扫，等于把死锁行全部静默漏掉
    /// （灵动岛工作台为此预热两拍，见 `ToolboxView.scannerWarm`）。
    public static func warmedForOneShot(settleInterval: TimeInterval = 0.35) -> AgentCleaner {
        let provider = ProcessProvider()
        _ = provider.snapshot()
        Thread.sleep(forTimeInterval: settleInterval)
        return AgentCleaner(processMonitor: provider)
    }

    public init(processMonitor: ProcessProviding) {
        self.processMonitor = processMonitor
    }

    /// 扫描当前系统运行的所有 Agent 相关异常进程
    /// - Parameters:
    ///   - runningBundleIDs: 已由主线程抓取的 bundle 集合（NSWorkspace 不可跨线程）；
    ///     为 nil 时现场抓取（仅供主线程调用方）。
    ///   - gates: 死锁集与孤儿佐证集。**没有默认值是有意的**——这两道闸门缺任一个，
    ///     症状都不是崩溃而是「扫不出死锁 / 把活进程报成孤儿」。拿不准就用
    ///     `AnomalyScanGates(snapshots:sustainedObservation:)`，UI 与 CLI 同一条口径。
    public func scanAnomalies(profiles: [AgentProfile], gates: AnomalyScanGates,
                              runningBundleIDs: Set<String>? = nil) -> [AgentAnomaly] {
        let snapshot = processMonitor.snapshot()
        let bundleIDs = runningBundleIDs ?? processMonitor.runningBundleIDs()
        let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: bundleIDs, profiles: profiles)

        var anomalies: [AgentAnomaly] = []

        for profile in profiles {
            let entries = matcher.matchingEntries(for: profile)
            guard !entries.isEmpty else { continue }

            for entry in entries where entry.pid > 1 {
                // 标准 App 主进程（/Applications/xxx.app/Contents/MacOS/...）不作为可清理对象：
                // 它是用户正在使用的应用本体，杀它等于关掉用户的编辑器（可能丢未保存内容）。
                let isStandardAppBundle = entry.path.contains(".app/Contents/MacOS")

                // 1. 检查是否为已知死锁 / 假死 agent
                // 死锁是「整个 Agent 持续过载」的判定（engine 侧基于 profile 聚合 CPU），
                // 因此这里也必须排除 GUI 主进程：聚合高负载时单个子进程 >10% 属正常现象，
                // 按单条判定会把正常渲染进程也列成「疑似死锁」。
                if gates.hungAgentIDs.contains(profile.id), entry.cpuPercent > 10.0, !isStandardAppBundle {
                    anomalies.append(AgentAnomaly(
                        id: AgentAnomaly.identifier(profileId: profile.id, type: .hung, pid: entry.pid),
                        pid: entry.pid,
                        ppid: entry.ppid,
                        agentName: profile.name,
                        profileId: profile.id,
                        commandPath: entry.path,
                        cpuPercent: entry.cpuPercent,
                        memoryBytes: entry.rssBytes,
                        anomalyType: .hung,
                        reason: "持续过载超阈值，疑似处于死循环或线程死锁状态"
                    ))
                    continue
                }

                // 2. 检查孤儿进程 (PPID == 1)
                // 仅针对 CLI / 派生命令行工具（非 /Applications/ 下的标准 App 主进程）。
                // 佐证门槛：近期仍有会话写入的 profile 是活进程在干活（launchd 托管的
                // 常驻服务与「终端关闭的遗孤」无法从 ppid 区分，但前者必有活动佐证），直接跳过；
                // 其余孤儿仍列出（原因文案如实说明），但只允许逐条手动清理。
                if entry.ppid == 1 && !isStandardAppBundle && !profile.processNames.isEmpty {
                    guard !gates.recentlyActiveProfileIDs.contains(profile.id) else { continue }
                    anomalies.append(AgentAnomaly(
                        id: AgentAnomaly.identifier(profileId: profile.id, type: .orphan, pid: entry.pid),
                        pid: entry.pid,
                        ppid: entry.ppid,
                        agentName: profile.name,
                        profileId: profile.id,
                        commandPath: entry.path,
                        cpuPercent: entry.cpuPercent,
                        memoryBytes: entry.rssBytes,
                        anomalyType: .orphan,
                        reason: "主控终端已关闭，已脱离原会话成为孤儿进程 (PPID=1)；为防误杀常驻服务，仅支持逐条确认清理",
                        batchCleanable: false
                    ))
                    continue
                }

                // 3. 检查单进程物理内存超限 (> 2GB)
                // 同样排除 GUI 主进程：开着大项目的 Electron IDE 占 2.5GB 完全正常，
                // 无条件判为「内存泄露」会诱导用户清理掉正在使用的编辑器。
                if entry.rssBytes > 2_147_483_648, !isStandardAppBundle {
                    anomalies.append(AgentAnomaly(
                        id: AgentAnomaly.identifier(profileId: profile.id, type: .overweight, pid: entry.pid),
                        pid: entry.pid,
                        ppid: entry.ppid,
                        agentName: profile.name,
                        profileId: profile.id,
                        commandPath: entry.path,
                        cpuPercent: entry.cpuPercent,
                        memoryBytes: entry.rssBytes,
                        anomalyType: .overweight,
                        reason: "物理内存持续占用超过 2.0GB，疑似发生堆内存泄露或超长上下文堆积"
                    ))
                }
            }
        }

        return anomalies
    }

    /// 安全终止指定异常进程并返回清理结果。
    /// 终止前带 `commandPath` 做身份复核：工具箱列表可能已陈旧（扫描后进程退出、
    /// PID 被系统复用给无关程序），不带复核的 kill 会误杀。
    @discardableResult
    public func clean(anomalies: [AgentAnomaly]) -> CleanResult {
        var killed: [Int32] = []
        var reclaimed: UInt64 = 0

        for a in anomalies where a.pid > 1 {
            if ProcessTerminator.terminate(pid: a.pid, expectedPath: a.commandPath) == .signalSent {
                killed.append(a.pid)
                reclaimed += a.memoryBytes
            }
        }

        return CleanResult(terminatedCount: killed.count,
                           reclaimedMemoryBytes: reclaimed, terminatedPids: killed)
    }

    /// 复核批量清理的结论。`probe` 是注入点：默认走真实探活 + 路径校验，
    /// 而「收到信号又杀不掉」那一支用真进程造不出来（SIGKILL 兜底一定带走），
    /// 所以调用方（引擎/CLI）必须能替换它。
    public func verifyTermination(of signaled: [AgentAnomaly],
                                  probe: (Int32, String) -> Bool = { pid, path in
                                      ProcessTerminator.isAlive(pid: pid, expectedPath: path)
                                  }) -> CleanVerification {
        var gone: [Int32] = []
        var stuck: [Int32] = []
        var reclaimed: UInt64 = 0
        for a in signaled where a.pid > 1 {
            if probe(a.pid, a.commandPath) {
                stuck.append(a.pid)
            } else {
                gone.append(a.pid)
                reclaimed += a.memoryBytes
            }
        }
        return CleanVerification(confirmedPids: gone, stillRunningPids: stuck,
                                 reclaimedMemoryBytes: reclaimed)
    }
}
