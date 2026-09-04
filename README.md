# AgentIsland — Agent 会话灵动岛监控器

监控本机所有 Agent 软件（DimAgent / Claude / Codex / Cursor / Trae / WorkBuddy / Copilot / OpenCode 等）的会话状态；以 macOS 灵动岛风格悬浮卡片呈现在屏幕顶部，QQ 式贴边隐藏，不占桌面空间。

## 功能（v1.2）

### 灵动岛交互
- **QQ 式贴边隐藏**：收起时只露 5px 细条（忙时深色+绿点，闲时浅色），触碰屏幕顶部或悬停细条即滑出卡片，移开 0.5s 自动收回
- **忙碌提醒 peek**：Agent 从空闲变忙碌时细条下滑弹一下
- **卡内三级导航**：主列表 → 点 Agent 行看详情（双口径总览 + 按模型拆分）→ 点模型看会话列表（时间/消息数/token/花费，点击跳 Finder 目录）
- **玻璃拟态**：NSVisualEffectView(.hudWindow) + 动态蒙层，背景内容微透，与系统通知中心一致
- **浅色 / 深色 / 跟随系统**：设置里一键切换，颜色全动态适配
- **菜单栏**：working 时图标变绿 + 圆点角标；下拉菜单精简（展开/收起、重置位置、设置、退出）

### Token 用量统计
- **DimAgent**：读取 `~/.dimcode/v2/dimcode.sqlite` 的 usage_ledger（token 精确统计）
- **OpenCode**：读取 `opencode.db` 消息表（token + 花费 $）
- 行内徽标显示 24h 用量；卡片底部汇总栏双口径（24h / 累计 + 花费）；详情页按模型/按会话下钻
- 60s 后台轮询，SQLite 只读打开，不锁库、不碰凭证

### 检测层
- **内置注册表 + 自动发现 + 自定义**：内置 10 条（按真实 bundle id 修正），启动扫描 /Applications + PATH 自动补充 CLI，设置里可添加自定义 Agent（进程名 + 会话目录）
- **双信号判定**：`working` = 进程在 且（60s 内有文件写入 **或** CPU > 1%）；`idle` = 进程在但两者皆不满足；`offline` = 进程不在
- **误报防护**：按完整路径匹配（非 basename），排除系统目录前缀（`/System/`、`/usr/libexec` 等）+ 黑名单（`CursorUIViewService`、`ssh-agent` 等）
- **高性能**：进程快照一次 `ps` 复用全部 profile；文件扫描用批量枚举 API（getattrlistbulk）替代逐文件 syscall，3s 节流 + 5s 会话数缓存，CPU 占用 <1%

### 生命周期
- **节电平衡模式**：有活动 2s 采样，全闲置降频 15s
- **多屏跟随**：岛显示在鼠标所在屏幕顶部
- **全屏 Space 跟随**（fullScreenAuxiliary）
- **开机自启**：设置里开关（SMAppService），默认关
- **设置实时生效**：启停开关、阈值、采样间隔、外观直接接入，无需重启

### 其他
- 只读监控：不读取任何会话内容，不需要辅助功能/完全磁盘访问权限

## 安装

从 [Releases](https://github.com/bitterSmilezzz/AgentIsland/releases) 下载 `AgentIsland-1.2.0.zip`，解压后拖入「应用程序」或直接运行。

> 未公证（ad-hoc 签名），首次打开需右键 → 打开。

## 构建与运行

本机无 Xcode，使用 SwiftPM + CommandLineTools 构建，手工组装 .app：

```bash
# 开发构建 + 自建测试套件（27 用例，含状态机/双信号/误报排除/文件监控/token 统计）
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
│   │   ├── Models.swift              # ActivityLevel / AgentProfile / AgentSnapshot / EngineConfig
│   │   ├── AgentRegistry.swift       # 内置集 + 自动发现 + 自定义存储
│   │   ├── ProcessMonitor.swift      # ps 快照（路径+CPU）/ 系统路径过滤 / 黑名单 / matcher
│   │   ├── FileMonitor.swift         # 后台批量枚举扫描 + 节流 + 缓存（主线程只读）
│   │   ├── TokenUsageMonitor.swift   # token 用量统计（dimcode.sqlite / opencode.db 只读查询）
│   │   ├── ActivityEngine.swift      # 双信号状态机 + 节电动态采样 + 启停接线
│   │   ├── Probe.swift               # --probe 无头探测
│   │   └── Selftest.swift            # --selftest 进程内自检
│   └── AgentIsland/                  # UI（SwiftUI + AppKit）
│       ├── AgentIslandApp.swift      # @main + MenuBarExtra + AppContext 单例
│       ├── IslandPanel.swift         # NSPanel 控制器（docked/expanded 状态机 + 多屏跟随 + peek）
│       ├── IslandView.swift          # 玻璃拟态卡片 + QQ 交互 + 贴边细条
│       ├── DetailViews.swift         # 卡内二级/三级详情页（模型拆分 + 会话列表）
│       ├── SettingsView.swift        # 设置（启停/自定义/参数/自启/外观）
│       └── Theme.swift               # Apple 设计令牌 + 动态色（浅色/深色适配）
└── Tests/AgentIslandTestsRunner/     # 自建测试框架（零依赖，CLT 可用）
```

## 监控原理

```
┌──────────────┐   ┌────────────────────┐   ┌──────────────────┐
│ ProcessMatcher│   │ FileActivityMonitor │   │ TokenUsageMonitor │
│ · ps 快照一次 │   │ · 后台批量枚举+缓存  │   │ · SQLite 只读     │
│ · 路径白名单  │   │ · 3s 节流           │   │ · 60s 轮询        │
│ · 黑名单排除  │   └─────────┬──────────┘   └────────┬─────────┘
└──────┬───────┘             │                        │
       └──────────┬──────────┴────────────────────────┘
                  ▼
        ActivityEngine（活动 2s / 闲置 15s 采样）
        · 进程在 + 写入 60s 内 或 CPU>1% → working
        · 进程在但静默                    → idle
        · 进程不在                        → offline
                  ▼
        IslandPanel（NSPanel，非激活置顶，多屏跟随，全屏跟随）
        · docked：176×5 贴边细条（忙绿点/闲浅色）
        · expanded：280 宽玻璃卡片（列表/详情/会话三级导航）
```

## 已知限制

- 「会话进行中」以进程 + 文件写入/CPU 为信号，无法区分「思考中/已暂停」（不读内容，隐私优先）
- Token 统计仅覆盖有本地数据源的 agent（DimAgent / OpenCode）；其他 agent 无本地 usage 记录则不显示
- 闲置降频 15s 时，Agent 开始工作的检测最多延迟一个采样周期（可调「闲置降频间隔」）
- 多显示器跟随鼠标所在屏（NSScreen.screens）
