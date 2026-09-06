import Foundation

// MARK: - 已安装 CLI / GUI bundle 缓存
// 实例化注入（取代 AgentRegistry 全局静态缓存）：调用方无时序约定——
// 「冷启动必扫、窗口内跳过」由 refreshIfNeeded 调度即标记保证，双扫在结构上不可能。
// 扫描器闭包注入：生产默认真实扫描，测试传 canned 闭包零文件系统。

public final class InstalledAppsCache: @unchecked Sendable {

    public typealias CLIScanner = () -> Set<String>
    public typealias BundleScanner = () -> Set<String>

    private let lock = NSLock()
    private var clis: Set<String> = []      // 小写命令名
    private var bundles: Set<String> = []   // 小写 bundle id
    private var lastRefreshAt: Date?

    private let scanCLIs: CLIScanner
    private let scanBundles: BundleScanner

    /// 生产默认：PATH + 常见非 PATH 安装目录 + /Applications
    public init(scanCLIs: @escaping CLIScanner = InstalledAppsCache.defaultCLIScanner,
                scanBundles: @escaping BundleScanner = InstalledAppsCache.defaultBundleScanner) {
        self.scanCLIs = scanCLIs
        self.scanBundles = scanBundles
    }

    // MARK: 读取

    public func installedCLIs() -> Set<String> {
        lock.lock()
        defer { lock.unlock() }
        return clis
    }

    public func installedBundleIDs() -> Set<String> {
        lock.lock()
        defer { lock.unlock() }
        return bundles
    }

    /// 档案是否已安装（bundle id 或 CLI 命中任一）
    public func isInstalled(_ profile: AgentProfile) -> Bool {
        let installedCLIs = self.installedCLIs()
        let installedBundles = self.installedBundleIDs()
        if profile.bundleIDs.contains(where: { installedBundles.contains($0.lowercased()) }) {
            return true
        }
        return profile.processNames.contains(where: { installedCLIs.contains($0.lowercased()) })
    }

    // MARK: 刷新

    /// 缓存是否已完成过至少一次刷新（含测试注入的预热）
    public var isWarmed: Bool {
        lock.lock()
        defer { lock.unlock() }
        return lastRefreshAt != nil
    }

    /// 冷缓存才首刷（调度即标记，后台执行）；已热缓存幂等跳过。
    /// 给「调用方可能已预热、引擎也要保证冷启动有首刷」的两方场景用——
    /// 双方都调 warmUp 也只扫一次。completion 主线程回调（跳过时不回调）。
    @discardableResult
    public func warmUp(completion: (@MainActor () -> Void)? = nil) -> Bool {
        if isWarmed { return false }
        return refreshIfNeeded(maxAge: 0, completion: completion)
    }

    /// 距上次刷新超过 maxAge 则后台重扫，完成后主线程回调 completion。
    /// 「调度即标记」：返回 true 表示本次已安排扫描，false 表示仍在窗口内跳过——
    /// refreshIfNeeded/warmUp 调用方之间双扫在结构上不可能（与强制 refresh() 混用除外）。
    @discardableResult
    public func refreshIfNeeded(maxAge: TimeInterval, completion: (@MainActor () -> Void)? = nil) -> Bool {
        lock.lock()
        if let last = lastRefreshAt, Date().timeIntervalSince(last) < maxAge {
            lock.unlock()
            return false
        }
        lastRefreshAt = Date()
        lock.unlock()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            self?.refresh()
            if let completion {
                Task { @MainActor in completion() }
            }
        }
        return true
    }

    /// 立即同步扫描并写回（测试 / --probe 用；UI 路径一律走 refreshIfNeeded）
    public func refresh() {
        let foundCLIs = scanCLIs()
        let foundBundles = scanBundles()
        lock.lock()
        clis = foundCLIs
        bundles = foundBundles
        lastRefreshAt = Date()
        lock.unlock()
    }

    // MARK: 真实扫描（生产默认）

    /// 已知 CLI 名（PATH 扫描用）
    private static let knownCLIs = ["dim", "codex", "claude", "cursor", "trae", "opencode",
                            "hermes-agent", "aider", "gemini", "windsurf", "agent-browser",
                            "tiny-agents", "continue"]

    /// 已知 GUI bundle id（/Applications 扫描用）
    private static let knownBundleIDs: [String: String] = [
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

    /// PATH 扫描 + 补扫常见非 PATH 安装目录（~/.local/bin 等未入 PATH 时 CLI 实际可用）
    public static func defaultCLIScanner() -> Set<String> {
        var found = Set<String>()
        var dirs = (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":").map(String.init)
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

    /// /Applications 枚举 + Info.plist 解析（app 多时可达 30-100ms，须后台执行）
    public static func defaultBundleScanner() -> Set<String> {
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
}
