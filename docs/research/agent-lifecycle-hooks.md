# 各家 Agent 能不能自己上报生命周期（hook / notify / plugin 面核实）

> 目的：在写自报通道（`.scratch/agent-selfreport/spec.md` §3）之前，把「哪一家有**进程级确定
> 触发**的生命周期载体」测清楚。本仓在同类问题上吃过亏（`docs/adr/0006`：没核实到正文就预置
> 字段名，症状是「界面显示已送达、其实没送」），所以本文每条结论后面只允许两种东西：
> **可引用的正文原句**，或**本机配置文件实样**。两者都没有的，写成排除结论，不写成猜测。
>
> 核实日期：2026-09-23。方法：官方文档抓取 + 本机 `~/.claude`、`~/.codex`、
> `~/.config/opencode` 实样 + 已安装 SDK 的类型定义。文中不出现任何令牌值与个人路径。

## 一句话结论

**五家有可用的生命周期载体**（Claude Code、Codex、Qoder、Trae、OpenCode），且比 spec 原先
假设的更好：Claude Code / Codex / Qoder 共用**同一套 PascalCase 事件词汇**
（`SessionStart` / `Notification` / `Stop` / `PermissionRequest` / …），一份桥接脚本三家通吃；
**Qoder 还有 `type: "http"` 的 hook**，事件 JSON 直接 POST 到 URL ⇒ 它连桥接脚本都不需要，
配一条 http hook 就能打进 §3 的 `/session`；Codex 的 `notify` 已是 **legacy**（载荷走 argv、
stdin 被置空），照它设计会订错接口；OpenCode 的 plugin 事件总线里有 `session.idle` 与
`session.error`——唯一「终态不依赖模型自觉」的载体，满足 spec 判断 ①。
Cursor 也有完整 hooks，但事件名是 **camelCase**，得走映射表；Trae 原文写着它**直接读 Claude
Code 的配置**；Roo Code 是唯一一条「按现行官方文档无此能力」。

> **本文的核实纪律**（第一版在这里栽过一次）：`本机没有配置文件` 与 `官方没有这个能力`
> 是两个命题。第一版把前者当成后者，把 Qoder / Cursor / Trae / Cline 全记成「排除」，
> 而它们都有公开正文。现在每家都必须落到「抓到的正文原句」或「本机实样」上，
> 两者都没有的，进文末「待核实清单」领一个编号并标状态（状态的分类与出路见那张表），
> 不许进实现，也不许用一个笼统的词糊过去。

## 为什么「载体是什么」比「有没有埋点」重要

spec 判断 ①：生命周期不能交给 MCP 工具调用，因为调不调是**模型决定**的，而任务结束那一刻
模型已经不再产生动作。按这条尺子量，三类载体的差别是决定性的：

| 载体 | 谁触发 | 能不能承载终态 |
| :--- | :--- | :--- |
| command hook（Claude / Codex） | 运行时在事件点直接起进程 | ✅ 能，`Stop` / `SessionEnd` 就是终态 |
| plugin 事件总线（OpenCode） | 运行时 `emit`，插件被动收 | ✅ 能，`session.idle` 确定发出 |
| legacy `notify`（Codex 旧） | 运行时起进程，但只给 argv | ⚠️ 能，但接口与新一代不同形 |
| MCP 工具调用 | **模型自己决定** | ❌ 不能承载生命周期（spec 判断 ①） |

## Claude Code —— P0 首选

- **配置文件**（文档列举）：`~/.claude/settings.json`（用户级）、`.claude/settings.json`
  （项目级）、`.claude/settings.local.json`；插件另用 `hooks/hooks.json`。
- **键形状**（文档示例，原样照抄）：

  ```json
  {"hooks": {"PreToolUse": [{"matcher": "Edit|Write",
              "hooks": [{"type": "command", "command": "/path/to/lint-check.sh"}]}]}}
  ```

- **事件名**（文档列举，取与生命周期相关的十个）：`SessionStart`、`SessionEnd`、
  `UserPromptSubmit`、`Notification`、`Stop`、`SubagentStop`、`PermissionRequest`、
  `PreToolUse`、`PostToolUse`、`PreCompact`。文档列出的全集更长，实现不必一次订完。
- **载荷**：命令从 **stdin 收 JSON**，含 `session_id`、`cwd`；`CLAUDE_PROJECT_DIR` 在环境里可读。
  `matcher` 按事件相关值做正则或精确匹配。
- **本机实样（交叉验证）**：`~/.claude/settings.json` 里确有
  `hooks.UserPromptSubmit[].hooks[] = {type: "command", command: "codegraph prompt-hook"}`
  （第三方工具 codegraph 于 2026-08-28 写入）⇒ 与文档形状一致，不是只信了文档。
- **无头模式**：`claude -p` 下是否逐事件触发 ⇒ **V2**。
- **接入片段**（`~/.claude/settings.json`，桥到 spec §3 的 `/session`）：

  ```json
  {"hooks": {
     "Notification":     [{"hooks": [{"type": "command", "command": "agentisland-hook claude attention"}]}],
     "Stop":             [{"hooks": [{"type": "command", "command": "agentisland-hook claude completed"}]}],
     "SessionStart":     [{"hooks": [{"type": "command", "command": "agentisland-hook claude working"}]}]}}
  ```

  `agentisland-hook` 读 stdin 的 JSON，取 `session_id` 当会话标识，按 `state` 上报，
  令牌从 `~/Library/Application Support/AgentIsland/report.token` 取。

## Codex —— P0 次选，但别照 `notify` 设计

**两代机制并存**，spec 原先只写了 `notify`，会订错接口。

- **`notify` 是 legacy**：`~/.codex/config.toml` 顶层 `notify = ["<program>", …]`。
  载荷**追加为最后一个 argv**——`codex-rs/hooks/src/legacy_notify.rs` 注释原文：
  *"Legacy notify payload appended as the final argv argument for backward compatibility."*
  且 **stdin 被显式置空**（同文件 `Stdio::null()`）。载荷 JSON 的判别字段是
  `"type": "agent-turn-complete"`，其余字段 kebab-case：`thread-id`、`turn-id`、`client`。
- **新一代是 `hooks.json`**，事件名原文见 `codex-rs/hooks/src/lib.rs`，注释原文：
  *"Hook event names as they appear in hooks JSON and config files."*

  ```
  PreToolUse · PermissionRequest · PostToolUse · PreCompact · PostCompact ·
  SessionStart · SessionEnd · UserPromptSubmit · SubagentStart · SubagentStop ·
  Stop · Interrupt
  ```

  **与 Claude Code 同一套词汇** ⇒ 桥接脚本可以两家共用，只差配置落点。
- **matcher 有坑**：同文件的 `HOOK_EVENT_NAMES_WITH_MATCHERS` 只含 9 个事件，注释原文说明
  其余事件「can appear in hooks JSON, but Codex ignores their matcher fields」。
  `Stop` / `Interrupt` 正在被忽略之列 ⇒ **别想用 matcher 过滤终态事件**。
- **载荷**：stdin JSON——`codex-rs/core/src/hook_runtime.rs` 注释原文：
  *"`tool_name` is the canonical name serialized to hook stdin"*。
- **托管环境会让自报整体失效（必须记着）**：`docs/config.md` 原文——管理员可在
  `requirements.toml` 里设 `allow_managed_hooks_only = true`，"to ignore user, project, and
  session hook configs"。⇒ 企业机器上 hooks 可能一条都不发，岛必须照旧推断，
  这正是 spec 判断 ②「存在性判定不许被关掉」的现实理由。
- **插件 hooks**：`<plugin_root>/hooks/hooks.json`，handler 除 `command` 外还有
  `command_windows`（跨平台）。
- **本机实样**：`~/.codex/hooks.json` 存在（2026-09-06，同为 codegraph 写入，形状与
  Claude Code 一致）；`~/.codex/config.toml` 首行即 `notify = […​, "turn-ended", …]`；
  MCP 面在 `[mcp_servers.*]`。
- **无头模式**：`codex exec` 下是否触发 ⇒ **V3**。

## OpenCode —— 唯一给到「确定终态」的一家

- **载体是 plugin，不是 hooks.json**：目录 `~/.config/opencode/plugins/`（本机存在，当前为空），
  SDK 包名 `@opencode-ai/plugin`。
- **`Hooks` 接口键名**（本机安装的 `@opencode-ai/plugin/dist/index.d.ts`，第 173 行起，原文）：
  `event`、`config`、`auth`、`provider`、`dispose`、`tool`、`"chat.message"`、`"chat.params"`、
  `"chat.headers"`、`"permission.ask"`、`"command.execute.before"`、`"shell.env"`、
  `"tool.execute.before"`、`"tool.execute.after"`、`"experimental.session.compacting"`、
  `"experimental.compaction.autocontinue"`。
- **事件总线的名字**（本机 `@opencode-ai/sdk/dist/gen/types.gen.d.ts`，原文）：
  `session.idle`、`session.error`、`session.created`、`session.updated`、`session.compacted`、
  `session.interrupt`、`permission.updated`、`permission.replied`、`message.updated`、
  `message.part.updated`、`file.edited` 等。
- **为什么单独记它**：`event(input: { event: Event })` 是运行时 emit、插件被动收。
  `session.idle` 在回合结束时**确定发出**，`session.error` 覆盖报错——判断 ① 要的东西
  在这里是现成的，不需要模型配合。
- **MCP**：`~/.config/opencode/opencode.json` 顶层键含 `mcp`、`permission`（本机实样）。

## Qoder —— 有公开文档的完整 hooks 面（上一版把它误判为排除）

出处：`docs.qoder.com/cli/hooks` 与 `docs.qoder.com/extensions/hooks`（2026-09-23 抓取正文）。

- **配置文件**（原文列举）：`~/.qoder/settings.json`（用户级）、`${project}/.qoder/settings.json`
  （项目级，可提交给团队共享）、`${project}/.qoder/settings.local.json`（本地，建议进 .gitignore）。
- **键形状**（原文示例）：`hooks.<EventName>[] = {matcher, hooks: [{type, command, timeout}]}`，
  `type: "command"` 那层还支持 `args: [...]`（exec 形式，不过 shell）与 `env: {…}`（额外环境变量）。
- **事件名 23 个**（CLI 全集，原文照抄）：`SessionStart`、`SessionEnd`、`UserPromptSubmit`、
  `PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`PermissionDenied`、
  `Stop`、`StopFailure`、`SubagentStart`、`SubagentStop`、`PreCompact`、`PostCompact`、
  `Notification`、`InstructionsLoaded`、`ConfigChange`、`CwdChanged`、`FileChanged`、
  `WorktreeCreate`、`WorktreeRemove`、`Elicitation`、`ElicitationResult`。
  IDE 侧只有其中 12 个，文档原文说明配置在 IDE 与 CLI 之间共享，但**每个入口只跑自己支持的事件**
  ⇒ 同一份配置在两边行为不同，自报接入要按入口分别验。
- **载荷**：stdin JSON，公共字段 `session_id`、`transcript_path`、`cwd`、`hook_event_name`、
  `permission_mode`、`agent_id`、`agent_type`；env 有 `QODER_PROJECT_DIR`、`QODER_PLUGIN_ROOT`、
  `QODER_PLUGIN_DATA`。
- **`type: "http"` 是本轮最有用的一条**：文档说明 hook 输入会以 JSON POST 到指定 URL，
  并期待一个 JSON 响应；`headers` 的值支持 `${ENV_VAR}` 插值，且由 `allowedEnvVars` 白名单约束。
  ⇒ 对 §3 的设计意味着：**Qoder 不需要桥接脚本**，一条 http hook 直接打到
  `127.0.0.1:41999/session`，令牌用 header 插值带进去。这是五家里唯一「配置即接入」的一家。
- **本机实样**：`~/.qoder/settings.json` 只有 `enabledPlugins` 一个键 ⇒ 机制存在、本机未配置。
  （上一版把这条观察写成了「Qoder 无可配置面」，那是错的：本机没配 ≠ 官方没有。）
- **无头模式**：headless / 脚本模式下是否触发 ⇒ **V4**。

## Cursor —— 有，但事件词汇与 Claude 家不同构

出处：`cursor.com/docs/hooks`（2026-09-23 抓取正文）。

- **配置文件**（原文）：`~/.cursor/hooks.json`、`<project-root>/.cursor/hooks.json`，
  企业分发 `/Library/Application Support/Cursor/hooks.json`、`/etc/cursor/hooks.json`、
  `C:\\ProgramData\\Cursor\\hooks.json`。
- **键形状**：根上要有 `"version": 1` 与 `"hooks": {}`，内层数组每项至少给 `command`。
- **事件名是 camelCase**（原文列举）：`sessionStart`、`sessionEnd`、`preToolUse`、`postToolUse`、
  `postToolUseFailure`、`subagentStart`、`subagentStop`、`beforeShellExecution`、
  `afterShellExecution`、`beforeMCPExecution`、`afterMCPExecution`、`beforeReadFile`、
  `afterFileEdit`、`beforeSubmitPrompt`、`preCompact`、`stop`、`afterAgentResponse`、
  `afterAgentThought`、`beforeTabFileRead`、`afterTabFileEdit`、`workspaceOpen`。
  ⇒ **别家都是 `PascalCase`，Cursor 是 `camelCase`**：共用桥接脚本时事件名要过一张映射表，
  不能像 Claude/Codex/Qoder 那样直接复用。
- **载荷**：原文"receive JSON input via stdin"；env 含 `CURSOR_PROJECT_DIR`、`CURSOR_VERSION`、
  `CURSOR_USER_EMAIL`、`CURSOR_TRANSCRIPT_PATH`、`CURSOR_CODE_REMOTE`，还兼容 `CLAUDE_PROJECT_DIR`。
- **云端**：调研记录称仓库里的 `.cursor/hooks.json` 会被 cloud agents 执行、而 `~/.cursor/` 那份
  不可用；本轮抓取的正文只确认到「cloud agents 跑仓库里的 command hooks」那一句
  ⇒ 后半句的原文位置见 **V5b**。
- **无头模式**：`agent -p`（Headless/CI）下是否触发 ⇒ **V5a**。

## Trae —— 直接读 Claude Code 的配置

出处：`docs.trae.ai/ide/automate-actions-with-hooks`（2026-09-23 抓取正文）。

- 原文：**"TraeCode supports reading hook configurations from Claude Code."**
  ⇒ 已为 Claude Code 写好的那份 hooks 配置可能被 Trae 直接吃下，接入成本近乎零。
- 六个事件与触发时机（原文）：`SessionStart`（"After creating a session, before initiating the
  first chat"）、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`Stop`（"When the agent
  completes output and prepares to end the current query"）、`Notification`。
  `Notification` 原文注明"triggered asynchronously and does not block the main process"，
  触发条件是「工具调用等确认」或「任务完成」——**一个事件同时覆盖 attention 与 completed**，
  桥接时不能只按事件名定 state，得读载荷。
- **stdin 契约与 matcher 适用范围** ⇒ **V6**（在 configuration reference 页）。

## Cline —— 有 lifecycle hooks 入口，细节本轮没核完

出处：`docs.cline.bot/cli/cli-reference`（2026-09-23 抓取）。正文确认的只有：CLI 的
`--hooks-dir <path>`、环境变量 `CLINE_HOOKS_DIR`（默认 `~/.cline/hooks`）、项目级
`.cline/hooks/` 的 "Lifecycle hooks"。**事件名、stdin/stdout 契约、`--yolo` 下是否禁用**
都在别的页（后台调研指向 `cline/sdk/examples/hooks/README.md`）⇒ **V7**，本轮不给接入片段。

## Roo Code —— 唯一一条真正的「无文档证据」

我自己抓了 `docs.roocode.com/sitemap.xml`：**510 条 URL，全文出现 "hook" 0 次**。
（后台调研另记：官方文档仓库 grep 只命中 webhook / React hook / git pre-push；
GitHub issue #10834 标题提到过 `PreToolUse`/`PostToolUse` hooks，属历史痕迹、无现行文档。）
⇒ 结论是「按现行官方文档无此能力」，这一条可以写进实现的不覆盖面。

## DimAgent / WorkBuddy

本机有 `~/.dimcode`、`~/.workbuddy`，两者都**没有** hook/notify 配置面（`find` 无命中）。
它们是自有工具，加不加自报字段是**产品决策**，不走第三方核实这条路径；需要时另开票与作者协商。

## 对 P0 的直接影响

1. **桥接脚本按 Claude Code 的 stdin JSON 形状写，Codex 与 Qoder 复用同一份**——三家事件名
   同构（PascalCase），这是本轮最省的一笔：原 spec 以为要各订一套。
1b. **Qoder 走 http hook，不写脚本**。字段形状按正文逐项核过：
   `url` 是**唯一必填**（表格原文 `| url | Yes | URL that receives the POST |`
   ⇒ **方法固定 POST，正文里没有 `method` 这个参数**），`headers` 与 `timeout` 可选；
   `allowedEnvVars` 与 `type`/`url` **同层**、值是环境变量名数组、可省略，
   而省略的行为原文明确写着 **"all are allowed if omitted"**。

   ```json
   {"type": "http", "url": "http://127.0.0.1:41999/session",
    "headers": {"X-AgentIsland-Token": "${AGENTISLAND_TOKEN}"},
    "allowedEnvVars": ["AGENTISLAND_TOKEN"], "timeout": 10}
   ```

   ⇒ **白名单必须显式写**：默认「全允许」意味着 `headers` 里任何 `${VAR}` 都能把任意环境
   变量插值出去，而这条配置的作用恰恰是「往一个端口送令牌」。少写这一个字段不是少一层
   保险，是把令牌面的最小权限直接关掉。
   反过来，**不在白名单里的变量会怎样**（丢 header / 置空 / 整条拒绝）⇒ **V1**，
   别按「应该会拒绝」来设计。
   ⇒ §3 的 `/session` 必须能直接吃 hook 形状的事件 JSON（字段名与 stdin 那套一致），
   否则「配置即接入」这条最省的路要用不上。
1c. **Cursor 单独一张事件名映射表**（camelCase：`sessionStart` / `stop` / `preToolUse`…），
   别指望复用 PascalCase 那份。
1d. **Trae 可以蹭 Claude Code 的配置格式**（官方正文原句），所以 P3 铺开时它不是新增工作量，
   但它的 `Notification` 一个事件同时覆盖「等确认」与「任务完成」⇒ 定 `state` 要读载荷，
   不能只看事件名。
1e. **Roo Code 进不覆盖面**：510 条文档 URL 里 "hook" 出现 0 次，本轮唯一一条有依据的「无此能力」。
2. **Codex 的 `notify` 不进 P0**：它是 legacy、载荷走 argv、stdin 被置空，与新一代不同形。
   真要在托管环境之外兜底，P3 再单独适配。
3. **`session.idle` 改变 OpenCode 的接入方式**：不是「装一个 hook 命令」而是「装一个 plugin」，
   工作项与 03 号契约测试的夹具都要按这个分。
4. **托管环境要有「自报可能一条都不到」的预期**：Codex 的 `allow_managed_hooks_only` 会让
   用户/项目/会话层 hooks 全被忽略 ⇒ 岛的推断链路不能因为「配了自报」而变弱（判断 ②）。
5. **所有未完成项集中在文末的「待核实清单」**（V1–V7），带状态与复核方式。
   正文里不再出现那个笼统的说法——三种不同的「还不知道」各有各的名字与出路。

## 待核实清单

上一版的三条未完成项只以散文散在正文里，没编号、没状态、没复核方式——结果是「Cline 那条
到底核完没有」必须通读全文才能判断，而重复劳动已经发生过一次（v0.0.125 把有正文的四家
误判成排除）。所以这里给每条一个编号，并把那个笼统的说法拆成三种状态：
<!-- 状态词表 -->
`未找到出处`（搜过、没有）· `待逐字复核`（有出处线索、本轮没打开原文）· `待真机实测`
（文档说了但没逐事件/逐入口验证，或行为只能实测）。
<!-- /状态词表 -->
这个块是断言唯一放行的正文侧状态词出口：**靠声明放行，不靠小节位置**——定义搬到哪儿都合法，
标记丢了会红。

| 编号 | 待核实 | 状态 | 复核方式 |
| :--- | :--- | :--- | :--- |
| **V1** | Qoder http hook：`${VAR}` 不在 `allowedEnvVars` 白名单里时的行为（丢 header / 置空 / 整条拒绝） | 未找到出处（正文只写了「省略即全允许」） | 配一条 http hook 指向本机回显端口，比对 header 实际到达值 |
| **V2** | Claude Code 在 `claude -p` 下**逐事件**是否触发 | 待真机实测（文档摘要称会执行，未逐条列） | 配 `Notification` + `Stop` 两条 hook 写文件，跑 `claude -p` 数落盘 |
| **V3** | Codex 在 `codex exec` 下 hooks 是否触发 | 未找到出处 | 同 V2 的办法，跑 `codex exec "<prompt>"` |
| **V4** | Qoder headless / 脚本模式（`docs.qoder.com/cli/run-in-scripts`）下是否触发 | 未找到出处 | 真机跑一次无头任务 |
| **V5a** | Cursor `agent -p`（Headless/CI）下是否触发 | 未找到出处（该页正文未出现 hooks） | 抓正文 + 真机各一次 |
| **V5b** | Cursor cloud agents 是否读 `~/.cursor/hooks.json` | 待逐字复核（现为二手转述） | 抓 cursor cloud agents 页原文，确认「不可用」那句是否存在 |
| **V6** | Trae 的 stdin 契约与 matcher 适用范围 | 待逐字复核（在 hook-configuration-reference 页） | 抓 `docs.trae.ai/ide/hook-configuration-reference` |
| **V7** | Cline 的事件名、stdin/stdout 契约、`--yolo` 是否禁用 hooks | 待逐字复核（在别的页与仓库 README） | 抓 `docs.cline.bot` hooks 相关页 + `cline/sdk/examples/hooks/README.md` |

**归属**：V1–V4 在 02 号票（`/session` 协议）打通当天一起测——那时正好有真实的接收端；
V5–V7 属于 P3 铺开范围，不阻断 P0。
<!-- 禁词说明 -->
**清单是状态的唯一出口**：正文只写「问哪件事 + `⇒ **V#**`」，证据与状态一律在表里。
上一版删掉了 6 处括号里的状态词，却把同一个判断留在括号外（「本轮没逐字复核」「未找到正文依据」），
而断言只数那三个词的字面量——它绿着，第二出口还在。**改写不等于拆掉**。
本块是断言唯一放行的「引用禁词」出口：上面是在**复述规则**，不是给某条待核事项定性。
判定类说法（对「我们核没核」下结论）在本文其余任何位置出现都会红——禁词的作用域是**整篇**，
不按行也不按段；对别家正文的陈述（「该页未列」「文档未提」）只在清单小节内禁，厂商那一节里它们是证据。
<!-- /禁词说明 -->
**收口动作**：核完一条就删掉那一行，并把结论与日期写进对应的票与 CHANGELOG。表里三个状态
都是「未完成」态，没有「已核实」可填，所以删行是唯一合法收口——别往状态格里自造第四种词。

## 复现这些结论的命令

```sh
# Claude Code / Codex 的键形状与本机实样
python3 -c "import json,pathlib;print(json.dumps(json.loads(pathlib.Path.home().joinpath('.claude/settings.json').read_text())['hooks'],indent=1)[:400])"
head -3 ~/.codex/config.toml && ls -l ~/.codex/hooks.json
# Qoder / Cursor / Trae 的正文（要人工看，无本地实样）
#   https://docs.qoder.com/cli/hooks      https://cursor.com/docs/hooks
#   https://docs.trae.ai/ide/automate-actions-with-hooks
# Roo Code「无文档证据」的自证：抓 sitemap 数 hook 出现次数（本轮结果：510 条 URL / 0 次）
curl -sL https://docs.roocode.com/sitemap.xml | grep -o "<loc>" | wc -l
curl -sL https://docs.roocode.com/sitemap.xml | grep -oi hook | wc -l
# OpenCode 的接口与事件名（键名原文，不含任何值）
sed -n '173,215p' ~/.config/opencode/node_modules/@opencode-ai/plugin/dist/index.d.ts
grep -ohE '"session\\.[a-z.]+"' ~/.config/opencode/node_modules/@opencode-ai/sdk/dist/gen/types.gen.d.ts | sort -u
```
