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
            // 这条断言守的是「未完成项必须有编号、状态与复核方式」，起因见 v0.0.126/127 两份 review。
            // 它自己被审出来过五个静默失效面，每一个都比上一条更隐蔽，所以形状是钉出来的：
            // ① 编号识别有**行形下界**——清单每个数据行的编号格必须严格写成 `**V<数字>[单个字母]**`，
            //    写成 `v4`、裸 `V9`、少一个空格都会红。只放开字符集是不够的：字符集再宽一格，
            //    「两侧同时隐身」的触发条件只是从「用了 c」换成「用了小写」，而后者在 markdown
            //    表格里更容易发生（从别的表复制一行、重排列宽，样式就掉了）。
            // ② 列索引一律从表头推。上一版的定位条件是「表头以 `| 编号 |` 开头」，那等于把
            //    「编号必须是第一列」写死，于是 `cells[1]` 与 `idx("编号")` 恒等——注释说
            //    「插一列会误诊」实测**不成立**（旧代码先红在定位条件，报的还是正确那句）。
            //    本轮改成认「含编号与状态两列」的表头：列序不再有意义，注释那句话第一次变成真话；
            //    四列齐不齐是下一步的事——少一列报「缺哪一列」，不和「根本没建表」混成同一条消息。
            // ③ 状态词的合法区 = 表格数据行 ∪ **对应那一类词**的显式声明块。原先靠「标题之后」
            //    划豁免区（标题之后任何小节都是免检区），改成声明块之后又漏在「块里写什么、写多长
            //    都不查」——那是一把万能钥匙，本轮给块本身加了下界（review P1-1）。
            // ④ 判定说法整篇扫，不按行也不按段：行级能被「挪到下一行」绕过（v0.0.128 review），
            //    段级能被「隔一个空行」绕过（v0.0.129 review P1-5）——单位换成文档，两类藏身处一起没。
            //    代价是名单必须窄：只有「对我们自己核没核」下结论的词进名单，
            //    「该页未列 hooks」这种对**别家正文**的陈述是发现不是定性，另档处理。
            // ⑤ 覆盖面是整个 `docs/research/` 而不是一份文件（`.scratch/audit-42`）：
            //    原先硬编码 `agent-lifecycle-hooks.md`，换一份研究文档用散文写未完成项不会有任何东西红
            //    ——而「散文的未完成项」正是 v0.0.125/126 付过学费的那个坑。
            // 只校验**确实欠着东西**的文档，不逼每份研究文档都建一张表：
            // 为了断言而扭曲内容，v0.0.127 的 review 已经批评过一次。
            // 触发原先只有一个出口 `contains("待核实")`，而那恰好是清单的列名——把列名改成
            // 「未决项」整份文档就静默进 skipped（review P1-6）。现在四个出口任一即校验。
            // 「本文档暂无待核实项」这种**否定陈述**不算欠项：把它判红的话，最省事的修法
            // 恰恰是把触发词删掉，而那才是让守卫对该文档永久失效的动作——激励方向必须对。
            func owesChecklist(_ raw: String) -> Bool {
                var t = raw
                for neg in ["暂无待核实", "无待核实", "没有待核实", "没有新的待核实"] {
                    t = t.replacingOccurrences(of: neg, with: "")
                }
                if t.contains("待核实") || t.contains("<!-- 状态词表 -->") { return true }
                // 有表就一定命中上一条（清单的列名本身就叫「待核实」），所以这里只补
                // 「编号被引用但整份文档没提待核实」这一种。锚在行首的写法要不得：
                // `String.range(of:options:.regularExpression)` 不认多行锚，`^` 只匹配整个字符串的开头
                return t.range(of: #"\*\*V[0-9]"#, options: .regularExpression) != nil
            }
            // 下限留一格给新建的研究文档（今天 3 份）：写死 3 会让「合并掉一份文档」这种
            // 正当改动变红，而这一格不会放过「目录被清空」
            let docs = try SourceTree.markdownTexts(under: "docs/research", atLeast: 2)
            var checked: [String] = [], skipped: [String] = []
            var perDoc: [(path: String, bad: [String])] = []
            for doc in docs {
                guard owesChecklist(doc.text) else { skipped.append(doc.relativePath); continue }
                checked.append(doc.relativePath)
                let bad = checklistViolations(doc.text)
                if !bad.isEmpty { perDoc.append((doc.relativePath, bad)) }
            }
            // 「一份都没校验」必须是失败：否则⑤的扩围可以靠匹配条件写错退化成空跑
            try expectTrue(!checked.isEmpty,
                           "`docs/research/` 下没有任何一份文档被这份守卫校验（扫到 \(docs.count) 份）"
                           + "——要么约定变了要么触发条件写错了，绿在这里没有意义")
            let total = perDoc.reduce(0) { $0 + $1.bad.count }
            // 按文档分组再截：全局 `prefix(10)` 会被第一份文档的噪声挤满，
            // 第二份文档一条违规都读不到（review P2）
            let shown = perDoc.map { d in
                "\(d.path)（\(d.bad.count) 条）：\n      "
                    + d.bad.prefix(3).joined(separator: "\n      ")
                    + (d.bad.count > 3 ? "\n      …另有 \(d.bad.count - 3) 条" : "")
            }.joined(separator: "\n")
            try expectTrue(perDoc.isEmpty,
                           "研究文档的「待核实」没有编号、没有状态或没有复核方式"
                           + "（共 \(total) 条／校验 \(checked.count) 份：\(checked)／跳过 \(skipped.count) 份：\(skipped)）\n    "
                           + shown)
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

// MARK: - 「待核实清单」的逐文档校验器
//
// 从断言里抽出来、并且返回违规清单而不是直接 expect：这条守卫现在要跑遍整个 `docs/research/`，
// 内联的话第一份文档的第一处失败就抛，剩下几份是合规还是没看过，读者完全读不出来。
private func checklistViolations(_ text: String) -> [String] {
    let allowed = ["未找到出处", "待逐字复核", "待真机实测"]
    // 两类「没核完」的说法，作用域不同：
    // · 判定类（对**我们**核没核下结论）整篇禁——它出现在正文就是状态的第二出口；
    // · 证据类（对**别人的正文**下结论，如「该页未列 hooks」）是研究文档该有的措辞，
    //   只在清单小节内禁，区外误伤会把作者推向「换个更含糊的说法」，那比第二出口更糟。
    let verdictWords = ["没逐字复核", "未找到正文依据", "没有逐条出处", "二手转述", "正文没说"]
    let evidenceWords = ["正文未出现", "文档未提", "该页未列"]
    let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    // 一律按 trim 后的形态比对：表头与声明块标记常写在列表项的缩进里，
    // 按原文精确匹配会把一份合规文档判成「你没建表 / 你没声明」（review P2）
    let trimmed = lines.map { $0.trimmingCharacters(in: .whitespaces) }
    var bad: [String] = []

    func cells(_ line: String) -> [String] {
        line.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }
    func tokenID(_ tok: String) -> String? {
        let s = Substring(tok)
        guard s.first == "V", s.count >= 2 else { return nil }
        let rest = s.dropFirst()
        let digits = rest.prefix(while: { $0.isASCII && $0.isNumber })
        guard !digits.isEmpty else { return nil }
        let tail = rest.dropFirst(digits.count)
        guard tail.count <= 1, tail.allSatisfy({ $0.isASCII && $0.isLetter }) else { return nil }
        return tok
    }
    func boldID(_ cell: String) -> String? {
        guard cell.hasPrefix("**"), cell.hasSuffix("**"), cell.count >= 5 else { return nil }
        return tokenID(String(cell.dropFirst(2).dropLast(2)))
    }
    func boldIDs(_ s: String) -> Set<String> {
        var out = Set<String>()
        for tok in s.components(separatedBy: "**") {
            if let id = tokenID(tok) { out.insert(id) }
        }
        return out
    }

    // ── 表格：先按**列名**定位表头，列索引一律从表头推 ──────────────────────
    // 定位原先是 `hasPrefix("| 编号 |")`——那等于把「编号必须是第一列」写进约定，
    // 于是 `cells[1]` 与 `idx("编号")` 恒等，注释里「一律从表头推」是句空话（review P1-7）。
    // 现在认「编号 + 状态」两列即认定它是清单表，列序不再有含义；
    // 四列齐不齐是**下一步**的事：少一列要报「缺哪一列」，不能和「根本没建表」混成同一条消息。
    let required = ["编号", "待核实", "状态", "复核方式"]
    var headerAt: Int?
    for (n, t) in trimmed.enumerated() where t.hasPrefix("|") {
        let c = cells(t)
        if c.contains("编号"), c.contains("状态") { headerAt = n; break }
    }
    guard let h = headerAt else {
        return ["没有清单表：正文写了「待核实」就必须给一张带表头的清单"
                + "（列名是断言定位表格的唯一依据，也是「哪条还没核」的唯一出口）"]
    }
    let headerCells = cells(lines[h])
    let missing = required.filter { !headerCells.contains($0) }
    if !missing.isEmpty {
        // 缺列就直接收工：逐行校验会把它放大成每行两条下游噪声，把真因埋掉（review P2）
        return ["清单表头缺列：\(missing)——「编号/待核实/状态/复核方式」四列齐备，逐行校验才有意义"]
    }
    let idx = { (name: String) -> Int in headerCells.firstIndex(of: name) ?? -1 }
    // 取不到列返回 nil 而不是下标越界：崩掉整个 runner 比一条失败消息糟得多，
    // 断言不能靠崩溃来表达意见
    func column(_ row: [String], _ name: String) -> String? {
        let i = idx(name)
        return (i >= 0 && i < row.count) ? row[i] : nil
    }

    // ── 对齐行：不假设它存在，也不跳过第一行数据 ────────────────────────────
    // 上一版写死 `headerAt + 2`：表头下一行若不是 `| :--- |`，第一条数据行既不进逐行校验、
    // 也不进任何一侧的编号集合——一条清单项彻底隐形（review P1-4）。
    var dataStart = h + 1
    if h + 1 < lines.count {
        let next = trimmed[h + 1]
        if next.contains("-"), next.allSatisfy({ "|:- ".contains($0) }) { dataStart = h + 2 }
        else { bad.append("表头下一行不是 `| :--- |` 对齐行，按第一条数据行处理：\(next.prefix(50))") }
    }

    var dataRows: [(n: Int, cells: [String])] = []
    var rowCursor = dataStart
    while rowCursor < lines.count, trimmed[rowCursor].hasPrefix("|") {
        let row = cells(lines[rowCursor])
        if row.count != headerCells.count {
            bad.append("第 \(rowCursor + 1) 行列数与表头不符（表头 \(headerCells.count) 列，本行 \(row.count) 列）："
                        + "\(lines[rowCursor].prefix(70))")
        } else {
            dataRows.append((rowCursor + 1, row))
        }
        rowCursor += 1
    }
    if dataRows.isEmpty { bad.append("清单表头下面一行数据都没有") }

    let placeholders = ["-", "—", "–", "待补", "TBD", "N/A", "见正文", "同上", ""]
    var idCount: [String: Int] = [:]
    for row in dataRows {
        let at = "第 \(row.n) 行"
        let idCell = column(row.cells, "编号") ?? ""
        // ① 行形下界
        if let id = boldID(idCell) { idCount[id, default: 0] += 1 }
        else { bad.append("\(at) 的编号格必须写成 **V<数字>[单个字母]**（小写、裸文本、少空格一律不认）：\(idCell)") }
        // 状态格：cell 级校验，不是整行 contains
        let state = column(row.cells, "状态") ?? ""
        if !allowed.contains(where: { state.hasPrefix($0) }) {
            bad.append("\(at)「状态」格必须以三分类之一开头：\(state.prefix(40))")
        }
        // 「问哪件事」这一列原先只查列名不查格子：八行描述全清空而另三列合规，
        // 这张表依然能过——可它恰恰没写明「这条到底在问什么」（review P1-2）
        let what = column(row.cells, "待核实") ?? ""
        if placeholders.contains(what) || what.count < 6 {
            bad.append("\(at)「待核实」格没写这条在问什么（空、占位符或少于 6 字）：\(what.prefix(30))")
        }
        // 复核方式：非空、非占位符、且真写了动作
        let how = column(row.cells, "复核方式") ?? ""
        if placeholders.contains(how) {
            bad.append("\(at) 的「复核方式」是占位符（\(how.prefix(10))）——没有它，这条永远不会被核")
        }
        if !["抓", "跑", "配", "比对", "数", "看"].contains(where: { how.contains($0) }) {
            bad.append("\(at) 的「复核方式」没写动作（抓/跑/配/比对/数/看）：\(how.prefix(40))")
        }
    }
    // ② 编号必须唯一：`tableIDs` 是 Set，复制一行会被静默折叠，
    //    于是「8 条待核」实际欠 9 条，而两侧配对看不出来（review P1-3）
    let dup = idCount.filter { $0.value > 1 }.keys.sorted()
    if !dup.isEmpty {
        bad.append("有编号占了多行（Set 会把重复折叠掉，条目数读不出来）：\(dup)——一条待核事项一行")
    }
    let tableIDs = Set(idCount.keys)

    // ── 声明块：一块只管一类词，且块本身要被审 ─────────────────────────────
    // 上一版是一把不校验内容的万能钥匙：把任何带状态词与判定说法的正文包进
    // `<!-- 状态词表 -->` 就全绿，块里空无一字也算声明（review P1-1）。
    // 放行靠声明是对的，但声明必须真是那份词汇定义，而且只放行它定义的那一类词。
    func declaredBlock(_ label: String, requires: [String], maxLines: Int, allowsVerdicts: Bool) -> Set<Int> {
        let openMark = "<!-- \(label) -->", closeMark = "<!-- /\(label) -->"
        guard let o = trimmed.firstIndex(of: openMark) else {
            if requires.isEmpty == false {
                bad.append("三种状态名的词汇定义必须包在 \(openMark) 块里——放行靠声明，不靠小节位置")
            }
            return []
        }
        guard let c = trimmed.firstIndex(of: closeMark), c > o else {
            bad.append("\(openMark) 没有配对的 \(closeMark)——划不出范围，等于把后文整段划成免检区")
            return []
        }
        let inner = Array((o + 1)..<c)
        if inner.allSatisfy({ trimmed[$0].isEmpty }) {
            bad.append("\(openMark) 是空块——空豁免键不算词汇定义"); return Set(inner)
        }
        if inner.count > maxLines {
            bad.append("\(openMark) 块有 \(inner.count) 行（上限 \(maxLines)）——声明块是给词汇定义用的，"
                        + "写这么长是在给整节开免检区")
        }
        let body = inner.map { lines[$0] }.joined(separator: "\n")
        if !boldIDs(body).isEmpty {
            bad.append("\(openMark) 块里出现了编号引用——条目定性只能写在清单表的状态列")
        }
        // 认**带反引号**的形态：声明块的要务是「定义这个词」，不是「用它说句话」。
        // 不这么窄的话，把任意一段带三个状态词的散文包进块里就全绿（review P1-1）
        for w in requires where !body.contains("`\(w)`") {
            bad.append("\(openMark) 块里没有以 `\(w)` 的形式定义这个词——豁免键必须就是那份定义本身")
        }
        if !allowsVerdicts {
            for m in verdictWords where body.contains(m) {
                bad.append("\(openMark) 块里用「\(m)」下了判定——定义块只放行词汇，不放行结论")
            }
        }
        return Set(inner)
    }
    let statusZone = declaredBlock("状态词表", requires: allowed, maxLines: 4, allowsVerdicts: false)
    let quoteZone = declaredBlock("禁词说明", requires: [], maxLines: 6, allowsVerdicts: true)

    // 合法区按**行号**而不是行内容划：同形表行若按内容豁免，会在文档之间互相放行。
    let tableRegion = Set(h..<max(h + 1, rowCursor))
    func isLegal(_ n: Int, zone: Set<Int>) -> Bool { tableRegion.contains(n) || zone.contains(n) }

    // ── 配对：正文引用的编号 ↔ 表格里的编号 ────────────────────────────────
    let body = lines.enumerated().filter { !isLegal($0.offset, zone: statusZone.union(quoteZone)) }
        .map(\.element).joined(separator: "\n")
    let bodyIDs = boldIDs(body)
    let noRow = bodyIDs.subtracting(tableIDs)
    if !noRow.isEmpty { bad.append("正文引用了清单里没有的编号：\(noRow.sorted())") }
    let orphan = tableIDs.subtracting(bodyIDs)
    if !orphan.isEmpty { bad.append("清单里有正文从未引用的编号（等于没人会去核）：\(orphan.sorted())") }

    // ── 第二出口：状态词整篇只许出现在状态列；判定说法整篇禁，证据说法只在清单小节内禁 ──
    // 清单小节的边界按 `## ` 标题切：上一版的禁词单位是**行**，把判定挪到下一行就看不见；
    // 改成段之后仍然隔一个空行就绕（review P1-5）。所以这里干脆不按段——判定类整篇扫。
    let secStart = trimmed.firstIndex { $0.hasPrefix("## ") && $0.contains("待核实") }
    let sectionRange: Range<Int>
    if let s = secStart {
        // 下一个 `## ` 才是终点——原先写成 `trimmed.firstIndex{…}` 会找到全文**第一个**二级标题，
        // 那个通常在清单之前，于是作用域算空，证据类词形同没禁
        let e = ((s + 1)..<max(s + 1, lines.count)).first { trimmed[$0].hasPrefix("## ") } ?? lines.count
        sectionRange = (s + 1)..<max(s + 1, e)
    } else {
        sectionRange = 0..<0
    }

    for (n, line) in lines.enumerated() {
        if isLegal(n, zone: statusZone) { continue }
        for w in allowed where line.contains(w) { bad.append("第 \(n + 1) 行又写了状态词「\(w)」") }
        if quoteZone.contains(n) { continue }
        if line.contains("未证实") { bad.append("第 \(n + 1) 行：「未证实」这个笼统说法回来了") }
        for m in verdictWords where line.contains(m) {
            bad.append("第 \(n + 1) 行用判定说法「\(m)」下了个状态——状态只写在清单表的状态列："
                        + "\(line.prefix(60))")
        }
        if sectionRange.contains(n) {
            for m in evidenceWords where line.contains(m) {
                bad.append("清单小节里第 \(n + 1) 行出现「\(m)」——它是对**我们**的定性，"
                            + "对别家正文的陈述请写在厂商那一节")
            }
        }
    }
    return bad
}
