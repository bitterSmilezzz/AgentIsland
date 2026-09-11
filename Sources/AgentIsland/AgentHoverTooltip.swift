import SwiftUI
import AppKit
import AgentIslandCore

// MARK: - 悬停指向小尾巴（Tooltip Tail）
// 灵感源自 CodeNotch 的 TooltipTail：精准指向被 Hover 的 Agent 环或列表项

struct TooltipTail: Shape {
    enum Direction: Sendable {
        case leading   // 卡片在左侧，尾巴尖朝右
        case trailing  // 卡片在右侧，尾巴尖朝左
        case up        // 卡片在上方，尾巴尖朝下
        case down      // 卡片在下方，尾巴尖朝上
    }

    let direction: Direction

    init(direction: Direction) {
        self.direction = direction
    }

    func path(in rect: CGRect) -> Path {
        var path = Path()
        switch direction {
        case .leading:
            path.move(to: CGPoint(x: rect.minX, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.maxX, y: rect.midY))
            path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        case .trailing:
            path.move(to: CGPoint(x: rect.maxX, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.minX, y: rect.midY))
            path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        case .up:
            path.move(to: CGPoint(x: rect.minX, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
            path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        case .down:
            path.move(to: CGPoint(x: rect.minX, y: rect.maxY))
            path.addLine(to: CGPoint(x: rect.midX, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        }
        path.closeSubpath()
        return path
    }
}

// MARK: - 悬停透视浮层卡片（Agent Hover Tooltip View）

struct AgentHoverTooltipCard: View {
    let snapshot: AgentSnapshot
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @State private var confirmingKill = false
    /// 终止确认态的自动复位任务（可取消）：与 Agent 行「终止?」同一写法，
    /// 视图销毁时不会被一个仍在排队的 DispatchQueue 回调写回已失效的状态
    @State private var confirmResetTask: Task<Void, Never>?

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
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Color(dynamicLight: 0xf5f5f7, dark: 0x18181b).opacity(0.96))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Theme.glassSpecularBorder, lineWidth: 1)
        )
        .shadow(color: Color.black.opacity(0.35), radius: 12, x: -2, y: 4)
    }

    // MARK: 头部
    private var headerView: some View {
        HStack(spacing: 8) {
            AgentRingView(snapshot: snapshot, size: 24)

            VStack(alignment: .leading, spacing: 1) {
                Text(snapshot.profile.name)
                    .font(Theme.bodyFont(12, weight: .bold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(1)

                if let pid = snapshot.pid, snapshot.processRunning {
                    let cpuStr = snapshot.cpuPercent > 0 ? String(format: "%.1f%%", snapshot.cpuPercent) : "0%"
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

            Spacer(minLength: 4)

            if snapshot.isHung {
                Text("卡死告警")
                    .font(Theme.bodyFont(8, weight: .bold))
                    .foregroundColor(Theme.dangerRed)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Theme.dangerRed.opacity(0.2)))
            } else {
                Text(snapshot.level.label)
                    .font(Theme.bodyFont(9, weight: .semibold))
                    .foregroundColor(snapshot.level.color)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(snapshot.level.color.opacity(0.18)))
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
                            .font(.system(size: 8))
                            .foregroundColor(Theme.statusWorking)
                        Text("当前实时执行")
                            .font(Theme.bodyFont(9, weight: .medium))
                            .foregroundColor(Theme.onDarkFaint)
                    }

                    Text(action)
                        .font(Theme.monoFont(10))
                        .foregroundColor(Theme.statusWorking)
                        .lineLimit(2)
                        .padding(6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        // 动态色：硬编码 black 0.35 在浅色卡片（0xf5f5f7）上是一块深色板
                        .background(RoundedRectangle(cornerRadius: 6)
                            .fill(Color(dynamic: NSColor(hex: 0x000000, alpha: 0.05),
                                        dark: NSColor(hex: 0x000000, alpha: 0.35))))
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
                        .foregroundColor(Theme.dangerRed.opacity(0.9))
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: 6).fill(Theme.dangerRed.opacity(0.15)))
                    }
                    .buttonStyle(.plain)
                    .help("终止该 Agent 进程树（需二次确认）")
                    .accessibilityLabel("终止 \(snapshot.profile.name)")
                }

                Button {
                    AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.up.forward.app")
                            .font(.system(size: 9))
                        Text("直达窗口")
                            .font(Theme.bodyFont(10, weight: .semibold))
                    }
                    .foregroundColor(Theme.onDark)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .frame(maxWidth: .infinity)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Theme.chipFill))
                }
                .buttonStyle(.plain)
            }
        }
    }
}
