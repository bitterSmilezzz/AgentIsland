import AgentIslandCore
import SwiftUI
import AppKit
import UserNotifications

// MARK: - 共享应用上下文（引擎单例，App 与 AppDelegate 共用同一实例）

@MainActor
final class AppContext {
    static let shared = AppContext()
    let engine: ActivityEngine
    /// 已安装缓存（组合根创建；刷新节律由引擎驱动，设置页只读 + 打开时触发重扫）
    let installedApps = InstalledAppsCache()
    private var _controller: IslandPanelController?

    /// 控制器延迟创建：首次访问才实例化，全局唯一
    var controller: IslandPanelController {
        if let c = _controller { return c }
        let c = IslandPanelController(engine: engine)
        _controller = c
        return c
    }

    /// 本地 Webhook / 事件接收服务 (v0.0.77)
    lazy var localEventServer: LocalEventServer = LocalEventServer(engine: engine)

    /// 启停集读取与自愈：纯核 resolvedEnabled 计算（空数组主动全关照常生效；存量升级自动补全新增内置项）
    private static func enabledOrDefault(registry: [AgentProfile]) -> Set<String> {
        EnabledAgentStore.resolvedEnabled(registry: registry)
    }

    private init() {
        // 配置：Core 唯一读取路径（缺项回落默认 + 归一化启动自愈脏值）
        let config = EngineConfig.load(from: .standard)
        let registry = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs(),
                                                  installedBundles: installedApps.installedBundleIDs())
        let enabled = Self.enabledOrDefault(registry: registry)
        engine = ActivityEngine(
            profiles: registry.filter { enabled.contains($0.id) },
            config: config,
            installedApps: installedApps,
            enabledIDs: enabled
        )
        // 安装缓存首刷由引擎 init 自排（唯一刷新驱动）；首刷完成回调里引擎自行重放
        // 启用集恢复自动发现监控——组合根不再编排「刷新+打标+二次 setEnabled」舞步
    }
}

// MARK: - AgentIsland 入口
// 菜单栏常驻 App（LSUIElement）：MenuBarExtra + 设置窗口 + 灵动岛面板

@main
struct AgentIslandApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    init() {
        // 无头验证模式：--probe 打印状态表后退出
        if CommandLine.arguments.contains("--probe") {
            exit(Probe.run())
        }
        // 无头自检模式：进程内断言
        if CommandLine.arguments.contains("--selftest") {
            exit(Selftest.run())
        }
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarPopoverView(controller: AppContext.shared.controller, engine: AppContext.shared.engine)
                .onOpenURL { url in
                    URLSchemeRouter.handle(url: url, controller: AppContext.shared.controller, engine: AppContext.shared.engine)
                }
        } label: {
            MenuBarIconView(engine: AppContext.shared.engine)
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsView(engine: AppContext.shared.engine,
                         controller: AppContext.shared.controller,
                         installedApps: AppContext.shared.installedApps)
        }
    }
}

// MARK: - 菜单栏 Popover 内容视图（Compact Island Popover）

struct MenuBarPopoverView: View {
    @ObservedObject var controller: IslandPanelController
    @ObservedObject var engine: ActivityEngine
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // 顶部状态条
            headerBar

            DarkDivider()

            // 活跃 Agent 微缩列表
            agentQuickSection

            // Token 概览
            if !engine.grandTotal.isEmpty {
                DarkDivider()
                tokenMiniSummary
            }

            DarkDivider()

            // 底部操作栏
            actionBar
        }
        .padding(14)
        .frame(width: 300)
        .background(
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(colorScheme == .light ? .regularMaterial : .ultraThinMaterial)
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(
                        colorScheme == .light
                            ? LinearGradient(
                                colors: [Color.white.opacity(0.96), Color.white.opacity(0.92)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                            : LinearGradient(
                                colors: [Color(hex: 0x141416).opacity(0.95), Color(hex: 0x0e0e10).opacity(0.92)],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                    )
            }
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(
                    colorScheme == .light ? Ramp.slate200 : Color.white.opacity(0.12),
                    lineWidth: 0.75
                )
        )
        .preferredColorScheme(controller.appearanceMode.colorScheme)
        .onAppear {
            // popover 打开时按需单次刷新 token（R34/F6）：popover 不参与「呈现活跃」
            // 生命周期（docked 常态轮询已暂停），不刷新会显示冻结值、首次打开甚至空白
            engine.refreshTokenUsageOnce()
        }
    }

    // MARK: 顶部状态条
    private var headerBar: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(statusColor)
                .frame(width: 8, height: 8)
                .overlay(
                    Circle()
                        .stroke(statusColor.opacity(0.4), lineWidth: engine.anyWorking ? 3 : 0)
                        .scaleEffect(engine.anyWorking ? 1.4 : 1.0)
                )

            Text(statusTitle)
                .font(Theme.bodyFont(13, weight: .semibold))
                .foregroundColor(Theme.ink)

            Spacer()

            Text("\(engine.visibleSnapshots.count) 在线")
                    .help("当前进程仍在的智能体数；离线项不显示")
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(Theme.chipFill))
        }
    }

    private var statusTitle: String {
        let waiting = engine.visibleSnapshots.filter { $0.level == .attention }
        if !waiting.isEmpty {
            return "\(waiting.count) 个 Agent 等待确认"
        } else if engine.anyWorking {
            let count = engine.workingAgents().count
            return "\(count) 个 Agent 工作中"
        } else if engine.visibleSnapshots.contains(where: { $0.level == .completed }) {
            return "任务已完成"
        } else if engine.visibleSnapshots.isEmpty {
            return "暂无活跃 Agent"
        } else {
            return "全部待机中"
        }
    }

    private var statusColor: Color {
        engine.visibleSnapshots.contains { $0.level == .attention }
            ? Theme.warningOrange
            : (engine.anyWorking ? Theme.statusWorking : (engine.visibleSnapshots.isEmpty ? Theme.statusOffline : Theme.statusIdle))
    }

    // MARK: 活跃 Agent 概览列表
    private var agentQuickSection: some View {
        VStack(spacing: 4) {
            let urgent = engine.visibleSnapshots.filter { $0.level == .attention }
            let working = engine.workingAgents()
            let displayList = !urgent.isEmpty ? urgent : (working.isEmpty ? Array(engine.visibleSnapshots.prefix(3)) : working)
            if displayList.isEmpty {
                HStack {
                    Spacer()
                    Text("无运行中的 Agent")
                        .font(Theme.bodyFont(11))
                        .foregroundColor(Theme.inkMuted48)
                        .padding(.vertical, 8)
                    Spacer()
                }
            } else {
                ForEach(displayList) { s in
                    HStack(spacing: 8) {
                        Image(systemName: s.profile.icon)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundColor(Theme.ink)
                            .frame(width: 22, height: 22)
                            .background(Circle().fill(Theme.tile1))

                        Text(s.profile.name)
                            .font(Theme.bodyFont(12, weight: .medium))
                            .foregroundColor(Theme.ink)
                            .readableSingleLine(
                                fullText: s.profile.name,
                                minWidth: 72,
                                priority: 2
                            )

                        Spacer()

                        if let usage = s.tokenUsage, usage.tokens24h > 0 {
                            Text(TokenUsage.compact(usage.tokens24h))
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.inkMuted48)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(Theme.chipFill))
                        }

                        Text(s.level.label)
                            .font(Theme.bodyFont(10, weight: .semibold))
                            .foregroundColor(levelForegroundColor(s.level))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(
                                Capsule()
                                    .fill(levelBackgroundColor(s.level))
                                    .overlay(Capsule().strokeBorder(levelBorderColor(s.level), lineWidth: 0.5))
                            )
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 4)
                    .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: .clear)
                    .onTapGesture {
                        controller.route = .agentDetail(s.profile.id)
                        if controller.displayState == .docked {
                            controller.toggle()
                        }
                    }
                    // a11y：与主卡 Agent 行同口径（R27）
                    .accessibilityAddTraits(.isButton)
                    .accessibilityLabel("\(s.profile.name)，\(s.level.label)，点按查看详情")
                    .accessibilityAction {
                        controller.route = .agentDetail(s.profile.id)
                        if controller.displayState == .docked {
                            controller.toggle()
                        }
                    }
                }
            }
        }
    }

    // MARK: Token 概览
    private var tokenMiniSummary: some View {
        Button {
            controller.route = .tokenAnalytics
            if controller.displayState == .docked { controller.toggle() }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "chart.bar.fill")
                    .font(.system(size: 9))
                    .foregroundColor(Theme.inkMuted48)
                Text("24h \(TokenUsage.compact(engine.grandTotal.tokens24h))")
                    .font(Theme.monoFont(10, weight: .medium))
                    .foregroundColor(Theme.inkMuted80)
                if !TokenUsage.cost(engine.grandTotal.cost24h).isEmpty {
                    Text(TokenUsage.cost(engine.grandTotal.cost24h))
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.inkMuted48)
                }
                Spacer()
                Text("累计 \(TokenUsage.compact(engine.grandTotal.tokensTotal))")
                    .font(Theme.monoFont(10))
                    .foregroundColor(Theme.inkMuted48)
                Image(systemName: "chevron.right")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundColor(Theme.inkMuted48)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 4)
        .help("打开 Token 时间分析")
        .accessibilityLabel("打开 Token 时间分析")
    }

    // MARK: 底部操作栏
    private var actionBar: some View {
        HStack(spacing: 8) {
            // 展开/收起灵动岛（文案按实体：岛可贴顶，不叫侧边栏）
            Button {
                controller.toggle()
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "sidebar.right")
                        .font(.system(size: 11))
                    Text(controller.displayState == .expanded ? "收起灵动岛" : "展开灵动岛")
                        .font(Theme.bodyFont(11, weight: .medium))
                }
                .foregroundColor(Theme.ink)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .frame(maxWidth: .infinity)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(Theme.chipFill)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                        )
                )
            }
            .buttonStyle(.plain)

            // 外观主题切换
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
                    .font(.system(size: 11))
                    .foregroundColor(Theme.inkMuted80)
                    .padding(6)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Theme.chipFill)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                            )
                    )
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .help("外观主题：\(controller.appearanceMode.label)")
            .accessibilityLabel("外观主题，当前 \(controller.appearanceMode.label)")

            // 通知模式切换
            Menu {
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
            } label: {
                Image(systemName: controller.notificationPolicy.icon)
                    .font(.system(size: 11))
                    .foregroundColor(controller.notificationPolicy == .focus ? Theme.focusBlue : Theme.inkMuted80)
                    .padding(6)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Theme.chipFill)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                            )
                    )
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .help("通知策略：\(controller.notificationPolicy.label)（\(controller.notificationPolicy.detailDescription)）")
            .accessibilityLabel("通知策略，当前 \(controller.notificationPolicy.label)")

            // 设置
            Button {
                SettingsOpener.open()
            } label: {
                Image(systemName: "gearshape")
                    .font(.system(size: 11))
                    .foregroundColor(Theme.inkMuted80)
                    .padding(6)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Theme.chipFill)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                            )
                    )
            }
            .buttonStyle(.plain)
            .help("设置…")
            .accessibilityLabel("打开偏好设置")

            // 退出
            Button {
                NSApp.terminate(nil)
            } label: {
                Image(systemName: "power")
                    .font(.system(size: 11))
                    .foregroundColor(Theme.dangerRed)
                    .padding(6)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Theme.chipFill)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                            )
                    )
            }
            .buttonStyle(.plain)
            .help("退出 AgentIsland")
            .accessibilityLabel("退出 AgentIsland")
        }
    }

    private func levelForegroundColor(_ level: ActivityLevel) -> Color {
        colorScheme == .light ? level.lightText : level.color
    }

    private func levelBackgroundColor(_ level: ActivityLevel) -> Color {
        colorScheme == .light ? level.lightFill : level.color.opacity(0.14)
    }

    private func levelBorderColor(_ level: ActivityLevel) -> Color {
        colorScheme == .light ? level.lightBorder : Color.clear
    }
}

// MARK: - 菜单栏图标（Q10：working 状态色 + 圆点角标）

struct MenuBarIconView: View {
    @ObservedObject var engine: ActivityEngine
    @AppStorage(SettingKey.menuBarBadgeMode) private var badgeMode: String = MenuBarBadgeMode.iconOnly.rawValue

    private var needsAttention: Bool {
        engine.visibleSnapshots.contains { $0.level == .attention }
    }

    private var workingCount: Int {
        engine.visibleSnapshots.filter { $0.level == .working }.count
    }

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: needsAttention ? "bell.badge.fill" : (engine.anyWorking ? "dot.radiowaves.left.and.right" : "sparkles"))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(needsAttention ? Theme.warningOrange : (engine.anyWorking ? Theme.statusWorking : Theme.inkMuted48))
                // H2：状态切换淡入过渡（macOS 13 无 symbolEffect，用内容过渡替代）
                .contentTransition(.opacity)
                .animation(.easeInOut(duration: 0.25), value: engine.anyWorking)
                .overlay(alignment: .topTrailing) {
                    if engine.anyWorking || needsAttention {
                        // H1：角标用 alignment+padding 完全收进图标内圈（不用 offset，避免越出被裁）
                        // 随图标同节奏淡入淡出
                        Circle()
                            .fill(needsAttention ? Theme.warningOrange : Theme.statusWorking)
                            .frame(width: 4, height: 4)
                            .padding(1)
                            .transition(.opacity)
                    }
                }

            if badgeMode == MenuBarBadgeMode.activeCount.rawValue && (engine.anyWorking || needsAttention) {
                Text(needsAttention ? "!" : "\(workingCount)")
                    .font(.system(size: 9.5, weight: .bold, design: .monospaced))
                    .foregroundColor(needsAttention ? Theme.warningOrange : Theme.statusWorking)
            } else if badgeMode == MenuBarBadgeMode.tokenUsage.rawValue && engine.grandTotal.tokens24h > 0 {
                Text(TokenUsage.compact(engine.grandTotal.tokens24h))
                    .font(.system(size: 9, weight: .semibold, design: .monospaced))
                    .foregroundColor(Theme.inkMuted80)
            }
        }
        // VoiceOver：菜单栏图标是应用的第一入口，纯图标无文案，需显式播报当前状态
        .accessibilityLabel(needsAttention ? "AgentIsland：有智能体等待你确认" : (engine.anyWorking ? "AgentIsland：有智能体正在工作" : "AgentIsland：全部空闲"))
        .accessibilityHint("打开监控面板")
    }
}

// MARK: - AppDelegate（创建灵动岛面板）

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory) // 无 Dock 图标
        UNUserNotificationCenter.current().delegate = self
        CompletionNotification.requestAuthorization()

        let context = AppContext.shared
        let controller = context.controller // 触发延迟创建
        if CommandLine.arguments.contains("--expanded") {
            controller.expand(graceDuration: 30.0)
        }
        context.engine.start()
        context.localEventServer.start()
        controller.show()
    }

    func applicationWillTerminate(_ notification: Notification) {
        AppContext.shared.localEventServer.stop()
        AppContext.shared.engine.stop()
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            URLSchemeRouter.handle(url: url, controller: AppContext.shared.controller, engine: AppContext.shared.engine)
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        guard let agentId = AgentNotificationRoute.agentId(from: response.notification.request.content.userInfo) else {
            completionHandler()
            return
        }
        Task { @MainActor in
            defer { completionHandler() }
            let context = AppContext.shared
            // 用户点击或划掉/点掉通知：同步消除灵动岛上对应的事件横幅
            if context.engine.latestEvent?.agentId == agentId {
                context.engine.clearLatestEvent()
            }

            // 仅在默认点击通知时激活目标窗口；若用户是关闭/划掉通知则不抢焦点
            guard response.actionIdentifier == UNNotificationDefaultActionIdentifier else {
                return
            }

            let snapshot = context.engine.snapshots.first { $0.id == agentId }
            let profile = snapshot?.profile ?? context.engine.allProfiles.first { $0.id == agentId }
            if AppActivator.activate(pid: snapshot?.pid, bundleIDs: profile?.bundleIDs ?? []) {
                return
            }

            // CLI 经 ssh/tmux 启动或目标刚退出时可能没有可激活宿主；至少展开对应详情，
            // 明确告诉用户是哪一个 Agent 在等待，避免点击通知后毫无反馈。
            context.controller.route = .agentDetail(agentId)
            context.controller.expand(graceDuration: 30)
            context.controller.show()
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list])
    }
}

// MARK: - 打开设置窗口的唯一出口

/// 齿轮按钮与 `agentisland://settings` 深链共用这一处。
/// 两处各写一遍 `sendAction(Selector(("showSettingsWindow:")))` 的话，
/// 系统改名（macOS 13/14 在 `showPreferencesWindow:` 与 `showSettingsWindow:` 之间
/// 来回过）时就只有一处会坏，症状还是「点了没反应」这种最难报的问题
enum SettingsOpener {
    /// `tab` 来自 `agentisland://settings?tab=remote`；nil 表示只开窗不动当前页
    @MainActor
    static func open(tab: String? = nil) {
        SettingsSelection.shared.select(raw: tab)
        NSApp.activate(ignoringOtherApps: true)
        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
    }
}

