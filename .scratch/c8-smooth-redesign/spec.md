# Spec: C8 — 侧边栏与菜单栏全方位丝滑度重构（Popover、弹簧动效、防误触与分栏设置）

> 产出：用户反馈「这个插件现在不够丝滑 很笨拙 从设置 侧边栏和顶部栏的ui设计 动画等 做重构优化」，2026-09-06。经两轮 8 项盘问对齐确认。

## Problem Statement

1. **顶部菜单栏体验粗糙**：`.menuBarExtraStyle(.menu)` 只有原生文本菜单，残留旧版提示词，无法直观浏览工作状态与 Token。
2. **侧边栏触发与动效生硬**：全高 24pt 热区在屏幕角落易误触；`0.42s easeInEaseOut` 位移生硬无弹力质感。
3. **设置界面长页堆积**：480x620 单列滚动，表单分类扁平拥挤，缺乏 macOS 标准分栏。
4. **视觉质感欠缺**：磨砂毛玻璃缺少 1px 细微高光勾勒，呼吸动画缺少层级过渡。

## Solution

1. **菜单栏 Popover 升级**：
   - `MenuBarExtra` 改用 `.window` 样式。
   - 打造 `MenuBarPopoverView`：紧凑灵动岛微卡片（宽 300pt），上部为呼吸状态指示灯与概览标题，中部为活跃 Agent 胶囊徽章与 Token 24h 概览，下部为操作条（展开侧边栏 / 设置 / 退出）。
2. **侧边栏物理弹簧与防误触重构**：
   - 热区仅覆盖屏幕垂直居中 60% 区域（避开顶部菜单栏与底部 Dock/通知区），宽度收窄为 12pt。
   - 动画采用 Apple 物理弹簧阻尼模型（Response: 0.34s, Damping Fraction: 0.82）。
   - 展开进场与收起出场与 SwiftUI 内容状态深度协同。
3. **设置界面 SplitView 分栏重构**：
   - 现代 macOS 导航分栏（左侧 4 个类别：通用与外观、Agent 监控、引擎性能、关于）。
   - 右侧内容区采用 Apple Inset-Grouped 卡片式容器，提升各表单项的呼吸感与操作体验。
4. **细节质感打磨**：
   - `GlassCardBackground` 补齐 1px 柔和微高光内描边。
   - 统一主题色与状态胶囊微光效。

## Tickets

- `01-menubar-popover.md`: 菜单栏升级为精致 Popover 状态卡片（替换原生 menu）
- `02-spring-motion-and-edge-zone.md`: 侧边栏 60% 垂直居中防误触热区 + 物理弹簧动效
- `03-splitview-settings.md`: 设置窗口升级为现代 macOS 分栏导航与卡片布局
- `04-visual-polish.md`: 玻璃拟态 1px 微反光边缘与全局视觉打磨

## Verification Plan

- `swift build` 编译无 warning。
- `.build/debug/AgentIslandTestsRunner` 48 个核心测试全绿。
- 人工视觉与交互验证：
  - 点击菜单栏图标弹出精致 Popover 卡片。
  - 鼠标移动到屏幕右侧居中 60% 区域（12pt 内）弹性滑出侧边栏；移开后收回。
  - 打开设置窗口，验证左侧 4 个 Tab 切换正常，所有配置（外观、Agent 开关、自定义增删、滑块）双向绑定与持久化正常。
