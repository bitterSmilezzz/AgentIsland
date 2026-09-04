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
    public let tokenMonitor = TokenUsageMonitor()   // token 用量（expanded 时轮询，采样时取快照）
    private var lastWrites: [String: Date] = [:]   // dir -> 最近写入时间
    private var timer: Timer?
    private var samplingInFlight = false   // 后台采样进行中标志：丢弃重叠请求，防 CPU% 差分交错
    private var installedCLIs: Set<String> = []
    private var installedBundles: Set<String> = []
    /// 滞回：CPU 信号瞬时抖动时保持 working 的最短时长（防 peek 高频弹跳）
    private var workingSince: [String: Date] = [:]
    /// 安装缓存重扫计数（每 120 次采样刷一次，见 sampleCore）
    private var installedRefreshCounter = 0

    public init(profiles: [AgentProfile] = AgentRegistry.fullRegistry(),
         config: EngineConfig = EngineConfig(),
         processMonitor: ProcessMonitor = ProcessMonitor(),
         fileMonitor: FileActivityProviding = FileActivityMonitor()) {
        self.profiles = profiles
        self.config = config
        self.processMonitor = processMonitor
        self.fileMonitor = fileMonitor
        refreshInstalled()
        // 注册监控目录（后台扫描用，全量替换）
        fileMonitor.replaceWatchedDirs(profiles.flatMap(\.sessionDirs))
    }

    private var running = false   // stop() 后阻止在飞回调重建定时器（阿剩N1）

    public func start() {
        guard !running else { return }
        running = true
        guard timer == nil else { return }
        // token 数据刷完后触发一次重采样（快照带上用量 + 卡片高度重算）
        tokenMonitor.onRefresh = { [weak self] in
            self?.sampleInBackground()
        }
        tokenMonitor.start()   // token 用量轮询（60s，后台队列）
        sample()               // sample() 末尾自带 scheduleNext()
    }

    public func stop() {
        running = false
        timer?.invalidate()
        timer = nil
        tokenMonitor.stop()
    }

    // MARK: - 动态配置（设置界面接线）

    /// 更新启停集合（enabledAgents）
    /// 从全量注册表（内置+自动发现+自定义）过滤：避免只从当前已缩水列表过滤，
    /// 否则「关闭后再开启」的 agent 本会话内永久丢失监控（阿剩高优）
    public func setEnabled(_ enabledIDs: Set<String>) {
        let all = AgentRegistry.fullRegistry()
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
        // 活跃会话判定窗口同步给后台扫描器
        fileMonitor.setActiveSessionWindow(config.activeSessionWindow)
        // 采样间隔变化 → 重启定时器
        if timer != nil {
            timer?.invalidate()
            timer = nil
            scheduleNext()
        }
    }

    private func refreshWatchedDirs() {
        // 全量替换：当前启用的 profile 目录集合（移除自定义 agent 后其目录停止扫描）
        fileMonitor.replaceWatchedDirs(profiles.flatMap(\.sessionDirs))
        // M5：清理已移除 profile 的滞回状态，防止长期累积
        let activeIDs = Set(profiles.map(\.id))
        workingSince = workingSince.filter { activeIDs.contains($0.key) }
        // 目录级缓存同步清理：移除 agent 后其会话目录的最近写入时间不再保留
        let activeDirs = Set(profiles.flatMap(\.sessionDirs))
        lastWrites = lastWrites.filter { activeDirs.contains($0.key) }
    }

    // MARK: - 安装检测（A4）

    /// 只读快照：后台已重扫（refreshInstalledCache 在后台队列执行），这里仅加锁拷贝
    private func refreshInstalled() {
        installedCLIs = AgentRegistry.installedCLIs()
        installedBundles = AgentRegistry.installedBundleIDs()
    }

    /// 后台重扫安装缓存（阿证中1：/Applications plist 解析可达 30-100ms，
    /// 不应占用主线程；扫描与写回均带锁，可安全后台执行）
    private func refreshInstalledInBackground() {
        DispatchQueue.global(qos: .utility).async {
            AgentRegistry.refreshInstalledCache()
            DispatchQueue.main.async { [weak self] in
                self?.refreshInstalled()
            }
        }
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

    // MARK: - 采样

    /// 手动触发一次采样（也用于测试与 --probe）
    /// 受 samplingInFlight 约束（阿证中1）：后台采样在飞时丢弃本次，
    /// 避免同步路径与后台采样并发导致 CPU 差分短暂失真 + 主线程瞬时开销；
    /// 配置变更/启动等低频场景下丢弃一次无影响（下个周期自动补采）。
    /// 不变量：samplingInFlight=true ⟺ 有在飞后台采样，且其必然走 sampleCore→
    /// scheduleNext() 补调度，故丢弃分支返回 [] 不会造成采样停摆。
    @discardableResult
    public func sample(now: Date = Date()) -> [AgentSnapshot] {
        guard !samplingInFlight else { return [] }
        samplingInFlight = true
        defer { samplingInFlight = false }
        return sampleCore(matcher: processMonitor.matcher(profiles: profiles), now: now)
    }

    /// 定时采样入口：进程遍历（proc_listpids/proc_pidpath，开销毫秒级）在后台执行，
    /// 主线程只做装配与发布，避免与动画抢主线程。文件扫描/会话数/token 均为缓存读取。
    func sampleInBackground() {
        guard !samplingInFlight else { return }   // 丢弃重叠请求（onRefresh 与 Timer 可能相邻）
        samplingInFlight = true
        let monitor = processMonitor
        // NSWorkspace 必须主线程访问（无线程安全保证），先抓 bundle 集合
        let bundleIDs = monitor.runningBundleIDs()
        DispatchQueue.global(qos: .utility).async {
            let snapshot = monitor.snapshot()
            Task { @MainActor [weak self] in
                defer { self?.samplingInFlight = false }
                guard let self else { return }
                self.sampleCore(matcher: monitor.matcher(snapshot: snapshot, runningBundleIDs: bundleIDs, profiles: self.profiles),
                                now: Date())
            }
        }
    }

    /// 采样主体（主线程）：双信号判定 + 快照组装 + 发布
    @discardableResult
    private func sampleCore(matcher: ProcessMatcher, now: Date) -> [AgentSnapshot] {
        fileMonitor.scanAsync()
        // 低频重扫安装缓存（阿剩低3：运行中装新 CLI/App 不必重启）；
        // 后台执行避免主线程卡顿（阿证中1）。每 120 次采样在工作态 ≈ 每 4 分钟
        installedRefreshCounter += 1
        if installedRefreshCounter >= 120 {
            installedRefreshCounter = 0
            refreshInstalledInBackground()
        }

        var results: [AgentSnapshot] = []
        var anyWork = false

        for profile in profiles {
            // 每 profile 只调一次 matchingEntries（阿剩低1：isRunning+cpuPercent 各遍历一遍
            // 全表，合并为单趟；running 由「有匹配条目」推导，与 isRunning 语义等价——
            // bundleHit 无名字匹配时返回 [pid:-1] 占位条目，CPU 合计为 0）
            let entries = matcher.matchingEntries(for: profile)
            let running = !entries.isEmpty
            let cpu = entries.reduce(0) { $0 + $1.cpuPercent }

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
                workingSince[profile.id] = nil
            } else if (newestAgo.map { $0 <= config.workingWindow } ?? false) || cpu > config.cpuThreshold {
                level = .working
                if workingSince[profile.id] == nil { workingSince[profile.id] = now }
            } else if let since = workingSince[profile.id],
                      now.timeIntervalSince(since) < config.minWorkingHold {
                // 滞回：信号刚消失时保持 working 最短时长，防 CPU 临界抖动导致 peek 高频弹跳
                level = .working
            } else {
                level = .idle
                workingSince[profile.id] = nil
            }

            if level == .working { anyWork = true }
            results.append(AgentSnapshot(
                profile: profile,
                level: level,
                processRunning: running,
                cpuPercent: cpu,
                installed: isInstalled(profile),
                activeSessions: sessionCount(for: profile, running: running),
                lastActivityAgo: newestAgo,
                lastActivityText: Self.formatAgo(newestAgo),
                tokenUsage: tokenMonitor.usage[profile.id]
            ))
        }

        // 内容实质变化才发布（@Published 触发所有观察者重算）
        if results != snapshots || anyWork != anyWorking {
            snapshots = results
            anyWorking = anyWork
            updatedAt = now
        }
        scheduleNext()
        return results
    }

    /// 活跃会话数（离线 agent 直接 0；在线读 FileMonitor 后台扫描缓存，主线程零扫描）
    private func sessionCount(for profile: AgentProfile, running: Bool) -> Int {
        guard running else { return 0 }
        let counts = fileMonitor.activeSessionCounts(for: profile.sessionDirs)
        return counts.values.reduce(0, +)
    }

    /// 节电调度：有 working 快采样，闲置降频，全离线进一步拉大间隔（无 UI 需求）
    private func scheduleNext() {
        guard running else { return }   // stop() 后在飞回调不再重建定时器（阿剩N1）
        timer?.invalidate()
        let interval: TimeInterval
        if anyWorking {
            interval = config.sampleInterval
        } else if snapshots.contains(where: { $0.processRunning }) {
            interval = config.idleSampleInterval
        } else {
            interval = max(config.idleSampleInterval, 60)   // 全离线：60s 起
        }
        let t = Timer(timeInterval: interval, repeats: false) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.sampleInBackground()
            }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    /// 最近 N 秒内有 Agent 变为 working（供通知/自动展开用）
    public func workingAgents() -> [AgentSnapshot] {
        snapshots.filter { $0.level == .working }
    }

    /// 可见口径（唯一实现）：在线 + 24h 内活跃。
    /// 展开卡片列表、菜单摘要、高度计算统一消费此属性，改口径只改这一处。
    public var visibleSnapshots: [AgentSnapshot] {
        snapshots.filter {
            $0.processRunning || ($0.lastActivityAgo ?? .infinity) < 24 * 3600
        }
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
