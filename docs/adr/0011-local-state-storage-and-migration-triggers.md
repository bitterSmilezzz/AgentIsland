# 本地状态用原子写 JSON；迁到嵌入式数据库的判据先钉死

2026-09-27 定。起因是一次「我们要不要用 SQLite、有没有更能替代它的开源项目」的询问。
答复里真正有长期价值的是**澄清与判据**，所以落成这份 ADR；具体选型调查另见下面的依据。

## 先分开两件答案相反的事

本仓碰 SQLite 有两个场合，**选择权完全不同**，混在一起谈必然谈糊：

| 场合 | 格式谁定 | 我们能选的 |
| :--- | :--- | :--- |
| **读**：解析第三方 Agent 自己写的库（`dimcode.sqlite` / `tasks-index.sqlite` / `opencode.db` / `mimocode.db` / `workbuddy.db` ×2） | **别人**——那 6 个 Agent | 只有「用哪个 SQLite 绑定」，**没有「换个数据库」这个选项** |
| **写**：我们自己的状态（设置、档位、待办、锚点坐标） | 我们 | 可以根本不用数据库 |

## 决定（写侧）

**继续「临时文件 + rename 的原子写 JSON」，不引入任何嵌入式数据库。**

现状：`settings.rs`（183 行）落 `settings.json`；Phase 2 的档位与 Phase 3 的 ToDos
在方案里都写成 JSON，ToDos 的验收标准原文就是「把 json 写坏后重启 App 不崩且列表为空」
（[04-plan](../workbench/04-plan.md)）。数据规模是几 KB 级。

### 为什么不上数据库

JSON 有三个本仓**已经靠测试依赖上**的性质：可 diff、可人工核对、**坏了能降级**
（`LoadState.corrupt` 只读降级、「坏元素只丢自身，其余抢救回来」、空数组不被回退默认值）。
换成二进制库文件，这些会变成「要么全有要么全无」。而换来的东西——索引、事务、并发写——
在几 KB、单进程写的规模上一项都用不上，纯粹是依赖、schema 迁移与二进制体积。

### 什么时候该翻案（可判定的触发条件）

不是「感觉会变多」。同时满足 ① 与 ② 就迁：

1. **单个状态文件的稳态超过约 1MB，且每次写入都要整份重写**（说明它其实在当 append-only 日志用）；
2. **需要在 ≥10⁴ 行上按非主键条件查询或聚合**（例如事件历史按时间范围 + 按 agent 过滤）。

另有第三个信号：**出现两个进程同时「写」同一份状态的真实需求。** 只有它时不迁——
先看能不能继续走今天这条路（CLI 通过本机 HTTP 问运行中的 app，压根不共享文件写）。
数据库的跨进程写锁语义是这个决定里最贵的一块，不该为了省一次 HTTP 就买。

## 真要迁时选什么

| 候选 | 一手依据 | 判决 |
| :--- | :--- | :--- |
| **SQLite**（`rusqlite` + `bundled`） | 公共领域；`bundled` 内含 SQLite 3.53.2，已含 3.51.3 修掉的 WAL-reset bug；多进程并发是一等公民 | ✅ **选它**。不是因为它快，而是**主流嵌入库里只有它把「多进程 + 崩溃安全 + 成熟度」同时做到位**，而我们 GUI 与 CLI 是两个进程 |
| **redb** | MIT/Apache-2.0，官方标 *Stable and maintained*，设计文档有完整的多进程三种模式（exclusive writer / single writer / multi-writer，文件区间锁） | ⚠️ **备选第二位**。唯一像样的纯 Rust 选项（无 C 依赖），代价是无 SQL、生态小、多进程下不支持 non-durable commit、只读进程打开不检测损坏 |
| Turso Database（原 Limbo） | MIT、纯 Rust；其 [COMPAT.md](https://github.com/tursodatabase/turso/blob/main/COMPAT.md) 的 **Guarantee #4 原文**：*"We don't support mixed SQLite and Turso in multi-process scenarios"*；rollback journal 模式全标 Not Needed（只支持 WAL）；未到 1.0 | ❌ 我们的读场景**正是「混用多进程」**，被它自己的保证排除 |
| fjall | MIT/Apache-2.0；README 显式 **WARNING**：*"A single database may not be loaded in parallel from separate processes"* | ❌ 多进程写不成立 |
| sled | MIT/Apache-2.0；README 自己写着 *"if reliability is your primary constraint, use SQLite. sled is beta."*，且 1.0 前磁盘格式会变、需手工迁移、不支持多实例 | ❌ 上游劝退 |
| SurrealDB | [Business Source License 1.1](https://support.surrealdb.com/en/articles/11538829-pricing-and-licensing) | ❌ 本仓是 MIT，不引入 BSL 依赖 |
| Realm / Atlas Device SDK | MongoDB [官方弃用公告](https://www.mongodb.com/docs/atlas/device-sdks/deprecation/)，EOL 2025-09 | ❌ 已弃用 |
| DuckDB | MIT，OLAP，为整表扫描设计 | ❌ 与「每秒一拍的 `LIMIT 1` 点查」不匹配 |

## 迁移时必须保住的不变量

迁移最贵的地方不是换 API，是这些口径——**任何一条丢了都算迁移失败**，
所以它们先进这份 ADR，而不是等迁移时再想：

1. **「没读到 ≠ 0」**：查询失败、锁争用、表不存在必须能与「真的是 0」区分开
   （今天的 `ReadonlyDB.ConnectionFailure` 与 `SessionProbeHealth` 就是干这个的）。
2. **损坏必须降级，不许 panic、不许覆写原件**：坏文件退化成空清单/只读，且原字节留证。
3. **写入必须原子**：写一半崩掉不许留下半个文件（见下面的「同批落定」）。
4. **一个口径一处实现**：本仓已经反复吃过「同一事实两份实现」的账
   （`dimcode.sqlite` 路径、`LIMIT 200`、复核窗口 0.8s），迁移不许再添第五处。
5. **只读那侧永远只读**：绝不 `PRAGMA journal_mode`、绝不 checkpoint、**绝不 `immutable=1`**
   （它等于向 SQLite 声明文件不会变，对正在被 Agent 写的库会读到陈旧内容）。
   官方只读打开 WAL 库的三个条件里，走「`-shm`/`-wal` 可读」那条即可（同用户，成立）。
6. **跨进程**：选的存储必须支持 GUI 与 CLI 两个进程，这是硬条件，也是上表排除 fjall/sled 的依据。

## 同批落定的相邻口径

调查这条 ADR 时顺手发现并修掉两处**已经成立**的缺口，都属于同一层（本地状态与只读库）：

- `settings.json` 此前是**裸 `fs::write`**，写一半崩掉就留下截断的 JSON，下次启动解析失败
  **静默回落出厂值**——用户设置整份消失且毫无提示。现在两条落盘路径（档位配置、设置）
  共用 `atomicfile.rs` 的原子替换，写坏的暂存内容在 rename 前就被校验挡下。
- `ReadonlyDB`（会话探测与流水的只读层）此前**没设 `busy_timeout`**，而 `TokenUsageMonitor`
  设了 1000ms：同一类锁争用在两个出口给出两种结论，前者会当场把瞬态记成「读不到」。
  现在秒数收成 `ReadonlyDB.busyTimeoutMs` 一处，两条打开路径共用（依据见
  [CONTEXT.md](../../CONTEXT.md) 的「只读库的锁争用」）。

## 代价与边界

- **不预先抽象存储接口**（YAGNI）：真要迁是替换实现，不是现在就加一层抽象垫着。
- JSON 的已知代价照旧：整份重写、无并发写、无索引。规模没到上面的判据之前，这些都不构成问题。
- 判据里的「约 1MB」「≥10⁴ 行」是**工程量级**，不是实测出来的阈值；定它们是为了让
  「要不要迁」有可判定的说法，而不是为了精确。真触发时按当次的实测数据重估。
- 读第三方 SQLite 那侧**永远不会有「换数据库」这个选项**。上表里的候选只服务写侧；
  读侧的选型空间只有「哪个绑定 + 哪些只读契约」，后者见上面第 5 条与 CONTEXT。
