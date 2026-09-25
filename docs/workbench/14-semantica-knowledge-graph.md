# Semantica：企业知识图谱层（13469 star）——基本不重叠，但有两条纪律可借

> 目的：[semantica-agi/semantica](https://github.com/semantica-agi/semantica)（GitHub API 2026-09-26：
> 13469 star / 1521 fork / **Python** / MIT / 创建于 2025-06-25 / 90 open issues / homepage getsemantica.ai）。
> 自我描述 "Graph-Native Infrastructure for Context and Accountable AI Systems"。
> 本文先说清它**与我们的定位差多远**，再只记真正可借的两条纪律。
> 依据为 README 全文（60438 字符）与 GitHub API，**源码未读**——见文末。

## 一句话结论

**与我们几乎不重叠：它是给企业做知识图谱 + 因果推理 + 合规审计的基础设施（Rete / Datalog /
SPARQL / RDF / PROV-O / OWL / SHACL / SKOS），我们是读本机会话尾、统计 token 的监控器。**
值得记的只有两条**工程纪律**：① 它把"决策 provenance"做成**从数据结构里自然掉落的东西**而不是一个产品卖点；
② 它敢于在 README 里写明"这个引擎这一版就是简单的，接生产合规闸前请自己验证"。

## 它是什么（README 原文口径）

> Most AI agents run on embeddings, not meaning: similarity scores with no structure, no relationships,
> and no way to explain why a result came back.
>
> Semantica is the semantic/context layer underneath your LLM, vector store, and agent framework:
> **deterministic infrastructure** (no LLM required for graph construction, reasoning, or provenance;
> where an LLM is used, it's optional and vendor-neutral …) that turns fragmented enterprise data into a
> structured, queryable Context Graph and knowledge graph …

能力域（README 标题行）：Context Management · Knowledge Modeling · Deterministic Reasoning ·
Ontology Management · Decision Intelligence · End-to-End Traceability；
存储立场 Polyglot Graph Storage · RDF & LPG Support · W3C Standards · Interoperable；
目标域 "Built for High-Stakes, Regulated Domains"。安装 `pip install semantica`，自检 `semantica doctor`。

## 与我们的边界（为什么大概率用不上）

| | Semantica | AgentIsland |
| :--- | :--- | :--- |
| 数据源 | 企业数据（合同 PDF/DOCX、SQL、Databricks/Snowflake、live web） | 本机会话文件与进程表（只读） |
| 核心资产 | 知识图谱 + 本体（OWL/SHACL/SKOS）+ 时序事实 | 五态状态机 + 五种会话方言解析 + token 统计 |
| 推理 | Forward Chaining / Rete / Datalog / SPARQL | 无推理，只有规则判定（CPU 阈值、写入窗口） |
| 合规 | PROV-O 导出、决策因果链、bi-temporal facts | 「读不到 ≠ 闲着」的可观测性五类结论 |
| LLM 关系 | LLM 是可选组件（图构建与推理**不需要** LLM） | **完全不碰 LLM**（不发起任何模型请求） |
| 规模 | Python 服务/库，13469 star | SwiftPM 应用，97 文件 29526 行 |

**它不是我们的依赖、不是我们的竞品、也不是我们的参照实现。** 若在企业场景里要"解释 agent 为什么
做了某个决定"，它是对的工具；我们的场景是"agent 此刻在干什么、花了多少"，两个问题不相通。

**一条具体的「不要做」**：不要因为它有 Context Graph 就给自己加"知识图谱"或"决策链"模块。
我们的会话尾是**别家 agent 写的日志**，连字段都可能缺（README 已知限制载明未知格式会降级），
在它上面建图谱等于把不确定性当结构化输入。

## 可借的两条纪律

### 1. 「Decision provenance 是副产品，不是产品」

README 原话（这条措辞本身值得抄）：

> Decision provenance and audit trails **aren't the product**. They **fall out of that structure for free**,
> and in domains a regulator can question, the same structure that makes your agent smarter also gives you
> a straight answer to "why."

对照我们：`Models.swift` 的 `provenance` 枚举（`observed` / `inferred` / 缺省）与
「自报说 X，进程表说 Y」双显，**已经是这个形态**——可信度不是额外加的一层标签，
是同一个状态机的自然产出。README 已知限制里那几条（`/notify` 只信到令牌、pid 不能建立可信度）
也是同一取向：**把"我不知道"做成结构的一部分，而不是事后补一句 disclaimer。**

→ 可迁移的不是技术，是**这条判断标准**：新加一个能力时问一句——它的可信度信息是长在数据结构里，
还是我在外面贴的标签？贴标签的那个一定会过期。

### 2. 在 README 里写「这一版它就是简单的」

全文只有一处自陈 limitation（`README.md:545`，ReteEngine），但写得极干净：

> Current limitation: ReteEngine's alpha-node condition matcher is **intentionally simple** in this
> release — **validate match_patterns() output against your actual rule set before wiring it into a
> production compliance gate**; more selective condition evaluation is on the roadmap.

三点都值得学：① 点名具体是什么简单（alpha-node 条件匹配）而不是"可能有 bug"；
② 给出**用户侧动作**（接生产闸前自己验证输出）；③ 说清 roadmap 上有没有。
对照我们的写法：README 已知限制条（如 `/notify` 无鉴权、`/state` 令牌校验只到形态）
与 CHANGELOG 的「没能核实的」节都是这个路子——**但可以更严格一点**：
Semantica 那条限定的适用场景是"接进生产合规闸之前"，我们的条目里凡涉及"做前先读代码"的
也该写清是哪个场景下必须先验证。

## 一条与我们无关但记录了的事实

`semantica doctor` 这样的自检命令（README 说 5 秒验证安装）与我们的
`agentisland doctor` / `--selftest` / `--probe` 是同一种用户入口设计。
我们没有可改的，只是记下同类项目都收敛到了这个形态。

## 没能核实的

- **源码一行未读**。本文只依据 README（60438 字符）与 GitHub API。
  "deterministic infrastructure" 这个自述**是 README 的话，不是我们验证过的**
- **实际成熟度未评估**：13469 star / 90 open issues 看起来健康，但我们对它没有任何使用经验
- **Python 实现的质量、测试规模、CI 一概未看**（不像 MonoCode 那次逐文件核实过）
- `semantica doctor` 未运行（本机未 `pip install`，不装）
- 它的 Polyglot Graph Storage 具体支持哪些后端，README 提到了 Databricks/Snowflake 的 extra，
  但完整列表未逐一核实
- README 说"no LLM required for graph construction, reasoning, or provenance"——
  这是它的核心卖点之一，但我们**没有验证这个声明**

## 取证命令

```sh
# 仓库元数据
curl -sSL https://api.github.com/repos/semantica-agi/semantica | python3 -c "
import sys,json; d=json.load(sys.stdin)
print({k:d.get(k) for k in ['stargazers_count','forks_count','language','created_at','pushed_at','open_issues_count']})"

# README 全文（本文所有引文出处）
curl -sSL https://raw.githubusercontent.com/semantica-agi/semantica/main/README.md

# 只取那一处自陈 limitation
curl -sSL https://raw.githubusercontent.com/semantica-agi/semantica/main/README.md | grep -n 'Current limitation'

# 本机直连不通时走系统代理
scutil --proxy   # 127.0.0.1:10808
```
