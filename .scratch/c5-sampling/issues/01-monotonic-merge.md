# 01 — FileMonitor 单调 merge 写回 + 引擎影子缓存退役 + ADR-0001

**What to build:** 最近写入时间单一事实来源：FileMonitor 写回改逐目录 max（失败保留旧值），引擎删 lastWrites 影子缓存直读缓存；scanMinInterval 可注入使 merge 可测；ADR-0001 记录「不抽 Sampler」决策。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [ ] FileMonitor cache 写回为单调 merge；sessionCounts 仍整表替换
- [ ] 引擎零 lastWrites 残留（grep）
- [ ] merge 语义测试（目录暂缺保留旧值）+ 现有引擎用例全绿
- [ ] docs/adr/0001 落盘；`swift build` + runner 全绿 + `--probe` 冒烟

## Comments

- 2026-09-06 发布（弹窗决策：轻量收尾 + 单调 merge，均按推荐）。
- 2026-09-06 完成。
