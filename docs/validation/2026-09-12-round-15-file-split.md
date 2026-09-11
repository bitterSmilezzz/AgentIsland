# R15 验证记录：结构拆分与吸附收敛（v0.0.32）

日期：2026-09-12 · 执行：双执行者并行（A: IslandView 拆分 / B: IslandPanel 收敛）+ 独立验收官（轻量）

## 改动
- IslandView.swift 1168 → 513：AgentRowView(240)/TokenSummaryBar(47)/DockedSliver(137)/EventBannerView(252) 字节级纯搬移；CardRoute 留原位（哨兵）
- dockTargetFrame 单实现（placeWindow + snapToDockEdge 双消费点，逐行等价）
- ShadowHostView 删除（容器改 NSView，假调用删除）；注释去引用
- 测试 176/0 + test-token-layout.py PASS

## 验收证据
- 字节级纯搬移证明（diff exit=0）、dockTargetFrame 与 HEAD 两处旧计算逐行一致、哨兵用例通过、shadowHost 层级不变
