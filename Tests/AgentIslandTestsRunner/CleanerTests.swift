import Foundation
@testable import AgentIslandCore

// MARK: - 工作台异常扫描规则矩阵（唯一会杀用户进程的判定）
//
// 全部用注入的进程快照 fixture，不读真实进程表、不依赖系统状态。
// 每条规则都钉住「临界值不报」与「GUI 主进程不报」两侧：
// 前者防误报（把正常负载当死锁），后者防误杀（关掉用户正在用的编辑器）。

@MainActor
enum CleanerTests {

    /// 注入固定快照的进程提供者（不触碰系统）
    private struct FixtureProcessProvider: ProcessProviding {
        let entries: [ProcessSnapshot.Entry]
        var bundles: Set<String> = []
        func snapshot() -> ProcessSnapshot { ProcessSnapshot(entries: entries) }
        func runningBundleIDs() -> Set<String> { bundles }
    }

    private static let profileId = "fixture-agent"

    private static var profile: AgentProfile {
        AgentProfile(id: profileId, name: "Fixture Agent", icon: "terminal",
                     bundleIDs: [], processNames: ["fakecli"], sessionDirs: [])
    }

    private static func entry(pid: Int32 = 4242, ppid: Int32 = 2,
                              path: String = "/opt/fake/bin/fakecli",
                              basename: String = "fakecli",
                              cpu: Double = 0, rss: UInt64 = 0) -> ProcessSnapshot.Entry {
        ProcessSnapshot.Entry(pid: pid, path: path, basename: basename,
                              cpuPercent: cpu, rssBytes: rss, ppid: ppid)
    }

    /// GUI 主进程路径（/Applications/xxx.app/Contents/MacOS/...）
    private static func guiPath(_ name: String = "FakeCLI") -> String {
        "/Applications/FakeApp.app/Contents/MacOS/\(name)"
    }

    /// 闸门只读 `profile.id` / `lastActivityAgo` / `isHung`，其余字段给合法的占位值。
    /// `hung` 传 nil 即「本轮判不出」——一次性采样的真实形状，与 false（判过且清白）不同。
    private static func snap(_ id: String, ago: TimeInterval?, hung: Bool? = false) -> AgentSnapshot {
        AgentSnapshot(profile: AgentProfile(id: id, name: id, icon: "terminal",
                                            bundleIDs: [], processNames: [id], sessionDirs: []),
                      level: hung == true ? .working : .idle, processRunning: true,
                      cpuPercent: hung == true ? 80 : 0, installed: true, activeSessions: 1,
                      lastActivityAgo: ago, lastActivityText: "", isHung: hung)
    }

    private static func scan(_ entries: [ProcessSnapshot.Entry], hung: Set<String> = [],
                             bundles: Set<String> = [],
                             profiles: [AgentProfile]? = nil,
                             recentlyActive: Set<String> = []) -> [AgentAnomaly] {
        let provider = FixtureProcessProvider(entries: entries, bundles: bundles)
        return AgentCleaner(processMonitor: provider)
            .scanAnomalies(profiles: profiles ?? [profile],
                           gates: AnomalyScanGates(hungAgentIDs: hung,
                                                   recentlyActiveProfileIDs: recentlyActive),
                           runningBundleIDs: bundles)
    }

    static func register() {

        TestKit.test("工作台: 孤儿进程——CLI(ppid=1) 报告，GUI 主进程与正常父进程不报") {
            let cli = scan([entry(pid: 4242, ppid: 1, rss: 2048)])
            try expectEqual(cli.count, 1, "CLI 孤儿应报告（实际 \(cli.count) 条）")
            let a = cli[0]
            try expectEqual(a.anomalyType, .orphan, "类型")
            try expectEqual(a.id, AgentAnomaly.identifier(profileId: profileId, type: .orphan, pid: 4242),
                            "身份必须走唯一实现（列表 ForEach 身份）")
            try expectEqual(a.pid, 4242, "pid 透传")
            try expectEqual(a.ppid, 1, "ppid 透传")
            try expectEqual(a.profileId, profileId, "profileId 透传")
            try expectEqual(a.agentName, "Fixture Agent", "agentName 透传")
            try expectEqual(a.commandPath, "/opt/fake/bin/fakecli", "路径透传")
            try expectEqual(a.memoryBytes, 2048, "内存透传")
            try expectTrue(!a.batchCleanable, "孤儿不得进入批量清理（防误杀 launchd 托管服务）")

            // GUI 主进程（用户正在使用的应用本体）永不作为清理对象
            let gui = scan([entry(pid: 5001, ppid: 1, path: guiPath())])
            try expectTrue(gui.isEmpty, "GUI 主进程不得报孤儿（否则等于关掉用户编辑器）")

            // 父进程仍在：不是孤儿
            let alive = scan([entry(pid: 5002, ppid: 900)])
            try expectTrue(alive.isEmpty, "ppid != 1 不得报孤儿")
        }

        TestKit.test("工作台: 孤儿佐证——近期仍有会话写入的 Agent 不报孤儿（launchd 托管）") {
            // ppid==1 但 profile 正在产出会话写入：是活进程（LaunchAgent 常驻托管），
            // 报成孤儿会诱导用户杀掉刻意后台化的智能体（任务静默丢失）
            let active = scan([entry(pid: 4301, ppid: 1)], recentlyActive: [profileId])
            try expectTrue(active.isEmpty, "有近期会话写入的 profile 不得报孤儿")

            // 同样条目，无近期写入 → 仍报孤儿（但只允许逐条清理）
            let idle = scan([entry(pid: 4302, ppid: 1)], recentlyActive: [])
            try expectEqual(idle.count, 1, "无活动佐证仍应报告孤儿")
            try expectTrue(!idle[0].batchCleanable, "无佐证孤儿同样不进批量清理")

            // 佐证只豁免孤儿：死锁与内存超限不受影响（证据充分）
            let hungNotExempt = scan([entry(pid: 4303, cpu: 95)], hung: [profileId],
                                     recentlyActive: [profileId])
            try expectEqual(hungNotExempt.count, 1, "有活动的死锁进程仍应报告")
            try expectTrue(hungNotExempt[0].batchCleanable, "死锁条目可批量清理")
        }

        TestKit.test("工作台: 死锁判定严格大于 10% CPU（临界值不报）") {
            let over = scan([entry(pid: 6001, cpu: 10.5)], hung: [profileId])
            try expectEqual(over.count, 1, "10.5% > 10% 应报告")
            try expectEqual(over[0].anomalyType, .hung, "类型")
            try expectEqual(over[0].id, AgentAnomaly.identifier(profileId: profileId, type: .hung, pid: 6001),
                            "身份必须走唯一实现")
            try expectEqual(over[0].cpuPercent, 10.5, "CPU 透传")
            try expectEqual(over[0].ppid, 2, "ppid 透传")

            let exactly = scan([entry(pid: 6002, cpu: 10.0)], hung: [profileId])
            try expectTrue(exactly.isEmpty,
                           "恰好 10.0% 属临界值，不得判死锁（否则正常负载被误报，实际 \(exactly.map(\.id))）")

            let below = scan([entry(pid: 6003, cpu: 9.9)], hung: [profileId])
            try expectTrue(below.isEmpty, "低于阈值不得报告")

            // 未进入引擎死锁集合：单进程高 CPU 不足以判死锁（聚合判定在引擎侧）
            let notHung = scan([entry(pid: 6004, cpu: 95)], hung: [])
            try expectTrue(notHung.isEmpty, "未命中 hungAgentIDs 不得报死锁")

            // 其它 agent 的死锁标记不得波及本 profile
            let otherHung = scan([entry(pid: 6005, cpu: 95)], hung: ["another-agent"])
            try expectTrue(otherHung.isEmpty, "别人的死锁标记不得牵连")
        }

        TestKit.test("工作台: 死锁判定不覆盖 GUI 主进程（聚合高负载下的渲染子进程属正常）") {
            let gui = scan([entry(pid: 7001, path: guiPath(), cpu: 88)], hung: [profileId])
            try expectTrue(gui.isEmpty, "GUI 主进程即使高 CPU 也不得判死锁")
        }

        TestKit.test("工作台: 内存超限严格大于 2GiB（恰好 2GiB 不报，GUI 不报）") {
            let twoGiB: UInt64 = 2_147_483_648
            let over = scan([entry(pid: 8001, rss: twoGiB + 1)])
            try expectEqual(over.count, 1, "2GiB+1 应报告")
            try expectEqual(over[0].anomalyType, .overweight, "类型")
            try expectEqual(over[0].id, AgentAnomaly.identifier(profileId: profileId, type: .overweight, pid: 8001),
                            "身份必须走唯一实现")
            try expectEqual(over[0].memoryBytes, twoGiB + 1, "内存透传")

            let exactly = scan([entry(pid: 8002, rss: twoGiB)])
            try expectTrue(exactly.isEmpty, "恰好 2GiB 属临界值不得报告（避免误报正常的大项目 IDE）")

            let gui = scan([entry(pid: 8003, path: guiPath(), rss: 5 * 1024 * 1024 * 1024)])
            try expectTrue(gui.isEmpty, "GUI 主进程占用 5GiB 也不得判内存泄漏")
        }

        TestKit.test("工作台: 判定优先级——死锁 > 孤儿 > 内存超限，单条目只报一条") {
            let hungWins = scan([entry(pid: 9001, ppid: 1, cpu: 50,
                                       rss: 5 * 1024 * 1024 * 1024)], hung: [profileId])
            try expectEqual(hungWins.count, 1, "同一进程只报一条（实际 \(hungWins.map(\.id))）")
            try expectEqual(hungWins[0].anomalyType, .hung, "死锁优先于孤儿/超限")

            let orphanWins = scan([entry(pid: 9002, ppid: 1, cpu: 5,
                                         rss: 5 * 1024 * 1024 * 1024)], hung: [])
            try expectEqual(orphanWins.count, 1, "同一进程只报一条（实际 \(orphanWins.map(\.id))）")
            try expectEqual(orphanWins[0].anomalyType, .orphan, "孤儿优先于超限")
        }

        TestKit.test("工作台: pid<=1 的条目与无匹配条目一律跳过") {
            let port = scan([entry(pid: 1, ppid: 1)])
            try expectTrue(port.isEmpty, "pid 1 不得作为可清理对象")

            let zero = scan([entry(pid: 0, ppid: 1)])
            try expectTrue(zero.isEmpty, "pid 0 不得作为可清理对象")

            // bundle 命中但无同名进程时，matcher 返回 pid:-1 占位条目（不得变成幽灵异常）
            let bundleProfile = AgentProfile(id: "bundle-agent", name: "Bundle Agent", icon: "app",
                                             bundleIDs: ["com.fake.app"], processNames: [], sessionDirs: [])
            let placeholder = scan([entry(pid: -1, path: "", basename: "")],
                                   bundles: ["com.fake.app"], profiles: [bundleProfile])
            try expectTrue(placeholder.isEmpty, "bundle 占位条目不得产生异常（幽灵条目）")

            let none = scan([])
            try expectTrue(none.isEmpty, "空快照不得报告")

            let otherName = scan([entry(pid: 9100, ppid: 1, path: "/opt/fake/bin/unrelated",
                                        basename: "unrelated")])
            try expectTrue(otherName.isEmpty, "未命中 profile 的进程不得报告")
        }

        TestKit.test("工作台: 多条目混合快照——各规则各自生效且互不串味") {
            let entries = [
                entry(pid: 10001, ppid: 1, rss: 4096),                       // 孤儿
                entry(pid: 10002, ppid: 2, cpu: 42),                         // 死锁
                entry(pid: 10003, ppid: 2, rss: 4_294_967_296),              // 超限
                entry(pid: 10004, ppid: 1, path: guiPath()),                 // GUI 孤儿：跳过
                entry(pid: 10005, ppid: 2, path: guiPath(), cpu: 99),        // GUI 死锁：跳过
                entry(pid: 10006, ppid: 1, path: "/opt/fake/bin/unrelated",
                      basename: "unrelated"),                              // 未匹配：跳过
            ]
            let result = scan(entries, hung: [profileId])
            try expectEqual(result.count, 3, "应报 3 条（实际 \(result.map(\.id))）")
            try expectEqual(Set(result.map(\.anomalyType)), [.orphan, .hung, .overweight], "三类各一条")
            try expectEqual(Set(result.map(\.pid)), [10001, 10002, 10003], "pid 集合")
            try expectTrue(result.allSatisfy { !$0.commandPath.contains(".app/Contents/MacOS") },
                           "结果里不得出现 GUI 主进程")
        }

        TestKit.test("工作台: 列表身份唯一——同一 pid 被多个 profile 匹配也不得重复 id") {
            // 同一份进程可被多个 profile 命中（如同名 CLI 与宿主）。SwiftUI ForEach
            // 遇到重复 id 会条目互相顶替，因此身份必须带 profileId。
            let p1 = AgentProfile(id: "dup-a", name: "A", icon: "terminal",
                                  bundleIDs: [], processNames: ["sharedcli"], sessionDirs: [])
            let p2 = AgentProfile(id: "dup-b", name: "B", icon: "terminal",
                                  bundleIDs: [], processNames: ["sharedcli"], sessionDirs: [])
            let entries = [entry(pid: 12001, ppid: 1, path: "/opt/fake/bin/sharedcli",
                                 basename: "sharedcli")]
            let result = scan(entries, hung: [], profiles: [p1, p2])
            try expectEqual(result.count, 2, "两个 profile 各报一条（同一 pid）")
            try expectEqual(Set(result.map(\.pid)).count, 1, "前置条件：两条指向同一 pid")
            try expectEqual(Set(result.map(\.id)).count, result.count,
                            "身份必须两两不同，否则列表条目互相顶替（实际 \(result.map(\.id))）")
            try expectEqual(Set(result.map(\.profileId)), ["dup-a", "dup-b"], "归属区分开")
        }

        TestKit.test("工作台: 多 profile 各自独立判定") {
            let p1 = AgentProfile(id: "agent-a", name: "A", icon: "terminal",
                                  bundleIDs: [], processNames: ["proca"], sessionDirs: [])
            let p2 = AgentProfile(id: "agent-b", name: "B", icon: "terminal",
                                  bundleIDs: [], processNames: ["procb"], sessionDirs: [])
            let entries = [
                entry(pid: 11001, ppid: 1, path: "/opt/fake/bin/proca", basename: "proca"),
                entry(pid: 11002, ppid: 1, path: "/opt/fake/bin/procb", basename: "procb"),
            ]
            let result = scan(entries, hung: [], profiles: [p1, p2])
            try expectEqual(result.count, 2, "两个 profile 各报一条")
            try expectEqual(Set(result.map(\.profileId)), ["agent-a", "agent-b"], "归属正确")
            try expectEqual(result.first { $0.pid == 11001 }?.agentName, "A", "名称归属")
            try expectEqual(result.first { $0.pid == 11002 }?.agentName, "B", "名称归属")
        }

        // MARK: - 两道闸门（UI 与 CLI 共用的那份实现）

        TestKit.test("闸门: 「没测到死锁」与「没有死锁」必须分得开") {
            // 判成死锁 → 进集，且有值本身就说明本轮观测窗口够长
            let live = AnomalyScanGates(snapshots: [snap(profileId, ago: 30, hung: true)])
            try expectEqual(live.hungAgentIDs, [profileId], "持续观测下死锁集要非空")
            try expectTrue(live.canJudgeHung, "isHung 有值 ⇒ 本轮有资格判死锁")

            // 判过且清白 vs 根本没判：两者都是「死锁集为空」，区别全在 canJudgeHung 一位上
            let clear = AnomalyScanGates(snapshots: [snap(profileId, ago: 30, hung: false)])
            try expectTrue(clear.hungAgentIDs.isEmpty, "清白不进集")
            try expectTrue(clear.canJudgeHung, "false 是判过的结论，不是没测")

            // 一次性进程拿不到那段连续观测，快照里的 isHung 是 nil：
            // 集合为空但必须显式作废，由 canJudgeHung 逼调用方改口
            let oneShot = AnomalyScanGates(snapshots: [snap(profileId, ago: 30, hung: nil)])
            try expectTrue(oneShot.hungAgentIDs.isEmpty, "没测时死锁集必须为空")
            try expectTrue(!oneShot.canJudgeHung, "一次性扫描本轮无资格判死锁")
            try expectEqual(oneShot.recentlyActiveProfileIDs, [profileId],
                            "佐证集与死锁资格无关，一次性扫描照样要有")
        }

        TestKit.test("闸门: 孤儿佐证窗口——临界值内/外/从未活动") {
            let snaps = [snap("in-window", ago: 599), snap("out-window", ago: 601),
                         snap("never", ago: nil)]
            let ids = AnomalyScanGates.recentlyActiveProfileIDs(in: snaps)
            try expectEqual(ids, ["in-window"], "只有 10 分钟内确有写入的算活进程（临界外一律不入围）")
            try expectEqual(AnomalyScanGates.orphanEvidenceWindow, 600, "窗口口径钉住，改它要连着改断言")
        }

        TestKit.test("闸门: 一次性扫描端到端仍拦住 launchd 托管的活进程") {
            // 这是 CLI 那条链路的真实形状：调用方拿一次性快照、本轮不判死锁。
            // 此前 check/clean/top 连闸门都不传，走默认空集 ⇒ 这条 ppid=1 的活服务
            // 被报成孤儿（还提示用户去杀），而工作台不会。
            let entries = [entry(pid: 4242, ppid: 1, cpu: 80, rss: 4096)]
            let gates = AnomalyScanGates(snapshots: [snap(profileId, ago: 30, hung: nil)])
            let provider = FixtureProcessProvider(entries: entries)
            let live = AgentCleaner(processMonitor: provider)
                .scanAnomalies(profiles: [profile], gates: gates)
            try expectTrue(live.isEmpty, "近期仍在写会话的 ppid=1 进程不得进清理候选：\(live.map(\.reason))")

            // 同一份快照、佐证过期后仍要报出来（闸门不是「把孤儿全关掉」）
            let staleGates = AnomalyScanGates(snapshots: [snap(profileId, ago: 700, hung: nil)])
            let orphan = AgentCleaner(processMonitor: provider)
                .scanAnomalies(profiles: [profile], gates: staleGates)
            try expectEqual(orphan.map(\.anomalyType), [.orphan], "无活动佐证的 ppid=1 仍报孤儿")
            try expectTrue(!orphan[0].batchCleanable, "孤儿只支持逐条人工确认")
        }

        TestKit.test("闸门: 「本轮没判死锁」的措辞取自配置而非写死") {
            let note = AnomalyScanGates.hungNotEvaluatedNote(
                config: EngineConfig(runawayCpuThreshold: 50.0, runawayDurationThreshold: 600))
            try expectTrue(note.contains("50%"), "CPU 阈值要跟传入配置一致：\(note)")
            try expectTrue(note.contains("10 分钟"), "时长阈值要跟传入配置一致：\(note)")
            try expectTrue(note.contains("未评估"), "必须说「没测」，不许说「没有」：\(note)")
        }

        TestKit.test("结构: 异常扫描的观测资格不许由调用方自报") {
            // `scanAnomalies` 的两道闸门原先带默认值，漏传即静默失效（症状是「扫不出死锁」
            // 而不是崩溃），上一版改成调用方声明 sustainedObservation——那还是一个可以说谎的
            // 参数：新加的一次性入口传 true 就编译得过、跑得起来照样谎报。现在资格只从快照的
            // 三态 isHung 推出来，所以这里的棘轮是「源码里不许再出现那个参数」。
            let files = try SourceTree.requireSourceTexts()
            var callSites = 0
            for (name, text) in files {
                let code = SourceTree.codeOnly(text)
                try expectTrue(!code.contains("sustainedObservation"),
                               "\(name): 观测资格又变成调用方自报的了")
                let lines = code.components(separatedBy: "\n")
                for (idx, line) in lines.enumerated() where line.contains("scanAnomalies(") {
                    callSites += 1
                    // 调用是跨行的实参列表，只看命中那一行会冤枉正确的写法
                    let statement = lines[idx..<min(idx + 6, lines.count)].joined(separator: " ")
                    try expectTrue(statement.contains("gates:"),
                                   "\(name):\(idx + 1) 的调用点没传闸门：\(line.trimmingCharacters(in: .whitespaces))")
                }
            }
            try expectTrue(callSites >= 4, "只扫到 \(callSites) 处调用点，断言没有意义")
        }
    }
}
