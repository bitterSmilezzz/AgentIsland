# R09 验证记录：popover 与收起交互冲突（v0.0.26）

日期：2026-09-12 · 执行：主 agent + 独立验收官（两轮：首轮 FAIL 打回 → 修复 → 复验 PASS）

## 改动（IslandPanel.swift +44/−9）
- isMouseInsidePanelOrFloatingLayers：面板 + 浮层窗口（活体实证类名：_NSPopoverWindow / MenuBarExtraWindow<AnyView> / NSStatusBarWindow）
- evaluateEdgeZone 展开分支、scheduleCollapse 终局守卫、peekForEvent 守卫、两点击监听全部换用
- clickLocalMonitor collapse 延迟一拍复核（Task 在 mouseDown 处理后、mouseUp 前执行；守卫含 grace + 浮层）

## 关键过程（打回与复验）
- 首轮验收官 FAIL：我猜的浮层类名漏了 MenuBarExtraWindow（活体枚举实证），且注释时序模型写反
- 复验活体证据：修复谓词在光标位于菜单栏 popover 内返回 true（同位置上轮 false）；E2E：grace 恒过期的 hover 展开 + 光标驻留 popover 90s 恒 expanded（旧版首拍即计划收起）；移开 3s 内收起（排除 Timer 失效假通过）

## 运行证据
- 175/0 ×2；--selftest 全过；打包重启（0.0.26）

## 已知边界
- MenuBarExtra 无公开 dismiss API，增强 5 以 grace 替代
- 鼠标级端到端待屏幕可用后人工复认

## Backlog（验收官通报，非本轮引入）
- 熄屏/CA 动画挂起场景下 placeWindow(animated: true) 的帧状态与状态机脱钩（一次 EXC_BREAKPOINT 崩溃栈全部在 Apple 布局框架内，与本轮无关）→ 记入 R11 候选
