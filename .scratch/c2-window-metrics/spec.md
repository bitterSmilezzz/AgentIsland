# Spec: C2 — 窗口几何单一来源（IslandMetrics）

> 产出：/improve-codebase-architecture 候选 C2 → grilling 两问全按推荐（Q1 单一来源计算式；Q2 度量放 UI target），2026-09-06。

## Problem Statement

IslandPanel.expandedHeight() 用 7 个镜像常量（header 37 / detailHeader 47 / divider 1 / row 48 / listMax 300 / emptyState 87 / summaryBar 28 / detailContent 310）逐项复刻 IslandView 的 padding 实现，注释自证「任一 padding 改动需同步此处」；宽度 280 在 IslandPanel/IslandView/DetailViews 三处独立定义，docked 176×5 与 listMaxHeight 300 亦双份。改一处布局必须同步多处，漏改即裁切/留白——批次 6/7/10/11/14 五轮「布局对齐/高度精确对齐/空态/毛刺」回归全部源于此。

## Solution

立 IslandMetrics 小 module（UI target）：全部窗口几何常量定义一次 + 纯函数 expandedHeight(route:visibleCount:hasSummary:)；IslandPanel 删除全部镜像常量、消费该函数；IslandView/DetailViews 的 frame/padding 引用 IslandMetrics。布局度量只有一个家。

## User Stories

1. As a 维护者, I want 布局度量只定义在一个文件, so that 改 padding 不再需要跨文件同步
2. As a 面板控制器, I want 消费一个纯函数取窗口高度, so that 我不再持有任何布局知识
3. As a 后续 agent, I want 度量的推导注释与常量同处, so that 理解「37 从哪来」零跳转
4. As a 用户, I want 窗口高度与内容严丝合缝, so that 无裁切无留白（行为逐值保留）
5. As a 用户, I want docked/expanded/peek 动画手感不变, so that 交互资产不受影响（placeWindow 编排零改动）

## Implementation Decisions

- 方案 A（单一来源计算式），不做 NSHostingView.sizingOptions 系统撑高（保住 placeWindow 动画编排与已打磨动效）
- IslandMetrics 放 UI target；几何可测性接受不达成（runner 只依赖 Core，为常量建测试 target 属 YAGNI；守护靠 --probe + 视觉冒烟）
- 度量推导注释随常量走；IslandView 相关 padding/frame 改引 IslandMetrics，使「布局与算高」的耦合落在一个文件内
- rowHeight 48 等实测常量保留推导注释（阿剩低4：46 低估会裁最后一行）

## Testing Decisions

- 纯函数同输入同输出（窗口几何逐值不变是硬约束）
- `swift build` + runner 全绿；`--probe` 实机冒烟；docked↔expanded 切换与空态/列表/详情三路由视觉验收

## Out of Scope

- CardShell/HoverRow 样式收敛、Theme 残留硬编码（C6）
- collapseDelay 迁移（C6）
- 系统驱动撑高（未来候选）

## Tickets

- 01：IslandMetrics 单一来源（无阻塞，单垂直切片）
