import SwiftUI
import AgentIslandCore

// MARK: - 岛内共享 UI 基元
// 同一视觉语义只有一份实现（批次11「滚动回归」的复制品各自为政根源收敛于此）。

// MARK: 卡片外壳

extension View {
    /// 卡片外壳三连：固定卡宽 + 背景铺满窗口消除透明带 + 玻璃拟态背景 + 空白区原生拖拽。
    /// IslandView 展开卡 / DetailViews 两页统一入口。
    func cardShell(dockEdge: DockEdge = .right, controller: IslandPanelController? = nil) -> some View {
        self
            // 右侧造型上下各内缩 notchInset；顶部造型左右各内缩同样距离。
            // 内容与窗口度量共用留白，防止消息移除后 Token 文字落入透明倒角区。
            .padding(.vertical, IslandMetrics.notchInset)
            .padding(.horizontal, dockEdge == .top ? IslandMetrics.notchInset : 0)
            .frame(width: IslandMetrics.cardWidth)
            // 背景铺满整个窗口（窗口高度可能略大于内容，消除底部透明带）
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .background(
                ZStack {
                    GlassCardBackground(cornerRadius: Theme.radiusLg, dockEdge: dockEdge)
                    if let controller {
                        WindowDragHandleView(
                            onDragStart: { controller.beginDrag() },
                            onDragEnded: { controller.dragEnded() }
                        )
                    }
                }
            )
    }
}

// MARK: 可 hover 圆角行

/// hover 态自持在行内，避免整列表重绘
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
            .animation(.easeOut(duration: 0.12), value: hovering)
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

// MARK: 原生窗口拖拽视图与修饰符

/// 原生 AppKit 硬件级窗口拖拽视图（120Hz WindowServer 直接接管）
final class WindowDragNSView: NSView {
    var onDragStart: (() -> Void)?
    var onDragEnded: (() -> Void)?

    override func mouseDown(with event: NSEvent) {
        guard let window = self.window else { return }
        onDragStart?()
        window.performDrag(with: event)
        onDragEnded?()
    }
}

struct WindowDragHandleView: NSViewRepresentable {
    var onDragStart: () -> Void
    var onDragEnded: () -> Void

    func makeNSView(context: Context) -> WindowDragNSView {
        let view = WindowDragNSView()
        view.onDragStart = onDragStart
        view.onDragEnded = onDragEnded
        return view
    }

    func updateNSView(_ nsView: WindowDragNSView, context: Context) {
        nsView.onDragStart = onDragStart
        nsView.onDragEnded = onDragEnded
    }
}

extension View {
    /// 顶栏与特定区域原生拖拽（按住调用 performDrag，由 WindowServer 硬件级直接移动）
    func cardDrag(controller: IslandPanelController) -> some View {
        self.background(
            WindowDragHandleView(
                onDragStart: { controller.beginDrag() },
                onDragEnded: { controller.dragEnded() }
            )
        )
    }
}

