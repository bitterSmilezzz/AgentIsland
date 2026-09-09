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

        TestKit.test("设置: resolvedEnabled 首次全新安装默认启用并记录已知") {
            let name = "agentisland-settings-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            let p1 = AgentProfile(id: "dim", name: "Dim", icon: "x", bundleIDs: [], processNames: ["dim"], sessionDirs: [], defaultEnabled: true)
            let p2 = AgentProfile(id: "off", name: "Off", icon: "x", bundleIDs: [], processNames: ["off"], sessionDirs: [], defaultEnabled: false)

            let resolved = EnabledAgentStore.resolvedEnabled(registry: [p1, p2], defaults: suite)
            try expectEqual(resolved, ["dim"], "首次安装只启用 defaultEnabled: true")
            try expectEqual(EnabledAgentStore.load(from: suite), ["dim"], "已持久化")
            try expectEqual(EnabledAgentStore.loadKnownAgents(from: suite), ["dim", "off"], "已知列表记录全量 registry")
        }

        TestKit.test("设置: resolvedEnabled 用户主动全关保持空数组") {
            let name = "agentisland-settings-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            EnabledAgentStore.save([], to: suite)
            let p1 = AgentProfile(id: "dim", name: "Dim", icon: "x", bundleIDs: [], processNames: ["dim"], sessionDirs: [], defaultEnabled: true)
            let resolved = EnabledAgentStore.resolvedEnabled(registry: [p1], defaults: suite)
            try expectEqual(resolved, [], "空集合尊重用户意图，不强行覆写")
        }

        TestKit.test("设置: resolvedEnabled 旧版存量迁移自愈（自动补齐 antigravity 等新增内置项）") {
            let name = "agentisland-settings-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            // 模拟旧版本环境：用户存了 ["workbuddy", "dim"]，但没有 knownAgents 键
            EnabledAgentStore.save(["workbuddy", "dim"], to: suite)

            let pDim = AgentProfile(id: "dim", name: "Dim", icon: "x", bundleIDs: [], processNames: ["dim"], sessionDirs: [], defaultEnabled: true)
            let pClaude = AgentProfile(id: "claude", name: "Claude", icon: "x", bundleIDs: [], processNames: ["claude"], sessionDirs: [], defaultEnabled: true)
            let pAnti = AgentProfile(id: "antigravity", name: "Antigravity", icon: "atom", bundleIDs: ["com.google.antigravity"], processNames: ["Antigravity"], sessionDirs: [], defaultEnabled: true)
            let pContinue = AgentProfile(id: "continue", name: "Continue", icon: "x", bundleIDs: [], processNames: ["continue"], sessionDirs: [], defaultEnabled: false)

            let resolved = EnabledAgentStore.resolvedEnabled(registry: [pDim, pClaude, pAnti, pContinue], defaults: suite)

            // antigravity 作为新增 defaultEnabled 项应自动补齐
            try expectTrue(resolved.contains("antigravity"), "新内置项 antigravity 必须自动自愈补入启用集")
            try expectTrue(resolved.contains("dim"), "原有启用项保留")
            try expectTrue(resolved.contains("workbuddy"), "原有启用项保留")
            // claude 属于 legacyKnownAgentIDs，旧配置里没开就不应强行开启
            try expectFalse(resolved.contains("claude"), "旧版已知的未勾选项不应被误开启")
            // continue 默认关闭，不应开启
            try expectFalse(resolved.contains("continue"), "默认关闭项不应开启")

            // 验证持久化写回
            let saved = EnabledAgentStore.load(from: suite) ?? []
            try expectTrue(saved.contains("antigravity"), "写回持久化")
        }

        TestKit.test("设置: resolvedEnabled 用户显式关闭某项后不被重复开启") {
            let name = "agentisland-settings-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            let pAnti = AgentProfile(id: "antigravity", name: "Antigravity", icon: "atom", bundleIDs: [], processNames: ["Antigravity"], sessionDirs: [], defaultEnabled: true)
            _ = EnabledAgentStore.resolvedEnabled(registry: [pAnti], defaults: suite)

            // 用户在设置界面关闭 antigravity
            EnabledAgentStore.save(["other"], to: suite)

            // 下次启动
            let resolved = EnabledAgentStore.resolvedEnabled(registry: [pAnti], defaults: suite)
            try expectFalse(resolved.contains("antigravity"), "已在 knownAgents 中的项被用户关闭后，不再被强行开启")
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

        TestKit.test("设置: IslandAppearance 枚举、轮换与持久化") {
            try expectEqual(IslandAppearance.system.rawValue, "system")
            try expectEqual(IslandAppearance.light.rawValue, "light")
            try expectEqual(IslandAppearance.dark.rawValue, "dark")

            try expectEqual(IslandAppearance.system.label, "跟随系统")
            try expectEqual(IslandAppearance.light.label, "浅色模式")
            try expectEqual(IslandAppearance.dark.label, "深色模式")

            try expectEqual(IslandAppearance.system.icon, "laptopcomputer")
            try expectEqual(IslandAppearance.light.icon, "sun.max.fill")
            try expectEqual(IslandAppearance.dark.icon, "moon.fill")

            // 轮换测试
            try expectEqual(IslandAppearance.system.next(), .light)
            try expectEqual(IslandAppearance.light.next(), .dark)
            try expectEqual(IslandAppearance.dark.next(), .system)

            // 持久化测试
            let name = "agentisland-appearance-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            suite.set(IslandAppearance.dark.rawValue, forKey: SettingKey.islandAppearance)
            let loaded = suite.string(forKey: SettingKey.islandAppearance).flatMap(IslandAppearance.init)
            try expectEqual(loaded, .dark, "IslandAppearance 应正确持久化并读取")
        }

        TestKit.test("设置: NotificationPolicy 分级策略、微窥判定与声音判定及持久化") {
            try expectEqual(NotificationPolicy.standard.rawValue, "standard")
            try expectEqual(NotificationPolicy.focus.rawValue, "focus")
            try expectEqual(NotificationPolicy.silent.rawValue, "silent")

            try expectEqual(NotificationPolicy.standard.label, "标准模式")
            try expectEqual(NotificationPolicy.focus.label, "专注免打扰")
            try expectEqual(NotificationPolicy.silent.label, "完全静默")

            // 1. shouldPeek 规则测试
            // 标准模式：全部事件均 Peek
            try expectTrue(NotificationPolicy.standard.shouldPeek(for: .completed), "标准模式: completed 触发微窥")
            try expectTrue(NotificationPolicy.standard.shouldPeek(for: .attention), "标准模式: attention 触发微窥")
            try expectTrue(NotificationPolicy.standard.shouldPeek(for: .costSpike), "标准模式: costSpike 触发微窥")

            // 专注模式：普通完成静默，仅 costSpike 触发 Peek
            try expectFalse(NotificationPolicy.focus.shouldPeek(for: .completed), "专注模式: completed 不触发微窥")
            try expectFalse(NotificationPolicy.focus.shouldPeek(for: .attention), "专注模式: attention 不触发微窥")
            try expectTrue(NotificationPolicy.focus.shouldPeek(for: .costSpike), "专注模式: costSpike 触发微窥")

            // 静默模式：全部事件均不 Peek
            try expectFalse(NotificationPolicy.silent.shouldPeek(for: .completed), "静默模式: completed 不触发微窥")
            try expectFalse(NotificationPolicy.silent.shouldPeek(for: .attention), "静默模式: attention 不触发微窥")
            try expectFalse(NotificationPolicy.silent.shouldPeek(for: .costSpike), "静默模式: costSpike 不触发微窥")

            // 2. shouldPlaySound 规则测试
            // 全局声音主开关关闭时，任何策略均不发声
            try expectFalse(NotificationPolicy.standard.shouldPlaySound(for: .completed, soundEnabled: false), "主开关关: 标准模式不发声")
            try expectFalse(NotificationPolicy.focus.shouldPlaySound(for: .costSpike, soundEnabled: false), "主开关关: 专注模式不发声")
            try expectFalse(NotificationPolicy.silent.shouldPlaySound(for: .costSpike, soundEnabled: false), "主开关关: 静默模式不发声")

            // 全局声音主开关开启时：
            // 标准模式：全部事件均发声
            try expectTrue(NotificationPolicy.standard.shouldPlaySound(for: .completed, soundEnabled: true), "主开关开: 标准模式 completed 发声")
            try expectTrue(NotificationPolicy.standard.shouldPlaySound(for: .attention, soundEnabled: true), "主开关开: 标准模式 attention 发声")
            try expectTrue(NotificationPolicy.standard.shouldPlaySound(for: .costSpike, soundEnabled: true), "主开关开: 标准模式 costSpike 发声")

            // 专注模式：仅 costSpike 报警发声
            try expectFalse(NotificationPolicy.focus.shouldPlaySound(for: .completed, soundEnabled: true), "主开关开: 专注模式 completed 不发声")
            try expectFalse(NotificationPolicy.focus.shouldPlaySound(for: .attention, soundEnabled: true), "主开关开: 专注模式 attention 不发声")
            try expectTrue(NotificationPolicy.focus.shouldPlaySound(for: .costSpike, soundEnabled: true), "主开关开: 专注模式 costSpike 发声报警")

            // 静默模式：全部事件均不发声
            try expectFalse(NotificationPolicy.silent.shouldPlaySound(for: .completed, soundEnabled: true), "主开关开: 静默模式 completed 不发声")
            try expectFalse(NotificationPolicy.silent.shouldPlaySound(for: .costSpike, soundEnabled: true), "主开关开: 静默模式 costSpike 不发声")

            // 3. 持久化测试
            let name = "agentisland-policy-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            suite.set(NotificationPolicy.focus.rawValue, forKey: SettingKey.notificationPolicy)
            let loaded = suite.string(forKey: SettingKey.notificationPolicy).flatMap(NotificationPolicy.init)
            try expectEqual(loaded, .focus, "NotificationPolicy 应正确持久化并读取")
        }
    }
}
