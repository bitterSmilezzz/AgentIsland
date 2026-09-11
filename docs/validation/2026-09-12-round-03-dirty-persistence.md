# R03 验证记录：脏持久化防护（v0.0.20）

日期：2026-09-12 · 执行：主 agent + 独立验收官（轻量）

## 改动
- Models.swift：6 个区间常量 + normalized() 全字段钳制 + NaN 先归位再钳 + load() 非持久化字段守则注释
- SettingsStore.swift：SettingLimits.collapseDelayRange = 0.2...5.0
- IslandPanel.swift：init collapseDelay 钳制（NaN 归位 0.5）+ applyCollapseDelay 同口径 + dockAnchor isFinite
- ConfigTests：169 用例（+3）

## 运行证据
- 169/0 连跑；--selftest 全过；0 警告；打包重启（0.0.20）
- 验收：pass（区间常量与滑杆逐一比对一致；NaN 归位顺序正确；UInt64 转换点输入恒合法；锚点消费点全链路核对）

## 遗留
- collapseDelay 用例只守护常量口径，app 侧接线由评审守护（结构性限制，已注释声明）
