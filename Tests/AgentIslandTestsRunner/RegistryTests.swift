import Foundation
@testable import AgentIslandCore

// MARK: - 注册表 / 动态集合 测试（第二轮审查补：addCustom/removeCustom/setEnabled/
// discoverCLI 幂等、持久化往返、全关持久化）

@MainActor
enum RegistryTests {

    static func register() {
        TestKit.test("注册表: discoverCLIProfiles 幂等去重") {
            let a = AgentRegistry.discoverCLIProfiles()
            let b = AgentRegistry.discoverCLIProfiles()
            try expectEqual(Set(a.map(\.id)), Set(b.map(\.id)), "两次发现应一致")
            try expectEqual(a.count, Set(a.map(\.id)).count, "id 不应重复")
            // 内置集已含的 CLI 不应出现在自动发现里
            let existing = Set(AgentRegistry.builtin.flatMap { $0.processNames.map { $0.lowercased() } })
            try expectTrue(!a.contains { existing.contains($0.processNames.first?.lowercased() ?? "") },
                           "内置 CLI 不应重复发现")
        }

        TestKit.test("注册表: 自定义 profile 持久化往返") {
            let p = AgentProfile(id: "custom-qa", name: "QA Agent", icon: "terminal",
                                 bundleIDs: [], processNames: ["qa-agent"],
                                 sessionDirs: ["/tmp/qa-sessions"], isCustom: true)
            AgentRegistry.saveCustomProfiles([p])
            defer { AgentRegistry.saveCustomProfiles([]) }   // 清理，避免污染后续用例
            let loaded = AgentRegistry.loadCustomProfiles()
            try expectEqual(loaded.first?.id, p.id, "往返后 id 一致")
            try expectEqual(loaded.first?.processNames, p.processNames, "进程名一致")
            try expectEqual(loaded.first?.isCustom, true, "isCustom 保留")
        }

        TestKit.test("注册表: fullRegistry 含内置+自定义（自动发现不炸）") {
            let p = AgentProfile(id: "custom-tmp", name: "TMP", icon: "terminal",
                                 bundleIDs: [], processNames: ["tmp-agent"],
                                 sessionDirs: [], isCustom: true)
            AgentRegistry.saveCustomProfiles([p])
            defer { AgentRegistry.saveCustomProfiles([]) }
            let full = AgentRegistry.fullRegistry()
            try expectTrue(full.contains { $0.id == "dim" }, "含内置 dim")
            try expectTrue(full.contains { $0.id == "custom-tmp" }, "含自定义")
            // 引擎的 guard 防重复
            let engine = ActivityEngine(profiles: full, config: EngineConfig(),
                                        processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: [], bundleIDs: [])),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]))
            engine.addCustomProfile(p)
            let count = engine.allProfiles.filter { $0.id == "custom-tmp" }.count
            try expectEqual(count, 1, "重复添加应被引擎 guard 拒绝")
            engine.removeCustomProfile("custom-tmp")
            try expectTrue(!engine.allProfiles.contains { $0.id == "custom-tmp" }, "移除后不存在")
        }

        TestKit.test("引擎: setEnabled 过滤启停集合") {
            let engine = ActivityEngine(profiles: AgentRegistry.builtin, config: EngineConfig(),
                                        processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: [])),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]))
            engine.setEnabled(["dim"])
            try expectEqual(engine.allProfiles.map(\.id), ["dim"], "只保留启用项")
        }

        TestKit.test("引擎: setEnabled 从全量注册表过滤（关→再开不丢失监控）") {
            // 回归：阿剩第六轮高优——之前 setEnabled 只从当前已缩水列表过滤，
            // 「关闭后再开启」的 agent 本会话内永久丢失监控
            let engine = ActivityEngine(profiles: AgentRegistry.builtin, config: EngineConfig(),
                                        processMonitor: ProcessMonitor(provider: FakeProcessProvider(processNames: ["DimAgent", "Codex"], bundleIDs: [])),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]))
            engine.setEnabled(["dim"])                      // 只启用 dim
            try expectEqual(engine.allProfiles.map(\.id), ["dim"], "只保留启用项")
            engine.setEnabled(["dim", "claude"])            // 再开启 claude
            let ids = engine.allProfiles.map(\.id)
            try expectTrue(ids.contains("dim"), "dim 应保留")
            try expectTrue(ids.contains("claude"), "claude 应能加回（修复后从 fullRegistry 取源）")
            try expectEqual(Set(ids).count, ids.count, "无重复")
        }

        TestKit.test("持久化: 全关（空数组）不被回退为默认") {
            // 模拟「用户主动全关」：写入空数组后，读取路径应返回空而非默认集
            let empty: [String] = []
            if let data = try? JSONEncoder().encode(empty) {
                UserDefaults.standard.set(data, forKey: "enabledAgents")
                defer { UserDefaults.standard.removeObject(forKey: "enabledAgents") }
                let raw = UserDefaults.standard.data(forKey: "enabledAgents")
                let decoded = raw.flatMap { try? JSONDecoder().decode([String].self, from: $0) }
                try expectEqual(decoded ?? ["sentinel"], [], "空数组应解码为空集（无记录才算默认）")
            }
        }

        TestKit.test("引擎: bundle 命中但进程名未匹配 → 标记运行且 CPU=0（[pid:-1] 占位）") {
            // 回归：sampleCore 单趟 matchingEntries 后 running/cpu 语义等价性
            //（阿剩测试缺口）——bundleHit 无名字匹配应返回占位条目
            let provider = FakeProcessProvider(processNames: ["some-other-app"],
                                               bundleIDs: ["com.dimcode.app"])
            let engine = ActivityEngine(profiles: AgentRegistry.builtin.filter { $0.id == "dim" },
                                        config: EngineConfig(),
                                        processMonitor: ProcessMonitor(provider: provider),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]))
            let snaps = engine.sample(now: Date())
            let dim = snaps.first { $0.id == "dim" }
            try expectTrue(dim?.processRunning == true, "bundle 运行即 processRunning=true")
            try expectEqual(dim?.cpuPercent ?? -1, 0, "无进程名匹配时 CPU 合计为 0")
        }
    }
}
