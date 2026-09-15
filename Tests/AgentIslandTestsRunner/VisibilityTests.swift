import Foundation
@testable import AgentIslandCore

// MARK: - 可见口径（visibleSnapshots / ringShelfSnapshots）
//
// 列表渲染与窗口高度共用这两个口径：口径放宽会出现「幽灵条目」（视图有、高度没算），
// 收紧会「内容被裁」。全部用注入 fake 驱动引擎采样，不依赖真实进程表。
// 时间基准显式传入（writes 与 sample 共用同一 now），避免边界用例受调用耗时抖动影响。

@MainActor
enum VisibilityTests {

    private static let agentId = "vis-agent"
    private static let sessionDir = "/tmp/agentisland-vis-sessions"

    private static var profile: AgentProfile {
        AgentProfile(id: agentId, name: "Vis Agent", icon: "terminal",
                     bundleIDs: [], processNames: ["viscli"], sessionDirs: [sessionDir])
    }

    /// 单 profile 引擎 + 采样时间基准（两者同源，边界用例才确定）
    private static func makeEngine(processRunning: Bool, writeAgo: TimeInterval?,
                                   usage: TokenUsage? = nil) -> (engine: ActivityEngine, now: Date) {
        let now = Date()
        let token = FakeTokenUsageMonitor()
        if let usage { token.usage[agentId] = usage }
        let writes: [String: Date] = writeAgo.map { [sessionDir: now.addingTimeInterval(-$0)] } ?? [:]
        let engine = ActivityEngine(
            profiles: [profile],
            processMonitor: FakeProcessProvider(processNames: processRunning ? ["viscli"] : [], bundleIDs: []),
            fileMonitor: FakeFileActivityProvider(writes: writes),
            tokenMonitor: token,
            installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
        )
        return (engine, now)
    }

    private static func sample(_ made: (engine: ActivityEngine, now: Date)) {
        _ = made.engine.sample(now: made.now)
    }

    static func register() {

        TestKit.test("可见口径: 离线 Agent 无论近期活动或 Token 均不显示") {
            let inside = makeEngine(processRunning: false, writeAgo: 86_399)
            sample(inside)
            try expectTrue(inside.engine.visibleSnapshots.isEmpty,
                           "离线但 24h 内有活动也必须隐藏")

            let withTokens = makeEngine(
                processRunning: false,
                writeAgo: 5,
                usage: TokenUsage(tokens24h: 9_999, tokensTotal: 9_999, cost24h: 1, costTotal: 1)
            )
            sample(withTokens)
            try expectTrue(withTokens.engine.visibleSnapshots.isEmpty,
                           "近期活动与 Token 不能让离线 Agent 重新进入主列表")
        }

        TestKit.test("可见口径: 待机与运行中的在线 Agent 均显示") {
            let idle = makeEngine(processRunning: true, writeAgo: nil)
            sample(idle)
            try expectEqual(idle.engine.snapshots.first?.level, .idle)
            try expectEqual(idle.engine.visibleSnapshots.map(\.id), [agentId], "在线待机应显示")

            let working = makeEngine(processRunning: true, writeAgo: 1)
            sample(working)
            try expectEqual(working.engine.snapshots.first?.level, .working)
            try expectEqual(working.engine.visibleSnapshots.map(\.id), [agentId], "在线运行中应显示")
        }

        TestKit.test("可见口径: 离线且从未有活动记录（lastActivityAgo=nil）不可见") {
            let engine = makeEngine(processRunning: false, writeAgo: nil)
            sample(engine)
            try expectTrue(engine.engine.visibleSnapshots.isEmpty, "nil 视为无穷远，不可见")
            let snap = engine.engine.snapshots.first
            try expectNil(snap?.lastActivityAgo, "离线未活跃的 lastActivityAgo 应为 nil")
            try expectEqual(snap?.level, .offline, "进程不在应为 offline")
        }

        TestKit.test("活动环看板口径: 工作态 / 24h 有用量 才进看板") {
            // 工作态：进程在 + 近期写入
            let working = makeEngine(processRunning: true, writeAgo: 5)
            sample(working)
            try expectEqual(working.engine.ringShelfSnapshots.map(\.id), [agentId], "工作态进看板")

            // 空闲但有 24h 用量
            let idleWithTokens = makeEngine(processRunning: true, writeAgo: 3_600,
                                            usage: TokenUsage(tokens24h: 1, tokensTotal: 1, cost24h: 0, costTotal: 0))
            sample(idleWithTokens)
            try expectEqual(idleWithTokens.engine.snapshots.first?.level, .idle, "前置条件：应为空闲态")
            try expectEqual(idleWithTokens.engine.ringShelfSnapshots.map(\.id), [agentId], "空闲但 24h 有用量进看板")

            // 空闲且无 24h 用量 → 不进看板（累计有量也不算）
            let idleNoTokens = makeEngine(processRunning: true, writeAgo: 3_600,
                                          usage: TokenUsage(tokens24h: 0, tokensTotal: 12_345, cost24h: 0, costTotal: 9))
            sample(idleNoTokens)
            try expectEqual(idleNoTokens.engine.snapshots.first?.level, .idle, "前置条件：应为空闲态")
            try expectTrue(idleNoTokens.engine.ringShelfSnapshots.isEmpty,
                           "看板口径是 24h 用量而非累计用量：tokens24h=0 即便累计 12345 也不进看板")
        }

        TestKit.test("活动环看板口径: 离线 Agent 即使有近期活动与用量也不进看板") {
            let offlineStale = makeEngine(processRunning: false, writeAgo: 86_401,
                                          usage: TokenUsage(tokens24h: 5_000, tokensTotal: 5_000, cost24h: 0, costTotal: 0))
            sample(offlineStale)
            try expectTrue(offlineStale.engine.visibleSnapshots.isEmpty, "离线不可见")
            try expectTrue(offlineStale.engine.ringShelfSnapshots.isEmpty, "不可见者不进看板（看板是可见集子集）")

            let offlineFresh = makeEngine(processRunning: false, writeAgo: 86_399,
                                          usage: TokenUsage(tokens24h: 5_000, tokensTotal: 5_000, cost24h: 0, costTotal: 0))
            sample(offlineFresh)
            try expectTrue(offlineFresh.engine.ringShelfSnapshots.isEmpty,
                           "离线但 24h 内有量也不得进看板")
        }

        TestKit.test("可见口径: 始终是快照总集的子集（口径不外溢）") {
            let made = makeEngine(processRunning: true, writeAgo: 5,
                                  usage: TokenUsage(tokens24h: 10, tokensTotal: 10, cost24h: 0, costTotal: 0))
            sample(made)
            let visible = Set(made.engine.visibleSnapshots.map(\.id))
            let shelf = Set(made.engine.ringShelfSnapshots.map(\.id))
            let all = Set(made.engine.snapshots.map(\.id))
            try expectTrue(visible.isSubset(of: all), "可见集必须是总集子集")
            try expectTrue(shelf.isSubset(of: visible), "看板集必须是可见集子集")
            try expectEqual(all.count, 1, "注入单 profile 时总集只有一条（实际 \(all.count)）")
        }
    }
}
