import Foundation
@testable import AgentIslandCore

// MARK: - TokenUsage 单元测试

@MainActor
enum TokenUsageTests {

    static func register() {
        TestKit.test("TokenUsage.compact 紧凑格式") {
            try expectEqual(TokenUsage.compact(890), "890")
            try expectEqual(TokenUsage.compact(45_600), "45.6k")
            try expectEqual(TokenUsage.compact(1_234_567), "1.23M")
        }

        TestKit.test("TokenUsage.cost 格式与零值") {
            try expectEqual(TokenUsage.cost(0), "")
            try expectEqual(TokenUsage.cost(-1), "")
            try expectEqual(TokenUsage.cost(0.004), "<$0.01")
            try expectEqual(TokenUsage.cost(0.42), "$0.42")
            try expectEqual(TokenUsage.cost(14.8275), "$14.83")
        }

        TestKit.test("TokenUsage 相加合并") {
            let a = TokenUsage(tokens24h: 100, tokensTotal: 1000, cost24h: 0.1, costTotal: 1.0)
            let b = TokenUsage(tokens24h: 200, tokensTotal: 2000, cost24h: 0.2, costTotal: 2.0)
            let s = a + b
            try expectEqual(s.tokens24h, 300, "tokens24h")
            try expectEqual(s.tokensTotal, 3000, "tokensTotal")
            try expectTrue(abs(s.costTotal - 3.0) < 0.0001, "costTotal")
        }

        TestKit.test("TokenUsageMonitor 双源汇总（本机真实数据）") {
            let m = TokenUsageMonitor()
            m.refresh()
            let total = m.grandTotal
            // 本机 DimAgent + OpenCode 都有历史数据，累计必为正
            try expectTrue(total.tokensTotal > 0, "累计 token 应 > 0，实际 \(total.tokensTotal)")
            // 24h 口径不应大于累计
            try expectTrue(total.tokens24h <= total.tokensTotal, "24h 不应大于累计")
            // 本机 OpenCode 有 $14+ 花费
            try expectTrue(total.costTotal > 0, "costTotal 应 > 0，实际 \(total.costTotal)")
        }

        TestKit.test("AgentSnapshot tokenUsage 默认 nil 兼容") {
            let profile = AgentRegistry.profile(id: "dim")!
            let snap = AgentSnapshot(profile: profile, level: .idle, processRunning: false,
                                     cpuPercent: 0, installed: true, activeSessions: 0,
                                     lastActivityAgo: nil, lastActivityText: "从未")
            try expectNil(snap.tokenUsage, "未传 tokenUsage 应为 nil")
        }

        TestKit.test("TokenUsage.compact B 级格式") {
            try expectEqual(TokenUsage.compact(1_647_916_622), "1.65B")
        }

        TestKit.test("SQL 转义") {
            try expectEqual("it's".escaped, "it''s")
            try expectEqual("plain".escaped, "plain")
        }

        TestKit.test("模型拆分查询（DimAgent 真实库）") {
            let m = TokenUsageMonitor()
            var rows: [ModelUsage]?
            let exp = Self.makeExpectation()
            Task { @MainActor in
                m.modelBreakdown(agentId: "dim") { r in
                    rows = r
                    exp.fulfill()
                }
            }
            Self.waitMainActor(exp, timeout: 10)
            let r = try XCTUnwrap(rows, "超时无回调")
            try expectTrue(!r.isEmpty, "DimAgent 应有模型记录")
            // 按 token 降序
            for i in 1..<r.count {
                try expectTrue(r[i - 1].tokens >= r[i].tokens, "应按 token 降序")
            }
        }

        TestKit.test("模型拆分查询（OpenCode 真实库）") {
            let m = TokenUsageMonitor()
            var rows: [ModelUsage]?
            let exp = Self.makeExpectation()
            Task { @MainActor in
                m.modelBreakdown(agentId: "opencode") { r in
                    rows = r
                    exp.fulfill()
                }
            }
            Self.waitMainActor(exp, timeout: 10)
            let r = try XCTUnwrap(rows, "超时无回调")
            try expectTrue(!r.isEmpty, "OpenCode 应有模型记录")
            try expectTrue(r.contains { $0.cost > 0 }, "OpenCode 模型应带花费")
        }

        TestKit.test("会话列表查询（DimAgent 首个模型）") {
            let m = TokenUsageMonitor()
            var models: [ModelUsage]?
            var sessions: [SessionUsage]?
            let exp = Self.makeExpectation()
            Task { @MainActor in
                m.modelBreakdown(agentId: "dim") { ms in
                    models = ms
                    guard let first = ms.first?.modelId else { exp.fulfill(); return }
                    m.sessions(agentId: "dim", modelId: first) { ss in
                        sessions = ss
                        exp.fulfill()
                    }
                }
            }
            Self.waitMainActor(exp, timeout: 15)
            let ms = try XCTUnwrap(models, "超时")
            try expectTrue(!ms.isEmpty, "应有模型")
            let ss = try XCTUnwrap(sessions, "会话查询超时")
            try expectTrue(!ss.isEmpty, "首个模型下应有会话")
            // 最后活动时间降序
            let times = ss.compactMap(\.lastTime)
            for i in 1..<times.count {
                try expectTrue(times[i - 1] >= times[i], "应按最后活动降序")
            }
        }

        TestKit.test("未知 agent 返回空列表") {
            let m = TokenUsageMonitor()
            var rows: [ModelUsage]?
            let exp = Self.makeExpectation()
            Task { @MainActor in
                m.modelBreakdown(agentId: "no-such-agent") { r in
                    rows = r
                    exp.fulfill()
                }
            }
            Self.waitMainActor(exp, timeout: 5)
            try expectEqual(try XCTUnwrap(rows).count, 0, "未知 agent 应返回空")
        }

        // MARK: 等待辅助：主线程轮询 RunLoop（避免信号量死锁 MainActor）
    }

    @MainActor
    private static func makeExpectation() -> SelfPollExpectation { SelfPollExpectation() }

    @MainActor
    private static func waitMainActor(_ exp: SelfPollExpectation, timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while !exp.isFulfilled && Date() < deadline {
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
        }
    }
}

/// 极简期望（配合 RunLoop 轮询，避免 DispatchSemaphore 阻塞主线程导致回调饿死）
final class SelfPollExpectation: @unchecked Sendable {
    private let lock = NSLock()
    private var fulfilled = false
    var isFulfilled: Bool { lock.lock(); defer { lock.unlock() }; return fulfilled }
    func fulfill() { lock.lock(); fulfilled = true; lock.unlock() }
}

// 测试辅助：可选值解包（无 XCTest 环境自建）
private func XCTUnwrap<T>(_ value: T?, _ message: String = "值为 nil") throws -> T {
    guard let value else { throw TestError(message: message) }
    return value
}
