# MonoCode 一手调研

> 一手核实记录。来源：[usemono.dev](https://usemono.dev/) 首页全文（2026-09-26 抓取）、
> 开源仓库 [github.com/hardbeat920/monocode](https://github.com/hardbeat920/monocode)（MIT，branch `main`）。
> GitHub API 元数据（2026-09-26）：1303 star / 147 fork / 创建于 2026-08-20 / 127 open issues（含 PR）/ 版本 `0.2.0`。
> 本文只记录「它是什么」「它怎么做到的」「和我们已有能力的关系」「我们要不要抄」，
> 不记录它每一版改了什么。所有引用按 `path:line` 给出，读不到的文件写「未获取」。

---

## 1. 一句话结论

**MonoCode 是「给 coding agents 当桌面前端」的路线**：它把 10 家 agent CLI 的全部协议吃进自己体内
（一家一个 adapter 约 700–37000 行），自己变成会话宿主；而我们走的是「监控 + 管理」路线——
只读会话尾与进程表。**这让我们的实现成本远低于它，但也让我们给不了它「agent 在 MonoCode 里跑」这件事。**
它最值得我们抄的不是任何功能，而是三件工程纪律：**Rust 侧与前后端与 `#[test]` 同级配的测试量**、
**凭据零落盘的分离式 profile 设计**、**不依赖协商的 `capabilities` 写法**。

---

## 2. A. 产品层

### 2.1 它到底是什么形态

README 第一句：「A desktop UI for your coding agents.」；首页 meta description：
「A desktop UI for the coding agents already on your machine.」

- **tabs = sessions，composer = 输入**（README 原文即这两句）——是「agent 的多标签外壳」。
- **不卖 token**：「MonoCode does not sell tokens.」它不碰钱，也不碰模型 API key。
- 界面规模从源码树看是 IDE 级别，不是「监控面板」级别：`src/features/` 下 20 个功能域
  （sessions / inbox / files / source-control / terminal / quick-composer / workspace /
  orchestration / automations / notes / skills / reminders / projects / providers / settings /
  notifications / search / agent-app）。sessions 一个域就有 204 个文件。
- usemono.dev 首页**只有一句标题 + 10 个 agent 名 + 两个下载按钮 + 五个平台徽标**，
  没有功能清单、没有截图、没有 pricing（`mono2.html` 可见文本节点共 21 个）。
  这与我们的「一个面板看全」是同一种克制。

### 2.2 十家 agent 怎么跑起来的

**不是 PTY，不是 ACP 通用协议，而是 spawn 子进程 + 按 provider 吃它自家的线协议。**
但这两条路它都有，且分工明确：

| 通道 | 用途 | 证据 |
| :--- | :--- | :--- |
| `harness_spawn`（管道 stdin/stdout 逐行） | 主通道。给 agent CLI 喂 JSON-RPC/协议行 | `src-tauri/src/harness.rs:412`（`Stdio::piped()`，`harness.rs:435-440`） |
| `pty_spawn` | 内嵌终端，用户自己敲命令 | `src-tauri/src/pty.rs`，`lib.rs:422-427` |
| `harness_http` / `harness_sse_open` | 走 HTTP/SSE 的 provider | `harness.rs:914`、`child.ts:333-349` |
| ACP（Agent Client Protocol） | **只有 Cursor 用** | `src/integrations/harness/core/acp.ts:19` 注释原文：「ACP JSON-RPC client — thin wrapper preserving the **Cursor** numeric-id API.」 |

所以「ACP」在它这里是**一个 provider 的方言适配器，不是跨 agent 的通用骨架**。
跨 agent 的通用骨架是 TypeScript 侧的 `HarnessAdapter`（见 §3.1）。
Rust 侧的自我定位写得很清楚（`lib.rs:45-46`）：

```rust
// Phase 1 seam: spawn / kill harness children per MonoCode thread.
// Adapters own the protocol; this host only supervises processes.
```

**这句话就是我们要不要抄的分界线**：它把协议留给前端，Rust 只管进程。

启动方式上它是**按需懒加载**：`refreshHarnessCatalogs(ids)` 只刷调用方真正需要的几家，
注释里给了理由（`core/registry.ts:375-379`）：「Boot used to refresh every adapter; that spawned
unused CLIs (Pi with extensions can sit at ~1GB) even when the workspace never touched them.」
它对缺失 CLI 的处理是探测后禁用 + 给安装提示，一个缺失的 Codex 不挡别的（CONTRIBUTING.md:29）。

### 2.3 `/operator`：让 agent 反过来操作 MonoCode 自己

这是它最有辨识度的一处设计，README 用了一整段讲。**它的权限模型是这套（全部有源码对应）：**

**(1) 入口是一次性的 composer 前缀。**
`consumeOperatorCommand`（`src/features/sessions/model/operatorCommand.ts:15-23`）匹配
`^\s*/(?:operator|mono|monocode)(?=\s|$)` 并从发往 agent 的文本里**剥掉这个前缀**
（README：「MonoCode removes the command from the request sent to the agent」）。
匹配后该 user block 打上 `monocode: true` 持久化字段，于是
`operatorEnabledInThread`（同文件:48-50）判定**该 thread 后续所有 turn 都带权限**，
换一个 thread 就没有（README：「Later turns in the same thread can use the CLI without repeating
`/operator`; other threads receive no CLI instructions or app access.」）。

**(2) 通道是一条本机回环 TCP + 一次性 token，而不是 Tauri command。**
`src-tauri/src/control.rs:1` 开头注释即设计声明：

```
//! Authenticated loopback transport. App windows own execution; callers never
//! receive arbitrary Tauri command access or direct database write access.
```

- 监听 `127.0.0.1:0`（随机端口，不写死），`control.rs:183`。
- 每行一个 JSON 请求，读超时 3s、写超时 3s、**单请求上限 256 KiB**（`control.rs:219-224`）。
- token 形如两个 `uuid::Uuid::new_v4().simple()` 拼接（`control.rs:56-59`），**进程内一次性**。
- **执行权在窗口，不在 agent**：Rust 侧只把请求 `emit_to(grant.window, "monocode-control-request")`
  转发给对应 webview 窗口，由前端的 control handler 执行再 `control_reply` 回填
  （`control.rs:254`、`control.rs:495-516`）。agent 拿不到任何 Tauri command 面。
- 单会话 pending 上限 24、读连接上限 32、工作线程 8（`control.rs:241`、`control.rs:196-197`）；
  回复超时 35s（`control.rs:261`）。
- 错误分「可重试/不可重试」两类，`retryable:false` 时告诉 agent 别重试（`control.rs:275-281`）。

**(3) 三条同时成立才放行（`request_grant`，`control.rs:161-176`）：**
1. namespace 是 `app`（`control` 是 orchestration 的另一套，见下）；
2. token 在 `app_grants` 里对得上；
3. 该 session 有**当前活跃的、且 `app_allowed=true` 的 turn**，且窗口还是当初那个窗口。

任一条不满足，返回固定文案（`control.rs:15`）：
「MonoCode app access is inactive. Use /operator once in this thread to enable it, then call the CLI
during an active agent turn. Retrying this request now will not enable access.」
**即：CLI 只在 agent 正在跑的那一个 turn 里可用，turn 一结束立刻失效**
（`control_turn_finished`，`control.rs:443-448`）。

**(4) 命令面是白名单，且白名单在前端强校验。**
`monoCodeToolCall`（`src/features/sessions/model/monocodeToolCall.ts:78-110`）：
agent 必须 shell 出一个严格形如 `monocode[.exe] app <action> [--json|--input|--request-id <v>]...` 的调用，
action 只能取自 9 个：`models.list`、`sessions.list`、`sessions.read`、`sessions.send`、
`sessions.draft`、`sessions.start`、`folders.list`、`folders.move`、`notes.list`、`notes.read`。
它自己写了一个**保守的 shell 词法分析器** `shellWords`（同文件:23-75）：
遇到 `$` / 反引号 / `;|<>` / 未闭合引号**直接返回 undefined（即判定「这不是 app 调用」）**，
注释写明「Conservatively parse one shell invocation; compound commands use the shell row」。
**它连"伪装成 app 调用"的路径都堵了。**

**(5) orchestration（多 agent 编排）是另一套、且被显式排除在 app 权限之外。**
README 原句：「Orchestration workers keep their existing scoped control workflow and do not receive
this app access.」源码对应：`request_grant` 里 `app` namespace 会检查该 session 是否在 `workers`
或 `grants` 里，是则拒绝（`control.rs:169-175`）；`control_attach_worker` 里更直接
`inner.app_grants.remove(&session_id)`（`control.rs:381`）——**成为 worker 的那一刻，app token 被删掉**。
worker 拿到的替代物是：一个私有 scratch 目录 + `TMPDIR/TMP/TEMP` 三个环境变量指向它（0700，
`control.rs:385-403`）。

**(6) 路径沙箱是真做了的，不是摆设。**
`resolve_scope`（`control.rs:549-578`）：写 scope 必须是项目相对路径、**禁止 `..`、禁止绝对路径、
禁止 Windows `Prefix`**，且 canonicalize 后必须仍 `starts_with(root)`；
`control_write_path`（`control.rs:581-606`）允许不存在的文件逐级上跳，但遇到 dangling symlink
直接报错——注释：「aliases and symlinks must not turn a private scratch directory into an exemption
for another worker's files.」写 scope 上限 64 个（`control.rs:610`）。

**(7) 沙箱与网络的已知边界，它自己写在注释里。**
`src/integrations/harness/core/types.ts:144-149`：

```
This session drives MonoCode's control CLI, which reaches the app over
loopback. Sandboxes deny network by default, so a lead that cannot open
that socket cannot supervise its agents at all.
```

**它承认「编排者必须能开回环 socket」这一点与默认沙箱冲突**，并把 `controlsAgents` 作为显式输入字段
暴露给调用方。这是我们做 `/operator` 类功能时第一个会撞上的墙。

**(8) 一次只能有一个编排者占一个 checkout。**
`control_enable`（`control.rs:301-311`）与 `control_authorize_turn`（`control.rs:419-431`）用
`paths_overlap` 做目录重叠判定（`control.rs:126-128`，含嵌套目录），重叠即拒绝，错误文案分别是
「Another session (…) is running in this checkout. Stop it before enabling orchestration.」
与「This checkout is controlled by an orchestrator. Stop that run before starting independent work.」
Windows 路径统一小写后比较（`comparison_path`，`control.rs:131-136`）。

### 2.4 它自己承认的限制

- README:46：「This is very early and you should expect bugs.」
- CONTRIBUTING.md:5 —— **暂停接受新 provider**：「Please don't open PRs that add a new provider
  right now. The existing harnesses still need to agree on a few patterns…」
  第 52 行重申「A PR that adds another provider will be closed for now, even if the work is good.」
  **这说明 10 家 adapter 之间的抽象还没收敛**——与我们「五种方言解析」处在同一阶段，只是它的面宽 10 倍。
- 127 个 open issue 里点名的一手限制（标题即原文）：
  - 「[Critical] Linux AppImage opens a black window: EGL_BAD_PARAMETER on CachyOS / AMD / KDE Wayland」
  - 「[Bug] macOS: LAN access denied on first launch after auto-update (relaunch fixes it)」
  - 「Sessions started directly in the Codex/Claude Code CLI aren't picked up by MonoCode (and vice versa)」
  - 「[Bug] Interface Scale slider jumps uncontrollably on Windows, snaps to 200」
  - 「Agent's shell tool calls doesn't show anything until the operation is completed」
  - 「Model catalogs go stale — latest Codex / OpenCode / provider models missing in-session and across restarts」
  - 「Handoff & Second Opinion disabled when only one harness is enabled」
  - 尚未做但被请求的：MCP 支持、Floating Terminal、SSH 远程目录、Keybindings 定制、Tasks、界面语言/简体中文。

---

## 3. B. 工程层

### 3.1 harness 抽象层（本项目最该看懂的一节）

**三层，职责钉死：**

```
src/integrations/harness/
├── core/                      与 provider 无关，全部是纯函数 + 单测
│   ├── registry.ts            HarnessAdapter 接口 + 注册表 + 并发队列 + 空闲停车
│   ├── types.ts               HarnessEvent（联合类型，~24 种）
│   ├── child.ts               spawn/write/kill 的 Tauri invoke 封装 + 事件桥
│   ├── jsonRpc.ts             JSON-RPC 编解码
│   ├── acp.ts                 ACP 客户端（只给 Cursor 用）
│   ├── streamText.ts          流式文本合并的唯一实现
│   ├── nativeCommands.ts      provider 自有 slash 命令的发现与转义
│   ├── abortTextPrompt.ts     一次性提问的 AbortSignal 竞争
│   ├── apply.ts preview.ts shellIntent.ts   事件→UI 模型的三层转换（34k/31k/17k 行）
│   └── availability.ts        探测哪家 CLI 装了
└── providers/<10 家>/         <name>.ts 协议 + <name>Adapter.ts 注册 + Catalog/Title/Git/Text
```

**① 接口 `HarnessAdapter`（`core/registry.ts:47-102`）** —— 24 个成员，其中 8 个是可选能力。
这比我们的「五种方言解析」更优的地方：**它把能力探测做成了可选的 `?` 成员 + 四个前置判定函数**：

```ts
canSteer?: boolean;                    // 默认同 live
compactContext?(input): Promise<void>;
rewindLastTurn?(input): Promise<...>;
refreshCatalog?(): Promise<void>;
generateTitle? / generateCommitMessage? / generatePrContent? / generateBranchName?
runTextPrompt?(input): Promise<string>;   // 旁路提问，不动主会话
```

对应 `canCompactHarnessContext` / `canSteerHarness` / `canRewindHarnessLastTurn` /
`canRunHarnessTextPrompt`（`registry.ts:235-267`、`443-446`）。UI 因此永远不做
`if (harness === "codex")` 分支——`registry.ts:43-46` 注释原话：
「App.tsx dispatches through the registry instead of harness-specific branches.」
**这条对我们的价值最高：我们的五种方言目前是分支，改成「可选能力 + 能力查询」后，
新增第六种方言不需要改任何调用点。**

**② `HarnessEvent`（`core/types.ts:13-132`）** —— 约 24 个成员的判别联合，覆盖
session/turn/message/reasoning/tool/agent.step(子代理)/approval/question/tasks/plan/context/
turn.metrics/background.updated/usage.limited。两个细节值得抄：

- `usage.limited` 带 `resetsAt`（`types.ts:26`）——**provider 主动说「我限流了，此时恢复」**，
  而不是我们去猜。
- `background.updated`（`types.ts:28-31`）带注释「The agent has yielded but the turn is not over:
  work it started is still running and will wake it again.」——**agent 主动让出但任务未完**的状态。
  这正是我们五态状态机里最难判的那种 "idle but not done"。

**③ `sendTurn` 的并发纪律（`registry.ts:116-160`）** —— 两条 promise 尾链：
`sessionOperationTails`（串行化 provider 状态操作）+ `sessionSteerTails`（steer 专用，且
`activeTurnSessions.has(sessionId)` 时**跳过 barrier**，注释：「A live turn must remain steerable
while its long-running send is pending.」）。这是我们「并发差分/状态机」最容易写错的一段。

**④ 空闲停车（`registry.ts:110`）** —— `HARNESS_IDLE_PARK_MS = 5 * 60_000`：turn 结束不杀子进程，
保温 5 分钟等下一句，超时才 `stopSession`（保留 resume 状态，下次 respawn）。
对照我们：我们根本没有子进程，但**这个"保温"思想对我们读会话尾的缓存 TTL 有直接参考价值**。

**⑤ 分层判断**：它的分层比我们**更薄也更正交**（core 纯函数 / providers 各管协议 / Rust 只管进程），
但代价是**每个 provider 一个巨型协议文件**：`claude.ts` 49754 字节、
`codex.ts` 37482、`opencode.ts` 43367、`antigravity.ts` 29825、`cursor.ts` 47338。
加上各自的 `*Protocol.ts`（codex 37028、claude 33028、fx 20779、pi 24054、opencode 16156…
**光 protocol 翻译层就是 20 万字节级**）。**这不是"更优的分层"，这是"更贵的分层"**——
它的可维护性靠 327 个前端测试 + 40 个 harness Rust 测试硬撑（见 §3.2）。
我们的五个方言解析合起来不到它一家的一半，维持现状（甚至加一个可��能力位）就够，不必引入 adapter 层。

### 3.2 测试规模（本项目最痛的对照点）

| 维度 | MonoCode | AgentIsland（实测） |
| :--- | :--- | :--- |
| 前端 `.test.ts` | **327 个**（harness 一个域 40 个，sessions 域 95 个） | 0 |
| Rust `#[test]` | **≥ 333 个**（实测逐文件 grep，见下） | **0** |
| Rust `tests/` 目录 | **无**（全部内联 `mod tests`） | 无 |
| CI | `.github/workflows/ci.yml`，macOS + Ubuntu + Windows 三平台矩阵 | 未获取（`.github/` 仅有 workflows/ci.yml？见未核实清单） |
| 本地一条命令 | `npm run check` = `check:web` + `check:rust`（package.json:13-15） | 自建 runner |

**CI 全文**（`ci.yml`，仅 41 行，一次跑全部）：
```yaml
strategy: { fail-fast: false, matrix: { os: [macos-latest, ubuntu-latest, windows-latest] } }
- run: npm ci
- run: npm test
- run: npx tsc --noEmit
- run: cargo fmt --check
- run: cargo clippy --workspace --all-targets -- -D warnings
- run: cargo check
- run: cargo test
```
注意两条**硬门**：`clippy … -D warnings`（warning 即失败）与 `cargo test` 与 `cargo check` 并列。
CONTRIBUTING.md:48 的承诺是「If it's green locally it should be green on GitHub.」

**Rust 侧测试逐文件实测数**（`grep -c '#\[test\]'`，2026-09-26）：
`fs.rs` 112、`harness.rs` 40、`session_store.rs` 43、`skills.rs` 20、`azure_devops.rs` 19、
`linear.rs` 11、`notes.rs` 10、`worktrees.rs` 10、`notifications.rs` 9、`gitlab.rs` 9、
`rate_limits.rs` 14、`window.rs` 13、`account_identity.rs` 6、`reminders.rs` 6、`automations.rs` 10、
`checkpoint.rs` 20、`control.rs` 7、`control_cli.rs` 6、`cursor_store.rs` 8、`pty.rs` 5、
`macos.rs` 5、`jira.rs` 15、其余 0–3。**合计 ≥ 333。**

三个可直接学的测试手法：

1. **协议层是纯函数 + 同级单测**。CONTRIBUTING.md:40 原话：
   「The protocol modules are pure functions with unit tests beside them, so you can fix a Codex
   parsing bug with only Claude Code installed.」
   **没有 agent CLI 也能测 agent 协议。** 我们的会话方言解析现在就是不可测的（依赖真实日志文件）。
2. **测试夹具自造，不连真服务**。`account_identity.rs:144-148` 用 `base64` 现场编码假 JWT claims 造
   `auth.json`；同文件 `:225-236` 专测畸形 token（`"no-dots"`、`"header.!!!.signature"`、非 JSON payload、
   以及 `{"OPENAI_API_KEY": "sk-x"}` **必须返回 None**）。
3. **跨平台行为用 `#[cfg]` 分测**。`lib.rs:583-593` 三个测试分别断言 `should_request_quit` 在
   Linux/Windows 下 `true`、macOS 下 `false`；`control.rs:694` 专测 `comparison_path` 保留 POSIX 大小写。

### 3.3 凭据处理（本项目最关心的安全对照点）

**结论：它不存任何凭据。它复用 agent CLI 自己的登录态，并用地�文件系统 + 钥匙串做隔离。**

**(1) 多账号 profile 的做法（`harness.rs:534-624`）**：
profile 目录 = `app_data_dir()/provider-accounts/{claude|codex}/{account_id}`，
`account_id` 校验为 ASCII 字母数字 + `-` `_`、≤80 字符（`harness.rs:551-559`）。
spawn 时注入隔离环境变量（`harness.rs:628-648`）：

| provider | 注入 | 同时移除 |
| :--- | :--- | :--- |
| claude | `CLAUDE_CONFIG_DIR`、`CLAUDE_SECURESTORAGE_CONFIG_DIR` 指向 profile 目录 | `ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN`、`CLAUDE_CODE_OAUTH_TOKEN` |
| codex | `CODEX_HOME` 指向 profile 目录 | `OPENAI_API_KEY`、`CODEX_API_KEY`、`CODEX_ACCESS_TOKEN` |

注释解释理由：「Claude scopes both its ordinary config and its macOS Keychain credential to these
exact strings. Setting both keeps profiles isolated on every supported platform.」
**它连"钥匙串条目也随 profile 分家"都处理了。**

**(2) 删号是真删（`harness.rs:581-604`）**：`provider_account_remove` 先 kill 该 profile 的全部子进程，
macOS 下调 `security delete-generic-password` 删掉该 profile 的钥匙串条目
（`rate_limits.rs:609-620`），再删目录；且先 `symlink_metadata` 检查，**符号链接只删链接本身、不递归**
（`harness.rs:594-600`）。

**(3) 它读 keychain 的唯一理由是拿 usage，不拿 token 用。**
`fetch_claude_usage`（`rate_limits.rs:402`）需要读 `~/.claude/.credentials.json` 或调
`security find-generic-password -w`，注释口径是「Read the signed-in identity a provider CLI already
cached on disk, so **no token is sent anywhere**」（`account_identity.rs:20-21`）。
`provider_account_identity` 只返回 `{email, name, plan, organization}` 四个字段
（`account_identity.rs:10-17`）——**它是一个显式的 DTO，不含任何 token 字段**。

**(4) DTO 纪律**：`ProviderAccountIdentity` 结构体带 `#[derive(Serialize)]`，
四个字段全 `Option<String>`，**没有任何凭据类型字段**；Codex 侧甚至只解 JWT payload 里的
`email`/`name`/`chatgpt_plan_type`/organizations（`account_identity.rs:95-123`），
**id_token 本身从不离开 Rust**。前端 `providerAccountCredentials.ts` 全文只有 13 行、
只有一个 `removeProviderAccountCredentials()` 函数——**因为前端根本没有任何凭据可操作**。
账号元数据（label、选择）走 `localStorage`（`providerAccounts.ts:5-6`），**只有标签没有密钥**。

**对照我们**：我们的口径（10-replan §7 决议③）已经是「档位元数据明文 + 凭据进钥匙串」。
MonoCode 给我们的**增量**是两条：
- 我们的档位若含 Claude Code profile，就应同时设 `CLAUDE_CONFIG_DIR` + `CLAUDE_SECURESTORAGE_CONFIG_DIR`
  两个变量，否则钥匙串条目会串号（这是我们未意识到的坑）。
- 删档位时必须 `security delete-generic-password -s <service>`，且 service 名是
  `"Claude Code-credentials"` 的前缀 + `SHA256(NFC(绝对路径))[:8]`（`rate_limits.rs:596-606`，
  含 NFC 归一化细节）。**不删就是留一个孤儿凭据在用户钥匙串里。**

### 3.4 代码组织与规模

| 指标 | 实测 |
| :--- | :--- |
| 仓库文件总数 | 1037（git tree API，`truncated: false`） |
| TS | 643 个文件 / 4.61 MB |
| TSX | 157 个文件 / 2.43 MB |
| Rust | 41 个文件 / 1.30 MB（`lib.rs` 20 KB，最大 `fs.rs` 265 KB） |
| 前端测试 | 327 个 |
| Rust 测试 | ≥ 333 个 |

`src/features/<域>/{model,ui,hooks}` 三件套，**测试与实现同目录**
（`CONTRIBUTING.md:34` 原话：「`src/features/` - product behavior grouped by feature, with UI,
model, data access, hooks, and tests kept together」）。
`src/shared/` 明确限定「contains no feature behavior」（同文件:37）。
`src/platform/tauri/` 是所有 Tauri invoke 的唯一出口——**UI 层不直接 import `@tauri-apps/api`**
（harness 的 `child.ts` import 了，但 features 层走 platform 适配器）。

**它在这么大体积下保持可维护，靠的不是分层而是四道硬门**：
`tsc --noEmit`、327 个 vitest、`clippy -D warnings`、333 个 `cargo test`，加上 CI 三平台矩阵。
CONTRIBUTING.md:5 说 past small fixes, the door is open——**单人维护者主动限流**（连新 provider PR
都关）。这与我们的单人开发同构，但我们的门比它松一到两个数量级。

### 3.5 `capabilities/default.json` 与窗口声明（对照我们的 bug）

它的 `windows` 通配，不列窗口名（`src-tauri/capabilities/default.json`）：

```json
{ "identifier": "default", "description": "Capability for the main window",
  "windows": ["*"], "permissions": [ ... 21 项 ... ] }
```

**这是正确的写法**：`"windows": ["*"]` 覆盖 conf 里声明的 `main` 与运行时创建的
`quick-composer` / `quick-composer-git`（`src-tauri/src/window.rs:19-21`）以及
`WebviewWindowBuilder` 动态新建的窗口（`window.rs:98`）。
我们的 `app/src-tauri/capabilities/default.json:5` 写成 `"windows": ["island", "main"]`，
而 `app/src-tauri/tauri.conf.json:13` 只声明了 `island` —— **引用了一个不存在的 `main`**。
按 MonoCode 的做法，我们应改成 `["island"]` 或 `["*"]`（若保留未来多窗口）。

另两条可抄的 conf 纪律：
- CSP 写全，且 `connect-src` 只放 `ipc: http://ipc.localhost https://ipc.localhost` 与
  `asset: http://asset.localhost`（`tauri.conf.json:31`）——**前端 fetch 只被允许打自己的 asset 协议**。
  `devCsp` 单独放宽到 1420 端口（同文件:32）。我们的 `"csp": null` 等于关掉。
- 平台差异用 `tauri.linux.conf.json` / `tauri.windows.conf.json` **自动合并**
  （README:83「Tauri loads src-tauri/tauri.linux.conf.json automatically for Linux development and
  builds.」），三个文件都声明 `label: "main"` 且只改 `decorations`/`transparent`/`backgroundColor`/
  `bundle.targets`。**窗口 label 在三平台保持一致，只有外观分平台。**

---

## 4. C. 差距与取舍

### 4.1 功能面重合度

| MonoCode 的功能 | 我们已有的对应物 | 重合度 | 判断 |
| :--- | :--- | :--- | :--- |
| **Agent 运行状态监控**（`liveAgents.ts`：activity 文案 / 计时 / needsApproval / done / 后台任务） | 五态状态机、CPU 差分、会话尾读、可观测性五类 | **部分重合** | **我们更强**：它靠 agent 主动 emit event；我们读进程表与日志尾，**不依赖 agent 配合**。它 issue 里「Sessions started directly in the Codex CLI aren't picked up by MonoCode」正是我们的强项场景 |
| **notes** | ToDos（待办） | 低 | 不同物。它的 notes 是 `notes.list` 只回标题+预览、`notes.read` 按 ID 取全文（README:59）——**设计成给 agent 低成本浏览**；我们的 ToDos 是用户自己管 |
| **inbox**（GitHub / GitLab / Jira / Linear / Azure DevOps 五家集成） | 无 | 无 | **不做**。见 §6 |
| **automations**（`automations.rs` 42 KB，cron/事件触发） | 无 | 无 | **不做**（Phase 内） |
| **worktrees / checkpoint**（`worktrees.rs` 45 KB / `checkpoint.rs` 72 KB） | 无 | 无 | **不做** |
| **orchestration**（多 agent 编排 + worker scratch + checkout 独占） | 无 | 无 | **不做**。这是它最重也最危险的一块 |
| **quick-composer**（全局快捷键浮层输入） | 灵动岛本身 | **高度重合** | 见 §5.1，**唯一值得细看的 UI 项** |
| **rate limits**（Claude usage / OpenCode Go usage） | Token 净消耗统计（读本机会话日志） | 部分重合 | 我们已有且口径不同（它读 keychain 打官方 API；我们读日志）。**不合并** |
| **reminders / notifications** | 远程通知 + 在场判定 | 重合 | 已有，不必改 |
| **skills**（`skills.rs` 38 KB） | `npx skills` + `.agents/skills` | **高度重合** | 见 §5.3，**它的目录优先级表可直接抄** |
| **provider accounts**（多账号切换） | CC Switch（档位切换） | **高度重合** | 见 §5.2 |
| **search / files / terminal / source-control** | 无 | 无 | IDE 功能，**与我们定位冲突，不做** |

### 4.2 根本分歧（这一节是全文重点）

```
MonoCode：  agent CLI ──spawn──▶ MonoCode（自己当会话宿主）
                                    │  吃下全部协议的客户端侧
                                    ▼
                                用户在这里打字、看 diff、批 approval

AgentIsland：agent CLI ──照常独立运行──▶ 用户的终端
                    │                            │
                    └── 我们只读 ◀───────────────┘
                        会话日志尾 / 进程表 / 配置文件
                                    │
                                    ▼
                            灵动岛/侧边栏：看清 + 管配置 + 管待办
```

**这个分歧决定了三件事：**

1. **实现成本差一个数量级，且不可比。** 它 1037 个文件、41 个 Rust 文件里有 265 KB 的 `fs.rs`
   （IDE 的文件树 + git 全部操作）与 42 KB 的 `automations.rs`；我们 11 个 Rust 文件 2673 行。
   **它的复杂度不在"监控"上，在"当 IDE 前端"上。**
2. **我们给不了它「agent 在 App 里跑」。** 这是产品级差异，不是工程量差异。
3. **我们给得了它给不了的：不依赖 agent 配合。** 它必须每家 CLI 都装好、登录好、协议吃透
   （10 个 adapter 至今未收敛，连新 provider PR 都关）；我们的 `ProcessMonitor` 只要进程在就有状态。

**因此：MonoCode 是我们的「形态参照」而不是「功能参照」。** 它证明了三件事对我们有市场意义——
「agent 的桌面 GUI 有人愿意做」「Tauri v2 + Rust 后端这条路走得通（1303 star / 37 天）」
「克制首页也卖得动」。它**不**证明我们该做会话宿主。

---

## 5. 可直接抄的五件事

### 5.1 quick-composer 的 NSPanel 做法（`src-tauri/src/quick_composer.rs:590-676`）

**它解决的是和我们灵动岛同一个问题：一个不抢焦点的全局输入浮层。** 具体手法：

- 运行时**动态建 NSPanel 子类**，且必须带 Tao 的 `focusable` ivar（`quick_composer.rs:596-600`
  注释：「A runtime `NSPanel` subclass rather than `define_class!`, because it has to carry Tao's
  `focusable` ivar: objc2 only re-classes an object into a class of exactly the same instance size」）。
- `canBecomeKeyWindow → YES`（要收键盘）、`canBecomeMainWindow → NO`
  （同文件:607-616，注释：「Never main: that is what would pull the workspace window forward.」）。
- 换类前先断言 `panel_class.instance_size() == ns_window.class().instance_size()`，
  不等就**降级为普通窗口并打 stderr**（同文件:641-647）——**这是对上游 Tao 改布局的防御**。
- `_setPreventsActivation:YES` 走私有 selector，且**先 `respondsToSelector:` 探测**
  （同文件:654-660，注释：「Setting the mask after creation does not update WindowServer's
  activation tag on its own; this private setter does.」）。
- `setLevel(NSStatusWindowLevel)` + collectionBehavior 四件套
  `CanJoinAllSpaces | FullScreenAuxiliary | Transparent | IgnoresCycle`（同文件:663-669）。
- 位置策略：无 monitor 时居中，否则水平居中、垂直取 `area.height * 0.22`
  （`TOP_FRACTION = 0.22` 定义在 `quick_composer.rs:48`，用于 `:586`）。
- 快捷键默认 `Command+Shift+Space`，且**自定义快捷键有一套校验**
  （`quickComposerShortcut.ts:41-58`：必须含 Command 或 Control、修饰键 1–4 个不重复、键必须在白名单）。
- `window-state` 插件**把 quick-composer 两个窗口加进 denylist**，不持久化它们的位置
  （`lib.rs:213-220`）。

**对我们的动作**：读一遍这段，对照 `IslandPanelInteraction.swift`。重点是第 4、6 条——
我们若用过私有 selector / 未探测 `respondsToSelector`，存在系统升级即失效的风险。

### 5.2 CLI 化的开关与状态探测（`quickComposerShortcut.ts` + `window.rs:19-21` + `lib.rs:213-220`）

**一个布尔开关 + 一个快捷键 + 一个不持久化窗口**，三行配置就能加一个全局唤出形态。
`quick_composer_set_enabled`（`lib.rs:470`）走 Tauri command，快捷键的动态注册在
`quick_composer.rs:186-192`（`app.global_shortcut()`）。**这条直接可搬到我们的「双形态并存、可切换」**：
侧边栏是主窗口、灵动岛是 denylist 化的独立浮层，互不抢 focus。

### 5.3 skills 的目录优先级表（`src-tauri/src/skills.rs:120-170`）

**它与我们的 `npx skills` / `.agents/skills` 是同一件事，且它的优先级表比我们更细**：

```
.agents/skills（project）          ← 最高
~/.agents/skills（user）
.claude/skills · .cursor/skills · .codex/skills · .opencode/skills · .pi/skills
.omp/skills    · .fx/skills        · .grok/skills   · .hermes/skills   （project 然后 user）
~/.pi/agent/skills · ~/.omp/agent/skills（user）
~/.gemini/antigravity/skills（user，仅当目录存在）
Claude plugin 的 skill（命名空间化为 <plugin>:<name>）
```

三条可抄的规则：
- **`.agents/skills` 第一**，且 `by_name.entry(name).or_insert(...)` 保证**同名不覆盖**
  （`skills.rs:125`）。
- **新 provider 根永远排在所有既有根之后**，注释：「New-provider roots come after every
  pre-existing root so an identically named skill can never shadow an established provider.」
  （`skills.rs:160-162）。
- **去重按 canonicalize 后的根**，`seen_roots` 去重（`skills.rs:105-110`）；
  禁用列表同时按规范化字符串与 canonical path 双匹配（`skills.rs:28-60）。
- 上限 300 个 skill、frontmatter 上限 16 KiB（`skills.rs:9-10`）。
- frontmatter 自己解析（不引 serde_yaml），且**显式处理 folded scalar `>`**
  （`skills.rs:490-497`、`is_folded_scalar`）。

**对我们的动作**：我们的 skills 发现逻辑加 `.agents/skills` 优先 + `or_insert` 同名不覆盖 +
canonical 去重。另外**它把「禁用」存在 localStorage 按路径匹配**
（`DISABLED_SKILL_PATHS_KEY = "monocode.disabledSkillPaths"`，`skills.ts:26`），
我们目前 disable 的口径值得对一遍。

### 5.4 用假 JWT 测凭据解析（`account_identity.rs:125-237`）

`codex_auth()` 夹具（同文件:144-148）用 `base64` 现场编码假 claims 造 `auth.json`，
四条测试覆盖：完整 claims / 缺 email 退到 `profile.email` / 无 `oauthAccount` 判未登录 /
**畸形 token 一律 None（含 `{"OPENAI_API_KEY":"sk-x"}` 这种"看起来像凭据"的输入）**。
**这条对我们要写的钥匙串读写是直接可复用的测试模板**，且它示范了「输入像凭据也返回 None」的边界。

### 5.5 `streamText.ts` 的流式合并唯一实现（`src/integrations/harness/core/streamText.ts`）

全文 56 行两个纯函数，注释把踩过的坑全写进去了：
`joinStreamText`（token 追加 vs 全量快照的判别，**明确否决 overlap 匹配**——
「Overlap matching is never used: it ate blank lines, headings, table rows, and doubled letters.」
同文件:8-9）+ `snapshotRemainder`（完成后快照与已流式内容的差分）。
以及 `\n` + `\n` 不是"一个字符的快照"而是 Markdown 段落分隔的边界（同文件:15-17）。
**我们的会话尾读若要做「增量 vs 快照」判别，直接抄这两个函数并补单测**——这是全场性价比最高的一段。

---

## 6. 明确不抄的三件事

### 6.1 不做 inbox / automations / orchestration / worktrees / checkpoint

**理由一：与「监控 + 管理」的定位正面冲突。** 它们要求我们**写用户的仓库、开 PR、起子 agent、
管多个 worktree**。我们的 `--help` 定位是「监控 Agent」；一旦开始 `git_commit`/`git_pr_create`
（`lib.rs:299-327` 那一串 Tauri command），产品性质就变了，与 Magpie 一节的判断同构
（`05-magpie-research.md` §3.3「与 `--help` 声明的定位漂移」）。

**理由二：代价不可比。** `azure_devops.rs` 81 KB + `jira.rs` 42 KB + `linear.rs` 30 KB +
`gitlab.rs` 40 KB = **单是五家 issue tracker 集成就 ~193 KB Rust**。加上 `checkpoint.rs` 72 KB
（文件级快照/undo）、`worktrees.rs` 45 KB、`automations.rs` 42 KB、`fs.rs` 265 KB。
**我们整个 Rust 端 2673 行。** 抄任何一件都意味着把工作台改造成 IDE。

**理由三：orchestration 的安全面是我们明确不要的。** `control.rs` 844 行里，
光是 grant 生命周期 / 路径沙箱 / checkout 独占 / worker scratch 就占了大半，且它自己承认
「沙箱默认禁网，编排者必须能开回环 socket」这个未解冲突（`types.ts:144-149`）。
**为这个付 844 行 Rust 的安全责任，换来的东西用户用我们的灵动岛时不需要。**

### 6.2 不引 `src/features/` 的 feature-based 目录到 Rust 端

它 6.61 MB TypeScript 靠 660 个测试撑着；我们的 UI 体量（`Sources/AgentIsland/` 97 个 Swift
文件 29526 行 + `app/ui`）远小于它，**照搬目录规范只会增加导航成本、不减少 bug**。
另外它的 `features/*/model` 里塞了 30–50 KB 的纯逻辑文件（`ompInterjections.ts` 14 KB、
`sessionFolders.ts` 20 KB），**这在我们这里是过度设计**：我们的等价逻辑在 Swift Core 里，
且拆分粒度应由我们自己的功能面决定。

### 6.3 不抄它的「provider 一个巨型协议文件」结构

`codex.ts` 37 KB + `codexProtocol.ts` 37 KB = 一家 74 KB TypeScript。
我们的五种方言解析加起来不到这个数的一半，**引入 adapter 层会让"加一种方言"从「加一个 parser」
变成「加一个目录 + 注册 + 能力位 + catalog + title + git + text 八个文件」**。
只抄它的**能力位思想**（`registry.ts:47-102` 的 `?` 可选成员 + `canXxx()` 查询函数），
不抄它的目录体积。CONTRIBUTING.md:5 连它自己都说新 provider PR 要关——
**这个结构还没被证明收敛**，我们不该在它收敛前照抄。

---

## 7. 未核实清单

1. **CI 的完整内容**：`ci.yml` 已逐行读到（三平台矩阵 + 7 条命令），但
   `.github/workflows/release.yml`（17.6 KB）**未获取**——不知道它的签名/公证/发布流程。
2. **`main.rs`**：582 字节，抓取超时未成功；推测只调 `monocode_lib::run()`，**未证实**。
3. **`providerAccounts.ts` 后半段**（`selectedProviderAccountId` / `writeJson` / `validAccountId`）
   只读到前 6 KB，localStorage 的具体 key 前缀 `monocode.providerAccounts.v1` 已确认（第 5 行），
   **键名是否含明文 token 未逐行核对**（从头 6 KB 看只有 id 与 label）。
4. **Rust 测试数 ≥ 333 是逐文件 grep `#[test]` 的和**；有几个文件（`lib.rs` 出现两次超时）
   实测值可能低估。`fs.rs` 112 是单文件测得的最大值。
5. **`notes.rs` / `reminders.rs` / `checkpoint.rs` 的具体存储位置**（sqlite? 文件?）
   **未读**。`session_store.rs` 用 rusqlite 已从 `control.rs:533-545` 的 SQL 佐证。
6. **`usemono.dev` 只有一句 slogan**（首页可提取文本 21 个节点，Next.js 客户端渲染，
   其余内容在 JS bundle 里）。**它的完整功能列表、定价（若有）未获取。**
7. **acceptance / 性能数据**：未核实 MonoCode 的内存占用、启动时间、包体积。
   README 只声称支持 macOS/Linux/Windows，Magpie 那类「<15 MB」口径在 MonoCode README 中**不存在**。
8. **它是否处理 `prefers-reduced-motion`**：**部分核实**。`src/styles/index.css` 有 **12 处**
   `@media (prefers-reduced-motion: reduce)`；`useQuickPickerMotion.ts:44` 在 JS 侧
   `if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;` 提前退出。
   **但 `shared/lib/motion.ts` 本身（849 字节，3 个函数）只读 CSS 变量，不读 reduce-motion**——
   `reorderMotion()` / `tabCloseDuration()` 是无条件返回时长的。**结论：CSS 全覆盖，JS 动画逐个函数手动兜。**
9. **我们 `.github/` 是否有 CI**：未核实（本次任务不要求，且不得改项目文件）。
10. **`quick_composer/screenshots.rs`（截屏捕获）与 `quick_composer/git_popup.rs`（git 快弹）**
    未读内容，只知存在（`lib.rs:482-494`）。
