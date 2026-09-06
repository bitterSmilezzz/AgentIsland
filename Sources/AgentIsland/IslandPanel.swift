import AgentIslandCore
import AppKit
import SwiftUI
import Combine

// MARK: - 灵动岛窗口控制器（右侧侧边栏）
// 2026-09-06 由顶部悬浮岛改造（用户反馈顶部扰乱），E 系列交互语义迁移到右缘：
// - docked：整个面板滑出屏幕右缘外，完全隐藏（右缘零残条）
// - expanded：贴右缘 flush、垂直居中；鼠标触碰右缘热区（24pt 全高）滑出
// - 鼠标离开 + collapseDelay → 收回屏外
// - idle→busy：整体滑出提醒再收回（peek），30s 冷却
// - 菜单栏「展开/收起」保留；拖动定位已移除（贴边锚定，无位置状态）

@MainActor
final class IslandPanelController: NSObject, NSWindowDelegate, ObservableObject {

    @Published var displayState: IslandDisplayState = .docked
    /// 卡内导航路由（仅 expanded 时有意义）
    @Published var route: CardRoute = .list

    private var panel: NSPanel!
    private var hostingView: NSHostingView<IslandView>!
    private var shadowHost: ShadowHostView?
    private var clipContainer: NSView?
    private var engine: ActivityEngine
    private var cancellables = Set<AnyCancellable>()
    private var collapseTask: Task<Void, Never>?
    private var peekTask: Task<Void, Never>?
    /// 收起动画结束后重置导航的任务（阿证中：延迟避免同帧布局毛刺）
    private var routeResetTask: Task<Void, Never>?
    private var didShowOnce = false
    /// 菜单收起后的冷却：光标可能仍停在右缘热区，收起后不立即重展开
    private var expandCooldownUntil: Date = .distantPast
    /// docked 态重绘守卫：working 状态没变就不强制重绘（peek 滑出时内容须为最新）
    private var lastAnyWorking: Bool?

    /// 右缘热区宽度（全高条带）
    private let edgeZoneWidth: CGFloat = 24
    /// 侧边栏与屏幕上下的最小边距（垂直居中钳制）
    private let screenVerticalMargin: CGFloat = 24
    /// 鼠标离开后自动收起延迟（纯面板行为参数；持久化键 SettingKey.collapseDelay）
    private var collapseDelay: TimeInterval

    /// 设置页直调生效（与外观同一模式；持久化由设置页 @AppStorage 负责）
    func applyCollapseDelay(_ delay: TimeInterval) {
        collapseDelay = max(0.2, delay)
    }

    init(engine: ActivityEngine) {
        self.engine = engine
        // 收起延迟初始值（持久化由设置页 @AppStorage 负责；非法/缺项回落 0.5s）
        self.collapseDelay = UserDefaults.standard.object(forKey: SettingKey.collapseDelay) as? Double ?? 0.5
        super.init()
        setupPanel()
        observe()
    }

    private func setupPanel() {
        panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: cardSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.isMovable = false
        panel.isMovableByWindowBackground = false
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.animationBehavior = .none
        panel.delegate = self
        panel.isRestorable = false
        applyAppearance(Self.persistedAppearance())

        let content = IslandView(
            engine: engine,
            controller: self
        )
        hostingView = NSHostingView(rootView: content)
        hostingView.frame = NSRect(origin: .zero, size: cardSize)

        // 双层结构（H1 修复）：
        // 外层 shadowHost 不裁剪，负责画圆角阴影（shadowPath 跟随圆角，随尺寸更新）；
        // 内层 container 用 masksToBounds 裁剪内容为圆角——单层 + masksToBounds 会把
        // SwiftUI 阴影也裁掉（之前阴影从未显示）。
        let container = NSView(frame: NSRect(origin: .zero, size: cardSize))
        hostingView.autoresizingMask = [.width, .height]
        container.addSubview(hostingView)
        container.wantsLayer = true
        container.layer?.masksToBounds = true
        container.layer?.cornerCurve = .continuous

        let shadowHost = ShadowHostView(frame: NSRect(origin: .zero, size: cardSize))
        shadowHost.wantsLayer = true
        container.autoresizingMask = [.width, .height]
        shadowHost.addSubview(container)
        panel.contentView = shadowHost
        self.shadowHost = shadowHost
        self.clipContainer = container
        setupChrome()

        $displayState
            .removeDuplicates()
            .sink { [weak self] state in
                Task { @MainActor [weak self] in
                    self?.onStateChanged(state)
                }
            }
            .store(in: &cancellables)

        // 卡内导航切换 → 重新计算窗口高度
        $route
            .removeDuplicates()
            .dropFirst()
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in
                    self?.syncExpandedHeight()
                }
            }
            .store(in: &cancellables)
    }

    /// 贴边造型：右侧两角为直角（flush 屏幕右缘），左侧两角 18pt 圆角
    private func setupChrome() {
        clipContainer?.layer?.cornerRadius = Theme.radiusLg
        clipContainer?.layer?.maskedCorners = [.layerMinXMinYCorner, .layerMinXMaxYCorner]
        shadowHost?.setShadow(enabled: true, cornerRadius: Theme.radiusLg)
    }

    /// 应用外观设置（浅色/深色/跟随系统）到悬浮面板（设置页直调，带 payload）
    func applyAppearance(_ mode: IslandAppearance) {
        panel.appearance = mode.nsAppearance
    }

    /// 启动时的持久化外观（键唯一来源 SettingKey；运行期变更由设置页直调，不回读）
    private static func persistedAppearance() -> IslandAppearance {
        IslandAppearance(rawValue: UserDefaults.standard.string(forKey: SettingKey.islandAppearance) ?? "") ?? .system
    }

    private func observe() {
        // E4：idle→busy 侧边栏滑出提醒一下
        engine.$anyWorking
            .dropFirst()
            .removeDuplicates()
            .sink { [weak self] working in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if working {
                        Self.log("anyWorking=true → peek")
                        self.peek()
                    }
                }
            }
            .store(in: &cancellables)

        // 采样后：刷新内容 + 展开时同步高度
        engine.$updatedAt
            .sink { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    // docked 态内容只依赖 anyWorking：working 状态没变就跳过整窗重绘
                    //（peek 滑出时依赖此重绘保证内容最新）
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
        // 事件驱动补充：启动时鼠标可能已静止在热区（无 mouseMoved 事件），补一次评估；
        // 屏幕拓扑变化（插拔外接屏）后光标位置可能跳变，同样补一次
        NotificationCenter.default.addObserver(
            self, selector: #selector(screenConfigChanged),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)
        Task { @MainActor [weak self] in
            self?.evaluateEdgeZone()
        }
        // 启动即隐藏（滑出屏外）
        // 注意：controller 可能在 SwiftUI 场景求值阶段被创建，那时 NSScreen 未就绪、
        // placeWindow 会静默失败；这里强制重新定位一次。
        displayState = .docked
        // 显式调用是双保险：displayState 赋值经 sink 也会触发 onStateChanged(.docked)→失活，
        // 但 sink 无 removeDuplicates 且首帧时序不稳；setPresentationActive 幂等，重复调用无代价
        engine.setPresentationActive(false)
        placeWindow(animated: false)
        panel.orderFrontRegardless()
    }

    @objc private func screenConfigChanged() {
        Task { @MainActor [weak self] in
            self?.evaluateEdgeZone()
        }
    }

    deinit {
        if let m = mouseLocalMonitor { NSEvent.removeMonitor(m) }
        if let m = mouseGlobalMonitor { NSEvent.removeMonitor(m) }
        NotificationCenter.default.removeObserver(self)
    }

    /// 手动切换（菜单栏命令）：隐藏 ↔ 侧边栏
    func toggle() {
        switch displayState {
        case .docked:
            cancelPendingTasks()
            displayState = .expanded
        case .expanded:
            displayState = .docked
            // 菜单「收起」时光标可能仍停在右缘热区，冷却 1.2s 防止立即重展开
            expandCooldownUntil = Date().addingTimeInterval(1.2)
        }
    }

    // MARK: - 右缘触发区监控
    // 事件驱动（替代 0.2s 永久轮询）：仅鼠标移动时才评估，空闲零开销。
    // local monitor 只收本应用前台事件；global monitor 兜底非前台（常驻菜单栏应用常态）。
    // 展开后收起由 collapseTask 驱动，无需持续轮询。

    private var mouseLocalMonitor: Any?
    private var mouseGlobalMonitor: Any?
    /// 评估节流：事件可能 60Hz 涌入，限制实际评估频率
    private var lastEvalAt = Date.distantPast
    private let evalMinInterval: TimeInterval = 0.1   // 最大 10Hz，悬停响应足够
    /// 日志去重：状态没变化不打 tick（打日志本身有文件 I/O 开销）
    private var lastLoggedTick: String?

    private func startEdgeZoneMonitor() {
        guard mouseLocalMonitor == nil else { return }
        // local monitor 回调在主线程，直接节流评估，不再每事件 spawn Task
        mouseLocalMonitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved]) { [weak self] event in
            self?.onMouseMoved()
            return event
        }
        // global monitor 回调在后台线程，需 hop 回主线程
        mouseGlobalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved]) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.onMouseMoved()
            }
        }
    }

    private func onMouseMoved() {
        let now = Date()
        guard now.timeIntervalSince(lastEvalAt) >= evalMinInterval else { return }
        lastEvalAt = now
        evaluateEdgeZone()
    }

    /// 事件驱动：触碰右缘 → 滑出侧边栏；离开 → 收回屏外（D1 离开即收，不区分忙闲）
    private func evaluateEdgeZone() {
        guard didShowOnce else { return }
        // 按下鼠标（选择/滚动中）不触发展开
        let mousePressed = NSEvent.pressedMouseButtons != 0
        guard !mousePressed else { return }
        let inZone = Self.isMouseInRightZone(zoneWidth: edgeZoneWidth)
        // 日志去重：仅在状态组合变化时记录（高频事件下避免每拍写盘）
        let sig = "\(inZone)|\(displayState)|\(engine.anyWorking)"
        if sig != lastLoggedTick {
            lastLoggedTick = sig
            Self.log("tick inZone=\(inZone) state=\(displayState) anyWorking=\(engine.anyWorking)")
        }

        if inZone {
            if displayState == .docked {
                guard Date() >= expandCooldownUntil else { return }
                // 统一取消 peek/collapse 任务，避免与滑出动画并发改同一 frame
                cancelPendingTasks()
                Self.log("edgezone → expand")
                displayState = .expanded
            }
        } else if displayState == .expanded {
            scheduleCollapse()
        }
    }

    // MARK: - 收回

    private func scheduleCollapse() {
        guard displayState == .expanded else { return }
        if collapseTask != nil { return } // 幂等
        let delay = collapseDelay
        Self.log("collapse scheduled, delay \(delay)")
        collapseTask = Task { [weak self] in
            defer { self?.collapseTask = nil }
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self,
                  !Task.isCancelled,
                  displayState == .expanded,
                  // 悬停标记可能因页面切换重建视图而短暂丢失（onHover 发 false），
                  // 故同时检查鼠标真实位置：仍在面板内或右缘热区就不收
                  !Self.isMouseInsidePanel(panel),
                  !Self.isMouseInRightZone(zoneWidth: self.edgeZoneWidth) else { return }
            Self.log("collapse → docked")
            self.displayState = .docked
        }
    }

    /// 鼠标是否在面板窗口内（屏幕坐标，含 2pt 余量）
    static func isMouseInsidePanel(_ panel: NSPanel?) -> Bool {
        guard let panel, panel.isVisible else { return false }
        let frame = panel.frame.insetBy(dx: -2, dy: -2)
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

    // MARK: - Peek（E4：忙起来时滑出提醒一下）

    /// peek 冷却：30s 内最多弹一次（防 working↔idle 抖动时连续滑出）
    private var lastPeekAt: Date = .distantPast

    private func peek() {
        guard displayState == .docked, peekTask == nil else { return }
        guard Date().timeIntervalSince(lastPeekAt) >= 30 else { return }
        lastPeekAt = Date()
        peekTask = Task { [weak self] in
            defer { self?.peekTask = nil }
            guard let self, let panel = self.panel else { return }
            let hidden = panel.frame
            // 整体滑到可见位（贴右缘垂直居中）停留 0.5s 再收回
            guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
            let visible = screen.visibleFrame
            let height = min(hidden.height, visible.height - self.screenVerticalMargin * 2)
            let shown = NSRect(x: visible.maxX - hidden.width,
                               y: visible.midY - height / 2,
                               width: hidden.width, height: height)
            await withCheckedContinuation { cont in
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.24
                    panel.animator().setFrame(shown, display: true)
                }, completionHandler: { cont.resume() })
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
            guard !Task.isCancelled, self.displayState == .docked else { return }
            await withCheckedContinuation { cont in
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.22
                    panel.animator().setFrame(hidden, display: true)
                }, completionHandler: { cont.resume() })
            }
        }
    }

    // MARK: - 状态切换

    private func onStateChanged(_ state: IslandDisplayState) {
        Self.log("onStateChanged → \(state)")
        switch state {
        case .docked:
            // 收起时延迟到动画结束后重置导航（阿证中：与收起动画同帧重排会造成
            // 主线程 0.3s 布局毛刺）；无动画事务防 route 0.35s spring 闪现列表内容
            routeResetTask?.cancel()
            routeResetTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: 450_000_000)   // 0.42s 收起动画 + 余量
                guard let self, !Task.isCancelled, self.displayState == .docked else { return }
                var t = Transaction(animation: nil)
                t.disablesAnimations = true
                withTransaction(t) { self.route = .list }
            }
            engine.setPresentationActive(false)   // 隐藏态无展示需求：呈现失活，暂停轮询省电
            placeWindow(animated: true)
        case .expanded:
            engine.setPresentationActive(true)   // 呈现活跃：启动/恢复轮询并立即刷新
            panel.orderFrontRegardless()
            placeWindow(animated: true)
        }
    }

    // MARK: - 屏幕检测

    static func screenContainingMouse() -> NSScreen? {
        let loc = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(loc) } ?? NSScreen.main
    }

    static func isMouseInRightZone(zoneWidth: CGFloat) -> Bool {
        guard let screen = screenContainingMouse() else { return false }
        let loc = NSEvent.mouseLocation
        return loc.x >= screen.frame.maxX - zoneWidth
    }

    // MARK: - 窗口帧

    /// 面板恒为卡尺寸（docked/expanded 只差位置，滑入滑出只动 x）
    private var cardSize: NSSize {
        NSSize(width: IslandMetrics.cardWidth, height: expandedHeight())
    }

    // MARK: - 展开高度
    // 度量与推导全部在 IslandMetrics（唯一事实来源），此处只装配输入

    private func expandedHeight() -> CGFloat {
        IslandMetrics.expandedHeight(route: route, visibleCount: visibleCount(), hasSummary: !engine.grandTotal.isEmpty)
    }

    /// 可见行数（统一走 engine.visibleSnapshots，口径单一实现）
    private func visibleCount() -> Int {
        engine.visibleSnapshots.count
    }

    /// 面板目标位置：expanded 贴右缘 flush、垂直居中；docked 完全滑出屏外（+2pt 防残条）。
    /// docked/expanded 尺寸相同（卡尺寸），滑入滑出只动 x——动画天然水平。
    private func placeWindow(animated: Bool) {
        guard let screen = panel.screen ?? Self.screenContainingMouse() else { return }
        let visible = screen.visibleFrame
        var size = NSSize(width: IslandMetrics.cardWidth, height: expandedHeight())
        size.height = min(size.height, visible.height - screenVerticalMargin * 2)
        let y = visible.midY - size.height / 2
        let target: NSRect
        switch displayState {
        case .expanded:
            target = NSRect(x: visible.maxX - size.width, y: y, width: size.width, height: size.height)
        case .docked:
            target = NSRect(x: visible.maxX + 2, y: y, width: size.width, height: size.height)
        }
        if animated {
            let duration: TimeInterval = 0.42   // 与内容 spring(response: 0.42) 对齐
            NSAnimationContext.runAnimationGroup { context in
                context.duration = duration
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrame(target, display: true)
            }
        } else {
            panel.setFrame(target, display: false)
        }
    }

    private func syncExpandedHeight() {
        if abs(panel.frame.height - expandedHeight()) > 1 {
            placeWindow(animated: true)
        }
    }

    // MARK: - 调试日志

    static let debugEnabled = ProcessInfo.processInfo.environment["AGENTISLAND_DEBUG"] == "1"
    static func log(_ message: String) {
        guard debugEnabled else { return }
        let line = "[\(Date().timeIntervalSince1970)] \(message)\n"
        if let data = line.data(using: .utf8),
           let handle = FileHandle(forWritingAtPath: "/tmp/agentisland.log") {
            handle.seekToEndOfFile()
            handle.write(data)
            try? handle.close()
        } else {
            try? line.data(using: .utf8)?.write(to: URL(fileURLWithPath: "/tmp/agentisland.log"))
        }
    }

    // MARK: - NSWindowDelegate

    func windowDidResize(_ notification: Notification) {
        hostingView?.frame = panel.contentView?.bounds ?? hostingView.frame
    }
}

// MARK: - 阴影宿主视图（H1：阴影不被 masksToBounds 裁剪）
// 外层不裁剪，负责画圆角阴影（仅左侧两角，右缘 flush）；内层容器裁剪内容。
// shadowPath 跟随圆角并随尺寸变化更新。

final class ShadowHostView: NSView {
    private var shadowEnabled = false
    private var cornerRadius: CGFloat = Theme.radiusLg

    override var frame: NSRect {
        didSet { updateShadowPath() }
    }

    override func layout() {
        super.layout()
        updateShadowPath()
    }

    func setShadow(enabled: Bool, cornerRadius: CGFloat) {
        self.cornerRadius = cornerRadius
        guard let layer else { return }
        layer.shadowColor = NSColor.black.cgColor
        // 阴影随窗口 frame 动画同步淡入/淡出（0.42s easeInEaseOut），避免"啪"地突现
        CATransaction.begin()
        CATransaction.setAnimationDuration(0.42)
        CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeInEaseOut))
        layer.shadowOpacity = enabled ? Theme.panelShadowOpacity : 0
        layer.shadowRadius = enabled ? Theme.panelShadowRadius : 0
        layer.shadowOffset = CGSize(width: 0, height: Theme.panelShadowOffsetY)
        CATransaction.commit()
        updateShadowPath()
    }

    private func updateShadowPath() {
        guard let layer else { return }
        if layer.shadowOpacity > 0 {
            layer.shadowPath = Self.leftRoundedPath(bounds: bounds, radius: cornerRadius)
        } else {
            layer.shadowPath = nil
        }
    }

    /// 仅左侧两角圆角的路径（右缘 flush 直角）
    private static func leftRoundedPath(bounds: NSRect, radius: CGFloat) -> CGPath {
        let r = min(radius, bounds.height / 2)
        let path = CGMutablePath()
        path.move(to: CGPoint(x: bounds.minX, y: bounds.minY + r))
        path.addArc(center: CGPoint(x: bounds.minX + r, y: bounds.minY + r), radius: r,
                    startAngle: .pi, endAngle: .pi * 1.5, clockwise: false)   // 左下角
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.minY))
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.maxY))   // 右缘直线
        path.addLine(to: CGPoint(x: bounds.minX + r, y: bounds.maxY))
        path.addArc(center: CGPoint(x: bounds.minX + r, y: bounds.maxY - r), radius: r,
                    startAngle: .pi * 1.5, endAngle: .pi * 2, clockwise: false)   // 左上角
        path.closeSubpath()
        return path
    }
}
