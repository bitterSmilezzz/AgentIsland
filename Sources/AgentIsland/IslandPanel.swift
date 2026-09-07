import AgentIslandCore
import AppKit
import SwiftUI
import Combine

// MARK: - 灵动岛窗口控制器（支持自由拖拽与顶部/右侧智能吸附贴边）

@MainActor
final class IslandPanelController: NSObject, NSWindowDelegate, ObservableObject {

    @Published var displayState: IslandDisplayState = .docked
    /// 卡内导航路由（仅 expanded 时有意义）
    @Published var route: CardRoute = .list
    /// 停靠贴边方位（顶部 / 右侧）
    @Published var dockEdge: DockEdge = .right

    private var panel: NSPanel!
    private var hostingView: NSHostingView<IslandView>!
    private var shadowHost: ShadowHostView?
    private var clipContainer: NSView?
    private var engine: ActivityEngine
    private var cancellables = Set<AnyCancellable>()
    private var collapseTask: Task<Void, Never>?
    private var peekTask: Task<Void, Never>?
    private var routeResetTask: Task<Void, Never>?
    private var didShowOnce = false

    /// 外观模式状态（跟随系统 / 浅色 / 深色）
    @Published public private(set) var appearanceMode: IslandAppearance = .system

    /// 通知策略分级（标准模式 / 专注免打扰 / 完全静默）
    @Published public private(set) var notificationPolicy: NotificationPolicy = .focus

    /// 拖动相关状态
    private var dragStartOrigin: NSPoint?
    private var isDragging = false
    private var dragCooldownUntil: Date = .distantPast

    /// 记忆锚点坐标（顶部存 X，右侧存 Y）
    private var savedTopX: CGFloat?
    private var savedRightY: CGFloat?

    /// 冷却守卫
    private var expandCooldownUntil: Date = .distantPast
    private var lastAnyWorking: Bool?

    /// 边距常量
    private let screenVerticalMargin: CGFloat = 20
    private let screenHorizontalMargin: CGFloat = 20

    /// 自动收起延迟
    private var collapseDelay: TimeInterval

    func applyCollapseDelay(_ delay: TimeInterval) {
        collapseDelay = max(0.2, delay)
    }

    func setDockEdge(_ edge: DockEdge) {
        dockEdge = edge
        UserDefaults.standard.set(edge.rawValue, forKey: SettingKey.dockEdge)
        updateChrome()
        placeWindow(animated: true)
    }

    func resetPosition() {
        savedTopX = nil
        savedRightY = nil
        UserDefaults.standard.removeObject(forKey: SettingKey.dockAnchorX)
        UserDefaults.standard.removeObject(forKey: SettingKey.dockAnchorY)
        updateChrome()
        placeWindow(animated: true)
    }

    init(engine: ActivityEngine) {
        self.engine = engine
        self.collapseDelay = UserDefaults.standard.object(forKey: SettingKey.collapseDelay) as? Double ?? 0.5

        if let edgeStr = UserDefaults.standard.string(forKey: SettingKey.dockEdge),
           let edge = DockEdge(rawValue: edgeStr) {
            self.dockEdge = edge
        } else {
            self.dockEdge = .right
        }

        if let sx = UserDefaults.standard.object(forKey: SettingKey.dockAnchorX) as? Double {
            self.savedTopX = CGFloat(sx)
        }
        if let sy = UserDefaults.standard.object(forKey: SettingKey.dockAnchorY) as? Double {
            self.savedRightY = CGFloat(sy)
        }
        self.appearanceMode = Self.persistedAppearance()
        self.notificationPolicy = Self.persistedNotificationPolicy()

        super.init()
        setupPanel()
        observe()
    }

    private func setupPanel() {
        let initialSize = sizeForState()
        panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: initialSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.isMovable = true
        panel.isMovableByWindowBackground = false
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.animationBehavior = .none
        panel.delegate = self
        panel.isRestorable = false
        applyAppearance(Self.persistedAppearance())

        let content = IslandView(engine: engine, controller: self)
        hostingView = NSHostingView(rootView: content)
        hostingView.frame = NSRect(origin: .zero, size: initialSize)

        let container = NSView(frame: NSRect(origin: .zero, size: initialSize))
        hostingView.autoresizingMask = [.width, .height]
        container.addSubview(hostingView)
        container.wantsLayer = true
        container.layer?.masksToBounds = true
        container.layer?.cornerCurve = .continuous

        let shadowHost = ShadowHostView(frame: NSRect(origin: .zero, size: initialSize))
        shadowHost.wantsLayer = true
        container.autoresizingMask = [.width, .height]
        shadowHost.addSubview(container)
        panel.contentView = shadowHost
        self.shadowHost = shadowHost
        self.clipContainer = container

        updateChrome()

        $displayState
            .removeDuplicates()
            .sink { [weak self] state in
                Task { @MainActor [weak self] in
                    self?.onStateChanged(state)
                }
            }
            .store(in: &cancellables)

        $route
            .removeDuplicates()
            .dropFirst()
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in
                    self?.syncExpandedHeight()
                }
            }
            .store(in: &cancellables)

        engine.$latestEvent
            .removeDuplicates()
            .sink { [weak self] event in
                Task { @MainActor [weak self] in
                    self?.handleTaskEvent(event)
                }
            }
            .store(in: &cancellables)
    }

    func updateChrome() {
        guard let clip = clipContainer?.layer else { return }
        if displayState == .docked {
            let radius = (dockEdge == .top ? IslandMetrics.topSliverHeight : IslandMetrics.rightSliverWidth) / 2
            clip.cornerRadius = radius
            clip.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner, .layerMinXMaxYCorner, .layerMaxXMaxYCorner]
            shadowHost?.setShadow(enabled: false, cornerRadius: 0, dockEdge: dockEdge)
        } else {
            clip.cornerRadius = Theme.radiusLg
            if dockEdge == .right {
                clip.maskedCorners = [.layerMinXMinYCorner, .layerMinXMaxYCorner]
            } else {
                clip.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner]
            }
            shadowHost?.setShadow(enabled: true, cornerRadius: Theme.radiusLg, dockEdge: dockEdge)
        }
    }

    func applyAppearance(_ mode: IslandAppearance) {
        appearanceMode = mode
        UserDefaults.standard.set(mode.rawValue, forKey: SettingKey.islandAppearance)
        panel?.appearance = mode.nsAppearance
        // 同步应用到非面板窗口（例如设置窗口）
        for window in NSApp.windows {
            if !(window is NSPanel) {
                window.appearance = mode.nsAppearance
            }
        }
    }

    private static func persistedAppearance() -> IslandAppearance {
        IslandAppearance(rawValue: UserDefaults.standard.string(forKey: SettingKey.islandAppearance) ?? "") ?? .system
    }

    func applyNotificationPolicy(_ policy: NotificationPolicy) {
        notificationPolicy = policy
        UserDefaults.standard.set(policy.rawValue, forKey: SettingKey.notificationPolicy)
    }

    private static func persistedNotificationPolicy() -> NotificationPolicy {
        if let raw = UserDefaults.standard.string(forKey: SettingKey.notificationPolicy),
           let policy = NotificationPolicy(rawValue: raw) {
            return policy
        }
        return .focus // 默认推荐专注免打扰
    }

    private func observe() {
        engine.$updatedAt
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if self.displayState == .docked, self.lastAnyWorking == self.engine.anyWorking {
                        return
                    }
                    self.lastAnyWorking = self.engine.anyWorking
                    self.hostingView?.needsDisplay = true
                    if self.displayState == .expanded {
                        self.syncExpandedHeight()
                    }
                }
            }
            .store(in: &cancellables)
    }

    // MARK: - 对外控制

    func show() {
        guard !didShowOnce else { return }
        didShowOnce = true
        startEdgeZoneMonitor()

        NotificationCenter.default.addObserver(
            self, selector: #selector(screenConfigChanged),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)

        displayState = .docked
        engine.setPresentationActive(false)
        updateChrome()
        placeWindow(animated: false)
        panel.orderFrontRegardless()
    }

    @objc private func screenConfigChanged() {
        Task { @MainActor [weak self] in
            self?.evaluateEdgeZone()
        }
    }

    deinit {
        edgeZoneTimer?.invalidate()
        if let m = mouseLocalMonitor { NSEvent.removeMonitor(m) }
        if let m = mouseGlobalMonitor { NSEvent.removeMonitor(m) }
        if let m = clickLocalMonitor { NSEvent.removeMonitor(m) }
        if let m = clickGlobalMonitor { NSEvent.removeMonitor(m) }
        NotificationCenter.default.removeObserver(self)
    }

    private var manualOpenGraceUntil: Date = .distantPast

    func toggle() {
        switch displayState {
        case .docked:
            cancelPendingTasks()
            expandCooldownUntil = .distantPast
            dragCooldownUntil = .distantPast
            manualOpenGraceUntil = Date().addingTimeInterval(3.0)
            displayState = .expanded
        case .expanded:
            collapse()
        }
    }

    /// 显式收起灵动岛（带防抖冷却并即刻重置保护期）
    func collapse() {
        guard displayState == .expanded else { return }
        cancelPendingTasks()
        manualOpenGraceUntil = .distantPast
        expandCooldownUntil = Date().addingTimeInterval(0.4)
        displayState = .docked
    }

    func expandFromHover() {
        guard displayState == .docked, !isDragging, Date() >= dragCooldownUntil else { return }
        cancelPendingTasks()
        manualOpenGraceUntil = .distantPast
        displayState = .expanded
    }

    // MARK: - 拖动与智能吸附

    func beginDrag() {
        guard panel != nil, displayState == .expanded else { return }
        isDragging = true
        cancelPendingTasks()
    }

    func dragMoved(translation: CGSize) {
        guard let panel, displayState == .expanded else { return }
        if dragStartOrigin == nil {
            dragStartOrigin = panel.frame.origin
            isDragging = true
            cancelPendingTasks()
        }
        guard let start = dragStartOrigin,
              let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        var x = start.x + translation.width
        var y = start.y - translation.height
        x = min(max(x, visible.minX), visible.maxX - panel.frame.width)
        y = min(max(y, visible.minY), visible.maxY - panel.frame.height)
        panel.setFrameOrigin(NSPoint(x: x, y: y))
    }

    func dragEnded() {
        guard let panel, isDragging else { return }
        isDragging = false
        dragStartOrigin = nil
        dragCooldownUntil = Date().addingTimeInterval(0.6)

        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        let cx = panel.frame.midX
        let cy = panel.frame.midY

        let distToTop = abs(visible.maxY - panel.frame.maxY)
        let distToRight = abs(visible.maxX - panel.frame.maxX)

        if distToRight < distToTop {
            dockEdge = .right
            savedRightY = min(max(cy, visible.minY + 60), visible.maxY - 60)
            UserDefaults.standard.set(dockEdge.rawValue, forKey: SettingKey.dockEdge)
            UserDefaults.standard.set(Double(savedRightY!), forKey: SettingKey.dockAnchorY)
        } else {
            dockEdge = .top
            savedTopX = min(max(cx, visible.minX + 70), visible.maxX - 70)
            UserDefaults.standard.set(dockEdge.rawValue, forKey: SettingKey.dockEdge)
            UserDefaults.standard.set(Double(savedTopX!), forKey: SettingKey.dockAnchorX)
        }

        updateChrome()
        if displayState == .expanded {
            snapToDockEdge(animated: true)
        } else {
            placeWindow(animated: true)
        }
    }

    func snapToDockEdge(animated: Bool) {
        guard displayState == .expanded else { return }
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        let cardW = IslandMetrics.cardWidth
        let cardH = expandedHeight()
        let targetOrigin: NSPoint

        switch dockEdge {
        case .right:
            var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
            y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
            targetOrigin = NSPoint(x: visible.maxX - cardW, y: y)
        case .top:
            var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
            x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
            targetOrigin = NSPoint(x: x, y: visible.maxY - cardH)
        }

        let targetRect = NSRect(origin: targetOrigin, size: NSSize(width: cardW, height: cardH))
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

    // MARK: - 边缘与光标监控

    private var mouseLocalMonitor: Any?
    private var mouseGlobalMonitor: Any?
    private var clickLocalMonitor: Any?
    private var clickGlobalMonitor: Any?
    private var edgeZoneTimer: Timer?
    private var lastEvalAt = Date.distantPast
    private let evalMinInterval: TimeInterval = 0.05

    private func startEdgeZoneMonitor() {
        guard edgeZoneTimer == nil else { return }

        // 1. 定时检测光标位置（0.06s 间隔，低开销无辅助功能权限要求）
        let timer = Timer(timeInterval: 0.06, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.evaluateEdgeZone()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        edgeZoneTimer = timer

        // 2. 本地与全局鼠标移动监听
        mouseLocalMonitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved]) { [weak self] event in
            MainActor.assumeIsolated {
                self?.evaluateEdgeZone()
            }
            return event
        }
        mouseGlobalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved]) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.evaluateEdgeZone()
            }
        }

        // 3. 点击外部区域即刻收起（全局与应用内失焦收起）
        clickGlobalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self,
                      self.displayState == .expanded,
                      !self.isDragging,
                      !Self.isMouseInsidePanel(self.panel) else { return }
                self.collapse()
            }
        }
        clickLocalMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] event in
            MainActor.assumeIsolated {
                guard let self,
                      self.displayState == .expanded,
                      !self.isDragging,
                      !Self.isMouseInsidePanel(self.panel) else { return }
                self.collapse()
            }
            return event
        }
    }

    private func onMouseMoved() {
        let now = Date()
        guard now.timeIntervalSince(lastEvalAt) >= evalMinInterval else { return }
        lastEvalAt = now
        evaluateEdgeZone()
    }

    private func evaluateEdgeZone() {
        guard didShowOnce, !isDragging else { return }
        let mousePressed = NSEvent.pressedMouseButtons != 0
        guard !mousePressed else { return }

        let loc = NSEvent.mouseLocation
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame

        if displayState == .docked {
            guard Date() >= expandCooldownUntil, Date() >= dragCooldownUntil else { return }
            var inZone = false
            switch dockEdge {
            case .right:
                // 右边缘细条检测：光标位于屏幕右边缘微区域且在细条垂直范围内
                let cy = savedRightY.map { min(max($0, visible.minY + 60), visible.maxY - 60) } ?? visible.midY
                if loc.x >= visible.maxX - (IslandMetrics.rightSliverWidth + 12) {
                    inZone = abs(loc.y - cy) <= (IslandMetrics.rightSliverHeight / 2 + 12)
                }
            case .top:
                // 顶边缘细条检测：光标位于屏幕顶部边缘微区域且在细条水平范围内
                let cx = savedTopX.map { min(max($0, visible.minX + 70), visible.maxX - 70) } ?? visible.midX
                if loc.y >= visible.maxY - (IslandMetrics.topSliverHeight + 12) {
                    inZone = abs(loc.x - cx) <= (IslandMetrics.topSliverWidth / 2 + 12)
                }
            }
            if inZone {
                cancelPendingTasks()
                manualOpenGraceUntil = .distantPast
                displayState = .expanded
            }
        } else if displayState == .expanded {
            if Self.isMouseInsidePanel(panel) {
                // 光标已在面板内，即刻解除手动展开保护期，取消任何收起计划
                manualOpenGraceUntil = .distantPast
                if collapseTask != nil {
                    collapseTask?.cancel()
                    collapseTask = nil
                }
            } else {
                // 光标离开面板，且超过了保护期，安排收起
                guard Date() >= manualOpenGraceUntil else { return }
                scheduleCollapse()
            }
        }
    }

    // MARK: - 收回

    private func scheduleCollapse() {
        guard displayState == .expanded, !isDragging else { return }
        if collapseTask != nil { return }
        let delay = collapseDelay
        collapseTask = Task { [weak self] in
            defer { self?.collapseTask = nil }
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self,
                  !Task.isCancelled,
                  !self.isDragging,
                  self.displayState == .expanded,
                  Date() >= self.manualOpenGraceUntil,
                  !Self.isMouseInsidePanel(self.panel) else { return }
            self.expandCooldownUntil = Date().addingTimeInterval(0.35)
            self.displayState = .docked
        }
    }

    static func isMouseInsidePanel(_ panel: NSPanel?) -> Bool {
        guard let panel, panel.isVisible else { return false }
        let frame = panel.frame.insetBy(dx: -8, dy: -8)
        return frame.contains(NSEvent.mouseLocation)
    }

    private func cancelPendingTasks() {
        collapseTask?.cancel()
        peekTask?.cancel()
        routeResetTask?.cancel()
        collapseTask = nil
        peekTask = nil
        routeResetTask = nil
    }

    // MARK: - Peek

    private func handleTaskEvent(_ event: AgentTaskEvent?) {
        guard let event else {
            syncExpandedHeight()
            return
        }

        // 1. 播放系统提示音（受 notificationPolicy 与全局开关 playCompletionSound 共同裁决）
        let soundEnabled = UserDefaults.standard.object(forKey: SettingKey.playCompletionSound) as? Bool ?? true
        if notificationPolicy.shouldPlaySound(for: event.eventType, soundEnabled: soundEnabled) {
            if event.eventType == .costSpike {
                if let sound = NSSound(named: "Sosumi") ?? NSSound(named: "Basso") {
                    sound.play()
                } else {
                    NSSound(named: "Glass")?.play()
                }
            } else {
                NSSound(named: "Glass")?.play()
            }
        }

        // 2. 窗口高度自适应扩展
        syncExpandedHeight()

        // 3. 若当前处于收起态（docked），按分级策略决定是否触发微弹窗 Peek
        if displayState == .docked && notificationPolicy.shouldPeek(for: event.eventType) {
            peekForEvent(event)
        }
    }

    private func peekForEvent(_ event: AgentTaskEvent) {
        guard displayState == .docked, peekTask == nil, !isDragging else { return }
        peekTask = Task { [weak self] in
            defer { self?.peekTask = nil }
            guard let self, let panel = self.panel else { return }
            let initialRect = panel.frame
            var shownRect = panel.frame
            guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
            let visible = screen.visibleFrame
            let cardH = self.expandedHeight()

            switch self.dockEdge {
            case .right:
                var y = self.savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + self.screenVerticalMargin), visible.maxY - cardH - self.screenVerticalMargin)
                shownRect = NSRect(x: visible.maxX - IslandMetrics.cardWidth, y: y,
                                   width: IslandMetrics.cardWidth, height: cardH)
            case .top:
                var x = self.savedTopX.map { $0 - IslandMetrics.cardWidth / 2 } ?? (visible.midX - IslandMetrics.cardWidth / 2)
                x = min(max(x, visible.minX + self.screenHorizontalMargin), visible.maxX - IslandMetrics.cardWidth - self.screenHorizontalMargin)
                shownRect = NSRect(x: x, y: visible.maxY - cardH,
                                   width: IslandMetrics.cardWidth, height: cardH)
            }

            // 平滑滑出
            await withCheckedContinuation { cont in
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.32
                    ctx.timingFunction = CAMediaTimingFunction(controlPoints: 0.22, 1.0, 0.36, 1.0)
                    panel.animator().setFrame(shownRect, display: false)
                }, completionHandler: { cont.resume() })
            }

            // 保持展示供用户查看（普通事件 3.5 秒，成本/死循环熔断保持 6 秒供用户查看或操作）
            let peekDuration: UInt64 = event.eventType == .costSpike ? 6_000_000_000 : 3_500_000_000
            try? await Task.sleep(nanoseconds: peekDuration)

            // 若用户光标在展示期间已移入面板，转为常驻展开态，不自动收回！
            guard !Task.isCancelled, self.displayState == .docked else { return }
            if Self.isMouseInsidePanel(self.panel) {
                self.displayState = .expanded
                return
            }

            // 平滑收回
            await withCheckedContinuation { cont in
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.28
                    ctx.timingFunction = CAMediaTimingFunction(controlPoints: 0.25, 1.0, 0.35, 1.0)
                    panel.animator().setFrame(initialRect, display: false)
                }, completionHandler: { cont.resume() })
            }
        }
    }

    // MARK: - 状态切换

    private func onStateChanged(_ state: IslandDisplayState) {
        updateChrome()
        switch state {
        case .docked:
            routeResetTask?.cancel()
            routeResetTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: 320_000_000)
                guard let self, !Task.isCancelled, self.displayState == .docked else { return }
                var t = Transaction(animation: nil)
                t.disablesAnimations = true
                withTransaction(t) { self.route = .list }
            }
            engine.setPresentationActive(false)
            placeWindow(animated: true)
        case .expanded:
            engine.setPresentationActive(true)
            panel.orderFrontRegardless()
            placeWindow(animated: true)
        }
    }

    static func screenContainingMouse() -> NSScreen? {
        let loc = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(loc) } ?? NSScreen.main
    }

    // MARK: - 窗口帧度量与放置

    private func sizeForState() -> NSSize {
        NSSize(width: IslandMetrics.cardWidth, height: expandedHeight())
    }

    private func expandedHeight() -> CGFloat {
        IslandMetrics.expandedHeight(
            route: route,
            visibleCount: visibleCount(),
            hasSummary: !engine.grandTotal.isEmpty,
            hasEvent: engine.latestEvent != nil
        )
    }

    private func visibleCount() -> Int {
        engine.visibleSnapshots.count
    }

    private func placeWindow(animated: Bool) {
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        let cardW = IslandMetrics.cardWidth
        let cardH = expandedHeight()
        let targetSize = NSSize(width: cardW, height: cardH)

        let targetOrigin: NSPoint
        switch displayState {
        case .expanded:
            switch dockEdge {
            case .right:
                var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
                targetOrigin = NSPoint(x: visible.maxX - cardW, y: y)
            case .top:
                var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
                x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
                targetOrigin = NSPoint(x: x, y: visible.maxY - cardH)
            }
        case .docked:
            switch dockEdge {
            case .right:
                var y = savedRightY.map { $0 - cardH / 2 } ?? (visible.midY - cardH / 2)
                y = min(max(y, visible.minY + screenVerticalMargin), visible.maxY - cardH - screenVerticalMargin)
                targetOrigin = NSPoint(x: visible.maxX - IslandMetrics.rightSliverWidth, y: y)
            case .top:
                var x = savedTopX.map { $0 - cardW / 2 } ?? (visible.midX - cardW / 2)
                x = min(max(x, visible.minX + screenHorizontalMargin), visible.maxX - cardW - screenHorizontalMargin)
                targetOrigin = NSPoint(x: x, y: visible.maxY - IslandMetrics.topSliverHeight)
            }
        }

        let targetRect = NSRect(origin: targetOrigin, size: targetSize)
        let dx = targetOrigin.x - panel.frame.origin.x
        let dy = targetOrigin.y - panel.frame.origin.y
        let dist = hypot(dx, dy)

        if animated, dist > 1.0 {
            let isExpanding = (displayState == .expanded)
            let baseDuration: TimeInterval = isExpanding ? 0.32 : 0.26
            let duration: TimeInterval = min(0.36, max(0.18, Double(sqrt(dist / (isExpanding ? cardW : 300.0))) * baseDuration))
            let timing = isExpanding
                ? CAMediaTimingFunction(controlPoints: 0.22, 1.0, 0.36, 1.0)
                : CAMediaTimingFunction(controlPoints: 0.25, 1.0, 0.35, 1.0)

            NSAnimationContext.runAnimationGroup { context in
                context.duration = duration
                context.timingFunction = timing
                panel.animator().setFrame(targetRect, display: false)
            }
        } else {
            panel.setFrame(targetRect, display: false)
        }
    }

    private func syncExpandedHeight() {
        if displayState == .expanded, abs(panel.frame.height - expandedHeight()) > 1 {
            placeWindow(animated: true)
        }
    }

    // MARK: - NSWindowDelegate

    func windowDidResize(_ notification: Notification) {
        hostingView?.frame = panel.contentView?.bounds ?? hostingView.frame
    }
}

// MARK: - 阴影宿主视图

final class ShadowHostView: NSView {
    private var shadowEnabled = false
    private var cornerRadius: CGFloat = Theme.radiusLg
    private var dockEdge: DockEdge = .right

    override var frame: NSRect {
        didSet { updateShadowPath() }
    }

    override func layout() {
        super.layout()
        updateShadowPath()
    }

    func setShadow(enabled: Bool, cornerRadius: CGFloat, dockEdge: DockEdge) {
        self.cornerRadius = cornerRadius
        self.dockEdge = dockEdge
        guard let layer else { return }
        layer.shadowColor = NSColor.black.cgColor
        CATransaction.begin()
        CATransaction.setAnimationDuration(enabled ? 0.34 : 0.28)
        CATransaction.setAnimationTimingFunction(
            enabled ? CAMediaTimingFunction(controlPoints: 0.16, 1.0, 0.3, 1.0)
                    : CAMediaTimingFunction(controlPoints: 0.25, 0.1, 0.25, 1.0)
        )
        layer.shadowOpacity = enabled ? Theme.panelShadowOpacity : 0
        layer.shadowRadius = enabled ? Theme.panelShadowRadius : 0
        layer.shadowOffset = CGSize(width: 0, height: Theme.panelShadowOffsetY)
        CATransaction.commit()
        updateShadowPath()
    }

    private func updateShadowPath() {
        guard let layer else { return }
        if layer.shadowOpacity > 0 {
            if dockEdge == .top {
                layer.shadowPath = Self.bottomRoundedPath(bounds: bounds, radius: cornerRadius)
            } else {
                layer.shadowPath = Self.leftRoundedPath(bounds: bounds, radius: cornerRadius)
            }
        } else {
            layer.shadowPath = nil
        }
    }

    private static func leftRoundedPath(bounds: NSRect, radius: CGFloat) -> CGPath {
        let r = min(radius, bounds.height / 2)
        let path = CGMutablePath()
        path.move(to: CGPoint(x: bounds.minX, y: bounds.minY + r))
        path.addArc(center: CGPoint(x: bounds.minX + r, y: bounds.minY + r), radius: r,
                    startAngle: .pi, endAngle: .pi * 1.5, clockwise: false)
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.minY))
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.maxY))
        path.addLine(to: CGPoint(x: bounds.minX + r, y: bounds.maxY))
        path.addArc(center: CGPoint(x: bounds.minX + r, y: bounds.maxY - r), radius: r,
                    startAngle: .pi * 1.5, endAngle: .pi * 2, clockwise: false)
        path.closeSubpath()
        return path
    }

    private static func bottomRoundedPath(bounds: NSRect, radius: CGFloat) -> CGPath {
        let r = min(radius, min(bounds.width / 2, bounds.height / 2))
        let path = CGMutablePath()
        path.move(to: CGPoint(x: bounds.minX, y: bounds.maxY))
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.maxY))
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.minY + r))
        path.addArc(center: CGPoint(x: bounds.maxX - r, y: bounds.minY + r), radius: r,
                    startAngle: 0, endAngle: .pi * 1.5, clockwise: false)
        path.addLine(to: CGPoint(x: bounds.minX + r, y: bounds.minY))
        path.addArc(center: CGPoint(x: bounds.minX + r, y: bounds.minY + r), radius: r,
                    startAngle: .pi * 1.5, endAngle: .pi, clockwise: false)
        path.closeSubpath()
        return path
    }
}
