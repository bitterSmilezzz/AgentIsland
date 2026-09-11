import Foundation

// MARK: - 文件活动监控
// 后台队列递归扫描 + 缓存：主线程只读缓存（O(1)），绝不阻塞 UI。
// 轮询目录树 mtime（有限深度递归），捕获深层会话文件写入。

public protocol FileActivityProviding {
    /// 返回 [目录: 最近一次写入时间]（缓存读取，必须快）
    func lastWriteDates(for dirs: [String]) -> [String: Date]

    /// 返回 [目录: 活跃会话数]（后台扫描时算好，主线程只读缓存）
    func activeSessionCounts(for dirs: [String]) -> [String: Int]

    /// 设置活跃会话判定窗口（引擎 config 同步）
    func setActiveSessionWindow(_ window: TimeInterval)

    /// 设置「工作中」判定窗口（引擎 config 同步）：决定快跳过兜底的重扫周期，
    /// 保证文件信号时效性与用户设定的窗口一致
    func setWorkingWindow(_ window: TimeInterval)

    /// 扫描紧迫度（引擎按 anyWorking 同步）：全闲置时放宽兜底重扫周期以省电
    func setScanUrgency(highFrequency: Bool)

    /// 注册监控目录（假实现为空操作）
    func watch(dirs: [String])

    /// 全量替换监控目录（启停集合变化时用；假实现为空操作）
    func replaceWatchedDirs(_ dirs: [String])

    /// 触发后台扫描（假实现为空操作）
    func scanAsync()
}

public extension FileActivityProviding {
    func watch(dirs: [String]) {}
    func replaceWatchedDirs(_ dirs: [String]) {}
    func scanAsync() {}
    func activeSessionCounts(for dirs: [String]) -> [String: Int] { [:] }
    func setActiveSessionWindow(_ window: TimeInterval) {}
    func setWorkingWindow(_ window: TimeInterval) {}
    func setScanUrgency(highFrequency: Bool) {}
}

/// 后台扫描 + 缓存实现：
/// - `watch` 注册目录，`scanAsync` 在后台队列执行全量递归扫描（合并并发，不堆积）
/// - `lastWriteDates` 主线程读缓存（微秒级）
public final class FileActivityMonitor: FileActivityProviding {
    /// 递归扫描最大深度（会话结构一般为 3-4 层）
    private let maxDepth: Int

    private var watchedDirs: Set<String> = []
    private var cache: [String: Date] = [:]
    /// 活跃会话数缓存（后台扫描一并算好，主线程只读）
    private var sessionCounts: [String: Int] = [:]
    /// 活跃会话判定窗口（引擎 config 同步，经 setActiveSessionWindow 加锁写入）
    private var activeSessionWindow: TimeInterval = 600
    private let lock = NSLock()
    private let scanQueue = DispatchQueue(label: "com.agentisland.filemonitor", qos: .utility)
    private var scanGeneration: UInt64 = 0
    private var isScanning = false
    private var lastScanAt = Date.distantPast
    /// 扫描最小间隔（引擎 working 时 2s 采样，扫描节流避免每拍全量扫）。
    /// 深层会话文件不会改变根目录 mtime，因此这里不能过大，否则 UI 会长时间滞后。
    private let scanMinInterval: TimeInterval
    /// 目录级快跳过缓存：目录自身 mtime + 最近写入时间（mtime 未变且 newest 仍活跃 → 复用，零枚举）
    private var lastRootDates: [String: Date] = [:]
    /// 每目录上次全量扫描时间（快跳过兜底：深层写入不改变根 mtime，
    /// 超过 forceRescanInterval 未全量扫 → 强制重扫，保证文件信号时效性）
    private var lastFullScans: [String: Date] = [:]
    /// 快跳过兜底周期：深层持续写入的文件信号最长延迟该周期即被发现（原 60s 过长）。
    /// 由引擎按 config.workingWindow 注入（见 setWorkingWindow）：写死 60s 会与
    /// 用户可调窗口脱钩——窗口调小则漏判 working，调大则把旧时间戳当新写入。
    private var forceRescanInterval: TimeInterval = 5
    /// 全闲置时的快跳过兜底周期（更省电；有 working 时用 forceRescanInterval）
    private let idleRescanInterval: TimeInterval = 30
    /// 当前是否处于高频扫描模式（引擎按 anyWorking 同步）
    private var isHighFrequencyScan = true

    public init(maxDepth: Int = 4, scanMinInterval: TimeInterval = 3.0) {
        self.maxDepth = maxDepth
        self.scanMinInterval = scanMinInterval
    }

    // MARK: 协议实现

    public func watch(dirs: [String]) {
        lock.lock()
        let previous = watchedDirs
        watchedDirs.formUnion(dirs)
        if watchedDirs != previous { invalidateScan() }
        lock.unlock()
    }

    public func replaceWatchedDirs(_ dirs: [String]) {
        lock.lock()
        let newSet = Set(dirs)
        if watchedDirs != newSet { invalidateScan() }
        watchedDirs = newSet
        // L6：清理不在新集合中的残留缓存（防止过期数据在集合变化后残留）
        cache = cache.filter { newSet.contains($0.key) }
        sessionCounts = sessionCounts.filter { newSet.contains($0.key) }
        lastRootDates = lastRootDates.filter { newSet.contains($0.key) }
        lastFullScans = lastFullScans.filter { newSet.contains($0.key) }
        lock.unlock()
    }

    public func lastWriteDates(for dirs: [String]) -> [String: Date] {
        lock.lock()
        defer { lock.unlock() }
        var result: [String: Date] = [:]
        for dir in dirs {
            if let date = cache[dir] { result[dir] = date }
        }
        return result
    }

    public func activeSessionCounts(for dirs: [String]) -> [String: Int] {
        lock.lock()
        defer { lock.unlock() }
        var result: [String: Int] = [:]
        for dir in dirs {
            if let count = sessionCounts[dir] { result[dir] = count }
        }
        return result
    }

    public func setActiveSessionWindow(_ window: TimeInterval) {
        lock.lock()
        if activeSessionWindow != window {
            activeSessionWindow = window
            sessionCounts.removeAll()
            invalidateScan()
        }
        lock.unlock()
    }

    public func setWorkingWindow(_ window: TimeInterval) {
        lock.lock()
        // 深层会话写入不会更新根目录 mtime，最多 5s 重扫一次即可保持状态及时。
        let interval = min(max(window, 3), 5)
        if forceRescanInterval != interval {
            forceRescanInterval = interval
            invalidateScan()
        }
        lock.unlock()
    }

    /// 扫描紧迫度（引擎在 `anyWorking` 变化时同步）。
    ///
    /// 为什么需要：快跳过兜底周期与采样节律存在拍频——实测采样 2s + 节流 3s 会让扫描
    /// 实际每 4s 一次，而 5s 兜底导致**每隔一次就强制全量**，快跳过形同虚设，
    /// 全量扫描占了 1.7% 平均 CPU 与每 6–8s 一次的 11–14% 尖峰。
    /// 有 Agent 在 working 时必须保持灵敏（文件信号消失要尽快回落 idle）；
    /// 全部闲置/离线时把兜底周期放宽，省掉绝大部分无谓枚举。
    ///
    /// 刻意**不**调用 `invalidateScan()`：紧迫度只影响后续扫描的兜底判断，
    /// 不需要立即重扫。若在此清空 `lastFullScans`，Agent 状态每次翻转都会强制
    /// 全量重扫一轮（实测使平均 CPU 由 2.5% 升到 6.7%），得不偿失。
    public func setScanUrgency(highFrequency: Bool) {
        lock.lock()
        isHighFrequencyScan = highFrequency
        lock.unlock()
    }

    public func scanAsync() {
        scanQueue.async { [weak self] in
            self?.runScan()
        }
    }

    /// 同步扫描（供 --probe / 测试使用）
    public func scanSync() {
        runScan()
    }

    // MARK: 内部

    /// 调用方持锁。旧扫描不得覆盖新配置，也不能推迟新目录的首扫。
    private func invalidateScan() {
        scanGeneration &+= 1
        lastFullScans.removeAll()
        lastScanAt = .distantPast
    }

    private func runScan() {
        lock.lock()
        guard !isScanning else {
            lock.unlock()
            return
        }
        // 节流：距上次扫描完成不足最小间隔则跳过。
        // 用「完成时间」而非开始时间（若单趟耗时 ≥ 间隔，按开始计时会连续重扫）
        guard Date().timeIntervalSince(lastScanAt) >= scanMinInterval else {
            lock.unlock()
            return
        }
        isScanning = true
        let dirs = Array(watchedDirs)
        let generation = scanGeneration
        let window = activeSessionWindow
        // 兜底周期按紧迫度取值：有 working 时保持灵敏，全闲置时放宽
        let rescanInterval = isHighFrequencyScan ? forceRescanInterval : idleRescanInterval
        lock.unlock()

        // 单趟扫描：每目录一次遍历，同时产出最近写入时间 + 活跃会话数
        // 快跳过：根目录 mtime 未变（无新顶层子项）且距上次全量扫描 < 兜底周期
        // → 复用缓存，零枚举。深层写入不改变根 mtime，故兜底强制重扫保证信号时效；
        //   空闲超周期后同样强制重扫，确认 idle 期间无新会话/写入
        //   （不再要求 newest 活跃——否则长空闲时「newest 活跃」恒不满足，每次扫描都全量枚举）。
        var fresh: [String: Date] = [:]
        var freshCounts: [String: Int] = [:]
        var freshRoots: [String: Date] = [:]
        var freshFullScans: [String: Date] = [:]
        let now = Date()
        for dir in dirs {
            let rootURL = URL(fileURLWithPath: dir)
            let rootDate = (try? rootURL.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            lock.lock()
            let cachedRoot = lastRootDates[dir]
            let cachedNewest = cache[dir]
            let cachedCount = sessionCounts[dir]
            let lastFull = lastFullScans[dir]
            lock.unlock()
            if let rootDate, let cachedRoot, rootDate == cachedRoot,
               let cachedNewest, let cachedCount,
               now.timeIntervalSince(lastFull ?? .distantPast) < rescanInterval {
                // 根 mtime 未变（无新顶层子项）+ 兜底周期内刚全量扫过：复用缓存，不枚举目录树
                fresh[dir] = cachedNewest
                freshCounts[dir] = cachedCount
                continue
            }
            let r = Self.scanTree(in: dir, maxDepth: maxDepth, window: window, now: now)
            fresh[dir] = r.newest
            freshCounts[dir] = r.activeSessions
            freshRoots[dir] = rootDate ?? Date.distantPast
            freshFullScans[dir] = now
        }

        lock.lock()
        guard generation == scanGeneration else {
            isScanning = false
            lock.unlock()
            return
        }
        lastRootDates.merge(freshRoots) { _, new in new }
        lastFullScans.merge(freshFullScans) { _, new in new }
        // 竞态防护：扫描期间 watchedDirs 可能被 replaceWatchedDirs 替换，
        // 迟到的扫描结果只写回仍在监控的目录，已停用目录的脏数据丢弃
        let current = watchedDirs
        // 成功扫描到的目录直接替换缓存：最近写入时间必须允许自然变旧，
        // 否则一次历史写入会永久把 Agent 判成 working。扫描失败/目录暂缺时
        // fresh 不含该目录，才保留旧值，避免短暂 I/O 抖动清空工作信号。
        for (dir, date) in fresh where current.contains(dir) {
            cache[dir] = date
        }
        sessionCounts = freshCounts.filter { current.contains($0.key) }
        isScanning = false
        lastScanAt = Date()   // 记录完成时间（节流基准）
        lock.unlock()
    }

    public struct DirScanResult {
        public let newest: Date?
        public let activeSessions: Int
    }

    /// 单趟全树扫描：最近写入时间 + window 内活跃的顶层子目录数（一次枚举完成）
    /// 兼容两种 level 语义（根子项为 0 或 1）：动态记录首个目录层级作为「顶层」
    public static func scanTree(in dir: String, maxDepth: Int = 4, window: TimeInterval, now: Date) -> DirScanResult {
        let fm = FileManager.default
        let url = URL(fileURLWithPath: dir)
        guard let values = try? url.resourceValues(forKeys: [.contentModificationDateKey]),
              let dirDate = values.contentModificationDate else {
            return DirScanResult(newest: nil, activeSessions: 0)
        }
        var newest = dirDate
        var activeTops = Set<String>()
        var topLevel: Int? = nil        // 首个目录条目的层级 = 顶层会话目录层级
        var currentTop: String? = nil   // 当前所属顶层目录路径

        let keys: [URLResourceKey] = [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]
        guard let en = fm.enumerator(at: url,
                                     includingPropertiesForKeys: keys,
                                     options: [.skipsHiddenFiles]) else {
            return DirScanResult(newest: newest, activeSessions: 0)
        }
        while let item = en.nextObject() as? URL {
            guard en.level <= maxDepth else {
                en.skipDescendants()
                continue
            }
            guard let v = try? item.resourceValues(forKeys: [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]) else {
                continue
            }
            let isDir = v.isDirectory == true
            let isLink = v.isSymbolicLink == true
            if isLink && isDir {
                en.skipDescendants()   // 符号链接目录跳过，防循环
                continue
            }
            // DimAgent 会在 sessions 下持续维护编辑历史和附件缓存；这些目录的 mtime
            // 会随后台同步变化，但不代表有 Agent 任务在执行。目录本身也必须跳过，
            // 否则即使过滤了文件，父目录 mtime 仍会把它们算进 newest。
            if isIgnoredActivityPath(item) {
                if isDir { en.skipDescendants() }
                continue
            }
            // 过滤无实质代码任务的纯心跳/锁/守护进程 PID 保活文件
            if !isDir && isHeartbeatOrNoiseFile(item) {
                continue
            }
            if let date = v.contentModificationDate, date > newest {
                newest = date
            }
            if topLevel == nil, isDir {
                topLevel = en.level
            }
            if let top = topLevel {
                if en.level == top && isDir {
                    currentTop = item.path
                    // 顶层会话目录自身在窗口内有写入 → 活跃
                    if let date = v.contentModificationDate, now.timeIntervalSince(date) <= window {
                        activeTops.insert(item.path)
                    }
                } else if en.level > top {
                    // 顶层目录内的任意写入 → 该会话活跃
                    if let date = v.contentModificationDate,
                       now.timeIntervalSince(date) <= window,
                       let ct = currentTop {
                        activeTops.insert(ct)
                    }
                }
            }
        }
        return DirScanResult(newest: newest, activeSessions: activeTops.count)
    }

    /// 目录树内最近写入时间（测试/Selftest 兼容入口，基于单趟 scanTree）
    public static func newestWrite(in dir: String, maxDepth: Int = 4) -> Date? {
        scanTree(in: dir, maxDepth: maxDepth, window: 0, now: Date()).newest
    }

    /// 排除纯心跳/锁文件（不代表智能体实质任务代码工作）
    public static func isHeartbeatOrNoiseFile(_ url: URL) -> Bool {
        let name = url.lastPathComponent
        // 1. 锁/套接字/临时文件
        if name.hasSuffix(".lock") || name.hasSuffix(".pid") || name.hasSuffix(".sock") || name.hasSuffix(".tmp") {
            return true
        }
        // 2. 纯 PID 形式的心跳 JSON（如 WorkBuddy 的 33485.json、33162.json）
        let stem = url.deletingPathExtension().lastPathComponent
        if url.pathExtension == "json" && Int(stem) != nil {
            return true
        }
        // 3. 显式心跳与遥测文件
        let lower = name.lowercased()
        if lower.contains("heartbeat") || lower.contains("crashpad") || lower.contains("telemetry") {
            return true
        }
        return false
    }

    /// 不代表任务执行的会话子树。路径组件匹配而不是字符串 contains，避免误伤项目名。
    ///
    /// 依赖安装树也在此列：`node_modules` / `site-packages` / `.venv` 等是包管理器产物，
    /// 数量可达数万且会被安装动作刷新，但「装依赖」不是 Agent 的任务写入。
    /// 实测忽略后全量扫描枚举量减少约 50%（9762 → 3419 项，294ms → 147ms）。
    private static let ignoredActivityPathComponents: Set<String> = [
        "file-history", "blobs",                                        // DimAgent 编辑历史/附件缓存
        "node_modules", "site-packages", ".venv", "venv", "__pycache__", // 依赖树
        ".git", "deriveddata", "caches",                                 // 仓库与构建缓存
    ]

    private static func isIgnoredActivityPath(_ url: URL) -> Bool {
        url.pathComponents.contains { ignoredActivityPathComponents.contains($0.lowercased()) }
    }
}

// MARK: - 测试用假实现（class 引用语义：外部推进时间，引擎内可见）

public final class FakeFileActivityProvider: FileActivityProviding {
    public var writes: [String: Date]

    public init(writes: [String: Date]) {
        self.writes = writes
    }

    public func lastWriteDates(for dirs: [String]) -> [String: Date] {
        dirs.reduce(into: [String: Date]()) { partial, dir in
            if let date = writes[dir] { partial[dir] = date }
        }
    }
}
