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
// 3. 几何适配：顶部边缘 (Top) 横向 140x6pt，右侧边缘 (Right) 纵向 6x120pt。

struct DockedSliverCapsule: View {
    let dockEdge: DockEdge
    let isWorking: Bool
    let hasAlert: Bool
    let onTap: () -> Void
    let onHover: (Bool) -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var breathing = false

    private var activeColor: Color {
        if hasAlert {
            return Theme.dangerRed
        } else if isWorking {
            return Theme.statusWorking
        }
        return Theme.statusIdle
    }

    private var shouldAnimate: Bool {
        hasAlert || isWorking
    }

    /// VoiceOver 标签：把状态说清楚，否则细条只是一个无名可点区域
    private var accessibilityLabelText: String {
        let state = hasAlert ? "有告警" : (isWorking ? "智能体工作中" : "待机")
        let edge = dockEdge == .top ? "顶部" : "右侧"
        return "AgentIsland 灵动岛（\(edge)贴边，\(state)）"
    }

    var body: some View {
        ZStack {
            // 底层胶囊：带微光与反光边
            Capsule()
                .fill(Theme.dockedSliverFill(working: isWorking, alert: hasAlert))
                .overlay(
                    Capsule()
                        .strokeBorder(Theme.dockedSliverStroke(working: isWorking, alert: hasAlert), lineWidth: 0.75)
                )

            // 工作/告警呼吸微光晕
            if shouldAnimate {
                Capsule()
                    .fill(activeColor.opacity(hasAlert ? 0.35 : 0.22))
                    .blur(radius: 2)
                    .opacity(breathing ? 0.9 : 0.25)
            }

            // 中心微呼吸状态点 (4pt)
            if shouldAnimate {
                Circle()
                    .fill(activeColor)
                    .frame(width: 4, height: 4)
                    .scaleEffect(breathing ? 1.15 : 0.85)
                    .opacity(breathing ? 1.0 : 0.6)
            }
        }
        .frame(
            width: dockEdge == .top ? IslandMetrics.topSliverWidth : IslandMetrics.rightSliverWidth,
            height: dockEdge == .top ? IslandMetrics.topSliverHeight : IslandMetrics.rightSliverHeight
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
        .onAppear {
            updateAnimationState()
        }
        .onChange(of: isWorking) { _ in
            updateAnimationState()
        }
        .onChange(of: hasAlert) { _ in
            updateAnimationState()
        }
    }

    private func updateAnimationState() {
        // 尊重系统「减弱动态效果」：开启时只保留静态状态色，不跑无限循环动画
        guard shouldAnimate, !reduceMotion else {
            breathing = false
            return
        }
        breathing = false
        withAnimation(.easeInOut(duration: hasAlert ? 1.2 : 2.0).repeatForever(autoreverses: true)) {
            breathing = true
        }
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
