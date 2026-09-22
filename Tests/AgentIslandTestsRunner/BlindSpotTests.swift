import Foundation
import Darwin
import SQLite3
@testable import AgentIslandCore

// MARK: - 盲区补齐（第三轮测试有效性审计报回、复核确认为真的缺口）
//
// 这些用例的存在理由不是「多几条测试」，而是审计给出的**具体变异**：
// 每条都写明「把生产的哪一行改坏，本用例必须变红」。改完逐条验证过。

enum BlindSpotTests {
    @MainActor
    static func register() {
        // MARK: 1. 第三方 JSON 的类型漂移

        TestKit.test("SafeNumber.jsonInt: Int/Double/数字字符串都认，脏值不 trap 也不静默归零") {
            try expectEqual(SafeNumber.jsonInt(3), 3)
            try expectEqual(SafeNumber.jsonInt(3.0), 3)
            try expectEqual(SafeNumber.jsonInt("3"), 3)
            try expectEqual(SafeNumber.jsonInt(" 4 "), 4)
            try expectEqual(SafeNumber.jsonInt(1e30), SafeNumber.magnitudeCeiling, "越界必须饱和而非 trap")
            try expectNil(SafeNumber.jsonInt(nil))
            try expectNil(SafeNumber.jsonInt(Double.nan))
            try expectNil(SafeNumber.jsonInt("step-7"), "非数字字符串不该编造一个值")
            try expectNil(SafeNumber.jsonInt([1, 2]))
        }

        TestKit.test("Antigravity: step_index 写成字符串或浮点，指纹不得退化成 step-0") {
            // 变异验证：把生产的 `SafeNumber.jsonInt(obj["step_index"])` 改回 `as? Int ?? 0`，
            // 本用例必须变红（此前所有夹具都是整数，所以这个缺口零覆盖）
            let toolLine = #"{"step_index":"101","source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"toolAction":"Running swift build","CommandLine":"swift build"}}]}"#
            let userLine = #"{"step_index":100,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"build"}"#
            guard case let .active(fingerprint, _)? =
                    AgentSessionInspector.detectAntigravitySession(lines: [userLine, toolLine], fileAge: 10) else {
                throw TestError(message: "字符串型 step_index 应仍识别为活跃工具调用")
            }
            try expectEqual(fingerprint, "antigravity-step-101",
                            "step_index 漂移成字符串后指纹退化，进度与「已回答」比较全部失效")

            let floatLine = toolLine.replacingOccurrences(of: "\"101\"", with: "101.0")
            if case let .active(fp2, _)? =
                    AgentSessionInspector.detectAntigravitySession(lines: [userLine, floatLine], fileAge: 10) {
                try expectEqual(fp2, "antigravity-step-101", "浮点型 step_index 同样不得退化")
            } else {
                throw TestError(message: "浮点型 step_index 也应识别为活跃工具调用")
            }
        }

        // 注：审计还断言「parsedMeta 里的 step_index 漂移会让已答复的提问误报 attention」。
        // 复核时把该站点单独退回旧写法，整套测试全绿——这条因果在本机夹具上不成立
        // （该值只参与一次比较，漂移后走的分支与不漂移时相同），因此没有为它写用例。
        // jsonInt 的容错照留：它让 :914 与 :1047 两个站点口径一致，不再有半边严格半边宽容。

        // MARK: 2. ReadonlyDB 的「已缓存连接随后文件消失」分支

        TestKit.test("ReadonlyDB: 连接已缓存、文件随后被删 → 上报 .missing（不是静默 nil）") {
            // 变异验证：删掉 cached 分支里的 `onFailure(.missing)`，本用例必须变红。
            // 此前只有「从未打开过」那一支被测到，两条码长得像但只测了一条
            let path = FileManager.default.temporaryDirectory
                .appendingPathComponent("aibleak-\(UUID().uuidString).db").path
            var db: OpaquePointer?
            guard sqlite3_open(path, &db) == SQLITE_OK, let db else {
                throw TestError(message: "夹具库创建失败")
            }
            sqlite3_exec(db, "CREATE TABLE t(x);", nil, nil, nil)
            sqlite3_close(db)

            var warmed = false
            _ = ReadonlyDB.withConnection(path) { _ in warmed = true }
            try expectTrue(warmed, "前置：先建立缓存连接，否则走的是另一条分支")

            try FileManager.default.removeItem(atPath: path)
            var failure: ReadonlyDB.ConnectionFailure?
            let result = ReadonlyDB.withConnection(path, onFailure: { failure = $0 }) { _ in 1 }
            try expectNil(result, "文件已删除不得返回结果")
            try expectEqual(failure, .missing,
                            "已缓存连接遇到文件消失也要说清是 .missing，否则「读不到」与「没数据」同形")
        }

        // MARK: 3. SQL 转义穿过真实查询

        TestKit.test("SQL 转义: 含单引号的 modelId 必须查得到，而不是静默空结果") {
            // 变异验证：去掉 TokenUsageMonitor 两处 `modelId.escaped`，本用例必须变红
            let dbs = try TokenFixture.make()
            defer { TokenFixture.cleanup(dbs) }
            try TokenFixture.exec(dbs.dimDB, [
                #"INSERT INTO usage_ledger VALUES ('"# + TokenFixture.iso(Date()) +
                    #"', 'it''s-a-model', '{"promptTokens":40,"completionTokens":10}', 0.20, 'sess-q')"#
            ])
            let m = TokenUsageMonitor(dimAgentDB: dbs.dimDB, openCodeDB: dbs.openCodeDB)
            var found: [SessionUsage]?
            let deadline = Date().addingTimeInterval(10)
            m.sessions(agentId: "dim", modelId: "it's-a-model") { rows in found = rows }
            while found == nil && Date() < deadline {
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            }
            guard let rows = found else { throw TestError(message: "会话查询超时") }
            try expectEqual(rows.first?.sessionId, "sess-q",
                            "modelId 里的单引号没转义时 SQL 语法错，rawRows 会静默返回空")
            try expectEqual(rows.first?.tokens, 50, "净 token 口径")
        }

        // MARK: 4. 结构不变式：字符串进 SQL 必须过 .escaped

        TestKit.test("结构: 被引号包住插值进 SQL 的表达式一律先过 .escaped") {
            // `'\(...)'` 形态就是「把一段文本当成 SQL 里的字符串字面量」，
            // 少一次 .escaped 就是注入面或语法崩面；内部生成的 ISO 时间戳也一并要求
            // （对无引号的值是恒等操作，换来的是这条规则不需要例外清单）
            var offenders: [String] = []
            for (name, text) in try SourceTree.requireSourceTexts() {
                for (index, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
                    let trimmed = line.trimmingCharacters(in: .whitespaces)
                    if trimmed.hasPrefix("//") { continue }
                    guard trimmed.contains("'\\(") else { continue }
                    if !trimmed.contains(".escaped") {
                        offenders.append("\(name):\(index + 1) \(trimmed.prefix(72))")
                    }
                }
            }
            try expectTrue(offenders.isEmpty,
                           "有字符串被裸插值进 SQL 的引号里（未过 String.escaped）："
                           + offenders.joined(separator: " ; "))
        }

        // MARK: 5. doctor / 工作台卡片实际打印的结论文案

        TestKit.test("可观测性: 五类结论的 summary 文案逐字锁定") {
            // 变异验证：对调 .blindSessionSource 与 .noLocalData 的返回串，此前全绿
            let pairs: [(AgentObservability.Code, String)] = [
                (.observed, "结论可信"),
                (.blindSessionSource, "待机不可信：会话源读不到"),
                (.noLocalData, "无本地明细：读不到会话与用量"),
                (.sourceNotWired, "未接入明细源"),
                (.notInstalled, "未安装：不该期待状态"),
            ]
            try expectEqual(pairs.count, AgentObservability.Code.allCases.count, "新增结论码必须同时补文案断言")
            for (code, expected) in pairs {
                let verdict = AgentObservability.Verdict(code: code, evidence: [])
                try expectEqual(verdict.summary, expected, "\(code.rawValue) 文案漂移")
            }
        }

        // MARK: 6. 视图层的重复观察与重复筛选（结构棘轮）

        TestKit.test("结构: 视图不得无谓观察整个引擎，也不得在渲染里重复同一份筛选") {
            // AgentRowView / AgentHoverTooltip 的 body 不读 engine 任何 @Published 状态，
            // 观察它等于每拍让整行重新求值一次（正落在展开弹簧期）
            let all = try SourceTree.requireSourceTexts()
            var observed = 0
            for (_, text) in all where text.contains("@ObservedObject var engine: ActivityEngine") {
                observed += 1
            }
            try expectTrue(observed <= 9,
                           "观察整个引擎的文件从 9 个涨到 \(observed) 个：新视图请只观察它真正读取的状态")

            // 直接按路径读：清单里「没扫到这个文件」曾经是这条断言静默通过的形态
            let stream = try SourceTree.text(relativePath: "Sources/AgentIsland/LiveLogStreamView.swift")
            let refilters = stream.components(separatedBy: "events.filter {").count - 1
            // 合法的两处：① 当前筛选的列表（body 顶部 hoist 成一次）；
            // ② counts(for:) 里一次算齐所有口径。再多的就是「每个 chip 各扫一遍」回潮
            try expectTrue(refilters <= 2,
                           "流水页又回到「每个 chip 各 filter+count 一遍」（实得 \(refilters) 处，基线 2）："
                           + "条数应在刷新回调里一次算好存进 countsByFilter")
        }

        // MARK: 7. 深链解析（v0.0.96 之前零覆盖：解析待在 @MainActor 的 UI 目标里，runner 够不着）

        TestKit.test("深链解析: 别名、host/path 两种形态、查询键大小写与重复键") {
            func p(_ s: String) -> URLSchemeCommand? {
                URLSchemeParser.parse(url: URL(string: s)!)
            }
            try expectEqual(p("agentisland://toggle"), .toggle)
            try expectNil(p("https://example.com/toggle"), "非本 scheme 必须拒掉")
            try expectEqual(p("agentisland:///collapse"), .collapse, "path 形态的命令名")
            try expectEqual(p("agentisland://close"), .collapse)
            try expectEqual(p("agentisland://hide"), .collapse)
            try expectEqual(p("agentisland://tokens"), .analytics)
            try expectEqual(p("agentisland://workbench"), .toolbox)
            try expectEqual(p("agentisland://kill-orphans"), .clean)
            try expectEqual(p("agentisland://report"), .export)
            try expectEqual(p("agentisland://whatever"), .unknown("whatever"))
            // expand?id=claude 等价于 agent?id=claude
            try expectEqual(p("agentisland://expand?id=claude"), .agent(id: "claude"))
            try expectEqual(p("agentisland://agent/claude"), .agent(id: "claude"), "路径段兜底取 id")
            // 查询键大小写不敏感、重复键 first-wins（与 URLComponents 的顺序一致）
            try expectEqual(p("agentisland://agent?ID=dim"), .agent(id: "dim"))
            if case .agent(let id)? = p("agentisland://agent?id=first&id=second") {
                try expectEqual(id, "first", "重复键必须 first-wins，不能后值覆盖")
            } else { throw TestError(message: "重复键解析结果非 agent") }
            // notify 的默认值与别名
            if case let .notify(agent, type, msg, detail) = p("agentisland://notify?agent=dim&MSG=hi")! {
                try expectEqual(agent, "dim")
                try expectEqual(type, "completed", "缺 type 时默认 completed")
                try expectEqual(msg, "hi", "msg 是 message 的别名且大小写不敏感")
                try expectNil(detail)
            } else { throw TestError(message: "notify 解析失败") }
            // %0A 必须解成真换行——报告表格若不过 cell() 就会被它撑出新行
            if case let .notify(_, _, msg, _) = p("agentisland://notify?agent=dim&message=a%0Ab")! {
                try expectEqual(msg, "a\nb", "percent 解码语义")
            } else { throw TestError(message: "换行样本解析失败") }
        }

        TestKit.test("深链来源: 外部投递的告警不得登记保护期") {
            // 变异验证：把 publish() 里的 `&& !event.externallyDelivered` 去掉，
            // 60 条循环链接就能把真实告警的横幅与系统通知双双压掉
            let engine = EngineTests.makeEngine(processNames: ["DimAgent"], writes: [:])
            let external = AgentTaskEvent(agentId: "dim", agentName: "DimAgent", eventType: .costSpike,
                                          duration: 0, message: "占位", externallyDelivered: true)
            engine.postEvent(external)
            let real = AgentTaskEvent(agentId: "dim", agentName: "DimAgent", eventType: .completed,
                                      duration: 12, message: "真完成")
            engine.postEvent(real)
            try expectEqual(engine.latestEvent?.message, "真完成",
                            "外部 costSpike 抢了保护期，真实事件再也进不了展示位")
            try expectTrue(engine.latestEvent?.externallyDelivered == false, "展示位应换成真实事件")
        }

        TestKit.test("审计报告: 表格单元必须挡住换行与竖线伪造") {
            // 变异验证：把 cell() 改成原样返回，攻击者可用 message=a%0A%7C伪造行%7C 自造报告行
            let forged = AuditReportExporter.cell("x\n| 伪造告警 | y |\n| 小节 |")
            try expectFalse(forged.contains("\n"), "换行没被折叠，报告里会出现攻击者自造的行")
            try expectEqual(forged, "x \\| 伪造告警 \\| y \\| \\| 小节 \\|")
        }

        TestKit.test("设置: 布尔开关默认值只有一套口径（UI 默认 true 的键不得裸读 bool(forKey:)）") {
            let suite = TestDefaults.suite("bool-default")
            try expectEqual(SettingBool.read("absentTrue", default: true, defaults: suite), true,
                            "缺键必须回落到声明的默认，而不是 UserDefaults 的 false")
            try expectEqual(SettingBool.read("absentFalse", default: false, defaults: suite), false)
            suite.set(false, forKey: "explicitOff")
            try expectEqual(SettingBool.read("explicitOff", default: true, defaults: suite), false,
                            "用户显式关掉必须生效，不能被默认值吃掉")

            // 通用规则（不写死键名）：凡 @AppStorage 里默认 true 的 SettingKey，
            // 任何地方都不准再用 bool(forKey:) 裸读——那会把「没拨过开关」读成「用户关了」
            var trueDefaults = Set<String>()
            for (_, text) in try SourceTree.requireSourceTexts() {
                for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
                    let s = String(line)
                    guard s.contains("@AppStorage(SettingKey."), s.hasSuffix("= true") else { continue }
                    if let key = SettingKey.name(in: s) { trueDefaults.insert(key) }
                }
            }
            try expectTrue(trueDefaults.contains("budgetAlertEnabled"),
                           "夹具前提变了：@AppStorage 默认 true 的键集合里应有 budgetAlertEnabled")
            var offenders: [String] = []
            for (name, text) in try SourceTree.requireSourceTexts() {
                for (i, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
                    let s = String(line)
                    if s.trimmingCharacters(in: .whitespaces).hasPrefix("//") { continue }
                    guard s.contains(".bool(forKey: SettingKey.") else { continue }
                    if let key = SettingKey.name(in: s), trueDefaults.contains(key) {
                        offenders.append("\(name):\(i + 1) \(key)")
                    }
                }
            }
            try expectTrue(offenders.isEmpty,
                           "UI 默认 true 的开关被 bool(forKey:) 裸读（缺键读成 false，功能对新用户永久失效）："
                           + offenders.joined(separator: "、") + "；请改用 SettingBool.read(_:default:)")
        }

        TestKit.test("清理复核: 按 pid 探活判定存活，路径不符时不得算同一个进程") {
            // 变异验证：把 isAlive 改成恒返回 false（等于回到「列表空了就算清理成功」），
            // 第一条断言立刻变红
            try expectTrue(ProcessTerminator.isAlive(pid: Int32(getpid())),
                           "本进程自己的 pid 必须探到存活")
            try expectFalse(ProcessTerminator.isAlive(pid: 0), "pid 0 不是可清理目标")
            try expectFalse(ProcessTerminator.isAlive(pid: -1), "非法 pid 一律不存活")
            // pid 被复用给别的程序时，带路径的复核必须判「不是同一个进程」
            try expectFalse(ProcessTerminator.isAlive(pid: Int32(getpid()),
                                                      expectedPath: "/applications/some-other.app/contents/macos/other"),
                            "路径不符即身份不符，不能算作同一目标存活")
            // 已退出的子进程不得再被算作存活
            let child = Process()
            child.executableURL = URL(fileURLWithPath: "/usr/bin/true")
            try child.run()
            child.waitUntilExit()
            let deadPid = child.processIdentifier
            let deadline = Date().addingTimeInterval(2)
            while ProcessTerminator.isAlive(pid: deadPid) && Date() < deadline {
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            }
            try expectFalse(ProcessTerminator.isAlive(pid: deadPid), "已退出的子进程仍被判存活")
        }

    }
}
