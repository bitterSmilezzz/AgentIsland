import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 玻璃拟态背景（Q4：NSVisualEffectView + 黑蒙层 + 顶部高光）

struct GlassCardBackground: View {
    var cornerRadius: CGFloat = Theme.radiusLg
    var dockEdge: DockEdge = .right

    /// 贴边造型：右侧贴边左侧两角圆角；顶部贴边下方两角圆角
    private var edgeShape: UnevenRoundedRectangle {
        if dockEdge == .top {
            return UnevenRoundedRectangle(topLeadingRadius: 0,
                                          bottomLeadingRadius: cornerRadius,
                                          bottomTrailingRadius: cornerRadius,
                                          topTrailingRadius: 0,
                                          style: .continuous)
        } else {
            return UnevenRoundedRectangle(topLeadingRadius: cornerRadius,
                                          bottomLeadingRadius: cornerRadius,
                                          bottomTrailingRadius: 0,
                                          topTrailingRadius: 0,
                                          style: .continuous)
        }
    }

    var body: some View {
        ZStack {
            VisualEffectView(material: .hudWindow, blendingMode: .behindWindow)
                .clipShape(edgeShape)
            // 蒙层：深色下黑蒙，浅色下白蒙（动态）
            edgeShape
                .fill(Color(dynamicLight: 0xffffff, dark: 0x000000).opacity(Theme.glassOverlayOpacity))
            // 1px 晶莹微反光描边（深色微白高光，浅色微暗勾边）
            edgeShape
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
}

// MARK: - 灵动岛视图

struct IslandView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    var body: some View {
        Group {
            if controller.displayState == .expanded {
                expandedContent
                    .transition(.opacity)
            } else {
                dockedSliver
                    .transition(.opacity)
            }
        }
        .animation(.spring(response: 0.34, dampingFraction: 0.82), value: controller.displayState)
        .animation(.spring(response: 0.34, dampingFraction: 0.82), value: controller.route)
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
            }
        }
    }

    // MARK: 贴边微细条（露 6pt，晶莹质感 + 呼吸状态点）

    private var dockedSliver: some View {
        Group {
            if controller.dockEdge == .top {
                Capsule()
                    .fill(Theme.dockedSliverFill(working: engine.anyWorking))
                    .overlay(alignment: .center) {
                        if engine.anyWorking {
                            Circle()
                                .fill(Theme.statusWorking)
                                .frame(width: 4, height: 4)
                        }
                    }
                    .overlay(Capsule().strokeBorder(Theme.dockedSliverStroke, lineWidth: 0.5))
                    .frame(width: IslandMetrics.topSliverWidth, height: IslandMetrics.topSliverHeight)
            } else {
                Capsule()
                    .fill(Theme.dockedSliverFill(working: engine.anyWorking))
                    .overlay(alignment: .center) {
                        if engine.anyWorking {
                            Circle()
                                .fill(Theme.statusWorking)
                                .frame(width: 4, height: 4)
                        }
                    }
                    .overlay(Capsule().strokeBorder(Theme.dockedSliverStroke, lineWidth: 0.5))
                    .frame(width: IslandMetrics.rightSliverWidth, height: IslandMetrics.rightSliverHeight)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            if controller.displayState == .docked {
                controller.toggle()
            }
        }
        .onHover { hovering in
            if hovering && controller.displayState == .docked {
                controller.expandFromHover()
            }
        }
    }

    // MARK: 展开卡片

    private var expandedCard: some View {
        VStack(alignment: .leading, spacing: 0) {
            // 顶栏：状态摘要（支持长按拖拽卡片自由移动并贴边吸附）
            HStack(spacing: 8) {
                statusDot
                    .frame(width: 9, height: 9)
                Text(engine.anyWorking
                     ? "\(engine.workingAgents().count) 个 Agent 正在工作"
                     : "当前没有 Agent 在工作")
                    .font(Theme.bodyFont(13, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                Spacer()
                Text("\(engine.visibleSnapshots.count)/\(engine.snapshots.count) 可见")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.onDarkFaint)
            }
            .padding(.horizontal, Theme.pageMargin)
            .padding(.top, IslandMetrics.headerPaddingTop)
            .padding(.bottom, IslandMetrics.headerPaddingBottom)
            .contentShape(Rectangle())
            .cardDrag(
                onMoved: { controller.dragMoved(translation: $0) },
                onEnded: { controller.dragEnded() }
            )

            DarkDivider()

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
                            AgentRowView(snapshot: snapshot, controller: controller)
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
        .cardShell(dockEdge: controller.dockEdge)
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
    @ObservedObject var controller: IslandPanelController

    /// Token 徽标文本："1.23M" 或 "1.23M $0.42"
    static func tokenBadge(_ usage: TokenUsage) -> String {
        let tokens = TokenUsage.compact(usage.tokens24h)
        let cost = TokenUsage.cost(usage.cost24h)
        return cost.isEmpty ? tokens : "\(tokens) \(cost)"
    }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: snapshot.profile.icon)
                .font(.system(size: 13, weight: .medium))
                .foregroundColor(Theme.onDark)
                .frame(width: 26, height: 26)
                .background(Circle().fill(Theme.tile1))

            VStack(alignment: .leading, spacing: 2) {
                Text(snapshot.profile.name)
                    .font(Theme.bodyFont(12, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(1)
                    .help(snapshot.profile.name)
                HStack(spacing: 6) {
                    // Token 徽标：24h 净消耗 + 花费（有数据才显示）
                    if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
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

            Text(snapshot.level.label)
                .font(Theme.bodyFont(10, weight: .semibold))
                .foregroundColor(snapshot.level.color)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Capsule().fill(snapshot.level.color.opacity(0.16)))
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

// MARK: - 呼吸动画

struct PulseAnimation: ViewModifier {
    /// 是否运行动画：isActive=false 时停止 repeatForever 并复位
    /// （SwiftUI repeatForever 无法取消，只能通过条件移除动画环境实现）
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
