import AgentIslandCore
import SwiftUI

// MARK: - 收起态贴边微胶囊与展开态脉冲动画（DockedSliverCapsule / PulseAnimation）

// MARK: - 收起态边缘微胶囊（DockedSliverCapsule，v1.7.5 视觉强化）
// 设计考量：
// 1. 低功耗零负担：呼吸动画采用平滑缓和的 2.4s 循环，仅在有明确状态（工作或告警）时运行；
// 2. 状态分级：
//    - 告警态 (Alert)：微红/琥珀金光晕与微红呼吸点，第一眼感知死循环或突增事件；
//    - 工作态 (Working)：翠绿微光呼吸流动，多任务时亦能清晰感知运作；
//    - 待机态 (Idle)：优雅深色半透晶莹胶囊，静默无扰；
// 3. 几何适配：顶部/底部横向 140x6pt，左侧/右侧纵向 6x120pt。

// MARK: - 硬件合成零 CPU 呼吸微光层（CoreAnimation）
// 原理：直接挂载 CALayer CABasicAnimation，由 WindowServer/GPU 硬件合成器自主调度，
// 宿主进程 CPU 占用为 0.0%，彻底消除 SwiftUI @State repeatForever 引发的 120Hz 递归布局风暴。

final class CoreAnimationGlowView: NSView {
    private let glowLayer = CALayer()
    private let dotLayer = CALayer()
    private var isCurrentlyAnimating = false
    private var currentAnimKey = ""

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.addSublayer(glowLayer)
        layer?.addSublayer(dotLayer)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        glowLayer.frame = bounds
        glowLayer.cornerRadius = min(bounds.width, bounds.height) / 2

        let dotSize: CGFloat = 4
        dotLayer.frame = CGRect(
            x: (bounds.width - dotSize) / 2,
            y: (bounds.height - dotSize) / 2,
            width: dotSize,
            height: dotSize
        )
        dotLayer.cornerRadius = dotSize / 2
        CATransaction.commit()
    }

    func update(activeColor: NSColor, shouldAnimate: Bool, hasAlert: Bool, reduceMotion: Bool) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)

        if !shouldAnimate {
            if isCurrentlyAnimating {
                glowLayer.removeAllAnimations()
                dotLayer.removeAllAnimations()
                isCurrentlyAnimating = false
                currentAnimKey = ""
            }
            glowLayer.opacity = 0
            dotLayer.opacity = 0
            CATransaction.commit()
            return
        }

        let glowAlpha: CGFloat = hasAlert ? 0.35 : 0.22
        glowLayer.backgroundColor = activeColor.withAlphaComponent(glowAlpha).cgColor
        dotLayer.backgroundColor = activeColor.cgColor

        if reduceMotion {
            if isCurrentlyAnimating {
                glowLayer.removeAllAnimations()
                dotLayer.removeAllAnimations()
                isCurrentlyAnimating = false
                currentAnimKey = ""
            }
            glowLayer.opacity = 0.6
            dotLayer.opacity = 0.8
        } else {
            let animKey = "\(hasAlert)"
            if currentAnimKey != animKey || !isCurrentlyAnimating {
                currentAnimKey = animKey
                isCurrentlyAnimating = true
                glowLayer.removeAllAnimations()
                dotLayer.removeAllAnimations()

                let duration: TimeInterval = hasAlert ? 1.2 : 2.2
                let anim = CABasicAnimation(keyPath: "opacity")
                anim.fromValue = 0.25
                anim.toValue = 0.95
                anim.duration = duration
                anim.autoreverses = true
                anim.repeatCount = .infinity
                anim.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
                glowLayer.add(anim, forKey: "breathing")
                dotLayer.add(anim, forKey: "breathing")
            }
        }
        CATransaction.commit()
    }
}

struct CoreAnimationBreathingGlow: NSViewRepresentable {
    let activeColor: NSColor
    let shouldAnimate: Bool
    let hasAlert: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeNSView(context: Context) -> CoreAnimationGlowView {
        let v = CoreAnimationGlowView(frame: .zero)
        v.update(activeColor: activeColor, shouldAnimate: shouldAnimate, hasAlert: hasAlert, reduceMotion: reduceMotion)
        return v
    }

    func updateNSView(_ nsView: CoreAnimationGlowView, context: Context) {
        nsView.update(activeColor: activeColor, shouldAnimate: shouldAnimate, hasAlert: hasAlert, reduceMotion: reduceMotion)
    }
}

struct DockedSliverCapsule: View {
    let dockEdge: DockEdge
    let isWorking: Bool
    let hasAlert: Bool
    let onTap: () -> Void
    let onHover: (Bool) -> Void

    private var activeNSColor: NSColor {
        if hasAlert {
            return Theme.glowAlert
        } else if isWorking {
            return Theme.glowWorking
        }
        return Theme.glowIdle
    }

    private var shouldAnimate: Bool {
        hasAlert || isWorking
    }

    /// VoiceOver 标签：把状态说清楚，否则细条只是一个无名可点区域
    private var accessibilityLabelText: String {
        let state = hasAlert ? "有告警" : (isWorking ? "智能体工作中" : "待机")
        let edge: String
        switch dockEdge {
        case .top: edge = "顶部"
        case .right: edge = "右侧"
        case .bottom: edge = "底部"
        case .left: edge = "左侧"
        }
        return "AgentIsland 灵动岛（\(edge)贴边，\(state)）"
    }

    var body: some View {
        ZStack {
            // 原生超薄毛玻璃底层 + 半透黑曜石胶囊底 + 反光边
            Capsule()
                .fill(.ultraThinMaterial)
            Capsule()
                .fill(Theme.dockedSliverFill(working: isWorking, alert: hasAlert))
                .overlay(
                    Capsule()
                        .strokeBorder(Theme.dockedSliverStroke(working: isWorking, alert: hasAlert), lineWidth: 0.75)
                )

            // 硬件合成的零 CPU 呼吸微光晕与状态点（CoreAnimation）
            CoreAnimationBreathingGlow(
                activeColor: activeNSColor,
                shouldAnimate: shouldAnimate,
                hasAlert: hasAlert
            )
        }
        .frame(
            width: dockEdge.isHorizontal ? IslandMetrics.topSliverWidth : IslandMetrics.rightSliverWidth,
            height: dockEdge.isHorizontal ? IslandMetrics.topSliverHeight : IslandMetrics.rightSliverHeight
        )
        .contentShape(Rectangle())
        .onTapGesture {
            onTap()
        }
        .onHover { hovering in
            onHover(hovering)
        }
        // 收起态细条是面板的第一入口，此前对 VoiceOver 完全不可见
        .accessibilityElement()
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(accessibilityLabelText)
        .accessibilityHint("展开灵动岛卡片")
        // .accessibilityAddTraits(.isButton) 只改语义、不装 AXPress：不加这一行，
        // VoiceOver 用户听得到「按钮」却按不动（点按手势不是 Button）
        .accessibilityAction { onTap() }
    }
}

// MARK: - 展开态状态点脉冲动画

struct PulseAnimation: ViewModifier {
    let isActive: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulsing = false

    private func startPulse() {
        guard isActive, !reduceMotion else {
            pulsing = false
            return
        }
        pulsing = false
        withAnimation(.easeOut(duration: 1.1).repeatForever(autoreverses: false)) {
            pulsing = true
        }
    }

    func body(content: Content) -> some View {
        content
            .scaleEffect(pulsing ? 1.0 : 0.6)
            .opacity(pulsing ? 0 : 0.4)
            .onChange(of: isActive) { _ in startPulse() }
            .onAppear { startPulse() }
    }
}
