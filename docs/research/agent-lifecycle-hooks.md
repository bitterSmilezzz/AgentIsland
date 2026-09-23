# 各家 Agent 能不能自己上报生命周期（hook / notify / plugin 面核实）

> 目的：在写自报通道（`.scratch/agent-selfreport/spec.md` §3）之前，把「哪一家有**进程级确定
> 触发**的生命周期载体」测清楚。本仓在同类问题上吃过亏（`docs/adr/0006`：没核实到正文就预置
> 字段名，症状是「界面显示已送达、其实没送」），所以本文每条结论后面只允许两种东西：
> **可引用的正文原句**，或**本机配置文件实样**。两者都没有的，写成排除结论，不写成猜测。
>
> 核实日期：2026-09-23。方法：官方文档抓取 + 本机 `~/.claude`、`~/.codex`、
> `~/.config/opencode` 实样 + 已安装 SDK 的类型定义。文中不出现任何令牌值与个人路径。

## 一句话结论

**三家可用，且比 spec 原先假设的更好**：Claude Code 与 Codex 共用**同一套 hook 事件词汇**
（`SessionStart` / `Notification` / `Stop` / `PermissionRequest` / …），一份桥接脚本两家通吃；
Codex 的 `notify` 已经是 **legacy**（载荷走 argv、stdin 被置空），按它设计会订错接口；
OpenCode 不给 hooks 而给 plugin 的**事件总线**，其中有 `session.idle` 与 `session.error`——
这是唯一一处「终态不依赖模型自觉」的载体，正好满足 spec 判断 ①。

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

## 本轮排除的几家（以及为什么是「排除」而不是「没有」）

排除的理由不是「它们没有这个能力」，而是**拿不到可引用的正文、也没有本机实样**。
按 `docs/adr/0006` 的教训，没有出处就不许往实现里写字段名，所以本轮不给这几家写接入片段：

- **Qoder**：本机 `~/.qoder/settings.json` 只有 `enabledPlugins` 一个键，没有 hooks 配置面。
  本会话的运行时确实提到 hooks（能拦工具调用、有 `<user-prompt-submit-hook>` 这类事件），
  但那是**运行环境自述、不是公开文档正文**，字段名与配置文件落点无从引用 ⇒ 排除。
  要接的话，向 Qoder 官方文档拿到键名再开票。
- **Cursor / Trae / Cline / Roo**：本机 `~/.cursor/` 只有 `skills/`，没有 hook/notify 配置；
  其余三家本机未安装 ⇒ 无实样。官方文档本轮未取到可引用正文 ⇒ 排除，P3 之前不动。
- **DimAgent / WorkBuddy**：本机有 `~/.dimcode`、`~/.workbuddy`，两者都**没有** hook/notify
  配置面（`find` 无命中）。它们是自有工具，加不加自报字段是**产品决策**，不走第三方核实
  这条路径；需要时另开票与作者协商。

## 对 P0 的直接影响

1. **桥接脚本按 Claude Code 的 stdin JSON 形状写，Codex 复用同一份**——两家事件名同构，
   这是本轮最省的一笔：原 spec 以为要各订一套。
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
# OpenCode 的接口与事件名（键名原文，不含任何值）
sed -n '173,215p' ~/.config/opencode/node_modules/@opencode-ai/plugin/dist/index.d.ts
grep -ohE '"session\\.[a-z.]+"' ~/.config/opencode/node_modules/@opencode-ai/sdk/dist/gen/types.gen.d.ts | sort -u
```
