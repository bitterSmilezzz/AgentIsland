import Foundation

// MARK: - 岛窗口几何（唯一事实来源）
// IslandPanel 据此开窗（sizeForState/placeWindow），IslandView/DetailViews 据此排内容。
// 改任何布局度量从本文件入手；标注「实测校准」的值含文本行高，无法由常量推导，
// 改动时需连可视化验收一起看（--probe / 三路由视觉核对）。

enum IslandMetrics {

    // MARK: 窗口尺寸

    /// 展开卡宽度（IslandView 展开卡 / DetailViews 两页的 .frame(width:) 同源）
    static let cardWidth: CGFloat = 280
    /// 展开高度上限（内容超出即滚动）
    static let expandedMaxHeight: CGFloat = 420

    // MARK: 微细条尺寸（收起态保留的 6pt 悬浮指示微胶囊）
    static let topSliverWidth: CGFloat = 140
    static let topSliverHeight: CGFloat = 6
    static let rightSliverWidth: CGFloat = 6
    static let rightSliverHeight: CGFloat = 120

    // MARK: 展开卡纵向度量（逐项对应 IslandView.expandedCard）

    /// 顶栏：padding(.top) + 内容（状态点 9 / 13pt semibold 文本行高 ≈17，取大者）+ padding(.bottom) ≈ 37
    static let headerPaddingTop: CGFloat = 12
    static let headerPaddingBottom: CGFloat = 8
    static let headerContentHeight: CGFloat = 17
    static var headerHeight: CGFloat { headerPaddingTop + headerContentHeight + headerPaddingBottom }

    static let dividerHeight: CGFloat = 1

    /// Agent 行高：实测单行 45.5~48pt（名称 13pt semibold + 9pt mono 徽标 + padding 14 + spacing 2）；
    /// 46 低估会静默裁最后一行下 padding（阿剩低4）；窗口背景铺满，略高不可见，取上界安全
    static let rowHeight: CGFloat = 48
    /// 列表区：行间 spacing 2 + ScrollView .padding(.vertical) 6×2 ≈ +10
    static let listVerticalPadding: CGFloat = 6
    /// 实测校准值（spacing+padding 净贡献，勿按推导式单独改）
    static let listExtraHeight: CGFloat = 10
    /// 与 ScrollView .frame(maxHeight:) 同源
    static let listMaxHeight: CGFloat = 300

    /// 空态：zzz 图标 22 + spacing 6 + 文案 + padding(.vertical)×2 ≈ 87（阿菜实测）
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

    // MARK: 展开高度（纯函数）

    /// 展开卡窗口高度。visibleCount 经 engine.visibleSnapshots（可见口径唯一实现）；
    /// hasSummary = !engine.grandTotal.isEmpty（汇总栏有数据才占高）
    static func expandedHeight(route: CardRoute, visibleCount: Int, hasSummary: Bool) -> CGFloat {
        switch route {
        case .list:
            let summary: CGFloat = hasSummary ? summaryBarHeight : 0
            if visibleCount == 0 {
                return min(headerHeight + dividerHeight + emptyStateHeight + summary, expandedMaxHeight)
            }
            let listHeight = min(CGFloat(max(visibleCount, 1)) * rowHeight + listExtraHeight, listMaxHeight)
            return min(headerHeight + dividerHeight + listHeight + summary, expandedMaxHeight)
        case .agentDetail, .sessions:
            return min(detailHeaderHeight + dividerHeight + detailContentHeight, expandedMaxHeight)
        }
    }
}
