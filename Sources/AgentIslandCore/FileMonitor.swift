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
    /// 活跃会话判定窗口（引擎 config 同步进来）
    public var activeSessionWindow: TimeInterval = 600
    private let lock = NSLock()
    private let scanQueue = DispatchQueue(label: "com.agentisland.filemonitor", qos: .utility)
    private var isScanning = false
    private var lastScanAt = Date.distantPast
    /// 扫描最小间隔（引擎 working 时 2s 采样，扫描节流避免每拍全量扫）
    private let scanMinInterval: TimeInterval = 3.0

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
        watchedDirs = Set(dirs)
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
        // 节流：距上次扫描不足最小间隔则跳过（引擎 2s 采样时扫描只按 3s 节奏跑）
        guard Date().timeIntervalSince(lastScanAt) >= scanMinInterval else {
            lock.unlock()
            return
        }
        isScanning = true
        lastScanAt = Date()
        let dirs = Array(watchedDirs)
        let window = activeSessionWindow
        lock.unlock()

        var fresh: [String: Date] = [:]
        var freshCounts: [String: Int] = [:]
        for dir in dirs {
            fresh[dir] = Self.newestWrite(in: dir, maxDepth: maxDepth)
            freshCounts[dir] = Self.activeSessionCount(in: [dir], window: window, now: Date())
        }

        lock.lock()
        cache = fresh
        sessionCounts = freshCounts
        isScanning = false
        lock.unlock()
    }

    /// 会话目录下的活跃会话数：一级子目录中「window 内存在文件写入」的数量
    /// （目录 mtime 只在增删条目时变，不能反映文件内容写入，必须递归查文件 mtime）
    /// 批量实现：用 enumerator 一次取一批，避免逐文件 syscall 风暴。
    public static func activeSessionCount(in dirs: [String], window: TimeInterval, now: Date) -> Int {
        var count = 0
        let keys: [URLResourceKey] = [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]
        for dir in dirs {
            let url = URL(fileURLWithPath: dir)
            guard let entries = try? FileManager.default.contentsOfDirectory(
                at: url, includingPropertiesForKeys: keys, options: [.skipsHiddenFiles]) else { continue }
            for entry in entries {
                guard let v = try? entry.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey]),
                      v.isDirectory == true, v.isSymbolicLink != true else { continue }
                if dirHasRecentWrite(entry.path, window: window, now: now) {
                    count += 1
                }
            }
        }
        return count
    }

    /// 目录树内是否存在 window 内的文件写入（有限深度，找到即停）
    static func dirHasRecentWrite(_ path: String, window: TimeInterval, now: Date, maxDepth: Int = 4) -> Bool {
        let fm = FileManager.default
        let url = URL(fileURLWithPath: path)
        let keys: [URLResourceKey] = [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]
        guard let en = fm.enumerator(at: url,
                                     includingPropertiesForKeys: keys,
                                     options: [.skipsHiddenFiles]) else { return false }
        while let item = en.nextObject() as? URL {
            guard en.level <= maxDepth else {
                en.skipDescendants()
                continue
            }
            guard let v = try? item.resourceValues(forKeys: [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]) else {
                continue
            }
            if let date = v.contentModificationDate,
               now.timeIntervalSince(date) <= window {
                return true
            }
            if v.isDirectory == true, v.isSymbolicLink == true {
                en.skipDescendants()   // 符号链接目录跳过，防循环
            }
        }
        return false
    }

    /// 目录树内最近写入时间：目录 mtime 与递归子项 mtime 的最大值
    /// 批量实现：FileManager.enumerator + 资源键（底层 getattrlistbulk 一次取一批，
    /// 远快于逐文件 attributesOfItem 的 lstat/getxattr 风暴）。
    public static func newestWrite(in dir: String, maxDepth: Int = 4) -> Date? {
        let fm = FileManager.default
        let url = URL(fileURLWithPath: dir)
        guard let values = try? url.resourceValues(forKeys: [.contentModificationDateKey]),
              let dirDate = values.contentModificationDate else {
            return nil
        }
        var newest = dirDate

        let keys: [URLResourceKey] = [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]
        guard let en = fm.enumerator(at: url,
                                     includingPropertiesForKeys: keys,
                                     options: [.skipsHiddenFiles]) else {
            return newest
        }
        while let item = en.nextObject() as? URL {
            // level：根目录子项为 1，递归深度限制
            guard en.level <= maxDepth else {
                en.skipDescendants()
                continue
            }
            guard let v = try? item.resourceValues(forKeys: [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]) else {
                continue
            }
            if let date = v.contentModificationDate, date > newest {
                newest = date
            }
            if v.isDirectory == true, v.isSymbolicLink == true {
                en.skipDescendants()   // 符号链接目录跳过，防循环
            }
        }
        return newest
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
