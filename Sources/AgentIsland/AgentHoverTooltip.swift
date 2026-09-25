import SwiftUI
import AppKit
import AgentIslandCore

// MARK: - 悬停透视浮层卡片（Agent Hover Tooltip View）
// （早期的几何小尾巴 TooltipTail Shape 从未接线，SwiftUI .popover 自带箭头，已删除）

struct AgentHoverTooltipCard: View {
    let snapshot: AgentSnapshot
    let engine: ActivityEngine   // 只用于 terminateAgent；卡片渲染的是传下来的 snapshot
    @ObservedObject var controller: IslandPanelController
    @State private var confirmingKill = false
    /// 直达失败反馈：activate 返回 false 时短暂切换按钮文案（ssh/tmux 启动的 CLI 无可激活窗口）
    @State private var activateFailed = false
    /// 终止确认态的自动复位任务（可取消）：与 Agent 行「终止?」同一写法，
    /// 视图销毁时不会被一个仍在排队的 DispatchQueue 回调写回已失效的状态
    @State private var confirmResetTask: Task<Void, Never>?
    @Environment(\.colorScheme) private var colorScheme

    init(snapshot: AgentSnapshot, engine: ActivityEngine, controller: IslandPanelController) {
        self.snapshot = snapshot
        self.engine = engine
        self.controller = controller
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            headerView
            actionView
            if hasUsageData {
                usageView
            }
            if snapshot.processRunning {
                actionBarView
            }
        }
        .padding(12)
        .frame(width: 230)
        .background(
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(colorScheme == .light ? .regularMaterial : .ultraThinMaterial)
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(
                        colorScheme == .light
                            ? LinearGradient(
                                colors: [Color.white.opacity(0.96), Color.white.opacity(0.92)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            : LinearGradient(
                                colors: [Color(hex: 0x13131a).opacity(0.92), Color(hex: 0x0c0c10).opacity(0.88)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                    )
            }
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(
                    LinearGradient(
                        colors: colorScheme == .light ? [
                            Color.white,
                            Ramp.slate200
                        ] : [
                            Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.18),
                            Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.06)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    ),
                    lineWidth: 0.75
                )
        )
        .shadow(color: Color.black.opacity(colorScheme == .light ? 0.10 : 0.35), radius: colorScheme == .light ? 10 : 12, x: 0, y: 4)
    }

    // MARK: 头部
    private var headerView: some View {
        HStack(spacing: 8) {
            AgentRingView(snapshot: snapshot, size: 24)

            VStack(alignment: .leading, spacing: 1) {
                Text(snapshot.profile.name)
                    .font(Theme.bodyFont(12, weight: .bold))
                    .foregroundColor(Theme.onDark)
                    .readableSingleLine(
                        fullText: snapshot.profile.name,
                        minWidth: 64,
                        priority: 2
                    )

                if let pid = snapshot.pid, snapshot.processRunning {
                    // nil = 本拍没有 CPU 差分窗口，「—」而不是 0%
                    let cpuStr = snapshot.cpuPercent.map { $0 > 0 ? String(format: "%.1f%%", $0) : "0%" } ?? "—"
                    let memStr = snapshot.memoryBytes > 0 ? " · \(snapshot.memoryText)" : ""
                    Text("PID: \(pid) · \(cpuStr)\(memStr)")
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                } else {
                    Text(snapshot.processRunning ? "后台运行中" : "未启动")
                        .font(Theme.bodyFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Spacer(minLength: 4)

            if snapshot.isHung == true {
                Text("卡死告警")
                    .font(Theme.bodyFont(9, weight: .bold))
                    .foregroundColor(colorScheme == .light ? Ramp.red700 : Theme.dangerRed)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(
                        Capsule()
                            .fill(colorScheme == .light ? Ramp.red50 : Theme.dangerRed.opacity(0.2))
                            .overlay(Capsule().strokeBorder(colorScheme == .light ? Ramp.red200 : Color.clear, lineWidth: 0.5))
                    )
            } else {
                Text(snapshot.level.label + AgentProvenance.badgeSuffix(snapshot.provenance))
                    .font(Theme.bodyFont(9, weight: .semibold))
                    .foregroundColor(levelForegroundColor(snapshot.level))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(
                        Capsule()
                            .fill(levelBackgroundColor(snapshot.level))
                            .overlay(Capsule().strokeBorder(levelBorderColor(snapshot.level), lineWidth: 0.5))
                    )
            }
        }
    }

    // MARK: 实时动作
    private var actionView: some View {
        Group {
            if let action = snapshot.currentAction, !action.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 4) {
                        Image(systemName: "terminal.fill")
                            .font(.system(size: 9))
                            .foregroundColor(Theme.statusWorking)
                        Text("当前实时执行")
                            .font(Theme.bodyFont(9, weight: .medium))
                            .foregroundColor(Theme.onDarkFaint)
                    }

                    Text(action)
                        .font(Theme.monoFont(10))
                        .foregroundColor(Theme.statusWorking)
                        .lineLimit(2)
                        .help(action)
                        .accessibilityLabel(action)
                        .padding(6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            RoundedRectangle(cornerRadius: 6)
                                .fill(colorScheme == .light ? Ramp.slate100 : Color(dynamic: NSColor(hex: 0x000000, alpha: 0.05), dark: NSColor(hex: 0x000000, alpha: 0.35)))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 6)
                                        .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                                )
                        )
                }
            } else {
                HStack(spacing: 4) {
                    Image(systemName: "clock.arrow.circlepath")
                        .font(.system(size: 9))
                        .foregroundColor(Theme.onDarkFaint)
                    Text("最近活跃: \(snapshot.lastActivityText)")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }
        }
    }

    // MARK: Token 用量
    private var hasUsageData: Bool {
        if let usage = snapshot.tokenUsage {
            return usage.tokens24h > 0 || usage.tokensTotal > 0
        }
        return false
    }

    private var usageView: some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider().overlay(Theme.onDark.opacity(0.1))

            if let usage = snapshot.tokenUsage {
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("24H 用量")
                            .font(Theme.bodyFont(9))
                            .foregroundColor(Theme.onDarkFaint)
                        Text(TokenUsage.compact(usage.tokens24h))
                            .font(Theme.monoFont(11, weight: .semibold))
                            .foregroundColor(Theme.onDark)
                    }

                    if usage.cost24h > 0 {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("24H 花费")
                                .font(Theme.bodyFont(9))
                                .foregroundColor(Theme.onDarkFaint)
                            Text(TokenUsage.cost(usage.cost24h))
                                .font(Theme.monoFont(11, weight: .semibold))
                                .foregroundColor(Palette.ringYellow)
                        }
                    }

                    Spacer()

                    VStack(alignment: .trailing, spacing: 2) {
                        Text("历史累计")
                            .font(Theme.bodyFont(9))
                            .foregroundColor(Theme.onDarkFaint)
                        Text(TokenUsage.compact(usage.tokensTotal))
                            .font(Theme.monoFont(11, weight: .semibold))
                            .foregroundColor(Theme.onDarkMuted)
                    }
                }
            }
        }
    }

    // MARK: 快捷操作
    private var actionBarView: some View {
        VStack(spacing: 6) {
            Divider().overlay(Theme.onDark.opacity(0.1))

            HStack(spacing: 8) {
                if confirmingKill {
                    Button {
                        engine.terminateAgent(pid: snapshot.pid, agentId: snapshot.profile.id)
                        confirmingKill = false
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "xmark.octagon.fill")
                                .font(.system(size: 9))
                            Text("确定终止?")
                                .font(Theme.bodyFont(10, weight: .bold))
                        }
                        .foregroundColor(.white)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .frame(maxWidth: .infinity)
                        .background(RoundedRectangle(cornerRadius: 6).fill(Theme.dangerRed.opacity(0.9)))
                    }
                    .buttonStyle(.plain)
                    .help("再次点击确认终止 \(snapshot.profile.name) 及其子进程（不可撤销）")
                    .accessibilityLabel("确认终止 \(snapshot.profile.name)")
                    .onAppear {
                        confirmResetTask?.cancel()
                        confirmResetTask = Task { @MainActor in
                            try? await Task.sleep(nanoseconds: 3_000_000_000)
                            guard !Task.isCancelled else { return }
                            confirmingKill = false
                        }
                    }
                } else if snapshot.level == .working {
                    Button {
                        confirmingKill = true
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "xmark.circle")
                                .font(.system(size: 9))
                            Text("终止")
                                .font(Theme.bodyFont(10, weight: .medium))
                        }
                        .foregroundColor(colorScheme == .light ? Ramp.red700 : Theme.dangerRed.opacity(0.9))
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(
                            RoundedRectangle(cornerRadius: 6)
                                .fill(colorScheme == .light ? Ramp.red50 : Theme.dangerRed.opacity(0.15))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 6)
                                        .strokeBorder(colorScheme == .light ? Ramp.red200 : Color.clear, lineWidth: 0.5)
                                )
                        )
                    }
                    .buttonStyle(.plain)
                    .help("终止该 Agent 进程树（需二次确认）")
                    .accessibilityLabel("终止 \(snapshot.profile.name)")
                }

                Button {
                    if !AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs) {
                        activateFailed = true
                        Task { @MainActor in
                            try? await Task.sleep(nanoseconds: 1_500_000_000)
                            activateFailed = false
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: activateFailed ? "exclamationmark.triangle" : "arrow.up.forward.app")
                            .font(.system(size: 9))
                        Text(activateFailed ? "未找到窗口" : "直达窗口")
                            .font(Theme.bodyFont(10, weight: .semibold))
                    }
                    .foregroundColor(activateFailed ? Theme.warningOrange : Theme.onDark)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .frame(maxWidth: .infinity)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Theme.chipFill)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                            )
                    )
                }
                .buttonStyle(.plain)
                .help(activateFailed ? "未找到可激活的窗口（CLI 经 ssh/tmux 启动时无窗口可带）" : "置顶并激活该智能体窗口/终端")
            }
        }
    }

    private func levelForegroundColor(_ level: ActivityLevel) -> Color {
        colorScheme == .light ? level.lightText : level.color
    }

    private func levelBackgroundColor(_ level: ActivityLevel) -> Color {
        colorScheme == .light ? level.lightFill : level.color.opacity(0.18)
    }

    private func levelBorderColor(_ level: ActivityLevel) -> Color {
        colorScheme == .light ? level.lightBorder : Color.clear
    }
}
