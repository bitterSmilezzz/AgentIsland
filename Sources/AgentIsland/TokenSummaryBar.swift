import AgentIslandCore
import SwiftUI

// MARK: - Token 汇总栏（卡片底部，双口径）

struct TokenSummaryBar: View {
    let total: TokenUsage
    /// 默认空动作保留布局探针/预览等只读调用点；现网主卡始终传入分析页导航。
    let onOpen: () -> Void

    @State private var isHovered = false

    init(total: TokenUsage, onOpen: @escaping () -> Void = {}) {
        self.total = total
        self.onOpen = onOpen
    }

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 8) {
                // 24h 指标跑道微胶囊（高对比亮色）
                HStack(spacing: 5) {
                    Text("24H")
                        .font(Theme.badgeFont(.bold))
                        .foregroundColor(Theme.sydedockCyan)
                        .padding(.horizontal, 4.5)
                        .padding(.vertical, 1.5)
                        .background(
                            RoundedRectangle(cornerRadius: 3.5, style: .continuous)
                                .fill(Theme.sydedockCyan.opacity(0.16))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 3.5, style: .continuous)
                                        .strokeBorder(Theme.sydedockCyan.opacity(0.35), lineWidth: 0.5)
                                )
                        )
                    Text(TokenUsage.compact(total.tokens24h))
                        .font(Theme.monoDigitFont(10.5, weight: .bold))
                        .foregroundColor(Theme.onDark)
                        .lineLimit(1)
                        .help("24h Token 用量")
                    if let cost = cost24hText {
                        Text(cost)
                            .font(Theme.monoDigitFont(9.5, weight: .semibold))
                            .foregroundColor(Theme.sydedockAmber)
                            .lineLimit(1)
                            .help("24h 花费")
                    }
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 2.5)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.06))
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .strokeBorder(Theme.obsidianHairline, lineWidth: 0.5)
                        )
                )

                Spacer()

                // 累计指标跑道微胶囊（柔和银白次要层次）
                HStack(spacing: 4.5) {
                    Text("TOTAL")
                        .font(Theme.badgeFont(.semibold))
                        .foregroundColor(Theme.onDarkMuted)
                        .padding(.horizontal, 3.5)
                        .padding(.vertical, 1)
                        .background(
                            RoundedRectangle(cornerRadius: 3, style: .continuous)
                                .fill(Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.06))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 3, style: .continuous)
                                        .strokeBorder(Theme.obsidianHairline, lineWidth: 0.5)
                                )
                        )
                    Text(TokenUsage.compact(total.tokensTotal))
                        .font(Theme.monoDigitFont(10, weight: .semibold))
                        .foregroundColor(Theme.onDark.opacity(0.92))
                        .lineLimit(1)
                        .help("累计 Token 用量")
                    if !TokenUsage.cost(total.costTotal).isEmpty {
                        Text(TokenUsage.cost(total.costTotal))
                            .font(Theme.monoDigitFont(9.5, weight: .medium))
                            .foregroundColor(Theme.sydedockAmber.opacity(0.90))
                            .lineLimit(1)
                            .help("累计花费")
                            .layoutPriority(1)
                    }
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 2.5)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.045))
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .strokeBorder(Theme.obsidianHairline, lineWidth: 0.5)
                        )
                )

                Image(systemName: "chevron.right")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundColor(isHovered ? Theme.sydedockCyan : Theme.onDarkFaint)
                    .offset(x: isHovered ? 1.5 : 0)
                    .animation(.easeOut(duration: 0.15), value: isHovered)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 3.5)
            .background(
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(isHovered ? Theme.obsidianCardHoverFill : Theme.obsidianCardFill)
                    .overlay(
                        RoundedRectangle(cornerRadius: 9, style: .continuous)
                            .strokeBorder(
                                LinearGradient(
                                    colors: [
                                        Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(isHovered ? 0.26 : 0.16),
                                        Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(isHovered ? 0.08 : 0.03)
                                    ],
                                    startPoint: .top,
                                    endPoint: .bottom
                                ),
                                lineWidth: 0.75
                            )
                    )
            )
            .contentShape(RoundedRectangle(cornerRadius: 9, style: .continuous))
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 6)
        .padding(.vertical, 3.5)
        .onHover { hovering in
            withAnimation(.easeOut(duration: 0.15)) {
                isHovered = hovering
            }
        }
        .help("打开 Token 时间分析")
        .accessibilityLabel("打开 Token 时间分析，24 小时 \(TokenUsage.compact(total.tokens24h))，累计 \(TokenUsage.compact(total.tokensTotal))")
    }

    private var cost24hText: String? {
        let c = TokenUsage.cost(total.cost24h)
        return c.isEmpty ? nil : c
    }
}
