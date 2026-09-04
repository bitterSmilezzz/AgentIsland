import AppKit
import Foundation

// MARK: - 进程快照
// 一次 `ps` 拿全部进程（pid / 完整路径 / CPU%），引擎对每个 profile 复用同一快照，
// 避免「每 agent 每次采样 fork 一次 ps」的浪费。

public struct ProcessSnapshot {
    public struct Entry: Equatable {
        public let pid: Int32
        public let path: String       // 完整可执行路径
        public let basename: String   // 路径最后一段（小写）
        public let cpuPercent: Double
    }

    public let entries: [Entry]

    public init(entries: [Entry]) {
        self.entries = entries
    }

    /// 系统目录前缀（排除误报）
    public static let systemPathPrefixes = [
        "/System/", "/usr/libexec/", "/usr/sbin/", "/usr/lib/", "/usr/bin/", "/bin/", "/sbin/",
    ]

    /// 已知非 AI-Agent 的同名进程黑名单
    public static let blacklist: Set<String> = [
        "cursoruiviewservice",   // 苹果 TextInputUIMacHelper 的 XPC 服务（常驻）
        "doubleagentd",
        "ssh-agent",
        "gpg-agent",
        "keychainagent",
    ]

    public static func isSystemPath(_ path: String) -> Bool {
        systemPathPrefixes.contains { path.hasPrefix($0) }
    }

    public static func isBlacklisted(_ basename: String) -> Bool {
        blacklist.contains(basename.lowercased())
    }
}

// MARK: - 进程提供协议

public protocol ProcessProviding {
    /// 当前全部进程快照（一次采样）
    func snapshot() -> ProcessSnapshot
    /// 当前运行中的 GUI App bundle identifiers（小写）
    func runningBundleIDs() -> Set<String>
}

// MARK: - 真实实现

public struct ProcessProvider: ProcessProviding {

    public init() {}

    public func snapshot() -> ProcessSnapshot {
        // 分两次 ps（macOS ps 组合字段会截断列宽到 15 字符）：
        // 1) command= 拿完整可执行路径  2) pcpu= 拿 CPU%，按 PID 合并
        let pathsOut = Shell.run("/bin/ps", args: ["-axo", "pid=,command="])
        var paths: [Int32: String] = [:]
        for line in pathsOut.split(separator: "\n") {
            let parts = line.split(separator: " ", omittingEmptySubsequences: true)
            guard parts.count >= 2, let pid = Int32(parts[0]) else { continue }
            // command 第一段 = 可执行文件路径（含空格路径被 ps 原样输出，第一段即路径）
            paths[pid] = String(parts[1])
        }

        let cpuOut = Shell.run("/bin/ps", args: ["-axo", "pid=,pcpu="])
        var entries: [ProcessSnapshot.Entry] = []
        for line in cpuOut.split(separator: "\n") {
            let parts = line.split(separator: " ", omittingEmptySubsequences: true)
            guard parts.count >= 2,
                  let pid = Int32(parts[0]),
                  let cpu = Double(parts[parts.count - 1]),
                  let path = paths[pid] else { continue }
            let base = (path as NSString).lastPathComponent.lowercased()
            entries.append(ProcessSnapshot.Entry(pid: pid, path: path, basename: base, cpuPercent: cpu))
        }
        return ProcessSnapshot(entries: entries)
    }

    public func runningBundleIDs() -> Set<String> {
        var ids = Set<String>()
        for app in NSWorkspace.shared.runningApplications {
            if let id = app.bundleIdentifier {
                ids.insert(id.lowercased())
            }
        }
        return ids
    }
}

// MARK: - 匹配引擎（对 profile 判定）

public struct ProcessMatcher {
    let snapshot: ProcessSnapshot
    let runningBundleIDs: Set<String>

    public init(snapshot: ProcessSnapshot, runningBundleIDs: Set<String>) {
        self.snapshot = snapshot
        self.runningBundleIDs = runningBundleIDs
    }

    /// 进程名前缀匹配（Q2：覆盖 Electron Helper / Helper (Renderer) 变体）
    /// profile 配 "dimagent"，则 dimagent、dimagent helper、dimagent-helper 命中；
    /// dimagentmalware 之类不会误命中（要求词边界：空格/连字符/精确相等）
    static func matchesProcessNames(_ names: Set<String>, basename: String) -> Bool {
        guard !basename.isEmpty else { return false }
        let b = basename.lowercased()
        return names.contains { name in
            b == name || b.hasPrefix(name + " ") || b.hasPrefix(name + "-")
        }
    }

    /// 路径子串匹配（Electron 应用主进程都叫 "Electron"，靠应用路径区分）
    /// profile 配 pathContains "trae"，则 /Applications/TRAE SOLO CN.app/.../Electron 命中
    static func matchesPathContains(_ pathContains: Set<String>, path: String) -> Bool {
        guard !path.isEmpty else { return false }
        let p = path.lowercased()
        return pathContains.contains { p.contains($0) }
    }

    /// 单个进程条目是否匹配 profile（进程名前缀 + 路径子串 + 非系统路径 + 非黑名单）
    func matchesProfile(_ profile: AgentProfile, entry: ProcessSnapshot.Entry) -> Bool {
        let names = Set(profile.processNames.map { $0.lowercased() })
        let nameHit = Self.matchesProcessNames(names, basename: entry.basename)
        let pathHit = Self.matchesPathContains(Set(profile.pathContains.map { $0.lowercased() }), path: entry.path)
        return (nameHit || pathHit)
            && !ProcessSnapshot.isSystemPath(entry.path)
            && !ProcessSnapshot.isBlacklisted(entry.basename)
    }

    /// 与 profile 相关的进程条目（bundle 或进程名命中，且非系统/黑名单）
    func matchingEntries(for profile: AgentProfile) -> [ProcessSnapshot.Entry] {
        // 1. bundle id 命中
        let bundleHit = profile.bundleIDs.contains { runningBundleIDs.contains($0.lowercased()) }
        if bundleHit {
            let byName = snapshot.entries.filter { matchesProfile(profile, entry: $0) }
            if !byName.isEmpty { return byName }
            // bundle 运行但进程名没匹配上（Electron helper 等）→ 标记运行（CPU 未知）
            return [ProcessSnapshot.Entry(pid: -1, path: "", basename: "", cpuPercent: 0)]
        }

        // 2. 进程名前缀命中（排除系统路径 + 黑名单）
        return snapshot.entries.filter { matchesProfile(profile, entry: $0) }
    }

    /// 进程是否在运行
    public func isRunning(_ profile: AgentProfile) -> Bool {
        let bundleHit = profile.bundleIDs.contains { runningBundleIDs.contains($0.lowercased()) }
        if bundleHit { return true }
        return snapshot.entries.contains { matchesProfile(profile, entry: $0) }
    }

    /// 相关进程 CPU 总和（双信号用）
    public func cpuPercent(_ profile: AgentProfile) -> Double {
        matchingEntries(for: profile).reduce(0) { $0 + $1.cpuPercent }
    }
}

// MARK: - 兼容封装（引擎用）

public struct ProcessMonitor {
    public let provider: ProcessProviding

    public init(provider: ProcessProviding = ProcessProvider()) {
        self.provider = provider
    }

    public init() {
        self.provider = ProcessProvider()
    }

    /// 快照 + bundle 一次性获取（引擎每采样调一次）
    public func matcher() -> ProcessMatcher {
        ProcessMatcher(snapshot: provider.snapshot(), runningBundleIDs: provider.runningBundleIDs())
    }
}

// MARK: - 工具

public enum Shell {
    @discardableResult
    public static func run(_ path: String, args: [String], timeout: TimeInterval = 3.0) -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = args
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        do {
            try process.run()
        } catch {
            return ""
        }
        let semaphore = DispatchSemaphore(value: 0)
        var output = ""
        pipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                pipe.fileHandleForReading.readabilityHandler = nil
                semaphore.signal()
            } else if let s = String(data: data, encoding: .utf8) {
                output += s
            }
        }
        DispatchQueue.global().asyncAfter(deadline: .now() + timeout) {
            if process.isRunning {
                process.terminate()
            }
            pipe.fileHandleForReading.readabilityHandler = nil
            semaphore.signal()
        }
        semaphore.wait()
        return output
    }
}

// MARK: - 测试用假实现

public struct FakeProcessProvider: ProcessProviding {
    public var processNames: Set<String>   // 小写 basename
    public var bundleIDs: Set<String>      // 小写
    public var cpuByProcess: [String: Double]  // 进程名 → CPU%

    public init(processNames: Set<String>, bundleIDs: Set<String>, cpu: Double = 0) {
        self.processNames = processNames
        self.bundleIDs = bundleIDs
        self.cpuByProcess = Dictionary(uniqueKeysWithValues: processNames.map { ($0, cpu) })
    }

    public func snapshot() -> ProcessSnapshot {
        let entries = processNames.map { name -> ProcessSnapshot.Entry in
            let path = "/Applications/FakeApp.app/Contents/MacOS/\(name)"
            return ProcessSnapshot.Entry(
                pid: 1,
                path: path,
                basename: name.lowercased(),
                cpuPercent: cpuByProcess[name] ?? 0
            )
        }
        return ProcessSnapshot(entries: entries)
    }

    public func runningBundleIDs() -> Set<String> { bundleIDs }
}
