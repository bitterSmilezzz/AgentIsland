import SwiftUI
import AgentIslandCore

// MARK: - 环形微仪表盘与动态活动弧（Agent Ring & Activity Arc）
// 灵感源自 CodeNotch 的 ProviderRing 与 ActivityArc：
// 外圈彩色水位分级环 + 居中极简 Agent Glyph + 内圈工作态高帧率旋转微弧 + 琥珀呼吸警示环。

struct AgentRingView: View {
    let snapshot: AgentSnapshot
    var size: CGFloat = 34
    var showNumericBadge: Bool = false

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(snapshot: AgentSnapshot, size: CGFloat = 34, showNumericBadge: Bool = false) {
        self.snapshot = snapshot
        self.size = size
        self.showNumericBadge = showNumericBadge
    }

    /// 外圈进度（0.0 ~ 1.0）
    private var progress: CGFloat {
        switch snapshot.level {
        case .working:
            // 工作中根据 CPU 与近期活动计算活跃度（保底 0.35 弧长，随 CPU 增高）
            let cpuFrac = min(CGFloat(snapshot.cpuPercent) / 100.0, 1.0)
            return max(0.35, min(0.35 + cpuFrac * 0.65, 1.0))
        case .idle:
            if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                // 闲置但有 24h token 消耗时，按水位绘制环
                let frac = min(CGFloat(usage.tokens24h) / 200_000.0, 1.0)
                return max(0.15, frac)
            }
            return 0
        case .offline:
            return 0
        }
    }

    /// 状态分级色彩（CodeNotch 4 级饱和色标）
    private var ringColor: Color {
        switch snapshot.level {
        case .working:
            if snapshot.cpuPercent >= 40 {
                return Palette.ringOrange
            } else {
                return Palette.ringGreen
            }
        case .idle:
            if let usage = snapshot.tokenUsage {
                if usage.tokens24h >= 200_000 {
                    return Palette.ringOrange
                } else if usage.tokens24h >= 50_000 {
                    return Palette.ringYellow
                } else if usage.tokens24h > 0 {
                    return Palette.ringGreen
                }
            }
            return Palette.ringTrack
        case .offline:
            return Palette.ringTrack
        }
    }

    private var trackColor: Color {
        Palette.ringTrack
    }

    private var strokeWidth: CGFloat {
        size >= 34 ? 2.5 : 2.0
    }

    private var glyphSize: CGFloat {
        size >= 34 ? 13 : 11
    }

    var body: some View {
        VStack(spacing: 3) {
            ZStack {
                // 1. 底轨环（暗灰色极细圆环）
                Circle()
                    .strokeBorder(trackColor, lineWidth: strokeWidth)

                // 2. 外圈分级彩色进度环
                if progress > 0 {
                    Circle()
                        .inset(by: strokeWidth / 2)
                        .trim(from: 0, to: progress)
                        .stroke(
                            ringColor,
                            style: StrokeStyle(lineWidth: strokeWidth, lineCap: .round)
                        )
                        .rotationEffect(.degrees(-90))
                        .animation(.spring(response: 0.35, dampingFraction: 0.78), value: progress)
                }

                // 3. 内圈活动动效（Activity Arc）
                if snapshot.level == .working {
                    SpinningActivityArc(color: ringColor, size: size, stroke: strokeWidth)
                } else if snapshot.currentAction != nil {
                    // 等待或有未完成指令时，呈现呼吸琥珀环
                    PulsingAttentionArc(color: Palette.ringYellow, size: size, stroke: strokeWidth)
                }

                // 4. 居中智能体专属 Glyph 图标
                Image(systemName: snapshot.profile.icon)
                    .font(.system(size: glyphSize, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                    .opacity(snapshot.level == .offline ? 0.35 : (snapshot.level == .idle ? 0.65 : 1.0))
            }
            .frame(width: size, height: size)

            // 5. 可选数值标签（用于紧凑看板等场景）
            if showNumericBadge {
                Text(badgeText)
                    .font(Theme.monoFont(9, weight: .medium))
                    .foregroundColor(Theme.onDarkMuted)
                    .lineLimit(1)
                    .contentTransition(.numericText())
            }
        }
    }

    private var badgeText: String {
        if snapshot.level == .working {
            return "\(Int(snapshot.cpuPercent))%"
        }
        if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
            return TokenUsage.compact(usage.tokens24h)
        }
        return snapshot.level.label
    }
}

// MARK: - 内圈旋转微弧（Activity Arc · Spinning）

private struct SpinningActivityArc: View {
    let color: Color
    let size: CGFloat
    let stroke: CGFloat

    @State private var angle: Double = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var innerInset: CGFloat {
        stroke + 3.0
    }

    var body: some View {
        Circle()
            .inset(by: innerInset)
            .trim(from: 0, to: 0.25)
            .stroke(
                color.opacity(0.85),
                style: StrokeStyle(lineWidth: max(1.2, stroke * 0.6), lineCap: .round)
            )
            .rotationEffect(.degrees(angle))
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.linear(duration: 1.2).repeatForever(autoreverses: false)) {
                    angle = 360
                }
            }
            .onDisappear {
                angle = 0
            }
    }
}

// MARK: - 内圈呼吸警示环（Attention Arc · Pulsing）

private struct PulsingAttentionArc: View {
    let color: Color
    let size: CGFloat
    let stroke: CGFloat

    @State private var pulsing = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var innerInset: CGFloat {
        stroke + 3.0
    }

    var body: some View {
        Circle()
            .inset(by: innerInset)
            .stroke(
                color,
                style: StrokeStyle(lineWidth: max(1.2, stroke * 0.6), lineCap: .round)
            )
            .opacity(pulsing ? 0.25 : 0.95)
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.easeInOut(duration: 0.85).repeatForever(autoreverses: true)) {
                    pulsing = true
                }
            }
            .onDisappear {
                pulsing = false
            }
    }
}

// MARK: - CodeNotch 灵感色板（Palette）

/// 水位色板。深色值维持原有荧光观感不变；浅色值统一加深，保证白玻璃上
/// 环线 ≥3:1、被当作文字用时（如环看板副标题、tooltip 花费）≥4.5:1。
/// 此前四个颜色全部硬编码亮色：ringYellow 白底 ≈1.3:1、ringGreen ≈1.8:1，浅色主题下几乎不可见。
enum Palette {
    /// 环形底轨灰
    static let ringTrack = Color(dynamicLight: 0xdcdce0, dark: 0x333338)
    /// 水位 0~49% 荧光鲜绿（浅色取 Theme.statusWorking 同款加深绿，白底约 5:1）
    static let ringGreen = Color(dynamicLight: 0x157f3c, dark: 0x28E07B)
    /// 水位 50~79% 琥珀黄（浅色取 Theme.statusIdle 同款深琥珀，白底约 5:1）
    static let ringYellow = Color(dynamicLight: 0x8f6a00, dark: 0xF5E400)
    /// 水位 80~99% 预警亮橙
    static let ringOrange = Color(dynamicLight: 0xc23a00, dark: 0xFF4500)
    /// 水位 100% / 熔断 极光赤红
    static let ringRed = Color(dynamicLight: 0xc62828, dark: 0xFF3B30)
}
