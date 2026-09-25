# AgentIsland for Windows

macOS 端 AgentIsland 的 Windows 端实现：Agent 会话灵动岛监控器，四边贴边微细条 + 弹性展开玻璃卡片 + 五态监控 + Token 用量统计 + 本地 Webhook。与本仓库 `Sources/`（SwiftUI/macOS 端）共用同一套设计令牌、几何度量、五态判定口径与 Webhook 协议。

## 构建与运行

零第三方 NuGet 依赖（WPF + WinForms 托盘，均为 Windows 桌面框架自带），需要 .NET 9 SDK：

```powershell
cd windows/AgentIsland.Wpf
dotnet build -c Debug
./bin/Debug/net9.0-windows10.0.19041.0/AgentIsland.exe
```

启动后常驻托盘；屏幕四边之一出现 6pt 悬浮微细条（默认顶部），光标移入即弹性展开完整卡片，光标移出自动收起（可配延迟）。顶栏按住可拖拽，松手吸附最近边缘并持久化。

### 验收 / 调试参数

| 参数 | 作用 |
|---|---|
| `--expand` | 启动 2 秒后自动展开卡片（自动化截图用） |
| `--demo` | 注入演示快照（对齐 macOS 端 site 截图的样例状态），不读真实进程 |
| `--route=tokenAnalytics` | 展开后直接进入 Token 分析页 |
| `--route=agentDetail:<id>` | 展开后直接进入某 Agent 详情页 |

异常日志：`%TEMP%\agentisland.log`（仅 UI 线程未处理异常）。

## 与 macOS 端的对应关系

| macOS（Sources/） | Windows（windows/AgentIsland.Wpf/） | 说明 |
|---|---|---|
| `Theme.swift` 设计令牌 | `UI/Theme.cs`（Theme/Ramp/Palette + IslandMetrics） | 色值一一对应：深色荧光（neonGreen/amber/red、sydedockCyan…）、浅色 AA 加深档；几何常量同源（cardWidth 330、sliver 140×6、curl 10、radiusLg 18…） |
| `SideNotchShape.swift` | `UI/Theme.cs → SideNotchGeometry` | 四边反向倒角 Bézier/圆弧路径，左右/上下互为镜像，同 macOS 实现策略 |
| `IslandPanel.swift`（NSPanel） | `UI/IslandWindow.cs` | 无边框置顶透明窗；统一锚点模型（细条中心=卡片中心）；阻尼弹簧尺寸动画（response 0.42 / damping 0.8） |
| `DockedSliver.swift` | `UI/IslandWindow.cs → SliverVisual` | 6pt 胶囊 + 工作态/告警态呼吸微光（动画时钟驱动）+ 12pt 命中延伸 |
| `IslandView/AgentRowView/TokenSummaryBar` | `UI/CardViews.cs` | 顶栏状态摘要（稳定标题+实时动作双层）、活动环看板、事件横幅（原因/直达/两段式关闭）、搜索、Agent 行（环+Token 徽标+活动点阵+内存徽标+状态药丸+工作态动作横条）、24H/TOTAL 汇总栏 |
| `AgentRingView.swift` | `UI/Controls.cs → AgentRing` | 圆角矩形水位环 + 工作态旋转微弧 + 等待态呼吸环 + 居中 Glyph；水位弧长/分级色规则同源 |
| `ActivityEngine.swift` | `Core/ActivityEngine.cs` | 五态机：结构化信号优先（attention/completed），降级双信号（写入 60s 内 ∨ CPU≥6%，桌面类档案 20% 下限），minWorkingHold 滞回；完成事件带时长、attention 指纹去重、CPU 熔断（70%×5min）与 Token 告警 |
| `ProcessMonitor/FileMonitor` | `Core/ProcessMonitor.cs`、`Core/FileMonitor.cs` | 进程快照 + CPU 两拍差分（首拍返回「没测」而不是 0）；目录树扫描 + 节流缓存，跳过依赖/缓存目录 |
| `AgentSessionInspector` | `Core/SessionInspector.cs` | JSONL 有界尾读：Claude（pending tool_use→在途动作/AskUserQuestion→待确认）、Codex（function_call/agent_message）、Cline/Roo（ui_messages ask/say）；解析失败给健康结论不谎报待机 |
| `TokenUsageMonitor` | `Core/TokenUsageMonitor.cs` | 只读解析 JSONL 结构化 usage（净消耗，不含缓存读取），文件指纹增量缓存，按模型拆分 + 30 天逐小时桶；成本用内置价目表估算 |
| `AgentRegistry.swift` | `Core/AgentRegistry.cs` | 内置档案改 Windows 路径（`~/.claude/projects`、`~/.codex/sessions`、`%APPDATA%/Cursor`、Cline/Roo globalStorage 等） |
| 本地 Webhook `127.0.0.1:41999` | `Core/LocalEventServer.cs` | 同端口同协议：`POST /notify`、`/event` 无鉴权直推（岛内标「外部投递」）；`POST|DELETE /session` 需 `X-AgentIsland-Token`（令牌在 `%APPDATA%\AgentIsland\report.token`，TTL 钳 15–600s；自报暂不参与显示，与 macOS 端现状一致） |
| 菜单栏 MenuBarExtra | `UI/Tray.cs`（NotifyIcon 托盘） | 左键切换展开/收起，右键吸附边缘/外观/偏好设置/退出 |
| `SettingsView.swift` | `UI/SettingsWindow.cs` | 通用与外观 / Agent 监控 / 引擎与性能 / 关于；设置落盘 `%APPDATA%\AgentIsland\settings.json`，改「收起延迟/贴边」实时生效 |

## 已知与 macOS 端的差距

- 玻璃卡背景用近不透明冷炭/白瓷渐变近似 macOS 的 `ultraThinMaterial`（Windows 无等价系统级毛玻璃；如需真 Acrylic 可后续接 `SetWindowCompositionAttribute`）。
- 键盘流（`1~3`/`j,k`/`Enter`/`?` HUD）、Peek 微弹窗、远程通知（ntfy/HTTP 模板/SMTP）、维护工作台（孤儿/死锁扫描清理）、事件历史时间线尚未实现；主列表的终止/直达/实时流水快捷操作已就位。
- Token 成本为内置价目表估算（与 macOS 端「估算」口径一致，非账单精确值）。
- 多显示器跟随光标所在屏已支持（per-monitor DPI 已换算）；跨屏拖拽后的再吸附持久化与 macOS 端一致走设置存档。

## Webhook 快速验证

```bash
curl -X POST http://127.0.0.1:41999/notify \
  -H "Content-Type: application/json" \
  -d '{"agent":"Claude","event":"attention","message":"需要确认: 是否允许执行 swift test"}'
```

`event` 取值：`completed` / `attention` / `costSpike`。岛内横幅会带「外部确认/外部告警」徽标。
