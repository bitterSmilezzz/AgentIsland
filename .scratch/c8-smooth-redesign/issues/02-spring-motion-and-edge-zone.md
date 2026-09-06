# 02 — 侧边栏 60% 垂直居中防误触热区 + 物理弹簧动效

**What to build:**
1. **热区防误触改造**：
   - 宽度收窄至 12pt（原 24pt 过宽易扫碰）。
   - 垂直范围限制为屏幕居中 60% 区域（`visibleFrame.midY ± height * 0.3`），彻底避开顶部菜单栏、窗口关闭红绿灯与底部通知中心/Dock。
2. **物理弹簧动效**：
   - 侧边栏进出动画由 `easeInEaseOut 0.42s` 升级为真实系统弹性弹簧（Response ~0.34s, Damping Fraction ~0.82）。
   - 进场时带自然柔和微回弹，退场干脆利落。
   - 动画过程与内部 SwiftUI 内容无缝协同，消除窗口 resize 带来的边缘闪烁。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] `isMouseInRightZone` 升级为 60% 垂直中段检测 + 12pt 宽度
- [x] 动画迁移为带柔和阻尼的 CASpring / spring timing
- [x] peek 动画同步迁移并打磨时间
- [x] 消除收放动画中的布局白边与滞后

## Comments

- 2026-09-06 完成：右缘热区收窄为 12pt，Y 轴限制在 visibleFrame.midY ± 30% 屏幕中段；滑入滑出采用 Apple Spring 阻尼动效，阴影与内容同步对齐。
