import AgentIslandCore
import SwiftUI

// MARK: - 卡内二级/三级详情页
// 主卡列表 → 点行 → AgentDetailView（总览+模型拆分）→ 点模型 → SessionListView（会话列表）

// MARK: - 二级页顶栏（返回 + 标题；M4：与主列表一致可拖动整卡）

struct DetailHeader: View {
    let title: String
    var subtitle: String?
    let onBack: () -> Void
    /// 拖动整卡（与主列表顶栏一致；由外部传入 controller 的 drag 处理）
    var onDragMoved: ((CGSize) -> Void)?
    var onDragEnded: (() -> Void)?
    @State private var backHovered = false

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onBack) {
                Image(systemName: "chevron.left")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(Theme.onDark.opacity(0.9))
                    .frame(width: 28, height: 28)
                    .background(Circle().fill(backHovered ? Theme.hoverFill : Theme.chipFill))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .onHover { backHovered = $0 }
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(Theme.bodyFont(12, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(1)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                        .lineLimit(1)
                }
            }
            Spacer()
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
        .padding(.bottom, 8)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 3)
                .onChanged { value in onDragMoved?(value.translation) }
                .onEnded { _ in onDragEnded?() }
        )
    }
}

// MARK: - Agent 详情页

struct AgentDetailView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    let agentId: String

    @State private var models: [ModelUsage] = []
    @State private var loading = true
    @State private var queryToken = UUID()
    @State private var modelHoveredID: String?

    private var snapshot: AgentSnapshot? {
        engine.snapshots.first { $0.id == agentId }
    }
    private var usage: TokenUsage? { snapshot?.tokenUsage }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(title: snapshot?.profile.name ?? agentId,
                         subtitle: usage.map {
                             "24h \(TokenUsage.compact($0.tokens24h)) · 累计 \(TokenUsage.compact($0.tokensTotal))"
                         },
                         onBack: { controller.route = .list },
                         onDragMoved: { controller.dragMoved(translation: $0) },
                         onDragEnded: { controller.dragEnded() })

            Divider().overlay(Theme.onDark.opacity(0.12))

            ScrollView(.vertical, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 10) {
                    if loading {
                        HStack { Spacer(); ProgressView().controlSize(.small); Spacer() }
                            .padding(.top, 36)
                    } else if let usage, !usage.isEmpty {
                        overviewCard(usage)
                        if !models.isEmpty { modelList }
                    } else {
                        basicInfoCard
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
            }
        }
        .frame(width: 280)
        .background(GlassCardBackground(cornerRadius: Theme.radiusLg))
        .task(id: agentId) {
            loading = true
            models = []
            let token = UUID()   // 代际标记：防止旧查询结果覆盖已切换的新页面
            queryToken = token
            engine.tokenMonitor.modelBreakdown(agentId: agentId) { rows in
                guard queryToken == token else { return }
                models = rows
                loading = false
            }
        }
    }

    private func overviewCard(_ u: TokenUsage) -> some View {
        HStack(spacing: 0) {
            overviewCell("24h", TokenUsage.compact(u.tokens24h),
                         cost: TokenUsage.cost(u.cost24h).isEmpty ? nil : TokenUsage.cost(u.cost24h))
            Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 30)
            overviewCell("累计", TokenUsage.compact(u.tokensTotal),
                         cost: TokenUsage.cost(u.costTotal).isEmpty ? nil : TokenUsage.cost(u.costTotal))
        }
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.cardFill))
    }

    private func overviewCell(_ label: String, _ value: String, cost: String?) -> some View {
        VStack(spacing: 3) {
            Text(value)
                .font(Theme.monoFont(15, weight: .bold))
                .foregroundColor(Theme.onDark)
            HStack(spacing: 4) {
                Text(label)
                    .font(Theme.bodyFont(9))
                    .foregroundColor(Theme.onDarkFaint)
                if let cost {
                    Text(cost)
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var modelList: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("按模型")
                .font(Theme.bodyFont(10, weight: .semibold))
                .foregroundColor(Theme.onDarkFaint)
            ForEach(models) { m in
                HStack(spacing: 8) {
                    Image(systemName: "cube.transparent")
                        .font(.system(size: 10))
                        .foregroundColor(Theme.onDarkFaint)
                        .frame(width: 14)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(m.modelId)
                            .font(Theme.monoFont(10, weight: .semibold))
                            .foregroundColor(Theme.onDark)
                            .lineLimit(1)
                        HStack(spacing: 5) {
                            Text("\(TokenUsage.compact(m.tokens)) tok")
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.onDarkFaint)
                            let c = TokenUsage.cost(m.cost)
                            if !c.isEmpty {
                                Text(c)
                                    .font(Theme.monoFont(9))
                                    .foregroundColor(Theme.onDarkFaint)
                            }
                            Text("\(m.messages) 次")
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.system(size: 8, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(modelHoveredID == m.modelId ? Theme.hoverFill : Theme.chipFill))
                .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                .onHover { modelHoveredID = $0 ? m.modelId : nil }
                .onTapGesture {
                    controller.route = .sessions(agentId, m.modelId)
                }
            }
        }
    }

    /// 无 token 数据的 agent：显示主卡瘦身撤下的基础信息
    private var basicInfoCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let s = snapshot {
                infoRow("状态", s.level.label)
                infoRow("活动", s.lastActivityText)
                if s.activeSessions > 0 { infoRow("会话", "\(s.activeSessions) 个活跃") }
                if s.cpuPercent > 1 { infoRow("CPU", String(format: "%.0f%%", s.cpuPercent)) }
                infoRow("Token", "暂无本地 token 数据")
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.cardFill))
    }

    private func infoRow(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label)
                .font(Theme.bodyFont(10))
                .foregroundColor(Theme.onDarkFaint)
                .frame(width: 42, alignment: .leading)
            Text(value)
                .font(Theme.bodyFont(10, weight: .medium))
                .foregroundColor(Theme.onDark)
            Spacer()
        }
    }
}

// MARK: - 会话列表页（agent × 模型）

struct SessionListView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    let agentId: String
    let modelId: String

    @State private var sessions: [SessionUsage] = []
    @State private var loading = true
    @State private var queryToken = UUID()
    /// 当前 hover 的会话行 id（多行共享一个状态，避免每行各自 @State）
    @State private var hoveredSessionID: String?

    /// 静态化：行 body 每次重算不再新建 DateFormatter（昂贵对象）
    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "MM-dd HH:mm"
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(title: modelId,
                         subtitle: "\(sessions.count) 个会话",
                         onBack: { controller.route = .agentDetail(agentId) },
                         onDragMoved: { controller.dragMoved(translation: $0) },
                         onDragEnded: { controller.dragEnded() })

            Divider().overlay(Theme.onDark.opacity(0.12))

            ScrollView(.vertical, showsIndicators: false) {
                VStack(spacing: 4) {
                    if loading {
                        HStack { Spacer(); ProgressView().controlSize(.small); Spacer() }
                            .padding(.top, 36)
                    } else if sessions.isEmpty {
                        Text("该模型暂无会话记录")
                            .font(Theme.bodyFont(11))
                            .foregroundColor(Theme.onDarkFaint)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 28)
                    } else {
                        ForEach(sessions) { s in
                            sessionRow(s)
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
            }
        }
        .frame(width: 280)
        .background(GlassCardBackground(cornerRadius: Theme.radiusLg))
        .task(id: "\(agentId)/\(modelId)") {
            loading = true
            sessions = []
            let token = UUID()   // 代际标记：防止旧查询结果覆盖已切换的新页面
            queryToken = token
            engine.tokenMonitor.sessions(agentId: agentId, modelId: modelId) { rows in
                guard queryToken == token else { return }
                sessions = rows
                loading = false
            }
        }
    }

    private func sessionRow(_ s: SessionUsage) -> some View {
        let timeText: String = {
            guard let t = s.lastTime else { return "—" }
            return SessionListView.timeFormatter.string(from: t)
        }()
        let detail = "\(s.messages) 条 · \(TokenUsage.compact(s.tokens)) tok"
            + (TokenUsage.cost(s.cost).isEmpty ? "" : " · \(TokenUsage.cost(s.cost))")
        let isHovered = hoveredSessionID == s.id
        let hasDir = s.directory != nil
        return HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(timeText)
                    .font(Theme.monoFont(10, weight: .semibold))
                    .foregroundColor(hasDir ? Theme.onDark : Theme.onDark.opacity(0.55))
                Text(detail)
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkFaint)
                    .lineLimit(1)
            }
            Spacer()
            if hasDir {
                Image(systemName: "folder")
                    .font(.system(size: 10))
                    .foregroundColor(Theme.onDarkFaint)
            } else {
                // L7：无目录的会话行视觉降级（不可点）
                Text("无目录")
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkFaint.opacity(0.6))
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(isHovered && hasDir ? Theme.hoverFill : Theme.chipFill))
        .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .onHover { hovering in
            hoveredSessionID = hovering ? s.id : nil
        }
        .onDisappear {
            if hoveredSessionID == s.id { hoveredSessionID = nil }
        }
        .onTapGesture {
            if let dir = s.directory {
                NSWorkspace.shared.open(URL(fileURLWithPath: dir))
            }
        }
        .opacity(hasDir ? 1.0 : 0.75)
    }
}
