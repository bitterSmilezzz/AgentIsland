import SwiftUI
import AgentIslandCore

// MARK: - 环形微仪表盘与动态活动弧（Agent Ring & Activity Arc）
// 灵感源自 CodeNotch 的 ProviderRing 与 ActivityArc：
// 外圈彩色水位分级环 + 居中极简 Agent Glyph + 内圈工作态高帧率旋转微弧 + 琥珀呼吸警示环。

struct AgentRingView: View {
    let snapshot: AgentSnapshot
    var size: CGFloat = 34

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(snapshot: AgentSnapshot, size: CGFloat = 34) {
        self.snapshot = snapshot
        self.size = size
    }

    /// 外圈进度（0.0 ~ 1.0）
    private var progress: CGFloat {
        switch snapshot.level {
        case .working:
            // 工作中根据 CPU 与近期活动计算活跃度（保底 0.35 弧长，随 CPU 增高）
            let cpuFrac = min(CGFloat(snapshot.cpuPercent) / 100.0, 1.0)
            return max(0.35, min(0.35 + cpuFrac * 0.65, 1.0))
        case .attention:
            return 0.78
        case .completed:
            return 0.45
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
            // 熔断语义（isHung = 疑似死锁/死循环）：环呈极光红，与细条/横幅的
            // 告警红一致——此前 ringRed 从未接线，四级色标实际只有三级
            if snapshot.isHung {
                return Palette.ringRed
            }
            if snapshot.cpuPercent >= 80 {
                return Palette.ringOrange
            } else {
                return Palette.ringGreen
            }
        case .attention:
            return Palette.ringYellow
        case .completed:
            return Palette.ringGreen
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

    private var cornerRadius: CGFloat {
        size * 0.28
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
                // 1. 底轨环（Sydedock 风格圆角超椭圆跑道微底）
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .strokeBorder(trackColor, lineWidth: strokeWidth)
                    .background(
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(Color(dynamic: NSColor(hex: 0xffffff, alpha: 0.9), dark: NSColor(hex: 0xffffff, alpha: 0.04)))
                    )

                // 2. 外圈分级彩色进度环
                if progress > 0 {
                    RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
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
                } else if snapshot.level == .attention {
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

        }
    }
}

// MARK: - 内圈旋转微弧（Activity Arc · Spinning 流光微弧）

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
            .trim(from: 0, to: 0.35)
            .stroke(
                AngularGradient(
                    gradient: Gradient(colors: [
                        color.opacity(0.0),
                        color.opacity(0.35),
                        color.opacity(0.95)
                    ]),
                    center: .center,
                    startAngle: .degrees(0),
                    endAngle: .degrees(126)
                ),
                style: StrokeStyle(lineWidth: max(1.4, stroke * 0.65), lineCap: .round)
            )
            .rotationEffect(.degrees(angle))
            .onAppear {
                guard !reduceMotion else { return }
                withAnimation(.linear(duration: 1.1).repeatForever(autoreverses: false)) {
                    angle = 360
                }
            }
            .onDisappear {
                angle = 0
            }
    }
}

// MARK: - 内圈呼吸警示环（Attention Arc · 双层光晕脉冲）

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
        ZStack {
            // 外层柔和微光扩散光晕
            Circle()
                .inset(by: innerInset)
                .stroke(
                    color.opacity(pulsing ? 0.85 : 0.18),
                    style: StrokeStyle(lineWidth: max(1.4, stroke * 0.7), lineCap: .round)
                )
                .scaleEffect(pulsing ? 1.08 : 0.94)

            // 内层核心常驻呼吸环
            Circle()
                .inset(by: innerInset)
                .stroke(
                    color.opacity(pulsing ? 0.95 : 0.55),
                    style: StrokeStyle(lineWidth: max(1.2, stroke * 0.5), lineCap: .round)
                )
        }
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true)) {
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
    /// 环形底轨灰（黑曜石微轨）
    static let ringTrack = Color(dynamicLight: Ramp.slate200Hex, dark: 0x22222a)
    /// 水位 0~49% Sydedock 翡翠绿（浅色半边与 Theme.statusWorking 同源）
    static let ringGreen = Color(dynamicLight: Ramp.inkGreenHex, dark: 0x28E07B)
    /// 水位 50~79% Sydedock 金琥珀（浅色半边与 Theme.statusIdle 同源）
    static let ringYellow = Color(dynamicLight: Ramp.inkAmberHex, dark: 0xE3C567)
    /// 水位 80~99% 预警橙
    static let ringOrange = Color(dynamicLight: 0xc23a00, dark: 0xFF6B35)
    /// 水位 100% / 熔断 极光赤红
    static let ringRed = Color(dynamicLight: 0xc62828, dark: Ramp.neonRedHex)
}
