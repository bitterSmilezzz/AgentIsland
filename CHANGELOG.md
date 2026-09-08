# 更新日志 (CHANGELOG)

所有关于 AgentIsland 的重要版本演进与功能更新均记录在此。

---

## [1.7.1] - 2026-09-08

### 🔧 Agent 识别与状态检测全面修复

- **修复 ChatGPT/Codex BundleID 冲突**：`com.openai.codex` 实为 ChatGPT 桌面版，已独立为 ChatGPT profile，Codex 改为纯 CLI 检测
- **新增 ChatGPT 内置 profile**：`com.openai.codex` bundleID，独立监控 ChatGPT 桌面版运行状态
- **新增 DSH (DeepSeek Harness) 内置 profile**：进程名 `dsh` + pathContains `deepseek-harness`，会话目录 `~/.dsh/sessions` & `~/.dsh/storages`
- **修复 OpenCode.app 不被识别**：添加 `ai.opencode.desktop` bundleID，GUI 与 CLI 双路径均可检测
- **修复 Hermes sessionDirs 路径错误**：`~/.local/share/hermes`（不存在）→ `~/.hermes/sessions` + `~/.hermes/logs`（实际数据位置）
- **Probe CPU 双拍差分**：`--probe` 改为两次采样（1.5s 间隔），输出真实 CPU% 窗口值（修复永远 0.0 的问题）
- **闲置态采样提速 3×**：`idleSampleInterval` 从 15s 降至 5s，Agent 开始工作后最迟 5s 即被感知
- **InstalledAppsCache 同步**：`knownBundleIDs` 与 `knownCLIs` 与注册表完全同步

---

## [1.7.0] - 2026-09-07

### 💎 深度设计重构（CodeNotch 灵感：一体化反向倒角、微仪表环与悬停透视卡片）
- **反向倒角一体化贴边造型（SideNotchShape / Bezel Flares）**:
  - 彻底去除普通矩形切边的生硬感，借鉴 CodeNotch 与苹果硬件刘海的数学级 Bézier 曲线反向倒角（Flare）；
  - 小岛在屏幕右侧或顶部停靠时，贴边边缘自然平滑向屏幕边框弯曲延伸，如同从屏幕边框一体化生长出来；
  - 毛玻璃拟态、双色渐变蒙层、晶莹描边与窗口投射高斯阴影全链路对齐反向倒角曲线。
- **环形微仪表盘与双层动态活动弧（AgentRingView & ActivityArc）**:
  - 为所有 Agent 列表项与快速微看板引入 4 级彩色水位环（荧光绿、琥珀黄、预警橙、极光红）；
  - 居中渲染智能体高辨识度专属 Glyph；
  - **双层动效**：Agent 工作时内圈展开 0.25 长度的极细旋转微弧（1.2s 周期平滑旋转）；等待确认时呈现琥珀色呼吸警戒环；
  - 展开卡片顶部新增活跃智能体微看板（Quick Rings Shelf），一眼看清全局负载。
- **精准悬停透视卡片与指向小尾巴（AgentHoverTooltip & TooltipTail）**:
  - 鼠标悬停在任意 Agent 环上时，滑出带有指向尖角的悬浮透视浮层；
  - 零点击直达实时事实：当前执行的具体命令/正在修改的文件、进程 PID、24h/累计 Token 消耗与花费；
  - 卡片内一键直达终端/IDE 窗口或触发终止逃生舱。

---

## [1.6.0] - 2026-09-07

### 🔕 核心新特性：免打扰与通知分级 (Focus Mode & Notification Filtering)
- **三大通知策略模式（NotificationPolicy）**:
  - **专注免打扰（Focus Mode，推荐并默认）**：普通任务执行完毕静默更新（不弹窗微窥、不响提示音），仅在微细条或手动展开卡片内呼吸展示，彻底解决频繁编码被打扰的痛点；当发生**成本暴涨突增、死循环熔断告警（costSpike）**等紧急危险事件时，依然立即滑出 6 秒微弹窗并播放告警音。
  - **标准模式（Standard Mode）**：所有任务完成、等待确认、异常告警均滑出 3.5s 微弹窗并播放轻脆提示音（适合挂机等待智能体交付）。
  - **完全静默（Silent Mode）**：绝不滑出任何微弹窗，绝不播放任何提示音，纯后台静默记录与展示。
- **彻底根治无谓弹窗遮挡**:
  - 移除此前智能体一启动工作（`working`）就触发弹窗的过度打扰行为，还用户沉浸式专注编码体验。
- **全入口一键切换**:
  - 顶部菜单栏 Popover 底部操作栏新增通知模式切换快捷菜单（带对号与动态高亮）；
  - 灵动岛右键上下文菜单提供「通知模式」子菜单；
  - 设置面板「通用与外观」新增精致「通知与免打扰模式」卡片，并配有直观规则说明。

---

## [1.5.1] - 2026-09-07

### ✨ 核心新特性 (Appearance & Native Integrations)
- **深浅外观模式与跟随系统 (Appearance Modes)**:
  - 全局支持「跟随系统」、「浅色模式」、「深色模式」三态实时无缝热切换；
  - 灵动岛展开态顶栏右侧新增快捷主题切换图标按钮；
  - 菜单栏状态项 Popover、分栏设置面板、灵动岛右键上下文菜单均支持一键切换；
  - 玻璃拟态与单向圆角阴影（深色浓郁高斯阴影 vs 浅色晶莹投影）随主题自适应。
- **原生支持 Google Antigravity 智能体**:
  - 原生识别 Antigravity 进程与 CLI 会话目录；
  - 实时解析智能体执行轨迹（trajectory logs）与 tool_use 动作（如 `正在修改: ...`、`正在运行命令: ...`）；
  - 任务完成主动提醒并支持终端/IDE 窗口深度直达。
- **原生支持 ZCode 智能体**:
  - 会话日志感知、实时动作透视与一键直达。

### 🐞 关键修复与交互打磨 (Stability & Interaction Fixes)
- **灵动岛折叠与展开防抖重构 (Collapse & Gesture Refactor)**:
  - 修复边缘微细条几何判定误判（彻底移除卡片滑动过渡态对光标的误判），光标移出卡片后 0.5 秒平滑稳定收回贴边，绝不回弹；
  - 鼠标移入卡片立即解除手动展开保护期，无需等待固定延迟；
  - **点击外部区域自动收起（Click-outside to dismiss）**：在展开卡片外部任意桌面或窗口点击即刻平滑折叠；
  - **顶栏新增一键显式收起按钮**（右侧边栏为 `chevron.right`，顶部灵动岛为 `chevron.up`），右键上下文菜单同步提供「收起灵动岛」；
  - 重构顶栏拖拽把手为背景层，彻底解决拖拽手势遮挡外观切换菜单与收起按钮的问题。
- **死循环告警判定与横幅排版调优**:
  - 重构高负载检测算法，引入基准采样比对，避免短时编译与常规编码误报；
  - 告警横幅重构为双行自适应卡片，告警描述完整展示支持 Tooltip，操作按钮独立成行。
- **进程路径误报防御**:
  - 强化 `pathContains` 约束，避免同名 Electron 进程导致未安装智能体误报。

### ⚡️ 质量与用例
- 测试套件扩充至 56 个全自动化单元测试用例，全绿通过（`56 通过, 0 失败`）。

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
