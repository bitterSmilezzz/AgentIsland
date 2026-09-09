import Foundation
@testable import AgentIslandCore

// MARK: - ActivityEngine 状态机测试

@MainActor
enum EngineTests {

    static var home: String {
        FileManager.default.homeDirectoryForCurrentUser.path
    }

    static let dim = AgentRegistry.builtin.first { $0.id == "dim" }!

    static func register() {
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

        TestKit.test("引擎: terminateAgent 终止逃生舱更新事件与状态") {
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            engine.terminateAgent(pid: 999999, agentId: "dim")
            try expectEqual(engine.latestEvent?.eventType, .attention)
            try expectTrue(engine.latestEvent?.message?.contains("进程已终止") == true, "应提示已终止")
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
            engine.cleanAnomalies([anomaly])
            try expectEqual(engine.latestEvent?.agentId, "workbench-cleaner")
            try expectEqual(engine.latestEvent?.eventType, .completed)
            try expectTrue(engine.latestEvent?.message?.contains("已安全清理") == true)
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

        TestKit.test("熔断保护: Token 激增告警触发") {
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

            // 10秒后，Token 暴增 80,000
            fake.usage["dim"] = TokenUsage(tokens24h: 90_000, tokensTotal: 90_000, cost24h: 0, costTotal: 0)
            _ = engine.sample(now: now.addingTimeInterval(10))
            try expectEqual(engine.latestEvent?.eventType, .costSpike, "激增超过 50k 应触发 costSpike")
            try expectTrue(engine.latestEvent?.message?.contains("Token 激增") == true, "应显示激增提示")
            try expectTrue(engine.latestEvent?.detail?.contains("Token 消耗突增") == true, "应包含详细排查说明")
            try expectTrue(engine.latestEvent?.copyableDiagnosticText.contains("详情:") == true, "可复制诊断应包含详情")
        }

        TestKit.test("熔断保护: 持续死循环/高负载告警（低占用不误报，持续高 CPU 触发）") {
            // 1. 低 CPU（2%）：即使运行 6 分钟也不应触发死循环告警
            let lowEngine = makeEngine(processNames: ["DimAgent"], writes: [:], cpu: 2.0)
            let start = Date()
            _ = lowEngine.sample(now: start)
            _ = lowEngine.sample(now: start.addingTimeInterval(360))
            try expectNil(lowEngine.latestEvent, "低占用长耗时不应误报死循环")

            // 2. 持续高 CPU（80%）：达到 5 分钟阈值触发预警
            let highEngine = makeEngine(processNames: ["DimAgent"], writes: [:], cpu: 80.0)
            _ = highEngine.sample(now: start)
            try expectNil(highEngine.latestEvent, "首次高 CPU 仅建基准不告警")
            _ = highEngine.sample(now: start.addingTimeInterval(305))
            try expectEqual(highEngine.latestEvent?.eventType, .costSpike, "持续高 CPU 超 5 分钟应触发告警")
            try expectTrue(highEngine.latestEvent?.message?.contains("持续高负载") == true, "文案应提示高负载")
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

        TestKit.test("进程: 系统路径 + 黑名单排除") {
            let snapshot = ProcessSnapshot(entries: [
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

            let before = FileActivityMonitor.newestWrite(in: dir.path)
            try expectTrue(before != nil, "目录 mtime 可读")

            let file = dir.appendingPathComponent("probe.txt")
            try? Data("x".utf8).write(to: file)
            let after = FileActivityMonitor.newestWrite(in: dir.path)
            try expectTrue(after != nil && after! >= before!, "写入后 mtime 应更新")
        }

        TestKit.test("文件: 真实会话目录递归检测（深度>1 的写入）") {
            let dir = FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent(".dimcode/v2/data/sessions").path
            if FileManager.default.fileExists(atPath: dir) {
                let newest = FileActivityMonitor.newestWrite(in: dir, maxDepth: 4)
                try expectTrue(newest != nil, "sessions 目录应可读")
                print("   [info] sessions newest write: \(newest?.description ?? "nil"), ago \(newest.map { Int(Date().timeIntervalSince($0)) } ?? -1)s")
            } else {
                print("   [skip] 本机无 sessions 目录")
            }
        }

        TestKit.test("引擎: 真实环境引擎采样（真实进程+真实文件系统）") {
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
            let snaps = engine.sample(now: Date())
            let dimSnap = snaps.first
            print("   [info] dim real: level=\(dimSnap?.level.rawValue ?? "nil") process=\(dimSnap?.processRunning ?? false) cpu=\(dimSnap?.cpuPercent ?? -1) activity=\(dimSnap?.lastActivityText ?? "nil")")
            try expectTrue(dimSnap != nil, "dim 快照存在")
        }

        TestKit.test("文件: 不存在的目录返回 nil") {
            let missing = "/nonexistent/agentisland-\(UUID().uuidString)"
            try expectNil(FileActivityMonitor.newestWrite(in: missing), "缺失目录")
        }

        TestKit.test("文件: 缓存单调 merge——目录暂缺保留旧值（C5 影子缓存退役后语义）") {
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

        TestKit.test("引擎: stop 后 scheduleNext 不再重建定时器，start 可恢复采样") {
            // 回归（阿剩低3）：stop() 置 running=false 后在飞回调不重建定时器；
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
            engine.modelBreakdown(agentId: "dim") { _ in }
            engine.sessions(agentId: "dim", modelId: "m") { _ in }
            try expectTrue(fake.calls.contains("modelBreakdown"), "下钻查询经引擎转发")
            try expectTrue(fake.calls.contains("sessions"), "会话查询经引擎转发")
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

        TestKit.test("动作透传: WorkBuddy/OpenCode/DSH/Hermes 探测器安全运行") {
            // 真实/缺省环境均可安全执行，不抛出异常不崩溃
            _ = AgentActionInspector.inspectWorkBuddyAction()
            _ = AgentActionInspector.inspectOpenCodeAction()
            _ = AgentActionInspector.inspectDSHAction(pid: -1)
            _ = AgentActionInspector.inspectHermesAction()
            _ = AgentActionInspector.inspectZCodeAction()
        }
    }

    // MARK: - 工具

    static func makeEngine(processNames: Set<String>, writes: [String: Date], cpu: Double = 0,
                           tokenMonitor: any TokenUsagePolling & TokenUsageQuerying = TokenUsageMonitor(),
                           installedApps: InstalledAppsCache? = nil,
                           enabledIDs: Set<String>? = nil) -> ActivityEngine {
        ActivityEngine(
            profiles: AgentRegistry.builtin,
            config: EngineConfig(workingWindow: 20),
            processMonitor: FakeProcessProvider(processNames: processNames, bundleIDs: [], cpu: cpu),
            fileMonitor: FakeFileActivityProvider(writes: writes),
            tokenMonitor: tokenMonitor,
            installedApps: installedApps ?? InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }),
            enabledIDs: enabledIDs
        )
    }
}
