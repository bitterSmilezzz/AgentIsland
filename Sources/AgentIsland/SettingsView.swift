import AgentIslandCore
import SwiftUI
import AppKit
import ServiceManagement

// MARK: - 设置窗口
// Apple 原生风格：Form + 分组 + Action Blue 强调色

struct SettingsView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    @AppStorage("workingWindow") private var workingWindow: Double = 60
    @AppStorage("sampleInterval") private var sampleInterval: Double = 2
    @AppStorage("idleSampleInterval") private var idleSampleInterval: Double = 15
    @AppStorage("cpuThreshold") private var cpuThreshold: Double = 1
    @AppStorage("activeSessionWindow") private var activeSessionWindow: Double = 600
    @AppStorage("collapseDelay") private var collapseDelay: Double = 0.5
    @AppStorage("launchAtLogin") private var launchAtLogin = false
    @AppStorage("islandAppearance") private var islandAppearanceRaw = IslandAppearance.system.rawValue
    @AppStorage("enabledAgents") private var enabledAgentsData: Data = Data()

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

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
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
        .onAppear(perform: loadState)
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
            AddCustomAgentSheet(existingIDs: Set(customProfiles.map(\.id)),
                                knownProcessNames: Self.conflictProcessNames(engine: engine)) { profile in
                addCustom(profile)
            }
        }
    }

    /// 冲突校验进程名集合：覆盖「已安装 或 已启用」的全部条目（内置+自动发现+自定义）。
    /// - 已启用条目：同进程名必然双份计数，必须拦（阿剩第五轮基础）
    /// - 已安装但禁用的内置：日后启用会同进程双份，也要拦（阿剩第六轮指正——仅引擎启用集漏检）
    /// - 未安装的 CLI：进程不会运行，同进程名无实际冲突，不误拦（第五轮「未安装不误拦」）
    private static func conflictProcessNames(engine: ActivityEngine) -> [String] {
        let installedCLIs = AgentRegistry.installedCLIs()
        let installedBundles = AgentRegistry.installedBundleIDs()
        var names = Set<String>()
        for p in AgentRegistry.fullRegistry() {
            let installed = p.bundleIDs.contains { installedBundles.contains($0.lowercased()) }
                || p.processNames.contains { installedCLIs.contains($0.lowercased()) }
            let enabled = engine.allProfiles.contains { $0.id == p.id }
            if installed || enabled {
                names.formUnion(p.processNames.map { $0.lowercased() })
            }
        }
        return Array(names)
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
                // 显式通知面板实时切换（不依赖跨进程 UserDefaults 通知）
                NotificationCenter.default.post(name: .islandAppearanceChanged, object: nil)
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
            sliderRow(title: "CPU 判定阈值", value: $cpuThreshold, range: 0.5...50, unit: "%")
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

    private func sliderRow(title: String, value: Binding<Double>, range: ClosedRange<Double>, step: Double = 1, unit: String) -> some View {
        HStack {
            Text(title)
                .font(Theme.bodyFont(12))
                .foregroundColor(Theme.inkMuted80)
            Spacer()
            Text(value.wrappedValue >= 10 ? "\(Int(value.wrappedValue)) \(unit)" : String(format: "%.1f \(unit)", value.wrappedValue))
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
        // 注意：用户主动全关会存「空数组」，不算无记录，不能回退默认。
        if let saved = try? JSONDecoder().decode([String].self, from: enabledAgentsData) {
            enabledAgents = Set(saved)
        } else {
            enabledAgents = Set(engine.allProfiles.map(\.id))
        }
        // 自定义条目
        customProfiles = AgentRegistry.loadCustomProfiles()
        // 参数同步到引擎
        applyConfig()
        // 自启状态回显
        launchAtLogin = (SMAppService.mainApp.status == .enabled)
    }

    private func applyConfig() {
        // 参数钳制：sample ≤ idle（否则「闲置降频」逻辑反转）
        if sampleInterval > idleSampleInterval {
            sampleInterval = idleSampleInterval
        }
        engine.config.sampleInterval = sampleInterval
        engine.config.idleSampleInterval = idleSampleInterval
        engine.config.workingWindow = workingWindow
        engine.config.cpuThreshold = cpuThreshold
        engine.config.activeSessionWindow = activeSessionWindow
        engine.config.collapseDelay = collapseDelay
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
        if let data = try? JSONEncoder().encode(Array(enabledAgents)) {
            enabledAgentsData = data
        }
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
            Text(profile.processNames.joined(separator: ","))
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
                .lineLimit(1)
            Spacer()
            // 启用开关：binding(for:) → enabledAgents + engine.setEnabled（统一路径）
            Toggle("", isOn: isEnabled)
                .toggleStyle(.switch)
                .controlSize(.mini)
                .tint(Theme.actionBlue)
            Button(role: .destructive, action: onRemove) {
                Image(systemName: "trash")
            }
            .buttonStyle(.plain)
            .foregroundColor(Theme.dangerRed)
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
        let id = "custom-\(trimmed.lowercased().replacingOccurrences(of: " ", with: "-"))"
        return existingIDs.contains(id)
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
                    let id = "custom-\(trimmedProc.lowercased().replacingOccurrences(of: " ", with: "-"))"
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
