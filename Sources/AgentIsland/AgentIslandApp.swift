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
            MenuBarPopoverView(controller: AppContext.shared.controller, engine: AppContext.shared.engine)
        } label: {
            MenuBarIconView(engine: AppContext.shared.engine)
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsView(engine: AppContext.shared.engine,
                         controller: AppContext.shared.controller,
                         installedApps: AppContext.shared.installedApps)
        }
    }
}

// MARK: - 菜单栏 Popover 内容视图（Compact Island Popover）

struct MenuBarPopoverView: View {
    @ObservedObject var controller: IslandPanelController
    @ObservedObject var engine: ActivityEngine

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // 顶部状态条
            headerBar

            Divider()

            // 活跃 Agent 微缩列表
            agentQuickSection

            // Token 概览
            if !engine.grandTotal.isEmpty {
                Divider()
                tokenMiniSummary
            }

            Divider()

            // 底部操作栏
            actionBar
        }
        .padding(14)
        .frame(width: 300)
        .background(Theme.canvas)
    }

    // MARK: 顶部状态条
    private var headerBar: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(statusColor)
                .frame(width: 8, height: 8)
                .overlay(
                    Circle()
                        .stroke(statusColor.opacity(0.4), lineWidth: engine.anyWorking ? 3 : 0)
                        .scaleEffect(engine.anyWorking ? 1.4 : 1.0)
                )

            Text(statusTitle)
                .font(Theme.bodyFont(13, weight: .semibold))
                .foregroundColor(Theme.ink)

            Spacer()

            Text("\(engine.visibleSnapshots.count) 在线")
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(Theme.chipFill))
        }
    }

    private var statusTitle: String {
        if engine.anyWorking {
            let count = engine.workingAgents().count
            return "\(count) 个 Agent 工作中"
        } else if engine.visibleSnapshots.isEmpty {
            return "暂无活跃 Agent"
        } else {
            return "全部待机中"
        }
    }

    private var statusColor: Color {
        engine.anyWorking ? Theme.statusWorking : (engine.visibleSnapshots.isEmpty ? Theme.statusOffline : Theme.statusIdle)
    }

    // MARK: 活跃 Agent 概览列表
    private var agentQuickSection: some View {
        VStack(spacing: 4) {
            let working = engine.workingAgents()
            let displayList = working.isEmpty ? Array(engine.visibleSnapshots.prefix(3)) : working
            if displayList.isEmpty {
                HStack {
                    Spacer()
                    Text("无运行中的 Agent")
                        .font(Theme.bodyFont(11))
                        .foregroundColor(Theme.inkMuted48)
                        .padding(.vertical, 8)
                    Spacer()
                }
            } else {
                ForEach(displayList) { s in
                    HStack(spacing: 8) {
                        Image(systemName: s.profile.icon)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundColor(Theme.ink)
                            .frame(width: 22, height: 22)
                            .background(Circle().fill(Theme.tile1))

                        Text(s.profile.name)
                            .font(Theme.bodyFont(12, weight: .medium))
                            .foregroundColor(Theme.ink)
                            .lineLimit(1)

                        Spacer()

                        if let usage = s.tokenUsage, usage.tokens24h > 0 {
                            Text(TokenUsage.compact(usage.tokens24h))
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.inkMuted48)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(Theme.chipFill))
                        }

                        Text(s.level.label)
                            .font(Theme.bodyFont(10, weight: .semibold))
                            .foregroundColor(s.level.color)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Capsule().fill(s.level.color.opacity(0.14)))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 4)
                    .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: .clear)
                    .onTapGesture {
                        controller.route = .agentDetail(s.profile.id)
                        if controller.displayState == .docked {
                            controller.toggle()
                        }
                    }
                }
            }
        }
    }

    // MARK: Token 概览
    private var tokenMiniSummary: some View {
        HStack(spacing: 6) {
            Image(systemName: "chart.bar.fill")
                .font(.system(size: 9))
                .foregroundColor(Theme.inkMuted48)
            Text("24h \(TokenUsage.compact(engine.grandTotal.tokens24h))")
                .font(Theme.monoFont(10, weight: .medium))
                .foregroundColor(Theme.inkMuted80)
            if !TokenUsage.cost(engine.grandTotal.cost24h).isEmpty {
                Text(TokenUsage.cost(engine.grandTotal.cost24h))
                    .font(Theme.monoFont(9))
                    .foregroundColor(Theme.inkMuted48)
            }
            Spacer()
            Text("累计 \(TokenUsage.compact(engine.grandTotal.tokensTotal))")
                .font(Theme.monoFont(10))
                .foregroundColor(Theme.inkMuted48)
        }
        .padding(.horizontal, 4)
    }

    // MARK: 底部操作栏
    private var actionBar: some View {
        HStack(spacing: 8) {
            // 展开/收起侧边栏
            Button {
                controller.toggle()
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "sidebar.right")
                        .font(.system(size: 11))
                    Text(controller.displayState == .expanded ? "收起侧边栏" : "展开侧边栏")
                        .font(Theme.bodyFont(11, weight: .medium))
                }
                .foregroundColor(Theme.ink)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .frame(maxWidth: .infinity)
                .background(RoundedRectangle(cornerRadius: 6).fill(Theme.chipFill))
            }
            .buttonStyle(.plain)

            // 设置
            Button {
                NSApp.activate(ignoringOtherApps: true)
                NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
            } label: {
                Image(systemName: "gearshape")
                    .font(.system(size: 11))
                    .foregroundColor(Theme.inkMuted80)
                    .padding(6)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Theme.chipFill))
            }
            .buttonStyle(.plain)
            .help("设置…")

            // 退出
            Button {
                NSApp.terminate(nil)
            } label: {
                Image(systemName: "power")
                    .font(.system(size: 11))
                    .foregroundColor(Theme.dangerRed)
                    .padding(6)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Theme.chipFill))
            }
            .buttonStyle(.plain)
            .help("退出 AgentIsland")
        }
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
