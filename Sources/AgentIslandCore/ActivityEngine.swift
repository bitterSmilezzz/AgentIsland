import Foundation
import Combine

// MARK: - 活动引擎
// 采样状态机（Q1 双信号）：
//   working = 进程在 且（workingWindow 内有文件写入 或 CPU ≥ 判定阈值）
//   idle    = 进程在 但两者皆不满足
//   offline = 进程不在
// CPU 判定阈值 = max(AgentProfile.cpuWorkingThreshold, EngineConfig.cpuThreshold)：
// 前者是桌面类 Agent 的防抖下限（空闲渲染抖动 4%~15%），后者是用户在设置页可调的值，
// 取 max 保证用户调高时对所有 Agent 生效、调低时不会突破防抖下限。
// 节电（Q11）：有 working 用 sampleInterval 采样；全闲置降频 idleSampleInterval。
// 断点（G4）：采样间隔超过 resumeGapThreshold 视为睡眠/挂起，跳过完成事件并重置工作区间。

@MainActor
public final class ActivityEngine: ObservableObject {

    @Published public private(set) var snapshots: [AgentSnapshot] = []
    @Published public private(set) var anyWorking = false
    @Published public private(set) var updatedAt = Date()
    @Published public private(set) var latestEvent: AgentTaskEvent? = nil
    /// 每一条需要对外投递的事件流。latestEvent 是 UI 单槽位，多个 Agent 同拍进入
    /// 等待确认时会互相覆盖；独立事件流保证系统通知逐条收到，不丢任一 Agent。
    public let taskEvents = PassthroughSubject<AgentTaskEvent, Never>()
    public private(set) var cleaner: AgentCleaner!

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
    private var timer: Timer?
    /// 测试观察点：定时器是否已创建（stop 后应为 nil）
    var timerIsNil: Bool { timer == nil }
    private var samplingInFlight = false   // 后台采样进行中标志：丢弃重叠请求，防 CPU% 差分交错
    /// 滞回：CPU 信号瞬时抖动时保持 working 的最短时长（防 peek 高频弹跳）
    private var workingSince: [String: Date] = [:]
    /// 最近一次「有工作信号」的时刻（滞回锚点）。
    /// 与 workingSince 的区别：workingSince 是本次工作区间的起点（仅用于计算任务时长），
    /// lastSignalAt 每拍有信号就刷新（用于滞回判断，否则长任务滞回永不生效）。
    private var lastSignalAt: [String: Date] = [:]
    /// 本次工作区间内是否出现过写入信号（R37 完成事件的准入条件）。
    /// CPU 高只证明「进程在烧 CPU」：桌面应用空闲时也会周期性冲到 20%~90%
    /// （实测 ChatGPT 静置时反复出现 20.6%/43.4% 尖峰），若以 CPU 为准补发完成事件，
    /// 用户就会在「只是打开了应用、什么都没做」时听到完成提示音。
    /// 文件写入是任务产物，只有写入驱动的区间才算做完一件事；纯 CPU 区间仍照常显示
    /// working（双信号判定不变），但不产生完成事件。
    private var workingPeriodHadWrite: Set<String> = []
    /// 当前仍未解除的确认请求（agentId → request fingerprint），用于逐拍去重。
    private var activeAttentionFingerprints: [String: String] = [:]
    /// 最近识别过的显式完成标记，防同一 task_complete 在 15 分钟展示期内重复响铃。
    private var handledCompletionFingerprints: [String: String] = [:]
    /// 上一次采样的时刻，用于识别睡眠/挂起造成的采样断点（见 sampleCore）
    private var lastSampleAt: Date?
    /// 采样断点判定阈值：超过则视为发生睡眠/挂起。
    /// 必须显著大于正常调度间隔——最慢节律是全离线态的 60s（且用户可把闲置间隔
    /// 调到 60s），叠加调度延迟后仍不应误判。取「2 分钟」与「3 倍闲置间隔」的较大者，
    /// 并钳 180s 上界（R32/F1）：idle 滑杆上限 60s 时 3 倍 = 180s，而 2–3 分钟的
    /// App Nap/挂起若逃过断点检测会补发「任务完成（时长含整段睡眠）」假事件。
    /// 上界 180 仍留有余量：全离线 60s 节律 + 调度延迟不会误判。
    private var resumeGapThreshold: TimeInterval {
        min(max(120, config.idleSampleInterval * 3), 180)
    }

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
        self.cleaner = AgentCleaner(processMonitor: processMonitor)
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
        // 初始配置同步（applyConfig 只在 config didSet 时触发，init 传入的配置需显式应用）
        fileMonitor.setActiveSessionWindow(config.activeSessionWindow)
        fileMonitor.setWorkingWindow(config.workingWindow)

        // 绿色节能调度：监听系统低电量模式状态改变
        powerStateObserver = NotificationCenter.default.addObserver(
            forName: NSNotification.Name.NSProcessInfoPowerStateDidChange,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let isLow = ProcessInfo.processInfo.isLowPowerModeEnabled
                if self.isLowPowerModeActive != isLow {
                    self.isLowPowerModeActive = isLow
                    if self.running {
                        self.scheduleNext()
                    }
                }
            }
        }
    }

    deinit {
        if let observer = powerStateObserver {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    /// 低电量节能模式感知（macOS 12+ 原生支持）
    @Published public private(set) var isLowPowerModeActive: Bool = ProcessInfo.processInfo.isLowPowerModeEnabled
    private var powerStateObserver: NSObjectProtocol?

    /// 任务耗时与效率统计追踪器
    public let durationTracker = TaskDurationTracker()

    private var running = false   // stop() 后阻止在飞回调重建定时器
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
    /// 上次记录的 Token 用量与时间基准（agentId → (timestamp, tokensTotal)），用于速率差分。
    /// 非 private：@testable 下断言「offline 清基线」不变量用（模块内勿直接写）
    var tokenRateBaseline: [String: (timestamp: Date, tokens: Int)] = [:]
    /// 动作探测注入点（测试用）：nil 走 AgentActionInspector.inspectAction 默认实现。
    /// 「idle 不探测」的门控测试靠计数 hook 断言，无需真实会话目录。
    var inspectActionHook: ((Int32?, AgentProfile, [String], ProcessSnapshot?) -> String?)?
    /// 会话强语义探测注入点（测试用）：生产只尾读 FileMonitor 缓存的最新文件。
    var inspectSessionHook: ((AgentProfile, [URL], Date) -> AgentSessionSignal?)?
    /// 连续超过速率阈值的评估档数（agentId → 档数）；达到确认档数才告警
    private var tokenSpikeStreak: [String: Int] = [:]
    /// 同一轮持续超阈值只提醒一次，速率恢复后重新武装。
    /// 非 private：@testable 下断言「断点清除去重标记」用（模块内勿直接写）
    var tokenSpikeAlerted: Set<String> = []
    /// 激增评估档长：满一档才结算一次速率，一次性落盘会被摊平
    static let tokenRateWindow: TimeInterval = 60
    /// 需连续多少档超阈值才告警（滤掉长任务结束时的一次性账本落盘）
    static let tokenSpikeConfirmations = 3
    /// 持续高负载时间追踪（agentId → 开始高负载的时间戳），用于判定真正的死循环
    private var highCpuSince: [String: Date] = [:]
    /// 上次高负载告警时间（agentId → 告警时间），防止每个采样周期重复轰炸
    private var lastRunawayAlertedAt: [String: Date] = [:]

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
        tokenMonitor.onRefresh = nil   // 停止后 in-flight token 刷新不再触发重采样
        // 条件调用：轮询未启动时无需关闭连接（连接惰性打开，未启动即不存在）
        if tokenPollingStarted {
            tokenMonitor.stop()
            tokenPollingStarted = false
        }
    }

    /// 系统休眠标志：为 true 时彻底停止定时器和监控，极致节能
    private var isSystemSleeping = false

    /// 系统休眠联动（Mac 盒盖或进入睡眠）：冻结轮询定时器与令牌拉取
    public func handleSystemSleep() {
        guard running, !isSystemSleeping else { return }
        isSystemSleeping = true
        timer?.invalidate()
        timer = nil
        if tokenPollingStarted {
            tokenMonitor.pause()
        }
    }

    /// 系统唤醒联动（Mac 开盖或从睡眠唤醒）：恢复定时器并立即派发毫秒级热同步
    public func handleSystemWake() {
        guard running, isSystemSleeping else { return }
        isSystemSleeping = false
        if presentationActive {
            startTokenPollingIfNeeded()
        }
        scheduleNext()
        sampleInBackground()
    }

    // MARK: - 动态配置（设置界面接线）

    /// 更新启停集合（enabledAgents）
    /// 从全量注册表（内置+自动发现+自定义）过滤：避免只从当前已缩水列表过滤，
    /// 否则「关闭后再开启」的 agent 本会话内永久丢失监控。
    /// 记录启用集供安装缓存首刷完成后重放（见 init）
    public func setEnabled(_ enabledIDs: Set<String>) {
        lastEnabledIDs = enabledIDs
        let all = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs(),
                                             installedBundles: installedApps.installedBundleIDs())
        profiles = all.filter { enabledIDs.contains($0.id) }
        refreshWatchedDirs()
        // 后台采样（R34/G1）：snapshot 全表 + 17 profile 匹配 + 动作探测此前全在
        // 主线程同步执行（设置页每次开关可感知卡顿）；后台路径具备同等的
        // running/samplingInFlight 防护
        sampleInBackground()
    }

    /// 增加自定义 profile（设置界面新增）
    public func addCustomProfile(_ profile: AgentProfile) {
        guard !profiles.contains(where: { $0.id == profile.id }) else { return }
        profiles.append(profile)
        fileMonitor.watch(dirs: profile.sessionDirs)
        sampleInBackground()
    }

    /// 移除自定义 profile
    public func removeCustomProfile(_ id: String) {
        profiles.removeAll { $0.id == id && $0.isCustom }
        refreshWatchedDirs()
        sampleInBackground()
    }

    public var allProfiles: [AgentProfile] { profiles }

    /// 按需单次刷新 token 用量（R34/F6）：菜单栏 popover 打开时调用——
    /// popover 是 token 数据的第三消费方但不参与「呈现活跃」生命周期
    /// （docked 常态下轮询已暂停，此前 popover 显示自上次收起以来冻结的值；
    /// 从未展开过面板时甚至永远为空）。一次按需查询即可，不引入常驻轮询
    public func refreshTokenUsageOnce() {
        tokenMonitor.refreshAsync()
    }

    private func applyConfig() {
        // 活跃会话判定窗口同步给后台扫描器
        fileMonitor.setActiveSessionWindow(config.activeSessionWindow)
        // 工作判定窗口同步给扫描器：快跳过兜底周期必须与 workingWindow 同口径，
        // 否则用户调小窗口会漏判、调大窗口会把旧时间戳当新写入
        fileMonitor.setWorkingWindow(config.workingWindow)
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
        workingPeriodHadWrite = workingPeriodHadWrite.filter { activeIDs.contains($0) }
        activeAttentionFingerprints = activeAttentionFingerprints.filter { activeIDs.contains($0.key) }
        handledCompletionFingerprints = handledCompletionFingerprints.filter { activeIDs.contains($0.key) }
        lastSignalAt = lastSignalAt.filter { activeIDs.contains($0.key) }
        highCpuSince = highCpuSince.filter { activeIDs.contains($0.key) }
        lastRunawayAlertedAt = lastRunawayAlertedAt.filter { activeIDs.contains($0.key) }
        tokenSpikeStreak = tokenSpikeStreak.filter { activeIDs.contains($0.key) }
        tokenSpikeAlerted = tokenSpikeAlerted.filter { activeIDs.contains($0) }
        tokenRateBaseline = tokenRateBaseline.filter { activeIDs.contains($0.key) }
    }

    // MARK: - 安装检测（A4；缓存与刷新节律见 InstalledAppsCache，引擎只读判定）

    // MARK: - 采样

    /// 手动触发一次采样（也用于测试与 --probe）
    /// 受 samplingInFlight 约束：后台采样在飞时丢弃本次，
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
    public func sampleInBackground() {
        guard running else { return }   // stop() 后在飞 token 刷新不再触发采样
        guard !samplingInFlight else { return }   // 丢弃重叠请求（onRefresh 与 Timer 可能相邻）
        samplingInFlight = true
        let provider = processMonitor
        // NSWorkspace 必须主线程访问（无线程安全保证，见 ProcessProviding 契约），先抓 bundle 集合
        let bundleIDs = provider.runningBundleIDs()
        DispatchQueue.global(qos: .utility).async { [weak self, provider, bundleIDs] in
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
        // 采样断点检测（睡眠/唤醒、App Nap、长时间挂起）：
        // 合盖或挂起期间时间在走但没有任何采样，唤醒后的第一拍会看到
        // 「信号早已消失 + 距上次工作很久」——按常规逻辑会补发一条
        // 「任务完成 (480分0秒)」并弹系统通知，而 Agent 其实只是被挂起了。
        // 检测到断点就跳过完成事件，并把工作区间起点顺延到本拍（视为重新开始）。
        let isResumeGap: Bool = {
            guard let last = lastSampleAt else { return false }
            return now.timeIntervalSince(last) > resumeGapThreshold
        }()
        if isResumeGap {
            workingSince.removeAll()
            workingPeriodHadWrite.removeAll()
            lastSignalAt.removeAll()
            highCpuSince.removeAll()
            lastRunawayAlertedAt.removeAll()
            // token 速率基准同步重置：跨越睡眠的窗口会把「睡眠期间的账本变化」
            // 折算成极高速率，重置后从本拍重新起算
            tokenRateBaseline.removeAll()
            tokenSpikeStreak.removeAll()
            // 激增告警去重标记一并清（R32/F7）：睡前已告警、醒后持续高速率的
            // Agent 会被残留标记静默压制到出现低于阈值的一档才重新武装
            tokenSpikeAlerted.removeAll()
        }
        lastSampleAt = now

        // 提取当前有运行中进程/应用的 sessionDirs，告知 FileMonitor 优化扫描（离线目录免深搜）
        var runningSessionDirs: Set<String> = []
        for profile in profiles {
            if matcher.isRunning(profile) {
                runningSessionDirs.formUnion(profile.sessionDirs)
            }
        }
        fileMonitor.setRunningDirs(runningSessionDirs)
        fileMonitor.scanAsync()
        // 低频重扫安装缓存（运行中装新 CLI/App 不必重启；调度即标记，后台执行）。
        // 时间戳阈值 300s：计数在工作态 2s/离线 60s 间隔下粒度漂移 30 倍
        // （离线态最长拖 2 小时），时间戳保证两态刷新粒度一致
        installedApps.refreshIfNeeded(maxAge: 300)

        var results: [AgentSnapshot] = []
        // 本拍取一次 token 用量快照（getter 持锁并整字典拷贝）：每拍 17 profile ×
        // 2 处 = 34 次「锁+全字典拷贝」收敛为 1 次，告警链路复用同一快照
        let usageSnapshot = tokenMonitor.usage
        var anyWork = false
        /// 本拍每 profile 的 CPU 与 PID，供告警链路复用
        var sampleInfo: [String: SampleInfo] = [:]

        for profile in profiles {
            // 时钟回拨重锚（防御）：系统时钟被回拨（NTP 阶跃 / 手动调整 / 虚拟机恢复快照）
            // 后墙钟差值不可信——now 落在锚点之前时，滞回区间变负会让 Agent 永远卡在
            // working（now - lastSignal 恒 < minWorkingHold），任务时长 / 高负载追踪同样失真。
            // 把仍落在「未来」的锚点重置为本拍时刻，让滞回与计时在当前时钟下重新计起。
            if let t = workingSince[profile.id], t > now { workingSince[profile.id] = now }
            if let t = lastSignalAt[profile.id], t > now { lastSignalAt[profile.id] = now }
            if let t = highCpuSince[profile.id], t > now { highCpuSince[profile.id] = now }
            if let t = lastRunawayAlertedAt[profile.id], t > now { lastRunawayAlertedAt[profile.id] = now }

            // 每 profile 只调一次 matchingEntries（isRunning+cpuPercent 各遍历一遍
            // 全表，合并为单趟；running 由「有匹配条目」推导，与 isRunning 语义等价——
            // bundleHit 无名字匹配时返回 [pid:-1] 占位条目，CPU 合计为 0）
            let entries = matcher.matchingEntries(for: profile)
            let running = !entries.isEmpty
            let matchedPID = entries.first(where: { $0.pid > 0 })?.pid
            let cpu = entries.reduce(0) { $0 + $1.cpuPercent }

            // 最近写入时间直读 FileMonitor 缓存（写回为单调 merge：扫描失败保留旧值、
            // 只进不退，引擎无需影子副本——见 FileMonitor.runScan 与 ADR-0001）
            let fresh = fileMonitor.lastWriteDates(for: profile.sessionDirs)
            let newestAgo: TimeInterval? = {
                guard let newest = fresh.values.max() else { return nil }
                // 时钟回拨防御：mtime 可能晚于本拍 now（NTP 阶跃 / 手动回拨后旧文件
                // 落在「未来」），负的经过时长会让 hasRecentWrite 与滞回区间失真——
                // 钳为 0（视作刚写入）：回拨前的真实写入本就发生在「不久前」
                let dt = now.timeIntervalSince(newest)
                return dt < 0 ? 0 : dt
            }()

            let hasRecentWrite = newestAgo.map { $0 <= config.workingWindow } ?? false

            // CPU 判定：阈值 = max(档案下限, 用户设置)。
            // 下限来自 AgentProfile.cpuWorkingThreshold —— 桌面类（Electron/多进程）Agent
            // 即使空闲也有 4%~15% 的渲染与 IPC 抖动，必须有下限才能与「真在工作」区分；
            // 用 max 而非覆盖，保证用户在设置页把阈值调高时对所有 Agent 都生效
            // （此前按 id 硬编码，设置项对 13/17 个内置 Agent 完全无效）。
            let cpuFloor = profile.cpuWorkingThreshold ?? 0
            let hasHighCpu = cpu >= max(cpuFloor, config.cpuThreshold)

            // 结构化会话终态优先于 CPU/mtime 近似值：等待用户时 CPU 通常为 0，旧逻辑会
            // 谎报“待机”；task_complete 刚写入文件时旧逻辑反而会被 workingWindow 拖住。
            // FileMonitor 已在后台给出最新文件，主线程这里只做有界尾读，不重新遍历目录。
            let sessionSignal: AgentSessionSignal? = {
                guard running else { return nil }
                let files = Array(fileMonitor.latestActivityFiles(for: profile.sessionDirs).values)
                if let inspectSessionHook {
                    return inspectSessionHook(profile, files, now)
                }
                // 没有本轮扫描命中的活动文件时不得凭 profile 的约定路径旁路读取：
                // 一方面避免把旧数据库中的终态套到当前进程，另一方面保持采样输入可复现。
                guard !files.isEmpty else { return nil }
                return AgentSessionInspector.inspect(profile: profile, activityFiles: files, now: now)
            }()

            let level: ActivityLevel
            if !running {
                // 进程消失 = Agent 被关闭/退出，不是任务完成：静默转 offline，不发完成事件。
                // （此前会误报「任务已完成」——完成事件只应由「进程仍在但工作信号消失」产生）
                level = .offline
                workingSince[profile.id] = nil
                workingPeriodHadWrite.remove(profile.id)
                activeAttentionFingerprints[profile.id] = nil
                handledCompletionFingerprints[profile.id] = nil
                lastSignalAt[profile.id] = nil
                highCpuSince[profile.id] = nil
                lastRunawayAlertedAt[profile.id] = nil
                tokenSpikeStreak[profile.id] = nil
                tokenSpikeAlerted.remove(profile.id)
                // 速率基线一并清除（与 resumeGap 断点处理口径一致）：否则重启后首个
                // 结算窗口把离线全程计入分母，速率被摊薄，本应触发的激增告警被推迟
                tokenRateBaseline[profile.id] = nil
            } else if case let .attention(request)? = sessionSignal {
                level = .attention
                // 等待用户不是任务完成：切断旧工作区间且绝不补完成事件。
                workingSince[profile.id] = nil
                workingPeriodHadWrite.remove(profile.id)
                lastSignalAt[profile.id] = nil
                if activeAttentionFingerprints[profile.id] != request.fingerprint {
                    activeAttentionFingerprints[profile.id] = request.fingerprint
                    publish(AgentTaskEvent(
                        agentId: profile.id,
                        agentName: profile.name,
                        eventType: .attention,
                        duration: 0,
                        timestamp: now,
                        pid: matchedPID,
                        message: "\(profile.name)：\(request.message)",
                        detail: "(profile.name) 已暂停执行，正在等待你的选择、权限批准或确认。点击通知可返回对应 Agent 窗口继续处理。"
                    ))
                }
            } else if case let .completed(fingerprint)? = sessionSignal {
                level = .completed
                activeAttentionFingerprints[profile.id] = nil
                // 只有本进程生命周期内确实观察过 working 区间才发完成通知；冷启动读到
                // 旧 task_complete 只展示“已完成”，不补一条陈年通知。
                if handledCompletionFingerprints[profile.id] != fingerprint {
                    if let since = workingSince[profile.id], workingPeriodHadWrite.contains(profile.id) {
                        recordTaskCompleted(profile: profile, since: since, now: now, pid: matchedPID)
                    }
                    handledCompletionFingerprints[profile.id] = fingerprint
                }
                workingSince[profile.id] = nil
                workingPeriodHadWrite.remove(profile.id)
                lastSignalAt[profile.id] = nil
            } else if case .active? = sessionSignal {
                activeAttentionFingerprints[profile.id] = nil
                level = .working
                if workingSince[profile.id] == nil {
                    workingSince[profile.id] = now
                    workingPeriodHadWrite.insert(profile.id)
                } else {
                    workingPeriodHadWrite.insert(profile.id)
                }
                lastSignalAt[profile.id] = now
            } else if hasRecentWrite || hasHighCpu {
                activeAttentionFingerprints[profile.id] = nil
                level = .working
                if workingSince[profile.id] == nil {
                    workingSince[profile.id] = now
                    // 区间起点确定写入证据：起点就是写入驱动的，或区间内稍后出现写入
                    // （下面每拍 OR 进去），才允许这条区间在结束时发完成事件
                    if hasRecentWrite {
                        workingPeriodHadWrite.insert(profile.id)
                    } else {
                        workingPeriodHadWrite.remove(profile.id)
                    }
                } else if hasRecentWrite {
                    workingPeriodHadWrite.insert(profile.id)
                }
                lastSignalAt[profile.id] = now
            } else if let lastSignal = lastSignalAt[profile.id],
                      now.timeIntervalSince(lastSignal) < config.minWorkingHold {
                activeAttentionFingerprints[profile.id] = nil
                // 滞回：信号刚消失时保持 working 最短时长，防 CPU 临界抖动导致 peek 高频弹跳。
                // 锚点必须是「最后一次有信号」的时刻而非首次进入 working 的时刻——
                // 用 workingSince 会让任何超过 minWorkingHold 的任务滞回完全失效。
                // （时钟回拨的负区间已由上方重锚防御：lastSignal 不可能晚于本拍 now）
                level = .working
            } else {
                activeAttentionFingerprints[profile.id] = nil
                level = .idle
                // 完成事件只在有写入证据时发（见 workingPeriodHadWrite 注释）：
                // 纯 CPU 高负载的区间到此静默收尾，不响铃、不弹横幅
                if let since = workingSince[profile.id], workingPeriodHadWrite.contains(profile.id) {
                    recordTaskCompleted(profile: profile, since: since, now: now, pid: matchedPID)
                }
                workingSince[profile.id] = nil
                workingPeriodHadWrite.remove(profile.id)
                lastSignalAt[profile.id] = nil
                highCpuSince[profile.id] = nil
            }

            // 状态消除与横幅联动：
            // 1. 若当前 Agent 不再处于 attention 态，自动消除该 Agent 的等待确认提醒横幅（用户已选择确认或已恢复执行）
            if level != .attention, latestEvent?.agentId == profile.id, latestEvent?.eventType == .attention {
                clearLatestEvent()
            }
            // 2. 若当前 Agent 重新进入 working 态，且当前展示的是该 Agent 上一次的已完成横幅，予以清理
            if level == .working, latestEvent?.agentId == profile.id, latestEvent?.eventType == .completed {
                clearLatestEvent()
            }

            let action: String?
            if level == .working {
                anyWork = true
                // 动作透传（仅展示「正在执行 xxx」，不参与状态判定）移到等级判定之后：
                // 只对 working（含滞回）探测——此前 idle 时也全树枚举会话目录/查库，
                // 结果在这里本来就会被丢弃，等于每 2s 白付一次主线程 I/O（重度会话树
                // 实测 5-20ms/次）。探测源（SQLite 最新行/日志尾部）在空闲时依然可能
                // 命中旧记录，工作状态只由「文件写入 + CPU」两个信号决定。
                // 传入 matcher.snapshot 复用本拍进程表：子进程查找零额外系统调用。
                let inspector = inspectActionHook ?? { pid, profile, dirs, snap in
                    AgentActionInspector.inspectAction(pid: pid, profile: profile, sessionDirs: dirs, snapshot: snap)
                }
                let detected = inspector(matchedPID, profile, profile.sessionDirs, matcher.snapshot)
                if let detected, !detected.isEmpty {
                    action = detected
                } else if case let .active(_, actionText)? = sessionSignal {
                    action = actionText
                } else {
                    action = nil
                }
            } else if case let .attention(request)? = sessionSignal {
                action = request.message
            } else {
                action = nil
            }

            let memory = entries.reduce(UInt64(0)) { $0 + $1.rssBytes }

            // 持续高负载追踪：采集与告警解耦。
            // isHung 驱动行内「疑似卡死」徽标、详情页状态与工作台死锁扫描，必须始终采集；
            // 此前该状态只在 runawayCpuAlert 开启时维护，用户关掉「死循环告警」会连带
            // 让卡死检测静默失效（设置文案只承诺关闭告警，未说会丢监控）。
            if running {
                if cpu >= config.runawayCpuThreshold {
                    if highCpuSince[profile.id] == nil { highCpuSince[profile.id] = now }
                } else {
                    highCpuSince[profile.id] = nil
                    lastRunawayAlertedAt[profile.id] = nil
                }
            }

            // 用当拍采集后的值判定，比此前「读上一拍残留」更及时
            let isHung = (highCpuSince[profile.id].map { now.timeIntervalSince($0) >= config.runawayDurationThreshold } ?? false)

            results.append(AgentSnapshot(
                profile: profile,
                level: level,
                processRunning: running,
                cpuPercent: cpu,
                installed: installedApps.isInstalled(profile),
                activeSessions: sessionCount(for: profile, running: running),
                lastActivityAgo: newestAgo,
                lastActivityText: Self.formatAgo(newestAgo),
                tokenUsage: usageSnapshot[profile.id],
                pid: matchedPID,
                currentAction: action,
                memoryBytes: memory,
                isHung: isHung
            ))
            // 本拍 CPU/PID 供告警链路复用（避免二次全表匹配）
            sampleInfo[profile.id] = SampleInfo(cpu: cpu, pid: matchedPID)
        }

        // 内容实质变化才发布（@Published 触发所有观察者重算）
        if results != snapshots || anyWork != anyWorking {
            snapshots = results
            // 扫描紧迫度跟随工作态：全闲置时放宽兜底重扫周期（省 CPU），
            // 有 working 时恢复灵敏（文件信号消失要尽快回落 idle）
            if anyWork != anyWorking {
                fileMonitor.setScanUrgency(highFrequency: anyWork)
            }
            anyWorking = anyWork
            updatedAt = now
        }

        // 成本与异常熔断保护：检测 Token 激增与死循环运行。
        // 传入主循环已算出的每 profile CPU/PID：函数内再算一遍会重复全表匹配
        // （实测 2.5ms/拍，占主线程采样 34%）。
        checkCostSpikeAndRunaway(samples: sampleInfo, now: now, usage: usageSnapshot)

        scheduleNext()
        return results
    }

    /// 每 profile 本拍已算出的采样值（供告警链路复用，避免重复全表匹配）
    private struct SampleInfo {
        let cpu: Double
        let pid: Int32?
    }

    /// 成本与异常熔断保护：只负责**判定与发事件**。
    /// 状态采集（`highCpuSince` / `tokenRateBaseline` 等）分别在主循环与下文中维护——
    /// 采集必须始终进行，否则关闭某个告警开关会连带让 isHung、卡死扫描等消费方静默失效。
    private func checkCostSpikeAndRunaway(samples: [String: SampleInfo], now: Date,
                                          usage: [String: TokenUsage]) {
        if config.tokenAlertEnabled {
            for profile in profiles {
                guard let usageInfo = usage[profile.id], usageInfo.tokensTotal > 0 else { continue }
                guard var base = tokenRateBaseline[profile.id] else {
                    tokenRateBaseline[profile.id] = (timestamp: now, tokens: usageInfo.tokensTotal)
                    continue
                }
                // 时钟回拨重锚：基线时间戳晚于本拍（系统时钟被回拨）会让结算窗口为负——
                // 重锚到当前拍，速率追踪在当前时钟下重新计起
                if base.timestamp > now { base.timestamp = now }
                let timeSpan = now.timeIntervalSince(base.timestamp)
                // 不足一档不结算：一次采样就把长任务的账本落盘当激增会误报
                guard timeSpan >= Self.tokenRateWindow else { continue }
                let deltaTokens = usageInfo.tokensTotal - base.tokens
                tokenRateBaseline[profile.id] = (timestamp: now, tokens: usageInfo.tokensTotal)
                // 折算为每分钟速率：正常绘画/长任务摊到多档后低于阈值，不再触发
                let tokensPerMinute = Double(deltaTokens) / timeSpan * 60.0

                // 实际阈值 = max(档案专属下限, 用户设置全局阈值)
                // 专为 WorkBuddy 等多专家团架构设计：日常 3-5 专家并行产生的高消耗不误报，
                // 但超大规模死循环或超过用户设置的更高档位依然能精准熔断告警。
                let alertFloor = profile.tokenAlertFloor ?? 0
                let effectiveThreshold = max(alertFloor, config.tokenAlertThreshold)

                if deltaTokens > 0, tokensPerMinute >= Double(effectiveThreshold) {
                    let streak = (tokenSpikeStreak[profile.id] ?? 0) + 1
                    tokenSpikeStreak[profile.id] = streak
                    // 需连续多档超阈值才告警：滤掉单次账本补写（如长任务结束时一次性落盘）
                    guard streak >= Self.tokenSpikeConfirmations,
                          !tokenSpikeAlerted.contains(profile.id) else { continue }
                    let matchedPID = samples[profile.id]?.pid
                    let floorNote = alertFloor > config.tokenAlertThreshold ? "，含 \(profile.name) 专家团保护下限 \(TokenUsage.compact(alertFloor))" : ""
                    postEvent(AgentTaskEvent(
                        agentId: profile.id,
                        agentName: profile.name,
                        eventType: .costSpike,
                        duration: timeSpan * Double(streak),
                        timestamp: now,
                        pid: matchedPID,
                        message: "⚠️ \(profile.name) Token 激增 (+\(TokenUsage.compact(deltaTokens)))",
                        detail: "近 \(Int(timeSpan)) 秒 Token 净消耗 +\(TokenUsage.compact(deltaTokens))，约 \(TokenUsage.compact(Int(tokensPerMinute)))/分钟，已连续 \(streak) 个周期超过阈值（生效报警阈值: \(TokenUsage.compact(effectiveThreshold))/分钟\(floorNote)）。常见原因：长上下文灌入、复杂循环或多 Agent 并发。建议点击直达检查会话状态。"
                    ))
                    tokenSpikeAlerted.insert(profile.id)
                } else {
                    tokenSpikeStreak[profile.id] = 0
                    tokenSpikeAlerted.remove(profile.id)
                }
            }
        }

        // 死循环告警：只读主循环已维护的 highCpuSince 状态并决定是否发事件。
        // 状态采集在主循环里无条件进行（见 sampleCore），此处只管发不发告警——
        // 关闭该开关不应影响 isHung 与工作台死锁扫描。
        if config.runawayCpuAlert {
            for profile in profiles {
                guard let info = samples[profile.id], let since = highCpuSince[profile.id] else { continue }
                let highDuration = now.timeIntervalSince(since)
                guard highDuration >= config.runawayDurationThreshold else { continue }
                guard lastRunawayAlertedAt[profile.id] == nil else { continue }
                lastRunawayAlertedAt[profile.id] = now
                let minutes = max(1, Int(highDuration / 60))
                postEvent(AgentTaskEvent(
                    agentId: profile.id,
                    agentName: profile.name,
                    eventType: .costSpike,
                    duration: highDuration,
                    timestamp: now,
                    pid: info.pid,
                    message: "⚠️ \(profile.name) 持续高负载超 \(minutes) 分钟 (CPU \(Int(info.cpu))%)",
                    detail: "进程持续高负载占用 CPU \(Int(info.cpu))% 已达 \(minutes) 分钟（报警阈值: ≥\(Int(config.runawayCpuThreshold))% 持续超 \(Int(config.runawayDurationThreshold / 60)) 分钟）。若任务卡死或非预期，可点击【熔断】安全结束。"
                ))
            }
        } else {
            // 仅重置「本次会话是否已告警」的去重标记，让重新开启开关后能立即告警；
            // highCpuSince 是 isHung 的数据源，不受开关影响、不在此清除
            lastRunawayAlertedAt.removeAll()
        }
    }

    /// 终止智能体进程逃生舱：关闭目标 Agent 及其子进程，并更新状态
    /// - Returns: 是否真正发出了终止信号。pid 缺失时返回 false 且不发送成功事件，
    ///   避免「假成功」——此前无论有无 pid 都宣告「进程已终止」，用户以为已熔断，
    ///   实际进程仍在烧 token（GUI bundle 命中但进程名未匹配时 pid 为 nil）。
    @discardableResult
    public func terminateAgent(pid: Int32?, agentId: String) -> Bool {
        let profile = profiles.first { $0.id == agentId }
        let name = profile?.name ?? agentId
        guard let pid, pid > 1 else {
            publish(AgentTaskEvent(
                agentId: agentId,
                agentName: name,
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: nil,
                message: "无法终止 \(name)：未定位到进程",
                detail: "检测到 \(name) 处于活动状态，但未能匹配到可终止的进程 PID（可能是 Electron 辅助进程或权限受限）。请从菜单栏图标或活动监视器手动处理。"
            ))
            return false
        }
        // 消费 terminate 的真实结果：无权限 / 进程已消失时 kill 返回非 0，
        // 不能一律宣告成功（否则「假成功」只修了 pid 缺失那一半）
        // 身份复核：pid 必须仍属于该 Agent 的当前进程。熔断横幅携带的 pid 来自事件产生
        // 时刻，用户可能数分钟后才点击；期间进程退出后 PID 被系统回收复用给无关程序时，
        // 直接 kill 会把别人的进程连同其子进程树整体终止。用最新快照重新匹配，匹配不到
        // 即「已退出或已变更」，拒绝发送信号。
        var verifiedPath: String?
        if let profile {
            let matcher = ProcessMatcher(
                snapshot: processMonitor.snapshot(),
                runningBundleIDs: processMonitor.runningBundleIDs(),
                profiles: profiles
            )
            verifiedPath = matcher.matchingEntries(for: profile).first { $0.pid == pid }?.path
        }
        guard let verifiedPath else {
            publish(AgentTaskEvent(
                agentId: agentId,
                agentName: name,
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: pid,
                message: "无法终止 \(name)：进程已退出或已变更",
                detail: "记录的 PID \(pid) 已不属于当前运行的 \(name) 进程（可能已自行退出，或 PID 已被系统回收复用）。为避免误杀无关进程，本次未发送终止信号。请重新确认进程状态后再操作。"
            ))
            return false
        }
        switch ProcessTerminator.terminate(pid: pid, expectedPath: verifiedPath) {
        case .signalSent:
            break
        case .identityMismatch:
            // 快照匹配通过但 kill 前一瞬身份再变（极小竞态窗口），同样拒绝
            publish(AgentTaskEvent(
                agentId: agentId,
                agentName: name,
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: pid,
                message: "无法终止 \(name)：进程已退出或已变更",
                detail: "PID \(pid) 的可执行文件在终止前发生了变化（疑似 PID 已被系统回收复用）。为避免误杀无关进程，本次未发送终止信号。"
            ))
            return false
        case .failed:
            publish(AgentTaskEvent(
                agentId: agentId,
                agentName: name,
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: pid,
                message: "无法终止 \(name)：信号发送失败",
                detail: "已尝试终止 PID \(pid)，但未能向其发送信号（进程可能已退出，或需要更高权限）。请确认进程状态或在活动监视器中处理。"
            ))
            return false
        }
        workingSince[agentId] = nil
        workingPeriodHadWrite.remove(agentId)
        activeAttentionFingerprints[agentId] = nil
        handledCompletionFingerprints[agentId] = nil
        lastSignalAt[agentId] = nil
        highCpuSince[agentId] = nil
        lastRunawayAlertedAt[agentId] = nil
        tokenSpikeStreak[agentId] = nil
        tokenSpikeAlerted.remove(agentId)
        publish(AgentTaskEvent(
            agentId: agentId,
            agentName: name,
            // 终止成功是「已完成」而非「需关注」：用 attention 会让收起态细条误报红色告警
            eventType: .completed,
            duration: 0,
            timestamp: Date(),
            pid: pid,
            message: "\(name) 进程已终止",
            detail: "已向 PID \(pid) 及其关联子进程发送 SIGTERM/SIGKILL 终止信号，系统资源已释放。"
        ))
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            self?.sampleInBackground()
        }
        return true
    }

    /// 智能体工作台一键清理：安全清理指定的异常/孤儿进程并展示清理横幅
    /// - Returns: 清理结果（终止数与回收内存），供 UI 判断成败。
    @discardableResult
    public func cleanAnomalies(_ anomalies: [AgentAnomaly]) -> CleanResult {
        guard !anomalies.isEmpty else { return CleanResult(terminatedCount: 0, reclaimedMemoryBytes: 0) }
        let res = cleaner.clean(anomalies: anomalies)
        for a in anomalies {
            workingSince[a.profileId] = nil
            workingPeriodHadWrite.remove(a.profileId)
            activeAttentionFingerprints[a.profileId] = nil
            handledCompletionFingerprints[a.profileId] = nil
            lastSignalAt[a.profileId] = nil
            highCpuSince[a.profileId] = nil
            lastRunawayAlertedAt[a.profileId] = nil
            tokenSpikeStreak[a.profileId] = nil
            tokenSpikeAlerted.remove(a.profileId)
        }
        // 一个都没杀掉 ≠ 清理成功：进程可能已退出、可能无权限、PID 可能已被复用。
        // 此前无论结果如何都宣告「已安全清理 N 个…系统资源已就绪」，用户看到条目消失
        // 便以为已处置，而目标进程其实还在。失败时改用 attention 如实反馈。
        if res.terminatedCount == 0 {
            publish(AgentTaskEvent(
                agentId: "workbench-cleaner",
                agentName: "工作台维护",
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: nil,
                message: "未能终止任何进程",
                detail: "扫描到 \(anomalies.count) 个异常进程，但未能向其中任何一个发送终止信号。可能原因：进程已自行退出、当前权限不足，或 PID 已被系统回收复用。请重新扫描确认当前状态。"
            ))
        } else {
            publish(AgentTaskEvent(
                agentId: "workbench-cleaner",
                agentName: "工作台维护",
                eventType: .completed,
                duration: 0,
                timestamp: Date(),
                pid: nil,
                message: "已安全清理 \(res.terminatedCount) 个异常进程",
                detail: "工作台已成功释放 \(res.terminatedCount) 个孤儿/假死智能体进程，预估回收 \(res.reclaimedMemoryText) 物理内存，系统资源已就绪。"
            ))
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
            self?.sampleInBackground()
        }
        return res
    }

    private func recordTaskCompleted(profile: AgentProfile, since: Date, now: Date, pid: Int32?) {
        let duration = now.timeIntervalSince(since)
        // 持续至少 3.5 秒的实质工作才视作完成一次任务（过滤瞬时微抖动）
        guard duration >= 3.5 else { return }
        durationTracker.record(agentId: profile.id, duration: duration, timestamp: now)
        let timeStr = AgentTaskEvent.durationText(duration)
        publish(AgentTaskEvent(
            agentId: profile.id,
            agentName: profile.name,
            eventType: .completed,
            duration: duration,
            timestamp: now,
            pid: pid,
            detail: "\(profile.name) 本次工作持续 \(timeStr)，所有子步骤已结束，当前任务已完成。"
        ))
    }

    public func clearLatestEvent() {
        latestEvent = nil
        // 用户已确认过告警，解除保护期，后续事件正常展示
        alertProtectedUntil = nil
    }

    public func postEvent(_ event: AgentTaskEvent) {
        publish(event)
    }

    /// 统一事件发布入口：保证严重告警不被普通事件挤掉。
    ///
    /// `latestEvent` 是单槽位，任何新事件都会覆盖旧事件。实测中 costSpike（Token 激增 /
    /// 死循环）横幅在几秒内就被其他 Agent 的「任务完成」顶掉——用户还没来得及看清处置入口，
    /// 告警就消失了。这里给告警一个保护窗口：窗口内的普通事件不覆盖它。
    /// 不排队：横幅只展示一条，把被抑制的普通事件补发出来只会让过期信息再次弹出。
    private func publish(_ event: AgentTaskEvent) {
        var acceptedForBanner = true
        if event.eventType == .costSpike {
            alertProtectedUntil = Date().addingTimeInterval(Self.alertProtectionWindow)
        } else if let until = alertProtectedUntil, Date() < until {
            acceptedForBanner = false   // 告警保护期内，普通横幅让位
        }
        if acceptedForBanner {
            latestEvent = event
        }
        // attention 即便恰逢熔断横幅保护期也必须逐条发系统通知；其余事件维持历史语义，
        // 只有真正进入 latestEvent 的才对外投递。
        if acceptedForBanner || event.eventType == .attention {
            taskEvents.send(event)
        }
    }

    /// 告警保护窗口：足够用户看到横幅并决定是否处置，又不至于长期占位
    static let alertProtectionWindow: TimeInterval = 30
    /// 告警保护截止时间（见 publish）
    private var alertProtectedUntil: Date?

    /// 活跃会话数（离线 agent 直接 0；在线读 FileMonitor 后台扫描缓存，主线程零扫描）
    private func sessionCount(for profile: AgentProfile, running: Bool) -> Int {
        guard running else { return 0 }
        let counts = fileMonitor.activeSessionCounts(for: profile.sessionDirs)
        return counts.values.reduce(0, +)
    }

    /// 节电调度：有 working 快采样，闲置降频，全离线进一步拉大间隔（系统低电量模式自动加码）
    private func scheduleNext() {
        guard running, !isSystemSleeping else { return }   // stop() 或休眠期间不再重建定时器
        timer?.invalidate()
        let interval: TimeInterval
        let isPowerSaving = isLowPowerModeActive
        if anyWorking {
            interval = isPowerSaving ? max(config.sampleInterval, 3.0) : config.sampleInterval
        } else if snapshots.contains(where: { $0.processRunning }) {
            interval = isPowerSaving ? max(config.idleSampleInterval * 2.0, 10.0) : config.idleSampleInterval
        } else {
            interval = isPowerSaving ? max(config.idleSampleInterval * 4.0, 120.0) : max(config.idleSampleInterval, 60.0)   // 全离线
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

    /// 某工具的聚合 Token 用量。来源可读并不要求该工具此刻作为独立进程被监控。
    public func tokenUsage(for agentId: String) -> TokenUsage? {
        tokenMonitor.usage[agentId]
    }

    /// 按模型拆分下钻（详情页）
    public func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void) {
        tokenMonitor.modelBreakdown(agentId: agentId, completion: completion)
    }

    /// 某模型下的会话列表下钻（会话页）
    public func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        tokenMonitor.sessions(agentId: agentId, modelId: modelId, completion: completion)
    }

    /// 全局 Token 时间分析（当前范围 + 上一等长周期 + 数据源构成）。
    public func tokenTimeline(range: TokenTimeRange, now: Date = Date(),
                              completion: @escaping @MainActor (TokenUsageTimeline) -> Void) {
        tokenMonitor.timeline(range: range, now: now, completion: completion)
    }

    /// 获取智能体实时事件流水（后台异步解析，主线程回调）
    public func fetchLogStream(agentId: String, limit: Int = 20, completion: @escaping @MainActor ([AgentLogEvent]) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            let events = AgentLogStreamer.fetchRecentEvents(agentId: agentId, limit: limit)
            Task { @MainActor in
                completion(events)
            }
        }
    }

    /// 可见口径（唯一实现）：仅当前在线（进程仍在）的 Agent。
    /// 历史活动与 token 只用于在线条目的内容展示，不得让已退出 Agent 形成幽灵列表项。
    /// 展开卡片列表、菜单摘要、高度计算统一消费此属性，改口径只改这一处。
    public var visibleSnapshots: [AgentSnapshot] {
        snapshots.filter(\.processRunning)
    }

    /// 顶部活动环微看板的数据源（工作态或 24h 有用量）。
    /// 视图渲染与窗口高度计算共用此口径，避免「视图显示了但高度没算」导致底部汇总栏被裁切。
    public var ringShelfSnapshots: [AgentSnapshot] {
        visibleSnapshots.filter {
            $0.level == .working || $0.level == .attention || ($0.tokenUsage?.tokens24h ?? 0) > 0
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
