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
    /// 进入实时流水页前的路由（用于返回时回到正确层级：主列表 or Agent 详情页）
    private var liveStreamOrigin: CardRoute = .list

    /// 打开实时流水页并记录来源（返回时据此还原）
    func openLiveStream(agentId: String) {
        if case .liveStream = route {} else { liveStreamOrigin = route }
        route = .liveStream(agentId)
    }

    /// 从实时流水页返回：回到进入前的页面
    func closeLiveStream() {
        route = liveStreamOrigin
    }
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
    /// 代际标记：任务在 `defer` 里清空自己的引用前先比对代际，
    /// 否则被取消的旧任务恢复执行时会清掉**新**任务的引用，令 `!= nil` 守卫失效。
    private var collapseGeneration = 0
    private var peekGeneration = 0
    private var didShowOnce = false

    /// 外观模式状态（跟随系统 / 浅色 / 深色）
    @Published public private(set) var appearanceMode: IslandAppearance = .system

    /// 通知策略分级（标准模式 / 专注免打扰 / 完全静默）
    @Published public private(set) var notificationPolicy: NotificationPolicy = .focus

    /// 事件提醒横幅是否展开详情（支持多行排查信息展示与动态窗口高度拓展）
    @Published public var eventBannerExpanded: Bool = false {
        didSet {
            if oldValue != eventBannerExpanded {
                syncExpandedHeight()
            }
        }
    }

    /// 拖动相关状态
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
            clip.masksToBounds = true
            clip.cornerRadius = radius
            clip.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner, .layerMinXMaxYCorner, .layerMaxXMaxYCorner]
            shadowHost?.setShadow(enabled: false, cornerRadius: 0, dockEdge: dockEdge)
        } else {
            // 展开态下由 SwiftUI 的 SideNotchShape 精准裁切反向倒角，避免 AppKit 简单矩形圆角切除倒角
            clip.masksToBounds = false
            clip.cornerRadius = 0
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

    private var lastEventId: UUID?

    private func observe() {
        engine.$updatedAt
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    let workingChanged = self.lastAnyWorking != self.engine.anyWorking
                    let eventChanged = self.lastEventId != self.engine.latestEvent?.id
                    if self.displayState == .docked && !workingChanged && !eventChanged {
                        return
                    }
                    self.lastAnyWorking = self.engine.anyWorking
                    self.lastEventId = self.engine.latestEvent?.id
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
            guard let self else { return }
            // 分辨率/显示器数量变化后可见区域随之改变，只做边缘判定会让展开面板留在屏幕外
            // （例如拔掉面板所在的那台显示器），因此必须重新放置窗口。
            self.placeWindow(animated: false)
            self.evaluateEdgeZone()
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

    // 注：早期还有一个 dragMoved(translation:) 做手动坐标钳制，但调用方
    // （闭包版 cardDrag）恒传 .zero，实际位移一直是 WindowServer 的 performDrag
    // 在负责，那段钳制从未生效。现已删除该路径，4 个详情页顶栏统一走原生拖拽。

    func dragEnded() {
        guard let panel, isDragging else { return }
        isDragging = false
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
        // 与 placeWindow 同源：吸附同样以内容实际高度为准，否则拖动结束后
        // 窗口会被缩回常量推导值，底部汇总栏再次被裁
        let cardH = resolvedExpandedHeight(availableHeight: visible.height)
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

    /// 鼠标移动节流器。鼠标事件可达数百 Hz，且全局监听回调不在主 actor 上——
    /// 此前每个事件都新建一个 Task 跳主线程做全量边缘判定（NSScreen / panel.frame 重算），
    /// 实测快速移动鼠标 ≈ +0.6% CPU。这里用带锁时间戳做零分配预筛，
    /// 只有通过节流的事件才进入主线程。0.05s（20Hz）不影响「进入热区立即展开」的体感：
    /// 光标停在热区内时后续事件仍会通过，最坏延迟 ≈ 一个节流窗口。
    private final class MouseMoveThrottle: @unchecked Sendable {
        private let lock = NSLock()
        private var lastPass = Date.distantPast
        private let minInterval: TimeInterval

        init(minInterval: TimeInterval) { self.minInterval = minInterval }

        func shouldPass(now: Date = Date()) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard now.timeIntervalSince(lastPass) >= minInterval else { return false }
            lastPass = now
            return true
        }
    }

    private let mouseMoveThrottle = MouseMoveThrottle(minInterval: 0.05)

    private func startEdgeZoneMonitor() {
        guard edgeZoneTimer == nil else { return }

        // 1. 定时检测光标位置（0.06s 间隔，低开销无辅助功能权限要求）。
        // 它同时是节流路径的兜底：鼠标停在热区内而事件被节流丢掉时，由它完成展开判定。
        let timer = Timer(timeInterval: 0.06, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.evaluateEdgeZone()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        edgeZoneTimer = timer

        // 2. 本地与全局鼠标移动监听（共用同一个节流器，避免同一物理移动被两条通道各算一次）
        let throttle = mouseMoveThrottle
        mouseLocalMonitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved]) { [weak self] event in
            guard throttle.shouldPass() else { return event }
            MainActor.assumeIsolated {
                self?.evaluateEdgeZone()
            }
            return event
        }
        mouseGlobalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved]) { [weak self] _ in
            // 先做非隔离的节流预筛，再起 Task：逐事件新建 Task 本身就是此前的 CPU 热点
            guard throttle.shouldPass() else { return }
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
        collapseGeneration &+= 1
        let generation = collapseGeneration
        collapseTask = Task { [weak self] in
            defer {
                // 只有仍是本次任务的引用时才清空
                if let self, self.collapseGeneration == generation { self.collapseTask = nil }
            }
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
            eventBannerExpanded = false
            syncExpandedHeight()
            return
        }

        // 任务完成/告警投递到 macOS 通知中心。投递与否由 notificationPolicy 裁决
        // （静默全不发 / 专注仅 costSpike / 标准全发），不再是「无论如何都发」。
        // 事件有唯一 UUID，不会因重复采样重复发送。
        CompletionNotification.post(for: event, policy: notificationPolicy)

        // 事件类型决定初始展开态：熔断类严重告警默认展开详情以提供排查指导，
        // 其他事件收起。此前只在 costSpike 时置 true、从不复位，导致一次告警后
        // 后续所有完成事件也保持 142pt 的展开高度。
        eventBannerExpanded = (event.eventType == .costSpike)

        // 1. 播放系统提示音（受 notificationPolicy 与全局开关 playCompletionSound 共同裁决）。
        // 声音只在这里发一次：系统通知已固定不带 sound，不会出现双重提示音；
        // 关闭「任务完成提示音」后两条通道都安静。
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
        // 普通事件 3.5 秒，成本/死循环熔断保持 6 秒供用户查看或操作
        let peekDuration: TimeInterval = event.eventType == .costSpike ? 6.0 : 3.5
        peekGeneration &+= 1
        let generation = peekGeneration
        peekTask = Task { [weak self] in
            defer {
                // 同 collapseTask：被取消的旧任务不得清掉新任务的引用
                if let self, self.peekGeneration == generation { self.peekTask = nil }
            }
            guard let self, self.displayState == .docked, !self.isDragging else { return }

            // 走真实状态切换：窗口尺寸与 SwiftUI 内容同源（此前只动窗口 frame、displayState
            // 仍为 docked，导致 6pt 细条被拉到展开位置且卡片内容缺失）
            self.manualOpenGraceUntil = Date().addingTimeInterval(peekDuration)
            self.displayState = .expanded

            try? await Task.sleep(nanoseconds: UInt64(peekDuration * 1_000_000_000))
            guard !Task.isCancelled, self.displayState == .expanded else { return }
            // 用户光标已移入面板（或主动操作）→ 转为常驻展开，不自动收回
            guard !Self.isMouseInsidePanel(self.panel) else { return }
            self.collapse()
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
            hasRings: !engine.ringShelfSnapshots.isEmpty,
            hasEvent: engine.latestEvent != nil,
            eventExpanded: eventBannerExpanded
        )
    }

    private func visibleCount() -> Int {
        engine.visibleSnapshots.count
    }

    /// 展开态窗口高度：常量推导值与 SwiftUI 内容实际理想高度取较大者。
    /// 常量漏算（文本行高、分割线等）时窗口会偏矮，超出部分从底部裁掉——被裁的
    /// 正是不可滚动的 Token 汇总栏。这里兜住这一类问题：内容多高，窗口至少多高。
    /// 上限取屏幕可用高度，避免常量大幅低估时窗口溢出屏幕。
    private func resolvedExpandedHeight(availableHeight: CGFloat) -> CGFloat {
        let computed = expandedHeight()
        guard let fitting = hostingView?.fittingSize.height, fitting.isFinite, fitting > 0 else {
            return computed
        }
        return min(max(computed, fitting), availableHeight)
    }

    private func placeWindow(animated: Bool) {
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        let cardW = IslandMetrics.cardWidth
        let cardH = displayState == .expanded
            ? resolvedExpandedHeight(availableHeight: visible.height)
            : expandedHeight()
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
        guard displayState == .expanded else { return }
        let available = (panel.screen ?? Self.screenContainingMouse())?.visibleFrame.height ?? .greatestFiniteMagnitude
        if abs(panel.frame.height - resolvedExpandedHeight(availableHeight: available)) > 1 {
            placeWindow(animated: true)
        }
    }

    // MARK: - NSWindowDelegate

    func windowDidResize(_ notification: Notification) {
        hostingView?.frame = panel.contentView?.bounds ?? hostingView.frame
    }
}

// MARK: - 阴影宿主视图

/// 面板阴影宿主。当前**不投射阴影**：
/// 面板窗口与玻璃卡尺寸完全相同（cardWidth × expandedHeight），CALayer 阴影没有
/// 卡片之外的落地空间，只会沿轮廓边缘向内渗入约一个模糊半径，在贴屏幕那一侧的
/// 上下直角区域形成暗块（用户反馈的「两侧直角矩形的上下阴影」），并使卡片边缘发灰。
/// 一体化贴边的观感依靠玻璃卡自身的 1px 高光边缘与反向倒角，无需阴影。
final class ShadowHostView: NSView {
    func setShadow(enabled: Bool, cornerRadius: CGFloat, dockEdge: DockEdge) {
        // 见类型注释：当前恒不投射阴影，保留签名以兼容调用点。
        layer?.shadowOpacity = 0
        layer?.shadowPath = nil
    }
}
