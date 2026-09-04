import AgentIslandCore
import AppKit
import SwiftUI
import Combine

// MARK: - 灵动岛窗口控制器
// QQ 式贴边隐藏（E1–E4 决策）：
// - docked：176×5 细条贴在菜单栏下缘（或用户拖动的锚点处），始终可见、几乎不占地方
// - expanded：280×N 卡片，鼠标触碰顶部热区 / 悬停细条时直接滑出
// - 鼠标离开 + collapseDelay → 收回细条（不区分忙闲）
// - idle→busy：细条「弹一下」提醒（peek），配合菜单栏绿点
// - 拖动：卡片顶栏可拖，位置记忆（细条悬停即展开，故只在卡片上拖）

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
    private var dragStartOrigin: NSPoint?
    private var dragCooldownUntil: Date = .distantPast
    /// docked 态重绘守卫：working 状态没变就不强制重绘细条
    private var lastAnyWorking: Bool?

    // 尺寸常量
    private let dockedSize = NSSize(width: 176, height: 5)
    private let expandedWidth: CGFloat = 280
    private let expandedMaxHeight: CGFloat = 420
    private let topZoneHeight: CGFloat = 80

    // D3：拖动锚点（顶部中心），UserDefaults 记忆
    private var savedCenterX: CGFloat?
    private var savedTopY: CGFloat?
    private let anchorXKey = "islandAnchorCenterX"
    private let anchorYKey = "islandAnchorTopY"

    init(engine: ActivityEngine) {
        self.engine = engine
        super.init()
        // 恢复锚点（校验必须落在某块屏幕内，否则丢弃走默认位）
        let ud = UserDefaults.standard
        let cx = ud.object(forKey: anchorXKey) as? CGFloat
        let cy = ud.object(forKey: anchorYKey) as? CGFloat
        if let cx, let cy,
           NSScreen.screens.contains(where: { screen in
               cx >= screen.frame.minX && cx <= screen.frame.maxX
                   && cy >= screen.frame.minY && cy <= screen.frame.maxY
           }) {
            savedCenterX = cx
            savedTopY = cy
        }
        setupPanel()
        observe()
    }

    private func setupPanel() {        panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: dockedSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        // 拖动由 SwiftUI DragGesture 驱动（卡片顶栏），禁用 AppKit 原生拖动避免双通道冲突
        panel.isMovable = false
        panel.isMovableByWindowBackground = false
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.animationBehavior = .none
        panel.delegate = self
        panel.isRestorable = false
        applyAppearance()

        // 设置窗口切换外观时实时生效（显式通知，可靠）
        NotificationCenter.default.publisher(for: .islandAppearanceChanged)
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in
                self?.applyAppearance()
            }
            .store(in: &cancellables)

        let content = IslandView(
            engine: engine,
            controller: self
        )
        hostingView = NSHostingView(rootView: content)
        hostingView.frame = NSRect(origin: .zero, size: dockedSize)

        // 双层结构（H1 修复）：
        // 外层 shadowHost 不裁剪，负责画圆角阴影（shadowPath 跟随圆角，随尺寸更新）；
        // 内层 container 用 masksToBounds 裁剪内容为圆角——单层 + masksToBounds 会把
        // SwiftUI 阴影也裁掉（之前阴影从未显示）。
        let container = NSView(frame: NSRect(origin: .zero, size: dockedSize))
        hostingView.autoresizingMask = [.width, .height]
        container.addSubview(hostingView)
        container.wantsLayer = true
        container.layer?.masksToBounds = true
        container.layer?.cornerCurve = .continuous

        let shadowHost = ShadowHostView(frame: NSRect(origin: .zero, size: dockedSize))
        shadowHost.wantsLayer = true
        container.autoresizingMask = [.width, .height]
        shadowHost.addSubview(container)
        panel.contentView = shadowHost
        self.shadowHost = shadowHost
        self.clipContainer = container

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

    /// 应用外观设置（浅色/深色/跟随系统）到悬浮面板
    private func applyAppearance() {
        let raw = UserDefaults.standard.string(forKey: "islandAppearance")
        let mode = IslandAppearance(rawValue: raw ?? "") ?? .system
        panel.appearance = mode.nsAppearance
    }

    private func observe() {
        // E4：idle→busy 细条弹一下
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
        startTopZoneMonitor()
        // 事件驱动补充：启动时鼠标可能已静止在热区（无 mouseMoved 事件），补一次评估；
        // 屏幕拓扑变化（插拔外接屏）后光标位置可能跳变，同样补一次
        NotificationCenter.default.addObserver(
            self, selector: #selector(screenConfigChanged),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)
        Task { @MainActor [weak self] in
            self?.evaluateTopZone()
        }
        // E3：细条始终保留 —— 启动即 docked
        // 注意：controller 可能在 SwiftUI 场景求值阶段被创建，那时 NSScreen 未就绪、
        // placeWindow 会静默失败；这里强制重新定位一次。
        displayState = .docked
        engine.tokenMonitor.pause()   // 初始 docked：sink 不触发 removeDuplicates，需显式暂停
        updateWindowChrome(docked: true)
        placeWindow(size: dockedSize, animated: false)
        panel.orderFrontRegardless()
    }

    @objc private func screenConfigChanged() {
        Task { @MainActor [weak self] in
            self?.evaluateTopZone()
        }
    }

    deinit {
        if let m = mouseLocalMonitor { NSEvent.removeMonitor(m) }
        if let m = mouseGlobalMonitor { NSEvent.removeMonitor(m) }
        NotificationCenter.default.removeObserver(self)
    }

    /// 手动切换（菜单栏命令）：细条 ↔ 卡片
    func toggle() {
        switch displayState {
        case .docked:
            cancelPendingTasks()
            displayState = .expanded
        case .expanded:
            displayState = .docked
            // 菜单「收起」时鼠标在菜单栏（顶部热区内），收起后热区会立即重新展开；
            // 冷却 1.2s（与 dragEnded 同款），给用户从菜单收起的机会
            dragCooldownUntil = Date().addingTimeInterval(1.2)
        }
    }

    /// 重置岛位置到默认（菜单栏下缘居中）
    func resetPosition() {
        savedCenterX = nil
        savedTopY = nil
        UserDefaults.standard.removeObject(forKey: anchorXKey)
        UserDefaults.standard.removeObject(forKey: anchorYKey)
        placeWindow(size: sizeForState(), animated: true)
    }

    // MARK: - 顶部触发区监控
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

    private func startTopZoneMonitor() {
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
        evaluateTopZone()
    }

    /// 事件驱动：触碰顶部 → 滑出卡片；离开 → 收回细条（D1 离开即收，不区分忙闲）
    private func evaluateTopZone() {
        guard didShowOnce else { return }
        // 按下鼠标（拖动中）不触发展开
        let mousePressed = NSEvent.pressedMouseButtons != 0
        guard !mousePressed else { return }
        let inZone = Self.isMouseInTopZone(topZoneHeight: topZoneHeight)
        // 日志去重：仅在状态组合变化时记录（高频事件下避免每拍写盘）
        let sig = "\(inZone)|\(displayState)|\(engine.anyWorking)"
        if sig != lastLoggedTick {
            lastLoggedTick = sig
            Self.log("tick inZone=\(inZone) state=\(displayState) anyWorking=\(engine.anyWorking)")
        }

        if inZone {
            if displayState == .docked {
                guard Date() >= dragCooldownUntil else { return }
                // 统一取消 peek/collapse 任务，避免与 expand 动画并发改同一 frame
                cancelPendingTasks()
                Self.log("topzone → expand")
                displayState = .expanded
            }
        } else if displayState == .expanded {
            scheduleCollapse()
        }
    }

    // MARK: - 悬停

    func islandHoveredChanged(_ hovering: Bool) {
        if hovering {
            guard Date() >= dragCooldownUntil else { return }
            guard NSEvent.pressedMouseButtons == 0 else { return }
            cancelPendingTasks()
            if displayState == .docked {
                displayState = .expanded
            }
        } else if displayState == .expanded,
                  !Self.isMouseInTopZone(topZoneHeight: topZoneHeight) {
            scheduleCollapse()
        }
    }

    // MARK: - 收回细条

    private func scheduleCollapse() {
        guard displayState == .expanded else { return }
        if collapseTask != nil { return } // 幂等
        let delay = engine.config.collapseDelay
        Self.log("collapse scheduled, delay \(delay)")
        collapseTask = Task { [weak self] in
            defer { self?.collapseTask = nil }
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self,
                  !Task.isCancelled,
                  displayState == .expanded,
                  // 悬停标记可能因页面切换重建视图而短暂丢失（onHover 发 false），
                  // 故同时检查鼠标真实位置：仍在面板内或顶部热区就不收
                  !Self.isMouseInsidePanel(panel),
                  !Self.isMouseInTopZone(topZoneHeight: self.topZoneHeight) else { return }
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
        // peek 中断时先把窗口拉回细条原始位置，避免从「下探 11pt」起跳展开
        if let restore = peekRestoreFrame {
            panel?.setFrame(restore, display: false)
            peekRestoreFrame = nil
        }
        collapseTask?.cancel()
        peekTask?.cancel()
        routeResetTask?.cancel()
        collapseTask = nil
        peekTask = nil
        routeResetTask = nil
    }

    // MARK: - Peek（E4：忙起来时细条弹一下）

    /// peek 冷却：30s 内最多弹一次（防 working↔idle 抖动时细条连续弹跳）
    private var lastPeekAt: Date = .distantPast
    /// peek 开始时的窗口 frame（中断时恢复用）
    private var peekRestoreFrame: NSRect?

    private func peek() {
        guard displayState == .docked, peekTask == nil else { return }
        guard Date().timeIntervalSince(lastPeekAt) >= 30 else { return }
        lastPeekAt = Date()
        peekTask = Task { [weak self] in
            defer { self?.peekTask = nil; self?.peekRestoreFrame = nil }
            guard let self, let panel = self.panel else { return }
            let frame = panel.frame
            self.peekRestoreFrame = frame
            // 下滑 11pt 再收回（顶边固定锚点视觉：整体下探）；钳制不出屏幕下缘
            // AppKit origin.y 是窗口底边，直接钳到 visible.minY（Dock 上缘）
            let screenBottom = (panel.screen?.visibleFrame ?? NSScreen.main?.visibleFrame ?? frame).minY
            let downY = max(frame.origin.y - 11, screenBottom)
            let down = NSRect(x: frame.origin.x, y: downY,
                              width: frame.width, height: frame.height + 11)
            await withCheckedContinuation { cont in
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.16
                    panel.animator().setFrame(down, display: true)
                }, completionHandler: { cont.resume() })
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
            guard !Task.isCancelled, self.displayState == .docked else { return }
            let back = NSRect(x: frame.origin.x, y: frame.origin.y,
                              width: frame.width, height: frame.height)
            await withCheckedContinuation { cont in
                NSAnimationContext.runAnimationGroup({ ctx in
                    ctx.duration = 0.22
                    panel.animator().setFrame(back, display: true)
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
            engine.tokenMonitor.pause()   // 细条态无展示需求，暂停 60s 轮询省电
            updateWindowChrome(docked: true)
            panel.orderFrontRegardless()
            placeWindow(size: dockedSize, animated: true)
        case .expanded:
            engine.tokenMonitor.resume()   // 展开即恢复轮询并立即刷新
            updateWindowChrome(docked: false)
            panel.orderFrontRegardless()
            placeWindow(size: sizeForState(), animated: true)
        }
    }

    /// 状态相关窗口外观（H1/M1 修复）：
    /// docked 细条：小圆角（胶囊半圆）、无阴影；expanded 卡片：18pt 圆角 + 柔和投影
    private func updateWindowChrome(docked: Bool) {
        let radius: CGFloat = docked ? 2.5 : Theme.radiusLg
        clipContainer?.layer?.cornerRadius = radius
        shadowHost?.setShadow(enabled: !docked, cornerRadius: radius)
    }

    // MARK: - 拖动（卡片顶栏）

    func dragMoved(translation: CGSize) {
        Self.log("dragMoved \(translation)")
        guard let panel else { return }
        if dragStartOrigin == nil {
            dragStartOrigin = panel.frame.origin
        }
        guard let start = dragStartOrigin,
              let screen = Self.screenContainingMouse() else { return }
        // SwiftUI 手势 y 向下为正，AppKit 窗口 y 向上为正
        var x = start.x + translation.width
        var y = start.y - translation.height
        let visible = screen.visibleFrame
        x = min(max(x, visible.minX), visible.maxX - panel.frame.width)
        y = min(max(y, visible.minY), visible.maxY - 6 - panel.frame.height)
        panel.setFrameOrigin(NSPoint(x: x, y: y))
        cancelPendingTasks()
    }

    func dragEnded() {
        Self.log("dragEnded")
        guard let panel else { return }
        dragStartOrigin = nil
        dragCooldownUntil = Date().addingTimeInterval(1.2)
        savedCenterX = panel.frame.midX
        savedTopY = panel.frame.maxY
        UserDefaults.standard.set(savedCenterX!, forKey: anchorXKey)
        UserDefaults.standard.set(savedTopY!, forKey: anchorYKey)
    }

    // MARK: - 屏幕检测

    static func screenContainingMouse() -> NSScreen? {
        let loc = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(loc) } ?? NSScreen.main
    }

    static func isMouseInTopZone(topZoneHeight: CGFloat) -> Bool {
        guard let screen = screenContainingMouse() else { return false }
        let loc = NSEvent.mouseLocation
        return loc.y >= screen.frame.maxY - topZoneHeight
    }

    // MARK: - 窗口帧

    private func sizeForState() -> NSSize {
        switch displayState {
        case .docked:
            return dockedSize
        case .expanded:
            return NSSize(width: expandedWidth, height: expandedHeight())
        }
    }

    // MARK: - 展开高度
    // 与 IslandView 布局逐项对应（任一 padding 改动需同步此处）：
    //   header  = padding(.top,12) + 内容(状态点9/文本~17) + padding(.bottom,8) ≈ 37
    //   divider = 1
    //   列表    = N×行(44) + 行间(2) + ScrollView padding(.vertical,6)×2 ≈ 46N+10，上限 300
    //   summary = Divider(1) + TokenSummaryBar ≈ 26（有数据才显示）
    private let headerHeight: CGFloat = 37
    private let detailHeaderHeight: CGFloat = 47   // 详情页顶栏：返回按钮 24 + padding(12+8) + subtitle 双行
    private let dividerHeight: CGFloat = 1
    // 行高 48：实测单行 45.5~48pt（名称 13pt semibold + 9pt mono 徽标 + padding 14 + spacing 2），
    // 46 低估会静默裁最后一行下 padding（阿剩低4）；窗口背景铺满，略高不可见，取上界安全
    private let rowHeight: CGFloat = 48
    private let listMaxHeight: CGFloat = 300     // 与 ScrollView.frame(maxHeight:) 一致
    private let emptyStateHeight: CGFloat = 87   // zzz 图标 + 文案 + padding(.vertical,22)（阿菜实测 ~87）
    private let summaryBarHeight: CGFloat = 28   // Divider + TokenSummaryBar（阿菜实测 ~28）
    /// 详情/会话页内容区高度（header + divider 之后；内容自身可滚动，故给足而不裁剪）
    private let detailContentHeight: CGFloat = 310

    private func expandedHeight() -> CGFloat {
        switch route {
        case .list:
            if visibleCount() == 0 {
                let summaryBar: CGFloat = engine.tokenMonitor.grandTotal.isEmpty ? 0 : summaryBarHeight
                return min(headerHeight + dividerHeight + emptyStateHeight + summaryBar, expandedMaxHeight)
            }
            let count = max(visibleCount(), 1)
            let listHeight = min(CGFloat(count) * rowHeight + 10, listMaxHeight)
            let summaryBar: CGFloat = engine.tokenMonitor.grandTotal.isEmpty ? 0 : summaryBarHeight
            return min(headerHeight + dividerHeight + listHeight + summaryBar, expandedMaxHeight)
        case .agentDetail:
            // 详情页用 detailHeaderHeight（显式，不再靠 +10 魔法补差）
            return min(detailHeaderHeight + dividerHeight + detailContentHeight, expandedMaxHeight)
        case .sessions:
            return min(detailHeaderHeight + dividerHeight + detailContentHeight, expandedMaxHeight)
        }
    }

    /// 可见行数（统一走 engine.visibleSnapshots，口径单一实现）
    private func visibleCount() -> Int {
        engine.visibleSnapshots.count
    }

    private func placeWindow(size: NSSize, animated: Bool) {
        var cx: CGFloat
        var topY: CGFloat
        if let sx = savedCenterX, let sy = savedTopY {
            cx = sx
            topY = sy
        } else {
            guard let screen = Self.screenContainingMouse() else { return }
            let visible = screen.visibleFrame
            cx = visible.midX
            topY = visible.maxY - 1   // 细条贴菜单栏下缘
        }
        // 钳制不出屏：基于「面板当前所在屏」而非鼠标所在屏，
        // 避免细条停在 A 屏、鼠标在 B 屏触顶时窗口跨屏长距离滑动
        if let screen = panel.screen ?? Self.screenContainingMouse() {
            let visible = screen.visibleFrame
            cx = min(max(cx, visible.minX + size.width / 2), visible.maxX - size.width / 2)
            topY = min(max(topY, visible.minY + size.height), visible.maxY - 1)
        }
        let target = NSRect(x: cx - size.width / 2, y: topY - size.height,
                            width: size.width, height: size.height)
        if animated {
            let duration: TimeInterval = 0.42   // 与内容 spring(response: 0.42) 对齐
            NSAnimationContext.runAnimationGroup { context in
                context.duration = duration
                context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                panel.animator().setFrame(target, display: true)
            }
            animateCornerRadius(duration: duration)
        } else {
            panel.setFrame(target, display: false)
        }
    }

    /// 圆角跟随 frame 动画插值（避免 docked↔expanded 圆角跳变：2.5 ↔ 18pt）
    private func animateCornerRadius(duration: CFTimeInterval) {
        guard let layer = clipContainer?.layer else { return }
        let anim = CABasicAnimation(keyPath: "cornerRadius")
        anim.fromValue = layer.presentation()?.cornerRadius ?? layer.cornerRadius
        anim.toValue = layer.cornerRadius
        anim.duration = duration
        anim.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        layer.add(anim, forKey: "cornerRadiusAnim")
    }

    private func syncExpandedHeight() {
        let size = sizeForState()
        if abs(panel.frame.height - size.height) > 1 {
            placeWindow(size: size, animated: true)
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
// 外层不裁剪，负责画圆角阴影；内层容器裁剪内容。shadowPath 跟随圆角并随尺寸变化更新。

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
        layer.shadowOpacity = enabled ? 0.30 : 0
        layer.shadowRadius = enabled ? 16 : 0
        layer.shadowOffset = CGSize(width: 0, height: -6)
        CATransaction.commit()
        updateShadowPath()
    }

    private func updateShadowPath() {
        guard let layer else { return }
        if layer.shadowOpacity > 0 {
            layer.shadowPath = CGPath(
                roundedRect: bounds,
                cornerWidth: cornerRadius,
                cornerHeight: cornerRadius,
                transform: nil
            )
        } else {
            layer.shadowPath = nil
        }
    }
}
