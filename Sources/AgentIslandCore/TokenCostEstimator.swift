import Foundation

/// 智能 Token 计费估算引擎：为本地未上报或缺失费用（cost <= 0）的模型提供主流官方费率参考估算
public enum TokenCostEstimator {

    /// 模型定价模型（单位：美元 / 百万 tokens）
    public struct Rate: Equatable, Sendable {
        public let inputPerMillion: Double
        public let outputPerMillion: Double

        public init(input: Double, output: Double) {
            self.inputPerMillion = input
            self.outputPerMillion = output
        }

        /// 按典型 3:1 输入/输出 token 比例进行混合加权单价（单位：美元 / token）
        public var blendedPerToken: Double {
            let blendedMillion = (inputPerMillion * 3.0 + outputPerMillion * 1.0) / 4.0
            return blendedMillion / 1_000_000.0
        }
    }

    /// 常见主流模型官方基准费率表（按特定度由长到短排布）
    public static let knownRates: [(pattern: String, rate: Rate)] = [
        // Claude 系列
        ("claude-3-7-sonnet", Rate(input: 3.0, output: 15.0)),
        ("claude-3-5-sonnet", Rate(input: 3.0, output: 15.0)),
        ("claude-3-5-haiku", Rate(input: 0.8, output: 4.0)),
        ("claude-3-haiku", Rate(input: 0.25, output: 1.25)),
        ("claude-3-opus", Rate(input: 15.0, output: 75.0)),
        ("claude-opus", Rate(input: 15.0, output: 75.0)),
        ("claude-sonnet", Rate(input: 3.0, output: 15.0)),
        ("claude-haiku", Rate(input: 0.8, output: 4.0)),

        // OpenAI 系列
        ("gpt-4o-mini", Rate(input: 0.15, output: 0.60)),
        ("gpt-4o", Rate(input: 2.50, output: 10.00)),
        ("gpt-4-turbo", Rate(input: 10.0, output: 30.0)),
        ("gpt-4", Rate(input: 30.0, output: 60.0)),
        ("gpt-3.5-turbo", Rate(input: 0.5, output: 1.5)),
        ("o1-mini", Rate(input: 1.10, output: 4.40)),
        ("o1-preview", Rate(input: 15.0, output: 60.0)),
        ("o1", Rate(input: 15.0, output: 60.0)),
        ("o3-mini", Rate(input: 1.10, output: 4.40)),
        ("o3", Rate(input: 2.50, output: 10.0)),

        // DeepSeek 系列
        ("deepseek-reasoner", Rate(input: 0.55, output: 2.19)),
        ("deepseek-r1", Rate(input: 0.55, output: 2.19)),
        ("deepseek-chat", Rate(input: 0.14, output: 0.28)),
        ("deepseek-v3", Rate(input: 0.14, output: 0.28)),
        ("deepseek", Rate(input: 0.14, output: 0.28)),

        // Google Gemini 系列
        ("gemini-2.0-flash", Rate(input: 0.10, output: 0.40)),
        ("gemini-2.5-flash", Rate(input: 0.10, output: 0.40)),
        ("gemini-1.5-flash", Rate(input: 0.075, output: 0.30)),
        ("gemini-2.0-pro", Rate(input: 1.25, output: 5.00)),
        ("gemini-2.5-pro", Rate(input: 1.25, output: 5.00)),
        ("gemini-1.5-pro", Rate(input: 1.25, output: 5.00)),
        ("gemini-flash", Rate(input: 0.10, output: 0.40)),
        ("gemini-pro", Rate(input: 1.25, output: 5.00)),

        // Qwen 系列
        ("qwen-max", Rate(input: 2.4, output: 9.6)),
        ("qwen-plus", Rate(input: 0.4, output: 1.2)),
        ("qwen-turbo", Rate(input: 0.1, output: 0.2)),

        // Mistral / Codestral 系列
        ("codestral", Rate(input: 0.3, output: 0.9)),
        ("mistral-large", Rate(input: 2.0, output: 6.0)),
        ("mistral-small", Rate(input: 0.2, output: 0.6))
    ]

    /// 查找指定模型 ID 匹配的费率模型
    public static func rate(for modelId: String) -> Rate? {
        let lower = modelId.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !lower.isEmpty else { return nil }

        // 1. 优先按前缀/包含匹配
        for item in knownRates {
            if lower == item.pattern || lower.contains(item.pattern) {
                return item.rate
            }
        }
        return nil
    }

    /// 根据模型 ID 与总 tokens 数进行估算
    public static func estimateCost(modelId: String, tokens: Int) -> Double? {
        guard tokens > 0, let rate = rate(for: modelId) else { return nil }
        let cost = Double(tokens) * rate.blendedPerToken
        return cost.isFinite ? cost : nil
    }

    /// 格式化估算费用，带有 `~` 估算标识（如 `~$0.42` 或 `~<$0.01`）
    public static func formatEstimate(_ c: Double) -> String {
        guard c > 0 else { return "" }
        if c < 0.01 { return "~<$0.01" }
        return String(format: "~$%.2f", c)
    }

    /// 解析费用：优先使用真实记录费用；若无真实费用且可估算，返回估算费用与文本
    public static func resolveCost(actual: Double, modelId: String? = nil, tokens: Int? = nil) -> (cost: Double, text: String, isEstimated: Bool) {
        if actual > 0 {
            return (actual, TokenUsage.cost(actual), false)
        }
        if let modelId, let tokens, tokens > 0, let est = estimateCost(modelId: modelId, tokens: tokens) {
            return (est, formatEstimate(est), true)
        }
        return (0, "", false)
    }
}
