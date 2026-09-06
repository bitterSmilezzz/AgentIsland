import Foundation
@testable import AgentIslandCore

// MARK: - 安装缓存测试（canned 扫描器注入，零真实文件系统）

@MainActor
enum InstalledAppsTests {

    static func register() {
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
            var scans = 0
            let cache = InstalledAppsCache(
                scanCLIs: { scans += 1; return [] },
                scanBundles: { scans += 1; return [] })
            let scheduled = cache.refreshIfNeeded(maxAge: 0)
            try expectTrue(scheduled, "无时间戳（冷启动）应调度扫描")
            let skipped = cache.refreshIfNeeded(maxAge: 300)
            try expectTrue(!skipped, "窗口内应跳过（调度即标记）")

            let exp = SelfPollExpectation()
            let warm = InstalledAppsCache(scanCLIs: { scans += 1; return [] }, scanBundles: { [] })
            warm.refreshIfNeeded(maxAge: 0) { exp.fulfill() }
            let deadline = Date().addingTimeInterval(5)
            while !exp.isFulfilled && Date() < deadline {
                RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
            }
            try expectTrue(exp.isFulfilled, "completion 应在刷新完成后送达")
        }
    }
}
