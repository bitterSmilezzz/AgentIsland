# 通用检测器能不能移植到那 16 个档案：真数据并排对拍的结论

> 2026-09-29。v0.0.247 量出「16 个档案声明了会话源却读不出信号」，
> 并推测「正解是把 Swift 那个通用检测器移植过来」。**这一版用真数据检验那个推测，
> 结论是：不能直接移植。**

## 要验的问题

Swift 侧只有一个 `detect(lines:)`，按**内容**走整棵 JSON 树；
Rust 侧是逐方言手写解析器。移植之前必须先回答：

> 通用检测器与手写解析器，在**真实会话文件**上判得一样吗？

造夹具回答不了——问题恰恰出在真实文件长什么样。

## 怎么做的

1. 把 Swift 的 `detect` + `collectFacts` + 全部词表**逐条照搬**成一份离线脚本
   （`collectFacts` 的 DFS、`LineFacts` 的四个派生判定、行状态机、
   `applyPlainText` 纯文本降级、`isInterruptionNotice`、
   在途命令拦截的六词表）。**不交付、不入库**，只作取证。
2. Rust 侧加一条手工探针（`#[ignore]`，读本机真实文件）打出 `probe_codex` / `probe_zcode` 的判定。
3. 两边跑**同一批文件**（各取最新 8 个），并排比。

## 结论一：codex 上 8/8 完全一致

| | 判定 |
| :--- | :--- |
| 手写 `probe_codex` | 8 个全部 `completed` |
| 通用 `detect`（照搬） | 8 个全部 `completed` |

**内容驱动的思路本身站得住**——至少对 rollout 那种形状是。

## 结论二：zcode 上出现一处**假阳**，而且它否掉了「直接移植」

| 文件 | 手写 `probe_zcode` | 通用 `detect` |
| :--- | :--- | :--- |
| `model-io-sess_53c5ec18…` | None | None ✅ |
| `model-io-sess_subagent_agent_24a34102…` | None | **attention** ❌ |
| `model-io-sess_subagent_agent_556c33aa…` | None | None ✅ |

**分歧点（已定位到具体一行）**：那个 subagent 文件的**第 0 行**里，
`request.body.tools[].function.name == "AskUserQuestion"`。

那是**发给模型的工具目录（schema）**，不是一次真实的提问。
但 `collectFacts` 把**整棵树**都走了一遍——包括 `request.body.tools`——
于是把一份工具声明读成了「正在等你批准」，并把这个信号**锁死**在文件末尾。

### 为什么这在 Swift 侧一直没暴露

因为 Swift 的 `probe(profile:)` 把 ZCode 路由到 `.statusIndex`（查 SQLite 状态索引），
**通用 `detect` 从来没被套在 ZCode 的 model-io 文件上**。

⚠️ 顺带修正 v0.0.247 的一句过头话：那里写「Swift 那边一个通用收集器覆盖全部」。
**不准确**。准确说法是：

> 通用检测器只被用在 `genericTail` 那一族档案上，
> 而这一族恰好都是 Claude / Codex 形状。
> ZCode / Qoder / Antigravity / DSH / Cline 在 Swift 侧各自有专用路径。

所以「移植它来覆盖那 16 个档案」= **把它从没设计过的形状上硬用**，
而这条假阳就是硬用的第一个代价。

## 这意味着什么

- **不能把通用检测器当通用解直接接进那 16 个档案。**
  凡是**文件里嵌了请求体**的格式（model-io 那一族就是），
  工具目录会被读成提问，于是「等你批准」永远亮着。
- 顺带一个**推断**（未证实）：codex 那 8 个之所以没踩到，是因为它们的
  `payload` 里没有带 `AskUserQuestion` 的工具目录。换一台装了带该工具的 Agent
  就可能踩到同一条路。**这是推断，没有实测**。
- 那 16 个档案要补，**仍然得逐格式来**（像现有手写解析器那样），
  或者给通用检测器加一条「**不进入请求体子树**」的规则——
  但那是**设计改动**，得单独验，不能顺手加。

## 仍然成立的部分

- 「16 个档案读不出信号」这个**度量**成立（v0.0.247），本记录没有推翻它。
- 本记录只推翻**「怎么补」**：不是移植，是逐格式。

## 取证命令

```sh
# Rust 侧判定（本机真实文件）
cargo test --bin agentisland real_session_side_by_side -- --ignored --nocapture

# 真数据的键名形状（只取键名，不取任何内容）
# codex: {timestamp,type,ordinal,payload:{type,role,name,status,call_id,item:{command,…}}}
# zcode: {type,requestId,startedAt,completedAt,request:{body:{tools,system,…},messages:[{role,…}]},response:{text,toolCalls}}
```

⚠️ 探针会读**用户的真实会话文件**。因此它 `#[ignore]`、不读进仓库、不进 zip；
取证只取**键名与判定**，不取任何正文。
