# 01 — 菜单栏升级为精致 Popover 状态卡片

**What to build:**
将 `MenuBarExtra` 从 `.menu` 升级为 `.window` 样式。实现 `MenuBarPopoverView`，呈现灵动岛状态微卡片：
- 顶部：发光呼吸状态灯、工作中的 Agent 计数 / 待机状态摘要
- 中部：活跃 Agent 微缩胶囊列表（显示 Agent 图标、名称、活动状态，以及 24h Token 用量徽标）+ 24h/累计 Token 总额概览
- 底部：精美的操作栏（「展开侧边栏」、「设置…」、「退出」按钮）
- 修复旧有「提示：鼠标碰屏幕顶部滑出卡片」的错误文字

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] `MenuBarExtra` 声明为 `.window`
- [x] `MenuBarPopoverView` 实现并适配深浅色
- [x] 点击「展开/收起侧边栏」联动 controller
- [x] 顶部栏图标与弹出面板状态实时刷新

## Comments

- 2026-09-06 完成：MenuBarExtra 从 .menu 升级为 .window，Popover 卡片实现完整状态指示、活跃 Agent 列表、Token 统计与操作栏。旧提示文案已清除。
