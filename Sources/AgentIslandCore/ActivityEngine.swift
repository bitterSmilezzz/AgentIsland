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
    @Published public private(set) var latestEvent: AgentTaskEvent? = nil
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
    }

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
    /// 上次记录的 Token 用量与时间基准（agentId → (timestamp, tokensTotal)），用于速率差分
    private var tokenRateBaseline: [String: (timestamp: Date, tokens: Int)] = [:]
    /// 连续超过速率阈值的评估档数（agentId → 档数）；达到确认档数才告警
    private var tokenSpikeStreak: [String: Int] = [:]
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
        lastSignalAt = lastSignalAt.filter { activeIDs.contains($0.key) }
        highCpuSince = highCpuSince.filter { activeIDs.contains($0.key) }
        lastRunawayAlertedAt = lastRunawayAlertedAt.filter { activeIDs.contains($0.key) }
        tokenSpikeStreak = tokenSpikeStreak.filter { activeIDs.contains($0.key) }
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
    func sampleInBackground() {
        guard running else { return }   // stop() 后在飞 token 刷新不再触发采样
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
        // （离线态最长拖 2 小时），时间戳保证两态刷新粒度一致
        installedApps.refreshIfNeeded(maxAge: 300)

        var results: [AgentSnapshot] = []
        var anyWork = false

        for profile in profiles {
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
                return now.timeIntervalSince(newest)
            }()

            // 动作透传：仅用于展示「正在执行 xxx」。
            // 传入 matcher.snapshot 复用本次采样已枚举的进程表：子进程查找零额外系统调用。
            //
            // 注意：动作不参与工作状态判定。它只是「最近 90s 有过活动」的弱信号，
            // 且各 Agent 的探测源（SQLite 最新行 / 日志尾部）在 Agent 空闲挂起时依然可能
            // 命中旧记录，一旦当作工作信号会让 Agent 恒显「工作中」并永不产生完成事件。
            // 工作状态只由「文件写入 + CPU」这两个可观测信号决定。
            let detectedAction = running ? AgentActionInspector.inspectAction(
                pid: matchedPID, profile: profile, sessionDirs: profile.sessionDirs,
                snapshot: matcher.snapshot) : nil

            let hasRecentWrite = newestAgo.map { $0 <= config.workingWindow } ?? false

            // 智能 CPU 判定：
            // 对于桌面/GUI 智能体（如 DimAgent、WorkBuddy、ChatGPT，或声明了 bundleIDs 的桌面应用）：
            // 过滤 Electron / Chromium / WebKit 辅助进程渲染与 IPC 空闲微抖动（4%~15%），仅当 CPU 达真正高算力（>= 20.0%）才触发 working；
            // 纯 CLI 智能体维持通用 cpuThreshold（默认 6.0%）。
            let hasHighCpu: Bool
            if profile.id == "dim" || profile.id == "workbuddy" || profile.id == "chatgpt" || !profile.bundleIDs.isEmpty {
                hasHighCpu = cpu >= 20.0
            } else {
                hasHighCpu = cpu > config.cpuThreshold
            }

            let level: ActivityLevel
            if !running {
                // 进程消失 = Agent 被关闭/退出，不是任务完成：静默转 offline，不发完成事件。
                // （此前会误报「任务已完成」——完成事件只应由「进程仍在但工作信号消失」产生）
                level = .offline
                workingSince[profile.id] = nil
                lastSignalAt[profile.id] = nil
                highCpuSince[profile.id] = nil
                lastRunawayAlertedAt[profile.id] = nil
                tokenSpikeStreak[profile.id] = nil
            } else if hasRecentWrite || hasHighCpu {
                level = .working
                if workingSince[profile.id] == nil { workingSince[profile.id] = now }
                lastSignalAt[profile.id] = now
            } else if let lastSignal = lastSignalAt[profile.id],
                      now.timeIntervalSince(lastSignal) < config.minWorkingHold {
                // 滞回：信号刚消失时保持 working 最短时长，防 CPU 临界抖动导致 peek 高频弹跳。
                // 锚点必须是「最后一次有信号」的时刻而非首次进入 working 的时刻——
                // 用 workingSince 会让任何超过 minWorkingHold 的任务滞回完全失效。
                level = .working
            } else {
                level = .idle
                if let since = workingSince[profile.id] {
                    recordTaskCompleted(profile: profile, since: since, now: now, pid: matchedPID)
                }
                workingSince[profile.id] = nil
                lastSignalAt[profile.id] = nil
                highCpuSince[profile.id] = nil
            }

            let action: String?
            if level == .working {
                anyWork = true
                action = detectedAction
            } else {
                action = nil
            }

            let memory = entries.reduce(UInt64(0)) { $0 + $1.rssBytes }
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
                tokenUsage: tokenMonitor.usage[profile.id],
                pid: matchedPID,
                currentAction: action,
                memoryBytes: memory,
                isHung: isHung
            ))
        }

        // 内容实质变化才发布（@Published 触发所有观察者重算）
        if results != snapshots || anyWork != anyWorking {
            snapshots = results
            anyWorking = anyWork
            updatedAt = now
        }

        // 成本与异常熔断保护：检测 Token 激增与死循环运行
        checkCostSpikeAndRunaway(matcher: matcher, now: now)

        scheduleNext()
        return results
    }

    private func checkCostSpikeAndRunaway(matcher: ProcessMatcher, now: Date) {
        if config.tokenAlertEnabled {
            for profile in profiles {
                guard let usage = tokenMonitor.usage[profile.id], usage.tokensTotal > 0 else { continue }
                guard let base = tokenRateBaseline[profile.id] else {
                    tokenRateBaseline[profile.id] = (timestamp: now, tokens: usage.tokensTotal)
                    continue
                }
                let timeSpan = now.timeIntervalSince(base.timestamp)
                // 不足一档不结算：一次采样就把长任务的账本落盘当激增会误报
                guard timeSpan >= Self.tokenRateWindow else { continue }
                let deltaTokens = usage.tokensTotal - base.tokens
                tokenRateBaseline[profile.id] = (timestamp: now, tokens: usage.tokensTotal)
                // 折算为每分钟速率：正常绘画/长任务摊到多档后低于阈值，不再触发
                let tokensPerMinute = Double(deltaTokens) / timeSpan * 60.0
                if deltaTokens > 0, tokensPerMinute >= Double(config.tokenAlertThreshold) {
                    let streak = (tokenSpikeStreak[profile.id] ?? 0) + 1
                    tokenSpikeStreak[profile.id] = streak
                    // 需连续多档超阈值才告警：滤掉单次账本补写（如长任务结束时一次性落盘）
                    guard streak >= Self.tokenSpikeConfirmations else { continue }
                    let matchedPID = matcher.matchingEntries(for: profile).first(where: { $0.pid > 0 })?.pid
                    postEvent(AgentTaskEvent(
                        agentId: profile.id,
                        agentName: profile.name,
                        eventType: .costSpike,
                        duration: timeSpan * Double(streak),
                        timestamp: now,
                        pid: matchedPID,
                        message: "⚠️ \(profile.name) Token 激增 (+\(TokenUsage.compact(deltaTokens)))",
                        detail: "近 \(Int(timeSpan)) 秒 Token 净消耗 +\(TokenUsage.compact(deltaTokens))，约 \(TokenUsage.compact(Int(tokensPerMinute)))/分钟，已连续 \(streak) 个周期超过阈值（设置报警阈值: \(TokenUsage.compact(config.tokenAlertThreshold))/分钟）。常见原因：长上下文灌入、复杂循环或多 Agent 并发。建议点击直达检查会话状态。"
                    ))
                    tokenSpikeStreak[profile.id] = 0   // 告警后重置，避免每档重复轰炸
                } else {
                    tokenSpikeStreak[profile.id] = 0
                }
            }
        }

        if config.runawayCpuAlert {
            for profile in profiles {
                let cpu = matcher.cpuPercent(profile)
                if cpu >= config.runawayCpuThreshold {
                    let start = highCpuSince[profile.id] ?? now
                    if highCpuSince[profile.id] == nil {
                        highCpuSince[profile.id] = now
                    }
                    let highDuration = now.timeIntervalSince(start)
                    if highDuration >= config.runawayDurationThreshold {
                        let lastAlert = lastRunawayAlertedAt[profile.id]
                        if lastAlert == nil || now.timeIntervalSince(lastAlert!) >= config.runawayDurationThreshold {
                            lastRunawayAlertedAt[profile.id] = now
                            let matchedPID = matcher.matchingEntries(for: profile).first(where: { $0.pid > 0 })?.pid
                            let minutes = max(1, Int(highDuration / 60))
                            postEvent(AgentTaskEvent(
                                agentId: profile.id,
                                agentName: profile.name,
                                eventType: .costSpike,
                                duration: highDuration,
                                timestamp: now,
                                pid: matchedPID,
                                message: "⚠️ \(profile.name) 持续高负载超 \(minutes) 分钟 (CPU \(Int(cpu))%)",
                                detail: "进程持续高负载占用 CPU \(Int(cpu))% 已达 \(minutes) 分钟（报警阈值: ≥\(Int(config.runawayCpuThreshold))% 持续超 \(Int(config.runawayDurationThreshold / 60)) 分钟）。若任务卡死或非预期，可点击【熔断】安全结束。"
                            ))
                        }
                    }
                } else {
                    highCpuSince[profile.id] = nil
                    lastRunawayAlertedAt[profile.id] = nil
                }
            }
        } else {
            highCpuSince.removeAll()
            lastRunawayAlertedAt.removeAll()
        }
    }

    /// 终止智能体进程逃生舱：关闭目标 Agent 及其子进程，并更新状态
    /// - Returns: 是否真正发出了终止信号。pid 缺失时返回 false 且不发送成功事件，
    ///   避免「假成功」——此前无论有无 pid 都宣告「进程已终止」，用户以为已熔断，
    ///   实际进程仍在烧 token（GUI bundle 命中但进程名未匹配时 pid 为 nil）。
    @discardableResult
    public func terminateAgent(pid: Int32?, agentId: String) -> Bool {
        let name = profiles.first(where: { $0.id == agentId })?.name ?? agentId
        guard let pid, pid > 1 else {
            latestEvent = AgentTaskEvent(
                agentId: agentId,
                agentName: name,
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: nil,
                message: "无法终止 \(name)：未定位到进程",
                detail: "检测到 \(name) 处于活动状态，但未能匹配到可终止的进程 PID（可能是 Electron 辅助进程或权限受限）。请从菜单栏图标或活动监视器手动处理。"
            )
            return false
        }
        // 消费 terminate 的真实结果：无权限 / 进程已消失时 kill 返回非 0，
        // 不能一律宣告成功（否则「假成功」只修了 pid 缺失那一半）
        guard ProcessTerminator.terminate(pid: pid) else {
            latestEvent = AgentTaskEvent(
                agentId: agentId,
                agentName: name,
                eventType: .attention,
                duration: 0,
                timestamp: Date(),
                pid: pid,
                message: "无法终止 \(name)：信号发送失败",
                detail: "已尝试终止 PID \(pid)，但未能向其发送信号（进程可能已退出，或需要更高权限）。请确认进程状态或在活动监视器中处理。"
            )
            return false
        }
        workingSince[agentId] = nil
        lastSignalAt[agentId] = nil
        highCpuSince[agentId] = nil
        lastRunawayAlertedAt[agentId] = nil
        tokenSpikeStreak[agentId] = nil
        latestEvent = AgentTaskEvent(
            agentId: agentId,
            agentName: name,
            // 终止成功是「已完成」而非「需关注」：用 attention 会让收起态细条误报红色告警
            eventType: .completed,
            duration: 0,
            timestamp: Date(),
            pid: pid,
            message: "\(name) 进程已终止",
            detail: "已向 PID \(pid) 及其关联子进程发送 SIGTERM/SIGKILL 终止信号，系统资源已释放。"
        )
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            self?.sample()
        }
        return true
    }

    /// 智能体工作台一键清理：安全清理指定的异常/孤儿进程并展示清理横幅
    public func cleanAnomalies(_ anomalies: [AgentAnomaly]) {
        guard !anomalies.isEmpty else { return }
        let res = cleaner.clean(anomalies: anomalies)
        for a in anomalies {
            workingSince[a.profileId] = nil
            lastSignalAt[a.profileId] = nil
            highCpuSince[a.profileId] = nil
            lastRunawayAlertedAt[a.profileId] = nil
            tokenSpikeStreak[a.profileId] = nil
        }
        latestEvent = AgentTaskEvent(
            agentId: "workbench-cleaner",
            agentName: "工作台维护",
            eventType: .completed,
            duration: 0,
            timestamp: Date(),
            pid: nil,
            message: "已安全清理 \(res.terminatedCount) 个异常进程",
            detail: "工作台已成功释放 \(res.terminatedCount) 个孤儿/假死智能体进程，预估回收 \(res.reclaimedMemoryText) 物理内存，系统资源已就绪。"
        )
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
            self?.sample()
        }
    }

    private func recordTaskCompleted(profile: AgentProfile, since: Date, now: Date, pid: Int32?) {
        let duration = now.timeIntervalSince(since)
        // 持续至少 3.5 秒的实质工作才视作完成一次任务（过滤瞬时微抖动）
        guard duration >= 3.5 else { return }
        let durationText = duration >= 60 ? "\(Int(duration / 60))分\(Int(duration) % 60)秒" : "\(Int(duration))秒"
        latestEvent = AgentTaskEvent(
            agentId: profile.id,
            agentName: profile.name,
            eventType: .completed,
            duration: duration,
            timestamp: now,
            pid: pid,
            detail: "\(profile.name) 本次工作持续 \(durationText)，所有子步骤已完成，现已转为空闲状态。"
        )
    }

    public func clearLatestEvent() {
        latestEvent = nil
    }

    public func postEvent(_ event: AgentTaskEvent) {
        latestEvent = event
    }

    /// 活跃会话数（离线 agent 直接 0；在线读 FileMonitor 后台扫描缓存，主线程零扫描）
    private func sessionCount(for profile: AgentProfile, running: Bool) -> Int {
        guard running else { return 0 }
        let counts = fileMonitor.activeSessionCounts(for: profile.sessionDirs)
        return counts.values.reduce(0, +)
    }

    /// 节电调度：有 working 快采样，闲置降频，全离线进一步拉大间隔（无 UI 需求）
    private func scheduleNext() {
        guard running else { return }   // stop() 后在飞回调不再重建定时器
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

    /// 获取智能体实时事件流水（后台异步解析，主线程回调）
    public func fetchLogStream(agentId: String, limit: Int = 20, completion: @escaping @MainActor ([AgentLogEvent]) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            let events = AgentLogStreamer.fetchRecentEvents(agentId: agentId, limit: limit)
            Task { @MainActor in
                completion(events)
            }
        }
    }

    /// 可见口径（唯一实现）：在线 + 24h 内活跃。
    /// 展开卡片列表、菜单摘要、高度计算统一消费此属性，改口径只改这一处。
    public var visibleSnapshots: [AgentSnapshot] {
        snapshots.filter {
            $0.processRunning || ($0.lastActivityAgo ?? .infinity) < 24 * 3600
        }
    }

    /// 顶部活动环微看板的数据源（工作态或 24h 有用量）。
    /// 视图渲染与窗口高度计算共用此口径，避免「视图显示了但高度没算」导致底部汇总栏被裁切。
    public var ringShelfSnapshots: [AgentSnapshot] {
        visibleSnapshots.filter { $0.level == .working || ($0.tokenUsage?.tokens24h ?? 0) > 0 }
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
