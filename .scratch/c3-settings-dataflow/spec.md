# Spec: C3 — 设置系统一条数据流

> 产出：/improve-codebase-architecture 候选 C3 → grilling 四问全按推荐，2026-09-06。

## Problem Statement

同一组默认值写三份（SettingsView @AppStorage / EngineConfig / AppContext fallback 字面量）、cpuThreshold 合法区间写三份（slider range / applyConfig 钳制 / AppContext 内联钳制）；启停集合持久化契约分裂（SettingsView 写 / AppContext 读 / key 字面量三处），最关键的「空数组是主动全关不回退」不变量在 UI target 内零测试守护；外观设置走「通知广播（无 payload）→ IslandPanel 回读 UserDefaults」绕路；自定义 Agent id 规则同文件写两遍。一致性靠「重开设置页 re-apply」兜底——批次 5/8/14/17 的「设置脱节/同步」回归的结构根源。

## Solution

规则下沉 Core：EngineConfig.normalized() 唯一钳制实现 + cpuThresholdRange 常量；SettingKey 常量集 + EngineConfig.load(from:) 替代 AppContext 手写 fallback；EnabledAgentStore 单一 owner（key/编解码/空数组语义），两处回退策略有意保留在调用方；AgentProfile.makeCustomID 与 AgentRegistry.conflictingProcessNames 下沉；外观改 controller 直调（带 payload），删通知与回读。

## User Stories

1. As a 维护者, I want 默认值/键名/合法区间各只有一处定义, so that 改参数不再三处同步
2. As a 维护者, I want 钳制规则是 Core 可测纯函数, so that 「旧版半值自愈」「sample≤idle」有测试守护
3. As a 引擎, I want 启动时读取的配置已归一化, so that 脏持久化值不再带病运行（启动即自愈，原为开设置页才自愈——有意的行为收敛）
4. As a 设置表单, I want 启停集合经单一 owner 存取, so that 空数组语义只有一处解释
5. As a 测试, I want 直接测 store 的 nil/空/有值三态, so that 「全关不回退」不再靠 UI 兜底
6. As a 用户, I want 切换外观立即生效, so that 无需理解通知机制（行为保留，机制简化）
7. As a 添加自定义 Agent 的用户, I want id/冲突校验规则稳定一致, so that 校验提示与实际落库永不打架

## Implementation Decisions

- EngineConfig.normalized()：cpuThreshold 钳入 cpuThresholdRange(1...50)、sample≤idle 钳平；AppContext 启动 load() 与 SettingsView.applyConfig 共用
- SettingKey 常量集（6 行为参数 + enabledAgents/islandAppearance/customAgents/launchAtLogin）
- EnabledAgentStore：load(from:) -> Set<String>?（nil=无记录，回退策略归调用方：AppContext→defaultEnabled 集；SettingsView→引擎当前集，两语义有意不同，注释已论证）/ save(_:to:)（空集合照常写入）
- SettingsView.applyConfig 改为「表单值 → normalized → 一次性 engine.config = c」（didSet 触发一次，替代逐属性六次触发）；表单回写自愈保留
- 外观：SettingsView.onChange → controller.applyAppearance(mode) 直调；IslandPanel.applyAppearance 带 payload；IslandAppearance.storageKey 单一 key；通知名与回读删除
- AgentProfile.makeCustomID / AgentRegistry.conflictingProcessNames：后者拆纯核（registry/installed 集注入，可穷举测试）+ 读全局缓存的便捷入口

## Testing Decisions

- normalized 三态（半值自愈/超上限/sample>idle）；load：无键→默认、脏值→归一化；store 三态（nil/空数组/有值往返）；makeCustomID 大小写与空格；conflictingProcessNames 纯核（fixture registry + canned installed 集合，覆盖启用/已安装/未安装三路）
- 测试用 UserDefaults(suiteName:) 隔离，替代直写 standard + defer 清理

## Out of Scope

- collapseDelay 迁出 EngineConfig（C6）；「重开设置页 re-apply」机制重构（现有机制保留，规则与来源已单点化）
- SMAppService 自启通路（已单一事实来源，勿动）

## Tickets

- 01：Core 规则下沉（normalized/ranges/SettingKey/load/store/makeCustomID/conflict 纯核）+ 测试（无阻塞）
- 02：UI 三视图接线（AppContext/SettingsView/IslandPanel/Theme）+ 通知拆除（阻塞于 01）
