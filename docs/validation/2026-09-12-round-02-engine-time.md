# R02 验证记录：引擎时间正确性（v0.0.19）

日期：2026-09-12 · 执行：主 agent + 独立验收官

## 改动
- ActivityEngine.sampleCore：每拍重锚 workingSince/lastSignalAt/highCpuSince/lastRunawayAlertedAt（晚于本拍即重锚）
- newestAgo 负值钳 0（未来 mtime 防御）
- offline 清 tokenRateBaseline；checkCostSpikeAndRunaway 基线时间戳重锚；tokenRateBaseline private → internal（@testable）
- 测试 163 → 166（+FakeTokenUsageProvider）

## 设计偏离（验收官认定成立）
单调时钟迁移 → 回拨安全的墙钟（重锚 + 钳制）。理由：sample(now:) seam 49 处、ADR-0001；前跳由 resumeGap 兜底。

## 运行证据
- 166/0 多次连跑；--selftest 全过；0 警告
- 验收官变异实验：删防御代码 → 164/2（重锚用例红，证实卡死 bug 真实且被修复）
- 应用已打包重启（0.0.19）

## 遗留
- 「未来 mtime 告警事件」未实施（负值已不外泄，发事件反成骚扰）——验收官认可
