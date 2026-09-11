import AgentIslandCore
import SwiftUI

// MARK: - 卡内二级/三级详情页
// 主卡列表 → 点行 → AgentDetailView（总览+模型拆分）→ 点模型 → SessionListView（会话列表）

// MARK: - 二级页顶栏（返回 + 标题；M4：与主列表一致可拖动整卡）

struct DetailHeader: View {
    let title: String
    let subtitle: String?
    let onBack: () -> Void
    /// 面板控制器：拖拽统一走原生 `performDrag`（与主卡同一实现）。
    /// 早期这里走的是闭包版 `cardDrag(onMoved:onEnded:)`，而该闭包版把 translation
    /// 恒传 `.zero`（`onDragStart` 无位移信息），使得控制器里那套坐标钳制逻辑从不生效，
    /// 真正移动窗口的一直是底层的 performDrag——两套机制并存只会让后来者误判。
    let controller: IslandPanelController
    @State private var backHovered = false

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onBack) {
                Image(systemName: "chevron.left")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundColor(Theme.onDark.opacity(0.9))
                    .frame(width: 24, height: 24)
                    .background(Circle().fill(backHovered ? Theme.hoverFill : Theme.chipFill))
                    .contentShape(Circle())
                    .help("返回")
            }
            .buttonStyle(.plain)
            .onHover { backHovered = $0 }
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel("返回")
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(Theme.bodyFont(12, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(1)
                    .help(title)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                        .lineLimit(1)
                        .help(subtitle)
                }
            }
            Spacer()
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.top, IslandMetrics.detailHeaderPaddingTop)
        .padding(.bottom, IslandMetrics.detailHeaderPaddingBottom)
        .contentShape(Rectangle())
        .cardDrag(controller: controller)
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
                         controller: controller)

            DarkDivider()

            // GeometryReader 必须在 ScrollView 外层（同 SessionListView，防长内容不可滚动）
            GeometryReader { geo in
                ScrollView(.vertical, showsIndicators: true) {
                    VStack(alignment: .leading, spacing: 10) {
                        if loading {
                            // 与 SessionListView 一致的居中 loading
                            CenteredSpinner()
                                .frame(maxWidth: .infinity, minHeight: max(geo.size.height - 20, 0))
                        } else {
                            // 统一垂直居中：内容短时居中消除贴顶留白，
                            // 内容超过视口时 Spacer(minLength:0) 归零、贴顶正常滚动
                            Spacer(minLength: 0)
                            Group {
                                if snapshot == nil {
                                    // agent 被禁用/移除后仍停留在详情页时，此前内容区一片空白
                                    VStack(spacing: 6) {
                                        Image(systemName: "questionmark.circle")
                                            .font(.system(size: 20))
                                            .foregroundColor(Theme.onDarkFaint)
                                        Text("该智能体已不在监控列表")
                                            .font(Theme.bodyFont(11))
                                            .foregroundColor(Theme.onDarkFaint)
                                        Button("返回列表") { controller.route = .list }
                                            .buttonStyle(.plain)
                                            .font(Theme.bodyFont(11, weight: .semibold))
                                            .foregroundColor(Theme.actionBlue)
                                    }
                                    .frame(maxWidth: .infinity)
                                } else if let usage, !usage.isEmpty {
                                    VStack(alignment: .leading, spacing: 10) {
                                        overviewCard(usage)
                                        if let s = snapshot, s.processRunning {
                                            performanceCard(s)
                                        }
                                        if !models.isEmpty {
                                            modelList
                                        }
                                    }
                                } else {
                                    VStack(alignment: .leading, spacing: 10) {
                                        basicInfoCard
                                        if let s = snapshot, s.processRunning {
                                            performanceCard(s)
                                        }
                                    }
                                }
                            }
                            Spacer(minLength: 0)
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: max(geo.size.height - 20, 0))
                    .padding(.horizontal, Theme.pageMargin)
                    .padding(.vertical, 10)
                }
            }
        }
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
        .task(id: agentId) {
            loading = true
            models = []
            let token = UUID()   // 代际标记：防止旧查询结果覆盖已切换的新页面
            queryToken = token
            engine.modelBreakdown(agentId: agentId) { rows in
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
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous).fill(Theme.cardFill))
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
                            .help(m.modelId)   // 长模型名截断时可看全名
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
                .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: Theme.chipFill)
                .onTapGesture {
                    controller.route = .sessions(agentId, m.modelId)
                }
                .accessibilityAddTraits(.isButton)
                .accessibilityLabel(m.modelId)
            }
        }
    }

    /// 性能与健康监控卡片（v1.7.6）
    private func performanceCard(_ s: AgentSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("性能与健康")
                    .font(Theme.bodyFont(10, weight: .semibold))
                    .foregroundColor(Theme.onDarkFaint)
                Spacer()
                if s.isHung {
                    Text("疑似卡死")
                        .font(Theme.bodyFont(9, weight: .bold))
                        .foregroundColor(Theme.dangerRed)
                } else if s.processRunning {
                    Text("运行正常")
                        .font(Theme.bodyFont(9, weight: .medium))
                        .foregroundColor(Theme.statusWorking)
                }
            }

            HStack(spacing: 0) {
                overviewCell("CPU", s.cpuPercent > 0 ? String(format: "%.1f%%", s.cpuPercent) : "0%", cost: nil)
                Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
                overviewCell("内存 (RSS)", s.memoryText, cost: nil)
                Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
                overviewCell("PID", s.pid.map { "\($0)" } ?? "—", cost: nil)
            }
            .padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous).fill(Theme.cardFill))

            // 实时流水抽屉入口
            Button {
                controller.openLiveStream(agentId: agentId)
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 10))
                        .foregroundColor(Theme.statusWorking)
                    Text("查看实时事件与输出流水")
                        .font(Theme.bodyFont(11, weight: .medium))
                        .foregroundColor(Theme.onDark)
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.system(size: 8, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.chipFill))
            }
            .buttonStyle(.plain)
            .help("展开该智能体的实时工具调用与输出时序抽屉")
        }
    }

    /// 无 token 数据的 agent：显示主卡瘦身撤下的基础信息
    private var basicInfoCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let s = snapshot {
                infoRow("状态", s.level.label)
                infoRow("活动", s.lastActivityText)
                if s.activeSessions > 0 { infoRow("会话", "\(s.activeSessions) 个活跃") }
                if s.cpuPercent > 0.1 { infoRow("CPU", String(format: "%.1f%%", s.cpuPercent)) }
                if s.memoryBytes > 0 { infoRow("内存", s.memoryText) }
                if let pid = s.pid { infoRow("PID", "\(pid)") }
                infoRow("Token", "暂无本地 token 数据")
            }
        }
        .padding(.horizontal, 10)   // 与模型行内边距对齐（之前 12 造成文字基线差 2pt）
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous).fill(Theme.cardFill))
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

    /// 静态化：行 body 每次重算不再新建 DateFormatter（昂贵对象）
    fileprivate static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "MM-dd HH:mm"
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(title: modelId,
                         subtitle: "\(sessions.count) 个会话",
                         onBack: { controller.route = .agentDetail(agentId) },
                         controller: controller)

            DarkDivider()

            // 列表态用 ScrollView+LazyVStack（懒加载）；loading/空态直接铺满剩余空间并居中，
            // 避免窗口固定高度下大片空白玻璃
            if loading {
                CenteredSpinner()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if sessions.isEmpty {
                Text("该模型暂无会话记录")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.onDarkFaint)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                // 注意：GeometryReader 必须在 ScrollView 外层——放内层会令 ScrollView
                // 内容尺寸 = 视口，长内容不可滚动（documentH=视口）
                GeometryReader { geo in
                    ScrollView(.vertical, showsIndicators: true) {
                        // LazyVStack：会话可能成百上千条，懒加载避免一次性构建全部行。
                        // minHeight = 视口 - padding(10×2)：短内容不产生多余滚动，
                        // 行间留白而不是整片底部玻璃；会话多时内容超过视口，正常滚动
                        LazyVStack(spacing: 4) {
                            ForEach(sessions) { s in
                                sessionRow(s)
                            }
                        }
                        .frame(maxWidth: .infinity, minHeight: max(geo.size.height - 20, 0))
                        .padding(.horizontal, Theme.pageMargin)
                        .padding(.vertical, 10)
                    }
                }
            }
        }
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
        .task(id: "\(agentId)/\(modelId)") {
            loading = true
            sessions = []
            let token = UUID()   // 代际标记：防止旧查询结果覆盖已切换的新页面
            queryToken = token
            engine.sessions(agentId: agentId, modelId: modelId) { rows in
                guard queryToken == token else { return }
                sessions = rows
                loading = false
            }
        }
    }

    private func sessionRow(_ s: SessionUsage) -> some View {
        SessionRowView(session: s)
    }
}

// MARK: - 会话行（hover 态在行内自持，避免整列表重绘）
private struct SessionRowView: View {
    let session: SessionUsage

    var body: some View {
        let timeText: String = {
            guard let t = session.lastTime else { return "—" }
            return SessionListView.timeFormatter.string(from: t)
        }()
        let detail = "\(session.messages) 条 · \(TokenUsage.compact(session.tokens)) tok"
            + (TokenUsage.cost(session.cost).isEmpty ? "" : " · \(TokenUsage.cost(session.cost))")
        let hasDir = session.directory != nil
        return HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(timeText)
                    .font(Theme.monoFont(10, weight: .semibold))
                    .foregroundColor(hasDir ? Theme.onDark : Theme.onDark.opacity(0.55))
                    .lineLimit(1)
                    .help(session.directory ?? session.sessionId)
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
        .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: Theme.chipFill, hoverEnabled: hasDir)
        .onTapGesture {
            if let dir = session.directory {
                NSWorkspace.shared.open(URL(fileURLWithPath: dir))
            }
        }
        // a11y：有目录的行是按钮（打开目录）；无目录行隐藏交互语义
        .accessibilityAddTraits(hasDir ? .isButton : [])
        .accessibilityLabel(hasDir ? "打开 \(session.directory ?? "")" : "会话，无目录")
        .opacity(hasDir ? 1.0 : 0.75)
    }
}
