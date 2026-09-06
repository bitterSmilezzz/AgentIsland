import SwiftUI

// MARK: - 岛内共享 UI 基元
// 同一视觉语义只有一份实现（批次11「滚动回归」的复制品各自为政根源收敛于此）。

// MARK: 卡片外壳

extension View {
    /// 卡片外壳三连：固定卡宽 + 背景铺满窗口消除透明带 + 玻璃拟态背景。
    /// IslandView 展开卡 / DetailViews 两页统一入口。
    func cardShell() -> some View {
        self
            .frame(width: IslandMetrics.cardWidth)
            // 背景铺满整个窗口（窗口高度可能略大于内容，消除底部透明带）
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .background(GlassCardBackground(cornerRadius: Theme.radiusLg))
    }
}

// MARK: 可 hover 圆角行

/// hover 态自持在行内，避免整列表重绘（阿证低优）
private struct HoverRowBackground: ViewModifier {
    let cornerRadius: CGFloat
    let idleFill: Color
    var hoverEnabled: Bool = true
    @State private var hovering = false

    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill((hovering && hoverEnabled) ? Theme.hoverFill : idleFill))
            .contentShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .onHover { hovering = $0 }
    }
}

extension View {
    /// 可 hover 圆角行背景（AgentRowView / 模型行 / 会话行统一入口）
    func hoverRowBackground(cornerRadius: CGFloat, idleFill: Color, hoverEnabled: Bool = true) -> some View {
        modifier(HoverRowBackground(cornerRadius: cornerRadius, idleFill: idleFill, hoverEnabled: hoverEnabled))
    }
}

// MARK: 深色卡内分割线

/// Divider + 12% onDark 覆盖（主列表/汇总栏/详情/会话四处分隔统一）
struct DarkDivider: View {
    var body: some View {
        Divider().overlay(Theme.onDark.opacity(0.12))
    }
}

// MARK: 居中加载指示

/// 居中 loading（横向撑满；纵向撑满/最小高由调用方 frame）
struct CenteredSpinner: View {
    var body: some View {
        HStack { Spacer(); ProgressView().controlSize(.small); Spacer() }
    }
}

