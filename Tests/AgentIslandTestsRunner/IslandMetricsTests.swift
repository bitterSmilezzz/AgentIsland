import Foundation
@testable import AgentIslandCore
@testable import IslandMetricsKit

// MARK: - 岛窗口几何回归（IslandMetrics 纯函数）
//
// 背景：「底部 Token 汇总栏被裁」在历史上反复出现（窗口高度 = 各子项高度之和，
// 任一项漏算/算小就会裁切）。这里用**显式字面量**锁定当前标定值：本文件里任何
// 数字变化都必须先做可视化验收（scripts/test-token-layout.py），不允许只改常量改断言。
//
// 覆盖：chromeHeight 全 16 组合、listHeight 空态/单行/封顶/极端组合、
// expandedHeight 五条路由 + 不变量（各子项之和、不低于一行、不超过高度上限）。

@MainActor
enum IslandMetricsTests {

    // 常量字面量（标定值）：改动即是行为变更，需可视化验收
    private static let notchInset: CGFloat = 10
    private static let headerHeight: CGFloat = 43          // 12 + 23 + 8
    private static let dividerHeight: CGFloat = 1
    private static let ringsShelf: CGFloat = 40
    private static let bannerCollapsed: CGFloat = 66
    private static let bannerExpanded: CGFloat = 142
    private static let summaryBar: CGFloat = 28
    private static let emptyState: CGFloat = 87
    private static let detailPage: CGFloat = 378            // 20 + 47 + 1 + 310
    private static let rowHeight: CGFloat = 52
    private static let listExtra: CGFloat = 10
    private static let listMax: CGFloat = 340
    private static let maxHeight: CGFloat = 460

    private struct Combo {
        let summary: Bool
        let rings: Bool
        let event: Bool
        let expanded: Bool
        let chrome: CGFloat
        var label: String { "summary:\(summary) rings:\(rings) event:\(event) expanded:\(expanded)" }
    }

    /// 期望 chrome 高度：**独立**的字面量算式（不引用 IslandMetrics 的常量），
    /// 增量 29 = 汇总栏 28 + 分割线 1，41 = 看板 40 + 1，67 / 143 = 事件栏 66 / 142 + 1。
    /// 事件栏展开与收起互斥——两者同时相加是曾经的错算来源，故这里写成三元式。
    private static func chromeLiteral(summary: Bool, rings: Bool, event: Bool, expanded: Bool) -> CGFloat {
        var h: CGFloat = 64                       // 2×10 倒角 + 43 顶栏 + 1 分割线
        if summary { h += 29 }
        if rings { h += 41 }
        if event { h += (expanded ? 143 : 67) }
        return h
    }

    /// chromeHeight 全 16 组合（穷举，避免手抄漏项）
    private static let combos: [Combo] = {
        var out: [Combo] = []
        for s in [false, true] {
            for r in [false, true] {
                for e in [false, true] {
                    for x in [false, true] {
                        out.append(Combo(summary: s, rings: r, event: e, expanded: x,
                                         chrome: chromeLiteral(summary: s, rings: r, event: e, expanded: x)))
                    }
                }
            }
        }
        return out
    }()

    /// 关键组合的字面量钉子（防止上面穷举表被顺手改成从实现推导）
    private static let chromePins: [(Bool, Bool, Bool, Bool, CGFloat)] = [
        (false, false, false, false, 64),      // 裸列表
        (true,  false, false, false, 93),      // + 汇总栏
        (false, true,  false, false, 105),     // + 活动环看板
        (false, false, true,  false, 131),     // + 收起事件栏
        (false, false, true,  true,  207),     // + 展开事件栏
        (true,  true,  false, false, 134),     // + 汇总栏 + 看板
        (true,  true,  true,  false, 201),     // + 汇总栏 + 看板 + 事件栏
        (false, true,  true,  true,  248),
        (true,  true,  true,  true,  277),     // 全开（当前最高 chrome）
    ]

    static func register() {

        TestKit.test("岛几何: chromeHeight 全 16 组合 = 顶栏 + 分割线 + 看板 + 事件栏 + 汇总栏") {
            try expectEqual(combos.count, 16, "组合数（全排列）")
            for c in combos {
                let actual = IslandMetrics.chromeHeight(hasSummary: c.summary, hasRings: c.rings,
                                                        hasEvent: c.event, eventExpanded: c.expanded)
                try expectEqual(actual, c.chrome, "chrome(\(c.label))")
                // 与字面量表相互印证：任一项漏算会两处同时失败
                let expected = 2 * notchInset + headerHeight + dividerHeight
                    + (c.rings ? ringsShelf + dividerHeight : 0)
                    + (c.event ? (c.expanded ? bannerExpanded : bannerCollapsed) + dividerHeight : 0)
                    + (c.summary ? summaryBar + dividerHeight : 0)
                try expectEqual(actual, expected, "chrome 子项之和(\(c.label))")
            }
            for p in chromePins {
                let actual = IslandMetrics.chromeHeight(hasSummary: p.0, hasRings: p.1,
                                                        hasEvent: p.2, eventExpanded: p.3)
                try expectEqual(actual, p.4, "chrome 钉子(summary:\(p.0) rings:\(p.1) event:\(p.2) expanded:\(p.3))")
            }
        }

        TestKit.test("岛几何: chromeHeight 各可选项增量固定（汇总 29 / 看板 41 / 事件 67 或 143）") {
            let base = IslandMetrics.chromeHeight(hasSummary: false, hasRings: false,
                                                  hasEvent: false, eventExpanded: false)
            try expectEqual(base, 2 * notchInset + headerHeight + dividerHeight, "无任何可选项时的基础高度")
            for c in combos {
                let actual = IslandMetrics.chromeHeight(hasSummary: c.summary, hasRings: c.rings,
                                                        hasEvent: c.event, eventExpanded: c.expanded)
                let expectedDelta = (c.summary ? summaryBar + dividerHeight : 0)
                    + (c.rings ? ringsShelf + dividerHeight : 0)
                    + (c.event ? (c.expanded ? bannerExpanded : bannerCollapsed) + dividerHeight : 0)
                try expectEqual(actual - base, expectedDelta, "加项增量(\(c.label))")
                try expectTrue(actual >= base, "加项不应降低高度(\(c.label))")
            }
            // 无事件栏时，事件栏展开标志不得贡献高度（避免幽灵高度把窗口撑高）
            for summary in [false, true] {
                for rings in [false, true] {
                    let collapsed = IslandMetrics.chromeHeight(hasSummary: summary, hasRings: rings,
                                                               hasEvent: false, eventExpanded: false)
                    let expanded = IslandMetrics.chromeHeight(hasSummary: summary, hasRings: rings,
                                                              hasEvent: false, eventExpanded: true)
                    try expectEqual(expanded, collapsed,
                                    "hasEvent=false 时 eventExpanded 不应参与高度(summary:\(summary) rings:\(rings))")
                }
            }
        }

        TestKit.test("岛几何: listHeight 空态/单行/封顶/满标志极端组合") {
            // (visibleCount, summary, rings, event, expanded, 期望)
            let cases: [(Int, Bool, Bool, Bool, Bool, CGFloat)] = [
                (0,   false, false, false, false, 62),    // 空态仍按 1 行算（max(count,1)）
                (1,   false, false, false, false, 62),
                (2,   false, false, false, false, 114),   // 2×52 + 10
                (7,   false, false, false, false, 340),   // 内容 374 触到 listMaxHeight
                (100, false, false, false, false, 340),
                (0,   true,  true,  true,  false, 62),    // chrome 201 → 剩余 259，内容 62 更小
                (7,   true,  true,  true,  false, 259),   // chrome 201 → 剩余 259 封顶
                (7,   false, false, true,  false, 329),   // chrome 131 → 剩余 329（低于 listMaxHeight）
                (2,   false, false, true,  false, 114),
                (7,   true,  true,  false, true,  326),   // 事件栏未开时 expanded 无影响 → chrome 134
                (100, true,  true,  false, true,  326),
                (2,   true,  true,  false, true,  114),
                (1,   true,  true,  false, true,  62),
                (0,   true,  true,  false, true,  62),
                (100, true,  true,  true,  true,  183),   // chrome 277 → 剩余 183（最挤）
            ]
            for c in cases {
                let actual = IslandMetrics.listHeight(visibleCount: c.0, hasSummary: c.1,
                                                      hasRings: c.2, hasEvent: c.3, eventExpanded: c.4)
                let label = "listHeight(count:\(c.0), summary:\(c.1), rings:\(c.2), event:\(c.3), expanded:\(c.4))"
                try expectEqual(actual, c.5, label)
            }
        }

        TestKit.test("岛几何: listHeight 不变量——封顶时压缩列表、保底一行、不超过上限") {
            for c in combos {
                for n in [0, 1, 2, 7, 100] {
                    let list = IslandMetrics.listHeight(visibleCount: n, hasSummary: c.summary,
                                                        hasRings: c.rings, hasEvent: c.event,
                                                        eventExpanded: c.expanded)
                    let label = "n:\(n) \(c.label)"
                    let content = CGFloat(max(n, 1)) * rowHeight + listExtra
                    try expectTrue(list >= rowHeight, "列表不得被压到一行以下（\(label) 实际 \(list)）")
                    try expectTrue(list <= listMax, "列表不得超过 listMaxHeight（\(label) 实际 \(list)）")
                    try expectTrue(list <= content, "列表不得超过自身内容高度（\(label) 实际 \(list) 内容 \(content)）")
                    let available = max(maxHeight - c.chrome, rowHeight)
                    try expectEqual(list, min(min(content, listMax), available), "listHeight 口径（\(label)）")
                    // 核心回归锁：列表 + 其它区合计不得超高，否则底部汇总栏被裁
                    try expectTrue(c.chrome + list <= maxHeight,
                                   "chrome+list=\(c.chrome + list) 超出窗口上限 \(maxHeight)（\(label)）→ 汇总栏会被裁切")
                }
            }
        }

        TestKit.test("岛几何: expandedHeight 列表页空态取空态高度") {
            let cases: [(Bool, Bool, Bool, Bool, CGFloat)] = [
                (false, false, false, false, 151),   // 64 + 87
                (true,  true,  true,  false, 288),   // 201 + 87
                (true,  true,  true,  true,  364),   // 277 + 87（空态也不得超 460）
            ]
            for c in cases {
                let actual = IslandMetrics.expandedHeight(route: .list, visibleCount: 0, hasSummary: c.0,
                                                          hasRings: c.1, hasEvent: c.2, eventExpanded: c.3)
                try expectEqual(actual, c.4,
                                "expandedHeight(.list, count:0, summary:\(c.0), rings:\(c.1), event:\(c.2), expanded:\(c.3))")
            }
        }

        TestKit.test("岛几何: expandedHeight 列表页 = chrome + 列表，触顶即顶格") {
            let cases: [(Int, Bool, Bool, Bool, Bool, CGFloat)] = [
                (1,   false, false, false, false, 126),   // 64 + 62
                (2,   false, false, false, false, 178),   // 64 + 114
                (7,   false, false, false, false, 404),   // 64 + 340
                (100, false, false, false, false, 404),
                (7,   false, false, true,  false, 460),   // 131 + 329 恰好顶格
                (7,   true,  true,  true,  false, 460),   // 201 + 259 恰好顶格
                (7,   true,  true,  false, true,  460),   // 134 + 326 恰好顶格（事件栏未开）
                (7,   true,  true,  true,  true,  460),   // 277 + 183 恰好顶格
                (100, true,  true,  true,  true,  460),   // 最挤组合顶格
            ]
            for c in cases {
                let actual = IslandMetrics.expandedHeight(route: .list, visibleCount: c.0, hasSummary: c.1,
                                                          hasRings: c.2, hasEvent: c.3, eventExpanded: c.4)
                try expectEqual(actual, c.5,
                                "expandedHeight(.list, count:\(c.0), summary:\(c.1), rings:\(c.2), event:\(c.3), expanded:\(c.4))")
            }
        }

        TestKit.test("岛几何: expandedHeight 详情/工具/流水页固定高度且不超上限") {
            // 四类非列表路由同源（两侧倒角 + detailHeader + divider + detailContent）
            let routes: [CardRoute] = [.toolbox, .liveStream("agent-x"),
                                       .agentDetail("agent-x"), .sessions("agent-x", "model-y")]
            for route in routes {
                let actual = IslandMetrics.expandedHeight(route: route, visibleCount: 0, hasSummary: false)
                try expectEqual(actual, detailPage, "详情类路由高度 \(route)")
                let withFlags = IslandMetrics.expandedHeight(route: route, visibleCount: 3, hasSummary: true,
                                                             hasRings: true, hasEvent: true, eventExpanded: true)
                try expectEqual(withFlags, detailPage, "详情类路由高度不受列表标志影响 \(route)")
            }
        }

        TestKit.test("岛几何: expandedHeight 任意组合都不超过 expandedMaxHeight 且等于子项之和") {
            for c in combos {
                for n in [0, 1, 2, 7, 100] {
                    let actual = IslandMetrics.expandedHeight(route: .list, visibleCount: n, hasSummary: c.summary,
                                                              hasRings: c.rings, hasEvent: c.event,
                                                              eventExpanded: c.expanded)
                    let label = "n:\(n) \(c.label)"
                    try expectTrue(actual <= maxHeight, "超过窗口上限（\(label) 实际 \(actual)）")
                    let body = n == 0
                        ? emptyState
                        : IslandMetrics.listHeight(visibleCount: n, hasSummary: c.summary, hasRings: c.rings,
                                                   hasEvent: c.event, eventExpanded: c.expanded)
                    try expectEqual(actual, min(c.chrome + body, maxHeight), "chrome+body（\(label)）")
                }
            }
        }

        TestKit.test("岛几何: 测试镜像的 CardRoute 与真实源码一致（漂移哨兵）") {
            guard let real = realCardRouteCases() else {
                return   // 非仓库布局运行（源码不可达）时跳过，不误报
            }
            let mirror: Set<String> = ["list", "agentDetail", "sessions", "toolbox", "liveStream"]
            try expectEqual(real, mirror,
                            "Sources/AgentIsland/IslandView.swift 的 CardRoute 已变化，需同步 Tests/IslandMetricsKit/CardRoute.swift 并补路由高度覆盖")
        }
    }

    /// 读取真实 UI 源码里的 CardRoute case 集合（编译期 #filePath，不依赖工作目录）
    private static func realCardRouteCases() -> Set<String>? {
        let root = URL(fileURLWithPath: #filePath)          // Tests/AgentIslandTestsRunner/IslandMetricsTests.swift
            .deletingLastPathComponent()                    // Tests/AgentIslandTestsRunner
            .deletingLastPathComponent()                    // Tests
            .deletingLastPathComponent()                    // 仓库根
        let file = root.appendingPathComponent("Sources/AgentIsland/IslandView.swift")
        guard let text = try? String(contentsOf: file, encoding: .utf8),
              let start = text.range(of: "enum CardRoute"),
              let end = text.range(of: "\n}", range: start.upperBound..<text.endIndex) else { return nil }
        var cases: Set<String> = []
        for raw in text[start.upperBound..<end.lowerBound].split(separator: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix("case ") else { continue }
            let name = line.dropFirst("case ".count).prefix { $0.isLetter || $0.isNumber || $0 == "_" }
            if !name.isEmpty { cases.insert(String(name)) }
        }
        return cases.isEmpty ? nil : cases
    }
}
