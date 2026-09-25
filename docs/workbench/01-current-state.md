# 01 · 现状（已核实）

> 本文所有数字都是本次调研实扫实量得到的，不是估算。凡未核实的一律标「未核实」。
> 核实时间：2026-09-25。

## 1. 仓库形态

SwiftPM 主包（macOS 本体）与两个多平台克隆并存：

```
Sources/            97 个 .swift，29,526 行 —— macOS 本体（本计划不改动）
Tests/              自建测试 runner（无 XCTest），544 项断言
app/                Tauri v2 多平台端：Rust 核心 2,670 行 + 纯静态前端（无 npm）
windows/            WPF 原生实现，保留作功能偏离对照（不承担新形态）
site/               GitHub Pages 主页
scripts/            构建 / 测试 / 发版 / 脱敏扫描
docs/               ADR、调研、验收记录、agent 协同约定
```

`git ls-tree HEAD` 顶层：`Sources Tests Package.swift README.md CHANGELOG.md CONTEXT.md AGENTS.md CLAUDE.md GEMINI.md skills-lock.json app docs scripts site windows`。

## 2. 多平台端（`app/`）—— 本轮的主战场

### 2.1 Rust 核心：2,670 行，12 模块

| 文件 | 行数 | 职责 | 与形态耦合度 |
| :--- | :--- | :--- | :--- |
| `engine.rs` | 575 | 五态状态机、采样节奏、事件装配 | **无耦合**，纯逻辑 |
| `session.rs` | 414 | 五种会话方言的 JSONL 尾读 | **无耦合** |
| `tokens.rs` | 318 | Token 净消耗统计、保留窗口、折入 | **无耦合** |
| `main.rs` | 397 | 窗口壳、command 注册、引擎循环、托盘 | **强耦合**，需按形态分派 |
| `views`/`models.rs` | 183 | 数据模型 | 弱 |
| `registry.rs` | 184 | Agent 档案内置集 | 无 |
| `webhook.rs` | 152 | 本机 HTTP 41999 入口 | 无 |
| `procmon.rs` | 129 | 进程 CPU 差分 | 无 |
| `filemon.rs` | 138 | 文件监控 | 无 |
| `settings.rs` | 80 | 设置持久化 | 弱（要加 `shell_mode`） |
| `placement.rs` | 100 | 灵动岛四边吸附定位 | **强耦合**，需按形态分派 |

**结论：约 2,100 行（79%）与界面形态无关，可直接复用。**

### 2.2 前端：3 个 JS + 2 个 CSS

| 文件 | 行数 | 现状 | 处置 |
| :--- | :--- | :--- | :--- |
| `ui/js/views.js` | 690 | `st.route` 字符串路由到 `list` / `tokenAnalytics` / `agentDetail:<id>`（`views.js:282-295`） | 抽成监控模块，两种形态共用 |
| `ui/js/main.js` | 177 | 入口，装载灵动岛壳 | 读 `shell_mode` 分派 |
| `ui/js/tauri.js` | 16 | invoke 封装 | 原样复用 |
| `ui/css/island.css` | 505 | 玻璃卡、环图、横幅、notch、sliver | 拆出 island 专属与共用组件 |
| `ui/css/tokens.css` | 130 | 三端同源设计令牌，深/浅双主题 | **零改动复用** |

### 2.3 已注册的 Tauri command（12 个）

`get_boot_args` / `get_settings` / `save_settings` / `set_dock_edge` / `place_island` /
`snap_nearest_edge` / `reposition_now` / `get_report` / `clear_latest_event` /
`terminate_agent` / `collapse_to_tray` / `log_from_ui`

## 3. 已实现的功能面（要保住的资产）

### 3.1 监控

- **五态状态机**：`offline` / `idle` / `completed` / `working` / `attention`
- **三种判定链路**：会话强语义（五种方言尾读）优先，无命中降级到双信号（文件写入 或 CPU 超阈值）
- **采样节奏**：有活动 2s，全闲置降频 5s；Docked 态 CPU 0.0–0.1%
- **性能手段**：一次 libproc 遍历复用全部档案、CPU 两次采样差分、会话定位缓存（TTL 10s + 根目录 mtime 失效）、尾读合并、32MB 内存映射上限

### 3.2 Token

- 24h 与累计两组，净消耗口径（不含 cache.read）
- **「没查到」与「确实是零」是两个状态**：所有对外表面都必须能分辨（`—` / `null` / 空字段）
- 明细保留 70 天，更早折入按工具合计（保和不丢）
- 日历日切桶 + 滚动 24h 预算

### 3.3 可信度与复核

- 死锁判定与 CPU 差分都是**三态**（卡死 / 不卡死 / 本轮判不出），对外一律 `null` 不是 `false`
- 可观测性五类结论：`observed` / `blindSessionSource` / `noLocalData` / `sourceNotWired` / `notInstalled`
- `doctor`、岛内自查卡、审计报表共用同一实现，不许各说各话

### 3.4 外发与在场

- ntfy / 自建 HTTP / SMTP 三条通道，默认关闭
- **凭据只进 macOS 钥匙串**，UserDefaults 零密钥，界面只出掩码
- 在场判定三信号：锁屏、显示器睡眠、无输入超时（fail-open）
- 明确拒绝不重试，暂时失败自动重试一次

### 3.5 面板与菜单栏

- `docked` / `expanded` 两态、6pt 微细条、光标碰触自动弹出
- 四边停靠 + 智能吸附 + 轴向锚点持久化、peek 短暂示警、MenuBarExtra 浮窗

## 4. macOS 本体的定位

`Sources/`（97 文件 29,526 行）实现了与 Rust 端**同一套**功能、令牌与判定口径。
它在本轮**不改动**，但双形态一旦在 Rust 端成立，Swift 端会出现
「它是唯一形态还是并列形态」的口径问题。已在 [04-plan.md](04-plan.md) 列为开放项。

## 5. 本机配置文件面（CC Switch 的输入条件）

2026-09-25 实扫 `~`：

| 工具 | 文件 | 大小 | 关键结构 |
| :--- | :--- | --- | :--- |
| Claude Code | `~/.claude/settings.json` | 271B | `{permissions, hooks}`，**`env` 为空** |
| Claude Code | `~/.claude.json` | 548B | — |
| Codex | `~/.codex/config.toml` | 3533B | `[desktop]` / `[mcp_servers.*]` / `[marketplaces.*]` / `[model_providers]` |
| Codex | `~/.codex/auth.json` | 3954B | 凭据载体 |
| Gemini CLI | `~/.gemini/settings.json` | — | **不存在** |
| OpenCode | `~/.opencode/config.json` | — | **不存在** |
| Copilot / Cursor CLI | — | — | **不存在** |

→ 第一阶段 CC Switch **只做 Claude Code 与 Codex**。

## 6. 技术债（必须先修的）

### 6.1 脱敏闸门假绿（Phase 0 已修）

`scripts/scan-secrets.sh` 有两处失败静默，症状都是「什么都没扫，却打印 ✓ 通过」：

1. `file_list > files` 后只判断列表空不空。git fatal 产出空列表，被当成「无待扫文件」放行。
   权威复现：输出 `fatal: not a git repository` 紧跟 `✓ 无待扫文件`，返回 0。
2. 中间文件（命中表 / 文件列表）写入无人接，空文件被下游读成「零命中」。

另发现 `sort -u "$HITS" -o "$HITS"` 对同一文件 in-place 排序会触发 Bus error（SIGBUS 136）。

修法：所有中间文件统一走 `put()`（临时文件 + `mv`，每步失败 `exit 3`）；
`load_file_list()` 接住 git 退出码并拒绝空列表；基线缺失不再等价于零命中。
配套 [scripts/test-scan-secrets.sh](../../scripts/test-scan-secrets.sh) 7 条守卫用例，
并用修复前版本反向验证过测试确实能炸。

### 6.2 本机构建依赖降级 SDK

`MacOSX27.0.sdk`（8/31）的 `SwiftUICore.swiftinterface` 引用 `SwiftUIMacros` 模块，
但 CLT 插件目录（8/26）里只有 `libObservationMacros.dylib` 与 `libSwiftMacros.dylib`，
缺少对应插件 → 任何 `@State` 都报
`external macro implementation type 'SwiftUIMacros.StateMacro' could not be found`。

`scripts/build-app.sh:8-10` 已内置 `MacOSX26.5.sdk` 兜底，`README.md:127` 也写了 `SDKROOT`。
**Rust 端不受影响**，但直接 `swift build` 的路径必须先设环境变量。

### 6.3 CLI 读不到 App 内的自报记录

`agentisland status` / `doctor` 是独立进程，登记表永远为空，所以 CLI 与报表里的
`provenance` 只会出现 `observed` / `inferred`。已知限制，未修。

## 7. 干净基线

8 条规则 × 293 个 git 文件 = **18 条命中，全部已备案，零新增**
（`secrets-baseline.txt` 18 条；RemoteNotify 相关的测试假邮箱与假口令）。
