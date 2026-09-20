import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 事件通知横幅富文本卡片 (v1.7.2)

struct EventBannerView: View {
    let event: AgentTaskEvent
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @State private var copiedFeedback = false
    /// 复制反馈的复位任务（可取消）
    @State private var copyFeedbackTask: Task<Void, Never>?
    /// 熔断二次确认态：杀的是整棵进程树（用户终端/编辑器可能一起退出），不能单击生效
    @State private var confirmingKill = false
    /// 熔断确认态的自动复位任务（可取消）
    @State private var killConfirmTask: Task<Void, Never>?
    /// 直达失败反馈：activate 返回 false 时短暂切换文案（ssh/tmux 启动的 CLI 无可激活窗口）
    @State private var activateFailed = false
    @Environment(\.colorScheme) private var colorScheme

    private var isExpanded: Bool {
        controller.eventBannerExpanded
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            // 第一行：状态图标 + 摘要标题 + 展开/折叠按钮 + 关闭
            HStack(alignment: .center, spacing: 6) {
                Image(systemName: eventIcon(for: event.eventType))
                    .font(.system(size: 11, weight: .bold))
                    .foregroundColor(eventColor(for: event.eventType))

                Group {
                    if isExpanded {
                        Text(event.summaryText)
                            .fixedSize(horizontal: false, vertical: true)
                            .help(event.summaryText)
                            .accessibilityLabel(event.summaryText)
                    } else {
                        Text(event.summaryText)
                            .readableSingleLine(
                                fullText: event.summaryText,
                                minWidth: 72,
                                priority: 2
                            )
                    }
                }
                .font(Theme.bodyFont(11, weight: .semibold))
                .foregroundColor(Theme.onDark)

                Spacer(minLength: 4)

                // 详情折叠/展开指示器
                if event.detail != nil {
                    Button {
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.8)) {
                            controller.eventBannerExpanded.toggle()
                        }
                    } label: {
                        HStack(spacing: 2) {
                            Text(isExpanded ? "收起" : "原因")
                                .font(Theme.bodyFont(9, weight: .medium))
                            Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                                .font(.system(size: 8, weight: .bold))
                        }
                        .foregroundColor(Theme.onDarkMuted)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 2)
                        // 动态色：浅色主题下白色胶囊不可见
                        .background(Capsule().fill(Theme.chipFill))
                    }
                    .buttonStyle(.plain)
                    .help(isExpanded ? "收起排查详情" : "展开警告产生的原因与排查建议")
                    .accessibilityLabel(isExpanded ? "收起排查详情" : "展开排查详情")
                }

                Button {
                    withAnimation(.easeInOut(duration: 0.24)) {
                        controller.eventBannerExpanded = false
                        engine.clearLatestEvent()
                    }
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(6)   // 15×15 → 21×21 热区：贴边小图标此前很难点中
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("关闭提醒")
                .accessibilityLabel("关闭提醒")
            }

            // 第二部分：展开态下的完整排查建议与触发原因（富文本自适应高度）
            if isExpanded, let detail = event.detail {
                VStack(alignment: .leading, spacing: 3) {
                    Text(detail)
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.onDarkMuted)
                        .lineSpacing(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 7)
                .padding(.vertical, 5)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(colorScheme == .light ? Color.white.opacity(0.92) : Color.black.opacity(0.20))
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.white.opacity(0.08), lineWidth: 0.5)
                        )
                )
            }

            // 第三行：快捷操作栏
            HStack(spacing: 6) {
                if isExpanded {
                    // 复制诊断信息
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(event.copyableDiagnosticText, forType: .string)
                        copiedFeedback = true
                        // 用 Task 而非 DispatchQueue：可随视图身份变化取消，
                        // 避免新事件到来后仍显示上一条的「已复制」
                        copyFeedbackTask?.cancel()
                        copyFeedbackTask = Task { @MainActor in
                            try? await Task.sleep(nanoseconds: 1_800_000_000)
                            guard !Task.isCancelled else { return }
                            copiedFeedback = false
                        }
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: copiedFeedback ? "checkmark" : "doc.on.doc")
                                .font(.system(size: 9))
                            Text(copiedFeedback ? "已复制" : "复制诊断")
                                .font(Theme.bodyFont(9, weight: .medium))
                        }
                        .foregroundColor(copiedFeedback ? Theme.statusWorking : Theme.onDarkMuted)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2.5)
                        // 用动态色而非硬编码白色：浅色主题下白玻璃 + 白色 6% 会让按钮完全不可见
                        .background(Capsule().fill(Theme.chipFill))
                    }
                    .buttonStyle(.plain)
                    .help("一键复制告警信息、PID 及触发时间戳")
                    .accessibilityLabel("复制诊断信息")
                }

                Spacer()

                // 熔断：costSpike 且能定位到进程时才提供；pid 缺失时给出去向指引而非直接消失。
                // 与 Agent 行「终止?」、工作台「确认?」保持同一套两段式确认：
                // 首击只进入确认态，3 秒内不再点击则自动复位（Task 随视图身份变化取消）。
                if event.eventType == .costSpike {
                    if let pid = event.pid {
                        if confirmingKill {
                            Button {
                                engine.terminateAgent(pid: pid, agentId: event.agentId)
                                confirmingKill = false
                            } label: {
                                HStack(spacing: 3) {
                                    Image(systemName: "xmark.octagon.fill")
                                        .font(.system(size: 9))
                                    Text("确认熔断?")
                                        .font(Theme.bodyFont(10, weight: .bold))
                                }
                                .foregroundColor(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(Capsule().fill(Theme.dangerRed))
                            }
                            .buttonStyle(.plain)
                            .help("再次点击确认终止 \(event.agentName) 及其子进程（不可撤销）")
                            .accessibilityLabel("确认熔断 \(event.agentName)")
                            .onAppear {
                                killConfirmTask?.cancel()
                                killConfirmTask = Task { @MainActor in
                                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                                    guard !Task.isCancelled else { return }
                                    confirmingKill = false
                                }
                            }
                        } else {
                            Button {
                                confirmingKill = true
                            } label: {
                                HStack(spacing: 3) {
                                    Image(systemName: "xmark.octagon.fill")
                                        .font(.system(size: 9))
                                    Text("熔断")
                                        .font(Theme.bodyFont(10, weight: .bold))
                                }
                                .foregroundColor(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(Capsule().fill(Theme.dangerRed.opacity(0.85)))
                            }
                            .buttonStyle(.plain)
                            .help("终止该 Agent 进程树，阻止持续消耗（需二次确认）")
                            .accessibilityLabel("熔断 \(event.agentName)")
                        }
                    } else {
                        Text("未定位到进程，请在活动监视器处理")
                            .font(Theme.bodyFont(9))
                            .foregroundColor(Theme.onDarkMuted)
                    }
                }

                // 直达：仅在事件对应 Agent 仍在快照中时可点（工作台清理事件没有对应 Agent，
                // 此前点击完全无反馈，用户以为按钮坏了）
                let target = engine.snapshots.first { $0.id == event.agentId }
                Button {
                    guard let target else { return }
                    if !AppActivator.activate(pid: target.pid, bundleIDs: target.profile.bundleIDs) {
                        activateFailed = true
                        Task { @MainActor in
                            try? await Task.sleep(nanoseconds: 1_500_000_000)
                            activateFailed = false
                        }
                    }
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: activateFailed ? "exclamationmark.triangle" : "arrow.up.forward.app")
                            .font(.system(size: 9))
                        Text(activateFailed ? "未找到窗口" : "直达")
                            .font(Theme.bodyFont(10, weight: .semibold))
                    }
                    .foregroundColor(Theme.onDark)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(
                        Capsule()
                            .fill(colorScheme == .light ? Color.white : Theme.chipFill)
                            .overlay(
                                Capsule()
                                    .strokeBorder(colorScheme == .light ? Ramp.slate300 : Color.clear, lineWidth: 0.5)
                            )
                            .shadow(color: Color.black.opacity(colorScheme == .light ? 0.03 : 0), radius: 1, y: 0.5)
                    )
                    .opacity(target == nil ? 0.45 : 1)
                }
                .buttonStyle(.plain)
                .disabled(target == nil)
                .help(activateFailed ? "未找到可激活的窗口（CLI 经 ssh/tmux 启动时无窗口可带）" : (target == nil ? "该提醒没有对应的运行中 Agent" : "拉至前台并激活窗口"))
                .accessibilityLabel("直达 \(event.agentName) 窗口")
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 6)
        .background(
            event.eventType == .costSpike
                ? (colorScheme == .light ? Ramp.red50 : Theme.dangerRed.opacity(0.18))
                : (event.eventType == .attention
                   ? (colorScheme == .light ? Ramp.amber50 : Theme.warningOrange.opacity(0.15))
                   : (colorScheme == .light ? Ramp.green50 : Color.white.opacity(0.06)))
        )
    }

    private func eventIcon(for type: AgentTaskEvent.EventType) -> String {
        switch type {
        case .completed: return "checkmark.circle.fill"
        case .attention: return "exclamationmark.triangle.fill"
        case .costSpike: return "exclamationmark.octagon.fill"
        }
    }

    private func eventColor(for type: AgentTaskEvent.EventType) -> Color {
        switch type {
        case .completed: return Theme.statusWorking
        // 复用语义色而非系统 .orange / .red：与顶栏状态摘要、Agent 行徽标同一套配色，
        // 且集中到 Theme 便于后续统一加深浅色对比（系统 .orange 在浅底仅约 2.2:1）
        case .attention: return Theme.warningOrange
        case .costSpike: return Theme.dangerRed
        }
    }
}
