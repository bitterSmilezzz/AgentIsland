import Foundation

// MARK: - agentisland:// 深链解析（v0.0.97 自 UI 侧 URLSchemeRouter 抽出）

/// 深链是**外部输入面**：任何本地进程（npm postinstall、`.command`、cron、快捷指令、
/// 网页里的 `window.open`）都能构造一条 `agentisland://...`，且不需要权限提示。
/// 解析原先待在 `@MainActor` 的 UI 目标里，本仓的测试 runner 完全够不着——
/// 这个入口在 v0.0.96 之前是零覆盖的。挪进 Core 只搬「怎么解析」，
/// 「解析出来做什么」（含身份校验与来源标记）仍留在 UI 侧路由器。
public enum URLSchemeCommand: Equatable {
    case toggle
    case expand
    case collapse
    case agent(id: String)
    case analytics
    case toolbox
    case settings(tab: String?)
    case clean
    case export
    case notify(agentId: String, type: String, message: String?, detail: String?)
    case unknown(String)
}

public enum URLSchemeParser {
    /// 解析 URL 得到语义化指令。scheme 不是 `agentisland` 时返回 nil（调用方据此什么都不做）。
    /// 命令名可在 host（`agentisland://toggle`）也可在 path（`agentisland:///toggle`）；
    /// 查询键大小写不敏感，重复键 first-wins。
    public static func parse(url: URL) -> URLSchemeCommand? {
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
        case "settings", "config", "preferences":
            // `?tab=` 与既有的 analytics/toolbox 同一思路：深链可以直接落到某个界面，
            // 而不必先开窗口再让人手动点。也是自动化脚本与 UI 自检的入口
            return .settings(tab: queryDict["tab"] ?? queryDict["pane"])
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
}
