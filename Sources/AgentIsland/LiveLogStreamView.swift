import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 智能体实时事件流与日志抽屉视图 (LiveLogStreamView · v1.7.8)

struct LiveLogStreamView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    let agentId: String

    @State private var events: [AgentLogEvent] = []
    @State private var loading = true
    @State private var autoRefresh = true
    @State private var copiedFeedback = false
    @State private var expandedEventId: String? = nil
    @State private var timer: Timer?

    private var snapshot: AgentSnapshot? {
        engine.snapshots.first { $0.id == agentId }
    }

    private var agentName: String {
        snapshot?.profile.name ?? agentId
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(
                title: "\(agentName) 实时流水",
                subtitle: "Live Log Stream · \(events.count) 条事件",
                onBack: { controller.closeLiveStream() },
                onMoved: { controller.dragMoved(translation: $0) },
                onEnded: { controller.dragEnded() }
            )

            DarkDivider()

            // 顶部工具条（自动刷新控制 + 一键复制流水）
            topControlBar

            DarkDivider()

            GeometryReader { geo in
                ScrollView(.vertical, showsIndicators: true) {
                    ScrollViewReader { proxy in
                        VStack(alignment: .leading, spacing: 6) {
                            if loading && events.isEmpty {
                                CenteredSpinner()
                                    .frame(maxWidth: .infinity, minHeight: 120)
                            } else if events.isEmpty {
                                emptyStreamView
                            } else {
                                LazyVStack(alignment: .leading, spacing: 5) {
                                    ForEach(events) { event in
                                        eventRow(event)
                                            .id(event.id)
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, Theme.pageMargin)
                        .padding(.vertical, 8)
                        .frame(maxWidth: .infinity, minHeight: max(geo.size.height - 16, 0))
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
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 5)
        .background(Color.black.opacity(0.12))
    }

    // MARK: - 单条事件行

    private func eventRow(_ event: AgentLogEvent) -> some View {
        let isExpanded = (expandedEventId == event.id)
        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                // 类型胶囊
                badgeView(for: event.kind)

                // 时间戳
                Text(Self.timeFormatter.string(from: event.timestamp))
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkFaint)

                // 标题
                Text(event.title)
                    .font(Theme.monoFont(10, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(isExpanded ? nil : 1)
                    .truncationMode(.tail)

                Spacer(minLength: 2)

                if event.detail != nil {
                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }

            // 详情区域展开
            if isExpanded, let detail = event.detail, !detail.isEmpty {
                Text(detail)
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkMuted)
                    .lineSpacing(2)
                    .padding(6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .fill(Color.black.opacity(0.35))
                    )
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(isExpanded ? Theme.hoverFill : Color.white.opacity(0.03))
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
    }

    // MARK: - 辅助组件与格式化

    private func badgeView(for kind: AgentLogEvent.EventKind) -> some View {
        Text(kind.label)
            .font(Theme.monoFont(8, weight: .bold))
            .foregroundColor(badgeTextColor(for: kind))
            .padding(.horizontal, 4)
            .padding(.vertical, 1.5)
            .background(Capsule().fill(badgeBgColor(for: kind)))
    }

    private func badgeTextColor(for kind: AgentLogEvent.EventKind) -> Color {
        switch kind {
        case .command: return Theme.statusWorking
        case .toolCall: return Theme.actionBlue
        case .fileEdit: return Theme.warningOrange
        case .thinking: return Color(hex: 0xaf52de)
        case .message: return Theme.onDark
        case .info: return Theme.onDarkFaint
        }
    }

    private func badgeBgColor(for kind: AgentLogEvent.EventKind) -> Color {
        badgeTextColor(for: kind).opacity(0.18)
    }

    private var emptyStreamView: some View {
        VStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 24))
                .foregroundColor(Theme.onDarkFaint)
            Text("暂未捕获到该智能体的实时日志")
                .font(Theme.bodyFont(11))
                .foregroundColor(Theme.onDarkFaint)
            Text("当智能体触发思考、工具调用或命令执行时将在此自动滚动显示")
                .font(Theme.bodyFont(9))
                .foregroundColor(Theme.onDarkFaint.opacity(0.7))
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, minHeight: 140)
        .padding(.horizontal, 20)
    }

    // MARK: - 数据流控制

    private func refreshLogs() {
        engine.fetchLogStream(agentId: agentId, limit: 30) { result in
            self.events = result
            self.loading = false
        }
    }

    private func startTimer() {
        stopTimer()
        timer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak engine] _ in
            Task { @MainActor in
                guard autoRefresh, let engine else { return }
                engine.fetchLogStream(agentId: agentId, limit: 30) { result in
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
        return f
    }()
}
