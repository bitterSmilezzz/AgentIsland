import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 玻璃拟态背景（Q4：NSVisualEffectView + 黑蒙层 + 顶部高光）

struct GlassCardBackground: View {
    var cornerRadius: CGFloat = Theme.radiusLg

    var body: some View {
        ZStack {
            VisualEffectView(material: .hudWindow, blendingMode: .behindWindow)
                .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            // 蒙层：深色下黑蒙，浅色下白蒙（动态）
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                .fill(Color(dynamicLight: 0xffffff, dark: 0x000000).opacity(0.42))
            // 无描边：灵动岛悬浮质感靠材质对比 + 阴影，描边在浅色下会呈现为矩形框
        }
        // 阴影由 ShadowHostView（AppKit layer）绘制，这里不再加 SwiftUI 阴影
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
    case docked         // QQ 式贴边细条（始终可见，仅露 5px）
    case expanded       // 下拉展开卡片（多内容）
}

// MARK: - 卡内导航（主列表 → agent 详情 → 模型会话列表）

enum CardRoute: Equatable {
    case list                     // 主卡：agent 列表 + 汇总栏
    case agentDetail(String)      // agent 详情：总览 + 模型拆分
    case sessions(String, String) // agentId + modelId：该模型会话列表
}

// MARK: - 灵动岛视图
// QQ 交互逻辑：细条常驻 → 鼠标触碰屏幕顶部/悬停细条 → 直接滑出卡片
// → 鼠标离开 → 延迟收回细条。忙闲都显示细条（菜单栏绿点 + 细条颜色深浅表状态）。

struct IslandView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    /// 可见列表（Q3）：在线 + 24h 内有活动的 Agent；从未活跃的隐藏
    // 可见列表统一走 engine.visibleSnapshots（口径单一实现，防漏改）

    var body: some View {
        ZStack(alignment: .top) {
            if controller.displayState == .expanded {
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
                .transition(.asymmetric(
                    insertion: .opacity.combined(with: .scale(scale: 0.85, anchor: .top)),
                    removal: .opacity.combined(with: .scale(scale: 0.85, anchor: .top))
                ))
            } else {
                dockedSliver
                    .transition(.opacity)
            }
        }
        // 成对弹簧（boring.notch 最佳实践）：展开有回弹，收起零过冲更干脆
        .animation(controller.displayState == .expanded
                   ? .spring(response: 0.42, dampingFraction: 0.8)
                   : .spring(response: 0.45, dampingFraction: 1.0),
                   value: controller.displayState)
        .animation(.spring(response: 0.35, dampingFraction: 0.85), value: controller.route)
        .onHover { hovering in
            controller.islandHoveredChanged(hovering)
        }
    }

    // MARK: 贴边细条（E1：QQ 式收起态，仅露 5px；E3：忙闲都常驻）

    private var dockedSliver: some View {
        Capsule()
            // 浅色模式主体加深：5pt 细条在亮壁纸上 0.22 几乎不可见，0.34 起才稳定可见
            // 动态色随面板 effectiveAppearance 切换（避免 @Environment(\.colorScheme) 与强制外观不同步）
            .fill(Color(dynamic: NSColor(hex: 0x000000, alpha: engine.anyWorking ? 0.55 : 0.34),
                        dark: NSColor(hex: 0x000000, alpha: engine.anyWorking ? 0.85 : 0.55)))
            .overlay(alignment: .leading) {
                // 忙碌时左侧一粒绿点（细条内的微状态提示）
                if engine.anyWorking {
                    Circle()
                        .fill(Theme.statusWorking)
                        .frame(width: 3, height: 3)
                        .padding(.leading, 3)
                }
            }
            .overlay(Capsule().strokeBorder(
                Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.22), lineWidth: 0.5))
            .frame(width: IslandMetrics.dockedWidth, height: IslandMetrics.dockedHeight)
            .contentShape(Capsule())
            // 点击细条兜底展开（悬停通常已展开；防止 mouseMoved 事件被系统吞掉时无响应）
            .onTapGesture {
                if controller.displayState == .docked {
                    controller.toggle()
                }
            }
    }

    // MARK: 展开卡片

    private var expandedCard: some View {
        VStack(alignment: .leading, spacing: 0) {
            // 顶栏：状态摘要（D3：顶栏可拖动整卡）
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
            .gesture(
                DragGesture(minimumDistance: 3)
                    .onChanged { value in
                        controller.dragMoved(translation: value.translation)
                    }
                    .onEnded { _ in
                        controller.dragEnded()
                    }
            )

            Divider().overlay(Theme.onDark.opacity(0.12))

            // Agent 列表（Q3：只显示在线 + 24h 活跃）
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

            // Token 汇总栏（双口径：24h / 累计；有数据才显示）
            if !engine.grandTotal.isEmpty {
                Divider().overlay(Theme.onDark.opacity(0.12))
                TokenSummaryBar(total: engine.grandTotal)
            }
        }
        .frame(width: IslandMetrics.cardWidth)
        // 背景铺满整个窗口（窗口高度可能略大于内容，消除底部透明带）
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(GlassCardBackground(cornerRadius: Theme.radiusLg))
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
    @State private var hovering = false

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
        .background(
            // 圆角与详情页模型/会话行统一为 Theme.radiusSm（阿菜低5）
            RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous)
                .fill(hovering ? Theme.hoverFill : Color.clear)
        )
        .contentShape(RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous))
        .onHover { hovering = $0 }
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

extension Notification.Name {
    /// 设置窗口切换外观模式（同进程内可靠触发）
    static let islandAppearanceChanged = Notification.Name("islandAppearanceChanged")
}
