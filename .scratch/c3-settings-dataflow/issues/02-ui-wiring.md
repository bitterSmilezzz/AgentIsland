# 02 — UI 接线：AppContext / SettingsView / IslandPanel / Theme 换装 + 通知拆除

**What to build:** UI 全部改走 01 的 Core 面：AppContext.init 用 EngineConfig.load(from:)（删 6 行 fallback 字面量 + 内联钳制）与 EnabledAgentStore.load（nil→defaultEnabled 集）；SettingsView 的 @AppStorage 键引 SettingKey、默认值引 EngineConfig()、applyConfig 改 normalized + 一次性 engine.config 赋值、loadState/saveEnabled 走 store、AddCustomAgentSheet 用 makeCustomID 与 conflict 便捷入口；外观改 controller.applyAppearance(mode) 直调（IslandPanel 带 payload，init 恢复走 IslandAppearance.storageKey），通知名与 UserDefaults 回读删除；AgentRegistry customAgents 键换 SettingKey；RegistryTests 键字面量换常量。行为逐值等价（除启动即自愈的有意收敛，见 spec）。

**Blocked by:** 01 — Core 规则下沉

**Status:** resolved

- [ ] 默认值/键名/区间在 UI target 内零字面量残留（grep 验收）
- [ ] engine.config 赋值一次性（didSet 单次触发）
- [ ] 外观：通知名扩展删除，直调生效（含 init 恢复路径）
- [ ] `swift build` + runner 全绿 + `--probe` 冒烟；设置页手测：拖滑块/切启停/换外观/加删自定义

## Comments

- 2026-09-06 发布。
- 2026-09-06 完成。grep 验收零字面量残留；engine.config 整体赋值经核实等价（minWorkingHold 无非默认路径）。
