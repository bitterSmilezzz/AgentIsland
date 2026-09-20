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

public final class AgentCleaner {
    private let processMonitor: ProcessProviding

    public init(processMonitor: ProcessProviding) {
        self.processMonitor = processMonitor
    }

    /// 扫描当前系统运行的所有 Agent 相关异常进程
    /// - Parameters:
    ///   - runningBundleIDs: 已由主线程抓取的 bundle 集合（NSWorkspace 不可跨线程）；
    ///     为 nil 时现场抓取（仅供主线程调用方）。
    ///   - recentlyActiveProfileIDs: 会话目录近期有写入的 profile 集合（由调用方从
    ///     引擎快照的 lastActivityAgo 汇总）。孤儿判定需要它做佐证：ppid==1 但仍在
    ///     产出会话写入的 Agent 是活着的（很可能由 LaunchAgent 刻意托管），绝不能报成孤儿。
    public func scanAnomalies(profiles: [AgentProfile], hungAgentIDs: Set<String> = [],
                              runningBundleIDs: Set<String>? = nil,
                              recentlyActiveProfileIDs: Set<String> = []) -> [AgentAnomaly] {
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
                if hungAgentIDs.contains(profile.id), entry.cpuPercent > 10.0, !isStandardAppBundle {
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
                    guard !recentlyActiveProfileIDs.contains(profile.id) else { continue }
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
}
