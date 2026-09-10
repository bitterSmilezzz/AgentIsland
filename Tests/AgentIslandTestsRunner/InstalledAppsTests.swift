import Foundation
@testable import AgentIslandCore

// MARK: - 安装缓存测试（canned 扫描器注入，零真实文件系统）

@MainActor
enum InstalledAppsTests {

    static func register() {
        TestKit.test("安装缓存: 首扫中保持冷态，加入的调用方均收到完成通知") {
            let entered = DispatchSemaphore(value: 0)
            let release = DispatchSemaphore(value: 0)
            let cache = InstalledAppsCache(scanCLIs: {
                entered.signal()
                _ = release.wait(timeout: .now() + 5)
                return ["dim"]
            }, scanBundles: { [] })
            defer { release.signal() }
            let first = SelfPollExpectation()
            let joined = SelfPollExpectation()
            try expectTrue(cache.warmUp { first.fulfill() })
            try expectTrue(entered.wait(timeout: .now() + 2) == .success)
            let prematurelyWarm = cache.isWarmed
            let duplicate = cache.warmUp { joined.fulfill() }
            release.signal()
            let deadline = Date().addingTimeInterval(3)
            while (!first.isFulfilled || !joined.isFulfilled) && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            }
            try expectFalse(prematurelyWarm, "扫描完成前不能标记已预热")
            try expectFalse(duplicate, "加入在途扫描不重复安排")
            try expectTrue(first.isFulfilled && joined.isFulfilled, "两个等待方都收到完成通知")
            try expectTrue(cache.isWarmed)
            try expectEqual(cache.installedCLIs(), ["dim"])
        }

        TestKit.test("安装缓存: 强制刷新请求合并到在途扫描") {
            let entered = DispatchSemaphore(value: 0)
            let release = DispatchSemaphore(value: 0)
            let cache = InstalledAppsCache(scanCLIs: {
                entered.signal()
                _ = release.wait(timeout: .now() + 5)
                return ["dim"]
            }, scanBundles: { [] })
            defer { release.signal(); release.signal() }
            let done = SelfPollExpectation()
            try expectTrue(cache.refreshIfNeeded(maxAge: 0))
            try expectTrue(entered.wait(timeout: .now() + 2) == .success)
            let duplicate = cache.refreshIfNeeded(maxAge: 0) { done.fulfill() }
            release.signal()
            let deadline = Date().addingTimeInterval(3)
            while !done.isFulfilled && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            }
            try expectFalse(duplicate, "maxAge=0 也不能重复扫描")
            try expectTrue(done.isFulfilled)
            try expectTrue(entered.wait(timeout: .now()) == .timedOut, "扫描器只运行一次")
        }

        TestKit.test("安装缓存: 引擎加入已有首扫后恢复自动发现档案") {
            let entered = DispatchSemaphore(value: 0)
            let release = DispatchSemaphore(value: 0)
            let cache = InstalledAppsCache(scanCLIs: {
                entered.signal()
                _ = release.wait(timeout: .now() + 5)
                return ["fakecli"]
            }, scanBundles: { [] })
            defer { release.signal() }
            cache.warmUp()
            try expectTrue(entered.wait(timeout: .now() + 2) == .success)
            let engine = ActivityEngine(
                profiles: [], processMonitor: FakeProcessProvider(processNames: [], bundleIDs: []),
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                tokenMonitor: FakeTokenUsageMonitor(), installedApps: cache,
                enabledIDs: ["cli-fakecli"])
            release.signal()
            let deadline = Date().addingTimeInterval(3)
            while !engine.allProfiles.contains(where: { $0.id == "cli-fakecli" }) && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            }
            try expectTrue(engine.allProfiles.contains { $0.id == "cli-fakecli" }, "首扫完成必须重放启用集")
        }

        TestKit.test("安装缓存: canned 扫描器 + isInstalled 判定（CLI/bundle/未安装三路）") {
            let cache = InstalledAppsCache(
                scanCLIs: { ["dim"] },
                scanBundles: { ["com.anthropic.claudefordesktop"] })
            cache.refresh()
            let dim = AgentRegistry.builtin.first { $0.id == "dim" }!
            let claude = AgentRegistry.builtin.first { $0.id == "claude" }!
            let codex = AgentRegistry.builtin.first { $0.id == "codex" }!
            try expectTrue(cache.isInstalled(dim), "CLI 名命中")
            try expectTrue(cache.isInstalled(claude), "bundle id 命中")
            try expectTrue(!cache.isInstalled(codex), "未安装不误报")
        }

        TestKit.test("安装缓存: refreshIfNeeded 首次必扫、窗口内跳过、completion 送达") {
            let cache = InstalledAppsCache(
                scanCLIs: { return [] },
                scanBundles: { return [] })
            let scheduled = cache.refreshIfNeeded(maxAge: 0)
            try expectTrue(scheduled, "无时间戳（冷启动）应调度扫描")
            let skipped = cache.refreshIfNeeded(maxAge: 300)
            try expectTrue(!skipped, "窗口内应跳过（调度即标记）")

            let exp = SelfPollExpectation()
            let warm = InstalledAppsCache(scanCLIs: { return [] }, scanBundles: { [] })
            warm.refreshIfNeeded(maxAge: 0) { exp.fulfill() }
            let deadline = Date().addingTimeInterval(5)
            while !exp.isFulfilled && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
            }
            try expectTrue(exp.isFulfilled, "completion 应在刷新完成后送达")
        }
    }
}
