import AgentIslandCore
import SwiftUI
import AppKit

// MARK: - 共享应用上下文（引擎单例，App 与 AppDelegate 共用同一实例）

@MainActor
final class AppContext {
    static let shared = AppContext()
    let engine: ActivityEngine
    /// 已安装缓存（组合根创建；刷新节律由引擎驱动，设置页只读 + 打开时触发重扫）
    let installedApps = InstalledAppsCache()
    private var _controller: IslandPanelController?

    /// 控制器延迟创建：首次访问才实例化，全局唯一
    var controller: IslandPanelController {
        if let c = _controller { return c }
        let c = IslandPanelController(engine: engine)
        _controller = c
        return c
    }

    /// 启停集读取：无记录（nil）回退 defaultEnabled 集；空数组是主动全关，照常生效
    private static func enabledOrDefault(registry: [AgentProfile]) -> Set<String> {
        EnabledAgentStore.load() ?? Set(registry.filter(\.defaultEnabled).map(\.id))
    }

    private init() {
        // 配置：Core 唯一读取路径（缺项回落默认 + 归一化启动自愈脏值）
        let config = EngineConfig.load(from: .standard)
        let registry = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs())
        let enabled = Self.enabledOrDefault(registry: registry)
        engine = ActivityEngine(
            profiles: registry.filter { enabled.contains($0.id) },
            config: config,
            installedApps: installedApps,
            enabledIDs: enabled
        )
        // 安装缓存首刷由引擎 init 自排（唯一刷新驱动）；首刷完成回调里引擎自行重放
        // 启用集恢复自动发现监控——组合根不再编排「刷新+打标+二次 setEnabled」舞步
    }
}

// MARK: - AgentIsland 入口
// 菜单栏常驻 App（LSUIElement）：MenuBarExtra + 设置窗口 + 灵动岛面板

@main
struct AgentIslandApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    init() {
        // 无头验证模式：--probe 打印状态表后退出
        if CommandLine.arguments.contains("--probe") {
            exit(Probe.run())
        }
        // 无头自检模式：进程内断言
        if CommandLine.arguments.contains("--selftest") {
            exit(Selftest.run())
        }
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarMenuView(controller: AppContext.shared.controller, engine: AppContext.shared.engine)
        } label: {
            MenuBarIconView(engine: AppContext.shared.engine)
        }
        .menuBarExtraStyle(.menu)

        Settings {
            SettingsView(engine: AppContext.shared.engine,
                         controller: AppContext.shared.controller,
                         installedApps: AppContext.shared.installedApps)
        }
    }
}

// MARK: - 菜单内容视图

struct MenuBarMenuView: View {
    @ObservedObject var controller: IslandPanelController
    @ObservedObject var engine: ActivityEngine

    var body: some View {
        // 可发现性提示（M2）：两级交互入口语义分离，首行说明
        Text("提示：鼠标碰屏幕顶部滑出卡片")
            .font(Theme.bodyFont(11))
            .foregroundColor(Theme.inkMuted48)
            .fixedSize(horizontal: false, vertical: true)

        Divider()

        // 状态摘要（M1）：忙碌时带工作数；全离线时菜单仍可展开（空态有提示价值）
        // 口径与展开卡片一致：统一走 engine.visibleSnapshots（阿剩低2/阿菜低2）
        let workingCount = engine.workingAgents().count
        let anyOfflineOnly = engine.visibleSnapshots.isEmpty
        Button(controller.displayState == .expanded
               ? "收起灵动岛"
               : (anyOfflineOnly ? "展开灵动岛（暂无 Agent 在线）" : "展开灵动岛\(workingCount > 0 ? "（\(workingCount) 个工作中）" : "")")) {
            controller.toggle()
        }

        Divider()

        // 打开 Settings 场景：@State 布尔不会触发窗口，必须发系统动作（macOS 13 兼容）
        Button("设置…") {
            NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
        }
        Button("重置岛位置") {
            AppContext.shared.controller.resetPosition()
        }
        Button("退出") { NSApp.terminate(nil) }
    }
}

// MARK: - 菜单栏图标（Q10：working 状态色 + 圆点角标）

struct MenuBarIconView: View {
    @ObservedObject var engine: ActivityEngine

    var body: some View {
        Image(systemName: engine.anyWorking ? "dot.radiowaves.left.and.right" : "sparkles")
            .symbolRenderingMode(.hierarchical)
            .foregroundStyle(engine.anyWorking ? Theme.statusWorking : Theme.inkMuted48)
            // H2：状态切换淡入过渡（macOS 13 无 symbolEffect，用内容过渡替代）
            .contentTransition(.opacity)
            .animation(.easeInOut(duration: 0.25), value: engine.anyWorking)
            .overlay(alignment: .topTrailing) {
                if engine.anyWorking {
                    // H1：角标用 alignment+padding 完全收进图标内圈（不用 offset，避免越出被裁）
                    // 随图标同节奏淡入淡出
                    Circle()
                        .fill(Theme.statusWorking)
                        .frame(width: 4, height: 4)
                        .padding(1)
                        .transition(.opacity)
                }
            }
    }
}

// MARK: - AppDelegate（创建灵动岛面板）

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory) // 无 Dock 图标

        let context = AppContext.shared
        let controller = context.controller // 触发延迟创建
        context.engine.start()
        controller.show()
    }

    func applicationWillTerminate(_ notification: Notification) {
        AppContext.shared.engine.stop()
    }
}
