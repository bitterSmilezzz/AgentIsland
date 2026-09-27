# OpenCode 表名失配 · 与真实 SQLite 库的对拍记录

> 取证时间：2026-09-27 · 机器：Max（darwin 27.0.0 arm64）
> 起因：对照表 §7 第 7 条「Rust 侧 SQLite 读取只在合成夹具上验过」。
> 本文记录**一手核实结果**，命令可复跑。凡是推断的地方都标了出来。

## 0. 取证原则

原注释写着「实测（本机 `opencode.db` 缺失，故用夹具库对拍）」——
**这是当时的事实，不是现在的事实**。现在库在，于是去取真值。

所有查询都打在**拷贝出来的副本**上，原库只读；副本用完即删。

```sh
cp ~/.dimcode/v2/dimcode.sqlite   /tmp/dim_probe.sqlite
cp ~/.local/share/opencode/opencode.db /tmp/oc_probe.sqlite
cp ~/.local/share/mimocode/mimocode.db /tmp/mc_probe.sqlite
```

## 1. `dim`（DimAgent）—— 对拍通过

| 项 | 值 |
| :--- | :--- |
| 库大小 | 469,086,208 字节（447M） |
| `usage_ledger` 行数 | 1368 |
| `completionTokens` 为 NULL 的行 | 0 |
| `completionTokens` 为负的行 | 0 |
| `prompt − cacheRead` 为负的行 | 0 |

### 三方对拍

| 来源 | 累计 token |
| :--- | ---: |
| `sqlite3` 按 **Rust** 公式算 | 198,997,695 |
| `sqlite3` 按 **Swift** 公式算 | 198,997,695 |
| `agentisland tokens --json` 实报 | **198,997,695** |

逐位相同。

### 复跑命令

```sh
sqlite3 /tmp/dim_probe.sqlite "
SELECT SUM( MAX(COALESCE(json_extract(usage,'\$.promptTokens'),0)
                - COALESCE(json_extract(usage,'\$.cacheReadTokens'),0), 0)
            + COALESCE(json_extract(usage,'\$.completionTokens'),0) )
FROM usage_ledger;"
```

### 顺带证实净口径的必要性

不扣缓存读的错口径：

```sh
sqlite3 /tmp/dim_probe.sqlite "
SELECT SUM(COALESCE(json_extract(usage,'\$.promptTokens'),0)
         + COALESCE(json_extract(usage,'\$.completionTokens'),0)) FROM usage_ledger;"
# → 7450151420
```

**7,450,151,420 vs 198,997,695 = 37.4 倍虚高。** v0.0.169 那个修正不是洁癖。

### 一处潜在差异（本机未触发）

| | Swift `DimUsageSQL.netTokens` | Rust `NET` |
| :--- | :--- | :--- |
| `prompt − cacheRead` | `MAX(…, 0)` 钳非负 | `MAX(…, 0)` 钳非负 |
| `completionTokens` | **不钳**（`SUM` 直接加） | `MAX(…, 0)` 钳非负 |

Swift 那段注释写的是「逐行钳制非负」，**代码只做到了 prompt 那一半**。
本机 1368 行里没有任何负的 `completionTokens`，所以两边今天结果一致；
但**注释与实现的落差**是真的，Rust 实现的是注释声明的意图。

## 2. `opencode` —— 查出真 bug（两端都有）

### 库的真实结构

```sh
sqlite3 /tmp/oc_probe.sqlite ".tables"
# account  event_sequence  migration  session_message  session_v2  …
#   ↑ 没有 message，也没有 session
```

| 代码期望 | 库里实际 |
| :--- | :--- |
| `FROM message` | `session_message`（0 行） |
| `JOIN session` | `session_v2`（0 行） |

最新一条 migration：

```sh
sqlite3 /tmp/oc_probe.sqlite "SELECT * FROM migration ORDER BY rowid DESC LIMIT 5;"
# 20260923013825_project_time_active|1790516458553
# 20260910120000_clear_v1_session_permission|…
# …
```

`session_message` 的结构与代码期望的 `message` 逐字段对得上
（`id` / `session_id` / `data` JSON / `time_created` / `seq`）。

> **推断**（非直接证据）：`session_message` 是 `message` 的继任者，`session_v2` 是 `session` 的继任者。
> 依据是①结构同构 ②该库带 migration 机制 ③表名带版本后缀。
> **无法用数据证明**——`session_message` 0 行。

### 后果

两端都写死 `FROM message` ⇒ **都读不到 opencode 用量**。
`agentisland tokens --json` 的输出里确实没有 `opencode` 这一项。

**这不是「报错」而是「读到零」**——而「读到零」在界面上与「这个 Agent 真的没用过」
完全一样。这正是本仓最在意的那类静默失败。

### 修法（v0.0.198，两端各一份）

一次查 `sqlite_master` 定下这张库里真实存在的表名，之后所有 SQL 都用它。
候选表**只认字面量**（表名不能绑参，必须拼进 SQL，因此没有注入面）：

| | 消息表候选 | 会话表候选 |
| :--- | :--- | :--- |
| Swift `OpenCodeTables`（`ReadonlyDB.swift`） | `session_message` → `message` | `session_v2` → `session` |
| Rust `OpenCodeTables`（`sqlite.rs`） | 同上 | 同上 |

顺序优先新名：万一某个 fork 同时留着两张表，新的是当前在写的那张。
两种 message 表都没有 ⇒ 不是这一族的库，**返回 nil 说「读不到」，绝不退回一个猜的表名**。

Swift 侧共 7 处 SQL 接上（`TokenUsageMonitor` 4 处 / `AgentSessionInspector` 2 处 /
`AgentActionInspector` 1 处），Rust 侧 2 处（`tokens.rs`）。

会话表缺失时，钻取那一层降级为「不带目录」，而不是整层消失。

## 3. `mimocode`（小米 MiMo Code）—— 旧 schema，仍可读

```sh
sqlite3 /tmp/mc_probe.sqlite "SELECT name FROM sqlite_master WHERE type='table'
  AND name IN ('message','session_message','session','session_v2');"
# message
# session
```

`message` 7 行，净消耗 **48,934**。它是旧 schema 的 fork，修复后行为不变。

## 4. 仍未闭合的部分

**新 schema 下的数据层等价性没有真实数据可证。** `session_message` 0 行，
所以「两种 schema 读出同一个数」这条只由合成夹具守护（两端各一条用例）。

要真正闭合，需要 opencode 在这台机器上跑过一轮真实会话后再对拍。

## 5. 顺带查明的一件事

`agentisland tokens --json` 的输出里**没有 `mimocode`**，尽管它的库可读、净消耗 48,934。
本轮**未追查**这个（与表名失配是两件事），留作下一条。
