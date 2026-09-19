import AgentIslandCore
import SwiftUI

// MARK: - 事件历史时间线浮层 (v0.0.73)

struct EventHistoryPopoverView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // 顶栏
            HStack {
                HStack(spacing: 5) {
                    Image(systemName: "clock.arrow.circlepath")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(Theme.sydedockCyan)
                    Text("最近任务与事件历史")
                        .font(Theme.bodyFont(11, weight: .bold))
                        .foregroundColor(Theme.onDark)
                }

                Spacer()

                if !engine.eventHistory.isEmpty {
                    Button("清空") {
                        engine.clearEventHistory()
                    }
                    .buttonStyle(.plain)
                    .font(Theme.bodyFont(9.5, weight: .medium))
                    .foregroundColor(Theme.onDarkFaint)
                }
            }
            .padding(.bottom, 2)

            DarkDivider()

            if engine.eventHistory.isEmpty {
                VStack(spacing: 6) {
                    Image(systemName: "tray")
                        .font(.system(size: 20))
                        .foregroundColor(Theme.onDarkFaint.opacity(0.6))
                    Text("暂无任务历史记录")
                        .font(Theme.bodyFont(11))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .frame(maxWidth: .infinity, minHeight: 120)
            } else {
                ScrollView(.vertical, showsIndicators: true) {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(engine.eventHistory) { event in
                            eventRow(event)
                        }
                    }
                    .padding(.vertical, 2)
                }
                .frame(maxHeight: 280)
            }
        }
        .padding(10)
        .frame(width: 290)
    }

    private func eventRow(_ event: AgentTaskEvent) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 5) {
                // 类型图标
                Image(systemName: eventIcon(event.eventType))
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundColor(eventColor(event.eventType))

                Text(event.agentName)
                    .font(Theme.bodyFont(10.5, weight: .bold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(1)

                Spacer()

                // 时长或标签
                if event.eventType == .completed && event.duration > 0 {
                    Text(formatDuration(event.duration))
                        .font(Theme.monoDigitFont(8.5, weight: .medium))
                        .foregroundColor(Theme.sydedockEmerald)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                        .background(
                            Capsule().fill(Theme.sydedockEmerald.opacity(0.12))
                        )
                }

                // 时间点
                Text(formatTime(event.timestamp))
                    .font(Theme.monoFont(8.5))
                    .foregroundColor(Theme.onDarkFaint)
            }

            // 摘要文案
            Text(event.summaryText)
                .font(Theme.bodyFont(9.5))
                .foregroundColor(Theme.onDarkMuted)
                .lineLimit(2)
        }
        .padding(6)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Theme.obsidianCardFill)
        )
    }

    private func eventIcon(_ type: AgentTaskEvent.EventType) -> String {
        switch type {
        case .completed: return "checkmark.circle.fill"
        case .attention: return "bell.badge.fill"
        case .costSpike: return "exclamationmark.octagon.fill"
        }
    }

    private func eventColor(_ type: AgentTaskEvent.EventType) -> Color {
        switch type {
        case .completed: return Theme.sydedockEmerald
        case .attention: return Theme.warningOrange
        case .costSpike: return Theme.dangerRed
        }
    }

    private func formatTime(_ date: Date) -> String {
        // 共享 POSIX 口径实例：既不跟随系统 locale 改写数字，也不再每次调用新建 DateFormatter
        TimeFormat.clock.string(from: date)
    }

    private func formatDuration(_ sec: TimeInterval) -> String {
        let total = Int(sec)
        if total < 60 { return "\(total)s" }
        return "\(total / 60)m\(total % 60)s"
    }
}
