# 29 · Agent 汇总与提示词能力：外部开源扫描与功能聚合方案

> 状态：**§6 的三项已由用户 2026-10-14 拍板（见 §0.1），问题 4 仍待定**。
> 本文只做扫描与方案，不改产品代码。
> 2026-10-14 复核注：本文是并行会话材料，其“四态已定”与本会话 [ADR 0039](../adr/0039-feature-plan-and-monitoring-kernel.md)冲突，不能自动合并为当前规格；保留原记录，冲突见 [梳理报告](../agent/project-document-audit-2026-10-14.md)。源码已具备提示词管理、任务人工关口及本机事件入口，文中的“新增/尚未”须先对照存量。Star、许可证、同名与市场数据本轮未重新向上游核实，不能只凭本文断言已确认事实；原始取证应补可复查响应或固定快照。
> 落笔时点：2026-10-14。基线 `07c04ec` / `v0.0.322`（工作树含 v0.0.323 未提交改动）。
> 上游：本文是 [28 号](28-feature-replan.md) 的**增补**，不是替代。28 号回答「Magpie 与 Todo S
> 做到哪一档」，本文回答「除了它们，还有谁，以及把谁的能力拿进来」。
> 取证方式：GitHub REST API（`/repos/*` 与 `/git/trees`）+ 三个仓库的本地浅克隆
> （`/tmp/aiagg/`，commit 见 §7）。**未安装、未运行任何上游应用。**

---

## 0.1 已拍板的三项（2026-10-14）

| 问题 | 决定 | 落点 |
| :--- | :--- | :--- |
| §6 问题 1 · 命名 | **保留 `AgentIsland`，先不动** | 长期口径已沉淀 [CONTEXT.md](../../CONTEXT.md)「产品命名」；冲突登记保留在本文 §3.1。**这是「先不动」不是「确认无冲突」** |
| §6 问题 2 · 提示词管理 | **做 M4，先接 Claude Code 与 Codex 两家** | 范围见 §5.3 的 S3'；横向扩到其余 Agent 需另立一轮，逐家核实文件名与生效语义 |
| §6 问题 3 · 灵动岛形态 | **采用四态，并落成编号规则** | 已写入 [24 号](24-interface-quality.md)「灵动岛四态几何与编号行为规则」，R1–R12 成为门禁依据 |
| §6 问题 4 · 本地事件服务 | **未拍板** | 影响 M6/M7。取「不接受」则 M6 退回「逐家薄适配 + 无钩子只读」，覆盖面显著收窄 |

**⚠️ 24 号 §1 的 `p` / `c` / `e` 三个几何值留空待定。** 参照项目的偏移量按它自己的刘海尺寸
算出，直接沿用会错位；定值时须同时处理无刘海屏与缩放/旋转显示器的逻辑坐标换算。


---

## 0. 先给结论

> **起草后自查更正了两处基线错误**，先看这里再看正文：
> （a）本文原以 28 号「待拍板」为基线，实际 28 号已定案为 **ADR 0039**；
> （b）本文原把 M4/M6/M7 当作「待新建」，实际**提示词写入（ADR 0034）、本地事件服务
> （`webhook.rs`）、Claude Code hook 管理均已实现**。下文相关小节已逐条标注更正。

**① 名字已经被占了，而且不止一个。** 与 AgentIsland 同形态（macOS 刘海/菜单栏 + Agent 状态）
的开源项目至少有 6 个在活跃维护，其中 `stablyai/orca` 已有 **92,143 star**。用户已拍板
**保留 AgentIsland 先不动**，但这是「先不动」不是「确认无冲突」（§3.1）。

**② 外部扫描的真实增量，比起草时估计的小得多，也更具体。**
本仓的提示词写入、本地事件服务、Claude hook 管理都已存在。外部项目比本仓多的只有：
**档位预设切换、手改回填、跨副本一致性门禁、多宿主规则文件分发**（提示词侧），
**请求体上限、恒定时间 token 比较**（服务侧）。**没有一条需要新建子系统。**

**③ 但「同一套能力分发给 N 个宿主」这件事本身仍被大规模验证。** Ponytail **163,254 star**
（纯文本规则集、零运行时）与 CC Switch **144,189 star**（仓库内独立 Prompts 模块）
两个独立信号说明需求真实存在，且正对本项目定位里的「同质」。**价值在机制，不在功能条数。**

**④ 最有价值的是两条可验收契约，不是功能清单。** Ponytail 的
「真源 + 逐字比对 + 不可比时用承重短语 canary」给「25 家一套口径」提供了机器可守的判据；
Coucou 把状态机写成**编号行为规则**而非描述性文字，让「什么时候打扰」成为可测项。
后者与 ADR 0039 冲突，见 §0.2。

**⑤ 不要追 Orca 的规模。** Orca 单仓库 **36,786 个文件**，含 cloud/relay/mobile/多 provider
适配与自研 worktree 调度。它做的是「执行侧编排 IDE」，AgentIsland 做的是「监控侧工作台」。
规模不可比，可比的是它的三条信号（§3.2）。

**⑥ 本仓本地服务的安全基线高于参照项目。** `webhook.rs` 已有回环绑定、token 鉴权、
`0o700`/`0o600` 私有权限、两端 token 互通、端口冲突退场记录——且注释里留着
「先启动的占住、后启动的静默失效，而请求还会正常返回，只是答话的是另一个进程」这类真实教训。
外部项目在此只贡献了两个具体缺口（§2.4）。

---

## 1. 竞品全景（GitHub API 一手数据，2026-10-14 核实）

> 下表的 star / 许可 / 语言 / 最后 push **全部来自 GitHub REST API 响应**，
> 不是搜索引擎摘要。搜索引擎给出的数字大量失真，见 §1.1。

### 1.1 先说数据可靠性：不要信搜索结果里的数字

同一次检索里，各站点对 Orca 的 star 说法分别是 **53k / 4486 / 14.3K / 52,966**，
GitHub API 的真值是 **92,143**。对 `agent-isle` 的说法在 **38 / 44 / 4486** 之间摆动
（最后一个是把 Orca 的数字串行了）。

检索结果里还混进了若干**机器生成的内容农场**（`myaiexp.com`、`agents-report.net`、
`launchpad.ms`、`tolo.ro`、`almighty-literacy.myshopify.com` 等），它们会以新闻口吻
复述 release note 并附引号人名。**这些来源一律只作线索，不作证据。**

> **可迁移规则**：任何要写进文档的项目指标，只接受 GitHub API 响应；
> 搜索引擎与聚合站只用于「发现有哪些项目」，不用于「这个项目是什么」。

### 1.2 A 类：Agent 状态汇总 / 编排（正面竞品）

| 项目 | 仓库 | ★ | 许可 | 语言 | 最后 push | 一句话 |
| :--- | :--- | ---: | :--- | :--- | :--- | :--- |
| **Orca** | `stablyai/orca` | **92,143** | MIT | TypeScript | 2026-10-14 | 多 Agent 并行编排 ADE，worktree 隔离，桌面+移动+远程 |
| **CodexBar** | `steipete/CodexBar` | **22,484** | MIT | Swift | 2026-10-14 | 菜单栏用量统计，只支持 Codex + Claude Code |
| **Omnigent** | `omnigent-ai/omnigent` | **10,805** | Apache-2.0 | Python | 2026-10-14 | Agent 元框架，换 harness 不换上层 |
| **Coucou** | `Louis-CFM/coucou` | **5,163** | MIT | Swift | 2026-10-14 | 刘海角色 Mochi，macOS + Windows + Linux |
| **MioIsland** | `MioMioOS/MioIsland` | 539 | 无声明 | Swift | 2026-10-13 | macOS 刘海，监控/批准/跳转 |
| **Agent Isle** | `DevLab-Technologies/agent-isle` | 44 | MIT | Swift | 2026-09-09 | 刘海，**无钩子**读各工具自己的会话历史 |

调研库已覆盖、本次不重复取证：`codenotch`（[17 号](17-codenotch-direct-competitor-audit.md)）、
`Runnock`（调研库 [ui/50](../../../../03-行业调研/ui-research/ui/50-runnock-notch-status-island-and-a-four-way-data-split.md)）。

长尾（均 <30 star，本轮不单独立项，仅登记）：`jwintz/pi-island`(29)、
`yazelin/AgentPulse`(5, Rust)、`esaradev/hermes-island`(4)、
`grandfinale009-creator/open-island-windows`(2, Rust, Windows 侧) 等。

### 1.3 B 类：配置与路由（28 号已覆盖）

| 项目 | 仓库 | ★ | 许可 | 语言 |
| :--- | :--- | ---: | :--- | :--- |
| CC Switch | `farion1231/cc-switch` | 144,189 | MIT | Rust |
| Magpie | `yetone/magpie` | 12,513 | MIT | Go |

### 1.4 C 类：提示词 / 规则集（**本仓已有基础，外部多出四个能力**）

> 起草时把本类写成「28 号完全未覆盖的空白」。**更正**：本仓已有
> [ADR 0034](../adr/0034-native-prompts-and-user-instructions.md) 与 `prompts.rs`，
> Codex 与 Claude Code 的写入路径可用。外部项目多的不是功能条数，是**四个具体机制**（§2.1）。

| 项目 | 仓库 | ★ | 许可 | 语言 | 定位 |
| :--- | :--- | ---: | :--- | :--- | :--- |
| **Ponytail** | `DietrichGebert/ponytail` | **163,254** | MIT | JavaScript | 一套 YAGNI 规则集，分发给 20+ 宿主 |
| CC Switch · Prompts | 同上仓库 | （含在 144k 内） | MIT | Rust | 提示词预设 + 与 live 文件双向同步 |
| **Keysmith Switch** | `Jia-Ethan/keysmith-switch` | 314 | **PolyForm Noncommercial 1.0.0** | Rust | Tauri 2 桌面提示词管理器 |
| Agent Switch | `qunqin24/agent-switch` | 1 | MIT | Rust | CC Switch 的第三方衍生 |

**许可证警告**：Keysmith Switch 是 **PolyForm Noncommercial**（禁止商业使用），
AgentIsland 是对外发布的产品。**该仓库只能读设计，不能复制代码或移植实现。**
这类「同技术栈但许可证不可用」的案例应写进选型判据（§5.2）。

---

## 2. 可迁移能力清单

每条标注证据层级：**源码级**（读过本地克隆）/ **API 级**（GitHub API）/ **文档级**（README 或产品文档）。

### 2.1 M4 · 提示词与规则集管理（**基础已实现，缺四个具体能力**）

> ⚠️ 起草时把 M4 写成「新增模块」，且拍板口径是「先接 Claude Code 与 Codex 两家」。
> **核实后更正**：这两家的写入路径**已经实现**——[ADR 0034](../adr/0034-native-prompts-and-user-instructions.md)
> （2026-10-04）与 `app/src-tauri/src/prompts.rs`（772 行）。该模块已有 `Client` 枚举（含 Codex +
> Claude Code）、`Draft` / `Prompt` / `List` / `Preview` / `Applied` / `Backup`，
> 三档容量上限（正文 32 KiB / 文件 64 KiB / 库 512 KiB），预览绑定库·来源·覆盖文件版本，
> 原子替换、读回、备份与还原，非空 `AGENTS.override.md` 优先时拒绝写入。
> **M4 不是从零做提示词，是补下面四个缺口。**

本仓现状核对（源码级，`prompts.rs` 全文检索）：

| 能力 | 本仓现状 | 证据 |
| :--- | :--- | :--- |
| 提示词库 CRUD + 预览 + 原子写 + 备份还原 | ✅ **已实现** | `prompts.rs:21-77` 结构体齐全 |
| Codex / Claude Code 写入 | ✅ **已实现** | `Client` 枚举；ADR 0034「适配扩展」已覆盖 Claude Code |
| **档位 / 预设切换** | ❌ `profile` / `preset` 命中 0 | 无多套预设与单激活语义 |
| **智能回填（手改检测）** | ❌ `backfill` / `drift` / `manual_edit` 命中 0 | 切换/应用时不知道用户是否手改过 live 文件 |
| **跨副本一致性门禁** | ❌ 无 | 无法证明「多宿主副本没有漂移」 |
| **非提示词类规则文件分发** | ❌ `cursor` / `windsurf` / `qoder` / `copilot` 命中 0 | 只覆盖两类提示词主文件，不覆盖 `.cursor/rules/` 这类规则副本 |

CC Switch Prompts 的机制（文档级，描述具体到代码位置）：

| 能力 | 上游机制 | 对 AgentIsland 的增量价值 |
| :--- | :--- | :--- |
| **提示词档位** | 每 app 一组预设，同一时刻只激活一个，激活即写入 live 文件 | **缺口 1**：本仓是单条提示词，无多套切换。执行侧的「档案/配置档位」可复用同一套存储与备份流程 |
| **智能回填（backfill）** | 切换前先读 live 文件；与库中不同说明用户手改过 → 先存回当前预设，再切换 | **缺口 2**：Magpie 的 `bypassed` 依赖其网关，本仓拿不到；但**文件级手改检测 + 不丢用户手改**完全可用，且正对 ADR 0034 里「用户可能直接改 `~/.claude/CLAUDE.md`」的现实 |
| 激活的预设不可删除 | 避免 app 突然没有有效 system prompt | 随缺口 1 一并考虑 |
| 首次启动自动导入 | 把现有 live 文件导入为第一条预设 | 随缺口 1 一并考虑 |
| SSOT + 双向同步 / 原子写 | 预设存库，md 文件是投影；写用原子替换 | **已实现**，不需要 |

来自 Ponytail（**源码级**，commit `2961b52`）：

| 能力 | 上游机制 | 对 AgentIsland 的价值 |
| :--- | :--- | :--- |
| **单一真源 + 副本一致性门禁** | `AGENTS.md` 为 canonical，7 个宿主副本（`.cursor/rules/`、`.windsurf/`、`.clinerules/`、`.qoder/`、`.agents/`、`.github/copilot-instructions`、`.kiro/steering/`）由 `scripts/check-rule-copies.js` **逐字比对，drift 即失败** | **直接命中「同质」定位**。「25 家同一套口径」目前靠人工纪律；上游证明它可以被机器守住 |
| **不能字节比时的降级：canary 不变量** | `SKILL.md` 比 `AGENTS.md` 长，无法字节相等，于是定义 9 条承重短语（如 `'validation at trust boundaries'`、`'security'`、`'accessibility'`）要求两边逐字出现 | **这是本轮最值得抄的一条**。给「同质」提供了可执行的判据，而不是口号 |
| **按需注入而非全量常驻** | `PreToolUse` matcher `task\|Task` 只在**派子 agent 时**注入规则 | 与 AgentIsland「重模块进入页面后才加载」同构，可作佐证 |
| **配置优先级链** | 会话内 `/ponytail lite\|full\|ultra\|off` > 环境变量 `PONYTAIL_DEFAULT_MODE` > `~/.config/ponytail/config.json` > 默认 `full` | 四级优先级的成熟形态，可直接参照 |
| **零配置默认** | 不装配置文件也能工作 | 与 AgentIsland「不要求安装 CC Switch/Magpie 即能用」的既有口径一致 |
| **`ponytail:` 注释标记 + `/ponytail-debt` 收割** | 主动放弃的简化写成注释，由命令批量收割成账本 | 小而实用，可作任务流的一个轻量入口 |

**标准层事实**：`AGENTS.md` 是 Linux Foundation **Agentic AI Foundation** 的开放标准
（2025-12 由 OpenAI / Anthropic / Block 共同发起）。这意味着「统一提示词源」不必自己发明标准，
直接对齐 `AGENTS.md` 即可。

### 2.2 M5 · 灵动岛判据（⚠️ **形态部分与 ADR 0039 冲突，未裁决**）

> 本节起草时把 28 号的两个问题当作「悬案」，**实际已由 ADR 0039 §1 定案，且结论相反**
> （保留窄条 / 不新增档位）。规则文本与打扰判据仍有价值，但**不得直接作为实施依据**，
> 详见 §0.2 与 §4.1。24 号已由并行会话加注同一冲突。

来自 Coucou `docs/SPEC.md`（**源码级**，commit `1456bd3`）。它把状态机写成**编号的十条行为规则**，
而不是描述性文字——这正是本项目缺的「可验收判据」形态。

| 规则 | 内容 | 对 AgentIsland 的价值 | 与 ADR 0039 |
| :--- | :--- | :--- | :--- |
| 四态几何契约 | `hidden`（完全不可见）→ `peek`（刘海 +64pt，鼠标悬停即露头）→ `compact`（+104pt，次要 Agent 收进 2×2 网格）→ `expanded`（固定 640pt 宽） | 把「窄条」从单形态改成状态机，次要 Agent 在 compact 态用网格承载而不挤占主位。**具体偏移量按其自身刘海尺寸算，不可照搬** | ⛔ **直接对立**：ADR 0039 定「保留窄条、不新增档位」 |
| 规则 6 · 注意力离开降级 | **3 分钟无鼠标移动 → `hidden`，即使有任务在跑**；首次移动回来 → `compact` | 把「完全不打扰」从口号变成可测判据。CONTEXT 外发一节要的不正是这个 | ✅ 可相容（与形态无关） |
| 规则 7 · 告警独占 | **权限 / 提问 / 错误告警时，即使人不在也自动打开，且不自动关闭，直到被回答** | **按事件类型分级**——普通状态不打扰，告警独占且不超时。**但关键在「自动展开」是否被算作「抢焦点」：ADR 0039 定义抢焦点为键盘焦点转移，若展开不夺取键盘焦点则两者可相容** | ⚠️ **需用户定义** |
| 规则 8 · 完成即收 | 完成视图停留 **5.2 秒**后移除任务并收起 | 完成提示有时长上限，避免完成通知堆积 | ✅ 可相容 |
| 规则 9 · 多告警排队 | 同时多条告警进入队列，**一次一个**，按到达顺序 | 避免多个 Agent 同时要人时弹一屏 | ✅ 可相容 |
| 规则 5 · 可见倒计时 | 展开后 60 秒无活动自动关闭；最后 10 秒在底部画一条 160pt→0 的线显示倒计时 | 自动收起对用户可感知，不是「突然消失」 | ⚠️ 依赖展开态；窄条方案下需另定 |
| 规则 10 · 焦点模型 | 大角色 = focus 任务（最后一个告警，否则第一个在工作）；其余为 mini；点击 mini 切换焦点 | 多 Agent 时的「当前该看谁」有确定规则 | ✅ 可相容 |
| 眨眼 / 音效 | 展开时眨眼；`open`/`close` 各一声；28 个手工合成音效（无音频文件） | 动效与声音的**事件驱动**用法，不做无意义的常驻动画 |

### 2.3 M6 · 采集侧双通道：钩子 / 无钩子（**两条通道本仓均已实现**）

> ⚠️ 起草时误把 M6 当作「待新建」。**核实后更正**：本仓已有
> `app/src-tauri/src/webhook.rs`（本地事件入口）与 `claude_hook_config_tests.rs`
> （Claude Code hook 归属、迁移与移除）。**M6 不是新模块，是给已有通道补契约与覆盖面。**

外部项目恰好演示了两条互斥路线，本仓两条都走了：

| 路线 | 本仓现状 | 外部样本 | 机制 |
| :--- | :--- | :--- | :--- |
| **无钩子** | ✅ 会话文件解析（`session.rs` / `filemon.rs` / 各 `sessionDialect`） | Agent Isle | 直接 tail 各工具自己的会话文件，不修改 agent 配置 |
| **钩子** | ✅ Claude Code hook（`claude_hook_config_tests.rs`）+ 本地事件入口（`webhook.rs`） | Coucou、Orca、Ponytail | 工具生命周期钩子 → 本地 socket / 管道 |

**本仓 hook 管理已具备的关键纪律**（外部项目多数没有，值得保留）：
不接管不属于自己的 hook（`unowned_or_duplicated_hook_is_never_adopted_or_removed`）；
移除自有 hook 时保留他人挂在同组的 hook
（`adding_unrelated_hook_to_owned_group_is_preserved_when_ours_is_removed`）；
应用路径变更后可迁移（`owned_hook_can_be_removed_or_migrated_after_application_path_changes`）。

**外部可补的增量**：Coucou 的统一载荷契约——任意工具往 socket 写**换行结尾 JSON**，
加 `coucou_agent` 字段即得自己的标识，名字须匹配 `^[a-z0-9-]{1,24}$`，无效则回落默认值。
本仓 `webhook.rs` 已有 `/event`、`/session` 与 `resolve_session_ids`，但**标识字段与命名校验未统一**。
要覆盖 25+ 个 Agent，钩子不可能逐家写，必须走「统一事件契约 + 各家一个薄适配」。

**Hook 绝不能阻塞 Agent**：本仓 hook 失败时不得让 Agent 卡住。Coucou 的做法可作参照——
桥接走本地 socket，App 不在运行则 hook 立即退出，Agent 退回原生提示。

### 2.4 M7 · 本地事件服务安全边界（**已实现 4/6，仅 2 条缺口**）

> ⚠️ 起草时误把 M7 当作「待新建」。**核实后更正**：本仓 `webhook.rs` 已实现下列多数措施，
> 且部分比参照项目更细。**M7 不是新模块，是补两个具体防护缺口。**

已实现（**本仓自证**，非外部建议）：

| 措施 | 本仓实现 | 位置 |
| :--- | :--- | :--- |
| 回环绑定 | `Server::http("127.0.0.1:{port}")` | `webhook.rs:188` |
| Token 鉴权 | `/session` 需 `X-AgentIsland-Token`；`/notify`、`/event` 显式**不鉴权**并标「外部投递」 | `webhook.rs:7-8` |
| 私有存储 | 目录 `0o700`、文件 `0o600`；注释记录了「写成 0o600 导致目录无执行位、创建不了文件」的旧坑 | `webhook.rs:41-50` |
| Token 长度与两端互通 | 24 字节 / 48 位十六进制，与 Swift `randomToken()` 同长，迁移期两端互相认得 | `webhook.rs:48-50` |
| 端口冲突 | ADR 0013：Swift 41999 / Rust 42000 并存。注释记录了真实故障——「先启动的占住、后启动的**静默失效**，而请求还会『正常返回』，只是答话的是另一个进程」 | `webhook.rs:10-15` |
| 环境变量容错 | `AGENTISLAND_EVENT_PORT` 认不出时**回默认而非 panic**；抽成纯函数是为避开并行测试里 `set_var` 的进程级共享状态 | `webhook.rs:18-23` |

**仍然缺的两条**（这是 M7 的全部内容）：

| 缺口 | 现状 | 建议 | 参照 |
| :--- | :--- | :--- | :--- |
| **请求体无上限** | `webhook.rs:233-235` 用 `read_to_string(&mut body)` 读到 EOF，无任何 `len()` 上限 | 加体积上限并在超限时**丢弃而非截断**（截断会产出半个合法 JSON，语义更糟） | Agent Isle `maxRequestSize = 64 * 1024` |
| **token 非恒定时间比较** | `webhook.rs:275` 为 `token == expected && token.len() >= 32`，普通字符串相等 | 改恒定时间比较，保留 `len() >= 32` 前置检查 | Agent Isle `EventAuthToken.constantTimeEqual` |

其余参照项目的措施（演示模式兜底、私有目录 helper）本仓已有等价或更强实现，**不需要照搬**。

**结论**：本仓本地服务的安全基线高于参照项目。外部扫描在此的贡献是**指认两个具体缺口**，
不是新增一套服务。

### 2.5 Orca 的三条信号（不是功能清单）

只取规模差异给出的判断，不抄功能：

| 信号 | 事实 | 对 AgentIsland 的含义 |
| :--- | :--- | :--- |
| **状态更新是热路径** | `config/scripts/agent-status-hot-path-benchmark.ts` + 独立 vitest 配置 `vitest.agent-status-benchmark.config.ts` | 灵动岛每拍刷新就是 AgentIsland 的热路径。**应建立同类的基准门禁**，而不只测内存与长时稳态 |
| **状态是结构化 feed** | `src/main/native-chat/agent-session-wire/structured-agent-session-status-feed-*.ts`（含 clock/stopping/child-work/last-message/launch-directory 等切片） | 把「状态」拆成可单测的投影切片，而不是一个大对象。与 AgentIsland 的「相关派生字段同进退」一致 |
| **执行侧与监控侧是两种产品** | worktree 调度、内置终端、内置浏览器、diff 审阅、cloud relay | **Orca 不是我们的路线对照，是相邻品类**。28 号「不架网关 / 不当 agent」的定案不用动 |

---

## 3. 三个必须现在说清的判断

### 3.1 命名与定位：AgentIsland 这个名字已被多个项目占据

GitHub 上活跃的同名/近名项目至少三个：

- `agent-isle`（刘海监控，MIT）
- `agent-island`（`tristan666666` 名下，菜单栏 + 刘海，macOS + Windows，MIT，官网 agent-island.dev）
- 本地同名：`diegogontijo/dynamic-island-ai-agents` 构建产物也叫 `AgentIsland.app`

其中第二款宣称 3,000+ 活跃用户。**这意味着 SEO、包名、Homebrew tap、代码托管组织名都已存在或被占用。**
本项目 AGENTS.md 未记录命名决策，本文只登记事实，处置方案待用户拍板（§6 问题 1）。

### 3.2 不追 Orca 的规模

Orca 单仓库 **36,786 个文件**，含 `cloud/apps/relay`、`mobile/`、多 provider 适配、
自研 worktree 调度与内置浏览器/终端。它把「同时跑很多 Agent 并择优合并」当默认场景，
连 fan-out 一个 prompt 给 5 个 Agent 竞速合并都是卖点。

AgentIsland 的定位（[21 号](21-product-positioning.md)）是**监控侧工作台**：知道每个 Agent
此刻在干什么、配置与待办能统一管。把工作台做成 IDE 是放弃定位换规模，28 号的定案不用动。

**但有三条工程信号该抄**（§2.5）。

### 3.3 提示词管理是真需求，不是「顺带做一下」

两个独立信号：

1. **Ponytail 163,254 star**——一套纯文本规则集、零运行时、MIT。
2. **CC Switch 144,189 star** 的仓库里，Prompts 是一个独立模块，有独立文档页与 DeepWiki 条目。

CC Switch 的文档原文把这件事说得很清楚：同一段提示词在 Gemini CLI 里调好了，
换到 Claude Code 就没有了，只能手工复制粘贴、反复重写——**这就是「同一能力 N 宿主」的漂移成本**。

而 AgentIsland 的「同质」定位（25 家同一套五态、同一套口径）目前靠人工纪律维持。
**提示词档位是「同质」最有价值的落点**：它和配置档位共享同一套 UI、同一套备份/回滚、
同一套漂移检测，且用户感知极强。

---

## 4. 与既有定案的关系

> 起草时本节写作「与 28 号方案的关系」，并称 28 号 §4 的两个问题「待用户拍板」。
> **更正**：28 号已定案为 ADR 0039，其 §1 已给出这两问的答案，且与本文 §4.1 的答案**冲突**。

### 4.1 ⚠️ 28 号/ADR 0039 的两个问题已有定案，且与本文冲突

| 问题 | ADR 0039 §1 已定案 | 本文 §2.2 的候选细化 | 关系 |
| :--- | :--- | :--- | :--- |
| 「前台弹出提示」的边界 | 提醒 = 岛内横幅 + 系统通知，**不抢焦点**。「前台可见不等于键盘焦点转移」 | Coucou 规则 7：告警自动展开且不自动关闭直到被回答 | **冲突**。关键在于「自动展开」是否等于「抢焦点」——ADR 的定义是不转移键盘焦点。若展开不夺取键盘焦点，两者可相容；**需用户明确定义，不能由文档推导** |
| 灵动岛窄条放什么 | 保持现有窄条，优化信息层级与几何，**不新增中间展开档位** | 四态 `hidden/peek/compact/expanded`，次要 Agent 在 compact 态进 2×2 网格 | **直接对立**。ADR 0039 §5 已登记：四态与 R1–R12 暂不作为实施依据 |

**这四条规则真正可迁移的部分，即使形态方案不采纳也成立**：把「什么时候打扰 / 什么时候安静」
写成编号且可测的判据（R6「3 分钟无输入降级」、R7「告警不设超时」、R9「多告警排队」、
R8「完成有时长上限」），而不是留在描述性讨论里。**这部分与「窄条还是四态」无关。**

若裁决为「保留窄条」，建议仍采纳上述打扰/静默判据；若裁决为「四态」，
则 24 号的 `p`/`c`/`e` 几何值需重新定，且须同步修订 ADR 0039 §1。

### 4.2 Magpie 范围已定案，本扫描不重开

**ADR 0039 §1 已定案**：Magpie 借鉴 = 配置与模型/接口目录 + 只读协议探测；
**当前不做内建网关和智能路由**。28 号 §4 问题 1（做 M1-a/b/c 哪一档）**已由此关闭**。

本文不重开这个决策，只补一条外部证据：CC Switch **144,189 star** 与 Ponytail **163,254 star**
说明「**统一管理（配置 / 提示词 / MCP / Skills）不需要网关**」这个需求已被大规模验证。
这与 ADR 0039 的结论方向一致，可作为该定案的市场侧佐证。

⚠️ 注意 ADR 0039 §3 同时纠正了一个技术误解，值得沿用：
**协议兼容不等于厂商相同。** 配置可指向支持客户端协议的第三方服务，服务也可能是用户自己的网关；
「不内建网关」不等于「只能同厂商切账号」。AgentIsland 不负责协议翻译，目标协议不兼容时明确拒绝或说明限制。

### 4.3 本文新增的内容（均为缺口补齐，非新建子系统）

| 模块 | 真实内容 | 与既有实现的关系 |
| :--- | :--- | :--- |
| **M4 · 提示词与规则集** | 四个缺口：档位预设切换、手改回填、跨副本一致性门禁、多宿主规则文件分发 | **已有** `prompts.rs`（772 行，Codex + Claude Code）+ ADR 0034；只加能力，不加架构 |
| **M5 · 灵动岛判据** | 编号行为规则（打扰/静默可测）；四态几何**待裁决** | 与 ADR 0039 §1 冲突（§4.1）；判据部分与形态无关，可独立采纳 |
| **M6 · 统一事件契约** | 标识字段与命名校验统一，使「第 N+1 个 Agent = 一个薄适配」 | **已有** `webhook.rs` + Claude hook 管理；只补契约与覆盖面 |
| **M7 · 服务侧两条防护** | 请求体上限、恒定时间 token 比较 | **已有**回环/token/私有权限/端口退场；只补这两条 |

---

## 5. 落地方案

### 5.1 事件契约先行（建议作为 M6 的第一刀）

不要先写第 N 个 Agent 的适配器，先定契约。草案：

- **只读通道**：各 Agent 的会话文件路径 + 方言标识（`sessionDialect` 已有此设计，见 28 号 §2.1）。
  读不到就明确说读不到，不猜。
- **关口通道**：本地 socket，**换行结尾 JSON**，字段 `agent`（须匹配 `^[a-z0-9-]{1,24}$`，无效回落默认）
  + `session_id` + `kind`（`status` / `permission` / `question` / `done` / `remove`）+ `payload`。
  鉴权用 token + 常量时间比较，绑回环地址，体积上限 64 KB。
- **契约版本**：`schema_version` 字段，便于上游工具升级时明确不兼容而不是静默误读。

验收：写一个第三方 Agent 的最小适配（不改 AgentIsland 产品代码，只发 socket 报文），
证明「第 N+1 个 Agent 的接入成本 = 一个薄适配」，而不是 = 一次产品改造。

### 5.2 选型判据要加两条

28 号之外，本次扫描暴露了两条应写进长期判据的：

1. **许可证是硬门槛，不是加分项。** Keysmith Switch 技术栈（Rust + Tauri 2）与本项目几乎一致，
   但 PolyForm Noncommercial 禁止商用——**技术栈匹配度再高也不能移植代码**。
   遇到「同栈不同许可」应立刻停止在实现层借鉴，只取设计意图。
2. **指标只认 GitHub API。** 搜索结果与聚合站的 star 数在同一检索内可差 17 倍（§1.1）。
   写进任何长期文档的数字必须可复现其取数命令。

### 5.3 建议顺序（接在 ADR 0039 §4 之后）

> ⚠️ 起草时写作「先接 Claude Code 与 Codex 两家」，但核实发现两家**已实现**（ADR 0034）。
> 下表已按真实缺口重排。ADR 0039 §4 的建议 1（核实支持矩阵、建立漏报/误报正反例）
> 优先级高于本表全部内容。

| 阶段 | 内容 | 依赖 | 验收出口 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **S0** | 命名与定位处置（§3.1） | — | 决策写入 CONTEXT | ✅ 已完成 |
| **S1'** | M7 两条防护：请求体上限 + 恒定时间 token 比较 | 无（已有回环/token/私有权限） | 有负例：超限请求被**丢弃而非截断**；错误 token 走恒定时间路径 | 可立即做 |
| **S2'** | M5 打扰/静默**判据**（R6/R7/R8/R9） | 与形态裁决无关 | 每条有可执行用例；不预设四态 | 待裁决后定 |
| **S3'** | M4 四缺口：档位预设 → 手改回填 → 副本一致性门禁 → 多宿主规则文件 | 复用 `prompts.rs` 存储与备份 | 不装任何上游工具即可切换档位；手改 live 文件不丢失；副本漂移可检出 | 已定范围（2026-10-14） |
| **S4'** | M6 统一事件契约（标识字段 + 命名校验） | S1' | 接入第 N+1 个 Agent 的成本被实测并记录 | 可立即做 |
| **S5'** | 形态裁决（四态 or 保留窄条） | **用户裁决** | ADR 0039 §1 与 24 号同步修订，二者不留并存 | ⛔ 阻塞 M5 全量 |

S1'/S4' 不依赖任何裁决，**可立即与 ADR 0039 §4 的建议 1 并行**。

---

## 6. 需要用户拍板的问题

**已拍板 3 项，见 §0.1。以下问题 4 仍未拍板。**

1. ~~**命名怎么处置？**~~ ✅ 保留 `AgentIsland`，先不动；冲突事实登记于 §3.1。
2. ~~**M4 提示词管理做不做？**~~ ✅ 做。⚠️ 口径更正：不是「先接 Claude Code 与 Codex 两家」——
   这两家**已实现**（ADR 0034 + `prompts.rs`）。M4 = 四个缺口（§2.1）。
3. ⚠️ **形态与提醒边界与 ADR 0039 冲突，未裁决。** 本会话拍板「四态 + R7 告警不自动关闭」，
   ADR 0039 §1 定案「保留窄条 + 不新增档位」。**二者不能同时成立，见 §0.2。**
   本会话拍板被登记为「原记录」，**当前不作为实施依据**。
4. ~~**是否接受本地事件服务？**~~ ✅ **无需再问——它已经存在**（`webhook.rs`，绑 `127.0.0.1`，
   `/notify` `/event` `/session`，ADR 0013 端口约定）。原问题作废，替换为：
   **是否补齐两条防护缺口**（请求体上限、恒定时间 token 比较）？这两条是纯加固，
   改变的是拒绝行为而非产品能力，代价极小；不做则本地端口仍可被任意本机进程灌入超大请求体。

---

## 6.1 需要用户裁决的冲突（最重要）

**§0.2 的形态冲突必须由用户裁决**，我不自行合并。两条路：

| 方向 | 后果 |
| :--- | :--- |
| 保留窄条（ADR 0039 §1） | 24 号四态节整节撤下，仅保留 R6/R7/R8/R9 四条打扰判据（与形态无关） |
| 采用四态（本会话拍板） | 24 号 `p`/`c`/`e` 几何值重新定，并**必须同步修订 ADR 0039 §1**，否则两处长期口径并存 |

**独立于该裁决可立即开始的**：S1'（两条服务加固）、S3'（提示词四缺口）、S4'（事件契约）。

---

## 7. 证据索引

### 7.1 一手（GitHub API，2026-10-14）

```sh
for r in stablyai/orca DietrichGebert/ponytail Louis-CFM/coucou \
         DevLab-Technologies/agent-isle yetone/magpie farion1231/cc-switch \
         steipete/CodexBar omnigent-ai/omnigent MioMioOS/MioIsland \
         Jia-Ethan/keysmith-switch qunqin24/agent-switch; do
  curl -s "https://api.github.com/repos/$r" | jq -r \
    '"\(.full_name)\t\(.stargazers_count)\t\(.license.spdx_id)\t\(.language)\t\(.pushed_at)"'
done
curl -s "https://api.github.com/repos/stablyai/orca/git/trees/main?recursive=1" | jq '.tree | length'   # 36786
```

### 7.2 本地浅克隆（只读源码，不运行）

| 仓库 | commit | 实际读到 |
| :--- | :--- | :--- |
| `stablyai/orca` | （克隆未完成，改用 API 的 file tree 与文件名） | `agent-status-hot-path-benchmark.ts`、`structured-agent-session-status-feed-*.ts`、`agent-hooks/agent-status-pane-index.ts` 的**存在性与路径**；**未读其源码实现** |
| `DietrichGebert/ponytail` | `2961b52` | `scripts/check-rule-copies.js`（INVARIANTS canary 机制）、`hooks/qoder-hooks.json`、`hooks/` 全部 7 个脚本的行数 |
| `Louis-CFM/coucou` | `1456bd3` | `docs/SPEC.md` §2 四态几何表与 §3 十条行为规则、`docs/AGENTS.md` hook 载荷协议 |
| `DevLab-Technologies/agent-isle` | `b3e7473` | `Server/EventServer.swift`（回环绑定、64 KB 上限、token 常量时间比较）、`Server/EventAuthToken.swift` |

### 7.3 本仓源码核实（2026-10-14，用于更正本文的「新增模块」判断）

| 核实对象 | 命令 | 结论 |
| :--- | :--- | :--- |
| 提示词模块 | `wc -l app/src-tauri/src/prompts.rs`；`grep -iE 'profile\|preset\|backfill\|drift\|cursor\|windsurf\|qoder\|copilot' prompts.rs` | 772 行；四类关键词**命中均为 0** → 库/写入已实现，四个缺口确认存在 |
| 提示词结构 | `grep -E '^(pub )?(fn\|struct\|enum\|const)' prompts.rs` | `Draft` / `Prompt` / `List` / `Preview` / `Applied` / `Backup` / `Client`；容量 32/64/512 KiB |
| 本地事件服务 | `grep -E 'TcpListener\|127\.0\.0\.1\|Server::http' src/*.rs` | `webhook.rs:188` 已绑 `127.0.0.1`，提供 `/notify` `/event` `/session`，ADR 0013 定端口 |
| 服务安全缺口 | 读 `webhook.rs:233-235`、`webhook.rs:275` | ① `read_to_string` 读到 EOF，**无体积上限**；② `token == expected` 普通比较，**非常量时间** |
| Claude hook 管理 | `claude_hook_config_tests.rs` 用例名 | 已有归属/不接管、保留他人 hook、路径迁移三类用例 |

### 7.4 明确未核实

- Orca 的**状态判定实现**（36k 文件仓库**浅克隆失败**，仅见 API 的 file tree 与文件名）。
  其「working / waiting on input / done」如何判定，**本文未取证**，不可当作已知。
- 上述所有项目的**运行行为**。均未安装、未启动。
- `tristan666666/agent-island` 的确切仓库名（`/repos/tristan666666/agent-island` 返回
  Not Found，可能已改名或迁移；官网 agent-island.dev 未核）。
- Ponytail 基准数据的有效性（n=4，作者已自行撤回早期的 80–94% 结论）——本文只取其
  **分发机制**，不取其效果数字。
- 本仓提示词模块**前端**是否已暴露全部后端能力（仅核实了 Rust 侧）——
  「配置存在不证明界面已接」这条纪律同样适用于此。
- 并行会话引用的 `docs/agent/project-document-audit-2026-10-14.md` **在本轮不存在**，
  但已被 24 号与本文引用。**死链，需补文件或改引用。**

### 7.5 调研库与既有文档

- [28 号 · 功能方案重规划](28-feature-replan.md)（本文上游）
- [21 号 · 产品定位](21-product-positioning.md)（「同质」与「不架网关」定案）
- [17 号 · codenotch 竞品审计](17-codenotch-direct-competitor-audit.md)（最接近的形态竞品）
- [24 号 · 界面品质](24-interface-quality.md)（M5 的判据落点）
- 调研库 [ui/41 Magpie](../../../../03-行业调研/ui-research/ui/41-magpie-agent-config-profiles-and-coexistence.md)、
  [ui/42 CC Switch](../../../../03-行业调研/ui-research/ui/42-cc-switch-controlled-config-and-capability-boundaries.md)、
  [ui/44 Todos](../../../../03-行业调研/ui-research/ui/44-todos-task-workflows-and-actionable-inbox.md)、
  [ui/50 Runnock](../../../../03-行业调研/ui-research/ui/50-runnock-notch-status-island-and-a-four-way-data-split.md)
- 契约引用：[ADR 0020](../adr/0020-local-task-attention.md)、
  [ADR 0023](../adr/0023-native-feature-integration.md)、
  [ADR 0029](../adr/0029-source-artifact-content.md)、
  [ADR 0033](../adr/0033-configuration-backup-publication.md)
