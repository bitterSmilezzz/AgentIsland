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
    private var isScanning = false
    private var lastScanAt = Date.distantPast
    /// 扫描最小间隔（引擎 working 时 2s 采样，扫描节流避免每拍全量扫）
    /// 实测单趟全量递归 2.9GB 会话树耗时 0.23-0.30s，3s 间隔 ≈ 持续 10% CPU；
    /// 提到 15s + 快跳过（见 runScan）后工作态开销降到 ~2%。
    private let scanMinInterval: TimeInterval = 15.0
    /// 目录级快跳过缓存：目录自身 mtime + 最近写入时间（mtime 未变且 newest 仍活跃 → 复用，零枚举）
    private var lastRootDates: [String: Date] = [:]
    /// 每目录上次全量扫描时间（快跳过兜底：深层写入不改变根 mtime，
    /// 超过 forceRescanInterval 未全量扫 → 强制重扫，保证文件信号时效性）
    private var lastFullScans: [String: Date] = [:]
    /// 快跳过兜底周期：与引擎 workingWindow(60s) 同量级，
    /// 深层持续写入的文件信号最长延迟该周期即被发现（阿证实测原 600s 过长）
    private let forceRescanInterval: TimeInterval = 60

    public init(maxDepth: Int = 4) {
        self.maxDepth = maxDepth
    }

    // MARK: 协议实现

    public func watch(dirs: [String]) {
        lock.lock()
        for dir in dirs { watchedDirs.insert(dir) }
        lock.unlock()
    }

    public func replaceWatchedDirs(_ dirs: [String]) {
        lock.lock()
        let newSet = Set(dirs)
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
        activeSessionWindow = window
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

    private func runScan() {
        lock.lock()
        guard !isScanning else {
            lock.unlock()
            return
        }
        // 节流：距上次扫描完成不足最小间隔则跳过。
        // 用「完成时间」而非开始时间（阿剩低3：若单趟耗时 ≥ 间隔，按开始计时会连续重扫）
        guard Date().timeIntervalSince(lastScanAt) >= scanMinInterval else {
            lock.unlock()
            return
        }
        isScanning = true
        let dirs = Array(watchedDirs)
        let window = activeSessionWindow
        lock.unlock()

        // 单趟扫描：每目录一次遍历，同时产出最近写入时间 + 活跃会话数（M1 修复）
        // 快跳过：根目录 mtime 未变（无新顶层子项）且距上次全量扫描 < forceRescanInterval（60s）
        // → 复用缓存，零枚举。深层写入不改变根 mtime，故 60s 兜底强制重扫保证信号时效
        //   （工作态信号最长延迟 60s 被发现）；空闲超 60s 后同样强制重扫，确认 idle 期间
        //   无新会话/写入（不再要求 newest 活跃——否则长空闲时「newest 活跃」恒不满足，
        //   每次扫描都全量枚举，阿证中1）。
        var fresh: [String: Date] = [:]
        var freshCounts: [String: Int] = [:]
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
               now.timeIntervalSince(lastFull ?? .distantPast) < forceRescanInterval {
                // 根 mtime 未变（无新顶层子项）+ 60s 内刚全量扫过：复用缓存，不枚举目录树
                fresh[dir] = cachedNewest
                freshCounts[dir] = cachedCount
                continue
            }
            let r = Self.scanTree(in: dir, maxDepth: maxDepth, window: window, now: now)
            fresh[dir] = r.newest
            freshCounts[dir] = r.activeSessions
            lock.lock()
            lastRootDates[dir] = rootDate ?? Date.distantPast
            lastFullScans[dir] = now
            lock.unlock()
        }

        lock.lock()
        // 竞态防护（阿证低3）：扫描期间 watchedDirs 可能被 replaceWatchedDirs 替换，
        // 迟到的扫描结果只写回仍在监控的目录，已停用目录的脏数据丢弃
        let current = watchedDirs
        cache = fresh.filter { current.contains($0.key) }
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
