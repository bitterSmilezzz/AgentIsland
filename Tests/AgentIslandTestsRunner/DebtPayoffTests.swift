import Foundation
@testable import AgentIslandCore

// MARK: - v0.0.97 记档但没修的债，这一轮的回归测试
//
// 每条都对应 CHANGELOG 里「这一轮明确没做的」的一项。测试名里带上是哪一项，
// 这样以后再有人问「这条债还欠着吗」，grep 测试名就能回答。

enum DebtPayoffTests {
    @MainActor
    static func register() {
        // MARK: 债 4：dailyTokenBudget 是唯一没有区间的数值设置（且溢出是崩溃）

        TestKit.test("预算: --budget 的解析与钳制不再能把进程崩掉") {
            // 实测旧行为：`agentisland tokens --budget 9000000000000000000` 以 SIGTRAP(133)
            // 退出且零输出——旧检查只挡「> Double(Int.max)」，而这种值过了检查
            // 再往下 `dailyBudget * 当月天数` 就溢出
            try expectEqual(DailyBudget.parseArgument("500k"), 500_000)
            try expectEqual(DailyBudget.parseArgument("2m"), 2_000_000)
            try expectEqual(DailyBudget.parseArgument("1000000"), 1_000_000)
            try expectEqual(DailyBudget.parseArgument("9000000000000000000"),
                            SettingLimits.dailyTokenBudgetRange.upperBound, "越界要钳制而不是崩溃或拒绝")
            try expectEqual(DailyBudget.parseArgument("1e308"),
                            SettingLimits.dailyTokenBudgetRange.upperBound)
            try expectNil(DailyBudget.parseArgument("-5"), "负数没有意义，拒绝")
            try expectNil(DailyBudget.parseArgument("abc"))
            try expectNil(DailyBudget.parseArgument(""))
            try expectNil(DailyBudget.parseArgument("nan"))
            try expectNil(DailyBudget.parseArgument("inf"))
            // 0 是合法值（表示不设预算），不能被当成「没解析出来」
            try expectEqual(DailyBudget.parseArgument("0"), 0)
        }

        TestKit.test("预算: 三处读取点统一走钳制，脏持久化值进不了算术") {
            let defaults = TestDefaults.suite("budget")
            defaults.set(9_000_000_000_000, forKey: SettingKey.dailyTokenBudget)
            try expectEqual(DailyBudget.read(defaults: defaults),
                            SettingLimits.dailyTokenBudgetRange.upperBound,
                            "defaults write 写进来的越界值必须钳住")
            defaults.set(-7, forKey: SettingKey.dailyTokenBudget)
            try expectEqual(DailyBudget.read(defaults: defaults), 0, "负值落到下限而不是参与比较")
            defaults.removeObject(forKey: SettingKey.dailyTokenBudget)
            try expectEqual(DailyBudget.read(defaults: defaults), 0, "没设过等于不设预算")
            // 月度预估里两个乘法都必须饱和：Int.max 预算 × 31 天是旧代码的 trap 点
            let report = TokenForecastEvaluator.evaluate(tokens24h: Int.max, cost24h: 1,
                                                         dailyBudget: Int.max,
                                                         now: Date(timeIntervalSince1970: 1_800_000_000))
            try expectEqual(report.projectedMonthEndTokens, Int.max, "溢出钳到上限而不是崩溃")
            try expectFalse(report.forecastSummary.isEmpty, "仍要给出可用的预估文案")
            // 费用那一侧此前是裸的 `cost24h × 天数`：脏 cost 会让「预估月末」比累计还大几个
            // 量级（同一屏上两个本该同量级的数自相矛盾），现在与 token 一样钳在 costCeiling
            let dirty = TokenForecastEvaluator.evaluate(tokens24h: 1_000, cost24h: 1e308,
                                                        dailyBudget: 0,
                                                        now: Date(timeIntervalSince1970: 1_800_000_000))
            try expectEqual(dirty.projectedMonthEndCost, SafeNumber.costCeiling,
                            "预估月末费用必须钳在 costCeiling")
            try expectTrue(dirty.projectedMonthEndCost.isFinite, "绝不能是 Inf")
        }

        TestKit.test("审计报告: 头条数字与面板汇总栏同源，两份口径都摆出来") {
            // 逐条相加只覆盖在册条目：离线但仍有 24h 用量的工具、与宿主合并的内嵌组件
            // 都不在那份列表里。报告与面板对不上，用户怀疑的是面板。
            let profile = AgentRegistry.builtin.first { $0.id == "dim" }!
            let snap = AgentSnapshot(profile: profile, level: .working, processRunning: true,
                                     cpuPercent: 1, installed: true, activeSessions: 1,
                                     lastActivityAgo: nil, lastActivityText: "—",
                                     tokenUsage: TokenUsage(tokens24h: 200, tokensTotal: 900,
                                                            cost24h: 0, costTotal: 0))
            let md = AuditReportExporter.generateMarkdown(
                snapshots: [snap],
                grandTotal: TokenUsage(tokens24h: 5_000, tokensTotal: 9_000, cost24h: 0, costTotal: 0),
                now: Date(timeIntervalSince1970: 1_800_000_000))
            try expectTrue(md.contains(TokenUsage.compact(5_000)), "头条必须是跨源总量 5,000")
            try expectTrue(md.contains(TokenUsage.compact(200)), "逐条之和也要写出来（口径差异可见）")
            try expectTrue(md.contains("离线但仍有用量记录"), "差额来源要说明，不是留个谜")
            // 不给 grandTotal 时行为不变（CLI 之外的老调用方仍可只用列表）
            let legacy = AuditReportExporter.generateMarkdown(
                snapshots: [snap], now: Date(timeIntervalSince1970: 1_800_000_000))
            try expectTrue(legacy.contains(TokenUsage.compact(200)), "缺省回落逐条相加")
            try expectFalse(legacy.contains("离线但仍有用量记录"), "没有差额就不该写口径说明")
        }

        TestKit.test("会话下钻: 取满上限时标题必须说「还有更多未列出」") {
            // 列表页标题写「N 个会话」而查询带 LIMIT：取满 200 时那句「200 个会话」
            // 把「只看了前 200」讲成「一共 200」。没看到不等于没有。
            try expectEqual(TokenUsageMonitor.sessionListSubtitle(count: 3), "3 个会话")
            try expectEqual(TokenUsageMonitor.sessionListSubtitle(count: 199), "199 个会话")
            let full = TokenUsageMonitor.sessionListSubtitle(
                count: TokenUsageMonitor.sessionDrilldownLimit)
            try expectTrue(full.contains("还有更多未列出"), "取满必须承认截断，实际 \(full)")
            // 标题口径与 SQL 里的 LIMIT 必须同一个数：两处各写一遍迟早漂移
            let text = try SourceTree.text(
                relativePath: "Sources/AgentIslandCore/TokenUsageMonitor.swift")
            let bare = text.components(separatedBy: "\n").filter {
                $0.contains("LIMIT 200") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//")
            }
            try expectTrue(bare.isEmpty, "SQL 里的 LIMIT 必须引用 sessionDrilldownLimit，"
                                       + "还留着字面量 200：\(bare)")
        }

        TestKit.test("算术: SafeNumber.costProduct 把脏 cost 钳在上限而不是放大成 Inf") {
            try expectEqual(SafeNumber.costProduct(2.5, 30), 75.0)
            try expectEqual(SafeNumber.costProduct(1e308, 31), SafeNumber.costCeiling,
                            "溢出到 Inf 要钳住，不是把 Inf 交给界面去格式化")
            try expectEqual(SafeNumber.costProduct(Double.infinity, 3), SafeNumber.costCeiling)
            try expectEqual(SafeNumber.costProduct(Double.nan, 3), 0.0, "NaN 落到 0")
            try expectEqual(SafeNumber.costProduct(-5, 3), 0.0, "负数不落进累计")
            try expectEqual(SafeNumber.costProduct(5, -3), 0.0, "负天数按 0 处理")
        }

        TestKit.test("算术: SafeNumber.product 在两端溢出时都不 trap") {
            try expectEqual(SafeNumber.product(3, 7), 21)
            try expectEqual(SafeNumber.product(Int.max, 2), Int.max)
            try expectEqual(SafeNumber.product(Int.max, -2), Int.min)
            try expectEqual(SafeNumber.product(Int.min, -1), Int.max,
                            "Int.min × -1 正是会 trap 的那一个（结果超出 Int.max）")
            try expectEqual(SafeNumber.product(0, Int.min), 0)
        }

        // MARK: 债 5：customAgents 损坏存档被覆写 + 自定义档案字段零校验

        TestKit.test("自定义档案: 写入闸门去掉空 id 与重复 id，超限直接拒写") {
            let defaults = TestDefaults.suite("custom")
            func profile(_ id: String, _ name: String = "n") -> AgentProfile {
                AgentProfile(id: id, name: name, icon: "🤖", bundleIDs: [],
                             processNames: [id.isEmpty ? "x" : id], sessionDirs: [])
            }
            let mixed = [profile("a"), profile("a"), profile(""), profile("  "), profile("b")]
            let result = AgentRegistry.saveCustomProfiles(mixed, defaults: defaults)
            try expectEqual(result, .saved(2), "重复 id 与空/纯空白 id 都要去掉，剩下的照常保存")
            let loaded = AgentRegistry.loadCustomProfiles(defaults: defaults)
            try expectEqual(loaded.map(\.id), ["a", "b"], "顺序按首次出现保留")

            var many: [AgentProfile] = []
            for i in 0...(AgentRegistry.maxCustomAgents) { many.append(profile("id-\(i)")) }
            if case .refused(let reason) = AgentRegistry.saveCustomProfiles(many, defaults: defaults) {
                try expectTrue(reason.contains("上限"), "要说明为什么没写进去：\(reason)")
            } else {
                throw TestError(message: "超过上限却写入成功——静默截断等于丢用户的档案")
            }
            try expectEqual(AgentRegistry.loadCustomProfiles(defaults: defaults).count, 2,
                            "被拒的写入不能碰原存档")
        }

        TestKit.test("自定义档案: 顶层损坏与「没配过」是两个状态，覆写前原件要留证") {
            let defaults = TestDefaults.suite("custom-corrupt")
            // 没这个键
            try expectEqual(AgentRegistry.customArchiveState(defaults: defaults), .absent)
            // 合法空数组不算损坏
            defaults.set(Data("[]".utf8), forKey: SettingKey.customAgents)
            try expectEqual(AgentRegistry.customArchiveState(defaults: defaults), .ok)
            // 顶层不是数组 → 损坏
            defaults.set(Data("not json at all".utf8), forKey: SettingKey.customAgents)
            try expectEqual(AgentRegistry.customArchiveState(defaults: defaults), .corrupt)
            try expectEqual(AgentRegistry.loadCustomProfiles(defaults: defaults).count, 0)
            // 用户此时新增一条：写不可避免，但原件必须留在备份键里
            let one = AgentProfile(id: "newone", name: "新条目", icon: "🤖", bundleIDs: [],
                                   processNames: ["newone"], sessionDirs: [])
            if case .refused(let reason) = AgentRegistry.saveCustomProfiles([one], defaults: defaults) {
                throw TestError(message: "不该拒绝：\(reason)")
            }
            let backup = try XCTRequire(defaults.data(forKey: SettingKey.customAgentsCorruptBackup),
                                        "损坏存档被覆写前没留原件——用户就再也没有恢复路径了")
            try expectEqual(String(decoding: backup, as: UTF8.self), "not json at all")
            try expectEqual(AgentRegistry.loadCustomProfiles(defaults: defaults).map(\.id), ["newone"])
            // 备份键不能被注册表当档案解码
            try expectFalse(AgentRegistry.customArchiveState(defaults: defaults) == .corrupt,
                            "覆写之后状态要恢复正常，否则每次写都留一份新备份")
        }

        TestKit.test("自定义档案: 阈值字段里的 Infinity 不再让开关静默失效") {
            // `1e999` 会被 JSON 解成 Infinity，而 `max(cpu, inf)` 恒为 inf：
            // 症状是「阈值设了却永远不触发」，看起来像没生效而不是像数据坏了
            let bad = #"{"id":"x","name":"X","icon":"🤖","bundleIDs":[],"processNames":["x"],"cpuWorkingThreshold":1e999,"tokenAlertFloor":-9}"#
            let data = Data(bad.utf8)
            let decoded = try? JSONDecoder().decode(AgentProfile.self, from: data)
            let profile = try XCTRequire(decoded, "含异常数值的档案应能解码（数值被丢弃而不是整条失败）")
            try expectNil(profile.cpuWorkingThreshold, "非有限的阈值要当没设，而不是参与 max")
            try expectNil(profile.tokenAlertFloor, "负数 token 下限同样要丢弃")
            let good = #"{"id":"x","name":"X","icon":"🤖","bundleIDs":[],"processNames":["x"],"cpuWorkingThreshold":42.5,"tokenAlertFloor":1000}"#
            let okProfile = try XCTRequire(try? JSONDecoder().decode(AgentProfile.self, from: Data(good.utf8)))
            try expectEqual(okProfile.cpuWorkingThreshold, 42.5, "正常值不该被动过")
            try expectEqual(okProfile.tokenAlertFloor, 1000)
        }

        // MARK: 债 6：resolvedEnabled 在 CLI 侧带写副作用（跨进程丢更新）

        TestKit.test("启停集: 只读求解不写回，写回仍归设置页") {
            let defaults = TestDefaults.suite("readonly")
            let registry = [AgentProfile(id: "a", name: "A", icon: "🤖", bundleIDs: [], processNames: ["a"], sessionDirs: []),
                            AgentProfile(id: "b", name: "B", icon: "🤖", bundleIDs: [], processNames: ["b"], sessionDirs: [])]
            // 首次求解：只读模式给出的集合与非只读一致，但不落盘
            let readOnlyResult = EnabledAgentStore.resolvedEnabled(registry: registry,
                                                                   defaults: defaults, readOnly: true)
            try expectEqual(readOnlyResult, Set(["a", "b"]), "只读也要给出同样的默认启用集")
            try expectNil(defaults.data(forKey: SettingKey.enabledAgents),
                          "一次性进程写回的是它此刻的注册表快照，会覆盖并发的用户开关")
            try expectNil(defaults.data(forKey: SettingKey.knownAgents), "knownAgents 同样不该被写")
            // 非只读（应用侧）保持原行为：会固化
            _ = EnabledAgentStore.resolvedEnabled(registry: registry, defaults: defaults)
            try expectNotNil(defaults.data(forKey: SettingKey.enabledAgents), "设置页仍负责写回")
        }

        // MARK: 设置窗口入口

        TestKit.test("深链: agentisland://settings 认三种写法，且打开动作只有一处实现") {
            for raw in ["agentisland://settings", "agentisland://config", "agentisland://preferences"] {
                let url = try XCTRequire(URL(string: raw))
                try expectEqual(URLSchemeParser.parse(url: url), .settings(tab: nil), raw)
            }
            // `?tab=` 是这一轮加的第二段：没有它，深链只能开窗口，
            // 想核对某个具体页面就只能改源码
            try expectEqual(URLSchemeParser.parse(url: try XCTRequire(URL(string: "agentisland://settings?tab=remote"))),
                            .settings(tab: "remote"))
            try expectEqual(URLSchemeParser.parse(url: try XCTRequire(URL(string: "agentisland://config?pane=engine"))),
                            .settings(tab: "engine"))
            // 认不出的 tab 不是错误：解析层原样交出，由 SettingsSelection 决定「保持当前页」
            try expectEqual(URLSchemeParser.parse(url: try XCTRequire(URL(string: "agentisland://settings?tab=nope"))),
                            .settings(tab: "nope"))
            // 齿轮按钮与深链必须共用 SettingsOpener：两处各写一遍 selector 的话，
            // 系统改过一次名字（showPreferencesWindow: ↔ showSettingsWindow:）
            // 就会出现「一个能开一个不能开」，而这类差异没人会去回归
            let root = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            let app = try XCTRequire(try? String(contentsOf: root
                .appendingPathComponent("Sources/AgentIsland/AgentIslandApp.swift"), encoding: .utf8))
            let router = try XCTRequire(try? String(contentsOf: root
                .appendingPathComponent("Sources/AgentIsland/URLSchemeRouter.swift"), encoding: .utf8))
            for (name, text) in [("AgentIslandApp", app), ("URLSchemeRouter", router)] {
                let callSites = text.components(separatedBy: "\n")
                    .filter { $0.contains("showSettingsWindow") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
                try expectTrue(callSites.count <= 1,
                               "\(name) 里出现多处 selector 调用，应统一走 SettingsOpener：\(callSites)")
            }
            try expectTrue(app.contains("SettingsOpener.open("), "齿轮按钮要经唯一出口")
            try expectTrue(router.contains("SettingsOpener.open("), "深链要经唯一出口")
        }

        // MARK: 债 7：check 与 clean 的注册表不一致（提示了做不到的事）

        TestKit.test("结构: check 与 clean 必须看同一份注册表") {
            // check 列出某个 cli-*/自定义 Agent 的异常并提示「运行 agentisland clean 可一键终止」，
            // 而 clean 只看内置集 → 回答「未发现需要清理的异常进程」。
            // 两个入口口径不一致时，用户按提示做的那一步必然失败
            let dir = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("Sources/AgentIslandCLI/Commands")
            let check = try XCTRequire(try? String(contentsOf: dir.appendingPathComponent("CheckCommand.swift"),
                                                   encoding: .utf8))
            let clean = try XCTRequire(try? String(contentsOf: dir.appendingPathComponent("CleanCommand.swift"),
                                                   encoding: .utf8))
            for (name, text) in [("CheckCommand", check), ("CleanCommand", clean)] {
                try expectTrue(text.contains("LiveSampler.context()"),
                               "\(name) 不再通过 LiveSampler.context() 取注册表，两个入口会重新分叉")
            }
            let builtinLines = clean.components(separatedBy: "\n").enumerated()
                .filter { $0.element.contains("AgentRegistry.builtin")
                    && !$0.element.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
                .map { "\($0.offset + 1): \($0.element.trimmingCharacters(in: .whitespaces))" }
            try expectTrue(builtinLines.isEmpty,
                           "clean 退回内置集就等于对自定义/自动发现的 Agent 免疫：\(builtinLines)")
        }

        TestKit.test("结构: 预算与孤儿闸门不再有裸读或静默绕过") {
            let sources: [(String, String)] = [
                ("ActivityEngine", "Sources/AgentIslandCore/ActivityEngine.swift"),
                ("TokenAnalyticsView", "Sources/AgentIsland/TokenAnalyticsView.swift"),
                ("TokensCommand", "Sources/AgentIslandCLI/Commands/TokensCommand.swift"),
            ]
            let root = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            for (name, rel) in sources {
                let text = try XCTRequire(try? String(contentsOf: root.appendingPathComponent(rel),
                                                      encoding: .utf8),
                                          "读不到 \(rel)，这条断言就成了永真")
                try expectFalse(text.contains("integer(forKey: SettingKey.dailyTokenBudget)"),
                                "\(name) 绕过 DailyBudget.read 裸读预算：越界值会直接进算术")
            }
            // 孤儿的逐条确认闸门不许再被 --force 跨过：force 与非 force 的差别
            // 只能是「批量项问不问」
            let clean = try XCTRequire(try? String(contentsOf: root
                .appendingPathComponent("Sources/AgentIslandCLI/Commands/CleanCommand.swift"), encoding: .utf8),
                "读不到 CleanCommand.swift")
            try expectTrue(clean.contains("--include-orphans"), "孤儿需要独立的显式开关")
            try expectTrue(clean.contains("isatty"), "非交互环境必须拒绝清理孤儿，而不是静默放行")
            try expectFalse(clean.contains("targets = anomalies"),
                            "--force 又变成「连孤儿一起杀」了")
        }

        // MARK: 研究文档的待核实清单：编号成对、状态可判定

        TestKit.test("结构: 研究文档的「待核实」必须编号且可判定") {
            // v0.0.126 的 review 指出：未完成项以散文散在正文里，没编号、没状态、没复核方式，
            // 于是「这条到底核完没有」要通读全文才能判断——重复劳动已经发生过一次
            // （v0.0.125 把有公开正文的四家误判成「排除」）。这条断言把约定钉住：
            // 正文引用的编号 ↔ 清单里的编号一一对应，状态只许三种，
            // 且不许再用笼统的「未证实」把三种不同的「还不知道」混成一个词。
            let text = try SourceTree.text(relativePath: "docs/research/agent-lifecycle-hooks.md")
            let parts = text.components(separatedBy: "## 待核实清单")
            try expectEqual(parts.count, 2, "「## 待核实清单」必须恰好出现一次")
            let body = parts[0], checklist = parts[1]

            // 取 **V#** 里的编号。用切分而不是正则：这条断言守的是文档约定，
            // 值不得因为一个字面量转义写错而整栋编译不过
            func ids(_ s: String) -> Set<String> {
                Set(s.components(separatedBy: "**").filter { tok in
                    guard tok.hasPrefix("V"), tok.count >= 2 else { return false }
                    return tok.dropFirst().allSatisfy { $0.isNumber || $0 == "a" || $0 == "b" }
                })
            }
            let bodyIDs = ids(body), tableIDs = ids(checklist)
            try expectTrue(!tableIDs.isEmpty, "清单不能是空的")
            let noRow = bodyIDs.subtracting(tableIDs)
            try expectTrue(noRow.isEmpty, "正文引用了清单里没有的编号：\(noRow.sorted())")
            let orphan = tableIDs.subtracting(bodyIDs)
            try expectTrue(orphan.isEmpty, "清单里有正文从未引用的编号（等于没人会去核）：\(orphan.sorted())")

            let rows = checklist.split(separator: "\n").filter { $0.hasPrefix("| **V") }
            try expectEqual(rows.count, tableIDs.count, "每个编号恰好一行")
            let allowed = ["未找到出处", "待逐字复核", "待真机实测"]
            for row in rows {
                try expectTrue(allowed.contains(where: { row.contains($0) }),
                               "状态必须是三分类之一（每种都写着怎么往下走）：\(row.prefix(90))")
            }
            try expectTrue(!body.contains("未证实"),
                           "「未证实」这个笼统说法回来了——它把「没找到出处」「没逐字复核」「没实测」混成一件事")
        }

        // MARK: README 的文体：功能说明，不是更新记录

        TestKit.test("结构: README 不许长成更新日志") {
            // README 的分工是「这个工具是什么、能做什么」，逐版记录归 CHANGELOG。
            // 这条不是洁癖：README 一旦开始记流水，就会长出重复小节与过期下载链接
            // （曾经真长出过 5 组重复标题），而读者拿它当现状说明书。
            let readme = try XCTRequire(try? String(contentsOf: SourceTree.repoRoot
                .appendingPathComponent("README.md"), encoding: .utf8), "读不到 README.md")
            let lines = readme.components(separatedBy: "\n")
            var offenses: [String] = []
            for (idx, raw) in lines.enumerated() {
                let line = raw.trimmingCharacters(in: .whitespaces)
                guard !line.isEmpty, !line.hasPrefix("#"), !line.hasPrefix("```") else { continue }
                // 版本行是唯一被允许的显式版本引用（build-app.sh 靠它校验文档跟没跟上）
                if line.contains("本文档描述") { continue }
                // 交付链路那一节里的示例命令与产物名，不是逐版记录
                if line.contains("scripts/release.sh") || line.contains("AgentIsland-") { continue }
                if line.range(of: #"v0\.0\.\d+"#, options: .regularExpression) != nil {
                    offenses.append("\(idx + 1): 出现具体版本号 → \(line.prefix(50))")
                }
                // 只收「回归叙事」的固定搭配。单个「这次」不列：中文里它更常指
                // 「本次请求/这一轮」，把正常句子判成跑题的断言活不过第二周
                for marker in ["此前", "原先", "曾经", "上一版", "本次改造", "不再", "新增了"] {
                    if line.contains(marker) {
                        offenses.append("\(idx + 1): 回归叙事「\(marker)」→ \(line.prefix(50))")
                    }
                }
                if line.contains("实测") {
                    offenses.append("\(idx + 1): 带着测量数字的解释属于 CHANGELOG → \(line.prefix(50))")
                }
            }
            try expectTrue(offenses.isEmpty,
                           "README 里出现逐版叙事（读者没法分辨哪句还作数）：\n    "
                            + offenses.joined(separator: "\n    "))
        }
    }
}
