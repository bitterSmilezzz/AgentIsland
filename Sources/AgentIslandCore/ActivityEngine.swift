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
    /// 进程提供 seam（线程契约见协议注释：主线程抓 bundle + 任意线程快照）
    private let processMonitor: ProcessProviding
    private let fileMonitor: FileActivityProviding
    /// token 用量子系统（seam：轮询面 + 查询面；测试经 init 注入 fake）。
    /// 私有实现细节——UI 一律走本类的数据出口（grandTotal / modelBreakdown / sessions）
    private let tokenMonitor: any TokenUsagePolling & TokenUsageQuerying
    /// 已安装缓存（注入实例；引擎是常态刷新的唯一调度者——init 首刷 + 采样循环 300s 周期。
    /// 设置页打开时的显式重扫是用户触发的例外路径，与引擎共用同一实例）
    private let installedApps: InstalledAppsCache
    private var lastWrites: [String: Date] = [:]   // dir -> 最近写入时间
    private var timer: Timer?
    /// 测试观察点：定时器是否已创建（stop 后应为 nil）
    var timerIsNil: Bool { timer == nil }
    private var samplingInFlight = false   // 后台采样进行中标志：丢弃重叠请求，防 CPU% 差分交错
    /// 滞回：CPU 信号瞬时抖动时保持 working 的最短时长（防 peek 高频弹跳）
    private var workingSince: [String: Date] = [:]

    /// - Parameters:
    ///   - profiles: 初始启用档案（组合根/测试显式给定；无默认值——安装判定依赖注入的缓存，
    ///     隐式全量会掩盖「组合根没接缓存」的错误）
    ///   - enabledIDs: 持久化启用集（首刷重放用；nil 则按 profiles 推导）
    public init(profiles: [AgentProfile],
         config: EngineConfig = EngineConfig(),
         processMonitor: ProcessProviding = ProcessProvider(),
         fileMonitor: FileActivityProviding = FileActivityMonitor(),
         tokenMonitor: any TokenUsagePolling & TokenUsageQuerying = TokenUsageMonitor(),
         installedApps: InstalledAppsCache,
         enabledIDs: Set<String>? = nil) {
        self.profiles = profiles
        self.config = config
        self.processMonitor = processMonitor
        self.fileMonitor = fileMonitor
        self.tokenMonitor = tokenMonitor
        self.installedApps = installedApps
        // 启用集记录（组合根传持久化集；nil 则按当前 profiles 推导）——首刷完成后重放用
        self.lastEnabledIDs = enabledIDs ?? Set(profiles.map(\.id))
        // 安装缓存首刷（warmUp：已热幂等跳过，与组合根/Probe 预热互不双扫）。
        // 冷启动完成后重放启用集：自动发现 cli-* 依据扫描结果才可见，重放让「用户已启用
        // 的自动发现项」重启后恢复监控（原 AppContext 二次 setEnabled 舞步的等价物）
        installedApps.warmUp { [weak self] in
            guard let self else { return }
            self.setEnabled(self.lastEnabledIDs)
        }
        // 注册监控目录（后台扫描用，全量替换）
        fileMonitor.replaceWatchedDirs(profiles.flatMap(\.sessionDirs))
    }

    private var running = false   // stop() 后阻止在飞回调重建定时器（阿剩N1）
    // MARK: token 轮询生命周期（单一 owner：引擎）
    // 三状态合法组合（其余组合按不变量不可达）：
    //   running=false ∧ tokenPollingStarted=false   —— 初始 / 已停止（presentationActive 任意，
    //      stop() 刻意不清它：重启后按面板最后一次状态补开）
    //   running=true  ∧ tokenPollingStarted=false   —— 未呈现活跃（docked）或激活先于 start（记状态待补开）
    //   running=true  ∧ tokenPollingStarted=true    —— presentationActive=true（轮询运行中）
    // start() 的 timer 守卫提前返回时不得跳过补开（当前无此路径：running 置位后守卫只在重复 start 触发）
    /// 呈现活跃（面板展开）：token 轮询的唯一驱动源（懒启动）
    private var presentationActive = false
    /// token 轮询当前是否已启动（引擎侧幂等标记；失活暂停后复位置 false）
    private var tokenPollingStarted = false
    /// 最近一次启停集合（首刷完成后重放；见 init）
    private var lastEnabledIDs: Set<String>

    public func start() {
        guard !running else { return }
        running = true
        guard timer == nil else { return }
        sample()               // sample() 末尾自带 scheduleNext()
        // token 轮询懒启动：仅呈现活跃时开启（面板 docked 态启动时零开销；
        // 先展开后启动的顺序在启动时补开）
        if presentationActive {
            startTokenPollingIfNeeded()
        }
    }

    /// 呈现活跃切换（面板 docked/expanded 唯一入口；幂等，吸收「sink 不去重」的重复通知）
    /// - active=true：启动轮询（内含立即首刷；暂停后重启连接沿用缓存）
    /// - active=false：暂停轮询（连接保留，保持 docked 态省电语义）
    public func setPresentationActive(_ active: Bool) {
        guard presentationActive != active else { return }
        presentationActive = active
        guard running else { return }   // 引擎未运行：仅记状态，start() 时补开
        if active {
            startTokenPollingIfNeeded()
        } else {
            tokenMonitor.pause()
            tokenPollingStarted = false
        }
    }

    private func startTokenPollingIfNeeded() {
        guard !tokenPollingStarted else { return }
        tokenPollingStarted = true
        // token 数据刷完后触发一次重采样（快照带上用量 + 卡片高度重算）
        tokenMonitor.onRefresh = { [weak self] in
            self?.sampleInBackground()
        }
        tokenMonitor.start()   // start 内含立即首刷
    }

    public func stop() {
        running = false
        timer?.invalidate()
        timer = nil
        tokenMonitor.onRefresh = nil   // 停止后 in-flight token 刷新不再触发重采样（阿证低3）
        // 条件调用：轮询未启动时无需关闭连接（连接惰性打开，未启动即不存在）
        if tokenPollingStarted {
            tokenMonitor.stop()
            tokenPollingStarted = false
        }
    }

    // MARK: - 动态配置（设置界面接线）

    /// 更新启停集合（enabledAgents）
    /// 从全量注册表（内置+自动发现+自定义）过滤：避免只从当前已缩水列表过滤，
    /// 否则「关闭后再开启」的 agent 本会话内永久丢失监控（阿剩高优）。
    /// 记录启用集供安装缓存首刷完成后重放（见 init）
    public func setEnabled(_ enabledIDs: Set<String>) {
        lastEnabledIDs = enabledIDs
        let all = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs())
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

    // MARK: - 安装检测（A4；缓存与刷新节律见 InstalledAppsCache，引擎只读判定）

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
        // 主线程同步路径：bundle（NSWorkspace）与快照（libproc）都可在此线程直接取
        let matcher = ProcessMatcher(
            snapshot: processMonitor.snapshot(),
            runningBundleIDs: processMonitor.runningBundleIDs(),
            profiles: profiles)
        return sampleCore(matcher: matcher, now: now)
    }

    /// 定时采样入口：进程遍历（proc_listpids/proc_pidpath，开销毫秒级）在后台执行，
    /// 主线程只做装配与发布，避免与动画抢主线程。文件扫描/会话数/token 均为缓存读取。
    func sampleInBackground() {
        guard running else { return }   // stop() 后在飞 token 刷新不再触发采样（阿剩低C）
        guard !samplingInFlight else { return }   // 丢弃重叠请求（onRefresh 与 Timer 可能相邻）
        samplingInFlight = true
        let provider = processMonitor
        // NSWorkspace 必须主线程访问（无线程安全保证，见 ProcessProviding 契约），先抓 bundle 集合
        let bundleIDs = provider.runningBundleIDs()
        DispatchQueue.global(qos: .utility).async {
            let snapshot = provider.snapshot()   // libproc 任意线程
            Task { @MainActor [weak self] in
                defer { self?.samplingInFlight = false }
                guard let self else { return }
                let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: bundleIDs, profiles: self.profiles)
                self.sampleCore(matcher: matcher, now: Date())
            }
        }
    }

    /// 采样主体（主线程）：双信号判定 + 快照组装 + 发布
    @discardableResult
    private func sampleCore(matcher: ProcessMatcher, now: Date) -> [AgentSnapshot] {
        fileMonitor.scanAsync()
        // 低频重扫安装缓存（运行中装新 CLI/App 不必重启；调度即标记，后台执行）。
        // 时间戳阈值 300s：计数在工作态 2s/离线 60s 间隔下粒度漂移 30 倍
        // （阿剩低：离线态最长拖 2 小时），时间戳保证两态刷新粒度一致
        installedApps.refreshIfNeeded(maxAge: 300)

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
                installed: installedApps.isInstalled(profile),
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

    // MARK: - token 数据出口（UI 唯一入口；经 seam 转发给 token 子系统）

    /// 所有数据源总和（语义见 TokenUsagePolling.grandTotal；汇总栏显示 / 展开高度判断）
    public var grandTotal: TokenUsage { tokenMonitor.grandTotal }

    /// 按模型拆分下钻（详情页）
    public func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void) {
        tokenMonitor.modelBreakdown(agentId: agentId, completion: completion)
    }

    /// 某模型下的会话列表下钻（会话页）
    public func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        tokenMonitor.sessions(agentId: agentId, modelId: modelId, completion: completion)
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
