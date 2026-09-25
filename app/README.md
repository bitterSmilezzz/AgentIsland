# AgentIsland — 跨平台端 (Tauri / Web + Rust)

AgentIsland 的多平台实现：一套代码出 **Windows / macOS / Linux** 三端。UI 用纯静态 Web 技术（HTML/CSS/JS，无 npm 依赖），核心监控用 Rust，灵动岛贴边/悬浮玻璃卡/五态监控/Token 统计/本地 Webhook 与 macOS 端（`Sources/`）和 Windows 原生端（`windows/`）共用同一套设计令牌与判定口径。

```
app/
├── ui/                      # 静态前端（Tauri 直接托管，无需 Node/npm）
│   ├── index.html
│   ├── css/tokens.css       # 设计令牌（移植 Theme.swift，深色黑曜石/浅色白瓷）
│   ├── css/island.css       # 灵动岛布局（细条/玻璃卡/环看板/横幅/行/汇总栏）
│   └── js/
│       ├── main.js          # 状态管理、展开/收起、贴边交互
│       ├── views.js         # 主卡/事件横幅/Agent 行/Token 分析页/详情页
│       └── tauri.js         # Tauri API 薄封装
└── src-tauri/               # Rust 核心
    ├── src/
    │   ├── models.rs        # 五态/快照/事件/Token（与 macOS Models.swift 同源）
    │   ├── registry.rs      # Agent 注册表（内置集，路径按平台展开）
    │   ├── engine.rs        # 五态状态机 + 采样循环（强语义优先/双信号降级/滞回/熔断）
    │   ├── procmon.rs       # 进程快照 + CPU 差分（sysinfo；首拍「没测」不谎报 0）
    │   ├── filemon.rs       # 会话目录扫描（节流缓存/跳过依赖目录）
    │   ├── session.rs       # JSONL 尾读强语义（Claude/Codex/Cline 方言族）
    │   ├── tokens.rs        # Token 用量（净消耗口径/指纹增量/按模型拆分/30天逐时桶）
    │   ├── webhook.rs       # 127.0.0.1:41999（/notify /event /session，与 macOS 同协议）
    │   ├── placement.rs     # 屏幕工作区 + DPI 换算 + 统一锚点放置
    │   └── main.rs          # 组装：托盘/命令/引擎线程
    └── tauri.conf.json      # 透明无边框置顶窗（330 宽卡 + 6pt 细条同窗形态切换）
```

## 构建与运行

需要 Rust（stable-msvc）+ MSVC Build Tools（链接器）+ WebView2（Win10/11 自带）。**不需要 Node/npm**：

```powershell
cd app/src-tauri
cargo tauri build          # 打包（NSIS 安装包）
# 或开发调试：
cargo build
./target/debug/agentisland.exe --expand --demo --route=tokenAnalytics
```

启动参数：`--demo` 注入演示数据；`--expand` 自动展开；`--route=tokenAnalytics | agentDetail:<id>` 直达子页。

## 交互（与 macOS 端一致）

- 收起态：屏幕任一边缘 6pt 呼吸微光细条，光标碰触弹性展开
- 展开态：顶栏按住拖拽，松手吸附最近边并持久化；光标移出防抖收起；失焦收起
- 卡内导航：主列表 → Token 分析 / Agent 详情；搜索 `/`；汇总栏直达分析页
- 托盘：左键切换展开，右键退出

## 与 macOS 端的口径对齐

- 设计令牌：`tokens.css` ← `Theme.swift`（同一批 Tailwind 色阶与荧光强调，深浅两套 AA 校准）
- 几何：cardWidth 330 / curl 10 / radiusLg 18 / sliver 140×6 / 命中区 +12pt
- 五态判定：attention/completed 强语义优先 → 双信号降级（写入 60s ∨ CPU≥6%）→ idle → offline；完成事件带时长、指纹去重、CPU 熔断（70%×5min）
- Token：净消耗（不含缓存读取）、按模型拆分、内置价目表估算成本
- Webhook：`POST /notify` `{"agent","event":"completed|attention|costSpike","message","detail"}` → 横幅带「外部确认/外部告警」徽标

## 已知差距（相对 macOS 原生端）

- 设置窗口用系统对话框替代（设置读写已通，UI 面板待补）
- 键盘流（1~3 / j,k / ? HUD）、Peek 微弹窗、远程通知、维护工作台未实现
- macOS/Linux 的窗口放置需各自补齐 `placement.rs` 的非 Windows 分支（接口已留好）
