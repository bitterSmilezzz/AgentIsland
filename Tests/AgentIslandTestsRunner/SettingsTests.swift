import Foundation
@testable import AgentIslandCore

// MARK: - 设置规则测试（归一化 / load / store / makeCustomID / conflict 纯核）

@MainActor
enum SettingsTests {

    static func register() {
        TestKit.test("设置: 损坏的启停存档在第一次写回前被备份，不是就地蒸发") {
            // 读取侧的「只读降级、不写回」保护不了设置页：它把降级后的默认集装进 @State，
            // 用户拨任何一个开关就整体写回。备份这一步让「保留可恢复原状」真的可恢复。
            let suite = TestDefaults.suite("settings-corrupt")
            suite.set(Data("不是 JSON 的旧字节".utf8), forKey: SettingKey.enabledAgents)
            let state = EnabledAgentStore.loadDetailed(from: suite).state
            try expectTrue(state == .corrupt, "前置：非 JSON 字节判为损坏")

            EnabledAgentStore.save(["dim", "codex"], to: suite)
            let backup = suite.data(forKey: EnabledAgentStore.corruptBackupKey)
            try expectEqual(backup, Data("不是 JSON 的旧字节".utf8), "原字节必须被抢救到备份键")
            try expectEqual(EnabledAgentStore.load(from: suite), Set(["dim", "codex"]),
                            "新值照常落库（用户能改设置，不必为了保数据而冻结界面）")

            // 再损坏一次并保存：备份里留的仍是最早那一份，不被第二次的垃圾覆盖
            suite.set(Data("第二份坏字节".utf8), forKey: SettingKey.enabledAgents)
            EnabledAgentStore.save(["claude"], to: suite)
            try expectEqual(suite.data(forKey: EnabledAgentStore.corruptBackupKey),
                            Data("不是 JSON 的旧字节".utf8), "只备份最早那一份")

            // 健康存档不产生备份（否则每次保存都留个孤儿键）
            let clean = TestDefaults.suite("settings-clean")
            EnabledAgentStore.save(["dim"], to: clean)
            EnabledAgentStore.save(["codex"], to: clean)
            try expectNil(clean.object(forKey: EnabledAgentStore.corruptBackupKey),
                          "能解码的存档不该被备份")
        }

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
            let suite = TestDefaults.suite("settings-1")

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
            let suite = TestDefaults.suite("settings-2")
 
             try expectNil(EnabledAgentStore.load(from: suite), "无记录 → nil（回退策略归调用方）")
             EnabledAgentStore.save([], to: suite)
             let empty = EnabledAgentStore.load(from: suite)
             try expectTrue(empty != nil && empty!.isEmpty, "空数组是有意全关，不得当无记录")
             EnabledAgentStore.save(["dim", "claude"], to: suite)
             try expectEqual(EnabledAgentStore.load(from: suite), ["claude", "dim"], "有值往返")
         }
 
         TestKit.test("设置: resolvedEnabled 首次全新安装默认启用并记录已知") {
            let suite = TestDefaults.suite("settings-3")
 
             let p1 = AgentProfile(id: "dim", name: "Dim", icon: "x", bundleIDs: [], processNames: ["dim"], sessionDirs: [], defaultEnabled: true)
             let p2 = AgentProfile(id: "off", name: "Off", icon: "x", bundleIDs: [], processNames: ["off"], sessionDirs: [], defaultEnabled: false)
 
             let resolved = EnabledAgentStore.resolvedEnabled(registry: [p1, p2], defaults: suite)
             try expectEqual(resolved, ["dim"], "首次安装只启用 defaultEnabled: true")
             try expectEqual(EnabledAgentStore.load(from: suite), ["dim"], "已持久化")
             try expectEqual(EnabledAgentStore.loadKnownAgents(from: suite), ["dim", "off"], "已知列表记录全量 registry")
         }
 
         TestKit.test("设置: resolvedEnabled 用户主动全关保持空数组") {
            let suite = TestDefaults.suite("settings-4")
 
             EnabledAgentStore.save([], to: suite)
             let p1 = AgentProfile(id: "dim", name: "Dim", icon: "x", bundleIDs: [], processNames: ["dim"], sessionDirs: [], defaultEnabled: true)
             let resolved = EnabledAgentStore.resolvedEnabled(registry: [p1], defaults: suite)
             try expectEqual(resolved, [], "空集合尊重用户意图，不强行覆写")
         }
 
         TestKit.test("设置: resolvedEnabled 旧版存量迁移自愈（自动补齐 antigravity 等新增内置项）") {
            let suite = TestDefaults.suite("settings-5")
 
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
            let suite = TestDefaults.suite("settings-6")
 
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
 
         TestKit.test("设置: DockEdge 四边枚举、方向语义与持久化键值") {
             try expectEqual(DockEdge.top.rawValue, "top")
             try expectEqual(DockEdge.right.rawValue, "right")
             try expectEqual(DockEdge.bottom.rawValue, "bottom")
             try expectEqual(DockEdge.left.rawValue, "left")
             try expectEqual(DockEdge(rawValue: "top"), .top)
             try expectEqual(DockEdge(rawValue: "right"), .right)
             try expectEqual(DockEdge(rawValue: "bottom"), .bottom)
             try expectEqual(DockEdge(rawValue: "left"), .left)
             try expectNil(DockEdge(rawValue: "unknown"))
             try expectTrue(DockEdge.top.isHorizontal, "顶部沿 X 轴保存锚点")
             try expectTrue(DockEdge.bottom.isHorizontal, "底部沿 X 轴保存锚点")
             try expectFalse(DockEdge.left.isHorizontal, "左侧沿 Y 轴保存锚点")
             try expectFalse(DockEdge.right.isHorizontal, "右侧沿 Y 轴保存锚点")
 
            let suite = TestDefaults.suite("settings-7")

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

        TestKit.test("面板交互: 状态栏宿主窗口不得阻止自动收起") {
            try expectFalse(
                IslandPanelInteraction.isMouseInsideFloatingLayer(
                    className: "NSStatusBarWindow", isVisible: true, containsMouse: false),
                "状态栏窗口仅可见而鼠标在面板外时，应允许自动收起"
            )
            try expectFalse(
                IslandPanelInteraction.isMouseInsideFloatingLayer(
                    className: "NSStatusBarWindow", isVisible: true, containsMouse: true),
                "状态栏宿主的宽泛 frame 不能被当作岛的交互浮层"
            )
            try expectTrue(
                IslandPanelInteraction.isMouseInsideFloatingLayer(
                    className: "_NSPopoverWindow", isVisible: true, containsMouse: true),
                "鼠标实际位于 popover 时应保持展开"
            )
            try expectFalse(
                IslandPanelInteraction.isMouseInsideFloatingLayer(
                    className: "NSWindow", isVisible: true, containsMouse: true),
                "无关窗口不得被当作岛的浮层"
            )
        }

        TestKit.test("面板交互: 松手按最近距离吸附到四条屏幕边") {
            let visible = CGRect(x: 0, y: 0, width: 1_200, height: 800)
            let cases: [(CGRect, DockEdge)] = [
                (CGRect(x: 430, y: 620, width: 330, height: 160), .top),
                (CGRect(x: 850, y: 300, width: 330, height: 160), .right),
                (CGRect(x: 430, y: 20, width: 330, height: 160), .bottom),
                (CGRect(x: 20, y: 300, width: 330, height: 160), .left),
            ]
            for (panel, expected) in cases {
                try expectEqual(IslandPanelInteraction.nearestDockEdge(panelFrame: panel, visibleFrame: visible),
                                expected, "面板 \(panel) 应吸附到 \(expected)")
            }
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
            let suite = TestDefaults.suite("settings-8")

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

            // 专注模式：普通完成静默；需要用户确认与严重告警仍需提醒
            try expectFalse(NotificationPolicy.focus.shouldPeek(for: .completed), "专注模式: completed 不触发微窥")
            try expectTrue(NotificationPolicy.focus.shouldPeek(for: .attention), "专注模式: attention 仍触发微窥")
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

            // 专注模式：完成静默；确认请求与 costSpike 发声
            try expectFalse(NotificationPolicy.focus.shouldPlaySound(for: .completed, soundEnabled: true), "主开关开: 专注模式 completed 不发声")
            try expectTrue(NotificationPolicy.focus.shouldPlaySound(for: .attention, soundEnabled: true), "主开关开: 专注模式 attention 发声")
            try expectTrue(NotificationPolicy.focus.shouldPlaySound(for: .costSpike, soundEnabled: true), "主开关开: 专注模式 costSpike 发声报警")

            // 静默模式：全部事件均不发声
            try expectFalse(NotificationPolicy.silent.shouldPlaySound(for: .completed, soundEnabled: true), "主开关开: 静默模式 completed 不发声")
            try expectFalse(NotificationPolicy.silent.shouldPlaySound(for: .costSpike, soundEnabled: true), "主开关开: 静默模式 costSpike 不发声")

            // 3. 持久化测试
            let suite = TestDefaults.suite("settings-9")

            suite.set(NotificationPolicy.focus.rawValue, forKey: SettingKey.notificationPolicy)
            let loaded = suite.string(forKey: SettingKey.notificationPolicy).flatMap(NotificationPolicy.init)
            try expectEqual(loaded, .focus, "NotificationPolicy 应正确持久化并读取")
        }

        TestKit.test("存档损坏只读降级：不覆写、不丢原数据（LoadState.corrupt）") {
            let suite = TestDefaults.suite("settings-corrupt")
            suite.set(Data("not json".utf8), forKey: SettingKey.enabledAgents)
            let resolved = EnabledAgentStore.resolvedEnabled(
                registry: AgentRegistry.builtin.filter { $0.id == "dim" }, defaults: suite)
            try expectEqual(resolved, ["dim"], "corrupt 时按默认启用集只读运行")
            // 关键断言：原损坏数据必须原样保留（写回 = 用户数据永久丢失）
            try expectEqual(suite.object(forKey: SettingKey.enabledAgents) as? Data,
                            Data("not json".utf8), "损坏存档不得被覆写")
        }

        TestKit.test("全关分支 knownAgents 统一 union 口径（保留 registry 外历史项）") {
            let suite = TestDefaults.suite("settings-knownunion")
            EnabledAgentStore.saveKnownAgents(["dim", "custom-historical"], to: suite)
            EnabledAgentStore.save([], to: suite)   // 用户主动全关
            let resolved = EnabledAgentStore.resolvedEnabled(
                registry: AgentRegistry.builtin.filter { $0.id == "dim" }, defaults: suite)
            try expectEqual(resolved, [], "全关意图保留")
            let known = EnabledAgentStore.loadKnownAgents(from: suite)
            try expectTrue(known?.contains("custom-historical") == true,
                            "registry 外历史 known 项不得被空集分支丢弃")
        }
    }
}
