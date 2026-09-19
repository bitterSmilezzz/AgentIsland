import Foundation

// MARK: - 面板呈现口径回归（搜索命中 / 模型占比取舍 / 无明细诚实性）
//
// 三处都曾是「用户看不出来」的逻辑错误：搜索谓词写两遍会让可见列表与 j/k 聚焦集分叉；
// 环形图 prefix(4) 丢模型后百分比之和不再为 100%；热力图在零数据时画一张全灰的合法图。
// 纯判定逻辑（AgentSearchFilter / ModelDonutAggregate）经符号链接编进本 runner 直接断言，
// 与 IslandMetricsKit 同一套做法；视图侧只核对两处调用是否同源。

@MainActor
enum PanelPresentationTests {

    static func register() {
        registerSearchFilter()
        registerDonutBreakdown()
        registerHonestyGuards()
    }

    // MARK: 搜索命中口径

    private static func registerSearchFilter() {
        TestKit.test("搜索命中: 空/纯空白关键词一律命中，绝不静默清空列表") {
            try expectTrue(AgentSearchFilter.matches(name: "DimAgent", id: "dim",
                                                    processNames: ["dim-cli"], query: ""),
                           "空关键词应命中")
            try expectTrue(AgentSearchFilter.matches(name: "DimAgent", id: "dim",
                                                    processNames: ["dim-cli"], query: "   \n "),
                           "纯空白关键词应命中")
            try expectEqual(AgentSearchFilter.normalized("  ClAU-Code \n"), "clau-code",
                            "归一化应去首尾空白并小写")
        }

        TestKit.test("搜索命中: 名称 / 档案 id / 进程名三字段任一命中，大小写不敏感") {
            try expectTrue(AgentSearchFilter.matches(name: "Claude Code", id: "custom-claude",
                                                    processNames: ["node"], query: "cla"),
                           "名称命中")
            try expectTrue(AgentSearchFilter.matches(name: "Claude Code", id: "custom-claude",
                                                    processNames: ["node"], query: "CUSTOM-"),
                           "档案 id 命中")
            try expectTrue(AgentSearchFilter.matches(name: "Claude Code", id: "custom-claude",
                                                    processNames: ["claude-cli"], query: "cli"),
                           "进程名命中")
            try expectFalse(AgentSearchFilter.matches(name: "Claude Code", id: "custom-claude",
                                                     processNames: ["node"], query: "codex"),
                            "无交集不应命中")
        }

        TestKit.test("搜索命中: 列表侧与键盘聚焦侧共用同一入口，不再各写一份谓词") {
            let island = try readSource("IslandView.swift")
            let routing = try readSource("IslandPanelRouting.swift")
            try expectTrue(island.contains("IslandPanelController.focusableAgents"),
                           "主列表必须复用共享命中口径")
            try expectTrue(routing.contains("AgentSearchFilter.matches"),
                           "聚焦口径必须落在 AgentSearchFilter 上")
            // 两份谓词一旦重新分叉，这里就会先响
            try expectFalse(island.contains(".lowercased().contains(q)"),
                            "IslandView 不应再自带一份匹配实现")
            try expectFalse(routing.contains(".lowercased().contains(q)"),
                            "IslandPanelRouting 不应再自带一份匹配实现")
        }
    }

    // MARK: 模型占比取舍

    private static func registerDonutBreakdown() {
        TestKit.test("模型占比: 不超过 4 个模型时不虚构「其余」行") {
            let b = ModelDonutAggregate.breakdown([
                ModelDonutAggregate.Entry(modelId: "a", tokens: 60),
                ModelDonutAggregate.Entry(modelId: "b", tokens: 30),
                ModelDonutAggregate.Entry(modelId: "c", tokens: 10),
            ])
            try expectEqual(b.shownCount, 3, "三个模型应全部单列")
            try expectFalse(b.hasHidden, "无遗漏时不应出现其余行")
            try expectEqual(b.hiddenText, "", "其余文案应为空")
            try expectEqual(b.totalTokens, 100, "总数需覆盖全部模型")
        }

        TestKit.test("模型占比: 超出图例上限时把遗漏部分显式算出来") {
            let b = ModelDonutAggregate.breakdown((0..<6).map {
                ModelDonutAggregate.Entry(modelId: "m\($0)", tokens: [50, 25, 10, 5, 4, 3][$0])
            })
            try expectEqual(b.shownCount, ModelDonutAggregate.maxShown, "图例上限 4 项")
            try expectEqual(b.hiddenCount, 2, "遗漏两个模型")
            try expectEqual(b.hiddenTokens, 7, "其余 Token 合计")
            try expectEqual(b.hiddenText, "其余 2 个 · 7%", "其余行需同时给出数量与占比")
            try expectTrue(b.hiddenDetailText.contains("m4") && b.hiddenDetailText.contains("m5"),
                           "悬停明细要逐个列出被折掉的模型")
        }

        TestKit.test("模型占比: 剩余不足 1% 不得四舍五入成 0%（缺失数据不伪装成零）") {
            let b = ModelDonutAggregate.breakdown(
                [5000, 5000, 5000, 4950, 50].enumerated().map {
                    ModelDonutAggregate.Entry(modelId: "m\($0.offset)", tokens: $0.element)
                }
            )
            try expectTrue(b.hiddenPercent < 1, "样本的剩余占比应小于 1%")
            try expectEqual(ModelDonutAggregate.Breakdown.percentText(b.hiddenPercent), "<1%",
                            "非零不足 1% 应播报为 <1%")
            try expectEqual(ModelDonutAggregate.Breakdown.percentText(0), "0%",
                            "真正的零占比仍写 0%")
            try expectEqual(ModelDonutAggregate.Breakdown.percentText(12.6), "13%", "四舍五入")
        }

        TestKit.test("模型占比: 超限但剩余为 0 时不出现空转的其余行") {
            let b = ModelDonutAggregate.breakdown(
                [10, 10, 10, 10, 0].enumerated().map {
                    ModelDonutAggregate.Entry(modelId: "m\($0.offset)", tokens: $0.element)
                }
            )
            try expectEqual(b.hiddenCount, 1, "模型数仍超上限")
            try expectFalse(b.hasHidden, "剩余用量为 0 时不必占一行")
        }
    }

    // MARK: 诚实性与可读性护栏

    private static func registerHonestyGuards() {
        TestKit.test("热力图: 零明细走显式无数据态，不再合成全零 24 格") {
            let heatmap = try readSource("ActivityHeatmapView.swift")
            try expectTrue(heatmap.contains("本时段未发现 Token 明细"),
                           "无明细必须有显式状态（CONTEXT.md「Token 数据覆盖」）")
            try expectTrue(heatmap.contains("hasDetail"),
                           "有数据/无数据两条呈现路径要分开")
            try expectFalse(heatmap.contains("(0..<24).map { ($0, 0, 0.0) }"),
                           "不得再合成一份全零节律")
        }

        TestKit.test("自绘图表: 四张卡各自给出一句 VoiceOver 播报") {
            for file in ["ModelDonutChartView.swift", "ActivityHeatmapView.swift",
                         "EventHistoryView.swift", "ProcessTreeView.swift"] {
                let src = try readSource(file)
                try expectTrue(src.contains(".accessibilityElement(children: .ignore)"),
                               "\(file) 应把自绘内容收敛为单一元素")
                try expectTrue(src.contains(".accessibilityLabel("),
                               "\(file) 应提供口语化摘要")
            }
        }

        TestKit.test("字号下限: 图表卡内不得再出现 9pt 以下的文字") {
            for file in ["ModelDonutChartView.swift", "ActivityHeatmapView.swift"] {
                let sizes = fontSizes(in: try readSource(file))
                try expectTrue(!sizes.isEmpty, "\(file) 应能扫到字号字面量")
                let tooSmall = sizes.filter { $0 < 9 }
                try expectTrue(tooSmall.isEmpty, "\(file) 低于 9pt 下限的字号 \(tooSmall)")
            }
        }

        TestKit.test("调色板: 模型配色走 Theme 的动态浅/深成对色") {
            let palette = try paletteLines(in: readSource("ModelDonutChartView.swift"))
            try expectFalse(palette.contains { $0.contains("Color(hex:") },
                            "调色板不应出现单值硬编码色（浅色模式对比度不达标）")
            let declared = palette.filter { $0.contains("dynamicLight") || $0.contains("Theme.sydedock") }
            try expectTrue(declared.count >= 7, "七种切片色都应走 Theme 令牌或浅/深成对声明")
        }

        TestKit.test("破坏性动作: 清空事件历史与删除档案均需二次确认") {
            let history = try readSource("EventHistoryView.swift")
            try expectTrue(history.contains("confirmingClear = true"),
                           "清空按钮应先落确认态")
            try expectTrue(history.contains("Button(\"清空\", role: .destructive)"),
                           "真正的清空动作要在 destructive 确认后")
            let settings = try readSource("SettingsView.swift")
            try expectTrue(settings.contains("onRemove: { pendingRemove = profile }"),
                           "删除自定义档案须走确认态而非直接删")
            try expectFalse(settings.contains("监控配置将被移除"),
                            "档案是 CONTEXT.md 术语，「配置」为 Avoid 词")
        }
    }

    /// 截出 `static let palette: [Color] = [...]` 内的条目行，逐行核对声明方式
    private static func paletteLines(in src: String) throws -> [String] {
        guard let start = src.range(of: "static let palette"),
              let open = src[start.upperBound...].range(of: "= [") else {
            throw TestError(message: "未找到调色板声明")
        }
        return src[open.upperBound...]
            .components(separatedBy: "\n")
            .prefix { !$0.contains("]") }
            .filter { $0.contains("Theme.") || $0.contains("Color(") }
    }

    /// 扫出 `Theme.bodyFont(<size>` / `monoFont(` / `monoDigitFont(` / `displayFont(` 的字号字面量
    /// （badgeFont 只接收 weight，无字号可扫）
    private static func fontSizes(in src: String) -> [Double] {
        var sizes: [Double] = []
        for marker in ["bodyFont(", "monoFont(", "monoDigitFont(", "displayFont("] {
            var tail = src[src.startIndex...]
            while let hit = tail.range(of: marker) {
                tail = tail[hit.upperBound...]
                let literal = tail.prefix { $0.isNumber || $0 == "." }
                if let value = Double(literal) { sizes.append(value) }
            }
        }
        return sizes
    }

    private static func readSource(_ name: String) throws -> String {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        return try String(contentsOf: root.appendingPathComponent("Sources/AgentIsland/\(name)"),
                         encoding: .utf8)
    }
}
