// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "AgentIsland",
    platforms: [.macOS(.v13)],
    targets: [
        // 核心库：模型/监控引擎/注册表（可被测试 runner import）
        .target(
            name: "AgentIslandCore",
            path: "Sources/AgentIslandCore"
        ),
        // 主程序：UI + 入口
        .executableTarget(
            name: "AgentIsland",
            dependencies: ["AgentIslandCore"],
            path: "Sources/AgentIsland"
        ),
        // 自建测试 runner（CLT 环境无 XCTest/Testing 框架）
        .executableTarget(
            name: "AgentIslandTestsRunner",
            dependencies: ["AgentIslandCore"],
            path: "Tests/AgentIslandTestsRunner"
        ),
    ]
)
