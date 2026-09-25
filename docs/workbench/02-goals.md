# 02 · 目标与非目标

## 1. 目标

把 AgentIsland 从「macOS 灵动岛监控器」扩展为**跨平台轻量工作台**，
在不丢失既有能力的前提下融合三块功能：

| 来源 | 内容 | 入口 |
| :--- | :--- | :--- |
| **CC Switch** | AI 编程工具的 API Provider 多配置保存与一键切换 | 侧边栏「Provider」 |
| **ToDos** | 待办清单 | 侧边栏「待办」 |
| **AgentIsland 本体** | 本机 Agent 运行状态 + token 消耗监控 | 侧边栏「监控」 |

> **Provider 这一项的能力边界**：只改配置文件，**不做 LLM 网关**。它切的是**同厂商的
> 多套 key/账号**，不能让 Claude Code 用上 DeepSeek/Kimi。原因与代价见
> [05-magpie-research.md](05-magpie-research.md)。

### 1.1 形态与定位

- **侧边栏为主形态**，功能覆盖面较广，但整体保持轻量
- **首层极简**，通过「高级设置」进入详情页——入口简单，内部信息丰富
- **低系统资源占用**是持续约束（有预算表），不是第一版的优化专项

### 1.2 性能要求

用户原话：「目前先按此方案推进开发，待完成后再评估资源占用的优化空间。」

所以本轮的定位是：**不设资源优化专项，但所有新增模块必须遵守性能预算**，
否则后续优化是在还债。预算表见 [03-approach.md](03-approach.md) §5。

## 2. 命名：为何保留 `AgentIsland`

这一条单独成节，因为它曾差点被改错。

`AgentBench` 这个名字（以及任何 Bench 类命名）**不可用**，有三个独立理由：

1. **已被占用两次。**
   - THUDM / [arXiv 2308.03688](https://arxiv.org/abs/2308.03688)
     《AgentBench: Evaluating LLMs as Agents》——8 个环境的 LLM-as-Agent 评测基准，圈内知名。
   - **npm `agentbench`** 已存在，描述是 *"Lighthouse for AI coding harnesses.
     Benchmark your Claude Code setup and get a score out of 100"*。

2. **第二条占用是撞功能，不只是撞名。** 那个 npm 包干的事——给 Claude Code 配置打分、
   满分 100——正是本项目 `doctor`、审计报表、健康评级、可观测性五类结论
   （见 [01-current-state.md](01-current-state.md) §3.3）在做的事。

3. **语义与产品相反。** 本项目一次跑分都没有；它做的是状态感知
   （`offline`/`idle`/`working`/`attention`/`completed` + token 净消耗）。
   在 AI 语境里 "Bench" 只会被读成「给 agent 打分的跑分台」，把用户预期带向反方向。

附带理由：保留 `AgentIsland` 让 38 个版本的品牌资产（GitHub 仓库、Releases、`site/` 主页）
继续有效，也保住了它自洽的空间隐喻体系（灵动岛、四边停靠、微细条、`docked`/`expanded`、`peek`）。

## 3. 形态决策：双形态并存，而非二选一

`AgentIsland` 这个名字的隐喻、38 个版本的品牌资产、以及「不打断注意力」这一核心价值
都绑在灵动岛形态上。删掉它，名字与资产一起失效。

而侧边栏带来的是**键盘可达性**与**信息容量**——这是灵动岛（372×520、`focus: false`）
给不了的。两者不可互相替代。

→ **并存，设置里切换。** 灵动岛不是「退役」，是降级为「可选形态之一」，代码不删除。

## 4. 非目标（第一版明确不做）

| 不做 | 原因 |
| :--- | :--- |
| 新建仓库 | 原地改造（已确认） |
| Windows WPF 新版 | `windows/` 保留作既有功能偏离对照，不承担新形态 |
| CC Switch 全九工具 | 本机只扫到 2 家有配置文件；给没装的东西做 UI 是空转 |
| **LLM 网关（协议翻译）** | 数千行 Rust、改变产品性质、与低占用目标直接冲突。用户已确认第一版不做，见 [05-magpie-research.md](05-magpie-research.md) |
| 重构 Rust 核心 | 2,100 行与形态无关，改了只增风险 |
| 删除灵动岛任何代码 | 双形态并存，island 走「可选形态」路径 |
| 资源占用优化专项 | 用户口径：先做完再评估 |
| 改动 `Sources/`（macOS 本体） | 本轮不碰；其形态口径问题列为开放项 |

## 5. 成功标准

第一版交付时，下面每一条都必须成立：

1. 老用户升级后首屏**零变化**（默认 `shell_mode=island`）。
2. 切到侧边栏后：首层极简，`高级设置` 能进到既有分析页与各 Agent 详情，功能不退化。
3. 既有 544 项 Swift 测试全绿，Rust 端新增测试覆盖原子写与密钥掩码。
4. CC Switch 能保存 / 切换 / 还原 Claude 或 Codex 的配置，密钥**零泄漏**到 DTO 与日志。
5. 待办增删改查可用，配置文件被写坏后降级为空清单而非崩溃。
6. 性能预算表每一条都达标（见方案文档 §5）。
7. 文档口径一致：README / CONTEXT / CHANGELOG / ADR 不留下已不成立的描述
   （AGENTS.md：过期的限制比没有限制更误导）。
