import Foundation
@testable import AgentIslandCore

// MARK: - 测试入口（自建 runner，零框架依赖）

@MainActor
func runAllTests() -> Int32 {
    EngineTests.register()
    TokenUsageTests.register()
    RegistryTests.register()
    SettingsTests.register()
    InstalledAppsTests.register()
    return TestKit.runAll()
}

// 顶层代码运行在主线程，用 assumeIsolated 满足 MainActor 隔离检查
exit(MainActor.assumeIsolated { runAllTests() })
