import Foundation
import Combine

// MARK: - 活动引擎
// 采样状态机（Q1 双信号）：
//   working = 进程在 且（workingWindow 内有文件写入 或 CPU > cpuThreshold）
//   idle    = 进程在 但两者皆不满足
//   offline = 进程不在
// 节电（Q11）：有 working 用 sampleInterval 采样；全闲置降频 idleSampleInterval。

@MainActor
public final class ActivityEngine: ObservableObject {

    @Published public private(set) var snapshots: [AgentSnapshot] = []
    @Published public private(set) var anyWorking = false
    @Published public private(set) var updatedAt = Date()

    public var config: EngineConfig {
        didSet { applyConfig() }
    }

    private var profiles: [AgentProfile]
    private let processMonitor: ProcessMonitor
    private let fileMonitor: FileActivityProviding
    public let tokenMonitor = TokenUsageMonitor()   // token 用量（60s 后台轮询，采样时取快照）
    private var lastWrites: [String: Date] = [:]   // dir -> 最近写入时间
    private var timer: Timer?
    private var installedCLIs: Set<String> = []
    private var installedBundles: Set<String> = []
    /// 会话数缓存（主线程全树扫描太贵，5s 才重算一次；离线 agent 直接记 0）
    private var sessionCountCache: [String: Int] = [:]
    private var sessionCountAt = Date.distantPast

    public init(profiles: [AgentProfile] = AgentRegistry.fullRegistry(),
         config: EngineConfig = EngineConfig(),
         processMonitor: ProcessMonitor = ProcessMonitor(),
         fileMonitor: FileActivityProviding = FileActivityMonitor()) {
        self.profiles = profiles
        self.config = config
        self.processMonitor = processMonitor
        self.fileMonitor = fileMonitor
        refreshInstalled()
        // 注册监控目录（后台扫描用）
        fileMonitor.watch(dirs: profiles.flatMap(\.sessionDirs))
    }

    public func start() {
        guard timer == nil else { return }
        // token 数据刷完后触发一次重采样（快照带上用量 + 卡片高度重算）
        tokenMonitor.onRefresh = { [weak self] in
            self?.sample()
        }
        tokenMonitor.start()   // token 用量轮询（60s，后台队列）
        sample()
        scheduleNext()
    }

    public func stop() {
        timer?.invalidate()
        timer = nil
        tokenMonitor.stop()
    }

    // MARK: - 动态配置（设置界面接线）

    /// 更新启停集合（enabledAgents）
    public func setEnabled(_ enabledIDs: Set<String>) {
        let all = profiles
        profiles = all.filter { enabledIDs.contains($0.id) }
        refreshWatchedDirs()
        sample()
    }

    /// 增加自定义 profile（设置界面新增）
    public func addCustomProfile(_ profile: AgentProfile) {
        guard !profiles.contains(where: { $0.id == profile.id }) else { return }
        profiles.append(profile)
        fileMonitor.watch(dirs: profile.sessionDirs)
        sample()
    }

    /// 移除自定义 profile
    public func removeCustomProfile(_ id: String) {
        profiles.removeAll { $0.id == id && $0.isCustom }
        refreshWatchedDirs()
        sample()
    }

    public var allProfiles: [AgentProfile] { profiles }

    private func applyConfig() {
        // 采样间隔变化 → 重启定时器
        if timer != nil {
            timer?.invalidate()
            timer = nil
            scheduleNext()
        }
    }

    private func refreshWatchedDirs() {
        fileMonitor.watch(dirs: profiles.flatMap(\.sessionDirs))
    }

    // MARK: - 安装检测（A4）

    private func refreshInstalled() {
        installedCLIs = AgentRegistry.installedCLIs()
        installedBundles = AgentRegistry.installedBundleIDs()
    }

    private func isInstalled(_ profile: AgentProfile) -> Bool {
        if profile.bundleIDs.contains(where: { installedBundles.contains($0.lowercased()) }) {
            return true
        }
        if profile.processNames.contains(where: { installedCLIs.contains($0.lowercased()) }) {
            return true
        }
        return false
    }

    /// 会话目录下的活跃会话数：一级子目录中「window 内存在文件写入」的数量
    /// （目录 mtime 只在增删条目时变，不能反映文件内容写入，必须递归查文件 mtime）
    /// 批量实现：用 enumerator 一次取一批，避免逐文件 syscall 风暴。
    static func activeSessionCount(in dirs: [String], window: TimeInterval, now: Date) -> Int {
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
    private static func dirHasRecentWrite(_ path: String, window: TimeInterval, now: Date, maxDepth: Int = 4) -> Bool {
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

    // MARK: - 采样

    /// 手动触发一次采样（也用于测试与 --probe）
    @discardableResult
    public func sample(now: Date = Date()) -> [AgentSnapshot] {
        // 双信号来源：进程快照（一次）+ 后台文件扫描
        let matcher = processMonitor.matcher()
        fileMonitor.scanAsync()

        var results: [AgentSnapshot] = []
        var anyWork = false

        for profile in profiles {
            let running = matcher.isRunning(profile)
            let cpu = matcher.cpuPercent(profile)

            // 合并本次探测到的最新写入时间（目录不存在时保留上次值）
            let fresh = fileMonitor.lastWriteDates(for: profile.sessionDirs)
            for (dir, date) in fresh {
                if date > (lastWrites[dir] ?? .distantPast) {
                    lastWrites[dir] = date
                }
            }
            let newestAgo: TimeInterval? = {
                let dates = profile.sessionDirs.compactMap { lastWrites[$0] }
                guard let newest = dates.max() else { return nil }
                return now.timeIntervalSince(newest)
            }()

            let level: ActivityLevel
            if !running {
                level = .offline
            } else if (newestAgo.map { $0 <= config.workingWindow } ?? false) || cpu > config.cpuThreshold {
                level = .working
            } else {
                level = .idle
            }

            if level == .working { anyWork = true }
            results.append(AgentSnapshot(
                profile: profile,
                level: level,
                processRunning: running,
                cpuPercent: cpu,
                installed: isInstalled(profile),
                activeSessions: cachedSessionCount(for: profile, running: running, now: now),
                lastActivityAgo: newestAgo,
                lastActivityText: Self.formatAgo(newestAgo),
                tokenUsage: tokenMonitor.usage[profile.id]
            ))
        }

        snapshots = results
        anyWorking = anyWork
        updatedAt = now
        scheduleNext()
        return results
    }

    /// 活跃会话数（缓存版）：离线 agent 直接 0（不扫盘）；运行中的每 5s 重算一次，
    /// 避免主线程在每次采样时全树扫描（大目录下会卡 UI）。
    private func cachedSessionCount(for profile: AgentProfile, running: Bool, now: Date) -> Int {
        guard running else { return 0 }
        if now.timeIntervalSince(sessionCountAt) >= 5 {
            sessionCountAt = now
            sessionCountCache.removeAll()
        }
        if let cached = sessionCountCache[profile.id] { return cached }
        let count = Self.activeSessionCount(in: profile.sessionDirs, window: config.activeSessionWindow, now: now)
        sessionCountCache[profile.id] = count
        return count
    }

    /// 节电调度：有 working 快采样，全闲置降频
    private func scheduleNext() {
        timer?.invalidate()
        let interval = anyWorking ? config.sampleInterval : config.idleSampleInterval
        let t = Timer(timeInterval: interval, repeats: false) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.sample()
            }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    /// 最近 N 秒内有 Agent 变为 working（供通知/自动展开用）
    public func workingAgents() -> [AgentSnapshot] {
        snapshots.filter { $0.level == .working }
    }

    public static func formatAgo(_ interval: TimeInterval?) -> String {
        guard let interval = interval else { return "—" }
        let i = Int(interval.rounded())
        if i < 5 { return "刚刚" }
        if i < 60 { return "\(i)s 前" }
        if i < 3600 { return "\(i / 60)m 前" }
        return "\(i / 3600)h 前"
    }
}
