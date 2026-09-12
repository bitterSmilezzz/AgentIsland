# R21/R22 验证记录：数据防丢失 + 口径对齐（v0.0.38 / v0.0.39）

日期：2026-09-12 · 执行：主 agent + 独立验收官（R21 轻量；R22 小轮自验 + 单测矩阵）

## R21（0.0.38）
- 档案逐元素容错（坏元素丢弃 + AppLog；合法空数组不打日志——验收官日志保真修正）
- EnabledAgentStore LoadState 三态 + corrupt 只读降级（绝不写回）
- knownAgents union 口径统一；测试 183
- 验收 pass：corrupt 提前 return 不经任何 save；引擎 setEnabled 仅内存（重放不覆写）

## R22（0.0.39）
- hasPrefixFamilyConflict 共享判定（Core）+ UI 校验调用（单一事实源）；双向矩阵 7 断言
- tokenAlertThreshold 非档位吸附最近档（onAppear）
- defaultBundleScanner 并入 ~/Applications
- 测试 184；selftest 全过；打包重启（0.0.39）

## R22 验收说明
小轮自验：共享判定有 Core 单测矩阵锁定；UI 调用为纯函数替换（等价性由 Core 测试守护）；扫描器目录合并为纯增量。前缀族 UI 行为需在设置页人工验证一次（添加 codex-helper 应被拦）。
