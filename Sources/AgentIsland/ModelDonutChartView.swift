import AgentIslandCore
import SwiftUI

// MARK: - 模型 Token 消耗环形占比图 (v0.0.73)

struct ModelDonutChartView: View {
    let models: [ModelUsage]
    @Environment(\.colorScheme) private var colorScheme
    @State private var hoveredModelId: String? = nil

    /// 调色板必须走 Theme 的动态浅/深成对色（Theme.swift 的强调色规则）：
    /// 浅色模式加深、深色模式高亮，硬编码 hex 在浅色背景下对比度不达标
    private static let palette: [Color] = [
        Theme.sydedockCyan,
        Theme.sydedockEmerald,
        Theme.sydedockAmber,
        Color(dynamicLight: 0x7c3aed, dark: 0xa855f7), // 紫
        Theme.sydedockBlue,
        Color(dynamicLight: 0xdb2777, dark: 0xec4899), // 粉
        Color(dynamicLight: Ramp.slate600Hex, dark: Ramp.slate500Hex)  // 灰
    ]

    private struct Slice: Identifiable {
        let id: String
        let model: ModelUsage
        let startRatio: Double
        let endRatio: Double
        let color: Color
        let percentage: Double
    }

    private var totalTokens: Int {
        models.reduce(0) { $0 + $1.tokens }
    }

    /// 图例取舍口径（含「其余」合并）收敛到 ModelDonutAggregate，可被测试 runner 断言
    private var breakdown: ModelDonutAggregate.Breakdown {
        ModelDonutAggregate.breakdown(models.map {
            ModelDonutAggregate.Entry(modelId: $0.modelId, tokens: $0.tokens)
        })
    }

    private var slices: [Slice] {
        let total = Double(totalTokens)
        guard total > 0 else { return [] }

        var current = 0.0
        var result: [Slice] = []
        for (i, m) in models.enumerated() {
            let ratio = Double(m.tokens) / total
            let color = Self.palette[i % Self.palette.count]
            result.append(Slice(
                id: m.modelId,
                model: m,
                startRatio: current,
                endRatio: current + ratio,
                color: color,
                percentage: ratio * 100
            ))
            current += ratio
        }
        return result
    }

    var body: some View {
        if totalTokens > 0 && !models.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    // 环形图
                    ZStack {
                        // 底底槽
                        Circle()
                            .stroke(colorScheme == .light ? Ramp.slate200 : Theme.obsidianHairline, lineWidth: 8)
                            .frame(width: 56, height: 56)

                        // 扇形切片
                        ForEach(slices) { slice in
                            Circle()
                                .trim(from: CGFloat(slice.startRatio), to: CGFloat(slice.endRatio))
                                .stroke(slice.color, style: StrokeStyle(lineWidth: hoveredModelId == slice.id ? 10 : 8, lineCap: .butt))
                                .rotationEffect(.degrees(-90))
                                .frame(width: 56, height: 56)
                                .scaleEffect(hoveredModelId == slice.id ? 1.05 : 1.0)
                                .animation(.spring(response: 0.2, dampingFraction: 0.7), value: hoveredModelId)
                        }

                        // 中心统计
                        VStack(spacing: 0) {
                            Text("\(models.count)")
                                .font(Theme.monoDigitFont(11, weight: .bold))
                                .foregroundColor(Theme.onDark)
                            Text("模型")
                                .font(Theme.bodyFont(9))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                    .frame(width: 64, height: 64)

                    // 图例与占比列表
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(slices.prefix(breakdown.shownCount)) { slice in
                            HStack(spacing: 5) {
                                Circle()
                                    .fill(slice.color)
                                    .frame(width: 5, height: 5)
                                Text(slice.model.modelId)
                                    .font(Theme.monoFont(9))
                                    .foregroundColor(hoveredModelId == slice.id ? Theme.onDark : Theme.onDarkMuted)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                Spacer()
                                Text(ModelDonutAggregate.Breakdown.percentText(slice.percentage))
                                    .font(Theme.monoDigitFont(9, weight: .semibold))
                                    .foregroundColor(slice.color)
                                Text(TokenUsage.compact(slice.model.tokens))
                                    .font(Theme.monoDigitFont(9))
                                    .foregroundColor(Theme.onDarkFaint)
                            }
                            .contentShape(Rectangle())
                            .onHover { isHover in
                                hoveredModelId = isHover ? slice.id : nil
                            }
                        }

                        // 被折进「其余」的模型必须显式出现，否则图例百分比之和不到 100%
                        if breakdown.hasHidden {
                            HStack(spacing: 5) {
                                Circle()
                                    .fill(Color(dynamicLight: Ramp.slate400Hex, dark: Ramp.slate600Hex))
                                    .frame(width: 5, height: 5)
                                Text(breakdown.hiddenText)
                                    .font(Theme.monoFont(9))
                                    .foregroundColor(Theme.onDarkFaint)
                                    .lineLimit(1)
                                Spacer()
                                Text(TokenUsage.compact(breakdown.hiddenTokens))
                                    .font(Theme.monoDigitFont(9))
                                    .foregroundColor(Theme.onDarkFaint)
                            }
                            .help(breakdown.hiddenDetailText)
                        }
                    }
                }

                // 图表口径标注：总数 + 遗漏说明，读图时不必自己换算
                VStack(alignment: .leading, spacing: 3) {
                    DarkDivider()
                    Text("总计 \(models.count) 个模型 · \(TokenUsage.compact(totalTokens)) tokens"
                         + (breakdown.hasHidden ? " · 图例仅列前 \(breakdown.shownCount) 项" : ""))
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }
            .padding(8)
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Theme.obsidianCardFill)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .strokeBorder(Theme.obsidianHairline, lineWidth: 0.5)
                    )
            )
            // 环与图例都是自绘内容，VoiceOver 读不到任何信息，收敛成一句占比播报
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(accessibilitySummary)
        }
    }

    private var accessibilitySummary: String {
        let shown = slices.prefix(breakdown.shownCount).map { slice in
            "\(slice.model.modelId) \(ModelDonutAggregate.Breakdown.percentText(slice.percentage))"
        }.joined(separator: "，")
        var text = "按模型占比：\(shown)，总计 \(totalTokens) Token"
        if breakdown.hasHidden {
            text += "，其余 \(breakdown.hiddenCount) 个模型合计 \(ModelDonutAggregate.Breakdown.percentText(breakdown.hiddenPercent))"
        }
        return text
    }
}
