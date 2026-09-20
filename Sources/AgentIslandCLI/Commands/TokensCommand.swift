import Foundation
import AgentIslandCore

public enum TokensCommand {
    public static func run(args: [String]) async {
        let isJson = args.contains("--json")

        // 支持 --budget / -b 参数自定义或读取系统配置
        var explicitBudget: Int? = nil
        var i = 0
        while i < args.count {
            let arg = args[i]
            if (arg == "--budget" || arg == "-b") && i + 1 < args.count {
                let text = args[i + 1].lowercased().trimmingCharacters(in: .whitespaces)
                let multiplier: Double
                let digits: String
                if text.hasSuffix("m") {
                    multiplier = 1_000_000; digits = String(text.dropLast())
                } else if text.hasSuffix("k") {
                    multiplier = 1_000; digits = String(text.dropLast())
                } else {
                    multiplier = 1; digits = text
                }
                // 必须走饱和/可失败路径：`Int(1e308)` 与 `Int(Double.nan)` 是运行时 trap，
                // 实测 `tokens --budget 1e308m` 以 SIGTRAP(133) 退出且不留任何诊断
                guard let parsed = Double(digits), parsed.isFinite, parsed >= 0,
                      parsed * multiplier <= Double(Int.max) else {
                    CLIExit.fail("无效的预算值: \(args[i + 1])（需要非负数字，可带 k/m 后缀）",
                                 code: CLIExit.badUsage)
                }
                explicitBudget = Int(parsed * multiplier)
                i += 2
                continue
            }
            i += 1
        }

        let dailyBudget = explicitBudget ?? UserDefaults.standard.integer(forKey: SettingKey.dailyTokenBudget)

        let monitor = TokenUsageMonitor()
        monitor.refresh()

        let grandTotal = monitor.grandTotal
        let usageMap = monitor.usage

        let forecast = TokenForecastEvaluator.evaluate(
            tokens24h: grandTotal.tokens24h,
            cost24h: grandTotal.cost24h,
            dailyBudget: dailyBudget
        )

        let budgetRatio = dailyBudget > 0 ? (Double(grandTotal.tokens24h) / Double(dailyBudget)) : 0.0
        let budgetStatus: String
        if dailyBudget <= 0 {
            budgetStatus = "未设置"
        } else if budgetRatio >= 1.0 {
            budgetStatus = "已超额"
        } else if budgetRatio >= 0.8 {
            budgetStatus = "接近上限"
        } else {
            budgetStatus = "正常"
        }

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
                dailyBudget: dailyBudget > 0 ? dailyBudget : nil,
                budgetRatio: dailyBudget > 0 ? budgetRatio : nil,
                budgetStatus: budgetStatus,
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

        if dailyBudget > 0 {
            print("\n" + CLIColor.bold("🎯 今日 Token 预算进度与警戒状态"))
            let bar = renderProgressBar(ratio: budgetRatio, width: 20)
            let pct = Int(budgetRatio * 100)
            let usedStr = TokenUsage.compact(grandTotal.tokens24h)
            let budgetStr = TokenUsage.compact(dailyBudget)
            let statusTag: String
            if budgetRatio >= 1.0 {
                statusTag = CLIColor.red("🚨 已超额 (\(pct)%)")
            } else if budgetRatio >= 0.8 {
                statusTag = CLIColor.yellow("⚠️ 接近上限 (\(pct)%)")
            } else {
                statusTag = CLIColor.green("✓ 正常 (\(pct)%)")
            }
            print("  预算上限:        \(CLIColor.white(budgetStr)) tokens")
            print("  消耗进度:        \(bar)  \(statusTag)  [\(usedStr) / \(budgetStr)]")
            if let exhaustDay = forecast.budgetExhaustionDay {
                print("  预计耗尽时间:    本月第 \(exhaustDay) 天")
            }
        }

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

    public static func renderProgressBar(ratio: Double, width: Int = 20) -> String {
        let filledCount = min(width, max(0, Int((ratio * Double(width)).rounded())))
        let emptyCount = max(0, width - filledCount)
        let filled = String(repeating: "█", count: filledCount)
        let empty = String(repeating: "░", count: emptyCount)
        let rawBar = "[\(filled)\(empty)]"
        if ratio >= 1.0 {
            return CLIColor.red(rawBar)
        } else if ratio >= 0.8 {
            return CLIColor.yellow(rawBar)
        } else {
            return CLIColor.cyan(rawBar)
        }
    }
}
