import AgentIslandCore
import AppKit
import Foundation

// MARK: - URL Scheme 深度链接路由器 (v0.0.75)

/// 解析并分发 `agentisland://` 协议指令，支持与 Raycast、Alfred、快捷指令或终端联动。
/// 常见链接示例：
/// - agentisland://toggle             (展开/收起切换)
/// - agentisland://expand             (展开主面板)
/// - agentisland://collapse           (收起至灵动岛停靠态)
/// - agentisland://agent?id=claude    (直达某智能体详情页)
/// - agentisland://analytics          (打开 Token 用量分析)
/// - agentisland://toolbox            (打开维护工作台)
/// - agentisland://clean              (一键静默清理孤儿与异常进程)
/// - agentisland://export             (导出审计报告 Markdown 至剪贴板)
@MainActor
public enum URLSchemeRouter {

    public enum Action: Equatable {
        case toggle
        case expand
        case collapse
        case agent(id: String)
        case analytics
        case toolbox
        case clean
        case export
        case notify(agentId: String, type: String, message: String?, detail: String?)
        case unknown(String)
    }

    /// 解析 URL 得到语义化 Action
    public static func parse(url: URL) -> Action? {
        guard let scheme = url.scheme?.lowercased(), scheme == "agentisland" else {
            return nil
        }

        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }

        // 命令名可能位于 host (如 agentisland://toggle) 或 path (如 agentisland:///toggle)
        let rawCommand: String
        if let host = components.host, !host.isEmpty {
            rawCommand = host.lowercased()
        } else {
            let trimmedPath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            rawCommand = trimmedPath.components(separatedBy: "/").first?.lowercased() ?? ""
        }

        let queryItems = components.queryItems ?? []
        let queryDict = Dictionary(queryItems.compactMap { item in
            item.value.map { (item.name.lowercased(), $0) }
        }, uniquingKeysWith: { first, _ in first })

        switch rawCommand {
        case "toggle":
            return .toggle
        case "expand", "open":
            if let agentId = queryDict["id"] ?? queryDict["agent"] {
                return .agent(id: agentId)
            }
            return .expand
        case "collapse", "close", "hide":
            return .collapse
        case "agent", "detail":
            var agentId = queryDict["id"] ?? queryDict["agent"]
            if agentId == nil {
                let pathParts = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).components(separatedBy: "/")
                if pathParts.count > 1 {
                    agentId = pathParts[1]
                } else if pathParts.count == 1 && !pathParts[0].isEmpty && pathParts[0] != rawCommand {
                    agentId = pathParts[0]
                }
            }
            if let id = agentId, !id.isEmpty {
                return .agent(id: id)
            }
            return .unknown(rawCommand)
        case "analytics", "tokens", "cost":
            return .analytics
        case "toolbox", "workbench", "cleaner":
            return .toolbox
        case "clean", "kill-orphans":
            return .clean
        case "export", "report":
            return .export
        case "notify", "event", "alert":
            let agentId = queryDict["agent"] ?? queryDict["id"] ?? "system"
            let type = queryDict["type"] ?? queryDict["event"] ?? "completed"
            let msg = queryDict["message"] ?? queryDict["msg"] ?? queryDict["title"]
            let detail = queryDict["detail"]
            return .notify(agentId: agentId, type: type, message: msg, detail: detail)
        default:
            return .unknown(rawCommand)
        }
    }

    /// 执行 URL 指令分发
    @discardableResult
    static func handle(url: URL, controller: IslandPanelController, engine: ActivityEngine) -> Bool {
        guard let action = parse(url: url) else { return false }

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

        case .clean:
            controller.expand()
            controller.navigateToToolbox()

        case .export:
            let md = AuditReportExporter.generateMarkdown(
                snapshots: engine.snapshots,
                history: engine.eventHistory,
                now: Date()
            )
            let pb = NSPasteboard.general
            pb.clearContents()
            pb.setString(md, forType: .string)

        case .notify(let agentId, let type, let message, let detail):
            let eventType: AgentTaskEvent.EventType
            switch type.lowercased() {
            case "attention", "wait", "confirm":
                eventType = .attention
            case "costspike", "cost", "budget", "alert":
                eventType = .costSpike
            default:
                eventType = .completed
            }
            let event = AgentTaskEvent(
                agentId: agentId,
                agentName: agentId,
                eventType: eventType,
                duration: 0,
                timestamp: Date(),
                pid: nil,
                message: message ?? "\(agentId) 任务通知",
                detail: detail
            )
            engine.postEvent(event)

        case .unknown:
            return false
        }

        return true
    }
}
