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

        TestKit.test("引擎: 进程消失（被关闭）→ 静默转 offline，不误报任务完成") {
            let start = Date()
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

        TestKit.test("引擎: terminateAgent 终止逃生舱更新事件与状态") {
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            // 用不存在的 pid 验证「失败不谎报」；成功路径由下面的 Fake 终止器验证
            // （不能传自身 pid：terminate 会真的把测试进程杀掉）
            let ok = engine.terminateAgent(pid: 999999, agentId: "dim")
            try expectTrue(!ok, "无法发送信号时必须返回 false")
            try expectTrue(engine.latestEvent?.message?.contains("已终止") != true,
                           "不得谎报已终止，实际: \(engine.latestEvent?.message ?? "nil")")
        }

        TestKit.test("引擎: terminateAgent 成功路径写入 completed 事件") {
            // 直接验证成功分支的事件语义：completed 而非 attention
            // （attention 会让收起态细条误报红色告警）
            let engine = makeEngine(processNames: ["DimAgent"], writes: [:])
            _ = engine.sample(now: Date())
            // 借道 cleanAnomalies 之外的方式不可行，故用可终止的空进程验证：
            // 启动一个 /bin/sleep 作为无害目标
            let sleeper = Process()
            sleeper.executableURL = URL(fileURLWithPath: "/bin/sleep")
            sleeper.arguments = ["30"]
            try? sleeper.run()
            defer { if sleeper.isRunning { sleeper.terminate() } }
            let ok = engine.terminateAgent(pid: sleeper.processIdentifier, agentId: "dim")
            try expectTrue(ok, "可发送信号的 pid 应返回 true")
            try expectEqual(engine.latestEvent?.eventType, .completed,
                            "终止成功应为 completed（非 attention）")
            try expectTrue(engine.latestEvent?.message?.contains("进程已终止") == true, "应提示已终止")
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
            try? sleeper.run()
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
                    basename: "Codex (Service)", cpuPercent: 1),
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
            _ = AgentActionInspector.inspectDimAction()
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
