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
> 两者都没有的才写「未证实」，并且不许进实现。

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
- **无头模式**：文档摘要称 `claude -p` 下 hooks 会执行，但**逐事件是否都触发没有逐条出处**。
  实现时按「未证实」处理，P0 打通当天用真机测一遍再定。
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
- **无头模式**：`codex exec` 下 hooks 是否触发**未找到正文依据**，按未证实处理。

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
- **无头模式**：文档未提 headless 下是否触发 ⇒ 未证实。

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
  不可用；本轮抓取的正文只确认到「cloud agents 跑仓库里的 command hooks」那一句 ⇒ 后半句记为二手。
- **无头模式**：`agent -p`（Headless/CI 页）正文未出现 hooks ⇒ 未证实。

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
- 该页未列 stdin 契约与 matcher 适用范围（另有 configuration reference 页）⇒ 本轮未逐字核实。

## Cline —— 有 lifecycle hooks 入口，细节本轮没核完

出处：`docs.cline.bot/cli/cli-reference`（2026-09-23 抓取）。正文确认的只有：CLI 的
`--hooks-dir <path>`、环境变量 `CLINE_HOOKS_DIR`（默认 `~/.cline/hooks`）、项目级
`.cline/hooks/` 的 "Lifecycle hooks"。**事件名、stdin/stdout 契约、`--yolo` 下是否禁用**
都在别的页（后台调研指向 `cline/sdk/examples/hooks/README.md`），本轮没逐字复核 ⇒
记为「有面、细节待核」，不给接入片段。

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
1b. **Qoder 走 http hook，不写脚本**：`{"type":"http","url":"http://127.0.0.1:41999/session",
   "headers":{"X-AgentIsland-Token":"${AGENTISLAND_TOKEN}"}}` 一条配置即接入；
   `allowedEnvVars` 白名单是它自己文档里的约束，实现侧要按它给的方式取令牌。
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
5. **仍未证实的两件事**（P0 当天实测，不许当已知）：Claude Code 在 `claude -p` 下逐事件是否
   触发；Codex 在 `codex exec` 下 hooks 是否触发。

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
