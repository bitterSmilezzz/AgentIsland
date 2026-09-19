import AgentIslandCore
import SwiftUI

// MARK: - 24 小时活跃节律热力图

struct ActivityHeatmapView: View {
    let points: [TokenUsagePoint]
    @Environment(\.colorScheme) private var colorScheme
    @State private var hoveredHour: Int? = nil

    /// 「本时段没读到任何明细」与「读到了、用量为 0」是两种状态（CONTEXT.md「Token 数据覆盖」）：
    /// points 为空时不能画一张全灰的 24 格图并写「活跃 0/24h」——那是把缺失数据伪装成零用量。
    private var hasDetail: Bool { !points.isEmpty }

    private var hourlyData: [(hour: Int, tokens: Int, cost: Double)] {
        let calendar = Calendar.current
        var dict: [Int: (tokens: Int, cost: Double)] = [:]
        for p in points {
            let h = calendar.component(.hour, from: p.start)
            let prev = dict[h] ?? (0, 0.0)
            dict[h] = (prev.tokens + p.tokens, prev.cost + p.cost)
        }
        return (0..<24).map { h in
            let item = dict[h] ?? (0, 0.0)
            return (h, item.tokens, item.cost)
        }
    }

    private var maxTokens: Int {
        max(hourlyData.map(\.tokens).max() ?? 0, 1)
    }

    private var activeHours: Int {
        hourlyData.filter { $0.tokens > 0 }.count
    }

    private func cellColor(tokens: Int) -> Color {
        guard tokens > 0 else {
            return colorScheme == .light ? Color(hex: 0xe2e8f0) : Theme.obsidianHairline.opacity(0.6)
        }
        let ratio = min(1.0, Double(tokens) / Double(maxTokens))
        if ratio > 0.75 {
            return Theme.sydedockCyan
        } else if ratio > 0.4 {
            return Theme.sydedockCyan.opacity(0.7)
        } else if ratio > 0.15 {
            return Theme.sydedockCyan.opacity(0.45)
        } else {
            return Theme.sydedockCyan.opacity(0.25)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if hasDetail {
                header
                grid
                clockAnchors
            } else {
                noDetailState
            }
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(colorScheme == .light ? Color.white : Theme.obsidianCardFill)
                .overlay(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(colorScheme == .light ? Color(hex: 0xe2e8f0) : Theme.obsidianHairline, lineWidth: 0.5)
                )
        )
        // 自绘网格对 VoiceOver 完全不可见，汇总成一句可读结论（与趋势图同一处理标准）
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilitySummary)
    }

    private var header: some View {
        HStack {
            Text("24h 协同节律")
                .font(Theme.bodyFont(10, weight: .semibold))
                .foregroundColor(Theme.onDarkFaint)
            Spacer()
            if let h = hoveredHour, let item = hourlyData.first(where: { $0.hour == h }) {
                Text("\(String(format: "%02d:00", h))–\(String(format: "%02d:59", h))  \(TokenUsage.compact(item.tokens))")
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.sydedockCyan)
                    .transition(.opacity)
            } else {
                Text("活跃 \(activeHours)/24h")
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
            }
        }
    }

    /// 无明细态：沿用来源卡「未发现明细」的措辞与弱化样式，两处口径不分叉
    private var noDetailState: some View {
        HStack(spacing: 6) {
            Image(systemName: "questionmark.square.dashed")
                .font(.system(size: 10, weight: .semibold))
                .foregroundColor(Theme.onDarkFaint)
            VStack(alignment: .leading, spacing: 2) {
                Text("24h 协同节律")
                    .font(Theme.bodyFont(10, weight: .semibold))
                    .foregroundColor(Theme.onDarkFaint)
                Text("本时段未发现 Token 明细")
                    .font(Theme.bodyFont(9.5))
                    .foregroundColor(Theme.onDarkMuted)
            }
            Spacer()
        }
        .padding(.vertical, 4)
    }

    private var grid: some View {
        // 24 格热力图网格
        HStack(spacing: 3) {
            ForEach(hourlyData, id: \.hour) { item in
                RoundedRectangle(cornerRadius: 2, style: .continuous)
                    .fill(cellColor(tokens: item.tokens))
                    .frame(maxWidth: .infinity, minHeight: 14, maxHeight: 14)
                    .overlay(
                        RoundedRectangle(cornerRadius: 2, style: .continuous)
                            .stroke(hoveredHour == item.hour ? Color.white.opacity(0.8) : Color.clear, lineWidth: 1)
                    )
                    .onHover { isHover in
                        if isHover {
                            hoveredHour = item.hour
                        } else if hoveredHour == item.hour {
                            hoveredHour = nil
                        }
                    }
            }
        }
    }

    private var clockAnchors: some View {
        // 时钟锚点标注（9pt 为字号下限，见 Theme.badgeFont 的 HIG 说明）
        HStack {
            Text("00:00")
            Spacer()
            Text("06:00")
            Spacer()
            Text("12:00")
            Spacer()
            Text("18:00")
            Spacer()
            Text("23:00")
        }
        .font(Theme.monoFont(9))
        .foregroundColor(Theme.onDarkFaint.opacity(0.8))
    }

    private var accessibilitySummary: String {
        guard hasDetail else {
            return "24 小时协同节律，本时段未发现 Token 明细"
        }
        let peak = hourlyData.map(\.tokens).max() ?? 0
        let total = hourlyData.reduce(0) { $0 + $1.tokens }
        return "最近 24 小时活跃 \(activeHours)/24 小时，峰值 \(TokenUsage.compact(peak)) tokens，合计 \(TokenUsage.compact(total)) tokens"
    }
}
