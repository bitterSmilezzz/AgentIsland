# AgentIsland — 跨平台端 (Tauri / Web + Rust)

AgentIsland 的跨平台迁移目标：Rust 核心 + 纯静态 Web UI（HTML/CSS/JS，无 npm 依赖）。当前提供灵动岛窗口、五态监控、Token 统计与本地 Webhook；侧边栏、配置档位与 ToDos 尚未实现。macOS 的 `.app` / `.dmg` 可在本机构建，Windows/Linux 尚需对应平台构建与真机验收。与 Swift 原生端的能力差异见 [两端对照表](../docs/workbench/23-swift-rust-parity-matrix.md)。

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

macOS 需要 Rust stable、Command Line Tools 和 `cargo-tauri`。**不需要 Node/npm**：

```sh
cd app/src-tauri
cargo test --locked        # 合成样本回归，不依赖已安装的 Agent CLI
cargo tauri build --bundles app,dmg
# 或开发调试：
cargo build
./target/debug/agentisland --expand --demo --route=tokenAnalytics
```

Windows 需要 Rust stable-msvc、MSVC Build Tools 与 WebView2；打包时显式选择 `--bundles nsis`，可执行文件带 `.exe` 后缀。默认 bundle 配置为 macOS 的 `app` / `dmg`。

测试入口覆盖五态连续采样、保持时限、时钟回拨、CPU 阈值、事件去重，以及 Claude/Codex/Cline 合成会话的文件解析。完整发布由仓库根目录的 `scripts/release.sh` 执行 Rust 与 Swift 测试；正式下载包仍为 Swift 原生端，Tauri 产物位于 `src-tauri/target/release/bundle/`。

启动参数：`--demo` 注入演示数据；`--expand` 自动展开；`--route=tokenAnalytics | agentDetail:<id>` 直达子页。

## 交互（与 macOS 端一致）

- 收起态：屏幕任一边缘 6pt 呼吸微光细条，光标碰触弹性展开
- 展开态：顶栏按住拖拽，松手吸附最近边并持久化；光标移出防抖收起；失焦收起
- 卡内导航：主列表 → Token 分析 / Agent 详情；搜索 `/`；汇总栏直达分析页
- 托盘：左键切换展开，右键退出

## 与 macOS 端的口径对齐

- 设计令牌：`tokens.css` ← `Theme.swift`（同一批 Tailwind 色阶与荧光强调，深浅两套 AA 校准）
- 几何：cardWidth 330 / curl 10 / radiusLg 18 / sliver 140×6 / 命中区 +12pt
- 五态判定：进程不在即 offline；在线时 attention/completed 强语义优先，随后按双信号降级（写入 60s ∨ CPU≥阈值），信号消失后保持 10s 再回 idle。完成事件按指纹去重，CPU 熔断为持续高负载（70%×5min）。通知与会话新鲜度的完整 Swift 对齐仍待迁移。
- Token：净消耗（不含缓存读取）、按模型拆分、内置价目表估算成本
- Webhook：`POST /notify` `{"agent","event":"completed|attention|costSpike","message","detail"}` → 横幅带「外部确认/外部告警」徽标

## 已知差距（相对 macOS 原生端）

- 设置窗口用系统对话框替代（设置读写已通，UI 面板待补）
- 键盘流（1~3 / j,k / ? HUD）、Peek 微弹窗、远程通知、维护工作台未实现
- macOS/Linux 工作区由 Tauri monitor API 提供，Windows 使用 Win32 工作区；跨屏与不同 DPI 的真实设备验收仍需分别进行。
- `provider.rs` 与凭据掩码/原子写入守护尚无实现；Rust 回归通过不代表迁移前置门已全部完成。
