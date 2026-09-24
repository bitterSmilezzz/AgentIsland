import AgentIslandCore
import AppKit
import Foundation

// MARK: - URL Scheme 深度链接路由器 (v0.0.75)

/// 分发 `agentisland://` 协议指令，支持与 Raycast、Alfred、快捷指令或终端联动。
/// 纯解析部分在 `AgentIslandCore.URLSchemeParser`（可测）；这里只做「解析出来做什么」。
/// 常见链接示例：
/// - agentisland://toggle             (展开/收起切换)
/// - agentisland://expand             (展开主面板)
/// - agentisland://collapse           (收起至灵动岛停靠态)
/// - agentisland://agent?id=claude    (直达某智能体详情页)
/// - agentisland://analytics          (打开 Token 用量分析)
/// - agentisland://toolbox            (打开维护工作台)
/// - agentisland://clean              (跳到工作台的清理区——不直接杀进程)
/// - agentisland://export             (跳到工作台；深链不再代用户写剪贴板)
@MainActor
public enum URLSchemeRouter {

    /// 执行 URL 指令分发
    @discardableResult
    static func handle(url: URL, controller: IslandPanelController, engine: ActivityEngine) -> Bool {
        guard let action = URLSchemeParser.parse(url: url) else { return false }

        switch action {
        case .toggle:
            controller.toggle()

        case .expand:
            controller.expand()

        case .collapse:
            controller.collapse()

        case .agent(let id):
            controller.expand()
            controller.openAgentDetail(id)

        case .analytics:
            controller.expand()
            controller.navigateToTokenAnalytics()

        case .toolbox:
            controller.expand()
            controller.navigateToToolbox()

        case .settings(let tab):
            // 设置窗口是独立 scene，不经过岛内路由；岛保持当前形态
            SettingsOpener.open(tab: tab)

        case .clean:
            controller.expand()
            controller.navigateToToolbox()

        case .export:
            // 深链不再直接写剪贴板：`open agentisland://export` 无需任何确认，
            // 而 copyToPasteboard 会 clearContents——一个 npm postinstall 或 .command
            // 脚本就能静默销毁用户正准备粘贴的密码/命令。自动化请走显式的
            // `agentisland report --copy`（那是用户自己敲的）
            controller.expand()
            controller.navigateToToolbox()

        case .notify(let rawAgentId, let type, let message, let detail):
            // 投递目标必须能解析到已知档案：此前 `agent=..%2Fwhatever` 这种
            // 任意字符串会直接成为事件身份，岛内出现一条不存在的 Agent 的告警
            let needle = rawAgentId.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard let profile = engine.allProfiles.first(where: {
                $0.id.lowercased() == needle || $0.name.lowercased() == needle
            }) else {
                AppLog.warn("URLScheme: 拒绝投递给未知智能体 \(rawAgentId.prefix(60))")
                return false
            }
            let agentId = profile.id
            // 与 `POST /notify` 共用同一张表：此前两边各写一份，今天抄得一样，
            // 但下一格只改一边就是「同一个 type 在深链里红着叫、在 curl 里绿着收」
            let eventType = AgentTaskEvent.externalType(from: type)
            let event = AgentTaskEvent(
                agentId: agentId,
                agentName: profile.name,
                eventType: eventType,
                duration: 0,
                timestamp: Date(),
                pid: nil,
                message: message ?? "\(profile.name) 任务通知",
                detail: detail,
                // 来源标记：横幅与系统通知会带「外部投递」前缀，
                // 外部事件因此无法与引擎自己判定的「等待你确认」逐字节同形
                externallyDelivered: true
            )
            engine.postEvent(event)

        case .unknown:
            return false
        }

        return true
    }
}
