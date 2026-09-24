import AppKit
import Foundation
import Darwin

// MARK: - 进程快照
// libproc 直接读进程表（proc_listpids + proc_pidpath + proc_pid_rusage），
// 免子进程/管道/超时，主线程耗时微秒级；CPU 用两次采样间差分得到真实窗口利用率。

public struct ProcessSnapshot: Sendable {
    public struct Entry: Equatable, Sendable {
        public let pid: Int32
        public let path: String       // 完整可执行路径（libproc 无空格截断问题）
        public let basename: String   // 路径最后一段（小写）
        public let cpuPercent: Double // 窗口利用率（差分），首拍为 0
        public let rssBytes: UInt64   // 物理内存占用（RSS），字节数
        public let ppid: Int32        // 父进程 PID
        /// `path` 的小写形式（构造时预计算）。
        /// 匹配器要对每个 profile 遍历整张进程表，若每次现算 `lowercased()`，
        /// 二十多个 profile × 全表会白白烧掉约 1.5ms/拍（实测）。构造期算一次即可。
        public let pathLower: String

        public init(pid: Int32, path: String, basename: String, cpuPercent: Double, rssBytes: UInt64 = 0, ppid: Int32 = 0, pathLower: String? = nil) {
            self.pid = pid
            self.path = path
            self.basename = basename
            self.cpuPercent = cpuPercent
            self.rssBytes = rssBytes
            self.ppid = ppid
            self.pathLower = pathLower ?? path.lowercased()
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
//
// 协议要求 Sendable：`sampleInBackground` 会把 provider 捕获进 @Sendable 闭包交给
// 后台队列。此前契约只写在注释里，类型系统无法约束（编译器报 non-Sendable capture）；
// 现在把契约落到协议上——实现者必须显式声明其线程安全性（真实实现有锁保护的
// CpuCache，测试替身由单线程驱动，均为 @unchecked Sendable）。

public protocol ProcessProviding: Sendable {
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
    /// 进程路径与元数据多级缓存（Unix 进程存活期路径与可执行元数据恒定）
    /// 仅对首次出现的 PID 调 proc_pidpath，避免每 2s 对 600+ 进程重复系统调用与堆分配
    private final class PathCache: @unchecked Sendable {
        struct Info {
            let path: String
            let basename: String
            let pathLower: String
        }
        var entries: [Int32: Info] = [:]

        func prune(keeping alivePids: Set<Int32>) {
            entries = entries.filter { alivePids.contains($0.key) }
        }
    }
    private let pathCache = PathCache()
    private let cache = CpuCache()
    /// 快照互斥：CPU 差分窗口（lastWall 读取 → 遍历内 update → setWall）必须原子，
    /// 否则并发快照（引擎采样 vs 工作台扫描 vs 终止前身份复核）互相消费差分窗口，
    /// 后到方的分母被先到方重置过，CPU% 单拍失真（骤降或钳满）
    private let snapshotLock = NSLock()

    public init() {}

    public func snapshot() -> ProcessSnapshot {
        snapshotLock.lock()
        defer { snapshotLock.unlock() }
        let now = Date().timeIntervalSince1970
        let wallDelta = now - cache.lastWallTime()   // 首拍可能为 0

        // 1) 全部 pid
        // 单次 sysctl(KERN_PROC_ALL) 同时取全部 pid 与 ppid（R25）：
        // 一次 syscall 拿全 kinfo_proc 数组，省 ~N 次 syscall/拍（N≈进程数 500+）。
        // 缓冲区按 size 预测分配；进程在两次调用间增加时重试一次（官方惯用法）
        var size = 0
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_ALL]
        var attempt = 0
        var procs: [kinfo_proc] = []
        // 成功那一次才置真：三次 ENOMEM 之后原先会带着**全零缓冲区**继续往下走，
        // 于是「拿不到进程表」被当成「机器上一个进程都没有」处理——所有档案判 offline、
        // 岛整个清空，而日志里一个字都没有。
        var tableRead = false
        while attempt < 3 {
            guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > 0 else {
                AppLog.warn("processTable: sysctl(KERN_PROC_ALL) 取长度失败 errno=\(errno)，"
                            + "本轮无进程表（岛会短暂清空，不是「什么都没在跑」）")
                return ProcessSnapshot(entries: [])
            }
            let count = size / MemoryLayout<kinfo_proc>.stride
            procs = [kinfo_proc](repeating: kinfo_proc(), count: count)
            var outSize = size
            guard sysctl(&mib, 3, &procs, &outSize, nil, 0) == 0 else {
                if errno == ENOMEM { attempt += 1; continue }   // 表在增长，重试更大缓冲
                AppLog.warn("processTable: sysctl(KERN_PROC_ALL) 取表失败 errno=\(errno)，"
                            + "重试 \(attempt) 次后本轮放弃")
                return ProcessSnapshot(entries: [])
            }
            size = outSize
            tableRead = true
            break
        }
        guard tableRead else {
            AppLog.warn("processTable: 连续 \(attempt) 次 ENOMEM，进程表本轮读不到；"
                        + "空表会让所有档案显示离线——那是「没看到」，不是「没在跑」")
            return ProcessSnapshot(entries: [])
        }

        var entries: [ProcessSnapshot.Entry] = []
        let pidCount = size / MemoryLayout<kinfo_proc>.stride
        var alivePids = Set<Int32>()
        alivePids.reserveCapacity(pidCount)
        entries.reserveCapacity(pidCount)
        let pathBufSize = 4096   // 足够容纳最长可执行路径（PROC_PIDPATHINFO_MAXSIZE ≈ 4KB）
        // 复用单次分配的 4KB 缓冲，彻底消除每拍循环内部 600+ 次 4KB 堆数组分配
        var pathBuf = [CChar](repeating: 0, count: pathBufSize)

        for i in 0..<pidCount {
            let pid = procs[i].kp_proc.p_pid
            guard pid > 0 else { continue }
            alivePids.insert(pid)

            // 2) 完整可执行路径（优先命中缓存，新 PID 才执行系统调用）
            let path: String
            let base: String
            let pathLower: String
            if let cached = pathCache.entries[pid] {
                path = cached.path
                base = cached.basename
                pathLower = cached.pathLower
            } else {
                let len = proc_pidpath(pid, &pathBuf, UInt32(pathBufSize))
                guard len > 0, Int(len) < pathBufSize else { continue }
                // 用 withUnsafeBytes 闭包保持缓冲区作用域（数组隐式指针转换会悬垂）
                path = pathBuf.withUnsafeBytes { raw -> String in
                    String(decoding: raw[..<Int(len)], as: UTF8.self)
                }
                guard !path.isEmpty else { continue }
                base = (path as NSString).lastPathComponent.lowercased()
                pathLower = path.lowercased()
                pathCache.entries[pid] = PathCache.Info(path: path, basename: base, pathLower: pathLower)
            }

            // 3) CPU 累计时间（rusage_info_v2）
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

            // 5) 父进程 PPID（kinfo_proc 已随单次 sysctl 一并给出，零额外 syscall）
            let ppid: Int32 = procs[i].kp_eproc.e_ppid

            entries.append(ProcessSnapshot.Entry(pid: pid, path: path, basename: base, cpuPercent: cpuPercent, rssBytes: rss, ppid: ppid, pathLower: pathLower))
        }

        cache.setWall(now)
        cache.prune(keeping: alivePids)
        pathCache.prune(keeping: alivePids)
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
    /// - Parameter basename: 必须已是小写。Entry 构造时统一 lowercased()（见 snapshot），
    ///   本函数位于「entries × profiles」每拍 ~8.7k 次调用的热路径，不再逐次分配小写副本
    ///
    /// 公开的原因：可信自报的 pid 绑定问的是同一个问题（「这个进程是不是它声称的那个
    /// Agent」）。那里再写一份 `hasPrefix` 就会有两套答案，而且朴素前缀会把 `claude`
    /// 认成 `claudex`——词边界这条规则正是为了不这么干才存在的。
    public static func matchesProcessNames(_ names: Set<String>, basename: String) -> Bool {
        guard !basename.isEmpty else { return false }
        let b = basename
        return names.contains { name in
            b == name || b.hasPrefix(name + " ") || b.hasPrefix(name + "-")
        }
    }

    /// 两个进程名是否「前缀族冲突」（R22）：相等或互为「name + 分隔符」前缀。
    /// 校验与匹配必须同口径——自定义名命中已知名前缀族（codex-helper vs codex）
    /// 与已知名命中自定义名前缀族（codex vs 自定义 codex）都会双份计数。
    /// 供 UI 的新增校验与匹配器共用（单一事实源）
    public static func hasPrefixFamilyConflict(_ a: String, _ b: String) -> Bool {
        let x = a.lowercased(), y = b.lowercased()
        guard !x.isEmpty, !y.isEmpty else { return false }
        if x == y { return true }
        return x.hasPrefix(y + " ") || x.hasPrefix(y + "-")
            || y.hasPrefix(x + " ") || y.hasPrefix(x + "-")
    }

    /// 路径约束匹配（Electron 应用主进程都叫 "Electron"，靠应用路径区分）
    ///
    /// 裸子串匹配会把**用户自己的程序**认成 Agent：profile 配 `trae` 时，
    /// 用户在 `~/code/trae-sandbox/` 里跑的 `npx electron .` 路径含 "trae" →
    /// 命中 TRAE 档案 → 出现在列表里并可被一键终止。这不是理论风险，
    /// 是 v0.0.97 复核记在案的已知缺陷。
    ///
    /// 改成按**路径段**判定，规则分两类：
    /// - 含 "/" 的（`/applications/qoder.app`、`.workbuddy/`）：仍是子串，但 needle 自带
    ///   目录锚，本来就不会误伤；
    /// - 不含 "/" 的（`trae`、`antigravity`）：必须整段相等、整段等于 `<needle>.app`，
    ///   或以 `<needle> ` 开头且以 `.app` 结尾（`TRAE SOLO CN.app` 这类带空格的
    ///   应用包名）。也就是说：**只有应用包或同名目录段能命中**，
    ///   `trae-sandbox`、`mytraetool` 这类都不算。
    static func matchesPathContains(_ pathContains: Set<String>, pathLower: String) -> Bool {
        guard !pathLower.isEmpty else { return false }
        // 段只在出现裸 needle 时才需要切，切一次的成本摊到「有裸 needle 的 profile」上
        var components: [String]? = nil
        return pathContains.contains { needle in
            if needle.isEmpty { return false }
            if needle.contains("/") { return pathLower.contains(needle) }
            if needle.contains(" ") || needle.contains(".") { return pathLower.contains(needle) }
            if components == nil {
                components = pathLower.split(separator: "/", omittingEmptySubsequences: true)
                    .map(String.init)
            }
            let parts = components ?? []
            return parts.contains(needle)
                || parts.contains(needle + ".app")
                || parts.contains { $0.hasPrefix(needle + " ") && $0.hasSuffix(".app") }
        }
    }

    /// 单个进程条目是否匹配 profile（进程名前缀 + 路径子串约束 + 路径排除 + 非系统路径 + 非黑名单）
    func matchesProfile(_ profile: AgentProfile, entry: ProcessSnapshot.Entry) -> Bool {
        let s = sets(for: profile)
        var nameHit = Self.matchesProcessNames(s.names, basename: entry.basename)
        // DeepSeek Harness 的 web 模式由外部 Node 启动，进程 basename 只有 node，
        // 可执行路径也不包含仓库名；用已有的无 fork sysctl 命令行探测补齐这一形态。
        // 仅对 node/dsh 候选调用（basename 已恒小写，无需再 lowercased），
        // 且走 10s TTL 缓存——否则 node 进程多的机器每拍要做十几次 KERN_PROCARGS2
        // sysctl（含环境块整块拷贝解析），命中后 inspectDSHAction 还会重复一次
        if !nameHit, profile.id == "dsh",
           ["node", "dsh"].contains(entry.basename), entry.pid > 1,
           let command = AgentActionInspector.cachedCommandLine(of: entry.pid)?.lowercased(),
           command.contains("deepseek-harness") || command.contains("/dsh ") {
            nameHit = true
        }
        guard nameHit else { return false }

        // 路径排除优先于包含：宿主应用内嵌的同名二进制不算本 Agent
        // （ChatGPT.app 内的 codex 属于 ChatGPT，不属于独立 Codex CLI）
        if !s.excludes.isEmpty, Self.matchesPathContains(s.excludes, pathLower: entry.pathLower) {
            return false
        }

        // 若配置了 pathContains（如 Electron 应用通用名 "Electron"），可执行路径必须同时命中子串约束
        let dshCommandHit = profile.id == "dsh" && nameHit
            && !Self.matchesProcessNames(s.names, basename: entry.basename)
        if !s.paths.isEmpty && !dshCommandHit {
            guard Self.matchesPathContains(s.paths, pathLower: entry.pathLower) else { return false }
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

}
// MARK: - 测试用假实现

public struct FakeProcessProvider: ProcessProviding {
    public var processNames: Set<String>   // 小写 basename
    public var bundleIDs: Set<String>      // 小写
    public var cpuByProcess: [String: Double]  // 进程名 → CPU%
    /// 完全自定义条目（测试身份复核等需要指定 pid/path 的场景）；nil 时按 processNames 生成
    public var entries: [ProcessSnapshot.Entry]?

    public init(processNames: Set<String>, bundleIDs: Set<String>, cpu: Double = 0,
                entries: [ProcessSnapshot.Entry]? = nil) {
        self.processNames = processNames
        self.bundleIDs = bundleIDs
        self.cpuByProcess = Dictionary(uniqueKeysWithValues: processNames.map { ($0, cpu) })
        self.entries = entries
    }

    public func snapshot() -> ProcessSnapshot {
        if let entries { return ProcessSnapshot(entries: entries) }
        let generated = processNames.map { name -> ProcessSnapshot.Entry in
            let path = "/Applications/FakeApp.app/Contents/MacOS/\(name)"
            return ProcessSnapshot.Entry(
                pid: 1,
                path: path,
                basename: name.lowercased(),
                cpuPercent: cpuByProcess[name] ?? 0,
                rssBytes: 104_857_600 // 默认 100MB 假数据
            )
        }
        return ProcessSnapshot(entries: generated)
    }

    public func runningBundleIDs() -> Set<String> { bundleIDs }
}

/// 可变进程集 fake（引用语义）：模拟进程在采样之间退出/启动
/// （struct 的 FakeProcessProvider 无法在引擎持有后改变进程集合）
///
/// `@unchecked Sendable`：属性可变，但仅由测试在单线程（主线程串行）驱动——
/// 测试先改属性、再触发采样，不存在并发读写。协议的 Sendable 约束要求显式声明。
public final class MutableProcessProvider: ProcessProviding, @unchecked Sendable {
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

/// 终止结果。比 Bool 多区分「身份复核未通过」：PID 被系统回收复用后，
/// 旧 pid 可能已属于无关进程，必须拒绝发送信号而不是笼统的「失败」。
public enum TerminationOutcome: Equatable {
    case signalSent        // 身份复核通过，已向目标进程树发出信号
    case identityMismatch  // PID 当前可执行文件与预期不符（疑似被回收复用），未发送任何信号
    case failed            // 进程已消失 / 无权限等，未成功发送信号
}

public enum ProcessTerminator {
    /// 进程是否仍在。清理结果**只能这样复核**：异常列表变空不等于进程已终止——
    /// `cleanAnomalies` 会 `resetTracking` 清掉 hung 证据，1.2s 后重扫时条目自然消失，
    /// 而忽略 SIGTERM 的死锁进程其实还在跑，用户却被告知「清理完成」。
    /// - Parameter expectedPath: 给出时同时校验当前可执行文件名，避免 pid 已被复用仍算存活。
    public static func isAlive(pid: Int32, expectedPath: String? = nil) -> Bool {
        guard pid > 1 else { return false }
        guard kill(pid, 0) == 0 else { return false }
        // 僵尸不算存活：进程已经退出、只是父进程还没 wait 回收，此时 `kill(pid,0)` 照样
        // 返回 0。把它算成「收到终止信号后仍在运行」是把复核做成了假警报——僵尸既不占
        // CPU 也不占内存，而用户看到的是一句杀不掉。
        // （实测入口：终止一个由测试进程派生、尚未回收的 sleep，探活会一直返回 true。）
        if isZombie(pid) { return false }
        guard let expectedPath, !expectedPath.isEmpty else { return true }
        guard let current = currentExecutablePath(of: pid) else {
            // 探到活但取不到路径（权限/正在退出）：保守当作存活，宁可报失败不可谎报成功
            return true
        }
        let want = (expectedPath as NSString).lastPathComponent.lowercased()
        let got = (current as NSString).lastPathComponent.lowercased()
        return want == got
    }

    /// 是否已经是僵尸（`p_stat == SZOMB`）。实测口径：僵尸上 `proc_pidinfo(PROC_PIDTBSDINFO)`
    /// 直接返回 0（拿不到结构体），所以只能走 `sysctl(KERN_PROC, KERN_PROC_PID)`——它对僵尸
    /// 照样返回条目，`p_stat` 为 5；活进程为 2（SRUN）。取不到信息时按「不是僵尸」处理：
    /// `kill(pid,0)` 已经判过存活，这里不该再引入第二次误判。
    static func isZombie(_ pid: Int32) -> Bool {
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        guard sysctl(&mib, 4, &info, &size, nil, 0) == 0, size > 0 else { return false }
        // SZOMB = 5（<sys/proc_info.h>）；Swift 侧没导出该宏
        return Int32(info.kp_proc.p_stat) == 5
    }

    /// 终止指定 PID 进程（包括其派生的子进程树），先尝试 GUI terminate / SIGTERM，超时未退出则强制 SIGKILL
    /// - Parameters:
    ///   - expectedPath: 扫描/事件时刻记录的目标可执行路径。提供时先用 `proc_pidpath`
    ///     复核当前路径的 basename 是否一致（brew 升级等路径整体变化但同名视为同一程序）；
    ///     不一致说明 PID 已被系统回收复用给其他程序，立即放弃——杀错整棵进程树的后果
    ///     远比「漏杀一个真异常」严重。nil 表示跳过复核（仅限调用方刚从最新快照取得 pid 的场景）。
    /// - Returns: 终止结果。此前恒返回 true，导致上层把「进程已退出/无权限」也计为清理成功并谎报回收内存。
    @discardableResult
    public static func terminate(pid: Int32, expectedPath: String? = nil, force: Bool = false) -> TerminationOutcome {
        guard pid > 1 else { return .failed }

        // 0. 身份复核（在任何信号/GUI terminate 之前）
        if let expectedPath, !expectedPath.isEmpty {
            guard let current = currentExecutablePath(of: pid) else { return .failed }
            let expectedName = (expectedPath as NSString).lastPathComponent.lowercased()
            let currentName = (current as NSString).lastPathComponent.lowercased()
            guard expectedName == currentName else { return .identityMismatch }
        }

        // 1. 如果是 GUI App，先尝试标准 terminate
        if let app = NSRunningApplication(processIdentifier: pid) {
            app.terminate()
        }

        // 2. 收集整棵进程树（包括所有子进程）。
        // 子进程不逐个复核 basename：helper/renderer 的可执行路径本就与根不同，
        // 逐个比对会把合法子进程漏掉或误拒；正确策略是「根进程强复核后杀整棵树」。
        // 残余 TOCTOU：树采集后某子进程退出、其 pid 在 300ms 补发窗口内被内核复用——
        // macOS 顺序分配机制下概率可忽略，接受该风险。
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
        return signalSent ? .signalSent : .failed
    }

    /// 读取进程当前可执行路径（不可读 = 已退出或无权限，一律视为无法确认身份）
    private static func currentExecutablePath(of pid: Int32) -> String? {
        var buffer = [CChar](repeating: 0, count: 4096)   // PROC_PIDPATHINFO_MAXSIZE ≈ 4KB
        let len = proc_pidpath(pid, &buffer, UInt32(buffer.count))
        guard len > 0, Int(len) < buffer.count else { return nil }
        return buffer.withUnsafeBytes { raw in
            String(decoding: raw[..<Int(len)], as: UTF8.self)
        }
    }

    /// 进程可执行文件名的小写 basename；取不到（已退出 / 无权限）返回 nil。
    /// 公开它是因为「可信自报的 pid 必须真的是它声称的那个 Agent」也要按档案匹配 pid，
    /// 而那必须与 `ProcessMatcher` 同一个口径、同一个 libproc 出口——
    /// 再抄一份 `proc_pidpath` 就会有两套「这个进程是谁」的答案。
    public static func executableName(of pid: Int32) -> String? {
        guard let path = currentExecutablePath(of: pid) else { return nil }
        return (path as NSString).lastPathComponent.lowercased()
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

// MARK: - 进程环境与工作区探测

public enum ProcessInspector {
    /// 获取指定进程当前的工作目录（CWD）。
    /// 使用 macOS 内核 proc_pidinfo(PROC_PIDVNODEPATHINFO) 直读，微秒级响应、零子进程开销。
    public static func currentWorkingDirectory(of pid: Int32) -> String? {
        guard pid > 1 else { return nil }
        var vpi = proc_vnodepathinfo()
        let size = MemoryLayout<proc_vnodepathinfo>.stride
        let ret = proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, &vpi, Int32(size))
        guard ret == size else { return nil }
        return withUnsafePointer(to: &vpi.pvi_cdir.vip_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { cStr in
                let path = String(cString: cStr).trimmingCharacters(in: .whitespacesAndNewlines)
                return path.isEmpty ? nil : path
            }
        }
    }
}
