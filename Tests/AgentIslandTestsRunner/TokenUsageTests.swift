import Foundation
import SQLite3
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

        TestKit.test("TokenUsageMonitor 双源汇总（fixture 库）") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            let total = m.grandTotal
            // 口径 = 净消耗（input+output+reasoning / prompt+completion），cache.read 不计
            try expectEqual(total.tokens24h, 345, "24h token（dim 300 + opencode 45，cache.read 不计）")
            try expectEqual(total.tokensTotal, 1545, "累计 token")
            try expectTrue(abs(total.cost24h - 0.75) < 0.0001, "24h cost")
            try expectTrue(abs(total.costTotal - 3.75) < 0.0001, "累计 cost")
            try expectTrue(total.tokens24h <= total.tokensTotal, "24h 不应大于累计")

            let dim = try XCTUnwrap(m.usage["dim"], "dim 源缺席")
            try expectEqual(dim.tokens24h, 300, "dim 24h")
            try expectEqual(dim.tokensTotal, 1300, "dim 累计（含 48h 前旧记录）")
            let oc = try XCTUnwrap(m.usage["opencode"], "opencode 源缺席")
            try expectEqual(oc.tokens24h, 45, "opencode 24h：35+10，cache.read 999 必须不计")
            try expectEqual(oc.tokensTotal, 245, "opencode 累计")
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

        TestKit.test("模型拆分查询（fixture 双库）") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)

            var dimRows: [ModelUsage]?
            var ocRows: [ModelUsage]?
            let exp = Self.makeExpectation()
            Task { @MainActor in
                m.modelBreakdown(agentId: "dim") { r in
                    dimRows = r
                    m.modelBreakdown(agentId: "opencode") { r2 in
                        ocRows = r2
                        exp.fulfill()
                    }
                }
            }
            Self.waitMainActor(exp, timeout: 10)
            let dim = try XCTUnwrap(dimRows, "超时无回调")
            try expectEqual(dim.count, 2, "dim 应按模型分组为 2 行")
            try expectEqual(dim[0].modelId, "m2", "m2 累计 token 更高应排前")
            try expectEqual(dim[0].tokens, 1000, "m2 token")
            try expectEqual(dim[1].messages, 2, "m1 两条记录")
            try expectTrue(abs(dim[1].cost - 0.15) < 0.0001, "m1 cost 合计")
            let oc = try XCTUnwrap(ocRows, "超时无回调")
            try expectEqual(oc.count, 2, "user 角色消息不计入 → 仅 oc1/oc2 两组")
            try expectEqual(oc[0].modelId, "oc2", "累计口径下 oc2 token 更高排前")
            try expectTrue(abs(oc[0].cost - 2.0) < 0.0001, "oc2 cost")
            try expectEqual(oc[1].modelId, "oc1", "oc1 次之")
            try expectEqual(oc[1].tokens, 45, "oc1 净消耗 35+10（user 消息 50000 未混入）")
            try expectEqual(oc[1].messages, 2, "oc1 两条 assistant 记录")
        }

        TestKit.test("会话列表查询（fixture 双库）") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)

            var dimSessions: [SessionUsage]?
            var ocSessions: [SessionUsage]?
            let exp = Self.makeExpectation()
            Task { @MainActor in
                m.sessions(agentId: "dim", modelId: "m1") { ss in
                    dimSessions = ss
                    m.sessions(agentId: "opencode", modelId: "oc1") { ss2 in
                        ocSessions = ss2
                        exp.fulfill()
                    }
                }
            }
            Self.waitMainActor(exp, timeout: 10)

            let dim = try XCTUnwrap(dimSessions, "超时")
            try expectEqual(dim.count, 1, "dim m1 只有一个会话")
            try expectEqual(dim[0].sessionId, "sess-a")
            try expectEqual(dim[0].messages, 2, "sess-a 两条记录")
            try expectNil(dim[0].directory, "dim 目录前缀固定指向真实家目录，fixture 会话必不存在")
            let oc = try XCTUnwrap(ocSessions, "超时")
            try expectEqual(oc.count, 2, "opencode 两个会话（LEFT JOIN session 表）")
            try expectTrue(oc.contains { $0.directory != nil }, "session 表有目录且真实存在 → 非 nil")
            try expectTrue(oc.contains { $0.directory == nil }, "session 表缺席/目录已删 → nil")
            // 最后活动时间降序
            let times = oc.compactMap(\.lastTime)
            for i in 1..<times.count {
                try expectTrue(times[i - 1] >= times[i], "应按最后活动降序")
            }
        }

        TestKit.test("未知 agent 返回空列表") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
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

// MARK: - fixture SQLite（临时目录建库插行，环境无关）
// schema 与查询所触达的列一一对应；时间用相对 now 构造覆盖 24h 窗口两侧。

@MainActor
enum TokenFixture {

    struct DBs {
        let dir: URL
        let dimDB: String
        let openCodeDB: String
    }

    /// 数据布局（与断言强耦合，改动需同步用例期望值）：
    /// dim  : m1 两条近期记录（150+150 tok，cost 0.10+0.05，sess-a）+ m2 一条 48h 前记录（1000 tok，cost 1.00）
    /// oc   : s1 oc1 近期 assistant（10+20+5，cache.read 999 不计，cost 0.5）
    ///        s2 oc2 48h 前 assistant（200 tok，cost 2.0）
    ///        s3 oc1 近期 assistant（7+3，cost 0.1，session 表故意缺席 → directory nil）
    ///        s4 oc1 user 角色（50000 tok，cost 9.9，两条查询都不得计入）
    /// session 表: s1 → 真实存在的临时目录；s3/s2 故意缺席（LEFT JOIN 空 → directory nil）
    static func make() throws -> DBs {
        let now = Date()
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("agentisland-token-fixture-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let existingDir = dir.appendingPathComponent("existing-session")
        try FileManager.default.createDirectory(at: existingDir, withIntermediateDirectories: true)

        let dimDB = dir.appendingPathComponent("dimcode.sqlite").path
        let openCodeDB = dir.appendingPathComponent("opencode.db").path
        try exec(dimDB, [
            "CREATE TABLE usage_ledger (createdAt TEXT, modelId TEXT, usage TEXT, cost REAL, sessionId TEXT)",
            "INSERT INTO usage_ledger VALUES ('\(iso(now.addingTimeInterval(-60)))', 'm1', '{\"promptTokens\":100,\"completionTokens\":50}', 0.10, 'sess-a')",
            "INSERT INTO usage_ledger VALUES ('\(iso(now.addingTimeInterval(-30)))', 'm1', '{\"promptTokens\":100,\"completionTokens\":50}', 0.05, 'sess-a')",
            "INSERT INTO usage_ledger VALUES ('\(iso(now.addingTimeInterval(-48 * 3600)))', 'm2', '{\"promptTokens\":1000,\"completionTokens\":0}', 1.00, 'sess-b')",
        ])
        try exec(openCodeDB, [
            "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER)",
            "CREATE TABLE session (id TEXT, directory TEXT)",
            "INSERT INTO message VALUES ('s1', '{\"role\":\"assistant\",\"modelID\":\"oc1\",\"tokens\":{\"input\":10,\"output\":20,\"reasoning\":5,\"cache\":{\"read\":999}},\"cost\":0.5}', \(ms(now.addingTimeInterval(-60))))",
            "INSERT INTO message VALUES ('s2', '{\"role\":\"assistant\",\"modelID\":\"oc2\",\"tokens\":{\"input\":200,\"output\":0,\"reasoning\":0},\"cost\":2.0}', \(ms(now.addingTimeInterval(-48 * 3600))))",
            "INSERT INTO message VALUES ('s3', '{\"role\":\"assistant\",\"modelID\":\"oc1\",\"tokens\":{\"input\":7,\"output\":3,\"reasoning\":0},\"cost\":0.1}', \(ms(now.addingTimeInterval(-30))))",
            "INSERT INTO message VALUES ('s4', '{\"role\":\"user\",\"modelID\":\"oc1\",\"tokens\":{\"input\":50000,\"output\":0,\"reasoning\":0},\"cost\":9.9}', \(ms(now.addingTimeInterval(-60))))",
            "INSERT INTO session VALUES ('s1', '\(existingDir.path)')",
        ])
        return DBs(dir: dir, dimDB: dimDB, openCodeDB: openCodeDB)
    }

    static func cleanup(_ dbs: DBs) {
        try? FileManager.default.removeItem(at: dbs.dir)
    }

    private static func exec(_ path: String, _ statements: [String]) throws {
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE, nil) == SQLITE_OK, let db else {
            throw TestError(message: "fixture 建库失败 \(path)")
        }
        defer { sqlite3_close(db) }
        for sql in statements {
            guard sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK else {
                throw TestError(message: "fixture SQL 失败: \(sql)")
            }
        }
    }

    /// 与 TokenUsageMonitor 内部同格式的 ISO8601（含毫秒，UTC），保证字符串比较口径一致
    static func iso(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.string(from: date)
    }

    static func ms(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }
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
