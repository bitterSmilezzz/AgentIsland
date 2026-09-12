# R31 验证记录：WorkBuddy 国内版/国外版区分（v0.0.47）

日期：2026-09-12 · 执行：主 agent（用户实测驱动）

## 用户实测事实（含更正）
- 本机装有 `WorkBuddy.app`（com.tencent.workbuddy.mac = 国内版腾讯系）与 `WorkBuddy AI.app`（com.workbuddy.workbuddy = 国外版）
- **两个变体均可活跃**：~/.workbuddy-ai 今天全天活跃；~/.workbuddy 昨天仍在用（db 昨晚 23:41、tasks 内 77/82.json 近两天写入——初版记录误把 tasks 顶层目录 mtime 当作停更证据，已更正）

## 旧档案的混合误配
bundleIDs 只认国内版 + sessionDirs 只读国内版停更数据 + pathContains 宽口径 "workbuddy" 反向吸走国外版 Electron 进程 → 显示的 WorkBuddy 状态 = 国内版弃用数据 + 国外版进程

## 改动
- Registry：拆两条独立档案（workbuddy / workbuddy-ai，globe 图标），pathContains 精确到各自 .app 目录与数据目录（含尾部斜杠防前缀串线）
- inspectWorkBuddyAction(agentId:) 与 fetchWorkBuddyEvents(agentId:) 按变体分发数据目录（workbuddyDataDir(for:) 唯一口径）
- 新增双向隔离测试；probe 实证：WorkBuddy 显示最近活动 26h 前（≈昨天下午，与用户「昨天还在用」一致）+ WorkBuddy AI WORKING 45.5%（1 活跃会话）
- 测试 185/0

## 运行证据
- 测试 185/0 连跑；selftest 全过；0.0.47 打包成功；应用重启
- INST 列 WorkBuddy AI 显示 "no" 是 bundle 扫描缓存 300s 未刷新所致（defaultBundleScanner 已并入 ~/Applications，缓存刷新后自愈）——已复核为展示层时序非缺陷
