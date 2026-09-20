# Qoder 能被监控到什么（本机实测）

> 目的：在写代码之前把「Qoder 到底往磁盘上留了什么」测清楚，避免把「字段存在」当成「数据存在」。
> 本文所有数字与键名都是 2026-09-20 在本机（macOS 25.6 / Qoder 运行中）实测得到，不是推测。

## 一句话结论

**状态与动作可以完整监控（已实现）；Token 消耗监控不了——Qoder 落盘的 token 字段全是 0，
真正的计量单位是 `credits`。**

## 心智模型：监控一个 agent 需要三类事实

| 需要知道 | 靠什么回答 | Qoder 的情况 |
|---|---|---|
| 它装了吗 / 在跑吗 | 安装路径 + 进程表 | ✅ `/Applications/Qoder.app`，bundle id `com.qoder.app` |
| 它此刻在干什么 | 会话记录的最新一条「未闭合的动作」 | ✅ 逐行 JSONL 里有完整的 `tool_use` / `tool_result` 配对 |
| 它花了多少 | 每条回复附带的计量字段 | ⚠️ 只有 `credits`，token 字段恒为 0 |

第三类是这次唯一卡住的，所以先说清前两类怎么成立的。

## 安装与进程（实测）

```
/Applications/Qoder.app/Contents/MacOS/Qoder                          ← 主进程
/Applications/Qoder.app/Contents/Frameworks/Qoder Helper.app/...      ← GPU/工具进程
/Applications/Qoder.app/Contents/Frameworks/Qoder Helper (Renderer).app/...
/Applications/Qoder.app/Contents/Frameworks/Electron Framework.framework/Helpers/chrome_crashpad_handler
```

- 它是 Electron 应用：主进程与一堆 helper 都叫 `Qoder` / `Qoder Helper`，
  本仓的进程名匹配规则是「精确相等」或「name + 空格/连字符」前缀族，
  所以 `processNames: ["qoder"]` 一条就能同时覆盖主进程与 Helper。
- `pathContains` **锚定成 `/applications/qoder.app`**，不用裸子串 `"qoder"`：
  裸子串会把 `~/code/qoder-playground` 里跑的任何 Electron 程序认成 Qoder
  （这是本仓已记录的过宽匹配风险，新档案不再制造一例）。

## 会话与状态（实测）

```
~/.qoder/projects/<项目 slug>/<会话 uuid>.jsonl        ← 逐行对话记录
~/.qoder/projects/<项目 slug>/<会话 uuid>/state.json   ← {sessionId, revision, createdAt, updatedAt, items…}
~/.qoder/projects/<项目 slug>/<会话 uuid>/subagents/   ← agent-<名字>-<hash>.jsonl + .meta.json
~/.qoder/logs/runs/<时间戳>-p<pid>/                     ← 每次运行一个目录，含 qodercli.log
~/.qoder/.qoder-app-status.json                        ← {logged_in, name, email, version, product…}（含邮箱，不展示）
```

对话记录逐行的顶层键（取最近会话前 200 行统计）：

```
type(200) sessionId(200) timestamp(198) uuid(124) parentUuid(124) isSidechain(124)
cwd(124) gitBranch(124) message(117) requestTokenAnchor(78) toolUseResult(38) promptId(39)
```

`message.*` 的键：`role` `content` `id` `type` `model` `stop_reason` `stop_sequence` `usage`
——**与 Anthropic 的消息体同形**，工具名也是 Claude 系：`Bash` `Edit` `Write` `Read` `Grep`
`Agent` `AskUserQuestion` `mcp_call` `TaskCreate` `TaskUpdate`。

`stop_reason` 分布（一个 10MB 会话）：`tool_use` 1004 次、`end_turn` 10 次。

### 状态是怎么推出来的（关键设计）

「等待用户」不是一个字段，而是一个**结构事实**：

```
assistant 发出 tool_use(name=AskUserQuestion, id=tu-9)     ← 还没有 id=tu-9 的 tool_result
  ⇒ 球在用户这边 ⇒ 报「等待你回答或选择」

后面出现 user 的 tool_result(tool_use_id=tu-9)
  ⇒ 已经回答 ⇒ 绝不能再报等待
```

这就是为什么给它写**专用方言**而不是交给通用尾窗关键词扫描：只看关键字的实现会在
「刚答完的那一拍」反向误报——把用户叫回一个已经没事的窗口。本仓在 Claude 的
`AskUserQuestion` 上踩过同一形态，所以这条被单独写成了测试。

实测跑通（Qoder 正在跑本会话时）：

```
🟢 工作中  🖥️ Qoder   514  0.0%  984M  3  —  —  运行: cd /Users/fangshoufanji/workspace/Agent…  20s前
🖥️ Qoder  工作中  结论可信  本轮读到了会话强语义（工作中）      ← doctor
```

## Token 消耗：这条路走不通（实测）

`message.usage` 的键**看起来很全**：

```
input_tokens  cache_creation_input_tokens  cache_read_input_tokens  output_tokens
server_tool_use  service_tier  cache_creation  inference_geo  iterations  speed
credits  original_credits  billable  context_usage_ratio  request_id
```

但把本机所有会话文件的 1,033 条 `usage` 记录求和：

```
含 usage 的行: 1033    其中 token 全零: 1033    有值: 0
求和: {input: 0, out: 0, cache_read: 0, cache_create: 0}
credits 合计: 303.37
```

也就是说：**四个 token 字段恒为 0，唯一有值的是 `credits`（和 `context_usage_ratio`）。**

`requestTokenAnchor` 里那两个 `request` / `response` 字段是 64 字符的不透明串（不是 JSON），
也没有藏用量；`~/.qoder/logs/` 与 `.models/` 里同样没有 token 计数。

### 因此本仓当前的处理

- 档案的 `tokenRoots` **留空**，不接结构化用量采集。
  接上试过：`agentisland tokens` 冷跑从 ~2.05s 涨到 ~2.9–4.0s（单个会话文件可达 10MB），
  换回 0 条可用数据——为一个空字段让全机用量页慢一倍，不值。
- 用量列对 Qoder 显示 `—`（= 没取到），**不显示 0**。
  这是本仓的硬规矩：「明细源没被发现」和「消耗是 0」是两件事。
- `doctor` 会把它归到「未接入本地明细源」，依据文案明确说「不代表它没在工作」。

### 如果要做 credits，需要先定三件事

1. **单位**：credits 既不是 token 也不是美元。直接塞进 `cost` 字段会误导（用户看到 `$0.00`
   和 `303 credits` 混在一列）。要么新增一列，要么在 UI 上明确单位标签。
2. **换算**：Qoder 没有在本机留下 credits→人民币/美元的汇率。要显示金额只能让用户自己填
   （「1 credit = ? 元」），否则就是编数。
3. **口径**：`credits` 是每次请求的增量还是累计快照？本机样本里 `credits` 与
   `original_credits` 相等且 `billable: false` 并存，需要多取几个会话确认语义再定求和方式。

这三条定了之后，实现量很小：`StructuredTokenSource` 加一个 `.qoder` 格式 + 模型加一个
credits 字段 + 用量页加一列。

## 参考

- 本仓接入形态：`Sources/AgentIslandCore/AgentRegistry.swift`（档案）、
  `AgentSessionInspector.swift` 的 `inspectQoderTranscript` / `detectQoder`（方言）、
  `Tests/AgentIslandTestsRunner/QoderTrackingTests.swift`（6 条回归）
- 为什么不在 UI 里显示假 0：`CONTEXT.md` 的「可见口径」与「双信号」条目
