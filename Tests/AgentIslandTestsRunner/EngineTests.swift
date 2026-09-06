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
            let matcher = ProcessMonitor(provider: provider).matcher()
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
                processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: [])),
                fileMonitor: provider
            )
            _ = engine.sample(now: now)
            provider.writes = [dir: now.addingTimeInterval(-3)]
            let snaps = engine.sample(now: now.addingTimeInterval(2))
            try expectEqual(snaps.first { $0.id == "dim" }?.level, .working, "升级后应 working")
        }

        TestKit.test("引擎: 目录缺失时保持离线且不崩溃") {
            let engine = makeEngine(processNames: [], writes: [:])
            let snaps = engine.sample(now: Date())
            try expectEqual(snaps.count, AgentRegistry.builtin.count, "快照应覆盖全部内置 Agent")
        }

        TestKit.test("工具: formatAgo 文案") {
            try expectEqual(ActivityEngine.formatAgo(2), "刚刚")
            try expectEqual(ActivityEngine.formatAgo(30), "30s 前")
            try expectEqual(ActivityEngine.formatAgo(120), "2m 前")
            try expectEqual(ActivityEngine.formatAgo(nil), "—")
        }

        TestKit.test("注册表: id 唯一、含 dim") {
            let ids = AgentRegistry.builtin.map(\.id)
            try expectEqual(Set(ids).count, ids.count, "id 唯一")
            try expectTrue(AgentRegistry.builtin.contains { $0.id == "dim" }, "含 dim")
        }

        TestKit.test("进程: GUI bundle + CLI 进程名匹配") {
            let monitor = ProcessMonitor(provider: FakeProcessProvider(
                processNames: ["dim", "claude"],
                bundleIDs: ["com.dimcode.app"]
            ))
            let matcher = monitor.matcher()
            let claude = AgentRegistry.builtin.first { $0.id == "claude" }!
            let codex = AgentRegistry.builtin.first { $0.id == "codex" }!
            try expectTrue(matcher.isRunning(dim), "bundle/CLI 匹配 dim")
            try expectTrue(matcher.isRunning(claude), "CLI 匹配 claude")
            try expectTrue(!matcher.isRunning(codex), "codex 不应误报")
        }

        TestKit.test("进程: 大小写不敏感匹配") {
            let monitor = ProcessMonitor(provider: FakeProcessProvider(
                processNames: ["DIMAGENT"],
                bundleIDs: []
            ))
            try expectTrue(monitor.matcher().isRunning(dim), "大小写不敏感")
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
                processMonitor: ProcessMonitor(),
                fileMonitor: monitor
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
    }

    // MARK: - 工具

    static func makeEngine(processNames: Set<String>, writes: [String: Date], cpu: Double = 0,
                           tokenMonitor: any TokenUsagePolling & TokenUsageQuerying = TokenUsageMonitor()) -> ActivityEngine {
        ActivityEngine(
            profiles: AgentRegistry.builtin,
            config: EngineConfig(workingWindow: 20),
            processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: processNames, bundleIDs: [], cpu: cpu)),
            fileMonitor: FakeFileActivityProvider(writes: writes),
            tokenMonitor: tokenMonitor
        )
    }
}
