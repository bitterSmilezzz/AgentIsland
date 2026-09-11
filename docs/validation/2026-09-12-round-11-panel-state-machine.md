# R11 验证记录：面板状态机（v0.0.28）

日期：2026-09-12 · 执行：主 agent + 独立验收官（两轮：首轮活体测试 FAIL 打回 → 修复 → 复验 PASS）

## 改动（IslandPanel.swift +86/−8）
- peek 连发事件重排（代际保护 + 取消竞窗守卫）；syncExpandedHeight 拖拽守卫
- U3 幽灵命中区：updateClickThrough 动态 ignoresMouseEvents（细条可视矩形 ±8pt，sliverRect 与 placeWindow docked 数学同源）
- 睡眠/唤醒：CGDisplayIsAsleep 直落终态帧；NSWorkspace center 的 screensDidWake/didWake 唤醒重放

## 关键过程（打回与复验）
- 首轮 FAIL：① 细条判定误用整卡 frame（恒 no-op，lldb 实测幽灵柱内 ig=false）；② 唤醒通知挂 default center（熄屏唤醒不投递，死代码）——均由验收官 lldb 附着 + cliclick 活体定位
- 复验活体证据：right/top 贴边幽灵区穿透生效（同点位 ig true）、细条 hover 展开正常、人为破坏 frame 后唤醒自动重放终态（E2E）、移开收起不粘滞

## 运行证据
- 176/0 ×2；--selftest 全过；打包重启（0.0.28）

## Backlog（验收官通报）
1. 菜单栏内细条 x 跨度正上方光标导致 expand/collapse 振荡（expand 热区 y 无上界，既有几何行为）
2. sliverRect 极端锚点下与 SwiftUI 渲染位置最多偏差 ~25pt（可改从 panel.frame 中心推导）
3. deinit 未移除 workspace center 观察者（控制器同生命周期，无实际影响）
