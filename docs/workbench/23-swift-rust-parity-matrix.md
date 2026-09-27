# 两端口径对照表（ADR 0010 / M5 交付物）

> 本表回答一件事：**AgentIsland 现在有两份并列实现**——Swift 端（灵动岛形态，`Sources/`）
> 与 Rust 端（跨平台迁移实现，当前只有 island 壳，`app/`）。哪些能力只在一边有、哪些两边都要一致，
> 一条一条列清楚，好让人一眼看出「缺什么、哪里正在悄悄分叉」。
>
> 依据是 [ADR 0010](../../docs/adr/0010-swift-freeze-and-rust-prerequisites.md)：**允许「只在一边有」，
> 不允许「两边都有但算法不同」。** 本表的用途就是把第二类逐条揪出来——
> 迁移 Phase 1/2/3 的优先级由它决定：先修「不一致」，再补「只在 Swift 有」。
>
> Swift 端使用原生 SwiftUI；Rust 端使用自己的 `app/ui/`，其 sidebar 尚未实现。
> 本表最初写于 v0.0.158，所列文件行数和行号保留调查时的快照；v0.0.161–162 的口径修正标在 §2.3。

## 1. 总览：规模差

| 端 | 代码量 | 文件数 | 形态 |
| :--- | ---: | ---: | :--- |
| Swift（`Sources/AgentIslandCore`） | 15,705 行 | 47 个 swift | island（macOS only） |
| Swift（`Sources/AgentIsland` App 层 + CLI） | 12,495 行 | 39 + 12 | island 的 UI 与终端 |
| Rust（`app/src-tauri/src`） | 2,930 行（调查时） | 11 个 rs（调查时） | island；sidebar 待实现 |
| Rust 前端（`app/ui`） | 897 行（调查时） | 4 个文件（调查时） | 仅 Rust island 使用 |

结论先放：**Rust 端今天是一个「能采数、能定五态、能贴边、能弹窗」的最小闭环**，
Swift 端是一个「148 个版本验证过的产品体系」。所以绝大多数条目落在「只在 Swift 有」，
这不是缺陷而是 M3 要补的账；**真正危险的是第 5、6 节那几处「两边都有、值不一样」**——
它们不会报错，只会让两边用户看到不同数字。

## 2. Agent 档案对照（25 vs 14）

Swift 侧内置 **25 个**档案（`AgentRegistry.swift:29-400` 的 `builtin` 数组字面量）。
取证口径要小心：整文件 `grep -c 'AgentProfile('` 得 **26** 是**错的**——它把
`discoverCLIProfiles()` 里动态构造的那条也数了进去；档案数只认 `builtin` 数组。
Rust 侧内置 **14 个**（[registry.rs:5-238](../../app/src-tauri/src/registry.rs)）。
Rust 另有一条 `>= 12` 的守护测试（[registry.rs:268-276](../../app/src-tauri/src/registry.rs#L268)），
**刻意不要求两边相等**——所以档案数分叉不会被测试拦住，只能靠本表盯。

### 2.1 只在 Swift 有（12 个）

`qoder` `copilot` `workbuddy` `workbuddy-ai` `antigravity` `hermes`
`continue` `chatgpt` `dsh` `ego-browser` `vibe-usage` `openviking`

（v0.0.168 起 `dim` 与 `mimocode` 已补进 Rust——两者都带 SQLite 明细库，补它们是为了让
`DimTasks` / `OpenCode` 两条方言有**真实载体**，而不是先写方言再等档案。
出处：[AgentRegistry.swift:31-343](../../Sources/AgentIslandCore/AgentRegistry.swift#L31) 各 `id:` 行；
Rust 侧 14 个 id 见 [registry.rs:5-238](../../app/src-tauri/src/registry.rs#L5)，列表仍不同。）

### 2.2 只在 Rust 有（1 个）

`vscode` —— [registry.rs:103](../../app/src-tauri/src/registry.rs#L103)。Swift 侧无此档案，
但 Swift 有 `cline`/`roo-code` 用的是 VS Code 的 `globalStorage/<ext>` 路径，功能上覆盖同一批用户。

### 2.3 两边都有（13 个）——逐字段比

| id | sessionDirs / session_dirs | 一致？ | tokenRoots / token_roots | 一致？ | CPU 阈值 | 一致？ | sessionDialect | 一致？ |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `zcode` | 双方监控 `~/.zcode/cli/rollout`；Swift 另留 checkpoints 兼容路径（v0.0.162） | 核心路径✅，兼容路径仅 Swift | S **未登记** / R rollout（Rust 独有明细） | 有意不同 | 双方 20.0 | ✅ | S `genericTail` + 任务库 / R `probe_zcode`，解析深度不同 | ⚠️ |
| `claude` | 双方均含 `~/.claude/sessions` + `~/.claude/projects`（顺序不同） | ✅ | 双方均含两个目录（v0.0.161 补齐 Rust sessions） | ✅ | 双方 20.0（v0.0.161 对齐） | ✅ | S `genericTail` / R `probe_claude` | ✅（同协议） |
| `codex` | S `~/.codex/sessions` :94 | ✅ | S 同 :97 / R 同 :63 | ✅ | S 无 / R 无 | ✅ | 同 | ✅ |
| `cursor` | S `Library/.../Cursor/...` :108 | ✅（Rust 用 `%APPDATA%`） | S 无 / R 无 | ✅ | S 20.0 / R 20.0 | ✅ | 同 | ✅ |
| `trae` | S `.../Trae CN/User/workspaceStorage` :120 | ❌（R 含 Trae + Trae CN 两条 :179） | 均无 | ✅ | S 20.0 / R 20.0 | ✅ | 同 | ✅ |
| `cline` | 同路径 :372 | ✅ | 双方均无（v0.0.161 清除 Rust 假明细入口） | ✅ | S 无 / R 无 | ✅ | S `.clineTasks` :375 / R `probe_cline` | 基本一致 |
| `roo-code` | 双方 ID 已一致（v0.0.161；旧 `roo` 禁用设置迁移） | ✅ | 双方均无（v0.0.161 清除 Rust 假明细入口） | ✅ | 均无 | ✅ | S `.clineTasks` :386 / R `probe_cline` | 基本一致 |
| `opencode` | `~/.local/share/opencode` :227 | ✅ | 双方均无（v0.0.161 清除 Rust 假明细入口）；**用量在 `session_database` 的 message 表，v0.0.168 起 Rust 也读** | ✅ | 双方 20.0（v0.0.161 对齐） | ✅ | S 无方言 / R 无 | ✅ |
| `dim` | 双方 `~/.dimcode/v2/data/sessions` :37 | ✅ | 双方同路径；**用量实际走库（`usage_ledger`），v0.0.168 起 Rust 也读** | ✅ | 双方 20.0 | ✅ | S 状态读 `dimTasks` 库 :42 / R **无会话探测** | ⚠️ 状态未对齐 |
| `mimocode` | 双方 `~/.local/share/mimocode` :253 | ✅ | 双方均无（用量在库里的 message 表，v0.0.168 起 Rust 也读） | ✅ | 双方 20.0 | ✅ | S `.openCode` 库 :259 / R 无 | ⚠️ 状态未对齐 |
| `goose` | `~/.config/goose/sessions` :394 | ✅ | S 无 / R 无 | ✅ | 均无 | ✅ | 同 | ✅ |
| `aider` | `~/.aider` :362 | ✅ | S 无 / R 无 | ✅ | 均无 | ✅ | 同 | ✅ |
| `windsurf` | S `Library/.../Windsurf/...`，R `~/.codeium/windsurf` | ⚠️ 待核实 | S 无 / R 无 | ✅ | S 20.0 / R 20.0 | ✅ | 同 | ✅ |

**仍需裁决的差异与本轮修正：**

1. **`zcode` 的路径差异已按本机实样收敛（v0.0.162）**：rollout 有 89 条
   `model_io` 记录且均含正数净用量；旧 checkpoints 目录当前不存在，CLI log 是另一种
   日志结构且更新更晚。Swift 在保留旧路径的同时加入 rollout，Rust 去掉 log；
   两端都能监控已证实的任务目录，解析深度仍不同。取证见
   [ZCode/Trae/Windsurf 路径记录](../research/2026-09-26-agent-session-paths.md)。
   Trae 多出一个 Rust 目录、Windsurf 使用不同目录；检查的标准位置无实样，继续待核实。
2. **v0.0.161 已修**：Claude/OpenCode 的 CPU 下限、Claude 的 sessions token 根目录、
   OpenCode/Cline/Roo Code 没有可靠明细时不生成零报告、Roo Code 的 ID 与旧禁用设置迁移。
   Rust 回归测试从档案、会话、设置和报告四处守护；这不代表 §4–6 其他差异已解决。

## 3. 能力模块对照（Swift Core 47 文件 vs Rust 16 rs）

「两边都有」= Rust 侧有同名职责的模块，无论实现深浅；「字段/算法差异」列在最右。

### 3.1 只在 Swift 有（约 18 个，约 5,270 行）

逐个核实过（`ls Sources/AgentIslandCore/` vs `ls app/src-tauri/src/`，Rust 侧 18 个文件里
`grep -ri` 这些关键词零命中或仅命中注释）：

| Swift 模块 | 行 | 职责（为什么不能少） |
| :--- | ---: | :--- |
| <sub>下表是 M3 立项时的现状快照；**已迁的模块在职责列标注 `✅ 已迁 vX`**，逐条口径见 §3.2。</sub> |||
| [TokenBudgetTracker.swift](../../Sources/AgentIslandCore/TokenBudgetTracker.swift) | 81 | 每日预算告警；0 = 未设，`> 0` 才启用 ✅ 已迁 v0.0.181（`budget.rs`） |
| [TokenForecastEvaluator.swift](../../Sources/AgentIslandCore/TokenForecastEvaluator.swift) | 92 | 月末预测——CLI `tokens` 的招牌输出 ✅ 已迁 v0.0.181（`forecast.rs`） |
| [TaskDurationTracker.swift](../../Sources/AgentIslandCore/TaskDurationTracker.swift) | 117 | 单次任务时长记录 ✅ 已迁 v0.0.186（`duration.rs`） |
| [RemoteNotifier.swift](../../Sources/AgentIslandCore/RemoteNotifier.swift) | 387 | 外发调度：策略→渲染→传输→记账 ✅ 全部已迁（策略 v0.0.174、渲染 v0.0.175、闸门/节流/记账 v0.0.176、重试 v0.0.178、往返传输 v0.0.177/179） |
| [RemoteTransport.swift](../../Sources/AgentIslandCore/RemoteTransport.swift) | 340 | HTTP + SMTP 通道与「发送预览」 ✅ 已迁 v0.0.175–179 |
| [SMTPSocket.swift](../../Sources/AgentIslandCore/SMTPSocket.swift) | 174 | Network.framework SMTP 会话 ✅ 已迁 v0.0.179（`smtp.rs`；Rust 用 `native-tls`） |
| `RemoteNotification.swift` 的钥匙串部分 | ~90 | `RemoteSecret.write/read/exists/delete`（Security.framework） ✅ 已迁 v0.0.180（`secret.rs`） |
| [AuditReportExporter.swift](../../Sources/AgentIslandCore/AuditReportExporter.swift) | 255 | Markdown/CSV/JSON 运维审计报告 ✅ Markdown+CSV 已迁 v0.0.182（`audit.rs`）；**Raycast 清单未迁**（它要写版本号，而 Rust 的 Cargo 版本未同步，见 §3.2） |
| [TokenReportExporter.swift](../../Sources/AgentIslandCore/TokenReportExporter.swift) | 84 | Token 账单/会话报表导出 ✅ 已迁 v0.0.185（`report.rs`） |
| [StructuredTokenUsageIndex.swift](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift) | 512 | JSONL 工具 token 明细索引（只留时间与计数） ✅ 已迁 v0.0.168–170（`sqlite.rs` + `tokens.rs`） |
| [ProcessTreeInspector.swift](../../Sources/AgentIslandCore/ProcessTreeInspector.swift) | 137 | 进程树构建（详情页「谁在吃 CPU」） ✅ 已迁 v0.0.183（`trees.rs`） |
| [LogTailReader.swift](../../Sources/AgentIslandCore/LogTailReader.swift) | 201 | 有界尾读 + 截断首行丢弃；Rust 侧 `read_tail_lines`（[session.rs:44](../../app/src-tauri/src/session.rs#L44)）有同类逻辑但只服务 session.rs，未抽公共层 |
| [LogPatternAnalyzer.swift](../../Sources/AgentIslandCore/LogPatternAnalyzer.swift) | 87 | 实时日志错误模式识别 |
| [LiveSampler.swift](../../Sources/AgentIslandCore/LiveSampler.swift) | 90 | 一次性实况采样（status/top/doctor 共用地基） |
| <sub>（另：`ActivityEngine.formatAgo` 的口径）</sub> | ~8 | 「最近活动」这一列 ✅ v0.0.184 统一（见 §3.2） |
| [Selftest.swift](../../Sources/AgentIslandCore/Selftest.swift) | 184 | 无头自检（`agentisland selftest`） ✅ 已迁 v0.0.184（`selftest.rs` + `--selftest`） |
| [InstalledAppsCache.swift](../../Sources/AgentIslandCore/InstalledAppsCache.swift) | 215 | 已安装 CLI/bundle 缓存——**「读不到 ≠ 没装」的前提** |
| [BatterySaver.swift](../../Sources/AgentIslandCore/BatterySaver.swift) | 46 | 电源自适应降频 |
| [AgentCleaner.swift](../../Sources/AgentIslandCore/AgentCleaner.swift) | 307 | 一键清理异常 Agent（CLI `clean`） |
| [AgentLogStreamer.swift](../../Sources/AgentIslandCore/AgentLogStreamer.swift) + [AgentActionInspector.swift](../../Sources/AgentIslandCore/AgentActionInspector.swift) + [AgentSessionInspector.swift](../../Sources/AgentIslandCore/AgentSessionInspector.swift) + [SelfReport.swift](../../Sources/AgentIslandCore/SelfReport.swift) + [ProcessMonitor.swift](../../Sources/AgentIslandCore/ProcessMonitor.swift) + [FileMonitor.swift](../../Sources/AgentIslandCore/FileMonitor.swift) + [ReadonlyDB.swift](../../Sources/AgentIslandCore/ReadonlyDB.swift) + [ShellQuoting.swift](../../Sources/AgentIslandCore/ShellQuoting.swift) + [URLSchemeParser.swift](../../Sources/AgentIslandCore/URLSchemeParser.swift) + [WireText.swift](../../Sources/AgentIslandCore/WireText.swift) + [Probe.swift](../../Sources/AgentIslandCore/Probe.swift) + [ProbeFailureLog.swift](../../Sources/AgentIslandCore/ProbeFailureLog.swift) + [AppLog.swift](../../Sources/AgentIslandCore/AppLog.swift) + [BatterySaver](../../Sources/AgentIslandCore/BatterySaver.swift) | 5,000+ | 见 3.2：这些是「Rust 有薄版、Swift 有厚版」的一类 |

> 说明：最后一行把 Swift 里 14 个「Rust 有薄对应」的厚实现列在一起，避免把它们误读成
> 「完全缺失」。真正**零对应**的是 3.1 表内那 22 行（加上 AgentSessio/… 里的厚实现差异）。

### 3.2 两边都有（7 个）——实现深浅不同

| 职责 | Swift | Rust | 差异要点 |
| :--- | :--- | :--- | :--- |
| 档案表 | [AgentRegistry.swift](../../Sources/AgentIslandCore/AgentRegistry.swift) 604 行：25 档案 + 自动发现 CLI + 用户自定义（UserDefaults） | [registry.rs](../../app/src-tauri/src/registry.rs) 370 行：14 档案硬编码 | Rust **无自动发现、无自定义档案**；`builtin()` 每次调用重读 home dir。**v0.0.168 起两边都有 `sessionDatabase`/`session_database`**（路径 + 方言；Rust 声明了 dim/zcode/opencode/mimocode 四条） |
| 五态引擎 | [ActivityEngine.swift](../../Sources/AgentIslandCore/ActivityEngine.swift) 1492 行 | [engine.rs](../../app/src-tauri/src/engine.rs) 678 行 | Swift 有电源分级采样、`visibleSnapshots`/`ringShelfSnapshots` 可见口径、冲突双显；Rust 有 demo 模式与 webhook 事件源。**核心双信号与 CPU 熔断一致**，见 §4 |
| 事件投递 | `publish(ev)` 逐条进事件流，前端按需消费 | `push_event` → 队首 `latest_event` + 待发队列（v0.0.173 起，上限 64），`clear_latest_event` 确认一条推下一条；`EngineState.pending_events` 暴露剩余条数 | ✅ **v0.0.173 起不再丢事件**。此前是覆盖式赋值：同一拍里两条告警只活一条（卡死与内存同时到点必丢一条）。**形态仍不同**：Swift 是事件流，Rust 是「一次一条 + 确认」，所以 Rust 的高频事件会排队等用户点掉 |
| 进程监控 | [ProcessMonitor.swift](../../Sources/AgentIslandCore/ProcessMonitor.swift) 700 行：libproc 直读进程表 | [procmon.rs](../../app/src-tauri/src/procmon.rs) 129 行：`sysinfo` 0.33 | Swift 直读 `/dev` 级接口 + 自定义匹配（pathContains/pathExcludes/hostBundleIDs）；Rust 的 `AgentProfile` **没有 pathContains / hostBundleIDs 字段**——见 §2.3 与 §6 |
| 文件活动 | [FileMonitor.swift](../../Sources/AgentIslandCore/FileMonitor.swift) 634 行：后台递归 + O(1) 缓存 + 限深 | [filemon.rs](../../app/src-tauri/src/filemon.rs) 138 行：同步遍历 | Rust 无后台队列、无深度缓存分层；`max_depth 4` + 扩展名白名单（[filemon.rs:105](../../app/src-tauri/src/filemon.rs#L105)） |
| 会话解析 | [AgentSessionInspector.swift](../../Sources/AgentIslandCore/AgentSessionInspector.swift) 1687 行：5 种方言 + 专有协议 | [session.rs](../../app/src-tauri/src/session.rs) 414 行：4 个 id 专属 `probe_*` | 分派方式不同：Swift 按**档案声明的方言**，Rust 按 **agent id**（[session.rs:35-40](../../app/src-tauri/src/session.rs#L35)）。ADR 0010 要求「新增复用既有格式的 Agent 只改档案」，Rust 今天做不到 |
| Token 用量 | [TokenUsageMonitor.swift](../../Sources/AgentIslandCore/TokenUsageMonitor.swift) 1295 行 + [StructuredTokenUsageIndex.swift](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift) 512 行 + [ReadonlyDB.swift](../../Sources/AgentIslandCore/ReadonlyDB.swift) 159 行：SQLite + JSONL 双源 | [tokens.rs](../../app/src-tauri/src/tokens.rs) 1244 行 + [sqlite.rs](../../app/src-tauri/src/sqlite.rs) 113 行：JSONL + **两类 SQLite 方言** | ⚠️ **已对齐**：v0.0.168 起也读 SQLite（OpenCode 的 `message.data` 与 DimAgent 的 `usage_ledger`，净口径逐条照搬）；v0.0.169 起 JSONL 净口径、记录形状与响应级去重对齐；v0.0.170 起**保留口径对齐**（70 天窗口 / 折入保和 / 单文件 20,000 条上限 / 戳判定 / 只承认完整行的续读游标）。**仍缺**：① `statusIndex` 不含 token（设计如此）② Rust 侧没有 workbuddy/workbuddy-ai 档案，那两家的 JSONL 用量读不到 |
| Token 估价 | [TokenCostEstimator.swift](../../Sources/AgentIslandCore/TokenCostEstimator.swift) 113 行：37 条官方费率 + 3:1 混合加权；估不出来返回 nil | [cost.rs](../../app/src-tauri/src/cost.rs)：同表、同顺序、同算法、同边界（估不出来就不报） | ✅ **已对齐（v0.0.166）**：Rust 原 `price_lookup` 的 5 档粗分档已删。`cost::tests::swift_table_parity` 在 Rust 测试里**直接解析 Swift 源表**逐值逐序比对，改一个数就红（已双向验证）。成本是估的还是记录的由 `cost_estimated` 显式标出 |
| 本地 HTTP 入口 | [LocalEventHTTP.swift](../../Sources/AgentIslandCore/LocalEventHTTP.swift) 179 行 + `Sources/AgentIsland/LocalEventServer.swift` | [webhook.rs](../../app/src-tauri/src/webhook.rs) 152 行 | 端口/路由一致（41999，`/notify` `/event` `/session`）；**Token 落盘位置不同**：Swift 与引擎同进程管理，Rust 写 `config_dir()/report.token`（[webhook.rs:16-18](../../app/src-tauri/src/webhook.rs#L16)）。ADR 0009 口径需复核 |

### 3.3 只在 Rust 有（1 个）

[placement.rs](../../app/src-tauri/src/placement.rs) 处理 Win32 工作区、DPI 换算与几何放置。
**更新于 v0.0.158**：macOS/Linux 分支已改用 Tauri `Monitor::work_area()`，不再返回
1440×900 的假工作区；当前几何守护验证了边界钳制，多显示器真机验收仍待完成。
Swift 侧对应能力在 `Sources/AgentIsland/IslandPanelPositioning.swift`（按 NSScreen 真工作区）。

## 4. 五态与判定口径

| 项 | Swift | Rust | 判定 |
| :--- | :--- | :--- | :--- |
| 五态枚举 | `ActivityLevel` ∈ offline/idle/completed/working/attention（[Models.swift:44-72](../../Sources/AgentIslandCore/Models.swift#L44)） | `ActivityLevel` 同五名（[models.rs:14-40](../../app/src-tauri/src/models.rs#L14)），测试钉住 serde 名（[models.rs:test](../../app/src-tauri/src/models.rs)） | ✅ **必须一致，已一致** |
| 健康度与卡死判定 | [AgentHealthEvaluator.swift](../../Sources/AgentIslandCore/AgentHealthEvaluator.swift) 144 行 + `ActivityEngine` 里的 `isHung`（三态、资格来自 `observedRunningSince`） | [health.rs](../../app/src-tauri/src/health.rs)：同扣分表、同 85/70/50 分级、同「判不出的维度不许宣布健康」降级规则；`is_hung` 作为快照字段（[models.rs](../../app/src-tauri/src/models.rs)），界面按分数上屏 | ✅ **v0.0.171 起一致**（含文案逐字相同）。两处有意不同：① Swift 的 `HealthGrade.icon` 是五个 SF Symbol 名，网页渲染不了、**刻意不搬**（见 `health.rs` 注释）② Swift 在详情页展示，Rust 在列表行上只在非「健康」时显示分数徽标 |
| Token 预算告警 | [TokenBudgetTracker.swift](../../Sources/AgentIslandCore/TokenBudgetTracker.swift) 81 行：80% 预警 / 100% 超额、**只有跨级才报**、<75% 才重新武装 | [budget.rs](../../app/src-tauri/src/budget.rs)：同阈值、同滞回、同文案（含 `compact` 后的数字）；引擎在总量算完后评估，跨级时推 `system` /「Token 预算」/ `costSpike` 事件（预警与超额分开写） | ✅ **v0.0.181 起一致**。两处有意差异：① 负用量钳成 0（Swift 原样打印 `-1`，但那是统计回绕，显示出来等于说谎）② 关掉告警开关时仍给状态（Swift 同分支），但不推事件 |
| 审计报告导出 | [AuditReportExporter.swift](../../Sources/AgentIslandCore/AuditReportExporter.swift) 255 行：Markdown / CSV（+ Raycast 清单） | [audit.rs](../../app/src-tauri/src/audit.rs)：同表头同列序、同 `cell()` 转义（`|` 与换行）、同 `—`（没查）与 `0`（查了确实是零）之分、同「跨源总量与逐条相加不等时把差额说出来」；两条命令 `audit_report_markdown` / `audit_report_csv` 返回 `{filename, content}` | ⚠️ **v0.0.182 起 Markdown/CSV 一致**。未迁：① **Raycast 清单**（要写版本号，而 `Cargo.toml` 是 `0.1.0`、与 App 版本未同步——先得有「第四个版本位」的同步规则）② ~~Swift 的状态列带 `AgentProvenance` 后缀~~ ⇒ **v0.0.195 起已对齐**（见下表最后一行） |
| Token 消费报表导出 | [TokenReportExporter.swift](../../Sources/AgentIslandCore/TokenReportExporter.swift) 84 行：Markdown / CSV（+ 复制到剪贴板） | [report.rs](../../app/src-tauri/src/report.rs)：同表头同列序、同 BOM、同「费用格永远写钱数、`—` 留给状态列」、同「累计那一行用 `cost()` 所以零是**空**」；区间合计复用 24h 那一档的解析（`tokens::range_totals`，同一个 cutoff 参数）；接 `token_report_markdown` / `token_report_csv` | ✅ **v0.0.185 起一致**。差异：① Rust 的 `Timeline` **不带 `points`**（图表数据由 `get_report` 的 `hourly30d` 单一来源提供）② 剪贴板由界面做（Rust 侧返回正文，`NSPasteboard` 属 UI 层）。**发现**：汇总行那句「(参考估算)」两边都是死分支——那一行不带 modelId，而 `isEstimated` 只在那条路径上为真；真正可见的估价信号是行内的 `~` |
| 任务效能统计 | [TaskDurationTracker.swift](../../Sources/AgentIslandCore/TaskDurationTracker.swift) 117 行：按 Agent 记时长、每 Agent 上限 100 条、默认 24h 窗口、两处排版文案 | [duration.rs](../../app/src-tauri/src/duration.rs)：同上限（丢**最早**的）、同窗口边界（含边界）、同 `—`（无样本）与 `3分12秒/次` 文案；统计进快照的 `workStats` 字段（界面随状态轮询拿到，不必按需查） | ✅ **v0.0.186 起一致**。差异：① **不搬 `prune`**——它在 Swift 侧也从未被调用，而上限已经管住内存② 不加锁（引擎本身在 `Mutex` 后面，多一层锁会让人以为可以绕过引擎共享它）。**顺带对齐**：Swift 的完成门槛是「≥3.5 秒才算一次任务」，Rust 此前没有，于是 0.2 秒的抖动也会记一笔并推「任务完成 (0秒)」|
| 区间用量合计 | `TokenUsageMonitor` 按范围查 SQLite | [tokens.rs](../../app/src-tauri/src/tokens.rs) 的 `range_totals(profile, range_ms, now)`：复用 `parse_file` 的 cutoff 聚合，24h / 7d / 30d 同一套口径、缓存照旧命中 | ✅ **v0.0.185 起一致**（成本随区间缩放）。差异：Rust 的 `now` 由调用方给（可离线断言，不必等真实时钟）|
| 自报与出处（provenance） | [SelfReport.swift](../../Sources/AgentIslandCore/SelfReport.swift) 857 行 + [ActivityEngine.swift](../../Sources/AgentIslandCore/ActivityEngine.swift) 的 `believableSelfReport` 与 `AgentProvenance`（[Models.swift](../../Sources/AgentIslandCore/Models.swift) 两种说法：徽标短标签 / 报表完整说法） | [selfreport.rs](../../app/src-tauri/src/selfreport.rs)：同「TTL 钳 [15,600] 秒、缺省 90」，同「过期只盖戳不删」、同「文本截断 200 字」、同「pid 宁可拒绝也不静默截断」，同 `resolve` 三条规则（无自报⇒观测/推断、离线留 `None`；与强语义冲突⇒`Conflict` 且**显示按观测走**；与弱信号冲突⇒**采信自报**）；`/session` 走 `?agent=&session=` + `X-AgentIsland-Token`，回 `{bound, expiresAt}`；快照带 `provenance` 与**拼好的** `provenanceSuffix`，界面与审计报表都用它 | ✅ **v0.0.195 起对齐**（含端到端实样：POST 自报 ⇒ 界面 `工作中 · 自报 · <detail>`）。**差异**：① Rust 按 **agent** 存一条（后报覆盖先报）；Swift 的记录可按 `(agentID, sessionID)` 查——「同一 agent 两个会话同时自报时显示哪一条」没有逐行比对 ② 撤销的响应形状 Rust 是 `{revoked:bool}`，Swift 的未核 ③ Rust 的 `/session` 拒绝理由名与 Swift 的 `SelfReportRejection` 逐条对齐了四种（malformed/unknownState/badPid/unknownAgent/pidMismatch），但 `noToken` 那条 Rust 回 **401**、Swift 回 200——**两者都只回 noToken**（防注册表被当字典查）这一点一致 |
| 本地入口端口 | [SelfReport.swift](../../Sources/AgentIslandCore/SelfReport.swift) 的 `defaultPort = 41999`（Core 里唯一定义处）+ [LocalEventServer.swift](../../Sources/AgentIsland/LocalEventServer.swift) | [webhook.rs](../../app/src-tauri/src/webhook.rs) 的 `DEFAULT_EVENT_PORT = 42000`（可被 `AGENTISLAND_EVENT_PORT` 覆盖；绑定结果与端口写进日志） | ⚠️ **有意不同**（[ADR 0013](../adr/0013-local-event-port-during-migration.md)）：交付物保持 41999 这个对外承诺，Rust 端另用一端口以求并存；协议与令牌文件相同。Swift 退场后改回 41999。**实测**：两应用同时跑时各自答话、互不打扰；同一份「Swift 格式」请求两边都回 `{"bound":true,"expiresAt":…}`。**形状差异**：`expiresAt` Swift 是小数秒（`…695.9989`）、Rust 是整数秒 |
| 派生进程树 | [ProcessTreeInspector.swift](../../Sources/AgentIslandCore/ProcessTreeInspector.swift) 137 行：给定根 PID 与进程表快照递归建树 | [trees.rs](../../app/src-tauri/src/trees.rs)：同「按 ppid 分组建索引 + 全局已访问集合防环 + 根不算自己的后代」；接 `agent_process_tree` 命令；`procmon::table()` 提供整张进程表 | ✅ **v0.0.183 起一致**（含 `0%`/`512M`/空名回落 `unknown` 等文案）。两处**有意差异**：① Rust **全程不递归**（Swift 递归；深层嵌套会爆栈，Rust 侧实测 5,000 层链即 SIGABRT）② CPU 用 `Option`（Swift 的 `cpuPercent` 恒为 `Double`，无差分窗口时给 0）——Rust 把「这一拍没有读数」与「0%」分开，并额外给出 `cpu_measured_count`（部分无读数时合计偏低） |
| 无头自检 | [Selftest.swift](../../Sources/AgentIslandCore/Selftest.swift) 184 行：`agentisland selftest` 逐条断言核心判定 | [selftest.rs](../../app/src-tauri/src/selftest.rs)：22 条检查（五态判定 / 活动文案 / 注册表 / 进程匹配的命中与**不误报** / 文件活动 / 卡死三态），接 `run_selftest` 命令与**无头入口** `agentisland --selftest`（退出码 0/1，与 CLI 同约定） | ✅ **v0.0.184 起一致**。差异：Rust 的自检**只碰自己的临时目录**、不读用户的会话目录（Swift 的同类检查也只写临时目录），所以任何机器上都应全绿——有一条红就是包的问题，而不是「这台机器刚好有什么」|
| 「最近活动」文案 | `ActivityEngine.formatAgo`：`刚刚`（<5s）/ `30s 前` / `2m 前` / `2h 前`；`nil` → `—` | [filemon.rs](../../app/src-tauri/src/filemon.rs) 的 `time_ago_text(Option<f64>)`：同阈值、同后缀、同 `—` | ✅ **v0.0.184 统一**。此前 Rust 写「30秒前 / 2分钟前 / 3小时前」并把阈值放在 10 秒、还多一档「天」——**同一列数据在岛上说 `2m 前`、在侧边栏说 `2分钟前`**（与 `compact` 那次同一类问题）|
| 月末预估 | [TokenForecastEvaluator.swift](../../Sources/AgentIslandCore/TokenForecastEvaluator.swift) 92 行：以 24h 为基准日外推到当月 | [forecast.rs](../../app/src-tauri/src/forecast.rs)：同外推、同四段文案、同 `SafeNumber` 的饱和乘与金额上限；接 `token_forecast` 命令 | ✅ **v0.0.181 起一致**（含闰年/月长按本地日历）。一处发现：Swift 的 `else`（「预计月末消耗 …」）在「当天用量 > 日均预算」前提下**数学上不可达**，Rust 保留结构但有不变量用例证明它不会被走到 |
| token 数缩写（`compact`） | `TokenUsage.compact`（全仓唯一一份，卡片/悬停/热力图/详情页/CLI 都走它） | [tokens.rs](../../app/src-tauri/src/tokens.rs) 的 `compact`：同阈值（1e9 `B` / 1e6 `M` / **1e4** `k`）、同小数位（保留 `12.00M`） | ✅ **v0.0.181 统一**。此前 Rust 另有一份 `engine::compact`：1000 就打 `k`、十亿用 `G`、还剥尾零，而它正用在一句 Swift 用参考函数构造的告警里——**同功能两份实现**是这一轮修掉的真问题 |
| 异常驻留 / 死锁持续守护 | [AgentResilienceGuard.swift](../../Sources/AgentIslandCore/AgentResilienceGuard.swift) 112 行：卡死 180s / 内存 300s 门槛、600s 冷却，`autoAnomaliesAlertEnabled` 控制 | [resilience.rs](../../app/src-tauri/src/resilience.rs)：同三个数、同「进程消失即作废状态」「条件消失即清冷却」两条规则；事件走 `attention`，文案逐字相同 | ✅ **v0.0.172 起一致**。Rust 侧新增设置项 `auto_anomalies_alert`（默认 `true`，与 Swift 默认一致）。差异：Swift 的事件模型带 `duration`/`pid` 两列，Rust 的 `AgentTaskEvent` 没有，时长放在 `detail` 里 |
| 外发策略与配置校验 | [RemoteNotification.swift](../../Sources/AgentIslandCore/RemoteNotification.swift) 454 行：`RemoteNotifyPolicy`（节流/静默/在场/类型开关）、`RemoteChannelKind` 的三条校验、逐字段容错的解码 | [remote.rs](../../app/src-tauri/src/remote.rs)：同默认值、同区间（15–3600 / 30–3600）、同归一化（坏时刻退化成**不静默**）、同校验文案；容错解码用 `serde_json::Value` 手写，做到「坏一个字段只作废那一个」 | ✅ **v0.0.180 起策略层与密钥一致**。密钥走 [secret.rs](../../app/src-tauri/src/secret.rs)：同一 service、同一条目名（`remote.<kind>`，两处共用一个函数）、同样「只在策略闸门放行后才读」、同样「存在性查询不带 `kSecReturnData`」。Rust 侧尚未接 macOS 在场信号层，`away` 今天一律 fail-open 判成「已离开」 |
| 外发渲染与「发送预览」 | [RemoteNotifier.swift](../../Sources/AgentIslandCore/RemoteNotifier.swift) 的 `render`/`renderRequest`/`clamp` + [WireText.swift](../../Sources/AgentIslandCore/WireText.swift) + `RenderedRequest.maskedPreview` | [render.rs](../../app/src-tauri/src/render.rs)：同最小内容口径（动作详情只有 `includeActionDetail` 才进正文）、同四类**分开**的转义（头值 ttext 集 / 查询串 unreserved 集 / 表单 `+` / JSON 转义）、同 4096 字节上限（保住首行、标题编码后再截 255）、同掩码规则（`?key=` 与「16+ 位字母数字段」） | ⚠️ **v0.0.175 起渲染一致**，且由 `remote_preview` 命令真消费（Swift 侧是设置页的「发送预览」）。未迁：重试（见下一行）。差异：Rust 侧取不到密钥，预览里 `{key}` 一律显示掩码（Swift 会读真密钥再掩码） |
| 外发闸门 / 节流 / 记账 | [RemoteNotifier.swift](../../Sources/AgentIslandCore/RemoteNotifier.swift) 的 `attempt`（五道闸）/`claimThrottle`/`releaseThrottle`/`record` + `OutboundOutcome`/`OutboundAttempt` | [notifier.rs](../../app/src-tauri/src/notifier.rs)：同闸门顺序（总开关 → 类型开关 → 静默/在场 → **配置检查** → 节流）、同文案、同「失败即撤占位」、同 20 条有界账本 | ✅ **v0.0.178 起一致**：闸门顺序、文案、「失败即撤占位」、20 条有界账本，以及**失败后重试一次**（5 秒、对端明确拒绝不重试、发送测试不重试）都对齐。Rust 用**工作线程**达到 Swift async Task 的效果（`dispatch` 立刻返回，账本由工作线程回填），并有一条并发用例钉住「重试在途时同一键挤不进来」 |
| 外发传输 | [RemoteTransport.swift](../../Sources/AgentIslandCore/RemoteTransport.swift) 340 行 + [SMTPSocket.swift](../../Sources/AgentIslandCore/SMTPSocket.swift) 174 行：URLSession + Network.framework | [transport.rs](../../app/src-tauri/src/transport.rs)：超时 10s、拒绝重定向、状态码可重试判据与 Swift **逐条同值**；用 std TCP 实现 | ✅ **v0.0.179 起三条路都通了**：`https` 用 `native-tls`（macOS = Security.framework，与 Swift 同一套信任栈），本地自签证书做过**离线端到端验证**（含「不可信证书必须失败」），并逐个地址试连；**SMTP over 465 隐式 TLS 会话已接**（[smtp.rs](../../app/src-tauri/src/smtp.rs)：EHLO 多行续行、AUTH LOGIN、点号加倍、RFC 2047 主题、RFC 822 Date、`smtp_line` 折行、失败路径也关连接——全用脚本化会话离线逐条断言）。25/587 的 STARTTLS 与 Swift 同样**在连接之前**就被拒绝并说明。踩过的四个 TLS 坑见 [macOS 上用 native-tls 的记录](../research/2026-09-27-macos-tls-for-rust-outbound.md) |
| 状态序 | `<` 定义：offline(0)<idle(1)<completed(2)<working(3)<attention(4)（[Models.swift:66-72](../../Sources/AgentIslandCore/Models.swift#L66)） | `derive(Ord)` 声明顺序同 Swift（[models.rs](../../app/src-tauri/src/models.rs)） | ✅ v0.0.158 已纠正；两边均以 attention 为最高优先级 |
| 中文 label | 离线/待机/已完成/工作中/待确认 :52-58 | 同 :26-32 | ✅ |
| 双信号判定 | `working = 进程在 && (workingWindow 内有写入 \|\| CPU >= max(cpuFloor, cpuThreshold))`（[ActivityEngine.swift:8-12](../../Sources/AgentIslandCore/ActivityEngine.swift#L8)、:659） | 同一公式（[engine.rs:254-278](../../app/src-tauri/src/engine.rs#L254)） | ✅ 算法一致；但 `workingWindow=60` 在 Rust 是**硬编码字面量**（[engine.rs:82](../../app/src-tauri/src/engine.rs#L82)），Swift 来自可钳制的 `EngineConfig.workingWindow` |
| attention/completed 强语义 | 方言分派 + 有界尾读 + 指纹去重 | id 分派 `probe_claude/codex/cline/zcode` + 指纹去重；Codex 调用配对与完成标记有合成 fixture 守护 | ⚠️ 部分协议行为有守护，方言分派与来源覆盖仍不同，不能宣称整体一致 |
| CPU 熔断 | `runawayCpuAlert` + 70% / 5 分钟 | 三个字段同名同默认同区间，v0.0.200 起可配 | ✅ **v0.0.200 起一致**。⚠️ `is_hung` 已改为显式收时长参数——否则「告警响了、健康度还说不卡死」而两边都不报错 |
| Token 暴涨告警 | 每分钟净增量 > `tokenAlertThreshold`（200k） | 同一口径，注释明说「与 macOS 端口径一致」（[engine.rs:298](../../app/src-tauri/src/engine.rs#L298)） | ✅ |
| 降频 | 有活动 `sampleInterval` / 闲置 `idleSampleInterval`（5s）/ 全离线 60s / 节电三档（[ActivityEngine.swift:1395-1404](../../Sources/AgentIslandCore/ActivityEngine.swift#L1395)） | 有活动 `sample_interval`；闲置 `×2.5` 夹在 2.0–12.5s（[main.rs:302-310](../../app/src-tauri/src/main.rs#L302)） | ⚠️ 公式不同（`×2.5` vs 独立 `idleSampleInterval` 字段）；无节电三档。**需说清**：两侧耗电量与「岛多久变灰」不同 |
| 可见口径 | `visibleSnapshots`（只显在线）+ `ringShelfSnapshots`（[ActivityEngine.swift:1472-1483](../../Sources/AgentIslandCore/ActivityEngine.swift#L1472)） | `state()` 直接给全部 snapshot，前端可自行过滤（未逐行比对） | ⚠️ 未逐行比对 |

### 4.1 「读不到 ≠ 零」：Rust 已接五类判定，证据来源仍未对齐

Swift 的 [AgentObservability.evaluate](../../Sources/AgentIslandCore/AgentObservability.swift#L46)
返回**五类结论**（:14-25）：`observed` / `blindSessionSource`（源读不到）/
`noLocalData`（读不到会话与用量）/ `sourceNotWired`（档案未登记明细源）/
`notInstalled`（未安装且进程不在）。Swift 还依赖两项 Rust 侧尚未完整采集的证据：

- `snapshot.installed`（安装判定，来自 `InstalledAppsCache`）——[Models.swift:421](../../Sources/AgentIslandCore/Models.swift#L421)
- `snapshot.sessionProbeHealth`（源读到的失败原因）——[Models.swift:439](../../Sources/AgentIslandCore/Models.swift#L439)

Rust 的 [observability.rs](../../app/src-tauri/src/observability.rs) 现将五类代码和依据写入
`AgentSnapshot.observability`，前端对缺乏证据的在线待机给出诊断标签，并避免顶栏声称
「全部 Agent 待机」。但这只是**保守的第一层**，不能视为与 Swift 等价：

- Rust 尚无 ~~`InstalledAppsCache`~~ ✅ **v0.0.197 已补**（`installed.rs`，见上）。
  进程不在时安装状态不再一律传 `None`：探测能判定就判定，判不了才给 `None`。
- `blindSessionSource` 目前只证明已登记会话根目录的元数据/列举失败或路径不是目录；
  深层文件读取与解析失败仍可能被吞掉，尚无 Swift 的 `SessionProbeHealth` 原因链。
- Rust 没有 `activeSessions`，暂以十分钟内有会话文件写入作活动证据。文件新鲜度
  与活跃会话不是同一个量；token 总量大于零也只能说明曾有用量。

因此 M3 后续仍需补齐探测健康和活跃会话来源，再做两端行为对照。

**v0.0.197 结掉其中一条（安装判定），另两条仍在**：

- ✅ **安装判定已接**。新增 [installed.rs](../../app/src-tauri/src/installed.rs)（对齐 Swift
  `InstalledAppsCache`），快照带 `installed: Option<bool>`，引擎按 5 分钟 TTL 刷新。
  为此给档案补了 `bundle_ids`（14 个内置档案逐条对齐 Swift 声明；`vscode` 为 Rust 独有档案）。
  **关键口径**：`None` = 没核实，不是「没装」——档案登记了 bundle id 而这趟扫描没有否定能力时
  （Windows 无应用包概念、`Info.plist` 是 binary plist）给 `None` 而非 `Some(false)`。
  `notInstalled` 由此从「生产不可达」变成真结论。
  **未做**：Windows/Linux 未做真机验收（本机只能验 macOS 路径）；`Info.plist` 只认 XML 形状。
- ❌ **`SessionProbeHealth` 原因链仍缺**：`source_unreadable` 仍是 `bool`，
  只覆盖元数据/列举失败与「路径不是目录」，深层文件读取与解析失败仍会被吞掉。
  要迁它得先改 `session.rs` 的探测返回结构，不是加字段的量级。
- ❌ **`activeSessions` 仍缺**：暂以十分钟内有会话文件写入作活动证据。
  文件新鲜度与活跃会话不是同一个量；token 总量大于零也只能说明曾有用量。
- ⚠️ **自报归因已对齐（v0.0.197）**：`Evidence` 带 `provenance`，自报写
  「状态由带令牌、TTL 内的自报确认」、观测写「本轮读到了会话强语义（<态>）」，
  与 Swift 逐字相同。此前 Rust 一律写「本轮有活动或任务状态信号」，
  而那一拍的状态可能完全来自自报——等于拿别人的证据给自己的结论背书。

同理，CPU 的**两态**口径（有值=测到，nil=没测，[Models.swift:415-418](../../Sources/AgentIslandCore/Models.swift#L415)）
在 Rust 是 `Option<f64>` 且有 `None` 分支（[models.rs](../../app/src-tauri/src/models.rs)）——形式在，但
前端是否把 `null` 印成 `0.0%` **未逐行比对**。

## 5. CLI 能力对照

Swift CLI 有 **12 个子命令**（[main.swift:19-73](../../Sources/AgentIslandCLI/main.swift#L19)）：
`status`(默认) / `top` / `tokens` / `state` / `doctor` / `check` / `clean` / `selftest` /
`open` / `notify` / `report` / `raycast`，共 11 个 `Command` 文件（`status` 不单独成文件，是默认路径）。

**Rust 侧无 CLI。** [main.rs:323](../../app/src-tauri/src/main.rs#L323) 的 `main()` 只建 Tauri 应用
（`tauri::Builder`），`std::env::args()` 仅用于识别 `--demo` / `--expand` / `--route=`
（[main.rs:38-48](../../app/src-tauri/src/main.rs#L38)）。Cargo 未声明 `[[bin]]` 之外的第二二进制
（[Cargo.toml](../../app/src-tauri/Cargo.toml) 无 `[[bin]]` 节）。

| CLI 子命令 | 依赖的 Swift 模块 | Rust 侧状态 |
| :--- | :--- | :--- |
| `status` / `top` | LiveSampler、ProcessMonitor、CLIOutput、CLIModels | ❌ 无 |
| `tokens` | TokenUsageMonitor、TokenForecastEvaluator、TokenCostEstimator、DailyBudget | ❌ 无（Rust 有 `get_report` 供前端，无终端输出） |
| `state` | AgentState（读 App 进程内状态，含自报/冲突） | ❌ 无；且后端 `AgentSnapshot` 无 provenance 字段 |
| `doctor` | AgentObservability、AgentHealthEvaluator、SessionProbeHealth | ❌ 无 CLI；Rust 仅有 §4.1 的局部可观测性判定，**健康度已有（v0.0.171）但探测原因链仍缺** |
| `check` / `clean` | AgentResilienceGuard、AgentCleaner、ProcessTreeInspector | ⚠️ 无 CLI；持续守护判定已有（v0.0.172 → `resilience.rs`），**清理动作与进程树仍未迁** |
| `selftest` | Selftest（无头假数据断言） | ❌ 无同名 CLI 子命令；Rust 有 `cargo test --locked` 守护五态和会话解析 |
| `open` / `notify` | URLSchemeParser、LocalEventHTTP、SelfReport | ⚠️ 部分：webhook 服务端在（[webhook.rs](../../app/src-tauri/src/webhook.rs)），客户端/深链解析无 |
| `report` / `raycast` | AuditReportExporter、TokenReportExporter、AppVersion | ❌ 无 |

**排优先级含义：M3 的 12 个模块迁完后，Rust 侧仍没有 CLI。** 若希望 sidebar 形态可脚本化
（Raycast / CI / cron），CLI 是独立的一块工作量，不在当前 M3 清单内。

## 6. 格式与写盘口径（最容易悄悄分叉的一节）

### 6.1 存储介质与路径

| | Swift | Rust |
| :--- | :--- | :--- |
| 介质 | macOS `UserDefaults`（[SettingsStore.swift:20-53](../../Sources/AgentIslandCore/SettingsStore.swift#L20)） | JSON 文件 `~/Library/Application Support/AgentIsland/settings.json`（[settings.rs:41-45](../../app/src-tauri/src/settings.rs#L41)） |
| 读写 | `defaults.set`，键名集中在 `SettingKey` | `serde_json` 整份序列化，`#[serde(default)]` 兜缺字段 |
| 损坏处理 | `enabledAgents` 损坏→只读降级 + 备份键 `enabledAgents.corruptBackup`（:247-259） | 解析失败→整份回 `Settings::default()`（[settings.rs:50-55](../../app/src-tauri/src/settings.rs#L50)） |
| 归一化 | `EngineConfig.normalized()` 唯一入口，NaN 先归位再钳制（[Models.swift:789-809](../../Sources/AgentIslandCore/Models.swift#L789)） | `Settings::normalized()` 只钳 4 个数值字段（[settings.rs:67-79](../../app/src-tauri/src/settings.rs#L67)） |

> ⚠️ **语义差异**：Swift 对「坏设置」是**部分保留**（只坏的那个键降级，其余照旧），
> Rust 是**整体回厂**。注释里 Rust 侧自称「与 macOS EngineConfig.normalized() 同规则」（[settings.rs:66](../../app/src-tauri/src/settings.rs#L66)），
> 但实际范围（4 字段 vs 9 字段 + NaN 处理 + sample≤idle 钳平）**不一致**。已核实为差异，非误读。

### 6.2 逐字段对照

| 语义 | Swift 键 | Rust 字段 | 一致？ | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| 外观 | `islandAppearance`（system/light/dark，:28） | `appearance` | ✅ 值域同 | **键名不同** |
| 贴边 | `dockEdge`（right/top/bottom/left，:34） | `dock_edge` | ✅ 值域同 | Rust 未知值兜底 `Top`（[models.rs:parse](../../app/src-tauri/src/models.rs)） |
| 贴边锚点 | `dockAnchorX` + `dockAnchorY` 两个键（:35-36，水平边用 X、垂直边用 Y） | `dock_anchor: f64` 单值 | ❌ | 两个坐标 vs 一个标量 |
| 采样间隔（有活动） | `sampleInterval`（EngineConfig 默认 2.0） | `sample_interval` 默认 2.0 | ✅ | |
| 采样间隔（闲置） | `idleSampleInterval` 默认 **5.0** | `idle_sample_interval` 默认 **5.0** | ✅ **v0.0.200 起一致**。此前是 `sample_interval × 2.5`——另一个公式，于是两侧耗电量与「岛多久变灰」对不上 |
| 工作判定窗口 | `workingWindow` 默认 60，区间 10…300 | `working_window` 默认 60，同区间 | ✅ **v0.0.200 起可配** | |
| 活跃会话窗口 | `activeSessionWindow` 默认 600，区间 60…3600 | `active_session_window` 默认 600，同区间 | ⚠️ **v0.0.200 字段已补**，但**活跃会话数本身仍未迁**（`activeSessions` 仍是「十分钟内有写入」的代理，见 §4.1） |
| 工作滞回 | `minWorkingHold` 默认 10，区间 1…300 | `min_working_hold` 默认 10，同区间 | ✅ **v0.0.200 起可配** | |
| CPU 阈值 | `cpuThreshold` 默认 6，区间 **1...50** | `cpu_threshold` 默认 6，钳 **1...50** | ✅ | 区间一致 |
| 收起延迟 | `collapseDelay` 默认 0.5，区间 **0.2…5** | `collapse_delay` 默认 0.5，钳 **0.2…5** | ✅ **v0.0.200 起一致**（此前上限 30，脏值能让面板久驻十几秒） |
| Token 告警开关 | `tokenAlertEnabled`（:38） | `token_alert_enabled` | ✅ | |
| Token 告警阈值 | `tokenAlertThreshold` 默认 200k，区间 **1_000...10_000_000** | `token_alert_threshold` 默认 200k，钳 **1_000...10_000_000** | ✅ | 区间一致 |
| 死循环告警 | `runawayCpuAlert`(true) + `runawayCpuThreshold`(70, 10…100) + `runawayDurationThreshold`(300, 30…3600) | 三个字段同名同默认同区间 | ✅ **v0.0.200 起一致**。此前 Rust 无开关，用户无法关闭 CPU 熔断 |
| 启停集合 | `enabledAgents`（键存在=全集语义；空集合是有意全关，:226-229） | `disabled_agents`（**黑名单**） | ❌ | 空集语义相反：Swift 空=全关，Rust 空=全开。**新装用户在两边看到的 agent 数不同** |
| 自定义档案 | `customAgents`（+ 损坏备份键 :32） | **无** | ❌ |
| 远程通知**界面** | 设置 → 远程通知（三通道 + 掩码 + 预览） | ✅ **v0.0.201 起有页面**（通道切换 / 策略 / 密钥 / 发送预览）；`remote::Status` 补 `limitations` 能力边界原文 |
| Agent 启停**界面** | 主列表逐项开关 | ✅ **v0.0.201 起有页面**。⚠️ 语义相反已写进页头：Swift 存启用名单（空=全关），Rust 存禁用名单（空=全开） | |
| 通知策略 | `notificationPolicy`（standard/focus/silent，:414-418） | `notification_policy` | ✅ 值域同 | |
| 完成提示音 | `playCompletionSound` + `completionSoundOption`(Glass/Pop/Ping/Blow/mute) + `alertSoundOption`(Sosumi/…) | `play_completion_sound`（bool） | ⚠️ 开关已有，**音色选择仍无** |
| 每日 token 预算 | `dailyTokenBudget` + `budgetAlertEnabled`（:43-44，区间 0...10 亿） | **无字段** | ❌ | |
| 电池/节电 | `batterySaverEnabled`(true) + `PowerSourceMonitor` | `battery_saver_enabled`(true) + `power.rs`（`pmset` 探测 + 同规则纯函数） | ✅ **v0.0.203 起一致**（三档降频先落一档：分档收益本机测不到，多一档就多一处可配错的数） | |
| 异常告警开关 | `autoAnomaliesAlertEnabled`（默认 `true`；只关告警，**不影响** `isHung` 与健康度） | `auto_anomalies_alert`（默认 `true`，v0.0.172 补齐） | ✅ 默认值与语义同（关掉不影响采集） |
| 启动登录 / 全局热键 / 菜单栏徽标 / 屏幕跟随 / 紧凑视图 / 隐藏停靠条 | `launchAtLogin` `globalHotKeyEnabled` `menuBarBadgeMode` `screenFollowMode` `compactView` `hideDockedSliver` | 六个字段 + 控件 + **引擎消费** | ✅ **v0.0.202 起一致**。开机自启与全局热键用 Tauri 官方插件（为此把 `rust-version` 从 1.77 抬到 1.90——那条声明早已不成立，它挡住插件的代价到这一版才兑现） | ❌ `knownAgents`（新 agent 提示）仍无 |
| 远程通知通道 | `remote.notify.policy.v1` / `kind.v1` / `channel.<kind>.v1` 三个 UserDefaults 键（明文，非密钥）+ 钥匙串（密钥） | `remote_kind` / `remote_policy` / `remote_channels`（v0.0.174；按 ADR 0011 与其余设置同放 settings.json，**只存非密钥字段**）+ `remote_status` 命令 | ✅ **v0.0.180 起两侧都用钥匙串**（同 service `com.agentisland.remote`、同条目名 `remote.<kind>`，老用户存过的密钥直接可用）。`remote_status` 现在查**真存在性**；写入/删除接成两条命令 `remote_secret_set` / `remote_secret_delete`（刻意**没有**读回密钥的命令：界面只要「存过没有」与掩码） |

### 6.3 其他格式口径

| 项 | Swift | Rust | 一致？ |
| :--- | :--- | :--- | :--- |
| 明细保留与折入口径 | 70 天窗口 + 折入保和（`rolledUpTokens`）+ 单文件 20,000 条上限 + 戳（inode+mtime+size）复用（[StructuredTokenUsageIndex.swift:101-133](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift#L101)） | 同四个数、同一套判定（`RETENTION_MS` / `MAX_DETAIL_PER_FILE` / `Stamp` / `fold`）；**累计 = 折入 + Σ窗口内明细**，每次重算 | ✅ **v0.0.170 起一致**。结构差异：Swift 把折入合计放在快照的 per-agent 桶里、Rust 放在 per-file 状态里（Rust 有增量状态，不需要 Swift 的整体 memo）；**保和**在两边都是结构性成立的（Rust 靠「累计每次由折入+明细重算」，旧实现的累加器在文件被整体重写时会重复计数） |
| Token 净消耗口径 | SQLite 源：`max(input − cacheRead, 0) + output`；JSONL 源 `netTokens`：`max(input − min(cached, input), 0) + max(output, 0)`（[StructuredTokenUsageIndex.swift:487](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift#L487)） | 同两条公式，逐字对齐（[tokens.rs](../../app/src-tauri/src/tokens.rs) 的 `parse_usage_line` / `query_open_code` / `query_dim_tasks`） | ✅ **v0.0.169 起一致**。此前 Rust 的 JSONL 侧写成 `input + output + cache_write`——把缓存命中的上下文当新输入全额计，本机 26 份真实 codex 日志实测**虚高 29 倍**（3.9M → 114M）。取证与复跑命令见 [JSONL 净口径实测](../research/2026-09-27-jsonl-net-token-formula.md) |
| Token 成本来源 | 监控层只记**记录成本**（SQLite 读库里的 `cost` 列；JSONL 记 `cost: 0`，[StructuredTokenUsageIndex.swift:444](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift#L444)）；**估价发生在视图层**——`DetailViews` / `TokenAnalyticsView` 用 `TokenCostEstimator.resolveCost` 在记录值为 0 时显示带 `~` 的估价 | 监控层就估：JSONL 的非零成本来自 `cost::estimate_cost` 并置 `cost_estimated`；SQLite 用库里的记录值、不置标记 | ⚠️ **同一口径、落点不同**：两边都在「没有记录成本」时显示带 `~` 的估价，但 Rust 在数据层估、Swift 在视图层估。差别只在**混合汇总**——Rust 的总额把记录值与估价相加，Swift 的总额在有记录值时**只**用记录值（`costTot.isEmpty` 时才整块换成估价） |
| 会话方言枚举 | 5 种：`genericTail`/`antigravityBrain`/`dshProjection`/`clineTasks`/`qoderTranscript`（[Models.swift:100-113](../../Sources/AgentIslandCore/Models.swift#L100)） | **无枚举**，`match profile_id` 4 个 id | ❌ 无 `qoderTranscript`（Swift 有 Qoder 档案，Rust 无 Qoder 档案，暂不构成运行时差异） |
| 档案字段集 | `bundleIDs` `pathContains` `pathExcludes` `hostBundleIDs` `cpuWorkingThreshold` `tokenAlertFloor` `sessionDirs` `tokenRoots` `emoji` `sessionDialect` `sessionDatabase` `defaultEnabled` `category` `isCustom` | `process_names` `cmdline_hints` `path_excludes` `cpu_floor` `session_dirs` `token_roots` `session_database`（v0.0.168 补齐） `category` `glyph` `emoji` | ❌ **Rust 仍缺 `pathContains` `hostBundleIDs` `tokenAlertFloor` `sessionDialect`**；直接后果见 §2.3 的 zcode、Swift 的 Qoder/WorkBuddy 防呆（进程名族规则 + 数据目录区分）在 Rust 无法表达 |
| Token 上报口 | `/session` 自报（带令牌、TTL 内）→ `provenance` | `/session` 自报「登记但不参与显示」（[webhook.rs:96](../../app/src-tauri/src/webhook.rs#L96)） | ❌ 自报在 Rust 端无任何可见效果 |

## 7. 核实边界

**已逐字核实**（每条结论都有 `file:line`）：两边档案 id 与路径、五态枚举、判定公式、
settings 全部字段与钳制区间、CLI 子命令清单、模块文件清单与行数、`AgentProfile` 字段集
（含 v0.0.168 的 `session_database`）、`AgentObservability` 五类结论与依赖字段、
费率表（`cost.rs` 与 Swift 源表由测试逐值锁住）、两类 SQLite 方言的净 token 口径与
`role` 过滤（`tokens.rs` 用例逐条对照 Swift 的 SQL）、`normalized()` 范围差异。

**未核实 / 需人工确认**：

1. ~~`session.rs` 四个 `probe_*` 与 Swift 同名解析器未逐行比对语义~~
   ⚠️ **v0.0.199 比对了 `probe_claude` 一支**，结论见
   [会话强语义逐段比对](../research/2026-09-27-session-probe-parity-check.md)：
   - **查出并修掉一个真 bug**：用户中断（Ctrl-C）后 Rust 仍报在途，
     Agent 被永久钉在 working（2s 快采样与高频扫描一起被锁住）。
     `probe_claude` 已补上 Swift 侧 `isInterruptionNotice` 的等价机制（短语表 10 条照搬，
     含短行闸），**撤销只作用于中断之前的调用**，三条用例 + 反向验证。
   - **两处差异记录未修**：① 在途判定范围——Swift 只拦终端执行类工具，
     Rust 拦**任何**未收口 `tool_use`（多报方向）② 尾部窗口 96 行/256KB vs 600 行/8MB。
   - **两处未确认**：完成态 15 分钟过期在引擎层是否有等价物；更宽窗口会不会捞出旧 attention。
   - **仍未比对**：`probe_cline` / `probe_zcode` 与 Swift 对应实现，以及 `collectFacts`
     那些事实键的等价覆盖。
2. ~~Rust 前端 `app/ui/js/views.js` 是否把 `cpu_percent: null` 印成 `0.0%`、是否过滤离线 snapshot——未读~~
   ✅ **v0.0.198 已核实**：`cpu_percent` 只喂环形仪表的弧长（`?? 0`，对仪表是正确的），
   **任何出口都没有把它渲染成文字**，不存在「0.0%」；两壳都过滤离线项；
   24h 用量列的 `—` 规则与 Swift 一致（都是 `tokens24h > 0`）。
   **但发现一处潜在漂移**：同一条「只显示在线」规则有**三种写法**——灵动岛 `s.process_running`、
   侧边栏 `snap.process_running || snap.level !== 'offline'`、Swift `snapshots.filter(\.processRunning)`。
   今天三者等价（`decide_level` 在 `!process_running` 时必定返回 `Offline`），
   但三种拼法意味着判定层以后加一个分支就会漂。
3. `Swift 端 25 个档案中` `qoder`/`antigravity`/`dsh`/`workbuddy` 的方言解析与 Rust 无对应，
   无法比对；ZCode 路径已在 v0.0.162 经本机实样核实并修正，但两端状态/动作解析语义
   尚未逐行比对。
4. Swift 与 Rust 测试的**内容覆盖对照**未做。v0.0.159 增补五态回放与三方言合成 fixture 后，可执行测试数已变化；以 `cargo test --locked` 输出为准。
5. `report.token` 与 Swift 侧令牌文件的路径/权限差异未逐行比对（ADR 0009 相关，需单独看）。
6. §2.3 表中「token 明细是否需要」一行只对 13 个共有档案核对；12 个 Swift 独有档案的
   `tokenRoots` 现状未逐一列（不影响「两边都有」的判定）。
7. ~~**Rust 侧 SQLite 读取只在合成夹具上验过**：未在本机真实的 `opencode.db` / `dimcode.sqlite`
   上对拍过~~ ✅ **v0.0.198 已在真实库上对拍**，结论如下（取证与命令见
   [opencode 表名失配与真实库对拍](../research/2026-09-27-opencode-schema-and-real-db-check.md)）：
   - **`dim` 通过，逐位相同**：`dimcode.sqlite`（447MB / 1368 行）上，Rust 公式 = Swift 公式 =
     `agentisland tokens --json` 实报 = **198,997,695**。顺带证实净口径不是洁癖：
     不扣缓存读的错口径是 **7,450,151,420（37 倍虚高）**。
   - **发现一处真 bug（两端都有）**：真实 `opencode.db` 里**没有 `message` / `session` 表**，
     只有 `session_message` / `session_v2`（最新 migration `20260923013825_project_time_active`）。
     两端都写死 `FROM message` ⇒ **都读不到 opencode 用量**，而「读到零」与「真的没用过」
     在界面上完全一样。**v0.0.198 两端都已改为按 `sqlite_master` 现查真表名。**
   - `mimocode` 仍是旧 schema（`message` + `session` 在），净消耗 48,934，读得到。
   - **仍未闭合**：`session_message` 里 0 行，所以新 schema 下的**数据层等价性无法用真实数据证明**，
     只有合成夹具守护。新 schema 要有真实数据，得等 opencode 在这台机器上真跑过一轮。

**本表不含**：性能基准、UI 视觉对照、`app/` 的构建状态（已有 ADR 0010 M1/M2 记录）。
