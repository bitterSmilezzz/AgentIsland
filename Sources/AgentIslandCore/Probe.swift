import Foundation

// MARK: - 无头探测（--probe）
// 用真实监控器采样两次（间隔 1.5s），输出真实 CPU 差分百分比。

public enum Probe {

    @MainActor
    public static func run() -> Int32 {
        print("AgentIsland probe — 真实环境状态采样（双拍 CPU 差分）")
        // 构造与双采语义在 LiveSampler 里只定义一次，CLI 的 doctor 走同一条路
        let engine = LiveSampler.makeEngine(restrictToEnabled: false)   // 排障转储：全档案集
        print("  ⏳ 采集 CPU 基线…（1.5s）")
        let snaps = LiveSampler.twoBeatSample(engine)

        print("")
        print(pad("AGENT", 12) + pad("LEVEL", 9) + pad("CPU%", 6) + pad("MEM", 7) + pad("PROC", 5) + pad("INST", 5) + pad("SESS", 5) + pad("ACTION", 22) + "LAST ACTIVITY")
        print(String(repeating: "-", count: 91))
        for s in snaps {
            print(pad(s.profile.name, 12)
                  + pad(s.isHung == true ? "HUNG" : (s.isHung == nil ? "HUNG?" : s.level.rawValue.uppercased()), 9)
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
