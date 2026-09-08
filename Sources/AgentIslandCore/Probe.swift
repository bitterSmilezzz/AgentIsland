import Foundation

// MARK: - 无头探测（--probe）
// 用真实监控器采样两次（间隔 1.5s），输出真实 CPU 差分百分比。

public enum Probe {

    @MainActor
    public static func run() -> Int32 {
        print("AgentIsland probe — 真实环境状态采样（双拍 CPU 差分）")
        // 一次性 CLI 工具：同步扫描安装缓存可接受
        let installedApps = InstalledAppsCache()
        installedApps.refresh()
        let registry = AgentRegistry.fullRegistry(installedCLIs: installedApps.installedCLIs())
        // 真实文件监控：先同步扫一次填缓存，再采样
        let monitor = FileActivityMonitor()
        monitor.watch(dirs: registry.flatMap(\.sessionDirs))
        monitor.scanSync()

        let engine = ActivityEngine(
            profiles: registry,
            config: EngineConfig(),
            processMonitor: ProcessProvider(),         // 真实进程
            fileMonitor: monitor,                       // 真实文件系统（后台扫描）
            installedApps: installedApps                // 已热缓存（init 首刷跳过，不双扫）
        )
        // 第一次采样：建立 CPU 差分基线（所有 PID 首次见到返回 0）
        engine.sample()
        print("  ⏳ 采集 CPU 基线…（1.5s）")
        Thread.sleep(forTimeInterval: 1.5)
        // 第二次采样：此次 CPU% 为真实窗口差分值
        let snaps = engine.sample()

        print("")
        print(pad("AGENT", 12) + pad("LEVEL", 9) + pad("CPU%", 6) + pad("MEM", 7) + pad("PROC", 5) + pad("INST", 5) + pad("SESS", 5) + pad("ACTION", 22) + "LAST ACTIVITY")
        print(String(repeating: "-", count: 91))
        for s in snaps {
            print(pad(s.profile.name, 12)
                  + pad(s.isHung ? "HUNG" : s.level.rawValue.uppercased(), 9)
                  + pad(String(format: "%.1f", s.cpuPercent), 6)
                  + pad(s.memoryText, 7)
                  + pad(s.processRunning ? "YES" : "no", 5)
                  + pad(s.installed ? "yes" : "no", 5)
                  + pad("\(s.activeSessions)", 5)
                  + pad(s.currentAction ?? "—", 22)
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
