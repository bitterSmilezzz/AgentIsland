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
> 本表最初写于 v0.0.158，所列文件行数和行号保留调查时的快照；后续修正标在相关段落。

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

## 2. Agent 档案对照（26 vs 12）

Swift 侧内置 **26 个**档案（`AgentRegistry.swift:29-398`，`grep -c 'AgentProfile('` = 26）；
Rust 侧内置 **12 个**（[registry.rs:25-183](../../app/src-tauri/src/registry.rs)）。
Rust 另有一条 `>= 12` 的守护测试（[registry.rs:214-221](../../app/src-tauri/src/registry.rs)），
**刻意不要求两边相等**——所以档案数分叉不会被测试拦住，只能靠本表盯。

### 2.1 只在 Swift 有（14 个）

`dim` `qoder` `copilot` `workbuddy` `workbuddy-ai` `antigravity` `mimocode` `hermes`
`continue` `chatgpt` `dsh` `ego-browser` `vibe-usage` `openviking`

（出处：[AgentRegistry.swift:31-343](../../Sources/AgentIslandCore/AgentRegistry.swift#L31) 各 `id:` 行；
Rust 侧 12 个 id 见 [registry.rs:27-171](../../app/src-tauri/src/registry.rs#L27)，列表不同。）

### 2.2 只在 Rust 有（1 个）

`vscode` —— [registry.rs:80](../../app/src-tauri/src/registry.rs#L80)。Swift 侧无此档案，
但 Swift 有 `cline`/`roo-code` 用的是 VS Code 的 `globalStorage/<ext>` 路径，功能上覆盖同一批用户。

### 2.3 两边都有（12 个）——逐字段比

| id | sessionDirs / session_dirs | 一致？ | tokenRoots / token_roots | 一致？ | CPU 阈值 | 一致？ | sessionDialect | 一致？ |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `zcode` | S `~/.zcode/v2/checkpoints` :182-192 | ❌ | S **未登记** | ❌ | S 20.0 :188 / R 20.0 :34 | ✅ | S `genericTail`（默认）/ R 按 id 分派 :35 | ❌ |
| `claude` | S `~/.claude/sessions` + `~/.claude/projects` :51 | ❌ | S 同上两个 :52 / R 同两个 :50 | ✅ | S 20.0 :50 / R `None` :48 | ❌ | S `genericTail` / R `probe_claude` :35 | ✅（同协议） |
| `codex` | S `~/.codex/sessions` :94 | ✅ | S 同 :97 / R 同 :63 | ✅ | S 无 / R 无 | ✅ | 同 | ✅ |
| `cursor` | S `Library/.../Cursor/...` :108 | ✅（Rust 用 `%APPDATA%`） | S 无 / R 无 | ✅ | S 20.0 / R 20.0 | ✅ | 同 | ✅ |
| `trae` | S `.../Trae CN/User/workspaceStorage` :120 | ❌（R 含 Trae + Trae CN 两条 :179） | 均无 | ✅ | S 20.0 / R 20.0 | ✅ | 同 | ✅ |
| `cline` | 同路径 :372 | ✅ | S 无 / R 同 :102 | ✅ | S 无 / R 无 | ✅ | S `.clineTasks` :375 / R `probe_cline` :35 | 基本一致 |
| `roo-code` / Rust `roo` | **id 本身不同**：S `roo-code` :378，R `roo` :106 | ❌ | S 无 / R 同 :115 | ✅ | 均无 | ✅ | S `.clineTasks` :386 / R 同 :35 | 基本一致 |
| `opencode` | `~/.local/share/opencode` :227 | ✅ | S 无 / R 有 :128 | ❌ | S 20.0 :224 / R `None` | ❌ | S 无方言 / R 无 | ✅ |
| `goose` | `~/.config/goose/sessions` :394 | ✅ | S 无 / R 无 | ✅ | 均无 | ✅ | 同 | ✅ |
| `aider` | `~/.aider` :362 | ✅ | S 无 / R 无 | ✅ | 均无 | ✅ | 同 | ✅ |
| `windsurf` | `Library/.../Windsurf/...` :352 | ✅（R 用 `~/.codeium/windsurf` :166） | S 无 / R 无 | ✅ | S 20.0 / R 20.0 | ✅ | 同 | ✅ |

**必须「说清」而不是「抹平」的四处差异：**

1. **`claude` / `opencode` 的 CPU 阈值不一致**：Swift 给 `cpuWorkingThreshold: desktopCPUFloor(20.0)`
   （[AgentRegistry.swift:50](../../Sources/AgentIslandCore/AgentRegistry.swift#L50)、:224），
   Rust 是 `cpu_floor: None`（[registry.rs:48](../../app/src-tauri/src/registry.rs#L48)、:124）。
   后果：同一个 claude 进程在岛里**不会**被判 working，在 sidebar **会**。
   这属于「两边都有但算法不同」，ADR 0010 明确不允许。
2. **`zcode` 的会话目录完全不是同一棵树**：Swift 盯 `~/.zcode/v2/checkpoints`（Agent 运行落盘点，
   注释解释过 v2 根目录的空闲刷新问题），Rust 盯 `~/.zcode/cli/rollout` + `log`。
   两者**不可能同时为真**：至少一边读错了位置。ADR 0010 M3 里 `session` 是地基模块，这条要在迁之前裁。
3. **`opencode` 的 token 明细**：Swift 刻意**不登记** `tokenRoots`（同 Qoder 的理由：落盘 token 字段全 0），
   Rust 却登记了 `~/.local/share/opencode`（[registry.rs:128](../../app/src-tauri/src/registry.rs#L128)）。
   ① 的 CPU 差异 + 这一条，会让 opencode 在两边显示完全不同的 token 数。
4. **`roo-code` vs `roo` id 不一致**：`settings.disabled_agents` 是按 id 过滤的
   （[engine.rs:66-72](../../app/src-tauri/src/engine.rs#L66)），同一条设置在两边关掉的是不同字符串；
   跨形态导设置会静默失效。**建议在 Phase 1 统一成 `roo-code`，两边同时改。**

## 3. 能力模块对照（Swift Core 47 文件 vs Rust 11 rs）

「两边都有」= Rust 侧有同名职责的模块，无论实现深浅；「字段/算法差异」列在最右。

### 3.1 只在 Swift 有（约 22 个，约 5,700 行）

逐个核实过（`ls Sources/AgentIslandCore/` vs `ls app/src-tauri/src/`，Rust 侧 11 个文件里
`grep -ri` 这些关键词零命中或仅命中注释）：

| Swift 模块 | 行 | 职责（为什么不能少） |
| :--- | ---: | :--- |
| [TokenBudgetTracker.swift](../../Sources/AgentIslandCore/TokenBudgetTracker.swift) | 81 | 每日预算告警；0 = 未设，`> 0` 才启用 |
| [TokenForecastEvaluator.swift](../../Sources/AgentIslandCore/TokenForecastEvaluator.swift) | 92 | 月末预测——CLI `tokens` 的招牌输出 |
| [AgentHealthEvaluator.swift](../../Sources/AgentIslandCore/AgentHealthEvaluator.swift) | 144 | 健康度评分（`HealthGrade`） |
| [AgentResilienceGuard.swift](../../Sources/AgentIslandCore/AgentResilienceGuard.swift) | 112 | 异常驻留/死锁持续守护状态机 |
| [TaskDurationTracker.swift](../../Sources/AgentIslandCore/TaskDurationTracker.swift) | 117 | 单次任务时长记录 |
| [RemoteNotifier.swift](../../Sources/AgentIslandCore/RemoteNotifier.swift) | 387 | 外发调度：策略→渲染→传输→记账 |
| [RemoteNotification.swift](../../Sources/AgentIslandCore/RemoteNotification.swift) | 454 | 消息模型与策略（外发每字节都离开本机） |
| [RemoteTransport.swift](../../Sources/AgentIslandCore/RemoteTransport.swift) | 340 | HTTP + SMTP 通道与「发送预览」 |
| [RemoteNotifyStore.swift](../../Sources/AgentIslandCore/RemoteNotifyStore.swift) | 98 | 通道配置落盘（明文）与密钥（钥匙串）分离 |
| [SMTPSocket.swift](../../Sources/AgentIslandCore/SMTPSocket.swift) | 174 | Network.framework SMTP 会话 |
| [AuditReportExporter.swift](../../Sources/AgentIslandCore/AuditReportExporter.swift) | 255 | Markdown/CSV/JSON 运维审计报告 |
| [TokenReportExporter.swift](../../Sources/AgentIslandCore/TokenReportExporter.swift) | 84 | Token 账单/会话报表导出 |
| [StructuredTokenUsageIndex.swift](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift) | 512 | JSONL 工具 token 明细索引（只留时间与计数） |
| [ProcessTreeInspector.swift](../../Sources/AgentIslandCore/ProcessTreeInspector.swift) | 137 | 进程树构建（详情页「谁在吃 CPU」） |
| [LogTailReader.swift](../../Sources/AgentIslandCore/LogTailReader.swift) | 201 | 有界尾读 + 截断首行丢弃；Rust 侧 `read_tail_lines`（[session.rs:44](../../app/src-tauri/src/session.rs#L44)）有同类逻辑但只服务 session.rs，未抽公共层 |
| [LogPatternAnalyzer.swift](../../Sources/AgentIslandCore/LogPatternAnalyzer.swift) | 87 | 实时日志错误模式识别 |
| [LiveSampler.swift](../../Sources/AgentIslandCore/LiveSampler.swift) | 90 | 一次性实况采样（status/top/doctor 共用地基） |
| [Selftest.swift](../../Sources/AgentIslandCore/Selftest.swift) | 184 | 无头自检（`agentisland selftest`） |
| [InstalledAppsCache.swift](../../Sources/AgentIslandCore/InstalledAppsCache.swift) | 215 | 已安装 CLI/bundle 缓存——**「读不到 ≠ 没装」的前提** |
| [BatterySaver.swift](../../Sources/AgentIslandCore/BatterySaver.swift) | 46 | 电源自适应降频 |
| [AgentCleaner.swift](../../Sources/AgentIslandCore/AgentCleaner.swift) | 307 | 一键清理异常 Agent（CLI `clean`） |
| [AgentLogStreamer.swift](../../Sources/AgentIslandCore/AgentLogStreamer.swift) + [AgentActionInspector.swift](../../Sources/AgentIslandCore/AgentActionInspector.swift) + [AgentSessionInspector.swift](../../Sources/AgentIslandCore/AgentSessionInspector.swift) + [SelfReport.swift](../../Sources/AgentIslandCore/SelfReport.swift) + [ProcessMonitor.swift](../../Sources/AgentIslandCore/ProcessMonitor.swift) + [FileMonitor.swift](../../Sources/AgentIslandCore/FileMonitor.swift) + [ReadonlyDB.swift](../../Sources/AgentIslandCore/ReadonlyDB.swift) + [ShellQuoting.swift](../../Sources/AgentIslandCore/ShellQuoting.swift) + [URLSchemeParser.swift](../../Sources/AgentIslandCore/URLSchemeParser.swift) + [WireText.swift](../../Sources/AgentIslandCore/WireText.swift) + [Probe.swift](../../Sources/AgentIslandCore/Probe.swift) + [ProbeFailureLog.swift](../../Sources/AgentIslandCore/ProbeFailureLog.swift) + [AppLog.swift](../../Sources/AgentIslandCore/AppLog.swift) + [BatterySaver](../../Sources/AgentIslandCore/BatterySaver.swift) | 5,000+ | 见 3.2：这些是「Rust 有薄版、Swift 有厚版」的一类 |

> 说明：最后一行把 Swift 里 14 个「Rust 有薄对应」的厚实现列在一起，避免把它们误读成
> 「完全缺失」。真正**零对应**的是 3.1 表内那 22 行（加上 AgentSessio/… 里的厚实现差异）。

### 3.2 两边都有（7 个）——实现深浅不同

| 职责 | Swift | Rust | 差异要点 |
| :--- | :--- | :--- | :--- |
| 档案表 | [AgentRegistry.swift](../../Sources/AgentIslandCore/AgentRegistry.swift) 602 行：26 档案 + 自动发现 CLI + 用户自定义（UserDefaults） | [registry.rs](../../app/src-tauri/src/registry.rs) 232 行：12 档案硬编码 | Rust **无自动发现、无自定义档案**；`builtin()` 每次调用重读 home dir |
| 五态引擎 | [ActivityEngine.swift](../../Sources/AgentIslandCore/ActivityEngine.swift) 1492 行 | [engine.rs](../../app/src-tauri/src/engine.rs) 626 行 | Swift 有电源分级采样、`visibleSnapshots`/`ringShelfSnapshots` 可见口径、冲突双显；Rust 有 demo 模式与 webhook 事件源。**核心双信号与 CPU 熔断一致**，见 §4 |
| 进程监控 | [ProcessMonitor.swift](../../Sources/AgentIslandCore/ProcessMonitor.swift) 700 行：libproc 直读进程表 | [procmon.rs](../../app/src-tauri/src/procmon.rs) 129 行：`sysinfo` 0.33 | Swift 直读 `/dev` 级接口 + 自定义匹配（pathContains/pathExcludes/hostBundleIDs）；Rust 的 `AgentProfile` **没有 pathContains / hostBundleIDs 字段**——见 §2.3 与 §6 |
| 文件活动 | [FileMonitor.swift](../../Sources/AgentIslandCore/FileMonitor.swift) 634 行：后台递归 + O(1) 缓存 + 限深 | [filemon.rs](../../app/src-tauri/src/filemon.rs) 138 行：同步遍历 | Rust 无后台队列、无深度缓存分层；`max_depth 4` + 扩展名白名单（[filemon.rs:105](../../app/src-tauri/src/filemon.rs#L105)） |
| 会话解析 | [AgentSessionInspector.swift](../../Sources/AgentIslandCore/AgentSessionInspector.swift) 1687 行：5 种方言 + 专有协议 | [session.rs](../../app/src-tauri/src/session.rs) 414 行：4 个 id 专属 `probe_*` | 分派方式不同：Swift 按**档案声明的方言**，Rust 按 **agent id**（[session.rs:35-40](../../app/src-tauri/src/session.rs#L35)）。ADR 0010 要求「新增复用既有格式的 Agent 只改档案」，Rust 今天做不到 |
| Token 用量 | [TokenUsageMonitor.swift](../../Sources/AgentIslandCore/TokenUsageMonitor.swift) 1294 行 + [StructuredTokenUsageIndex.swift](../../Sources/AgentIslandCore/StructuredTokenUsageIndex.swift) 512 行 + [ReadonlyDB.swift](../../Sources/AgentIslandCore/ReadonlyDB.swift) 137 行：SQLite + JSONL 双源 | [tokens.rs](../../app/src-tauri/src/tokens.rs) 318 行：只 JSONL | Rust **完全不读 SQLite**（`grep -rn sqlite` 只命中 filemon 的扩展名白名单）；所有 `sessionDatabase` 档案（dim/zcode/opencode/mimocode/workbuddy…）在 Rust 侧 token 明细为 0 |
| Token 估价 | [TokenCostEstimator.swift](../../Sources/AgentIslandCore/TokenCostEstimator.swift) 113 行：25 条官方费率 + 3:1 混合加权 | [tokens.rs:302-318](../../app/src-tauri/src/tokens.rs#L302) `price_lookup`：5 档粗分 | **费率表不一致**：Swift 对 claude-3-7-sonnet 是 (3,15)，Rust 对 haiku 给 (1.0,5.0) 而 Swift 是 (0.8,4.0)；Rust 对 `glm` (0.55,2.0) 在 Swift 无对应。两边同日同量会算出不同钱 |
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
| 状态序 | `<` 定义：offline(0)<idle(1)<completed(2)<working(3)<attention(4)（[Models.swift:66-72](../../Sources/AgentIslandCore/Models.swift#L66)） | `derive(Ord)` 声明顺序同 Swift（[models.rs](../../app/src-tauri/src/models.rs)） | ✅ v0.0.158 已纠正；两边均以 attention 为最高优先级 |
| 中文 label | 离线/待机/已完成/工作中/待确认 :52-58 | 同 :26-32 | ✅ |
| 双信号判定 | `working = 进程在 && (workingWindow 内有写入 \|\| CPU >= max(cpuFloor, cpuThreshold))`（[ActivityEngine.swift:8-12](../../Sources/AgentIslandCore/ActivityEngine.swift#L8)、:659） | 同一公式（[engine.rs:254-278](../../app/src-tauri/src/engine.rs#L254)） | ✅ 算法一致；但 `workingWindow=60` 在 Rust 是**硬编码字面量**（[engine.rs:82](../../app/src-tauri/src/engine.rs#L82)），Swift 来自可钳制的 `EngineConfig.workingWindow` |
| attention/completed 强语义 | 方言分派 + 有界尾读 + 指纹去重 | id 分派 `probe_claude/codex/cline/zcode` + 指纹去重；Codex 调用配对与完成标记有合成 fixture 守护 | ⚠️ 部分协议行为有守护，方言分派与来源覆盖仍不同，不能宣称整体一致 |
| CPU 熔断 | `runawayCpuAlert`（70% / 5 分钟，[Models.swift:739-740](../../Sources/AgentIslandCore/Models.swift#L739)） | 70% / 5 分钟，硬编码（[engine.rs:280-296](../../app/src-tauri/src/engine.rs#L280)） | ✅ 数值一致；Swift 可关（`runawayCpuAlert`），Rust 无此开关 |
| Token 暴涨告警 | 每分钟净增量 > `tokenAlertThreshold`（200k） | 同一口径，注释明说「与 macOS 端口径一致」（[engine.rs:298](../../app/src-tauri/src/engine.rs#L298)） | ✅ |
| 降频 | 有活动 `sampleInterval` / 闲置 `idleSampleInterval`（5s）/ 全离线 60s / 节电三档（[ActivityEngine.swift:1395-1404](../../Sources/AgentIslandCore/ActivityEngine.swift#L1395)） | 有活动 `sample_interval`；闲置 `×2.5` 夹在 2.0–12.5s（[main.rs:302-310](../../app/src-tauri/src/main.rs#L302)） | ⚠️ 公式不同（`×2.5` vs 独立 `idleSampleInterval` 字段）；无节电三档。**需说清**：两侧耗电量与「岛多久变灰」不同 |
| 可见口径 | `visibleSnapshots`（只显在线）+ `ringShelfSnapshots`（[ActivityEngine.swift:1472-1483](../../Sources/AgentIslandCore/ActivityEngine.swift#L1472)） | `state()` 直接给全部 snapshot，前端可自行过滤（未逐行比对） | ⚠️ 未逐行比对 |

### 4.1 「读不到 ≠ 零」：Swift 有五类结论，Rust 侧无等价物

Swift 的 [AgentObservability.evaluate](../../Sources/AgentIslandCore/AgentObservability.swift#L46)
返回**五类结论**（:14-25）：`observed` / `blindSessionSource`（源读不到）/
`noLocalData`（读不到会话与用量）/ `sourceNotWired`（档案未登记明细源）/
`notInstalled`（未安装且进程不在）。它靠两个 Rust 侧**不存在的字段**才能判：

- `snapshot.installed`（安装判定，来自 `InstalledAppsCache`）——[Models.swift:421](../../Sources/AgentIslandCore/Models.swift#L421)
- `snapshot.sessionProbeHealth`（源读到的失败原因）——[Models.swift:439](../../Sources/AgentIslandCore/Models.swift#L439)

**Rust 侧结论：无等价物。** `grep -rni "selfreport|blind|provenance|不可信|读不到"` 在
`app/src-tauri/src/` + `app/ui/js/` 零命中（唯一命中是 registry 注释里的「读不到」一词）。
[models.rs](../../app/src-tauri/src/models.rs) 的 `AgentSnapshot` 既没有 `installed` 也没有
`sessionProbeHealth` 也没有 `provenance`；因此 Rust 端的「待机」**无法与「没读到」区分**。
这正是 Swift 那条被专门写过注释的口径（Models.swift:414-418「漏传只能得到没说，不能得到假的观测」）
在 Rust 侧整体缺失。ADR 0010 M3 的 `Health` 模块若有产出，这是第一个该带的字段。

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
| `doctor` | AgentObservability、AgentHealthEvaluator、SessionProbeHealth | ❌ 无（依赖 §4.1 缺失字段） |
| `check` / `clean` | AgentResilienceGuard、AgentCleaner、ProcessTreeInspector | ❌ 无；Rust 有 `terminate_agent` 但无异常判定 |
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
| 采样间隔（闲置） | `idleSampleInterval` 默认 **5.0** | **无此字段**（用 `sample_interval × 2.5`） | ❌ | 见 §4 |
| 工作判定窗口 | `workingWindow` 默认 60 | 硬编码 60（[engine.rs:82](../../app/src-tauri/src/engine.rs#L82)） | ⚠️ 值同、不可配 | |
| 活跃会话窗口 | `activeSessionWindow` 默认 600 | 无等价采样窗口（原表把未使用字面量误记为实现） | ❌ | Rust 尚未补齐活跃会话数口径 |
| 工作滞回 | `minWorkingHold` 默认 10 | 硬编码 10（[engine.rs:83](../../app/src-tauri/src/engine.rs#L83)） | ⚠️ 值同、不可配 | |
| CPU 阈值 | `cpuThreshold` 默认 6，区间 **1...50** | `cpu_threshold` 默认 6，钳 **1...50** | ✅ | 区间一致 |
| 收起延迟 | `collapseDelay` 默认 0.5，区间 `SettingLimits` **0.2...5**（:144） | `collapse_delay` 默认 0.5，钳 **0.2...30** | ❌ | 上限不同：5 vs 30 |
| Token 告警开关 | `tokenAlertEnabled`（:38） | `token_alert_enabled` | ✅ | |
| Token 告警阈值 | `tokenAlertThreshold` 默认 200k，区间 **1_000...10_000_000** | `token_alert_threshold` 默认 200k，钳 **1_000...10_000_000** | ✅ | 区间一致 |
| 死循环告警 | `runawayCpuAlert` + `runawayCpuThreshold`(70) + `runawayDurationThreshold`(300) | **无字段**（硬编码） | ❌ | Rust 用户无法关闭 CPU 熔断 |
| 启停集合 | `enabledAgents`（键存在=全集语义；空集合是有意全关，:226-229） | `disabled_agents`（**黑名单**） | ❌ | 空集语义相反：Swift 空=全关，Rust 空=全开。**新装用户在两边看到的 agent 数不同** |
| 自定义档案 | `customAgents`（+ 损坏备份键 :32） | **无** | ❌ | |
| 通知策略 | `notificationPolicy`（standard/focus/silent，:414-418） | `notification_policy` | ✅ 值域同 | |
| 完成提示音 | `playCompletionSound` + `completionSoundOption`(Glass/Pop/Ping/Blow/mute) + `alertSoundOption`(Sosumi/…) | `play_completion_sound`（bool） | ❌ | Rust 无音色选择 |
| 每日 token 预算 | `dailyTokenBudget` + `budgetAlertEnabled`（:43-44，区间 0...10 亿） | **无字段** | ❌ | |
| 电池/节电 | `batterySaverEnabled` + `PowerSourceMonitor` | **无字段** | ❌ | |
| 启动登录 / 全局热键 / 菜单栏徽标 / 屏幕跟随 / 紧凑视图 / 隐藏停靠条 / 新 agent 提示 | `launchAtLogin` `globalHotKeyEnabled` `menuBarBadgeMode` `screenFollowMode` `compactView` `hideDockedSliver` `autoAnomaliesAlertEnabled` `knownAgents` | **全部无** | ❌ | 共 7+ 键，Swift 侧 30 个键名里 Rust 只覆盖 11 个 |
| 远程通知通道 | `remote.notify.channel.<kind>.v1` JSON blob（明文）+ 钥匙串（密钥） | **无** | ❌ | ADR 0005/0006/0009 口径只落在 Swift |

### 6.3 其他格式口径

| 项 | Swift | Rust | 一致？ |
| :--- | :--- | :--- | :--- |
| Token 净消耗口径 | 不含缓存读取（[TokenUsageMonitor.swift](../../Sources/AgentIslandCore/TokenUsageMonitor.swift) 注释） | `net = input + output + cache_write`（[tokens.rs:236](../../app/src-tauri/src/tokens.rs#L236)） | ✅ 一致 |
| 会话方言枚举 | 5 种：`genericTail`/`antigravityBrain`/`dshProjection`/`clineTasks`/`qoderTranscript`（[Models.swift:100-113](../../Sources/AgentIslandCore/Models.swift#L100)） | **无枚举**，`match profile_id` 4 个 id | ❌ 无 `qoderTranscript`（Swift 有 Qoder 档案，Rust 无 Qoder 档案，暂不构成运行时差异） |
| 档案字段集 | `bundleIDs` `pathContains` `pathExcludes` `hostBundleIDs` `cpuWorkingThreshold` `tokenAlertFloor` `sessionDirs` `tokenRoots` `emoji` `sessionDialect` `sessionDatabase` `defaultEnabled` `category` `isCustom` | `process_names` `cmdline_hints` `path_excludes` `cpu_floor` `session_dirs` `token_roots` `category` `glyph` `emoji` | ❌ **Rust 缺 `pathContains` `hostBundleIDs` `tokenAlertFloor` `sessionDialect` `sessionDatabase`**；直接后果见 §2.3 的 zcode、Swift 的 Qoder/WorkBuddy 防呆（进程名族规则 + 数据目录区分）在 Rust 无法表达 |
| Token 上报口 | `/session` 自报（带令牌、TTL 内）→ `provenance` | `/session` 自报「登记但不参与显示」（[webhook.rs:96](../../app/src-tauri/src/webhook.rs#L96)） | ❌ 自报在 Rust 端无任何可见效果 |

## 7. 核实边界

**已逐字核实**（每条结论都有 `file:line`）：两边档案 id 与路径、五态枚举、判定公式、
settings 全部字段与钳制区间、CLI 子命令清单、模块文件清单与行数、`AgentProfile` 字段集、
`AgentObservability` 五类结论与依赖字段、`price_lookup` 费率表、`normalized()` 范围差异。

**未核实 / 需人工确认**：

1. `session.rs` 四个 `probe_*` 与 Swift 同名解析器**未逐行比对语义**（只确认了分派方式与信号种类一致）。
2. Rust 前端 `app/ui/js/views.js` 是否把 `cpu_percent: null` 印成 `0.0%`、是否过滤离线 snapshot——未读。
3. `Swift 端 26 个档案中` `qoder`/`antigravity`/`dsh`/`workbuddy` 的方言解析与 Rust 无对应，
   无法比对；Rust 侧 `zcode` 走 `probe_zcode`（读取 `~/.zcode/cli/rollout`），Swift 读 `~/.zcode/v2/checkpoints`，
   **哪一边对，未在本机实跑验证**（未跑 build/test，本任务为纯调研）。
4. Swift 与 Rust 测试的**内容覆盖对照**未做。v0.0.159 增补五态回放与三方言合成 fixture 后，可执行测试数已变化；以 `cargo test --locked` 输出为准。
5. `report.token` 与 Swift 侧令牌文件的路径/权限差异未逐行比对（ADR 0009 相关，需单独看）。
6. §2.3 表中「token 明细是否需要」一行只对 12 个共有档案核对；14 个 Swift 独有档案的
   `tokenRoots` 现状未逐一列（不影响「两边都有」的判定）。

**本表不含**：性能基准、UI 视觉对照、`app/` 的构建状态（已有 ADR 0010 M1/M2 记录）。
