import Foundation

// MARK: - 模型占比图的取舍口径

/// 环形图图例只单列前 maxShown 个模型。原先直接 `prefix(4)` 把余下的丢掉，屏上的百分比
/// 因此不再求和为 100%，而用户完全看不出少了一截。这里把「被合并掉的部分」显式算出来，
/// 供图例补一行「其余 N 个 · xx%」，并给出口语化的占比文案（不足 1% 不写成 0%）。
/// 本文件刻意不依赖 SwiftUI 与 AgentIslandCore：测试 runner 经符号链接把它编进来断言。
enum ModelDonutAggregate {

    /// 图例最多单列的模型数，超出合并进「其余」
    static let maxShown = 4

    struct Entry {
        let modelId: String
        let tokens: Int
    }

    struct Breakdown {
        let entries: [Entry]
        /// 图例单列的数量（即环形图上着色、且逐项标百分比的部分）
        let shownCount: Int
        let totalTokens: Int

        /// 被合并的模型数（可为 0）
        var hiddenCount: Int { max(0, entries.count - shownCount) }

        var hiddenTokens: Int {
            guard hiddenCount > 0 else { return 0 }
            return entries.dropFirst(shownCount).reduce(0) { $0 + $1.tokens }
        }

        var hiddenPercent: Double {
            guard totalTokens > 0 else { return 0 }
            return Double(hiddenTokens) / Double(totalTokens) * 100
        }

        /// 有实际遗漏才提示：模型数超限但剩余 Token 为 0 时不虚构一行「0%」
        var hasHidden: Bool { hiddenCount > 0 && hiddenTokens > 0 }

        /// 「其余 3 个 · 6%」
        var hiddenText: String {
            guard hasHidden else { return "" }
            return "其余 \(hiddenCount) 个 · \(Self.percentText(hiddenPercent))"
        }

        /// 悬停明细：逐个列出被折进去的模型，让「其余」这一行可核对
        var hiddenDetailText: String {
            guard hasHidden else { return "" }
            return entries.dropFirst(shownCount).map { "\($0.modelId) \($0.tokens)" }
                .joined(separator: "、")
        }

        /// 口语化百分比：<1% 的非零占比写成「<1%」，避免被四舍五入成 0% 后再次伪装成无数据
        static func percentText(_ percent: Double) -> String {
            if percent > 0 && percent < 1 { return "<1%" }
            return "\(Int(percent.rounded()))%"
        }
    }

    static func breakdown(_ entries: [Entry], shown: Int = maxShown) -> Breakdown {
        Breakdown(entries: entries,
                  shownCount: min(max(shown, 0), entries.count),
                  totalTokens: entries.reduce(0) { $0 + $1.tokens })
    }
}
