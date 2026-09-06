# 01 — 贴边状态模型与自由拖拽手势

**What to build:**
- 定义 `DockEdge: String, Codable { case top, right }`。
- 在 `SettingKey` 中增加贴边与锚点键：`dockEdge`, `dockAnchorX`, `dockAnchorY`。
- 在 `IslandComponents` 恢复 `cardDrag` 手势修饰符。
- 在 `IslandPanelController` 中实现 `dragMoved(translation:)` 与 `dragEnded()`。
- 卡片展开时，主列表顶栏与详情页顶栏均可拖拽。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] `DockEdge` 枚举与 `SettingKey`
- [x] `cardDrag` 手势修饰符
- [x] `dragMoved` 与 `dragEnded`

## Comments
已完成并验证。
