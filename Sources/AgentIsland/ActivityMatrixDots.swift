import SwiftUI
import AgentIslandCore

// MARK: - Sydedock 风格点阵活跃指示器（Activity Matrix Dots）
// 灵感源自 Sydedock 标志性的 36-dot 习惯打卡矩阵：
// 采用精巧微圆点（3.5pt），通过 4 级阶梯光亮（高亮白/荧光绿、75%、50%、空心微光环）
// 呈现智能体运行脉冲与近期负荷，赋予面板工业触觉仪表质感。

struct ActivityMatrixDots: View {
    let snapshot: AgentSnapshot
    var count: Int = 5

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pulse = false

    /// 点阵各点亮度级别计算（0: 空心, 1: 50%, 2: 75%, 3: 100% 亮）
    private func levelForDot(index: Int) -> Int {
        switch snapshot.level {
        case .working:
            // 工作态下，前面的点逐步点亮，末端点处于活动峰值
            if index == count - 1 { return 3 }
            if index >= count - 3 { return 2 }
            return 1
        case .attention:
            return index == count - 1 ? 3 : (index >= count - 3 ? 2 : 0)
        case .completed:
            return index < max(1, count - 1) ? 1 : 2
        case .idle:
            if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                let ratio = min(Double(usage.tokens24h) / 200_000.0, 1.0)
                let activeCount = max(1, Int(ratio * Double(count)))
                if index < activeCount {
                    return index == activeCount - 1 ? 2 : 1
                }
            }
            return 0
        case .offline:
            return 0
        }
    }

    private func dotColor(level: Int, isLast: Bool) -> Color {
        switch level {
        case 3:
            return snapshot.level == .attention ? Theme.warningOrange : Theme.sydedockEmerald
        case 2:
            return snapshot.level == .attention ? Theme.warningOrange.opacity(0.78) : Theme.onDark.opacity(0.75)
        case 1:
            return Theme.onDark.opacity(0.40)
        default:
            return Color.white.opacity(0.08)
        }
    }

    var body: some View {
        HStack(spacing: 2.5) {
            ForEach(0..<count, id: \.self) { index in
                let lvl = levelForDot(index: index)
                let isLast = (index == count - 1)

                Circle()
                    .fill(dotColor(level: lvl, isLast: isLast))
                    .frame(width: 3.5, height: 3.5)
                    .overlay(
                        Circle()
                            .strokeBorder(
                                Theme.onDark.opacity(lvl == 0 ? 0.22 : 0.0),
                                lineWidth: 0.5
                            )
                    )
                    .scaleEffect((isLast && snapshot.level == .working && pulse && !reduceMotion) ? 1.25 : 1.0)
                    .opacity((isLast && snapshot.level == .working && pulse && !reduceMotion) ? 1.0 : (lvl == 0 ? 0.6 : 1.0))
            }
        }
        .onAppear {
            if snapshot.level == .working && !reduceMotion {
                withAnimation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true)) {
                    pulse = true
                }
            }
        }
        .onChange(of: snapshot.level) { newLevel in
            if newLevel == .working && !reduceMotion {
                withAnimation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true)) {
                    pulse = true
                }
            } else {
                pulse = false
            }
        }
        .help("近期运行脉冲与负荷点阵")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("活跃度点阵，状态：\(snapshot.level.label)")
    }
}
