# R43 验证记录：四边贴靠与自动收起修复（v0.0.60）

## 目标

修复灵动岛不能自动收起的问题，并将拖拽停靠从顶部/右侧扩展为上、右、下、左四边。

## 根因

浮层守卫将 `NSStatusBarWindow` 当作岛的交互浮层；它只是状态栏宿主，frame 可能大于实际图标。鼠标离开岛后仍可能被误判为停在浮层中，令延时收起复核持续取消。

## 覆盖

- `IslandPanelInteraction.isMouseInsideFloatingLayer` 统一了真实交互浮层、可见性与鼠标命中三项判断；状态栏宿主被明确排除，面板层仅负责提供 AppKit 窗口状态。
- `DockEdge` 现有 `top`、`right`、`bottom`、`left` 四项。松手时以面板外框到 `visibleFrame` 四边的最近距离确定停靠边；顶部/底部保持 X 锚点，左侧/右侧保持 Y 锚点。
- 微细条、点击穿透矩形、悬停热区、离屏收起原点、展开落点、镜像倒角、收起箭头、tooltip 箭头和 VoiceOver 标签均覆盖四个方向。
- Token 分析页发现一处既有的构建阻塞：调用方传入已不存在的 `isCost` 参数。已按组件现有签名对齐，未改变费用展示样式。

## 自动验证

- 新回归：`NSStatusBarWindow` 无论 frame 是否命中都不阻止自动收起；真正命中的 `_NSPopoverWindow` 继续保持展开。
- 新回归：四个典型窗口位置分别选择 `top` / `right` / `bottom` / `left`。
- `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk swift build --build-tests` 通过。
- `AgentIslandTestsRunner`：**221/0**。
- 在已完成全量测试门禁后，`SKIP_TESTS=1 SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk ./scripts/build-app.sh` 通过，`dist/AgentIsland.app` 为 **0.0.60**，ad-hoc 签名验证通过。
- 已按工作约定重启 `AgentIsland`；辅助功能快照确认运行中的收起入口可见，标签为“右侧贴边”。
