import Foundation

// MARK: - 无头探测（--probe）
// 用真实监控器采样一次，打印各 Agent 状态表，用于快速验证引擎。

public enum Probe {

    @MainActor
    public static func run() -> Int32 {
        print("AgentIsland probe — 真实环境状态采样")
        let registry = AgentRegistry.fullRegistry()
        // 真实文件监控：先同步扫一次填缓存，再采样
        let monitor = FileActivityMonitor()
        monitor.watch(dirs: registry.flatMap(\.sessionDirs))
        monitor.scanSync()

        let engine = ActivityEngine(
            profiles: registry,
            config: EngineConfig(),
            processMonitor: ProcessMonitor(),          // 真实进程
            fileMonitor: monitor                        // 真实文件系统（后台扫描）
        )
        let snaps = engine.sample()

        print("")
        print(pad("AGENT", 12) + pad("LEVEL", 9) + pad("CPU%", 6) + pad("PROC", 5) + pad("INST", 5) + pad("SESS", 5) + "LAST ACTIVITY")
        print(String(repeating: "-", count: 62))
        for s in snaps {
            print(pad(s.profile.name, 12)
                  + pad(s.level.rawValue.uppercased(), 9)
                  + pad(String(format: "%.1f", s.cpuPercent), 6)
                  + pad(s.processRunning ? "YES" : "no", 5)
                  + pad(s.installed ? "yes" : "no", 5)
                  + pad("\(s.activeSessions)", 5)
                  + s.lastActivityText)
        }
        print("")
        print("anyWorking = \(engine.anyWorking)")
        return 0
    }

    private static func pad(_ text: String, _ width: Int) -> String {
        text.count >= width ? text : text + String(repeating: " ", count: width - text.count)
    }
}
