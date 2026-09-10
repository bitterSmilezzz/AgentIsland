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
        public let rssBytes: UInt64   // 物理内存占用（RSS），字节数
        public let ppid: Int32        // 父进程 PID

        public init(pid: Int32, path: String, basename: String, cpuPercent: Double, rssBytes: UInt64 = 0, ppid: Int32 = 0) {
            self.pid = pid
            self.path = path
            self.basename = basename
            self.cpuPercent = cpuPercent
            self.rssBytes = rssBytes
            self.ppid = ppid
        }
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
        /// 无条件按 alivePids 过滤（之前 2× 阈值在 pid 大量更替场景长期不触发）
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
        let pidCount = min(pids.count, Int(got) / MemoryLayout<pid_t>.size)
        var alivePids = Set<Int32>()
        alivePids.reserveCapacity(pidCount)
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
            let rss: UInt64 = rc == 0 ? rusage.ri_resident_size : 0

            // 4) 差分 CPU%（锁内更新缓存，线程安全）
            let cpuPercent = cpuTime >= 0 ? cache.update(pid: pid, cpuTime: cpuTime, wallDelta: wallDelta) : 0

            // 5) 父进程 PPID（用于孤儿进程检测）
            var bsd = proc_bsdinfo()
            let bsdRc = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &bsd, Int32(MemoryLayout<proc_bsdinfo>.size))
            let ppid: Int32 = bsdRc > 0 ? Int32(bsd.pbi_ppid) : 0

            let base = (path as NSString).lastPathComponent.lowercased()
            entries.append(ProcessSnapshot.Entry(pid: pid, path: path, basename: base, cpuPercent: cpuPercent, rssBytes: rss, ppid: ppid))
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
    /// 预计算缓存：profile.id → (小写 processNames, 小写 pathContains, 小写 pathExcludes)
    private let profileSets: [String: (names: Set<String>, paths: Set<String>, excludes: Set<String>)]

    public init(snapshot: ProcessSnapshot, runningBundleIDs: Set<String>, profiles: [AgentProfile] = []) {
        self.snapshot = snapshot
        self.runningBundleIDs = runningBundleIDs
        // 构造时一次性预计算（profile 数量个位数，成本可忽略），彻底避免每次匹配重建 Set
        var sets: [String: (names: Set<String>, paths: Set<String>, excludes: Set<String>)] = [:]
        for p in profiles {
            sets[p.id] = (
                names: Set(p.processNames.map { $0.lowercased() }),
                paths: Set(p.pathContains.map { $0.lowercased() }),
                excludes: Set(p.pathExcludes.map { $0.lowercased() })
            )
        }
        self.profileSets = sets
    }

    /// 获取 profile 的小写匹配集（预计算缓存，未命中则现场构建）
    private func sets(for profile: AgentProfile) -> (names: Set<String>, paths: Set<String>, excludes: Set<String>) {
        if let cached = profileSets[profile.id] { return cached }
        return (
            names: Set(profile.processNames.map { $0.lowercased() }),
            paths: Set(profile.pathContains.map { $0.lowercased() }),
            excludes: Set(profile.pathExcludes.map { $0.lowercased() })
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

    /// 单个进程条目是否匹配 profile（进程名前缀 + 路径子串约束 + 路径排除 + 非系统路径 + 非黑名单）
    func matchesProfile(_ profile: AgentProfile, entry: ProcessSnapshot.Entry) -> Bool {
        let s = sets(for: profile)
        var nameHit = Self.matchesProcessNames(s.names, basename: entry.basename)
        // DeepSeek Harness 的 web 模式由外部 Node 启动，进程 basename 只有 node，
        // 可执行路径也不包含仓库名；用已有的无 fork sysctl 命令行探测补齐这一形态。
        // 仅对 node/dsh 候选调用，避免每个采样周期遍历所有 PID。
        if !nameHit, profile.id == "dsh",
           ["node", "dsh"].contains(entry.basename.lowercased()), entry.pid > 1,
           let command = AgentActionInspector.commandLine(of: entry.pid)?.lowercased(),
           command.contains("deepseek-harness") || command.contains("/dsh ") {
            nameHit = true
        }
        guard nameHit else { return false }

        // 路径排除优先于包含：宿主应用内嵌的同名二进制不算本 Agent
        // （ChatGPT.app 内的 codex 属于 ChatGPT，不属于独立 Codex CLI）
        if !s.excludes.isEmpty, Self.matchesPathContains(s.excludes, path: entry.path) {
            return false
        }

        // 若配置了 pathContains（如 Electron 应用通用名 "Electron"），可执行路径必须同时命中子串约束
        let dshCommandHit = profile.id == "dsh" && nameHit
            && !Self.matchesProcessNames(s.names, basename: entry.basename)
        if !s.paths.isEmpty && !dshCommandHit {
            guard Self.matchesPathContains(s.paths, path: entry.path) else { return false }
        }

        return !ProcessSnapshot.isSystemPath(entry.path)
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

    /// 相关进程物理内存（RSS）总和（字节）
    public func memoryBytes(_ profile: AgentProfile) -> UInt64 {
        matchingEntries(for: profile).reduce(0) { $0 + $1.rssBytes }
    }

    /// 获取所有匹配任一已配置 Agent 的进程条目
    public func allMatchingEntries() -> [ProcessSnapshot.Entry] {
        var result: [ProcessSnapshot.Entry] = []
        var seenPids = Set<Int32>()
        for entry in snapshot.entries where entry.pid > 0 {
            for (_, sets) in profileSets {
                let nameHit = Self.matchesProcessNames(sets.names, basename: entry.basename)
                if nameHit {
                    let excluded = !sets.excludes.isEmpty
                        && Self.matchesPathContains(sets.excludes, path: entry.path)
                    if !excluded,
                       sets.paths.isEmpty || Self.matchesPathContains(sets.paths, path: entry.path) {
                        if !ProcessSnapshot.isSystemPath(entry.path) && !ProcessSnapshot.isBlacklisted(entry.basename) {
                            if !seenPids.contains(entry.pid) {
                                seenPids.insert(entry.pid)
                                result.append(entry)
                            }
                        }
                    }
                }
            }
        }
        return result
    }

    /// 获取底层 ProcessSnapshot
    public var rawSnapshot: ProcessSnapshot { snapshot }
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
                cpuPercent: cpuByProcess[name] ?? 0,
                rssBytes: 104_857_600 // 默认 100MB 假数据
            )
        }
        return ProcessSnapshot(entries: entries)
    }

    public func runningBundleIDs() -> Set<String> { bundleIDs }
}

/// 可变进程集 fake（引用语义）：模拟进程在采样之间退出/启动
/// （struct 的 FakeProcessProvider 无法在引擎持有后改变进程集合）
public final class MutableProcessProvider: ProcessProviding {
    public var names: Set<String>          // 小写 basename
    public var bundleIDs: Set<String>
    public var cpu: Double

    public init(names: Set<String>, bundleIDs: Set<String> = [], cpu: Double = 0) {
        self.names = names
        self.bundleIDs = bundleIDs
        self.cpu = cpu
    }

    public func snapshot() -> ProcessSnapshot {
        let entries = names.map { name -> ProcessSnapshot.Entry in
            ProcessSnapshot.Entry(
                pid: 1,
                path: "/Applications/FakeApp.app/Contents/MacOS/\(name)",
                basename: name.lowercased(),
                cpuPercent: cpu,
                rssBytes: 104_857_600
            )
        }
        return ProcessSnapshot(entries: entries)
    }

    public func runningBundleIDs() -> Set<String> { bundleIDs }
}

// MARK: - 智能体窗口/终端一键激活器

public enum AppActivator {
    /// 直达并置顶目标智能体对应的 App / 终端窗口
    @discardableResult
    public static func activate(pid: Int32?, bundleIDs: [String]) -> Bool {
        // 1. 优先尝试直接匹配 GUI Bundle
        for bid in bundleIDs {
            if let app = NSRunningApplication.runningApplications(withBundleIdentifier: bid).first {
                return activateApp(app)
            }
        }

        // 2. 若有 PID，尝试匹配本进程是否为普通 GUI App
        guard let pid = pid, pid > 1 else { return false }
        if let app = NSRunningApplication(processIdentifier: pid), app.activationPolicy == .regular {
            return activateApp(app)
        }

        // 3. 若为 CLI 进程（如 claude, codex, dim），向上追溯父进程找到承载的 GUI 终端（Terminal, iTerm2, VSCode, Cursor 等）
        // 用一次进程快照在内存里回溯（零 fork）；早期实现对每层父进程同步执行
        // `ps -o ppid=` 并 waitUntilExit，最多 10 次 fork，点击时主线程冻结数百毫秒。
        let snapshot = ProcessProvider().snapshot()
        var ppidByPid: [Int32: Int32] = [:]
        for e in snapshot.entries where e.ppid > 0 { ppidByPid[e.pid] = e.ppid }
        var current = pid
        for _ in 0..<10 {
            guard let next = ppidByPid[current], next > 1, next != current else { break }
            if let app = NSRunningApplication(processIdentifier: next), app.activationPolicy == .regular {
                return activateApp(app)
            }
            current = next
        }
        return false
    }

    private static func activateApp(_ app: NSRunningApplication) -> Bool {
        if #available(macOS 14.0, *) {
            app.activate()
            return true
        } else {
            return app.activate(options: [.activateIgnoringOtherApps])
        }
    }
}

// MARK: - 智能体进程安全熔断与终止

public enum ProcessTerminator {
    /// 终止指定 PID 进程（包括其派生的子进程树），先尝试 GUI terminate / SIGTERM，超时未退出则强制 SIGKILL
    /// - Returns: 是否至少成功向目标进程发出了信号（用 `kill(pid, 0)` 复核存活）。
    ///   此前恒返回 true，导致上层把「进程已退出/无权限」也计为清理成功并谎报回收内存。
    @discardableResult
    public static func terminate(pid: Int32, force: Bool = false) -> Bool {
        guard pid > 1 else { return false }

        // 1. 如果是 GUI App，先尝试标准 terminate
        if let app = NSRunningApplication(processIdentifier: pid) {
            app.terminate()
        }

        // 2. 收集整棵进程树（包括所有子进程）
        let pidsToKill = getProcessTree(rootPid: pid)
        let sig = force ? SIGKILL : SIGTERM

        var signalSent = false
        for p in pidsToKill.reversed() where kill(p, sig) == 0 {
            signalSent = true
        }

        if !force {
            // 给子进程/主进程 300ms 优雅退出机会，仍存活则发 SIGKILL
            DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.3) {
                for p in pidsToKill.reversed() {
                    if kill(p, 0) == 0 {
                        kill(p, SIGKILL)
                    }
                }
            }
        }
        return signalSent
    }

    /// 获取进程及其所有子进程 PID 列表。
    /// 用一次进程快照在内存里 BFS（零 fork）；早期实现对树中每个节点同步执行
    /// `pgrep -P` 并 `waitUntilExit()`，Electron 应用数十节点 = 数十次同步 fork，
    /// 调用点在主线程时冻结界面数百毫秒。
    public static func getProcessTree(rootPid: Int32) -> [Int32] {
        let snapshot = ProcessProvider().snapshot()
        return processTree(rootPid: rootPid, in: snapshot)
    }

    /// 纯函数版本：基于已有快照构建进程树（可测试，无系统调用）
    public static func processTree(rootPid: Int32, in snapshot: ProcessSnapshot) -> [Int32] {
        var childrenByParent: [Int32: [Int32]] = [:]
        for e in snapshot.entries where e.ppid > 0 {
            childrenByParent[e.ppid, default: []].append(e.pid)
        }
        var tree: [Int32] = [rootPid]
        var queue: [Int32] = [rootPid]
        var seen: Set<Int32> = [rootPid]
        while !queue.isEmpty {
            let current = queue.removeFirst()
            for child in childrenByParent[current] ?? [] where !seen.contains(child) {
                seen.insert(child)
                tree.append(child)
                queue.append(child)
            }
        }
        return tree
    }
}
