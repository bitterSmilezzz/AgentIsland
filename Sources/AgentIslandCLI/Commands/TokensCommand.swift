import Foundation
import AgentIslandCore

public enum TokensCommand {
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")

        let monitor = TokenUsageMonitor()
        monitor.refresh()

        let grandTotal = monitor.grandTotal
        let usageMap = monitor.usage

        let forecast = TokenForecastEvaluator.evaluate(
            tokens24h: grandTotal.tokens24h,
            cost24h: grandTotal.cost24h,
            dailyBudget: 0
        )

        if isJson {
            var agentDtos: [String: CLIAgentTokenDTO] = [:]
            for (id, u) in usageMap {
                agentDtos[id] = CLIAgentTokenDTO(
                    tokens24h: u.tokens24h,
                    cost24h: u.cost24h,
                    tokensTotal: u.tokensTotal,
                    costTotal: u.costTotal
                )
            }

            let report = CLITokenReportDTO(
                tokens24h: grandTotal.tokens24h,
                cost24h: grandTotal.cost24h,
                tokensTotal: grandTotal.tokensTotal,
                costTotal: grandTotal.costTotal,
                projectedMonthEndTokens: forecast.projectedMonthEndTokens,
                projectedMonthEndCost: forecast.projectedMonthEndCost,
                budgetExhaustionDay: forecast.budgetExhaustionDay,
                forecastSummary: forecast.forecastSummary,
                agents: agentDtos
            )

            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(report), let str = String(data: data, encoding: .utf8) {
                print(str)
            }
            return
        }

        print("\n" + CLIColor.bold("⚡️ AgentIsland Token 与成本核算看板"))
        print(CLIColor.dim("──────────────────────────────────────────"))

        let tokens24hStr = CLIColor.cyan(TokenUsage.compact(grandTotal.tokens24h))
        let cost24hStr = CLIColor.green(grandTotal.cost24h > 0 ? TokenUsage.cost(grandTotal.cost24h) : "$0.00")
        let tokensTotalStr = CLIColor.white(TokenUsage.compact(grandTotal.tokensTotal))
        let costTotalStr = CLIColor.white(grandTotal.costTotal > 0 ? TokenUsage.cost(grandTotal.costTotal) : "$0.00")

        print("  最近 24h 消耗:   \(tokens24hStr) tokens  |  \(cost24hStr)")
        print("  历史总计消耗:    \(tokensTotalStr) tokens  |  \(costTotalStr)")

        print("\n" + CLIColor.bold("🔮 月末趋势推算 (基于近 24h 消耗速率)"))
        print("  预估月末 Token:  \(CLIColor.cyan(forecast.formattedMonthlyTokens))")
        print("  预估月末费用:    \(CLIColor.green(forecast.formattedMonthlyCost))")
        print("  趋势分析结论:    \(forecast.forecastSummary)")

        let activeAgents = usageMap.filter { !$0.value.isEmpty }
            .sorted { $0.value.tokens24h > $1.value.tokens24h }

        if !activeAgents.isEmpty {
            print("\n" + CLIColor.bold("📊 各智能体消耗排行"))
            var table = CLITable(columns: [
                CLITable.Column("智能体 ID", minWidth: 16),
                CLITable.Column("24h Tokens", minWidth: 12, alignRight: true),
                CLITable.Column("24h 费用", minWidth: 10, alignRight: true),
                CLITable.Column("累计 Tokens", minWidth: 12, alignRight: true),
                CLITable.Column("累计费用", minWidth: 10, alignRight: true)
            ])

            for (id, u) in activeAgents {
                table.addRow([
                    id,
                    TokenUsage.compact(u.tokens24h),
                    u.cost24h > 0 ? TokenUsage.cost(u.cost24h) : "$0.00",
                    TokenUsage.compact(u.tokensTotal),
                    u.costTotal > 0 ? TokenUsage.cost(u.costTotal) : "$0.00"
                ])
            }
            print(table.render())
        }

        print("")
    }
}
