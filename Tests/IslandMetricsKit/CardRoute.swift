import Foundation

// MARK: - CardRoute 测试镜像
//
// 真实定义在 Sources/AgentIsland/IslandView.swift（UI target）。测试模块
// IslandMetricsKit 只把 UI 层的**纯几何**文件 IslandMetrics.swift 纳入编译
// （同目录下的符号链接，非拷贝），不编译 SwiftUI 视图，因此需要同名路由枚举
// 的镜像，才能调用 IslandMetrics.expandedHeight(route:)。
//
// 漂移防线：IslandMetricsTests 中有一条哨兵用例读取 IslandView.swift 的真实
// 源码，比对 case 集合与本镜像是否一致；新增/改名路由会让该用例失败，提示
// 同步镜像并补路由覆盖。
enum CardRoute: Equatable {
    case list
    case agentDetail(String)
    case sessions(String, String)
    case toolbox
    case liveStream(String)
}
