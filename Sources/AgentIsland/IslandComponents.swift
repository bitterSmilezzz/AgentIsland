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
            // 垂直贴边上下各内缩 notchInset；水平贴边左右各内缩同样距离。
            // 内容与窗口度量共用留白，防止消息移除后 Token 文字落入透明倒角区。
            .padding(.vertical, IslandMetrics.notchInset)
            .padding(.horizontal, dockEdge.isHorizontal ? IslandMetrics.notchInset : 0)
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
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill((hovering && hoverEnabled) ? Theme.obsidianCardHoverFill : idleFill)
                    .overlay(
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .strokeBorder(
                                LinearGradient(
                                    colors: colorScheme == .light ? [
                                        Color.white.opacity(hovering ? 1.0 : 0.90),
                                        Color(hex: 0x000000).opacity(hovering ? 0.08 : 0.05)
                                    ] : [
                                        Color.white.opacity(hovering ? 0.22 : 0.08),
                                        Color.white.opacity(hovering ? 0.06 : 0.02)
                                    ],
                                    startPoint: .top,
                                    endPoint: .bottom
                                ),
                                lineWidth: 0.75
                            )
                    )
                    .shadow(
                        color: Color.black.opacity(colorScheme == .light ? (hovering ? 0.05 : 0.025) : 0),
                        radius: hovering ? 3 : 1.5,
                        y: 1
                    )
            )
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

// MARK: 卡内分割线

/// 精致 0.75pt 细线（主列表/汇总栏/详情/会话四处分隔统一，浅色柔和冷石板灰，深色清亮白微透）
struct DarkDivider: View {
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Rectangle()
            .fill(colorScheme == .light ? Color(hex: 0xe2e8f0).opacity(0.85) : Color.white.opacity(0.10))
            .frame(height: 0.75)
    }
}

// MARK: 居中加载指示

/// 居中 loading（横向撑满；纵向撑满/最小高由调用方 frame）
struct CenteredSpinner: View {
    var body: some View {
        HStack { Spacer(); ProgressView().controlSize(.small); Spacer() }
    }
}

// MARK: 长文本可读性

private struct ReadableSingleLineModifier: ViewModifier {
    let fullText: String
    let minWidth: CGFloat
    let priority: Double

    func body(content: Content) -> some View {
        content
            .lineLimit(1)
            // 动作、文件名和模型名的辨识信息常在末尾；中间省略能同时保留语义前缀与后缀。
            .truncationMode(.middle)
            .layoutPriority(priority)
            .frame(minWidth: minWidth, alignment: .leading)
            .contentShape(Rectangle())
            .help(fullText)
            .accessibilityLabel(fullText)
    }
}

extension View {
    /// 单行长文本统一契约：永不静默消失、保留首尾、悬停与 VoiceOver 均可读取全文。
    func readableSingleLine(fullText: String, minWidth: CGFloat = 0, priority: Double = 1) -> some View {
        modifier(ReadableSingleLineModifier(
            fullText: fullText,
            minWidth: minWidth,
            priority: priority
        ))
    }
}

/// 主卡顶部的稳定信息层级：Agent/状态是主标题，实时动作是副标题。
/// 右侧固定按钮再多也至少保留 96pt 给主标题，不会把文字压成空白。
struct AdaptiveHeaderText: View {
    let title: String
    let subtitle: String?
    let badge: String?
    let tint: Color
    let subtitleIcon: String?
    let fullText: String

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 5) {
                Text(title)
                    .font(Theme.bodyFont(12.5, weight: .bold))
                    .foregroundColor(Theme.onDark)
                    .contentTransition(.numericText())
                    .readableSingleLine(fullText: title, minWidth: 96, priority: 3)

                if let badge, !badge.isEmpty {
                    Text(badge)
                        .font(Theme.badgeFont(.semibold))
                        .foregroundColor(tint)
                        .contentTransition(.numericText())
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(tint.opacity(0.12)))
                        .fixedSize()
                        .transition(.scale.combined(with: .opacity))
                }
            }
            .animation(.spring(response: 0.32, dampingFraction: 0.8), value: title)
            .animation(.spring(response: 0.32, dampingFraction: 0.8), value: badge)

            if let subtitle, !subtitle.isEmpty {
                HStack(spacing: 4) {
                    if let subtitleIcon {
                        Image(systemName: subtitleIcon)
                            .font(.system(size: 8, weight: .semibold))
                            .foregroundColor(tint)
                            .frame(width: 10)
                    }
                    Text(subtitle)
                        .font(Theme.monoFont(9.5, weight: .medium))
                        .foregroundColor(tint)
                        .readableSingleLine(fullText: subtitle, priority: 2)
                }
            }
        }
        .frame(minWidth: 96, maxWidth: .infinity, minHeight: 34, alignment: .leading)
        .help(fullText)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(fullText)
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

// MARK: - 原生触觉反馈工具 (Haptic Feedback)

enum HapticFeedback {
    static func perform(_ pattern: NSHapticFeedbackManager.FeedbackPattern = .alignment) {
        NSHapticFeedbackManager.defaultPerformer.perform(pattern, performanceTime: .default)
    }
}
