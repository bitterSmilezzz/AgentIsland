import Foundation

// MARK: - 设置持久化（键名与启停集合的唯一 owner）

/// 全部 UserDefaults 键名（UI/Core 共用；键名字面量不得散落在调用方）
public enum SettingKey {
    public static let sampleInterval = "sampleInterval"
    public static let idleSampleInterval = "idleSampleInterval"
    public static let workingWindow = "workingWindow"
    public static let cpuThreshold = "cpuThreshold"
    public static let activeSessionWindow = "activeSessionWindow"
    public static let collapseDelay = "collapseDelay"
    public static let enabledAgents = "enabledAgents"
    public static let islandAppearance = "islandAppearance"
    public static let customAgents = "customAgents"
    public static let launchAtLogin = "launchAtLogin"
    public static let dockEdge = "dockEdge"
    public static let dockAnchorX = "dockAnchorX"
    public static let dockAnchorY = "dockAnchorY"
}

/// 启停集合持久化：key/编解码/空数组语义单点持有。
/// 「无记录」与「空数组」是两个状态——空集合是用户主动全关，照常存取；
/// 无记录时的回退策略（defaultEnabled 集 / 引擎当前集）由调用方决定，store 不掺和。
public enum EnabledAgentStore {

    /// nil = 无记录（键不存在或解码失败）
    public static func load(from defaults: UserDefaults = .standard) -> Set<String>? {
        guard let data = defaults.data(forKey: SettingKey.enabledAgents),
              let saved = try? JSONDecoder().decode([String].self, from: data) else {
            return nil
        }
        return Set(saved)
    }

    /// 空集合是有意全关，照常写入（不得当作「清除记录」）
    public static func save(_ ids: Set<String>, to defaults: UserDefaults = .standard) {
        if let data = try? JSONEncoder().encode(Array(ids)) {
            defaults.set(data, forKey: SettingKey.enabledAgents)
        }
    }
}
