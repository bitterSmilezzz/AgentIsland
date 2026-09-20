import AgentIslandCore
import SwiftUI
import AppKit
import ServiceManagement

// MARK: - 设置分类 Tab

enum SettingsTab: String, CaseIterable, Identifiable {
    case general
    case agents
    case remote
    case engine
    case about

    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: return "通用与外观"
        case .agents: return "Agent 监控"
        case .remote: return "远程通知"
        case .engine: return "引擎与性能"
        case .about: return "关于"
        }
    }

    var icon: String {
        switch self {
        case .general: return "paintpalette"
        case .agents: return "person.2.badge.gearshape"
        case .remote: return "paperplane"
        case .engine: return "gauge.with.dots.needle.bottom.50percent"
        case .about: return "info.circle"
        }
    }
}

// MARK: - 设置卡片容器（Apple Inset-Grouped Style）

struct SettingsCard<Content: View>: View {
    let title: String?
    let content: Content
    @Environment(\.colorScheme) private var colorScheme

    init(title: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let title {
                Text(title)
                    .font(Theme.bodyFont(11, weight: .semibold))
                    .foregroundColor(Theme.inkMuted80)
                    .textCase(.uppercase)
                    .padding(.leading, 4)
            }
            VStack(alignment: .leading, spacing: 10) {
                content
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Theme.parchment)
                    .overlay(
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .stroke(
                                colorScheme == .light ? Ramp.slate200 : Theme.hairline.opacity(0.5),
                                lineWidth: 0.5
                            )
                    )
                    .shadow(color: Color.black.opacity(colorScheme == .light ? 0.03 : 0), radius: 2, y: 1)
            )
        }
    }
}

// MARK: - 设置窗口

struct SettingsView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    /// 已安装缓存（组合根注入；与引擎共用同一实例，本页只读 + 打开时触发重扫）
    let installedApps: InstalledAppsCache

    private static let defaultConfig = EngineConfig()

    /// 版本号从 bundle 读取（与 Info.plist 同源）。
    /// 此前硬编码 "v1.7.9"，发版后忘记同步就会显示错误版本。
    static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? AppVersion.string
    }

    @AppStorage(SettingKey.workingWindow) private var workingWindow: Double = SettingsView.defaultConfig.workingWindow
    @AppStorage(SettingKey.sampleInterval) private var sampleInterval: Double = SettingsView.defaultConfig.sampleInterval
    @AppStorage(SettingKey.idleSampleInterval) private var idleSampleInterval: Double = SettingsView.defaultConfig.idleSampleInterval
    @AppStorage(SettingKey.cpuThreshold) private var cpuThreshold: Double = SettingsView.defaultConfig.cpuThreshold
    @AppStorage(SettingKey.activeSessionWindow) private var activeSessionWindow: Double = SettingsView.defaultConfig.activeSessionWindow
    @AppStorage(SettingKey.collapseDelay) private var collapseDelay: Double = 0.5   // 面板行为参数（非引擎采样配置）
    @AppStorage(SettingKey.launchAtLogin) private var launchAtLogin = false
    @AppStorage(SettingKey.playCompletionSound) private var playCompletionSound = true
    @AppStorage(SettingKey.tokenAlertEnabled) private var tokenAlertEnabled = true
    @AppStorage(SettingKey.tokenAlertThreshold) private var tokenAlertThreshold = 200_000
    @AppStorage(SettingKey.runawayCpuAlert) private var runawayCpuAlert = true
    @AppStorage(SettingKey.dailyTokenBudget) private var dailyTokenBudget = 0
    @AppStorage(SettingKey.budgetAlertEnabled) private var budgetAlertEnabled = true
    @AppStorage(SettingKey.compactView) private var compactView = false
    @AppStorage(SettingKey.globalHotKeyEnabled) private var globalHotKeyEnabled = true
    @AppStorage(SettingKey.menuBarBadgeMode) private var menuBarBadgeMode = MenuBarBadgeMode.iconOnly.rawValue
    @AppStorage(SettingKey.screenFollowMode) private var screenFollowMode = ScreenFollowMode.followMouse.rawValue
    @AppStorage(SettingKey.completionSoundOption) private var completionSoundOption = CompletionSoundOption.glass.rawValue
    @AppStorage(SettingKey.alertSoundOption) private var alertSoundOption = AlertSoundOption.sosumi.rawValue
    @AppStorage(SettingKey.hapticFeedbackEnabled) private var hapticFeedbackEnabled = true
    @AppStorage(SettingKey.batterySaverEnabled) private var batterySaverEnabled = true
    @AppStorage(SettingKey.autoAnomaliesAlertEnabled) private var autoAnomaliesAlertEnabled = true

    /// 内置 Agent 列表缓存盒（引用语义）：设置页开着时引擎每拍发布使 body 重算，
    /// fullRegistry 每次都要 loadCustomProfiles（UserDefaults 读 + JSONDecoder 解码）
    /// 再过滤——按扫描版本缓存，失效信号与自动发现列表一致（installedScanVersion，
    /// 安装扫描完成时自增；installedCLIs()/installedBundleIDs() 本身是带锁内存读）
    private final class ProfilesCache {
        var version = -1
        var builtin: [AgentProfile] = []
        var discovered: [AgentProfile]?
    }
    @State private var profilesCache = ProfilesCache()

    /// 内置 Agent 列表（与 fullRegistry 同口径：排除宿主内嵌且未独立安装的组件）
    private var builtinProfiles: [AgentProfile] {
        if profilesCache.version != installedScanVersion {
            profilesCache.builtin = AgentRegistry.fullRegistry(
                installedCLIs: installedApps.installedCLIs(),
                installedBundles: installedApps.installedBundleIDs())
                .filter { !$0.isCustom && !$0.id.hasPrefix("cli-") }
            profilesCache.discovered = nil
            profilesCache.version = installedScanVersion
        }
        return profilesCache.builtin
    }

    /// 自动发现列表（同盒缓存；PATH / Applications 扫描较重，按扫描版本失效）。
    /// 显式调用方：agentsDetailView；调用前必须先求值 builtinProfiles（或本函数
    /// 自身对齐 version），两处共用同一失效判定
    private func discoveredProfiles() -> [AgentProfile] {
        if installedScanVersion == profilesCache.version, let cached = profilesCache.discovered {
            return cached
        }
        let found = AgentRegistry.discoverCLIProfiles(
            installedCLIs: installedApps.installedCLIs(),
            installedBundles: installedApps.installedBundleIDs())
        profilesCache.discovered = found
        profilesCache.version = installedScanVersion
        return found
    }

    @State private var selectedTab: SettingsTab = .general
    @State private var enabledAgents: Set<String> = []
    /// 自启动设置失败提示（SMAppService 未签名/非 /Applications 时 register 抛错）
    @State private var launchError: String?
    @State private var customProfiles: [AgentProfile] = []
    @State private var showAddCustom = false
    /// 待删除的自定义 Agent（用于二次确认弹窗）
    @State private var pendingRemove: AgentProfile?
    /// 重置位置确认
    @State private var confirmingResetPosition = false
    /// 安装缓存扫描完成版本号：触发 body 重算刷新自动发现列表
    @State private var installedScanVersion = 0

    var body: some View {
        _ = installedScanVersion
        return NavigationSplitView {
            sidebarView
        } detail: {
            detailView
        }
        .frame(width: 620, height: 480)
        .preferredColorScheme(controller.appearanceMode.colorScheme)
        .onAppear {
            installedApps.refreshIfNeeded(maxAge: 0) {
                installedScanVersion += 1
            }
            loadState()
        }
        .sheet(isPresented: $showAddCustom) {
            AddCustomAgentSheet(
                existingIDs: Set(customProfiles.map(\.id)),
                knownProcessNames: AgentRegistry.conflictingProcessNames(
                    enabledIDs: Set(engine.allProfiles.map(\.id)),
                    installedApps: installedApps)) { profile in
                addCustom(profile)
            }
        }
    }

    // MARK: 左侧导航栏

    private var sidebarView: some View {
        List(SettingsTab.allCases, selection: $selectedTab) { tab in
            Label(tab.title, systemImage: tab.icon)
                .font(Theme.bodyFont(13, weight: .medium))
                .tag(tab)
        }
        .listStyle(.sidebar)
        .navigationSplitViewColumnWidth(min: 140, ideal: 160, max: 180)
    }

    // MARK: 右侧详情路由

    private var detailView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                switch selectedTab {
                case .general:
                    generalDetailView
                case .agents:
                    agentsDetailView
                case .remote:
                    RemoteNotifySettingsView(notifier: controller.remoteNotifier)
                case .engine:
                    engineDetailView
                case .about:
                    aboutDetailView
                }
            }
            .padding(18)
        }
        .background(Theme.canvas)
    }

    // MARK: - Tab 1: 通用与外观

    private var generalDetailView: some View {
        VStack(alignment: .leading, spacing: 14) {
            SettingsCard(title: "外观主题") {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 12) {
                        ForEach(IslandAppearance.allCases) { mode in
                            let isSelected = controller.appearanceMode == mode
                            Button {
                                controller.applyAppearance(mode)
                            } label: {
                                VStack(spacing: 8) {
                                    Image(systemName: mode.icon)
                                        .font(.system(size: 20, weight: .medium))
                                        .foregroundColor(isSelected ? Theme.focusBlue : Theme.ink)
                                        .frame(height: 24)

                                    Text(mode.label)
                                        .font(Theme.bodyFont(12, weight: isSelected ? .semibold : .regular))
                                        .foregroundColor(isSelected ? Theme.focusBlue : Theme.ink)
                                }
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 12)
                                .background(
                                    RoundedRectangle(cornerRadius: 8)
                                        .fill(isSelected ? Theme.focusBlue.opacity(0.12) : Theme.tile1)
                                )
                                .overlay(
                                    RoundedRectangle(cornerRadius: 8)
                                        .stroke(isSelected ? Theme.focusBlue : Theme.hairline, lineWidth: isSelected ? 1.5 : 1)
                                )
                            }
                            .buttonStyle(.plain)
                            .accessibilityValue(isSelected ? "已选择" : "未选择")
                        }
                    }

                    Text("控制全局界面与悬浮灵动岛面板的显示风格，支持浅色模式、深色模式与跟随系统。")
                        .font(Theme.bodyFont(11))
                        .foregroundColor(Theme.inkMuted48)
                }
            }

            SettingsCard(title: "通知与免打扰模式") {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 12) {
                        ForEach(NotificationPolicy.allCases) { policy in
                            let isSelected = controller.notificationPolicy == policy
                            Button {
                                controller.applyNotificationPolicy(policy)
                            } label: {
                                VStack(spacing: 8) {
                                    Image(systemName: policy.icon)
                                        .font(.system(size: 20, weight: .medium))
                                        .foregroundColor(isSelected ? Theme.focusBlue : Theme.ink)
                                        .frame(height: 24)

                                    Text(policy.label)
                                        .font(Theme.bodyFont(12, weight: isSelected ? .semibold : .regular))
                                        .foregroundColor(isSelected ? Theme.focusBlue : Theme.ink)
                                }
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 12)
                                .background(
                                    RoundedRectangle(cornerRadius: 8)
                                        .fill(isSelected ? Theme.focusBlue.opacity(0.12) : Theme.tile1)
                                )
                                .overlay(
                                    RoundedRectangle(cornerRadius: 8)
                                        .stroke(isSelected ? Theme.focusBlue : Theme.hairline, lineWidth: isSelected ? 1.5 : 1)
                                )
                            }
                            .buttonStyle(.plain)
                            .accessibilityValue(isSelected ? "已选择" : "未选择")
                        }
                    }

                    Text(controller.notificationPolicy.detailDescription)
                        .font(Theme.bodyFont(11))
                        .foregroundColor(Theme.inkMuted48)
                }
            }

            SettingsCard(title: "启动与交互") {
                VStack(alignment: .leading, spacing: 12) {
                    Toggle(isOn: $launchAtLogin) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("登录时自动启动")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("系统启动后在后台自动运行并常驻菜单栏")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                    .onChange(of: launchAtLogin) { newValue in
                        applyLaunchAtLogin(newValue)
                    }

                    if let launchError {
                        Text(launchError)
                            .font(Theme.bodyFont(10))
                            .foregroundColor(Theme.dangerRed)
                    }

                    Divider()

                    Toggle(isOn: $playCompletionSound) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("任务完成提示音")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("智能体执行完毕从工作切入空闲时，播放轻微提示音（完全静默模式下不发声；专注免打扰模式下仅告警发声）")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)

                    if playCompletionSound {
                        HStack(spacing: 8) {
                            Text("完成音效")
                                .font(Theme.bodyFont(11, weight: .medium))
                                .foregroundColor(Theme.ink)
                            Picker("", selection: $completionSoundOption) {
                                ForEach(CompletionSoundOption.allCases) { opt in
                                    Text(opt.label).tag(opt.rawValue)
                                }
                            }
                            .pickerStyle(.menu)

                            Button {
                                SoundEffectsManager.previewSound(named: completionSoundOption)
                            } label: {
                                Image(systemName: "speaker.wave.2.fill")
                                    .font(.system(size: 11))
                                    .foregroundColor(Theme.actionBlue)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("试听任务完成音效")
                            .help("试听任务完成音效")
                        }
                        .padding(.leading, 12)

                        HStack(spacing: 8) {
                            Text("熔断告警音")
                                .font(Theme.bodyFont(11, weight: .medium))
                                .foregroundColor(Theme.ink)
                            Picker("", selection: $alertSoundOption) {
                                ForEach(AlertSoundOption.allCases) { opt in
                                    Text(opt.label).tag(opt.rawValue)
                                }
                            }
                            .pickerStyle(.menu)

                            Button {
                                SoundEffectsManager.previewSound(named: alertSoundOption)
                            } label: {
                                Image(systemName: "speaker.wave.3.fill")
                                    .font(.system(size: 11))
                                    .foregroundColor(Theme.dangerRed)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("试听熔断告警音效")
                            .help("试听熔断告警音效")
                        }
                        .padding(.leading, 12)
                    }

                    Toggle(isOn: $hapticFeedbackEnabled) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("触控板微触觉反馈 (Haptics)")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("任务完成、严重告警或面板边缘停靠时，提供细腻的触控板震动感知")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)

                    Divider()

                    sliderRow(
                        title: "自动收起延迟",
                        value: $collapseDelay,
                        range: 0.2...5,
                        step: 0.1,
                        unit: "秒",
                        onRelease: { controller.applyCollapseDelay(collapseDelay) }
                    )
                    Text("鼠标移出卡片后，延迟多长时间平滑收回为屏幕边缘的微细条。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)

                    Divider()

                    Toggle(isOn: $globalHotKeyEnabled) {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text("全局快捷键呼出 / 收起")
                                    .font(Theme.bodyFont(13))
                                    .foregroundColor(Theme.ink)
                                Text("⌥ A")
                                    .font(Theme.monoDigitFont(11, weight: .bold))
                                    .foregroundColor(Theme.actionBlue)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 1.5)
                                    .background(
                                        RoundedRectangle(cornerRadius: 4)
                                            .fill(Theme.tile1)
                                    )
                            }
                            Text("系统级 Carbon 全局热键（免辅助功能权限），在任何全屏应用下秒级呼出或收起灵动岛")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                    .onChange(of: globalHotKeyEnabled) { enabled in
                        GlobalHotKeyManager.shared.setEnabled(enabled)
                    }

                    Divider()

                    Toggle(isOn: $compactView) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("紧凑排版密度 (Compact View)")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("高密度条目与微型指示环，针对 13 寸 MacBook 或多 Agent 并发场景大幅减少滚动")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)

                    Divider()

                    VStack(alignment: .leading, spacing: 6) {
                        Text("系统状态栏图标附加动态徽标")
                            .font(Theme.bodyFont(13))
                            .foregroundColor(Theme.ink)
                        Picker("", selection: $menuBarBadgeMode) {
                            ForEach(MenuBarBadgeMode.allCases) { mode in
                                Text(mode.label).tag(mode.rawValue)
                            }
                        }
                        .pickerStyle(.segmented)
                        Text("在 macOS 顶栏图标旁动态显示活跃任务数或今日 Token 消耗，无需展开即可一瞥全局。")
                            .font(Theme.bodyFont(10))
                            .foregroundColor(Theme.inkMuted48)
                    }
                }
            }

            SettingsCard(title: "停靠贴边与吸附") {
                VStack(alignment: .leading, spacing: 10) {
                    Picker("停靠位置", selection: Binding(
                        get: { controller.dockEdge },
                        set: { controller.setDockEdge($0) }
                    )) {
                        ForEach(DockEdge.allCases) { edge in
                            Text(edge.label).tag(edge)
                        }
                    }
                    .pickerStyle(.segmented)

                    HStack {
                        Text("长按展开卡片的顶栏可自由拖动，松手自动智能吸附贴边；收起时在屏幕边缘保留 6pt 晶莹微细条。")
                            .font(Theme.bodyFont(10))
                            .foregroundColor(Theme.inkMuted48)
                        Spacer()
                        Button("重置位置") {
                            // 会抹掉用户拖好的吸附锚点且不可恢复，故二次确认
                            confirmingResetPosition = true
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        .confirmationDialog("重置灵动岛位置?",
                                            isPresented: $confirmingResetPosition,
                                            titleVisibility: .visible) {
                            Button("重置", role: .destructive) { controller.resetPosition() }
                            Button("取消", role: .cancel) {}
                        } message: {
                            Text("将清除已记住的贴边锚点，灵动岛回到默认停靠位置。")
                        }
                    }

                    Divider()

                    Toggle(isOn: Binding(
                        get: { controller.hideDockedSliver },
                        set: { controller.hideDockedSliver = $0 }
                    )) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("极简纯净模式（隐藏边缘微细条）")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("收起时完全隐去屏幕边缘的 6pt 微细条，追求极致清爽；展开或有任务提醒时正常显现")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)

                    Divider()

                    VStack(alignment: .leading, spacing: 6) {
                        Text("多显示器停靠策略 (Multi-Monitor)")
                            .font(Theme.bodyFont(13))
                            .foregroundColor(Theme.ink)
                        Picker("", selection: $screenFollowMode) {
                            ForEach(ScreenFollowMode.allCases) { mode in
                                Text(mode.label).tag(mode.rawValue)
                            }
                        }
                        .pickerStyle(.menu)
                        Text("外接显示器时，选择灵动岛是自动跟随当前活跃鼠标所在的屏幕，还是固定在指定屏幕")
                            .font(Theme.bodyFont(10))
                            .foregroundColor(Theme.inkMuted48)
                    }
                    .onChange(of: screenFollowMode) { _ in
                        controller.placeWindow(animated: true)
                    }
                }
            }
        }
    }

    // MARK: - Tab 2: Agent 监控

    private var agentsDetailView: some View {
        VStack(alignment: .leading, spacing: 14) {
            SettingsCard(title: "内置 Agent") {
                VStack(spacing: 8) {
                    // 用 fullRegistry 而非 builtin：与主列表/引擎同口径。
                    // 否则宿主内嵌且未独立安装的组件（如 ChatGPT 里的 Codex）已被主列表
                    // 隐藏，设置页却仍列出，用户会看到「幽灵条目」。
                    ForEach(builtinProfiles) { profile in
                        Toggle(isOn: binding(for: profile)) {
                            HStack(spacing: 8) {
                                Image(systemName: profile.icon)
                                    .foregroundColor(Theme.actionBlue)
                                    .frame(width: 18)
                                Text(profile.name)
                                    .font(Theme.bodyFont(13))
                                    .foregroundColor(Theme.ink)
                                Spacer()
                                if let snapshot = engine.snapshots.first(where: { $0.id == profile.id }) {
                                    Text(snapshot.level.label)
                                        .font(Theme.bodyFont(10, weight: .semibold))
                                        .foregroundColor(snapshot.level.color)
                                        .padding(.horizontal, 6)
                                        .padding(.vertical, 2)
                                        .background(Capsule().fill(snapshot.level.color.opacity(0.12)))
                                }
                            }
                        }
                        .toggleStyle(.switch)
                        .tint(Theme.actionBlue)
                    }
                }
            }

            let discovered = discoveredProfiles()
            if !discovered.isEmpty {
                SettingsCard(title: "自动发现 (PATH / Applications)") {
                    VStack(spacing: 8) {
                        ForEach(discovered) { profile in
                            Toggle(isOn: binding(for: profile)) {
                                HStack(spacing: 8) {
                                    Image(systemName: "terminal")
                                        .foregroundColor(Theme.inkMuted48)
                                        .frame(width: 18)
                                    Text(profile.name)
                                        .font(Theme.bodyFont(13))
                                        .foregroundColor(Theme.ink)
                                    Spacer()
                                    if let snapshot = engine.snapshots.first(where: { $0.id == profile.id }) {
                                        Text(snapshot.level.label)
                                            .font(Theme.bodyFont(10, weight: .semibold))
                                            .foregroundColor(snapshot.level.color)
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(Capsule().fill(snapshot.level.color.opacity(0.12)))
                                    }
                                }
                            }
                            .toggleStyle(.switch)
                            .tint(Theme.actionBlue)
                        }
                    }
                }
            }

            SettingsCard(title: "自定义 Agent") {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("自定义列表")
                            .font(Theme.bodyFont(12))
                            .foregroundColor(Theme.inkMuted80)
                        Spacer()
                        Button {
                            showAddCustom = true
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "plus.circle.fill")
                                Text("添加")
                            }
                            .font(Theme.bodyFont(11, weight: .medium))
                            .foregroundColor(Theme.actionBlue)
                        }
                        .buttonStyle(.plain)
                    }

                    if customProfiles.isEmpty {
                        Text("没有自定义条目。可添加内部脚本或自研 agent。")
                            .font(Theme.bodyFont(11))
                            .foregroundColor(Theme.inkMuted48)
                            .padding(.vertical, 4)
                    } else {
                        ForEach(Array(customProfiles), id: \.id) { profile in
                            CustomAgentRowView(
                                profile: profile,
                                isEnabled: binding(for: profile),
                                onRemove: { pendingRemove = profile }
                            )
                        }
                    }
                }
            }
        }
        // 删除是不可逆操作：与工作台清理保持同样的二次确认标准
        .confirmationDialog(
            "删除自定义 Agent「\(pendingRemove?.name ?? "")」?",
            isPresented: Binding(get: { pendingRemove != nil },
                                 set: { if !$0 { pendingRemove = nil } }),
            titleVisibility: .visible
        ) {
            Button("删除", role: .destructive) {
                if let p = pendingRemove { removeCustom(p) }
                pendingRemove = nil
            }
            Button("取消", role: .cancel) { pendingRemove = nil }
        } message: {
            Text("该条目的监控档案将被移除，且无法撤销。")
        }
    }

    // MARK: - Tab 3: 引擎与性能

    private var engineDetailView: some View {
        VStack(alignment: .leading, spacing: 14) {
            SettingsCard(title: "活动判定规则") {
                VStack(spacing: 12) {
                    sliderRow(title: "「工作中」写入窗口", value: $workingWindow, range: 10...300, unit: "秒")
                    sliderRow(title: "CPU 判定阈值", value: $cpuThreshold, range: EngineConfig.cpuThresholdRange, unit: "%")
                    sliderRow(title: "活跃会话窗口", value: $activeSessionWindow, range: 60...3600, unit: "秒")
                }
            }

            if engine.isLowPowerModeActive {
                HStack(spacing: 8) {
                    Image(systemName: "leaf.circle.fill")
                        .font(.system(size: 14))
                        .foregroundColor(Theme.statusWorking)
                    Text("系统「低电量模式」已开启：引擎已自动适配低能耗节律")
                        .font(Theme.bodyFont(11, weight: .medium))
                        .foregroundColor(Theme.statusWorking)
                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Theme.statusWorking.opacity(0.12))
                        .overlay(
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .stroke(Theme.statusWorking.opacity(0.35), lineWidth: 0.5)
                        )
                )
            }

            SettingsCard(title: "性能与能耗预设") {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("一键档位")
                            .font(Theme.bodyFont(12, weight: .semibold))
                            .foregroundColor(Theme.ink)
                        Spacer()
                        Button("恢复默认") {
                            resetEngineToDefaults()
                        }
                        .buttonStyle(.plain)
                        .font(Theme.bodyFont(11, weight: .medium))
                        .foregroundColor(Theme.actionBlue)
                    }

                    HStack(spacing: 8) {
                        presetButton(.fast, title: "⚡️ 极速灵敏")
                        presetButton(.balanced, title: "⚖️ 平衡标准")
                        presetButton(.eco, title: "🍃 极致省电")
                    }

                    Text("• 极速灵敏：采样 1s / 闲置 3s / 窗口 30s（高频并发编码推荐）\n• 平衡标准：采样 2s / 闲置 5s / 窗口 60s（默认平衡推荐）\n• 极致省电：采样 3s / 闲置 12s / 窗口 90s（电池供电出行推荐）")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)
                        .fixedSize(horizontal: false, vertical: true)

                    Divider()

                    Toggle(isOn: $batterySaverEnabled) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Mac 电池供电节能自适应")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("使用 MacBook 电池供电时自动适度平滑降频，插上电源即刻满血运行")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                }
            }

            SettingsCard(title: "采样节律精细微调") {
                VStack(alignment: .leading, spacing: 12) {
                    sliderRow(title: "活动采样间隔", value: $sampleInterval, range: 1...10, unit: "秒")
                    sliderRow(title: "闲置降频间隔", value: $idleSampleInterval, range: 5...60, unit: "秒")

                    Text("当有 Agent 处于活跃工作中时，引擎以活动采样间隔高频探测；当全部待机或离线时，自动降频至闲置间隔以节省 CPU 与电量。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)
                }
            }

            SettingsCard(title: "成本与异常熔断保护") {
                VStack(alignment: .leading, spacing: 12) {
                    Toggle(isOn: $tokenAlertEnabled) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Token 暴涨告警与熔断")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("短时间内 Token 消耗异常暴增时，灵动岛自动弹出警报并提供一键熔断")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                    .onChange(of: tokenAlertEnabled) { _ in applyConfig() }

                    if tokenAlertEnabled {
                        Picker("激增报警阈值", selection: $tokenAlertThreshold) {
                            Text("30k tokens / 分钟").tag(30_000)
                            Text("50k tokens / 分钟").tag(50_000)
                            Text("100k tokens / 分钟").tag(100_000)
                            Text("200k tokens / 分钟").tag(200_000)
                            Text("500k tokens / 分钟").tag(500_000)
                            Text("1.0M tokens / 分钟").tag(1_000_000)
                            Text("2.0M tokens / 分钟").tag(2_000_000)
                            Text("5.0M tokens / 分钟").tag(5_000_000)
                        }
                        .font(Theme.bodyFont(12))
                        .onChange(of: tokenAlertThreshold) { _ in applyConfig() }
                        .onAppear {
                            // 非档位持久值（外部 defaults write / 旧版遗留）会让 Picker
                            // selection 空白而引擎仍按该未知阈值告警——吸附到最近档位
                            let tiers = [30_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000, 5_000_000]
                            if !tiers.contains(tokenAlertThreshold) {
                                tokenAlertThreshold = tiers.min(by: {
                                    abs($0 - tokenAlertThreshold) < abs($1 - tokenAlertThreshold)
                                }) ?? 200_000
                            }
                        }

                        Text("注：WorkBuddy 等多专家团 Agent 自动应用 100万/分钟 专属保护下限，避免常规专家协同误报。")
                            .font(Theme.bodyFont(10))
                            .foregroundColor(Theme.inkMuted48)
                    }

                    Divider()

                    Toggle(isOn: $runawayCpuAlert) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("异常长耗时死循环告警")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("智能体持续高负荷运行（CPU > 70% 且超 5 分钟）时自动预警，防止死循环无限消耗")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                    .onChange(of: runawayCpuAlert) { _ in applyConfig() }

                    Divider()

                    Toggle(isOn: $budgetAlertEnabled) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("每日 Token 消费预算预警与封顶")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("设定每日消耗预算上限，用量达 80% 触发黄色预警，达 100% 触发红色超额告警")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)

                    if budgetAlertEnabled {
                        Picker("每日消费预算上限", selection: $dailyTokenBudget) {
                            Text("不设限额").tag(0)
                            Text("100k tokens / 天").tag(100_000)
                            Text("200k tokens / 天").tag(200_000)
                            Text("500k tokens / 天").tag(500_000)
                            Text("1.0M tokens / 天").tag(1_000_000)
                            Text("2.0M tokens / 天").tag(2_000_000)
                            Text("5.0M tokens / 天").tag(5_000_000)
                            Text("10.0M tokens / 天").tag(10_000_000)
                        }
                        .font(Theme.bodyFont(12))
                    }

                    Divider()

                    Toggle(isOn: $autoAnomaliesAlertEnabled) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("智能体长时间死锁与内存泄露守护")
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            Text("持续追踪进程长时间无响应（≥3分钟）或内存严重泄露（≥5分钟），主动发出自愈告警")
                                .font(Theme.bodyFont(10))
                                .foregroundColor(Theme.inkMuted48)
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                }
            }
        }
    }

    // MARK: - Tab 4: 关于

    private var aboutDetailView: some View {
        VStack(alignment: .leading, spacing: 14) {
            SettingsCard {
                HStack(spacing: 12) {
                    Image(systemName: "sparkles")
                        .font(.system(size: 28, weight: .medium))
                        .foregroundColor(Theme.actionBlue)
                        .frame(width: 48, height: 48)
                        .background(Circle().fill(Theme.actionBlue.opacity(0.12)))

                    VStack(alignment: .leading, spacing: 3) {
                        Text("AgentIsland")
                            .font(Theme.displayFont(16, weight: .bold))
                            .foregroundColor(Theme.ink)
                        Text("v\(Self.appVersion) · macOS 灵动岛 Agent 会话监控器")
                            .font(Theme.bodyFont(11))
                            .foregroundColor(Theme.inkMuted80)
                    }
                }
                .padding(.vertical, 4)

                Divider()

                VStack(alignment: .leading, spacing: 6) {
                    Label("只读监控：绝不读取会话内容、私钥凭据与隐私数据", systemImage: "lock.shield")
                    Label("高性能低能耗：空闲自动降频，工作态 CPU 开销约 1%", systemImage: "bolt.badge.clock")
                    Label("自由拖拽与智能贴边：上、右、下、左四边自动吸附，6pt 微细条触碰自动弹出", systemImage: "arrow.up.and.down.and.arrow.left.and.right")
                }
                .font(Theme.bodyFont(11))
                .foregroundColor(Theme.inkMuted80)
            }

            SettingsCard(title: "开源与仓库") {
                HStack {
                    Image(systemName: "link")
                        .foregroundColor(Theme.actionBlue)
                    Link("github.com/bitterSmilezzz/AgentIsland",
                         destination: URL(string: "https://github.com/bitterSmilezzz/AgentIsland")!)
                        .font(Theme.bodyFont(12))
                        .foregroundColor(Theme.actionBlue)
                    Spacer()
                }
            }
        }
    }

    // MARK: - 滑块行构建

    private static func formatSliderValue(_ v: Double, unit: String) -> String {
        let s = String(format: "%.1f", v)
        let trimmed = s.hasSuffix(".0") ? String(s.dropLast(2)) : s
        return "\(trimmed)\(unit)"
    }

    // MARK: - 性能预设档位与辅助

    private enum PerformancePreset: String, CaseIterable, Identifiable {
        case fast = "极速"
        case balanced = "平衡"
        case eco = "省电"

        var id: String { rawValue }
        var description: String {
            switch self {
            case .fast: return "活动 1.0s / 闲置 3.0s / 窗口 30s"
            case .balanced: return "活动 2.0s / 闲置 5.0s / 窗口 60s"
            case .eco: return "活动 3.0s / 闲置 12.0s / 窗口 90s"
            }
        }
    }

    private var currentPreset: PerformancePreset? {
        if abs(sampleInterval - 1.0) < 0.1 && abs(idleSampleInterval - 3.0) < 0.1 && abs(workingWindow - 30.0) < 0.1 {
            return .fast
        }
        if abs(sampleInterval - 2.0) < 0.1 && abs(idleSampleInterval - 5.0) < 0.1 && abs(workingWindow - 60.0) < 0.1 {
            return .balanced
        }
        if abs(sampleInterval - 3.0) < 0.1 && abs(idleSampleInterval - 12.0) < 0.1 && abs(workingWindow - 90.0) < 0.1 {
            return .eco
        }
        return nil
    }

    private func applyPreset(_ preset: PerformancePreset) {
        switch preset {
        case .fast:
            sampleInterval = 1.0
            idleSampleInterval = 3.0
            workingWindow = 30.0
        case .balanced:
            sampleInterval = 2.0
            idleSampleInterval = 5.0
            workingWindow = 60.0
        case .eco:
            sampleInterval = 3.0
            idleSampleInterval = 12.0
            workingWindow = 90.0
        }
        applyConfig()
    }

    private func resetEngineToDefaults() {
        let def = SettingsView.defaultConfig
        workingWindow = def.workingWindow
        sampleInterval = def.sampleInterval
        idleSampleInterval = def.idleSampleInterval
        cpuThreshold = def.cpuThreshold
        activeSessionWindow = def.activeSessionWindow
        applyConfig()
    }

    private func presetButton(_ preset: PerformancePreset, title: String) -> some View {
        let isSelected = currentPreset == preset
        return Button {
            applyPreset(preset)
        } label: {
            Text(title)
                .font(Theme.bodyFont(11.5, weight: isSelected ? .bold : .medium))
                .foregroundColor(isSelected ? Theme.actionBlue : Theme.ink)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity)
                .background(
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(isSelected ? Theme.actionBlue.opacity(0.12) : Theme.onDark.opacity(0.04))
                        .overlay(
                            RoundedRectangle(cornerRadius: 6, style: .continuous)
                                .stroke(isSelected ? Theme.actionBlue.opacity(0.6) : Theme.hairline.opacity(0.4), lineWidth: 0.8)
                        )
                )
        }
        .buttonStyle(.plain)
        .accessibilityValue(isSelected ? "已选择" : "未选择")
        .help(preset.description)
    }

    private func sliderRow(title: String, value: Binding<Double>, range: ClosedRange<Double>, step: Double = 1, unit: String,
                           onRelease: (() -> Void)? = nil) -> some View {
        HStack {
            Text(title)
                .font(Theme.bodyFont(12))
                .foregroundColor(Theme.ink)
            Spacer()
            Text(Self.formatSliderValue(value.wrappedValue, unit: unit))
                .font(Theme.monoFont(11))
                .foregroundColor(Theme.inkMuted48)
            Slider(value: value, in: range, step: step) { editing in
                if !editing {
                    applyConfig()
                    onRelease?()
                }
            }
            .tint(Theme.actionBlue)
            .frame(width: 140)
        }
    }

    // MARK: - 状态同步与持久化

    private func loadState() {
        let all = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs(),
                                             installedBundles: installedApps.installedBundleIDs())
        enabledAgents = EnabledAgentStore.resolvedEnabled(registry: all)
        customProfiles = AgentRegistry.loadCustomProfiles()
        applyConfig()
        launchAtLogin = (SMAppService.mainApp.status == .enabled)
    }

    private func applyConfig() {
        let normalized = EngineConfig(
            sampleInterval: sampleInterval,
            idleSampleInterval: idleSampleInterval,
            workingWindow: workingWindow,
            cpuThreshold: cpuThreshold,
            activeSessionWindow: activeSessionWindow,
            tokenAlertEnabled: tokenAlertEnabled,
            tokenAlertThreshold: tokenAlertThreshold,
            runawayCpuAlert: runawayCpuAlert
        ).normalized()
        sampleInterval = normalized.sampleInterval
        idleSampleInterval = normalized.idleSampleInterval
        workingWindow = normalized.workingWindow
        cpuThreshold = normalized.cpuThreshold
        activeSessionWindow = normalized.activeSessionWindow
        engine.config = normalized
    }

    private func applyLaunchAtLogin(_ enabled: Bool) {
        guard enabled != (SMAppService.mainApp.status == .enabled) else { return }
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            launchError = nil
        } catch {
            AppLog.error("SMAppService 注册失败: \(error)")
            launchAtLogin = SMAppService.mainApp.status == .enabled
            launchError = "自启动设置失败：\(error.localizedDescription)"
        }
    }

    private func binding(for profile: AgentProfile) -> Binding<Bool> {
        Binding(
            get: { enabledAgents.contains(profile.id) },
            set: { on in
                if on {
                    enabledAgents.insert(profile.id)
                } else {
                    enabledAgents.remove(profile.id)
                }
                saveEnabled()
                engine.setEnabled(enabledAgents)
            }
        )
    }

    private func saveEnabled() {
        EnabledAgentStore.save(enabledAgents)
    }

    // MARK: - 自定义增删

    private func addCustom(_ profile: AgentProfile) {
        customProfiles.append(profile)
        AgentRegistry.saveCustomProfiles(customProfiles)
        engine.addCustomProfile(profile)
        enabledAgents.insert(profile.id)
        saveEnabled()
        // 登记为「已知」：否则用户在同一次会话内关掉它后重开设置页，
        // resolvedEnabled 会把它当作版本升级新增项重新启用（静默覆盖用户意图）
        EnabledAgentStore.markKnown(profile.id)
    }

    private func removeCustom(_ profile: AgentProfile) {
        customProfiles.removeAll { $0.id == profile.id }
        AgentRegistry.saveCustomProfiles(customProfiles)
        engine.removeCustomProfile(profile.id)
        enabledAgents.remove(profile.id)
        saveEnabled()
    }
}

// MARK: - 自定义 Agent 行

struct CustomAgentRowView: View {
    let profile: AgentProfile
    let isEnabled: Binding<Bool>
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: profile.icon)
                .foregroundColor(Theme.actionBlue)
                .frame(width: 18)
            Text(profile.name)
                .font(Theme.bodyFont(13))
                .foregroundColor(Theme.ink)
                .lineLimit(1)
                .layoutPriority(1)
            Text(profile.processNames.joined(separator: ", "))
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
                .lineLimit(1)
                .layoutPriority(0)
                .help(profile.processNames.joined(separator: ", "))
            Spacer()
            Toggle("", isOn: isEnabled)
                .toggleStyle(.switch)
                .controlSize(.mini)
                .tint(Theme.actionBlue)
                .accessibilityLabel("启用 \(profile.name)")
            Button(role: .destructive, action: onRemove) {
                Image(systemName: "trash")
            }
            .buttonStyle(.plain)
            .foregroundColor(Theme.dangerRed)
            .help("删除自定义 Agent")
            .accessibilityLabel("删除 \(profile.name)")
        }
        .padding(.vertical, 2)
    }
}

// MARK: - 添加自定义 Agent 弹窗

struct AddCustomAgentSheet: View {
    @Environment(\.dismiss) private var dismiss
    let existingIDs: Set<String>
    let knownProcessNames: [String]
    let onAdd: (AgentProfile) -> Void

    @State private var name = ""
    @State private var icon = "terminal"
    @State private var processName = ""
    @State private var sessionDir = ""

    private static let procNameChars = CharacterSet.alphanumerics
        .union(CharacterSet(charactersIn: "_-."))
    private var processNameInvalid: Bool {
        let trimmed = processName.trimmingCharacters(in: .whitespaces)
        return !trimmed.isEmpty
            && trimmed.unicodeScalars.contains { !Self.procNameChars.contains($0) }
    }
    private var duplicateID: Bool {
        let trimmed = processName.trimmingCharacters(in: .whitespaces)
        return existingIDs.contains(AgentProfile.makeCustomID(trimmed))
    }
    private var nameConflict: Bool {
        let trimmed = processName.trimmingCharacters(in: .whitespaces).lowercased()
        guard !trimmed.isEmpty else { return false }
        // 与匹配器同口径（R22）：匹配是「相等或 name 分隔符前缀族」，校验必须双向拦——
        // 自定义名命中已知名前缀族（codex-helper vs codex）与已知名命中自定义名
        // 前缀族（codex vs 自定义 codex）都会让同一进程被两个 profile 计数
        return knownProcessNames.contains {
            ProcessMatcher.hasPrefixFamilyConflict(trimmed, $0)
        }
    }
    private var canAdd: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty
            && !processName.trimmingCharacters(in: .whitespaces).isEmpty
            && !processNameInvalid
            && !duplicateID
            && !nameConflict
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("添加自定义 Agent")
                .font(Theme.displayFont(15, weight: .semibold))
                .foregroundColor(Theme.ink)

            field("显示名", text: $name, placeholder: "例如：内部 QA Agent")
            field("进程名", text: $processName, placeholder: "例如：qa-agent（ps 里的命令名）")
            if processNameInvalid {
                Text("进程名含非法字符（仅允许字母、数字、_ - .）")
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.dangerRed)
            } else if duplicateID {
                Text("该进程已存在自定义条目")
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.dangerRed)
            } else if nameConflict {
                Text("该进程名已被内置/自动发现条目使用（会重复计数）")
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.dangerRed)
            }
            field("会话目录", text: $sessionDir, placeholder: "可选，例如：~/workspace/qa/sessions")

            HStack {
                Spacer()
                Button("取消") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("添加") {
                    let trimmedName = name.trimmingCharacters(in: .whitespaces)
                    let trimmedProc = processName.trimmingCharacters(in: .whitespaces)
                    let id = AgentProfile.makeCustomID(trimmedProc)
                    let dirs = sessionDir.isEmpty
                        ? []
                        : [(sessionDir as NSString).expandingTildeInPath]
                    onAdd(AgentProfile(
                        id: id,
                        name: trimmedName,
                        icon: icon,
                        bundleIDs: [],
                        processNames: [trimmedProc],
                        sessionDirs: dirs,
                        isCustom: true
                    ))
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .tint(Theme.actionBlue)
                .keyboardShortcut(.defaultAction)
                .disabled(!canAdd)
            }
        }
        .padding(24)
        .frame(width: 420)
        .background(Theme.parchment)
    }

    private func field(_ title: String, text: Binding<String>, placeholder: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(Theme.bodyFont(11, weight: .semibold))
                .foregroundColor(Theme.inkMuted80)
            TextField(placeholder, text: text)
                .textFieldStyle(.roundedBorder)
                .font(Theme.bodyFont(12))
        }
    }
}
