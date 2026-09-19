# AgentIsland — Agent 会话灵动岛监控器

监控本机所有 Agent 软件（DimAgent / Claude / Codex / Cursor / Trae / Google Antigravity / ZCode / WorkBuddy / Copilot / OpenCode 等）的会话状态；以 macOS 灵动岛风格呈现，支持**自由拖拽智能贴边（上、右、下、左四边）**、**6pt 晶莹微细条常驻感知**、**深浅外观切换**与**光标触碰自动弹性弹出**。

## 功能（v0.0.77）

### 多屏热插拔、本地 Webhook、Token 预算与全键盘流 (v0.0.77)
- **多显示器协同与断连自愈（Multi-display Resilience）**：外接屏拔出时自动平滑重定位，杜绝窗口悬空或坐标越界；
- **本地零依赖 Webhook 接收器（LocalEventServer）**：基于 `NWListener` 监听 `127.0.0.1:41999`，支持 `POST /notify` 毫秒级直推智能体完成与告警事件；
- **终端预算进度可视化（`agentisland tokens --budget`）**：终端字符进度条自适应渲染，超额/预警分级高亮显示；
- **展开态全键盘流操作**：`1~3` 数字键秒切视图、`j/k` Vim 风格智能体焦点移动、`Enter` 下钻详情、`Esc` 逐级返回。

### 原生终端命令行工具（agentisland-cli · v0.0.76）
- **终端全景快照（`agentisland status` / `agentisland`）**：快速输出所有已监控智能体状态、PID、CPU%、内存、会话数与用量表格，支持 `--json` 供 Raycast/脚本联动。
- **Token 趋势与成本看板（`agentisland tokens`）**：终端汇总最近 24h/累计消耗，并自动基于 `TokenForecastEvaluator` 推算当月末总消耗与预算枯竭预警。
- **死锁排查与一键清理（`agentisland check` / `agentisland clean`）**：排查挂起死锁或内存泄漏进程，支持 `--dry-run` 预览与安全批量一键终止释放。
- **深度链接呼出（`agentisland open`）**：终端直接控制桌面灵动岛展开、折叠或直达指定 Agent 与分析看板。
- **审计报告导出（`agentisland report`）**：一键生成 Markdown 运维审计报告，支持 `--copy` 复制到剪贴板或 `--output` 存盘。

### 深度集成与自愈增强 (v0.0.75)
- **URL Scheme 深度链接协议（`agentisland://`）**：支持外部应用通过 `open agentisland://toggle`、`expand`、`collapse`、`agent?id=<id>`、`analytics`、`toolbox`、`clean`、`export` 实现无缝操作联动。
- **硬件电源与电池节能自适应（PowerSourceMonitor）**：MacBook 电池供电时自动平滑降频后台采样，插电即刻恢复满血探测，在续航与灵敏之间智能平衡。
- **实时流水错误特征分析（LogPatternAnalyzer）**：自动提取 429 限流、编译报错、Git 冲突与鉴权失败，提供警示徽标与一键复制核心摘要。
- **月末 Token 用量与成本预测（TokenForecastEvaluator）**：基于近 24h 消耗速率推算当月末消耗与费用，实时预测预算枯竭天数。
- **智能体死锁与异常驻留自愈守护（AgentResilienceGuard）**：持续监视长期死锁与内存激增智能体，主动提供自愈警告与逃生终止通道。

### 设计美学与 CodeNotch 灵动交互
- **贴边收起常态 0.0% CPU 极致省电（CoreAnimation 硬件加速）**：
  - 收起态呼吸光晕由 CoreAnimation 独立硬件图层驱动，彻底消除高刷屏递归重排，Docked 态 CPU 稳定在 **0.0% ~ 0.1%**；
  - 全局鼠标移动近邻预过滤，屏幕中央滑动零 Task 派发；离线 Agent 免深搜，极大降低系统资源消耗。
- **反向倒角一体化贴边（Inverse Rounded Corner / Bezel Flares）**：
  - 基于数学级三次 Bézier 曲线实现 `SideNotchShape`，支持上、右、下、左四边贴边；
  - 边缘向屏幕物理边框平滑过渡，如同从屏幕外壳硬件级一体化生长出来，彻底告别悬空矩形与割裂感；
  - 玻璃拟态卡片背景与反向倒角边缘共用精准几何路径（AppKit 投射阴影已停用）。
- **环形微仪表盘与双层动态活动弧（Micro-Dashboards & Activity Arcs）**：
  - **5 态状态水位环**：工作、待确认、已完成、待机、离线使用独立语义与颜色；异常高负载继续以橙/红告警层叠呈现；
  - **内圈高精度动态弧**：工作状态下呈现 `0.25` 弧长、1.2s 无级平滑旋转的渐变微弧（`SpinningActivityArc`）；
  - **呼吸警戒环（PulsingAttentionArc）**：告警/等待状态下呈现平滑呼吸缩放光环，状态一目了然；
  - **顶部活跃微看板（Quick Rings Shelf）**：展开卡片顶部直观罗列当前活跃的所有 Agent 微仪表盘，点击秒切。
- **精准悬停透视卡片与指向小尾巴（Hover Tooltip Card & Tail）**：
  - 光标悬停在任意 Agent 环或列表项时，秒出半透明指向气泡浮层（带几何小尖角 `TooltipTail`）；
  - **零点击透视**：直达实时执行命令、正在修改的代码文件、系统进程 PID、24h / 累计 Token 消耗与计费；
  - **内嵌快捷动作**：悬停卡片直接集成「直达窗口」与一键防误触「终止进程逃生舱」。

### 实时伴侣与指令中心
- **免打扰与通知分级（Focus Mode）**：
  - **专注免打扰（默认推荐）**：普通任务完成静默更新；**需要用户确认/授权**与成本激增、死循环熔断等必须介入的事件仍即时通知并播放提示音；
  - **标准模式 / 完全静默**：挂机等待交付可一键切换为标准模式；需要绝对清净可随时一键切换为完全静默；
  - **移除冗余弹窗**：彻底移除 Agent 一启动工作就弹窗微窥的打扰行为。
- **原生支持 Google Antigravity & ZCode**：支持 Electron 与 CLI 进程扫描、实时轨迹日志解析（tool_use 与动作上下文）、会话监控与窗口直达
- **任务完成主动提醒与 Peek 微窥**：标准模式下 Agent 结束**有写入证据**的持续工作（≥3.5s）转为空闲时，自动播放轻脆系统提示音（Glass），并在收起态下自动滑出 3.5 秒 Peek 微弹窗；若光标移入则自动转换为常驻展开，无需手动翻找进度。纯 CPU 高负载（桌面应用空闲抖动、启动尖峰）只更新状态，不会误报完成
- **待确认主动提醒与点击直达**：识别 Agent 的结构化确认、权限申请与用户输入请求；每个请求只通知一次，多个 Agent 同时等待也不会漏报。点击 macOS 通知后直接激活对应 Agent GUI 或 CLI 所在终端，目标已退出时回退到岛内 Agent 详情
- **终端与 IDE 窗口一键直达**：
  - **GUI 智能体（Cursor、Trae、Antigravity 等）**：通过 BundleID / PID 一键拉至最前并聚焦；
  - **CLI 智能体（Claude Code、Codex、Dim 等）**：毫秒级递归追溯进程树父节点，精准定位 Terminal、iTerm2、VS Code、Ghostty、Warp 等终端宿主窗口，一键将黑底终端置顶呼出
- **实时操作透视**：
  - 子进程命令实时提取（`git diff`、`swift test`、`npm run build` 等）并智能清洗包裹层；
  - DimAgent / Claude / Codex / Antigravity 会话日志解析，直观呈现 `正在修改: IslandView.swift`、`正在执行: pytest` 等动态徽标；
- **成本与异常熔断保护（逃生舱）**：
  - **Token 暴涨检测**：滑动差分监测单分钟 Token 增量，超阈值时弹出双行自适应告警卡片并播放警示音；
  - **异常长耗时死循环告警**：基于基准采样的防误报算法，持续异常高负荷工作未释放时自动预警；
  - **一键 Kill 逃生舱**：红色熔断横幅直达终止，带两段式确认（首击进入确认态、3 秒自动复位）避免误触杀错进程树；列表行悬浮红色「终止」按钮同样带确认态；终止前强制**身份复核**——PID 已被系统回收复用给其他程序时拒绝执行，绝不误杀无关进程；
  - **告警保护**：严重告警在 30 秒内不会被其他 Agent 的完成事件顶掉，留出处置时间；

### 灵动岛交互与外观
- **浅色 / 深色 / 跟随系统模式**：灵动岛顶栏快捷图标、菜单栏 Popover、分栏设置面板与右键上下文菜单全入口支持一键热切换，玻璃拟态自适应
- **折叠与收起极致顺滑**：
  - **光标离开自动折叠**：无缝计算防抖延时，移出卡片稳定贴边，绝不回弹；
  - **全局失焦点击收起（Click-outside）**：点击外部桌面或其他窗口任意位置即刻平滑折叠；
  - **一键显式折叠**：顶栏右侧新增一键收起按钮，右键菜单同步支持「收起灵动岛」；
- **自由移动与智能贴边吸附**：展开卡片顶栏按住即可在屏幕任意位置自由拖拽；松手按最近距离吸附到上、右、下、左任一屏幕边缘，并持久化该边的轴向锚点坐标
- **6pt 晶莹微细条（Sliver）与触碰弹出**：收起时在屏幕边缘保留一条 6pt 厚度的半透明微细条（含 Agent 工作状态呼吸绿灯），光标碰触微细条即以 Apple 级流体弹簧动效自动弹出完整卡片
- **顶部菜单栏 Compact Island Popover**：菜单栏采用现代 macOS 原生浮窗（`.window`），实时显示呼吸状态灯、活跃 Agent 概览、Token 统计与操作直达
- **长标题自适应**：主顶栏将 Agent/状态与实时动作分层显示，始终保留标题最低可读宽度；文件、模型和动作长文本采用中间省略，悬停与 VoiceOver 可读取全文
- **卡内三级导航**：主列表 → 点 Agent 行看详情（双口径总览 + 按模型拆分）→ 点模型看会话列表（时间/消息数/token/花费，点击跳 Finder 目录）
- **现代分栏设置窗口**：macOS 原生 NavigationSplitView 四大分类（通用与外观、Agent 监控、引擎与性能、关于），支持外观模式、熔断阈值调节、提示音开关与停靠重置

### Token 用量统计
- **DimAgent**：读取 `~/.dimcode/v2/dimcode.sqlite` 的 usage_ledger（token 精确统计）
- **OpenCode**：读取 `opencode.db` 消息表（token + 花费 $）
- **Codex / Claude / WorkBuddy / WorkBuddy AI**：只读解析本机会话 JSONL 的结构化 usage 字段，不保留对话正文
- 行内徽标显示 24h 用量；卡片底部汇总栏双口径（跨工具 24h / 累计 + 花费）；点击汇总条进入 24h / 7天 / 30天时间分析，查看总体趋势、环比与按工具 Token / 占比 / 成本
- 分工具列表始终说明数据覆盖状态，真实零用量与“未发现本地明细”不会混为一谈；有本地明细的工具均可下钻查看摘要。内嵌 Codex 即使不单独显示为运行项，也不会失去用量入口；主卡仍保持一行汇总
- 60s 后台轮询；SQLite 只读打开，JSONL 按文件缓存并增量解析，不锁库、不碰凭证

### 检测层
- **内置注册表 + 自动发现 + 自定义**：内置 17 条（含 Antigravity、ZCode、WorkBuddy、OpenCode 等），启动扫描 /Applications + PATH 自动补充 CLI，设置里可添加自定义 Agent（进程名 + 会话目录）
- **五态判定**：结构化会话事件优先识别 `attention`（待确认）与 `completed`（已完成）；无强语义时按双信号降级：`working` = 进程在 且（60s 内有文件写入 **或** CPU ≥ 6%（桌面类档案有 20%/35% 下限）），`idle` = 进程在但两者皆不满足，`offline` = 进程不在
- **在线可见口径**：主列表、菜单摘要与顶部环只显示进程仍在的 Agent；待机、运行中、待确认、已完成均保留，离线项即使有近期活动或 Token 记录也隐藏
- **误报防护**：按完整路径匹配（非 basename），排除系统目录前缀（`/System/`、`/usr/libexec` 等）+ 黑名单（`CursorUIViewService`、`ssh-agent` 等）
- **高性能**：进程快照一次 libproc 遍历（proc_listpids/proc_pidpath）复用全部 profile，CPU 用两次采样差分；文件扫描后台递归 + 3s 节流 + 快跳过缓存，工作态开销约 1%

### 生命周期
- **节电平衡模式**：有活动 2s 采样，全闲置降频 5s
- **多屏跟随**：灵动岛跟随鼠标所在屏幕自适应停靠
- **全屏 Space 跟随**（fullScreenAuxiliary）
- **开机自启**：设置里开关（SMAppService），默认关
- **设置实时生效**：启停开关、熔断阈值、采样间隔、收起延迟、贴边位置、外观直接接入，无需重启

### 其他
- 只读监控：仅尾读本轮命中的会话文件并解析结构化事件字段，不展示确认问题/回答正文；不修改 Agent 数据，也不需要辅助功能/完全磁盘访问权限
- 任务通知：任务完成、等待确认、Token/CPU 异常均进入 macOS 通知中心；**投递与声音严格遵循通知策略**（完全静默不打扰、专注模式保留待确认与严重告警），提示音由应用内发声，不依赖通知权限
- 状态准确性：DeepSeek Harness 深层会话快速重扫；DimAgent 编辑历史与附件缓存不再误报工作中；睡眠/合盖唤醒后不会误报「任务完成」；**仅打开应用不做任何操作不再误报「任务完成」与提示音**（浏览器内核缓存/账号状态写入、SQLite `-shm` 空转触碰、纯 CPU 尖峰均不计入任务）
- 告警可读性：展开页显示告警摘要与排查说明，长文本单行省略，悬停可查看完整内容
- 低开销常驻：会话目录扫描跳过依赖树与配置目录，全闲置时自动放宽重扫周期；收起态 CPU 约 2%

## 安装

从 [Releases](https://github.com/bitterSmilezzz/AgentIsland/releases) 下载 `AgentIsland-0.0.35.zip`，解压后拖入「应用程序」或直接运行。

> 未公证（ad-hoc 签名），首次打开需右键 → 打开。

## 构建与运行

无需 Xcode，使用 SwiftPM + CommandLineTools 构建，手工组装 .app：

```bash
# 开发构建 + 自建测试套件（217 用例，含五态状态机/会话语义/通知路由/标题可读性/事件唤醒/进程树熔断/外观主题/命令清洗/token 时间统计）
swift build
.build/debug/AgentIslandTestsRunner     # 测试
.build/debug/AgentIsland --selftest     # 进程内自检
.build/debug/AgentIsland --probe        # 真实环境状态表（含 ACTION 操作列）

# 打包完整 .app（自动生成图标 + ad-hoc 签名）
./scripts/build-app.sh
open dist/AgentIsland.app

# 调试日志（数据源异常 / SafeNumber 告警 / 终止复核等诊断事件）
AGENTISLAND_DEBUG=1 open dist/AgentIsland.app
tail -f /tmp/agentisland.log
# 或不设开关直接用系统日志：
# log show --predicate 'subsystem == "com.agentisland.app"' --last 10m
```

## 目录结构

```
AgentIsland/
├── Package.swift                     # 4 target：Core 库 + App + IslandMetricsKit + 测试 runner
├── scripts/
│   ├── build-app.sh                  # .app 打包（无 Xcode 环境）
│   └── make-icon.swift               # 灵动岛风格图标生成
├── Sources/
│   ├── AgentIslandCore/              # 核心库（可被测试 import）
│   │   ├── Models.swift              # ActivityLevel / AgentProfile / AgentSnapshot / EngineConfig（含归一化/持久化读取）
│   │   ├── AgentRegistry.swift       # 内置集 + 自动发现 + 自定义存储（无全局状态）
│   │   ├── InstalledAppsCache.swift  # 已安装 CLI/bundle 缓存（注入实例，扫描器可替换）
│   │   ├── SettingsStore.swift       # SettingKey 键名 + EnabledAgentStore 启停集合持久化
│   │   ├── ProcessMonitor.swift      # libproc 快照（路径+CPU差分）/ 系统路径过滤 / 黑名单 / matcher / 测试 fake
│   │   ├── FileMonitor.swift         # 后台递归扫描 + 节流 + 单调 merge 缓存（主线程只读）
│   │   ├── AgentSessionInspector.swift # 会话尾部强语义解析（待确认/已完成）+ 已知 SQLite 适配
│   │   ├── TokenUsageMonitor.swift   # 跨工具 token 总体/时间线（SQLite + JSONL 只读）+ 轮询/查询协议 seam
│   │   ├── StructuredTokenUsageIndex.swift # Codex/Claude/WorkBuddy JSONL 缓存、增量解析与去重
│   │   ├── ActivityEngine.swift      # 五态状态机 + 双信号降级 + 逐事件通知流
│   │   ├── Probe.swift               # --probe 无头探测
│   │   └── Selftest.swift            # --selftest 进程内自检
│   └── AgentIsland/                  # UI（SwiftUI + AppKit）
│       ├── AgentIslandApp.swift      # @main + MenuBarExtra(Compact Island Popover) + AppContext 组合根
│       ├── IslandPanel.swift         # NSPanel 控制器（自由拖拽/智能贴边/微细条常驻/触碰弹出/peek）
│       ├── IslandView.swift          # 玻璃拟态卡片 + 贴边微细条 + 三级导航
│       ├── SideNotchShape.swift      # 反向倒角一体化贴边贝塞尔形状（边缘平滑向屏幕外壳过渡）
│       ├── AgentRingView.swift       # 环形微仪表盘（4级状态水位环 + 旋转微弧 + 呼吸警戒环 + 顶部微看板）
│       ├── AgentHoverTooltip.swift   # 悬停透视卡片与指向小尾巴（实时命令/文件/PID/Token/操作）
│       ├── IslandMetrics.swift       # 窗口几何唯一事实来源（尺寸常量 + 高度纯函数）
│       ├── IslandComponents.swift    # 共享 UI 基元（卡壳/hover 行/拖拽手势/分割线/loading）
│       ├── DetailViews.swift         # 卡内二级/三级详情页（模型拆分 + 会话列表）
│       ├── SettingsView.swift        # 现代分栏设置（NavigationSplitView 四大分类卡片）
│       └── Theme.swift               # Apple 设计令牌 + 动态色 + 晶莹描边（浅色/深色适配）
└── Tests/AgentIslandTestsRunner/     # 自建测试框架（零依赖，CLT 可用）
```

## 监控原理

```
┌──────────────┐   ┌────────────────────┐   ┌──────────────────┐
│ ProcessMatcher│   │ FileActivityMonitor │   │ TokenUsageMonitor │
│ · 进程表快照    │   │ · 后台批量枚举+缓存  │   │ · SQLite 只读     │
│ · 路径白名单  │   │ · 3s 节流           │   │ · SQLite + JSONL  │
│ · 黑名单排除  │   └─────────┬──────────┘   └────────┬─────────┘
└──────┬───────┘             │                        │
       └──────────┬──────────┴────────────────────────┘
                  ▼
        ActivityEngine（活动 2s / 闲置 5s 采样）
        · 未解决的确认/授权请求             → attention
        · 本轮明确结束                     → completed
        · 进程在 + 写入 60s 内 或 CPU≥6%  → working
        · 进程在但静默                     → idle
        · 进程不在                         → offline
                  ▼
        IslandPanel（NSPanel，非激活置顶，多屏跟随）
        · docked：上、右、下、左任一边贴边留存 6pt 晶莹微细条（触碰即弹性弹出）
        · expanded：330 宽玻璃卡片（自由长按拖动、智能吸附、列表/详情/会话三级导航）
```

## 已知限制

- 已适配结构化确认/完成事件的 Agent 可区分待确认与已完成；未知或改版后的日志格式会安全降级到进程 + 文件写入/CPU 三态近似
- Token 统计当前适配 DimAgent、OpenCode、Codex、Claude、WorkBuddy 与 WorkBuddy AI；其他工具若不提供稳定的本地 usage 明细，会在分析页明确标为未接入而不估算
- 闲置降频 5s 时，Agent 开始工作的检测最多延迟一个采样周期（可调「闲置降频间隔」）
- 多显示器跟随鼠标所在屏的右缘（NSScreen.screens）
