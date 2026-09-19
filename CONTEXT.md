# AgentIsland

macOS 灵动岛应用：监控本机 AI 编码智能体的运行状态与 token 消耗，在屏幕顶部以悬浮岛呈现，无需用户聚焦即可感知 Agent 是否在工作。

## Language

### 智能体与档案

**Agent（智能体）**:
被监控的 AI 编码助手（DimAgent、Claude、Codex、Cursor 等）。
_Avoid_: 助手、机器人、目标进程

**Agent Profile（档案）**:
一个 Agent 的静态识别定义：bundle id、进程名前缀、可执行路径特征、会话目录，以及展示符号、会话方言与只读会话库位置。档案是这些事实的**唯一声明处**，解析器不再按 id 反推。
_Avoid_: 配置、agent 定义

**会话方言（session dialect）**:
一个 Agent 的会话记录存放格式：`genericTail`（通用 JSONL 尾读）、`antigravityBrain`、`dshProjection`、`clineTasks`。解析器按**格式**分派而非按 agent id 分派——格式数量远少于 Agent 数量（Cline 与 Roo Code 同源），新增复用既有格式的 Agent 只改注册表。

**只读会话库（session database）**:
会话只写进 SQLite 的 Agent，其库文件位置与查询 schema（`dimTasks` / `statusIndex` / `openCode`）随档案声明。FileMonitor 对这类目录只能定位到二进制库本身，强语义须由末条索引查询给出。

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
`offline`（进程不在）/ `idle`（进程在但无活动信号）/ `completed`（本轮有明确完成标记）/ `working`（有活动信号）/ `attention`（存在未处理的用户确认或授权请求）。

**会话强语义（session signal）**:
从本轮文件扫描已定位的最新会话文件中，有界尾读结构化事件字段，提取 `attention` / `completed`。强语义优先于双信号；无命中或未知格式时降级到双信号。确认请求解除后不补完成事件，完成标记过期后自然回到 idle。

**通知路由（notification route）**:
系统通知携带 Agent id；点击后按快照 PID / 档案 bundle id 激活对应 GUI 或 CLI 宿主终端，无法激活时回退到 AgentIsland 对应 Agent 详情。

**双信号**:
没有会话强语义时，working 的两个判定依据：工作窗口内有会话文件写入、或进程 CPU 超过阈值；满足其一即 working。

**滞回（hysteresis）**:
working 信号消失后保持 working 的最短时长，防止临界抖动导致面板高频弹跳。

**活跃会话数（activeSessions）**:
会话目录下、判定窗口内有文件写入的顶层子目录数。

**会话定位缓存（session locator cache）**:
专有解析器「选哪个会话文件」的结果缓存（TTL 3s，并以根目录 mtime 作失效令牌）。**只缓存定位**——尾读与解析每拍照常进行，故新会话与状态转移无延迟。存在的前提是专有解析器要跨多层目录挑最新会话，实测单趟遍历达 500~800 次 stat，不能每拍在采样主线程上重走。
_Avoid_: 会话结果缓存（并不缓存信号本身）

**尾读合并（tail read memo）**:
同一文件在 mtime 与长度都未变时复用上一次读出的尾部行——「会话强语义」与「当前动作文案」两条链路每拍各读一次同一份 transcript，而尾读比解析更贵。有效期短于采样周期；新鲜度取 `stat()` 而非 `URL.resourceValues`（后者有毫秒级缓存窗口，恰好会在这两条链路的微秒间隔内误命中）。

**可见口径（visibleSnapshots）**:
仅进程仍在的 Agent 可见；待机、工作中、待确认、已完成均显示，离线一律隐藏。卡片列表、菜单摘要、高度计算都消费这一统一口径；历史活动与 token 不得让离线 Agent 形成幽灵条目。

### Token 用量

**Token 用量（TokenUsage）**:
24h 与累计两组 token 数与成本；总体跨 DimAgent、OpenCode、Codex、Claude、WorkBuddy 与 WorkBuddy AI 合并。口径为净消耗（prompt+completion / input+output），不含 cache.read，避免多轮会话重复计费虚高。

**Token 数据覆盖（Token source availability）**:
工具本地明细源是否被发现，与“当前范围用量为 0”是两个不同状态。分析页必须逐工具表达该差异，不得把未接入或缺失数据伪装成零用量。

**Token 用量下钻（Token source detail navigation）**:
本地明细可读且工具具备详情能力时，分析页允许进入用量详情；这条导航不依赖实时快照。实时 Agent 列表仍遵守宿主/内嵌组件去重，避免 ChatGPT 与内嵌 Codex 重复显示。

**呈现活跃（presentation active）**:
面板处于需要展示 token 数据的状态（展开态）。token 轮询仅在呈现活跃时运行；收起即暂停。
_Avoid_: 前台、可见（屏幕层面概念）

### 面板与菜单栏

**docked / expanded**:
收起态（在贴靠边缘保留 6pt 晶莹微细条）/ 展开态（完整卡片）。

**DockEdge（停靠边缘）**:
`top`（顶部灵动岛）、`right`（右侧边栏）、`bottom`（底部停靠条）与 `left`（左侧边栏）。自由拖拽释放时按距可用屏幕四边的最近距离吸附，水平边保存 X 锚点、垂直边保存 Y 锚点；设置面板也可直接指定。

**微细条（sliver）**:
收起态下在屏幕边缘留存的 6pt 极细晶莹胶囊，带工作呼吸绿灯，光标触碰或悬停即可自动弹出展开为卡片。

**自由拖拽与智能吸附（drag & snap）**:
长按展开卡片顶栏可自由移动，松手时根据与屏幕上、右、下、左四边的距离自动吸附到最近边缘并持久化轴向锚点坐标。

**peek**:
Agent 转为 working 时面板自动短暂展开示警，随后收回；有冷却间隔。

**菜单栏 Popover（Compact Island Popover）**:
MenuBarExtra 的 .window 浮窗微卡片，呈现工作状态呼吸灯、活跃 Agent 概览与快捷操作栏。

**安装缓存**:
已安装 CLI 与 GUI bundle 的扫描结果缓存，后台低频刷新，用于标记档案「已安装」。
