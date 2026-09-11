import AgentIslandCore
import SwiftUI

// MARK: - Token 汇总栏（卡片底部，双口径）

struct TokenSummaryBar: View {
    let total: TokenUsage

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "chart.bar.fill")
                .font(.system(size: 9, weight: .semibold))
                .foregroundColor(Theme.onDarkFaint)
            Text("Token 24h \(TokenUsage.compact(total.tokens24h))")
                .font(Theme.monoFont(10, weight: .semibold))
                .foregroundColor(Theme.onDark.opacity(0.85))
                .lineLimit(1)
                .help("24h Token 用量")
            if let cost = cost24hText {
                Text(cost)
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
                    .help("24h 花费")
            }
            Spacer()
            Text("累计 \(TokenUsage.compact(total.tokensTotal))")
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.onDarkFaint)
                .lineLimit(1)
                .help("累计 Token 用量")
            if !TokenUsage.cost(total.costTotal).isEmpty {
                Text(TokenUsage.cost(total.costTotal))
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 7)
    }

    private var cost24hText: String? {
        let c = TokenUsage.cost(total.cost24h)
        return c.isEmpty ? nil : c
    }
}
