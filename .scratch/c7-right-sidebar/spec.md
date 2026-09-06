# Spec: C7 — 顶部悬浮岛改为右侧隐藏侧边栏

> 产出：用户反馈「在上面有点扰乱」，2026-09-06。grilling 弹窗两轮未答，按既定模式取推荐：替换（顶部移除）/ 右缘热区 hover + 菜单保留 / 收起完全隐藏 / 不加设置项。

## Problem Statement

顶部常驻悬浮岛（即使收起也有 176×5 细条 + 热区行为）扰乱屏幕顶部。

## Solution

灵动岛整体移到屏幕右缘、垂直居中，贴边成侧边栏；收起时完全滑出屏幕外（右缘无残条）；鼠标碰右缘热区（24pt）滑出，离开 + collapseDelay 收回；菜单栏「展开/收起灵动岛」保留；顶部岛、拖动定位、细条全部移除。

## User Stories

1. As a 用户, I want 屏幕顶部完全干净, so that 不再被常驻细条与热区打扰
2. As a 用户, I want 鼠标碰右缘即滑出侧边栏, so that 查看状态零成本
3. As a 用户, I want 鼠标离开后自动收回, so that 侧边栏默认不可见（collapseDelay 设置继续生效）
4. As a 用户, I want Agent 转为 working 时侧边栏自动滑出提醒再收回（peek 保留），so that 不盯屏幕也不错过
5. As a 用户, I want 菜单栏仍可展开/收起, so that 无鼠标热区时也有入口
6. As a 维护者, I want 拖动/锚点持久化/圆角形变/重置位置整组删除, so that 侧边栏贴边定位无额外状态

## Implementation Decisions

- 面板尺寸恒为卡尺寸（280×内容高）；docked/expanded 只差位置——docked 滑出屏外 2pt，expanded 贴右缘 flush、垂直居中（高度钳制 visibleFrame 内边距 24）
- 热区：右缘 24pt 全高条带；事件驱动监控（local+global mouseMoved）沿用，10Hz 节流沿用
- Peek：docked 下整体滑到可见位停留 0.5s 再收回（原下探 11pt 改为滑出），30s 冷却保留
- 贴边造型：右缘 flush、仅左侧两角 18pt 圆角——GlassCardBackground 用 UnevenRoundedRectangle（macOS 13+），clipContainer 用 maskedCorners，shadowPath 用 byRoundingCorners
- 圆角形变动画（animateCornerRadius）、拖动（dragMoved/dragEnded/锚点持久化/resetPosition/dragCooldown）、dockedSliver、islandHoveredChanged、菜单「重置岛位置」全部删除；dragCooldown 语义改名为 expandCooldownUntil（菜单收起后 1.2s 内热区不重展开）
- IslandMetrics.dockedWidth/Height、Theme.dockedSliverFill/Stroke、IslandComponents.cardDrag 随之删除（零死代码）
- collapseDelay 设置与 setPresentationActive 通路不变；detail 页拖动属性（onDragMoved/onDragEnded）删除

## Testing Decisions

- UI target 不可测（既定）：build + runner 全绿 + `--probe` 冒烟（probe 不建面板不受影响）+ 人工视觉验收：右缘滑出/收回/peek/三路由/深浅色
- Core 测试零改动即全绿（引擎不涉及）

## Out of Scope

- 位置设置项（切回顶部需改代码）；多屏热区细节（沿用 screenContainingMouse 语义）

## Tickets

- 01：右侧侧边栏替换顶部悬浮岛（单垂直切片）
