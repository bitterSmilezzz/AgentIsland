import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 玻璃拟态背景（Q4：NSVisualEffectView + 黑蒙层 + 顶部高光）

struct GlassCardBackground: View {
    var cornerRadius: CGFloat = Theme.radiusLg
    var dockEdge: DockEdge = .right

    /// 贴边造型：采用 SideNotchShape 赋予反向倒角一体化贴边质感
    private var notchShape: SideNotchShape {
        SideNotchShape(dockEdge: dockEdge, curlRadius: 10, cornerRadius: cornerRadius)
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
        guard let event = engine.latestEvent else { return false }
        return event.eventType == .costSpike || event.eventType == .attention
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
                if let active = engine.visibleSnapshots.first(where: { $0.level == .working }),
                   let action = active.currentAction {
                    Text("\(active.profile.name): \(action)")
                        .font(Theme.bodyFont(13, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                        .lineLimit(1)
                } else {
                    Text(engine.anyWorking
                         ? "\(engine.workingAgents().count) 个 Agent 正在工作"
                         : "当前没有 Agent 在工作")
                        .font(Theme.bodyFont(13, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                }
                Spacer(minLength: 4)
                Text("\(engine.visibleSnapshots.count)/\(engine.snapshots.count) 可见")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.onDarkFaint)

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
                        .background(Circle().fill(Theme.chipFill))
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("外观主题：\(controller.appearanceMode.label)（点击切换）")

                // 工作台快捷维护工具箱（v1.7.7）
                Button {
                    controller.route = .toolbox
                } label: {
                    Image(systemName: "wrench.and.screwdriver")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(4)
                        .background(Circle().fill(Theme.chipFill))
                }
                .buttonStyle(.plain)
                .help("智能体维护工作台：扫描清理孤儿进程、死锁与内存泄露")

                // 一键收起按钮
                Button {
                    controller.collapse()
                } label: {
                    Image(systemName: controller.dockEdge == .top ? "chevron.up" : "chevron.right")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(4)
                        .background(Circle().fill(Theme.chipFill))
                }
                .buttonStyle(.plain)
                .help("收起灵动岛（或光标移出卡片自动收起）")
            }
            .padding(.horizontal, Theme.pageMargin)
            .padding(.top, IslandMetrics.headerPaddingTop)
            .padding(.bottom, IslandMetrics.headerPaddingBottom)
            .cardDrag(controller: controller)

            DarkDivider()

            // 实时活动环微看板（Quick Rings Shelf · CodeNotch 灵感）
            let activeSnapshots = engine.visibleSnapshots.filter { $0.level == .working || ($0.tokenUsage?.tokens24h ?? 0) > 0 }
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
                                        Text(snap.level == .working ? "工作中" : (snap.tokenUsage.map { TokenUsage.compact($0.tokens24h) } ?? snap.level.label))
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
                            .help("\(snap.profile.name): \(snap.level.label)")
                        }
                    }
                    .padding(.horizontal, Theme.pageMargin)
                    .padding(.vertical, 5)
                }
                DarkDivider()
            }

            // 任务事件横幅（完成/等待确认/资源与Token熔断告警富文本交互卡片）
            if let event = engine.latestEvent {
                EventBannerView(event: event, engine: engine, controller: controller)
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
                .frame(maxHeight: IslandMetrics.listMaxHeight)
            }

            // Token 汇总栏
            if !engine.grandTotal.isEmpty {
                DarkDivider()
                TokenSummaryBar(total: engine.grandTotal)
            }
        }
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
    }

    // MARK: 状态点

    private var statusDot: some View {
        ZStack {
            Circle().fill(statusColor)
            if engine.anyWorking {
                Circle()
                    .fill(statusColor)
                    .scaleEffect(1.6)
                    .opacity(0.35)
                    // 仅展开态运行动画（阿证中2：docked 态 repeatForever 60fps 布局风暴，
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

    /// Token 徽标文本："1.23M" 或 "1.23M $0.42"
    static func tokenBadge(_ usage: TokenUsage) -> String {
        let tokens = TokenUsage.compact(usage.tokens24h)
        let cost = TokenUsage.cost(usage.cost24h)
        return cost.isEmpty ? tokens : "\(tokens) \(cost)"
    }

    var body: some View {
        HStack(spacing: 10) {
            AgentRingView(snapshot: snapshot, size: 28)
                .onHover { h in
                    showingTooltip = h
                }
                .popover(isPresented: $showingTooltip, arrowEdge: controller.dockEdge == .top ? .bottom : .leading) {
                    AgentHoverTooltipCard(snapshot: snapshot, engine: engine, controller: controller)
                }

            VStack(alignment: .leading, spacing: 2) {
                Text(snapshot.profile.name)
                    .font(Theme.bodyFont(12, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(1)
                    .help(snapshot.profile.name)
                HStack(spacing: 6) {
                    if snapshot.level == .working, let action = snapshot.currentAction {
                        HStack(spacing: 3) {
                            Image(systemName: "terminal.fill")
                                .font(.system(size: 8))
                                .foregroundColor(Theme.statusWorking)
                            Text(action)
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.statusWorking)
                                .lineLimit(1)
                        }
                    } else if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                        Text(Self.tokenBadge(usage))
                            .font(Theme.monoFont(9))
                            .foregroundColor(Theme.onDark.opacity(0.75))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(Capsule().fill(Theme.chipFill))
                    } else {
                        Text(snapshot.lastActivityText)
                            .font(Theme.bodyFont(10))
                            .foregroundColor(Theme.onDarkFaint)
                    }
                }
            }

            Spacer()

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
                .padding(.vertical, 3)
                .background(Capsule().fill(Theme.dangerRed.opacity(0.18)))
                .help("检测到进程持续异常高负荷且缺乏会话响应，疑似处于死循环或线程死锁状态")
            } else {
                Text(snapshot.level.label)
                    .font(Theme.bodyFont(10, weight: .semibold))
                    .foregroundColor(snapshot.level.color)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Capsule().fill(snapshot.level.color.opacity(0.16)))
            }

            if snapshot.processRunning {
                HStack(spacing: 4) {
                    if confirmingKill {
                        Button {
                            engine.terminateAgent(pid: snapshot.pid, agentId: snapshot.profile.id)
                            confirmingKill = false
                        } label: {
                            Text("终止?")
                                .font(Theme.bodyFont(10, weight: .bold))
                                .foregroundColor(.white)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 3)
                                .background(Capsule().fill(Color.red.opacity(0.9)))
                        }
                        .buttonStyle(.plain)
                        .help("再次点击立即强制终止该 Agent 进程")
                        .onAppear {
                            DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) {
                                confirmingKill = false
                            }
                        }
                    } else if snapshot.level == .working {
                        Button {
                            confirmingKill = true
                        } label: {
                            Image(systemName: "xmark.circle")
                                .font(.system(size: 11, weight: .medium))
                                .foregroundColor(Color.red.opacity(0.85))
                                .padding(5)
                                .background(Circle().fill(Color.red.opacity(0.15)))
                        }
                        .buttonStyle(.plain)
                        .help("一键终止逃生舱：关闭该正在运行的 Agent 及其子任务")
                    }

                    Button {
                        controller.route = .liveStream(snapshot.profile.id)
                    } label: {
                        Image(systemName: "terminal")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundColor(Theme.onDark.opacity(0.7))
                            .padding(5)
                            .background(Circle().fill(Theme.chipFill))
                    }
                    .buttonStyle(.plain)
                    .help("查看 \(snapshot.profile.name) 实时事件与输出流水")

                    Button {
                        AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs)
                    } label: {
                        Image(systemName: "arrow.up.forward.app")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundColor(Theme.onDark.opacity(0.7))
                            .padding(5)
                            .background(Circle().fill(Theme.chipFill))
                    }
                    .buttonStyle(.plain)
                    .help("置顶并激活该智能体窗口/终端")
                }
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 7)
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
        if shouldAnimate {
            breathing = false
            withAnimation(.easeInOut(duration: hasAlert ? 1.2 : 2.0).repeatForever(autoreverses: true)) {
                breathing = true
            }
        } else {
            breathing = false
        }
    }
}

// MARK: - 展开态状态点脉冲动画

struct PulseAnimation: ViewModifier {
    let isActive: Bool
    @State private var pulsing = false

    func body(content: Content) -> some View {
        content
            .scaleEffect(pulsing ? 1.0 : 0.6)
            .opacity(pulsing ? 0 : 0.4)
            .onChange(of: isActive) { active in
                if active {
                    pulsing = false
                    withAnimation(.easeOut(duration: 1.1).repeatForever(autoreverses: false)) {
                        pulsing = true
                    }
                } else {
                    pulsing = false
                }
            }
            .onAppear {
                guard isActive else { return }
                pulsing = false
                withAnimation(.easeOut(duration: 1.1).repeatForever(autoreverses: false)) {
                    pulsing = true
                }
            }
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
                        .background(Capsule().fill(Color.white.opacity(0.08)))
                    }
                    .buttonStyle(.plain)
                    .help(isExpanded ? "收起排查详情" : "展开警告产生的原因与排查建议")
                }

                Button {
                    controller.eventBannerExpanded = false
                    engine.clearLatestEvent()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(3)
                }
                .buttonStyle(.plain)
                .help("关闭提醒")
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
                        .fill(Color.black.opacity(0.24))
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
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
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
                        .background(Capsule().fill(Color.white.opacity(0.06)))
                    }
                    .buttonStyle(.plain)
                    .help("一键复制告警信息、PID 及触发时间戳")
                }

                Spacer()

                if event.eventType == .costSpike, let pid = event.pid {
                    Button {
                        engine.terminateAgent(pid: pid, agentId: event.agentId)
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
                        .background(Capsule().fill(Color.red.opacity(0.85)))
                    }
                    .buttonStyle(.plain)
                    .help("立即终止该 Agent 进程树，阻止持续消耗")
                }

                Button {
                    if let snap = engine.snapshots.first(where: { $0.id == event.agentId }) {
                        AppActivator.activate(pid: snap.pid, bundleIDs: snap.profile.bundleIDs)
                    }
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
                }
                .buttonStyle(.plain)
                .help("拉至前台并激活窗口")
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 6)
        .background(event.eventType == .costSpike ? Color.red.opacity(0.18) : Color.white.opacity(0.06))
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
        case .attention: return .orange
        case .costSpike: return Color.red
        }
    }
}
