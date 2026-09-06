# 01 — IslandMetrics 单一来源

**What to build:** 窗口几何只有一个家：新建 IslandMetrics（UI target）持有全部尺寸常量（docked 176×5、cardWidth 280、expandedMaxHeight 420、header/detailHeader/divider/row/listMax/emptyState/summaryBar/detailContent 度量）与纯函数 expandedHeight(route:visibleCount:hasSummary:)。IslandPanel 删除自己的尺寸常量组与 7+1 个镜像常量，expandedHeight 消费纯函数；IslandView（dockedSliver 176×5、cardWidth 280、maxHeight 300、header/空态 padding）与 DetailViews（280×2）改引 IslandMetrics。动画编排（placeWindow/animateCornerRadius/syncExpandedHeight）零改动。

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] IslandPanel 内不再有任何布局镜像常量（grep headerHeight/rowHeight/emptyStateHeight 等仅 IslandMetrics）
- [ ] 280 / 300 / 176×5 在全 target 各只有一处定义
- [ ] expandedHeight 为纯函数，同输入同输出（窗口几何逐值不变）
- [ ] `swift build` + runner 全绿 + `--probe` 冒烟

## Comments

- 2026-09-06 发布。
