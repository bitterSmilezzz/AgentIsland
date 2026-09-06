# AgentIsland

macOS 灵动岛应用：监控本机 AI 编码智能体的运行状态与 token 消耗，在屏幕顶部以悬浮岛呈现，无需用户聚焦即可感知 Agent 是否在工作。

## Language

### 智能体与档案

**Agent（智能体）**:
被监控的 AI 编码助手（DimAgent、Claude、Codex、Cursor 等）。
_Avoid_: 助手、机器人、目标进程

**Agent Profile（档案）**:
一个 Agent 的静态识别定义：bundle id、进程名前缀、可执行路径特征、会话目录。
_Avoid_: 配置、agent 定义

**自定义 Agent**:
用户在设置界面手动新增的档案，id 以 `custom-` 为前缀。

**自动发现**:
扫描 PATH 与 /Applications 找到、但不在内置集里的 CLI 档案；默认关闭。

### 活动判定

**采样（sampling）**:
引擎周期性探测全部启用 Agent 并装配快照的动作；有 working 时快、全闲置时降频。
_Avoid_: 轮询（专指 token 数据的定时查询）

**快照（snapshot）**:
某时刻某 Agent 的状态装配：活动等级、进程在否、CPU、是否已安装、活跃会话数、最近活动时间、token 用量。

**活动等级（ActivityLevel）**:
`offline`（进程不在）/ `idle`（进程在但无活动信号）/ `working`（有活动信号）。

**双信号**:
working 的两个判定依据：工作窗口内有会话文件写入、或进程 CPU 超过阈值；满足其一即 working。

**滞回（hysteresis）**:
working 信号消失后保持 working 的最短时长，防止临界抖动导致面板高频弹跳。

**活跃会话数（activeSessions）**:
会话目录下、判定窗口内有文件写入的顶层子目录数。

**可见口径（visibleSnapshots）**:
「在线、或 24h 内有活动」才算可见的统一口径；卡片列表、菜单摘要、高度计算都消费它。

### Token 用量

**Token 用量（TokenUsage）**:
24h 与累计两组 token 数与成本；口径为净消耗（prompt+completion / input+output+reasoning），不含 cache.read，避免多轮会话重复计费虚高。

**呈现活跃（presentation active）**:
面板处于需要展示 token 数据的状态（展开态）。token 轮询仅在呈现活跃时运行；收起即暂停。
_Avoid_: 前台、可见（屏幕层面概念）

### 面板与菜单栏

**docked / expanded**:
收起态（在贴靠边缘保留 6pt 晶莹微细条）/ 展开态（完整卡片）。

**DockEdge（停靠边缘）**:
`top`（顶部灵动岛）与 `right`（右侧边栏），由自由拖拽释放时智能吸附或设置面板指定。

**微细条（sliver）**:
收起态下在屏幕边缘留存的 6pt 极细晶莹胶囊，带工作呼吸绿灯，光标触碰或悬停即可自动弹出展开为卡片。

**自由拖拽与智能吸附（drag & snap）**:
长按展开卡片顶栏可自由移动，松手时根据与屏幕顶缘/右缘的距离自动吸附到对应边缘并持久化锚点坐标。

**peek**:
Agent 转为 working 时面板自动短暂展开示警，随后收回；有冷却间隔。

**菜单栏 Popover（Compact Island Popover）**:
MenuBarExtra 的 .window 浮窗微卡片，呈现工作状态呼吸灯、活跃 Agent 概览与快捷操作栏。

**安装缓存**:
已安装 CLI 与 GUI bundle 的扫描结果缓存，后台低频刷新，用于标记档案「已安装」。
