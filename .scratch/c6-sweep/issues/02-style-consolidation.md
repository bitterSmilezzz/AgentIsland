# 02 — 样式收敛：CardShell/HoverRow/拖动/分割线/Loading 单一实现 + Theme 令牌补齐

**What to build:** 新建 IslandComponents.swift：CardShell（width/maxFrame/GlassCardBackground 三连 ×3 处归一）、HoverRow（可 hover 圆角行 ×3 归一）、cardDrag modifier（同款 DragGesture ×2 归一）、DarkDivider（overlay 12% ×4 归一）、LoadingRow（居中加载行 ×2 归一）；Theme 补残留令牌（dockedSliver 填充 0.55/0.34/0.85 + 描边 0.22、蒙层 0.42、面板阴影 0.30/16/-6）。queryToken 代际模式不抽。

**Blocked by:** None — can start immediately（与 01 无依赖，排后减小审查面）。

**Status:** resolved

- [ ] 卡片外壳三连/hover 行/拖动手势/分割线/loading 行各只剩一处定义（grep 验收）
- [ ] Theme 外硬编码色值清零（dockedSliver/蒙层/阴影）
- [ ] 界面逐像素不变（视觉验收：三路由 + 详情页 + docked 态）
- [ ] `swift build` + runner 全绿

## Comments

- 2026-09-06 发布（弹窗未答，按推荐执行）。
- 2026-09-06 完成。
