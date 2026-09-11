# R12 验证记录：主题可读性 + 死代码（v0.0.29）

日期：2026-09-12 · 执行：主 agent + 独立验收官（轻量，含 WCAG 实算）

## 改动
- LiveLogStreamView：thinking/toolCall 徽标双值动态色（thinking 浅 5.79:1、toolCall 深 4.54:1）
- AgentRingView：isHung → ringRed（熔断语义，与卡死徽标同源）
- 死代码批删 18 符号：TooltipTail、showNumericBadge/badgeText、eventBannerHeight、islandAppearance Binding、ProcessMatcher.cpuPercent、Theme 13 令牌
- IslandPanel：R09 遗留 assumeIsolated 两警告清零
- 测试 176/0（全量重编译 0 警告）

## 声明偏离（验收官认定合理）
<9pt 字号统一延后——涉及 IslandMetrics 实测校准常量 + 熄屏无法可视化验收

## 验收证据
- WCAG 对比按 18% badge 底混合实算；isHung 传染面核对（普通高 CPU 不染红）；死代码全仓零残留；漂移哨兵存活
