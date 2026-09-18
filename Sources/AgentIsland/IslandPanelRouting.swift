import AgentIslandCore
import SwiftUI

// MARK: - 灵动岛卡内导航与路由控制器

extension IslandPanelController {

    /// 打开 Agent 详情页并记录来源（返回时据此还原）
    func openAgentDetail(_ agentId: String, from origin: CardRoute? = nil) {
        if case .agentDetail = route {} else {
            agentDetailOrigin = origin ?? route
        }
        route = .agentDetail(agentId)
    }

    /// 从 Agent 详情页返回：回到进入前的页面（主列表 or Token 统计图等）
    func closeAgentDetail() {
        route = agentDetailOrigin
    }

    /// 打开实时流水页并记录来源（返回时据此还原）
    func openLiveStream(agentId: String) {
        if case .liveStream = route {} else { liveStreamOrigin = route }
        route = .liveStream(agentId)
    }

    /// 从实时流水页返回：回到进入前的页面
    func closeLiveStream() {
        route = liveStreamOrigin
    }

    /// 导航至 Token 时间趋势与用量图表
    func navigateToTokenAnalytics() {
        route = .tokenAnalytics
    }

    /// 导航至异常进程与死锁快捷工具箱
    func navigateToToolbox() {
        route = .toolbox
    }

    /// 导航回主列表
    func navigateToList() {
        route = .list
    }

    /// 逐级后退导航（Esc 快捷键调用）
    func stepBackRoute() {
        switch route {
        case .agentDetail:
            closeAgentDetail()
        case .liveStream:
            closeLiveStream()
        case .sessions(let agentId, _):
            openAgentDetail(agentId)
        case .tokenAnalytics, .toolbox:
            navigateToList()
        default:
            navigateToList()
        }
    }

    /// 键盘上下方向键选择列表中的 Agent
    func moveFocus(step: Int) {
        let list = engine.visibleSnapshots
        guard !list.isEmpty else {
            focusedAgentId = nil
            return
        }

        if let cur = focusedAgentId, let idx = list.firstIndex(where: { $0.id == cur }) {
            let nextIdx = min(max(0, idx + step), list.count - 1)
            focusedAgentId = list[nextIdx].id
        } else {
            focusedAgentId = (step >= 0) ? list.first?.id : list.last?.id
        }
    }
}
