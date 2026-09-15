import Foundation

// MARK: - 长标题可读性回归

@MainActor
enum TypographyTests {
    static func register() {
        TestKit.test("标题可读性: 顶栏使用稳定主标题与动态副标题，不再挤成单行") {
            let sources = try uiSources()
            try expectTrue(sources.components.contains("struct AdaptiveHeaderText"),
                           "共享 UI 应提供自适应双层标题组件")
            try expectTrue(sources.island.contains("AdaptiveHeaderText("),
                           "主卡顶栏必须使用自适应标题组件")
            try expectTrue(sources.components.contains("minWidth: 96"),
                           "标题需要最低可读宽度，不能被右侧按钮压到零")
        }

        TestKit.test("标题可读性: 截断统一保留首尾并提供全文提示与辅助功能文案") {
            let sources = try uiSources()
            try expectTrue(sources.components.contains("func readableSingleLine"),
                           "长文本截断规则应收敛为共享修饰符")
            try expectTrue(sources.components.contains(".truncationMode(.middle)"),
                           "文件名、模型名等长文本应保留首尾而非只保留开头")
            try expectTrue(sources.components.contains(".help(fullText)"),
                           "鼠标悬停必须能查看全文")
            try expectTrue(sources.components.contains(".accessibilityLabel(fullText)"),
                           "VoiceOver 必须始终读取全文")
            try expectTrue(sources.detail.contains("readableSingleLine"),
                           "详情标题必须复用同一可读截断规则")
            try expectTrue(sources.row.contains("readableSingleLine"),
                           "Agent 名称和动作必须复用同一可读截断规则")
            try expectTrue(sources.banner.contains("readableSingleLine"),
                           "事件横幅标题必须复用同一可读截断规则")
        }
    }

    private static func uiSources() throws -> (
        components: String, island: String, detail: String, row: String, banner: String
    ) {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        func read(_ name: String) throws -> String {
            try String(contentsOf: root.appendingPathComponent("Sources/AgentIsland/\(name)"),
                       encoding: .utf8)
        }
        return (
            try read("IslandComponents.swift"),
            try read("IslandView.swift"),
            try read("DetailViews.swift"),
            try read("AgentRowView.swift"),
            try read("EventBannerView.swift")
        )
    }
}
