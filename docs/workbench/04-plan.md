# 04 · 实施计划

> **当前执行门禁**以 [ADR 0010](../adr/0010-swift-freeze-and-rust-prerequisites.md) 的 M1–M5 为准。
> 下列 Phase 1–4 是最初的功能实施草案，不能跳过 Rust 测试与能力迁移直接开侧边栏。
> 当前：Phase 0 的扫描器及七条守护已入库，Rust 五态、三方言 fixture 与
> `provider.rs` 原子写/掩码测试使 M1 完成；M2 本机工作区/打包已达成。
> M3 已从已有档案的口径差异着手；ZCode 会话路径经本机实样核实并修正，
> Rust 可观测性五类判定已接入快照与界面，但安装、深层探测健康、活跃会话证据未对齐。
> Trae/Windsurf 待实样。12 个缺失模块已开始逐个迁入：v0.0.166 迁入 `TokenCostEstimator`
> （费率表与算法两端口径对齐，带跨语言漂移哨兵），**剩 11 个**。
> v0.0.168 把 token 明细的 SQLite 源接进 Rust（`sqlite.rs` 只读层 + 两类方言），
> 并补进 `dim`/`mimocode` 两个带库档案（Rust 12 → 14）。
> v0.0.169 按本机真实日志修掉 JSONL 净口径的 29 倍虚高，并对齐记录形状与去重。
> v0.0.170 迁入 `StructuredTokenUsageIndex` 的保留口径（70 天窗口 / 折入保和 / 单文件上限 / 戳判定），
> token 明细这一组收口。
> v0.0.171 进入判定增强组：迁入 `AgentHealthEvaluator` 与它依赖的卡死三态（含「连续观测资格」）。
> v0.0.172 同组继续：迁入 `AgentResilienceGuard`（持续死锁 / 内存驻留守护，含默认开启的开关）。
> v0.0.173 修掉事件投递的丢事件缺陷（覆盖式赋值 → 待发队列），并把 refresh 的接线抽成可测函数。
> v0.0.174 进外发组：迁入策略层（策略 / 通道校验 / 逐字段容错解码 → `remote.rs`），接进设置与 `remote_status`。
> v0.0.175 迁入渲染层（`render.rs`：最小内容口径 / 四类分开的转义 / 4096 上限 / 掩码），接 `remote_preview`（「发送预览」）。
> v0.0.176 迁入闸门/节流/记账（`notifier.rs`，含 20 条「最近外发」账本）与传输骨架（`transport.rs`：明文 http 端到端可发，https/SMTP 如实报未接入）。
> v0.0.177 接入 `https`（`native-tls` = 系统 TLS 栈），本地自签证书离线端到端验证；SMTP 会话仍缺。
> v0.0.178 外发改走工作线程（不再占用采样那一拍）并补上失败重试一次；顺手修掉一个并行测试的端口竞争。
> v0.0.179 迁入 SMTP over 465 会话（`smtp.rs`，含点号加倍、`smtp_line` 折行、失败也关连接）。
> v0.0.180 迁入钥匙串（`secret.rs`：同 service/条目名、「闸门放行后才读」、存在性不取数据），并接出 `remote_secret_set`/`remote_secret_delete`。**外发组迁完**。
> v0.0.181 迁入 Token 预算告警（`budget.rs`，含滞回与「只有跨级才报」）与月末预估（`forecast.rs`），> v0.0.181 迁入 Token 预算告警（`budget.rs`）与月末预估（`forecast.rs`），并把两份语义不同的 `compact` 统一到 Swift 的 `TokenUsage.compact`。
> v0.0.182 迁入审计报告导出（`audit.rs`：Markdown/CSV、表格转义、`—`≠`0`、跨源口径差额），并给事件补上 `duration`（报告「耗时」列与完成摘要从此有真值）。
> v0.0.183 迁入派生进程树（`trees.rs`：防环、根不算自己的后代、全程不递归；`procmon::table()` 提供整张表）。
> v0.0.184 迁入无头自检（`selftest.rs`：22 条检查；`agentisland --selftest` 与 `run_selftest` 命令），并把进程匹配抽成纯函数 `profile_matches` 让排除规则可夹具断言；顺手统一「最近活动」文案口径。
> v0.0.185 迁入 Token 消费报表导出（`report.rs`：Markdown/CSV、BOM、区间合计按 cutoff 分档；接两条命令）。
> v0.0.186 迁入 `TaskDurationTracker`（`duration.rs`，含 Swift 那条「≥3.5 秒才算一次任务」的门槛对齐）。
> **M3 收口：12 个模块全部迁完**（ADR 0010），侧边栏（Phase 1）门禁解除。下一步进 Phase 1（侧边栏双形态 + CC Switch + ToDos）。
> 顺序按 ADR 0010：token 明细 → 判定增强 → 外发 → 导出与自检。
> `scripts/release.sh` 现在执行脱敏守护、`cargo test --locked` 和 Swift 测试。

## Phase 0 — 修复不可信的发版门禁（✅ 已完成）

脱敏闸门 `scripts/scan-secrets.sh` 有两处失败静默，症状都是「什么都没扫却打印 ✓」。
详见 [01-current-state.md](01-current-state.md) §6.1。

**改动**

1. 新增 `put()` 作为所有中间文件的写入出口：先写 `.new` 再 `mv`，每步失败 `exit 3`。
2. 新增 `load_file_list()`：接住 git 退出码、拒绝空列表。
3. `sort -u "$HITS" -o "$HITS"` 改走临时文件（in-place 排序会触发 SIGBUS）。
4. 基线文件缺失不再等价于「零命中」，改为拒绝放行。
5. 命中行的掩码前移到写盘前，`HITS` 里永不出现密钥原文。

**新增测试**：[scripts/test-scan-secrets.sh](../../scripts/test-scan-secrets.sh)，7 条守卫用例：
干净工作区必须通过（对照基准）／git 退出码非零／扫描中途 SIGKILL／结果文件不可写／
文件列表为空／基线缺失／新增未备案命中仍要拦住。

**验收结果**

| 项 | 结果 |
| :--- | :--- |
| 干净工作区 | ✓ 18 条命中零新增，返回 0 |
| 544 项 Swift 测试 | 544 通过, 0 失败 |
| 闸门守卫测试 | 7 通过, 0 失败 |
| 反向验证 | 把被测脚本换回修复前版本，第 1 条用例即 `Bus error: 10` 崩溃 |

**文档**：README 构建段补入 `./scripts/test-scan-secrets.sh` 入口。

---

## Phase 1 — 侧边栏壳 + 双形态并存

> **进行中（v0.0.187 起）**。逐条对照下面的清单；未打勾的**没做**，不是「已完成但没写」。

1. ✅ `settings.rs` 增 `shell_mode`（`island` | `sidebar`，默认 `island`），另加 `sidebar_edge`（左/右）与
   `sidebar_width`（记忆宽度，钳在 280–720 且不超过工作区）；三者都带归一化（认不出的值一律回落）。
2. ✅ `tauri.conf.json`：island 窗口配置**未动**，新增 `sidebar` 窗口（初始 `visible: false`）；
   两个窗口各带 `?shell=island` / `?shell=sidebar`，前端据此分派（无需额外 IPC 询问）。
3. ✅ `app/ui/css/sidebar.css`：两栏骨架 + 导航 + 首层极简列表，**只用 `tokens.css` 的变量**，
   且全部规则挂在 `.shell-sidebar` 下（不串味）。
4. ✅ `js/main.js` 的形态分派（`js/shell.js` 提供形态事实，避免 `views.js` ↔ `main.js` 成环）；
   侧边栏分支不做灵动岛那套（细条几何 / 贴边放置 / 展开收起 / 失焦防抖）。**懒加载 `import()` 未做**。
   v0.0.188 补：`capabilities/default.json` 的 `windows` 加上 `sidebar`——
   否则它**没有监听权限**，`listen('engine://tick')` 被静默拒绝，侧边栏永远不刷新
   （详见 v0.0.188 的 CHANGELOG 与 review；这是运行时冒烟抓到的，单元测试抓不到）。
5. ✅（**内容层**）：`views.js` 新增 `agentRowModel(snap)` —— 一行 Agent「显示什么、怎么措辞、
   什么颜色、没取到写什么」**只有这一处**，灵动岛的行与侧边栏的行都走它；页面函数
   （`pageAnalytics` / `pageAgentDetail`）与报表注水路径也共用。
   **排版层按形态分开是有意的**（灵动岛的行有进度环/动作条/趋势点，侧边栏只是两行字），
   而且不再有漂移风险：漂移发生在「文案与字段」上，那一层已经收敛。
   两条哨兵守着它：`both_shells_render_agent_rows_through_one_shared_model` 与
   `every_sidebar_class_is_actually_styled`（都用例反证过）。
6. ✅ `main.rs` / `placement.rs`：按 `shell_mode` 分派（island 分支零改动；sidebar 分支走
   `placement::sidebar_frame`：贴左/右、铺满工作区高度、宽度取记忆值）；命令
   `set_shell_mode` / `set_sidebar_width` / `set_sidebar_edge` / `place_sidebar`；
   启动时按上次的形态显示对应窗口。
7. ❌ `island.css` 尚未拆分（专属部分与共用组件仍在一起；目前靠 `.shell-sidebar` 前缀隔离，够用但不够干净）。

**验收对照**

- ✅ 默认启动仍是灵动岛且行为零变化（默认值就是 `island`；island 分支代码未动）
- ⚠️ 切到 sidebar 后为侧边栏、首层极简：**功能层已验证**（`--shell=sidebar` 起得来、前端零错误、
  能收到引擎推送）；**像素未验证**——本机 `screencapture` 需要屏幕录制授权，拿不到
  （`could not create image from rect`）。
- ⚠️ 「高级设置」进分析页与详情页：侧边栏可进分析页与详情页（复用同一批页面函数），
  但**没有设置入口**（设置页还没做）
- ✅ 既有测试无回归（Rust 252 条；Swift 550 条由发版脚本跑）
- ❌ 「两种形态反复切换 10 次：无窗口残留、无样式串味」**未做运行时验证**
  （设计上不会残留：两个窗口都只建一次，切换只改显隐；但这需要真按十次）

## Phase 2 — Codex 配置档位（原 CC Switch 草案收窄）

> **进行中（v0.0.189 起）**。Rust 侧（1/2/5/7）已完成；界面（3/6）未做。

1. ✅ `provider.rs`（v0.0.160 建的地基 + v0.0.189 补全）：`scan_tools()`（第一阶段只认 Codex）、
   `ProviderStore::{list,save,delete}`（我们自己的 `profiles.json`，损坏即降级为空清单）、
   `plan_codex_apply()`（纯函数，保留注释/顺序/缩进）、`apply_codex_profile()`（**先备份再原子写**）、
   `restore_codex_backup()` / `restore_backup_by_name()`（按**名字**还原，拒绝路径穿越）、
   `is_codex_config_target()`（只认 `<某目录>/.codex/config.toml`，`auth.json` 进不来）。
2. ✅ `main.rs` 注册 8 条命令：`provider_scan_tools` / `provider_list_profiles` /
   `provider_save_profile` / `provider_delete_profile` / `provider_status` /
   `provider_apply_profile` / `provider_list_backups` / `provider_restore_backup`。
   **DTO 本来就无密钥字段**（档位只存 `env_key` 这个变量名），另有用例断言序列化结果里
   没有任何像密钥的长串。
3. ❌ 前端页面（档位列表 + 当前生效标记 + 切换确认 + 备份还原）：**未做**。
   **仅在 sidebar 形态下可达**（island 的 372×520 装不下）。
4. ✅ 只做 `Tool::Codex`；`scan_tools()` 的用例断言它**只承诺 Codex**，不为别的工具画未实现的 UI。
5. ✅ 用例：原子写失败不留半截文件、坏档位在碰到配置前就被拒、备份-还原逐字节一致、
   备份写不进去时配置一个字节不动（这条**专门分辨「先备份后写」与「先写后备份」**）、
   密钥形状串不出现在 DTO、tool scan 只承诺 Codex。
6. ⚠️ **文案已就位、界面未就位**：`PROVIDER_LIMITATIONS`（能力边界）在 Rust 侧拼好，
   任何一次切换的结果都带它——但今天还没有界面显示它。
7. ✅ 保留注释/顺序/缩进：写入走 `toml_edit`，有用例钉住行内注释与数组缩进；
   用户的额外 provider 键（如 `request_max_retries`）**不删**，也有用例。

**验收**

- 改一个 Codex 档位的配置片段 → 应用 → `~/.codex/config.toml`
  内容正确，且原始内容可从备份还原；生效需重启 Codex 进程并给用户提示
- DTO 里搜不到任何 4 位以上连续密钥形状串
- 只做 Codex 一家，不为其他工具画未实现的档位 UI
- 配置文件的注释与缩写在切换后仍然保留
- 界面上能看到能力边界说明

---

## Phase 3 — ToDos 模块

1. `todos.rs`：`list` / `add` / `toggle` / `remove` / `clear_done`，原子写。
2. 前端：极简列表，回车加条，勾选划掉，无分组无日期。
3. 侧栏图标带未完成计数。
4. 测试：损坏 JSON 不 panic（降级为空清单）、原子写、计数口径。

**验收**：增删改查往返一致；把 json 写坏后重启 App 不崩且列表为空。

---

## Phase 4 — 收尾

1. 文档同步：README / CONTEXT / CHANGELOG。README 里所有「灵动岛」表述从
   「唯一形态」改成「形态之一」，逐条核对，不留已不成立的描述
   （AGENTS.md：过期的限制比没有限制更误导）。
2. `docs/adr/0009-sidebar-alongside-island.md`：记录为何两种形态并存而非二选一、
   各自适用面（island = 不打断注意力的被动感知；sidebar = 键盘可达与信息容量）、
   以及 `shell_mode` 默认 island 的理由。
3. `scripts/scan-secrets.sh --release`，按 AGENTS.md 走 扫描 → commit → tag → release。
4. `docs/code-review/` 写本轮独立 review。

---

## 风险

| 风险 | 影响 | 对策 |
| :--- | :--- | :--- |
| **泻密**：Provider payload 含真实 key | 最高危 | DTO 掩码层做成结构断言（有测试盯着）；密钥只走文件不走 invoke 响应 |
| **写坏用户配置** | 高 | 切换前必备份 + 原子 rename；失败即回滚 |
| **扫描器假绿持续存在** | 中 | Phase 0 已修并有 7 条守卫测试盯住 |
| **双形态导致 CSS / 定位代码翻倍** | 中高 | island 分支零改动；共用组件抽到独立文件；两壳不共根容器 |
| **`views.js` 抽取时破坏 island 首屏** | 中 | 默认 `shell_mode=island`；重构后先在 island 下跑通 544 项再开 sidebar 开关 |
| **CC Switch 工具集扩张失控** | 中 | 第一阶段只做 Codex；其他工具待独立核实 |
| **构建门槛** | 低 | README 前置条件已写 `SDKROOT=MacOSX26.5.sdk` |

## 开放项

1. **macOS 本体迁移方向已定**：[ADR 0010](../adr/0010-swift-freeze-and-rust-prerequisites.md)
   要求前置门齐备后逐模块迁到 Rust；Swift 仅保留 Bug、安全与必要兼容修复。
2. **`shell_mode` 默认值是否要为「新用户默认 sidebar」加版本迁移逻辑**：
   当前一律默认 island，最安全但不能让新用户直接看到侧边栏。
3. **灵动岛形态下 Provider / 待办不可达**：这是有意的形态分工（372×520 装不下），
   但需在文档里写明，否则会被当成缺陷报上来。
4. **是否与 Magpie 共存**：第一版只做配置切换层。做完后再评估让工作台检测/启动本机
   Magpie 并显示各 agent 当前模型（见 [05-magpie-research.md](05-magpie-research.md) §4.1）。
   前提是核实 Magpie 的 CLI 输出是否稳定可解析、配置与令牌位置、端口是否可配——
   **这些尚未核实**，要读它的源码才能定。
5. **若将来要做网关**：那是独立的大改造立项，不是当前 Phase 的延伸。
   它会让本产品从「监控 Agent」变成「转发 Agent 的全部流量」——产品性质变了，
   凭据口径与资源占用预算都要重新论证。

## 待办（来自 OpenSquilla 调研，不阻塞 Phase 1）

见 [06-opensquilla-research.md](06-opensquilla-research.md) §4。

1. **`LocalEventServer` 补 Guest 降权三原则**：认证失败与未认证给同一套权限（不给"差一点的
   凭据"留半开的门）；远端连接不进审批队列（待审批项不能成为提权通道）；
   边界在所有执行面一致生效。这是本项目已知的安全短板
   （`Sources/AgentIsland/LocalEventServer.swift:10-15`：`/notify` 无鉴权、
   不设 `requiredLocalEndpoint` 时实际监听通配）。
2. **脱敏清单补上下文敏感项**：现有规则覆盖密钥 / 本机路径 / 邮箱 / 手机号，
   **不含客户名、项目名、channel 标识**。分享诊断或导出 session 前，这类信息同样要清。
