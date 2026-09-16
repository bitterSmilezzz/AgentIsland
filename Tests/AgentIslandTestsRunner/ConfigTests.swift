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
            try expectEqual(once.workingWindow, 42.0, "工作窗口在合法区间内不改动")
            try expectEqual(once.activeSessionWindow, 900.0, "活跃会话窗口在合法区间内不改动")
            try expectEqual(once.minWorkingHold, 3.0, "滞回时长在合法区间内不改动")
            try expectEqual(once.tokenAlertThreshold, 12_345, "告警阈值在合法区间内不改动")
        }

        TestKit.test("配置: normalized 补齐字段钳制（负窗口不得让信号通道失效）") {
            var dirty = EngineConfig()
            dirty.workingWindow = -1
            dirty.activeSessionWindow = -5
            dirty.minWorkingHold = 0
            dirty.runawayCpuThreshold = 500
            dirty.runawayDurationThreshold = 0.5
            dirty.tokenAlertThreshold = 0
            let fixed = dirty.normalized()
            try expectEqual(fixed.workingWindow, 10.0, "工作窗口下限（脏负值会让文件信号通道整体失效）")
            try expectEqual(fixed.activeSessionWindow, 60.0, "活跃会话窗口下限（脏负值让计数恒 0）")
            try expectEqual(fixed.minWorkingHold, 1.0, "滞回下限")
            try expectEqual(fixed.runawayCpuThreshold, 100.0, "死循环 CPU 阈值上限")
            try expectEqual(fixed.runawayDurationThreshold, 30.0, "死循环时长下限")
            try expectEqual(fixed.tokenAlertThreshold, 1_000, "告警阈值下限")
        }

        TestKit.test("配置: NaN 字段归位默认值再钳制") {
            var nan = EngineConfig()
            nan.cpuThreshold = Double.nan
            nan.sampleInterval = Double.nan
            nan.workingWindow = Double.nan
            let healed = nan.normalized()
            let base = EngineConfig()
            try expectEqual(healed.cpuThreshold, base.cpuThreshold, "NaN 必须归位默认（min/max 对 NaN 透传）")
            try expectEqual(healed.sampleInterval, base.sampleInterval, "NaN 必须归位默认")
            try expectEqual(healed.workingWindow, base.workingWindow, "NaN 必须归位默认")
        }

        TestKit.test("配置: 收起延迟合法区间（UI 读取钳制口径，防 UInt64 转换 trap）") {
            // 本用例只守护常量口径；app 侧接线（IslandPanelController init /
            // applyCollapseDelay）在 app target 内，runner 不可 import，由代码评审守护
            try expectEqual(SettingLimits.collapseDelayRange, 0.2...5.0)
            let clamp: (Double) -> Double = {
                min(max($0, SettingLimits.collapseDelayRange.lowerBound),
                    SettingLimits.collapseDelayRange.upperBound)
            }
            try expectEqual(clamp(-1.0), 0.2, "负值下限钳制（此前直通 UInt64 转换即 trap）")
            try expectEqual(clamp(1e30), 5.0, "超大值上限钳制（此前 UInt64(Double) 溢出 trap）")
        }

        TestKit.test("配置: load 缺项回落默认值、非空项按类型读取") {
            let name = "agentisland-config-test-\(UUID().uuidString)"
            let suite = TestDefaults.suite("config-1")
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
            let suite = TestDefaults.suite("config-2")

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
            try expectNil(p.tokenAlertFloor, "缺失 → 无 Token 下限")
            try expectEqual(p.defaultEnabled, true, "缺失 → 默认启用")
            try expectEqual(p.category, .assistant, "缺失 → 默认分类")
            try expectFalse(p.isCustom, "缺失 → 非自定义")

            // 显式 null 与显式值都要能读
            let withNull = #"{"id":"a","name":"A","icon":"i","cpuWorkingThreshold":null,"tokenAlertFloor":null,"category":"codeEditor"}"#
            let p2 = try JSONDecoder().decode(AgentProfile.self, from: Data(withNull.utf8))
            try expectNil(p2.cpuWorkingThreshold, "显式 null → nil")
            try expectNil(p2.tokenAlertFloor, "显式 null → nil")
            try expectEqual(p2.category, .codeEditor, "显式分类生效")

            let withValue = #"{"id":"a","name":"A","icon":"i","cpuWorkingThreshold":12.5,"tokenAlertFloor":1000000,"isCustom":true,"defaultEnabled":false}"#
            let p3 = try JSONDecoder().decode(AgentProfile.self, from: Data(withValue.utf8))
            try expectEqual(p3.cpuWorkingThreshold, 12.5, "显式下限生效")
            try expectEqual(p3.tokenAlertFloor, 1_000_000, "显式 Token 下限生效")
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

        TestKit.test("配置: ActivityLevel 次序包含已完成与待确认") {
            try expectTrue(ActivityLevel.offline < ActivityLevel.idle, "offline < idle")
            try expectTrue(ActivityLevel.idle < ActivityLevel.completed, "idle < completed")
            try expectTrue(ActivityLevel.completed < ActivityLevel.working, "completed < working")
            try expectTrue(ActivityLevel.working < ActivityLevel.attention, "working < attention")
            try expectFalse(ActivityLevel.working < ActivityLevel.working, "同值不严格小于")
            let sorted = [ActivityLevel.working, .offline, .idle].sorted()
            try expectEqual(sorted, [.offline, .idle, .working], "排序结果")
            try expectEqual([ActivityLevel.working, .idle, .offline].max(), ActivityLevel.working, "max")
        }

        TestKit.test("哨兵: CHANGELOG/build-app.sh/README 三处版本一致") {
            // 文档数字哨兵：三处版本漂移曾导致关于页/Info.plist/README 各说各话。
            // 文件缺失（非常规 cwd 运行）时显式跳过，不作断言
            guard FileManager.default.fileExists(atPath: "CHANGELOG.md"),
                  FileManager.default.fileExists(atPath: "scripts/build-app.sh"),
                  FileManager.default.fileExists(atPath: "README.md") else {
                print("   [skip] 非仓库根目录，跳过版本哨兵")
                return
            }
            let changelog = try String(contentsOfFile: "CHANGELOG.md", encoding: .utf8)
            let script = try String(contentsOfFile: "scripts/build-app.sh", encoding: .utf8)
            let readme = try String(contentsOfFile: "README.md", encoding: .utf8)
            let semver = "[0-9]+\\.[0-9]+\\.[0-9]+"
            guard let m1 = changelog.range(of: "## \\[" + semver + "]", options: .regularExpression),
                  let m2 = readme.range(of: "## 功能（v" + semver + "）", options: .regularExpression) else {
                throw TestError(message: "版本标题格式异常，哨兵无法解析")
            }
            func extractVersion(_ text: String, _ range: Range<String.Index>) -> String {
                let seg = String(text[range])
                guard let r = seg.range(of: semver, options: .regularExpression) else { return "" }
                return String(seg[r])
            }
            let changelogVersion = extractVersion(changelog, m1)
            let readmeVersion = extractVersion(readme, m2)
            try expectFalse(changelogVersion.isEmpty, "CHANGELOG 首条版本可解析")
            try expectEqual(readmeVersion, changelogVersion, "README 功能版本 = CHANGELOG 最新条目")
            // build-app.sh 自 R19 起从 CHANGELOG 抽取版本（无硬编码）——哨兵改为验证
            // 「抽取逻辑存在 + 无硬编码版本残留」：手工回填硬编码会破坏单一事实源
            try expectTrue(script.contains("CHANGELOG.md") && script.contains("VERSION="),
                            "build-app.sh 应包含从 CHANGELOG 抽取版本的逻辑")
            try expectTrue(script.range(of: "<string>" + semver + "</string>", options: .regularExpression) == nil,
                            "build-app.sh 不得硬编码版本号（应使用 $VERSION 变量）")
        }

TestKit.test("日志: AGENTISLAND_DEBUG=1 时镜像落 /tmp/agentisland.log") {
            setenv("AGENTISLAND_DEBUG", "1", 1)
            defer { unsetenv("AGENTISLAND_DEBUG") }
            let marker = "applog-mirror-\(UUID().uuidString)"
            AppLog.warn(marker)
            let content = (try? String(contentsOfFile: "/tmp/agentisland.log", encoding: .utf8)) ?? ""
            try expectTrue(content.contains(marker), "镜像文件应包含刚写入的标记行")
        }
    }
}
