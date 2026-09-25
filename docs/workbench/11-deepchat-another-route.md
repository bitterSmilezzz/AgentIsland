# DeepChat：本地优先的 AI agent 桌面客户端（6343 star，另一条路线）

> 目的：我们已确认走「监控 + 管理」路线，而 DeepChat 走的是完全不同的另一条——**当 agent 的前端**。
> 把它摸清不是为了抄功能，是为了**把两条路线的边界划清楚**，避免方案写着写着滑过去。
> [ThinkInAIXYZ/deepchat](https://github.com/ThinkInAIXYZ/deepchat)，GitHub API（2026-09-26 抓取）：
> 6343 star / 740 fork / Apache-2.0 / TypeScript / 创建于 2025-02-14 / 最近推送 2026-09-24 /
> 6 open issues。homepage deepchat.thinkinai.xyz。
> 本文只依据 README 全文（16264 字符）与 GitHub API，**未读源码**——见文末。

## 一句话结论

**它把自己定义为「local-first AI agent desktop client」：跑 agent 的前端，多窗口多标签、MCP、Skills、
ACP agent、还能从 Telegram/飞书/QQBot/Discord 远程控制会话。**
与我们最相关的一条：**它的 remote control 已经有 `/pending` 命令**（在消息应用里回答待确认的
权限请求）——这正是我们 `attention` 态远程推送的**另一种实现**，而且做得更早更完整。

## 它是什么（README 原文口径）

> DeepChat is a powerful open-source, local-first AI agent desktop client that brings together models,
> tools, Skills, agent runtimes, Tape, and long-running sessions in one desktop app. Whether you're
> using cloud APIs like OpenAI, Gemini, Anthropic, or locally deployed Ollama models, DeepChat
> delivers a smooth user experience.

> DeepChat's sessions and agent processes follow the **Tape.systems philosophy**: keep the process,
> so context, tool calls, requests, and results stay recoverable, traceable, and inspectable.

四个自述优势（原文标题）：Local-First Agent Desktop Client / Tape.systems Philosophy /
Skills That Travel / Native ACP Integration。

### 能力清单（README 自述）

| 域 | 内容 |
|---|---|
| 模型接入 | DeepSeek / OpenAI / Moonshot(Kimi) / Grok / Gemini / Anthropic…**任何 OpenAI/Gemini/Anthropic API 格式的 provider**；本地集成 Ollama（下载/部署/运行管理） |
| 会话 | 多窗口 + 多标签，并行多会话，「像用浏览器一样用大模型」；消息 retry 多变体、会话可 fork |
| **Tape & Trace** | Session Tape 记录结构化工作史；Trace 预览展示请求序列、provider/model 元数据、Tape view manifests、包含条目与 **token budgets** |
| **Skills** | 从文件夹/ZIP/URL 安装；**按会话启用**；与 Claude Code / Codex / Cursor / Windsurf / GitHub Copilot **互导互出** |
| **ACP**（Agent Client Protocol） | 把 ACP-compatible agent 当"模型"选；ACP workspace UI 展示结构化计划、工具调用、终端输出 |
| MCP | Resources / Prompts / Tools、多种 transport、inMemory services、一键安装 |
| **Remote control** | 从 Telegram / 飞书(Lark) / QQBot / Discord / 微信 iLink 控制会话 |
| 搜索 | BoSearch、Brave Search API；还能模拟人浏览 Google/Bing/百度/搜狗，让 LLM 像人一样读搜索引擎 |
| 渲染 | Markdown（CodeMirror）、Artifacts、图片、Mermaid、GPT-4o/Gemini/Grok 文生图 |

## 两条与我们直接相关的发现

### 1. 它的 remote control 已经有 `/pending`——我们 `attention` 的远程对应物

README 原文（`/tmp/uibatch/deepchat/README.md:296` 附近）：

> Supported channels include Telegram, Feishu/Lark, QQBot, Discord, and WeChat iLink. Remote endpoints
> can bind to one DeepChat session, then create new sessions, list and switch recent sessions,
> stop generation, open the current session on desktop, **answer pending questions or permission
> prompts**, switch models, and check runtime status.
>
> Common commands include `/start`, `/help`, `/pair`, `/new`, `/sessions`, `/use`, `/stop`,
> `/open`, **`/pending`**, `/model`, `/status`.

**对照我们**：README「远程通知（可选，默认关闭）」今天的形态是**外发告警**——把 `attention`
推到你手机/邮箱。而 DeepChat 是**双向控制**——在消息应用里就能回答那个待确认的请求。

差别是质变：通知是"告诉你该回去了"，控制是"你不用回去"。

对我们的意义分两层：
- **短期不变**：我们的边界是「只发通知、不在第三方平台上执行操作」——这条已在 README 写死
  （「深链与 /notify 来的事件一律不走这条路」），且**不持有任何可执行凭据**。改成双向控制
  等于在消息平台上开一个能替用户批准命令的面，安全面完全不同级。
- **但 `/pending` 这个命令的存在值得记**：它证明"待确认队列"是一个值得成为一等概念的交互件，
  而不只是我们内部的一个状态枚举。我们岛内的 attention 呈现可以更接近"一个队列"而不是"一个灯"。

另注意 `/pair`——配对命令。它说明 remote 接入需要一次显式绑定，不是打开就能连。
这与我们 `POST /session` 带令牌自报的思路同向（都要先建立可信关系）。

### 2. Skills That Travel：与我们的 `npx skills` 是同一件事，但它更细

README 原文：

> Install Skills from folders, ZIP files, or URLs
> Enable Skills **per conversation** so DeepChat can load task-specific instructions, references,
> and optional scripts
> Import and export Skills **with Claude Code, Codex, Cursor, Windsurf, GitHub Copilot**, and other
> compatible tools

对照我们：本仓用 `npx skills` 管理 agent skills，单份源在 `.agents/skills/<name>/`，
`.claude/skills/<name>` 是软链，`skills-lock.json` 记 hash（见 [AGENTS.md](../../AGENTS.md)）。
装的是 `libraries-dev` 一个，**全局生效**。

DeepChat 比我们多的两件事：① **按会话启用**（细粒度）；② **跨工具互导**（可移植）。
我们少的第三件事：它把 Skill 当"可安装、可分发的单元"，我们只装了一个别人的 skill。

这条不必立刻做，但它给了我们一个改进方向：**如果将来要装更多 skill，按会话/按项目启用比全局装更安全**
——一个第三方 skill 装着不启用，比装着随时可能被触发要好。这与我们在会话日志解析上
「把外部内容当数据不当指令」是同一条防线。

## 两条路线的边界（本文真正的用途）

| | DeepChat | AgentIsland |
| :--- | :--- | :--- |
| 角色 | **当 agent 的前端**：启动它、喂它、显示它的完整会话 | **管 agent**：看它状态、统计它花费、管它的配置与待办 |
| 与 agent 的关系 | 需要**协议级接入**（跑 agent 进程、解析流式输出、转发输入） | 只需要**读**（会话尾、进程表、配置文件），**不启动也不转发** |
| 要不要碰网络 | 要（模型 API、搜索、远程通道） | **不碰网络、不转发流量、不持有密钥**（README 已写死） |
| 多 agent 的意义 | 你能同时开多个会话窗口 | 你能同时**看见**多个 agent 在干什么 |
| 代表能力 | Tape/Trace、ACP、MCP、远程控制 | 五态状态机、token 统计、Provider 档位、ToDos |

**这个表是用来防方案写偏的**：我们的侧边栏形态与它的多标签界面看起来像（都是竖向容器 + 列表），
但底层关系完全不同。它每一个功能都要问"agent 协议支持吗"，我们每一个功能只问"磁盘上读得到吗"。
**成本差一个数量级，能给出的东西也差一个数量级。**

一条具体的警戒：**不要因为 DeepChat 有 Tape & Trace 就给自己加"可回放的诊断日志"**。
它的 Tape 是**它自己启动的 agent** 的结构化记录，天然完整；我们的会话尾是**读别人写的日志**，
格式会变、字段会缺、还可能读不到（README 已知限制载明「未知或改版后的日志格式会安全降级」）。
同样的功能名，我们的版本只能是降级的，且**必须说出降级了多少**。

## 可迁移的决策规则

1. **"待确认"应该被当成一个队列，不是一个灯。** DeepChat 的 `/pending` 是一条命令、一个可枚举的集合；
   我们五态里的 `attention` 若只显示"有几个在等"而不给"分别是哪几个、等了多久、怎么处理"，
   信息密度就低于一个 6343 star 的产品的最低标准。
2. **按会话/按项目启用第三方 skill，比全局装更安全。** 未启用 = 不可能被触发。
3. ** remote 接入要有显式配对步骤**（它的 `/pair`）。可信关系先建立，再谈控制。
4. **同样的功能名，降级的版本必须说出降级多少。** 见上。

## 应用建议清单（与实现解耦，尚未排期）

1. **`attention` 从"状态点"升级为"可枚举队列"**：侧边栏里每个待确认请求占一行（agent 名 + 等了多久
   + 请求摘要 + 一键处理）。**前提是先确认能拿到什么**——今天只从会话文件提取 `attention` /
   `completed` 两类强语义（README 判定优先级有载），中间内容拿不到就不能假装有。
2. **若要装第二个以上 skill，做按项目启用**：`skills-lock.json` 已记录 hash，加一列启用范围即可。
3. **远程外发保持单向**，不引入双向控制。这条与现有安全边界一致，**不建议为对齐 DeepChat 而改**。

## 没能核实的

- **源码一行没读**。本文只依据 README 与 GitHub API。它的 harness/协议层、测试规模、
  凭据处理、真实可维护性**全部未核实**。（MonoCode 那份调研会覆盖其中一部分对照）
- **Tape.systems 是什么**：README 反复引用这个 Philosophy，但**没有链接也没有展开说明**。
  我们没有获取到它的定义，不知道它是一套格式、一个库还是一种主张
- **「ACP」协议细节**未读（README 指向 agentclientprotocol.com，未抓取）
- DeepChat 的 **reduce-motion 处理**未核实（它是 Electron/TS 项目，不是 Tauri；README 无提及）
- 6343 star / 6 open issues 是 2026-09-26 的快照。6 个 open issues 对一个 6343 star 项目异常少，
  **可能有大量关闭或关闭策略不同**，未核实
- 赞助商出现在 README 显著位置（APIMart / OpenModel / PackyCode），**未核实这是否影响产品决策**
- 它是**本地优先但不等于本地-only**：README 明说支持云 API 与远程控制，所以「local-first」
  指数据主权不在服务端，不是断网可用。这一点 README 自己讲清了，但容易误读，记在此

## 取证命令

```sh
# 仓库元数据
curl -sSL https://api.github.com/repos/ThinkInAIXYZ/deepchat | python3 -c "
import sys,json; d=json.load(sys.stdin)
print({k:d.get(k) for k in ['stargazers_count','forks_count','language','created_at','pushed_at','open_issues_count']})"

# README 全文（本文所有引文出处）
curl -sSL https://raw.githubusercontent.com/ThinkInAIXYZ/deepchat/main/README.md

# 抓 remote control 那一节
curl -sSL https://raw.githubusercontent.com/ThinkInAIXYZ/deepchat/main/README.md | grep -n -A6 'Remote Control'

# 若本机直连不通，全部走系统代理
scutil --proxy      # 127.0.0.1:10808
curl -x http://127.0.0.1:10808 -sSL https://api.github.com/repos/ThinkInAIXYZ/deepchat
```
