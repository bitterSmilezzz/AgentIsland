# R16 验证记录：测试卫生（v0.0.33）

日期：2026-09-12 · 执行：主 agent（活体调试 sweeper 竞态）+ 全量运行证据

## 改动
- TestDefaults 登记制套件管理：12 处散点迁移、runAll 末统一清理 + 泄漏自守护 + 退出后独立进程 3 秒重试清扫（应对 cfprefsd flush 竞态）
- AgentRegistry defaults 注入（loadCustomProfiles/saveCustomProfiles/fullRegistry，默认 .standard）
- 环境用例 skip 化 + sleeper.run() 显式断言
- 测试 176/0

## 活体调试记录
1. 进程内删除 + 断言通过，但退出后 cfprefsd flush 重建文件（单跑 +3s 检查 0 残留 ✓；连跑两轮第一次 sweeper 0.8s 早于 flush 冒出 28 个）→ 清扫改 3 秒循环重试 → 连跑 3 轮零残留
2. 历史 2400+ 残留一次清零（agentisland-* 前缀精确匹配，真实 app 域不受影响）

## 验收证据
- 176/0 ×5；测试卫生断言内建在 runner 退出码（残留即失败）
