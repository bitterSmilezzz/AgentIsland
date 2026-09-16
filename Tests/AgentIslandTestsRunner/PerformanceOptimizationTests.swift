import Foundation
import Combine
@testable import AgentIslandCore

// MARK: - 性能优化与事件横幅自动清理回归测试

@MainActor
enum PerformanceOptimizationTests {
    static let dim = AgentRegistry.builtin.first { $0.id == "dim" }!

    static func register() {
        TestKit.test("性能优化: 心跳与噪声文件过滤零开销匹配") {
            let pidHeartbeat = URL(fileURLWithPath: "/tmp/sessions/12345.json")
            try expectTrue(FileActivityMonitor.isHeartbeatOrNoiseFile(pidHeartbeat), "纯 PID json 必须判定为心跳文件")

            let normalJson = URL(fileURLWithPath: "/tmp/sessions/history.json")
            try expectFalse(FileActivityMonitor.isHeartbeatOrNoiseFile(normalJson), "普通会话 json 不能被过滤")

            let shmDb = URL(fileURLWithPath: "/tmp/sessions/state.db-shm")
            try expectTrue(FileActivityMonitor.isHeartbeatOrNoiseFile(shmDb), "SQLite -shm 共享内存索引必须过滤")

            let lockFile = URL(fileURLWithPath: "/tmp/sessions/app.lock")
            try expectTrue(FileActivityMonitor.isHeartbeatOrNoiseFile(lockFile), ".lock 文件必须过滤")
        }

        TestKit.test("性能优化: FileMonitor 离线目录若根 mtime 未变跳过深层递归") {
            let tempDir = NSTemporaryDirectory() + "agentisland_perf_test_\(UUID().uuidString)"
            let subDir = tempDir + "/sub"
            try? FileManager.default.createDirectory(atPath: subDir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(atPath: tempDir) }

            let monitor = FileActivityMonitor(maxDepth: 3, scanMinInterval: 0)
            monitor.setWorkingWindow(1.0)
            monitor.watch(dirs: [tempDir])

            // 1. 首扫建立缓存
            let testFile = subDir + "/file1.txt"
            let date1 = Date().addingTimeInterval(-100)
            try? "hello".write(toFile: testFile, atomically: true, encoding: .utf8)
            try? FileManager.default.setAttributes([.modificationDate: date1], ofItemAtPath: testFile)

            monitor.scanSync()
            let dates1 = monitor.lastWriteDates(for: [tempDir])
            try expectTrue(dates1[tempDir] != nil, "首扫应建立缓存")

            // 2. 将目录标记为离线（setRunningDirs 为空），并在深层写入新文件
            monitor.setRunningDirs([])
            let file2 = subDir + "/file2.txt"
            let date2 = Date()
            try? "world".write(toFile: file2, atomically: true, encoding: .utf8)
            try? FileManager.default.setAttributes([.modificationDate: date2], ofItemAtPath: file2)

            // 离线态且根目录 mtime 未变 -> 即使深层有写入也复用缓存，不枚举目录树
            monitor.scanSync()
            let dates2 = monitor.lastWriteDates(for: [tempDir])
            try expectEqual(dates2[tempDir], dates1[tempDir], "离线目录在根 mtime 未变时复用缓存，跳过深层递归")

            // 3. 将目录标记为上线（setRunningDirs 包含 tempDir）-> 新上线目录立即触发全量深搜，发现新写入
            monitor.setRunningDirs([tempDir])
            monitor.scanSync()
            let dates3 = monitor.lastWriteDates(for: [tempDir])
            try expectTrue(abs((dates3[tempDir]?.timeIntervalSince1970 ?? 0) - date2.timeIntervalSince1970) < 2.0, "新上线目录必须执行深搜更新最近活动时间")
        }

        TestKit.test("横幅自动消除: Agent 退出 attention 态自动清除待确认横幅") {
            let start = Date()
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.dimcode/v2/data/sessions"
            let files = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: [dim],
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: files,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )

            // 1. 进入 attention 态
            var signal: AgentSessionSignal? = .attention(AgentAttentionRequest(
                fingerprint: "req-1", message: "等待你批准操作"))
            engine.inspectSessionHook = { _, _, _ in signal }
            _ = engine.sample(now: start)
            try expectEqual(engine.latestEvent?.eventType, .attention, "首次进入待确认应有横幅")

            // 2. 用户在客户端确认，Agent 恢复 working 态
            signal = .active(fingerprint: "req-1", action: "正在执行批准的操作")
            _ = engine.sample(now: start.addingTimeInterval(1))
            try expectNil(engine.latestEvent, "Agent 恢复 working 态时，等待确认横幅必须自动消除")

            // 3. 再次进入 attention 态
            signal = .attention(AgentAttentionRequest(
                fingerprint: "req-2", message: "第二次等待批准"))
            _ = engine.sample(now: start.addingTimeInterval(3))
            try expectEqual(engine.latestEvent?.eventType, .attention, "新等待确认应重新弹出横幅")

            // 4. 用户取消操作，Agent 恢复 idle 态
            signal = nil
            files.writes = [dir: start.addingTimeInterval(-600)]
            _ = engine.sample(now: start.addingTimeInterval(10))
            try expectNil(engine.latestEvent, "Agent 恢复 idle 态时，等待确认横幅必须自动消除")
        }

        TestKit.test("横幅自动消除: Agent 重新 working 自动清除上一次已完成横幅") {
            let start = Date()
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.dimcode/v2/data/sessions"
            let files = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: [dim],
                config: EngineConfig(workingWindow: 60, minWorkingHold: 10),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: files,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )

            // 1. Agent 处于 working 态
            var signal: AgentSessionSignal?
            engine.inspectSessionHook = { _, _, _ in signal }
            _ = engine.sample(now: start)

            // 2. 任务完成，生成完成横幅
            signal = .completed(fingerprint: "turn-1")
            _ = engine.sample(now: start.addingTimeInterval(5))
            try expectEqual(engine.latestEvent?.eventType, .completed, "任务完成生成 completed 横幅")

            // 3. 用户提交新指令，Agent 重新 working
            signal = .active(fingerprint: "turn-2", action: "正在执行新任务")
            _ = engine.sample(now: start.addingTimeInterval(7))
            try expectNil(engine.latestEvent, "重新开始工作时，上一次的完成横幅必须自动清除")
        }
    }
}
