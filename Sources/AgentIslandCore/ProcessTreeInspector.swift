import Foundation

// MARK: - 进程树节点与分析报告 (v0.0.74)

public struct ProcessTreeNode: Identifiable, Equatable, Sendable {
    public var id: Int32 { pid }
    public let pid: Int32
    public let ppid: Int32
    public let name: String
    public let path: String
    public let cpuPercent: Double
    public let memoryBytes: UInt64
    public var children: [ProcessTreeNode]

    public var memoryText: String {
        MemoryFormat.text(memoryBytes)
    }

    public var cpuText: String {
        if cpuPercent <= 0.05 { return "0%" }
        return String(format: "%.1f%%", cpuPercent)
    }

    public init(
        pid: Int32,
        ppid: Int32,
        name: String,
        path: String,
        cpuPercent: Double,
        memoryBytes: UInt64,
        children: [ProcessTreeNode] = []
    ) {
        self.pid = pid
        self.ppid = ppid
        self.name = name
        self.path = path
        self.cpuPercent = cpuPercent
        self.memoryBytes = memoryBytes
        self.children = children
    }
}

public struct ProcessTreeReport: Equatable, Sendable {
    public let rootPid: Int32
    public let totalSubprocessCpu: Double
    public let totalSubprocessMemory: UInt64
    public let subprocessCount: Int
    public let nodes: [ProcessTreeNode]

    public var totalSubprocessMemoryText: String {
        MemoryFormat.text(totalSubprocessMemory)
    }

    public var totalSubprocessCpuText: String {
        if totalSubprocessCpu <= 0.05 { return "0%" }
        return String(format: "%.1f%%", totalSubprocessCpu)
    }

    public init(
        rootPid: Int32,
        totalSubprocessCpu: Double,
        totalSubprocessMemory: UInt64,
        subprocessCount: Int,
        nodes: [ProcessTreeNode]
    ) {
        self.rootPid = rootPid
        self.totalSubprocessCpu = totalSubprocessCpu
        self.totalSubprocessMemory = totalSubprocessMemory
        self.subprocessCount = subprocessCount
        self.nodes = nodes
    }
}

public enum ProcessTreeInspector {

    /// 纯函数：给定 Agent 根进程 PID 及当前系统的进程表快照，递归构建派生工具与子进程树
    public static func buildTree(for rootPid: Int32, from entries: [ProcessSnapshot.Entry]) -> ProcessTreeReport {
        guard rootPid > 0 else {
            return ProcessTreeReport(
                rootPid: rootPid,
                totalSubprocessCpu: 0,
                totalSubprocessMemory: 0,
                subprocessCount: 0,
                nodes: []
            )
        }

        // 按父进程 PID 分组建立索引
        var childrenByParent: [Int32: [ProcessSnapshot.Entry]] = [:]
        for entry in entries {
            childrenByParent[entry.ppid, default: []].append(entry)
        }

        var totalCpu: Double = 0
        var totalMem: UInt64 = 0
        var count: Int = 0

        func collectChildren(of parentId: Int32, visited: inout Set<Int32>) -> [ProcessTreeNode] {
            guard let directChildren = childrenByParent[parentId] else { return [] }
            var result: [ProcessTreeNode] = []

            for child in directChildren {
                // 防环检测（极罕见情况 PID 环）
                guard !visited.contains(child.pid) else { continue }
                visited.insert(child.pid)

                totalCpu += child.cpuPercent
                totalMem += child.rssBytes
                count += 1

                let subChildren = collectChildren(of: child.pid, visited: &visited)
                let node = ProcessTreeNode(
                    pid: child.pid,
                    ppid: child.ppid,
                    name: child.basename.isEmpty ? "unknown" : child.basename,
                    path: child.path,
                    cpuPercent: child.cpuPercent,
                    memoryBytes: child.rssBytes,
                    children: subChildren
                )
                result.append(node)
            }
            return result
        }

        var visitedPids: Set<Int32> = [rootPid]
        let rootChildren = collectChildren(of: rootPid, visited: &visitedPids)

        return ProcessTreeReport(
            rootPid: rootPid,
            totalSubprocessCpu: totalCpu,
            totalSubprocessMemory: totalMem,
            subprocessCount: count,
            nodes: rootChildren
        )
    }
}
