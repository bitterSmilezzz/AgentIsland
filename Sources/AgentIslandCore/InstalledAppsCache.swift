import Foundation

// MARK: - 已安装 CLI / GUI bundle 缓存
// 实例化注入（取代 AgentRegistry 全局静态缓存）：调用方无时序约定——
// 以扫描完成时间计算缓存年龄；在途请求合并扫描并共享完成通知。
// 扫描器闭包注入：生产默认真实扫描，测试传 canned 闭包零文件系统。

public final class InstalledAppsCache: @unchecked Sendable {

    public typealias CLIScanner = () -> Set<String>
    public typealias BundleScanner = () -> Set<String>

    private let lock = NSCondition()
    private var clis: Set<String> = []      // 小写命令名
    private var bundles: Set<String> = []   // 小写 bundle id
    private var lastRefreshAt: Date?
    private var refreshing = false
    private var completions: [@MainActor () -> Void] = []

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

    /// 冷缓存首刷；在途时加入完成通知，已热且没有在途扫描时跳过且不回调。
    /// 返回 true 表示安排了新扫描，false 表示合并或命中缓存。
    @discardableResult
    public func warmUp(completion: (@MainActor () -> Void)? = nil) -> Bool {
        scheduleRefresh(maxAge: nil, completion: completion)
    }

    /// 过期时后台重扫；在途请求（包括 maxAge=0）共享扫描与主线程完成通知。
    /// 缓存仍新鲜且没有在途扫描时，返回 false 且不回调。
    @discardableResult
    public func refreshIfNeeded(maxAge: TimeInterval, completion: (@MainActor () -> Void)? = nil) -> Bool {
        scheduleRefresh(maxAge: maxAge, completion: completion)
    }

    private func scheduleRefresh(maxAge: TimeInterval?, completion: (@MainActor () -> Void)?) -> Bool {
        lock.lock()
        if refreshing {
            if let completion { completions.append(completion) }
            lock.unlock()
            return false
        }
        if let last = lastRefreshAt,
           maxAge.map({ Date().timeIntervalSince(last) < $0 }) ?? true {
            lock.unlock()
            return false
        }
        refreshing = true
        if let completion { completions.append(completion) }
        lock.unlock()
        DispatchQueue.global(qos: .utility).async { [self] in
            performRefresh()
        }
        return true
    }

    /// 同步刷新（测试 / --probe）；已有扫描时等待其结果，避免扫描器并发运行。
    /// UI 路径使用异步入口。扫描器不得递归调用刷新入口。
    public func refresh() {
        lock.lock()
        if refreshing {
            while refreshing { lock.wait() }
            lock.unlock()
            return
        }
        refreshing = true
        lock.unlock()
        performRefresh()
    }

    private func performRefresh() {
        let foundCLIs = scanCLIs()
        let foundBundles = scanBundles()
        lock.lock()
        clis = foundCLIs
        bundles = foundBundles
        lastRefreshAt = Date()
        refreshing = false
        let callbacks = completions
        completions.removeAll()
        lock.broadcast()
        lock.unlock()
        if !callbacks.isEmpty {
            Task { @MainActor in
                for callback in callbacks { callback() }
            }
        }
    }

    // MARK: 真实扫描（生产默认）

    /// 已知 CLI 名（PATH 扫描用）
    private static let knownCLIs = ["dim", "codex", "claude", "cursor", "trae", "opencode",
                            "hermes-agent", "aider", "gemini", "windsurf", "agent-browser",
                            "tiny-agents", "continue", "zcode", "zcode-cli", "antigravity", "agy",
                            "dsh", "ego-browser", "vibe-usage", "openviking", "ov", "vikingbot",
                            "bsk", "cua-driver"]

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
