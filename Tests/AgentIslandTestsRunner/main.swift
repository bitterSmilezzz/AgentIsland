import Foundation
@testable import AgentIslandCore

// MARK: - 测试入口（自建 runner，零框架依赖）

@MainActor
func runAllTests() -> Int32 {
    FileIOTests.register()
    FileIOTests.registerTreeTests()
    EngineTests.register()
    AttentionTests.register()
    DSHTrackingTests.register()
    AntigravityTrackingTests.register()
    MultiAgentAdvancedTests.register()
    TypographyTests.register()
    TokenUsageTests.register()
    RegistryTests.register()
    SettingsTests.register()
    InstalledAppsTests.register()
    IslandMetricsTests.register()
    IslandMetricsTests.registerSignatureSentinel()
    PanelPresentationTests.register()
    DebtRatchetTests.register()
    HardeningTests.register()
    CleanerTests.register()
    VisibilityTests.register()
    ConfigTests.register()
    EventTextTests.register()
    MemoryTextTests.register()
    FormatTests.register()
    PerformanceOptimizationTests.register()
    CLITests.register()
    let result = TestKit.runAll()
    // R16 测试卫生：登记套件统一清理 + 自守护（零 plist 残留）
    TestDefaults.cleanupAll()
    let leaked = TestDefaults.leakedFiles
    if leaked > 0 {
        print("⚠️ 测试卫生: \(leaked) 个登记套件的 plist 仍未清理（cfprefs flush 竞态或删除失败）")
        return 1
    }
    // cfprefsd 会在本进程退出 flush 时把已删除的套件域重建为 plist（活体复现：
    // 进程内断言通过、退出后仍冒出 N 个文件）。派生独立进程在退出后兜底清扫，
    // 只匹配 agentisland-test- 前缀（全部为登记套件，不触碰其他域）
    let sweep = Process()
    sweep.executableURL = URL(fileURLWithPath: "/bin/zsh")
    // 3 秒内多次重试：cfprefsd 的退出 flush 与本清扫存在竞态（连跑两轮实测
    // 一次 rm 早于 flush），循环删除直至窗口期完全覆盖
    sweep.arguments = ["-c",
                       "for i in 1 2 3; do sleep 1; rm -f \"$HOME/Library/Preferences/\"agentisland-test-*.plist 2>/dev/null; done"]
    try? sweep.run()
    return result
}

// 顶层代码运行在主线程，用 assumeIsolated 满足 MainActor 隔离检查
exit(MainActor.assumeIsolated { runAllTests() })
