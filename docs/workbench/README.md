# 跨平台工作台改造 — 总览

> 本目录记录 AgentIsland 从「macOS 灵动岛监控器」扩展为「跨平台轻量工作台」的
> **现状、目标、方案与计划**。它是这一轮改造的入口文档，不是逐版变更日志——
> 逐版记录一律进 [CHANGELOG.md](../../CHANGELOG.md)。

## 先读这一份

[**10-replan-2026-09-26.md · 重划方案**](10-replan-2026-09-26.md) —— 四路调研（代码资产 / 14 篇 UI 案例 /
风险验收 / 竞品边界）之后的重划。它推翻了本目录九份文档里的 9 条地基断言，其中三条最关键：
**`app/` 从未构建成功**（无 target/gen、capabilities 引用不存在的 `main` 窗口、`placement.rs`
macOS 分支硬编码 1440×900）、**「Rust 核心零改动复用」不成立**（它是与 Swift 并列的第二份
独立实现：registry 12 vs 26、会话方言 4 vs 5）、**Phase 0 的测试脚本被 `.gitignore` 挡在库外**。
三个前置阻塞、修正后的目标/方案/Phase、以及 6 个待用户拍板的问题，都在那份里。

然后读 [**07-synthesis.md · 综合方案**](07-synthesis.md) 拿产品定位（这部分仍然有效）：
把用户先后给的多类输入（形态预期、CC Switch、Magpie、OpenSquilla、陈大黄的边缘架构推文、
Emil Kowalski 的「让 AI 搞坏它」）收敛成一个产品方案：
**AgentIsland 是一款跨 Agent 的本机管理工作台——不做 Agent、不架网关、不用 ML 路由。**

> ⚠️ 07 及之前文档里的**技术断言**以 10 号为准；**产品定位**以 07 为准。冲突清单见 10 号 §1。

其余文档按需查阅：

| 文档 | 内容 | 何时读 |
| :--- | :--- | :--- |
| [01-current-state.md](01-current-state.md) | 今天已有什么：代码资产、功能面、口径、技术债（全部核实过） | 动手前先读这份，避免重复实现已有能力 |
| [02-goals.md](02-goals.md) | 目标与非目标：形态、界面、性能要求，以及明确不做的事 | 判断某个需求该不该做 |
| [03-approach.md](03-approach.md) | 技术方案：双形态并存、分层、CC Switch 档位模型、ToDos、性能预算、安全红线 | 写代码前读这份 |
| [04-plan.md](04-plan.md) | 分 Phase 的实施计划、验收标准、风险与开放项 | 排期与验收 |
| [05-magpie-research.md](05-magpie-research.md) | Magpie（LLM 网关）一手调研：它是什么、与我们已有能力的重叠与冲突、三条可选路径的代价 | 决定「模型切换」这块做多深之前必读 |
| [06-opensquilla-research.md](06-opensquilla-research.md) | OpenSquilla 一手调研：harness-native 路由思想 + 四条可搬的经验（诊断回放/权限降权/一代模式名/分享前脱敏） | 补强 `LocalEventServer` 安全、实现 `shell_mode`、或评审降频/缓存类改动前读 |
| [08-chendahuang-cloudflare-research.md](08-chendahuang-cloudflare-research.md) | 陈大黄（@realchendahuang）「独立产品后端全部交给 Cloudflare 边缘」推文：原文 + 它对「脉冲式容量」的处理，以及为什么**我们只借思想、不上云** | 讨论容量/成本/是否自建守护进程类取舍前读 |
| [09-emilkowalski-let-ai-break-it.md](09-emilkowalski-let-ai-break-it.md) | Emil Kowalski「让 AI 尝试搞坏你做的东西」推文：原文 + 视频逐帧核对（1284 条 / 超长姓名 / 异常邮箱 / 布局被撑坏但零报错），以及我们手里比他的 demo 更真实的极端输入 | **写测试或定验收标准前读**——尤其决定要不要给侧边栏/灵动岛做最坏情况压测时 |
| [10-replan-2026-09-26.md](10-replan-2026-09-26.md) | **重划方案**：四路调研推翻的 9 条地基断言、三个前置阻塞、修正后的目标/方案/Phase、6 个待拍板问题 | **任何 Phase 开工前读**；它取代 03/04 里与代码不符的断言 |
| [11-deepchat-another-route.md](11-deepchat-another-route.md) | DeepChat（6343 star / Apache-2.0）一手调研：**另一条路线**（当 agent 的前端），两条路线的边界表 | 防方案写偏；尤其当想给自己加「可回放诊断日志」时先读它 |
| [13-opencode-monitorability-audit.md](13-opencode-monitorability-audit.md) | **被监控对象** opencode（210k star）一手调研：会话/token 落盘位置与字段口径、Desktop App 是不是 CLI 外壳、`plan` 只读 agent 的缺口；**并从源码核出一个已存在的字段名 bug（已修）** | 改任何 opencode 相关解析前必读；装 opencode 做真机验收前必读 |
| [14-semantica-knowledge-graph.md](14-semantica-knowledge-graph.md) | Semantica（13469 star / Python）一手调研：**与我们几乎不重叠**（企业知识图谱 + Rete/Datalog/SPARQL），只记两条纪律 | 只在想加「解释 agent 为什么这么做」类功能前读——它会告诉你那需要什么、我们给不了什么 |
| [15-omarchy-agents-panel-and-toggles.md](15-omarchy-agents-panel-and-toggles.md) | Omarchy（43140 star / Shell / DHH）一手调研：**Linux 侧的对应物**——agents panel（限额百分比 + 按天按模型 token）、静默通知进历史、indicators 的 inactive 隐藏、toggles 是模式不是设置 | 做侧边栏首层与 `shell_mode` 前读；尤其想加"静默/勿扰"前必读（本仓这块整块缺失，已核实） |
| [19-graphify-confidence-and-strict-hook.md](19-graphify-confidence-and-strict-hook.md) | graphify（121455 star / Python / **Apache-2.0**，源码可抄）一手调研：**用机制而非文档保证 agent 行为**的 PreToolUse guard + strict deny、三档置信度（AMBIGUOUS 只由 LLM 产生）、skillgen 片段化生成 13 平台 skill、git hook 嵌入解释器路径 | **想给本仓纪律加机制保障时必读**；判断「要不要给 provenance 加第三档」时必读 |
| [18-vorssaint-pluggable-architecture-audit.md](18-vorssaint-pluggable-architecture-audit.md) | Vorssaint（21355 star / Swift / **GPL-3.0**）一手调研：可插拔三层架构（availability ⊃ enable ⊃ 资源，卸载不删键）、教科书级权限 UX、**一整套 AgentUsage 子系统却只做 claude+codex 两家** | **Phase 1 做侧边栏模块化前必读；Phase 2 做配置授权前必读**；另含一条待拍板：本仓没有 LICENSE 文件 |
| [17-codenotch-direct-competitor-audit.md](17-codenotch-direct-competitor-audit.md) | **最直接的形态竞品** codenotch（2493 star / Swift / macOS）一手调研：26×210pt 黑 pill、17 家 provider、三级回退做到「与 /usage never disagree」、一维栈空间布局；含正面逐条对比与三条该跟进 | **做侧边栏壳（Phase 1）前必读**；遇到「凭据边界画在哪」前必读 |
| [16-mattpocock-skills-meta-specs.md](16-mattpocock-skills-meta-specs.md) | mattpocock/skills（269,700 star）一手调研：**本机 24 个 skill 的上游**。三份元规范中两条对我们有约束力（user-invoked 不可被调用；skill 依赖要指名工具而非相对链接） | 新增或改动任何 agent skill 前必读；判定这条指令该写给谁时必读 |
| [12-monocode-competitor-audit.md](12-monocode-competitor-audit.md) | MonoCode（1303 star / MIT）一手调研：**最近的同形态竞品**（同样 Tauri v2 + Rust）、harness 三层抽象、327 前端测试 + 333 Rust 测试、凭据零落盘 | **Phase 0 建测试基建前必读**；Phase 2 写 `provider.rs` 前必读（两个凭据坑） |

## 一句话

把 AgentIsland 扩成一款**跨平台轻量工作台**：保留灵动岛作为可选形态之一，
新增侧边栏主形态，融合 CC Switch（API Provider 切换）与 ToDos（待办）两块功能；
本机配置文件实扫后第一阶段只做 Claude Code 与 Codex。

（原写的「Rust 核心 2670 行几乎零改动复用」已被 [10 号](10-replan-2026-09-26.md) §1 推翻：
那 2670 行是**与 Swift 并列的第二份独立实现**，不是共享层。行数对，判断错。）

## 四个已拍板的决策

1. **原地改造**，不新建仓库。
2. **保留 `AgentIsland` 这个名字**，不改成 Bench 类命名。依据见 [02-goals.md](02-goals.md)。
3. **灵动岛与侧边栏并存、可切换**，灵动岛不是退役而是降级为可选形态。
4. **技术栈沿用 Tauri v2 + Rust 核心**，Windows WPF 端不承担新形态。
   （原括注「`app/` 已跑通」不成立：[10 号](10-replan-2026-09-26.md) §2 阻塞一实测无 target/gen、
   capabilities 引用不存在的 `main` 窗口、`placement.rs` macOS 分支硬编码 1440×900、本机无默认 rustc。
   **让 `app/` 构建成功是 Phase 1 的前置门，不是已完成事实。**）

**v0.0.142 六项已拍板**（详见 [10 号](10-replan-2026-09-26.md) §7）：① 装 Rust 工具链让 `app/` 构建成功
② Phase 0 不发版、与 Phase 1 一起发 ③ 档位元数据明文 + 凭据进钥匙串 ④ Codex 用原生 profile-v2
⑤ reduce-motion 与彩色存量维持 Phase 4 ⑥「Codex 需重启」只读检测 + 提示，不 kill 进程。
另**范围收窄：Phase 2 只做 Codex 一家，不做 Claude Code**（用户不用；且本机其 `settings.json` 无 `env` 块）。

## 尚待拍板

~~**「模型/Provider 切换」做多深。**~~ **已定：先只做配置切换，之后评估与 Magpie 共存。**

用户提出融合 [Magpie](https://usemagpie.ai/)，调研后发现它不只是配置切换器而是**本机 LLM 网关**
（拦全部请求、翻译三种 API 协议、多账号调度），与原计划的 `provider.rs` 差一个数量级，
且与「低资源占用」目标直接冲突。三条路径的代价对比见
[05-magpie-research.md](05-magpie-research.md)。

第一版只做配置切换层（`provider.rs`），不架网关。**必须接受它的局限**：agent 只说自家协议
（Codex 只讲 OpenAI Responses，Claude Code 只讲 Anthropic Messages），不架网关就没有路径把
DeepSeek/Kimi 的 key 接到 Claude Code 上——所以「配置切换」实际能切的是**同一厂商的多套
key/账号**，跨厂商模型这条路走不通。这一点要写进用户可见的界面说明，不能让人以为切了就能用。

## 进展

| Phase | 内容 | 状态 |
| :--- | :--- | :--- |
| 0 | 修脱敏闸门假绿（`scan-secrets.sh`）+ 补 7 条守卫测试 | ⚠️ 部分完成：扫描器修复已交付，但**测试托管与检出能力没完成**（见 10 号 §2 阻塞三） |
| 0' | 建可信地基：`test-scan-secrets.sh` 入库 + Rust 测试基建 + `app/` 能构建打包 + `dist/` 产物扫描 | **待开工**（用户已拍板：不发版，Phase 1 完成后一起发。第一件事 `rustup default stable`） |
| 1 | `shell_mode` 设置项 + 侧边栏壳 + 双形态并存 | 未开始，**前置条件见 10 号 §5** |
| 2 | **Codex 配置档位**（v0.0.142 收窄：只做 Codex 一家，不做 Claude Code） | 未开始，方案已按 CC Switch 一层「通用配置片段」重写；生效语义按「Codex 需重启进程」设计 |
| 3 | ToDos 模块 | 未开始 |
| 4 | 文档同步 + ADR + 脱敏扫描 + 发版 | 未开始 |
