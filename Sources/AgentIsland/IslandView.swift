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
            // Apple 原生 UltraThinMaterial 真实硬件毛玻璃，穿透底层桌面壁纸与窗口
            notchShape
                .fill(.ultraThinMaterial)

            // 细腻透光光罩：
            // 浅色模式：白冰高透微光（10%~22% 柔和白微光，彻底告别死白色块，保留通透质感）
            // 深色模式：深邃冷炭暗夜光罩（32%~38% 沉稳黑曜石）
            notchShape
                .fill(
                    LinearGradient(
                        colors: colorScheme == .light ? [
                            Color.white.opacity(0.22),
                            Color.white.opacity(0.10)
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
                            Color.white.opacity(0.65),
                            Color.black.opacity(0.12)
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

enum CardRoute: Equatable {
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
            Button("偏好设置…") {
                NSApp.activate(ignoringOtherApps: true)
                NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
            }
        }
        .animation(.easeInOut(duration: 0.22), value: controller.displayState)
        .animation(.spring(response: 0.32, dampingFraction: 0.84), value: controller.route)
    }

    private var expandedContent: some View {
        Group {
            switch controller.route {
            case .list:
                expandedCard
            case .tokenAnalytics:
                TokenAnalyticsView(engine: engine, controller: controller)
            case .agentDetail(let agentId):
                AgentDetailView(engine: engine, controller: controller, agentId: agentId)
            case .sessions(let agentId, let modelId):
                SessionListView(engine: engine, controller: controller,
                                agentId: agentId, modelId: modelId)
            case .toolbox:
                ToolboxView(engine: engine, controller: controller)
            case .liveStream(let agentId):
                LiveLogStreamView(engine: engine, controller: controller, agentId: agentId)
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

    /// 稳定身份/状态放第一行，长度不可控的实时动作放第二行。
    private var headerPresentation: HeaderPresentation {
        if let alert = activeAlertEvent {
            let tint = alertColor(for: alert.eventType)
            let fullText = alert.detail.map { "\(alert.summaryText)。\($0)" } ?? alert.summaryText
            return HeaderPresentation(
                title: alert.agentName,
                subtitle: alert.summaryText,
                badge: alert.eventType == .costSpike ? "告警" : "待确认",
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
                        .foregroundColor(Theme.onDarkFaint)
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
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(4)
                        .topBarChip()
                }
                .buttonStyle(.plain)
                .help("智能体维护工作台：扫描清理孤儿进程、死锁与内存泄露")
                .accessibilityLabel("智能体维护工作台")

                // 一键收起按钮
                Button {
                    controller.collapse()
                } label: {
                    Image(systemName: collapseIcon)
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(4)
                        .topBarChip()
                }
                .buttonStyle(.plain)
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
                                                .strokeBorder(snap.level == .attention ? Theme.warningOrange.opacity(0.45) : (snap.level == .working ? Theme.sydedockEmerald.opacity(0.3) : Theme.obsidianHairline), lineWidth: 0.5)
                                        )
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
            } else {
                ScrollView(.vertical, showsIndicators: true) {
                    VStack(spacing: 2) {
                        ForEach(engine.visibleSnapshots) { snapshot in
                            AgentRowView(snapshot: snapshot, engine: engine, controller: controller)
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

    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(hovering ? Theme.hoverFill : Theme.chipFill)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .strokeBorder(Theme.obsidianHairline, lineWidth: 0.5)
                    )
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

// MARK: - 状态标签

extension ActivityLevel {
    var label: String {
        switch self {
        case .working: return "工作中"
        case .attention: return "待确认"
        case .completed: return "已完成"
        case .idle: return "待机"
        case .offline: return "离线"
        }
    }

    var color: Color {
        switch self {
        case .working: return Theme.statusWorking
        case .attention: return Theme.warningOrange
        case .completed: return Theme.statusWorking
        case .idle: return Theme.statusIdle
        case .offline: return Theme.statusOffline
        }
    }
}

// EventBannerView 已拆分至 EventBannerView.swift（R15）
