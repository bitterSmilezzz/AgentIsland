import Foundation

// MARK: - Agent 注册表
// 内置集（按 2026-09 本机调查修正）+ 自动发现（按传入的已安装集判定）+ 用户自定义（UserDefaults）
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
            bundleIDs: ["com.openai.codex"],
            processNames: ["codex", "Codex"],
            sessionDirs: [home(".codex/sessions"), home(".codex")],
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
            sessionDirs: [home(".workbuddy/traces")],
            category: .assistant
        ),
        AgentProfile(
            id: "opencode",
            name: "OpenCode",
            icon: "terminal.fill",
            bundleIDs: [],
            processNames: ["opencode"],
            sessionDirs: [home(".config/opencode")],
            category: .assistant
        ),
        AgentProfile(
            id: "hermes",
            name: "Hermes Agent",
            icon: "wand.and.stars",
            bundleIDs: [],
            processNames: ["hermes-agent", "hermes"],
            sessionDirs: [home(".local/share/hermes")],
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
    ]

    // MARK: - 自动发现

    /// 自动发现的额外 CLI profile（不在内置集里的 CLI，如 aider/gemini/windsurf）。
    /// installedCLIs 由调用方从 InstalledAppsCache 取得——本类型不读任何全局状态
    public static func discoverCLIProfiles(installedCLIs: Set<String>) -> [AgentProfile] {
        let existing = Set(builtin.flatMap { $0.processNames.map { $0.lowercased() } })
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

    /// 完整注册表：内置 + 自动发现 CLI（按传入已安装集）+ 自定义
    public static func fullRegistry(installedCLIs: Set<String>) -> [AgentProfile] {
        var list = builtin
        list.append(contentsOf: discoverCLIProfiles(installedCLIs: installedCLIs))
        list.append(contentsOf: loadCustomProfiles())
        return list
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
            registry: fullRegistry(installedCLIs: installedApps.installedCLIs()),
            enabledIDs: enabledIDs,
            installedCLIs: installedApps.installedCLIs(),
            installedBundles: installedApps.installedBundleIDs())
    }

    // MARK: - 工具

    private static func home(_ path: String) -> String {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(path).path
    }
}
