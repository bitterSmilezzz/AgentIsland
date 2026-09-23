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

        TestKit.test("TokenCostEstimator 模型费率匹配与智能估算") {
            // 模糊与精确匹配测试
            try expectTrue(TokenCostEstimator.rate(for: "claude-3-7-sonnet-20250219") != nil, "Claude 3.7 版本后缀")
            try expectTrue(TokenCostEstimator.rate(for: "gpt-4o-2024-08-06") != nil, "GPT-4o 日期后缀")
            try expectTrue(TokenCostEstimator.rate(for: "deepseek-reasoner") != nil, "DeepSeek R1")
            try expectTrue(TokenCostEstimator.rate(for: "gemini-2.5-flash") != nil, "Gemini 2.5 Flash")

            // 混合估算公式：Claude 3.5 Sonnet: (3*3 + 15*1)/4 = 6.0 USD / 1M tokens
            let estSonnet = try XCTUnwrap(TokenCostEstimator.estimateCost(modelId: "claude-3-5-sonnet", tokens: 1_000_000))
            try expectTrue(abs(estSonnet - 6.0) < 0.001, "Sonnet 混合单价应为 $6.00 / 1M")

            // 格式化输出
            try expectEqual(TokenCostEstimator.formatEstimate(0.42), "~$0.42")
            try expectEqual(TokenCostEstimator.formatEstimate(0.004), "~<$0.01")
            try expectEqual(TokenCostEstimator.formatEstimate(0), "")

            // resolveCost 优先级保护：真实值优先，未提供时估算
            let actualOnly = TokenCostEstimator.resolveCost(actual: 1.25, modelId: "gpt-4o", tokens: 10_000)
            try expectEqual(actualOnly.cost, 1.25)
            try expectEqual(actualOnly.text, "$1.25")
            try expectFalse(actualOnly.isEstimated)

            let estimated = TokenCostEstimator.resolveCost(actual: 0, modelId: "claude-3-7-sonnet", tokens: 100_000)
            try expectTrue(estimated.isEstimated)
            try expectEqual(estimated.text, "~$0.60")

            let unknown = TokenCostEstimator.resolveCost(actual: 0, modelId: "my-custom-local-model", tokens: 50_000)
            try expectFalse(unknown.isEstimated)
            try expectEqual(unknown.text, "")
        }

        TestKit.test("TokenReportExporter 导出 Markdown 与 CSV 报表") {
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let sources = [
                TokenSourceUsage(agentId: "claude-3-7-sonnet", tokens: 100_000, cost: 0.60, isAvailable: true),
                TokenSourceUsage(agentId: "dim", tokens: 50_000, cost: 0, isAvailable: true)
            ]
            let timeline = TokenUsageTimeline(
                range: .day,
                points: [],
                sources: sources,
                tokens: 150_000,
                cost: 0.60,
                previousTokens: 80_000,
                previousCost: 0.30
            )
            let grand = TokenUsage(tokens24h: 150_000, tokensTotal: 500_000, cost24h: 0.60, costTotal: 3.50)

            let md = TokenReportExporter.generateMarkdown(timeline: timeline, range: .day, grandTotal: grand, now: now)
            try expectTrue(md.contains("# AgentIsland Token 消费与用量分析报表"))
            try expectTrue(md.contains("150.0k"))
            try expectTrue(md.contains("claude-3-7-sonnet"))

            let csv = TokenReportExporter.generateCSV(timeline: timeline, range: .day, now: now)
            try expectTrue(csv.hasPrefix("\u{FEFF}")) // UTF-8 BOM
            try expectTrue(csv.contains("报表时间,周期,Agent,Tokens,费用,状态"))
            try expectTrue(csv.contains("claude-3-7-sonnet"))

            // 零成本在两份导出里必须同形。此前 Markdown 写 `—`、CSV 写 `$0.00`，
            // 而「这个源查没查到」本来就由「已连接/未发现」那一列单独负责
            let dimRow = md.split(separator: "\n").first { $0.contains("`dim`") } ?? ""
            try expectEqual(String(dimRow), "| `dim` | 50.0k | $0.00 | 已连接 |",
                            "Markdown 里的零成本该是 $0.00：\(dimRow)")
            try expectTrue(csv.contains("dim\",50000,\"$0.00\",\"已连接\""),
                           "CSV 与 Markdown 对同一个零说法一致：\(csv.split(separator: "\n").first { $0.contains("dim") } ?? "")")
        }

        TestKit.test("TokenBudgetTracker 预算阈值与预警状态机") {
            let tracker = TokenBudgetTracker()
            let now = Date(timeIntervalSince1970: 1_700_000_000)

            // 1. 未设预算 (budget <= 0)
            let res0 = tracker.evaluate(used24h: 50_000, budget: 0, now: now)
            try expectEqual(res0.status, TokenBudgetStatus.disabled, "disabled")
            try expectNil(res0.alertMessage, "no alert when disabled")

            // 2. 正常区间 (used < 80%)
            let res1 = tracker.evaluate(used24h: 50_000, budget: 100_000, now: now)
            try expectTrue(res1.status == TokenBudgetStatus.normal(used: 50_000, budget: 100_000, ratio: 0.5), "normal")
            try expectNil(res1.alertMessage, "no alert below 80%")

            // 3. 达到 80% 预警线
            let res2 = tracker.evaluate(used24h: 85_000, budget: 100_000, now: now)
            try expectTrue(res2.status.isWarning, "warning flag")
            try expectTrue(res2.alertMessage != nil, "warning alert triggered")
            try expectTrue(res2.alertMessage?.contains("85%") ?? false, "pct in warning")

            // 4. 重复处于预警区间（去重不重复发 alertMessage）
            let res2Repeat = tracker.evaluate(used24h: 88_000, budget: 100_000, now: now)
            try expectTrue(res2Repeat.status.isWarning, "still warning")
            try expectNil(res2Repeat.alertMessage, "dedup alertMessage")

            // 5. 跨越到 100% 预算超额
            let res3 = tracker.evaluate(used24h: 110_000, budget: 100_000, now: now)
            try expectTrue(res3.status.isExceeded, "exceeded flag")
            try expectTrue(res3.alertMessage != nil, "exceeded alert triggered")
            try expectTrue(res3.alertMessage?.contains("110%") ?? false, "pct in exceeded")

            // 6. 再次采样超额去重
            let res3Repeat = tracker.evaluate(used24h: 120_000, budget: 100_000, now: now)
            try expectNil(res3Repeat.alertMessage, "dedup exceeded alert")
        }

        TestKit.test("ProcessInspector 读取当前进程工作目录") {
            let pid = ProcessInfo.processInfo.processIdentifier
            let cwd = ProcessInspector.currentWorkingDirectory(of: pid)
            try expectTrue(cwd != nil, "CWD must not be nil for current process")
            try expectTrue(cwd?.contains("AgentIsland") ?? false, "CWD contains project name")
            // 非法 PID 安全返回 nil
            try expectNil(ProcessInspector.currentWorkingDirectory(of: -1), "invalid pid returns nil")
            try expectNil(ProcessInspector.currentWorkingDirectory(of: 0), "pid 0 returns nil")
        }

        TestKit.test("TokenUsage 相加合并") {
            let a = TokenUsage(tokens24h: 100, tokensTotal: 1000, cost24h: 0.1, costTotal: 1.0)
            let b = TokenUsage(tokens24h: 200, tokensTotal: 2000, cost24h: 0.2, costTotal: 2.0)
            let s = a + b
            try expectEqual(s.tokens24h, 300, "tokens24h")
            try expectEqual(s.tokensTotal, 3000, "tokensTotal")
            try expectTrue(abs(s.costTotal - 3.0) < 0.0001, "costTotal")
        }

        TestKit.test("Token 时间线按范围分桶并计算上一周期") {
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let records = [
                TokenUsageRecord(agentId: "dim", time: now.addingTimeInterval(-24 * 3_600),
                                 tokens: 10, cost: 0.10),
                TokenUsageRecord(agentId: "opencode", time: now.addingTimeInterval(-30 * 60),
                                 tokens: 90, cost: 0.90),
                TokenUsageRecord(agentId: "dim", time: now.addingTimeInterval(-25 * 3_600),
                                 tokens: 40, cost: 0.40),
                TokenUsageRecord(agentId: "dim", time: now.addingTimeInterval(-49 * 3_600),
                                 tokens: 999, cost: 9.99),
                TokenUsageRecord(agentId: "dim", time: now.addingTimeInterval(60),
                                 tokens: 999, cost: 9.99),
            ]
            let result = TokenTimelineBuilder.build(records: records, range: .day, now: now)
            try expectEqual(result.points.count, 24, "24h 应固定 24 个小时桶")
            try expectEqual(result.tokens, 100, "当前周期只含边界及范围内记录")
            try expectEqual(result.previousTokens, 40, "上一等长周期独立统计")
            try expectTrue(abs(result.cost - 1.0) < 0.0001, "当前成本")
            try expectEqual(result.points.first?.tokens, 10, "当前周期左边界进入首桶")
            try expectEqual(result.points.last?.tokens, 90, "最近记录进入末桶")
            try expectEqual(result.sources.map(\.agentId), ["opencode", "dim"], "来源按用量降序")
        }

        TestKit.test("Token 时间线完整列出可统计工具与缺失状态") {
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let result = TokenTimelineBuilder.build(
                records: [TokenUsageRecord(agentId: "dim", time: now.addingTimeInterval(-60), tokens: 42, cost: 0)],
                range: .day,
                now: now,
                supportedSourceIds: ["dim", "codex", "claude"],
                availableSourceIds: ["dim", "codex"]
            )
            try expectEqual(result.sources.map(\.agentId), ["dim", "codex", "claude"])
            try expectEqual(result.sources.first { $0.agentId == "codex" }?.tokens, 0,
                            "已发现但当前范围无记录应明确显示 0")
            try expectTrue(result.sources.first { $0.agentId == "codex" }?.isAvailable == true)
            try expectTrue(result.sources.first { $0.agentId == "claude" }?.isAvailable == false,
                          "未发现数据源不能伪装成零用量")
        }

        TestKit.test("可读 Codex 用量在未独立监控时仍可进入详情") {
            let codex = TokenSourceUsage(agentId: "codex", tokens: 120, cost: 0, isAvailable: true)
            let target = TokenSourceDetailRoute.target(
                for: codex,
                detailCapableAgentIDs: ["chatgpt", "codex"]
            )
            try expectEqual(target, "codex",
                            "内嵌 Codex 不在实时快照时，历史用量行仍必须可下钻")
        }

        TestKit.test("Token 时间范围保持适合窄卡片的固定密度") {
            try expectEqual(TokenTimeRange.day.bucketCount, 24)
            try expectEqual(TokenTimeRange.week.bucketCount, 28)
            try expectEqual(TokenTimeRange.month.bucketCount, 30)
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let calendar = Calendar.current
            for range in TokenTimeRange.allCases {
                let empty = TokenUsageTimeline.empty(for: range, now: now)
                try expectEqual(empty.points.count, range.bucketCount)
                try expectTrue(empty.sources.isEmpty)
                // 空时间线也要铺满整个窗口：横轴是按 points 画的，起点错了就是「报表少画半天」
                try expectEqual(empty.points[0].start, range.windowStart(now: now),
                                "\(range) 首桶应从窗口起点开始")
                for index in 1..<empty.points.count {
                    try expectTrue(empty.points[index].start > empty.points[index - 1].start,
                                   "\(range) 第 \(index) 个桶的起点没有更晚")
                }
            }
            // 24h 档保持滚动（它的刻度带时分，滚动不说谎）；周/月按日历日切
            try expectEqual(TokenTimeRange.day.windowStart(now: now),
                            now.addingTimeInterval(-TokenTimeRange.day.duration),
                            "24h 档必须仍是滚动窗口，与卡片上的 24H 同一个口径")
            for range in [TokenTimeRange.week, TokenTimeRange.month] {
                let starts = TokenUsageTimeline.empty(for: range, now: now).points.map(\.start)
                let byDay = Dictionary(grouping: starts) { calendar.startOfDay(for: $0) }
                try expectEqual(byDay.count, range.alignedDays,
                                "\(range) 应覆盖 \(range.alignedDays) 个日历天（实得 \(byDay.count)）")
                for (day, items) in byDay {
                    try expectEqual(items.count, range.bucketsPerDay,
                                    "\(range) 在 \(day) 上切了 \(items.count) 格，每格不得跨日历日")
                }
            }
            // 月档一格就是一天，刻度写 M/d，所以起点必须是当地零点
            let monthStarts = TokenUsageTimeline.empty(for: .month, now: now).points.map(\.start)
            for start in monthStarts {
                try expectEqual(start, calendar.startOfDay(for: start),
                                "月档桶起点不在当地 00:00：标签 M/d 会说谎")
            }
        }

        TestKit.test("分析页分桶: 跨午夜的相邻两笔不得挤进同一根柱子") {
            // 旧口径从「此刻」往回推 duration，桶边界落在钟点上：一根标着 9/18 的柱子实际
            // 覆盖 9/17 21:37 → 9/18 03:37，昨天深夜的用量被算进今天那一格
            let calendar = Calendar.current
            let today = calendar.startOfDay(for: Date())
            let now = today.addingTimeInterval(12 * 3_600)          // 今天正午
            func at(_ dayOffset: Int, _ seconds: TimeInterval) -> Date {
                calendar.date(byAdding: .day, value: dayOffset, to: today)!.addingTimeInterval(seconds)
            }
            let records = [
                TokenUsageRecord(agentId: "dim", time: at(-1, 23 * 3_600 + 50 * 60), tokens: 100, cost: 1),
                TokenUsageRecord(agentId: "dim", time: at(0, 10 * 60), tokens: 250, cost: 2),
            ]
            let month = TokenTimelineBuilder.build(records: records, range: .month, now: now)
            let filled = month.points.filter { $0.tokens > 0 }
            try expectEqual(filled.count, 2, "昨天 23:50 与今天 00:10 必须分属两根柱子")
            try expectEqual(filled.map(\.tokens), [100, 250], "按时间先后对应各自的量")
            try expectEqual(filled.map { calendar.startOfDay(for: $0.start) }, [at(-1, 0), at(0, 0)],
                            "每根柱子的起点就是它标签上的那一天")
            try expectEqual(month.tokens, 350, "本期合计")
            try expectEqual(month.previousTokens, 0, "两笔都在本期，环比分母不得把它们算进去")

            // 周档同理，只是每天切 4 格：昨天深夜那一格不能与今天凌晨共用
            let week = TokenTimelineBuilder.build(records: records, range: .week, now: now)
            let weekFilled = week.points.filter { $0.tokens > 0 }
            try expectEqual(weekFilled.count, 2)
            try expectEqual(weekFilled.map(\.tokens), [100, 250])
        }

        TestKit.test("预算告警: 跨午夜不得对同一段滚动窗口重复告警") {
            // 预算度量的是 tokens24h（滚动 24 小时）。旧实现额外按自然日把告警级别归零，
            // 于是 23:50 报过的越线，00:10 同一段用量会再报一次，天天午夜响一遍
            let calendar = Calendar.current
            let midnight = calendar.startOfDay(for: Date())
            let beforeMidnight = midnight.addingTimeInterval(23 * 3_600 + 50 * 60)
            let afterMidnight = beforeMidnight.addingTimeInterval(20 * 60)   // 次日 00:10
            try expectTrue(calendar.component(.day, from: beforeMidnight)
                            != calendar.component(.day, from: afterMidnight),
                           "夹具前提：两个时刻必须跨日历日")

            let tracker = TokenBudgetTracker()
            try expectNotNil(tracker.evaluate(used24h: 110_000, budget: 100_000,
                                              now: beforeMidnight).alertMessage,
                             "首次越线必须告警")
            try expectNil(tracker.evaluate(used24h: 110_000, budget: 100_000,
                                           now: afterMidnight).alertMessage,
                          "跨日历日不是新的越线，同一段滚动窗口不得重复告警")
            // 真正回落到滞回线以下，才允许下一次越线重新告警
            _ = tracker.evaluate(used24h: 40_000, budget: 100_000,
                                 now: afterMidnight.addingTimeInterval(60))
            try expectNotNil(tracker.evaluate(used24h: 110_000, budget: 100_000,
                                              now: afterMidnight.addingTimeInterval(120)).alertMessage,
                             "回落后再次越线必须重新告警（去重不能变成永不告警）")
        }

        TestKit.test("TokenUsageMonitor 双源汇总（fixture 库）") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            let total = m.grandTotal
            // 口径 = 净消耗（input+output+reasoning / (prompt-cacheRead)+completion），cache.read 不计
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

        TestKit.test("TokenUsageMonitor 合并 Codex 与 WorkBuddy 结构化日志并去重") {
            let root = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-token-jsonl-\(UUID().uuidString)")
            let codexRoot = root.appendingPathComponent("codex")
            let workbuddyRoot = root.appendingPathComponent("workbuddy")
            try FileManager.default.createDirectory(at: codexRoot, withIntermediateDirectories: true)
            try FileManager.default.createDirectory(at: workbuddyRoot, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }

            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let iso = "2023-11-14T22:12:20.000Z"
            let codexLine = """
            {"timestamp":"\(iso)","type":"token_usage_record","payload":{"response_id":"response-1","usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":10,"reasoning_output_tokens":3}}}
            """
            // 同一 response_id 出现两次，只能计一次：(100-60)+10 = 50。
            try (codexLine + "\n" + codexLine + "\n").write(
                to: codexRoot.appendingPathComponent("rollout.jsonl"), atomically: true, encoding: .utf8
            )
            let workbuddyLine = """
            {"timestamp":1699999940000,"type":"function_call","id":"call-1","message":{"usage":{"input_tokens":80,"cache_read_input_tokens":20,"output_tokens":5,"total_tokens":85}}}
            """
            try (workbuddyLine + "\n").write(
                to: workbuddyRoot.appendingPathComponent("session.jsonl"), atomically: true, encoding: .utf8
            )

            let missing = root.appendingPathComponent("missing.sqlite").path
            let monitor = TokenUsageMonitor(
                dimAgentDB: missing,
                openCodeDB: missing + ".open",
                structuredSources: [
                    StructuredTokenSource(agentId: "codex", roots: [codexRoot.path], format: .codex),
                    StructuredTokenSource(agentId: "workbuddy", roots: [workbuddyRoot.path], format: .anthropic),
                    StructuredTokenSource(agentId: "claude", roots: [root.appendingPathComponent("claude").path],
                                          format: .anthropic),
                ]
            )
            monitor.refresh(now: now)
            try expectEqual(monitor.usage["codex"]?.tokensTotal, 50, "Codex reasoning 已含在 output，不重复相加")
            try expectEqual(monitor.usage["workbuddy"]?.tokensTotal, 65, "WorkBuddy 扣除 cache read")
            try expectEqual(monitor.grandTotal.tokensTotal, 115, "总体应跨工具求和")

            var timeline: TokenUsageTimeline?
            let done = Self.makeExpectation()
            monitor.timeline(range: .day, now: now) { value in
                timeline = value
                done.fulfill()
            }
            Self.waitMainActor(done, timeout: 10)
            let result = try XCTUnwrap(timeline, "结构化时间线查询超时")
            try expectEqual(result.tokens, 115)
            try expectEqual(result.sources.first { $0.agentId == "codex" }?.tokens, 50)
            try expectEqual(result.sources.first { $0.agentId == "workbuddy" }?.tokens, 65)
            try expectTrue(result.sources.first { $0.agentId == "claude" }?.isAvailable == false)
        }

        TestKit.test("Token 时间线查询合并双源并保留来源构成") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let monitor = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            var timeline: TokenUsageTimeline?
            let done = Self.makeExpectation()
            monitor.timeline(range: .week, now: Date()) { result in
                timeline = result
                done.fulfill()
            }
            Self.waitMainActor(done, timeout: 10)
            let result = try XCTUnwrap(timeline, "时间线查询超时")
            try expectEqual(result.tokens, 1545, "7 天覆盖 fixture 全部 assistant 净消耗")
            try expectEqual(result.points.reduce(0) { $0 + $1.tokens }, result.tokens, "桶合计必须等于范围合计")
            try expectEqual(result.sources.map(\.agentId), ["dim", "opencode"])
            try expectEqual(result.sources.first { $0.agentId == "dim" }?.tokens, 1300)
            try expectEqual(result.sources.first { $0.agentId == "opencode" }?.tokens, 245)
        }

        TestKit.test("dim 净消耗口径：promptTokens 中的 cacheRead 必须扣除") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            // m1 两条记录补上 cacheReadTokens（原 150 tok/条，缓存命中 120 → 净 30/条）
            try TokenFixture.exec(dbs.dimDB, [
                "UPDATE usage_ledger SET usage = '{\"promptTokens\":100,\"completionTokens\":50,\"cacheReadTokens\":120}' WHERE modelId = 'm1'",
            ])
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            // m1 净 = (100-120 钳制为 0) + 50 = 50/条 → 2 条 100；m2 1000 → 累计 1100
            try expectEqual(m.usage["dim"]?.tokensTotal, 1100, "dim 累计应扣除 cacheRead 并逐行钳制非负")
            try expectEqual(m.usage["dim"]?.tokens24h, 100, "dim 24h 同步扣除")

            var models: [ModelUsage]?
            var sessions: [SessionUsage]?
            let exp = Self.makeExpectation()
            m.modelBreakdown(agentId: "dim") { rows in
                models = rows
                m.sessions(agentId: "dim", modelId: "m1") { rows in
                    sessions = rows
                    exp.fulfill()
                }
            }
            Self.waitMainActor(exp, timeout: 10)
            let modelRows = try XCTUnwrap(models, "模型查询超时")
            let sessionRows = try XCTUnwrap(sessions, "会话查询超时")
            try expectEqual(modelRows.first { $0.modelId == "m1" }?.tokens, 100, "模型拆分口径一致")
            try expectEqual(sessionRows.reduce(0) { $0 + $1.tokens }, 100, "会话口径一致")
            try expectEqual(modelRows.reduce(0) { $0 + $1.tokens }, m.usage["dim"]?.tokensTotal, "模型合计 == 汇总")
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

        TestKit.test("数据库无写入时 24h 窗口仍会过期") {
            let now = Date()
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh(now: now)
            try expectEqual(m.grandTotal.tokens24h, 345, "初始窗口")
            m.refresh(now: now.addingTimeInterval(86_400))
            try expectEqual(m.grandTotal.tokens24h, 0, "未写入也应移出 24h 窗口")
            try expectEqual(m.grandTotal.tokensTotal, 1545, "累计保持")
        }

        TestKit.test("SQL 查询失败保留旧值，修复后继续更新") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            let before = m.grandTotal
            try TokenFixture.exec(dbs.dimDB, ["ALTER TABLE usage_ledger RENAME TO unavailable"])
            try TokenFixture.exec(dbs.openCodeDB, ["UPDATE message SET data = json_set(data, '$.tokens.input', 20) WHERE session_id = 's1'"])
            m.refresh()
            try expectEqual(m.usage["dim"]?.tokensTotal, 1300, "失败源保留")
            try expectEqual(m.grandTotal.tokensTotal, before.tokensTotal + 10, "健康源仍更新")
            try TokenFixture.exec(dbs.dimDB, ["ALTER TABLE unavailable RENAME TO usage_ledger", "UPDATE usage_ledger SET usage = '{\"promptTokens\":200,\"completionTokens\":50}' WHERE modelId = 'm1'"])
            m.refresh()
            try expectEqual(m.usage["dim"]?.tokensTotal, 1500, "恢复后重新查询")
        }

        TestKit.test("数据库暂时缺失保留旧统计") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            let before = m.grandTotal
            try FileManager.default.moveItem(atPath: dbs.dimDB, toPath: dbs.dimDB + ".backup")
            m.refresh()
            try expectEqual(m.grandTotal, before, "缺失不能表现为消耗下降")
        }

        TestKit.test("调度: refreshAsync 在飞去重（相邻触发合并为一次，R34/G4）") {
            // 首刷与 60s Timer 相邻触发时，第二个请求此前会在 refreshLock 上白等。
            // 观测：去重后连发 N 次只用一次刷新序列；用真实 monitor + fixture 库
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            for _ in 0..<8 { m.refreshAsync() }
            // 等待在飞刷新收尾
            let end = Date().addingTimeInterval(3.0)
            while Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
            // 原断言是 `usage["dim"] != nil || usage.isEmpty`——两个分支都真，恒过。
            // 夹具里 dim 近期两条各 150 净 token，合并后必须恰好 300，
            // 且 8 次连发不得把同一批记录累加出 8 倍
            try expectEqual(m.usage["dim"]?.tokens24h, 300,
                            "在飞去重后应恰好一次刷新序列的结果")
            try expectTrue(m.grandTotal.tokens24h >= 300, "汇总应包含 dim 的结果")
        }

        TestKit.test("数据源永久删除后连续缺失置空（源已消失终态）") {
            // R9：「查询失败保留旧值」不覆盖「源已消失」——永久删除后面板不应
            // 永久显示陈旧数字。连续 3 拍缺失（sourceMissingLimit）才置空，
            // 瞬时缺失（原子替换空窗）仍保留
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            try expectTrue(m.usage["dim"] != nil, "前置：dim 源有数据")
            try FileManager.default.removeItem(atPath: dbs.dimDB)
            m.refresh()
            try expectTrue(m.usage["dim"] != nil, "第 1 拍缺失保留旧值")
            m.refresh()
            try expectTrue(m.usage["dim"] != nil, "第 2 拍缺失保留旧值")
            m.refresh()
            try expectNil(m.usage["dim"], "第 3 拍缺失（达阈值）置空该源")
        }

        TestKit.test("成功查询空库会清除旧统计") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            try TokenFixture.exec(dbs.dimDB, ["DELETE FROM usage_ledger"])
            try TokenFixture.exec(dbs.openCodeDB, ["DELETE FROM message"])
            m.refresh()
            try expectEqual(m.grandTotal, TokenUsage(), "空数据与失败不同")
            try expectTrue(m.usage.isEmpty)
        }

        TestKit.test("详情缺少可选 token 字段仍与汇总一致") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            try TokenFixture.exec(dbs.dimDB, ["UPDATE usage_ledger SET usage = '{\"promptTokens\":100}' WHERE modelId = 'm1'"])
            try TokenFixture.exec(dbs.openCodeDB, ["UPDATE message SET data = json_remove(data, '$.tokens.reasoning')"])
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            m.refresh()
            for (agent, model, expected) in [("dim", "m1", 200), ("opencode", "oc1", 40)] {
                var models: [ModelUsage]?
                var sessions: [SessionUsage]?
                let exp = Self.makeExpectation()
                m.modelBreakdown(agentId: agent) { rows in
                    models = rows
                    m.sessions(agentId: agent, modelId: model) { rows in
                        sessions = rows
                        exp.fulfill()
                    }
                }
                Self.waitMainActor(exp, timeout: 10)
                let modelRows = try XCTUnwrap(models, "模型查询超时")
                let sessionRows = try XCTUnwrap(sessions, "会话查询超时")
                try expectEqual(modelRows.first { $0.modelId == model }?.tokens, expected, agent + " 模型")
                try expectEqual(sessionRows.reduce(0) { $0 + $1.tokens }, expected, agent + " 会话")
                try expectEqual(modelRows.reduce(0) { $0 + $1.tokens }, m.usage[agent]?.tokensTotal, agent + " 累计一致")
            }
        }

        TestKit.test("Token 刷新仅在统计变化时通知，窗口过期仍通知") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            let monitor = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            var notifications = 0
            monitor.onRefresh = { notifications += 1 }
            let now = Date()
            func drainCallbacks() {
                let end = Date().addingTimeInterval(0.1)
                while Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
            }
            monitor.refresh(now: now)
            monitor.refresh(now: now.addingTimeInterval(60))
            monitor.refresh(now: now.addingTimeInterval(120))
            drainCallbacks()
            try expectEqual(notifications, 1, "三次查询只有首次数据变化")
            try TokenFixture.exec(dbs.dimDB, ["UPDATE usage_ledger SET cost = cost + 1 WHERE modelId = 'm1'"])
            monitor.refresh(now: now.addingTimeInterval(180))
            drainCallbacks()
            try expectEqual(notifications, 2, "仅成本变化也应通知")
            monitor.refresh(now: now.addingTimeInterval(86_400))
            drainCallbacks()
            try expectEqual(notifications, 3, "24h 窗口过期应通知")
            try expectEqual(monitor.grandTotal.tokens24h, 0)
        }

        TestKit.test("REAL 形式 Token 在汇总与模型会话详情均能读取") {
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            try TokenFixture.exec(dbs.dimDB, ["UPDATE usage_ledger SET usage = '{\"promptTokens\":100.5,\"completionTokens\":50}' WHERE modelId = 'm1'"])
            try TokenFixture.exec(dbs.openCodeDB, ["UPDATE message SET data = json_set(data, '$.tokens.input', 10.5) WHERE session_id = 's1'"])
            let monitor = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            monitor.refresh()
            for (agent, model, expected) in [("dim", "m1", 301), ("opencode", "oc1", 45)] {
                var models: [ModelUsage]?
                var sessions: [SessionUsage]?
                let done = Self.makeExpectation()
                monitor.modelBreakdown(agentId: agent) { rows in
                    models = rows
                    monitor.sessions(agentId: agent, modelId: model) { rows in
                        sessions = rows
                        done.fulfill()
                    }
                }
                Self.waitMainActor(done, timeout: 5)
                let modelRows = try XCTUnwrap(models)
                let sessionRows = try XCTUnwrap(sessions)
                try expectEqual(modelRows.first { $0.modelId == model }?.tokens, expected, agent + " 模型")
                try expectEqual(sessionRows.reduce(0) { $0 + $1.tokens }, expected, agent + " 会话")
                try expectEqual(modelRows.reduce(0) { $0 + $1.tokens }, monitor.usage[agent]?.tokensTotal, agent + " 汇总")
            }
        }

        // MARK: 健壮性：外部脏数据不得令进程 trap（越界 / Inf / NaN / 哨兵时间戳）

        TestKit.test("SafeNumber.parseInt 饱和解析：越界不 trap，Inf/NaN/非数值归零") {
            try expectEqual(SafeNumber.parseInt("12", source: "t"), 12, "普通整数")
            try expectEqual(SafeNumber.parseInt("19067783.5", source: "t"), 19_067_783, "REAL 聚合文本取整数部分")
            try expectEqual(SafeNumber.parseInt("99999999999999999999", source: "t"),
                            SafeNumber.magnitudeCeiling, "超 Int64 的数字字符串饱和")
            try expectEqual(SafeNumber.parseInt("1e19", source: "t"),
                            SafeNumber.magnitudeCeiling, "科学计数法越界饱和")
            try expectEqual(SafeNumber.parseInt("Inf", source: "t"),
                            SafeNumber.magnitudeCeiling, "两行超大值求和溢出为 Inf")
            try expectEqual(SafeNumber.parseInt("NaN", source: "t"), 0, "NaN 归零")
            try expectEqual(SafeNumber.parseInt("", source: "t"), 0, "空串归零")
            try expectEqual(SafeNumber.parseInt("abc", source: "t"), 0, "非数值归零")
            try expectEqual(SafeNumber.parseInt("-1e19", source: "t"),
                            -SafeNumber.magnitudeCeiling, "负向同样饱和")
            try expectEqual(SafeNumber.saturatingInt(.greatestFiniteMagnitude, source: "t"),
                            SafeNumber.magnitudeCeiling, "Double 上限饱和")
            try expectEqual(SafeNumber.saturatingInt(.infinity, source: "t"),
                            SafeNumber.magnitudeCeiling, "Inf 饱和")
        }

        TestKit.test("SafeNumber.parseCost 非有限值归零、越界钳制") {
            try expectTrue(abs(SafeNumber.parseCost("0.42", source: "t") - 0.42) < 1e-9, "正常值")
            try expectEqual(SafeNumber.parseCost("Inf", source: "t"), 0, "Inf 归零")
            try expectEqual(SafeNumber.parseCost("NaN", source: "t"), 0, "NaN 归零")
            try expectEqual(SafeNumber.parseCost("1e300", source: "t"), SafeNumber.costCeiling, "越界钳制")
            try expectEqual(SafeNumber.parseCost("abc", source: "t"), 0, "非数值归零")
        }

        TestKit.test("SafeNumber.date(fromMillisText:) 非法时间戳返回 nil") {
            try expectNil(SafeNumber.date(fromMillisText: "Inf", source: "t"))
            try expectNil(SafeNumber.date(fromMillisText: "NaN", source: "t"))
            try expectNil(SafeNumber.date(fromMillisText: "0", source: "t"), "0 非合法纪元")
            try expectNil(SafeNumber.date(fromMillisText: "1e20", source: "t"), "超上限")
            try expectTrue(SafeNumber.date(fromMillisText: "1700000000000", source: "t") != nil, "正常毫秒")
        }

        TestKit.test("SafeNumber.date(fromEpochMillis:) 拒收哨兵 / 越界毫秒") {
            try expectNil(SafeNumber.date(fromEpochMillis: Int64.max, source: "t"), "Int64 上限哨兵")
            try expectNil(SafeNumber.date(fromEpochMillis: 0, source: "t"))
            try expectNil(SafeNumber.date(fromEpochMillis: -1, source: "t"))
            try expectTrue(SafeNumber.date(fromEpochMillis: 1_700_000_000_000, source: "t") != nil, "正常毫秒")
        }

        TestKit.test("Token 汇总：越界 / Inf 数据源不崩溃且按上限钳制") {
            // 修复前此用例直接 fatalError（exit 133）：Int(Double(1e19)) trap
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-dirty-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let dimDB = dir.appendingPathComponent("dim.sqlite").path
            try TokenFixture.exec(dimDB, [
                "CREATE TABLE usage_ledger (createdAt TEXT, modelId TEXT, usage TEXT, cost REAL, sessionId TEXT)",
                "INSERT INTO usage_ledger VALUES ('\(TokenFixture.iso(Date()))', 'm1', '{\"promptTokens\":1e19,\"completionTokens\":0}', 1e300, 's1')",
            ])
            let monitor = TokenUsageMonitor(dimAgentDB: dimDB,
                                            openCodeDB: dir.appendingPathComponent("none.db").path)
            monitor.refresh()
            try expectEqual(monitor.grandTotal.tokensTotal, SafeNumber.magnitudeCeiling, "越界 token 饱和")
            try expectEqual(monitor.grandTotal.costTotal, SafeNumber.costCeiling, "越界 cost 钳制")
        }

        TestKit.test("AgentLogEvent id：哨兵 / Inf / NaN 时间戳不 trap") {
            let sentinel = AgentLogEvent(timestamp: Date(timeIntervalSince1970: Double(Int64.max)),
                                         kind: .info, title: "t", agentId: "a")
            try expectTrue(sentinel.id.hasPrefix("a-"), "id 前缀应含 agentId")
            let inf = AgentLogEvent(timestamp: Date(timeIntervalSince1970: .infinity),
                                    kind: .info, title: "t", agentId: "a")
            try expectTrue(!inf.id.isEmpty, "Inf 时间戳仍应生成 id")
            let nan = AgentLogEvent(timestamp: Date(timeIntervalSince1970: .nan),
                                    kind: .info, title: "t", agentId: "a")
            try expectTrue(!nan.id.isEmpty, "NaN 时间戳仍应生成 id")
        }

        TestKit.test("AgentLogStreamer 流水窗口 SQL：仍返回最新 N 条") {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-events-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let dbPath = dir.appendingPathComponent("dimcode.sqlite").path
            var stmts = ["CREATE TABLE messages (createdAt TEXT, role TEXT, parts TEXT, toolMetadata TEXT)"]
            for i in 0..<600 {
                stmts.append("INSERT INTO messages VALUES ('\(TokenFixture.iso(Date(timeIntervalSince1970: 1_700_000_000 + Double(i))))', 'user', 'row-\(i)', '')")
            }
            try TokenFixture.exec(dbPath, stmts)

            let parts = try Self.queryColumn(dbPath, sql: AgentLogStreamer.dimEventsSQL(limit: 20), column: 2)
            try expectEqual(parts.count, 20, "窗口查询应返回 20 条")
            try expectEqual(parts.first, "row-599", "最近插入的排最前")
            try expectEqual(Set(parts).count, 20, "无重复")
        }

        TestKit.test("AgentLogStreamer opencode 流水窗口 SQL：仍返回最新 N 条") {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-parts-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let dbPath = dir.appendingPathComponent("opencode.db").path
            var stmts = ["CREATE TABLE part (data TEXT, time_created INTEGER)"]
            for i in 0..<600 {
                stmts.append("INSERT INTO part VALUES ('row-\(i)', \(1_700_000_000_000 + i))")
            }
            try TokenFixture.exec(dbPath, stmts)

            let data = try Self.queryColumn(dbPath, sql: AgentLogStreamer.openCodeEventsSQL(limit: 20), column: 0)
            try expectEqual(data.count, 20, "窗口查询应返回 20 条")
            try expectEqual(data.first, "row-599", "time_created 最大的排最前")
            try expectEqual(Set(data).count, 20, "无重复")
        }

        TestKit.test("AgentLogStreamer.recentWindow 边界") {
            try expectEqual(AgentLogStreamer.recentWindow(limit: 20), 500, "小 limit 保底 500")
            try expectEqual(AgentLogStreamer.recentWindow(limit: 100), 2500, "大 limit 等比放大")
            try expectEqual(AgentLogStreamer.recentWindow(limit: 0), 500, "limit 非正时保底")
        }

        TestKit.test("ReadonlyDB 失败路径不泄漏 handle 且缺失库返回 nil") {
            // sqlite3_open_v2 失败时 handle 仍可能非 NULL；不 close 则每次泄漏约 1.5KB。
            // 2 万次失败开库（权限拒绝 → rc=14，handle 非 NULL 的经典泄漏形态）：
            // 修复前 +28MB，修复后应接近 0。（原测 AgentLogStreamer.openReadonly；
            // R06 起 DB 开闭统一收口 ReadonlyDB。缺失库现在走 stat 快路径，需用
            // 「存在但不可读」文件才能命中 open 失败分支——runner 非 root，chmod 生效）
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let locked = dir.appendingPathComponent("locked.db")
            try Data("not a db".utf8).write(to: locked)
            try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: locked.path)
            defer {
                try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: locked.path)
                try? FileManager.default.removeItem(at: dir)
            }
            let before = Self.residentMemoryMB()
            for _ in 0..<20_000 {
                let got: String? = ReadonlyDB.withConnection(locked.path) { _ in "x" }
                try expectNil(got, "权限拒绝的库应打开失败")
            }
            let delta = Self.residentMemoryMB() - before
            try expectTrue(delta < 8, "2 万次失败开库不应持续增长（实测 +28MB 为泄漏），实际 +\(String(format: "%.1f", delta))MB")

            // 缺失库走 stat 快路径，同样返回 nil（不发起注定失败的 open）
            let missing = "/tmp/agentisland-no-such-dir-\(UUID().uuidString)/no-such.db"
            let got: String? = ReadonlyDB.withConnection(missing) { _ in "x" }
            try expectNil(got, "不存在的库应打开失败")

            // 失败原因出口：nil 结果本身分不出「没数据」与「读不到」，会话探测要用后者
            var openFailure: ReadonlyDB.ConnectionFailure?
            let gotLocked: String? = ReadonlyDB.withConnection(locked.path,
                                                              onFailure: { openFailure = $0 }) { _ in "x" }
            try expectNil(gotLocked, "onFailure 变体的降级语义不变")
            if case .openFailed? = openFailure {
                // 存在但打不开 → openFailed（带上 rc 供诊断）
            } else {
                throw TestError(message: "不可读库应记录 openFailed，实际 \(String(describing: openFailure))")
            }
            var missingFailure: ReadonlyDB.ConnectionFailure?
            _ = ReadonlyDB.withConnection(missing, onFailure: { missingFailure = $0 }) { _ in "x" }
            try expectEqual(missingFailure, .missing, "缺失库应单独成类（会话探测据此不误报故障）")
        }

        // MARK: - R38/P2·P3：单趟汇总与戳备忘录

        TestKit.test("Token汇总: dim 的 24h 与累计合并成单趟后必须逐列等于原两趟") {
            // 两趟→一趟（SUM(CASE WHEN …)）只允许改「怎么扫」，不允许改「扫出什么」。
            // 夹具把三条容易走偏的分支都摆进来了：cost 为 NULL、completionTokens 键缺失
            // （原式靠 SUM 跳过 NULL）、cacheRead > prompt 使净额逐行钳制为 0。
            // 四个期望值两两不同，任何「24h / 累计」列序写反都会露馅。
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let dimDB = dir.appendingPathComponent("dimcode.sqlite").path
            // 用闭包而不是嵌套 func：闭包继承外层 @MainActor 隔离，能直接调 TokenFixture.iso
            let row: (TimeInterval, String, String) -> String = { age, usage, cost in
                "INSERT INTO usage_ledger VALUES ('\(TokenFixture.iso(now.addingTimeInterval(age)))', 'm1', '\(usage)', \(cost), 's')"
            }
            try TokenFixture.exec(dimDB, [
                "CREATE TABLE usage_ledger (createdAt TEXT, modelId TEXT, usage TEXT, cost REAL, sessionId TEXT)",
                row(-60, "{\"promptTokens\":100,\"completionTokens\":50}", "0.10"),
                row(-48 * 3600, "{\"promptTokens\":1000}", "2.00"),
                row(-30, "{\"promptTokens\":30}", "NULL"),
                row(-20, "{\"promptTokens\":10,\"cacheReadTokens\":40,\"completionTokens\":7}", "0.50"),
            ])
            let cutoff = TokenUsageMonitor.iso24hAgo(now: now)
            let net = DimUsageSQL.netTokens
            // 参照 = 改造前的两条 SQL（各自全表扫一遍，逐字照搬）
            let refTokens24 = try queryColumn(dimDB, sql: "SELECT \(net) FROM usage_ledger WHERE createdAt >= '\(cutoff)'", column: 0).first ?? ""
            let refTokensAll = try queryColumn(dimDB, sql: "SELECT \(net) FROM usage_ledger", column: 0).first ?? ""
            let refCost24 = try queryColumn(dimDB, sql: "SELECT COALESCE(SUM(cost),0) FROM usage_ledger WHERE createdAt >= '\(cutoff)'", column: 0).first ?? ""
            let refCostAll = try queryColumn(dimDB, sql: "SELECT COALESCE(SUM(cost),0) FROM usage_ledger", column: 0).first ?? ""

            let m = TokenUsageMonitor(dimAgentDB: dimDB, openCodeDB: dir.appendingPathComponent("none.db").path)
            m.refresh(now: now)
            let dim = try XCTUnwrap(m.usage["dim"], "dim 源缺席")
            try expectEqual("\(dim.tokensTotal)", refTokensAll, "累计 token 必须等于原两趟")
            try expectEqual("\(dim.tokens24h)", refTokens24, "24h token 必须等于原两趟")
            try expectEqual(dim.costTotal, Double(refCostAll) ?? -1, "累计 cost 必须等于原两趟")
            try expectEqual(dim.cost24h, Double(refCost24) ?? -1, "24h cost 必须等于原两趟")
            try expectEqual(dim.tokensTotal, 1187, "(100+50)+1000+30+7")
            try expectEqual(dim.tokens24h, 187, "累计去掉 48h 前那条")
            try expectTrue(abs(dim.costTotal - 2.6) < 0.0001, "NULL cost 记 0")
            try expectTrue(abs(dim.cost24h - 0.6) < 0.0001)
        }

        TestKit.test("Token汇总: opencode 的 24h 与累计合并成单趟后必须逐列等于原两趟") {
            // 本机没有 opencode.db（该 Agent 未安装），所以这条只能靠同 schema 夹具对拍：
            // 原式是「每列一个 SUM、NULL 由 SUM 跳过」，现式是「逐行 COALESCE 后相加」，
            // 两者对缺失字段必须给出同一个数。
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let ocDB = dir.appendingPathComponent("opencode.db").path
            let row: (TimeInterval, String) -> String = { age, data in
                "INSERT INTO message VALUES ('s1', '\(data)', \(TokenFixture.ms(now.addingTimeInterval(age))))"
            }
            try TokenFixture.exec(ocDB, [
                "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER)",
                row(-60, "{\"role\":\"assistant\",\"tokens\":{\"input\":10,\"output\":20,\"reasoning\":5},\"cost\":0.5}"),
                row(-48 * 3600, "{\"role\":\"assistant\",\"tokens\":{\"input\":200},\"cost\":2.0}"),
                // 缺 output/reasoning 与 cost：原式三个 SUM 各跳过 NULL，现式逐项 COALESCE
                row(-30, "{\"role\":\"assistant\",\"tokens\":{\"input\":7}}"),
                // user 角色：两条查询都不得计入
                row(-60, "{\"role\":\"user\",\"tokens\":{\"input\":50000,\"output\":1},\"cost\":9.9}"),
            ])
            let cutoffMs = Int64(now.timeIntervalSince1970 * 1000) - 86_400_000
            let expr = """
            COALESCE(SUM(json_extract(data,'$.tokens.input')),0)
            + COALESCE(SUM(json_extract(data,'$.tokens.output')),0)
            + COALESCE(SUM(json_extract(data,'$.tokens.reasoning')),0)
            """
            let role = "json_extract(data,'$.role')='assistant'"
            let refTokens24 = try queryColumn(ocDB, sql: "SELECT \(expr) FROM message WHERE \(role) AND time_created >= \(cutoffMs)", column: 0).first ?? ""
            let refTokensAll = try queryColumn(ocDB, sql: "SELECT \(expr) FROM message WHERE \(role)", column: 0).first ?? ""
            let refCost24 = try queryColumn(ocDB, sql: "SELECT COALESCE(SUM(json_extract(data,'$.cost')),0) FROM message WHERE \(role) AND time_created >= \(cutoffMs)", column: 0).first ?? ""
            let refCostAll = try queryColumn(ocDB, sql: "SELECT COALESCE(SUM(json_extract(data,'$.cost')),0) FROM message WHERE \(role)", column: 0).first ?? ""

            let m = TokenUsageMonitor(dimAgentDB: dir.appendingPathComponent("none.sqlite").path, openCodeDB: ocDB)
            m.refresh(now: now)
            let oc = try XCTUnwrap(m.usage["opencode"], "opencode 源缺席")
            try expectEqual("\(oc.tokensTotal)", refTokensAll, "累计 token 必须等于原两趟")
            try expectEqual("\(oc.tokens24h)", refTokens24, "24h token 必须等于原两趟")
            try expectEqual(oc.costTotal, Double(refCostAll) ?? -1, "累计 cost 必须等于原两趟")
            try expectEqual(oc.cost24h, Double(refCost24) ?? -1, "24h cost 必须等于原两趟")
            try expectEqual(oc.tokensTotal, 242, "35+200+7，user 角色不计")
            try expectEqual(oc.tokens24h, 42, "去掉 48h 前那条")
        }

        TestKit.test("结构化Token索引: 无关文件变动不得改变聚合，日志自身变化必须计入（戳备忘录）") {
            // 稳态命中的全库戳备忘录把「摊平 + 全局去重 + 分桶」整套跳过（本机实测
            // 5,778 条记录时快照稳态 8.25ms → 2.70ms）。它只在整棵日志树的戳逐字不变时
            // 生效，因此两头都要钉住：无关变动不得改一个数字，日志变动一个数字都不能漏。
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            let logs = root.appendingPathComponent("codex")
            try FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [logs.path], format: .codex)
            ])
            let rollout = logs.appendingPathComponent("rollout.jsonl")
            // 同一 response_id 出现两次 → 只计一次（(100-60)+10 = 50）
            try (Self.codexLine("r1", 5) + "\n" + Self.codexLine("r1", 5) + "\n")
                .write(to: rollout, atomically: true, encoding: .utf8)

            let base = Self.structuredSignature(index.snapshot(now: Self.structuredClock))
            try expectTrue(base.contains("n=1"), "一条去重后的记录，实际 \(base)")

            // 1) 同一棵树里新增无关文件、改动无关文件：聚合数字必须逐字不变
            try Data("readme".utf8).write(to: logs.appendingPathComponent("README.md"))
            try expectEqual(Self.structuredSignature(index.snapshot(now: Self.structuredClock)), base, "同目录非 jsonl 文件不得影响聚合")
            // 2) 另一个 root 里新增 jsonl 之外的一堆目录/文件（戳集合不变）
            let other = root.appendingPathComponent("noise/deep/dir")
            try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
            try Data("x".utf8).write(to: other.appendingPathComponent("scratch.txt"))
            try expectEqual(Self.structuredSignature(index.snapshot(now: Self.structuredClock)), base, "监控根之外的变动不得影响聚合")
            // 3) 追加新响应到已有日志（同 inode 增量分支）：必须立刻出现在聚合里
            let handle = try FileHandle(forWritingTo: rollout)
            try handle.seekToEnd()
            try handle.write(contentsOf: Data((Self.codexLine("r2", 3, input: 300, cached: 100, output: 20)
                + "\n" + Self.codexLine("r3", 2, input: 40, cached: 0, output: 6) + "\n").utf8))
            try handle.close()
            let appended = Self.structuredSignature(index.snapshot(now: Self.structuredClock))
            try expectTrue(appended != base, "追加新响应必须重算")
            try expectTrue(appended.contains("n=3"), "r1 去重后共 3 条，实际 \(appended)")
            // 4) 新增日志文件：必须计入
            let extra = logs.appendingPathComponent("rollout2.jsonl")
            try (Self.codexLine("r4", 1, input: 10, cached: 0, output: 1) + "\n").write(to: extra, atomically: true, encoding: .utf8)
            try expectTrue(Self.structuredSignature(index.snapshot(now: Self.structuredClock)).contains("n=4"), "新文件的响应不得被备忘录吞掉")
            // 5) 删除日志文件：必须退出（备忘录不得把已消失的来源继续算进去）
            try FileManager.default.removeItem(at: extra)
            try expectTrue(Self.structuredSignature(index.snapshot(now: Self.structuredClock)).contains("n=3"), "删除文件后必须回退")
        }

        TestKit.test("结构化Token索引: 段尾截在半行既不重复计数，也不得触发整文件重解析") {
            // 增量解析只承认「扫描时量到的那段」，并且必须退回到最后一个完整行。
            // 不回退的话：半行被当完整行解析（重复计数），或者 endedWithNewline=false
            // 让下一轮 canAppend 不成立 → offset 归零 → 整文件重解析（比收口前更慢）
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            let logs = root.appendingPathComponent("codex")
            try FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [logs.path], format: .codex)
            ])
            let rollout = logs.appendingPathComponent("rollout.jsonl")
            try (Self.codexLine("r1", 5) + "\n").write(to: rollout, atomically: true, encoding: .utf8)
            let base = Self.structuredSignature(index.snapshot(now: Self.structuredClock))
            try expectTrue(base.contains("n=1"), "前置：一条记录，实际 \(base)")

            // 写一半（没有结尾换行）——正在被第三方追加的行不得计入
            let appending = try FileHandle(forWritingTo: rollout)
            try appending.seekToEnd()
            try appending.write(contentsOf: Data(Self.codexLine("r2", 3).utf8))
            try appending.close()
            let partial = Self.structuredSignature(index.snapshot(now: Self.structuredClock))
            try expectEqual(partial, base, "未写完的一行不该出现在聚合里")

            // 补齐换行：必须恰好 +1，且那半行不被重复计入
            let closing = try FileHandle(forWritingTo: rollout)
            try closing.seekToEnd()
            try closing.write(contentsOf: Data("\n".utf8))
            try closing.close()
            let done = Self.structuredSignature(index.snapshot(now: Self.structuredClock))
            try expectTrue(done.contains("n=2"), "补完整行后必须计入一条，实际 \(done)")
            // 反复采样数字必须稳定（重复计数或重解析回退都会在这里露出来）
            try expectEqual(Self.structuredSignature(index.snapshot(now: Self.structuredClock)), done, "第二轮采样数字漂移")
            try expectEqual(Self.structuredSignature(index.snapshot(now: Self.structuredClock)), done, "第三轮采样数字漂移")
        }

        TestKit.test("结构化Token索引: 掉出保留窗口的明细折进合计，累计一个数都不变") {
            // 保留窗口的全部意义在于这一条不变式：**明细可以少，累计不能变**。
            // 少了这行断言，「按窗口裁剪」就会被写成「按窗口丢数据」——面板上的累计
            // 会静默下降，而那是用户唯一能看到的历史总量。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            // 三条响应：2 小时前、75 天前、400 天前。净 token = (input - cached) + output
            // 净 token = (input - cached) + output ⇒ 120 / 240 / 480
            try [
                Self.codexLine("fresh", 120, input: 100, cached: 0, output: 20),
                Self.codexLine("mid", 75 * 24 * 60, input: 240, cached: 0, output: 0),
                Self.codexLine("ancient", 400 * 24 * 60, input: 480, cached: 0, output: 0),
            ].map { $0 + "\n" }.joined()
                .write(toFile: dir.appendingPathComponent("r.jsonl").path,
                       atomically: true, encoding: .utf8)

            let snap = index.snapshot(now: Self.structuredClock)
            try expectEqual(snap.records.count, 1, "只有窗口内的 1 条留在明细里")
            try expectEqual(snap.records.first?.tokens, 120, "留在明细里的必须是 2 小时前那条")
            try expectEqual(snap.rolledUpTokens["codex"], 720, "75 天与 400 天前的折进合计")
            try expectEqual(snap.rolledUpCount["codex"], 2, "折入条数要如实，不能只留一个总额")
            let usage = snap.usage(now: Self.structuredClock)
            try expectEqual(usage["codex"]?.tokensTotal, 840, "累计 = 明细 + 折入，一个数都不能少")
            try expectEqual(usage["codex"]?.tokens24h, 120, "24h 只看窗口内的，折入不得掺进来")
        }

        TestKit.test("结构化Token索引: 只剩折入历史的工具照样在册，累计不为零") {
            // 全量折入后 records 为空。若桶只从 records 派生，这个工具会整体从面板上
            // 消失——「没看到」被写成「没有」，正是本仓库最忌讳的那类错。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            try (Self.codexLine("old", 90 * 24 * 60, input: 300, cached: 100, output: 0) + "\n")
                .write(toFile: dir.appendingPathComponent("r.jsonl").path, atomically: true, encoding: .utf8)

            let snap = index.snapshot(now: Self.structuredClock)
            try expectTrue(snap.records.isEmpty, "90 天前的响应不该留在明细里")
            let usage = snap.usage(now: Self.structuredClock)
            try expectEqual(usage["codex"]?.tokensTotal, 200, "折入的累计必须照样发布")
            try expectEqual(usage["codex"]?.tokens24h, 0, "24h 口径不受折入影响")
            try expectTrue(snap.availableToolIds.contains("codex"), "工具活性与明细留存无关")
        }

        TestKit.test("结构化Token索引: 同一文件里抄了两遍的旧响应，折入后仍只计一次") {
            // fork/恢复会把父会话整段抄进新文件；旧的那一段一旦掉出窗口就再也不进 records，
            // 折入路径若在文件内去过重之前就把两份都加进合计，累计会凭空翻倍。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            let dup = Self.codexLine("same", 80 * 24 * 60, input: 100, cached: 60, output: 10)
            try [dup, dup].map { $0 + "\n" }.joined()
                .write(toFile: dir.appendingPathComponent("r.jsonl").path, atomically: true, encoding: .utf8)

            let snap = index.snapshot(now: Self.structuredClock)
            try expectEqual(snap.rolledUpTokens["codex"], 50, "同一 response_id 两份只算一次")
            try expectEqual(snap.rolledUpCount["codex"], 1, "折入条数同样要去重")
            try expectEqual(snap.usage(now: Self.structuredClock)["codex"]?.tokensTotal, 50, "累计")
        }

        TestKit.test("结构化Token索引: 折入之后再追加新响应，不得把已折入的历史再折一遍") {
            // 增量路径只该折「本轮新读到的这一段」。若把旧合计连同旧明细一起从头重算，
            // 已折入的响应会在合计里出现第二份——而且文件每追加一次就多算一次（线性膨胀）。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            let file = dir.appendingPathComponent("r.jsonl")
            let old = Self.codexLine("old", 75 * 24 * 60, input: 240, cached: 0, output: 0)
            try (old + "\n").write(toFile: file.path, atomically: true, encoding: .utf8)
            let first = index.snapshot(now: Self.structuredClock)
            try expectEqual(first.rolledUpTokens["codex"], 240, "前置：旧响应已折入")
            try expectTrue(first.records.isEmpty, "前置：明细为空")

            // 追加一条窗口内的新响应，并连采三轮
            let handle = try FileHandle(forWritingTo: file)
            try handle.seekToEnd()
            try handle.write(contentsOf: Data(
                (Self.codexLine("new", 60, input: 100, cached: 0, output: 0) + "\n").utf8))
            try handle.close()
            for round in 1...3 {
                let snap = index.snapshot(now: Self.structuredClock.addingTimeInterval(Double(round) * 60))
                try expectEqual(snap.rolledUpTokens["codex"], 240, "第 \(round) 轮：折入合计必须原地不动")
                try expectEqual(snap.rolledUpCount["codex"], 1, "第 \(round) 轮：折入条数")
                try expectEqual(snap.records.count, 1, "第 \(round) 轮：新响应留在明细里")
                try expectEqual(snap.usage(now: Self.structuredClock)["codex"]?.tokensTotal, 340,
                                "第 \(round) 轮：累计 = 240 + 100")
            }
        }

        TestKit.test("结构化Token索引: 已折入的响应又被追加一遍，同一文件内仍只能算一次") {
            // 折入去重必须看得见「已经折掉的那些键」。增量路径只带着留下的明细，
            // 于是老会话被重新追加（resume 一个 40 天前的 rollout 文件）时，被折过的
            // 响应会在新尾段里出现第二份：累计凭空多一条，而且这条落在 24h 里。
            // 改动前的实现是全摊平 + 全局去重，这条断言钉住那份语义。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            let file = dir.appendingPathComponent("r.jsonl")
            let aged = Self.codexLine("r1", 75 * 24 * 60, input: 240, cached: 0, output: 0)
            try (aged + "\n").write(toFile: file.path, atomically: true, encoding: .utf8)
            let first = index.snapshot(now: Self.structuredClock)
            try expectEqual(first.rolledUpTokens["codex"], 240, "前置：75 天前的响应已折入")
            try expectEqual(first.usage(now: Self.structuredClock)["codex"]?.tokensTotal, 240, "前置：累计")

            // 同一 response_id 再被写一遍，这次的时间戳在窗口内（重放/恢复的真实形状）
            let handle = try FileHandle(forWritingTo: file)
            try handle.seekToEnd()
            try handle.write(contentsOf: Data(
                (Self.codexLine("r1", 60, input: 240, cached: 0, output: 0) + "\n").utf8))
            try handle.close()
            let second = index.snapshot(now: Self.structuredClock)
            try expectEqual(second.usage(now: Self.structuredClock)["codex"]?.tokensTotal, 240,
                            "同一响应的两份拷贝只能算一次（全局去重的语义不能因折入而丢）")
            try expectEqual(second.usage(now: Self.structuredClock)["codex"]?.tokens24h, 0,
                            "先到先得的是 75 天前那条，24h 不该多出这一笔")
        }

        TestKit.test("结构化Token索引: 同路径换 inode 后从零重算，合计不重复累加") {
            // 整份重解析（!canAppend）看得见整个文件 ⇒ 折入必须整体重算而不是往上叠。
            // 同名文件被替换是备份恢复/迁移的常态，这里数字必须与第一趟逐字相同。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            let file = dir.appendingPathComponent("r.jsonl")
            // 局部捕获：`structuredClock` 是 MainActor 隔离的静态属性，非隔离的嵌套函数
            // 里直接引用在 Swift 6 语言模式下是错误
            let clock = Self.structuredClock
            func signature(_ snap: StructuredTokenUsageSnapshot) -> String {
                "n=\(snap.records.count) rolled=\(snap.rolledUpTokens["codex"] ?? 0)"
                    + "/\(snap.rolledUpCount["codex"] ?? 0) total="
                    + "\(snap.usage(now: clock)["codex"]?.tokensTotal ?? -1)"
            }
            let body = [Self.codexLine("a", 75 * 24 * 60, input: 240, cached: 0, output: 0),
                        Self.codexLine("b", 10, input: 100, cached: 0, output: 0)].map { $0 + "\n" }.joined()
            try body.write(toFile: file.path, atomically: true, encoding: .utf8)
            let base = signature(index.snapshot(now: Self.structuredClock))
            try expectEqual(base, "n=1 rolled=240/1 total=340", "前置签名")

            // 换 inode 重写同一路径（内容逐字相同）
            try FileManager.default.removeItem(at: file)
            try body.write(toFile: file.path, atomically: true, encoding: .utf8)
            try expectEqual(signature(index.snapshot(now: Self.structuredClock)), base,
                            "整份重解析不得让折入历史翻倍")
        }

        TestKit.test("结构化Token索引: 时钟推进把整份文件折掉时，备忘录不得继续吐旧明细") {
            // 这一条同时钉两件事：① 长期不动的会话能被回收（戳未变也折）；
            // ② 备忘录命中不能把刚回收的那份快照再吐回来（否则数组永远释不掉）。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            try (Self.codexLine("a", 60, input: 240, cached: 0, output: 0) + "\n")
                .write(toFile: dir.appendingPathComponent("r.jsonl").path, atomically: true, encoding: .utf8)
            let inWindow = index.snapshot(now: Self.structuredClock)
            try expectEqual(inWindow.records.count, 1, "前置：1 小时前的响应在明细里")

            // 此后磁盘一个字节没变，只是时钟走过了 71 天
            let later = index.snapshot(now: Self.structuredClock.addingTimeInterval(71 * 86_400))
            try expectTrue(later.records.isEmpty, "整份明细已掉出窗口，必须折掉")
            try expectEqual(later.rolledUpTokens["codex"], 240, "折入合计")
            try expectEqual(inWindow.usage(now: Self.structuredClock)["codex"]?.tokensTotal,
                            240, "折入前")
            try expectEqual(later.usage(now: Self.structuredClock)["codex"]?.tokensTotal,
                            240, "折入后累计不变")
        }

        TestKit.test("结构化Token索引: 单文件明细上限只裁明细，不裁累计") {
            // 第二道保险。触顶时折掉**最早**的那一段，而不是丢掉：累计照样成立。
            // 比上限多做 3 条就够——再多只是把同一个分支跑得慢一点。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            let over = StructuredTokenUsageIndex.maxDetailEventsPerFile + 3
            var lines = ""
            lines.reserveCapacity(over * 190)
            for i in 0..<over {
                // 每条净 token = 1（input 1），时间都在窗口内且互不相同
                lines += Self.codexLine("cap-\(i)", i, input: 1, cached: 0, output: 0) + "\n"
            }
            try lines.write(toFile: dir.appendingPathComponent("big.jsonl").path,
                            atomically: true, encoding: .utf8)

            let snap = index.snapshot(now: Self.structuredClock)
            try expectEqual(snap.records.count, StructuredTokenUsageIndex.maxDetailEventsPerFile,
                            "明细被裁到上限")
            try expectEqual(snap.rolledUpCount["codex"], 3, "裁掉的 3 条是折入，不是丢弃")
            try expectEqual(snap.usage(now: Self.structuredClock)["codex"]?.tokensTotal, over,
                            "累计一条都不能少")
            // 裁掉的必须是**最早**那 3 条：留在明细里的最早一条 = 第 20_000 新（19_999 分钟前）
            let oldestKept = try XCTUnwrap(snap.records.map(\.time).min())
            try expectTrue(abs(oldestKept.timeIntervalSince(
                Self.structuredClock.addingTimeInterval(-19_999 * 60))) < 1.0,
                "裁的必须是最早的那一段，实际最早=\(oldestKept.timeIntervalSince1970)")

            // 被上限裁掉的那些 id 也得留在折入去重的视野里：把它们之中的一个再追加一遍
            // （时间戳是新的），数字一个都不能动。做不到就得让这类文件走整份重读——
            // 上限裁的是**窗口内**的明细，比按时间折出更常见，这个洞更好踩到。
            let big = dir.appendingPathComponent("big.jsonl")
            let handle = try FileHandle(forWritingTo: big)
            try handle.seekToEnd()
            try handle.write(contentsOf: Data(
                (Self.codexLine("cap-0", 1, input: 1, cached: 0, output: 0) + "\n").utf8))
            try handle.close()
            let after = index.snapshot(now: Self.structuredClock)
            try expectEqual(after.usage(now: Self.structuredClock)["codex"]?.tokensTotal, over,
                            "重复的 cap-0 只能算一次（累计不能因为多一条重复而涨）")
            try expectEqual(after.rolledUpCount["codex"], 3, "折入条数不变")
            try expectEqual(after.records.count, StructuredTokenUsageIndex.maxDetailEventsPerFile,
                            "明细条数不变")
        }

        TestKit.test("结构化Token索引: 时钟回拨再前进，折入不重复累加也不丢账") {
            // 睡眠/唤醒、NTP 校正都会让 `now` 往回跳。折入状态必须经得起窗口来回挪：
            // 既不能把已经折掉的响应再算一遍（累计虚高），也不能在追加时把它抹掉
            // （累计凭空少一笔）。回折（把折掉的重新摊开）设计上不做——那要重读整份文件，
            // 而 40 天前的响应本来就不在任何分析窗口里。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            let file = dir.appendingPathComponent("r.jsonl")
            try (Self.codexLine("a", 60, input: 240, cached: 0, output: 0) + "\n")
                .write(toFile: file.path, atomically: true, encoding: .utf8)
            func totals(_ at: Date) -> String {
                let snap = index.snapshot(now: at)
                let usage = snap.usage(now: at)
                return "n=\(snap.records.count) rolled=\(snap.rolledUpTokens["codex"] ?? 0)"
                    + " total=\(usage["codex"]?.tokensTotal ?? -1)"
            }
            let t0 = Self.structuredClock
            try expectEqual(totals(t0), "n=1 rolled=0 total=240", "前置：窗口内")
            // 时钟往前跳 41 天 ⇒ 整份折掉
            try expectEqual(totals(t0.addingTimeInterval(71 * 86_400)), "n=0 rolled=240 total=240",
                            "往前跳：折成合计，累计不变")
            // 再跳回来 ⇒ 不重复累加，也不因为「又回到窗口内」就把合计抹掉
            try expectEqual(totals(t0), "n=0 rolled=240 total=240", "回拨：保持折入，数字不动")
            // 此时追加一条新响应：带折入的文件必须整份重读，两条各算一次
            let handle = try FileHandle(forWritingTo: file)
            try handle.seekToEnd()
            try handle.write(contentsOf: Data(
                (Self.codexLine("b", 60 - 41 * 24 * 60, input: 100, cached: 0, output: 0)
                    + "\n").utf8))  // 相对 t0 往前负 41 天 ⇒ 落在 at 前 1 小时
            try handle.close()
            let at = t0.addingTimeInterval(71 * 86_400)
            try expectEqual(totals(at), "n=1 rolled=240 total=340",
                            "追加后：新响应进明细，旧账不重复")
        }

        TestKit.test("结构化Token索引: 采集根读不了时不许报「已发现明细源」") {
            // 根目录在、整棵读不了（权限/卷没挂全）时，此前照样置真 ⇒ 分析页显示
            // 「有明细源 · 0 token」，把「没看到」讲成「真的没用量」。
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            let logs = root.appendingPathComponent("codex")
            try FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
            defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755],
                                                           ofItemAtPath: logs.path)
                    try? FileManager.default.removeItem(at: root) }
            try (Self.codexLine("r1", 60, input: 100, cached: 0, output: 0) + "\n")
                .write(to: logs.appendingPathComponent("rollout.jsonl"), atomically: true, encoding: .utf8)
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [logs.path], format: .codex)
            ])
            let ok = index.snapshot(now: Self.structuredClock)
            try expectTrue(ok.availableToolIds.contains("codex"), "前置：可读时算已发现")
            try expectEqual(ok.records.count, 1, "前置：明细读得到")

            try FileManager.default.setAttributes([.posixPermissions: 0o100], ofItemAtPath: logs.path)
            let blind = index.snapshot(now: Self.structuredClock)
            try expectFalse(blind.availableToolIds.contains("codex"),
                            "整棵读不了就不能报「已发现明细源」")
            try expectTrue(blind.records.isEmpty, "读不到就是没有明细，不能拿旧值充数")

            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: logs.path)
            let healed = index.snapshot(now: Self.structuredClock)
            try expectTrue(healed.availableToolIds.contains("codex"), "恢复后必须重新算已发现")
            try expectEqual(healed.records.count, 1, "恢复后明细回来")
        }

        TestKit.test("结构化Token索引: 保留窗口必须盖住最宽分析档的「上一周期」") {
            // 分析页 30 天档的对比项要往前读 2×30 = 60 天（`TokenTimelineBuilder` 的
            // previousStart）。窗口只要短于此，JSONL 工具在「上一周期」里就被静默少算：
            // 图表看不出来（最宽只画 30 天），但那行「较上一周期 ±N%」会跟着错。
            let widest = TokenTimeRange.allCases.map(\.duration).max() ?? 0
            try expectTrue(StructuredTokenUsageIndex.detailRetention >= 2 * widest,
                            "保留窗口必须 ≥ 2×最宽分析档（SQLite 源往前读这么远）")
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let index = StructuredTokenUsageIndex(sources: [
                StructuredTokenSource(agentId: "codex", roots: [dir.path], format: .codex)
            ])
            // 55 天前：图表看不见它，但 30 天档的上一周期要它
            try (Self.codexLine("prev", 55 * 24 * 60, input: 240, cached: 0, output: 0) + "\n")
                .write(toFile: dir.appendingPathComponent("r.jsonl").path, atomically: true, encoding: .utf8)
            let snap = index.snapshot(now: Self.structuredClock)
            try expectEqual(snap.records.count, 1, "55 天前的响应必须还在明细里")
            try expectEqual(snap.rolledUpTokens["codex"] ?? 0, 0, "它不该被折进合计")
        }

        TestKit.test("ReadonlyDB: 同路径被外部替换（新 inode）当拍即弃用旧连接") {
            // 连接身份比对已从 attributesOfItem 换成单次 stat(2)，这条钉住其语义：
            // 库被删除重建（备份恢复、VACUUM 后 rename）后，旧句柄指向的 inode 已失效。
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let target = dir.appendingPathComponent("state.db").path
            try TokenFixture.exec(target, ["CREATE TABLE t (v TEXT)", "INSERT INTO t VALUES ('第一份')"])
            try expectEqual(Self.readFirstValueViaCache(target), "第一份")
            try expectEqual(Self.readFirstValueViaCache(target), "第一份", "第二次走缓存连接，结果必须一致")
            // 删掉重建同名文件 → 新 inode，缓存连接必须作废
            try FileManager.default.removeItem(atPath: target)
            try TokenFixture.exec(target, ["CREATE TABLE t (v TEXT)", "INSERT INTO t VALUES ('第二份')"])
            try expectEqual(Self.readFirstValueViaCache(target), "第二份", "同路径换 inode 后不得再吃旧连接")
        }

        TestKit.test("Token 多方言库: 两个同构库各算各的，缺失判定也各自独立") {
            // OpenCode 方言不止 OpenCode：小米 MiMo Code 用同一套 message 表。
            // 单源时代的写法（一个 openCodeDB 字段 + 一份戳）接第二个产品时，
            // 要么把 MiMo 的量挂到 opencode 名下，要么让两个源共用「源已消失」计数
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let oc = dir.appendingPathComponent("opencode.db").path
            let mimo = dir.appendingPathComponent("mimocode.db").path
            @MainActor func insert(_ path: String, tokens: Int, cost: Double) throws {
                try TokenFixture.exec(path, [
                    "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER)",
                    "INSERT INTO message VALUES ('s1', '{\"role\":\"assistant\",\"tokens\":{\"input\":\(tokens),\"output\":0,\"reasoning\":0},\"cost\":\(cost)}', \(TokenFixture.ms(now.addingTimeInterval(-60))))"
                ])
            }
            try insert(oc, tokens: 100, cost: 0.5)
            try insert(mimo, tokens: 42_900, cost: 0)

            let m = TokenUsageMonitor(dimAgentDB: dir.appendingPathComponent("none.db").path,
                                      openCodeSources: [(agentId: "opencode", path: oc),
                                                        (agentId: "mimocode", path: mimo)],
                                      structuredSources: [])
            m.refresh(now: now)
            try expectEqual(m.usage["opencode"]?.tokens24h, 100, "第一个源")
            try expectEqual(m.usage["mimocode"]?.tokens24h, 42_900, "第二个源要挂在自己的 id 下")
            try expectEqual(m.grandTotal.tokens24h, 43_000, "汇总等于两源之和")

            // 删掉第二个库并连刷 sourceMissingLimit 拍：只允许清掉它自己
            try FileManager.default.removeItem(atPath: mimo)
            for offset in stride(from: 100.0, through: 400.0, by: 100.0) {
                m.refresh(now: now.addingTimeInterval(offset))
            }
            try expectEqual(m.usage["mimocode"], nil, "消失的源要清空，否则面板永久显示陈旧数字")
            try expectEqual(m.usage["opencode"]?.tokens24h, 100, "另一个源不受影响")
        }

        // MARK: 等待辅助：主线程轮询 RunLoop（避免信号量死锁 MainActor）
    }

    /// 结构化快照的可比签名：记录逐条（顺序敏感）+ 两口径用量 + 活性集合。
    /// 备忘录若吞掉任何一次变化，这里就会不等。
    @MainActor
    private static func structuredSignature(_ snap: StructuredTokenUsageSnapshot) -> String {
        let body = snap.records.map {
            "\($0.agentId)|\(String(format: "%.3f", $0.time.timeIntervalSince1970))|\($0.tokens)|\(String(format: "%.6f", $0.cost))"
        }.joined(separator: ";")
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let usage = snap.usage(now: now)
        let tail = usage.keys.sorted().map { key -> String in
            let value = usage[key]!
            return "u:\(key)=\(value.tokensTotal)/\(value.tokens24h)/\(String(format: "%.6f", value.costTotal))/\(String(format: "%.6f", value.cost24h))"
        }.joined(separator: ";")
        return "tools=\(snap.availableToolIds.sorted()) n=\(snap.records.count) [\(body)] [\(tail)]"
    }

    /// 结构化夹具的基准时钟：`codexLine` 的时间戳都从它往前推。索引的保留窗口按 `now`
    /// 定位，所以直连 `snapshot` 的用例必须显式带上它，否则明细会被当成 40 天前的历史折掉。
    @MainActor
    static let structuredClock = Date(timeIntervalSince1970: 1_700_000_000)

    /// Codex rollout 的一行 `token_usage_record`（基准时间固定，签名才可跨次比对）
    @MainActor
    private static func codexLine(_ responseId: String, _ minutes: Int,
                                  input: Int = 100, cached: Int = 60, output: Int = 10) -> String {
        let stamp = TokenFixture.iso(Date(timeIntervalSince1970: 1_700_000_000 - Double(minutes) * 60))
        return "{\"timestamp\":\"\(stamp)\",\"type\":\"token_usage_record\",\"payload\":{\"response_id\":\"\(responseId)\",\"usage\":{\"input_tokens\":\(input),\"cached_input_tokens\":\(cached),\"output_tokens\":\(output)}}}"
    }

    /// 与 TokenFixture.iso 同格式（含毫秒 UTC）；放在本文件的静态位是为了让嵌套 helper
    /// 能在 MainActor 上下文里直接调用
    @MainActor
    private static func testISO(_ date: Date) -> String {
        TokenFixture.iso(date)
    }

    /// 经 ReadonlyDB 的缓存连接读一个标量。返回值统一压成非可选字符串，
    /// 「无连接 / prepare 失败 / 无行」各有专名，免得双层可选把类型推断绕死
    @MainActor
    private static func readFirstValueViaCache(_ path: String) -> String {
        return ReadonlyDB.withConnection(path) { db -> String in
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, "SELECT v FROM t", -1, &stmt, nil) == SQLITE_OK, let stmt else {
                return "<prepare 失败>"
            }
            defer { sqlite3_finalize(stmt) }
            guard sqlite3_step(stmt) == SQLITE_ROW, let text = sqlite3_column_text(stmt, 0) else {
                return "<无行>"
            }
            return String(cString: text)
        } ?? "<无连接>"
    }

    /// 在临时库上执行查询并取指定列（流水窗口 SQL 的语义验证）
    @MainActor
    private static func queryColumn(_ dbPath: String, sql: String, column: Int32) throws -> [String] {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let db else {
            throw TestError(message: "打开 fixture 失败")
        }
        defer { sqlite3_close(db) }
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
            throw TestError(message: "prepare 失败: \(sql)")
        }
        defer { sqlite3_finalize(stmt) }
        var rows: [String] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            rows.append(sqlite3_column_text(stmt, column).map { String(cString: $0) } ?? "")
        }
        return rows

    }

    /// 进程常驻内存（MB）：用于泄漏用例的前后对比
    @MainActor
    private static func residentMemoryMB() -> Double {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) { ptr in
            ptr.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        return kr == KERN_SUCCESS ? Double(info.resident_size) / 1_048_576 : -1
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

    static func exec(_ path: String, _ statements: [String]) throws {
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
