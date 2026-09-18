import Foundation
import AppKit

/// Token 消费与会话账单报表导出引擎
public enum TokenReportExporter {

    /// 导出为结构化 Markdown 报表文本
    public static func generateMarkdown(
        timeline: TokenUsageTimeline,
        range: TokenTimeRange,
        grandTotal: TokenUsage,
        now: Date = Date()
    ) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
        let dateStr = formatter.string(from: now)

        let totalTokensText = TokenUsage.compact(timeline.tokens)
        let totalCostResolved = TokenCostEstimator.resolveCost(actual: timeline.cost, tokens: timeline.tokens)
        let costDisplay = !totalCostResolved.text.isEmpty ? totalCostResolved.text : "$0.00"

        var md = """
        # AgentIsland Token 消费与用量分析报表
        - 导出时间：\(dateStr)
        - 统计周期：\(range.label)
        - 周期内 Token 消耗：**\(totalTokensText)** (\(timeline.tokens) tokens)
        - 周期内费用估算：**\(costDisplay)**\(totalCostResolved.isEstimated ? " (参考估算)" : "")
        - 累计历史总用量：\(TokenUsage.compact(grandTotal.tokensTotal)) (\(TokenUsage.cost(grandTotal.costTotal)))

        ---

        ### 数据源与智能体分布

        | 智能体 (Agent) | Token 用量 | 预估/实际费用 | 状态 |
        | :--- | :--- | :--- | :--- |

        """

        let sortedSources = timeline.sources.sorted { $0.tokens > $1.tokens }
        for s in sortedSources {
            let res = TokenCostEstimator.resolveCost(actual: s.cost, modelId: s.agentId, tokens: s.tokens)
            let fee = !res.text.isEmpty ? res.text : "—"
            let status = s.isAvailable ? "已连接" : "未发现"
            md += "| `\(s.agentId)` | \(TokenUsage.compact(s.tokens)) | \(fee) | \(status) |\n"
        }

        md += "\n> 生成自 AgentIsland (macOS AI 智能体监控灵动岛)\n"
        return md
    }

    /// 导出为标准逗号分隔 CSV 格式（UTF-8）
    public static func generateCSV(
        timeline: TokenUsageTimeline,
        range: TokenTimeRange,
        now: Date = Date()
    ) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
        let dateStr = formatter.string(from: now)

        var csv = "\u{FEFF}" // 添加 UTF-8 BOM，防止 Excel 打开乱码
        csv += "报表时间,周期,Agent,Tokens,费用,状态\n"

        let sortedSources = timeline.sources.sorted { $0.tokens > $1.tokens }
        for s in sortedSources {
            let res = TokenCostEstimator.resolveCost(actual: s.cost, modelId: s.agentId, tokens: s.tokens)
            let fee = !res.text.isEmpty ? res.text : "$0.00"
            let status = s.isAvailable ? "已连接" : "未发现"
            csv += "\"\(dateStr)\",\"\(range.label)\",\"\(s.agentId)\",\(s.tokens),\"\(fee)\",\"\(status)\"\n"
        }
        return csv
    }

    /// 复制文本至系统剪贴板
    @discardableResult
    public static func copyToPasteboard(_ content: String) -> Bool {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        return pasteboard.setString(content, forType: .string)
    }
}
