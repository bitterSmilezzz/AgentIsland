import Foundation
@testable import AgentIslandCore

// MARK: - 配置健壮性（脏持久化自愈 / 旧存档兼容）

@MainActor
enum ConfigTests {

    private static func expectDecodeFailure(_ json: String, _ label: String) throws {
        do {
            _ = try JSONDecoder().decode(AgentProfile.self, from: Data(json.utf8))
            throw TestError(message: "\(label) 期望解码失败，实际成功")
        } catch is DecodingError {
            // 预期：字段缺失/类型不符必须抛错，不得静默造出半个 profile
        }
    }

    static func register() {

        TestKit.test("配置: normalized 把采样间隔钳入 0.5…600s（防忙循环/防失监）") {
            var zero = EngineConfig()
            zero.sampleInterval = 0
            zero = zero.normalized()
            try expectEqual(zero.sampleInterval, 0.5, "0 会让 timer 立即重排成忙循环，必须钳到下限")

            var negative = EngineConfig()
            negative.sampleInterval = -3
            negative = negative.normalized()
            try expectEqual(negative.sampleInterval, 0.5, "负数钳到下限")

            // 上限：采样间隔先钳入区间，再受「sample ≤ idle」约束，
            // 所以只有 idle 也放大到上限附近时 sample 才能停在 600
            var tooBig = EngineConfig()
            tooBig.sampleInterval = 9_999
            tooBig.idleSampleInterval = 9_999
            tooBig = tooBig.normalized()
            try expectEqual(tooBig.sampleInterval, 600.0, "超大值钳到上限")
            try expectEqual(tooBig.idleSampleInterval, 600.0, "闲置间隔同样钳到上限")

            // 单放大 sample（idle 保持默认 5s）：先钳到 600，再被 sample≤idle 压回 idle
            var sampleOnly = EngineConfig()
            sampleOnly.sampleInterval = 9_999
            sampleOnly = sampleOnly.normalized()
            try expectEqual(sampleOnly.sampleInterval, sampleOnly.idleSampleInterval,
                            "sample 高于 idle 时必须被钳平到 idle（闲置降频逻辑不允许反转），"
                            + "因此实际取值是 idle 而非 600")

            var idleSmall = EngineConfig()
            idleSmall.sampleInterval = 0.1
            idleSmall.idleSampleInterval = 0.2
            idleSmall = idleSmall.normalized()
            try expectEqual(idleSmall.idleSampleInterval, 0.5, "闲置间隔同样钳入区间")
            try expectEqual(idleSmall.sampleInterval, 0.5, "有活动间隔同样钳入区间")
            try expectTrue(idleSmall.sampleInterval <= idleSmall.idleSampleInterval,
                           "钳制后仍须保持 sample ≤ idle（闲置降频不反转）")
        }

        TestKit.test("配置: normalized 幂等且不动无关字段") {
            var dirty = EngineConfig()
            dirty.cpuThreshold = 0.2
            dirty.sampleInterval = 0.01
            dirty.idleSampleInterval = 10_000
            dirty.workingWindow = 42
            dirty.activeSessionWindow = 900
            dirty.minWorkingHold = 3
            dirty.tokenAlertThreshold = 12_345
            let once = dirty.normalized()
            let twice = once.normalized()
            try expectEqual(twice, once, "归一化必须幂等")
            try expectEqual(once.cpuThreshold, 1.0, "cpuThreshold 下限")
            try expectEqual(once.sampleInterval, 0.5, "sampleInterval 下限")
            try expectEqual(once.idleSampleInterval, 600.0, "idleSampleInterval 上限")
            try expectEqual(once.workingWindow, 42.0, "工作窗口不参与钳制")
            try expectEqual(once.activeSessionWindow, 900.0, "活跃会话窗口不参与钳制")
            try expectEqual(once.minWorkingHold, 3.0, "滞回时长不参与钳制")
            try expectEqual(once.tokenAlertThreshold, 12_345, "告警阈值不参与钳制")
        }

        TestKit.test("配置: load 缺项回落默认值、非空项按类型读取") {
            let name = "agentisland-config-test-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }
            let base = EngineConfig()

            // 全空：所有键回落默认
            let fresh = EngineConfig.load(from: suite)
            try expectEqual(fresh, base, "全新安装必须等于默认配置")

            // 只写部分键：其余仍回落默认
            suite.set(30.0, forKey: SettingKey.workingWindow)
            suite.set(true, forKey: SettingKey.tokenAlertEnabled)
            suite.set(42_000, forKey: SettingKey.tokenAlertThreshold)
            let partial = EngineConfig.load(from: suite)
            try expectEqual(partial.workingWindow, 30.0, "已写项生效")
            try expectEqual(partial.tokenAlertEnabled, true, "已写项生效")
            try expectEqual(partial.tokenAlertThreshold, 42_000, "已写项生效")
            try expectEqual(partial.sampleInterval, base.sampleInterval, "未写项回落默认")
            try expectEqual(partial.idleSampleInterval, base.idleSampleInterval, "未写项回落默认")
            try expectEqual(partial.cpuThreshold, base.cpuThreshold, "未写项回落默认")
            try expectEqual(partial.activeSessionWindow, base.activeSessionWindow, "未写项回落默认")
            try expectEqual(partial.runawayCpuAlert, base.runawayCpuAlert, "未写项回落默认")

            // 布尔项写成 false 也必须被读到（不能用 double(forKey:) 那套「0 即缺省」的读法）
            suite.set(false, forKey: SettingKey.tokenAlertEnabled)
            suite.set(false, forKey: SettingKey.runawayCpuAlert)
            let off = EngineConfig.load(from: suite)
            try expectFalse(off.tokenAlertEnabled, "显式 false 必须保留")
            try expectFalse(off.runawayCpuAlert, "显式 false 必须保留")
        }

        TestKit.test("配置: load 读出的脏值同样被归一化") {
            let name = "agentisland-config-dirty-\(UUID().uuidString)"
            let suite = UserDefaults(suiteName: name)!
            defer { suite.removePersistentDomain(forName: name) }

            suite.set(-5.0, forKey: SettingKey.cpuThreshold)
            suite.set(0.0, forKey: SettingKey.sampleInterval)
            suite.set(0.0, forKey: SettingKey.idleSampleInterval)
            let cfg = EngineConfig.load(from: suite)
            try expectEqual(cfg.cpuThreshold, 1.0, "脏 CPU 阈值自愈")
            try expectEqual(cfg.sampleInterval, 0.5, "脏采样间隔自愈（否则忙循环）")
            try expectEqual(cfg.idleSampleInterval, 0.5, "脏闲置间隔自愈")
        }

        TestKit.test("配置: AgentProfile 旧存档（仅 id/name/icon）解码成功且可选字段取默认") {
            let legacy = #"{"id":"legacy-agent","name":"Legacy","icon":"star"}"#
            let p = try JSONDecoder().decode(AgentProfile.self, from: Data(legacy.utf8))
            try expectEqual(p.id, "legacy-agent", "id 保真")
            try expectEqual(p.name, "Legacy", "name 保真")
            try expectEqual(p.icon, "star", "icon 保真")
            try expectEqual(p.bundleIDs, [], "缺失 → 默认空")
            try expectEqual(p.processNames, [], "缺失 → 默认空")
            try expectEqual(p.pathContains, [], "缺失 → 默认空")
            try expectEqual(p.pathExcludes, [], "缺失 → 默认空")
            try expectEqual(p.hostBundleIDs, [], "缺失 → 默认空")
            try expectEqual(p.sessionDirs, [], "缺失 → 默认空")
            try expectNil(p.cpuWorkingThreshold, "缺失 → 无下限")
            try expectEqual(p.defaultEnabled, true, "缺失 → 默认启用")
            try expectEqual(p.category, .assistant, "缺失 → 默认分类")
            try expectFalse(p.isCustom, "缺失 → 非自定义")

            // 显式 null 与显式值都要能读
            let withNull = #"{"id":"a","name":"A","icon":"i","cpuWorkingThreshold":null,"category":"codeEditor"}"#
            let p2 = try JSONDecoder().decode(AgentProfile.self, from: Data(withNull.utf8))
            try expectNil(p2.cpuWorkingThreshold, "显式 null → nil")
            try expectEqual(p2.category, .codeEditor, "显式分类生效")

            let withValue = #"{"id":"a","name":"A","icon":"i","cpuWorkingThreshold":12.5,"isCustom":true,"defaultEnabled":false}"#
            let p3 = try JSONDecoder().decode(AgentProfile.self, from: Data(withValue.utf8))
            try expectEqual(p3.cpuWorkingThreshold, 12.5, "显式下限生效")
            try expectTrue(p3.isCustom, "显式自定义标记生效")
            try expectFalse(p3.defaultEnabled, "显式默认关闭生效")
        }

        TestKit.test("配置: AgentProfile 缺 id/name 必须抛错（不得静默造半个条目）") {
            try expectDecodeFailure(#"{"name":"NoId","icon":"star"}"#, "缺 id")
            try expectDecodeFailure(#"{"id":"noid","icon":"star"}"#, "缺 name")
            try expectDecodeFailure(#"{"id":123,"name":"Bad","icon":"star"}"#, "id 类型不符")
            try expectDecodeFailure(#"{"id":"x","name":"X","icon":"star","bundleIDs":"not-an-array"}"#,
                                    "bundleIDs 类型不符")
        }

        TestKit.test("配置: AgentProfile 编解码往返（Equatable 全等）") {
            let original = AgentProfile(id: "round-trip", name: "Round Trip", icon: "bolt",
                                        bundleIDs: ["com.example.app"], processNames: ["roundtrip", "rt"],
                                        pathContains: ["/Applications/Example.app"],
                                        pathExcludes: ["/Applications/Host.app"],
                                        hostBundleIDs: ["com.example.host"],
                                        cpuWorkingThreshold: 7.5,
                                        sessionDirs: ["/tmp/agentisland-rt"],
                                        defaultEnabled: false, category: .codeEditor, isCustom: true)
            let data = try JSONEncoder().encode(original)
            let decoded = try JSONDecoder().decode(AgentProfile.self, from: data)
            try expectEqual(decoded, original, "往返必须全等（含新增可选字段）")
        }

        TestKit.test("配置: ActivityLevel 次序 offline < idle < working") {
            try expectTrue(ActivityLevel.offline < ActivityLevel.idle, "offline < idle")
            try expectTrue(ActivityLevel.idle < ActivityLevel.working, "idle < working")
            try expectTrue(ActivityLevel.offline < ActivityLevel.working, "offline < working")
            try expectFalse(ActivityLevel.working < ActivityLevel.working, "同值不严格小于")
            let sorted = [ActivityLevel.working, .offline, .idle].sorted()
            try expectEqual(sorted, [.offline, .idle, .working], "排序结果")
            try expectEqual([ActivityLevel.working, .idle, .offline].max(), ActivityLevel.working, "max")
        }
    }
}
