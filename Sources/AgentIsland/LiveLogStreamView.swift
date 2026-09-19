import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 智能体实时事件流与日志抽屉视图 (LiveLogStreamView · v1.7.8)

struct LiveLogStreamView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    let agentId: String

    enum LogFilter: String, CaseIterable, Identifiable {
        case all = "全部"
        case errors = "异常/报错"
        case tools = "工具/执行"
        case edits = "文件编辑"
        case reasoning = "思考/消息"
        case info = "系统"

        var id: String { rawValue }

        func matches(_ event: AgentLogEvent) -> Bool {
            switch self {
            case .all: return true
            case .errors:
                return LogPatternAnalyzer.analyze(title: event.title, detail: event.detail) != nil
            case .tools: return event.kind == .toolCall || event.kind == .command
            case .edits: return event.kind == .fileEdit
            case .reasoning: return event.kind == .thinking || event.kind == .message
            case .info: return event.kind == .info
            }
        }
    }

    @State private var events: [AgentLogEvent] = []
    @State private var selectedFilter: LogFilter = .all
    @State private var loading = true
    @State private var autoRefresh = true
    @State private var copiedFeedback = false
    @State private var expandedEventId: String? = nil
    @State private var timer: Timer?
    /// 日志解析可能超过 2 秒；合并在途请求，避免旧结果覆盖新结果和并发扫盘。
    @State private var refreshInFlight = false
    @Environment(\.colorScheme) private var colorScheme

    private var snapshot: AgentSnapshot? {
        engine.snapshots.first { $0.id == agentId }
    }

    private var agentName: String {
        snapshot?.profile.name ?? agentId
    }

    private var filteredEvents: [AgentLogEvent] {
        events.filter { selectedFilter.matches($0) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(
                title: "\(agentName) 实时流水",
                subtitle: refreshInFlight && events.isEmpty
                    ? "实时流水 · 加载中…"
                    : "实时流水 · \(filteredEvents.count)/\(events.count) 条事件",
                onBack: { controller.closeLiveStream() },
                controller: controller
            )

            DarkDivider()

            // 顶部工具条（自动刷新控制 + 一键复制流水）
            topControlBar

            DarkDivider()

            // 分类筛选条
            filterBar

            DarkDivider()

            GeometryReader { geo in
                ScrollView(.vertical, showsIndicators: false) {
                    ScrollViewReader { proxy in
                        VStack(alignment: .leading, spacing: 6) {
                            if loading && events.isEmpty {
                                CenteredSpinner()
                                    .frame(maxWidth: .infinity, minHeight: 120)
                            } else if filteredEvents.isEmpty {
                                emptyStreamView
                            } else {
                                LazyVStack(alignment: .leading, spacing: 5) {
                                    ForEach(filteredEvents) { event in
                                        eventRow(event)
                                            .id(event.id)
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, Theme.pageMargin)
                        .padding(.vertical, 8)
                        .frame(maxWidth: .infinity, minHeight: max(geo.size.height - 16, 0))
                        .onChange(of: filteredEvents.first?.id) { firstId in
                            if let firstId, autoRefresh {
                                withAnimation(.easeInOut(duration: 0.2)) {
                                    proxy.scrollTo(firstId, anchor: .top)
                                }
                            }
                        }
                    }
                }
            }
        }
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
        .onAppear {
            refreshLogs()
            startTimer()
        }
        .onDisappear {
            stopTimer()
        }
    }

    // MARK: - 顶部工具控制条

    private var topControlBar: some View {
        HStack(spacing: 8) {
            Button {
                autoRefresh.toggle()
                if autoRefresh {
                    startTimer()
                } else {
                    stopTimer()
                }
            } label: {
                HStack(spacing: 4) {
                    Circle()
                        .fill(autoRefresh ? Theme.statusWorking : Theme.onDarkFaint)
                        .frame(width: 6, height: 6)
                    Text(autoRefresh ? "实时跟踪中" : "已暂停刷新")
                        .font(Theme.bodyFont(10, weight: .medium))
                        .foregroundColor(autoRefresh ? Theme.statusWorking : Theme.onDarkFaint)
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Capsule().fill(autoRefresh ? Theme.statusWorking.opacity(0.12) : Theme.chipFill))
            }
            .buttonStyle(.plain)
            .help("切换实时流水自动拉取（开启时每 2 秒静默更新）")

            Spacer()

            // 复制全部流水
            Button {
                copyAllLogs()
            } label: {
                HStack(spacing: 3) {
                    Image(systemName: copiedFeedback ? "checkmark" : "doc.on.doc")
                        .font(.system(size: 9))
                    Text(copiedFeedback ? "已复制" : "复制全部")
                        .font(Theme.bodyFont(10, weight: .medium))
                }
                .foregroundColor(copiedFeedback ? Theme.statusWorking : Theme.onDarkMuted)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Capsule().fill(Theme.chipFill))
            }
            .buttonStyle(.plain)
            .disabled(events.isEmpty)
            .help("将当前所有捕获的事件流水复制至系统剪贴板")

            // 手动立即刷新
            Button {
                refreshLogs()
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 9, weight: .medium))
                    .foregroundColor(Theme.onDarkFaint)
                    .padding(4)
                    .background(Circle().fill(Theme.chipFill))
            }
            .buttonStyle(.plain)
            .help("立即刷新事件流水")
            .accessibilityLabel("立即刷新事件流水")
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 5)
        .background(
            colorScheme == .light
                ? Color(hex: 0xf8fafc).opacity(0.85)
                : Color(dynamic: NSColor(hex: 0x000000, alpha: 0.04), dark: NSColor(hex: 0x000000, alpha: 0.12))
        )
    }

    // MARK: - 分类筛选条

    private var filterBar: some View {
        FilterCapsuleBar(
            items: LogFilter.allCases,
            selectedItem: $selectedFilter,
            title: { $0.rawValue },
            count: { filter in
                filter == .all ? events.count : events.filter { filter.matches($0) }.count
            },
            contentInsets: EdgeInsets(top: 4, leading: Theme.pageMargin, bottom: 4, trailing: Theme.pageMargin)
        )
        .background(colorScheme == .light ? Color(hex: 0xf1f5f9).opacity(0.6) : Color(dynamic: NSColor(hex: 0x000000, alpha: 0.02), dark: NSColor(hex: 0x000000, alpha: 0.08)))
    }

    // MARK: - 单条事件行

    private func eventRow(_ event: AgentLogEvent) -> some View {
        let isExpanded = (expandedEventId == event.id)
        let analyzedIssue = LogPatternAnalyzer.analyze(title: event.title, detail: event.detail)
        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                // 类型胶囊
                badgeView(for: event.kind)

                // 错误特征徽标 (v0.0.75)
                if let issue = analyzedIssue {
                    issueBadge(issue)
                }

                // 时间戳
                Text(Self.timeFormatter.string(from: event.timestamp))
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkFaint)

                // 标题
                Text(event.title)
                    .font(Theme.monoFont(10, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(isExpanded ? nil : 1)
                    .truncationMode(.middle)
                    .layoutPriority(2)
                    .help(event.title)
                    .accessibilityLabel(event.title)

                Spacer(minLength: 2)

                if event.detail != nil {
                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(Theme.badgeFont(.bold))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }

            // 详情区域展开
            if isExpanded, let detail = event.detail, !detail.isEmpty {
                issueDetailSection(detail: detail, issue: analyzedIssue)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(isExpanded ? Theme.hoverFill : (colorScheme == .light ? Color.white.opacity(0.92) : Color.white.opacity(0.04)))
                .overlay(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .strokeBorder(colorScheme == .light ? Color(hex: 0xe2e8f0).opacity(0.8) : Color.clear, lineWidth: 0.5)
                )
                .shadow(color: Color.black.opacity(colorScheme == .light ? 0.02 : 0), radius: 1, y: 0.5)
        )
        .contentShape(Rectangle())
        .onTapGesture {
            if event.detail != nil {
                withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                    if expandedEventId == event.id {
                        expandedEventId = nil
                    } else {
                        expandedEventId = event.id
                    }
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(event.detail != nil ? .isButton : [])
        // 不设显式 label：.combine 自然合并徽标/时间/标题；显式 label 会把
        // 合并内容（含展开后的详情正文）整体覆盖，VO 用户读不到详情
        .accessibilityHint(event.detail != nil
            ? (expandedEventId == event.id ? "收起事件详情" : "展开事件详情")
            : "")
    }

    // MARK: - 辅助组件与格式化

    private func issueBadge(_ issue: AnalyzedLogIssue) -> some View {
        HStack(spacing: 3) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 8))
            Text(issue.kind.label)
                .font(Theme.badgeFont(.bold))
        }
        .foregroundColor(Theme.dangerRed)
        .padding(.horizontal, 4)
        .padding(.vertical, 1.5)
        .background(
            Capsule()
                .fill(Theme.dangerRed.opacity(0.15))
                .overlay(Capsule().strokeBorder(Theme.dangerRed.opacity(0.3), lineWidth: 0.5))
        )
    }

    @ViewBuilder
    private func issueDetailSection(detail: String, issue: AnalyzedLogIssue?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            if let issue = issue {
                HStack {
                    Text("特征识别: \(issue.kind.label)")
                        .font(Theme.bodyFont(9, weight: .semibold))
                        .foregroundColor(Theme.dangerRed)
                    Spacer()
                    if !issue.snippet.isEmpty {
                        Button {
                            let pb = NSPasteboard.general
                            pb.clearContents()
                            pb.setString(issue.snippet, forType: .string)
                        } label: {
                            HStack(spacing: 3) {
                                Image(systemName: "doc.on.doc")
                                    .font(.system(size: 8))
                                Text("复制错误摘要")
                                    .font(Theme.monoFont(8, weight: .medium))
                            }
                            .foregroundColor(Theme.dangerRed)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 2)
                            .background(Capsule().fill(Theme.dangerRed.opacity(0.12)))
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            Text(detail)
                .font(Theme.monoFont(9))
                .foregroundColor(Theme.onDarkMuted)
                .lineSpacing(2)
        }
        .padding(6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(colorScheme == .light ? Color(hex: 0xf1f5f9) : Color(dynamic: NSColor(hex: 0x000000, alpha: 0.05), dark: NSColor(hex: 0x000000, alpha: 0.35)))
                .overlay(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .strokeBorder(colorScheme == .light ? Color(hex: 0xe2e8f0) : Color.clear, lineWidth: 0.5)
                )
        )
    }

    private func badgeView(for kind: AgentLogEvent.EventKind) -> some View {
        Text(kind.label)
            .font(Theme.badgeFont(.bold))
            .foregroundColor(badgeTextColor(for: kind))
            .padding(.horizontal, 5)
            .padding(.vertical, 1.5)
            .background(
                Capsule()
                    .fill(badgeBgColor(for: kind))
                    .overlay(
                        Capsule()
                            .strokeBorder(badgeBorderColor(for: kind), lineWidth: 0.5)
                    )
            )
    }

    private func badgeTextColor(for kind: AgentLogEvent.EventKind) -> Color {
        if colorScheme == .light {
            switch kind {
            case .command: return Color(hex: 0x047857)
            case .toolCall: return Color(hex: 0x1d4ed8)
            case .fileEdit: return Color(hex: 0xb45309)
            case .thinking: return Color(hex: 0x7e22ce)
            case .message: return Color(hex: 0x334155)
            case .info: return Color(hex: 0x64748b)
            }
        }
        switch kind {
        case .command: return Theme.statusWorking
        // 双值动态：深色玻璃上 actionBlue（0x0066cc）对比 ≈2.6:1，深色取亮蓝
        case .toolCall: return Color(dynamicLight: 0x0066cc, dark: 0x409cff)
        case .fileEdit: return Theme.warningOrange
        // 系统紫浅色白玻璃上 ≈3:1，浅色取加深紫（WCAG ≥4.5:1）
        case .thinking: return Color(dynamicLight: 0x7a1fa2, dark: 0xaf52de)
        case .message: return Theme.onDark
        case .info: return Theme.onDarkFaint
        }
    }

    private func badgeBgColor(for kind: AgentLogEvent.EventKind) -> Color {
        if colorScheme == .light {
            switch kind {
            case .command: return Color(hex: 0xecfdf5)
            case .toolCall: return Color(hex: 0xeff6ff)
            case .fileEdit: return Color(hex: 0xfffbeb)
            case .thinking: return Color(hex: 0xfaf5ff)
            case .message: return Color(hex: 0xf8fafc)
            case .info: return Color(hex: 0xf1f5f9)
            }
        }
        return badgeTextColor(for: kind).opacity(0.18)
    }

    private func badgeBorderColor(for kind: AgentLogEvent.EventKind) -> Color {
        if colorScheme == .light {
            switch kind {
            case .command: return Color(hex: 0xa7f3d0)
            case .toolCall: return Color(hex: 0xbfdbfe)
            case .fileEdit: return Color(hex: 0xfde68a)
            case .thinking: return Color(hex: 0xe9d5ff)
            case .message: return Color(hex: 0xe2e8f0)
            case .info: return Color(hex: 0xe2e8f0)
            }
        }
        return badgeTextColor(for: kind).opacity(0.35)
    }

    private var emptyStreamView: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 24))
                .foregroundColor(Theme.onDarkFaint)
            Text("暂未捕获到该智能体的实时日志")
                .font(Theme.bodyFont(11))
                .foregroundColor(Theme.onDarkFaint)
            Text("当智能体触发思考、工具调用或命令执行时将在此自动显示（最新事件在最上方）")
                .font(Theme.bodyFont(9))
                .foregroundColor(Theme.onDarkFaint.opacity(0.7))
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, minHeight: 140)
        .padding(.horizontal, 20)
    }

    // MARK: - 数据流控制

    private func refreshLogs() {
        guard !refreshInFlight else { return }
        refreshInFlight = true
        engine.fetchLogStream(agentId: agentId, limit: 30) { result in
            defer { self.refreshInFlight = false }
            guard result != self.events else {
                self.loading = false
                return
            }
            self.events = result
            self.loading = false
        }
    }

    private func startTimer() {
        stopTimer()
        timer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak engine] _ in
            Task { @MainActor in
                guard autoRefresh, let engine else { return }
                guard !refreshInFlight else { return }
                refreshInFlight = true
                engine.fetchLogStream(agentId: agentId, limit: 30) { result in
                    defer { self.refreshInFlight = false }
                    if result != self.events {
                        withAnimation(.easeInOut(duration: 0.2)) {
                            self.events = result
                        }
                    }
                }
            }
        }
    }

    private func stopTimer() {
        timer?.invalidate()
        timer = nil
    }

    private func copyAllLogs() {
        guard !events.isEmpty else { return }
        var lines: [String] = [
            "=== AgentIsland 实时日志流水: \(agentName) (\(agentId)) ===",
            "导出时间: \(Date())",
            ""
        ]
        for e in events {
            let t = Self.timeFormatter.string(from: e.timestamp)
            lines.append("[\(t)] [\(e.kind.label)] \(e.title)")
            if let d = e.detail, !d.isEmpty {
                lines.append("    \(d)")
            }
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(lines.joined(separator: "\n"), forType: .string)
        copiedFeedback = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
            copiedFeedback = false
        }
    }

    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        f.locale = Locale(identifier: "en_US_POSIX")   // 固定数字口径，防 12 小时制 locale 改写
        return f
    }()
}
