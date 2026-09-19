import AgentIslandCore
import AppKit
import SwiftUI

// MARK: - 灵动岛窗口几何计算、吸附与屏幕适配

extension IslandPanelController {

    // MARK: - 拖动与智能吸附

    func beginDrag() {
        guard panel != nil, displayState == .expanded else { return }
        isDragging = true
        cancelPendingTasks()
        // 残留 grace 复位（R32/F8）：peek 任务体已把 grace 设为 now+peekDuration，
        // 取消 peek 后不清会让拖拽结束后的自动收起被压制最长 6s
        manualOpenGraceUntil = .distantPast
    }

    func dragEnded() {
        guard let panel, isDragging else { return }
        isDragging = false
        dragCooldownUntil = Date().addingTimeInterval(0.6)

        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        dockEdge = IslandPanelInteraction.nearestDockEdge(panelFrame: panel.frame, visibleFrame: visible)

        if dockEdge.isHorizontal {
            savedTopX = min(max(panel.frame.midX, visible.minX + 70), visible.maxX - 70)
            UserDefaults.standard.set(Double(savedTopX!), forKey: SettingKey.dockAnchorX)
        } else {
            savedRightY = min(max(panel.frame.midY, visible.minY + 60), visible.maxY - 60)
            UserDefaults.standard.set(Double(savedRightY!), forKey: SettingKey.dockAnchorY)
        }
        UserDefaults.standard.set(dockEdge.rawValue, forKey: SettingKey.dockEdge)

        updateChrome()
        SoundEffectsManager.performHapticClick()
        if displayState == .expanded {
            snapToDockEdge(animated: true)
        } else {
            placeWindow(animated: true)
        }
    }

    func snapToDockEdge(animated: Bool) {
        guard displayState == .expanded else { return }
        let modeRaw = UserDefaults.standard.string(forKey: SettingKey.screenFollowMode) ?? ScreenFollowMode.followMouse.rawValue
        let followMode = ScreenFollowMode(rawValue: modeRaw) ?? .followMouse
        let screen = Self.targetScreen(mode: followMode, currentPanelScreen: panel.screen)
        let targetRect = dockTargetFrame(for: screen)
        let targetOrigin = targetRect.origin
        let dx = targetOrigin.x - panel.frame.origin.x
        let dy = targetOrigin.y - panel.frame.origin.y
        let dist = hypot(dx, dy)

        guard animated, dist >= 1.5 else {
            panel.setFrame(targetRect, display: false)
            return
        }

        let duration: TimeInterval = min(0.35, max(0.18, Double(sqrt(dist / 800.0)) * 0.28))
        let timing = CAMediaTimingFunction(controlPoints: 0.22, 1.0, 0.36, 1.0)

        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = duration
            ctx.timingFunction = timing
            panel.animator().setFrame(targetRect, display: false)
        }
    }

    static func screenContainingMouse() -> NSScreen? {
        let loc = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(loc) } ?? NSScreen.main
    }

    /// 校验屏幕对象是否仍属于当前连接的有效屏幕集合 (防止拔掉显示器后悬空)
    static func isValidScreen(_ screen: NSScreen?) -> Bool {
        guard let screen else { return false }
        return NSScreen.screens.contains { $0 == screen || $0.frame.equalTo(screen.frame) }
    }

    /// 根据用户偏好解析目标屏幕 (v0.0.74, v0.0.77 断连自愈增强)
    static func targetScreen(mode: ScreenFollowMode, currentPanelScreen: NSScreen?) -> NSScreen {
        let currentScreens = NSScreen.screens
        guard !currentScreens.isEmpty else { return NSScreen.main ?? NSScreen() }

        let validCurrent = isValidScreen(currentPanelScreen) ? currentPanelScreen : nil

        switch mode {
        case .followMouse:
            return screenContainingMouse() ?? validCurrent ?? NSScreen.main ?? currentScreens.first!
        case .mainScreen:
            return NSScreen.main ?? currentScreens.first!
        case .builtInScreen:
            if let builtIn = currentScreens.first(where: {
                let name = $0.localizedName.lowercased()
                return name.contains("built-in") || name.contains("内建") || name.contains("color lcd")
            }) {
                return builtIn
            }
            return validCurrent ?? currentScreens.first!
        case .externalScreen:
            if let external = currentScreens.first(where: {
                let name = $0.localizedName.lowercased()
                return !name.contains("built-in") && !name.contains("内建") && !name.contains("color lcd")
            }) {
                return external
            }
            // 外接显示器拔出后自愈回落至主屏，不留在虚空
            return NSScreen.main ?? currentScreens.first!
        }
    }

    // MARK: - 窗口帧度量与放置

    func sizeForState() -> NSSize {
        NSSize(width: IslandMetrics.cardWidth, height: currentExpandedHeight())
    }

    /// 展开态窗口高度：常量推导值与 SwiftUI 内容实际理想高度取较大者。
    /// 常量漏算（文本行高、分割线等）时窗口会偏矮，超出部分从底部裁掉——被裁的
    /// 正是不可滚动的 Token 汇总栏。这里兜住这一类问题：内容多高，窗口至少多高。
    /// 上限取屏幕可用高度，避免常量大幅低估时窗口溢出屏幕。
    func resolvedExpandedHeight(availableHeight: CGFloat) -> CGFloat {
        let computed = currentExpandedHeight()
        guard let fitting = hostingView?.fittingSize.height, fitting.isFinite, fitting > 0 else {
            return computed
        }
        return min(max(computed, fitting), availableHeight)
    }

    /// 贴边目标帧（placeWindow 与 snapToDockEdge 的唯一口径）：
    /// 按 displayState/dockEdge 决定尺寸，clamp 锚点后给出目标 origin。
    /// expanded 态高度用 resolvedExpandedHeight(availableHeight:)，与 SwiftUI 内容
    /// 实际理想高度对齐——吸附同样以内容实际高度为准，否则拖动结束后窗口会被
    /// 缩回常量推导值，底部汇总栏再次被裁。
    /// docked 态保持历史几何：整卡 expandedHeight() 尺寸 + sliver 原点（窗口大部分
    /// 透明/出屏，仅细条可见），锚点钳制口径与 expanded 完全一致。
    func dockTargetFrame(for screen: NSScreen) -> NSRect {
        let visible = screen.visibleFrame
        let cardW = IslandMetrics.cardWidth
        let cardH = displayState == .expanded
            ? resolvedExpandedHeight(availableHeight: visible.height)
            : currentExpandedHeight()
        let targetSize = NSSize(width: cardW, height: cardH)

        let targetOrigin: NSPoint
        switch displayState {
        case .expanded:
            switch dockEdge {
            case .right:
                var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
                targetOrigin = NSPoint(x: visible.maxX - cardW, y: y)
            case .left:
                var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
                targetOrigin = NSPoint(x: visible.minX, y: y)
            case .top:
                var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
                x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
                targetOrigin = NSPoint(x: x, y: visible.maxY - cardH)
            case .bottom:
                var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
                x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
                targetOrigin = NSPoint(x: x, y: visible.minY)
            }
        case .docked:
            switch dockEdge {
            case .right:
                var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
                targetOrigin = NSPoint(x: visible.maxX - IslandMetrics.rightSliverWidth, y: y)
            case .left:
                var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
                targetOrigin = NSPoint(x: visible.minX - cardW + IslandMetrics.rightSliverWidth, y: y)
            case .top:
                var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
                x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
                targetOrigin = NSPoint(x: x, y: visible.maxY - IslandMetrics.topSliverHeight)
            case .bottom:
                var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
                x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
                targetOrigin = NSPoint(x: x, y: visible.minY - cardH + IslandMetrics.topSliverHeight)
            }
        }
        return NSRect(origin: targetOrigin, size: targetSize)
    }

    /// 收起态细条的可视矩形（与 dockTargetFrame docked 分支的锚点/钳制逻辑同源；两处必须同步修改）
    func sliverRect(for screen: NSScreen) -> NSRect {
        let visible = screen.visibleFrame
        switch dockEdge {
        case .right:
            let h = IslandMetrics.rightSliverHeight
            var y = savedRightY.map { $0 - h / 2 } ?? (visible.midY - h / 2)
            y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - h - screenVerticalMargin)
            return NSRect(x: visible.maxX - IslandMetrics.rightSliverWidth, y: y,
                          width: IslandMetrics.rightSliverWidth, height: h)
        case .left:
            let h = IslandMetrics.rightSliverHeight
            var y = savedRightY.map { $0 - h / 2 } ?? (visible.midY - h / 2)
            y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - h - screenVerticalMargin)
            return NSRect(x: visible.minX, y: y,
                          width: IslandMetrics.rightSliverWidth, height: h)
        case .top:
            let w = IslandMetrics.topSliverWidth
            var x = savedTopX.map { $0 - w / 2 } ?? (visible.midX - w / 2)
            x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - w - screenHorizontalMargin)
            return NSRect(x: x, y: visible.maxY - IslandMetrics.topSliverHeight,
                          width: w, height: IslandMetrics.topSliverHeight)
        case .bottom:
            let w = IslandMetrics.topSliverWidth
            var x = savedTopX.map { $0 - w / 2 } ?? (visible.midX - w / 2)
            x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - w - screenHorizontalMargin)
            return NSRect(x: x, y: visible.minY,
                          width: w, height: IslandMetrics.topSliverHeight)
        }
    }

    func placeWindow(animated: Bool) {
        let modeRaw = UserDefaults.standard.string(forKey: SettingKey.screenFollowMode) ?? ScreenFollowMode.followMouse.rawValue
        let followMode = ScreenFollowMode(rawValue: modeRaw) ?? .followMouse
        let screen = Self.targetScreen(mode: followMode, currentPanelScreen: panel.screen)

        let targetRect = dockTargetFrame(for: screen)
        let targetOrigin = targetRect.origin
        let dx = targetOrigin.x - panel.frame.origin.x
        let dy = targetOrigin.y - panel.frame.origin.y
        let dist = hypot(dx, dy)

        // 极简纯净模式：收起态且开启隐藏时淡出至 0，展开时平滑还原
        let targetAlpha: CGFloat = (displayState == .docked && hideDockedSliver) ? 0.0 : 1.0

        // 显示器睡眠时 CA 动画会被挂起：frame 停在动画起点、与 displayState 脱钩
        //（活体验证中观测到该形态的布局崩溃）——睡眠期一律直落终态帧
        let displayAsleep = CGDisplayIsAsleep(CGMainDisplayID()) == 1
        if animated, dist > 1.0, !displayAsleep {
            let isExpanding = (displayState == .expanded)
            let baseDuration: TimeInterval = isExpanding ? 0.32 : 0.26
            let duration: TimeInterval = min(0.36, max(0.18, Double(sqrt(dist / (isExpanding ? targetRect.width : 300.0))) * baseDuration))
            let timing = isExpanding
                ? CAMediaTimingFunction(controlPoints: 0.22, 1.0, 0.36, 1.0)
                : CAMediaTimingFunction(controlPoints: 0.25, 1.0, 0.35, 1.0)

            NSAnimationContext.runAnimationGroup { context in
                context.duration = duration
                context.timingFunction = timing
                panel.animator().setFrame(targetRect, display: false)
                panel.animator().alphaValue = targetAlpha
            }
        } else {
            panel.setFrame(targetRect, display: false)
            panel.alphaValue = targetAlpha
        }
        updateMouseThrottleProximity()
    }

    func updateMouseThrottleProximity() {
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let hit = sliverRect(for: screen).insetBy(dx: -40, dy: -40)
        mouseMoveThrottle.updateTarget(proximityRect: hit, isExpanded: displayState == .expanded)
    }

    func syncExpandedHeight() {
        // 拖拽进行中不抢 frame：拖拽由 WindowServer 驱动，这里的高度自适应动画
        // 会与拖拽争夺窗口 frame（抖动/松手后位置被二次动画改写）。
        // dragEnded 的 snapToDockEdge 已兜底最终位置，不会丢
        guard displayState == .expanded, !isDragging else { return }
        let available = (panel.screen ?? Self.screenContainingMouse())?.visibleFrame.height ?? .greatestFiniteMagnitude
        if abs(panel.frame.height - resolvedExpandedHeight(availableHeight: available)) > 1 {
            placeWindow(animated: true)
        }
    }
}
