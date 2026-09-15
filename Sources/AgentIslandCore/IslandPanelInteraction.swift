import Foundation

/// 面板交互的纯逻辑边界。
///
/// 保持在 Core 中使「浮层命中」和「四边吸附」可脱离 AppKit/SwiftUI 回归测试；
/// UI 层只负责把窗口、屏幕和鼠标状态转换为这些输入。
public enum IslandPanelInteraction {
    /// 浮层类名白名单。只有真正承载岛内交互内容的 SwiftUI popover 与菜单栏
    /// popover 可以阻止收起；状态栏本体的 frame 可能比可点击图标大得多，不能
    /// 用作“鼠标仍在岛上”的证据。
    public static func isKnownFloatingLayer(className: String) -> Bool {
        className.contains("Popover")
            || className.contains("MenuBarExtraWindow")
    }

    /// 仅当浮层可见、属于岛的派生窗口且鼠标实际位于其 frame 内，才应阻止收起。
    public static func isMouseInsideFloatingLayer(
        className: String,
        isVisible: Bool,
        containsMouse: Bool
    ) -> Bool {
        isVisible && containsMouse && isKnownFloatingLayer(className: className)
    }

    /// 按面板外框到可用屏幕四边的最短距离选择停靠边。数组顺序同时定义距离相等
    /// 时的稳定取舍，避免松手时在相邻边之间随机跳动。
    public static func nearestDockEdge(panelFrame: CGRect, visibleFrame: CGRect) -> DockEdge {
        let candidates: [(DockEdge, CGFloat)] = [
            (.top, abs(visibleFrame.maxY - panelFrame.maxY)),
            (.right, abs(visibleFrame.maxX - panelFrame.maxX)),
            (.bottom, abs(panelFrame.minY - visibleFrame.minY)),
            (.left, abs(panelFrame.minX - visibleFrame.minX)),
        ]
        return candidates.min { $0.1 < $1.1 }!.0
    }
}
