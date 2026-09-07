# 更新日志 (CHANGELOG)

所有关于 AgentIsland 的重要版本演进与功能更新均记录在此。

---

## [1.5.0] - 2026-09-07

### ✨ 核心新特性 (Practicality & Command Center)
- **任务完成主动提醒与智能微窥 (Event Peek & Sound)**:
  - 智能体经历持续工作（≥3.5秒）后转为空闲时，自动播放 macOS 原生轻脆 `Glass` 提示音；
  - 处于 6pt 贴边收起态时，自动平滑滑出 3.5 秒 Peek 微弹窗；若光标移入则自动升级为常驻展开态，移开后平滑收回；
- **终端与 IDE 窗口一键深度直达 (Window Deep-Linking)**:
  - 毫秒级递归追溯进程树父节点，精准定位承载 CLI 智能体（Claude Code、Codex、Dim 等）的 GUI 终端窗口（Terminal、iTerm2、VS Code、Cursor、Ghostty、Warp 等），一键拉至最前聚焦；
  - GUI 智能体（Cursor、Trae、DimAgent 等）直接通过 BundleID / PID 唤醒；
  - 任务完成横幅与 Agent 列表行均提供「直达」快捷按钮；
- **实时操作与工具调用透视 (Real-time Action Context)**:
  - 自动提取子进程实时执行的系统命令（如 `swift test`、`git diff`、`npm run build` 等），内置智能格式清洗器剥离 shell 包裹层；
  - 解析 DimAgent / Claude / Codex 运行时会话日志与 tool_use 元数据，顶部卡片与列表行实时呈现终端样式徽标（如 `> 正在修改: IslandView.swift`）；
  - CLI `--probe` 终端诊断命令新增 `ACTION` 实时动作列；
- **成本与异常死循环熔断保护 (Runaway Loop & Cost Circuit Breaker)**:
  - **Token 暴涨告警**：基于滑动窗口差分监测单分钟 Token 增量，超过阈值时触发红色/琥珀色高亮告警并播放低沉警示音；
  - **长耗时死循环告警**：高负荷工作超 3 分钟未释放时自动预警；
  - **一键 Kill 逃生舱**：告警横幅提供红色「熔断」按钮；列表行提供红色停止按钮，并配有 3 秒自动取消的「终止?」二次防误触确认，安全杀死整棵子进程树；
- **设置面板扩展 (Circuit Breaker Settings)**:
  - 「引擎与性能」Tab 新增「成本与异常熔断保护」配置卡片，支持 30k/50k/100k/200k tokens 阈值调节及各项开关。

### ⚡️ 质量与稳定性 (Quality & Testing)
- 测试套件扩充至 53 个全自动化测试用例，全绿通过；
- `--selftest` 进程内自检断言全部通过。

---

## [1.4.0] - 2026-09-07

### ✨ 交互与界面革新
- **自由移动与智能贴边吸附**: 支持按住顶栏全屏幕任意拖拽，松手根据物理距离智能吸附到屏幕顶部或右侧，并持久化锚点坐标；
- **6pt 晶莹微细条与弹性弹出**: 收起时保留 6pt 半透明微细条（含呼吸绿灯），光标碰触以流体弹簧动效自动弹出完整卡片；
- **现代分栏设置窗口**: NavigationSplitView 四大分类架构，支持贴边重置与外观切换；
- **顶部菜单栏 Compact Popover**: 现代原生浮窗浮动展示活跃 Agent 概览。
