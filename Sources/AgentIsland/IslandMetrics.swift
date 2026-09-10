import Foundation

// MARK: - 岛窗口几何（唯一事实来源）
// IslandPanel 据此开窗（sizeForState/placeWindow），IslandView/DetailViews 据此排内容。
// 改任何布局度量从本文件入手；标注「实测校准」的值含文本行高，无法由常量推导，
// 改动时需连可视化验收一起看（--probe / 三路由视觉核对）。

enum IslandMetrics {

    // MARK: 窗口尺寸

    /// 展开卡宽度（IslandView 展开卡 / DetailViews 两页的 .frame(width:) 同源）
    static let cardWidth: CGFloat = 330
    /// 展开高度上限（内容超出即滚动）
    static let expandedMaxHeight: CGFloat = 460

    // MARK: 微细条尺寸（收起态保留的 6pt 悬浮指示微胶囊）
    static let topSliverWidth: CGFloat = 140
    static let topSliverHeight: CGFloat = 6
    static let rightSliverWidth: CGFloat = 6
    static let rightSliverHeight: CGFloat = 120

    // MARK: 展开卡纵向度量（逐项对应 IslandView.expandedCard）

    /// 顶栏：padding(.top) + 内容 + padding(.bottom)
    static let headerPaddingTop: CGFloat = 12
    static let headerPaddingBottom: CGFloat = 8
    /// 顶栏内容行高：由右侧圆形图标按钮决定（10pt 图标 + 上下各 4pt padding ≈ 18，
    /// 加 SF Symbol 行高余量取 23）。实测校准：取值偏小会让窗口比 SwiftUI 内容矮
    /// （NSHostingView.fittingSize 比本组常量高约 5.5pt），底部 Token 汇总栏被裁。
    static let headerContentHeight: CGFloat = 23
    static var headerHeight: CGFloat { headerPaddingTop + headerContentHeight + headerPaddingBottom }

    static let dividerHeight: CGFloat = 1

    /// Agent 行高：实测单行 ~50-52pt（含名称、徽标及工作态动作横条）；
    /// 取 52pt 充足安全度量
    static let rowHeight: CGFloat = 52
    /// 列表区：行间 spacing 2 + ScrollView .padding(.vertical) 6×2 ≈ +10
    static let listVerticalPadding: CGFloat = 6
    /// 实测校准值（spacing+padding 净贡献，勿按推导式单独改）
    static let listExtraHeight: CGFloat = 10
    /// 与 ScrollView .frame(maxHeight:) 同源
    static let listMaxHeight: CGFloat = 340

    /// 空态：zzz 图标 22 + spacing 6 + 文案 + padding(.vertical)×2 ≈ 87
    static let emptyStatePaddingVertical: CGFloat = 22
    static let emptyStateHeight: CGFloat = 87

    /// 汇总栏：Divider(1) + TokenSummaryBar（实测校准 ~28，含文本行高；有数据才显示）
    static let summaryBarHeight: CGFloat = 28

    /// 详情/会话页顶栏：返回按钮 24 + 下方两 padding + 双行文本 ≈ 47
    static let detailHeaderPaddingTop: CGFloat = 12
    static let detailHeaderPaddingBottom: CGFloat = 8
    /// 实测校准值（含双行 subtitle 文本行高）
    static let detailHeaderHeight: CGFloat = 47
    /// 详情/会话页内容区高度（header + divider 之后；内容自身可滚动，故给足而不裁剪）
    static let detailContentHeight: CGFloat = 310

    /// 实时活动环微看板（Quick Rings Shelf）内容高度：AgentRingView 24 + 内层 padding 3×2
    /// + 外层 padding(.vertical) 5×2 = 40（其下 DarkDivider 另计）
    /// （实测校准值；漏算此项会导致底部 Token 汇总栏被窗口下边缘裁切）
    static let ringsShelfHeight: CGFloat = 40

    /// 事件提醒栏高度（紧凑态与展开态）
    static let eventBannerHeight: CGFloat = 66
    static let eventBannerCollapsedHeight: CGFloat = 66
    static let eventBannerExpandedHeight: CGFloat = 142

    // MARK: 展开高度（纯函数）

    /// 列表区之外的固定高度合计：顶栏 + 各分割线 + 活动环看板 + 事件栏 + 汇总栏。
    /// 与 IslandView.expandedCard 的子视图顺序一一对应，改动布局须同步本函数。
    static func chromeHeight(hasSummary: Bool, hasRings: Bool, hasEvent: Bool, eventExpanded: Bool) -> CGFloat {
        // 汇总栏 = 自身分割线 + TokenSummaryBar（分割线在 IslandView 中与栏一起出现）
        let summary: CGFloat = hasSummary ? (summaryBarHeight + dividerHeight) : 0
        let rings: CGFloat = hasRings ? (ringsShelfHeight + dividerHeight) : 0
        let bannerH = eventExpanded ? eventBannerExpandedHeight : eventBannerCollapsedHeight
        let eventH: CGFloat = hasEvent ? (bannerH + dividerHeight) : 0
        return headerHeight + dividerHeight + rings + eventH + summary
    }

    /// 列表区高度（封顶时压缩列表，保住底部汇总栏——列表可滚动，汇总栏不可）。
    /// IslandView 的 ScrollView frame 与 expandedHeight 共用本函数，避免两处口径漂移。
    static func listHeight(visibleCount: Int, hasSummary: Bool, hasRings: Bool,
                           hasEvent: Bool, eventExpanded: Bool) -> CGFloat {
        let chrome = chromeHeight(hasSummary: hasSummary, hasRings: hasRings,
                                  hasEvent: hasEvent, eventExpanded: eventExpanded)
        let contentHeight = CGFloat(max(visibleCount, 1)) * rowHeight + listExtraHeight
        let available = max(expandedMaxHeight - chrome, rowHeight)   // 保底一行，避免压成 0
        return min(min(contentHeight, listMaxHeight), available)
    }

    /// 展开卡窗口高度。visibleCount 经 engine.visibleSnapshots（可见口径唯一实现）；
    /// hasSummary = !engine.grandTotal.isEmpty（汇总栏有数据才占高）
    /// hasRings = 顶部活动环看板是否显示（engine.ringShelfSnapshots 非空）
    /// hasEvent = 存在活跃事件通知条；eventExpanded = 事件栏是否展开详细排查信息
    static func expandedHeight(route: CardRoute, visibleCount: Int, hasSummary: Bool,
                               hasRings: Bool = false, hasEvent: Bool = false,
                               eventExpanded: Bool = false) -> CGFloat {
        switch route {
        case .list:
            let chrome = chromeHeight(hasSummary: hasSummary, hasRings: hasRings,
                                      hasEvent: hasEvent, eventExpanded: eventExpanded)
            if visibleCount == 0 {
                return min(chrome + emptyStateHeight, expandedMaxHeight)
            }
            let list = listHeight(visibleCount: visibleCount, hasSummary: hasSummary,
                                  hasRings: hasRings, hasEvent: hasEvent,
                                  eventExpanded: eventExpanded)
            return min(chrome + list, expandedMaxHeight)
        case .agentDetail, .sessions:
            return min(detailHeaderHeight + dividerHeight + detailContentHeight, expandedMaxHeight)
        case .toolbox, .liveStream:
            return min(detailHeaderHeight + dividerHeight + detailContentHeight, expandedMaxHeight)
        }
    }
}
