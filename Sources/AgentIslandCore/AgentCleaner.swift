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

    public enum AnomalyType: String, Equatable {
        case orphan     // 孤儿进程：终端已断开/父进程转为 launchd (ppid=1) 且非 GUI 主程序
        case hung       // 疑似死锁/僵死卡顿：持续异常高 CPU 且无响应
        case overweight // 内存异常泄漏/超限（> 2.0GB）
    }

    public var memoryText: String {
        let mb = Double(memoryBytes) / (1024 * 1024)
        if mb >= 1024 {
            return String(format: "%.1fG", mb / 1024.0)
        } else {
            return "\(Int(mb))M"
        }
    }
}

public struct CleanResult: Equatable {
    public let terminatedCount: Int
    public let reclaimedMemoryBytes: UInt64

    public var reclaimedMemoryText: String {
        let mb = Double(reclaimedMemoryBytes) / (1024 * 1024)
        if mb >= 1024 {
            return String(format: "%.1fG", mb / 1024.0)
        } else {
            return "\(Int(mb))M"
        }
    }
}

public final class AgentCleaner {
    private let processMonitor: ProcessProviding

    public init(processMonitor: ProcessProviding) {
        self.processMonitor = processMonitor
    }

    /// 扫描当前系统运行的所有 Agent 相关异常进程
    public func scanAnomalies(profiles: [AgentProfile], hungAgentIDs: Set<String> = []) -> [AgentAnomaly] {
        let snapshot = processMonitor.snapshot()
        let runningBundleIDs = processMonitor.runningBundleIDs()
        let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: runningBundleIDs, profiles: profiles)

        var anomalies: [AgentAnomaly] = []

        for profile in profiles {
            let entries = matcher.matchingEntries(for: profile)
            guard !entries.isEmpty else { continue }

            for entry in entries where entry.pid > 1 {
                // 1. 检查是否为已知死锁 / 假死 agent
                if hungAgentIDs.contains(profile.id) && entry.cpuPercent > 10.0 {
                    anomalies.append(AgentAnomaly(
                        id: "hung-\(entry.pid)",
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
                let isStandardAppBundle = entry.path.contains(".app/Contents/MacOS")
                if entry.ppid == 1 && !isStandardAppBundle && !profile.processNames.isEmpty {
                    anomalies.append(AgentAnomaly(
                        id: "orphan-\(entry.pid)",
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
                if entry.rssBytes > 2_147_483_648 { // 2GB
                    anomalies.append(AgentAnomaly(
                        id: "overweight-\(entry.pid)",
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
