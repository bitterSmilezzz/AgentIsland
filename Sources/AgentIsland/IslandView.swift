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
        .shadow(color: Color.black.opacity(0.30), radius: 16, y: 6)
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
    private var visibleSnapshots: [AgentSnapshot] {
        engine.snapshots.filter {
            $0.processRunning || ($0.lastActivityAgo ?? .infinity) < 24 * 3600
        }
    }

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
        .animation(.spring(response: 0.42, dampingFraction: 0.78), value: controller.displayState)
        .animation(.spring(response: 0.35, dampingFraction: 0.85), value: controller.route)
        .onHover { hovering in
            controller.islandHoveredChanged(hovering)
        }
    }

    // MARK: 贴边细条（E1：QQ 式收起态，仅露 5px；E3：忙闲都常驻）

    @Environment(\.colorScheme) private var colorScheme
    private var isLight: Bool { colorScheme == .light }

    private var dockedSliver: some View {
        Capsule()
            .fill(Color.black.opacity(engine.anyWorking ? (isLight ? 0.50 : 0.85) : (isLight ? 0.22 : 0.55)))
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
                Color(dynamicLight: 0x000000, dark: 0xffffff).opacity(0.18), lineWidth: 0.5))
            .frame(width: 176, height: 5)
            .contentShape(Capsule())
            .onTapGesture { controller.toggle() }
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
                Text("\(visibleSnapshots.count)/\(engine.snapshots.count) 可见")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.onDarkFaint)
            }
            .padding(.horizontal, 14)
            .padding(.top, 12)
            .padding(.bottom, 8)
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
            if visibleSnapshots.isEmpty {
                VStack(spacing: 6) {
                    Image(systemName: "zzz")
                        .font(.system(size: 22))
                        .foregroundColor(Theme.onDarkFaint)
                    Text("没有活跃的 Agent")
                        .font(Theme.bodyFont(12))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 28)
            } else {
                ScrollView(.vertical, showsIndicators: true) {
                    VStack(spacing: 2) {
                        ForEach(visibleSnapshots) { snapshot in
                            AgentRowView(snapshot: snapshot, controller: controller)
                        }
                    }
                    .padding(.vertical, 6)
                }
                .frame(maxHeight: 300)
            }

            // Token 汇总栏（双口径：24h / 累计；有数据才显示）
            if !engine.tokenMonitor.grandTotal.isEmpty {
                Divider().overlay(Theme.onDark.opacity(0.12))
                TokenSummaryBar(total: engine.tokenMonitor.grandTotal)
            }
        }
        .frame(width: 280)
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
                    .modifier(PulseAnimation())
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
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(hovering ? Theme.hoverFill : Color.clear)
        )
        .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .onHover { hovering = $0 }
        .onTapGesture {
            // 点行进 agent 详情页（原 Finder 跳转移入详情页会话列表）
            controller.route = .agentDetail(snapshot.profile.id)
        }
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
            if let cost = cost24hText {
                Text(cost)
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
            }
            Spacer()
            Text("累计 \(TokenUsage.compact(total.tokensTotal))")
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.onDarkFaint)
            if !TokenUsage.cost(total.costTotal).isEmpty {
                Text(TokenUsage.cost(total.costTotal))
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.onDarkFaint)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    private var cost24hText: String? {
        let c = TokenUsage.cost(total.cost24h)
        return c.isEmpty ? nil : c
    }
}

// MARK: - 呼吸动画

struct PulseAnimation: ViewModifier {
    @State private var pulsing = false

    func body(content: Content) -> some View {
        content
            .scaleEffect(pulsing ? 1.0 : 0.6)
            .opacity(pulsing ? 0 : 0.4)
            .onAppear {
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

// MARK: - 全局事件（悬停状态传递）

final class AppEvents {
    static let shared = AppEvents()
    var islandHovered = false
}

extension Notification.Name {
    /// 设置窗口切换外观模式（同进程内可靠触发）
    static let islandAppearanceChanged = Notification.Name("islandAppearanceChanged")
}
