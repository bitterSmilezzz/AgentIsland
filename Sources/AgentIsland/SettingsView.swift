import AgentIslandCore
import SwiftUI
import AppKit
import ServiceManagement

// MARK: - 设置分类 Tab

enum SettingsTab: String, CaseIterable, Identifiable {
    case general
    case agents
    case engine
    case about

    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: return "通用与外观"
        case .agents: return "Agent 监控"
        case .engine: return "引擎与性能"
        case .about: return "关于"
        }
    }

    var icon: String {
        switch self {
        case .general: return "paintpalette"
        case .agents: return "person.2.badge.gearshape"
        case .engine: return "gauge.with.dots.needle.bottom.50percent"
        case .about: return "info.circle"
        }
    }
}

// MARK: - 设置卡片容器（Apple Inset-Grouped Style）

struct SettingsCard<Content: View>: View {
    let title: String?
    let content: Content

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
                            .stroke(Theme.hairline.opacity(0.5), lineWidth: 0.5)
                    )
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

    @AppStorage(SettingKey.workingWindow) private var workingWindow: Double = SettingsView.defaultConfig.workingWindow
    @AppStorage(SettingKey.sampleInterval) private var sampleInterval: Double = SettingsView.defaultConfig.sampleInterval
    @AppStorage(SettingKey.idleSampleInterval) private var idleSampleInterval: Double = SettingsView.defaultConfig.idleSampleInterval
    @AppStorage(SettingKey.cpuThreshold) private var cpuThreshold: Double = SettingsView.defaultConfig.cpuThreshold
    @AppStorage(SettingKey.activeSessionWindow) private var activeSessionWindow: Double = SettingsView.defaultConfig.activeSessionWindow
    @AppStorage(SettingKey.collapseDelay) private var collapseDelay: Double = 0.5   // 面板行为参数（非引擎采样配置）
    @AppStorage(SettingKey.launchAtLogin) private var launchAtLogin = false
    @AppStorage(SettingKey.islandAppearance) private var islandAppearanceRaw = IslandAppearance.system.rawValue

    private var islandAppearance: Binding<IslandAppearance> {
        Binding(
            get: { IslandAppearance(rawValue: islandAppearanceRaw) ?? .system },
            set: { islandAppearanceRaw = $0.rawValue }
        )
    }

    @State private var selectedTab: SettingsTab = .general
    @State private var enabledAgents: Set<String> = []
    /// 自启动设置失败提示（SMAppService 未签名/非 /Applications 时 register 抛错）
    @State private var launchError: String?
    @State private var customProfiles: [AgentProfile] = []
    @State private var showAddCustom = false
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
                VStack(alignment: .leading, spacing: 8) {
                    Picker("外观模式", selection: islandAppearance) {
                        ForEach(IslandAppearance.allCases) { mode in
                            Text(mode.label).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .onChange(of: islandAppearanceRaw) { _ in
                        controller.applyAppearance(IslandAppearance(rawValue: islandAppearanceRaw) ?? .system)
                    }

                    Text("控制侧边栏灵动岛面板的配色；设置窗口本身跟随系统。")
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
                            controller.resetPosition()
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
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
                    ForEach(AgentRegistry.builtin) { profile in
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

            let discovered = AgentRegistry.discoverCLIProfiles(installedCLIs: installedApps.installedCLIs())
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
                                onRemove: { removeCustom(profile) }
                            )
                        }
                    }
                }
            }
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

            SettingsCard(title: "采样节律与能耗") {
                VStack(alignment: .leading, spacing: 12) {
                    sliderRow(title: "活动采样间隔", value: $sampleInterval, range: 1...10, unit: "秒")
                    sliderRow(title: "闲置降频间隔", value: $idleSampleInterval, range: 5...60, unit: "秒")

                    Text("当有 Agent 处于活跃工作中时，引擎以活动采样间隔高频探测（默认 2s）；当全部待机或离线时，自动降频至闲置间隔以节省 CPU 与电量。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)
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
                        Text("v1.4.0 · macOS 灵动岛 Agent 会话监控器")
                            .font(Theme.bodyFont(11))
                            .foregroundColor(Theme.inkMuted80)
                    }
                }
                .padding(.vertical, 4)

                Divider()

                VStack(alignment: .leading, spacing: 6) {
                    Label("只读监控：绝不读取会话内容、私钥凭据与隐私数据", systemImage: "lock.shield")
                    Label("高性能低能耗：空闲自动降频，工作态 CPU 开销约 1%", systemImage: "bolt.badge.clock")
                    Label("自由拖拽与智能贴边：顶部/右侧自由拖动吸附，6pt 微细条触碰自动弹出", systemImage: "arrow.up.and.down.and.arrow.left.and.right")
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
        enabledAgents = EnabledAgentStore.load() ?? Set(engine.allProfiles.map(\.id))
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
            activeSessionWindow: activeSessionWindow
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
            print("SMAppService 失败: \(error)")
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
            Text(profile.processNames.joined(separator: ","))
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
                .lineLimit(1)
                .layoutPriority(0)
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
        return !trimmed.isEmpty && knownProcessNames.contains(trimmed)
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
