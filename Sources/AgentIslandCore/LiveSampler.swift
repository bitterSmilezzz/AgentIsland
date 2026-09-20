import Foundation

// MARK: - 一次性实况采样
//
// CLI 命令（status / report / check / doctor）与 .app 的 --probe 都需要「不启动 UI，就地看一次真实状态」。
// 这件事有几个容易做错的细节，过去分散在三处各自实现并已经分叉：
// · 档案集要用 fullRegistry（内置 ∪ 自动发现 ∪ 自定义）。`status` 此前只遍历内置集，
//   用户自定义与自动发现的 Agent 在 `status` / `status --json` 里根本不存在，而 `report` 能看到
// · 安装缓存必须先热起来，否则调用方与引擎 init 各扫一遍 PATH
// · 文件监控要用真实实现并预热一次，否则首拍没有任何文件信号
// · CPU% 是差分量：第一拍所有 PID 都是「首次见到」→ 恒为 0，要拿真实利用率必须双采
@MainActor
public enum LiveSampler {

    /// 一次性工具的共享上下文：热好的安装缓存 + 完整档案集 + 解析后的启停集
    public struct Context {
        public let installedApps: InstalledAppsCache
        public let registry: [AgentProfile]
        public let enabledIDs: Set<String>
    }

    /// 扫描安装集并解析档案与启停（同步扫 PATH/Applications，一次性工具可接受）
    public static func context(defaults: UserDefaults = .standard) -> Context {
        let installedApps = InstalledAppsCache()
        installedApps.refresh()
        let registry = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs(),
                                                  installedBundles: installedApps.installedBundleIDs(),
                                                  defaults: defaults)
        // 启停集与组合根同一套解析（含存档损坏时只读降级，绝不覆写用户选择）
        return Context(installedApps: installedApps,
                       registry: registry,
                       enabledIDs: EnabledAgentStore.resolvedEnabled(registry: registry, defaults: defaults))
    }

    /// 构造一次性实况引擎。调用方自己决定采几拍。
    /// - Parameter restrictToEnabled: `true`（默认）与灵动岛同口径——只跑用户启用的档案，
    ///   否则会把 PATH 上每个已知 CLI 都列成一行（实测 10 行涨到 27 行）；
    ///   `false` 留给排障场景（`--all`、审计报告、`--probe`）。
    public static func makeEngine(config: EngineConfig = EngineConfig(),
                                  defaults: UserDefaults = .standard,
                                  restrictToEnabled: Bool = true) -> ActivityEngine {
        makeEngine(from: context(defaults: defaults), config: config, restrictToEnabled: restrictToEnabled)
    }

    /// 已有上下文时复用之（避免同一进程里重复扫描安装集）
    public static func makeEngine(from ctx: Context,
                                  config: EngineConfig = EngineConfig(),
                                  restrictToEnabled: Bool = true) -> ActivityEngine {
        let profiles = restrictToEnabled ? ctx.registry.filter { ctx.enabledIDs.contains($0.id) } : ctx.registry
        let monitor = FileActivityMonitor()
        monitor.watch(dirs: profiles.flatMap(\.sessionDirs))
        monitor.scanSync()

        return ActivityEngine(
            profiles: profiles,
            config: config,
            processMonitor: ProcessProvider(),
            fileMonitor: monitor,
            installedApps: ctx.installedApps,      // 已热缓存，init 首刷跳过
            enabledIDs: ctx.enabledIDs
        )
    }

    /// 在已构造的引擎上走「基线拍 → interval → 实况拍」，返回第二拍快照。
    /// 拆出来是为了让调用方能在两拍之间打印自己的进度提示，双采语义仍只定义一处。
    /// 会阻塞调用线程 `interval` —— 只给一次性诊断入口用（doctor / --probe）。
    public static func twoBeatSample(_ engine: ActivityEngine, interval: TimeInterval = 1.5) -> [AgentSnapshot] {
        engine.sample()                                   // 建立 CPU 差分基线
        Thread.sleep(forTimeInterval: interval)
        return engine.sample()                            // 这一拍才是真实窗口利用率
    }

    /// 双采实况（阻塞 `interval`）。脚本友好的 status / report 走 `makeEngine()` + 单拍。
    public static func sample(config: EngineConfig = EngineConfig(),
                              interval: TimeInterval = 1.5) -> [AgentSnapshot] {
        twoBeatSample(makeEngine(config: config), interval: interval)
    }
}
