import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 玻璃拟态背景（Q4：NSVisualEffectView + 黑蒙层 + 顶部高光）

struct GlassCardBackground: View {
    var cornerRadius: CGFloat = Theme.radiusLg
    var dockEdge: DockEdge = .right

    /// 贴边造型：采用 SideNotchShape 赋予反向倒角一体化贴边质感
    private var notchShape: SideNotchShape {
        SideNotchShape(dockEdge: dockEdge, curlRadius: IslandMetrics.notchInset, cornerRadius: cornerRadius)
    }

    var body: some View {
        ZStack {
            VisualEffectView(material: .popover, blendingMode: .behindWindow)
                .clipShape(notchShape)
            // 蒙层：深色下黑蒙，浅色下白蒙（动态）
            notchShape
                .fill(Color(dynamicLight: 0xffffff, dark: 0x000000).opacity(Theme.glassOverlayOpacity))
            // 1px 晶莹微反光描边（深色微白高光，浅色微暗勾边）
            notchShape
                .stroke(Theme.glassSpecularBorder, lineWidth: 1)
        }
    }
}

struct VisualEffectView: NSViewRepresentable {
    var material: NSVisualEffectView.Material
    var blendingMode: NSVisualEffectView.BlendingMode

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = blendingMode
        view.state = .active
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blendingMode
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
    case agentDetail(String)      // agent 详情：总览 + 模型拆分
    case sessions(String, String) // agentId + modelId：该模型会话列表
    case toolbox                  // 快捷工具箱：孤儿进程/假死死锁扫描与清理
    case liveStream(String)       // 实时事件与日志抽屉：agentId
}

// MARK: - 灵动岛视图

struct IslandView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    var body: some View {
        ZStack(alignment: controller.dockEdge == .top ? .bottom : .leading) {
            if controller.displayState == .expanded {
                expandedContent
                    .transition(.opacity)
            } else {
                dockedSliver
                    .transition(.opacity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: controller.dockEdge == .top ? .bottom : .leading)
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
        activeAlertEvent != nil
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
                // 优先展示「有动作」的 working agent：若第一个 working 恰好无动作，
                // 此前会整段退化，明明有 Agent 带动作也不显示
                if let alert = activeAlertEvent {
                    Text(alert.summaryText)
                        .font(Theme.bodyFont(12, weight: .semibold))
                        .foregroundColor(alertColor(for: alert.eventType))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(1)
                        .contentShape(Rectangle())
                        .help(alert.detail.map { "\(alert.summaryText)\n\($0)" } ?? alert.summaryText)
                        .accessibilityLabel(alert.detail.map { "\(alert.summaryText)。\($0)" } ?? alert.summaryText)
                } else if let active = engine.visibleSnapshots.first(where: {
                    $0.level == .working && !($0.currentAction ?? "").isEmpty
                }), let action = active.currentAction {
                    HStack(spacing: 4) {
                        Text(active.profile.name)
                            .font(Theme.bodyFont(13, weight: .bold))
                            .foregroundColor(Theme.onDark)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Text("·")
                            .foregroundColor(Theme.onDarkFaint)
                        Text(action)
                            .font(Theme.bodyFont(12, weight: .medium))
                            .foregroundColor(Theme.statusWorking)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        // 多 Agent 并行时提示还有几个在工作，避免只看到第一个造成误解
                        let others = engine.workingAgents().count - 1
                        if others > 0 {
                            Text("+\(others)")
                                .font(Theme.monoFont(10, weight: .semibold))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                    // 顶栏必须恒为单行：IslandMetrics.headerContentHeight 按单行文本行高
                    // 校准，一旦名称/动作折行，顶栏实际高度会多出约 17pt，使总内容超过
                    // 窗口高度上限，底部 Token 汇总栏被裁（拖动后动作文字变长时必现）。
                    .lineLimit(1)
                    .fixedSize(horizontal: false, vertical: true)
                    .layoutPriority(1)
                    .contentShape(Rectangle())
                    // 正文保持单行省略；悬停时展示未截断的 Agent 名和动作。
                    .help("\(active.profile.name): \(action)")
                    .accessibilityLabel("\(active.profile.name)：\(action)")
                } else {
                    let statusText = engine.anyWorking
                        ? "\(engine.workingAgents().count) 个 Agent 正在工作"
                        : "当前没有 Agent 在工作"
                    Text(statusText)
                        .font(Theme.bodyFont(13, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(1)
                        .contentShape(Rectangle())
                        .help(statusText)
                        .accessibilityLabel(statusText)
                }
                Spacer(minLength: 4)
                // 可见计数是次要信息：去掉 fixedSize 让它可被压缩，
                // 避免挤占左侧「Agent 名 + 实时动作」这一最需要看的信息
                Text("\(engine.visibleSnapshots.count)/\(engine.snapshots.count) 可见")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
                    .layoutPriority(-1)
                    .help("当前可见 \(engine.visibleSnapshots.count) 个，共监控 \(engine.snapshots.count) 个")

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
                    Image(systemName: controller.dockEdge == .top ? "chevron.up" : "chevron.right")
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
                                            .lineLimit(1)
                                        Text(shelfSubtitle(snap))
                                            .font(Theme.monoFont(8))
                                            .foregroundColor(snap.level == .working ? Palette.ringGreen : Theme.onDarkFaint)
                                            .lineLimit(1)
                                    }
                                }
                                .padding(.vertical, 3)
                                .padding(.horizontal, 6)
                                .background(
                                    RoundedRectangle(cornerRadius: 6)
                                        .fill(snap.level == .working ? Theme.hoverFill : Theme.chipFill)
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
                VStack(spacing: 6) {
                    Image(systemName: "zzz")
                        .font(.system(size: 22))
                        .foregroundColor(Theme.onDarkFaint)
                    Text("没有活跃的 Agent")
                        .font(Theme.bodyFont(12))
                        .foregroundColor(Theme.onDarkFaint)
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
                TokenSummaryBar(total: engine.grandTotal)
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
        engine.anyWorking ? Theme.statusWorking : Theme.statusIdle
    }
}

// MARK: - Agent 行

struct AgentRowView: View {
    let snapshot: AgentSnapshot
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @State private var confirmingKill = false
    @State private var showingTooltip = false
    /// 关闭 tooltip 的延迟任务（可取消）：鼠标从环移向 popover 的途中会先触发
    /// onHover(false)，若立即关闭则 popover 里的按钮永远点不到。
    @State private var tooltipCloseTask: Task<Void, Never>?
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
        snapshot.level == .working && !(snapshot.currentAction ?? "").isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                AgentRingView(snapshot: snapshot, size: 26)
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
                    .popover(isPresented: $showingTooltip, arrowEdge: controller.dockEdge == .top ? .bottom : .leading) {
                        AgentHoverTooltipCard(snapshot: snapshot, engine: engine, controller: controller)
                            // 鼠标进入 popover 时取消关闭任务，让按钮可点
                            .onHover { inside in
                                if inside { tooltipCloseTask?.cancel() }
                            }
                    }

                VStack(alignment: .leading, spacing: 1) {
                    Text(snapshot.profile.name)
                        .font(Theme.bodyFont(12.5, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                        .lineLimit(1)
                        .help(snapshot.profile.name)

                    if snapshot.level != .working || snapshot.currentAction == nil {
                        if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                            Text(Self.tokenBadge(usage))
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.onDark.opacity(0.75))
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(Theme.chipFill))
                        } else {
                            Text(snapshot.lastActivityText)
                                .font(Theme.bodyFont(9.5))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                }

                Spacer(minLength: 4)

                // 内存占用与健康状态指示（v1.7.6）
                if snapshot.processRunning && snapshot.memoryBytes > 0 {
                    Text(snapshot.memoryText)
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(RoundedRectangle(cornerRadius: 4).fill(Theme.chipFill))
                        .help("物理内存驻留集 (RSS): \(snapshot.memoryText)")
                }

                if snapshot.isHung {
                    HStack(spacing: 2) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: 8))
                        Text("疑似卡死")
                            .font(Theme.bodyFont(9, weight: .bold))
                    }
                    .foregroundColor(Theme.dangerRed)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2.5)
                    .background(Capsule().fill(Theme.dangerRed.opacity(0.18)))
                    .help("检测到进程持续异常高负荷且缺乏会话响应，疑似处于死循环或线程死锁状态")
                } else {
                    Text(snapshot.level.label)
                        .font(Theme.bodyFont(10, weight: .semibold))
                        .foregroundColor(snapshot.level.color)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2.5)
                        .background(Capsule().fill(snapshot.level.color.opacity(0.16)))
                }

                if snapshot.processRunning {
                    HStack(spacing: 3) {
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
                                    .padding(.horizontal, 5)
                                    .padding(.vertical, 2)
                                    .background(Capsule().fill(Theme.dangerRed.opacity(0.9)))
                            }
                            .buttonStyle(.plain)
                            .help("再次点击立即强制终止该 Agent 进程")
                            .accessibilityLabel("确认终止 \(snapshot.profile.name)")
                            .onAppear {
                                // Task 可随视图销毁取消；DispatchQueue 版本会在行消失后
                                // 继续向失效的 @State 写值
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
                                    .background(Circle().fill(Theme.dangerRed.opacity(0.15)))
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
                                .foregroundColor(Theme.onDark.opacity(0.75))
                                .padding(4)
                                .background(Circle().fill(Theme.chipFill))
                        }
                        .buttonStyle(.plain)
                        .help("查看 \(snapshot.profile.name) 实时事件与输出流水")

                        Button {
                            AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs)
                        } label: {
                            Image(systemName: "arrow.up.forward.app")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundColor(Theme.onDark.opacity(0.75))
                                .padding(4)
                                .background(Circle().fill(Theme.chipFill))
                        }
                        .buttonStyle(.plain)
                        .help("置顶并激活该智能体窗口/终端")
                    }
                }
            }

            // 第二行：工作状态下的专属实时动作横条（全宽展示，彻底根治截断问题）
            if hasActionBar, let action = snapshot.currentAction {
                HStack(spacing: 5) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 8))
                        .foregroundColor(Theme.statusWorking)
                    Text(action)
                        .font(Theme.monoFont(9.5))
                        .foregroundColor(Theme.statusWorking)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 0)
                    // 工作态也保留 token 徽标：正在消耗的 Agent 恰是最需要关注的，
                    // 此前它只在非工作态显示，工作中反而看不到用量
                    if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                        Text(Self.tokenBadge(usage))
                            .font(Theme.monoFont(8))
                            .foregroundColor(Theme.statusWorking.opacity(0.85))
                            .lineLimit(1)
                            .help("24h \(TokenUsage.compact(usage.tokens24h)) token")
                    }
                }
                .padding(.horizontal, 7)
                .padding(.vertical, 3)
                .background(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(Theme.statusWorking.opacity(0.10))
                )
                .padding(.leading, 34) // 与 Agent 名称对齐
                .help(action)
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        // 与横条显示条件严格一致：空字符串动作不显示横条，也不应多出 2pt 内边距
        .padding(.vertical, hasActionBar ? 5 : 4)
        .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: .clear)
        .onTapGesture {
            // 点行进 agent 详情页（原 Finder 跳转移入详情页会话列表）
            controller.route = .agentDetail(snapshot.profile.id)
        }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(snapshot.profile.name)
    }
}

// MARK: - Token 汇总栏（卡片底部，双口径）

struct TokenSummaryBar: View {
    let total: TokenUsage

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "chart.bar.fill")
                .font(.system(size: 9, weight: .semibold))
                .foregroundColor(Theme.onDarkFaint)
            Text("Token 24h \(TokenUsage.compact(total.tokens24h))")
                .font(Theme.monoFont(10, weight: .semibold))
                .foregroundColor(Theme.onDark.opacity(0.85))
                .lineLimit(1)
                .help("24h Token 用量")
            if let cost = cost24hText {
                Text(cost)
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
                    .help("24h 花费")
            }
            Spacer()
            Text("累计 \(TokenUsage.compact(total.tokensTotal))")
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.onDarkFaint)
                .lineLimit(1)
                .help("累计 Token 用量")
            if !TokenUsage.cost(total.costTotal).isEmpty {
                Text(TokenUsage.cost(total.costTotal))
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 7)
    }

    private var cost24hText: String? {
        let c = TokenUsage.cost(total.cost24h)
        return c.isEmpty ? nil : c
    }
}

// MARK: - 顶栏圆形图标按钮底

/// 顶栏图标按钮的圆形底：默认 chipFill，hover 升到 hoverFill。
/// 顶栏此前只有 .help 文案、没有任何视觉反馈，鼠标扫过时无法确认按钮可点
/// （详情页返回键早有同款反馈，此处补齐一致性）。
private struct TopBarChipModifier: ViewModifier {
    @State private var hovering = false

    func body(content: Content) -> some View {
        content
            .background(Circle().fill(hovering ? Theme.hoverFill : Theme.chipFill))
            .onHover { hovering = $0 }
            .animation(.easeOut(duration: 0.12), value: hovering)
    }
}

extension View {
    fileprivate func topBarChip() -> some View {
        modifier(TopBarChipModifier())
    }
}

// MARK: - 收起态边缘微胶囊（DockedSliverCapsule，v1.7.5 视觉强化）
// 设计考量：
// 1. 低功耗零负担：呼吸动画采用平滑缓和的 2.4s 循环，仅在有明确状态（工作或告警）时运行；
// 2. 状态分级：
//    - 告警态 (Alert)：微红/琥珀金光晕与微红呼吸点，第一眼感知死循环或突增事件；
//    - 工作态 (Working)：翠绿微光呼吸流动，多任务时亦能清晰感知运作；
//    - 待机态 (Idle)：优雅深色半透晶莹胶囊，静默无扰；
// 3. 几何适配：顶部边缘 (Top) 横向 140x6pt，右侧边缘 (Right) 纵向 6x120pt。

struct DockedSliverCapsule: View {
    let dockEdge: DockEdge
    let isWorking: Bool
    let hasAlert: Bool
    let onTap: () -> Void
    let onHover: (Bool) -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var breathing = false

    private var activeColor: Color {
        if hasAlert {
            return Theme.dangerRed
        } else if isWorking {
            return Theme.statusWorking
        }
        return Theme.statusIdle
    }

    private var shouldAnimate: Bool {
        hasAlert || isWorking
    }

    /// VoiceOver 标签：把状态说清楚，否则细条只是一个无名可点区域
    private var accessibilityLabelText: String {
        let state = hasAlert ? "有告警" : (isWorking ? "智能体工作中" : "待机")
        let edge = dockEdge == .top ? "顶部" : "右侧"
        return "AgentIsland 灵动岛（\(edge)贴边，\(state)）"
    }

    var body: some View {
        ZStack {
            // 底层胶囊：带微光与反光边
            Capsule()
                .fill(Theme.dockedSliverFill(working: isWorking, alert: hasAlert))
                .overlay(
                    Capsule()
                        .strokeBorder(Theme.dockedSliverStroke(working: isWorking, alert: hasAlert), lineWidth: 0.75)
                )

            // 工作/告警呼吸微光晕
            if shouldAnimate {
                Capsule()
                    .fill(activeColor.opacity(hasAlert ? 0.35 : 0.22))
                    .blur(radius: 2)
                    .opacity(breathing ? 0.9 : 0.25)
            }

            // 中心微呼吸状态点 (4pt)
            if shouldAnimate {
                Circle()
                    .fill(activeColor)
                    .frame(width: 4, height: 4)
                    .scaleEffect(breathing ? 1.15 : 0.85)
                    .opacity(breathing ? 1.0 : 0.6)
            }
        }
        .frame(
            width: dockEdge == .top ? IslandMetrics.topSliverWidth : IslandMetrics.rightSliverWidth,
            height: dockEdge == .top ? IslandMetrics.topSliverHeight : IslandMetrics.rightSliverHeight
        )
        .contentShape(Rectangle())
        .onTapGesture {
            onTap()
        }
        .onHover { hovering in
            onHover(hovering)
        }
        // 收起态细条是面板的第一入口，此前对 VoiceOver 完全不可见
        .accessibilityElement()
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(accessibilityLabelText)
        .accessibilityHint("展开灵动岛卡片")
        .onAppear {
            updateAnimationState()
        }
        .onChange(of: isWorking) { _ in
            updateAnimationState()
        }
        .onChange(of: hasAlert) { _ in
            updateAnimationState()
        }
    }

    private func updateAnimationState() {
        // 尊重系统「减弱动态效果」：开启时只保留静态状态色，不跑无限循环动画
        guard shouldAnimate, !reduceMotion else {
            breathing = false
            return
        }
        breathing = false
        withAnimation(.easeInOut(duration: hasAlert ? 1.2 : 2.0).repeatForever(autoreverses: true)) {
            breathing = true
        }
    }
}

// MARK: - 展开态状态点脉冲动画

struct PulseAnimation: ViewModifier {
    let isActive: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulsing = false

    private func startPulse() {
        guard isActive, !reduceMotion else {
            pulsing = false
            return
        }
        pulsing = false
        withAnimation(.easeOut(duration: 1.1).repeatForever(autoreverses: false)) {
            pulsing = true
        }
    }

    func body(content: Content) -> some View {
        content
            .scaleEffect(pulsing ? 1.0 : 0.6)
            .opacity(pulsing ? 0 : 0.4)
            .onChange(of: isActive) { _ in startPulse() }
            .onAppear { startPulse() }
    }
}

// MARK: - 状态标签

extension ActivityLevel {
    var label: String {
        switch self {
        case .working: return "工作中"
        case .idle: return "待机"
        case .offline: return "离线"
        }
    }

    var color: Color {
        switch self {
        case .working: return Theme.statusWorking
        case .idle: return Theme.statusIdle
        case .offline: return Theme.statusOffline
        }
    }
}

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

                Text(event.summaryText)
                    .font(Theme.bodyFont(11, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(isExpanded ? nil : 1)
                    .fixedSize(horizontal: false, vertical: isExpanded)
                    .help(event.summaryText)

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
                                .font(.system(size: 7, weight: .bold))
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
                        // 动态色：浅色主题下深色块过重且与白玻璃割裂
                        .fill(Color(dynamicLight: 0x000000, dark: 0x000000).opacity(0.06))
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
                                .font(.system(size: 8))
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
                    AppActivator.activate(pid: target.pid, bundleIDs: target.profile.bundleIDs)
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "arrow.up.forward.app")
                            .font(.system(size: 9))
                        Text("直达")
                            .font(Theme.bodyFont(10, weight: .semibold))
                    }
                    .foregroundColor(Theme.onDark)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(Theme.chipFill))
                    .opacity(target == nil ? 0.45 : 1)
                }
                .buttonStyle(.plain)
                .disabled(target == nil)
                .help(target == nil ? "该提醒没有对应的运行中 Agent" : "拉至前台并激活窗口")
                .accessibilityLabel("直达 \(event.agentName) 窗口")
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 6)
        // 动态色：硬编码白色在浅色主题下会让整条横幅失去视觉分组
        .background(event.eventType == .costSpike
                    ? Theme.dangerRed.opacity(0.18)
                    : Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.06))
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
