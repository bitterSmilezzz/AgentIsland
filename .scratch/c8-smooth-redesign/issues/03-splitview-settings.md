# 03 — 设置窗口升级为现代 macOS 分栏导航与卡片布局

**What to build:**
重构 `SettingsView`：
- 从单列大长滚动页升级为现代 macOS `NavigationSplitView` / 侧边导航栏架构。
- 4 大模块导航：
  1. **通用与外观**：灵动岛卡片外观（跟随系统/浅色/深色）、开机自启开关、鼠标离开自动收起延迟滑块。
  2. **Agent 监控**：内置 Agent 启停开关列表、自动发现 CLI Agent 启停列表、自定义 Agent 列表与新增弹窗。
  3. **引擎与性能**：工作中写入判定窗口、CPU 判定阈值、活跃会话窗口、活动采样间隔、闲置降频间隔。
  4. **关于**：应用版本、GitHub 仓库链接、只读监控说明、开发团队信息。
- 右侧内容区采用现代 Inset-Grouped 卡片式分块与自适应排版，滑块与文本拥有优雅间距与即时反馈。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] 拆分为 4 个子视图 / 导航模式
- [x] 保持与 `EngineConfig`、`SettingsStore`、`ActivityEngine` 的完整数据流响应
- [x] 确保新增自定义 Agent 与安装缓存扫描刷新功能保持 100% 健全

## Comments

- 2026-09-06 完成：SettingsView 升级为现代 NavigationSplitView 架构，左侧 4 个类别导航，右侧 Inset-Grouped 卡片容器，表单、滑块、增删自定义与启停保持完整数据绑定与实时响应。
