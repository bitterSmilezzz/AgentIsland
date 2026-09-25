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
| 0' | 建可信地基：`test-scan-secrets.sh` 入库 + Rust 测试基建 + `app/` 能构建打包 + `dist/` 产物扫描 | 未开始（[10 号](10-replan-2026-09-26.md) 新增，是 Phase 1 的前置门） |
| 1 | `shell_mode` 设置项 + 侧边栏壳 + 双形态并存 | 未开始，**前置条件见 10 号 §5** |
| 2 | 知道 agent 状态的配置切换器（Provider 档位读写/切换） | 未开始，方案已按 CC Switch 一层「通用配置片段」重写 |
| 3 | ToDos 模块 | 未开始 |
| 4 | 文档同步 + ADR + 脱敏扫描 + 发版 | 未开始 |
