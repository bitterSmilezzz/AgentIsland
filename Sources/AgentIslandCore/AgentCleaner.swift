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
}

public struct CleanResult: Equatable {
    public let terminatedCount: Int
    public let reclaimedMemoryBytes: UInt64

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
    /// - Parameter runningBundleIDs: 已由主线程抓取的 bundle 集合（NSWorkspace 不可跨线程）；
    ///   为 nil 时现场抓取（仅供主线程调用方）。
    public func scanAnomalies(profiles: [AgentProfile], hungAgentIDs: Set<String> = [],
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
                // 仅针对 CLI / 派生命令行工具（非 /Applications/ 下的标准 App 主进程）
                if entry.ppid == 1 && !isStandardAppBundle && !profile.processNames.isEmpty {
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
                        reason: "主控终端已关闭，已脱离原会话成为孤儿进程 (PPID=1)"
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

    /// 安全终止指定异常进程并返回清理结果
    @discardableResult
    public func clean(anomalies: [AgentAnomaly]) -> CleanResult {
        var terminated = 0
        var reclaimed: UInt64 = 0

        for a in anomalies {
            if ProcessTerminator.terminate(pid: a.pid) {
                terminated += 1
                reclaimed += a.memoryBytes
            }
        }

        return CleanResult(terminatedCount: terminated, reclaimedMemoryBytes: reclaimed)
    }
}
