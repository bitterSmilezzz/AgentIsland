# 会话强语义：Rust 与 Swift 的逐段比对（对照表 §7 第 1 条）

> 比对时间：2026-09-27 · 对象：`app/src-tauri/src/session.rs` 的四个 `probe_*`
> 与 `Sources/AgentIslandCore/AgentSessionInspector.swift` 的 `detect(lines:)`
> 起因：对照表 §7 第 1 条「`session.rs` 四个 `probe_*` 与 Swift 同名解析器未逐行比对语义」。

## 0. 先说一个结构事实，它决定了后面所有差异的性质

| | Swift | Rust |
| :--- | :--- | :--- |
| 分派方式 | 按**档案声明的方言**（`SessionDialect`） | 按 **agent id** |
| 解析器 | **一个** `detect(lines:)`，靠 `collectFacts` 一次遍历收集工具调用/结果/请求标记/完成标记/活跃标记 | **四个** `probe_claude` / `probe_codex` / `probe_cline` / `probe_zcode` |
| 覆盖面 | 任何走 `genericTail` 的档案都吃同一条检测线 | 只有 `claude` / `codex` / `cline` / `roo-code` / `roo` / `zcode`，其余一律返回无信号 |

**后果一（已知，见对照表 §3.2）**：新增一个复用既有格式的 Agent，Rust 要改代码；
Swift 只改注册表。ADR 0010 要求的正是后者。

**后果二（本轮新发现）**：Swift 的 `detect` 里那些保护机制是**共享**的，
所以所有 `genericTail` 档案都受保护；Rust 把它拆成四个独立解析器之后，
**每一条共享保护都要单独搬**，漏一条就只在那条分支上失效。本轮查出的正是这样一条。

## 1. 尾部窗口：有意不同，但方向要说清

| | Swift | Rust |
| :--- | :--- | :--- |
| 字节上限 | 262,144（256KB） | 8,388,608（8MB） |
| 行数上限 | 96 行 | 600 行 |

Rust 更大，因为 `model-io` 一类会话文件单行可达数 MB（见 `read_tail_lines` 的注释）。
**方向是「看得更远」**：两者都从尾部往回扫，Rust 能看到 Swift 看不到的更早的行。

风险在完成态：一条**很旧**的 `attention` 若落在 96–600 行之间，
Swift 看不到（会继续试下一个候选文件），Rust 会看到并报出来。
**本轮未确认这是否已在别处兜住**——`decide_level` 侧对 attention 没有时间窗
（只有 completed 有 15 分钟窗，见 §3）。列为待查，不下定论。

## 2. 在途命令的**判定范围**：真差异

Swift 在产出 `.completed` 之后还有一道关键防御（`AgentSessionInspector.swift:298-317`）：

```swift
let unresolvedCommands = unresolved.values.filter { call in
    !requestNames.contains(n) && (n.contains("bash") || n.contains("command")
        || n.contains("terminal") || n.contains("exec") || call.command != nil)
}
```

即：**只有终端执行类**的未决调用才把状态按住在 working；
「普通提问或读取类工具不破坏通用状态」。

Rust 的 `probe_claude` 只要发现**任何**未收口的 `tool_use` 就报 `Active`
（`tail_has_tool_result` 为假即返回），不区分工具类别。

**后果**：Claude Code 挂着一个未收口的 `Read` 时，Swift 显示待机，Rust 显示
「正在读取：X」的工作态。这是**噪声方向的差异**——多报，不漏报。
方向上比漏报安全，但会让「在工作中」的指示灯更亮。

**本轮未修**：修它要引入 Swift 那套「工具名归类 + command 字段」判定，
属于把 Swift 的 `requestNames` 分类表整份搬过来，是独立一块工作量。
先记进对照表。

## 3. 完成态 15 分钟过期：Rust 侧未见等价物

Swift（`AgentSessionInspector.swift:182`）：

```swift
if case .completed = signal, age > 15 * 60 { continue }
```

Rust 的 `session.rs` 里搜不到任何时间窗。但**引擎层可能有等价物**
（`decide_level` 有 `last_completed_fp` 与滞回），本轮**未追到底**，
不下结论。列为待查。

## 4. 用户中断撤销在途命令：**真 bug，已修（v0.0.199）**

这是本轮查出的**唯一会咬人的差异**，且有可复现的症状。

### 症状（实测，不是推演）

夹具：assistant 发起 `Bash: sleep 999`，随后一行 `Request interrupted by user`。

```
DEBUG 中断后 signal = Some(Active("6790587addef1d37:571b791944821cc9", Some("运行: sleep 999")))
```

中断之后，Rust **仍然报在途**。

### 为什么这是 bug 而不是「保守」

Ctrl-C / 点停止之后，那条 `tool_result` **永远不会来了**。
于是每一拍都拿到 `.active` → 滞回与完成分支永远走不到 →
Agent 被钉死在 working → 2s 快采样与高频全树扫描一起被锁住（耗电与 CPU 双输）。

Swift 侧早有防护与专用用例：
`AgentSessionInspector.isInterruptionNotice` + `interruptionPhrases`（10 条），
用例在 `MultiAgentAdvancedTests.swift:149`
「用户中断撤销在途命令: 不得把 Agent 永久钉在 working」。

README 也把这条当既有能力写着：「Ctrl-C 留下的僵尸 `tool_use` 会立即撤销在途命令，
Agent 不会被钉在工作态不放」。**Rust 侧此前不具备该能力。**

### 修法

`probe_claude` 在向后扫描前先定位**最后一条**中断通知；
扫描时走到该行即止。中断之前的调用一律作废。

**中断之后的调用仍然是在途**——Swift 的语义是「撤销只作用于中断之前的调用」，
只做前半截会把「真的在跑」也一并抹掉。这两半各有一条用例。

短语表 10 条逐条照搬；短行闸（`line.count <= 400`）也照搬——
长行里出现 `aborted by user` 多半是在转述别人的话。

### 用例

| 用例 | 钉住什么 |
| :--- | :--- |
| `a_user_interruption_voids_the_calls_it_interrupted` | 中断后不得再报在途 |
| `a_command_started_after_the_interruption_is_still_in_flight` | 撤销只作用于中断之前的调用 |
| `an_interruption_phrase_inside_a_long_line_does_not_void_anything` | 短行闸 |

三条都做了反向验证：去掉中断撤销 → 第一条红；去掉短行闸 → 第三条红。

## 5. 本轮的比对结论

| 差异 | 性质 | 状态 |
| :--- | :--- | :--- |
| 分派按方言 vs 按 id | 架构差异 | 已在对照表 §3.2，本轮不修 |
| 尾部窗口 96/256KB vs 600/8MB | 有意不同 | 记录；旧 attention 的影响待查 |
| 在途判定范围（任何工具 vs 仅执行类） | **真差异（多报）** | 记录，本轮不修 |
| 完成态 15 分钟过期 | **未确认** | 引擎层可能有等价物，待查 |
| 用户中断撤销在途命令 | **真 bug（钉死）** | ✅ 已修（v0.0.199） |
| attention 被结果解除 | 两边都有 | ✅ 一致（Rust 靠 `tail_has_tool_result` 回扫） |

**仍未做的比对**：`probe_cline` / `probe_zcode` 与 Swift
`detectClineOrRoo` / zcode 路径的逐行比对，以及 `collectFacts` 那些事实键
（`resolutionTypes`、`attributedToUser`、`states` 等）的等价覆盖。
本轮只完成了 `probe_claude` 这一支。
