import Foundation

// MARK: - Token 趋势推算与月末预测模型 (v0.0.75)

public struct TokenForecastReport: Equatable, Sendable {
    public let projectedMonthEndTokens: Int
    public let projectedMonthEndCost: Double
    public let daysRemainingInMonth: Int
    public let totalDaysInMonth: Int
    public let budgetExhaustionDay: Int?
    public let forecastSummary: String

    public var formattedMonthlyTokens: String {
        TokenUsage.compact(projectedMonthEndTokens)
    }

    public var formattedMonthlyCost: String {
        return TokenUsage.costText(projectedMonthEndCost, zero: "$0.00")
    }

    public init(
        projectedMonthEndTokens: Int,
        projectedMonthEndCost: Double,
        daysRemainingInMonth: Int,
        totalDaysInMonth: Int,
        budgetExhaustionDay: Int?,
        forecastSummary: String
    ) {
        self.projectedMonthEndTokens = projectedMonthEndTokens
        self.projectedMonthEndCost = projectedMonthEndCost
        self.daysRemainingInMonth = daysRemainingInMonth
        self.totalDaysInMonth = totalDaysInMonth
        self.budgetExhaustionDay = budgetExhaustionDay
        self.forecastSummary = forecastSummary
    }
}

public enum TokenForecastEvaluator {

    public static func evaluate(
        tokens24h: Int,
        cost24h: Double,
        dailyBudget: Int = 0,
        now: Date = Date()
    ) -> TokenForecastReport {
        let calendar = Calendar.current
        let totalDays = calendar.range(of: .day, in: .month, for: now)?.count ?? 30
        let currentDay = max(1, min(totalDays, calendar.component(.day, from: now)))
        let daysRemaining = max(0, totalDays - currentDay)

        // 以 24h 活跃度作为当前基准日消耗率
        let dailyTokens = max(0, tokens24h)
        let dailyCost = max(0, cost24h)

        let projectedMonthEndTokens = SafeNumber.product(dailyTokens, totalDays)
        let projectedMonthEndCost = SafeNumber.costProduct(dailyCost, totalDays)

        var exhaustionDay: Int? = nil
        var summary: String = ""

        if dailyTokens <= 0 {
            summary = "近期暂无活跃消耗，月末预估平稳"
        } else if dailyBudget > 0 {
            let monthlyBudget = SafeNumber.product(dailyBudget, totalDays)
            if dailyTokens > dailyBudget {
                // 超出日均预算，推算何时耗尽月度总池
                let days = max(1, monthlyBudget / max(1, dailyTokens))
                if days < totalDays {
                    exhaustionDay = days
                    summary = "按当前增速，预估本月第 \(days) 天将耗尽月度预算配额"
                } else {
                    summary = "按当前增速，预计月末消耗 \(TokenUsage.compact(projectedMonthEndTokens)) tokens"
                }
            } else {
                let usageRatio = Int((Double(projectedMonthEndTokens) / Double(max(1, monthlyBudget))) * 100)
                summary = "预算健康，预计月末使用率约为 \(usageRatio)%"
            }
        } else {
            let costStr = projectedMonthEndCost > 0 ? " (\(TokenUsage.cost(projectedMonthEndCost)))" : ""
            summary = "按当前增速，预计月末总消耗 \(TokenUsage.compact(projectedMonthEndTokens)) tokens\(costStr)"
        }

        return TokenForecastReport(
            projectedMonthEndTokens: projectedMonthEndTokens,
            projectedMonthEndCost: projectedMonthEndCost,
            daysRemainingInMonth: daysRemaining,
            totalDaysInMonth: totalDays,
            budgetExhaustionDay: exhaustionDay,
            forecastSummary: summary
        )
    }
}
