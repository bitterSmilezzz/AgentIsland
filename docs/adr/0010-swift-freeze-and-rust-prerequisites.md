# Swift 端冻结，全面转 Rust；但前置条件未齐之前不动一行

一句话说清本决定的两半：**方向上 Swift 本体改为只修 Bug、不再加功能，本体演进交给 Rust；执行上前置条件全齐之前不许开始迁移。** 前一句是 2026-09-26 用户拍板的，后一句是同一次拍板附带的——「先把文档、改造方案、测试框架等前期准备工作都弄齐了再转 Rust」。

## 决策时的现象与定位

> 下表是决定作出时的调查记录，不是当前构建与测试状态；当前进度见 M1–M5 状态列。

先摊开成本，因为这是全仓最贵的一次方向决定：

| 事实 | 数字 | 出处 |
|---|---|---|
| Swift 四 target | **28,200 行**（其中 Core 15,705 行，47 个文件） | `wc -l Sources/*/*.swift` |
| --- | --- | --- |
| Rust 端已有 | **2,670 行** / 11 个 rs | `wc -l app/src-tauri/src/*.rs` || 交换比 | 每 1 行 Rust 要换 **约 10.6 行 Swift Core** | 15,705 / 2,670…，按现有 Rust 规模折算 |
| Rust 缺的 Swift 模块 | **至少 12 个，约 5,000+ 行** | AgentHealthEvaluator、AgentResilienceGuard、TaskDurationTracker、TokenBudgetTracker、TokenForecastEvaluator、TokenCostEstimator、TokenReportExporter、AuditReportExporter、StructuredTokenUsageIndex、ProcessTreeInspector、RemoteNotifier ×4、SMTPSocket、Selftest |
| 测试分布 | Swift 34 个文件 548 条 `test(` → **Rust 侧今天 0 条** | `grep -rn '#\[test\]'` 零命中 |
| `app/` 构建史 | **从未成功构建**；capabilities 引用不存在的 `main` 窗口、`placement.rs` macOS 硬编码 1440×900、`bundle.targets` 只有 nsis | `capabilities/default.json:5`、`placement.rs:57-62`、`tauri.conf.json:40` |

三条最伤的：① 把一个 148 个版本验证过的产品体系押到一个**从没跑起来过的壳**上；② 548 条测试护的是 Swift 那一半，冻结后它们是沉没成本而不是资产；③ 「全面转 Rust」实操上是「先补约 5,000 行移植，再谈任何新功能」——**比 Phase 1+2+3 全部加起来都大**。

## 决定

**两半都要，且第二半是第一半的守门。**

### 第一半 · 方向：Swift 本体进入只修状态

- Swift 侧此后**只修 Bug、只做安全修复、只补必须的兼容**，不再新增功能模块。
- 新功能一律落在 Rust 侧（`app/src-tauri/src/`）+ 前端（`app/ui/`）。
- **但 `app/` 一天没构建成功，Swift 侧就不许动**——那条「只修 Bug」的通道保持开着，
  否则灵动岛本体在迁移期间退化而没有替代品。

### 第二半 · 执行：五道前置门，全绿才开始迁第一行

| # | 门 | 完成判据（可执行） | 状态 |
|---|---|---|---|
| M1 | **`cargo test` 能跑且有守护** | `cargo test` 在本机通过；至少覆盖 `engine` 五态转移、`provider.rs` 原子写+掩码、三种会话方言解析（**fixture 化，不依赖本机装 agent CLI**） | **部分完成**：五态回放见 `app/src-tauri/src/engine/state_tests.rs`，Claude/Codex/Cline 的合成 fixture 守护见 `session.rs` 测试；用 `cargo test --locked --manifest-path app/src-tauri/Cargo.toml` 验证。`provider.rs` 尚不存在，原子写与掩码守护未完成，M1 尚未全绿 |
| M2 | **`cargo tauri build` 本机成功** | 产出一个 `.app`；capabilities 窗口声明与 `tauri.conf.json` 的 label 一致；`placement.rs` 按 OS 真工作区；`bundle.targets` 含 macOS | **本机构建与工作区实现已达成**：`.app` / `.dmg` 已产出，窗口 label 一致；非 Windows 分支使用 Tauri `Monitor::work_area()`，几何守护见 `placement.rs`。这不表示 Windows/Linux 打包或多屏混合 DPI 已完成真机验收 |
| M3 | **Rust 侧补齐 12 个缺失模块** | 上表那 12 个模块在 Rust 侧存在且有测试；**RemoteNotify 三通道一并迁**（否则侧边栏一上线就是「能力比灵动岛少」） | **未开始**；当前工作限于已有 Rust 地基的修复与守护，未宣称缺失模块已迁入 |
| M4 | **license 与凭据口径先落定** | `LICENSE`（MIT）+ [ADR 0009](0009-credential-boundary-borrow-dont-hold.md) 凭据边界 | **已完成** |
| M5 | **有一份「两端口径对照表」** | 逐条列出哪些能力只在 island 有、哪些只在 sidebar 有、哪些两边都要一致；不一致条目写明「说清」而不是「抹平」 | **对照表已建立**：[23 号](../workbench/23-swift-rust-parity-matrix.md)；表内差异与未核实项仍须逐项处理，不代表行为已一致 |

**迁移按 M3 的模块逐个走，不整批搬**：每迁一个模块 → 补 Rust 测试 → 在 island 形态下确认行为不变 → 才迁下一个。**任何一个模块迁完无法证实行为不变，该模块回退，不带着不确定上线。**

## 为什么不反过来选

以下保留决策时的取舍理由；其中「从未构建成功」是当时的阻塞，当前状态以 M2 为准。

**A. 现在就冻结 Swift 并开始迁移**——这是原决定的前半句单独执行。不行：`app/` 从未构建成功，迁移目标是个没验证过的容器，而源端同时被冻结，中间没有退路。

**B. 分形态冻结**（island 继续 Swift、sidebar 走 Rust，两套源码并存）——是我在上一步推荐过的方案，因为它便宜一个数量级。用户否了。代价记在这里备查：两套实现长期分叉，口径漂移会反复出现（本仓已反复吃过「同一口径两处实现」的账：`dimcode.sqlite` 路径、`LIMIT 200`、复核窗口 0.8s）。**但它被否的理由同样成立**：那会让「全面转 Rust」永远不发生，最后变成两套永久的疤。

**C. 先补 Rust 的 5,000 行再冻结 Swift**——不行：那要求 Swift 侧在冻结前继续加功能，而新功能本就不该再加到即将冻结的那一侧。顺序必须是「测试框架 → 可构建 → 迁模块」，函数逐个落地。

## 明确接受的边界

- **M3 里的 12 个模块不是一次性全迁**。顺序按依赖：`engine`/`session`/`tokens`（读数地基）→ `StructuredTokenUsageIndex`/`TokenUsageMonitor`（token 明细）→ `Health`/`Resilience`/`TaskDuration`（判定增强）→ `RemoteNotify` 四文件 + `SMTPSocket`（外发）→ 导出与自检。**外发放最后**，因为它单独有 ADR 0005/0006 约束。
- **M5 的口径对照表允许「只在一边有」**。不允许的是「两边都有但算法不同」。
- **548 条 Swift 测试保留至 M3 全部完成**。它们是迁移期间唯一的行为基线；M3 完成后 Swift 侧转为归档，测试随之退役，退役本身要写进 CHANGELOG 而不是静默删除。
- **越早开始越好的部分只有一件**：M1 的 Rust 测试基建。它不依赖 M2，与迁移并行做没有风险。
