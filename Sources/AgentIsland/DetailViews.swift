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
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 8) {
            Button(action: {
                withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) {
                    onBack()
                }
            }) {
                Image(systemName: "chevron.left")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundColor(colorScheme == .light ? Color(hex: 0x334155) : Theme.onDark.opacity(0.9))
                    .frame(width: 24, height: 24)
                    .background(
                        Circle()
                            .fill(colorScheme == .light ? (backHovered ? Color.white : Color(hex: 0xf1f5f9)) : (backHovered ? Theme.hoverFill : Theme.chipFill))
                            .overlay(
                                Circle()
                                    .strokeBorder(colorScheme == .light ? (backHovered ? Color(hex: 0xcbd5e1) : Color(hex: 0xe2e8f0)) : Color.clear, lineWidth: 0.5)
                            )
                            .shadow(color: Color.black.opacity(colorScheme == .light ? (backHovered ? 0.06 : 0.02) : 0), radius: 1, y: 0.5)
                    )
                    .contentShape(Circle())
                    .scaleEffect(backHovered ? 1.08 : 1.0)
                    .animation(.spring(response: 0.22, dampingFraction: 0.75), value: backHovered)
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
                    .readableSingleLine(fullText: title, minWidth: 96, priority: 2)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                        .readableSingleLine(fullText: subtitle, minWidth: 96, priority: 1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
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
    @Environment(\.colorScheme) private var colorScheme

    private var snapshot: AgentSnapshot? {
        engine.snapshots.first { $0.id == agentId }
    }
    /// 详情也可由 Token 时间页进入。内嵌工具没有独立实时快照时，仍要保留其
    /// 可读用量，而非把它误判成一个失效的 Agent。
    private var profile: AgentProfile? {
        snapshot?.profile
            ?? engine.allProfiles.first { $0.id == agentId }
            ?? AgentRegistry.profile(id: agentId)
    }
    private var usage: TokenUsage? {
        snapshot?.tokenUsage ?? engine.tokenUsage(for: agentId)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(title: profile?.name ?? agentId,
                         subtitle: usage.map {
                             "24h \(TokenUsage.compact($0.tokens24h)) · 累计 \(TokenUsage.compact($0.tokensTotal))"
                         },
                         onBack: { controller.closeAgentDetail() },
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
                                if snapshot == nil, usage == nil {
                                    // agent 被禁用/移除后仍停留在详情页时，此前内容区一片空白
                                    VStack(spacing: 6) {
                                        Image(systemName: "questionmark.circle")
                                            .font(.system(size: 20))
                                            .foregroundColor(Theme.onDarkFaint)
                                        Text("该智能体已不在监控列表")
                                            .font(Theme.bodyFont(11))
                                            .foregroundColor(Theme.onDarkFaint)
                                        Button("返回") { controller.closeAgentDetail() }
                                            .buttonStyle(.plain)
                                            .font(Theme.bodyFont(11, weight: .semibold))
                                            .foregroundColor(Theme.actionBlue)
                                    }
                                    .frame(maxWidth: .infinity)
                                } else if let usage, !usage.isEmpty {
                                    VStack(alignment: .leading, spacing: 10) {
                                        overviewCard(usage)
                                        workEfficiencyCard
                                        if let s = snapshot, s.processRunning {
                                            if !s.backgroundTasks.isEmpty || !s.subagents.isEmpty {
                                                activeTasksAndSubagentsCard(s)
                                            }
                                            if let tb = s.tokenBreakdown {
                                                tokenBreakdownCard(tb)
                                            }
                                            workspaceCard(s)
                                            performanceCard(s)
                                        }
                                        if !models.isEmpty {
                                            modelList
                                        } else if usage.tokensTotal > 0 {
                                            // dim/opencode 以外的数据源（claude/codex 等）无按模型
                                            // 拆分数据——此前无声缺席，用户分不清「无数据」与「不支持」
                                            Text("该数据源暂不支持按模型拆分")
                                                .font(Theme.bodyFont(10))
                                                .foregroundColor(Theme.onDarkFaint)
                                                .frame(maxWidth: .infinity, alignment: .leading)
                                        }
                                        if snapshot == nil {
                                            Text("用量来自本地会话记录；该工具当前未作为独立运行项显示。")
                                                .font(Theme.bodyFont(10))
                                                .foregroundColor(Theme.onDarkFaint)
                                                .frame(maxWidth: .infinity, alignment: .leading)
                                        }
                                    }
                                } else {
                                    VStack(alignment: .leading, spacing: 10) {
                                        basicInfoCard
                                        workEfficiencyCard
                                        if let s = snapshot, s.processRunning {
                                            if !s.backgroundTasks.isEmpty || !s.subagents.isEmpty {
                                                activeTasksAndSubagentsCard(s)
                                            }
                                            if let tb = s.tokenBreakdown {
                                                tokenBreakdownCard(tb)
                                            }
                                            workspaceCard(s)
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

    private var estimatedCostTotal: Double? {
        guard !models.isEmpty else { return nil }
        let sum = models.reduce(0.0) { acc, m in
            let res = TokenCostEstimator.resolveCost(actual: m.cost, modelId: m.modelId, tokens: m.tokens)
            return acc + res.cost
        }
        return sum > 0 ? sum : nil
    }

    private func overviewCard(_ u: TokenUsage) -> some View {
        let cost24 = TokenUsage.cost(u.cost24h)
        let costTot = TokenUsage.cost(u.costTotal)
        let estTotal = (!costTot.isEmpty) ? nil : estimatedCostTotal
        let displayCostTotal = !costTot.isEmpty ? costTot : (estTotal != nil ? TokenCostEstimator.formatEstimate(estTotal!) : nil)

        return HStack(spacing: 0) {
            overviewCell("24h", TokenUsage.compact(u.tokens24h),
                         cost: cost24.isEmpty ? nil : cost24)
            Rectangle()
                .fill(
                    LinearGradient(
                        colors: [
                            Theme.onDark.opacity(0.03),
                            Theme.onDark.opacity(0.18),
                            Theme.onDark.opacity(0.03)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
                .frame(width: 1, height: 30)
            overviewCell("累计", TokenUsage.compact(u.tokensTotal),
                         cost: displayCostTotal)
        }
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .obsidianCardStyle()
    }

    private func overviewCell(_ label: String, _ value: String, cost: String?, valueColor: Color? = nil) -> some View {
        VStack(spacing: 3) {
            Text(value)
                .font(Theme.monoDigitFont(16, weight: .bold))
                .foregroundColor(valueColor ?? Theme.onDark)
            HStack(spacing: 4) {
                Text(label)
                    .font(Theme.bodyFont(9.5, weight: .medium))
                    .foregroundColor(Theme.onDarkMuted)
                if let cost {
                    Text(cost)
                        .font(Theme.monoDigitFont(9.5, weight: .semibold))
                        .foregroundColor(Theme.sydedockAmber)
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder
    private var workEfficiencyCard: some View {
        let stats = engine.durationTracker.stats(for: agentId)
        if stats.taskCount > 0 {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 5) {
                    Image(systemName: "timer")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(Theme.sydedockCyan)
                    Text("任务耗时与效率（24h）")
                        .font(Theme.bodyFont(10.5, weight: .bold))
                        .foregroundColor(Theme.onDark)
                    Spacer()
                    Text("共 \(stats.taskCount) 次完成")
                        .font(Theme.monoDigitFont(9.5))
                        .foregroundColor(Theme.onDarkMuted)
                }
                HStack(spacing: 0) {
                    overviewCell("工作总耗时", stats.formattedTotalTime, cost: nil, valueColor: Theme.sydedockCyan)
                    Rectangle()
                        .fill(Theme.hairline.opacity(0.3))
                        .frame(width: 1, height: 26)
                    overviewCell("平均用时", stats.formattedAverageDuration, cost: nil)
                    Rectangle()
                        .fill(Theme.hairline.opacity(0.3))
                        .frame(width: 1, height: 26)
                    overviewCell("单次最长", AgentTaskEvent.durationText(stats.maxDuration), cost: nil)
                }
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity)
                .subtleCardStyle()
            }
        }
    }

    private var modelList: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("按模型")
                .font(Theme.bodyFont(10.5, weight: .bold))
                .foregroundColor(Theme.onDark)
            if models.count > 1 {
                ModelDonutChartView(models: models)
            }
            ForEach(models) { m in
                HStack(spacing: 8) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 4.5, style: .continuous)
                            .fill(Theme.sydedockCyan.opacity(0.14))
                        Image(systemName: "cube.transparent")
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundColor(Theme.sydedockCyan)
                    }
                    .frame(width: 18, height: 18)

                    VStack(alignment: .leading, spacing: 1.5) {
                        Text(m.modelId)
                            .font(Theme.monoFont(10.5, weight: .semibold))
                            .foregroundColor(Theme.onDark)
                            .readableSingleLine(fullText: m.modelId, minWidth: 96, priority: 2)
                        HStack(spacing: 5) {
                            Text("\(TokenUsage.compact(m.tokens)) tok")
                                .font(Theme.monoDigitFont(9.5, weight: .medium))
                                .foregroundColor(Theme.onDarkMuted)
                            let resolved = TokenCostEstimator.resolveCost(actual: m.cost, modelId: m.modelId, tokens: m.tokens)
                            if !resolved.text.isEmpty {
                                HStack(spacing: 2) {
                                    Text(resolved.text)
                                        .font(Theme.monoDigitFont(9.5, weight: .semibold))
                                        .foregroundColor(Theme.sydedockAmber)
                                    if resolved.isEstimated {
                                        Text("估算")
                                            .font(.system(size: 7.5, weight: .medium))
                                            .foregroundColor(Theme.sydedockAmber.opacity(0.75))
                                            .padding(.horizontal, 3)
                                            .padding(.vertical, 0.5)
                                            .background(
                                                RoundedRectangle(cornerRadius: 3)
                                                    .fill(Theme.sydedockAmber.opacity(0.12))
                                            )
                                    }
                                }
                            }
                            Text("\(m.messages) 次")
                                .font(Theme.monoDigitFont(9))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(Theme.badgeFont(.semibold))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: Theme.obsidianCardFill)
                .onTapGesture {
                    controller.route = .sessions(agentId, m.modelId)
                }
                .accessibilityAddTraits(.isButton)
                .accessibilityLabel(m.modelId)
            }
        }
    }

    /// 工作区与直达终端卡片
    @ViewBuilder
    private func workspaceCard(_ s: AgentSnapshot) -> some View {
        let cwd: String? = {
            if let pid = s.pid, let dir = ProcessInspector.currentWorkingDirectory(of: pid) {
                return dir
            }
            return nil
        }()

        if let dir = cwd {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Image(systemName: "folder.fill")
                        .font(.system(size: 9.5))
                        .foregroundColor(Theme.sydedockCyan)
                    Text("当前工作区")
                        .font(Theme.bodyFont(10, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                    Spacer()
                }

                Text(dir)
                    .font(Theme.monoFont(9.5))
                    .foregroundColor(Theme.onDark)
                    .lineLimit(2)
                    .truncationMode(.middle)

                HStack(spacing: 8) {
                    Button {
                        let script = "open -a Terminal \"\(dir)\""
                        var err: NSDictionary?
                        NSAppleScript(source: "do shell script \"\(script.replacingOccurrences(of: "\"", with: "\\\""))\"")?.executeAndReturnError(&err)
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "terminal.fill")
                                .font(.system(size: 8.5))
                            Text("打开终端")
                                .font(Theme.bodyFont(9.5, weight: .medium))
                        }
                        .foregroundColor(Theme.sydedockCyan)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3.5)
                        .background(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .fill(Theme.sydedockCyan.opacity(0.12))
                        )
                    }
                    .buttonStyle(.plain)

                    Button {
                        NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: dir)
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "macwindow")
                                .font(.system(size: 8.5))
                            Text("在访达中显示")
                                .font(Theme.bodyFont(9.5, weight: .medium))
                        }
                        .foregroundColor(Theme.onDarkMuted)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3.5)
                        .background(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .fill(Theme.chipFill)
                        )
                    }
                    .buttonStyle(.plain)

                    // 常用已安装 IDE 一键呼出
                    ForEach(Self.knownEditors.filter { NSWorkspace.shared.urlForApplication(withBundleIdentifier: $0.bundleID) != nil }, id: \.bundleID) { editor in
                        Button {
                            if let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: editor.bundleID) {
                                NSWorkspace.shared.open([URL(fileURLWithPath: dir)], withApplicationAt: appURL, configuration: NSWorkspace.OpenConfiguration(), completionHandler: nil)
                            }
                        } label: {
                            HStack(spacing: 3) {
                                Image(systemName: editor.icon)
                                    .font(.system(size: 8))
                                Text(editor.name)
                                    .font(Theme.bodyFont(9.5, weight: .medium))
                            }
                            .foregroundColor(Theme.sydedockCyan)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3.5)
                            .background(
                                RoundedRectangle(cornerRadius: 4, style: .continuous)
                                    .fill(Theme.sydedockCyan.opacity(0.12))
                            )
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous)
                    .fill(Theme.obsidianCardFill)
            )
        }
    }

    private struct KnownEditor {
        let name: String
        let bundleID: String
        let icon: String
    }

    private static let knownEditors: [KnownEditor] = [
        KnownEditor(name: "VS Code", bundleID: "com.microsoft.VSCode", icon: "chevron.left.forwardslash.chevron.right"),
        KnownEditor(name: "Cursor", bundleID: "com.todesktop.230313mzl4w4u92", icon: "cursorarrow.rays"),
        KnownEditor(name: "Windsurf", bundleID: "com.exafunction.windsurf", icon: "wind"),
        KnownEditor(name: "Xcode", bundleID: "com.apple.dt.Xcode", icon: "hammer.fill")
    ]

    /// 智能体健康诊断卡片 (v0.0.73)
    private func healthDiagnosticCard(_ s: AgentSnapshot) -> some View {
        let report = AgentHealthEvaluator.evaluate(snapshot: s)
        let gradeColor: Color = {
            switch report.grade {
            case .healthy: return Theme.sydedockEmerald
            case .attention: return Theme.sydedockCyan
            case .warning: return Theme.warningOrange
            case .critical: return Theme.dangerRed
            }
        }()

        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                HStack(spacing: 4) {
                    Image(systemName: report.grade.icon)
                        .font(.system(size: 9.5, weight: .semibold))
                        .foregroundColor(gradeColor)
                    Text("稳定性诊断")
                        .font(Theme.bodyFont(9.5, weight: .semibold))
                        .foregroundColor(Theme.onDarkFaint)
                }
                Spacer()
                Text("\(report.score)分 · \(report.grade.rawValue)")
                    .font(Theme.monoDigitFont(9.5, weight: .bold))
                    .foregroundColor(gradeColor)
            }

            Text(report.suggestion)
                .font(Theme.bodyFont(9))
                .foregroundColor(Theme.onDarkMuted)
                .lineLimit(2)
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(gradeColor.opacity(0.08))
                .overlay(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .strokeBorder(gradeColor.opacity(0.20), lineWidth: 0.5)
                )
        )
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
                } else if s.processRunning, let health = s.sessionProbeHealth {
                    // 复用同一处状态位，不新增行：进程在跑却读不到会话源时，卡上的
                    // 「待机」是不可信的——把这条证据显式化（悬停给原因与路径）
                    Text("会话源不可读")
                        .font(Theme.bodyFont(9, weight: .medium))
                        .foregroundColor(Theme.warningOrange)
                        .help(health.diagnosticText)
                        .accessibilityLabel(health.diagnosticText)
                } else if s.processRunning {
                    Text("运行正常")
                        .font(Theme.bodyFont(9, weight: .medium))
                        .foregroundColor(Theme.statusWorking)
                }
            }

            let cpuColor: Color = {
                if s.cpuPercent >= 80 { return Theme.dangerRed }
                if s.cpuPercent >= 30 { return Theme.sydedockAmber }
                return Theme.onDark
            }()

            HStack(spacing: 0) {
                overviewCell("CPU", s.cpuPercent > 0 ? String(format: "%.1f%%", s.cpuPercent) : "0%", cost: nil, valueColor: cpuColor)
                Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
                overviewCell("内存 (RSS)", s.memoryText, cost: nil)
                    .help(s.memoryBytes > 0 ? "\(s.memoryBytes.formatted()) 字节" : "暂无内存数据")
                Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
                overviewCell("PID", s.pid.map { "\($0)" } ?? "—", cost: nil)
                    .help(s.pid.map { "进程 PID: \($0)" } ?? "未运行")
            }
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous)
                    .fill(Theme.cardFill)
                    .overlay(
                        RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous)
                            .strokeBorder(
                                colorScheme == .light ? Color(hex: 0xe2e8f0) : Theme.obsidianHairline,
                                lineWidth: 0.75
                            )
                    )
                    .shadow(color: Color.black.opacity(colorScheme == .light ? 0.025 : 0), radius: 1.5, y: 1)
            )

            // 稳定性诊断微卡片
            healthDiagnosticCard(s)

            // 派生工具与子进程全景树 (v0.0.74)
            if let report = engine.inspectProcessTree(agentId: agentId) {
                ProcessTreeView(report: report)
            }

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
                        .font(Theme.badgeFont(.semibold))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(
                    RoundedRectangle(cornerRadius: Theme.radiusSm)
                        .fill(Theme.chipFill)
                        .overlay(
                            RoundedRectangle(cornerRadius: Theme.radiusSm)
                                .strokeBorder(colorScheme == .light ? Color(hex: 0xe2e8f0) : Color.clear, lineWidth: 0.5)
                        )
                )
            }
            .buttonStyle(.plain)
            .help("展开该智能体的实时工具调用与输出时序抽屉")
        }
    }

    /// 在途后台任务与活跃子智能体卡片
    private func activeTasksAndSubagentsCard(_ s: AgentSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                Image(systemName: "bolt.badge.clock.fill")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(Theme.sydedockAmber)
                Text("在途后台任务与子智能体")
                    .font(Theme.bodyFont(10.5, weight: .bold))
                    .foregroundColor(Theme.onDark)
                Spacer()
                Text("\(s.backgroundTasks.count + s.subagents.count) 个并行")
                    .font(Theme.monoDigitFont(9.5))
                    .foregroundColor(Theme.onDarkMuted)
            }

            if !s.backgroundTasks.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(s.backgroundTasks, id: \.id) { task in
                        HStack(spacing: 6) {
                            Image(systemName: "terminal.fill")
                                .font(.system(size: 9))
                                .foregroundColor(Theme.sydedockAmber)
                            Text(task.action)
                                .font(Theme.monoFont(9.5))
                                .foregroundColor(Theme.onDark)
                                .lineLimit(1)
                            Spacer(minLength: 4)
                            Text("后台运行")
                                .font(Theme.badgeFont())
                                .foregroundColor(Theme.sydedockAmber)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(Theme.sydedockAmber.opacity(0.12)))
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: 5).fill(Theme.obsidianCardFill))
                    }
                }
            }

            if !s.subagents.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(s.subagents, id: \.conversationId) { sub in
                        HStack(spacing: 6) {
                            Image(systemName: "person.2.fill")
                                .font(.system(size: 9))
                                .foregroundColor(Theme.sydedockCyan)
                            Text(sub.role)
                                .font(Theme.bodyFont(9.5, weight: .semibold))
                                .foregroundColor(Theme.onDark)
                            if let model = sub.model {
                                Text(model)
                                    .font(Theme.badgeFont())
                                    .foregroundColor(Theme.onDarkFaint)
                                    .padding(.horizontal, 3)
                                    .padding(.vertical, 0.5)
                                    .background(RoundedRectangle(cornerRadius: 3).fill(Theme.chipFill))
                            }
                            Spacer(minLength: 4)
                            Text(sub.state ?? "active")
                                .font(Theme.badgeFont())
                                .foregroundColor(Theme.sydedockCyan)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(Theme.sydedockCyan.opacity(0.12)))
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(RoundedRectangle(cornerRadius: 5).fill(Theme.obsidianCardFill))
                    }
                }
            }
        }
        .padding(10)
        .subtleCardStyle()
    }

    /// Token 深度构成分析卡片
    private func tokenBreakdownCard(_ tb: AgentTokenBreakdown) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                Image(systemName: "chart.bar.xaxis")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(Theme.sydedockCyan)
                Text("Token 深度指标拆解")
                    .font(Theme.bodyFont(10.5, weight: .bold))
                    .foregroundColor(Theme.onDark)
                Spacer()
                Text("共 \(TokenUsage.compact(tb.totalTokens)) tok")
                    .font(Theme.monoDigitFont(9.5, weight: .bold))
                    .foregroundColor(Theme.sydedockCyan)
            }

            HStack(spacing: 4) {
                tokenMetricCell("输入/提示", tb.promptTokens, color: Theme.onDark)
                tokenMetricCell("输出/生成", tb.completionTokens, color: Theme.sydedockCyan)
                tokenMetricCell("缓存读取", tb.cacheReadTokens, color: Theme.sydedockEmerald)
                if tb.reasoningTokens > 0 {
                    tokenMetricCell("深度思考", tb.reasoningTokens, color: Theme.sydedockAmber)
                }
            }
        }
        .padding(10)
        .subtleCardStyle()
    }

    private func tokenMetricCell(_ label: String, _ value: Int, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(TokenUsage.compact(value))
                .font(Theme.monoDigitFont(11, weight: .bold))
                .foregroundColor(color)
            Text(label)
                .font(Theme.bodyFont(8.5))
                .foregroundColor(Theme.onDarkFaint)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(6)
        .background(RoundedRectangle(cornerRadius: 4).fill(Theme.obsidianCardFill))
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
        .subtleCardStyle()
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
        f.locale = Locale(identifier: "en_US_POSIX")   // 固定数字口径，防 12 小时制 locale 改写
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
                    ScrollView(.vertical, showsIndicators: false) {
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
    /// 打开目录失败反馈：目录失效（删除/改名/卸载）时点击静默无效此前无任何提示
    @State private var openFailureFeedback = false

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
                    .readableSingleLine(
                        fullText: session.directory ?? session.sessionId,
                        minWidth: 72,
                        priority: 2
                    )
                Text(detail)
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkFaint)
                    .readableSingleLine(fullText: detail, minWidth: 72, priority: 1)
            }
            Spacer()
            if hasDir {
                Button {
                    RecentSessionNavigator.resumeInTerminal(directory: session.directory)
                } label: {
                    Image(systemName: "terminal")
                        .font(.system(size: 9.5))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .buttonStyle(.plain)
                .help("在终端中打开该会话工作区")

                Image(systemName: "folder")
                    .font(.system(size: 10))
                    .foregroundColor(Theme.onDarkFaint)
            } else {
                // L7：无目录的会话行视觉降级（不可点）
                Text("无目录")
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.onDarkFaint.opacity(0.6))
            }

            Button {
                RecentSessionNavigator.copySessionId(session.sessionId)
            } label: {
                Image(systemName: "doc.on.doc")
                    .font(.system(size: 9.5))
                    .foregroundColor(Theme.onDarkFaint)
            }
            .buttonStyle(.plain)
            .help("拷贝会话 ID: \(session.sessionId)")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: Theme.chipFill, hoverEnabled: hasDir)
        .onTapGesture {
            guard let dir = session.directory, hasDir else { return }
            if !NSWorkspace.shared.open(URL(fileURLWithPath: dir)) {
                openFailureFeedback = true
                Task { @MainActor in
                    try? await Task.sleep(nanoseconds: 1_500_000_000)
                    openFailureFeedback = false
                }
            }
        }
        // a11y：有目录的行是按钮（打开目录）；无目录行隐藏交互语义
        .accessibilityAddTraits(hasDir ? .isButton : [])
        .overlay(alignment: .trailing) {
            if openFailureFeedback {
                Text("目录未找到")
                    .font(Theme.bodyFont(9, weight: .medium))
                    .foregroundColor(Theme.warningOrange)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Theme.warningOrange.opacity(0.14)))
                    .padding(.trailing, Theme.pageMargin)
            }
        }
        .accessibilityLabel(hasDir ? "打开 \(session.directory ?? "")" : "会话，无目录")
        .opacity(hasDir ? 1.0 : 0.75)
    }
}
