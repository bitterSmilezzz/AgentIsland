import Foundation

// MARK: - 文件活动监控
// 后台队列递归扫描 + 缓存：主线程只读缓存（O(1)），绝不阻塞 UI。
// 轮询目录树 mtime（有限深度递归），捕获深层会话文件写入。

public protocol FileActivityProviding {
    /// 返回 [目录: 最近一次写入时间]（缓存读取，必须快）
    func lastWriteDates(for dirs: [String]) -> [String: Date]

    /// 返回 [目录: 活跃会话数]（后台扫描时算好，主线程只读缓存）
    func activeSessionCounts(for dirs: [String]) -> [String: Int]

    /// 返回 [目录: 最近活动文件]（后台扫描时定位，主线程只读缓存）。会话语义检查器
    /// 直接尾读这些文件，避免每个采样周期再次递归枚举整棵会话树。
    func latestActivityFiles(for dirs: [String]) -> [String: URL]

    /// 设置活跃会话判定窗口（引擎 config 同步）
    func setActiveSessionWindow(_ window: TimeInterval)

    /// 设置「工作中」判定窗口（引擎 config 同步）：决定快跳过兜底的重扫周期，
    /// 保证文件信号时效性与用户设定的窗口一致
    func setWorkingWindow(_ window: TimeInterval)

    /// 扫描紧迫度（引擎按 anyWorking 同步）：全闲置时放宽兜底重扫周期以省电
    func setScanUrgency(highFrequency: Bool)

    /// 当前有进程运行中的活动会话目录集合（离线 Agent 目录若根 mtime 未变，跳过深层递归枚举）
    func setRunningDirs(_ dirs: Set<String>)

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
    func latestActivityFiles(for dirs: [String]) -> [String: URL] { [:] }
    func setActiveSessionWindow(_ window: TimeInterval) {}
    func setWorkingWindow(_ window: TimeInterval) {}
    func setScanUrgency(highFrequency: Bool) {}
    func setRunningDirs(_ dirs: Set<String>) {}
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
    private var latestFiles: [String: URL] = [:]
    /// 活跃会话判定窗口（引擎 config 同步，经 setActiveSessionWindow 加锁写入）
    private var activeSessionWindow: TimeInterval = 600
    private let lock = NSLock()
    private let scanQueue = DispatchQueue(label: "com.agentisland.filemonitor", qos: .utility)
    private var scanGeneration: UInt64 = 0
    private var isScanning = false
    private var lastScanAt = Date.distantPast
    /// 目录缺失连续计数（R33/F3 终态判定；跨扫描持久，runScan 单飞互斥下读写）
    private var missingStreaks: [String: Int] = [:]
    /// 完成扫描所属代际（R33/F4）：invalidateScan（配置变更）后，
    /// 完成代际落后于当前代际 → 队列上的下一趟扫描绕过 3s 节流立即落地，
    /// 否则新目录的文件信号会空白到引擎下一拍（idle 节律最长 60s）
    private var lastScanGeneration: UInt64 = 0
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
    /// 当前有进程运行中的会话目录集合（引擎同步）：用于离线 Agent 目录智能跳过深度递归
    private var runningDirs: Set<String> = []

    public init(maxDepth: Int = 4, scanMinInterval: TimeInterval = 3.0) {
        self.maxDepth = maxDepth
        self.scanMinInterval = scanMinInterval
    }

    // MARK: 协议实现

    public func setRunningDirs(_ dirs: Set<String>) {
        lock.lock()
        let newlyOnline = dirs.subtracting(runningDirs)
        for dir in newlyOnline {
            lastFullScans[dir] = nil
        }
        runningDirs = dirs
        lock.unlock()
    }

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
        latestFiles = latestFiles.filter { newSet.contains($0.key) }
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

    public func latestActivityFiles(for dirs: [String]) -> [String: URL] {
        lock.lock()
        defer { lock.unlock() }
        var result: [String: URL] = [:]
        for dir in dirs {
            if let file = latestFiles[dir] { result[dir] = file }
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
        // 用「完成时间」而非开始时间（若单趟耗时 ≥ 间隔，按开始计时会连续重扫）。
        // 例外（R33/F4）：完成扫描的代际落后于当前代际 = 配置已变更（新目录首扫），
        // 节流不得吃掉这次扫描——此前两头都不补扫，新目录信号空白到引擎下一拍
        guard Date().timeIntervalSince(lastScanAt) >= scanMinInterval
              || lastScanGeneration == scanGeneration else {
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
        var freshFiles: [String: URL] = [:]
        var freshRoots: [String: Date] = [:]
        var freshFullScans: [String: Date] = [:]
        var clearedDirs: Set<String> = []   // 扫描成功但已无信号文件 → 活动清零（R33/F2）
        let now = Date()
        for dir in dirs {
            let rootURL = URL(fileURLWithPath: dir)
            let rootDate = (try? rootURL.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            lock.lock()
            let cachedRoot = lastRootDates[dir]
            let cachedNewest = cache[dir]
            let cachedCount = sessionCounts[dir]
            let cachedFile = latestFiles[dir]
            let lastFull = lastFullScans[dir]
            let isOnline = runningDirs.contains(dir)
            lock.unlock()
            if let rootDate, let cachedRoot, abs(rootDate.timeIntervalSince(cachedRoot)) < 0.001,
               let cachedNewest, let cachedCount {
                // 快跳过：
                // 1. 若该智能体离线且根目录 mtime 未变：不会产生深层写入，直接复用缓存（活跃数随时间窗口衰减）
                // 2. 若在兜底周期内刚全量扫过：复用缓存，不枚举目录树
                if !isOnline || now.timeIntervalSince(lastFull ?? .distantPast) < rescanInterval {
                    fresh[dir] = cachedNewest
                    freshCounts[dir] = (now.timeIntervalSince(cachedNewest) <= window) ? cachedCount : 0
                    if let cachedFile { freshFiles[dir] = cachedFile }
                    continue
                }
            }
            let r = Self.scanTree(in: dir, maxDepth: maxDepth, window: window, now: now)
            // 信号面终态语义（R33/F3）：
            // - 扫描成功且有信号文件 → 记入 fresh（写回替换，允许自然变旧）
            // - 扫描成功但无信号文件（产物清理/全被过滤）→ 活动清零
            // - 目录缺失（root stat 失败）**或整棵没看完**（枚举器失败）→ 保留旧值 + 连续缺失计数
            //   （连续 ≥3 趟仍缺失 → 终态清零；阈值吸收原子替换/迁移的瞬时空窗）
            if let d = r.newest {
                fresh[dir] = d
                if let file = r.newestFile { freshFiles[dir] = file }
                missingStreaks[dir] = 0
            } else if rootDate == nil || r.scanFailed {
                missingStreaks[dir, default: 0] += 1
            } else {
                clearedDirs.insert(dir)
                missingStreaks[dir] = 0
            }
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
        for (dir, file) in freshFiles where current.contains(dir) {
            latestFiles[dir] = file
        }
        // 活动清零（R33/F2/F3）：扫描成功但已无信号文件（产物清理）——
        // 此前走「保留旧值」分支，删除产物/目录永久删除后幽灵活动时间残留
        for dir in clearedDirs where current.contains(dir) {
            cache[dir] = nil
            latestFiles[dir] = nil
        }
        // 目录缺失终态（R33/F3）：连续 ≥3 趟仍缺失 → 清零缓存与快跳过键
        // （阈值吸收原子替换/迁移的瞬时空窗；目录重建后下一扫自动恢复）
        for (dir, streak) in missingStreaks where current.contains(dir) && streak >= 3 {
            cache[dir] = nil
            latestFiles[dir] = nil
            lastRootDates[dir] = nil
            lastFullScans[dir] = nil
            missingStreaks[dir] = 0
        }
        missingStreaks = missingStreaks.filter { current.contains($0.key) }
        sessionCounts = freshCounts.filter { current.contains($0.key) }
        isScanning = false
        lastScanAt = Date()   // 记录完成时间（节流基准）
        lastScanGeneration = generation   // 完成代际（R33/F4 节流豁免的判定基准）
        lock.unlock()
    }

    public struct DirScanResult {
        public let newest: Date?
        public let activeSessions: Int
        public let newestFile: URL?
        /// 这棵树**没能看完**（枚举器建不起来：权限、卷在扫描中途被弹掉…）。
        /// 与「看完了但一个信号文件都没有」必须分开：后者才该把活动清零，
        /// 前者按上面那份终态语义走「保留旧值 + 连续缺失计数」——否则一个正在写文件的
        /// Agent 会因为一次读不了目录而被判成待机，而岛上的措辞是「没在干活」。
        public let scanFailed: Bool

        public init(newest: Date?, activeSessions: Int, newestFile: URL? = nil,
                    scanFailed: Bool = false) {
            self.newest = newest
            self.activeSessions = activeSessions
            self.newestFile = newestFile
            self.scanFailed = scanFailed
        }
    }

    /// 单趟全树扫描：最近写入时间 + window 内活跃的顶层子目录数（一次枚举完成）
    /// 兼容两种 level 语义（根子项为 0 或 1）：动态记录首个目录层级作为「顶层」
    public static func scanTree(in dir: String, maxDepth: Int = 4, window: TimeInterval, now: Date) -> DirScanResult {
        let fm = FileManager.default
        let url = URL(fileURLWithPath: dir)
        guard let values = try? url.resourceValues(forKeys: [.contentModificationDateKey]),
              values.contentModificationDate != nil else {
            return DirScanResult(newest: nil, activeSessions: 0)
        }
        // newest 仅由「非忽略、非噪声的常规文件」聚合（R33/F2）：
        // 噪声文件（.lock/.pid/心跳）写入会刷新父目录 mtime——目录计入 newest 会把
        // 已过滤的噪声反向传播回工作信号（working 误报），且每次噪声写入都改变根
        // mtime 使快跳过永久失效。任务产物是文件；目录条目只参与 activeTops 判定
        var newest: Date? = nil
        var newestFile: URL? = nil
        var activeTops = Set<String>()
        var topLevel: Int? = nil        // 首个目录条目的层级 = 顶层会话目录层级
        var currentTop: String? = nil   // 当前所属顶层目录路径

        let keys: [URLResourceKey] = [.contentModificationDateKey, .isDirectoryKey, .isSymbolicLinkKey]
        // 枚举中途失败（根目录没有读权限、子目录被弹掉…）**不会**让 `enumerator(...)` 返回
        // nil：它照样给你一个枚举器，只是第一个对象都拿不到，错误被咽进返回值里。
        // 于是「读不了」在调用方看来与「这个目录里一个产物都没有」完全同形——正在写文件的
        // Agent 被判成待机。必须挂 errorHandler 才收得到。
        var enumerationFailed = false
        guard let en = fm.enumerator(at: url,
                                     includingPropertiesForKeys: keys,
                                     options: [],
                                     errorHandler: { _, _ in
                                         enumerationFailed = true
                                         return true      // 继续走完，能看到的还是要看
                                     }) else {
            return DirScanResult(newest: newest, activeSessions: 0, newestFile: newestFile,
                                 scanFailed: true)
        }
        while let item = en.nextObject() as? URL {
            guard en.level <= maxDepth else {
                en.skipDescendants()
                continue
            }
            let name = item.lastPathComponent
            // 允许以 .system_generated 命名的 Antigravity 核心会话日志子树；其余隐藏条目全部跳过
            if name.hasPrefix(".") && name != ".system_generated" {
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
            if !isDir, let date = v.contentModificationDate, newest == nil || date > newest! {
                newest = date
                newestFile = item
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
            // 深度上限真正生效的地方：`level == maxDepth` 的**目录**条目照常参与上面的
            // topLevel/activeTops 判定（其 mtime 是合法信号，不可跳），但它的子项落在
            // maxDepth+1，一律被开头的 `en.level <= maxDepth` 守卫丢弃——枚举器却仍然
            // 为每个这样的目录 opendir + readdir 一轮，纯支出、零收入。
            // 实测 `~/.gemini/antigravity/brain`：8,716 个 `.system_generated/steps/<n>`
            // 目录正卡在这一层（内层 `output.txt` 在 level 5，今天就读不到），
            // 趟数 19,490 → 10,756，单趟全量扫描 218ms → 74ms（同机同树，
            // newest / activeSessions / newestFile 逐项相等）。
            // 注意别把这行「优化」成按目录名剪掉 steps 子树：那会连 level-4 目录自身的
            // mtime 一起丢掉，activeSessions 不再等价（夹具测试已锁死这一点）。
            if isDir, en.level >= maxDepth {
                en.skipDescendants()
            }
        }
        return DirScanResult(newest: newest, activeSessions: activeTops.count,
                             newestFile: newestFile, scanFailed: enumerationFailed)
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
        // 直接按文件名后缀与纯数字判断，零 URL 分配与路径解析开销
        if name.hasSuffix(".json") {
            let stem = name.dropLast(5)
            if !stem.isEmpty && stem.allSatisfy({ $0.isNumber }) {
                return true
            }
        }
        // 3. 显式心跳与遥测文件
        let lower = name.lowercased()
        if lower.contains("heartbeat") || lower.contains("crashpad") || lower.contains("telemetry") {
            return true
        }
        // 3b. Sparkle 更新器（appcast 清单）与应用账号/状态文件：随应用启动与定时检查刷新
        if lower.contains("appcast") || lower == "oauth_credentials.json" || lower == "app_storage.json" {
            return true
        }
        // 4. SQLite 共享内存侧车（-shm）不算写入。
        // -shm 是连接共享的 mmap 索引页，任何进程「打开」数据库（即使只读）都会刷新
        // 它的 mtime：实测 Antigravity 空闲时 10 个会话库的 -shm 每 200s 被同步刷一次、
        // size 恒为 32768 字节，而主库 mtime 停在 22 小时前。引擎看到「刚刚
        // 写入」就把空闲应用判成 working，随后补发完成事件并响铃。
        // 任务数据落在主库或 -wal（写事务追加），过滤 -shm 不丢任务信号。
        if lower.hasSuffix(".db-shm") {
            return true
        }
        // 5. 浏览器内核（Chromium/Electron）用户数据根目录里的状态文件：
        // 空闲时也会被后台刷新（账号、偏好、缓存索引、崩溃与指标残留），
        // 与 Agent 任务无关。实测 Antigravity 仅被打开，20 分钟内有 36 次写入
        // 全部落在这类路径上。
        if Self.chromiumNoiseFiles.contains(lower) {
            return true
        }
        return Self.chromiumNoiseFilePrefixes.contains { lower.hasPrefix($0) }
    }

    /// Chromium/Electron 用户数据根目录内的状态文件名（小写精确匹配）。
    /// 均为浏览器内核自用状态：随应用启动、定时任务或崩溃上报刷新，不代表 Agent 任务。
    private static let chromiumNoiseFiles: Set<String> = [
        "network persistent state", "devtoolsactiveport", "transportsecurity",
        "dips", "dips-wal", "sharedstorage", "sharedstorage-wal",
        "trust tokens", "trust tokens-journal",
        "singletonlock", "singletoncookie", "singletonsocket",
        "preferences", "secure preferences", "local state",
        "cookies", "cookies-journal", "history", "visited links",
        "web data", "web data-journal", "login data", "login data-journal",
        "top sites", "favicons", "shortcuts", "networkactionpredictor",
        "first run", "last version", "variations",
        "quota manager", "quota manager-journal", "preloaded data",
    ]

    /// 带序号/后缀变体的状态文件族（`BrowserMetrics-spare.pma` 一类）
    private static let chromiumNoiseFilePrefixes: [String] = [
        "browsermetrics",
    ]

    /// 不代表任务执行的会话子树。路径组件匹配而不是字符串 contains，避免误伤项目名。
    ///
    /// 依赖安装树也在此列：`node_modules` / `site-packages` / `.venv` 等是包管理器产物，
    /// 数量可达数万且会被安装动作刷新，但「装依赖」不是 Agent 的任务写入。
    /// 实测忽略后全量扫描枚举量减少约 50%（9762 → 3419 项，294ms → 147ms）。
    ///
    /// 浏览器内核（Chromium/Electron）内部目录同样在此列（R37）：缓存、会话存储、
    /// 崩溃上报、指标等子树在应用空闲时持续被后台刷新——实测 Antigravity 仅被打开
    /// （无任何任务）时，20 分钟内 36 次写入全部落在 `Cache` / `Code Cache` /
    /// `Local Storage` / `Session Storage` / `GPUCache` / `DIPS` 一类路径上，
    /// 会把空闲应用顶成 working 并补发完成事件（响铃）。这些目录只装内核自用数据，
    /// 不含任务产物，剪掉后既消噪又省扫描量。
    private static let ignoredActivityPathComponents: Set<String> = [
        "file-history", "blobs",                                        // DimAgent 编辑历史/附件缓存
        "node_modules", "site-packages", ".venv", "venv", "__pycache__", // 依赖树
        ".git", "deriveddata", "caches", "cache",                        // 仓库与构建缓存
        // —— 浏览器内核用户数据（大小写不敏感，here 全部小写）——
        "code cache", "gpucache", "gpupersistentcache", "shadercache", "grshadercache",
        "dawncache", "dawngraphitecache", "dawnwebgpucache", "graphitedawncache",
        "session storage", "local storage", "indexeddb", "service worker", "cachestorage",
        "file system", "blob_storage", "shared dictionary", "crashpad",
        "component_crx_cache", "extensions_crx_cache", "browsermetrics",
        "optimizationhints", "sslerrorassistant", "safetytips", "subresource filter",
        "zxcvbndata", "meipreload", "widevinecdm", "nativemessaginghosts",
        "segmentation_platform", "clientcertificates", "crowd deny", "filetypepolicies",
        "firstpartysetspreloaded", "hyphen-data", "origin_trials", "pki metadata",
        "smartcardmanager", "speech recognition", "probabilisticrevealtokenregistry",
        "autofillstrikedatabase", "recoveryimproved",
    ]

    private static func isIgnoredActivityPath(_ url: URL) -> Bool {
        // 只比对条目自身的 basename（小写化一次）而非切分整条 pathComponents：
        // 被忽略子树在其目录条目处已被 skipDescendants 剪枝（见 scanTree），后代条目
        // 根本不会被枚举到，因此「祖先组件命中」与「自身命中」语义等价——每条目省掉
        // 一次数组分配与逐组件小写化（数万条目 × 全量扫描的可观占比）
        ignoredActivityPathComponents.contains(url.lastPathComponent.lowercased())
    }
}

// MARK: - 测试用假实现（class 引用语义：外部推进时间，引擎内可见）

public final class FakeFileActivityProvider: FileActivityProviding {
    public var writes: [String: Date]
    public var files: [String: URL]

    public init(writes: [String: Date], files: [String: URL] = [:]) {
        self.writes = writes
        self.files = files
    }

    public func lastWriteDates(for dirs: [String]) -> [String: Date] {
        dirs.reduce(into: [String: Date]()) { partial, dir in
            if let date = writes[dir] { partial[dir] = date }
        }
    }

    public func latestActivityFiles(for dirs: [String]) -> [String: URL] {
        dirs.reduce(into: [String: URL]()) { partial, dir in
            if let file = files[dir] { partial[dir] = file }
        }
    }
}
