import AgentIslandCore
import SwiftUI

// MARK: - Token 时间分析页

struct TokenAnalyticsView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    @State private var range: TokenTimeRange = .day
    @State private var timeline = TokenUsageTimeline.empty(for: .day)
    @State private var hasLoadedOnce = false
    @State private var cachedTimelines: [TokenTimeRange: TokenUsageTimeline] = [:]
    @State private var queryToken = UUID()
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(
                title: "Token 用量",
                subtitle: "净消耗 · 不含缓存读取",
                onBack: { controller.route = .list },
                controller: controller
            )

            DarkDivider()

            ScrollView(.vertical, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 10) {
                    TokenRangePicker(selection: $range)

                    if !hasLoadedOnce {
                        TokenAnalyticsSkeleton()
                            .transition(.opacity)
                    } else {
                        metricsCard
                        trendCard
                        sourceCard
                        methodologyNote
                    }
                }
                .padding(.horizontal, Theme.pageMargin)
                .padding(.vertical, 10)
            }
        }
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
        .task(id: range) { loadTimeline() }
    }

    private var metricsCard: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(alignment: .firstTextBaseline) {
                Text("总体用量")
                    .font(Theme.bodyFont(10.5, weight: .bold))
                    .foregroundColor(Theme.onDark)
                Spacer()
                Text("汇总 \(availableToolCount) 个可读工具")
                    .font(Theme.badgeFont(.medium))
                    .foregroundColor(Theme.onDarkFaint)
            }

            HStack(spacing: 0) {
                TokenMetricCell(label: range.metricLabel,
                                value: TokenUsage.compact(timeline.tokens),
                                fullValue: timeline.tokens.formatted())
                metricDivider
                TokenMetricCell(label: "费用", value: costText(timeline.cost), isCost: true)
                metricDivider
                TokenMetricCell(label: "累计", value: TokenUsage.compact(engine.grandTotal.tokensTotal),
                                fullValue: engine.grandTotal.tokensTotal.formatted())
            }

            Rectangle()
                .fill(Theme.onDark.opacity(0.08))
                .frame(height: 1)

            HStack(spacing: 5) {
                Image(systemName: comparison.icon)
                    .font(.system(size: 9.5, weight: .bold))
                Text(comparison.text)
                    .font(Theme.bodyFont(9.5, weight: .semibold))
                    .contentTransition(.opacity)
                Spacer()
                Text("上一周期 \(TokenUsage.compact(timeline.previousTokens))")
                    .font(Theme.monoDigitFont(9, weight: .medium))
                    .foregroundColor(Theme.onDarkFaint)
                    .contentTransition(.numericText())
            }
            .foregroundColor(comparison.tint)
            .accessibilityElement(children: .combine)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 10)
        .background(
            RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                .fill(Theme.obsidianCardFill)
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                        .strokeBorder(
                            LinearGradient(
                                colors: colorScheme == .light ? [
                                    Color.white.opacity(0.95),
                                    Color(hex: 0x000000).opacity(0.06)
                                ] : [
                                    Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.16),
                                    Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.04)
                                ],
                                startPoint: .top,
                                endPoint: .bottom
                            ),
                            lineWidth: 0.75
                        )
                )
                .shadow(color: Color.black.opacity(colorScheme == .light ? 0.025 : 0), radius: 1.5, y: 1)
        )
    }

    private var metricDivider: some View {
        Rectangle()
            .fill(
                LinearGradient(
                    colors: [
                        Theme.onDark.opacity(0.03),
                        Theme.onDark.opacity(0.18),
                        Theme.onDark.opacity(0.03)
                    ],
                    startPoint: .top,
                    endPoint: .bottom
                )
            )
            .frame(width: 1, height: 30)
    }

    private var trendCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text("使用趋势")
                    .font(Theme.bodyFont(10.5, weight: .bold))
                    .foregroundColor(Theme.onDark)
                Spacer()
                if let peak = peakPoint, peak.tokens > 0 {
                    HStack(spacing: 3.5) {
                        Circle()
                            .fill(Theme.sydedockCyan)
                            .frame(width: 4.5, height: 4.5)
                        Text("峰值 \(TokenUsage.compact(peak.tokens))")
                            .font(Theme.badgeFont(.bold))
                            .foregroundColor(Theme.sydedockCyan)
                            .contentTransition(.numericText())
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(
                        Capsule()
                            .fill(Theme.sydedockCyan.opacity(0.14))
                            .overlay(Capsule().strokeBorder(Theme.sydedockCyan.opacity(0.35), lineWidth: 0.5))
                    )
                }
            }

            if timeline.tokens == 0 {
                VStack(spacing: 5) {
                    Image(systemName: "chart.xyaxis.line")
                        .font(.system(size: 17))
                    Text("该时间范围暂无 Token 记录")
                        .font(Theme.bodyFont(10))
                }
                .foregroundColor(Theme.onDarkFaint)
                .frame(maxWidth: .infinity, minHeight: 92)
                .accessibilityElement(children: .combine)
            } else {
                TokenTrendChart(points: timeline.points, range: range)
                    .frame(height: 96)
                    .id(range)
                    .transition(.opacity)

                HStack {
                    Text(axisText(for: timeline.points.first?.start))
                    Spacer()
                    Text(axisText(for: midpoint?.start))
                    Spacer()
                    Text(axisText(for: timeline.points.last?.start))
                }
                .font(Theme.monoDigitFont(9, weight: .medium))
                .foregroundColor(Theme.onDarkMuted)
                .contentTransition(.opacity)
            }
        }
        .padding(10)
        .background(
            RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                .fill(Theme.obsidianCardFill)
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                        .strokeBorder(
                            LinearGradient(
                                colors: colorScheme == .light ? [
                                    Color.white.opacity(0.95),
                                    Color(hex: 0x000000).opacity(0.06)
                                ] : [
                                    Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.16),
                                    Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.04)
                                ],
                                startPoint: .top,
                                endPoint: .bottom
                            ),
                            lineWidth: 0.75
                        )
                )
                .shadow(color: Color.black.opacity(colorScheme == .light ? 0.025 : 0), radius: 1.5, y: 1)
        )
    }

    @ViewBuilder
    private var sourceCard: some View {
        if !timeline.sources.isEmpty {
            VStack(alignment: .leading, spacing: 7) {
                HStack(alignment: .firstTextBaseline) {
                    Text("按工具用量")
                        .font(Theme.bodyFont(10.5, weight: .bold))
                        .foregroundColor(Theme.onDark)
                    Spacer()
                    Text("\(usedToolCount) 个有记录")
                        .font(Theme.badgeFont(.medium))
                        .foregroundColor(Theme.onDarkFaint)
                }

                ForEach(timeline.sources) { source in
                    toolRow(source)
                }
            }
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                    .fill(Theme.obsidianCardFill)
                    .overlay(
                        RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                            .strokeBorder(
                                LinearGradient(
                                    colors: colorScheme == .light ? [
                                        Color.white.opacity(0.95),
                                        Color(hex: 0x000000).opacity(0.06)
                                    ] : [
                                        Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.16),
                                        Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.04)
                                    ],
                                    startPoint: .top,
                                    endPoint: .bottom
                                ),
                                lineWidth: 0.75
                            )
                    )
                    .shadow(color: Color.black.opacity(colorScheme == .light ? 0.025 : 0), radius: 1.5, y: 1)
            )
        }
    }

    private var methodologyNote: some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: "info.circle")
                .font(.system(size: 9))
            Text("总体为各工具净消耗之和；缓存读取不重复计入。未发现本地明细的工具单独标注，不会误报为 0。")
                .font(Theme.bodyFont(9))
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundColor(Theme.onDarkFaint)
        .padding(.horizontal, 2)
        .accessibilityElement(children: .combine)
    }

    private var peakPoint: TokenUsagePoint? {
        timeline.points.max { $0.tokens < $1.tokens }
    }

    private var availableToolCount: Int {
        timeline.sources.filter(\.isAvailable).count
    }

    private var usedToolCount: Int {
        timeline.sources.filter { $0.tokens > 0 }.count
    }

    private var midpoint: TokenUsagePoint? {
        guard !timeline.points.isEmpty else { return nil }
        return timeline.points[timeline.points.count / 2]
    }

    private var comparison: (icon: String, text: String, tint: Color) {
        let current = timeline.tokens
        let previous = timeline.previousTokens
        if previous == 0 {
            return current == 0
                ? ("minus", "与上一周期相同", Theme.onDarkFaint)
                : ("sparkles", "本周期开始产生用量", Theme.sydedockCyan)
        }
        let percent = Int((Double(current - previous) / Double(previous) * 100).rounded())
        if percent == 0 { return ("minus", "与上一周期基本持平", Theme.onDarkFaint) }
        let direction = percent > 0 ? "增加" : "减少"
        let icon = percent > 0 ? "arrow.up.right" : "arrow.down.right"
        let tint = percent > 0 ? Theme.sydedockAmber : Theme.sydedockEmerald
        return (icon, "较上一周期\(direction) \(abs(percent))%", tint)
    }

    private func loadTimeline() {
        let targetRange = range
        if let cached = cachedTimelines[targetRange] {
            withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) {
                timeline = cached
                hasLoadedOnce = true
            }
        }

        let token = UUID()
        queryToken = token
        engine.tokenTimeline(range: targetRange) { result in
            guard queryToken == token, result.range == targetRange else { return }
            cachedTimelines[targetRange] = result
            withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) {
                timeline = result
                hasLoadedOnce = true
            }
            preloadOtherRanges()
        }
    }

    private func preloadOtherRanges() {
        for otherRange in TokenTimeRange.allCases where otherRange != range && cachedTimelines[otherRange] == nil {
            engine.tokenTimeline(range: otherRange) { result in
                guard result.range == otherRange else { return }
                cachedTimelines[otherRange] = result
            }
        }
    }

    @ViewBuilder
    private func toolRow(_ source: TokenSourceUsage) -> some View {
        let detailTarget = TokenSourceDetailRoute.target(
            for: source,
            detailCapableAgentIDs: detailCapableAgentIDs
        )
        let canOpenDetail = detailTarget != nil
        let row = TokenSourceRow(
            name: sourceName(source.agentId),
            icon: sourceIcon(source.agentId),
            usage: source,
            total: timeline.tokens,
            showsDetailChevron: canOpenDetail
        )
        if let detailTarget {
            Button { controller.openAgentDetail(detailTarget) } label: { row }
                .buttonStyle(.plain)
                .help("查看 \(sourceName(source.agentId)) 的用量明细")
        } else {
            row.help(source.isAvailable
                     ? "\(sourceName(source.agentId)) 当前未在监控列表中；此处保留历史用量统计"
                     : "当前机器未发现 \(sourceName(source.agentId)) 的本地 Token 明细")
        }
    }

    /// 内嵌工具可以没有独立实时快照，但只要本地 Token 源可读，仍应允许进入用量详情。
    private var detailCapableAgentIDs: Set<String> {
        Set(engine.allProfiles.map(\.id)).union(AgentRegistry.builtin.map(\.id))
    }

    private func sourceName(_ id: String) -> String {
        engine.allProfiles.first { $0.id == id }?.name
            ?? AgentRegistry.profile(id: id)?.name
            ?? id
    }

    private func sourceIcon(_ id: String) -> String {
        engine.allProfiles.first { $0.id == id }?.icon
            ?? AgentRegistry.profile(id: id)?.icon
            ?? "terminal"
    }

    private func costText(_ cost: Double) -> String {
        let text = TokenUsage.cost(cost)
        return text.isEmpty ? "—" : text
    }

    private func axisText(for date: Date?) -> String {
        guard let date else { return "—" }
        switch range {
        case .day: return Self.hourFormatter.string(from: date)
        case .week, .month: return Self.dayFormatter.string(from: date)
        }
    }

    private static let hourFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm"
        return formatter
    }()

    private static let dayFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "M/d"
        return formatter
    }()
}

private struct TokenRangePicker: View {
    @Binding var selection: TokenTimeRange
    @Namespace private var pickerNamespace
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 2) {
            ForEach(TokenTimeRange.allCases) { range in
                let isSelected = selection == range
                Button {
                    withAnimation(.spring(response: 0.28, dampingFraction: 0.76)) {
                        selection = range
                    }
                } label: {
                    Text(range.pickerLabel)
                        .font(Theme.bodyFont(10.5, weight: isSelected ? .bold : .medium))
                        .foregroundColor(isSelected ? Theme.onDark : Theme.onDarkMuted)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 5)
                        .background {
                            if isSelected {
                                RoundedRectangle(cornerRadius: 6, style: .continuous)
                                    .fill(
                                        colorScheme == .light
                                            ? Color.white.opacity(0.90)
                                            : Color.white.opacity(0.16)
                                    )
                                    .overlay(
                                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                                            .strokeBorder(
                                                LinearGradient(
                                                    colors: [
                                                        Color.white.opacity(colorScheme == .light ? 0.8 : 0.30),
                                                        Color.white.opacity(colorScheme == .light ? 0.2 : 0.08)
                                                    ],
                                                    startPoint: .top,
                                                    endPoint: .bottom
                                                ),
                                                lineWidth: 0.5
                                            )
                                    )
                                    .shadow(color: Color.black.opacity(colorScheme == .light ? 0.08 : 0.25), radius: 2, y: 1)
                                    .matchedGeometryEffect(id: "range_picker_active_pill", in: pickerNamespace)
                            }
                        }
                }
                .buttonStyle(.plain)
                .accessibilityLabel("查看 \(range.accessibilityLabel) Token 用量")
                .accessibilityValue(isSelected ? "已选择" : "未选择")
            }
        }
        .padding(2.5)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(colorScheme == .light ? Color(hex: 0xf1f5f9) : Color.white.opacity(0.06))
                .overlay(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(colorScheme == .light ? Color(hex: 0xe2e8f0) : Theme.obsidianHairline, lineWidth: 0.5)
                )
        )
    }
}

// MARK: - 趋势图

private struct TokenTrendChart: View {
    let points: [TokenUsagePoint]
    let range: TokenTimeRange
    @State private var hoveredIndex: Int? = nil

    private static let timeTooltipFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f
    }()

    private static let dateTooltipFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "M月d日"
        return f
    }()

    private func tooltipTimeText(for point: TokenUsagePoint) -> String {
        switch range {
        case .day:
            let start = Self.timeTooltipFormatter.string(from: point.start)
            let end = Self.timeTooltipFormatter.string(from: point.start.addingTimeInterval(3600))
            return "\(start)–\(end)"
        case .week, .month:
            return Self.dateTooltipFormatter.string(from: point.start)
        }
    }

    var body: some View {
        GeometryReader { geo in
            let maximum = max(points.map(\.tokens).max() ?? 0, 1)
            let coordinates = points.enumerated().map { index, point in
                CGPoint(
                    x: points.count > 1
                        ? geo.size.width * CGFloat(index) / CGFloat(points.count - 1)
                        : geo.size.width / 2,
                    y: geo.size.height - (CGFloat(point.tokens) / CGFloat(maximum)) * (geo.size.height - 10) - 2
                )
            }

            ZStack {
                // 网格线
                VStack(spacing: 0) {
                    ForEach(0..<4, id: \.self) { index in
                        Rectangle()
                            .fill(Theme.onDark.opacity(index == 3 ? 0.16 : 0.08))
                            .frame(height: 0.5)
                        if index < 3 { Spacer() }
                    }
                }

                // 面积渐变
                Path { path in
                    guard let first = coordinates.first, let last = coordinates.last else { return }
                    path.move(to: CGPoint(x: first.x, y: geo.size.height))
                    path.addLine(to: first)
                    for point in coordinates.dropFirst() { path.addLine(to: point) }
                    path.addLine(to: CGPoint(x: last.x, y: geo.size.height))
                    path.closeSubpath()
                }
                .fill(
                    LinearGradient(
                        colors: [
                            Theme.sydedockCyan.opacity(0.25),
                            Theme.sydedockCyan.opacity(0.02)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )

                // 趋势线
                Path { path in
                    guard let first = coordinates.first else { return }
                    path.move(to: first)
                    for point in coordinates.dropFirst() { path.addLine(to: point) }
                }
                .stroke(
                    Theme.sydedockCyan,
                    style: StrokeStyle(lineWidth: 2.0, lineCap: .round, lineJoin: .round)
                )

                // 峰值高亮圈
                if let peakIndex = points.indices.max(by: { points[$0].tokens < points[$1].tokens }),
                   coordinates.indices.contains(peakIndex),
                   points[peakIndex].tokens > 0 {
                    ZStack {
                        Circle()
                            .fill(Theme.sydedockCyan.opacity(0.35))
                            .frame(width: 10, height: 10)
                        Circle()
                            .fill(Theme.sydedockCyan)
                            .frame(width: 5.5, height: 5.5)
                        Circle()
                            .fill(Color.white)
                            .frame(width: 2.5, height: 2.5)
                    }
                    .position(coordinates[peakIndex])
                }

                // 交互式游标垂直标尺与浮动标签
                if let activeIndex = hoveredIndex,
                   coordinates.indices.contains(activeIndex) {
                    let point = points[activeIndex]
                    let coord = coordinates[activeIndex]

                    // 垂直参考线
                    Path { path in
                        path.move(to: CGPoint(x: coord.x, y: 0))
                        path.addLine(to: CGPoint(x: coord.x, y: geo.size.height))
                    }
                    .stroke(Theme.sydedockCyan.opacity(0.55), style: StrokeStyle(lineWidth: 1, dash: [2.5, 2.5]))

                    // 游标焦点圆点
                    Circle()
                        .fill(Theme.sydedockCyan)
                        .frame(width: 7, height: 7)
                        .overlay(Circle().stroke(Color.white, lineWidth: 1.5))
                        .position(coord)

                    // 浮动提示微徽标
                    HStack(spacing: 4) {
                        Text(tooltipTimeText(for: point))
                            .font(Theme.monoFont(8.5, weight: .medium))
                            .foregroundColor(Theme.onDarkMuted)
                        Text(TokenUsage.compact(point.tokens))
                            .font(Theme.monoDigitFont(9, weight: .bold))
                            .foregroundColor(Theme.sydedockCyan)
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2.5)
                    .background(
                        Capsule()
                            .fill(Theme.cardFill.opacity(0.95))
                            .overlay(Capsule().strokeBorder(Theme.sydedockCyan.opacity(0.4), lineWidth: 0.5))
                            .shadow(color: Color.black.opacity(0.2), radius: 3, y: 1)
                    )
                }
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        guard points.count > 1 else {
                            hoveredIndex = 0
                            return
                        }
                        let step = geo.size.width / CGFloat(points.count - 1)
                        let idx = Int(round(value.location.x / step))
                        hoveredIndex = min(max(idx, 0), points.count - 1)
                    }
                    .onEnded { _ in
                        Task { @MainActor in
                            try? await Task.sleep(nanoseconds: 1_200_000_000)
                            hoveredIndex = nil
                        }
                    }
            )
            .onContinuousHover { phase in
                switch phase {
                case .active(let location):
                    guard points.count > 1 else {
                        hoveredIndex = 0
                        return
                    }
                    let step = geo.size.width / CGFloat(points.count - 1)
                    let idx = Int(round(location.x / step))
                    hoveredIndex = min(max(idx, 0), points.count - 1)
                case .ended:
                    hoveredIndex = nil
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilitySummary)
    }

    private var accessibilitySummary: String {
        let total = points.reduce(0) { $0 + $1.tokens }
        let peak = points.map(\.tokens).max() ?? 0
        return "\(range.accessibilityLabel) Token 趋势，总量 \(total)，单个时间段峰值 \(peak)"
    }
}

// MARK: - 指标与来源

private struct TokenMetricCell: View {
    let label: String
    let value: String
    var fullValue: String? = nil
    var isCost: Bool = false

    var body: some View {
        VStack(spacing: 3) {
            Text(value)
                .font(Theme.monoDigitFont(15, weight: .bold))
                .foregroundColor(isCost ? Theme.sydedockAmber : Theme.onDark)
                .lineLimit(1)
                .minimumScaleFactor(0.75)
                .contentTransition(.numericText())
            Text(label)
                .font(Theme.bodyFont(9.5, weight: .medium))
                .foregroundColor(Theme.onDarkMuted)
        }
        .frame(maxWidth: .infinity)
        .help("\(label)：\(fullValue ?? value)")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(label)：\(fullValue ?? value)")
    }
}

private struct TokenSourceRow: View {
    let name: String
    let icon: String
    let usage: TokenSourceUsage
    let total: Int
    let showsDetailChevron: Bool
    @State private var isHovered = false
    @Environment(\.colorScheme) private var colorScheme

    private var ratio: Double {
        guard total > 0 else { return 0 }
        return min(max(Double(usage.tokens) / Double(total), 0), 1)
    }

    var body: some View {
        VStack(spacing: 5) {
            HStack(spacing: 7) {
                ZStack {
                    RoundedRectangle(cornerRadius: 4.5, style: .continuous)
                        .fill(usage.isAvailable ? Theme.sydedockCyan.opacity(0.14) : Theme.chipFill)
                    Image(systemName: icon)
                        .font(.system(size: 9.5, weight: .semibold))
                        .foregroundColor(usage.isAvailable ? Theme.sydedockCyan : Theme.onDarkFaint)
                }
                .frame(width: 18, height: 18)

                Text(name)
                    .font(Theme.bodyFont(10.5, weight: .semibold))
                    .foregroundColor(usage.isAvailable ? Theme.onDark : Theme.onDarkMuted)
                    .readableSingleLine(fullText: name, minWidth: 72, priority: 2)
                Spacer()
                if usage.isAvailable {
                    VStack(alignment: .trailing, spacing: 1) {
                        Text(TokenUsage.compact(usage.tokens))
                            .font(Theme.monoDigitFont(10, weight: .bold))
                            .foregroundColor(Theme.onDark)
                            .contentTransition(.numericText())
                        if !TokenUsage.cost(usage.cost).isEmpty {
                            Text(TokenUsage.cost(usage.cost))
                                .font(Theme.monoDigitFont(9, weight: .medium))
                                .foregroundColor(Theme.sydedockAmber)
                        }
                    }
                    Text("\(Int((ratio * 100).rounded()))%")
                        .font(Theme.monoDigitFont(9.5, weight: .semibold))
                        .foregroundColor(Theme.onDarkMuted)
                        .frame(width: 30, alignment: .trailing)
                        .contentTransition(.numericText())
                    if showsDetailChevron {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 8, weight: .bold))
                            .foregroundColor(isHovered ? Theme.sydedockCyan : Theme.onDarkFaint)
                            .offset(x: isHovered ? 1.5 : 0)
                            .animation(.easeOut(duration: 0.15), value: isHovered)
                    }
                } else {
                    Text("未发现明细")
                        .font(Theme.bodyFont(8.5, weight: .medium))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }

            if usage.isAvailable {
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(colorScheme == .light ? Color(hex: 0xe2e8f0) : Color.white.opacity(0.08))
                        Capsule()
                            .fill(Theme.trackGradient)
                            .frame(width: max(geo.size.width * ratio, ratio > 0 ? 4 : 0))
                            .animation(.spring(response: 0.35, dampingFraction: 0.8), value: ratio)
                    }
                }
                .frame(height: 3.5)
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .fill((isHovered && showsDetailChevron) ? Theme.obsidianCardHoverFill : Color.clear)
                .overlay(
                    RoundedRectangle(cornerRadius: 7, style: .continuous)
                        .strokeBorder(colorScheme == .light && isHovered && showsDetailChevron ? Color(hex: 0xe2e8f0) : Color.clear, lineWidth: 0.5)
                )
        )
        .contentShape(Rectangle())
        .onHover { hovering in
            withAnimation(.easeOut(duration: 0.15)) {
                isHovered = hovering
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(usage.isAvailable
                            ? "\(name)，\(usage.tokens.formatted()) Token，占 \(Int((ratio * 100).rounded()))%"
                            : "\(name)，当前机器未发现本地 Token 明细")
    }
}

private struct TokenAnalyticsSkeleton: View {
    var body: some View {
        VStack(spacing: 10) {
            RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.cardFill).frame(height: 96)
            RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.cardFill).frame(height: 138)
            RoundedRectangle(cornerRadius: Theme.radiusMd).fill(Theme.cardFill).frame(height: 62)
        }
        .opacity(0.65)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("正在加载 Token 时间分析")
    }
}

private extension TokenTimeRange {
    var pickerLabel: String {
        switch self {
        case .day: return "24h"
        case .week: return "7天"
        case .month: return "30天"
        }
    }

    var metricLabel: String { pickerLabel + " 用量" }

    var accessibilityLabel: String {
        switch self {
        case .day: return "最近 24 小时"
        case .week: return "最近 7 天"
        case .month: return "最近 30 天"
        }
    }
}
