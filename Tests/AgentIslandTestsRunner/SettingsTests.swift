import Foundation
@testable import AgentIslandCore

// MARK: - 设置规则测试（归一化 / load / store / makeCustomID / conflict 纯核）

@MainActor
enum SettingsTests {

    static func register() {
        TestKit.test("设置: normalized 半值自愈/超上限/sample>idle 钳平") {
            var half = EngineConfig()
            half.cpuThreshold = 0.5
            half = half.normalized()
            try expectEqual(half.cpuThreshold, 1.0, "半值自愈到下限")

            var over = EngineConfig()
            over.cpuThreshold = 100
            over = over.normalized()
            try expectEqual(over.cpuThreshold, 50.0, "超上限钳制")

            var reversed = EngineConfig()
            reversed.sampleInterval = 30
            reversed.idleSampleInterval = 15
            reversed = reversed.normalized()
            try expectEqual(reversed.sampleInterval, 15.0, "sample>idle 钳平（闲置降频不反转）")

            let normal = EngineConfig().normalized()
            try expectEqual(normal.sampleInterval, EngineConfig().sampleInterval, "正常值不动")
            try expectEqual(normal.cpuThreshold, EngineConfig().cpuThreshold, "正常值不动")
        }

        TestKit.test("设置: load 无键默认、脏值归一化（suiteName 隔离）") {
            let name = "agentisland-settings-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            let fresh = EngineConfig.load(from: suite)
            try expectEqual(fresh.sampleInterval, EngineConfig().sampleInterval, "无键应回落默认")
            try expectEqual(fresh.cpuThreshold, EngineConfig().cpuThreshold, "无键应回落默认")

            suite.set(0.5, forKey: SettingKey.cpuThreshold)
            suite.set(30.0, forKey: SettingKey.sampleInterval)
            suite.set(15.0, forKey: SettingKey.idleSampleInterval)
            let dirty = EngineConfig.load(from: suite)
            try expectEqual(dirty.cpuThreshold, 1.0, "旧版半值残留自愈")
            try expectEqual(dirty.sampleInterval, 15.0, "反转钳平")
        }

        TestKit.test("设置: store 三态 nil/空数组/有值往返") {
            let name = "agentisland-settings-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            try expectNil(EnabledAgentStore.load(from: suite), "无记录 → nil（回退策略归调用方）")
            EnabledAgentStore.save([], to: suite)
            let empty = EnabledAgentStore.load(from: suite)
            try expectTrue(empty != nil && empty!.isEmpty, "空数组是有意全关，不得当无记录")
            EnabledAgentStore.save(["dim", "claude"], to: suite)
            try expectEqual(EnabledAgentStore.load(from: suite), ["claude", "dim"], "有值往返")
        }

        TestKit.test("设置: makeCustomID 大小写与空格归一") {
            try expectEqual(AgentProfile.makeCustomID("QA Agent"), "custom-qa-agent")
            try expectEqual(AgentProfile.makeCustomID("DimAgent"), "custom-dimagent")
            try expectEqual(AgentProfile.makeCustomID("dim"), "custom-dim")
        }

        TestKit.test("设置: conflict 纯核——启用/已安装/未安装三路") {
            let dim = AgentProfile(id: "dim", name: "DimAgent", icon: "x", bundleIDs: ["com.dimcode.app"],
                                   processNames: ["DimAgent", "dim"], sessionDirs: [])
            let off = AgentProfile(id: "off", name: "Off", icon: "x", bundleIDs: [],
                                   processNames: ["offproc"], sessionDirs: [], defaultEnabled: false)
            let names = AgentRegistry.conflictingProcessNames(
                registry: [dim, off],
                enabledIDs: ["off"],
                installedCLIs: ["dim"],
                installedBundles: [])
            try expectTrue(names.contains("offproc"), "已启用（虽未安装）必须拦")
            try expectTrue(names.contains("dimagent") && names.contains("dim"), "未启用但已安装（CLI 命中）也要拦")
            try expectEqual(names.count, 3, "无关进程不得误拦")
        }

        TestKit.test("设置: DockEdge 枚举与持久化键值") {
            try expectEqual(DockEdge.top.rawValue, "top")
            try expectEqual(DockEdge.right.rawValue, "right")
            try expectEqual(DockEdge(rawValue: "top"), .top)
            try expectEqual(DockEdge(rawValue: "right"), .right)
            try expectNil(DockEdge(rawValue: "unknown"))

            let name = "agentisland-dock-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            suite.set(DockEdge.top.rawValue, forKey: SettingKey.dockEdge)
            suite.set(120.5, forKey: SettingKey.dockAnchorX)
            suite.set(450.0, forKey: SettingKey.dockAnchorY)

            let savedEdge = suite.string(forKey: SettingKey.dockEdge).flatMap(DockEdge.init)
            let savedX = suite.double(forKey: SettingKey.dockAnchorX)
            let savedY = suite.double(forKey: SettingKey.dockAnchorY)

            try expectEqual(savedEdge, .top, "DockEdge 应正确持久化与读取")
            try expectEqual(savedX, 120.5, "dockAnchorX 应正确持久化")
            try expectEqual(savedY, 450.0, "dockAnchorY 应正确持久化")
        }
    }
}
