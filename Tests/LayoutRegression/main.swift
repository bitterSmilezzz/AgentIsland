import AppKit
import SwiftUI
import AgentIslandCore
@MainActor enum LayoutProbe { static var frame = CGRect.zero }
@MainActor func run() {
    _ = NSApplication.shared
    let cache = InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }); cache.refresh()
    let token = FakeTokenUsageMonitor()
    token.grandTotal = TokenUsage(tokens24h: 15000, tokensTotal: 200000, cost24h: 0.2, costTotal: 3)
    let profile = AgentRegistry.builtin.first { $0.id == "dim" }!
    let engine = ActivityEngine(profiles: (0..<8).map { AgentProfile(id: "test-\($0)", name: "Agent \($0)", icon: "terminal", bundleIDs: [], processNames: ["dim"], sessionDirs: profile.sessionDirs) }, processMonitor: FakeProcessProvider(processNames: ["dim"], bundleIDs: []), fileMonitor: FakeFileActivityProvider(writes: Dictionary(uniqueKeysWithValues: profile.sessionDirs.map { ($0, Date()) })), tokenMonitor: token, installedApps: cache)
    _ = engine.sample()
    let controller = IslandPanelController(engine: engine)
    controller.dockEdge = .right
    controller.toggle()
    func settle() { let end = Date().addingTimeInterval(0.5); while Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) } }
    engine.postEvent(AgentTaskEvent(agentId: "dim", agentName: "Dim", eventType: .completed, duration: 10, timestamp: Date()))
    settle()
    print("WITH_EVENT token=\(LayoutProbe.frame)")
    engine.clearLatestEvent()
    settle()
    print("NO_EVENT token=\(LayoutProbe.frame)")
    guard let window = NSApplication.shared.windows.first else { fatalError("No test window") }
    let shape = SideNotchShape.cgPath(bounds: CGRect(origin: .zero, size: window.frame.size), dockEdge: .right, curlRadius: 10)
    // TokenSummaryBar 上下 padding 为 7pt；检查正文下沿而非透明 padding。
    let textBottom = CGPoint(x: 100, y: LayoutProbe.frame.maxY - 7)
    guard shape.contains(textBottom) else {
        print("FAIL: Token text bottom \(textBottom.y) outside notch; window=\(window.frame.height)")
        exit(1)
    }
    print("PASS: dismissed-event Token text stays inside notch")
}
MainActor.assumeIsolated { run() }
