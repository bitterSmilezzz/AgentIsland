import AgentIslandCore
import AppKit
import SwiftUI
import Combine

// MARK: - 灵动岛窗口控制器（支持自由拖拽与四边智能吸附贴边）

@MainActor
final class IslandPanelController: NSObject, NSWindowDelegate, ObservableObject {

    @Published var displayState: IslandDisplayState = .docked
    /// 卡内导航路由（仅 expanded 时有意义）
    @Published var route: CardRoute = .list {
        didSet {
            guard oldValue != route else { return }
            if case .agentDetail = route {
                switch oldValue {
                case .agentDetail, .sessions, .liveStream:
                    // 处于详情子树内部跳转（如从 sessions 或 liveStream 返回），保持原进入来源不变
                    break
                default:
                    agentDetailOrigin = oldValue
                }
            } else if route == .list {
                agentDetailOrigin = .list
                liveStreamOrigin = .list
            }
        }
    }
    /// 进入实时流水页前的路由（用于返回时回到正确层级：主列表 or Agent 详情页）
    var liveStreamOrigin: CardRoute = .list
    /// 进入 Agent 详情页前的路由（用于返回时回到正确层级：主列表 or Token 统计图等）
    var agentDetailOrigin: CardRoute = .list
    /// 停靠贴边方位（上、右、下、左）
    @Published var dockEdge: DockEdge = .right

    var panel: NSPanel!
    var hostingView: NSHostingView<IslandView>!
    private var shadowHost: NSView?
    private var clipContainer: NSView?
    var engine: ActivityEngine
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

    /// 键盘导航高亮选中的 Agent ID
    @Published public var focusedAgentId: String? = nil

    /// 拖动相关状态
    var isDragging = false
    var dragCooldownUntil: Date = .distantPast

    /// 记忆锚点坐标（水平边缘存 X，垂直边缘存 Y）
    var savedTopX: CGFloat?
    var savedRightY: CGFloat?

    /// 冷却守卫
    private var expandCooldownUntil: Date = .distantPast
    private var lastAnyWorking: Bool?

    /// 边距常量
    let screenVerticalMargin: CGFloat = 20
    let screenHorizontalMargin: CGFloat = 20

    /// 自动收起延迟
    private var collapseDelay: TimeInterval

    func applyCollapseDelay(_ delay: TimeInterval) {
        // 与 init 读取同口径钳制（脏值自愈唯一通道，避免两处口径漂移）
        collapseDelay = min(max(delay, SettingLimits.collapseDelayRange.lowerBound),
                            SettingLimits.collapseDelayRange.upperBound)
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
        // init 即钳制：脏持久化值（负数/超大）此前直通 scheduleCollapse 的
        // UInt64(delay * 1e9) 转换，负值/溢出直接运行时 trap——鼠标每次离开展开卡
        // 都触发收起调度，等于「收起必崩」死循环（设置页路径的钳制覆盖不到这里）
        let persistedDelay = UserDefaults.standard.object(forKey: SettingKey.collapseDelay) as? Double ?? 0.5
        self.collapseDelay = min(max(persistedDelay.isFinite ? persistedDelay : 0.5,
                                     SettingLimits.collapseDelayRange.lowerBound),
                                 SettingLimits.collapseDelayRange.upperBound)

        if let edgeStr = UserDefaults.standard.string(forKey: SettingKey.dockEdge),
           let edge = DockEdge(rawValue: edgeStr) {
            self.dockEdge = edge
        } else {
            self.dockEdge = .right
        }

        // 锚点坐标防 NaN/Inf：损坏值会让后续 min/max 钳制失效（NaN 比较恒 false），
        // 窗口可能被放到可见区域外且再也无法拖回
        if let sx = UserDefaults.standard.object(forKey: SettingKey.dockAnchorX) as? Double, sx.isFinite {
            self.savedTopX = CGFloat(sx)
        }
        if let sy = UserDefaults.standard.object(forKey: SettingKey.dockAnchorY) as? Double, sy.isFinite {
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

        let shadowHost = NSView(frame: NSRect(origin: .zero, size: initialSize))
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

        // 独立事件流保证多个 Agent 在同一采样批次里同时等待确认时，系统通知与声音
        // 逐条送达；latestEvent 仍只负责岛内单槽横幅，不再承担投递队列职责。
        engine.taskEvents
            .sink { [weak self] event in
                Task { @MainActor [weak self] in
                    self?.deliverTaskEventAlert(event)
                }
            }
            .store(in: &cancellables)
    }

    func updateChrome() {
        guard let clip = clipContainer?.layer else { return }
        if displayState == .docked {
            let radius = (dockEdge.isHorizontal
                ? IslandMetrics.topSliverHeight
                : IslandMetrics.rightSliverWidth) / 2
            clip.masksToBounds = true
            clip.cornerRadius = radius
            clip.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner, .layerMinXMaxYCorner, .layerMaxXMaxYCorner]
        } else {
            // 展开态下由 SwiftUI 的 SideNotchShape 精准裁切反向倒角，避免 AppKit 简单矩形圆角切除倒角
            clip.masksToBounds = false
            clip.cornerRadius = 0
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
    /// 展开态「高度影响签名」去重：引擎每拍发布（lastActivityAgo/CPU 每拍变化），
    /// 签名不变时跳过 needsDisplay 与 fittingSize 全量排版（后者估 1-3ms/拍）
    private var lastHeightSignature: String?

    private func observe() {
        engine.$updatedAt
            .sink { [weak self] _ in
                // 引擎是 @MainActor、发布在主线程：无需 Task 跳跃（每拍一次分配+调度）
                guard let self else { return }
                let workingChanged = self.lastAnyWorking != self.engine.anyWorking
                let eventChanged = self.lastEventId != self.engine.latestEvent?.id
                if self.displayState == .docked && !workingChanged && !eventChanged {
                    return
                }
                self.lastAnyWorking = self.engine.anyWorking
                self.lastEventId = self.engine.latestEvent?.id
                if self.displayState == .expanded {
                    // 高度影响签名去重：窗口高度只由 route/可见数/汇总栏/环架/事件横幅决定，
                    // 快照的逐拍字段（CPU、lastActivityAgo、currentAction）不影响高度，
                    // SwiftUI 已通过 @ObservedObject 自行失效重绘，无需整卡 needsDisplay
                    let sig = self.expandedHeightSignature()
                    guard sig != self.lastHeightSignature else { return }
                    self.lastHeightSignature = sig
                    self.syncExpandedHeight()
                } else {
                    // docked：细条呼吸灯/事件横幅的 AppKit 层显式失效保留
                    self.hostingView?.needsDisplay = true
                }
            }
            .store(in: &cancellables)
    }

    /// 全部影响展开卡窗口高度的信号（与 expandedHeight() 的输入一一对应）。
    /// 改动 expandedHeight 的输入依赖时必须同步这里，否则高度会漏更新。
    private func expandedHeightSignature() -> String {
        "\(route)|\(visibleCount())|\(engine.grandTotal.isEmpty)|\(engine.ringShelfSnapshots.isEmpty)|\(engine.latestEvent?.id.uuidString ?? "-")|\(eventBannerExpanded)"
    }

    func currentExpandedHeight() -> CGFloat {
        expandedHeight()
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

    func visibleCount() -> Int {
        engine.visibleSnapshots.count
    }

    // MARK: - 对外控制

    func show() {
        guard !didShowOnce else { return }
        didShowOnce = true
        startEdgeZoneMonitor()

        NotificationCenter.default.addObserver(
            self, selector: #selector(screenConfigChanged),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)
        // 唤醒后帧重同步：睡眠期 CA 动画挂起可能让窗口 frame 停在动画起点
        //（与 displayState 脱钩），唤醒后直落终态帧并重判点击穿透。
        // 熄屏唤醒走 NSWorkspace 的 screensDidWake（default center 不投递、
        // 纯熄屏场景 didWake 不触发——活体实证），两个通知都挂 workspace center
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(systemDidWake),
            name: NSWorkspace.screensDidWakeNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(systemDidWake),
            name: NSWorkspace.didWakeNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(systemWillSleep),
            name: NSWorkspace.willSleepNotification, object: nil)

        if CommandLine.arguments.contains("--expanded") || CommandLine.arguments.contains("--analytics") {
            displayState = .expanded
            if CommandLine.arguments.contains("--analytics") {
                route = .tokenAnalytics
            }
            engine.setPresentationActive(true)
        } else {
            displayState = .docked
            engine.setPresentationActive(false)
        }
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

    @objc private func systemWillSleep() {
        Task { @MainActor [weak self] in
            self?.engine.handleSystemSleep()
        }
    }

    @objc private func systemDidWake() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.screenConfigChanged()
            self.engine.handleSystemWake()
        }
    }

    /// 边缘检测 Timer 节律（R35/G2）：活跃 0.12s，静止 3 秒后降档 0.5s。
    /// hover 展开的主路径是 0.05s 节流的鼠标事件（不受降档影响）；Timer 只是
    /// 「光标停在热区内、事件被节流丢掉」的兜底——静止（远离热区且未展开）时
    /// 高频唤醒纯属浪费（能效影响大于 CPU%）。光标进入热区/展开态立即恢复快档
    private static let edgeZoneFastInterval: TimeInterval = 0.12
    private static let edgeZoneSlowInterval: TimeInterval = 1.0
    private static let edgeZoneQuietTicksToSlow = 25   // 25 × 0.12s ≈ 3s
    private var edgeZoneSlowMode = false
    private var edgeZoneQuietTicks = 0

    private func installEdgeZoneTimer(interval: TimeInterval) {
        edgeZoneTimer?.invalidate()
        let timer = Timer(timeInterval: interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let active = self.evaluateEdgeZone()
                self.updateEdgeZoneTimerMode(active: active)
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        edgeZoneTimer = timer
    }

    private func updateEdgeZoneTimerMode(active: Bool) {
        if active || displayState == .expanded {
            edgeZoneQuietTicks = 0
            if edgeZoneSlowMode {
                edgeZoneSlowMode = false
                installEdgeZoneTimer(interval: Self.edgeZoneFastInterval)
            }
            return
        }
        edgeZoneQuietTicks += 1
        if !edgeZoneSlowMode, edgeZoneQuietTicks >= Self.edgeZoneQuietTicksToSlow {
            edgeZoneSlowMode = true
            installEdgeZoneTimer(interval: Self.edgeZoneSlowInterval)
        }
    }

    deinit {
        edgeZoneTimer?.invalidate()
        if let m = mouseLocalMonitor { NSEvent.removeMonitor(m) }
        if let m = mouseGlobalMonitor { NSEvent.removeMonitor(m) }
        if let m = clickLocalMonitor { NSEvent.removeMonitor(m) }
        if let m = clickGlobalMonitor { NSEvent.removeMonitor(m) }
        if let m = keyEscapeMonitor { NSEvent.removeMonitor(m) }
        NotificationCenter.default.removeObserver(self)
    }

    var manualOpenGraceUntil: Date = .distantPast

    func expand(graceDuration: TimeInterval = 0) {
        cancelPendingTasks()
        expandCooldownUntil = .distantPast
        dragCooldownUntil = .distantPast
        if graceDuration > 0 {
            manualOpenGraceUntil = Date().addingTimeInterval(graceDuration)
        }
        displayState = .expanded
        HapticFeedback.perform(.levelChange)
        engine.sampleInBackground()
    }

    func toggle() {
        switch displayState {
        case .docked:
            expand(graceDuration: 3.0)
        case .expanded:
            collapse()
        }
    }

    /// 显式收起灵动岛（带防抖冷却并即刻重置保护期）
    func collapse() {
        guard displayState == .expanded else { return }
        if CommandLine.arguments.contains("--keep-expanded") { return }
        cancelPendingTasks()
        manualOpenGraceUntil = .distantPast
        expandCooldownUntil = Date().addingTimeInterval(0.4)
        displayState = .docked
        HapticFeedback.perform(.levelChange)
    }

    func expandFromHover() {
        guard displayState == .docked, !isDragging, Date() >= dragCooldownUntil else { return }
        cancelPendingTasks()
        manualOpenGraceUntil = .distantPast
        displayState = .expanded
        HapticFeedback.perform(.alignment)
        engine.sampleInBackground()
    }

    // MARK: - 边缘与光标监控

    private var mouseLocalMonitor: Any?
    private var mouseGlobalMonitor: Any?
    private var clickLocalMonitor: Any?
    private var clickGlobalMonitor: Any?
    private var keyEscapeMonitor: Any?
    private var edgeZoneTimer: Timer?

    /// 鼠标移动节流器。鼠标事件可达数百 Hz，且全局监听回调不在主 actor 上——
    /// 此前每个事件都新建一个 Task 跳主线程做全量边缘判定（NSScreen / panel.frame 重算），
    /// 实测快速移动鼠标 ≈ +0.6% CPU。这里用带锁时间戳做零分配预筛，
    /// 只有通过节流的事件才进入主线程。0.05s（20Hz）不影响「进入热区立即展开」的体感：
    /// 光标停在热区内时后续事件仍会通过，最坏延迟 ≈ 一个节流窗口。
    final class MouseMoveThrottle: @unchecked Sendable {
        private let lock = NSLock()
        private var lastPass = Date.distantPast
        private let minInterval: TimeInterval
        private var proximityRect: NSRect = .zero
        private var isExpanded: Bool = false

        init(minInterval: TimeInterval) { self.minInterval = minInterval }

        func updateTarget(proximityRect: NSRect, isExpanded: Bool) {
            lock.lock()
            self.proximityRect = proximityRect
            self.isExpanded = isExpanded
            lock.unlock()
        }

        func shouldPass(mouseLocation: NSPoint, now: Date = Date()) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard now.timeIntervalSince(lastPass) >= minInterval else { return false }
            // 处于收起态（docked）时，若光标不在边缘感知区（细条邻域 40pt），零开销丢弃，防全屏鼠标 Task 轰炸
            if !isExpanded && !proximityRect.isEmpty && !proximityRect.contains(mouseLocation) {
                return false
            }
            lastPass = now
            return true
        }

        func shouldPassLocal(now: Date = Date()) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard now.timeIntervalSince(lastPass) >= minInterval else { return false }
            lastPass = now
            return true
        }
    }

    let mouseMoveThrottle = MouseMoveThrottle(minInterval: 0.05)

    private func startEdgeZoneMonitor() {
        guard edgeZoneTimer == nil else { return }
        installEdgeZoneTimer(interval: Self.edgeZoneFastInterval)

        // 2. 本地与全局鼠标移动监听（共用同一个节流器，避免同一物理移动被两条通道各算一次）
        let throttle = mouseMoveThrottle
        mouseLocalMonitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved]) { [weak self] event in
            guard throttle.shouldPassLocal() else { return event }
            _ = MainActor.assumeIsolated {
                self?.evaluateEdgeZone()
            }
            return event
        }
        mouseGlobalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved]) { [weak self] _ in
            let loc = NSEvent.mouseLocation
            // 边缘感知区零开销预筛：屏幕中央 99.9% 的鼠标移动直接跳过，零 Task 分配、零主线程派发
            guard throttle.shouldPass(mouseLocation: loc) else { return }
            Task { @MainActor [weak self] in
                self?.evaluateEdgeZone()
            }
        }

        // 3. 点击外部区域即刻收起（全局与应用内失焦收起）。
        // 全局路径（点击其他 App）：立即收起。本地路径（点击本 App 的其他窗口：
        // 菜单栏 popover / tooltip / 设置）：监听只做预筛，collapse 延迟到 Task
        // 复核（该 Task 在 mouseDown 处理后、mouseUp 前执行）——真正的防误伤
        // 是复核守卫：manualOpenGraceUntil（popover 导航 3s 保护期）与浮层判定
        // （isMouseInsidePanelOrFloatingLayers 覆盖 MenuBarExtraWindow）。
        // 立即收起会让「菜单栏 popover 的收起侧边栏」翻面成展开、让 popover 里
        // 点击 Agent 行下钻的详情页一闪而过
        clickGlobalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self,
                      self.displayState == .expanded,
                      !self.isDragging,
                      !Self.isMouseInsidePanelOrFloatingLayers(self.panel) else { return }
                self.collapse()
            }
        }
        // 4. 全键盘交互与快捷键导航（Esc 逐级返回 / 方向键选择 / Enter 详情 / ⌘R 刷新 / ⌘, 设置）
        keyEscapeMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown]) { [weak self] event in
            var handled = false
            MainActor.assumeIsolated {
                guard let self,
                      self.displayState == .expanded,
                      !self.isDragging else { return }

                // 快捷键: ⌘R (立即重新采样刷新)
                if event.modifierFlags.contains(.command), event.keyCode == 15 {
                    self.engine.sampleInBackground()
                    handled = true
                    return
                }

                // 快捷键: ⌘, (打开偏好设置)
                if event.modifierFlags.contains(.command), event.keyCode == 43 {
                    NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
                    handled = true
                    return
                }

                // Esc: 逐级返回上一层；若已在主列表则收起面板
                if event.keyCode == 53 {
                    if self.route != .list {
                        self.stepBackRoute()
                    } else {
                        self.collapse()
                    }
                    handled = true
                    return
                }

                // 方向键上下移动与回车下钻详情（仅主列表下启用）
                if self.route == .list {
                    if event.keyCode == 125 { // Down
                        self.moveFocus(step: 1)
                        handled = true
                        return
                    } else if event.keyCode == 126 { // Up
                        self.moveFocus(step: -1)
                        handled = true
                        return
                    } else if event.keyCode == 36 { // Return / Enter
                        if let fid = self.focusedAgentId {
                            self.openAgentDetail(fid)
                            handled = true
                            return
                        }
                    }
                }
            }
            return handled ? nil : event
        }

        clickLocalMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] event in
            MainActor.assumeIsolated {
                guard let self,
                      self.displayState == .expanded,
                      !self.isDragging,
                      !Self.isMouseInsidePanel(self.panel) else { return }
                Task { @MainActor [weak self] in
                    guard let self,
                          self.displayState == .expanded,
                          !self.isDragging,
                          Date() >= self.manualOpenGraceUntil,
                          !Self.isMouseInsidePanelOrFloatingLayers(self.panel) else { return }
                    self.collapse()
                }
            }
            return event
        }
    }

    /// 返回本次判定是否「活跃」（光标在热区/面板内或展开态）——R35/G2 的 Timer
    /// 降档依据；鼠标事件调用方不关心返回值
    @discardableResult
    private func evaluateEdgeZone() -> Bool {
        guard didShowOnce else { return false }
        guard !isDragging else { return true }   // 拖拽视为活跃（保持快档）
        let mousePressed = NSEvent.pressedMouseButtons != 0
        guard !mousePressed else { return true }   // 按压视为活跃

        let loc = NSEvent.mouseLocation
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return false }
        let visible = screen.visibleFrame
        var active = false

        if displayState == .docked {
            guard Date() >= expandCooldownUntil, Date() >= dragCooldownUntil else { return true }
            var inZone = false
            switch dockEdge {
            case .right:
                // 右边缘细条检测：光标位于屏幕右边缘微区域且在细条垂直范围内
                let cy = savedRightY.map { min(max($0, visible.minY + 60), visible.maxY - 60) } ?? visible.midY
                if loc.x >= visible.maxX - (IslandMetrics.rightSliverWidth + 12) {
                    inZone = abs(loc.y - cy) <= (IslandMetrics.rightSliverHeight / 2 + 12)
                }
            case .left:
                // 左边与右边保持对称：仅在可见区域最左侧的细条邻域内响应。
                let cy = savedRightY.map { min(max($0, visible.minY + 60), visible.maxY - 60) } ?? visible.midY
                if loc.x >= visible.minX,
                   loc.x <= visible.minX + IslandMetrics.rightSliverWidth + 12 {
                    inZone = abs(loc.y - cy) <= (IslandMetrics.rightSliverHeight / 2 + 12)
                }
            case .top:
                // 顶边缘细条检测：光标位于屏幕顶部边缘微区域且在细条水平范围内。
                // 上界必须钳到 visible.maxY（R23）：无上界时菜单栏内整条 x 跨度都是
                // 展开热区——光标停在菜单栏（面板外）触发展开，随即「不在面板内」
                // 触发 0.5s 收起，再触发展开……无限振荡
                let cx = savedTopX.map { min(max($0, visible.minX + 70), visible.maxX - 70) } ?? visible.midX
                if loc.y >= visible.maxY - (IslandMetrics.topSliverHeight + 12),
                   loc.y <= visible.maxY {
                    inZone = abs(loc.x - cx) <= (IslandMetrics.topSliverWidth / 2 + 12)
                }
            case .bottom:
                // 底边不延伸进 Dock 外的不可见区域，避免鼠标停在 Dock 时反复展开/收起。
                let cx = savedTopX.map { min(max($0, visible.minX + 70), visible.maxX - 70) } ?? visible.midX
                if loc.y >= visible.minY,
                   loc.y <= visible.minY + IslandMetrics.topSliverHeight + 12 {
                    inZone = abs(loc.x - cx) <= (IslandMetrics.topSliverWidth / 2 + 12)
                }
            }
            active = inZone
            if inZone {
                cancelPendingTasks()
                manualOpenGraceUntil = .distantPast
                displayState = .expanded
            }
        } else if displayState == .expanded {
            if CommandLine.arguments.contains("--keep-expanded") {
                return true
            }
            if Self.isMouseInsidePanelOrFloatingLayers(panel) {
                // 光标在面板或其浮层（tooltip popover / 菜单栏 popover）内：
                // 即刻解除手动展开保护期，取消任何收起计划
                active = true
                manualOpenGraceUntil = .distantPast
                if collapseTask != nil {
                    collapseTask?.cancel()
                    collapseTask = nil
                }
            } else {
                // 光标离开面板，且超过了保护期，安排收起
                guard Date() >= manualOpenGraceUntil else { return true }
                scheduleCollapse()
            }
        }
        updateClickThrough()
        return active
    }

    /// 收起态细条热区之外的点击穿透（U3 幽灵命中区修复）。
    /// 边框透明 NSPanel 的透明区域仍参与命中测试：top 贴边时收起窗口自菜单栏
    /// 向下延伸整卡高度（挡住菜单栏与顶部内容），right 贴边也有一列竖向盲区
    ///（lldb 实测：细条外 213pt 处 hitTest 命中 NSHostingView）。
    /// 不改窗口几何（动画与布局零回归风险），改为动态切换 ignoresMouseEvents：
    /// 光标在「细条可视矩形 ±8pt」内才接收事件，其余全部穿透。hover 展开主
    /// 路径由 evaluateEdgeZone 的全局光标判定驱动，不受穿透影响。
    /// 注意接收区必须按 dockEdge 计算细条矩形——panel.frame 是整卡尺寸，
    /// 用它判定恒为 no-op（首轮验收实测打回项）
    private func updateClickThrough() {
        let shouldIgnore: Bool
        if displayState == .docked,
           let screen = panel.screen ?? Self.screenContainingMouse() {
            let hitRect = sliverRect(for: screen).insetBy(dx: -8, dy: -8)
            shouldIgnore = !hitRect.contains(NSEvent.mouseLocation)
        } else {
            shouldIgnore = false
        }
        if panel.ignoresMouseEvents != shouldIgnore {
            panel.ignoresMouseEvents = shouldIgnore
        }
    }

    // MARK: - 收回

    private func scheduleCollapse() {
        guard displayState == .expanded, !isDragging else { return }
        if collapseTask != nil { return }
        // 任务体内实时读 collapseDelay（R32/F10）：sleep 用创建时刻值无妨，
        // 执行判定读新值——设置页拖动滑杆后 ≤5s 窗口内的收起不再按旧延迟执行
        let delay = collapseDelay
        collapseGeneration &+= 1
        let generation = collapseGeneration
        collapseTask = Task { [weak self] in
            defer {
                // 只有仍是本次任务的引用时才清空
                if let self, self.collapseGeneration == generation { self.collapseTask = nil }
            }
            let now0 = Date()
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self,
                  !Task.isCancelled,
                  !self.isDragging,
                  self.displayState == .expanded,
                  Date() >= self.manualOpenGraceUntil,
                  !Self.isMouseInsidePanelOrFloatingLayers(self.panel) else { return }
            // 设置页在挂起期间改过延迟：执行时点以新值为准重新校验是否已到期
            if CommandLine.arguments.contains("--keep-expanded") { return }
            let effectiveDelay = self.collapseDelay
            if Date().timeIntervalSince(now0) < effectiveDelay { return }
            self.expandCooldownUntil = Date().addingTimeInterval(0.35)
            self.displayState = .docked
        }
    }

    static func isMouseInsidePanel(_ panel: NSPanel?) -> Bool {
        guard let panel, panel.isVisible else { return false }
        let frame = panel.frame.insetBy(dx: -8, dy: -8)
        return frame.contains(NSEvent.mouseLocation)
    }

    /// 光标是否在面板或其派生浮层内。
    /// tooltip popover（SwiftUI .popover）与菜单栏 popover 渲染在面板 frame 之外——
    /// hover 移入浮层若判为「离开面板」会触发自动收起（默认 0.5s），浮层里的
    /// 两段式终止确认与直达按钮实际只有半秒可用窗口。
    /// 浮层窗口类名活体枚举（lldb 附着实证）：tooltip = `_NSPopoverWindow`、
    /// 菜单栏 popover = `MenuBarExtraWindow<AnyView>`。状态项 `NSStatusBarWindow`
    /// 只是宿主，不属于交互浮层——其 frame 可能覆盖远大于图标的区域，纳入会让
    /// 面板持续误判为悬停、永远不收起。
    static func isMouseInsidePanelOrFloatingLayers(_ panel: NSPanel?) -> Bool {
        if isMouseInsidePanel(panel) { return true }
        let mouse = NSEvent.mouseLocation
        return NSApp.windows.contains { window in
            let cls = String(describing: type(of: window))
            return IslandPanelInteraction.isMouseInsideFloatingLayer(
                className: cls,
                isVisible: window.isVisible,
                containsMouse: window.frame.contains(mouse)
            )
        }
    }

    func cancelPendingTasks() {
        collapseTask?.cancel()
        peekTask?.cancel()
        routeResetTask?.cancel()
        collapseTask = nil
        peekTask = nil
        routeResetTask = nil
    }

    // MARK: - Peek

    private func handleTaskEvent(_ event: AgentTaskEvent?) {
        // 代际守卫（R32/F5）：乱序时旧事件处理（音效/横幅展开/peek 时长）会覆盖
        // 新事件已建立的语义。nil 侧对称：清空处理仅当当前确实无事件
        if let event, engine.latestEvent?.id != event.id { return }
        if event == nil, engine.latestEvent != nil { return }
        guard let event else {
            eventBannerExpanded = false
            syncExpandedHeight()
            return
        }

        // 事件类型决定初始展开态：熔断类严重告警默认展开详情以提供排查指导，
        // 其他事件收起。此前只在 costSpike 时置 true、从不复位，导致一次告警后
        // 后续所有完成事件也保持 142pt 的展开高度。
        eventBannerExpanded = (event.eventType == .costSpike)

        // 1. 通知与声音由 taskEvents 独立投递；这里仅维护岛内横幅/Peek。

        // 2. 窗口高度自适应扩展
        syncExpandedHeight()

        // 3. 若当前处于收起态（docked），或 peek 进行中（expanded 来自上一次 peek），
        // 按分级策略决定是否触发/重排微弹窗 Peek
        if notificationPolicy.shouldPeek(for: event.eventType) {
            if displayState == .docked || peekTask != nil {
                peekForEvent(event)
            }
        }
    }

    /// 系统通知与唯一提示音出口。与 latestEvent 横幅解耦后，同拍多 Agent attention
    /// 不会因单槽位覆盖和代际守卫而只送达最后一个。
    private func deliverTaskEventAlert(_ event: AgentTaskEvent) {
        CompletionNotification.post(for: event, policy: notificationPolicy)
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
    }

    private func peekForEvent(_ event: AgentTaskEvent) {
        guard !isDragging else { return }
        let isPeekOngoing = peekTask != nil
        guard displayState == .docked || isPeekOngoing else { return }
        // 连发事件重排：peek 进行中来了新事件（如 costSpike 6s 跟在 completed 3.5s 后），
        // 取消旧 peek 按新事件时长重排——单任务槽会把更严重告警的展示时长吞成前一个
        // 事件的剩余时间（最严重告警可能只 peek 不到 1s 就缩回）。
        // peekGeneration 代际机制保证被取消的旧任务安全退出、不清掉新任务引用
        peekTask?.cancel()
        peekTask = nil
        // 普通事件 3.5 秒，成本/死循环熔断保持 6 秒供用户查看或操作
        let peekDuration: TimeInterval = event.eventType == .costSpike ? 6.0 : 3.5
        peekGeneration &+= 1
        let generation = peekGeneration
        peekTask = Task { [weak self] in
            defer {
                // 同 collapseTask：被取消的旧任务不得清掉新任务的引用
                if let self, self.peekGeneration == generation { self.peekTask = nil }
            }
            guard let self, !self.isDragging else { return }
            // 取消竞窗：被重排取消的旧任务可能已越过挂起点，覆写 grace 会把
            // hover 展开后的自动收起推迟一整个 peek 时长
            guard !Task.isCancelled else { return }
            guard self.displayState == .docked || self.displayState == .expanded else { return }

            // 走真实状态切换：窗口尺寸与 SwiftUI 内容同源（此前只动窗口 frame、displayState
            // 仍为 docked，导致 6pt 细条被拉到展开位置且卡片内容缺失）
            self.manualOpenGraceUntil = Date().addingTimeInterval(peekDuration)
            if self.displayState == .docked {
                self.displayState = .expanded
            }

            try? await Task.sleep(nanoseconds: UInt64(peekDuration * 1_000_000_000))
            guard !Task.isCancelled, self.displayState == .expanded else { return }
            // 用户光标已移入面板或其浮层（tooltip/菜单栏 popover）→ 转为常驻展开，
            // 不自动收回（与 evaluateEdgeZone 的展开保持判定同口径）
            guard !Self.isMouseInsidePanelOrFloatingLayers(self.panel) else { return }
            self.collapse()
        }
    }

    // MARK: - 状态切换

    private func onStateChanged(_ state: IslandDisplayState) {
        // 代际守卫（R32/F5）：sink 经 Task @MainActor 跳跃，MainActor 上无 FIFO
        // 保证——乱序执行时旧态处理会覆盖新态（setPresentationActive 空转开启、
        // placeWindow 按 docked 几何算动画错帧）。捕获值 != 当前值即过期
        guard state == displayState else { return }
        updateChrome()
        updateClickThrough()
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

    // MARK: - NSWindowDelegate

    func windowDidResize(_ notification: Notification) {
        hostingView?.frame = panel.contentView?.bounds ?? hostingView.frame
    }
}
