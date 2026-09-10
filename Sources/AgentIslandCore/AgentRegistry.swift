import Foundation

// MARK: - Agent 注册表
// 内置集（按主流 Agent 工具的实际安装形态维护）+ 自动发现（按传入的已安装集判定）+ 用户自定义（UserDefaults）
// 无全局可变状态：安装判定一律经注入的 InstalledAppsCache（见 InstalledAppsCache.swift）

public enum AgentRegistry {

    /// 内置定义（覆盖常见 Agent；自动发现负责标记哪些真实安装）
    public static let builtin: [AgentProfile] = [
        AgentProfile(
            id: "dim",
            name: "DimAgent",
            icon: "sparkles.rectangle.stack.fill",
            bundleIDs: ["com.dimcode.app"],
            processNames: ["DimAgent", "DimRemote", "dim"],
            sessionDirs: [home(".dimcode/v2/data/sessions")],
            category: .assistant
        ),
        AgentProfile(
            id: "claude",
            name: "Claude",
            icon: "bubble.left.and.bubble.right.fill",
            bundleIDs: ["com.anthropic.claudefordesktop", "com.anthropic.claudecode"],
            processNames: ["claude", "Claude", "claude-code"],
            sessionDirs: [home(".claude/sessions"), home(".claude/projects")],
            category: .assistant
        ),
        AgentProfile(
            id: "codex",
            name: "Codex",
            icon: "chevron.left.forwardslash.chevron.right",
            bundleIDs: [],
            processNames: ["codex", "Codex"],
            // 排除 ChatGPT 桌面版内嵌的 Codex（/Applications/ChatGPT.app/Contents/Resources/codex
            // 及其 Codex Framework 辅助进程）：与独立 codex CLI 同名同 basename，
            // 不排除会把同一个 ChatGPT 同时数成 ChatGPT 和 Codex 两个 Agent
            pathExcludes: ["/Applications/ChatGPT.app/"],
            // ChatGPT 桌面版内嵌 Codex（同一份 ~/.codex 会话目录、同一进程族）：
            // 宿主已装且 codex 无独立安装时不单独成条目，避免同一份程序显示两行
            hostBundleIDs: ["com.openai.codex"],
            // 只监控会话 JSONL 目录；~/.codex 根目录含 sqlite/WAL/cache，被 app 后台高频刷新，
            // 会导致 Codex 仅打开但未运行任务时被误判为 WORKING
            sessionDirs: [home(".codex/sessions")],
            category: .assistant
        ),
        AgentProfile(
            id: "cursor",
            name: "Cursor",
            icon: "cursorarrow.click.2",
            bundleIDs: ["com.todesktop.230113mital1efw", "com.cursor.cursor"],
            processNames: ["Cursor", "cursor"],
            sessionDirs: [home("Library/Application Support/Cursor/User/workspaceStorage")],
            category: .codeEditor
        ),
        AgentProfile(
            id: "trae",
            name: "Trae",
            icon: "paintbrush.pointed.fill",
            bundleIDs: ["cn.trae.solo.app", "com.trae.ai"],
            processNames: ["TRAE SOLO CN", "Trae", "trae", "Electron"],
            pathContains: ["trae"],
            sessionDirs: [home("Library/Application Support/Trae CN/User/workspaceStorage")],
            category: .codeEditor
        ),
        AgentProfile(
            id: "copilot",
            name: "ima.copilot",
            icon: "sparkles",
            bundleIDs: ["com.tencent.imamac"],
            processNames: ["ima.copilot", "Copilot"],
            sessionDirs: [home("Library/Application Support/com.tencent.imamac")],
            category: .assistant
        ),
        AgentProfile(
            id: "workbuddy",
            name: "WorkBuddy",
            icon: "briefcase.fill",
            bundleIDs: ["com.tencent.workbuddy.mac"],
            processNames: ["WorkBuddy", "workbuddy", "Electron"],
            pathContains: ["workbuddy"],
            sessionDirs: [
                home(".workbuddy/sessions"),
                home(".workbuddy/tasks"),
                home(".workbuddy/memory")
            ],
            category: .assistant
        ),
        AgentProfile(
            id: "zcode",
            name: "ZCode",
            icon: "curlybraces.square.fill",
            bundleIDs: ["dev.zcode.app"],
            processNames: ["ZCode", "zcode-host-local-1", "zcode-cli", "Electron"],
            pathContains: ["zcode"],
            sessionDirs: [
                home(".zcode/v2"),
                home("Library/Application Support/ZCode/session")
            ],
            category: .codeEditor
        ),
        AgentProfile(
            id: "antigravity",
            name: "Antigravity",
            icon: "atom",
            bundleIDs: ["com.google.antigravity", "com.yuzhiqiang.antigravity.studio"],
            processNames: ["Antigravity", "language_server", "agentapi", "Electron"],
            pathContains: ["antigravity"],
            sessionDirs: [
                home(".gemini/antigravity/conversations"),
                home(".gemini/antigravity/brain"),
                home("Library/Application Support/Antigravity")
            ],
            category: .codeEditor
        ),
        AgentProfile(
            id: "opencode",
            name: "OpenCode",
            icon: "terminal.fill",
            bundleIDs: ["ai.opencode.desktop"],
            processNames: ["opencode"],
            sessionDirs: [home(".config/opencode"), home(".local/share/opencode")],
            category: .assistant
        ),
        AgentProfile(
            id: "hermes",
            name: "Hermes Agent",
            icon: "wand.and.stars",
            bundleIDs: [],
            processNames: ["hermes-agent", "hermes"],
            sessionDirs: [home(".hermes/sessions"), home(".hermes/logs")],
            category: .assistant
        ),
        AgentProfile(
            id: "continue",
            name: "Continue",
            icon: "arrow.triangle.2.circlepath",
            bundleIDs: ["com.continue.continue"],
            processNames: ["Continue", "continue"],
            sessionDirs: [home(".continue")],
            defaultEnabled: false,
            category: .codeEditor
        ),
        AgentProfile(
            id: "chatgpt",
            name: "ChatGPT",
            icon: "brain.head.profile",
            bundleIDs: ["com.openai.codex"],
            processNames: ["ChatGPT"],
            sessionDirs: [home("Library/Application Support/com.openai.codex")],
            category: .assistant
        ),
        AgentProfile(
            id: "dsh",
            name: "DSH",
            icon: "bolt.horizontal.fill",
            bundleIDs: [],
            processNames: ["dsh"],
            pathContains: ["deepseek-harness"],
            sessionDirs: [home(".dsh/sessions"), home(".dsh/storages")],
            category: .assistant
        ),
        AgentProfile(
            id: "ego-browser",
            name: "Ego Browser",
            icon: "globe.americas.fill",
            bundleIDs: ["com.citrolabs.ego.lite"],
            processNames: ["ego-browser", "ego lite", "ego"],
            sessionDirs: [home(".local/share/ego"), home("Library/Application Support/ego lite")],
            category: .assistant
        ),
        AgentProfile(
            id: "vibe-usage",
            name: "Vibe Usage",
            icon: "chart.bar.xaxis",
            bundleIDs: ["ai.vibecafe.vibe-usage"],
            processNames: ["vibe-usage", "Vibe Usage"],
            sessionDirs: [home(".vibe-usage"), home("Library/Application Support/Vibe Usage")],
            category: .assistant
        ),
        AgentProfile(
            id: "openviking",
            name: "OpenViking",
            icon: "shippingbox.fill",
            bundleIDs: [],
            processNames: ["openviking", "openviking-server", "ov", "vikingbot"],
            pathContains: ["openviking"],
            sessionDirs: [home(".openviking"), home(".local/share/uv/tools/openviking")],
            category: .assistant
        ),
    ]

    // MARK: - 自动发现

    /// 自动发现的额外 CLI profile（不在内置集里的 CLI，如 aider/gemini/windsurf）。
    /// installedCLIs 由调用方从 InstalledAppsCache 取得——本类型不读任何全局状态
    public static func discoverCLIProfiles(installedCLIs: Set<String>,
                                           installedBundles: Set<String> = []) -> [AgentProfile] {
        // 判重基线用 filteredBuiltin（含宿主内嵌过滤），与调用方展示/启停口径一致：
        // 否则被 hostBundleIDs 过滤掉的组件仍会在这里以 CLI 形式重复出现
        let existing = Set(filteredBuiltin(installedCLIs: installedCLIs,
                                           installedBundles: installedBundles)
            .flatMap { $0.processNames.map { $0.lowercased() } })
        var extra: [AgentProfile] = []
        for cli in installedCLIs where !existing.contains(cli) {
            extra.append(AgentProfile(
                id: "cli-\(cli)",
                name: cli,
                icon: "terminal",
                bundleIDs: [],
                processNames: [cli],
                sessionDirs: [],
                defaultEnabled: false,   // 自动发现默认关闭（与设置页注释一致，避免首启全开）
                category: .assistant
            ))
        }
        return extra
    }

    // MARK: - 用户自定义（UserDefaults）

    public static func loadCustomProfiles() -> [AgentProfile] {
        guard let data = UserDefaults.standard.data(forKey: SettingKey.customAgents),
              let list = try? JSONDecoder().decode([AgentProfile].self, from: data) else {
            return []
        }
        return list
    }

    public static func saveCustomProfiles(_ profiles: [AgentProfile]) {
        if let data = try? JSONEncoder().encode(profiles) {
            UserDefaults.standard.set(data, forKey: SettingKey.customAgents)
        }
    }

    /// 完整注册表：内置 + 自动发现 CLI（按传入已安装集）+ 自定义。
    /// installedBundles 用于识别「宿主内嵌组件」：宿主已装而本组件无独立安装时跳过，
    /// 避免同一份程序（ChatGPT 桌面版内嵌的 Codex）在列表里显示成两个 Agent。
    public static func fullRegistry(installedCLIs: Set<String>,
                                    installedBundles: Set<String> = []) -> [AgentProfile] {
        var list = filteredBuiltin(installedCLIs: installedCLIs, installedBundles: installedBundles)
        list.append(contentsOf: discoverCLIProfiles(installedCLIs: installedCLIs,
                                                    installedBundles: installedBundles))
        list.append(contentsOf: loadCustomProfiles())
        return list
    }

    /// 内置条目按「宿主内嵌」规则过滤（fullRegistry 与 discoverCLIProfiles 的共用基线，
    /// 抽成独立函数以打断两者的相互调用）
    static func filteredBuiltin(installedCLIs: Set<String>,
                                installedBundles: Set<String>) -> [AgentProfile] {
        builtin.filter { profile in
            guard !profile.hostBundleIDs.isEmpty else { return true }
            let hostInstalled = profile.hostBundleIDs.contains { installedBundles.contains($0.lowercased()) }
            guard hostInstalled else { return true }
            let selfInstalled = profile.bundleIDs.contains { installedBundles.contains($0.lowercased()) }
                || profile.processNames.contains { installedCLIs.contains($0.lowercased()) }
            return selfInstalled
        }
    }

    /// 单条查找（内置 + 自定义；自动发现条目不在本查找范围，须走 fullRegistry）
    public static func profile(id: String) -> AgentProfile? {
        fullRegistry(installedCLIs: []).first { $0.id == id }
    }

    // MARK: - 进程名冲突校验（自定义 Agent 用）

    /// 纯核：「已安装 或 已启用」条目的进程名集合（小写）。
    /// - 已启用条目：同进程名必然双份计数，必须拦
    /// - 已安装但禁用：日后启用会同名双份，也要拦
    /// - 未安装且未启用：进程不会运行，不误拦
    /// installed 集注入（测试 canned，生产读本类型全局缓存）
    public static func conflictingProcessNames(
        registry: [AgentProfile],
        enabledIDs: Set<String>,
        installedCLIs: Set<String>,
        installedBundles: Set<String>
    ) -> [String] {
        var names = Set<String>()
        for p in registry {
            let installed = p.bundleIDs.contains { installedBundles.contains($0.lowercased()) }
                || p.processNames.contains { installedCLIs.contains($0.lowercased()) }
            if installed || enabledIDs.contains(p.id) {
                names.formUnion(p.processNames.map { $0.lowercased() })
            }
        }
        return Array(names)
    }

    /// 便捷入口：registry 取全量、installed 集取自注入缓存（设置表单用）
    public static func conflictingProcessNames(
        enabledIDs: Set<String>,
        installedApps: InstalledAppsCache
    ) -> [String] {
        conflictingProcessNames(
            registry: fullRegistry(installedCLIs: installedApps.installedCLIs(),
                                   installedBundles: installedApps.installedBundleIDs()),
            enabledIDs: enabledIDs,
            installedCLIs: installedApps.installedCLIs(),
            installedBundles: installedApps.installedBundleIDs())
    }

    // MARK: - 工具

    private static func home(_ path: String) -> String {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(path).path
    }
}
