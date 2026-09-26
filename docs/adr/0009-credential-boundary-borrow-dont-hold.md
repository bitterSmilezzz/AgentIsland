# 我们不持有密钥，只借本机既有的登录态；不主动打厂商端点

三句话今天在仓里互相打架，谁都没说错，凑在一起就是谎。

## 现象与定位

`CONTEXT.md:196` 写「本项目不碰网络、不转发流量、不持有密钥」。同文件 `:167` 明写 SMTP 授权码、ntfy token、webhook key **本来就在钥匙串里**，而远程外发这功能本身就主动出网（`RemoteNotifier.swift` / `RemoteTransport.swift` / `SMTPSocket.swift` 三个文件，`SMTPSocket.swift` 走原生 socket 连 465）。同文件 `:209-210` 又沉淀着「Rust 核心 2,100 行跨形态复用」这条已被推翻的地基断言。三处各说各话。

更麻烦的是 Phase 2 还没写：Codex 配置档位的文件里**必然**会出现 key（档位就是 provider + key 的组合），`provider.rs` 的原子写与掩码都还没做。按 `CONTEXT.md:196` 的原文，Phase 2 每一行都在违背这条口径；按 `:167`，这条口径从来没成立过。

## 决定

把口径改成一句可执行、可检查的话，写进 `CONTEXT.md` 取代原来的三处表述：

> **本项目不持有非本机既有登录态的密钥，不主动打厂商端点。**
> 分两层，两层都写清楚边界：
>
> **① 读：只借本机已有登录态，不向 agent 厂商索取任何东西。** 岛上的天数/用量/会话一律来自 agent 自己在本地落的文件与会话尾（`.codex/sessions/`、`~/.cursor/projects/*.sqlite`、`~/.dimcode/*.jsonl`）。**不调任何厂商 API**——不查 `api.openai.com/usage`，不查 Anthropic 的 quota-usage，不碰 OAuth 刷新。代价写明白：**配额百分比统读不到**，卡片上那一格空着（并显示「读不到」而不是 0），这与 `CONTEXT.md:79-92` 的「没查到 ≠ 零」是同一套记号。
>
> **② 写与发：用户自己配的通道用自己的凭据出网，我们不碰厂商端点。** SMTP/ntfy/webhook 的凭据由用户填、存钥匙串，只用于该通道；绝不将这些凭据复用去访问任何 agent 厂商的服务。这条与 ADR 0005（邮件出网只走 465）、ADR 0006（未核实的通道字段不承诺）是同一族：出网这件事必须窄、必须可枚举、必须逐条有降级行为。

**Phase 2 的档位文件按第 ① 层豁免处理，但加两条硬约束**：
- 档位文件里出现的 key **只在用户点上「显示」后才可读**，掩码做成类型而非布尔量（`MaskedSecret(Box<str>)`，`Debug`/`Display`/`Serialize` 全走掩码），漏掩码编译不过。
- 切换档位**不写 `$CODEX_HOME/auth.json`**——它含 `refresh_token`，写它等于碰登录态，与第 ① 层直接冲突。只读展示。

## 为什么不反过来选

另一条路是「收紧为真的零凭据」：删掉远程外发。代价是这个功能已经产品化（设置页 `RemoteNotifySettingsView`、四个 `RemoteNotify*` 文件、CLI 有 `notify` 子命令），且它解决的是真需求——agent 跑长任务时人不在电脑前。为了一句更干净的口径删掉一条能用的通路，是本末倒置。

第二条路是「维持原文」。代价是从今往后每个新功能都要跟这句谎打架：Phase 2 写第一行时就违规，Phase 3 若接任何后端同样违规。**口径必须是已做与将做的并集，不是过去某个时点的快照。**

## 明确接受的边界

- **配额百分比不接**。这是本决定最贵的代价，且它**有真实用户价值**（codenotch 靠它做卖点、Vorssaint 在 Linux 侧读 `plan-usage-history.json` 拿到了）。但它要么读 OEM 私库文件（Claude Code 的 JSONL USB API 未公开）、要么调厂商 API（第 ① 层禁止）、要么碰 OAuth 刷新（同禁）。**结论是 deferred，不是 decided-against**：哪天决定要接，先改这条 ADR，不要偷偷接。接入的判据必须一起写：读不到时显示什么（答案是「读不到」不是 0）、用户配了通道但出网失败时降级行为是什么。
- **本地 HTTP 服务面不出本机**。`LocalEventServer` 只监听回环，远程外发走用户自己配的通道——这两条不能混。
- **钥匙串里有什么不写进任何文档**。脱敏红线见 `docs/agent/desensitization.md`，本文只描述结构与边界，不出现值。
