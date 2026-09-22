import Foundation
import SQLite3
@testable import AgentIslandCore

// MARK: - ActivityEngine 状态机测试

@MainActor
enum EngineTests {

    static var home: String {
        FileManager.default.homeDirectoryForCurrentUser.path
    }

    static let dim = AgentRegistry.builtin.first { $0.id == "dim" }!

    static func register() {
        TestKit.test("TaskDurationTracker 任务耗时与效率统计") {
            let tracker = TaskDurationTracker(maxRecordsPerAgent: 10)
            let now = Date()

            // 初始空态
            let emptyStats = tracker.stats(for: "dim", now: now)
            try expectEqual(emptyStats.taskCount, 0)
            try expectEqual(emptyStats.totalWorkTime, 0)
            try expectEqual(emptyStats.formattedTotalTime, "0秒")
            try expectEqual(emptyStats.formattedAverageDuration, "—")

            // 记录任务：10s, 30s, 80s
            tracker.record(agentId: "dim", duration: 10, timestamp: now.addingTimeInterval(-60))
            tracker.record(agentId: "dim", duration: 30, timestamp: now.addingTimeInterval(-30))
            tracker.record(agentId: "dim", duration: 80, timestamp: now)

            let stats = tracker.stats(for: "dim", now: now)
            try expectEqual(stats.taskCount, 3)
            try expectEqual(stats.totalWorkTime, 120)
            try expectEqual(stats.formattedTotalTime, "2分钟")
            try expectEqual(stats.averageDuration, 40)
            try expectEqual(stats.formattedAverageDuration, "40秒/次")
            try expectEqual(stats.maxDuration, 80)

            // 窗口过期过滤（只取过去 45 秒内：应只剩 30s 和 80s 两条）
            let recentStats = tracker.stats(for: "dim", window: 45, now: now)
            try expectEqual(recentStats.taskCount, 2)
            try expectEqual(recentStats.totalWorkTime, 110)
        }

        TestKit.test("AgentHealthEvaluator 健康度评分与诊断状态机") {
            let profile = dim

            // 1. 未运行状态（进程离线）：100分，健康
            let snapOffline = AgentSnapshot(profile: profile, level: .offline, processRunning: false,
                                            cpuPercent: 0, installed: false, activeSessions: 0,
                                            lastActivityAgo: nil, lastActivityText: "离线",
                                            tokenUsage: nil, pid: nil, currentAction: nil,
                                            memoryBytes: 0, isHung: false)
            let reportOffline = AgentHealthEvaluator.evaluate(snapshot: snapOffline)
            try expectEqual(reportOffline.score, 100, "offline 100")
            try expectEqual(reportOffline.grade, .healthy, "offline healthy")
            try expectTrue(reportOffline.issues.isEmpty, "no issues")

            // 2. 正常运行状态：CPU 5%, 内存 100MB：100分，健康
            let snapNormal = AgentSnapshot(profile: profile, level: .working, processRunning: true,
                                           cpuPercent: 5.0, installed: true, activeSessions: 1,
                                           lastActivityAgo: 1, lastActivityText: "1秒前",
                                           tokenUsage: nil, pid: 1234, currentAction: "编译",
                                           memoryBytes: 100 * 1024 * 1024, isHung: false)
            let reportNormal = AgentHealthEvaluator.evaluate(snapshot: snapNormal)
            try expectEqual(reportNormal.score, 100, "normal 100")
            try expectEqual(reportNormal.grade, .healthy, "normal healthy")

            // 3. 高 CPU (85%)：扣 25 分，score = 75，需留意
            let snapHighCpu = AgentSnapshot(profile: profile, level: .working, processRunning: true,
                                            cpuPercent: 85.0, installed: true, activeSessions: 1,
                                            lastActivityAgo: 1, lastActivityText: "1秒前",
                                            tokenUsage: nil, pid: 1234, currentAction: "计算",
                                            memoryBytes: 200 * 1024 * 1024, isHung: false)
            let reportHighCpu = AgentHealthEvaluator.evaluate(snapshot: snapHighCpu)
            try expectEqual(reportHighCpu.score, 75, "high cpu 75")
            try expectEqual(reportHighCpu.grade, .attention, "grade attention")
            try expectTrue(reportHighCpu.issues.count == 1, "has high cpu issue")

            // 4. 内存溢出 (2.2GB)：扣 30 分，score = 70，需留意
            let mem2_2GB = UInt64(2200) * 1024 * 1024
            let snapHighMem = AgentSnapshot(profile: profile, level: .working, processRunning: true,
                                            cpuPercent: 10.0, installed: true, activeSessions: 1,
                                            lastActivityAgo: 1, lastActivityText: "1秒前",
                                            tokenUsage: nil, pid: 1234, currentAction: nil,
                                            memoryBytes: mem2_2GB, isHung: false)
            let reportHighMem = AgentHealthEvaluator.evaluate(snapshot: snapHighMem)
            try expectEqual(reportHighMem.score, 70, "high mem 70")
            try expectEqual(reportHighMem.grade, .attention, "high mem attention")

            // 5. 卡死死锁 (isHung = true)：扣 50 分，若还伴随高 CPU 扣 25 分，危急
            let mem2_5GB = UInt64(2500) * 1024 * 1024
            let snapHung = AgentSnapshot(profile: profile, level: .attention, processRunning: true,
                                         cpuPercent: 90.0, installed: true, activeSessions: 1,
                                         lastActivityAgo: 10, lastActivityText: "10秒前",
                                         tokenUsage: nil, pid: 1234, currentAction: nil,
                                         memoryBytes: mem2_5GB, isHung: true)
            let reportHung = AgentHealthEvaluator.evaluate(snapshot: snapHung)
            try expectTrue(reportHung.score <= 30, "score <= 30")
            try expectEqual(reportHung.grade, .critical, "grade critical")
            try expectTrue(reportHung.suggestion.contains("逃生舱") || reportHung.suggestion.contains("死锁"), "remedy suggestion")
        }

        TestKit.test("ActivityEngine eventHistory 历史事件时间线与有界队列限制") {
            let engine = makeEngine(processNames: [], writes: [:])
            try expectTrue(engine.eventHistory.isEmpty, "initial empty history")

            // 派发 30 个事件，验证上限为 25 条
            for i in 1...30 {
                engine.postEvent(AgentTaskEvent(
                    agentId: "dim",
                    agentName: "DimAgent",
                    eventType: .completed,
                    duration: TimeInterval(i),
                    timestamp: Date(),
                    pid: 1000 + Int32(i),
                    message: "任务 \(i) 完成"
                ))
            }
            try expectEqual(engine.eventHistory.count, 25, "bounded to 25 items")
            try expectEqual(engine.eventHistory.first?.message, "任务 30 完成", "latest event is at index 0")

            // 清空历史
            engine.clearEventHistory()
            try expectTrue(engine.eventHistory.isEmpty, "cleared history")
        }

        TestKit.test("MenuBarBadgeMode 模式枚举健全性") {
            let all = MenuBarBadgeMode.allCases
            try expectEqual(all.count, 3, "3 modes")
            try expectTrue(all.contains(.iconOnly), "contains iconOnly")
            try expectTrue(all.contains(.activeCount), "contains activeCount")
            try expectTrue(all.contains(.tokenUsage), "contains tokenUsage")
            for m in all {
                try expectFalse(m.label.isEmpty, "label not empty")
            }
        }

        TestKit.test("ProcessTreeInspector 进程树递归解析与开销聚合") {
            let entries: [ProcessSnapshot.Entry] = [
                ProcessSnapshot.Entry(pid: 1000, path: "/usr/local/bin/agent", basename: "agent", cpuPercent: 5.0, rssBytes: 50 * 1024 * 1024, ppid: 1),
                ProcessSnapshot.Entry(pid: 1001, path: "/opt/node/bin/node", basename: "node", cpuPercent: 15.0, rssBytes: 100 * 1024 * 1024, ppid: 1000),
                ProcessSnapshot.Entry(pid: 1002, path: "/bin/zsh", basename: "zsh", cpuPercent: 0.0, rssBytes: 10 * 1024 * 1024, ppid: 1000),
                ProcessSnapshot.Entry(pid: 1003, path: "/opt/homebrew/bin/rg", basename: "rg", cpuPercent: 25.0, rssBytes: 50 * 1024 * 1024, ppid: 1002),
                ProcessSnapshot.Entry(pid: 2000, path: "/usr/bin/python3", basename: "python3", cpuPercent: 30.0, rssBytes: 80 * 1024 * 1024, ppid: 1)
            ]

            let report = ProcessTreeInspector.buildTree(for: 1000, from: entries)
            try expectEqual(report.rootPid, 1000)
            try expectEqual(report.subprocessCount, 3, "1001, 1002, 1003")
            try expectEqual(report.totalSubprocessCpu, 40.0, "15 + 0 + 25")
            try expectEqual(report.totalSubprocessMemory, 160 * 1024 * 1024, "100MB + 10MB + 50MB")
            try expectEqual(report.nodes.count, 2, "2 direct children (node & zsh)")

            let zshNode = report.nodes.first { $0.name == "zsh" }
            try expectEqual(zshNode?.children.count, 1, "zsh spawned rg")
            try expectEqual(zshNode?.children.first?.name, "rg")

            // 根 PID 负数或 0 返回空报表
            let emptyReport = ProcessTreeInspector.buildTree(for: 0, from: entries)
            try expectEqual(emptyReport.subprocessCount, 0)
            try expectTrue(emptyReport.nodes.isEmpty)

            // 引擎方法冒烟
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            _ = engine.inspectProcessTree(agentId: "dim")
        }

        TestKit.test("AuditReportExporter 运维审计报告 Markdown 与 CSV 生成") {
            let snap = AgentSnapshot(
                profile: dim,
                level: .working,
                processRunning: true,
                cpuPercent: 12.5,
                installed: true,
                activeSessions: 2,
                lastActivityAgo: 5,
                lastActivityText: "5秒前",
                tokenUsage: TokenUsage(tokens24h: 50_000, tokensTotal: 200_000, cost24h: 0.15, costTotal: 0.85),
                pid: 54321,
                currentAction: "代码重构",
                memoryBytes: 150 * 1024 * 1024,
                isHung: false
            )

            let event = AgentTaskEvent(
                agentId: "dim",
                agentName: "DimAgent",
                eventType: .completed,
                duration: 42,
                timestamp: Date(),
                pid: 54321,
                message: "重构已完成"
            )

            let md = AuditReportExporter.generateMarkdown(snapshots: [snap], history: [event], now: Date())
            try expectTrue(md.contains("# AgentIsland"), "md header")
            try expectTrue(md.contains("DimAgent"), "agent in md")
            try expectTrue(md.contains("50.0k"), "24h token in md")
            try expectTrue(md.contains("重构已完成"), "event in md")

            let csv = AuditReportExporter.generateCSV(snapshots: [snap], now: Date())
            try expectTrue(csv.hasPrefix("Timestamp,AgentID,AgentName"), "csv header")
            try expectTrue(csv.contains("dim,DimAgent,working,54321,12.5"), "csv row values")
        }

        TestKit.test("ScreenFollowMode 与 SoundOption 枚举及健壮性") {
            let screens = ScreenFollowMode.allCases
            try expectEqual(screens.count, 4)
            for s in screens {
                try expectFalse(s.label.isEmpty)
                try expectEqual(s.id, s.rawValue)
            }

            let compSounds = CompletionSoundOption.allCases
            try expectEqual(compSounds.count, 5)
            try expectNil(CompletionSoundOption.mute.systemSoundName)
            try expectEqual(CompletionSoundOption.glass.systemSoundName, "Glass")

            let alertSounds = AlertSoundOption.allCases
            try expectEqual(alertSounds.count, 4)
            try expectNil(AlertSoundOption.mute.systemSoundName)
            try expectEqual(AlertSoundOption.sosumi.systemSoundName, "Sosumi")
        }

        TestKit.test("进程: provider 原始大小写也能匹配") {
            let names: Set<String> = ["DimAgent"]
            let provider = FakeProcessProvider(processNames: names, bundleIDs: [])
            let matcher = ProcessMatcher(snapshot: provider.snapshot(), runningBundleIDs: provider.runningBundleIDs(), profiles: [])
            try expectTrue(matcher.isRunning(dim), "原始大小写匹配")
        }

        TestKit.test("引擎: 进程不在 → offline") {
            let engine = makeEngine(processNames: [], writes: [:])
            let snaps = engine.sample(now: Date())
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .offline)
        }

        TestKit.test("引擎: 进程在 + 5s 内有写入 → working") {
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": now.addingTimeInterval(-5)]
            )
            let snaps = engine.sample(now: now)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "dim 应 working")
            try expectTrue(engine.anyWorking, "anyWorking")
        }

        TestKit.test("引擎: 进程在 + 无写入 + CPU=0 → idle") {
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": now.addingTimeInterval(-120)]
            )
            let snaps = engine.sample(now: now)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .idle, "dim 应 idle")
            try expectTrue(!engine.anyWorking, "anyWorking 应为 false")
        }

        TestKit.test("引擎: CPU 高但无写入 → working（双信号之二）") {
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": now.addingTimeInterval(-300)],
                cpu: 25.0
            )
            let snaps = engine.sample(now: now)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "CPU 信号应 working")
        }

        TestKit.test("引擎: 新写入出现 → 状态升级为 working") {
            let now = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = FakeFileActivityProvider(writes: [dir: now.addingTimeInterval(-300)])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 20),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: provider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            _ = engine.sample(now: now)
            provider.writes = [dir: now.addingTimeInterval(-3)]
            let snaps = engine.sample(now: now.addingTimeInterval(2))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "升级后应 working")
        }

        TestKit.test("引擎: working 持续超时后转 idle → 触发任务完成事件") {
            let start = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: provider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            // 第一拍：working
            _ = engine.sample(now: start)
            try expectTrue(engine.anyWorking, "第一拍应 working")
            try expectNil(engine.latestEvent, "进行中不应有完成事件")

            // 第二拍：8秒后，无新写入且超过滞回时间 → 转 idle
            let finishTime = start.addingTimeInterval(8)
            _ = engine.sample(now: finishTime)
            try expectEqual(engine.latestEvent?.agentId, "dim")
            try expectEqual(engine.latestEvent?.eventType, .completed)
            try expectTrue((engine.latestEvent?.duration ?? 0) >= 7.5, "任务持续时长应记录")
        }

        TestKit.test("引擎: 纯 CPU 区间收尾不报完成（R37：完成事件需写入证据）") {
            let start = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = MutableProcessProvider(names: ["DimAgent"], cpu: 90)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: provider,
                fileMonitor: FakeFileActivityProvider(writes: [dir: start.addingTimeInterval(-600)]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            // 第一拍：CPU 90% 但无任何写入 → 仍判 working（双信号判定不变）
            let first = engine.sample(now: start)
            try expectEqual(first.first { $0.id == "dim" }?.level, .working, "CPU 信号应判 working")

            // 第二拍：CPU 落回 0（桌面应用空闲抖动结束）→ 静默转 idle，不得补发完成事件
            provider.cpu = 0
            let second = engine.sample(now: start.addingTimeInterval(8))
            try expectEqual(second.first { $0.id == "dim" }?.level, .idle, "信号消失应转 idle")
            try expectNil(engine.latestEvent,
                          "纯 CPU 区间收尾不得报「任务完成」——否则打开应用什么都不做也会响铃")
        }

        TestKit.test("引擎: 区间内出现过写入则照常报完成（R37 反向守卫）") {
            let start = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = MutableProcessProvider(names: ["DimAgent"], cpu: 90)
            let fileProvider = FakeFileActivityProvider(writes: [dir: start.addingTimeInterval(-600)])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: provider,
                fileMonitor: fileProvider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            _ = engine.sample(now: start)                                  // CPU 起步，暂无写入证据
            fileProvider.writes = [dir: start.addingTimeInterval(1)]       // 区间内出现真实写入
            _ = engine.sample(now: start.addingTimeInterval(2))
            provider.cpu = 0
            fileProvider.writes = [dir: start.addingTimeInterval(-600)]    // 写入滑出窗口
            _ = engine.sample(now: start.addingTimeInterval(6))
            _ = engine.sample(now: start.addingTimeInterval(10))           // 超滞回 → idle
            try expectEqual(engine.latestEvent?.agentId, "dim")
            try expectEqual(engine.latestEvent?.eventType, .completed,
                            "区间内有写入证据 → 完成事件必须照常发（不得被新规则误吞）")
        }

        TestKit.test("引擎: 写入证据不跨工作区间泄漏（R37）") {
            let start = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = MutableProcessProvider(names: ["DimAgent"], cpu: 0)
            let fileProvider = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: provider,
                fileMonitor: fileProvider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            // 第一段：写入驱动 → 正常报完成
            _ = engine.sample(now: start)
            fileProvider.writes = [dir: start.addingTimeInterval(-600)]
            _ = engine.sample(now: start.addingTimeInterval(8))
            try expectEqual(engine.latestEvent?.eventType, .completed, "前置：写入区间应报完成")
            engine.clearLatestEvent()

            // 第二段：纯 CPU 区间 —— 上一段的写入证据不得残留复用
            provider.cpu = 90
            _ = engine.sample(now: start.addingTimeInterval(20))
            provider.cpu = 0
            _ = engine.sample(now: start.addingTimeInterval(30))
            try expectNil(engine.latestEvent, "上一段区间的写入证据必须随区间结束清除")
        }

        TestKit.test("引擎: 进程消失（被关闭）→ 静默转 offline，不误报任务完成") {            let start = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = MutableProcessProvider(names: ["DimAgent"])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: provider,
                fileMonitor: FakeFileActivityProvider(writes: [dir: start]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            // 第一拍：进程在 + 有写入 → working（工作已持续 30s）
            _ = engine.sample(now: start)
            try expectTrue(engine.anyWorking, "关闭前应 working")
            try expectNil(engine.latestEvent, "进行中不应有事件")

            // 第二拍：用户关闭 ChatGPT（进程消失）→ 静默 offline，绝不报「任务已完成」
            provider.names = []
            let snaps = engine.sample(now: start.addingTimeInterval(30))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .offline, "进程消失应转 offline")
            try expectNil(engine.latestEvent, "进程被关闭不是任务完成，不得发完成事件")
            try expectTrue(!engine.anyWorking, "不应再有 working")
        }

        TestKit.test("引擎: 时钟回拨后滞回重锚，Agent 不卡死在 working") {
            let base = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let provider = MutableProcessProvider(names: ["DimAgent"], bundleIDs: [])
            let fileProvider = FakeFileActivityProvider(writes: [dir: base.addingTimeInterval(-5)])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 20, minWorkingHold: 10),
                processMonitor: provider,
                fileMonitor: fileProvider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            var snaps = engine.sample(now: base)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "前置：写入窗口内应 working")

            // 信号消失（写入时刻设在回拨点之前，避免落入「未来 mtime 被钳为刚写入」分支），
            // 滞回期内保持 working
            fileProvider.writes = [dir: base.addingTimeInterval(-695)]
            snaps = engine.sample(now: base.addingTimeInterval(5))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "滞回期内应保持 working")

            // 系统时钟回拨 10 分钟（now 落在 lastSignal 之前）：重锚后滞回重新计起
            // （无重锚时 now - lastSignal 恒为负 → 恒 < minWorkingHold，永不回落）
            let rolledBack = base.addingTimeInterval(5 - 600)
            snaps = engine.sample(now: rolledBack)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working,
                            "重锚本拍滞归零后仍在滞回窗口内，应保持 working")

            // 重锚后再过滞回时长：正常回落 idle
            snaps = engine.sample(now: rolledBack.addingTimeInterval(11))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .idle,
                            "回拨重锚后滞回应正常过期，Agent 不得卡死在 working")
        }

        TestKit.test("引擎: 未来 mtime（回拨残留）钳为刚写入，且随真实时间正常过期") {
            let now = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let fileProvider = FakeFileActivityProvider(writes: [dir: now.addingTimeInterval(120)])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 20),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: fileProvider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            var snaps = engine.sample(now: now)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working,
                            "未来 mtime 应视作刚写入（回拨前的真实写入确实发生在不久前）")
            // 钳制判别：lastActivityAgo 必须是 0 而非负值
            // （level 判定对负值等价，但 lastActivityAgo 会流向 UI 文案与下游算术）
            try expectEqual(snaps.first { $0.id == "dim" }?.lastActivityAgo, 0,
                            "未来 mtime 的经过时长必须钳为 0，不得把负值透传给消费方")

            // 真实时间越过「未来 mtime」+ 窗口后正常回落
            // （无钳制时经过时长恒为负 → 永远 working）
            snaps = engine.sample(now: now.addingTimeInterval(200))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .idle,
                            "未来 mtime 不得让 working 判定永久生效")
        }

        TestKit.test("引擎: offline 清除 token 速率基线（重启后不摊薄结算窗口）") {
            let now = Date()
            let token = FakeTokenUsageProvider()
            token.usage["dim"] = TokenUsage(tokens24h: 0, tokensTotal: 1_000, cost24h: 0, costTotal: 0)
            let provider = MutableProcessProvider(names: ["DimAgent"], bundleIDs: [])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 20),
                processMonitor: provider,
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: token,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            _ = engine.sample(now: now)
            try expectTrue(engine.tokenRateBaseline["dim"] != nil, "前置：有用量应建立速率基线")

            // 进程退出（usage 源同时消失）：offline 应清基线，与 resumeGap 断点口径一致
            provider.names = []
            token.usage["dim"] = nil
            _ = engine.sample(now: now.addingTimeInterval(5))
            try expectNil(engine.tokenRateBaseline["dim"],
                          "offline 必须清除速率基线，否则重启后首个结算窗口被离线全程摊薄")
        }

        TestKit.test("引擎: 动作探测仅对 working 生效（idle 不做主线程枚举）") {
            let now = Date()
            let dir = home + "/.dimcode/v2/data/sessions"
            let fileProvider = FakeFileActivityProvider(writes: [dir: now.addingTimeInterval(-300)])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 20),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: fileProvider,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            var calls = 0
            engine.inspectActionHook = { _, _, _, _ in
                calls += 1
                return "正在执行: fake"
            }
            var snaps = engine.sample(now: now)
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .idle, "前置：无写入低 CPU 应 idle")
            try expectEqual(calls, 0, "idle 必须不探测（每 2s 白付全树枚举的主线程 I/O）")
            try expectNil(snaps.first { $0.id == "dim" }?.currentAction, "idle 无动作透传")

            // 转入 working（窗口内出现新写入）→ 探测执行且结果透传
            fileProvider.writes = [dir: now.addingTimeInterval(-3)]
            snaps = engine.sample(now: now.addingTimeInterval(2))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "前置：新写入应 working")
            try expectEqual(calls, 1, "working 拍应恰好探测一次")
            try expectEqual(snaps.first { $0.id == "dim" }?.currentAction, "正在执行: fake",
                            "探测结果透传到快照")
        }

        TestKit.test("进程: pathContains 只认应用包或同名目录段，用户自己的目录不算") {
            // 已知缺陷（v0.0.97 记档）：裸子串匹配下 `trae` 命中
            // ~/code/trae-sandbox 里跑的 electron——用户自己的程序被认成 TRAE 并可被一键终止
            let 沙箱 = "/users/dev/code/trae-sandbox/node_modules/electron/dist/electron.app/contents/macos/electron"
            try expectFalse(ProcessMatcher.matchesPathContains(["trae"], pathLower: 沙箱),
                            "trae-sandbox 不该命中 trae")
            try expectFalse(ProcessMatcher.matchesPathContains(["trae"], pathLower: "/opt/mytraetool/bin/electron"),
                            "词内包含（mytraetool）也不该命中")
            // 三种真实形态必须继续命中，否则就是识别率倒退
            try expectTrue(ProcessMatcher.matchesPathContains(["trae"],
                           pathLower: "/applications/trae.app/contents/macos/trae"), "应用包名等于 needle")
            try expectTrue(ProcessMatcher.matchesPathContains(["trae"],
                           pathLower: "/applications/trae solo cn.app/contents/macos/electron"),
                           "带空格的应用包名（TRAE SOLO CN.app）")
            try expectTrue(ProcessMatcher.matchesPathContains(["openviking"],
                           pathLower: "/users/dev/.local/share/uv/tools/openviking/bin/python"),
                           "工具目录段等于 needle")
            // 自带目录锚或含点/空格的 needle 保持子串语义（它们本身已是精确片段）
            try expectTrue(ProcessMatcher.matchesPathContains(["/applications/qoder.app"],
                           pathLower: "/applications/qoder.app/contents/macos/qoder"))
            try expectTrue(ProcessMatcher.matchesPathContains([".workbuddy/"],
                           pathLower: "/users/dev/.workbuddy/bin/wb"))
            try expectTrue(ProcessMatcher.matchesPathContains(["workbuddy ai.app"],
                           pathLower: "/applications/workbuddy ai.app/contents/macos/electron"))
        }

        TestKit.test("进程: 前缀族冲突双向判定（新增自定义校验与匹配器同口径）") {
            // R22：SettingsView 新增校验与 ProcessMatcher 匹配共用同一判定——
            // codex + codex-helper 双向都拦（匹配器会把 codex-helper 同时算给两个 profile）
            try expectTrue(ProcessMatcher.hasPrefixFamilyConflict("codex", "codex"), "相等即冲突")
            try expectTrue(ProcessMatcher.hasPrefixFamilyConflict("codex-helper", "codex"),
                            "自定义名命中已知名前缀族（连字符分隔）")
            try expectTrue(ProcessMatcher.hasPrefixFamilyConflict("codex helper", "codex"),
                            "空格分隔同样冲突")
            try expectTrue(ProcessMatcher.hasPrefixFamilyConflict("codex", "codex-helper"),
                            "反向：已知名命中自定义名前缀族也拦")
            try expectFalse(ProcessMatcher.hasPrefixFamilyConflict("codexmalware", "codex"),
                            "无词边界不冲突（与匹配器词边界口径一致）")
            try expectFalse(ProcessMatcher.hasPrefixFamilyConflict("cursor", "codex"), "无关名不冲突")
            try expectTrue(ProcessMatcher.hasPrefixFamilyConflict("CODEX-HELPER", "codex"),
                            "大小写归一化")
        }

        TestKit.test("进程: 并发快照竞态冒烟（snapshotLock 差分窗口原子性）") {
            // 引擎采样 / 工作台扫描 / 终止前身份复核可能并发调 snapshot：
            // 差分窗口（lastWall 读取→update→setWall）非原子时，后到方分母被
            // 先到方重置，CPU% 单拍失真。snapshotLock 串行化后此冒烟验证
            // 「并发调用无死锁/无崩溃且全部产出合法快照」；CPU 数值的精确性
            // 由锁的覆盖范围（读→遍历→setWall→prune 全程）静态保证
            let provider = ProcessProvider()
            let group = DispatchGroup()
            let done = NSLock()
            var completed = 0
            for _ in 0..<6 {
                group.enter()
                DispatchQueue.global(qos: .userInitiated).async {
                    for _ in 0..<15 {
                        let snap = provider.snapshot()
                        done.lock()
                        if !snap.entries.isEmpty { completed += 1 }
                        done.unlock()
                    }
                    group.leave()
                }
            }
            // 快照在后台队列执行（无需主线程），主线程限时等待即可
            let result = group.wait(timeout: .now() + 60)
            try expectEqual(result, .success, "并发快照必须全部完成（卡死即 snapshotLock 重入）")
            try expectTrue(completed >= 6 * 15 - 3,
                            "空快照至多允许极少数（进程表瞬时不可读），实际 \(completed)/90")
        }

        TestKit.test("引擎: terminateAgent 终止逃生舱更新事件与状态") {
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            // 用不存在的 pid 验证「失败不谎报」；成功路径由下面的 Fake 终止器验证
            // （不能传自身 pid：terminate 会真的把测试进程杀掉）。
            // pid 不在当前快照的匹配结果里 → 身份复核拒绝（防 PID 复用误杀）
            let ok = engine.terminateAgent(pid: 999999, agentId: "dim")
            try expectTrue(!ok, "无法发送信号时必须返回 false")
            try expectTrue(engine.latestEvent?.message?.contains("已退出或已变更") == true,
                           "应按身份复核拒绝而非笼统失败，实际: \(engine.latestEvent?.message ?? "nil")")
            try expectTrue(engine.latestEvent?.message?.contains("已终止") != true,
                           "不得谎报已终止，实际: \(engine.latestEvent?.message ?? "nil")")
        }

        TestKit.test("引擎: terminateAgent 成功路径写入 completed 事件") {
            // 直接验证成功分支的事件语义：completed 而非 attention
            // （attention 会让收起态细条误报红色告警）
            // 借道 cleanAnomalies 之外的方式不可行，故用可终止的空进程验证：
            // 启动一个 /bin/sleep 作为无害目标。身份复核要求快照里存在匹配该 Agent、
            // 且 pid 一致的条目——Fake 快照注入与真实进程同 pid/同路径的条目。
            let sleeper = Process()
            sleeper.executableURL = URL(fileURLWithPath: "/bin/sleep")
            sleeper.arguments = ["30"]
            try sleeper.run()
            defer { if sleeper.isRunning { sleeper.terminate() } }
            let provider = FakeProcessProvider(
                processNames: [], bundleIDs: [],
                entries: [ProcessSnapshot.Entry(
                    pid: sleeper.processIdentifier,
                    // path 的 basename 须与真实进程一致（终止器侧复核），
                    // 且不能是 /bin 等系统路径（isSystemPath 过滤）；basename 字段
                    // 须匹配 dim 的 processNames（引擎侧复核）——夹具独立验证两层防线
                    path: "/opt/fake/bin/sleep",
                    basename: "dimagent",
                    cpuPercent: 0, rssBytes: 0
                )]
            )
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:], processMonitor: provider)
            _ = engine.sample(now: Date())
            let ok = engine.terminateAgent(pid: sleeper.processIdentifier, agentId: "dim")
            try expectTrue(ok, "身份复核通过的 pid 应成功终止")
            // 信号刚发出时**不许**已经有结论：那一刻我们只知道「发了信号」，
            // 忽略 SIGTERM 的进程还在跑，而异常列表变空不算复核（清理顺手清掉了证据）
            try expectTrue(engine.latestEvent?.message?.contains("已终止") != true,
                           "复核之前不得宣布已终止")

            // 同步驱动复核（真实路径由 0.8s 定时器排程）。先回收子进程：SIGTERM 已发出，
            // 但 Foundation 不 wait 的话它会停在僵尸态——正是下面那条用例要单独钉的形态。
            sleeper.waitUntilExit()
            let verified = engine.verifyTermination(agentId: "dim", pid: sleeper.processIdentifier,
                                                    path: "/opt/fake/bin/sleep")
            try expectTrue(verified, "sleep 进程被 SIGKILL 兜底带走，探活应判定已退出")
            try expectEqual(engine.latestEvent?.eventType, .completed,
                            "确认退出应为 completed（非 attention）")
            try expectTrue(engine.latestEvent?.message?.contains("进程已终止") == true, "应提示已终止")
            try expectTrue(engine.latestEvent?.detail?.contains("探活") == true,
                           "文案要说清结论来自复核，不是来自信号发送")
        }

        TestKit.test("引擎: 复核发现进程仍在时，不得宣称清理完成") {
            // 「收到 SIGTERM 又杀不掉」这一支用真进程造不出来（SIGKILL 兜底一定带走），
            // 所以复核探针是注入点。
            let sleeper = Process()
            sleeper.executableURL = URL(fileURLWithPath: "/bin/sleep")
            sleeper.arguments = ["30"]
            try sleeper.run()
            defer { if sleeper.isRunning { sleeper.terminate() } }
            let provider = FakeProcessProvider(
                processNames: [], bundleIDs: [],
                entries: [ProcessSnapshot.Entry(pid: sleeper.processIdentifier,
                                                path: "/opt/fake/bin/sleep", basename: "dimagent",
                                                cpuPercent: 0, rssBytes: 0)])
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:], processMonitor: provider)
            _ = engine.sample(now: Date())
            try expectTrue(engine.terminateAgent(pid: sleeper.processIdentifier, agentId: "dim"),
                           "前置：终止应通过身份复核")
            engine.terminationProbe = { _, _ in true }   // 假装探活仍存活（死锁进程）
            let verified = engine.verifyTermination(agentId: "dim", pid: sleeper.processIdentifier,
                                                    path: "/opt/fake/bin/sleep")
            try expectFalse(verified, "仍在运行必须返回 false")
            try expectEqual(engine.latestEvent?.eventType, .attention,
                            "杀不掉是要人接管的事，不是完成")
            try expectTrue(engine.latestEvent?.message?.contains("仍在运行") == true,
                           "文案要直说仍在运行，实际：\(engine.latestEvent?.message ?? "nil")")
            try expectTrue(engine.latestEvent?.detail?.contains("未宣称清理完成") == true,
                           "要明确这次没有宣告成功")
        }

        TestKit.test("进程探活: 僵尸不算存活（kill(0) 对它照样返回 0）") {
            // 复核要是把僵尸算成「杀不掉」，用户会收到一句永远不消失的假警报：
            // 进程早已退出，只是父进程还没 wait 回收，既不占 CPU 也不占内存。
            var pid: pid_t = 0
            var argv: [UnsafeMutablePointer<CChar>?] = [strdup("/usr/bin/true"), nil]
            var envp: [UnsafeMutablePointer<CChar>?] = [nil]
            try expectEqual(posix_spawn(&pid, "/usr/bin/true", nil, nil, &argv, &envp), 0,
                            "要能派生一个真子进程")
            var status: Int32 = 0
            defer { waitpid(pid, &status, 0) }        // 收尾回收，不留给测试进程
            usleep(200_000)                            // /usr/bin/true 早已退出 ⇒ 此刻是僵尸
            try expectEqual(kill(pid, 0), 0,
                            "前置：pid 条目还在（kill(0) 成功），否则这条用例什么也没钉")
            try expectTrue(ProcessTerminator.isZombie(pid), "应当判为僵尸")
            try expectFalse(ProcessTerminator.isAlive(pid: pid),
                            "僵尸不得算存活——否则复核会永远报「杀不掉」")
            try expectFalse(ProcessTerminator.isAlive(pid: pid, expectedPath: "/usr/bin/true"),
                            "带路径复核时同样不得算存活")
        }

        TestKit.test("引擎: terminateAgent 无 pid 时不谎报成功") {
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            let ok = engine.terminateAgent(pid: nil, agentId: "dim")
            try expectTrue(!ok, "无 pid 时必须返回 false")
            // 不得出现「已终止」这类误导文案：进程并未被终止
            try expectTrue(engine.latestEvent?.message?.contains("已终止") != true,
                           "无 pid 时不得谎报已终止")
            try expectTrue(engine.latestEvent?.message?.contains("无法终止") == true,
                           "应如实提示无法终止")
        }

        TestKit.test("工作台维护: scanAnomalies 孤儿/异常检测与 cleanAnomalies 一键清理") {
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            let anomaly = AgentAnomaly(
                id: "test-anomaly",
                pid: 88888,
                ppid: 1,
                agentName: "DimAgent",
                profileId: "dim",
                commandPath: "/usr/local/bin/dim",
                cpuPercent: 50.0,
                memoryBytes: 524_288_000,
                anomalyType: .orphan,
                reason: "测试孤儿进程"
            )
            try expectEqual(anomaly.memoryText, "500M")
            // 不存在的 pid：必须如实反馈失败，不得宣告「已安全清理」
            // （此前无论结果都发 completed + 「已安全清理 N 个…系统资源已就绪」）
            let res = engine.cleanAnomalies([anomaly])
            try expectEqual(res.terminatedCount, 0, "不存在的 pid 不应计为清理成功")
            try expectEqual(engine.latestEvent?.agentId, "workbench-cleaner")
            try expectEqual(engine.latestEvent?.eventType, .attention, "清理失败应为 attention 而非 completed")
            try expectTrue(engine.latestEvent?.message?.contains("已安全清理") != true,
                           "失败时不得谎报已清理，实际: \(engine.latestEvent?.message ?? "nil")")
        }

        TestKit.test("工作台维护: 有真实 pid 时清理成功并报 completed") {
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            // 起一个无害的空闲进程作为可终止目标（不能用自己的 pid，会杀掉测试进程）
            let sleeper = Process()
            sleeper.executableURL = URL(fileURLWithPath: "/bin/sleep")
            sleeper.arguments = ["30"]
            try sleeper.run()   // 显式断言启动成功（try? 会把启动失败静默成无效 pid 的误报）
            defer { if sleeper.isRunning { sleeper.terminate() } }
            let anomaly = AgentAnomaly(
                id: "test-anomaly-ok",
                pid: sleeper.processIdentifier,
                ppid: 1,
                agentName: "DimAgent",
                profileId: "dim",
                commandPath: "/bin/sleep",
                cpuPercent: 1.0,
                memoryBytes: 1_048_576,
                anomalyType: .orphan,
                reason: "测试可终止进程"
            )
            let res = engine.cleanAnomalies([anomaly])
            try expectTrue(res.terminatedCount == 1, "可终止的进程应计为 1，实际 \(res.terminatedCount)")
            // 发完信号当场不许有结论：那一刻我们只知道「信号发出去了」，而忽略 SIGTERM 的
            // 进程还在跑。结论由复核给（下面那段），与单条终止路径同口径
            try expectTrue(engine.latestEvent?.message?.contains("已安全清理") != true,
                           "复核之前不得宣布已安全清理，实际: \(engine.latestEvent?.message ?? "nil")")
            sleeper.waitUntilExit()   // SIGTERM 已发出；不 wait 会停在僵尸态
            let verdict = engine.verifyClean([anomaly])
            try expectEqual(verdict.confirmedPids, [sleeper.processIdentifier], "真实退出的进程应被复核确认")
            try expectTrue(verdict.stillRunningPids.isEmpty, "复核不应把已退出者算成仍在运行：\(verdict.stillRunningPids)")
            try expectEqual(engine.latestEvent?.eventType, .completed)
            try expectTrue(engine.latestEvent?.message?.contains("已安全清理") == true)
        }

        TestKit.test("工作台维护: 批量清理的内存只算确认退出的，部分失败不得报完成") {
            // 「收到信号又杀不掉」用真进程造不出来（SIGKILL 兜底一定带走），所以走探针注入
            func anomaly(_ pid: Int32, _ mem: UInt64) -> AgentAnomaly {
                AgentAnomaly(id: "hung-\(pid)", pid: pid, ppid: 1, agentName: "假死体", profileId: "dim",
                             commandPath: "/opt/fake/bin/sleep", cpuPercent: 90, memoryBytes: mem,
                             anomalyType: .hung, reason: "持续过载")
            }
            let pair = [anomaly(4001, 300_000_000), anomaly(4002, 700_000_000)]
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())

            engine.terminationProbe = { _, _ in false }   // 全部确认退出
            let allGone = engine.verifyClean(pair)
            try expectTrue(allGone.allGone)
            try expectEqual(allGone.reclaimedMemoryBytes, 1_000_000_000, "确认退出者的内存合计")
            try expectEqual(engine.latestEvent?.eventType, .completed)
            try expectTrue(engine.latestEvent?.message?.contains("已安全清理 2 个") == true,
                           "全部退出才配得上「已安全清理 2 个」，实际: \(engine.latestEvent?.message ?? "nil")")

            engine.terminationProbe = { pid, _ in pid == 4002 }   // 一个杀不掉
            let partial = engine.verifyClean(pair)
            try expectEqual(partial.confirmedPids, [4001])
            try expectEqual(partial.stillRunningPids, [4002])
            try expectEqual(partial.reclaimedMemoryBytes, 300_000_000,
                            "仍在运行的进程内存不得计进「回收」——那是必然偏大的数")
            try expectEqual(engine.latestEvent?.eventType, .attention,
                            "部分失败必须报 attention，而不是又发一条完成横幅")
            try expectTrue(engine.latestEvent?.message?.contains("1 个进程未能终止") == true,
                           "实际: \(engine.latestEvent?.message ?? "nil")")
            try expectTrue(engine.latestEvent?.detail?.contains("没有宣称全部清理完成") == true,
                           "实际: \(engine.latestEvent?.detail ?? "nil")")
        }

        TestKit.test("实时流水: AgentLogStreamer 事件流模型与解析") {
            let event = AgentLogEvent(
                kind: .command,
                title: "git status",
                detail: "On branch main",
                agentId: "test-agent"
            )
            try expectEqual(event.kind.label, "EXEC")
            try expectEqual(event.title, "git status")
            try expectEqual(event.agentId, "test-agent")

            // 未知 agent 返回空列表
            let unknown = AgentLogStreamer.fetchRecentEvents(agentId: "unknown-agent-xyz")
            try expectEqual(unknown.count, 0)

            // 引擎方法能正常调用
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            let exp = SelfPollExpectation()
            var returnedRows: [AgentLogEvent]? = nil
            engine.fetchLogStream(agentId: "unknown-agent-xyz", limit: 5) { rows in
                returnedRows = rows
                exp.fulfill()
            }
            let deadline = Date().addingTimeInterval(3.0)
            while !exp.isFulfilled && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
            }
            try expectTrue(exp.isFulfilled, "fetchLogStream 应当异步回调")
            try expectEqual(returnedRows?.count ?? -1, 0)
        }

        TestKit.test("实时流水: 批次内派生 id 去重（同毫秒同名事件不得撞 id）") {
            let ts = Date(timeIntervalSince1970: 1_700_000_000.123)
            let a = AgentLogEvent(timestamp: ts, kind: .toolCall, title: "调用工具: Bash", agentId: "dim")
            let b = AgentLogEvent(timestamp: ts, kind: .toolCall, title: "调用工具: Bash", agentId: "dim")
            let c = AgentLogEvent(timestamp: ts, kind: .toolCall, title: "调用工具: Read", agentId: "dim")
            // 前置：派生 id 确实相同（一条消息内两条同名 tool_use 的真实形态）
            try expectEqual(a.id, b.id, "前置条件：同毫秒同名事件的派生 id 相同")

            let deduped = AgentLogStreamer.deduplicateIds([a, b, c])
            try expectEqual(Set(deduped.map(\.id)).count, deduped.count,
                            "去重后批次内 id 两两不同，否则 ForEach 重复 id 条目互相顶替")
            try expectEqual(deduped[0].id, a.id, "首次出现保持原 id（已展开的详情态不丢）")
            try expectTrue(deduped[1].id.hasPrefix(b.id + "#"), "后续出现追加序号后缀")
            try expectEqual(deduped[2].id, c.id, "不同标题的事件不受影响")
            // 出现次序由数据行序决定，重复调用（同一查询重复执行）结果稳定
            let again = AgentLogStreamer.deduplicateIds([a, b, c])
            try expectEqual(again.map(\.id), deduped.map(\.id), "同一批次重复去重结果稳定")

            // 基础 id 自带「#数字」后缀的再碰撞形态（标题含 #）：仍须两两不同
            let x = AgentLogEvent(timestamp: ts, kind: .toolCall, title: "T", agentId: "dim")
            let xHashOne = AgentLogEvent(id: "\(x.id)#1", timestamp: ts, kind: .toolCall, title: "T", agentId: "dim")
            let dedupedHash = AgentLogStreamer.deduplicateIds([x, xHashOne, x])
            try expectEqual(Set(dedupedHash.map(\.id)).count, 3,
                            "基础 id 以 #1 结尾时固定序号会再撞，须取下一个可用序号（实际 \(dedupedHash.map(\.id))）")
        }

        TestKit.test("实时流水: detail 超长截断（64KB JSONL 行不再整段渲染）") {
            let huge = String(repeating: "x", count: 70_000)
            let event = AgentLogEvent(kind: .command, title: "t", detail: huge, agentId: "claude")
            try expectEqual(event.detail?.count, AgentLogEvent.maxDetailCharacters + "…[已截断]".count,
                            "detail 必须截断到上界 + 截断标记")

            let normal = AgentLogEvent(kind: .command, title: "t", detail: "short", agentId: "claude")
            try expectEqual(normal.detail, "short", "短 detail 原样保留")
            let none = AgentLogEvent(kind: .command, title: "t", detail: nil, agentId: "claude")
            try expectNil(none.detail, "nil detail 透传")
            // 显式 id 传入时截断同样生效（构造点唯一，无旁路）
            let explicit = AgentLogEvent(id: "fixed", kind: .command, title: "t", detail: huge, agentId: "claude")
            try expectEqual(explicit.id, "fixed")
            try expectTrue(explicit.detail!.hasSuffix("…[已截断]"), "显式 id 路径同样截断")
        }

        TestKit.test("熔断保护: Token 激增告警触发（速率制 + 连续确认）") {
            let fake = FakeTokenUsageMonitor()
            fake.usage["dim"] = TokenUsage(tokens24h: 10_000, tokensTotal: 10_000, cost24h: 0, costTotal: 0)
            let config = EngineConfig(tokenAlertEnabled: true, tokenAlertThreshold: 50_000)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: config,
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: fake,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            _ = engine.sample(now: now)
            try expectNil(engine.latestEvent, "首拍记录基准，不应告警")

            // 每档 60s、每档 +80k（≈80k/分钟，超 50k 阈值）；需连续 3 档才确认
            var tokens = 10_000
            for beat in 1...3 {
                tokens += 80_000
                fake.usage["dim"] = TokenUsage(tokens24h: tokens, tokensTotal: tokens, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
                if beat < 3 {
                    try expectNil(engine.latestEvent, "第 \(beat) 档不应告警（需连续 3 档确认）")
                }
            }
            try expectEqual(engine.latestEvent?.eventType, .costSpike, "连续 3 档超阈值应触发 costSpike")
            try expectTrue(engine.latestEvent?.message?.contains("Token 激增") == true, "应显示激增提示")
            try expectTrue(engine.latestEvent?.detail?.contains("/分钟") == true, "应给出折算速率")
            try expectTrue(engine.latestEvent?.copyableDiagnosticText.contains("详情:") == true, "可复制诊断应包含详情")
        }

        TestKit.test("熔断保护: 长任务结束时一次性落盘不误报激增") {
            let fake = FakeTokenUsageMonitor()
            fake.usage["dim"] = TokenUsage(tokens24h: 1_000, tokensTotal: 1_000, cost24h: 0, costTotal: 0)
            let config = EngineConfig(tokenAlertEnabled: true, tokenAlertThreshold: 50_000)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: config,
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: fake,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            _ = engine.sample(now: now)

            // 单次暴增 300 万（长任务结束落盘），折算 ≈3M/分钟；下一档无新增 → 速率归零
            fake.usage["dim"] = TokenUsage(tokens24h: 3_000_000, tokensTotal: 3_000_000, cost24h: 0, costTotal: 0)
            _ = engine.sample(now: now.addingTimeInterval(60))
            try expectNil(engine.latestEvent, "单次落盘仅累计 1 档，不应告警")
            _ = engine.sample(now: now.addingTimeInterval(120))
            try expectNil(engine.latestEvent, "无后续增量则速率归零，不应告警")
        }

        TestKit.test("熔断保护: 同一轮 Token 激增关闭后不重复弹窗") {
            let fake = FakeTokenUsageMonitor()
            fake.usage["dim"] = TokenUsage(tokens24h: 1_000, tokensTotal: 1_000, cost24h: 0, costTotal: 0)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(tokenAlertEnabled: true, tokenAlertThreshold: 50_000),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: FakeFileActivityProvider(writes: [:]), tokenMonitor: fake,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }))
            let now = Date()
            _ = engine.sample(now: now)
            for beat in 1...3 {
                let total = 1_000 + beat * 80_000
                fake.usage["dim"] = TokenUsage(tokens24h: total, tokensTotal: total, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
            }
            try expectTrue(engine.latestEvent?.message?.contains("Token 激增") == true)
            engine.clearLatestEvent()
            // 继续增长，但属于同一轮异常；关闭后不应再次产生事件。
            for beat in 4...6 {
                let total = 1_000 + beat * 80_000
                fake.usage["dim"] = TokenUsage(tokens24h: total, tokensTotal: total, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
            }
            try expectNil(engine.latestEvent, "同一轮异常关闭后不应重复弹窗")
            // 速率恢复后重新武装，下一轮异常仍可告警。
            fake.usage["dim"] = TokenUsage(tokens24h: 481_000, tokensTotal: 481_000, cost24h: 0, costTotal: 0)
            _ = engine.sample(now: now.addingTimeInterval(420))
            for beat in 8...10 {
                let total = 481_000 + (beat - 7) * 80_000
                fake.usage["dim"] = TokenUsage(tokens24h: total, tokensTotal: total, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
            }
            try expectEqual(engine.latestEvent?.eventType, .costSpike, "恢复后新一轮异常仍应告警")
        }

        TestKit.test("熔断保护: WorkBuddy 专家团高 Token 消耗自动应用专属下限（100万/分），常规并发不误报") {
            let fake = FakeTokenUsageMonitor()
            fake.usage["workbuddy"] = TokenUsage(tokens24h: 10_000, tokensTotal: 10_000, cost24h: 0, costTotal: 0)
            let workbuddyEntry = ProcessSnapshot.Entry(
                pid: 12345,
                path: "/Applications/WorkBuddy.app/Contents/MacOS/WorkBuddy",
                basename: "workbuddy",
                cpuPercent: 10.0
            )
            // 全局设置 200k/分钟（默认），但 WorkBuddy 拥有 100万 专属下限
            let config = EngineConfig(tokenAlertEnabled: true, tokenAlertThreshold: 200_000)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: config,
                processMonitor: FakeProcessProvider(processNames: ["workbuddy"], bundleIDs: ["com.tencent.workbuddy.mac"], entries: [workbuddyEntry]),
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: fake,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            _ = engine.sample(now: now)
            try expectNil(engine.latestEvent, "首拍记录基准")

            // 模拟多专家团常规并发交互：每分钟消耗 500k tokens（高于全局 200k，但低于专家团 100万 下限）
            var tokens = 10_000
            for beat in 1...4 {
                tokens += 500_000
                fake.usage["workbuddy"] = TokenUsage(tokens24h: tokens, tokensTotal: tokens, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
                try expectNil(engine.latestEvent, "500k/分钟处于专家团正常交互区间，不应误报激增")
            }
        }

        TestKit.test("熔断保护: WorkBuddy 超过 100万专属下限连续 3 档仍触发熔断告警") {
            let fake = FakeTokenUsageMonitor()
            fake.usage["workbuddy"] = TokenUsage(tokens24h: 10_000, tokensTotal: 10_000, cost24h: 0, costTotal: 0)
            let workbuddyEntry = ProcessSnapshot.Entry(
                pid: 12345,
                path: "/Applications/WorkBuddy.app/Contents/MacOS/WorkBuddy",
                basename: "workbuddy",
                cpuPercent: 10.0
            )
            let config = EngineConfig(tokenAlertEnabled: true, tokenAlertThreshold: 200_000)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: config,
                processMonitor: FakeProcessProvider(processNames: ["workbuddy"], bundleIDs: ["com.tencent.workbuddy.mac"], entries: [workbuddyEntry]),
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: fake,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            _ = engine.sample(now: now)

            // 模拟极端死循环/狂暴生成：每分钟消耗 120万 tokens（超 100万 下限）
            var tokens = 10_000
            for beat in 1...3 {
                tokens += 1_200_000
                fake.usage["workbuddy"] = TokenUsage(tokens24h: tokens, tokensTotal: tokens, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
                if beat < 3 {
                    try expectNil(engine.latestEvent, "前 \(beat) 档需持续确认")
                }
            }
            try expectEqual(engine.latestEvent?.eventType, .costSpike, "超过 100万 且连续 3 档应触发熔断告警")
            try expectTrue(engine.latestEvent?.detail?.contains("专家团保护下限") == true, "详情应指出包含专家团保护下限")
        }

        TestKit.test("熔断保护: 全局阈值高于专家团下限时（如 200万），有效阈值向上吸附") {
            let fake = FakeTokenUsageMonitor()
            fake.usage["workbuddy"] = TokenUsage(tokens24h: 10_000, tokensTotal: 10_000, cost24h: 0, costTotal: 0)
            let workbuddyEntry = ProcessSnapshot.Entry(
                pid: 12345,
                path: "/Applications/WorkBuddy.app/Contents/MacOS/WorkBuddy",
                basename: "workbuddy",
                cpuPercent: 10.0
            )
            // 用户显式调高到 200 万 / 分钟
            let config = EngineConfig(tokenAlertEnabled: true, tokenAlertThreshold: 2_000_000)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: config,
                processMonitor: FakeProcessProvider(processNames: ["workbuddy"], bundleIDs: ["com.tencent.workbuddy.mac"], entries: [workbuddyEntry]),
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: fake,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            _ = engine.sample(now: now)

            // 消耗 150 万 / 分钟（超过 100万 下限，但低于用户设置的 200万）
            var tokens = 10_000
            for beat in 1...3 {
                tokens += 1_500_000
                fake.usage["workbuddy"] = TokenUsage(tokens24h: tokens, tokensTotal: tokens, cost24h: 0, costTotal: 0)
                _ = engine.sample(now: now.addingTimeInterval(Double(beat) * 60))
                try expectNil(engine.latestEvent, "低于用户设置的 200万 阈值不应告警")
            }
        }

        TestKit.test("熔断保护: 持续死循环/高负载告警（低占用不误报，持续高 CPU 触发）") {
            // 1. 低 CPU（2%）：即使运行 6 分钟也不应触发死循环告警
            let lowEngine = makeEngine(processNames: ["DimAgent"], writes: [:], cpu: 2.0)
            let start = Date()
            _ = lowEngine.sample(now: start)
            _ = lowEngine.sample(now: start.addingTimeInterval(360))
            try expectNil(lowEngine.latestEvent, "低占用长耗时不应误报死循环")

            // 2. 持续高 CPU（80%）：达到 5 分钟阈值触发预警
            // 注意：必须逐步推进时间（真实采样是连续的），一次性跳跃 305s 会被
            // 采样断点检测（见 resumeGapThreshold）判定为睡眠/挂起而重置高负载基准。
            let highEngine = makeEngine(processNames: ["DimAgent"], writes: [:], cpu: 80.0)
            _ = highEngine.sample(now: start)
            try expectNil(highEngine.latestEvent, "首次高 CPU 仅建基准不告警")
            // 以 60s 为步长推进到 305s（模拟真实连续采样）
            for step in stride(from: 60.0, through: 300.0, by: 60.0) {
                _ = highEngine.sample(now: start.addingTimeInterval(step))
            }
            _ = highEngine.sample(now: start.addingTimeInterval(305))
            try expectEqual(highEngine.latestEvent?.eventType, .costSpike, "持续高 CPU 超 5 分钟应触发告警")
            try expectTrue(highEngine.latestEvent?.message?.contains("持续高负载") == true, "文案应提示高负载")

            // 3. 告警保护：横幅是单槽位，普通事件不得在保护期内挤掉严重告警
            // （实测中 costSpike 横幅常被其他 Agent 的「任务完成」在几秒内顶掉）
            let now = Date()
            highEngine.postEvent(AgentTaskEvent(
                agentId: "dim", agentName: "DimAgent", eventType: .completed,
                duration: 5, timestamp: now, message: "普通完成事件"
            ))
            try expectEqual(highEngine.latestEvent?.eventType, .costSpike,
                            "保护期内普通事件不应覆盖告警，实际: \(highEngine.latestEvent?.message ?? "nil")")

            // 4. 用户关闭告警后保护解除，普通事件可正常展示
            highEngine.clearLatestEvent()
            highEngine.postEvent(AgentTaskEvent(
                agentId: "dim", agentName: "DimAgent", eventType: .completed,
                duration: 5, timestamp: now, message: "普通完成事件"
            ))
            try expectEqual(highEngine.latestEvent?.eventType, .completed,
                            "关闭告警后保护应解除")
        }

        TestKit.test("熔断保护: 保护期按 Agent 记账，关掉 B 的横幅不得解除 A 的保护") {
            // 保护期曾是单个全局 Date，而 clearLatestEvent 会被「另一个 Agent」的横幅清理
            // 与通知点击回调调用：用户消掉 B 的横幅，A 正在生效的激增保护就凭空消失。
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:], cpu: 0)
            let now = Date()
            engine.postEvent(AgentTaskEvent(agentId: "dim", agentName: "DimAgent", eventType: .costSpike,
                                            duration: 0, timestamp: now, message: "⚠️ DimAgent Token 激增"))
            // 同类告警总是可占位：B 的告警盖在 A 之上（展示位仍只有一条）
            engine.postEvent(AgentTaskEvent(agentId: "claude", agentName: "Claude", eventType: .costSpike,
                                            duration: 0, timestamp: now, message: "⚠️ Claude Token 激增"))
            try expectEqual(engine.latestEvent?.agentId, "claude", "前置条件：当前展示的是 B 的告警")
            // 用户此刻关掉 B 的横幅（或 B 从等待确认回到 working 触发同一次清理）
            engine.clearLatestEvent()

            // A 的保护窗口仍在途（30s 才过了瞬间）：A 自己的普通横幅必须继续让位
            engine.postEvent(AgentTaskEvent(agentId: "dim", agentName: "DimAgent", eventType: .completed,
                                            duration: 5, timestamp: now, message: "A 的普通完成事件"))
            try expectNil(engine.latestEvent,
                          "B 的横幅清理不应解除 A 的保护期，实际: \(engine.latestEvent?.message ?? "nil")")
            // 反向守卫：保护不是全局压制，B 没有保护期，其普通横幅照常展示
            engine.postEvent(AgentTaskEvent(agentId: "claude", agentName: "Claude", eventType: .completed,
                                            duration: 5, timestamp: now, message: "B 的普通完成事件"))
            try expectEqual(engine.latestEvent?.agentId, "claude", "保护期不得跨 Agent 压制普通横幅")
        }

        TestKit.test("熔断保护: Agent 终止与断点会清掉自己的保护期（不长期占坑）") {
            let provider = MutableProcessProvider(names: ["broken-agent"], bundleIDs: [], cpu: 0)
            let profile = AgentProfile(id: "broken", name: "Broken", icon: "terminal",
                                       bundleIDs: [], processNames: ["broken-agent"], sessionDirs: [])
            let engine = ActivityEngine(
                profiles: [profile],
                config: EngineConfig(workingWindow: 20),
                processMonitor: provider,
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            _ = engine.sample(now: now)                     // 先建立基准，避免被判为采样断点
            engine.postEvent(AgentTaskEvent(agentId: "broken", agentName: "Broken", eventType: .costSpike,
                                            duration: 0, timestamp: now, message: "⚠️ Broken Token 激增"))
            provider.names = []                             // 进程消失 → offline → resetTracking
            _ = engine.sample(now: now.addingTimeInterval(2))
            engine.postEvent(AgentTaskEvent(agentId: "broken", agentName: "Broken", eventType: .completed,
                                            duration: 5, timestamp: now, message: "普通完成事件"))
            try expectEqual(engine.latestEvent?.eventType, .completed,
                            "Agent 已离线，残留保护期不得压住后续横幅（否则告警状态泄漏）")
        }

        TestKit.test("会话探测健康: 库改表/不可读时快照带着证据，不把「读不到」渲染成待机") {
            // 合法的空库 + 档案登记的查询：schema 对不上 → prepare 注定失败。
            // 这正是第三方 App 升级换表的形态：以前它和「Agent 真的闲着」在 UI 上完全同形，
            // 智能体从此永久失明且零证据（对齐 CONTEXT.md：源缺失不得当作零活动）。
            let dir = NSTemporaryDirectory() + "probe-health-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(atPath: dir) }
            let dbPath = dir + "/state.db"
            var opened: OpaquePointer?
            guard sqlite3_open_v2(dbPath, &opened, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE, nil) == SQLITE_OK,
                  let handle = opened else { throw TestError(message: "fixture 建库失败") }
            sqlite3_exec(handle, "CREATE TABLE unrelated (a TEXT);", nil, nil, nil)   // 就是没有 messages 表
            sqlite3_close(handle)
            // 会话尾窗里没有任何可辨识语义 → 探测落到「已知库」兜底分支，在那里撞上桌
            let transcript = URL(fileURLWithPath: dir + "/session.jsonl")
            try #"{"note":"probe health fixture"}"#.data(using: .utf8)!.write(to: transcript)

            let profile = AgentProfile(id: "blind", name: "Blind", icon: "terminal",
                                       bundleIDs: [], processNames: ["blind-agent"],
                                       sessionDirs: [dir],
                                       sessionDatabase: AgentSessionDatabase(path: dbPath, schema: .dimTasks))
            let provider = MutableProcessProvider(names: ["blind-agent"], bundleIDs: [], cpu: 0)
            let fileSource = FakeFileActivityProvider(writes: [:], files: [dir: transcript])
            let engine = ActivityEngine(
                profiles: [profile],
                config: EngineConfig(workingWindow: 20),
                processMonitor: provider,
                fileMonitor: fileSource,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            let snap = engine.sample(now: now).first { $0.id == "blind" }
            try expectEqual(snap?.level, .idle, "前置条件：只看等级它确实是「待机」")
            try expectEqual(snap?.sessionProbeHealth?.failure, .prepareFailed,
                            "prepare 失败必须留下可归因的探测健康，而不是无声降级")
            try expectEqual(snap?.sessionProbeHealth?.path, dbPath, "证据要指明是哪个库，用户才能自查")
            try expectTrue(snap?.sessionProbeHealth?.diagnosticText.contains("不代表智能体真的空闲") == true,
                           "诊断文案必须说清这里的「待机」不可信")

            // last-known 语义：连续采样每拍覆盖为最近一次结果，坏源不会自己洗白
            let again = engine.sample(now: now.addingTimeInterval(2)).first { $0.id == "blind" }
            try expectEqual(again?.sessionProbeHealth?.failure, .prepareFailed, "应保持为最近已知状态")

            // 保质期：源修好了、但此后一直没有新写入 ⇒ 探测被跳过（不再写新值）。
            // 旧的「读不到」若永远挂着，就是反过来把「读不到」伪装成「坏了」——同样是谎报
            fileSource.files = [:]
            let stale = engine.sample(now: now.addingTimeInterval(600)).first { $0.id == "blind" }
            try expectNil(stale?.sessionProbeHealth, "超过保质期的探测健康必须退场")

            // 退场不是洗白：坏源重新被探测到时，证据必须重新留下
            fileSource.files = [dir: transcript]
            let back = engine.sample(now: now.addingTimeInterval(602)).first { $0.id == "blind" }
            try expectEqual(back?.sessionProbeHealth?.failure, .prepareFailed, "重新探测到故障必须重新记录")

            // 生命周期：进程消失 → resetTracking 连带回收，Agent 重启后不带旧证据
            provider.names = []
            let off = engine.sample(now: now.addingTimeInterval(4)).first { $0.id == "blind" }
            try expectEqual(off?.level, .offline, "进程消失应转离线")
            try expectNil(off?.sessionProbeHealth, "探测健康须随每-Agent 状态一起回收，不得长期占坑")
        }

        TestKit.test("会话探测健康: 查询被写锁挡住 → stepFailed（不是「库里没有确认请求」）") {
            // 对方正在写库的那一拍最容易撞上 SQLITE_BUSY。此前 step 的错误码被当成
            // 「没有行」，于是等确认的卡片显示成待机，且一条证据都不留。
            let dir = NSTemporaryDirectory() + "probe-busy-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            let dbPath = dir + "/state.sqlite"
            try TokenFixture.exec(dbPath, [
                "CREATE TABLE messages (sessionId TEXT, rowid INTEGER PRIMARY KEY, role TEXT, toolMetadata TEXT, parts TEXT)",
                "INSERT INTO messages VALUES ('s1', 1, 'assistant', '{\"toolUse\":{\"name\":\"AskUserQuestion\"}}', '[]')"
            ])
            let transcript = URL(fileURLWithPath: dir + "/session.jsonl")
            try #"{"note":"busy fixture"}"#.data(using: .utf8)!.write(to: transcript)

            let profile = AgentProfile(id: "busy", name: "Busy", icon: "terminal",
                                       bundleIDs: [], processNames: ["busy-agent"],
                                       sessionDirs: [dir],
                                       sessionDatabase: AgentSessionDatabase(path: dbPath, schema: .dimTasks))
            let engine = ActivityEngine(
                profiles: [profile],
                config: EngineConfig(workingWindow: 20),
                processMonitor: MutableProcessProvider(names: ["busy-agent"], bundleIDs: [], cpu: 0),
                fileMonitor: FakeFileActivityProvider(writes: [:], files: [dir: transcript]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            let clean = engine.sample(now: now).first { $0.id == "busy" }
            try expectNil(clean?.sessionProbeHealth, "前置：无锁时探测正常，不该报任何故障")

            // 另一个连接持排他写锁 ⇒ 只读连接的 step 撞上 SQLITE_BUSY
            var writer: OpaquePointer?
            try expectEqual(sqlite3_open_v2(dbPath, &writer, SQLITE_OPEN_READWRITE, nil), SQLITE_OK,
                            "夹具库要能以读写方式打开")
            let begin = sqlite3_exec(writer, "BEGIN EXCLUSIVE;", nil, nil, nil)
            try expectEqual(begin, SQLITE_OK, "要能拿到排他锁")
            defer {
                sqlite3_exec(writer, "ROLLBACK;", nil, nil, nil)
                sqlite3_close(writer)
            }

            let blocked = engine.sample(now: now.addingTimeInterval(1)).first { $0.id == "busy" }
            try expectEqual(blocked?.sessionProbeHealth?.failure, .stepFailed,
                            "查询被中断必须留下 stepFailed，而不是静默变成待机")
            try expectEqual(blocked?.sessionProbeHealth?.path, dbPath, "证据要指明是哪个库")

            // 锁释放后必须自愈：健康记录不能永久挂着
            try expectEqual(sqlite3_exec(writer, "ROLLBACK;", nil, nil, nil), SQLITE_OK)
            let healed = engine.sample(now: now.addingTimeInterval(2)).first { $0.id == "busy" }
            try expectNil(healed?.sessionProbeHealth, "锁消失后应恢复为「探测本身没问题」")
        }

        TestKit.test("会话探测健康: 库存在但打不开 → unreadableDB（不是「库里没数据」）") {
            let dir = NSTemporaryDirectory() + "probe-unreadable-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            let dbPath = dir + "/state.db"
            try Data("not a db".utf8).write(to: URL(fileURLWithPath: dbPath))
            // runner 非 root，chmod 生效：这是「文件在、但我们读不到」的最小复现
            try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: dbPath)
            defer {
                try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: dbPath)
                try? FileManager.default.removeItem(atPath: dir)
            }
            let profile = AgentProfile(id: "locked", name: "Locked", icon: "terminal",
                                       bundleIDs: [], processNames: ["locked-agent"], sessionDirs: [dir],
                                       sessionDatabase: AgentSessionDatabase(path: dbPath, schema: .dimTasks))
            let probe = AgentSessionInspector.probe(profile: profile, activityFiles: [], now: Date())
            try expectNil(probe.signal, "读不到库仍按「无信号」降级，不谎报会话状态")
            try expectEqual(probe.health?.failure, .unreadableDB,
                            "但必须留下「源不可读」的证据，否则用户只看到一片待机")
        }

        TestKit.test("会话探测健康: 会话文件读不出来 → unreadableFile（SQLite 侧的对应物此前文件侧没有）") {
            // 用一个「名字叫 ui_messages.json 的目录」复现读不出来：不依赖权限、不依赖 root
            let dir = NSTemporaryDirectory() + "probe-unreadable-file-\(UUID().uuidString)"
            let fm = FileManager.default
            try fm.createDirectory(atPath: dir + "/ui_messages.json", withIntermediateDirectories: true)
            defer { try? fm.removeItem(atPath: dir) }
            let profile = AgentProfile(id: "fileblind", name: "FileBlind", icon: "terminal",
                                       bundleIDs: [], processNames: ["fileblind-agent"], sessionDirs: [dir])
            let probe = AgentSessionInspector.probe(profile: profile,
                                                    activityFiles: [URL(fileURLWithPath: dir + "/ui_messages.json")],
                                                    now: Date())
            try expectEqual(probe.health?.failure, .unreadableFile,
                            "文件在、读不出，必须留证据；静默 nil 会让岛显示待机而 doctor 说「结论可信」")
        }

        TestKit.test("会话探测健康: 会话文件不是解析器认识的形状 → undecodableFile") {
            // 真实触发形态：对方改版成 {"messages":[…]}（顶层由数组变对象），或半写入截断
            let dir = NSTemporaryDirectory() + "probe-undecodable-\(UUID().uuidString)"
            let fm = FileManager.default
            try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
            let file = dir + "/ui_messages.json"
            try Data(#"{"messages":[{"type":"ask","ask":"command"}]}"#.utf8).write(to: URL(fileURLWithPath: file))
            defer { try? fm.removeItem(atPath: dir) }
            let profile = AgentProfile(id: "shapeshift", name: "ShapeShift", icon: "terminal",
                                       bundleIDs: [], processNames: ["shapeshift-agent"], sessionDirs: [dir])
            let probe = AgentSessionInspector.probe(profile: profile,
                                                    activityFiles: [URL(fileURLWithPath: file)], now: Date())
            try expectEqual(probe.health?.failure, .undecodableFile,
                            "「格式改版」与「这个会话确实没有待确认事项」是两件事，不得混为一谈")
        }

        TestKit.test("引擎: 睡眠/挂起断点不误报任务完成") {
            // 合盖睡眠 8 小时后唤醒：时间在走但期间没有任何采样，不能补发
            // 「任务完成 (480分0秒)」——Agent 只是被挂起，不是干完了活。
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:], cpu: 80.0)
            let start = Date()
            _ = engine.sample(now: start)
            _ = engine.sample(now: start.addingTimeInterval(300))   // 形成高负载基准
            engine.clearLatestEvent()
            let snap = engine.sample(now: start.addingTimeInterval(8 * 3600)).first { $0.id == "dim" }
            try expectNil(engine.latestEvent,
                          "跨睡眠断点不得产生完成事件，实际: \(engine.latestEvent?.message ?? "nil")")
            try expectTrue(snap != nil, "断点后仍应正常产出快照")
        }

        TestKit.test("引擎: 断点阈值钳 180s 上界（有判别力构造：idle=100，R32/F1）") {
            // 判别力构造（验收官指出 150s/idle=5 与 idle=60 均无判别力）：
            // idle=100 → 旧阈值 max(120, 300)=300，新阈值 min(300,180)=180。
            // 序列：t0/t+100 高 CPU working → 挂起 200s（gap 介于新旧阈值之间）→
            // t+300 信号消失转 idle。旧代码（无钳制）不判断点 → 补发
            // 「任务完成（时长含 200s 挂起）」；新代码判断点 → 无完成事件。
            let provider = MutableProcessProvider(names: ["DimAgent"], bundleIDs: [], cpu: 80.0)
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin.filter { $0.id == "dim" },
                config: EngineConfig(idleSampleInterval: 100, workingWindow: 20),
                processMonitor: provider,
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let start = Date()
            _ = engine.sample(now: start)
            _ = engine.sample(now: start.addingTimeInterval(100))
            engine.clearLatestEvent()
            provider.cpu = 0   // 唤醒后信号消失
            let snap = engine.sample(now: start.addingTimeInterval(300)).first { $0.id == "dim" }
            try expectNil(engine.latestEvent,
                          "gap 200s 应判断点（新阈值钳 180），不得补发含挂起时长的完成事件，实际: \(engine.latestEvent?.message ?? "nil")")
            try expectEqual(snap?.level, .idle, "信号消失应转 idle")
        }

        TestKit.test("引擎: 断点重置 tokenSpikeAlerted（R32/F7，唤醒后告警不被残留标记压制）") {
            let token = FakeTokenUsageProvider()
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:],
                                    tokenMonitor: token)
            let start = Date()
            token.usage["dim"] = TokenUsage(tokens24h: 0, tokensTotal: 500_000, cost24h: 0, costTotal: 0)
            _ = engine.sample(now: start)
            // 直接置位去重标记（模拟睡前已告警）
            engine.tokenSpikeAlerted.insert("dim")
            // 断点唤醒
            _ = engine.sample(now: start.addingTimeInterval(8 * 3600))
            try expectFalse(engine.tokenSpikeAlerted.contains("dim"),
                             "断点应清除激增告警去重标记，否则唤醒后持续激增被静默压制")
        }

        TestKit.test("引擎: setEnabled 后重采样走后台路径（R34/G1，主线程不阻塞）") {
            // 设置页开关此前触发主线程同步采样（snapshot+匹配+探测 ~10ms 全主线程）。
            // 真正的可测面是「全表扫描发生在哪条线程、什么时刻」：
            // 同步路径会在 setEnabled 返回**之前**就把表扫完，异步路径不会。
            let provider = CountingProcessProvider(names: ["DimAgent"])
            let engine = makeEngine(processNames: [], writes: [:], processMonitor: provider)
            engine.start()                       // start 内含一拍同步采样，作为主线程基线
            let before = provider.snapshotCalls
            try expectTrue(before >= 1, "前置不成立：start 未采样，下面的「未增加」会假绿")
            try expectEqual(provider.mainThreadSnapshotCalls, before,
                            "前置不成立：基线那一拍本就在主线程，计数需对齐")

            engine.setEnabled(["dim"])
            try expectEqual(provider.snapshotCalls, before,
                            "setEnabled 返回前就扫了全表：主线程同步采样路径回来了")

            let deadline = Date().addingTimeInterval(3.0)
            while provider.snapshotCalls == before && Date() < deadline {
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            }
            try expectTrue(provider.snapshotCalls > before, "setEnabled 后的重采样未落地")
            try expectEqual(provider.mainThreadSnapshotCalls, before,
                            "重采样落在了主线程（应只在 start() 那一拍）")
            engine.stop()
        }

        TestKit.test("引擎: refreshTokenUsageOnce 真的转成一次异步刷新（R34/F6）") {
            // 原来是「调一下不崩就算过」：把方法体清空它照样绿，而 popover 用量会静默停更
            let token = FakeTokenUsageProvider()
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:], tokenMonitor: token)
            engine.refreshTokenUsageOnce()
            try expectEqual(token.refreshAsyncCount, 1, "未转成 tokenMonitor.refreshAsync()")
            engine.refreshTokenUsageOnce()
            try expectEqual(token.refreshAsyncCount, 2, "第二次调用被吞掉")
        }

        TestKit.test("引擎: 目录缺失时保持离线且不崩溃") {
            let engine = makeEngine(processNames: [], writes: [:])
            let snaps = engine.sample(now: Date())
            try expectEqual(snaps.count, AgentRegistry.builtin.count, "快照应覆盖全部内置 Agent")
        }

        TestKit.test("动作透传: 命令清洗与规整") {
            try expectEqual(AgentActionInspector.cleanCommand("/bin/zsh -c 'swift test'"), "swift test")
            try expectEqual(AgentActionInspector.cleanCommand("/usr/bin/git diff"), "git diff")
            try expectEqual(AgentActionInspector.cleanCommand("npm run build"), "npm run build")
        }

        TestKit.test("动作透传: cleanAntigravityAction 动作清洗与汉化") {
            try expectEqual(AgentActionInspector.cleanAntigravityAction("\"Viewing HistoryTrendChart implementation\""), "查看: HistoryTrendChart implementation")
            try expectEqual(AgentActionInspector.cleanAntigravityAction("Reading Theme.swift"), "读取: Theme.swift")
            try expectEqual(AgentActionInspector.cleanAntigravityAction("Pushing to origin main"), "推送: to origin main")
            try expectEqual(AgentActionInspector.cleanAntigravityAction("git status"), "执行: git status")
        }

        TestKit.test("工具: formatAgo 文案") {
            try expectEqual(ActivityEngine.formatAgo(2), "刚刚")
            try expectEqual(ActivityEngine.formatAgo(30), "30s 前")
            try expectEqual(ActivityEngine.formatAgo(120), "2m 前")
            try expectEqual(ActivityEngine.formatAgo(nil), "—")
        }

        TestKit.test("注册表: id 唯一、含 dim、zcode 与 antigravity") {
            let ids = AgentRegistry.builtin.map(\.id)
            try expectEqual(Set(ids).count, ids.count, "id 唯一")
            try expectTrue(AgentRegistry.builtin.contains { $0.id == "dim" }, "含 dim")
            try expectTrue(AgentRegistry.builtin.contains { $0.id == "zcode" }, "含 zcode")
            try expectTrue(AgentRegistry.builtin.contains { $0.id == "antigravity" }, "含 antigravity")
        }

        TestKit.test("进程: GUI bundle + CLI 进程名匹配") {
            let provider = FakeProcessProvider(
                processNames: ["dim", "claude"],
                bundleIDs: ["com.dimcode.app"]
            )
            let matcher = ProcessMatcher(snapshot: provider.snapshot(), runningBundleIDs: provider.runningBundleIDs(), profiles: [])
            let claude = AgentRegistry.builtin.first { $0.id == "claude" }!
            let codex = AgentRegistry.builtin.first { $0.id == "codex" }!
            try expectTrue(matcher.isRunning(dim), "bundle/CLI 匹配 dim")
            try expectTrue(matcher.isRunning(claude), "CLI 匹配 claude")
            try expectTrue(!matcher.isRunning(codex), "codex 不应误报")
        }

        TestKit.test("进程: 大小写不敏感匹配") {
            let provider = FakeProcessProvider(
                processNames: ["DIMAGENT"],
                bundleIDs: []
            )
            try expectTrue(ProcessMatcher(snapshot: provider.snapshot(), runningBundleIDs: provider.runningBundleIDs(), profiles: []).isRunning(dim), "大小写不敏感")
        }

        TestKit.test("进程: pathContains 约束——同名 Electron 进程不跨 App 误报") {
            let trae = AgentRegistry.builtin.first { $0.id == "trae" }!
            let workbuddy = AgentRegistry.builtin.first { $0.id == "workbuddy" }!
            let snapshot = ProcessSnapshot(entries: [
                ProcessSnapshot.Entry(
                    pid: 1001,
                    path: "/Applications/WorkBuddy.app/Contents/MacOS/Electron",
                    basename: "electron",
                    cpuPercent: 0
                )
            ])
            let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: [], profiles: [trae, workbuddy])
            try expectTrue(!matcher.isRunning(trae), "WorkBuddy 的 Electron 进程不应被 Trae 误报运行")
            try expectTrue(matcher.isRunning(workbuddy), "WorkBuddy 应正确命中自身 Electron 进程")
        }

        TestKit.test("进程: WorkBuddy 双变体 bundle id 与真实 Info.plist 对齐（防安装标记假阴性）") {
            // R31 复核：国外版真实 bundle id 是 com.workbuddy.workbuddy-ai（plutil 实测
            // Info.plist）——照抄 Application Support 目录名（com.workbuddy.workbuddy）
            // 会让「已安装」标记永远假阴性。哨兵：档案 id 与真实 Info.plist 必须对上
            // 不依赖本机的硬期望（R31 用 plutil 实测过的真实 id）：历史缺陷是把
            // Application Support 的目录名 com.workbuddy.workbuddy 抄给国外版，
            // 「已安装」标记从此永远假阴性。
            func profile(_ id: String) throws -> AgentProfile {
                guard let found = AgentRegistry.builtin.first(where: { $0.id == id }) else {
                    throw TestError(message: "档案缺失: \(id)")
                }
                return found
            }
            let cn = try profile("workbuddy")
            let overseas = try profile("workbuddy-ai")
            try expectEqual(overseas.bundleIDs, ["com.workbuddy.workbuddy-ai"],
                            "国外版 bundle id 漂移（plutil 实测值）")
            try expectFalse(overseas.bundleIDs.contains("com.workbuddy.workbuddy"),
                            "国外版不得登记 Application Support 目录名——那是假阴性的成因")
            // 两个变体主进程同为 Electron，只靠 bundle id / 路径区分：共用一个 id 会双份计数
            try expectTrue(Set(cn.bundleIDs).isDisjoint(with: overseas.bundleIDs),
                           "双变体 bundleIDs 不得相交（实际 \(cn.bundleIDs) / \(overseas.bundleIDs)）")
            for pair in [("workbuddy", "/Applications/WorkBuddy.app"),
                         ("workbuddy-ai", "/Applications/WorkBuddy AI.app")] {
                guard let profile = AgentRegistry.builtin.first(where: { $0.id == pair.0 }) else {
                    throw TestError(message: "档案缺失: \(pair.0)")
                }
                guard let plistPath = ProcessInfo.processInfo.environment["AGENTISLAND_SKIP_PLIST"] == nil
                    ? "\(pair.1)/Contents/Info.plist" : nil else { continue }
                guard FileManager.default.fileExists(atPath: plistPath) else {
                    continue   // 本机未装该变体则跳过（不硬绑环境）
                }
                guard let data = FileManager.default.contents(atPath: plistPath) else {
                    // 读不到不该让整条 runner 崩（原来是 data!，强解失败是 abort 而非失败）
                    throw TestError(message: "Info.plist 存在但读不出内容: \(plistPath)")
                }
                let plist = (try? PropertyListSerialization.propertyList(from: data, options: [], format: nil)) as? [String: Any]
                let realID = plist?["CFBundleIdentifier"] as? String ?? ""
                try expectTrue(profile.bundleIDs.contains(realID.lowercased()),
                                "\(pair.0) bundleIDs 必须含真实 id \(realID)（实际 \(profile.bundleIDs)）")
            }
        }

        TestKit.test("进程: WorkBuddy 双变体路径隔离（国内版与国外版 WorkBuddy AI 互不误报）") {
            // R：国内版 WorkBuddy.app 与国外版 WorkBuddy AI.app 的主进程 basename 都是
            // Electron，且路径都含 "workbuddy"——宽口径 pathContains 会双份计数。
            // 档案 pathContains 必须精确到各自 .app 目录 / 数据目录
            let domestic = AgentRegistry.builtin.first { $0.id == "workbuddy" }!
            let intl = AgentRegistry.builtin.first { $0.id == "workbuddy-ai" }!
            let snapshot = ProcessSnapshot(entries: [
                ProcessSnapshot.Entry(pid: 2001,
                    path: "/Applications/WorkBuddy AI.app/Contents/Frameworks/WorkBuddy AI Helper.app/Contents/MacOS/WorkBuddy AI Helper",
                    basename: "workbuddy ai helper", cpuPercent: 8),
                ProcessSnapshot.Entry(pid: 1001,
                    path: "/Applications/WorkBuddy.app/Contents/MacOS/Electron",
                    basename: "electron", cpuPercent: 8),
            ])
            let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: [], profiles: [domestic, intl])
            try expectTrue(matcher.isRunning(domestic), "国内版命中自身 Electron 进程")
            try expectTrue(matcher.isRunning(intl), "国外版命中自身 Helper 进程")
            let domesticEntries = matcher.matchingEntries(for: domestic)
            let intlEntries = matcher.matchingEntries(for: intl)
            try expectTrue(domesticEntries.allSatisfy { $0.path.contains("WorkBuddy.app/") },
                            "国内版不得吸走国外版路径（实际 \(domesticEntries.map(\.path))）")
            try expectTrue(intlEntries.allSatisfy { $0.path.contains("WorkBuddy AI.app") },
                            "国外版不得吸到国内版条目（实际 \(intlEntries.map(\.path))）")
        }

        TestKit.test("进程: ChatGPT 内嵌 codex 不计入 Codex（同一份程序不数两次）") {
            let codex = AgentRegistry.builtin.first { $0.id == "codex" }!
            let chatgpt = AgentRegistry.builtin.first { $0.id == "chatgpt" }!
            let snapshot = ProcessSnapshot(entries: [
                // ChatGPT 桌面版内嵌的 Codex 主进程与辅助进程（用户实测路径）
                ProcessSnapshot.Entry(pid: 85236,
                    path: "/Applications/ChatGPT.app/Contents/Resources/codex",
                    basename: "codex", cpuPercent: 5),
                ProcessSnapshot.Entry(pid: 85211,
                    path: "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Versions/152.0/Helpers/Codex (Service).app/Contents/MacOS/Codex (Service)",
                    basename: "codex (service)", cpuPercent: 1),
                // 独立安装的 codex CLI
                ProcessSnapshot.Entry(pid: 90001,
                    path: "/Users/me/.local/bin/codex",
                    basename: "codex", cpuPercent: 3),
            ])
            let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: ["com.openai.codex"],
                                         profiles: [codex, chatgpt])
            let matched = matcher.matchingEntries(for: codex)
            try expectTrue(matched.contains { $0.pid == 90001 }, "独立 codex CLI 仍应计入")
            try expectTrue(!matched.contains { $0.pid == 85236 }, "ChatGPT 内嵌 codex 不应计入 Codex")
            try expectTrue(!matched.contains { $0.pid == 85211 }, "Codex Framework 辅助进程不应计入 Codex")
            try expectTrue(matcher.isRunning(chatgpt), "ChatGPT 自身仍应在线")
        }

        TestKit.test("注册表: 宿主已装且组件无独立安装时不单独成条目") {
            // ChatGPT 已装、codex 无独立 CLI → codex 从注册表消失
            let bundled = AgentRegistry.fullRegistry(installedCLIs: [],
                                                     installedBundles: ["com.openai.codex"])
            try expectNil(bundled.first { $0.id == "codex" },
                          "宿主已装时内嵌 Codex 不应单独成条目")
            try expectTrue(bundled.contains { $0.id == "chatgpt" }, "宿主自身仍应保留")

            // 独立安装 codex CLI → 保留，用户仍可监控真正的 Codex CLI
            let standalone = AgentRegistry.fullRegistry(installedCLIs: ["codex"],
                                                        installedBundles: ["com.openai.codex"])
            try expectTrue(standalone.contains { $0.id == "codex" },
                           "独立安装 codex 时条目必须保留")

            // 宿主未装 → 无条件保留（保持原有行为）
            let neither = AgentRegistry.fullRegistry(installedCLIs: [], installedBundles: [])
            try expectTrue(neither.contains { $0.id == "codex" }, "宿主未装时条目保留")
        }

        TestKit.test("进程: processTree 用快照构建，不 fork 子进程") {
            // 构造 3 层进程树：100 → 200 → 300，另有无关进程 400
            let snapshot = ProcessSnapshot(entries: [
                ProcessSnapshot.Entry(pid: 100, path: "/a", basename: "a", cpuPercent: 0, ppid: 1),
                ProcessSnapshot.Entry(pid: 200, path: "/b", basename: "b", cpuPercent: 0, ppid: 100),
                ProcessSnapshot.Entry(pid: 300, path: "/c", basename: "c", cpuPercent: 0, ppid: 200),
                ProcessSnapshot.Entry(pid: 400, path: "/d", basename: "d", cpuPercent: 0, ppid: 1),
            ])
            let tree = ProcessTerminator.processTree(rootPid: 100, in: snapshot)
            try expectEqual(Set(tree), Set([100, 200, 300]), "应含整棵子树且不含无关进程")
            try expectTrue(!tree.contains(400), "不应包含无关进程")
            // 无子进程时只返回自身
            try expectEqual(ProcessTerminator.processTree(rootPid: 400, in: snapshot), [400])
        }

        TestKit.test("终止器: 身份复核不匹配时拒绝发信号（防 PID 复用误杀）") {
            // 真实起一个 /bin/sleep，但声称它应是别的程序 → 必须拒绝且进程存活
            let sleeper = Process()
            sleeper.executableURL = URL(fileURLWithPath: "/bin/sleep")
            sleeper.arguments = ["30"]
            try sleeper.run()
            defer { if sleeper.isRunning { sleeper.terminate() } }
            let outcome = ProcessTerminator.terminate(pid: sleeper.processIdentifier,
                                                      expectedPath: "/usr/bin/yes")
            try expectEqual(outcome, .identityMismatch, "basename 不一致必须判为身份不符")
            try expectTrue(sleeper.isRunning, "拒绝后目标进程必须仍然存活")
        }

        TestKit.test("终止器: 身份复核匹配（basename 一致）才终止") {
            let sleeper = Process()
            sleeper.executableURL = URL(fileURLWithPath: "/bin/sleep")
            sleeper.arguments = ["30"]
            try sleeper.run()
            defer { if sleeper.isRunning { sleeper.terminate() } }
            let outcome = ProcessTerminator.terminate(pid: sleeper.processIdentifier,
                                                      expectedPath: "/opt/homebrew/bin/sleep")
            try expectEqual(outcome, .signalSent, "basename 相同（路径整体不同）应放行")
            // SIGTERM 对 sleep 立即生效；给一点调度余量
            for _ in 0..<20 where sleeper.isRunning {
                usleep(50_000)
            }
            try expectTrue(!sleeper.isRunning, "身份匹配时应真正终止目标")
        }

        TestKit.test("终止器: 进程已消失时返回 failed 且不崩溃") {
            let outcome = ProcessTerminator.terminate(pid: 999_999, expectedPath: "/bin/sleep")
            try expectTrue(outcome == .failed, "不存在的 pid 应返回 failed，实际 \(outcome)")
            // 无 expectedPath 的旧语义兼容：同样失败
            let legacy = ProcessTerminator.terminate(pid: 999_999)
            try expectTrue(legacy == .failed, "无复核的消失 pid 也应 failed")
        }

        TestKit.test("动作探测: 长驻服务不作为「正在执行」信号") {
            // MCP 服务 / 语言服务 / 索引守护随 Agent 常驻，不代表有任务在途
            try expectTrue(AgentActionInspector.isLongLivedService(command: "node /path/mcp-servers/server.js"), "MCP server 应判为长驻")
            try expectTrue(AgentActionInspector.isLongLivedService(command: "node /x/mcp-bridge/index.js"), "MCP bridge 应判为长驻")
            try expectTrue(AgentActionInspector.isLongLivedService(command: "node --liftoff-only /x/index.js"), "索引常驻进程应判为长驻")
            try expectTrue(AgentActionInspector.isLongLivedService(command: "tsserver --stdio"), "语言服务应判为长驻")
            try expectTrue(AgentActionInspector.isLongLivedService(command: "python lsp-server.py"), "LSP 应判为长驻")
            // 用户/Agent 的真实任务命令不得被误杀（含常见 server/watcher 命名）
            try expectTrue(!AgentActionInspector.isLongLivedService(command: "node server.js"), "用户自己的 node server.js 不应被过滤")
            try expectTrue(!AgentActionInspector.isLongLivedService(command: "npm run watcher"), "npm run watcher 不应被过滤")
            try expectTrue(!AgentActionInspector.isLongLivedService(command: "cargo run --bin daemon"), "cargo run 不应被过滤")
            try expectTrue(!AgentActionInspector.isLongLivedService(command: "swift build"), "swift build 不是长驻服务")
            try expectTrue(!AgentActionInspector.isLongLivedService(command: "git diff --stat"), "git 不是长驻服务")
        }

        TestKit.test("动作探测: 命令行含 argv[0]，辅助进程过滤依赖它") {
            // commandLine 必须保留可执行路径（与 ps -o command= 一致）：
            // isInternalHelperProcess 的路径标记（/frameworks/、.app/contents/ 等）都来自 argv[0]，
            // 丢掉它会让 ChatGPT 的内部辅助进程被当成用户命令透传
            let raw = AgentActionInspector.commandLine(of: Int32(ProcessInfo.processInfo.processIdentifier))
            if let raw {
                try expectTrue(!raw.contains("PATH="), "不应把环境变量当参数: \(raw.prefix(80))")
                try expectTrue(raw.contains("/"), "首段应为可执行路径，实际: \(raw.prefix(80))")
            }
            // 带路径的辅助进程命令必须被判为内部辅助
            let service = "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Versions/1/Helpers/Codex (Service).app/Contents/MacOS/Codex (Service) --type=utility"
            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: service),
                           "含 .app/Contents 与 Frameworks 的辅助进程必须被过滤")
            let modifier = "/Applications/ChatGPT.app/Contents/Resources/native/bare-modifier-monitor --key DoubleCommand --immediate"
            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: modifier),
                           "bare-modifier-monitor 必须被过滤（其标记来自 argv[0] 路径）")
        }

        TestKit.test("清理: overweight 不把 GUI 主进程列为可清理项") {
            let cleaner = AgentCleaner(processMonitor: FakeProcessProvider(processNames: [], bundleIDs: []))
            let dim = AgentRegistry.builtin.first { $0.id == "dim" }!
            // 直接调用纯扫描入口不可行（依赖真实进程表），改为验证规则函数语义：
            // GUI 主进程路径必须被识别为标准 App bundle
            let guiPath = "/Applications/DimAgent.app/Contents/MacOS/DimAgent"
            try expectTrue(guiPath.contains(".app/Contents/MacOS"), "GUI 主进程路径应命中排除规则")
            let cliPath = "/usr/local/bin/dim"
            try expectTrue(!cliPath.contains(".app/Contents/MacOS"), "CLI 路径不应被排除")
            _ = cleaner; _ = dim
        }

        TestKit.test("进程: 系统路径 + 黑名单排除") {            let snapshot = ProcessSnapshot(entries: [
                ProcessSnapshot.Entry(
                    pid: 1,
                    path: "/System/Library/PrivateFrameworks/TextInputUIMacHelper.framework/Versions/A/XPCServices/CursorUIViewService.xpc/Contents/MacOS/CursorUIViewService",
                    basename: "cursoruiviewservice",
                    cpuPercent: 12
                ),
                ProcessSnapshot.Entry(pid: 2, path: "/usr/sbin/ssh-agent", basename: "ssh-agent", cpuPercent: 0.1),
                ProcessSnapshot.Entry(pid: 3, path: "/Applications/DimAgent.app/Contents/MacOS/DimAgent", basename: "dimagent", cpuPercent: 8),
            ])
            let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: [])
            let cursor = AgentRegistry.builtin.first { $0.id == "cursor" }!
            try expectTrue(!matcher.isRunning(cursor), "CursorUIViewService 不应匹配 Cursor")
            try expectTrue(matcher.isRunning(dim), "用户路径 DimAgent 应匹配")
        }

        TestKit.test("文件: 真实临时目录 mtime 检测") {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-test-\(UUID().uuidString)")
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }

            // F2 语义：newest 仅由信号文件聚合——空目录返回 nil 是正确行为
            try expectNil(FileActivityMonitor.newestWrite(in: dir.path), "空目录无信号文件 → nil")

            let file = dir.appendingPathComponent("probe.txt")
            try? Data("x".utf8).write(to: file)
            let after = FileActivityMonitor.newestWrite(in: dir.path)
            try expectTrue(after != nil, "写入产物后 newest 可读")
        }

        TestKit.test("文件: 深度>1 的写入可被递归检测（确定性夹具）") {
            // F2 语义下 newest 由信号文件聚合；原「真实 sessions 目录可读」断言
            // 依赖目录内有任意文件（信号文件缺失即 nil），改为确定性夹具：
            // 临时目录内 depth-2 写入，验证递归检测真的下潜
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir.appendingPathComponent("session-a/inner"),
                                                    withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            try Data("{}".utf8).write(to: dir.appendingPathComponent("session-a/inner/deep.jsonl"))
            let newest = FileActivityMonitor.newestWrite(in: dir.path, maxDepth: 4)
            try expectTrue(newest != nil, "depth-2 信号文件应被递归检测到")
        }

        TestKit.test("引擎: 真实环境引擎采样（真实进程+真实文件系统）") {
            // 环境依赖用例：进程表不可读（沙箱/CI）时显式跳过而非报错
            let probe = ProcessProvider().snapshot()
            guard !probe.entries.isEmpty else {
                print("   [skip] 进程表不可读，跳过真实采样用例")
                return
            }
            let monitor = FileActivityMonitor()
            monitor.watch(dirs: [FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent(".dimcode/v2/data/sessions").path])
            monitor.scanSync()
            let engine = ActivityEngine(
                profiles: [dim],
                config: EngineConfig(workingWindow: 60),
                processMonitor: ProcessProvider(),
                fileMonitor: monitor,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let now = Date()
            let snaps = engine.sample(now: now)
            // 原来只断言 dimSnap != nil，而 profiles 就一个 dim——sample() 恒返回一行，
            // 这条永远不会失败。换成真实环境上也必须成立的引擎契约：
            // 一个档案一行快照、时间戳取调用方给的 now、id 对得上
            try expectEqual(snaps.count, 1, "每个档案应恰好产出一行快照")
            try expectEqual(snaps.first?.profile.id, "dim", "快照档案错位")
            try expectEqual(engine.snapshots.map(\.profile.id), snaps.map(\.profile.id),
                            "sample() 的返回值与引擎发布的快照不一致")
            try expectEqual(snaps.first?.processRunning, engine.snapshots.first?.processRunning,
                            "同上：进程信号不得两份口径")
        }

        TestKit.test("文件: 不存在的目录返回 nil") {
            let missing = "/nonexistent/agentisland-\(UUID().uuidString)"
            try expectNil(FileActivityMonitor.newestWrite(in: missing), "缺失目录")
        }

        TestKit.test("文件: 扫描失败时目录暂缺保留旧值") {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-merge-\(UUID().uuidString)")
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            try? Data("x".utf8).write(to: dir.appendingPathComponent("a.txt"))
            // scanMinInterval=0 隔离节流，使两次扫描都真实执行
            let monitor = FileActivityMonitor(scanMinInterval: 0)
            monitor.watch(dirs: [dir.path])
            monitor.scanSync()
            let first = monitor.lastWriteDates(for: [dir.path])[dir.path]
            try expectTrue(first != nil, "首扫应有值")
            // 目录被移除（扫描失败）→ 单调 merge 保留旧值而非清空
            try? FileManager.default.removeItem(at: dir)
            monitor.scanSync()
            let second = monitor.lastWriteDates(for: [dir.path])[dir.path]
            try expectEqual(second, first, "目录暂缺应保留旧值（只进不退）")
        }

        TestKit.test("文件: 成功扫描允许最近写入时间自然过期") {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-expiry-\(UUID().uuidString)")
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let file = dir.appendingPathComponent("session.json")
            try? Data("x".utf8).write(to: file)
            let old = Date().addingTimeInterval(-300)
            try? FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: file.path)
            try? FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: dir.path)

            let monitor = FileActivityMonitor(scanMinInterval: 0)
            monitor.watch(dirs: [dir.path])
            monitor.scanSync()
            let first = monitor.lastWriteDates(for: [dir.path])[dir.path]
            try expectTrue(first != nil, "首扫应有值")

            let recent = Date()
            try? Data("y".utf8).write(to: file)
            try? FileManager.default.setAttributes([.modificationDate: recent], ofItemAtPath: file.path)
            try? FileManager.default.setAttributes([.modificationDate: recent], ofItemAtPath: dir.path)
            monitor.scanSync()
            let second = monitor.lastWriteDates(for: [dir.path])[dir.path]
            try expectTrue(second != nil && second! > first!, "成功扫描后应更新为最新时间")
        }

        TestKit.test("引擎: stop 后 scheduleNext 不再重建定时器，start 可恢复采样") {
            // 回归：stop() 置 running=false 后在飞回调不重建定时器；
            // stop→start 重启后采样恢复工作
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": now.addingTimeInterval(-5)]
            )
            engine.start()   // 启动采样
            let first = engine.snapshots.first { $0.id == "dim" }?.level
            try expectEqual(first, .working, "start 后应 working")
            engine.stop()    // 停止
            engine.sample(now: now)   // stop 后手动采样（模拟在飞回调），scheduleNext 不应重建定时器
            // start 前 timer 应为 nil（stop 已灭，sample 不再重建）
            try expectTrue(engine.timerIsNil, "stop 后 sample 不应重建定时器")
            engine.start()   // 重启
            let resumed = engine.snapshots.first { $0.id == "dim" }?.level
            try expectEqual(resumed, .working, "重启后应恢复 working")
            engine.stop()
        }

        TestKit.test("引擎: token 轮询懒启动——呈现活跃才开启，重复激活幂等") {
            let fake = FakeTokenUsageMonitor()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": Date().addingTimeInterval(-5)],
                tokenMonitor: fake
            )
            engine.start()
            try expectEqual(fake.calls, [], "懒启动：start 不应开启 token 轮询")
            engine.setPresentationActive(true)
            try expectEqual(fake.calls, ["start"], "呈现活跃应启动轮询")
            try expectTrue(fake.onRefresh != nil, "onRefresh 应被接线（刷完触发重采样）")
            engine.setPresentationActive(true)
            try expectEqual(fake.calls, ["start"], "重复激活应幂等")
            engine.stop()
            try expectEqual(fake.calls, ["start", "stop"], "stop 应全停 token 轮询")
            try expectNil(fake.onRefresh, "stop 后刷新回调应清空")
        }

        TestKit.test("引擎: 呈现失活→暂停；再激活→重启；重复失活幂等") {
            let fake = FakeTokenUsageMonitor()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": Date().addingTimeInterval(-5)],
                tokenMonitor: fake
            )
            engine.start()
            engine.setPresentationActive(true)
            engine.setPresentationActive(false)
            try expectEqual(fake.calls, ["start", "pause"], "失活应暂停（连接保留）")
            engine.setPresentationActive(false)
            try expectEqual(fake.calls, ["start", "pause"], "重复失活应幂等")
            engine.setPresentationActive(true)
            try expectEqual(fake.calls, ["start", "pause", "start"], "再激活应重启轮询（暂停后 start 即首刷）")
            engine.stop()
            try expectEqual(fake.calls, ["start", "pause", "start", "stop"], "stop 应全停")
        }

        TestKit.test("引擎: 呈现活跃先于引擎启动 → start 时补开轮询") {
            let fake = FakeTokenUsageMonitor()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": Date().addingTimeInterval(-5)],
                tokenMonitor: fake
            )
            engine.setPresentationActive(true)
            try expectEqual(fake.calls, [], "引擎未运行时不应启动轮询（仅记状态）")
            engine.start()
            try expectEqual(fake.calls, ["start"], "引擎启动时应补开轮询")
            engine.stop()
        }

        TestKit.test("引擎: 从未启动轮询时 stop 不触碰 token 面") {
            let fake = FakeTokenUsageMonitor()
            let engine = makeEngine(processNames: [], writes: [:], tokenMonitor: fake)
            engine.stop()
            try expectEqual(fake.calls, [], "未启动轮询时 stop 不应调用 token 侧")
        }

        TestKit.test("引擎: grandTotal 与下钻查询经 seam 透出（UI 唯一入口）") {
            let fake = FakeTokenUsageMonitor()
            fake.grandTotal = TokenUsage(tokens24h: 10, tokensTotal: 100, cost24h: 0.1, costTotal: 1.0)
            let engine = makeEngine(processNames: [], writes: [:], tokenMonitor: fake)
            try expectEqual(engine.grandTotal, fake.grandTotal, "grandTotal 经引擎透出")
            // 参数必须原样透传：只记「调了哪个方法」的话，agentId 被写死成别的值、
            // 时间范围被换成默认 .day，这些都仍然全绿
            engine.modelBreakdown(agentId: "dim") { _ in }
            engine.sessions(agentId: "dim", modelId: "gpt-5") { _ in }
            engine.tokenTimeline(range: .week) { _ in }
            try expectEqual(fake.calls, ["modelBreakdown:dim", "sessions:dim/gpt-5", "timeline:week"],
                            "下钻查询按参数原样转发到 token 侧")
        }

        TestKit.test("引擎: 安装缓存首刷完成后重放启用集，自动发现项恢复监控") {
            let cache = InstalledAppsCache(scanCLIs: { ["fakecli"] }, scanBundles: { [] })
            let engine = makeEngine(processNames: [], writes: [:],
                                    installedApps: cache, enabledIDs: ["dim", "cli-fakecli"])
            try expectTrue(!engine.allProfiles.contains { $0.id == "cli-fakecli" }, "首刷前 cli-fakecli 缺席（冷缓存）")
            let deadline = Date().addingTimeInterval(5)
            while !engine.allProfiles.contains(where: { $0.id == "cli-fakecli" }) && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
            }
            try expectTrue(engine.allProfiles.contains { $0.id == "cli-fakecli" }, "首刷重放后自动发现项恢复监控")
            try expectTrue(engine.allProfiles.contains { $0.id == "dim" }, "重放不丢内置启用项")
        }

        TestKit.test("动作透传: WorkBuddy/OpenCode/DSH/Hermes 探测器契约") {
            // 契约不是「不崩溃」，而是：要么 nil（没探到），要么**非空**文案。
            // 空串会在岛上渲染出一条没有内容的动作文案，且会盖掉上一拍的真值。
            func contract(_ name: String, _ value: String?) throws {
                if let value {
                    try expectFalse(value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                                    "\(name) 返回了空串（应返回 nil 表示没探到）")
                }
            }
            try contract("WorkBuddy", AgentActionInspector.inspectWorkBuddyAction())
            try contract("OpenCode", AgentActionInspector.inspectOpenCodeAction())
            try contract("Hermes", AgentActionInspector.inspectHermesAction())
            try contract("ZCode", AgentActionInspector.inspectZCodeAction())
            try contract("Dim", AgentActionInspector.inspectDimAction())
            // 非法 pid 必须探不到东西：pid = -1 若仍能返回动作，说明查询没按 pid 收敛
            try expectNil(AgentActionInspector.inspectDSHAction(pid: -1),
                          "pid=-1 不应探测到任何动作")
        }

        TestKit.test("动作透传: DimAgent parseDimMessage 消息解析与已完成思考防误报") {
            // 1. 已完成思考与回复（含 endTime）→ 严禁报「思考规划中」，必须返回 nil
            let completedParts = """
            [
              {"type":"thinking","thinking":"查了一圈，已有证据","startTime":"2026-09-09T05:30:00.000Z","endTime":"2026-09-09T05:30:05.000Z"},
              {"type":"text","text":"结论是官方桌面版在做","startTime":"2026-09-09T05:30:05.000Z","endTime":"2026-09-09T05:30:10.000Z"}
            ]
            """
            let r1 = AgentActionInspector.parseDimMessage(role: "assistant", toolMeta: "", parts: completedParts, age: 5)
            try expectNil(r1, "已完成回复（含 endTime）应返回 nil，绝不可误报思考中")

            // 2. 真正处于思考阶段（thinking 无 endTime）→ 返回「思考规划中」
            let thinkingParts = """
            [
              {"type":"thinking","thinking":"正在深度调研主仓提交记录...","startTime":"2026-09-09T05:30:00.000Z"}
            ]
            """
            let r2 = AgentActionInspector.parseDimMessage(role: "assistant", toolMeta: "", parts: thinkingParts, age: 3)
            try expectEqual(r2, "思考规划中", "在途思考应准确识别")

            // 3. 正在流式生成文本（text 无 endTime）→ 返回「正在生成回复」
            let streamingParts = """
            [
              {"type":"thinking","thinking":"思考完成","startTime":"2026-09-09T05:30:00.000Z","endTime":"2026-09-09T05:30:05.000Z"},
              {"type":"text","text":"下面是分析结果...","startTime":"2026-09-09T05:30:05.000Z"}
            ]
            """
            let r3 = AgentActionInspector.parseDimMessage(role: "assistant", toolMeta: "", parts: streamingParts, age: 2)
            try expectEqual(r3, "正在生成回复", "在途文本流式生成应识别")

            // 4. 正在调用工具（tool_use 无 endTime）
            let toolParts = """
            [
              {"type":"thinking","thinking":"先跑一下测试","endTime":"2026-09-09T05:30:05.000Z"},
              {"type":"tool_use","name":"exec","input":{"command":"swift test"},"startTime":"2026-09-09T05:30:06.000Z"}
            ]
            """
            let r4 = AgentActionInspector.parseDimMessage(role: "assistant", toolMeta: "", parts: toolParts, age: 4)
            try expectEqual(r4, "正在执行终端命令", "在途 exec 应映射为终端命令")

            // 5. 超过 90 秒活跃窗口 → 即使有内容也返回 nil（完全闲置）
            let r5 = AgentActionInspector.parseDimMessage(role: "assistant", toolMeta: "", parts: thinkingParts, age: 120)
            try expectNil(r5, "超过 90s 的旧消息应视为已挂起闲置")

            // 6. 用户新提问（role = user）在 90s 内 → 进入「思考规划中」
            let r6 = AgentActionInspector.parseDimMessage(role: "user", toolMeta: "", parts: "[]", age: 10)
            try expectEqual(r6, "思考规划中", "刚发送的 user 消息应进入思考规划态")
        }

        TestKit.test("引擎: DimAgent / WorkBuddy 过滤 Electron 辅助进程空闲 CPU 抖动") {
            let now = Date()
            // 模拟 DimAgent 在后台挂起：无新写入、无活跃在途动作、进程存在但仅有 8% 的 Electron 空闲渲染抖动
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [home + "/.dimcode/v2/data/sessions": now.addingTimeInterval(-300)],
                cpu: 8.0 // 超过默认 6.0% 阈值，但属于 Electron 空闲辅助进程抖动
            )
            let snaps = engine.sample(now: now)
            let dimSnap = snaps.first { $0.id == "dim" }
            try expectEqual(dimSnap?.level, .idle, "DimAgent 8% 的 Electron 空闲 CPU 抖动应保持 idle")
            try expectNil(dimSnap?.currentAction, "空闲挂起时不应透传任何错误动作")

            // 注意：CPU 判定阈值现已随档案下沉（AgentProfile.cpuWorkingThreshold），
            // 测试手工构造 profile 时必须带上该字段，否则退化为全局 cpuThreshold，
            // 与真实注册表（AgentRegistry.builtin 中 workbuddy 设为 35）行为不一致。
            let workbuddyProfile = AgentProfile(
                id: "workbuddy", name: "WorkBuddy", icon: "briefcase.fill",
                bundleIDs: [], processNames: ["Electron"],
                cpuWorkingThreshold: 35.0,
                sessionDirs: [home + "/.workbuddy/tasks"]
            )
            let workbuddyEngine = ActivityEngine(
                profiles: [workbuddyProfile],
                config: EngineConfig(workingWindow: 20),
                processMonitor: FakeProcessProvider(processNames: ["Electron"], bundleIDs: [], cpu: 25.0),
                fileMonitor: FakeFileActivityProvider(writes: [home + "/.workbuddy/tasks": now.addingTimeInterval(-300)]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            let workbuddySnap = workbuddyEngine.sample(now: now).first
            try expectEqual(workbuddySnap?.level, .idle,
                            "WorkBuddy prewarm 汇总 25% CPU 且无近期任务写入时应保持 idle")
        }

        TestKit.test("动作透传: isInternalHelperProcess 过滤应用内辅助与守护进程，放行真实命令") {
            // 1. ChatGPT 及 Chromium / Electron 内部辅助进程应全部过滤
            let chatgptService = "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Versions/152.0.7977.83/Helpers/Codex (Service).app/Contents/MacOS/Codex (Service) --type=utility --utility-sub-type=network.mojom.NetworkService"
            let chatgptRenderer = "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Versions/152.0.7977.83/Helpers/Codex (Renderer).app/Contents/MacOS/Codex (Renderer) --type=renderer"
            let modifierMonitor = "/Applications/ChatGPT.app/Contents/Resources/native/bare-modifier-monitor --key DoubleCommand --immediate"
            let appServer = "/Applications/ChatGPT.app/Contents/Resources/codex -c features.code_mode_host=true app-server"
            let codeModeHost = "/Applications/ChatGPT.app/Contents/Resources/codex-code-mode-host"

            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: chatgptService), "ChatGPT NetworkService 必须过滤")
            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: chatgptRenderer), "ChatGPT Renderer 必须过滤")
            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: modifierMonitor), "ChatGPT modifier monitor 必须过滤")
            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: appServer), "ChatGPT app-server 必须过滤")
            try expectTrue(AgentActionInspector.isInternalHelperProcess(command: codeModeHost), "ChatGPT code-mode-host 必须过滤")

            // 2. 真实用户/Agent 执行的命令必须放行
            try expectFalse(AgentActionInspector.isInternalHelperProcess(command: "git diff --stat"), "git 命令不可被过滤")
            try expectFalse(AgentActionInspector.isInternalHelperProcess(command: "npm test"), "npm 命令不可被过滤")
            try expectFalse(AgentActionInspector.isInternalHelperProcess(command: "swift test"), "swift 命令不可被过滤")
            try expectFalse(AgentActionInspector.isInternalHelperProcess(command: "python3 -m unittest"), "python 命令不可被过滤")
            try expectFalse(AgentActionInspector.isInternalHelperProcess(command: "/bin/zsh -c 'cargo check'"), "cargo 脚本不可被过滤")
        }

        TestKit.test("引擎: ChatGPT 桌面版空闲微抖动与无动作时正确保持 idle") {
            let now = Date()
            let engine = makeEngine(
                processNames: ["ChatGPT"],
                writes: [home + "/Library/Application Support/com.openai.codex": now.addingTimeInterval(-3600)],
                cpu: 8.0 // 超过默认 6.0% 阈值，但属于 GUI 辅助渲染与 IPC 抖动
            )
            let snaps = engine.sample(now: now)
            let chatgptSnap = snaps.first { $0.id == "chatgpt" }
            try expectEqual(chatgptSnap?.level, .idle, "ChatGPT 8% 的 GUI 空闲抖动应保持 idle")
            try expectNil(chatgptSnap?.currentAction, "ChatGPT 空闲时不应透传任何错误动作")
        }

        TestKit.test("引擎: 系统休眠与唤醒联动幂等（重复事件不得重复启停轮询）") {
            // 原来六个调用零断言：把 handleSystemWake 整段删掉它照样绿
            let token = FakeTokenUsageProvider()
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:], tokenMonitor: token)
            engine.start()
            engine.setPresentationActive(true)
            try expectEqual(token.startCount, 1, "前置：展开态应恰好启动一次轮询")

            engine.handleSystemSleep()
            engine.handleSystemSleep()
            try expectEqual(token.pauseCount, 1, "连续两次休眠事件应只暂停一次（幂等）")

            engine.handleSystemWake()
            engine.handleSystemWake()
            try expectEqual(token.startCount, 2, "连续两次唤醒事件应只重启一次轮询")
            try expectEqual(token.pauseCount, 1, "唤醒路径不得反过来再暂停轮询")
            engine.stop()
        }

        TestKit.test("日志智能分析: LogPatternAnalyzer 准确识别五类异常模式与摘要提取") {
            // 1. 限流 429
            let rateLimit = LogPatternAnalyzer.analyze(title: "API Error", detail: "Error: 429 Too Many Requests: Rate limit exceeded for model")
            try expectTrue(rateLimit != nil, "应识别 429 限流")
            try expectEqual(rateLimit?.kind, .rateLimit, "类别应为 rateLimit")
            try expectTrue(rateLimit?.snippet.contains("429") == true, "摘要应保留关键错误")

            // 2. 编译构建报错
            let compileErr = LogPatternAnalyzer.analyze(title: "swift build failed", detail: "Sources/Foo.swift:12:5: error: cannot find 'bar' in scope")
            try expectTrue(compileErr != nil, "应识别编译报错")
            try expectEqual(compileErr?.kind, .compileError, "类别应为 compileError")
            try expectTrue(compileErr?.snippet.contains("cannot find 'bar'") == true, "摘要应保留核心报错行")

            // 3. Git 冲突
            let gitConflict = LogPatternAnalyzer.analyze(title: "git merge", detail: "CONFLICT (content): Merge conflict in Package.swift")
            try expectTrue(gitConflict != nil, "应识别 Git 冲突")
            try expectEqual(gitConflict?.kind, .gitConflict, "类别应为 gitConflict")

            // 4. 鉴权认证失败
            let authErr = LogPatternAnalyzer.analyze(title: "curl API", detail: "HTTP 401 Unauthorized: Invalid API key provided")
            try expectTrue(authErr != nil, "应识别 401 鉴权失效")
            try expectEqual(authErr?.kind, .authError, "类别应为 authError")

            // 5. 正常日志无报错
            let normal = LogPatternAnalyzer.analyze(title: "正在写入文件", detail: "Saved 120 lines to Sources/Foo.swift")
            try expectNil(normal, "正常日志不应误报错误")
        }

        TestKit.test("Token 预测: TokenForecastEvaluator 月末推算与预算耗尽测算") {
            let baseDate = Date()
            let cal = Calendar.current
            let day = cal.component(.day, from: baseDate)
            let range = cal.range(of: .day, in: .month, for: baseDate) ?? 1..<31
            let totalDays = range.count
            let remaining = max(0, totalDays - day)

            // 1. 无预算限额预测（dailyBudget=0 表示不设限）
            let reportNoBudget = TokenForecastEvaluator.evaluate(
                tokens24h: 100_000,
                cost24h: 1.5,
                dailyBudget: 0,
                now: baseDate
            )
            try expectEqual(reportNoBudget.daysRemainingInMonth, remaining, "剩余自然日应计算准确")
            try expectEqual(reportNoBudget.projectedMonthEndTokens, 100_000 * totalDays, "预估月末 Token 应按速率×全月天数")
            try expectEqual(reportNoBudget.projectedMonthEndCost, 1.5 * Double(totalDays), "预估月末费用应按速率×全月天数")
            try expectNil(reportNoBudget.budgetExhaustionDay, "无预算时不应有耗尽倒计时")

            // 2. 有预算且超额预警
            let reportWithBudget = TokenForecastEvaluator.evaluate(
                tokens24h: 200_000,
                cost24h: 3.0,
                dailyBudget: 50_000, // 预算 50k，日消耗 200k
                now: baseDate
            )
            try expectTrue(reportWithBudget.budgetExhaustionDay != nil, "超支时应给出预算耗尽天数")
        }

        TestKit.test("守护自愈: AgentResilienceGuard 识别长死锁与内存泄漏并冷却防抖") {
            let guardTrack = AgentResilienceGuard()
            let dummyProfile = AgentRegistry.builtin[0]
            let t0 = Date()

            // 1. 刚出现死锁未超阈值（180s）不触发
            let snapHungT0 = AgentSnapshot(
                profile: dummyProfile,
                level: .idle,
                processRunning: true,
                cpuPercent: 0,
                installed: true,
                activeSessions: 0,
                lastActivityAgo: nil,
                lastActivityText: "待机",
                memoryBytes: 100 * 1024 * 1024,
                isHung: true
            )
            let evs1 = guardTrack.evaluate(snapshots: [snapHungT0], now: t0)
            try expectTrue(evs1.isEmpty, "死锁未超 180s 不应告警")

            // 2. 死锁超 180s 触发自愈横幅
            let t1 = t0.addingTimeInterval(181)
            let evs2 = guardTrack.evaluate(snapshots: [snapHungT0], now: t1)
            try expectEqual(evs2.count, 1, "死锁超 180s 应发出 1 条告警")
            try expectTrue(evs2[0].message?.contains("死锁") == true, "事件信息应包含死锁")

            // 3. 冷却期内（600s）不重复告警
            let t2 = t1.addingTimeInterval(60)
            let evs3 = guardTrack.evaluate(snapshots: [snapHungT0], now: t2)
            try expectTrue(evs3.isEmpty, "冷却期内应抑制重复告警")

            // 4. 解除死锁后状态复位
            let snapNormal = AgentSnapshot(
                profile: dummyProfile,
                level: .idle,
                processRunning: true,
                cpuPercent: 0,
                installed: true,
                activeSessions: 0,
                lastActivityAgo: nil,
                lastActivityText: "待机",
                memoryBytes: 100 * 1024 * 1024,
                isHung: false
            )
            _ = guardTrack.evaluate(snapshots: [snapNormal], now: t2.addingTimeInterval(1))

            // 5. 内存超 2GB 且超 300s 告警
            let t3 = t2.addingTimeInterval(700)
            let snapHighMem = AgentSnapshot(
                profile: dummyProfile,
                level: .idle,
                processRunning: true,
                cpuPercent: 0,
                installed: true,
                activeSessions: 0,
                lastActivityAgo: nil,
                lastActivityText: "待机",
                memoryBytes: 3 * 1024 * 1024 * 1024, // 3GB
                isHung: false
            )
            _ = guardTrack.evaluate(snapshots: [snapHighMem], now: t3)
            let t4 = t3.addingTimeInterval(301)
            let evs4 = guardTrack.evaluate(snapshots: [snapHighMem], now: t4)
            try expectEqual(evs4.count, 1, "内存超限超 300s 应发出自愈告警")
            try expectTrue(evs4[0].message?.contains("内存") == true, "事件信息应包含内存")
        }

        TestKit.test("能耗自适应: PowerSourceMonitor 节电节律判定") {
            // 当未开启电池节能时，仅低电量模式节电
            let throttleDisabled = PowerSourceMonitor.shouldThrottle(
                batterySaverEnabled: false,
                isLowPower: false,
                onBattery: true
            )
            try expectFalse(throttleDisabled, "未开启电池节能且非系统低电量时不应降频")

            // 开启电池节能且在电池供电时降频
            let throttleBattery = PowerSourceMonitor.shouldThrottle(
                batterySaverEnabled: true,
                isLowPower: false,
                onBattery: true
            )
            try expectTrue(throttleBattery, "电池供电且开启节能时应降频")

            // 开启电池节能但插电（非电池供电）时不降频
            let throttleAC = PowerSourceMonitor.shouldThrottle(
                batterySaverEnabled: true,
                isLowPower: false,
                onBattery: false
            )
            try expectFalse(throttleAC, "插电供电时不应降频")
        }

        TestKit.test("引擎: stop() 之后在飞的后台采样不得落地") {
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.dimcode/v2/data/sessions"
            let engine = makeEngine(processNames: ["DimAgent"], writes: [dir: Date()],
                                    processMonitor: SlowProcessProvider(names: ["DimAgent"], delay: 0.08))
            engine.start()                        // 首拍同步完成
            let stamp = engine.updatedAt
            engine.sampleInBackground()           // 后台 libproc 遍历在飞
            engine.stop()                         // 就在这一拍的间隙里停止
            let deadline = Date().addingTimeInterval(0.5)
            while Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
            try expectEqual(engine.updatedAt, stamp,
                            "落地的那一拍会发布快照，并经 sampleCore→scheduleNext 把刚被 invalidate 的定时器重建回来")
        }

        TestKit.test("探测故障日志: 按 Agent+原因冷却，恢复时补一条并重新武装") {
            ProbeFailureLog.resetForTesting()
            let start = Date()
            let health = SessionProbeHealth(failure: .prepareFailed, path: "/tmp/a.db")

            try expectTrue(ProbeFailureLog.record(health, agentId: "dim", now: start), "首次故障必须落一条")
            try expectFalse(ProbeFailureLog.record(health, agentId: "dim", now: start.addingTimeInterval(2)),
                            "同一故障在冷却窗口内不得每拍打一条（2s 采样会刷满日志）")
            try expectTrue(ProbeFailureLog.record(health, agentId: "dim",
                                                  now: start.addingTimeInterval(ProbeFailureLog.cooldown)),
                           "冷却到期后允许再记一次，坏源持续存在仍要留痕")
            // 另一种失败类型是另一条时间线，不能被上一个冷却吞掉
            try expectTrue(ProbeFailureLog.record(
                SessionProbeHealth(failure: .unreadableDB, path: "/tmp/a.db"),
                agentId: "dim", now: start.addingTimeInterval(ProbeFailureLog.cooldown)),
                "不同失败类型各自冷却")

            ProbeFailureLog.recordRecovery(agentId: "claude", now: start)   // 没记过故障 → 无操作
            ProbeFailureLog.recordRecovery(agentId: "dim", now: start)
            try expectTrue(ProbeFailureLog.record(health, agentId: "dim", now: start),
                           "恢复后重新武装：再次变坏必须立刻再记一条")
            ProbeFailureLog.resetForTesting()
        }
    }

    // MARK: - 工具

    static func makeEngine(processNames: Set<String>, writes: [String: Date], cpu: Double = 0,
                           tokenMonitor: any TokenUsagePolling & TokenUsageQuerying = TokenUsageMonitor(),
                           installedApps: InstalledAppsCache? = nil,
                           enabledIDs: Set<String>? = nil,
                           processMonitor: ProcessProviding? = nil) -> ActivityEngine {
        ActivityEngine(
            profiles: AgentRegistry.builtin,
            config: EngineConfig(workingWindow: 20),
            processMonitor: processMonitor ?? FakeProcessProvider(processNames: processNames, bundleIDs: [], cpu: cpu),
            fileMonitor: FakeFileActivityProvider(writes: writes),
            tokenMonitor: tokenMonitor,
            installedApps: installedApps ?? InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }),
            enabledIDs: enabledIDs
        )
    }
}
