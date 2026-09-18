import AgentIslandCore
import SwiftUI

// MARK: - Agent 行（主列表行视图 + 行内快捷操作）

// MARK: - Agent 行

struct AgentRowView: View {
    let snapshot: AgentSnapshot
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage(SettingKey.compactView) private var isCompact = false
    @State private var confirmingKill = false
    @State private var showingTooltip = false
    @State private var isHoveringRow = false
    /// 直达失败反馈：activate 返回 false 时短暂切换图标（ssh/tmux 启动的 CLI 无可激活窗口）
    @State private var activateFailed = false
    /// 关闭 tooltip 的延迟任务（可取消）：鼠标从环移向 popover 的途中会先触发
    /// onHover(false)，若立即关闭则 popover 里的按钮永远点不到。
    @State private var tooltipCloseTask: Task<Void, Never>?

    private var tooltipArrowEdge: Edge {
        switch controller.dockEdge {
        case .top: return .bottom
        case .right: return .leading
        case .bottom: return .top
        case .left: return .trailing
        }
    }
    /// 终止确认态的自动复位任务（可取消）
    @State private var confirmResetTask: Task<Void, Never>?

    /// Token 徽标文本："1.23M" 或 "1.23M $0.42"
    static func tokenBadge(_ usage: TokenUsage) -> String {
        let tokens = TokenUsage.compact(usage.tokens24h)
        let cost = TokenUsage.cost(usage.cost24h)
        return cost.isEmpty ? tokens : "\(tokens) \(cost)"
    }

    /// 是否显示实时动作横条（与内边距共用同一判定，避免 2pt 行高漂移）
    private var hasActionBar: Bool {
        (snapshot.level == .working || snapshot.level == .attention)
            && !(snapshot.currentAction ?? "").isEmpty
    }

    private var actionColor: Color {
        snapshot.level == .attention ? Theme.warningOrange : Theme.statusWorking
    }

    private var actionIcon: String {
        snapshot.level == .attention ? "hand.tap.fill" : "terminal.fill"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: isCompact ? 2 : 4) {
            HStack(spacing: isCompact ? 6 : 8) {
                AgentRingView(snapshot: snapshot, size: isCompact ? 22 : 26)
                    .onHover { h in
                        // 延迟关闭：给用户时间把鼠标从 26×26 的环移到 popover 上，
                        // 否则 popover 里的「终止 / 直达窗口」永远来不及点。
                        tooltipCloseTask?.cancel()
                        if h {
                            showingTooltip = true
                        } else {
                            tooltipCloseTask = Task { @MainActor in
                                try? await Task.sleep(nanoseconds: 400_000_000)
                                guard !Task.isCancelled else { return }
                                showingTooltip = false
                            }
                        }
                    }
                    .popover(isPresented: $showingTooltip, arrowEdge: tooltipArrowEdge) {
                        AgentHoverTooltipCard(snapshot: snapshot, engine: engine, controller: controller)
                            // 鼠标进入 popover 时取消关闭任务，让按钮可点
                            .onHover { inside in
                                if inside { tooltipCloseTask?.cancel() }
                            }
                    }

                VStack(alignment: .leading, spacing: 1) {
                    Text(snapshot.profile.name)
                        .font(Theme.bodyFont(isCompact ? 11.5 : 12.5, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                        .readableSingleLine(
                            fullText: snapshot.profile.name,
                            minWidth: 72,
                            priority: 2
                        )

                    if !hasActionBar {
                        HStack(spacing: 5) {
                            if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                                HStack(spacing: 3) {
                                    Image(systemName: "bolt.fill")
                                        .font(.system(size: 7))
                                        .foregroundColor(Theme.sydedockCyan)
                                    Text(Self.tokenBadge(usage))
                                        .font(Theme.monoDigitFont(9, weight: .semibold))
                                        .foregroundColor(Theme.onDark)
                                }
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1.5)
                                .background(
                                    Capsule()
                                        .fill(Color(dynamic: NSColor(hex: 0xffffff, alpha: 0.85), dark: NSColor(hex: 0xffffff, alpha: 0.08)))
                                        .overlay(
                                            Capsule()
                                                .strokeBorder(
                                                    LinearGradient(
                                                        colors: colorScheme == .light ? [
                                                            Color.white.opacity(0.95),
                                                            Color.black.opacity(0.08)
                                                        ] : [
                                                            Color.white.opacity(0.24),
                                                            Color.white.opacity(0.06)
                                                        ],
                                                        startPoint: .top,
                                                        endPoint: .bottom
                                                    ),
                                                    lineWidth: 0.5
                                                )
                                        )
                                        .shadow(color: Color.black.opacity(colorScheme == .light ? 0.025 : 0), radius: 1, y: 0.5)
                                )
                            } else {
                                Text(snapshot.lastActivityText)
                                    .font(Theme.bodyFont(9.5))
                                    .foregroundColor(Theme.onDarkFaint)
                            }
                            ActivityMatrixDots(snapshot: snapshot, count: 5)
                        }
                    }
                }

                Spacer(minLength: 4)

                // 右侧展示区：支持渐进披露（悬停渐入快捷按钮，闲置态显示优雅状态）
                if (isHoveringRow || confirmingKill) && snapshot.processRunning {
                    HStack(spacing: 4) {
                        // 确认态仅在仍处于工作时有效：若 3 秒内 Agent 已转 idle，
                        // 继续显示红色「终止?」会诱导用户终止一个已空闲的进程
                        if confirmingKill && snapshot.level == .working {
                            Button {
                                engine.terminateAgent(pid: snapshot.pid, agentId: snapshot.profile.id)
                                confirmingKill = false
                            } label: {
                                Text("终止?")
                                    .font(Theme.bodyFont(9, weight: .bold))
                                    .foregroundColor(.white)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2.5)
                                    .background(Capsule().fill(Theme.dangerRed.opacity(0.92)))
                            }
                            .buttonStyle(.plain)
                            .help("再次点击立即强制终止该 Agent 进程")
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
                                Image(systemName: "xmark.circle")
                                    .font(.system(size: 11, weight: .medium))
                                    .foregroundColor(Theme.dangerRed.opacity(0.85))
                                    .padding(4)
                                    .background(
                                        Circle()
                                            .fill(Theme.dangerRed.opacity(0.15))
                                            .overlay(Circle().strokeBorder(Theme.dangerRed.opacity(0.35), lineWidth: 0.5))
                                    )
                            }
                            .buttonStyle(.plain)
                            .help("一键终止逃生舱：关闭该正在运行的 Agent 及其子任务")
                            .accessibilityLabel("终止 \(snapshot.profile.name)")
                        }

                        Button {
                            controller.openLiveStream(agentId: snapshot.profile.id)
                        } label: {
                            Image(systemName: "terminal")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundColor(Theme.onDark.opacity(0.85))
                                .padding(4)
                                .background(
                                    Circle()
                                        .fill(Theme.chipFill)
                                        .overlay(Circle().strokeBorder(Theme.obsidianHairline, lineWidth: 0.5))
                                )
                        }
                        .buttonStyle(.plain)
                        .help("查看 \(snapshot.profile.name) 实时事件与输出流水")
                        .accessibilityLabel("查看 \(snapshot.profile.name) 实时流水")

                        Button {
                            if !AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs) {
                                activateFailed = true
                                Task { @MainActor in
                                    try? await Task.sleep(nanoseconds: 1_500_000_000)
                                    activateFailed = false
                                }
                            }
                        } label: {
                            Image(systemName: activateFailed ? "exclamationmark.triangle" : "arrow.up.forward.app")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundColor(activateFailed ? Theme.warningOrange : Theme.onDark.opacity(0.85))
                                .padding(4)
                                .background(
                                    Circle()
                                        .fill(Theme.chipFill)
                                        .overlay(Circle().strokeBorder(Theme.obsidianHairline, lineWidth: 0.5))
                                )
                        }
                        .buttonStyle(.plain)
                        .help(activateFailed ? "未找到可激活的窗口（CLI 经 ssh/tmux 启动时无窗口可带）" : "置顶并激活该智能体窗口/终端")
                        .accessibilityLabel("直达 \(snapshot.profile.name) 窗口")
                    }
                    .transition(.opacity.combined(with: .scale(scale: 0.95)))
                } else {
                    HStack(spacing: 5) {
                        // 内存占用与健康状态指示（v1.7.6）
                        if snapshot.processRunning && snapshot.memoryBytes > 0 {
                            Text(snapshot.memoryText)
                                .font(Theme.monoDigitFont(9, weight: .medium))
                                .foregroundColor(colorScheme == .light ? Color(hex: 0x475569) : Theme.onDarkFaint)
                                .padding(.horizontal, 5)
                                .padding(.vertical, 2)
                                .background(
                                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                                        .fill(colorScheme == .light ? Color(hex: 0xf1f5f9) : Theme.obsidianPill)
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                                .strokeBorder(colorScheme == .light ? Color(hex: 0xe2e8f0) : Theme.obsidianHairline, lineWidth: 0.5)
                                        )
                                )
                                .help("物理内存驻留集 (RSS): \(snapshot.memoryText)")
                        }

                        if snapshot.isHung {
                            HStack(spacing: 2) {
                                Image(systemName: "exclamationmark.triangle.fill")
                                    .font(Theme.badgeFont())
                                Text("疑似卡死")
                                    .font(Theme.bodyFont(9, weight: .bold))
                            }
                            .foregroundColor(Theme.dangerRed)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2.5)
                            .background(
                                Capsule()
                                    .fill(Theme.dangerRed.opacity(0.18))
                                    .overlay(Capsule().strokeBorder(Theme.dangerRed.opacity(0.40), lineWidth: 0.5))
                            )
                            .help("检测到进程持续异常高负荷且缺乏会话响应，疑似处于死循环或线程死锁状态")
                        } else {
                            Text(snapshot.level.label)
                                .font(Theme.bodyFont(9.5, weight: .semibold))
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
                    .transition(.opacity)
                }
            }

            // 第二行：工作状态下的专属实时动作横条（全宽展示，彻底根治截断问题）
            if hasActionBar, let action = snapshot.currentAction {
                HStack(spacing: 5) {
                    Image(systemName: actionIcon)
                        .font(Theme.badgeFont())
                        .foregroundColor(colorScheme == .light ? (snapshot.level == .attention ? Color(hex: 0xb45309) : Color(hex: 0x047857)) : actionColor)
                    Text(action)
                        .font(Theme.monoFont(9.5))
                        .foregroundColor(colorScheme == .light ? (snapshot.level == .attention ? Color(hex: 0xb45309) : Color(hex: 0x065f46)) : actionColor)
                        .readableSingleLine(fullText: action, minWidth: 80, priority: 2)
                    Spacer(minLength: 0)
                    // 工作态也保留 token 徽标：正在消耗的 Agent 恰是最需要关注的，
                    // 此前它只在非工作态显示，工作中反而看不到用量
                    if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                        Text(Self.tokenBadge(usage))
                            .font(Theme.monoDigitFont(9, weight: .bold))
                            .foregroundColor(colorScheme == .light ? (snapshot.level == .attention ? Color(hex: 0xb45309) : Color(hex: 0x047857)) : Theme.onDark)
                            .lineLimit(1)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(
                                Capsule()
                                    .fill(colorScheme == .light ? Color.white.opacity(0.95) : actionColor.opacity(0.20))
                                    .overlay(Capsule().strokeBorder(colorScheme == .light ? (snapshot.level == .attention ? Color(hex: 0xfde68a) : Color(hex: 0xa7f3d0)) : actionColor.opacity(0.40), lineWidth: 0.5))
                            )
                            .help("24h \(TokenUsage.compact(usage.tokens24h)) token")
                    }
                }
                .padding(.horizontal, 7)
                .padding(.vertical, 3)
                .background(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(colorScheme == .light ? (snapshot.level == .attention ? Color(hex: 0xfffbeb) : Color(hex: 0xf0fdf4)) : actionColor.opacity(0.08))
                        .overlay(
                            RoundedRectangle(cornerRadius: 5, style: .continuous)
                                .strokeBorder(colorScheme == .light ? (snapshot.level == .attention ? Color(hex: 0xfde68a).opacity(0.8) : Color(hex: 0xbbf7d0).opacity(0.8)) : actionColor.opacity(0.22), lineWidth: 0.5)
                        )
                )
                .padding(.leading, isCompact ? 30 : 34) // 与 Agent 名称对齐
                .help(action)
            }
        }
        .padding(.horizontal, 8)
        // 与横条显示条件严格一致：空字符串动作不显示横条，也不应多出 2pt 内边距
        .padding(.vertical, isCompact ? (hasActionBar ? 3.5 : 2.5) : (hasActionBar ? 5 : 4))
        .hoverRowBackground(cornerRadius: 10, idleFill: Theme.obsidianCardFill)
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(controller.focusedAgentId == snapshot.profile.id ? Theme.sydedockCyan.opacity(0.85) : Color.clear, lineWidth: 1.2)
        )
        .padding(.horizontal, 6)
        .contentShape(Rectangle())
        .onHover { hovering in
            withAnimation(.spring(response: 0.22, dampingFraction: 0.75)) {
                isHoveringRow = hovering
            }
        }
        .scaleEffect(isHoveringRow ? 1.004 : 1.0)
        .animation(.spring(response: 0.22, dampingFraction: 0.75), value: isHoveringRow)
        .onTapGesture {
            controller.focusedAgentId = snapshot.profile.id
            // 点行进 agent 详情页（原 Finder 跳转移入详情页会话列表），带有丝滑弹簧转场
            withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) {
                controller.route = .agentDetail(snapshot.profile.id)
            }
        }
        .contextMenu {
            if snapshot.processRunning {
                Button {
                    _ = AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs)
                } label: {
                    Label("直达窗口 / 终端", systemImage: "arrow.up.forward.app")
                }
            }

            Button {
                withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) {
                    controller.route = .agentDetail(snapshot.profile.id)
                }
            } label: {
                Label("查看模型与 Token 详情", systemImage: "chart.bar.doc.horizontal")
            }

            Button {
                controller.openLiveStream(agentId: snapshot.profile.id)
            } label: {
                Label("查看实时流水抽屉", systemImage: "terminal")
            }

            if let dir = snapshot.profile.sessionDirs.first, FileManager.default.fileExists(atPath: dir) {
                Button {
                    NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: dir)
                } label: {
                    Label("在访达中显示会话数据", systemImage: "folder")
                }
            }

            Divider()

            if let pid = snapshot.pid {
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString("\(pid)", forType: .string)
                } label: {
                    Label("复制进程 PID (\(pid))", systemImage: "doc.on.doc")
                }
            }

            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(snapshot.profile.name, forType: .string)
            } label: {
                Label("复制 Agent 名称", systemImage: "doc.on.doc")
            }

            if snapshot.level == .working || snapshot.isHung {
                Divider()
                Button(role: .destructive) {
                    confirmingKill = true
                } label: {
                    Label("强制终止此进程 (逃生舱)", systemImage: "xmark.octagon")
                }
            }
        }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(snapshot.profile.name)，\(snapshot.level.label)，点按查看详情")
        .accessibilityHint(snapshot.processRunning ? "悬停可执行终止 / 流水 / 直达操作，点按查看详情" : "点按查看详情")
    }

    private func levelForegroundColor(_ level: ActivityLevel) -> Color {
        if colorScheme == .light {
            switch level {
            case .working: return Color(hex: 0x047857) // emerald-700
            case .attention: return Color(hex: 0xb45309) // amber-700
            case .completed: return Color(hex: 0x047857) // emerald-700
            case .idle: return Color(hex: 0x64748b) // slate-500
            case .offline: return Color(hex: 0x94a3b8)
            }
        }
        return level.color
    }

    private func levelBackgroundColor(_ level: ActivityLevel) -> Color {
        if colorScheme == .light {
            switch level {
            case .working: return Color(hex: 0xecfdf5) // emerald-50
            case .attention: return Color(hex: 0xfffbeb) // amber-50
            case .completed: return Color(hex: 0xecfdf5) // emerald-50
            case .idle: return Color(hex: 0xf1f5f9) // slate-100
            case .offline: return Color(hex: 0xf1f5f9).opacity(0.6)
            }
        }
        return level.color.opacity(0.12)
    }

    private func levelBorderColor(_ level: ActivityLevel) -> Color {
        if colorScheme == .light {
            switch level {
            case .working: return Color(hex: 0xa7f3d0).opacity(0.8) // emerald-200
            case .attention: return Color(hex: 0xfde68a).opacity(0.8) // amber-200
            case .completed: return Color(hex: 0xa7f3d0).opacity(0.8)
            case .idle: return Color(hex: 0xe2e8f0) // slate-200
            case .offline: return Color(hex: 0xe2e8f0).opacity(0.6)
            }
        }
        return level.color.opacity(0.32)
    }
}
