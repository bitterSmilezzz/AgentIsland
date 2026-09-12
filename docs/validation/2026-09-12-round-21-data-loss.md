# R21 验证记录：用户数据防丢失（v0.0.38）

日期：2026-09-12 · 执行：主 agent + 独立验收官（轻量）

## 改动
- AgentRegistry.loadCustomProfiles 逐元素容错（坏元素丢弃 + AppLog；合法空数组不打日志——验收官指出的日志保真缺陷已修）
- EnabledAgentStore LoadState 三态 + corrupt 只读降级（绝不写回，损坏存档保留可恢复原状）
- 全关分支 knownAgents union 口径统一
- 测试 180 → 183

## 验收证据
- corrupt 提前 return 不经过任何 save/saveKnownAgents；引擎 setEnabled 只改内存不写 defaults（重放不覆写）
- 防覆写断言真实性：若存在写回必红
- 183/0 ×2 + selftest 全过；打包重启（0.0.38）
