import AgentIslandCore
import SwiftUI

// MARK: - 模型 Token 消耗环形占比图 (v0.0.73)

struct ModelDonutChartView: View {
    let models: [ModelUsage]
    @Environment(\.colorScheme) private var colorScheme
    @State private var hoveredModelId: String? = nil

    private static let palette: [Color] = [
        Theme.sydedockCyan,
        Theme.sydedockEmerald,
        Theme.sydedockAmber,
        Color(hex: 0xa855f7), // 紫
        Color(hex: 0x3b82f6), // 蓝
        Color(hex: 0xec4899), // 粉
        Color(hex: 0x64748b)  // 灰
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
                            .stroke(colorScheme == .light ? Color(hex: 0xe2e8f0) : Theme.obsidianHairline, lineWidth: 8)
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
                                .font(Theme.bodyFont(7.5))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                    .frame(width: 64, height: 64)

                    // 图例与占比列表
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(slices.prefix(4)) { slice in
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
                                Text(String(format: "%.0f%%", slice.percentage))
                                    .font(Theme.monoDigitFont(8.5, weight: .semibold))
                                    .foregroundColor(slice.color)
                                Text(TokenUsage.compact(slice.model.tokens))
                                    .font(Theme.monoDigitFont(8.5))
                                    .foregroundColor(Theme.onDarkFaint)
                            }
                            .contentShape(Rectangle())
                            .onHover { isHover in
                                hoveredModelId = isHover ? slice.id : nil
                            }
                        }
                    }
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
        }
    }
}
