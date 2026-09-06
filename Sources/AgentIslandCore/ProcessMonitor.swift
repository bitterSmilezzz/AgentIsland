import AppKit
import Foundation
import Darwin

// MARK: - 进程快照
// libproc 直接读进程表（proc_listpids + proc_pidpath + proc_pid_rusage），
// 免子进程/管道/超时，主线程耗时微秒级；CPU 用两次采样间差分得到真实窗口利用率。

public struct ProcessSnapshot {
    public struct Entry: Equatable {
        public let pid: Int32
        public let path: String       // 完整可执行路径（libproc 无空格截断问题）
        public let basename: String   // 路径最后一段（小写）
        public let cpuPercent: Double // 窗口利用率（差分），首拍为 0
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
// 线程契约（调用方编排依据）：
//   snapshot()         —— 任意线程（libproc 无 UI 依赖，可后台执行）
//   runningBundleIDs() —— 必须主线程（内部 NSWorkspace，无线程安全保证）
// 消费方（ActivityEngine）据此做「主线程抓 bundle + 后台快照」两段式采样。

public protocol ProcessProviding {
    /// 当前全部进程快照（一次采样；CPU 为差分窗口值）
    func snapshot() -> ProcessSnapshot
    /// 当前运行中的 GUI App bundle identifiers（小写）
    func runningBundleIDs() -> Set<String>
}

// MARK: - 真实实现（libproc）
// @unchecked Sendable：内部 CpuCache 有锁保护，snapshot 可安全后台执行

public struct ProcessProvider: ProcessProviding, @unchecked Sendable {

    /// Mach tick → 秒换算（rusage_info_v2 的 ri_user_time/ri_system_time 单位是 tick：
    /// Apple Silicon 125/3 ns/tick，Intel 通常 1 ns/tick，必须经 timebase 换算）
    private static let tickToSeconds: Double = {
        var tb = mach_timebase_info_data_t()
        mach_timebase_info(&tb)
        return Double(tb.numer) / Double(tb.denom) / 1_000_000_000
    }()

    /// 上次采样的 CPU 累计时间（pid → 秒），用于差分
    /// 锁保护：matcher() 可能被主线程（同步采样/启动）与后台队列（定时采样）并发调用
    private final class CpuCache: @unchecked Sendable {
        private let lock = NSLock()
        private var last: [Int32: Double] = [:]   // pid → ru_utime+ru_stime 累计秒
        private var lastWall: TimeInterval = 0    // 上次采样墙钟

        /// 差分计算：返回本窗口 CPU%；首次见到返回 0
        func update(pid: Int32, cpuTime: Double, wallDelta: TimeInterval) -> Double {
            lock.lock()
            defer { lock.unlock() }
            guard let prev = last[pid], wallDelta > 0.01 else {
                last[pid] = cpuTime
                return 0
            }
            let delta = cpuTime - prev
            last[pid] = cpuTime
            return delta >= 0 ? min(delta / wallDelta * 100.0, 100.0) : 0
        }

        func lastWallTime() -> TimeInterval {
            lock.lock()
            defer { lock.unlock() }
            return lastWall
        }

        func setWall(_ t: TimeInterval) {
            lock.lock()
            lastWall = t
            lock.unlock()
        }

        /// 裁剪已退出进程：只保留本次采样仍存在的 pid，防止长期运行内存线性增长。
        /// 无条件按 alivePids 过滤（阿证低：之前 2× 阈值在 pid 大量更替场景长期不触发）
        func prune(keeping alivePids: Set<Int32>) {
            lock.lock()
            defer { lock.unlock() }
            last = last.filter { alivePids.contains($0.key) }
        }
    }
    private let cache = CpuCache()

    public init() {}

    public func snapshot() -> ProcessSnapshot {
        let now = Date().timeIntervalSince1970
        let wallDelta = now - cache.lastWallTime()   // 首拍可能为 0

        // 1) 全部 pid
        let count = proc_listpids(UInt32(PROC_ALL_PIDS), 0, nil, 0)
        guard count > 0 else { return ProcessSnapshot(entries: []) }
        var pids = [pid_t](repeating: 0, count: Int(count))
        let got = proc_listpids(UInt32(PROC_ALL_PIDS), 0, &pids, Int32(MemoryLayout<pid_t>.size * pids.count))
        guard got > 0 else { return ProcessSnapshot(entries: []) }

        var entries: [ProcessSnapshot.Entry] = []
        var alivePids = Set<Int32>()
        alivePids.reserveCapacity(Int(got))
        let pidCount = Int(got)
        let pathBufSize = 4096   // 足够容纳最长可执行路径（PROC_PIDPATHINFO_MAXSIZE ≈ 4KB）

        for i in 0..<pidCount {
            let pid = pids[i]
            guard pid > 0 else { continue }
            alivePids.insert(pid)

            // 2) 完整可执行路径
            var pathBuf = [CChar](repeating: 0, count: pathBufSize)
            let len = proc_pidpath(pid, &pathBuf, UInt32(pathBufSize))
            guard len > 0, Int(len) < pathBufSize else { continue }
            // 用 withUnsafeBytes 闭包保持缓冲区作用域（数组隐式指针转换会悬垂）
            let path = pathBuf.withUnsafeBytes { raw -> String in
                String(decoding: raw[..<Int(len)], as: UTF8.self)
            }
            guard !path.isEmpty else { continue }

            // 3) CPU 累计时间（rusage_info_v2）
            // 注意：proc_pid_rusage 把数据写入调用者缓冲区（rusage_info_t 只是类型伪装）；
            // ri_user_time/ri_system_time 是 Mach tick（Apple Silicon 125/3 ns/tick，
            // Intel 通常 1ns/tick），必须先经 mach_timebase_info 换算再使用。
            var rusage = rusage_info_v2()
            let rc = withUnsafeMutablePointer(to: &rusage) { ptr -> Int32 in
                let rebound = UnsafeMutableRawPointer(ptr).assumingMemoryBound(to: rusage_info_t?.self)
                return proc_pid_rusage(pid, RUSAGE_INFO_V2, rebound)
            }
            let cpuTime: Double = rc == 0
                ? (Double(rusage.ri_user_time) + Double(rusage.ri_system_time)) * Self.tickToSeconds
                : -1

            // 4) 差分 CPU%（锁内更新缓存，线程安全）
            let cpuPercent = cpuTime >= 0 ? cache.update(pid: pid, cpuTime: cpuTime, wallDelta: wallDelta) : 0

            let base = (path as NSString).lastPathComponent.lowercased()
            entries.append(ProcessSnapshot.Entry(pid: pid, path: path, basename: base, cpuPercent: cpuPercent))
        }

        cache.setWall(now)
        cache.prune(keeping: alivePids)
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
// @unchecked Sendable：纯值类型（struct + 不可变集合），跨线程传递安全

public struct ProcessMatcher: @unchecked Sendable {
    let snapshot: ProcessSnapshot
    let runningBundleIDs: Set<String>
    /// 预计算缓存：profile.id → (小写 processNames, 小写 pathContains)
    private let profileSets: [String: (names: Set<String>, paths: Set<String>)]

    public init(snapshot: ProcessSnapshot, runningBundleIDs: Set<String>, profiles: [AgentProfile] = []) {
        self.snapshot = snapshot
        self.runningBundleIDs = runningBundleIDs
        // 构造时一次性预计算（profile 数量个位数，成本可忽略），彻底避免每次匹配重建 Set
        var sets: [String: (names: Set<String>, paths: Set<String>)] = [:]
        for p in profiles {
            sets[p.id] = (
                names: Set(p.processNames.map { $0.lowercased() }),
                paths: Set(p.pathContains.map { $0.lowercased() })
            )
        }
        self.profileSets = sets
    }

    /// 获取 profile 的小写匹配集（预计算缓存，未命中则现场构建）
    private func sets(for profile: AgentProfile) -> (names: Set<String>, paths: Set<String>) {
        if let cached = profileSets[profile.id] { return cached }
        return (
            names: Set(profile.processNames.map { $0.lowercased() }),
            paths: Set(profile.pathContains.map { $0.lowercased() })
        )
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
        let s = sets(for: profile)
        let nameHit = Self.matchesProcessNames(s.names, basename: entry.basename)
        let pathHit = Self.matchesPathContains(s.paths, path: entry.path)
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
