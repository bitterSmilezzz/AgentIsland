import AgentIslandCore
import SwiftUI
import AppKit
import ServiceManagement

// MARK: - 设置窗口
// Apple 原生风格：Form + 分组 + Action Blue 强调色

struct SettingsView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    private static let defaultConfig = EngineConfig()

    @AppStorage(SettingKey.workingWindow) private var workingWindow: Double = SettingsView.defaultConfig.workingWindow
    @AppStorage(SettingKey.sampleInterval) private var sampleInterval: Double = SettingsView.defaultConfig.sampleInterval
    @AppStorage(SettingKey.idleSampleInterval) private var idleSampleInterval: Double = SettingsView.defaultConfig.idleSampleInterval
    @AppStorage(SettingKey.cpuThreshold) private var cpuThreshold: Double = SettingsView.defaultConfig.cpuThreshold
    @AppStorage(SettingKey.activeSessionWindow) private var activeSessionWindow: Double = SettingsView.defaultConfig.activeSessionWindow
    @AppStorage(SettingKey.collapseDelay) private var collapseDelay: Double = SettingsView.defaultConfig.collapseDelay
    @AppStorage(SettingKey.launchAtLogin) private var launchAtLogin = false
    @AppStorage(SettingKey.islandAppearance) private var islandAppearanceRaw = IslandAppearance.system.rawValue

    private var islandAppearance: Binding<IslandAppearance> {
        Binding(
            get: { IslandAppearance(rawValue: islandAppearanceRaw) ?? .system },
            set: { islandAppearanceRaw = $0.rawValue }
        )
    }

    @State private var enabledAgents: Set<String> = []
    /// 自启动设置失败提示（SMAppService 未签名/非 /Applications 时 register 抛错）
    @State private var launchError: String?
    @State private var customProfiles: [AgentProfile] = []
    @State private var showAddCustom = false
    /// 安装缓存扫描完成版本号：触发 body 重算刷新自动发现列表（阿剩低B）
    @State private var installedScanVersion = 0

    var body: some View {
        // 依赖注册：让 body 真正读取扫描版本号（阿剩低B 修正——@State 只有在 body
        // 求值中被读取才会建立依赖，bump 才能触发重算刷新自动发现列表）
        _ = installedScanVersion
        return VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    appearanceSection
                    Divider()
                    agentSection
                    Divider()
                    customSection
                    Divider()
                    behaviorSection
                    Divider()
                    aboutSection
                }
                .padding(20)
            }
        }
        .frame(width: 480, height: 620)
        .background(Theme.parchment)
        .onAppear {
            // 打开设置页即重扫安装缓存（装新 CLI/App 后进设置页确认是最常见场景，
            // 离线态下引擎低频刷新最长滞后 2 小时）。
            // 后台执行避免 /Applications plist 扫描阻塞窗口首帧（阿证中1/阿菜/阿剩
            // 三方同源：扫描 30-100ms 不应占用主线程）；完成后主线程 bump 版本号
            // 触发 body 重算刷新自动发现列表（阿剩低B）
            DispatchQueue.global(qos: .utility).async {
                AgentRegistry.refreshInstalledCache()
                DispatchQueue.main.async {
                    installedScanVersion += 1
                    // 设置页刷新后同步引擎时间戳（阿剩低3：markInstalledRefreshed 注释
                    // 声称覆盖 AppContext 首刷/设置页两路径，之前只接了首刷一处）
                    engine.markInstalledRefreshed()
                }
            }
            loadState()
        }
    }

    // MARK: 头部

    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "sparkles")
                .font(.system(size: 16, weight: .semibold))
                .foregroundColor(Theme.actionBlue)
            Text("AgentIsland 设置")
                .font(Theme.displayFont(17, weight: .semibold))
                .foregroundColor(Theme.ink)
            Spacer()
            Button("完成") {
                NSApp.keyWindow?.close()
            }
            .buttonStyle(.borderedProminent)
            .tint(Theme.actionBlue)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    // MARK: Agent 开关

    private var agentSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("监控的 Agent（内置）")
                .font(Theme.bodyFont(13, weight: .semibold))
                .foregroundColor(Theme.ink)

            ForEach(AgentRegistry.builtin) { profile in
                Toggle(isOn: binding(for: profile)) {
                    HStack(spacing: 8) {
                        Image(systemName: profile.icon)
                            .foregroundColor(Theme.actionBlue)
                            .frame(width: 18)
                        Text(profile.name)
                            .font(Theme.bodyFont(13))
                            .foregroundColor(Theme.ink)
                        if let snapshot = engine.snapshots.first(where: { $0.id == profile.id }) {
                            Text(snapshot.level.label)
                                .font(Theme.bodyFont(10, weight: .semibold))
                                .foregroundColor(snapshot.level.color)
                        }
                    }
                }
                .toggleStyle(.switch)
                .tint(Theme.actionBlue)
            }

            // 自动发现的额外 CLI：做成可开关条目（默认关闭，用户可启用监控）
            // 之前只读展示，用户无法启用；且 defaultEnabled=false 导致首次切换
            // 任意开关时这些项会被误塞进引擎或显示与实态脱节（阿剩中1）
            let discovered = AgentRegistry.discoverCLIProfiles()
            if !discovered.isEmpty {
                Text("自动发现")
                    .font(Theme.bodyFont(13, weight: .semibold))
                    .foregroundColor(Theme.ink)
                    .padding(.top, 4)
                ForEach(discovered) { profile in
                    Toggle(isOn: binding(for: profile)) {
                        HStack(spacing: 8) {
                            Image(systemName: "terminal")
                                .foregroundColor(Theme.inkMuted48)
                                .frame(width: 18)
                            Text(profile.name)
                                .font(Theme.bodyFont(13))
                                .foregroundColor(Theme.ink)
                            if let snapshot = engine.snapshots.first(where: { $0.id == profile.id }) {
                                Text(snapshot.level.label)
                                    .font(Theme.bodyFont(10, weight: .semibold))
                                    .foregroundColor(snapshot.level.color)
                            }
                        }
                    }
                    .toggleStyle(.switch)
                    .tint(Theme.actionBlue)
                }
            }
        }
    }

    // MARK: 自定义 Agent

    private var customSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("自定义 Agent")
                    .font(Theme.bodyFont(13, weight: .semibold))
                    .foregroundColor(Theme.ink)
                Spacer()
                Button {
                    showAddCustom = true
                } label: {
                    Image(systemName: "plus.circle.fill")
                        .foregroundColor(Theme.actionBlue)
                }
                .buttonStyle(.plain)
                .help("添加自定义 Agent（进程名 + 会话目录）")
            }

            if customProfiles.isEmpty {
                Text("没有自定义条目。可添加内部脚本、自研 agent 等。")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.inkMuted48)
            }

            ForEach(Array(customProfiles), id: \.id) { profile in
                // 自定义项可单独启停（阿剩中2：之前只能删了重加，暂停监控会丢数据）
                CustomAgentRowView(profile: profile,
                                   isEnabled: binding(for: profile),
                                   onRemove: { removeCustom(profile) })
            }
        }
        .sheet(isPresented: $showAddCustom) {
            // 冲突集合 = 「已安装 或 已启用」条目的进程名（规则与推导在 AgentRegistry，Core 可测）
            AddCustomAgentSheet(existingIDs: Set(customProfiles.map(\.id)),
                                knownProcessNames: AgentRegistry.conflictingProcessNames(
                                    enabledIDs: Set(engine.allProfiles.map(\.id)))) { profile in
                addCustom(profile)
            }
        }
    }

    // MARK: 外观

    private var appearanceSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("外观")
                .font(Theme.bodyFont(13, weight: .semibold))
                .foregroundColor(Theme.ink)

            Picker("灵动岛外观", selection: islandAppearance) {
                ForEach(IslandAppearance.allCases) { mode in
                    Text(mode.label).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .onChange(of: islandAppearanceRaw) { _ in
                // 直调面板实时生效（带 payload，无通知/回读绕路）
                controller.applyAppearance(IslandAppearance(rawValue: islandAppearanceRaw) ?? .system)
            }
            Text("控制灵动岛卡片的配色；设置窗口本身跟随系统。")
                .font(Theme.bodyFont(10))
                .foregroundColor(Theme.inkMuted48)
        }
    }

    // MARK: 行为参数

    private var behaviorSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("行为")
                .font(Theme.bodyFont(13, weight: .semibold))
                .foregroundColor(Theme.ink)

            sliderRow(title: "「工作中」写入窗口", value: $workingWindow, range: 10...300, unit: "秒")
            // range 1...50（阿菜低：之前 0.5...50 + step 1 档位全是半值 0.5/1.5/2.5…，
            // 默认 1 不在档位上首次拖动即跳变）
            sliderRow(title: "CPU 判定阈值", value: $cpuThreshold, range: EngineConfig.cpuThresholdRange, unit: "%")
            sliderRow(title: "活跃会话窗口", value: $activeSessionWindow, range: 60...3600, unit: "秒")
            sliderRow(title: "活动采样间隔", value: $sampleInterval, range: 1...10, unit: "秒")
            sliderRow(title: "闲置降频间隔", value: $idleSampleInterval, range: 5...60, unit: "秒")
            sliderRow(title: "自动收起延迟", value: $collapseDelay, range: 0.2...5, step: 0.1, unit: "秒")

            Toggle(isOn: $launchAtLogin) {
                Text("登录时自动启动")
                    .font(Theme.bodyFont(12))
                    .foregroundColor(Theme.inkMuted80)
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
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// 滑块值统一格式：一位小数去尾零（10.0→"10"、0.5→"0.5"、12.5→"12.5"）
    private static func formatSliderValue(_ v: Double, unit: String) -> String {
        let s = String(format: "%.1f", v)
        let trimmed = s.hasSuffix(".0") ? String(s.dropLast(2)) : s
        // 单位不加空格：与详情卡 "7.3%" 口径统一（阿菜记录在案：'1 %' vs '7.3%'）
        return "\(trimmed)\(unit)"
    }

    private func sliderRow(title: String, value: Binding<Double>, range: ClosedRange<Double>, step: Double = 1, unit: String) -> some View {
        HStack {
            Text(title)
                .font(Theme.bodyFont(12))
                .foregroundColor(Theme.inkMuted80)
            Spacer()
            // 统一格式：一位小数去尾零（10.0→"10"、0.5→"0.5"），避免 ≥10 截断丢精度（阿菜低1）
            Text(Self.formatSliderValue(value.wrappedValue, unit: unit))
                .font(Theme.monoFont(11))
                .foregroundColor(Theme.inkMuted48)
            // 拖动中只更新显示值，松手才 applyConfig——避免每帧重启采样定时器
            Slider(value: value, in: range, step: step) { editing in
                if !editing { applyConfig() }
            }
                .tint(Theme.actionBlue)
                .frame(width: 120)
        }
    }

    // MARK: 关于

    private var aboutSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("关于")
                .font(Theme.bodyFont(13, weight: .semibold))
                .foregroundColor(Theme.ink)
            Text("AgentIsland — 监控本机 Agent 软件会话活动的灵动岛工具")
                .font(Theme.bodyFont(12))
                .foregroundColor(Theme.inkMuted48)
            Text("v1.2.0 · 只读监控，不读取会话内容")
                .font(Theme.bodyFont(11))
                .foregroundColor(Theme.inkMuted48)
            Link("GitHub: bitterSmilezzz/AgentIsland",
                 destination: URL(string: "https://github.com/bitterSmilezzz/AgentIsland")!)
                .font(Theme.bodyFont(11))
                .foregroundColor(Theme.actionBlue)
        }
    }

    // MARK: - 状态同步

    private func loadState() {
        // 启停集合：以引擎当前启用集为准（避免界面显示与引擎实态脱节——
        // 之前用 fullRegistry().filter(defaultEnabled) 会把自动发现项一次性显示为开，
        // 但引擎首启只启用内置，切换任意开关会把所有自动发现项塞进引擎）。
        // 无记录（nil）回退引擎当前集；空数组是主动全关，照常显示为全关。
        enabledAgents = EnabledAgentStore.load() ?? Set(engine.allProfiles.map(\.id))
        // 自定义条目
        customProfiles = AgentRegistry.loadCustomProfiles()
        // 参数同步到引擎
        applyConfig()
        // 自启状态回显
        launchAtLogin = (SMAppService.mainApp.status == .enabled)
    }

    private func applyConfig() {
        // 归一化唯一实现在 EngineConfig.normalized（cpuThreshold 区间自愈 + sample≤idle 钳平），
        // 表单/引擎/持久化三方共用同一规则
        let normalized = EngineConfig(
            sampleInterval: sampleInterval,
            idleSampleInterval: idleSampleInterval,
            workingWindow: workingWindow,
            cpuThreshold: cpuThreshold,
            activeSessionWindow: activeSessionWindow,
            collapseDelay: collapseDelay
        ).normalized()
        // 回写归一化结果：表单/持久化与引擎同源（normalized 未来扩展钳制字段时此处零跟进；
        // 旧版半值残留即在此写回清除，阿菜记录在案）
        sampleInterval = normalized.sampleInterval
        idleSampleInterval = normalized.idleSampleInterval
        workingWindow = normalized.workingWindow
        cpuThreshold = normalized.cpuThreshold
        activeSessionWindow = normalized.activeSessionWindow
        collapseDelay = normalized.collapseDelay
        // 一次性赋值：config didSet 触发一次（原逐属性六次触发、五次冗余重启定时器）
        engine.config = normalized
    }

    private func applyLaunchAtLogin(_ enabled: Bool) {
        // 防误触：loadState 回显 launchAtLogin 也会走 onChange；
        // 目标状态与系统实际状态一致时直接跳过，避免每次打开设置页都 register/unregister
        guard enabled != (SMAppService.mainApp.status == .enabled) else { return }
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            launchError = nil
        } catch {
            // 失败必须回显到 UI（未签名/非 /Applications 安装时 register 会抛错）
            print("SMAppService 失败: \(error)")
            launchAtLogin = SMAppService.mainApp.status == .enabled   // 回滚开关状态
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
        // 空集合是有意全关，照常写入（语义单点在 EnabledAgentStore）
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
                .layoutPriority(1)   // 名称优先占位，进程名可压缩（阿剩低2）
            Text(profile.processNames.joined(separator: ","))
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
                .lineLimit(1)
                .layoutPriority(0)
            Spacer()
            // 启用开关：binding(for:) → enabledAgents + engine.setEnabled（统一路径）
            Toggle("", isOn: isEnabled)
                .toggleStyle(.switch)
                .controlSize(.mini)
                .tint(Theme.actionBlue)
                .accessibilityLabel("启用 \(profile.name)")   // 空标签 Toggle 补 a11y（阿剩低2）
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
    /// 内置 + 自动发现 + 已存自定义的全部进程名（小写），用于冲突提示
    let knownProcessNames: [String]
    let onAdd: (AgentProfile) -> Void

    @State private var name = ""
    @State private var icon = "terminal"
    @State private var processName = ""
    @State private var sessionDir = ""

    /// 进程名白名单：字母数字 + 下划线/连字符/点（ps 命令名合法字符集）
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
    /// 与内置/自动发现/已存自定义的进程名冲突（同一进程会被双份匹配计数）
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
