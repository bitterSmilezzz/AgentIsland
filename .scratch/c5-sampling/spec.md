# Spec: C5 — 采样生命周期轻量收尾（弹窗决策：轻量 + 单调 merge）

> 产出：/improve-codebase-architecture 候选 C5 → grilling 弹窗两问均按推荐，2026-09-06。
> 重估结论：C1/C4 已收编生命周期两大块，完整 Sampler 抽取边际收益低于回归风险 → 决策不抽，以 ADR-0001 存档。

## Problem Statement

「目录最近写入时间」概念存于两处：FileActivityMonitor.cache（整表替换写回）与 ActivityEngine.lastWrites（引擎为兜底替换语义自建的影子缓存，max 合并）。改口径要动两处，无 locality。

## Solution

FileMonitor 写回改逐目录单调 max（扫描失败/目录暂缺保留旧值、写入时间只进不退——与引擎现兜底语义一致）；引擎删 lastWrites 影子缓存，直读监控缓存。采样节律本身不动（ADR-0001）。

## User Stories

1. As a 维护者, I want 最近写入时间只有一个家, so that 改口径只动一处
2. As a 消费方, I want 监控缓存写回单调, so that 不必自建副本兜底
3. As a 后续贡献者, I want 「为什么不抽 Sampler」有 ADR, so that 六个月后不再被重复提议

## Implementation Decisions

- runScan 写回：cache 由整表替换改逐目录 max merge（watchedDirs 竞态防护保留；sessionCounts 仍整表替换——计数是时点值非单调值）
- 引擎：lastWrites 字段、merge 块、refreshWatchedDirs 清理全删；newestAgo 直读 lastWriteDates
- FileActivityMonitor.init 增加 scanMinInterval 参数（默认 15s 不变；测试传 0 使 merge 语义可测）
- ADR-0001 记录不抽 Sampler 的决策与重开条件

## Testing Decisions

- 新用例：首扫有值 → 目录移除（扫描失败）→ 再扫 → 旧值保留（merge 语义，scanMinInterval=0 隔离节流）
- 现有引擎用例（写入升级/双信号）全绿即等价

## Out of Scope

- Sampler 抽取（ADR-0001）；FileMonitor 快跳过/节流策略调整

## Tickets

- 01：单调 merge + 影子缓存退役 + merge 语义测试 + ADR-0001（单票）
