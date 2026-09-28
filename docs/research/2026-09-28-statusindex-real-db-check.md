# statusIndex 状态索引的真库对拍：查出 Swift 侧一条恒定失效的 SQL

日期：2026-09-28　版本：v0.0.221　机器：macOS 27.0.0 arm64

## 一句话

Swift `AgentRegistry` 里 zcode 那条 `statusSQL` 查的是 `id`，而真实的
`tasks` 表**没有 `id` 列**——它在 macOS 端**恒定 prepare 失败**，被当成
「这个 Agent 没有终态」，于是 **ZCode 的完成态信号从来没有生效过**，界面上看不出任何异样。

## 取证命令与原文

### 1. Swift 侧声明的那条 SQL

`Sources/AgentIslandCore/AgentRegistry.swift:198`：

```
SELECT id, task_status, updated_at FROM tasks WHERE deleted = 0 ORDER BY updated_at DESC LIMIT 1;
```

### 2. 真实 schema（本机 `~/.zcode/v2/tasks-index.sqlite`）

```sh
sqlite3 ~/.zcode/v2/tasks-index.sqlite "SELECT sql FROM sqlite_master WHERE name='tasks';"
```

原文（节选关键列）：

```sql
CREATE TABLE tasks (
        workspace_key TEXT NOT NULL,
        workspace_path TEXT NOT NULL,
        workspace_identity TEXT,
        task_id TEXT NOT NULL,
        title TEXT NOT NULL DEFAULT '',
        task_status TEXT,
        ...
        archived INTEGER NOT NULL DEFAULT 0,
        deleted INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (workspace_key, task_id)
      )
```

**主键是 `(workspace_key, task_id)`，没有 `id` 列。**

### 3. 照抄 Swift 那条 SQL 跑真库

```sh
sqlite3 ~/.zcode/v2/tasks-index.sqlite \
  "SELECT id, task_status, updated_at FROM tasks WHERE deleted = 0 ORDER BY updated_at DESC LIMIT 1;"
```

原文输出：

```
Parse error in 2nd command line argument: no such column: id
  SELECT id, task_status, updated_at FROM tasks WHERE deleted = 0 ORDER BY updat
         ^--- error here
```

### 4. 换成真实列名后（证明只有列名这一处错）

```sh
sqlite3 ~/.zcode/v2/tasks-index.sqlite \
  "SELECT task_id, task_status, updated_at FROM tasks WHERE deleted = 0 AND archived = 0 ORDER BY updated_at DESC LIMIT 3;"
```

```
sess_072e6396-f73b-4c78-9a4b-f0d529e85558|running|1790569993628
sess_53c5ec18-ad2e-448d-8b99-2f1f9b546fa0|error|1790557400811
sess_84f7cfad-f78e-4ed6-9bc9-5ff9c93be395|error|1790557212113
```

### 5. 状态取值分布（决定了「修了也只有什么收益」）

```sh
sqlite3 ~/.zcode/v2/tasks-index.sqlite "SELECT task_status, COUNT(*) FROM tasks GROUP BY 1 ORDER BY 2 DESC;"
```

```
error|2
running|1
completed|1
```

**推断（标注为推断）**：Swift 的 `requestStates` 15 个词全是「等待批准」类，
本机 ZCode 一个都不写。所以修好 SQL 之后，这一族能贡献的实际是**完成态**，
不是「等待你批准」。这不是说那条路径没用——`completed` 正是它能给的东西，
而在此之前它连这个也给不了。

### 6. 另外两个 `statusIndex` 档案的 SQL **是对的**（不能一刀切地改）

```sh
for db in ~/.workbuddy/workbuddy.db ~/.workbuddy-ai/workbuddy.db; do
  sqlite3 "$db" "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;"
done
```

两个库都正常返回，`PRAGMA table_info(sessions)` 确认
`id / cwd / user_id / title / status / created_at / updated_at / deleted_at / …` 都在。
⇒ **这个 bug 是 zcode 独有的**，修它时不能顺手改另两个。

### 7. 修复后的真机复核（Rust 侧 `--ignored` 探针）

```sh
cargo test --manifest-path app/src-tauri/Cargo.toml -- --ignored real_status_index_probe --nocapture
```

原文输出：

```
· zcode：库龄 45190s · 信号 Some(Completed("557fbe41f44b5f8f:a4e61bdc90a2ae42")) · 故障 None
· workbuddy：库龄 2488s · 信号 None · 故障 None
· workbuddy-ai：库龄 326353s · 信号 None · 故障 None
```

三个真实库全部查询成功、零故障。再与原生 SQL 交叉核对 zcode 那一条：

```sh
sqlite3 ~/.zcode/v2/tasks-index.sqlite \
  "SELECT task_id, task_status, updated_at, datetime(updated_at/1000,'unixepoch','localtime') \
   FROM tasks WHERE deleted = 0 AND archived = 0 ORDER BY updated_at DESC LIMIT 1;"
date "+%s  %Y-%m-%d %H:%M:%S"
```

```
sess_072e6396-f73b-4c78-9a4b-f0d529e85558|completed|1790572229551|2026-09-28 13:10:29
1790572329  2026-09-28 13:12:09
```

完成时间距探测时刻 100 秒，在 15 分钟窗内 ⇒ Rust 报 `Completed` 与数据一致。

## 为什么它一直没被发现

原有守护（`Tests/AgentIslandTestsRunner/RegistryTests.swift:258`）：

```swift
try expectTrue(db.statusSQL?.isEmpty == false, "\(profile.id) 的 statusIndex 方言缺少查询语句")
```

它检查的是「**有没有这个字符串**」，不是「**这条查询能不能跑**」。
一条恒为真的断言比没有断言更糟：它让人以为这里被守着。

## 换成的守护，以及变异验证

`RegistryTests.statusIndexSQLRunsAgainstTheRealSchema()`：为每个 `statusIndex` 档案
按**真实 DDL** 建一张表，真 `sqlite3_prepare_v2` 它的 `statusSQL`。

**变异验证**（把 SQL 改回原样）：

```
❌ 注册表: zcode 的 statusSQL 能在真实 schema 上 prepare
   — no such column: id
结果: 554 通过, 1 失败
```

报的正是真库那一句。

### 夹具的诚实边界

DDL 是从真库 `sqlite_master` 抄的**结构**，**不含任何一行真实数据**。
真实 schema 往后演进时，这份夹具会**先于现实失真**——那时它给的是
「需要重新采集」的提示，而不是继续绿灯。这也是为什么 §7 的那条探针
（`real_status_index_probe`，`--ignored`）值得留着：它跑的是真库。

## Rust 侧本轮补的内容

- `SessionDatabase.status_sql`：查询由**档案声明**，不在代码里写死
  （同一张表形下各产品列名毫无共同点，写死的那份在本机恒定失败）。
- `session::probe_status_index`：库龄 24h / 完成态 15min / 秒与毫秒 epoch 兼容 /
  prepare 与 step 失败都留下 `UnreadableDatabase(SQLite 原文)`。
- 位置：**文件探测全部落空之后**才查库（Swift `probe` 的最后一步才是
  `inspectKnownDatabase`）。反过来放前面，会让库里的旧状态盖过文件里刚发生的活动。
- `SessionProbeFailure::UnreadableDatabase(String)`：这一族的问题不是「文件读不了」
  而是「查询跑不通」，而它**静默得最彻底**——诊断文本必须一路带到界面上。
- workbuddy / workbuddy-ai 两个档案原先**根本没有声明 session_database**，本轮补上。

## 没做的

- **`probe_cline` 的四处差异没有据此改行为**：本机没装 Cline/Roo，
  `~/Library/Application Support/Cline/tasks` 与
  `…/globalStorage/saoudrizwan.claude-dev/tasks` 都不存在，
  只有代码比对没有实样。差异已逐条记进对照表 §7 第 1 条。
- **两端在 ZCode 上仍可能给出不同信号**：Swift 的通用检测器对 rollout JSONL
  多半读不出东西而落到状态库；Rust 的 `probe_zcode` 能从 `toolCalls` 提炼实时动作，
  所以「Rust 报在跑 / Swift 报完成」是可能的。这条要等拿到实样才能定谁是错的。
- **schema 漂移**：真实 schema 变更后夹具会失真，如上。
