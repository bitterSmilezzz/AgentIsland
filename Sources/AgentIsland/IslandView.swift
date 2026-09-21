import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 玻璃拟态背景（Sydedock 半透明黑曜石玻璃 + 晶莹高光边）

struct GlassCardBackground: View {
    var cornerRadius: CGFloat = Theme.radiusLg
    var dockEdge: DockEdge = .right
    @Environment(\.colorScheme) private var colorScheme

    /// 贴边造型：采用 SideNotchShape 赋予反向倒角一体化贴边质感
    private var notchShape: SideNotchShape {
        SideNotchShape(dockEdge: dockEdge, curlRadius: IslandMetrics.notchInset, cornerRadius: cornerRadius)
    }

    var body: some View {
        ZStack {
            // Apple 原生毛玻璃：浅色模式使用 regularMaterial 消除暗色背景文字透射，深色模式维持 ultraThinMaterial 黑曜石通透
            notchShape
                .fill(colorScheme == .light ? .regularMaterial : .ultraThinMaterial)

            // 细腻透光光罩：
            // 浅色模式：高定白瓷琉璃（88%~82% 纯净白瓷微光，彻底告别透底发脏，兼具通透感与实体感）
            // 深色模式：深邃冷炭暗夜光罩（32%~38% 沉稳黑曜石）
            notchShape
                .fill(
                    LinearGradient(
                        colors: colorScheme == .light ? [
                            Color.white.opacity(0.88),
                            Color.white.opacity(0.82)
                        ] : [
                            Color(red: 0.05, green: 0.05, blue: 0.08).opacity(0.32),
                            Color(red: 0.02, green: 0.02, blue: 0.04).opacity(0.38)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )

            // 0.75pt 晶莹高反光微描边（浅色顶白微光+底微暗勾边；深色钻石渐变微反光）
            notchShape
                .stroke(
                    LinearGradient(
                        colors: colorScheme == .light ? [
                            Color.white.opacity(0.96),
                            Color(hex: 0x000000).opacity(0.08)
                        ] : [
                            Color.white.opacity(0.40),
                            Color.white.opacity(0.10)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    ),
                    lineWidth: 0.75
                )
        }
    }
}

struct VisualEffectView: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .popover
    var blendingMode: NSVisualEffectView.BlendingMode = .behindWindow

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = blendingMode
        view.state = .active
        view.isEmphasized = true
        view.wantsLayer = true
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blendingMode
        nsView.state = .active
        nsView.isEmphasized = true
        nsView.wantsLayer = true
    }
}

// MARK: - 灵动岛视图状态

enum IslandDisplayState: Equatable {
    case docked         // 收起态：露出 6pt 晶莹微细条
    case expanded       // 展开态：完整卡片
}

// MARK: - 卡内导航（主列表 → agent 详情 → 模型会话列表）

enum CardRoute: Hashable {
    case list                     // 主卡：agent 列表 + 汇总栏
    case tokenAnalytics           // Token 时间趋势、环比与来源构成
    case agentDetail(String)      // agent 详情：总览 + 模型拆分
    case sessions(String, String) // agentId + modelId：该模型会话列表
    case toolbox                  // 快捷工具箱：孤儿进程/假死死锁扫描与清理
    case liveStream(String)       // 实时事件与日志抽屉：agentId
}

private struct HeaderPresentation {
    let title: String
    let subtitle: String?
    let badge: String?
    let tint: Color
    let subtitleIcon: String?
    let fullText: String
}

// MARK: - 灵动岛视图

struct IslandView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @Environment(\.colorScheme) private var colorScheme
    @State private var showingHistoryPopover = false

    /// 搜索条的键盘焦点。灵动岛本来就是全键盘优先的界面（/ 搜索、j/k 选择、Enter 下钻），
    /// 但「/」只是把搜索条挂出来，不落到输入框里的话用户还得再点一次，流程断在中间。
    @FocusState private var searchFieldFocused: Bool

    /// 收起窗口大部分会移出屏幕；把微细条对齐到仍在屏幕中的那一侧。
    private var dockedAlignment: Alignment {
        switch controller.dockEdge {
        case .top: return .bottom
        case .right: return .leading
        case .bottom: return .top
        case .left: return .trailing
        }
    }

    private var collapseIcon: String {
        switch controller.dockEdge {
        case .top: return "chevron.up"
        case .right: return "chevron.right"
        case .bottom: return "chevron.down"
        case .left: return "chevron.left"
        }
    }

    var body: some View {
        ZStack(alignment: dockedAlignment) {
            if controller.displayState == .expanded {
                expandedContent
                    .transition(.opacity)
            } else {
                dockedSliver
                    .transition(.opacity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: dockedAlignment)
        .preferredColorScheme(controller.appearanceMode.colorScheme)
        .contextMenu {
            if controller.displayState == .expanded {
                Button("收起灵动岛") {
                    controller.collapse()
                }
                Divider()
            }
            Menu("吸附边缘") {
                ForEach(DockEdge.allCases, id: \.self) { edge in
                    Button {
                        controller.setDockEdge(edge)
                    } label: {
                        HStack {
                            Text(edge.label)
                            if controller.dockEdge == edge {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
            }
            Menu("外观主题") {
                ForEach(IslandAppearance.allCases) { mode in
                    Button {
                        controller.applyAppearance(mode)
                    } label: {
                        HStack {
                            Text(mode.label)
                            if controller.appearanceMode == mode {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
            }
            Menu("通知模式") {
                ForEach(NotificationPolicy.allCases) { policy in
                    Button {
                        controller.applyNotificationPolicy(policy)
                    } label: {
                        HStack {
                            Text(policy.label)
                            if controller.notificationPolicy == policy {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
            }
            Divider()
            Button {
                let current = SettingBool.read(SettingKey.playCompletionSound, default: true)
                UserDefaults.standard.set(!current, forKey: SettingKey.playCompletionSound)
            } label: {
                let on = SettingBool.read(SettingKey.playCompletionSound, default: true)
                HStack {
                    Text(on ? "静音已完成提示音" : "开启已完成提示音")
                    Image(systemName: on ? "speaker.slash" : "speaker.wave.2")
                }
            }
            Button {
                let current = UserDefaults.standard.bool(forKey: SettingKey.compactView)
                UserDefaults.standard.set(!current, forKey: SettingKey.compactView)
            } label: {
                let compact = UserDefaults.standard.bool(forKey: SettingKey.compactView)
                HStack {
                    Text(compact ? "切换至常规视图" : "切换至紧凑视图")
                    Image(systemName: compact ? "arrow.up.left.and.arrow.down.right" : "arrow.down.right.and.arrow.up.left")
                }
            }
            Button("复制当前状态诊断快照") {
                DiagnosticsSnapshot.copyToPasteboard(from: engine)
            }
            Divider()
            Button("偏好设置…") {
                NSApp.activate(ignoringOtherApps: true)
                NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
            }
        }
        .animation(.easeInOut(duration: 0.22), value: controller.displayState)
        .animation(.spring(response: 0.32, dampingFraction: 0.84), value: controller.route)
        .onExitCommand {
            // Esc 键层级退回：在子页时丝滑返回主卡，在主卡展开态时平滑收起
            if controller.route != .list {
                withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) {
                    controller.route = .list
                }
                HapticFeedback.perform(.alignment)
            } else if controller.displayState == .expanded {
                controller.collapse()
            }
        }
    }

    private var expandedContent: some View {
        ZStack {
            switch controller.route {
            case .list:
                expandedCard
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .scale(scale: 0.98, anchor: .center)),
                        removal: .opacity.combined(with: .scale(scale: 0.98, anchor: .center))
                    ))
            case .tokenAnalytics:
                TokenAnalyticsView(engine: engine, controller: controller)
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center)),
                        removal: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center))
                    ))
            case .agentDetail(let agentId):
                AgentDetailView(engine: engine, controller: controller, agentId: agentId)
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center)),
                        removal: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center))
                    ))
            case .sessions(let agentId, let modelId):
                SessionListView(engine: engine, controller: controller,
                                agentId: agentId, modelId: modelId)
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center)),
                        removal: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center))
                    ))
            case .toolbox:
                ToolboxView(engine: engine, controller: controller)
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center)),
                        removal: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center))
                    ))
            case .liveStream(let agentId):
                LiveLogStreamView(engine: engine, controller: controller, agentId: agentId)
                    .transition(.asymmetric(
                        insertion: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center)),
                        removal: .opacity.combined(with: .offset(x: 12)).combined(with: .scale(scale: 0.98, anchor: .center))
                    ))
            }
        }
        .id(controller.route)
        .overlay {
            if controller.showShortcutHUD {
                ShortcutHUDView(controller: controller)
            }
        }
    }

    // MARK: 贴边微细条（露 6pt，晶莹质感 + 多状态状态呼吸光晕）

    private var hasActiveAlert: Bool {
        activeAlertEvent != nil || engine.snapshots.contains { $0.level == .attention }
    }

    /// 当前仍需用户注意的事件。完成事件不占用顶部状态摘要，避免把正常完成误显示成告警。
    private var activeAlertEvent: AgentTaskEvent? {
        guard let event = engine.latestEvent,
              event.eventType == .costSpike || event.eventType == .attention else { return nil }
        return event
    }

    private func alertColor(for eventType: AgentTaskEvent.EventType) -> Color {
        eventType == .costSpike ? Theme.dangerRed : Theme.warningOrange
    }

    /// 横幅角标：外部投递的事件要看得出来，不能与引擎自己判定的状态同形
    private func alertBadgeText(_ alert: AgentTaskEvent) -> String {
        if alert.externallyDelivered { return "外部\(alert.eventType == .costSpike ? "告警" : "确认")" }
        return alert.eventType == .costSpike ? "告警" : "待确认"
    }

    /// 稳定身份/状态放第一行，长度不可控的实时动作放第二行。
    private var headerPresentation: HeaderPresentation {
        if let alert = activeAlertEvent {
            let tint = alertColor(for: alert.eventType)
            let fullText = alert.detail.map { "\(alert.summaryText)。\($0)" } ?? alert.summaryText
            return HeaderPresentation(
                title: alert.agentName,
                subtitle: alert.summaryText,
                badge: alertBadgeText(alert),
                tint: tint,
                subtitleIcon: alert.eventType == .costSpike
                    ? "exclamationmark.octagon.fill" : "hand.tap.fill",
                fullText: fullText
            )
        }

        if let waiting = engine.visibleSnapshots.first(where: { $0.level == .attention }) {
            let action = waiting.currentAction ?? "等待你确认"
            return HeaderPresentation(
                title: waiting.profile.name,
                subtitle: action,
                badge: "待确认",
                tint: Theme.warningOrange,
                subtitleIcon: "hand.tap.fill",
                fullText: "\(waiting.profile.name)：\(action)"
            )
        }

        if let active = engine.visibleSnapshots.first(where: {
            $0.level == .working && !(($0.currentAction ?? "").isEmpty)
        }), let action = active.currentAction {
            let others = max(engine.workingAgents().count - 1, 0)
            return HeaderPresentation(
                title: active.profile.name,
                subtitle: action,
                badge: others > 0 ? "+\(others)" : "工作中",
                tint: Theme.statusWorking,
                subtitleIcon: "terminal.fill",
                fullText: "\(active.profile.name)：\(action)"
            )
        }

        let completedCount = engine.visibleSnapshots.filter { $0.level == .completed }.count
        let statusText = engine.anyWorking
            ? "\(engine.workingAgents().count) 个 Agent 正在工作"
            : (completedCount > 0 ? "\(completedCount) 个任务已完成" : "全部 Agent 待机")
        return HeaderPresentation(
            title: statusText,
            subtitle: nil,
            badge: nil,
            tint: engine.anyWorking ? Theme.statusWorking : Theme.onDarkMuted,
            subtitleIcon: nil,
            fullText: statusText
        )
    }

    private var dockedSliver: some View {
        DockedSliverCapsule(
            dockEdge: controller.dockEdge,
            isWorking: engine.anyWorking,
            hasAlert: hasActiveAlert,
            onTap: {
                if controller.displayState == .docked {
                    controller.toggle()
                }
            },
            onHover: { hovering in
                if hovering && controller.displayState == .docked {
                    controller.expandFromHover()
                }
            }
        )
    }

    // MARK: 展开卡片

    private var expandedCard: some View {
        VStack(alignment: .leading, spacing: 0) {
            // 顶栏：状态摘要（支持长按拖拽卡片自由移动并贴边吸附）
            HStack(spacing: 6) {
                statusDot
                    .frame(width: 9, height: 9)
                let header = headerPresentation
                AdaptiveHeaderText(
                    title: header.title,
                    subtitle: header.subtitle,
                    badge: header.badge,
                    tint: header.tint,
                    subtitleIcon: header.subtitleIcon,
                    fullText: header.fullText
                )
                Spacer(minLength: 4)
                // 在线计数宽度不足时主动退化为单个数字，不抢标题的最低可读区。
                ViewThatFits(in: .horizontal) {
                    Text("\(engine.visibleSnapshots.count)/\(engine.snapshots.count) 在线")
                        .fixedSize()
                    Text("\(engine.visibleSnapshots.count)")
                        .fixedSize()
                }
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
                    .layoutPriority(-1)
                    .help("当前在线 \(engine.visibleSnapshots.count) 个，共监控 \(engine.snapshots.count) 个；离线项不显示")
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("当前在线 \(engine.visibleSnapshots.count) 个，共监控 \(engine.snapshots.count) 个")

                // 即时搜索过滤按钮
                Button {
                    withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                        controller.isSearchActive.toggle()
                        if !controller.isSearchActive { controller.searchText = "" }
                    }
                } label: {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(controller.isSearchActive ? Theme.sydedockCyan : (colorScheme == .light ? Ramp.slate700 : Theme.onDarkFaint))
                        .padding(4)
                        .topBarChip()
                }
                .buttonStyle(.plain)
                .hitTargetHeight()
                .help(controller.isSearchActive ? "关闭即时过滤 (Esc)" : "即时搜索过滤 (快捷键 /)")
                .accessibilityLabel("搜索过滤智能体")

                // 外观模式切换
                Menu {
                    ForEach(IslandAppearance.allCases) { mode in
                        Button {
                            controller.applyAppearance(mode)
                        } label: {
                            HStack {
                                Text(mode.label)
                                if controller.appearanceMode == mode {
                                    Image(systemName: "checkmark")
                                }
                            }
                        }
                    }
                } label: {
                    Image(systemName: controller.appearanceMode.icon)
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(colorScheme == .light ? Ramp.slate700 : Theme.onDarkFaint)
                        .padding(4)
                        .topBarChip()
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("外观主题：\(controller.appearanceMode.label)（点击切换）")
                .accessibilityLabel("外观主题，当前 \(controller.appearanceMode.label)")

                // 工作台快捷维护工具箱（v1.7.7）
                Button {
                    controller.route = .toolbox
                } label: {
                    Image(systemName: "wrench.and.screwdriver")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(colorScheme == .light ? Ramp.slate700 : Theme.onDarkFaint)
                        .padding(4)
                        .topBarChip()
                }
                .buttonStyle(.plain)
                .hitTargetHeight()
                .help("智能体维护工作台：扫描清理孤儿进程、死锁与内存泄露")
                .accessibilityLabel("智能体维护工作台")

                // 任务与事件历史流 (v0.0.73)
                Button {
                    showingHistoryPopover.toggle()
                } label: {
                    Image(systemName: "clock.arrow.circlepath")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(showingHistoryPopover ? Theme.sydedockCyan : (colorScheme == .light ? Ramp.slate700 : Theme.onDarkFaint))
                        .padding(4)
                        .topBarChip()
                }
                .buttonStyle(.plain)
                .hitTargetHeight()
                .popover(isPresented: $showingHistoryPopover, arrowEdge: controller.dockEdge == .top ? .bottom : .leading) {
                    EventHistoryPopoverView(engine: engine, controller: controller)
                }
                .help("查看最近任务完成与告警事件历史时间线")
                .accessibilityLabel("查看最近事件历史")

                // 一键收起按钮
                Button {
                    controller.collapse()
                } label: {
                    Image(systemName: collapseIcon)
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(colorScheme == .light ? Ramp.slate700 : Theme.onDarkFaint)
                        .padding(4)
                        .topBarChip()
                }
                .buttonStyle(.plain)
                .hitTargetHeight()
                .help("收起灵动岛（或光标移出卡片自动收起）")
                .accessibilityLabel("收起灵动岛")
            }
            .padding(.horizontal, Theme.pageMargin)
            .padding(.top, IslandMetrics.headerPaddingTop)
            .padding(.bottom, IslandMetrics.headerPaddingBottom)
            .cardDrag(controller: controller)

            DarkDivider()

            // 实时活动环微看板（Quick Rings Shelf · CodeNotch 灵感）
            let activeSnapshots = engine.ringShelfSnapshots
            if !activeSnapshots.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(activeSnapshots) { snap in
                            Button {
                                controller.route = .agentDetail(snap.profile.id)
                            } label: {
                                HStack(spacing: 6) {
                                    AgentRingView(snapshot: snap, size: 24)
                                    VStack(alignment: .leading, spacing: 1) {
                                        Text(snap.profile.name)
                                            .font(Theme.bodyFont(10, weight: .semibold))
                                            .foregroundColor(Theme.onDark)
                                            .readableSingleLine(fullText: snap.profile.name, priority: 2)
                                        Text(shelfSubtitle(snap))
                                            .font(Theme.badgeFont())
                                            .foregroundColor(snap.level == .attention ? Theme.warningOrange : (snap.level == .working || snap.level == .completed ? Palette.ringGreen : Theme.onDarkFaint))
                                            .readableSingleLine(
                                                fullText: shelfSubtitle(snap),
                                                priority: 1
                                            )
                                    }
                                }
                                .padding(.vertical, 3)
                                .padding(.horizontal, 6)
                                .background(
                                    RoundedRectangle(cornerRadius: 7, style: .continuous)
                                        .fill(snap.level == .working || snap.level == .attention ? Theme.hoverFill : Theme.obsidianCardFill)
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 7, style: .continuous)
                                                .strokeBorder(
                                                    colorScheme == .light
                                                    ? (snap.level == .attention ? Ramp.amber200.opacity(0.8) : (snap.level == .working ? Ramp.emerald200.opacity(0.8) : Ramp.slate200))
                                                    : (snap.level == .attention ? Theme.warningOrange.opacity(0.45) : (snap.level == .working ? Theme.sydedockEmerald.opacity(0.3) : Theme.obsidianHairline)),
                                                    lineWidth: 0.5
                                                )
                                        )
                                        .shadow(color: Color.black.opacity(colorScheme == .light ? 0.025 : 0), radius: 1, y: 0.5)
                                )
                            }
                            .buttonStyle(.plain)
                            // help 与正文同口径：正文显示 token 时不能再提示「待机」
                            .help("\(snap.profile.name): \(shelfSubtitle(snap))")
                        }
                    }
                    .padding(.horizontal, Theme.pageMargin)
                    .padding(.vertical, 5)
                }
                DarkDivider()
            }

            // 任务事件横幅（完成/等待确认/资源与Token熔断告警富文本交互卡片）
            if let event = engine.latestEvent {
                // 按事件身份隔离视图：新事件到来时重置内部状态（如「已复制」反馈）
                EventBannerView(event: event, engine: engine, controller: controller)
                    .id(event.id)
                    // 关闭提醒时先淡出并收缩横幅，再让窗口同步缩短；
                    // 没有显式 transition 时 SwiftUI 会在窗口动画期间瞬间移除内容。
                    .transition(.asymmetric(
                        insertion: .opacity,
                        removal: .opacity.combined(with: .scale(scale: 0.98, anchor: .top))
                    ))
                DarkDivider()
            }

            // 即时搜索输入框
            if controller.isSearchActive {
                searchBar
            }

            // Agent 列表
            if engine.visibleSnapshots.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "zzz")
                        .font(.system(size: 22))
                        .foregroundColor(Theme.onDarkFaint)
                    Text("没有活跃的 Agent")
                        .font(Theme.bodyFont(12))
                        .foregroundColor(Theme.onDarkFaint)
                    Button {
                        NSApp.activate(ignoringOtherApps: true)
                        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
                    } label: {
                        Text("打开偏好设置")
                            .font(Theme.bodyFont(10, weight: .medium))
                            .foregroundColor(Theme.onDark.opacity(0.85))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(Capsule().fill(Theme.chipFill))
                    }
                    .buttonStyle(.plain)
                    .help("启用 Agent 或调整监控范围")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, IslandMetrics.emptyStatePaddingVertical)
            } else if filteredSnapshots.isEmpty && !controller.searchText.isEmpty {
                VStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 18))
                        .foregroundColor(Theme.onDarkFaint)
                    Text("未找到匹配「\(controller.searchText)」的智能体")
                        .font(Theme.bodyFont(11))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 24)
            } else {
                // ScrollViewReader：j/k 与方向键把焦点移到列表末尾时，原先只有行高亮在动、
                // 滚动位置不动（>6 个 Agent 就会滚出可视区），等于「按到但看不见」
                ScrollViewReader { proxy in
                    ScrollView(.vertical, showsIndicators: false) {
                        VStack(spacing: 2) {
                            ForEach(filteredSnapshots) { snapshot in
                                AgentRowView(snapshot: snapshot, engine: engine, controller: controller)
                                    .id(snapshot.id)
                            }
                        }
                        .padding(.vertical, IslandMetrics.listVerticalPadding)
                    }
                    // 与窗口高度同源（IslandMetrics.listHeight）：封顶时压缩列表而非裁掉底部汇总栏
                    .frame(maxHeight: IslandMetrics.listHeight(
                        visibleCount: engine.visibleSnapshots.count,
                        hasSummary: !engine.grandTotal.isEmpty,
                        hasRings: !activeSnapshots.isEmpty,
                        hasEvent: engine.latestEvent != nil,
                        eventExpanded: controller.eventBannerExpanded))
                    // 空间不足时只压列表：列表可滚动，压缩不丢信息；
                    // 汇总栏是不可滚动的定高条，被压就会截断（用户反馈「多一个元素就被截」）
                    .layoutPriority(-1)
                    .onChange(of: controller.focusedAgentId) { focused in
                        guard let focused else { return }
                        withAnimation(.easeOut(duration: 0.16)) {
                            proxy.scrollTo(focused, anchor: .center)
                        }
                    }
                }
            }

            // Token 汇总栏
            if !engine.grandTotal.isEmpty {
                DarkDivider()
                TokenSummaryBar(total: engine.grandTotal) {
                    controller.route = .tokenAnalytics
                }
                    .fixedSize(horizontal: false, vertical: true)
                    // 高优先级：VStack 分配空间时优先满足汇总栏的完整高度
                    .layoutPriority(1)
            }
        }
        // 事件横幅的插入/移除与列表布局使用同一时长，避免关闭按钮导致内容硬切。
        .animation(.easeInOut(duration: 0.24), value: engine.latestEvent?.id)
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
    }

    private var filteredSnapshots: [AgentSnapshot] {
        // 命中判定收敛到 IslandPanelController.focusableAgents：列表与 j/k 聚焦集同源
        IslandPanelController.focusableAgents(from: engine.visibleSnapshots,
                                              isSearchActive: controller.isSearchActive,
                                              searchText: controller.searchText)
    }

    private var searchBar: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 10, weight: .semibold))
                .foregroundColor(Theme.sydedockCyan)
            TextField("按名称或 CLI 快速过滤...", text: $controller.searchText)
                .textFieldStyle(.plain)
                .font(Theme.bodyFont(11))
                .foregroundColor(Theme.onDark)
                // 「/」只是把搜索条挂出来，键盘焦点要跟上，否则还得用鼠标点一次输入框
                .focused($searchFieldFocused)
                .onSubmit { searchFieldFocused = false }
                // 搜索态下 Enter 不下钻详情（全局快捷键让位给输入框），这里只交还键盘焦点
                .accessibilityLabel("按名称或 CLI 过滤智能体")
            if !controller.searchText.isEmpty {
                Button {
                    controller.searchText = ""
                    // 清空后仍留在输入框内，便于直接改词而不是重新点一次
                    searchFieldFocused = true
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 10))
                        .foregroundColor(Theme.onDarkFaint)
                        // 图标只有 10pt，热区靠 padding 撑开，保证输入框聚焦时仍点得到
                        .padding(3)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .hitTargetHeight()
                .accessibilityLabel("清空搜索关键词")
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Theme.obsidianCardFill)
                .overlay(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .stroke(Theme.sydedockCyan.opacity(0.4), lineWidth: 0.8)
                )
        )
        .padding(.horizontal, 10)
        .padding(.top, 4)
        .padding(.bottom, 2)
        // 焦点请求只能挂在搜索条自己身上：它与 isSearchActive 出自同一次更新，
        // 在 toggle 的那一刻直接赋值会落到还不存在的输入框上、静默失效
        .onAppear { searchFieldFocused = true }
        .onDisappear { searchFieldFocused = false }
    }

    // MARK: 状态点

    /// 活动环微看板副标题（正文与 help 共用，避免两处口径漂移）
    private func shelfSubtitle(_ snap: AgentSnapshot) -> String {
        if snap.level == .working { return "工作中" }
        if snap.level == .attention { return "等待你确认" }
        if snap.level == .completed { return "任务已完成" }
        if let usage = snap.tokenUsage, usage.tokens24h > 0 {
            return AgentRowView.tokenBadge(usage)
        }
        return snap.level.label
    }

    private var statusDot: some View {
        ZStack {
            Circle().fill(statusColor)
            if engine.anyWorking {
                Circle()
                    .fill(statusColor)
                    .scaleEffect(1.6)
                    .opacity(0.35)
                    // 仅展开态运行动画（docked 态 repeatForever 60fps 布局风暴，
                    // 是工作态 CPU 峰值主因；收起即停止）
                    .modifier(PulseAnimation(isActive: controller.displayState == .expanded))
            }
        }
        .animation(.easeInOut(duration: 0.3), value: statusColor)
    }

    private var statusColor: Color {
        engine.snapshots.contains { $0.level == .attention }
            ? Theme.warningOrange
            : (engine.anyWorking ? Theme.statusWorking : Theme.statusIdle)
    }
}

// AgentRowView 已拆分至 AgentRowView.swift（R15）

// TokenSummaryBar 已拆分至 TokenSummaryBar.swift（R15）

// MARK: - 顶栏圆形图标按钮底

/// 顶栏图标按钮的圆形底：默认 chipFill，hover 升到 hoverFill。
/// 顶栏此前只有 .help 文案、没有任何视觉反馈，鼠标扫过时无法确认按钮可点
/// （详情页返回键早有同款反馈，此处补齐一致性）。
private struct TopBarChipModifier: ViewModifier {
    @State private var hovering = false
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(colorScheme == .light
                          ? (hovering ? Color.white : Ramp.slate100)
                          : (hovering ? Theme.hoverFill : Theme.chipFill))
                    .overlay(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(
                                colorScheme == .light
                                ? (hovering ? Ramp.slate300 : Ramp.slate200)
                                : (hovering ? Color.white.opacity(0.22) : Color.white.opacity(0.08)),
                                lineWidth: 0.5
                            )
                    )
                    .shadow(color: Color.black.opacity(colorScheme == .light ? (hovering ? 0.06 : 0.02) : 0), radius: 1, y: 0.5)
            )
            .onHover { hovering = $0 }
            .animation(.easeOut(duration: 0.12), value: hovering)
    }
}

extension View {
    fileprivate func topBarChip() -> some View {
        modifier(TopBarChipModifier())
    }
}

// DockedSliverCapsule / PulseAnimation 已拆分至 DockedSliver.swift（R15）
// ActivityLevel 的 color / lightText / lightFill / lightBorder 与状态标签文案（Core 的 label）
// 统一在 Theme.swift —— 色阶此前在 IslandView + 三个视图里各抄一份，改一次要对齐四处。

// EventBannerView 已拆分至 EventBannerView.swift（R15）

// MARK: - 快捷键速查 HUD (v0.0.78)

struct ShortcutHUDView: View {
    @ObservedObject var controller: IslandPanelController
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        ZStack {
            Color.black.opacity(0.45)
                .edgesIgnoringSafeArea(.all)
                .onTapGesture {
                    controller.showShortcutHUD = false
                }

            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Image(systemName: "keyboard.fill")
                        .foregroundColor(Theme.sydedockCyan)
                        .font(.system(size: 13, weight: .bold))
                    Text("键盘快捷键速查")
                        .font(Theme.bodyFont(12, weight: .bold))
                        .foregroundColor(Theme.onDark)
                    Spacer()
                    Button {
                        controller.showShortcutHUD = false
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 13))
                            .foregroundColor(Theme.onDarkFaint)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("关闭快捷键速查")
                }

                DarkDivider()

                VStack(spacing: 6) {
                    shortcutRow(key: "1 ~ 3", desc: "快速切页 (1主页 / 2用量 / 3工具箱)")
                    shortcutRow(key: "j / k 或 ↓ / ↑", desc: "上下选择聚焦智能体")
                    shortcutRow(key: "Enter / Return", desc: "下钻查看智能体详情与会话")
                    shortcutRow(key: "/", desc: "激活智能体即时搜索过滤")
                    shortcutRow(key: "Esc", desc: "逐级返回或收起面板")
                    shortcutRow(key: "⌘R", desc: "立即重新采样与后台刷新")
                    shortcutRow(key: "⌘,", desc: "打开偏好设置窗口")
                    shortcutRow(key: "?", desc: "呼出 / 关闭此帮助面板")
                }
            }
            .padding(14)
            .frame(width: IslandMetrics.cardWidth - 24)
            .background(
                RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                    .fill(colorScheme == .light ? Color.white.opacity(0.95) : Color(hex: 0x1A1B23).opacity(0.95))
                    .shadow(color: Color.black.opacity(0.25), radius: 16, x: 0, y: 8)
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous)
                    .stroke(Theme.obsidianCardBorder, lineWidth: 0.75)
            )
        }
        .transition(.opacity.combined(with: .scale(scale: 0.95, anchor: .center)))
    }

    private func shortcutRow(key: String, desc: String) -> some View {
        HStack {
            Text(key)
                .font(Theme.bodyFont(10, weight: .semibold))
                .foregroundColor(Theme.sydedockCyan)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(Theme.sydedockCyan.opacity(0.12))
                )
            Spacer()
            Text(desc)
                .font(Theme.bodyFont(10))
                .foregroundColor(Theme.onDarkMuted)
        }
    }
}
