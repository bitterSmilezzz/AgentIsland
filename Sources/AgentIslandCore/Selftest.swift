import Foundation

// MARK: - 无头自检（--selftest）
// 进程内断言引擎核心逻辑，零 UI。返回 0=全部通过。

public enum Selftest {

    @MainActor
    public static func run() -> Int32 {
        print("AgentIsland selftest — 核心逻辑断言")
        var failures = 0

        // 1. 状态判定：进程不在 → offline
        do {
            let engine = makeEngine(processNames: [], writes: [:])
            let snaps = engine.sample(now: Date())
            let dim = snaps.first { $0.id == "dim" }
            check(dim?.level == .offline, "dim 无进程应 offline", failures: &failures)
        }

        // 2. 进程在 + 窗口内有写入 → working
        do {
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [sessionPath("~/.dimcode/v2/data/sessions"): now.addingTimeInterval(-5)]
            )
            let snaps = engine.sample(now: now)
            let dim = snaps.first { $0.id == "dim" }
            check(dim?.level == .working, "进程在且 5s 内有写入应 working", failures: &failures)
            check(engine.anyWorking, "anyWorking 应为 true", failures: &failures)
        }

        // 3. 进程在 + 超窗口无写入 + CPU=0 → idle（双信号均不命中）
        do {
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [sessionPath("~/.dimcode/v2/data/sessions"): now.addingTimeInterval(-120)]
            )
            let snaps = engine.sample(now: now)
            let dim = snaps.first { $0.id == "dim" }
            check(dim?.level == .idle, "进程在、无写入、CPU=0 应 idle", failures: &failures)
        }

        // 3b. 进程在 + CPU 高但无写入 → working（双信号之二）
        do {
            let now = Date()
            let engine = makeEngine(
                processNames: ["DimAgent"],
                writes: [sessionPath("~/.dimcode/v2/data/sessions"): now.addingTimeInterval(-300)],
                cpu: 25.0
            )
            let snaps = engine.sample(now: now)
            let dim = snaps.first { $0.id == "dim" }
            check(dim?.level == .working, "CPU 高无写入应 working（CPU 信号）", failures: &failures)
        }

        // 4. 目录写入时间推进 → working 升级
        do {
            let now = Date()
            let dir = sessionPath("~/.dimcode/v2/data/sessions")
            let provider = FakeFileActivityProvider(writes: [dir: now.addingTimeInterval(-300)])
            let engine = ActivityEngine(
                profiles: AgentRegistry.builtin,
                config: EngineConfig(workingWindow: 20),
                processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: [], cpu: 0)),
                fileMonitor: provider
            )
            _ = engine.sample(now: now)
            provider.writes = [dir: now.addingTimeInterval(-3)]
            let snaps = engine.sample(now: now.addingTimeInterval(2))
            let dim = snaps.first { $0.id == "dim" }
            check(dim?.level == .working, "新写入出现后应升级为 working", failures: &failures)
        }

        // 5. formatAgo 文案
        do {
            check(ActivityEngine.formatAgo(2) == "刚刚", "formatAgo(2)", failures: &failures)
            check(ActivityEngine.formatAgo(30) == "30s 前", "formatAgo(30)", failures: &failures)
            check(ActivityEngine.formatAgo(120) == "2m 前", "formatAgo(120)", failures: &failures)
            check(ActivityEngine.formatAgo(nil) == "—", "formatAgo(nil)", failures: &failures)
        }

        // 6. 注册表完整性
        do {
            let ids = AgentRegistry.builtin.map(\.id)
            check(Set(ids).count == ids.count, "注册表 id 唯一", failures: &failures)
            check(AgentRegistry.builtin.contains { $0.id == "dim" }, "注册表含 dim", failures: &failures)
        }

        // 7. 进程匹配（GUI bundle + CLI 名，经 matcher）
        do {
            let monitor = ProcessMonitor(provider: FakeProcessProvider(
                processNames: ["dim", "claude"],
                bundleIDs: ["com.dimcode.app"]
            ))
            let matcher = monitor.matcher()
            check(matcher.isRunning(AgentRegistry.builtin.first { $0.id == "dim" }!), "bundle/CLI 匹配 dim", failures: &failures)
            check(matcher.isRunning(AgentRegistry.builtin.first { $0.id == "claude" }!), "CLI 匹配 claude", failures: &failures)
            check(!matcher.isRunning(AgentRegistry.builtin.first { $0.id == "codex" }!), "codex 不应误报", failures: &failures)
        }

        // 7b. 系统路径 + 黑名单排除
        do {
            let snapshot = ProcessSnapshot(entries: [
                ProcessSnapshot.Entry(pid: 1, path: "/System/Library/PrivateFrameworks/TextInputUIMacHelper.framework/Versions/A/XPCServices/CursorUIViewService.xpc/Contents/MacOS/CursorUIViewService", basename: "cursoruiviewservice", cpuPercent: 12),
                ProcessSnapshot.Entry(pid: 2, path: "/usr/sbin/ssh-agent", basename: "ssh-agent", cpuPercent: 0.1),
                ProcessSnapshot.Entry(pid: 3, path: "/Applications/DimAgent.app/Contents/MacOS/DimAgent", basename: "dimagent", cpuPercent: 8),
            ])
            let matcher = ProcessMatcher(snapshot: snapshot, runningBundleIDs: [])
            check(!matcher.isRunning(AgentRegistry.builtin.first { $0.id == "cursor" }!), "CursorUIViewService 不应匹配 Cursor", failures: &failures)
            check(matcher.isRunning(AgentRegistry.builtin.first { $0.id == "dim" }!), "用户路径 DimAgent 应匹配", failures: &failures)
        }

        // 8. 文件活动：目录 mtime（真实临时目录）
        do {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-selftest-\(UUID().uuidString)")
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let before = FileActivityMonitor.newestWrite(in: dir.path)
            check(before != nil, "临时目录应可读 mtime", failures: &failures)
            let file = dir.appendingPathComponent("probe.txt")
            try? Data("x".utf8).write(to: file)
            let after = FileActivityMonitor.newestWrite(in: dir.path)
            check(after != nil && after! >= before!, "写入后 mtime 应更新", failures: &failures)
        }

        print(failures == 0 ? "✅ 全部通过" : "❌ \(failures) 项失败")
        return failures == 0 ? 0 : 1
    }

    // MARK: - 工具

    private static func sessionPath(_ path: String) -> String {
        (path as NSString).expandingTildeInPath
    }

    @MainActor
    private static func makeEngine(processNames: Set<String>, writes: [String: Date], cpu: Double = 0) -> ActivityEngine {
        ActivityEngine(
            profiles: AgentRegistry.builtin,
            config: EngineConfig(workingWindow: 20),
            processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: processNames, bundleIDs: [], cpu: cpu)),
            fileMonitor: FakeFileActivityProvider(writes: writes)
        )
    }

    private static func check(_ condition: Bool, _ name: String, failures: inout Int) {
        if condition {
            print("  ✅ \(name)")
        } else {
            print("  ❌ \(name)")
            failures += 1
        }
    }
}
