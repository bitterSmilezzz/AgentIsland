# 更新日志 (CHANGELOG)

所有关于 AgentIsland 的重要版本演进与功能更新均记录在此。

---

## [1.7.10] - 2026-09-09

### 🐛 修复状态跟踪误报、Token 口径虚高、底部汇总栏裁切与贴边阴影

- **进程关闭不再误报「任务已完成」**：
  - 此前 `offline` 分支复用完成事件路径，把「进程被用户关闭」等同于「任务执行完毕」，手动退出 ChatGPT 也会弹出完成横幅
  - 现在进程消失只静默转 `offline`；完成事件仅由「进程仍在但工作信号消失」产生
- **Token 净消耗口径统一（dim 源）**：
  - `usage_ledger.usage.promptTokens` 含缓存命中部分，此前直接与 completion 相加导致缓存重复计入，本机实测累计虚高 32 倍（39.6 亿 vs 净 1.22 亿）
  - 汇总、模型拆分、会话列表三处统一改为 `(prompt − cacheRead) + completion` 并逐行钳制非负，与 opencode 侧口径对齐
- **激增告警改为速率制 + 连续确认**：
  - 此前「300 秒内增量 ≥ 阈值」把长任务结束时的一次性账本落盘（单条 1700 万 token）当成瞬时激增
  - 现在按每分钟净消耗速率判定，且需连续 3 个周期超阈值才告警；正常绘画/长任务不再误报
- **修复 peek 微弹窗导致侧边条错位**：
  - 此前 peek 只拉伸窗口 frame、`displayState` 仍为 `docked`，SwiftUI 只渲染 6pt 细条，于是细条被拉到展开位置且卡片内容缺失
  - 现在 peek 走真实 `displayState` 切换，与窗口尺寸同源，展开内容正常呈现
- **修复展开卡底部 Token 汇总栏被裁切**：
  - `IslandMetrics.expandedHeight` 漏算顶部「实时活动环微看板」（40pt 内容 + 1pt 分割线），渲染内容比窗口高出约 56pt，超出 460pt 上限的部分从底部裁掉，汇总栏只露出半行
  - 现在展开高度计入看板（新增 `chromeHeight`/`listHeight` 纯函数统一口径），触顶时压缩可滚动的 Agent 列表（保底一行）而非裁切汇总栏
  - 二次修复（首次展开正常、挪动后又被裁）：实测 `NSHostingView.fittingSize` 比常量推导值高 5.5pt——顶栏内容行高按 17pt 估算偏小（圆形图标按钮实际约 23pt），且汇总栏自身那 1pt 分割线未计入 `chromeHeight`。现按实测校准常量，并让窗口高度取「常量推导」与「内容理想高度」的较大值（`resolvedExpandedHeight`），`placeWindow`/`snapToDockEdge`/`syncExpandedHeight` 三处同源，常量再漏算也不会把汇总栏挤出窗口
- **移除面板阴影内渗，消除贴边侧上下暗带**：
  - 面板窗口与玻璃卡尺寸完全相同（330×447），AppKit 阴影没有卡片之外的落地空间，只会沿轮廓边缘向卡内渗入约一个模糊半径，在贴屏幕一侧的上下直角区域形成暗块（用户反馈的「两侧直角矩形的上下阴影」）
  - 现已停用面板阴影；一体化贴边的观感由玻璃卡自身的 1px 高光边缘与反向倒角承担
- **修复 ChatGPT 与 Codex 重复显示为两个 Agent**：
  - ChatGPT 桌面版把 Codex 打包进 `/Applications/ChatGPT.app/Contents/Resources/codex`，其 basename 与独立 `codex` CLI 相同且共用 `~/.codex` 会话目录，此前被数成两个 Agent
  - `AgentProfile` 新增 `pathExcludes`（路径排除）与 `hostBundleIDs`（宿主识别）：宿主已安装且该组件无独立安装时不再单独成条目；独立安装 codex CLI 的用户仍照常监控
  - 手写 `Codable` 解码（`decodeIfPresent` + 默认值），保证升级前保存的自定义 Agent 不因新增字段而整条失效
- **修复展开卡内容超高时底部汇总栏被压掉**：
  - 事件提醒栏出现后内容超过 460pt 上限，此前由 `VStack` 自行分配压缩，末尾的 Token 汇总栏成了牺牲品
  - 现给列表 `layoutPriority(-1)`、汇总栏 `layoutPriority(1)`：空间不足时只压可滚动的列表，汇总栏保持完整
- **性能：消除每 2 秒的主线程阻塞与 CPU 尖峰**（多 Agent 审查实测发现）：
  - `AgentActionInspector.activeChildCommand` 原先 fork `/usr/bin/pgrep` + `/bin/ps` 并 `waitUntilExit()`，单次 67ms，17 个 Agent 一轮 762ms 全部落在主线程；改为 sysctl `KERN_PROCARGS2` 直读命令行 + 复用采样快照做内存 BFS，`inspectDimAction` 单次由 170–220ms 降至 **0.9ms**
  - `inspectDimAction` 的 `ORDER BY createdAt DESC LIMIT 1` 在 5.4 万行 / 254MB 的 `messages` 表上退化为全表扫描 + 临时 B 树排序（220ms/次）；改用 `rowid = (SELECT max(rowid) …)` 走主键查找
  - `inspectOpenCodeAction` 的 `session LEFT JOIN part` 全表排序实测 330–964ms；改为「先取最新会话，再取该会话最新 part」，降至 8ms
  - `ProcessTerminator.getProcessTree` 与 `AppActivator` 的父进程追溯同样去掉逐节点 fork，改用一次快照内存遍历
  - 实测收起态 CPU 由均值 8.0% / 峰值 39.2% 降至 **1.2% / 2.8%**
- **修复状态误判：Agent 恒显「工作中」**：
  - `inspectAction` 返回的「最近动作」被当作核心工作信号，而各探测源在 Agent 空闲挂起时仍可能命中旧记录（dim 分支甚至无视注入的 fake 直读真实 SQLite），导致 `working` 永不消退、完成事件永不产生。现在工作状态只由「文件写入 + CPU」决定，动作仅作展示字段
  - 长驻子进程（MCP server、language server、`server.js`、`--liftoff-only` 索引进程）不再被判为「正在执行的任务」
  - `KERN_PROCARGS2` 解析按 `argc` 截断，避免把 `PATH=…` 等环境变量当成用户命令
  - 修复滞回锚点：原用「首次进入 working 的时刻」判断，任何超过 `minWorkingHold` 的任务滞回完全失效；改用每拍刷新的 `lastSignalAt`
  - 修复后 3 个长期失败的环境依赖用例全部转绿，测试 **84 通过 / 0 失败**
- **修复危险操作的可信度**：
  - 终止按钮在 `pid == nil`（GUI bundle 命中但进程名未匹配）时此前不发信号却宣告「进程已终止」——假成功；现在如实提示「无法终止：未定位到进程」并返回 `false`
  - 终止成功由 `attention` 改为 `completed`，收起态细条不再误报红色告警
  - 工具箱单条清理补二次确认（杀的是整棵进程树）；`overweight`/`hung` 不再把 `/Applications/*.app/Contents/MacOS` 主进程列为可清理项（开着大项目的 Electron IDE 占 2.5GB 属正常）
  - `ProcessTerminator.terminate` 返回真实信号发送结果，清理横幅不再谎报「已释放 N 个进程」
  - 工具箱扫描的 `NSWorkspace` 调用移回主线程（`ProcessProviding` 线程契约）
- **修复交互与显示缺陷**：
  - hover tooltip 内的按钮永远点不到：popover 由 26×26 环的 `onHover` 驱动，鼠标移向 popover 时立即触发关闭；改为 400ms 延迟关闭 + popover 内 hover 取消
  - 终止确认态不再于 Agent 转 idle 后残留（避免误杀已空闲进程），并用可取消 `Task` 替代 `DispatchQueue` 定时器
  - 顶栏优先展示「带动作」的 working Agent 并显示 `+N` 并行数；可见计数降为可压缩，长名称/动作不再被挤断
  - 事件横幅的按钮/背景改用动态色，浅色主题下不再白底白字不可见；关闭按钮热区 15→21pt
  - 「直达」在无对应 Agent 时置灰并说明，不再点击无反馈；熔断按钮在无 PID 时给出去向指引
  - 工作态 Agent 也显示 token 徽标（正在消耗的最需要关注）；活动环副标题与行内口径统一
  - 实时流水页返回按钮回到进入前的层级；事件 id 改为确定性生成，展开的详情不再每 2 秒被强制折叠
  - 脉冲与呼吸动画尊重系统「减弱动态效果」；版本号从 bundle 读取而非硬编码

---

## [1.7.9] - 2026-09-09

### 🩺 修复 Antigravity 监控识别与偏好设置自动自愈迁移

- **启用集自愈与向前兼容算法 (`EnabledAgentStore.resolvedEnabled`)**：
  - 彻底解决旧版本持久化 `enabledAgents` 导致新增内置智能体（Antigravity、ZCode、DSH、ChatGPT 等）被静默过滤的问题
  - 引入 `knownAgents` 持久化机制与 `legacyKnownAgentIDs` 基线迁移，存量用户升级时自动将新加入且默认启用的内置智能体合入启用集
  - 严格保持用户主动全关（`[]`）与单项显式关闭偏好，不发生意外覆写
- **用户环境自动修复与直达**：
  - 启动阶段与偏好设置面板同步自动自愈，Antigravity 无需用户手动翻找开关即可立即呈现在灵动岛监控与菜单栏中
  - 新增 4 组单元测试（首次安装/主动全关/历史存量迁移/显式关闭记忆），全量测试 64/64 保持全绿

---

## [1.7.8] - 2026-09-08

### ⚡ 智能体实时事件流与日志流水抽屉 (Live Log Stream)

- **智能体事件流与日志流水提取引擎 (`AgentLogStreamer`)**：
  - 支持多源并发智能日志采集：零侵入解析 Antigravity (`transcript.jsonl`)、Codex (`rollout-*.jsonl`)、DimAgent (`dimcode.sqlite`)、Claude Code、OpenCode、ZCode、WorkBuddy、Hermes 等
  - 提取高价值时序结构化数据：精准识别终端命令执行 (`EXEC`)、工具/MCP调用 (`TOOL`)、代码文件编辑 (`EDIT`)、深度推理思考 (`THINK`) 与模型对话流 (`MSG`)
- **原生极客暗黑风实时流水视图 (`LiveLogStreamView`)**：
  - 灵动岛主卡行与二级详情页增设终端图标（`terminal`）直达流水抽屉
  - 极客暗黑终端配色、彩色事件类型徽标、时间戳微调与参数展开查看
  - 支持实时静默自动跟随刷新（每 2 秒）与随时暂停切换
  - 提供一键复制全部诊断日志流水至剪贴板功能，便于排查与分析
- **窗口几何与导航联动**：
  - 深度集成 `IslandMetrics` 与 `IslandPanel` 自适应高度体系，保障展开卡片内无缝平滑滚动

---

## [1.7.7] - 2026-09-08

### 🛠️ 智能体维护工作台与进程清理系统 (Agent Workbench Cleaner)

- **原生工作台维护视图 (`ToolboxView`)**：
  - 展开卡顶栏增设快捷工具箱按钮（`wrench.and.screwdriver`），一键进入系统级 Agent 维护工作台
  - 展示待维护异常项、预估可回收内存与智能体整体健康度评分
- **三维智能体异常诊断 (Anomalies Detection)**：
  - **孤儿进程检测 (Orphaned Processes)**：自动捕捉主控终端被关闭后、父进程转为 `launchd` (PPID=1) 且脱离控制台的遗留 Agent 进程
  - **疑似死锁/假死检测 (Deadlocked / Hung)**：检测持续异常高载且失去会话响应的卡死任务
  - **内存超限检测 (Overweight Leaks)**：标识单进程驻留集物理内存超过 2.0GB 的潜在堆内存泄漏
- **一键安全清理与资源回收**：
  - 支持多选单项清理与一键安全全量清理，先尝试优雅信号通知保存状态，超时未退出则强制杀死并级联释放子进程树
  - 清理完毕后动态展示释放进程数与物理内存回收横幅，并触发自动刷新与状态自愈

---

## [1.7.6] - 2026-09-08

### 🩺 Agent 性能健康仪表盘与线程死锁检测

- **物理内存微秒级精准读取 (RSS Footprint)**：
  - 基于 Darwin 原生 `proc_pid_rusage` 读取物理驻留集 `ri_resident_size`，零外部命令开销，毫秒级反映各 Agent 真实物理内存占用
  - 在 `AgentRowView` 列表中增设内存紧凑徽标（如 `280M`、`1.2G`），卡片悬停 Tooltip 及详情页均可实时透视各 Agent 内存与 PID
- **死循环/高负载死锁异常检测 (Deadlock / Hung Detection)**：
  - 关联采样时间序列，当智能体发生非预期的高 CPU 持续占用（超出熔断阈值）或无会话响应时，自动在列表与详情页中标记「疑似卡死」健康警示
  - 支持一键安全终止与逃生舱清理，保障系统资源与开发机温度
- **二级详情页性能与健康矩阵**：
  - `AgentDetailView` 嵌入 CPU、物理内存与 PID 概览小方块，与 Token 用量协同构成完整的 Agent 资源透视图

---

## [1.7.5] - 2026-09-08

### 💎 灵动岛收起态边缘微胶囊视觉动效强化 (DockedSliverCapsule)

- **多状态一眼感知**：
  - **工作中 (Working)**：翡翠绿微光呼吸光晕与中心状态点呼吸流动，多 Agent 并行或后台生成时屏幕边缘清晰可感
  - **严重告警/待关注 (Alert)**：微红/琥珀金微光呼吸警示（针对熔断、死循环高负荷及进程异常），无需展开面板即可在屏幕边缘获知关键事件
  - **空闲待机 (Idle)**：晶莹半透明微晶胶囊，极致低调不扰工作
- **零额外能耗保证**：
  - 采用平滑缓和的 2.0s/1.2s 周期呼吸动画，仅在有明确状态（工作或告警）时开启；空闲待机时完全复位休眠
  - 完善 `IslandPanel` 对事件变更的观察管道，收起态状态无缝同步
- **边缘贴合自适应**：
  - 完美适配顶部吸附（横向微胶囊 140x6pt）与右侧吸附（纵向微胶囊 6x120pt）两种形态

---

## [1.7.4] - 2026-09-08

### 🎯 状态判断精准化与 Agent 工具生态全覆盖

- **消除 Electron 后台待机误报 WORKING**：
  - 将 `cpuThreshold` 默认阈值微调为 `6.0%`，彻底避开 Electron/Chromium UI 渲染器固有空闲抖动（1%~5%），真正在进行代码编译、大文件检索或模型计算时才触发 CPU 状态跃迁
  - 修正 WorkBuddy `sessionDirs` 为实际数据目录（`~/.workbuddy/sessions`, `~/.workbuddy/tasks`, `~/.workbuddy/memory`）
- **WorkBuddy 动作与状态时效性强校验**：
  - 加入 5 分钟更新新鲜度校验，超过 5 分钟未更新的会话标记为「待机」，绝不因陈旧未归档记录误报「正在处理」
- **补全本机 AI Agent 工具全生态覆盖**：
  - **Antigravity Studio**：支持 `com.yuzhiqiang.antigravity.studio` 独立识别并归并至 Antigravity 生态
  - **Ego Browser (Ego Lite)**：支持 Agent 专用隔离浏览器（`com.citrolabs.ego.lite` / `ego-browser`）
  - **Vibe Usage**：支持智能体 Token 用量聚合看板（`ai.vibecafe.vibe-usage` / `vibe-usage`）
  - **OpenViking**：支持本地 AI 知识库与智能体执行器（`openviking`, `openviking-server`, `ov`, `vikingbot`）
  - **扩充 CLI 检测池**：覆盖 `bsk` (Browser Skill)、`cua-driver` (Computer Use Driver) 等

---

## [1.7.3] - 2026-09-08

### ⚡️ 扩展主流 Agent 深度动作透传与会话解析

- **WorkBuddy 实时任务解析**：从 `workbuddy.db` 深度解析当前活跃会话标题、状态（`正在: ...` 或 `任务: ...`），精准透传工作上下文
- **OpenCode 深度解析**：支持从 `opencode.db` 的 `session` 与 `part` 提取实时思考规划（`思考规划中`）、工具调用（`正在调用: ...`）或当前会话主题
- **DSH (DeepSeek Harness) 模式感知**：结合进程命令行参数与会话状态，透传 Web 协作模式（`Web 协作服务运行中`）或任务执行详情
- **Hermes 会话与动作透传**：从 `state.db` 提取最近会话与活动描述
- **ZCode 任务检查器增强**：加入 `deleted = 0` 软删除过滤并放宽时间窗口，使进行中的任务持久准确透传

---

## [1.7.2] - 2026-09-08

### 🔔 告警与通知卡片交互升级与全量排查信息展示

- **事件卡片富文本展开（EventBannerView）**：告警卡片支持「精简两行」与「完整展开」双态切换，告警发生原因、阈值说明与排查建议一览无余，彻底杜绝文本截断 `...`
- **可解释性告警详细排查说明**：
  - **Token 激增告警**：显示具体监控时间跨度、增量绝对值、报警阈值与多 Agent 并发/Prompt 死循环排查指引
  - **死循环与高 CPU 告警**：展示持续分钟数、当前 CPU 百分比、进程 PID 以及熔断逃生舱操作建议
  - **任务完成通知**：展示实际耗时与状态转空闲说明
  - **终止逃生舱通知**：记录释放的目标 PID 与信号处理结果
- **一键复制诊断信息**：展开态下提供「复制诊断」按钮，可一键将 Agent ID、PID、发生时间、摘要与完整排查建议复制至剪贴板
- **窗口高度动态弹性自适应**：展开/折叠事件卡片时，面板高度自动平滑扩展（从 66pt 增至 142pt），保证下方的 Agent 列表不被遮挡或挤出窗口
- **严重告警自动展开**：对 Token 激增与死循环等熔断级警告，提醒时默认展开排查建议，协助用户迅速决策

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
