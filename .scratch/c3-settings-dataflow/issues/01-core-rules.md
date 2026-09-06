# 01 — Core 规则下沉：normalized / SettingKey / load / store / makeCustomID / conflict 纯核

**What to build:** 设置规则获得 Core 可调用面：EngineConfig.cpuThresholdRange 常量 + normalized()（cpuThreshold 钳制 + sample≤idle 钳平）；SettingKey 常量集（全部持久化键名唯一来源）；EngineConfig.load(from: UserDefaults)（读键 → 默认打底 → normalized）；EnabledAgentStore（load 返回 nil=无记录 / 空数组照常返回空集 / save 空集照写）；AgentProfile.makeCustomID(_:)；AgentRegistry.conflictingProcessNames 纯核（registry/installedCLIs/installedBundles 注入）+ 读全局缓存便捷入口。runner 新增对应测试（UserDefaults suiteName 隔离）。

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] normalized 三态用例：半值自愈（0.5→1）/ 超上限（100→50）/ sample>idle 钳平 / 正常值不动
- [ ] load 用例：无键返回默认 / 脏值归一化；store 三态用例（nil / 空数组 / 有值往返）
- [ ] makeCustomID 与 conflict 纯核用例（启用/已安装/未安装三路 + 进程名大小写归一）
- [ ] 测试不碰 standard UserDefaults（suiteName 隔离）；`swift build` + runner 全绿

## Comments

- 2026-09-06 发布。
