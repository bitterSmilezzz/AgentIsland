# AgentIsland — Agent 会话灵动岛监控器

监控本机所有 Agent 软件（DimAgent / Claude / Codex / Cursor / Trae / Google Antigravity / ZCode / WorkBuddy / Copilot / OpenCode 等）的会话状态；以 macOS 灵动岛风格呈现，支持**自由拖拽智能贴边（顶部灵动岛 / 右侧边栏）**、**6pt 晶莹微细条常驻感知**、**深浅外观切换**与**光标触碰自动弹性弹出**。

## 功能（v0.0.17）

### 设计美学与 CodeNotch 灵动交互
- **反向倒角一体化贴边（Inverse Rounded Corner / Bezel Flares）**：
  - 基于数学级三次 Bézier 曲线实现 `SideNotchShape`，支持右侧（`.right`）与顶部（`.top`）贴边；
  - 边缘向屏幕物理边框平滑过渡，如同从屏幕外壳硬件级一体化生长出来，彻底告别悬空矩形与割裂感；
  - 玻璃拟态卡片背景与 AppKit 投射阴影（`ShadowHostView`）共用精准几何路径。
- **环形微仪表盘与双层动态活动弧（Micro-Dashboards & Activity Arcs）**：
  - **4 级状态水位环**：荧光绿（工作）、琥珀黄（等待/降频）、预警橙（高负载）、极光红（熔断告警/离线）；
  - **内圈高精度动态弧**：工作状态下呈现 `0.25` 弧长、1.2s 无级平滑旋转的渐变微弧（`SpinningActivityArc`）；
  - **呼吸警戒环（PulsingAttentionArc）**：告警/等待状态下呈现平滑呼吸缩放光环，状态一目了然；
  - **顶部活跃微看板（Quick Rings Shelf）**：展开卡片顶部直观罗列当前活跃的所有 Agent 微仪表盘，点击秒切。
- **精准悬停透视卡片与指向小尾巴（Hover Tooltip Card & Tail）**：
  - 光标悬停在任意 Agent 环或列表项时，秒出半透明指向气泡浮层（带几何小尖角 `TooltipTail`）；
  - **零点击透视**：直达实时执行命令、正在修改的代码文件、系统进程 PID、24h / 累计 Token 消耗与计费；
  - **内嵌快捷动作**：悬停卡片直接集成「直达窗口」与一键防误触「终止进程逃生舱」。

### 实时伴侣与指令中心
- **免打扰与通知分级（Focus Mode）**：
  - **专注免打扰（默认推荐）**：普通任务执行完毕静默更新（绝不弹窗微窥、不响提示音），不打断正常编码心流；仅在**成本激增、死循环熔断告警**等重大异常时立即滑出 6s 微弹窗并播放告警音；
  - **标准模式 / 完全静默**：挂机等待交付可一键切换为标准模式；需要绝对清净可随时一键切换为完全静默；
  - **移除冗余弹窗**：彻底移除 Agent 一启动工作就弹窗微窥的打扰行为。
- **原生支持 Google Antigravity & ZCode**：支持 Electron 与 CLI 进程扫描、实时轨迹日志解析（tool_use 与动作上下文）、会话监控与窗口直达
- **任务完成主动提醒与 Peek 微窥**：标准模式下 Agent 结束持续工作（≥3.5s）转为空闲时，自动播放轻脆系统提示音（Glass），并在收起态下自动滑出 3.5 秒 Peek 微弹窗；若光标移入则自动转换为常驻展开，无需手动翻找进度
- **终端与 IDE 窗口一键直达**：
  - **GUI 智能体（Cursor、Trae、Antigravity 等）**：通过 BundleID / PID 一键拉至最前并聚焦；
  - **CLI 智能体（Claude Code、Codex、Dim 等）**：毫秒级递归追溯进程树父节点，精准定位 Terminal、iTerm2、VS Code、Ghostty、Warp 等终端宿主窗口，一键将黑底终端置顶呼出
- **实时操作透视**：
  - 子进程命令实时提取（`git diff`、`swift test`、`npm run build` 等）并智能清洗包裹层；
  - DimAgent / Claude / Codex / Antigravity 会话日志解析，直观呈现 `正在修改: IslandView.swift`、`正在执行: pytest` 等动态徽标；
- **成本与异常熔断保护（逃生舱）**：
  - **Token 暴涨检测**：滑动差分监测单分钟 Token 增量，超阈值时弹出双行自适应告警卡片并播放警示音；
  - **异常长耗时死循环告警**：基于基准采样的防误报算法，持续异常高负荷工作未释放时自动预警；
  - **一键 Kill 逃生舱**：红色熔断横幅直达终止，带两段式确认（首击进入确认态、3 秒自动复位）避免误触杀错进程树；列表行悬浮红色「终止」按钮同样带确认态；
  - **告警保护**：严重告警在 30 秒内不会被其他 Agent 的完成事件顶掉，留出处置时间；

### 灵动岛交互与外观
- **浅色 / 深色 / 跟随系统模式**：灵动岛顶栏快捷图标、菜单栏 Popover、分栏设置面板与右键上下文菜单全入口支持一键热切换，玻璃拟态与阴影投影自适应
- **折叠与收起极致顺滑**：
  - **光标离开自动折叠**：无缝计算防抖延时，移出卡片稳定贴边，绝不回弹；
  - **全局失焦点击收起（Click-outside）**：点击外部桌面或其他窗口任意位置即刻平滑折叠；
  - **一键显式折叠**：顶栏右侧新增一键收起按钮，右键菜单同步支持「收起灵动岛」；
- **自由移动与智能贴边吸附**：展开卡片顶栏按住即可在屏幕任意位置自由拖拽；松手根据物理距离智能吸附到屏幕顶部或右侧，并持久化锚点坐标记忆
- **6pt 晶莹微细条（Sliver）与触碰弹出**：收起时在屏幕边缘保留一条 6pt 厚度的半透明微细条（含 Agent 工作状态呼吸绿灯），光标碰触微细条即以 Apple 级流体弹簧动效自动弹出完整卡片
- **顶部菜单栏 Compact Island Popover**：菜单栏采用现代 macOS 原生浮窗（`.window`），实时显示呼吸状态灯、活跃 Agent 概览、Token 统计与操作直达
- **卡内三级导航**：主列表 → 点 Agent 行看详情（双口径总览 + 按模型拆分）→ 点模型看会话列表（时间/消息数/token/花费，点击跳 Finder 目录）
- **现代分栏设置窗口**：macOS 原生 NavigationSplitView 四大分类（通用与外观、Agent 监控、引擎与性能、关于），支持外观模式、熔断阈值调节、提示音开关与停靠重置

### Token 用量统计
- **DimAgent**：读取 `~/.dimcode/v2/dimcode.sqlite` 的 usage_ledger（token 精确统计）
- **OpenCode**：读取 `opencode.db` 消息表（token + 花费 $）
- 行内徽标显示 24h 用量；卡片底部汇总栏双口径（24h / 累计 + 花费）；详情页按模型/按会话下钻
- 60s 后台轮询，SQLite 只读打开，不锁库、不碰凭证

### 检测层
- **内置注册表 + 自动发现 + 自定义**：内置 12 条（新增 Antigravity、ZCode），启动扫描 /Applications + PATH 自动补充 CLI，设置里可添加自定义 Agent（进程名 + 会话目录）
- **双信号判定**：`working` = 进程在 且（60s 内有文件写入 **或** CPU > 1%）；`idle` = 进程在但两者皆不满足；`offline` = 进程不在
- **误报防护**：按完整路径匹配（非 basename），排除系统目录前缀（`/System/`、`/usr/libexec` 等）+ 黑名单（`CursorUIViewService`、`ssh-agent` 等）
- **高性能**：进程快照一次 libproc 遍历（proc_listpids/proc_pidpath）复用全部 profile，CPU 用两次采样差分；文件扫描后台递归 + 15s 节流 + 快跳过缓存，工作态开销约 1%

### 生命周期
- **节电平衡模式**：有活动 2s 采样，全闲置降频 15s
- **多屏跟随**：灵动岛跟随鼠标所在屏幕自适应停靠
- **全屏 Space 跟随**（fullScreenAuxiliary）
- **开机自启**：设置里开关（SMAppService），默认关
- **设置实时生效**：启停开关、熔断阈值、采样间隔、收起延迟、贴边位置、外观直接接入，无需重启

### 其他
- 只读监控：不读取任何会话隐私数据，不需要辅助功能/完全磁盘访问权限
- 任务通知：任务完成、等待确认、Token/CPU 异常均进入 macOS 通知中心；**投递与声音严格遵循通知策略**（完全静默不打扰、专注免打扰仅告警发声），提示音由应用内发声，不依赖通知权限
- 状态准确性：DeepSeek Harness 深层会话快速重扫；DimAgent 编辑历史与附件缓存不再误报工作中；睡眠/合盖唤醒后不会误报「任务完成」
- 告警可读性：展开页显示告警摘要与排查说明，长文本单行省略，悬停可查看完整内容
- 低开销常驻：会话目录扫描跳过依赖树与配置目录，全闲置时自动放宽重扫周期；收起态 CPU 约 2%

## 安装

从 [Releases](https://github.com/bitterSmilezzz/AgentIsland/releases) 下载 `AgentIsland-0.0.17.zip`，解压后拖入「应用程序」或直接运行。

> 未公证（ad-hoc 签名），首次打开需右键 → 打开。

## 构建与运行

无需 Xcode，使用 SwiftPM + CommandLineTools 构建，手工组装 .app：

```bash
# 开发构建 + 自建测试套件（57 用例，含状态机/双信号/事件唤醒/进程树熔断/外观主题/通知策略/命令清洗/token 统计）
swift build
.build/debug/AgentIslandTestsRunner     # 测试
.build/debug/AgentIsland --selftest     # 进程内自检
.build/debug/AgentIsland --probe        # 真实环境状态表（含 ACTION 操作列）

# 打包完整 .app（自动生成图标 + ad-hoc 签名）
./scripts/build-app.sh
open dist/AgentIsland.app

# 调试日志（状态机跟踪）
AGENTISLAND_DEBUG=1 open dist/AgentIsland.app
tail -f /tmp/agentisland.log
```

## 目录结构

```
AgentIsland/
├── Package.swift                     # 3 target：Core 库 + App + 测试 runner
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
│   │   ├── TokenUsageMonitor.swift   # token 用量统计（dimcode.sqlite / opencode.db 只读查询）+ 轮询/查询协议 seam
│   │   ├── ActivityEngine.swift      # 双信号状态机 + 节电动态采样 + 呈现活跃单一 owner
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
│ · ps 快照一次 │   │ · 后台批量枚举+缓存  │   │ · SQLite 只读     │
│ · 路径白名单  │   │ · 15s 节流          │   │ · 60s 轮询        │
│ · 黑名单排除  │   └─────────┬──────────┘   └────────┬─────────┘
└──────┬───────┘             │                        │
       └──────────┬──────────┴────────────────────────┘
                  ▼
        ActivityEngine（活动 2s / 闲置 15s 采样）
        · 进程在 + 写入 60s 内 或 CPU>1% → working
        · 进程在但静默                    → idle
        · 进程不在                        → offline
                  ▼
        IslandPanel（NSPanel，非激活置顶，多屏跟随）
        · docked：顶部或右侧贴边留存 6pt 晶莹微细条（触碰即弹性弹出）
        · expanded：280 宽玻璃卡片（自由长按拖动、智能吸附、列表/详情/会话三级导航）
```

## 已知限制

- 「会话进行中」以进程 + 文件写入/CPU 为信号，无法区分「思考中/已暂停」（不读内容，隐私优先）
- Token 统计仅覆盖有本地数据源的 agent（DimAgent / OpenCode）；其他 agent 无本地 usage 记录则不显示
- 闲置降频 15s 时，Agent 开始工作的检测最多延迟一个采样周期（可调「闲置降频间隔」）
- 多显示器跟随鼠标所在屏的右缘（NSScreen.screens）
