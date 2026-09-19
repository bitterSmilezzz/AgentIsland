import Foundation

// MARK: - 主列表即时搜索的唯一判定口径

/// 「列表里看得见的行」与「j/k 能聚焦到的行」必须出自同一个谓词。此前 IslandView 的
/// filteredSnapshots 与 IslandPanelRouting.moveFocus 各写了一份完全相同的三字段匹配，
/// 任一处改动都会让两者静默分叉（过滤得到却键不到，或反之）。
/// 本文件刻意不依赖 SwiftUI 与 AgentIslandCore：测试 runner 经符号链接把它编进来，
/// 直接断言判定结果，而不是只读源码文本。
enum AgentSearchFilter {

    /// 关键词归一化：去首尾空白后小写（与匹配端同一口径，避免出现第三种写法）
    static func normalized(_ query: String) -> String {
        query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// 名称 / 档案 id / 进程名任一命中即匹配；空关键词表示不过滤
    static func matches(name: String, id: String, processNames: [String], query: String) -> Bool {
        let q = normalized(query)
        guard !q.isEmpty else { return true }
        if name.lowercased().contains(q) { return true }
        if id.lowercased().contains(q) { return true }
        return processNames.contains { $0.lowercased().contains(q) }
    }
}
