# AgentIsland — Agent 会话灵动岛监控器

监控本机所有 Agent 软件（DimAgent / Claude / Codex / Cursor / Trae / WorkBuddy / Copilot / OpenCode 等）的会话状态；以 macOS 灵动岛风格呈现，支持**自由拖拽智能贴边（顶部灵动岛 / 右侧边栏）**、**6pt 晶莹微细条常驻感知**与**光标触碰自动弹性弹出**。

## 功能（v1.4）

### 灵动岛交互
- **自由移动与智能贴边吸附**：展开卡片的顶栏按住即可在屏幕任意位置自由拖拽；松手根据物理距离智能吸附到屏幕顶部或右侧，并持久化锚点坐标记忆
- **6pt 晶莹微细条（Sliver）与触碰弹出**：收起时在屏幕边缘保留一条 6pt 厚度的半透明微细条（含 Agent 工作状态呼吸绿灯），光标碰触微细条即以 Apple 级流体弹簧动效自动弹出完整卡片，移开后按延迟时间平滑收回
- **顶部菜单栏 Compact Island Popover**：菜单栏采用现代 macOS 原生浮窗（`.window`），实时显示呼吸状态灯、活跃 Agent 概览、Token 统计与操作直达
- **忙碌提醒 peek**：Agent 从空闲变忙碌时灵动岛自动滑出提醒，随后平滑收回
- **卡内三级导航**：主列表 → 点 Agent 行看详情（双口径总览 + 按模型拆分）→ 点模型看会话列表（时间/消息数/token/花费，点击跳 Finder 目录）
- **动态单向圆角与玻璃拟态**：NSVisualEffectView(.hudWindow) + 动态蒙层 + 1px 晶莹微高光描边；顶部贴边下方圆角，右侧贴边左侧圆角
- **现代分栏设置窗口**：macOS 原生 NavigationSplitView 四大分类（通用与外观、Agent 监控、引擎与性能、关于），支持停靠位置切换与一键重置
- **浅色 / 深色 / 跟随系统**：设置里一键切换，颜色与微反光全动态自适应

### Token 用量统计
- **DimAgent**：读取 `~/.dimcode/v2/dimcode.sqlite` 的 usage_ledger（token 精确统计）
- **OpenCode**：读取 `opencode.db` 消息表（token + 花费 $）
- 行内徽标显示 24h 用量；卡片底部汇总栏双口径（24h / 累计 + 花费）；详情页按模型/按会话下钻
- 60s 后台轮询，SQLite 只读打开，不锁库、不碰凭证

### 检测层
- **内置注册表 + 自动发现 + 自定义**：内置 10 条（按真实 bundle id 修正），启动扫描 /Applications + PATH 自动补充 CLI，设置里可添加自定义 Agent（进程名 + 会话目录）
- **双信号判定**：`working` = 进程在 且（60s 内有文件写入 **或** CPU > 1%）；`idle` = 进程在但两者皆不满足；`offline` = 进程不在
- **误报防护**：按完整路径匹配（非 basename），排除系统目录前缀（`/System/`、`/usr/libexec` 等）+ 黑名单（`CursorUIViewService`、`ssh-agent` 等）
- **高性能**：进程快照一次 libproc 遍历（proc_listpids/proc_pidpath）复用全部 profile，CPU 用两次采样差分；文件扫描后台递归 + 15s 节流 + 快跳过缓存，工作态开销约 1%

### 生命周期
- **节电平衡模式**：有活动 2s 采样，全闲置降频 15s
- **多屏跟随**：灵动岛跟随鼠标所在屏幕自适应停靠
- **全屏 Space 跟随**（fullScreenAuxiliary）
- **开机自启**：设置里开关（SMAppService），默认关
- **设置实时生效**：启停开关、阈值、采样间隔、收起延迟、贴边位置、外观直接接入，无需重启

### 其他
- 只读监控：不读取任何会话内容，不需要辅助功能/完全磁盘访问权限

## 安装

从 [Releases](https://github.com/bitterSmilezzz/AgentIsland/releases) 下载 `AgentIsland-1.4.0.zip`，解压后拖入「应用程序」或直接运行。

> 未公证（ad-hoc 签名），首次打开需右键 → 打开。

## 构建与运行

本机无 Xcode，使用 SwiftPM + CommandLineTools 构建，手工组装 .app：

```bash
# 开发构建 + 自建测试套件（49 用例，含状态机/双信号/误报排除/文件监控/token 统计/设置与贴边规则）
swift build
.build/debug/AgentIslandTestsRunner     # 测试
.build/debug/AgentIsland --selftest     # 进程内自检
.build/debug/AgentIsland --probe        # 真实环境状态表

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
