import Foundation

// MARK: - Agent 注册表
// 内置集（按 2026-09 本机调查修正）+ 自动发现（/Applications + PATH）+ 用户自定义（UserDefaults）

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

    /// 已知 CLI 名（PATH 扫描用）
    static let knownCLIs = ["dim", "codex", "claude", "cursor", "trae", "opencode",
                            "hermes-agent", "aider", "gemini", "windsurf", "agent-browser",
                            "tiny-agents", "continue"]

    /// 已知 GUI bundle id（/Applications 扫描用）
    static let knownBundleIDs: [String: String] = [
        "com.dimcode.app": "dim",
        "com.anthropic.claudefordesktop": "claude",
        "com.anthropic.claudecode": "claude",
        "com.openai.codex": "codex",
        "com.todesktop.230113mital1efw": "cursor",
        "cn.trae.solo.app": "trae",
        "com.tencent.imamac": "copilot",
        "com.tencent.workbuddy.mac": "workbuddy",
        "com.continue.continue": "continue",
    ]

    /// 已安装的 CLI 集合（小写命令名）
    /// 静态缓存：避免 fullRegistry/discoverCLI/refreshInstalled 各扫一遍；
    /// 运行期可 refreshInstalledCache() 重扫（阿剩低3：装新 CLI 不必重启 App）
    /// 锁保护：refreshInstalledCache 可后台执行，读写都要加锁（阿证中1/阿剩N3）；
    /// 存储属性 private（阿剩低2：getter 公开可绕过锁，外部一律走加锁 getter）。
    /// 初始空集（阿剩中A：类型首次访问同步扫描 /Applications 30-100ms 会阻塞主线程
    /// 启动路径；改为启动后显式后台刷新一次）
    private static let installedLock = NSLock()
    private static var cachedInstalledCLIs: Set<String> = []

    private static func scanInstalledCLIs() -> Set<String> {
        var found = Set<String>()
        var dirs = (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":").map(String.init)
        // 补扫常见非 PATH 安装目录（阿剩低4：~/.local/bin、/opt/homebrew/bin 等
        // 未入 PATH 时 CLI 实际可用但会误判「未安装」）
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        dirs += [home + "/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", home + "/bin"]
        for cli in knownCLIs {
            for dir in dirs where FileManager.default.isExecutableFile(
                atPath: (dir as NSString).appendingPathComponent(cli)) {
                found.insert(cli)
                break
            }
        }
        return found
    }

    public static func installedCLIs() -> Set<String> {
        installedLock.lock()
        defer { installedLock.unlock() }
        return cachedInstalledCLIs
    }

    /// 已安装的 bundle id 集合（/Applications 枚举，小写）
    /// 静态缓存：一次扫描，全部消费方复用；运行期可 refreshInstalledCache() 重扫。
    /// 初始空集（同 cachedInstalledCLIs：避免类型首次访问同步扫描）
    private static var cachedInstalledBundleIDs: Set<String> = []

    private static func scanInstalledBundleIDs() -> Set<String> {
        var found = Set<String>()
        let fm = FileManager.default
        let appsDir = URL(fileURLWithPath: "/Applications")
        if let apps = try? fm.contentsOfDirectory(at: appsDir, includingPropertiesForKeys: [.isDirectoryKey]) {
            for app in apps where app.pathExtension == "app" {
                let plistPath = app.appendingPathComponent("Contents/Info.plist").path
                if let data = fm.contents(atPath: plistPath),
                   let plist = try? PropertyListSerialization.propertyList(from: data, options: [], format: nil) as? [String: Any],
                   let bid = plist["CFBundleIdentifier"] as? String {
                    found.insert(bid.lowercased())
                }
            }
        }
        return found
    }

    public static func installedBundleIDs() -> Set<String> {
        installedLock.lock()
        defer { installedLock.unlock() }
        return cachedInstalledBundleIDs
    }

    /// 运行期重扫安装缓存（面板展开/设置打开时调用，成本毫秒级）。
    /// 扫描本身可在任意线程执行；结果写回加锁（阿证中1：app 多时 plist 解析
    /// 可达 30-100ms，不应占用主线程）
    public static func refreshInstalledCache() {
        // [TEMP-INSTRUMENT] 启动扫描计数（验证后移除）
        if ProcessInfo.processInfo.environment["AI_SCAN_TRACE"] != nil {
            let line = "\(Date().timeIntervalSince1970)\n"
            let f = "/tmp/ai_scan_trace.txt"
            if let h = FileHandle(forWritingAtPath: f) {
                h.seekToEndOfFile(); h.write(Data(line.utf8)); try? h.close()
            } else {
                try? Data(line.utf8).write(to: URL(fileURLWithPath: f))
            }
        }
        let clis = scanInstalledCLIs()
        let bundles = scanInstalledBundleIDs()
        installedLock.lock()
        cachedInstalledCLIs = clis
        cachedInstalledBundleIDs = bundles
        installedLock.unlock()
    }

    /// 自动发现的额外 CLI profile（不在内置集里的 CLI，如 aider/gemini/windsurf）
    public static func discoverCLIProfiles() -> [AgentProfile] {
        let installed = installedCLIs()
        let existing = Set(builtin.flatMap { $0.processNames.map { $0.lowercased() } })
        var extra: [AgentProfile] = []
        for cli in installed where !existing.contains(cli) {
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
        guard let data = UserDefaults.standard.data(forKey: "customAgents"),
              let list = try? JSONDecoder().decode([AgentProfile].self, from: data) else {
            return []
        }
        return list
    }

    public static func saveCustomProfiles(_ profiles: [AgentProfile]) {
        if let data = try? JSONEncoder().encode(profiles) {
            UserDefaults.standard.set(data, forKey: "customAgents")
        }
    }

    /// 完整注册表：内置 + 自动发现 CLI + 自定义
    public static func fullRegistry() -> [AgentProfile] {
        var list = builtin
        list.append(contentsOf: discoverCLIProfiles())
        list.append(contentsOf: loadCustomProfiles())
        return list
    }

    public static func profile(id: String) -> AgentProfile? {
        fullRegistry().first { $0.id == id }
    }

    // MARK: - 工具

    private static func home(_ path: String) -> String {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(path).path
    }
}
