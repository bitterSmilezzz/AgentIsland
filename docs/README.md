# AgentIsland 文档索引

> **接手入口**：[`AGENTS.md`](../AGENTS.md)（工作约定，Agent 无关）→ 本文件（文档地图）→
> [`spec/需求.md`](spec/需求.md)（要做什么）→ [`需求状态.md`](需求状态.md)（做到哪了）。

本目录按**用途**分三类，子目录各管一件事。根目录只留本索引与 `需求状态.md`（状态总览），
其余文档一律归入子目录。

---

## 目录结构

```
docs/
├── README.md            ← 本文件：文档地图（先读这个）
├── 需求状态.md           ← 状态总览：已完成 / 在做 / 未做 + 未修 bug（自动生成）
├── spec/                要做什么、做到什么算完
├── testing/             怎么验
├── ops/                 怎么发
├── adr/                 长期决策（编号 ADR，只增不改）
├── agent/               写给 Agent 的约定与历史证据
├── code-review/         每轮改动的独立 review（时间点证据，不改写）
├── workbench/           进行中大改造的作战文档（现状/目标/方案/计划）
├── validation/          历史轮次的 validation 报告（Swift 主线）
├── research/            外部调研
└── agents/              issue tracker / triage / domain 的单上下文说明
```

### `spec/` — 要做什么、做到什么算完

| 文档 | 用途 |
| :--- | :--- |
| [需求.md](spec/需求.md) | **需求基线**：产品目标、需求目录与最低验收意图。新增功能前先读 |
| [需求测试矩阵.md](spec/需求测试矩阵.md) | 需求 → 验收场景 → 测试层。由 `scripts/automation/render-matrix.py` 从 registry 生成 |
| [功能对齐清单.md](spec/功能对齐清单.md) | 上游能力快照（Magpie / Todos），验收对象 |
| [通知入站账本设计.md](spec/通知入站账本设计.md) | 入站账本的口径与边界（设计） |
| [入队延迟交付验收.md](spec/入队延迟交付验收.md) | 入队延迟：交付与验收记录 |
| [通知账本交付验收.md](spec/通知账本交付验收.md) | 入站账本：交付与验收记录 |
| [外发交付与故障升级交付验收.md](spec/外发交付与故障升级交付验收.md) | NOT-05 外发交付状态与故障升级：交付与验收记录 |

### `testing/` — 怎么验

| 文档 | 用途 |
| :--- | :--- |
| [测试框架.md](testing/测试框架.md) | 测试层定义（L0–L8）、门禁与 profile 的组织方式 |
| [测试映射盘点.md](testing/测试映射盘点.md) | 历轮「测试资产 → 需求 ID」盘点（**历轮记录**，见文首更正说明） |
| [测试自动化交付.md](testing/测试自动化交付.md) | 自动化测试框架与发布流水线的交付记录（权威现状入口） |

### `ops/` — 怎么发

| 文档 | 用途 |
| :--- | :--- |
| [自动开发流水线.md](ops/自动开发流水线.md) | 需求→开发→测试→发布的流水线说明 |
| [version-mapping.md](ops/version-mapping.md) | 历史版本号 ↔ 时间映射 |

### `agent/` — 写给 Agent 的约定与历史证据

| 文档 | 用途 |
| :--- | :--- |
| [pitfalls-history.md](agent/pitfalls-history.md) | **改造踩坑史**：现象 / 根因 / 由此定下的规则 |
| [session-2026-10-16-problems.md](agent/session-2026-10-16-problems.md) | 该日会话问题与分诊 |
| [session-2026-10-17-problems.md](agent/session-2026-10-17-problems.md) | 该日会话问题与分诊 |
| [desensitization.md](agent/desensitization.md) | 脱敏细节 |
| [history-file-map.md](agent/history-file-map.md) | 历史路径映射（归档 Swift/WPF、迁出取证文件） |
| [project-document-audit-2026-10-14.md](agent/project-document-audit-2026-10-14.md) | 文档梳理与冲突登记 |

---

## 根目录的软链（兼容层）

`docs/` 根下**每个散落文档都保留了一个同名软链**指向子目录里的真实文件
（例如 `docs/需求.md` → `spec/需求.md`）。原因：

- 门禁脚本与 `scripts/automation/registry.json` **硬编码**了部分路径（`docs/需求.md`、
  `docs/需求测试矩阵.md`、`docs/测试映射盘点.md`）；
- 全仓有大量文档按旧路径互相引用。

软链让「分类」与「旧引用」同时成立。**写操作会穿透软链落到真实文件**（已实测），
所以生成器（如 `render-matrix.py`）无需改动。

> 新增文档**直接放进对应子目录**，不要再往根下放。
> 新增 `docs/*.md` 时，如非必要不要加软链——软链只为兼容既有硬编码路径而留。

---

## 三类文档别混（避免长成第二份 README）

| 想写什么 | 写到哪 |
| :--- | :--- |
| 这个工具是什么、怎么装怎么用、哪些限制今天还成立 | [`../README.md`](../README.md) |
| 每个版本改了什么、为什么 | [`../CHANGELOG.md`](../CHANGELOG.md) |
| 口径、术语、长期决策 | [`../CONTEXT.md`](../CONTEXT.md) / [`adr/`](adr/) |
| 某个需求做到哪了、有哪些未修 bug | [`需求状态.md`](需求状态.md) |
| 每轮改动的独立 review | [`code-review/`](code-review/) |
