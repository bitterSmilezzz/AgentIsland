# 更新日志 (CHANGELOG)

所有关于 AgentIsland 的重要版本演进与功能更新均记录在此。

历史发布按时间统一编号为 0.0.1–0.0.53；对应关系见 [版本映射](docs/version-mapping.md)。

## [0.0.150] - 2026-09-26

### 收 Vorssaint 调研：可插拔架构与权限 UX，并核实「只做两家」的独立佐证

[vorssaint/vorssaint-utils](https://github.com/vorssaint/vorssaint-utils)（Vorssaint，
GitHub API 2026-09-26：**21355 star / 799 fork / Swift / GPL-3.0 / 548 open issues**）——
"Free and open-source macOS menu bar toolkit"，副标题
"One menu bar icon doing the job of a dozen paid Mac apps."
新增 [`18-vorssaint-pluggable-architecture-audit.md`](docs/workbench/18-vorssaint-pluggable-architecture-audit.md)（603 行）。

**A｜可插拔架构（本文核心）**：73 个 feature、13 项系统权限、4800+ 用户可见开关，
全部由一个 `AppFeature` 枚举加一张 `[AppFeature: () -> Void]` 闭包表串起来。
三层模型（`Sources/Vorssaint/Core/FeatureCatalog.swift:6-14` 原话）：
**availability（安装层）⊃ enable（行为层）⊃ 资源层**。关键点：

- **装卸是运行时开关，不是编译期**：写 availability 键后立刻跑闭包表；
  服务的 `syncWithPreferences()` 一律「wanted && 已安装 && 有权限 && 前台 → start，else stop」，
  未安装 feature 的**单例连构造都不发生**
- **设置全留 UserDefaults，卸载从不删键**（这是"reinstalling restores its settings"的实现）。
  靠 `register(defaults:)` 而不是 `set(...)` 区分「全新安装」与「用户存过一个 false」，
  所以**卸了再装不会把人关掉的开关又打开**。对照我们的 `SettingsStore`：
  形态同构，**缺的只是「这个模块装没装」这一层**
- 一道穷尽 switch 做模块→配置面索引（**73 feature → 29 page**），让新增模块逼一次显式选择
- 它自己承认一条诚实边界：卸载当次只停服务，**真正卸载需重启**，于是有横幅 + 原地重启

**B｜权限 UX（可直接照抄）**：「不再需要」= `activeFeatures(using:)` **纯函数求值为空**，
不是引用计数也不是静态对比。函数用注入读取器（可单测），同时喂两个 UI：
行上的「使用者：A、B、C」与 `status == .granted && isEmpty` 时的
「已授予但没有已开启的功能需要它，可在系统设置撤销」。另有按需三段轮询
（有可见 UI 或待授权 2.5s；已授守撤销 60s；否则无定时器）与一条硬规则：
**撤销 Accessibility 前必须先停 event tap，否则整机输入冻结**。

**C｜本次最硬的发现（我(agent)亲自读了源码）**：它有一整套 `Sources/Vorssaint/Services/AgentUsage/`
（12 个文件），而 **`AgentProvider` 只有 `claude` 与 `codex` 两个 case**
（`Sources/Vorssaint/Services/AgentUsage/AgentUsageModels.swift:8-9`）。
**一个 21355 star / 255139 行的成熟产品，agent 用量这块也只做两家**
——这与我们「Phase 2 只做 Codex 一家」是同一判断的**独立佐证**，比我们自己的推理有力。

Claude 侧它走 `AgentClaudeAppUsage.swift`：只读
`~/Library/Application Support/Claude/plan-usage-history.json`（Claude Desktop 自己每 5–15 分钟
记一次的百分比采样），**零凭据、零网络、零子进程、零写入**；
`version == 1 || version == 2` 之外的格式整体返回 nil（原话 "left out rather than guessed"）；
四个窗口键 `fh`(session 300min) / `sd`(weekly) / `so`(weekly Opus) / `sn`(weekly Sonnet)，
值钳 0–100，新鲜度 30 分钟。**它比 codenotch 那套三级回退更干净**（后者要 keychain + 打端点），
且它还能**从历史样本反推 session 重置时刻**（"a session renews five hours after the hour its
use began, which the history brackets"）。

**D｜GPL 与一条必须先拍板的事**：
`LICENSE` 是 GPL-3.0（35149 B），每个文件头带
`// SPDX-License-Identifier: GPL-3.0-or-later`；`TRADEMARKS.md` 把源码版权与
商标/图标/bundle id/签名身份分开覆盖。**结论：`Sources/` 一行都不能进我们的仓**，
可抄的只有算法思想、数据形状、命名与流程，且要在我们自己的文件里用自己的话重写。

**顺带发现一条基础事实（比 GPL 更该先解决）：本仓当前没有 LICENSE 文件**
（`find . -maxdepth 2 -iname "LICENSE*"` 零命中），README / CONTEXT 里也**从未声明过本项目
采用什么 license**（README 那两处 "MIT" 命中是 `install-git-hooks` 子串误配，不是 license 声明）。
我(agent)在此前几轮的对话里多次称本项目为「MIT」——**那是我没有依据的说法，自此纠正**。
这条不解决，"能不能抄 GPL" 甚至"本项目允许别人怎么用" 都悬空，已列为待拍板。

**没核实的**：`Sources/Vorssaint` 745 个 Swift 文件（255139 行）只细读了 AgentUsage 与
FeatureCatalog 相关；它的 13 项权限只核对了设计没核对每项实现；`Tests/NotchAgentTests.swift`
未读；它的 Dynamic Island / Notch 那 69 个文件的子系统未看（那是它的主形态，不是我们的）。
Swift 侧无改动，测试基数仍为 548 条。

## [0.0.149] - 2026-09-26

### 收 codenotch 调研：最直接的形态竞品，以及一条我们自己都没意识到的口径矛盾

[vinzdg/codenotch](https://github.com/vinzdg/codenotch)（GitHub API 2026-09-26：
**2493 star / 383 fork / Swift / MIT / 创建于 2026-09-05（三周新仓库）/ 53 open issues**），
自述 "A macOS app that pins usage limits from Claude Code, Cursor, Codex, and Antigravity
to a screen edge." 新增 [`17-codenotch-direct-competitor-audit.md`](docs/workbench/17-codenotch-direct-competitor-audit.md)（469 行）。

**这是前几篇里唯一正面对上的竞品**：同样 Swift、同样 macOS、同样把"屏幕边缘小条"做成常驻 UI，
做的功能（usage limits、working/done/waiting 三态）与我们的灵动岛高度重叠。

**A｜形态真相**：不是"小条"，是 **26×210pt 的黑 pill**（`Sources/Notch/NotchLayout.swift:46-48`），
但 hover 卡有 600 个设计 px，比我们的 372 卡更宽——"小"只是静止态。
**17 家 provider**（10 纯官方 + 3 混合 + 2 derived + 2 本地运行时），其中 12 家"借机器上已有工具的登录态"。
手机端 wire 协议完整（v2→v3 自我否证），但 `Sources/PhoneLink/PhoneLinkServer.swift:13` 的
`isAvailable = false`——**主仓当前不监听**。

**B｜三个正面判断**：

1. **它的 notch 更聚焦：是，但代价是只答一个问题。** 静止态只答"还剩多少额度"，
   并**刻意让 working 取白色**，免得被误读成额度标尺（`Sources/Sessions/ActivitySummary.swift:62-70`）。
   我们答的是"agent 在干什么"。真正该问的不是谁更聚焦，而是：
   **它对"不许出现编出来的数字"有一整套保证，我们的"不可信"不在读数在判定**——
   同一 agent 两个形态状态不一致，正是 replan 风险 4。
2. **形态哲学不是大小，是三种密度**：静态一瞥 / 悬停展开 / 菜单栏常驻。
   它后来自己也加了菜单栏项——**这正面支持我们的 `shell_mode` 双形态**。
   差别在它的 notch 永不受键盘焦点。最值得抄的是 `Sources/Notch/NotchPlacement.swift` 的
   **一维 stack space + 唯一映射点**（所有布局在两个坐标系里算，只有一处知道"哪条边"），
   以及 `Sources/Notch/NotchPanel.swift` 用 `sendEvent` 接点击（可直接对照我们的
   `Sources/AgentIsland/IslandPanelInteraction.swift`）。
3. **它做对了我们没做的**：卖的是**一个恒等式**而不是一个品类集——
   README 原话 "Claude's ring shows the same current session window Claude Code's own /usage
   leads with, so the two **never disagree**"，且它真的为这句话建了**三级回退 + 一个 snapshot
   唯一出口**（`ClaudeOAuthProvider.swift:263-278` 注释原话："窗口顺序或 headline 在 endpoint 上改了，
   不能偷偷地和 CLI 的或 Desktop 的不一样"）。
   另有 `Design.scale = 44/117` 一个锚点定全局、**每 commit 出可装产物** + 18 天 16 个正式 release。

**「never disagree」的三级回退**（可直接对照我们的配额口径）：
① Claude Desktop 的 HTTP 缓存文件（零子进程、零 keychain、零网络；快照 > 30 分钟即弃用）；
② 起 `claude /usage`（一个子进程，5 分钟缓存一次，非零退出读作"它没登录"）；
③ keychain OAuth token 打 `api.anthropic.com/api/oauth/usage`。
`ClaudeDesktopUsageCache.swift:14-24` 的注释是模范文档，原话：
"No token, no cookie, no keychain, no request to Anthropic, no subprocess, and no write of any kind."
它还解决了我们一个潜在问题——**数字有多旧**（`capturedAt` + `isFresh(within:)`），
与我们的「读不到 ≠ 闲着」同类。

**C｜跟进 3 件 / 不跟 2 件**：跟进 `ProcessLiveness` 的 pid 起时比对（直击"进程崩了会话文件还在说 busy"）、
一维栈空间布局（Phase 1 顺手做，侧边栏贴左/右复用同一套数学）、
"无会话即隐藏 + 失败态必须可命名"（后者提醒我们：侧边栏若出现"未知/异常"这种笼统文案，
应至少拆成未安装/未登录/已登出被清/无配额可计/被限流五类）。
不跟：17 家 provider 覆盖（用户不用 Claude Code，且 CC Switch 已占据 provider 面）、
手机端/局域网 HTTP 面（我们的远程外发是另一条路）。

**D｜一条我们本来没意识到的口径矛盾（本文最有价值的副产品）**：
任务书要求显式提出"官方直读配额是否冲突"，但拆开三级后发现**冲突范围小得多**
（第一级读文件不碰网络，第二级半个冲突，第三级正面冲突）。
顺手逐条核实才发现：**本仓「不持有密钥」这条口径今天就已经有两处自相矛盾**——
`CONTEXT.md:167` 明写 SMTP 授权码/ntfi token/webhook key **本来就在钥匙串里**且远程外发本身
**主动出网**，而 `CONTEXT.md:196` 又写「不碰网络、不转发流量、不持有密钥」；
加上 Phase 2 档位文件会出现 key。所以问题不是"要不要为接配额破例"，
而是**「凭据在本机的边界到底画在哪」**——这条边界今天已经模糊，接配额只是把它推到台前。
三个候选 (a) 只读既有登录态用于展示 / (b) 借机器上已有 CLI 的登录态（只读）/ (c) 自己持钥打端点，
已在文档里列成表，留待拍板。

**没核实的**：`TASKS.md` 未读；12 家 monitor 只细读了 Claude 一家；
`NotchViewModel` 的动效与 reduce-motion 覆盖未看；**我们的 `ProcessMonitor` 是否已做起时比对未逐行读**
（这决定跟进项 1 是新增还是补测试）；codenotch 的 Tests 未读（它有 `ActivitySummaryTests.swift`）。

Swift 侧无改动，测试基数仍为 548 条。

## [0.0.148] - 2026-09-26

### 收 mattpocock/skills 元规范，并补掉 AGENTS.md 一处实缺

[mattpocock/skills](https://github.com/mattpocock/skills)（GitHub API 2026-09-26：
**269,700 star / 22,724 fork / Shell / MIT / 528 open issues**），自述
"Skills for Real Engineers. Straight from my `.agents` directory."
新增 [`16-mattpocock-skills-meta-specs.md`](docs/workbench/16-mattpocock-skills-meta-specs.md)。

它**不是竞品也不是被监控对象，是我们本机已装的 24 个 skill 的上游**——这一条是本轮最实的发现。
读完 README 与 `.agents/` 下三份元规范（`invocation.md` / `writing-docs.md` / `install-block.md`）后，
顺手核实并补掉了 [AGENTS.md](../../AGENTS.md) 的一处文档缺口。

**补掉的缺口**：AGENTS.md 的 Installed skills 一节原先**只写了 `libraries-dev` 一个**，
而本机 `~/.agents/skills/` 里有 **24 个来自 mattpocock/skills 的 skill 在生效**
（grill-me / grilling / grill-with-docs / implement / to-spec / to-tickets / code-review /
domain-modeling / codebase-design / prototype / diagnosing-bugs / research / tdd /
resolving-merge-conflicts / wizard / wayfinder / triage / ask-matt / teach / handoff /
wait-what / to-questionnaire / claude-handoff）。
于是"哪些纪律是本仓定的、哪些是上游定的"分不清。已改写为两类来源并分别说明纪律。

**上游三条元规范，两条对我们有约束力**（来自 `.agents/invocation.md`）：

1. **user-invoked 的 skill 不能被另一个 skill 调用**，连指名工具也不行
   （"nothing but the human can fire it: no other skill can"）。
   → 所以"跟我说一声就跑 X"这类指令，若 X 是 user-invoked，必须写成**对人的指令**
   （"请运行 `/x`"），不能写成对模型的工具调用。我们本机的 `grill-me` / `triage` /
   `to-spec` / `implement` 都属 user-invoked。
2. **skill 之间的依赖要写成"调用 Skill 工具并指名"**（`Call the Skill tool with "grilling"`），
   **不是 `../other-skill/FILE.md` 跨目录相对链接**。
   → 界线已写进 AGENTS.md：**给人看的路径链接可以用相对路径；给 agent 的操作指令必须点名工具。**

另有一条**我们踩过但它写明了**：两条安装路径**互斥**——Claude Code 的 plugin 路
（官方 marketplace，只读托管、自动更新）与 skills.sh 路（可编辑副本、自己 update）
**都装会得两份**。本机走 skills.sh 路，这点原先没记。

**`writing-docs.md` 给了三条可借的文档判据**：

- `## It's working if` 的门槛是**读者不打开 SKILL.md 就能验证**；
  上游点名一种伪信号："byte-identical to template.sh" 是
  "a compliance check on the skill's internals wearing this section's name"
  → 可搬到我们的验收标准：`risk-and-acceptance.md` 的 B 系列里若有"文件内容正确"这类，
  要问一句它是在测实现细节还是测用户能看到的结果
- `## Common questions` 只能收真实观察到的问题，且"the count stays honest to the evidence"，
  不许为对齐丰技能而编——我们案例库的「没能核实的」已是这个路子，它给了更硬的说法
- 文档页**不带安装命令**（站点自己渲染），理由是两份拷贝会漂移——
  与我们「README 不是更新日志」同源：**一件事只说一次**

**一个现成的自检问题**：上游用「模型能不能自己有用的伸手去拿」判 user/model 分界，
用来问我们本机那三个 grilling skill——`grill-me` / `grilling` / `grill-with-docs`
**分工没有一张表说清**。上游用"链上哪一环"说清角色，这条缺口已记进 AGENTS.md 与应用建议。

**没核实的**：三份元规范读全了，但 **24 个 skill 的 SKILL.md 本体一个都没读**
（所以"它们在本仓实际效果如何"未评估——我们用过 grill-me / to-spec / implement，
但从没做过效果评估）；`skills.sh` 的实现未读；269,700 star 对本仓的影响面没量化；
上游 README 说的 ~60,000 newsletter 订阅未核实；Codex 侧 `agents/openai.yaml` 那套元数据
我们本机没有对应物。

Swift 侧无改动，测试基数仍为 548 条。

## [0.0.147] - 2026-09-26

### 收 Omarchy 调研：Linux 侧对应物，并核实本仓两块整块缺失

[omacom/omarchy](https://github.com/omacom/omarchy)（43140 star / Shell / MIT，DHH 的
"beautiful, fun and agentic Linux distribution"）——它不是一个库或竞品，是**一套完整的桌面环境配置**
（Hyprland 加 Quickshell）。readme 只 1KB，实质全在 `manual/` 34 篇里。
新增 [`15-omarchy-agents-panel-and-toggles.md`](docs/workbench/15-omarchy-agents-panel-and-toggles.md)，
读了其中五篇原文（AI / toggles-idle-screensaver / notices / reminders / top-bar）。

**它与我们的关系是"同一件事的 Linux 侧实现"**：它的 agents panel（顶栏一个图标，
点开是套餐、5 小时与每周限额已用百分比、按天按模型的 token）就是我们在 macOS 上做的东西。
四条可直接搬的判据：

1. **静默通知必须进历史，且静默状态要有常驻记号**（manual 原文：
   "A silenced notification is written straight into your notification history, which is exactly
   the record you want when you come back and wonder what you missed"，外加一个划线铃铛
   "remind you why the desktop has gone quiet"）。这一条补的是我们产品语义的缺口：
   远程外发策略分级与岛内分级都在，但"静默期间的事件去哪了"没有对应设计。
2. **indicators 的可见性默认值**：`Inactive indicators are hidden. Hover ... they fade in dimmed,
   so you can click one to turn it on without knowing its hotkey.`
   ——"默认隐藏加 hover 淡入"这个**中间档**是案例库 14 篇没给过的答案，
   比"全常亮"更符合"克制是默认项"。
3. **toggles 是模式，不是设置**：manual 原话 "a lot of what you change day to day isn't really a
   setting. It's a mode you flip on for an hour and off again"。实现也干净：
   **每个 toggle 就是一个 flag 文件**，热键 / 菜单 / 命令三个入口打同一个开关，
   flag 按关态命名，另给 `omarchy-toggle-enabled` 返回退出码给脚本用。
4. **skill 分发**：它把一份 skill 软链到**六家** harness 的 skills 目录
   （Claude Code / Codex / Pi / Antigravity / Hermes 加 generic `~/.agents/skills`），
   比我们 AGENTS.md 里记的（`.agents/` 加 `.claude/` 两条）完整——那张目录映射表可直接补进来。

**顺带核实出本仓两块整块缺失**（不是"补一半"，是从零没有）：

- **没有任何勿扰/静默开关**：`grep -rn -i '勿扰|muteAll|silenceAll' Sources/` 只命中两处
  `AgentCleaner.swift:164` 与 `TopCommand.swift:204` 里"静默漏掉报警"的注释，与通知无关
- **`shell_mode` 一行实现都没有**：`grep -rn 'shell_mode|shellMode' Sources/ app/` 结果为 0，
  只活在 CONTEXT 与 CHANGELOG 里（与 [10 号方案](10-replan-2026-09-26.md) 已记的一致）
- 另：`TokenAnalyticsView.swift:418` 那个 `percent` 是**与上一周期比的增减百分比**，
  不是"配额已用百分之几"——两者回答不同问题，后者今天没有

**一个最值得追的问题单列了**：**它的 agents panel 怎么读到 5 小时与每周限额？**
我们从会话日志反推，它可能有 provider 的直读路径。若真有，我们的口径可能要改。
**未核实**（只读了 manual 五篇，没读 `omarchy` CLI 源码）。

**没核实的**：manual 34 篇只读 5 篇；它的任何 shell 脚本实现未读；"面板秒开""theme 同步到 agent"
是文档说法未验证；4555 open issues 对 43k star 偏高，未判断是否规模常态；
Hyprland/Quickshell 是 Wayland 专有，**与我们 macOS SwiftUI 无技术可复用性**——
本文全部可迁移项都是设计判据，不是代码。

Swift 侧无改动，测试基数仍为 548 条。

## [0.0.146] - 2026-09-26

### 收 Semantica 调研：一份"基本不重叠"的诚实记录，含两条可借纪律

[semantica-agi/semantica](https://github.com/semantica-agi/semantica)（13469 star / Python / MIT）
自述 "Graph-Native Infrastructure for Context and Accountable AI Systems"——企业知识图谱 +
因果推理（Rete / Datalog / SPARQL / RDF / PROV-O / OWL / SHACL / SKOS）+ 合规审计，
目标域 "High-Stakes, Regulated Domains"。
新增 [`14-semantica-knowledge-graph.md`](docs/workbench/14-semantica-knowledge-graph.md)。

**这次的记录以"用不上"为主**，并写清了为什么：它是给企业数据建图谱推事实的基础设施，
我们是读本机会话尾与进程表的监控器；它连 LLM 都声明为可选（"no LLM required for graph
construction, reasoning, or provenance"），我们则**完全不碰 LLM**。两个问题不相通。
一条明确的「不要做」也记了：不要因为它有 Context Graph 就给自己加图谱或决策链模块——
我们的会话尾是别家 agent 写的日志，字段都可能缺，在它上面建图谱等于把不确定性当结构化输入。

**仍借到两条纪律**：

1. **「provenance 是副产品，不是产品」**。README 原话 "Decision provenance and audit trails
   **aren't the product**. They **fall out of that structure for free**"。
   对照本仓：`Models.swift:11-12` 的 `provenance`（`observed` / `inferred`）与 README 的
   「自报说 X，进程表说 Y」双显**已经是这个形态**——可信度不是外加的标签层，
   是同一个状态机的自然产出。可迁移的是那条判断标准：**新加一个能力时问一句，它的可信度信息
   是长在数据结构里，还是我在外面贴的标签？贴标签的那个一定会过期。**
2. **在 README 里把"这一版它就是简单的"写具体**。全文只一处自陈 limitation
   （`README.md:545`，ReteEngine 的 alpha-node 条件匹配），但三点齐全：点名具体是什么简单、
   给出用户侧动作（"接生产合规闸前自己验证输出"）、说清 roadmap 上有没有。
   这与我们 README 已知限制条同路，但**可以更严格**：凡涉及"做前先读代码"的条目，
   也该写清是哪个场景下必须先验证。

**没核实的（已在篇末单列）**：源码一行未读，只依据 README 全文（60438 字符）与 GitHub API。
它那句 "deterministic infrastructure / no LLM required" 是自述，**我们未验证**；
测试规模、CI、实际成熟度一概未看（不同于 MonoCode 那次逐文件核实）；`semantica doctor` 未运行
（本机未 `pip install`）。`workbench/README.md` 文档表补 14 号一行。

Swift 侧无改动，测试基数仍为 548 条。

## [0.0.145] - 2026-09-26

### 修一个从上游源码核出来的实缺：OpenCode 的「正在调用: xxx」从来没显示过

[anomalyco/opencode](https://github.com/anomalyco/opencode)（210,053 star）不是竞品参照，是**我们README 里已适配的被监控对象**。调研它「能被监控到什么」时，从上游源码逐版本核出一处**我们今天代码里的字段名错误**。

**错的与对的**（上游 `packages/schema/src/v1/session.ts:315-322`）：

```ts
export const ToolPart = Schema.Struct({
  type: Schema.Literal("tool"),   // ← 我们写的是 "tool-call"
  callID: Schema.String,
  tool: Schema.String,            // ← 我们读的是 "toolName"
  state: ToolState,               // ← 我们完全没读
})
```

`v1.0.180`（`packages/opencode/src/session/message-v2.ts:274-282`）与 `v1.16.0`
（`packages/core/src/v1/session.ts:306-313`）**同此**——也就是 `tool-call` / `toolName`
**从来不是 opencode 的形状**，那是 DimAgent 侧的形状（`AgentLogStreamer.swift:323` 读的
`toolMeta.toolName` 是对的，两处被混在一起了）。

**后果**：`AgentActionInspector.swift:691` 的 `type == "tool-call"` 永不成立 →
**「正在调用: xxx」这条动作从来没在岛上出现过**；`AgentLogStreamer.swift:445` 同理，
流水的 toolCall 事件全部落空。`text` 与 `reasoning` 两个分支是对的，所以只有这一支坏。

**怎么修的**（不是简单改字符串）：

1. 解析逻辑从 DB 闭包抽成纯函数 `AgentActionInspector.openCodeAction(fromPartJSON:nowMs:timeUpdatedMs:sessionTitle:)`。
   抽出来是为**能测**——DB 闭包里塞断言只能靠真库，而本机没装 opencode。
   这也正是 12 篇 MonoCode 调研给的那条纪律（协议层做成纯函数才可测）。
2. **同时接受新旧两种字段名**（`tool`/`tool-call` × `tool`/`toolName`），
   注释写清兼容旧名是防御上游改名、**不是赌它今天长这样**——上游三版同此。
3. 顺手用上 `state` 四态（`pending`/`running`/`completed`/`error`），它们此前完全没读：
   `running`/`pending` → 「正在调用」；`completed` → 「调用过」；`error` → 「工具失败」。
   **`error` 不再混在完成里**，这是原实现连可能性都没有的一条。
4. Part 判别联合补齐到 12 种（`text`/`subtask`/`reasoning`/`file`/`tool`/`step-start`/
   `step-finish`/`snapshot`/`patch`/`agent`/`retry`/`compaction`），其余 10 种显式落到标题兜底。

**测试**：新增 4 条（548 全绿，v0.0.144 基线 544）。其中**有一条当场抓出我自己写的实现漏洞**——
`tool: "   "`（纯空白）时 `isEmpty` 为 false，会把空白原样拼进文案，岛上出现「正在调用:    」
这种半句话。测试先红，改实现（`trimmingCharacters` 后再判空）后转绿。

**同时落库** [`13-opencode-monitorability-audit.md`](docs/workbench/13-opencode-monitorability-audit.md)（543 行），
四类监控覆盖度判定 **token 全 / 状态对 / 动作半对（本版已修）/ 进程对一半**：

- token 完全命中：`message.data` 的 `role`/`time.completed`/`tokens.{input,output,reasoning}`/`cost`
  字段名与我们的 SQL 逐字一致；另发现 `step-finish` part 也带 `cost` + `tokens`（逐 step，比 message 级更细），我们今天没读
- 进程匹配只覆盖 CLI：opencode 还有一个 **Electron Desktop App（BETA）**，自己 fork utility process 跑 server，
  与 CLI 共用同一份库。档案里有 `bundleIDs: ["ai.opencode.desktop"]`，但 beta/dev 渠道 appId 未列，
  且档案缺 `pathExcludes`（MiMo 档案注释记过这个坑）
- `session.agent` 一列在库里且实时更新，我们没读——因此 **`plan`（只读 agent）与 `build` 分不出来**。
  这条对产品有直接价值：我们本来就能显示「agent 在改文件」与「只在探索」的区别，今天没有

**README 已知限制补一条**：OpenCode 的适配**只对过源码、没对过真库**
（`~/.local/share/opencode/opencode.db` 在本机不存在）。这条比原来的「已适配」三个字重要——
它会让人以为验证过。装上 opencode 跑一个会话是明确未做的验收。

**本轮没做**：不检测 agent 是否在跑时不 kill 进程（既定）；不读 `step-finish` 的逐 step 用量（有得做但
要改 SQL 与索引，单独一轮）；不给桌面 App 补 helper 进程匹配（先确认它的真实进程树）。
「装上 opencode 跑一个会话复现本版修复」是最高优先级的后续，但**需要用户本机装它**。

## [0.0.144] - 2026-09-26

### 收两个竞品一手调研（MonoCode / DeepChat），并据此修正方案三处

用户给了两个仓库：[hardbeat920/monocode](https://github.com/hardbeat920/monocode)（1303 star / MIT）
与 [ThinkInAIXYZ/deepchat](https://github.com/ThinkInAIXYZ/deepchat)（6343 star / Apache-2.0）。
它们不是动效案例（不进 `docs/research/ui/`），是**竞品与路线参照**，按 `05-magpie-research.md`
那类一手调研的规矩落进 `docs/workbench/`：

- **[12-monocode-competitor-audit.md](docs/workbench/12-monocode-competitor-audit.md)（573 行）**——
  **最近的同形态竞品**：同样 Tauri v2 + Rust 后端 + TS 前端，同样给 coding agents 做桌面 GUI，同样管 10 家。
  它最值得我们抄的不是任何功能，是三件工程纪律：
  **① Rust 侧与前端同级配的测试量**（逐文件实测 `#[test]` ≥ 333：`fs.rs` 112、`session_store.rs` 43、
  `harness.rs` 40、`skills.rs` 20；前端 327 个 `.test.ts`；CI 三平台矩阵 7 条命令含 `clippy -D warnings`），
  而且**它把协议层做成纯函数，于是没装 agent CLI 也能测 Codex 解析**；
  **② 凭据零落盘**——`account_identity.rs:12-17` 只返回 `{email,name,plan,organization}` 四字段的显式 DTO，
  多账号靠 `CLAUDE_CONFIG_DIR` + `CLAUDE_SECURESTORAGE_CONFIG_DIR` / `CODEX_HOME` 指向各自 profile 目录
  并 `env_remove` 掉 API key 变量（`harness.rs:634-638`），连钥匙串条目都随 profile 分家；
  读钥匙串只为拿 usage，注释原话 "no token is sent anywhere"；
  **③ `capabilities/default.json` 用 `"windows": ["*"]`**（本轮已亲自核实）——不用逐个列窗口名。
  harness 三层抽象（core 纯函数 / providers 各管协议 / Rust 只 spawn）与
  「可选能力成员 + `canXxx()` 查询、UI 因此零 `if (harness === ...)` 分支」也记了，
  但**明确判定不抄**：它一家一个 37KB 协议文件、protocol 层总量 20 万字节级，靠测试硬撑；
  我们只有 Codex 一家，引入 adapter 层是自找负担。
- **[11-deepchat-another-route.md](docs/workbench/11-deepchat-another-route.md)**——**另一条路线**
  （当 agent 的前端：跑 agent、喂 agent、显示完整会话）。文末那张「两条路线边界表」是防方案写偏用的：
  它每个功能都要问「agent 协议支持吗」，我们每个功能只问「磁盘上读得到吗」，成本差一个数量级。
  一条具体警戒：**不要因为它有 Tape & Trace 就给自己加「可回放诊断日志」**——它的 Tape 是它自己启动的
  agent 的结构化记录，天然完整；我们的会话尾是读别人写的日志，格式会变、字段会缺、还可能读不到。
  **同样的功能名，我们的版本只能是降级的，且必须说出降级了多少。**
  另记两条与我们直接相关的：它的 remote control 已有 **`/pending`** 命令（在消息应用里回答待确认的
  权限请求）与 `/pair` 配对——证明「待确认」是一个值得成为一等概念的交互件；
  它的 Skills 支持**按会话启用**与**跨工具互导**，比我们现在的全局安装更细更安全。

**据此修正了 [10-replan](docs/workbench/10-replan-2026-09-26.md) §5A 三处**：

1. **Phase 0 的 capabilities 修法改了**：原来打算把不存在的 `"main"` 从 `windows` 数组里删掉；
   现在采用 MonoCode 的写法 `"windows": ["*"]` + 按平台拆 conf。理由不只是修 bug——
   Phase 1 要新增 `sidebar` 窗口，每次加窗口都要回来改一次，`"*"` 让这件事不再发生。
2. **Phase 0 的 Rust 测试有了标尺**：`provider.rs` 的合并逻辑必须按纯函数写
   （这是它能被测试的前提），优先测纯函数模块而非需要 spawn 的路径。
3. **Phase 2 补两个凭据坑**（我们原本完全没意识到，都直接适用于只做 Codex 的方案）：
   ① 档位目录用 `CODEX_HOME` 指向外，**还必须 `env_remove` 掉 `OPENAI_API_KEY` /
   `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`**——否则环境里的 key 会盖过档位里的，
   用户以为切了其实没切；② **删档位必须真删钥匙串条目**（ MonoCode 按
   `SHA256(NFC(绝对路径))[:8]` 定位 service 条目，先 `security delete-generic-password` 再删目录），
   否则留孤儿凭据。这一条要配一条反向断言测试。

`docs/workbench/README.md` 文档表补 11/12 两行。

**本轮没做**：一行代码都没改。两份调研都是**只读**——MonoCode 读了 `harness.rs`（39KB）、
`account_identity.rs`、`rate_limits.rs`、`registry.ts`、`types.ts`、`providerAccounts.ts`、
`capabilities/default.json` 等核心文件与全量文件树；DeepChat **只读了 README 与 GitHub API，
源码一行未读**（已在篇末声明）。我们自己的代码一字未动。
Swift 侧无改动，测试基数仍为 544 条。

## [0.0.143] - 2026-09-26

上一轮把六个问题留给用户拍板，本轮已定。同时用户补了一句关键范围：「**我用 codex 但我不用 claude**」，
于是 Phase 2 从「Claude Code + Codex 两家」收窄为**只做 Codex 一家**。

**六项拍板**（全部记入 [10-replan-2026-09-26.md](docs/workbench/10-replan-2026-09-26.md) §7）：

1. **装 Rust 工具链让 `app/` 构建成功**（不改走 SwiftUI 侧边栏、不暂停）
2. **Phase 0 不发版**，推进到 Phase 1 完成后一起发
3. **档位元数据明文 + 凭据进钥匙串**——与 `CONTEXT.md` 凭据口径一致；
   代价照原样接受（`.bak` 备份必然含完整 key，靠 gitignore + `0600` + 不进 DTO 三重兜）。
   **这条推翻 CC Switch 可抄性的一半**（它全明文 SQLite + JSON）
4. **Codex 用原生 profile-v2**
5. **reduce-motion 与彩色存量维持 Phase 4**，本轮只保证侧边栏新建部分接唯一真源
6. **「Codex 需重启」只读检测 + 明确提示**，复用现成五态信号；kill 用户进程的选项明确排除

**范围收窄为只做 Codex 一家**，理由有两条且都硬：
- 用户不用 Claude Code；
- 本机 `~/.claude/settings.json` 顶层只有 `permissions` 与 `hooks`，**没有 `env` 块**——
  即没有 API key / base_url / model 的落点。**在一个没有 env 块的机器上，「切 Provider」没有可切的东西。**
- 附带好处：`provider.rs` 不必处理「两家配置格式不同 + 生效语义不同」的组合，只需吃透 Codex 一套。
- 连带改写：产品名从「Provider 切换」改为「**Codex 配置档位**」；「暂只支持 2 家」这类表述全部消失
  （没有「暂」，本来就只有一家要做）；界面必须写清「此档需 `codex --profile <name>` 启动」与
  「切档对已在跑的进程无效」两件事。

**顺带补了一条本机实况，它纠正了方案里的一处表述**（10 号 §7.0 / 问题A4）：

`codex --help` 的原文是 `-p, --profile <CONFIG_PROFILE_V2>` ——
**Layer `$CODEX_HOME/<name>.config.toml` on top of the base user config**。
关键在 **"on top of"**：profile-v2 **不是替代 base config，而是叠在它之上的一层**。
所以「用 profile」与「改 base」不是二选一——base 一直在底层，profile 只是覆盖它想覆盖的键。
本机 Codex 版本 `0.155.0-alpha.16.4`（比方案假设的 0.134+ 更新），
当前无任何独立 profile 文件、`CODEX_HOME` 未设 → Phase 2 是从零建档，不是迁移。
`model` / `model_reasoning_effort` / `service_tier` 这些要切的键目前都在 `config.toml` 顶层。

`docs/workbench/README.md` 同步：进展表 Phase 0' 改为「待开工（已拍板不发版，第一件事 `rustup default stable`）」、
Phase 2 行改为「Codex 配置档位（只做 Codex 一家）」、新增「v0.0.142 六项已拍板」一节。

Swift 侧无改动，测试基数仍为 544 条。


## [0.0.142] - 2026-09-26

### 四路调研后重划改造方案：三个前置阻塞与 9 条被推翻的地基断言

用户要求「根据所有收集到的信息重新规划方案，可以多 agent 分方向、由一个 agent 汇总」。
起了 5 个 agent：四路分头调研（代码资产 / 14 篇 UI 案例提炼 / 风险与验收 / 竞品与边界），
第五个做汇总。产出 [`docs/workbench/10-replan-2026-09-26.md`](docs/workbench/10-replan-2026-09-26.md)（403 行）。
四份原始报告在本机 `/tmp/uibatch/`（不入库）：`asset-audit.md` 476 行、`design-spec-draft.md` 654 行、
`risk-and-acceptance.md` 828 行、`competitor-boundary.md` 652 行。

**被推翻的 9 条 workbench 断言**（完整表在 10 号 §1，每条带证据），最重的三条：

1. **「`app/` 已跑通」不成立**——写完了没构建过。无 `app/src-tauri/target/`、无 `gen/schemas/`；
   `capabilities/default.json:5` 声明 `windows: ["island","main"]` 而 `tauri.conf.json:13`
   只定义 `island`（`main` 其实是 `main.rs:356` 的**托盘图标 id**，被误当成窗口）；
   `placement.rs:57-62` 的 macOS 分支返回写死的 `(0,0,1440,900,1)`，
   侧边栏「贴左/右」在 3024×1964 屏上位置全错；`bundle.targets` 只有 `nsis`（无 macOS 打包目标）；
   本机无默认 rustc 工具链。→ **Phase 1 的第一步不是写 `sidebar.css`，是让 `cargo tauri build` 成功一次。**
2. **「Rust 核心 2670 行几乎零改动复用」不成立**——2670 这个数字对，判断错。
   `engine.rs:1-7` 七条 `use crate::` 全指自家模块，它是**与 Swift 并列的第二份独立实现**：
   registry Rust 12 个档案 vs Swift 26 个（只重叠 8 个 id）、会话方言 Rust 4 种 vs Swift 5 种、
   `engine.rs:445-452` 的 `--demo` 播 6 个 id 而 5 个不在自己 registry 里。
   三端（Swift / WPF / Rust）之间**零一致性守卫**。
3. **「Phase 0 已完成」要降级**——扫描器修复是真的，托管是假的：
   `scripts/test-scan-secrets.sh` 在磁盘上存在但被 `.gitignore:38` 的 `*secrets*` 挡在库外，
   `git log --all -- <path>` 为空。而 `.gitignore:41-43` 给 `scan-secrets.sh` 与 baseline 开过三条例外，
   **没给它的测试开**。

另 6 条：`01-current-state.md:148` 的「293 个 git 文件」实测 **324**、「12 模块」实为 10 个 `mod`；
「保留注释与缩进」的自由归因一半错（确凿来自 **Magpie 官网**，CC Switch 的 Claude JSON 侧反而
按键名排序重排，只有 Codex TOML 侧用 `toml_edit` 保住）；「Codex config.toml 含 `[model_providers]`」
本机实扫已无该 section；「既有 544 项测试」对侧边栏**不构成任何保护**（544 项全部
`@testable import AgentIslandCore`，Rust 侧 `#[test]` / `#[cfg(test)]` / `tests/` 实测 0）；
「CC Switch 是主要借鉴对象」定位要改（它 v3.20.4 / 136,819 star / 管 10 工具，provider 档位这块
已被完全占据，最危险的相邻产品其实是 QuotaBar）。

**三个前置阻塞**（不解决就无法开始对应 Phase）：① `app/` 构建不通 → Phase 1 无从开始；
② Rust 端零测试 → 21 条验收大部分悬空；③ 扫描器对真 key 零命中，而 Phase 2 就要碰真 key。

第三条我自己复验过（不只是引用 agent 结论）：`scripts/scan-secrets.sh:45` 的 `cred_prefix` 要求
`sk-` 后紧跟 16 位字母数字，所以 **`sk-proj-`（OpenAI）与 `sk-ant-api03-`（Anthropic）的实际前缀
都因连字符断开而漏报**，只有 `AKIA`（AWS）命中；`apiKey = "..."`（camelCase、值带引号）也不命中
`secret_assign`（它只认 `password|passwd|api_key|access_token|auth_token|secret` 这些下划线形态）。
**一个反向事实**：万一真 key 长得标准，命中的是 `cred_prefix`——这一条是绝对零命中、不许进 baseline
（`scan-secrets.sh:56-57`），`release.sh` 是 `set -euo pipefail`，所以最坏是**发版时炸**而不是发出去后道歉。
对策按优先级：掩码做成**类型**而非函数（`ProviderSecret(String)` + crate-private `init` + 手写
`impl Serialize`，让「忘加掩码」编译不过）；`log_line` / `log_from_ui` 统一出口脱敏；`dist/*.zip` 进扫描范围。
扩规则降级为可选——它会立即产生新增命中要 `--rebaseline`，且不解决「只看内容不看数据流」这个根本缺口。

**竞品那一路挖到两个决定方案形状的事实**：
- **「切换是否真生效」按工具不对称**：Claude Code 的 `env` 块**热生效**（官方原话
  "A running session applies new and changed values to its environment when you save the file"，
  但有「只能加不能删」的语义坑）；**Codex 的 `Config` 启动时一次性构造**，
  `codex-rs` 全仓 grep `reload` / `watch` **零命中**，官方 issue #3860 正是这个缺口 →
  **Codex 切档必须重启进程**。→ Phase 2 的验收核心改成「这两件事在界面上必须是不同的可读状态」。
- **`provider.rs` 要从「档位整份覆盖」升级为「档位 + 通用配置片段」两层**：CC Switch 有一个专门修
  「切走就丢插件/hook」的设计（`json_deep_merge` / `merge_toml_table_like` + 切走前重新提取）。
  我们原来的 `payload = 整份配置片段`意味着用户每次切档，他新装的 MCP、改的 hook 会被**静默冲掉**。
  这不是将来的优化项，是 13.6w star 的同类产品踩过并专门修过的坑。
  另：**还原不能是「写旧字节」**——`auth.json` 含 `refresh_token` + `last_refresh`，
  写旧字节会把 codex 已刷新的新令牌退回旧的；Codex 档位改用原生 profile-v2
  （`${CODEX_HOME}/<name>.config.toml` + `--profile`），自造格式会把用户锁在我们 app 里。

**设计侧**：14 篇案例提炼成七层规范草案（动效准入门槛 / 形状容器 / 颜色配额 / 状态呈现口径 /
过渡曲线 / 可达性降级 / 声音），每条带篇目出处，README 23 条共识里 3 组张力已裁决
（总裁决规则：**按形态层级决定切换强度**）。明确排除项（gooey/metaball、液态玻璃折射 shader、
玻璃拟态卡片、光标入镜示能、React 特效库、SVG morph 数学）各附不适用的理由。
另核实出一条真缺口：`accessibilityReduceMotion` 只覆盖 3 个 View（6 处），
另有 2 处 AppKit 动画（`IslandPanelPositioning` 的 `NSAnimationContext`）**天然读不到 SwiftUI 环境值**，
Web/Windows 端零覆盖，且已接的三处各自读自己的、**没有唯一真源**。

**`docs/workbench/README.md` 已同步**：入口从 07 改为 10 号（07 的技术断言已被推翻，产品定位仍有效），
文档表补 10 号一行，进展表把 Phase 0 从「已完成」降级为「部分完成」并新增 Phase 0'「建可信地基」，
两处过期括注（「`app/` 已跑通」「Rust 2670 零改动复用」）改为带出处的更正而不是删掉。

**本轮没做**：一行代码都没改，六个前置问题全部留给用户拍板（都在 10 号 §7，各带推荐答案与代价）：
① `app/` 构建阻塞怎么解（推荐装 Rust 工具链让它构建成功）② Phase 0 是否单独发版（推荐不发）
③ provider 档存哪（推荐元数据明文 + 凭据进钥匙串，与现有凭据口径冲突需拍板）
④ Codex 档位是否接受用户带 `--profile` 启动（推荐接受）⑤ reduce-motion / 彩色存量是否提前（推荐维持 Phase 4）
⑥ 「Codex 需重启」要不要主动检测（推荐只读检测 + 提示，不要 kill 用户进程）。
Swift 侧无改动，测试基数仍为 544 条。

## [0.0.141] - 2026-09-26

### 案例库收 Progressive Payment Reveal：数字是数出来的，不是跳出来的

Mídé（[@mide_ajibade](https://x.com/mide_ajibade)，8454 followers）的
"🛳️ —• Progressive Payment Reveal Interaction."——238 赞 / 4629 浏览 / 113 收藏。
新增 [`14-mide-progressive-payment-reveal.md`](docs/research/ui/14-mide-progressive-payment-reveal.md)。

- **一手证据：26.97 秒视频完整下载并逐帧分析**（1080×1080@30fps，原始 2160×2160）。
  `ffmpeg select='gt(scene,0.02)'` 零命中，确认是一条连续操作录像；抽 21 帧，
  内容由 OCR + 语义读图逐字读出。
- **核心发现——「渐进」不在视觉炫技，在数字本身**：`0% paid` 用约半秒数到 `47%` 再落到 `50%`
  （d1.5 / d2 / d2.5 三帧 OCR 逐字确认：`0` → `47` → `50`），进度条同步从 0 长到一半。
  同一个数字若用 CSS transition 只会"跳一下"，而它是**可读地数上去的**——
  用户在半秒里看清了"正在从 0 变成 50"，而不是看到一个突然的 50。
- **界面是分期付款管理，三个 tab 同属一个外壳**：`Installment`（`Full Payment - UG` /
  `Plan 1 • 2 installments • ₱ 6,000 total` + 进度 + 两期明细 / `Autopay from wallet` 开关）、
  `Progression`（`Curriculum Progression` + `CGPA: 1.22` + 四色图例学分）+ `Courses`
  （`CS101` / `Chemistry for Engineers`，展开后六个字段）。切 tab 时外壳不动。
- **明暗主题切换被真实演示**：x14/x15 两帧确认月亮激活、界面确实变暗——不是只放个开关没切。
- **另一条可抄**：`Courses` 卡折叠态两行、展开态六个字段，同一张卡同一个箭头（箭头由下变上）。
  这是 [12](12-halogen-recorder-capsule-states.md)「一个控件的几种形态」的**第三个独立样本**。
- 四色图例用**色点 + 名称 + 数值**，颜色不是唯一载体；`Due 26 Feb 2027 • in 164 days`
  把绝对日期与相对天数并排（前者存档、后者决策）。

**对本仓最直接的一条**：Token 用量与耗时改成"数上去"。
**但附了前提**：必须先确认取值频率——若是每秒轮询，每秒都数一次反而比跳变更吵，
应当**只在值真正变化时数**。另有「进度条与数字必须同源」：两者不同步是这类控件最常见的 bug，
而且**截图看不出来，只在动的时候暴露**。这与 [11 篇](11-plasma-ui-liquid-glass-panels.md)
「同一光学参数的两个消费者共用一份来源」是同一条纪律在数据层的版本，已写成共识第 22、23 条。

**没核实的**：这是作者的个人 demo，**未找到公开仓库或站点**（推文只给视频），所有实现层结论
都是从画面推断、没有代码可核；数字滚动的曲线与步长只从 0.5s 间隔读到三个值，**没有逐帧追**；
进度条与数字是否严格同步没做像素测量；`0% → 50%` 是用户触发还是自动演示看不到输入；
界面是菲律宾学生场景（`₱` / GCash），**数字本身没有可迁移性，可迁移的是呈现方式**。

Swift 侧无改动，测试基数仍为 544 条。

## [0.0.140] - 2026-09-26

### 案例库收 Morphing Dropdown：形态列表范式的第二个独立样本

Kopp（[@koppkev](https://x.com/koppkev)，瑞士，build [@details_so](https://x.com/details_so)）的
"morphing dropdown ✨ available in the vault."——846 赞 / 39,659 浏览 / **975 收藏**。
收藏高于点赞是"想照着做"的信号，比点赞更值钱。新增
[`13-kopp-morphing-dropdown.md`](docs/research/ui/13-kopp-morphing-dropdown.md)。

- **一手证据：9.17 秒视频完整下载并逐帧分析**（1152×720@60fps，原始 1920×1200）。
  `ffmpeg select='gt(scene,0.02)'` 零命中，确认是一段连续操作而非拼接；抽 30 帧，内容全由 OCR 逐字读出。
- **它是什么**：Details.so 的 Vault 页面里演示一个 hero 导航下拉。同一段视频里，
  面板在**四种内容形态**之间反复变形——
  `Explore` 一时给**两栏图文**（`Guided Tours` / `Private Expeditions` + 描述），
  一时给**四格图文**（`Canyons` / `Forests` / `Mountains` / `Coastlines` 各配一句描述）；
  `Experiences` 一时给**纯文字洲际列表**（`North America` / `South America` / `Africa` / `Asia Pacific`），
  一时又给两栏图文。导航项的高亮指示器跟着内容同步换（OCR 在展开项后读到 `^` 残影）。
- **最该抄的一条**：这不是四个下拉菜单，是**一个面板的几个状态**。
  如果做成四个独立下拉，就有四套开合状态、四份定位逻辑、四倍测试面；
  做成一个面板换内容，只有一份。判据是**内容形态是否共享同一个容器几何**。
- 这与 [12 篇](docs/research/ui/12-halogen-recorder-capsule-states.md) 是同一取向的**第二个独立样本**，
  于是「形态列表」从个人风格升级为可当收敛结论用的范式：12 篇讲一个控件的四种高度（内容只增减行），
  本篇讲同一个面板整体换形（图文 ↔ 纯列表，容器不动）。已写成共识第 21 条。
- 另记两条：**收起要真的回到初始态**（d8.8 帧面板消失、hero 与导航完全恢复首帧构图，
  无残留半透明层、无残留高度）；**纯文字列表与图文网格是同一面板的两种内容密度**——
  轻内容不需要另做一个简版菜单。

**没核实的（已在篇内单列）**：Vault 里该 snippet 的**源码没拿到**（需登录，本篇未注册），
所以"同一个容器"是从画面判定而非代码确认；高度变化的具体数值与曲线未做像素测量；
是否用了 spring 及其参数未推测；**同一导航项在不同时刻为何给出不同内容，触发条件未读到**
（可能是依次演示预设，未确认）。站点 Vault 首屏抓到的 8 条 snippet 标题**不含**本条
（Parallax Images / Reveal Navigation 2.0 / Morphing Carousel 等），说明它在需登录的部分。

Swift 侧无改动，测试基数仍为 544 条。

## [0.0.139] - 2026-09-26

### 案例库收 Plasma UI：第一次拿到完整视频，逐帧抓到一次熔断-重连

[CruxGarden/plasma-ui](https://github.com/CruxGarden/plasma-ui)（npm `@cruxgarden/plasma-ui`，MIT，
v0.3.0）——液态玻璃面板库，`<Plasma>` 面板接触时因表面张力融合、背后一切可见物被折射、
松手吸附网格。新增 [`11-plasma-ui-liquid-glass-panels.md`](docs/research/ui/11-plasma-ui-liquid-glass-panels.md)。

- **这篇的一手证据比前面所有篇厚一档：视频完整下载并逐帧分析了。** 36.2 秒 / 1652×1080 /
  60fps / 2173 帧，用 `ffmpeg select='gt(scene,0.02)'` 独立测出两处硬切点（22.75–22.79s
  playground→landing，27.65–27.82s landing→GitHub 页），共抽 72 帧逐帧读。
- **最硬的发现：它的 playground 把融合状态做成了可读计数器**（`N panels, M joined`）。
  于是能用 0.2 秒间隔的帧抓到一次完整熔断-重连：**`4 joined → 0 joined`**，发生在 16.8s 与
  17.0s 之间（< 200ms），重连在 17.6s 与 17.8s 之间。同段里 **blend distance 从 40px 调到 56px，
  joined 上限从 3 升到 4**——参数与结果共变可读（共变是事实，因果标为推断）。
  另意外佐证：GitHub 页那条提交 `Show a longer demo: panels tearing away and fusi…`
  从字面就印证了这个熔断实验是库自己做的演示。
- **两种融合形态分别记录，不许互换引用**：细颈（t16.5 帧，颈宽约面板宽 1/5–1/4，S 形中点收细）
  与宽桥/团块（d17.8/f19 帧，Files 旋转后嵌进 Notes）。即使在 4 joined 时每个面板轮廓仍可辨，
  "三颗珠子串在绳上"，没有溶解成无形团。
- **上游亲手调好的 preset 表从 docs 站点源码提取**（README 只给参数不给组合）：Lumen / Studio /
  Slate / Aqua / Neon / Entropy 六个预设各带原话与关键值，另有 Water/Gel/Honey/Solid 四档粘度。
  其中 `Aqua` 是"透明如水，六个 sheen 全关只留透镜"。
- **shader 注释本身就是设计规则**，逐条抄了：只在朝向不同处融合（转角/缝隙/台阶）、
  法线由 2D 高度梯度求出、rim 要翻卷出"肩"而不是薄片、金属与云是行进出来不是画出来的
  （Beer-Lambert + Henyey-Greenstein）、颗粒活在页面坐标所以面板缩放时颗粒不动。
  谱系也记了：Blinn 1982 metaballs / IQ 的 SDF 与 smooth minimum / Fiedler 的定步长积分。
- **最该抄的一条**：它的 playground 常驻显示 `N panels, M joined` 与 `24px grid`，
  **那不是给用户的开关，是让效果可被验证**。与 `doctor` / 可信度自查同源——
  把"我看到的"和"实际是什么"分开显示。
- **第二条**：上游把"减去所有装饰"做成一个合法配置而不是 fork，playground 原话
  "Turn all four off... and the plasma is a plain lens - the Aqua tab above."
  → 任何材质系统都该能表达"只要功能不要装饰"，否则克制只是口号。

**逐帧分析里如实留下的落差（这是本篇最该被读的部分）**：README 宣称
"Each panel undulates like a Slinky when moving"、viscosity 0 水 1 糖浆可调，
但 **36.2 秒里没有任何一帧能证明拖尾或粘度差异造成的运动形态差异**——d17.0/d17.4 两帧
视觉读图明确写无泪滴、无彗尾、无拉长、无运动模糊。已写进「没能核实的」，
本项目若要写"拖尾可调"须另找素材或标为未验证。另有 9 条未验证（shimmer 流动、
dispersion 色散分裂、网格吸附、mood 生效、六材质对应、reduced-motion 等）与 4 条推断
均已与事实分开列。

**顺带修正上轮的一处取证失误**：07 篇写"twimg mp4 下载失败，拿不到逐帧"，
并在文末声明"所有动效过程的判断都不存在"。那是**当时只试了直连和两三个代理网关，
没试本机系统代理**——`127.0.0.1:10808` 是能完整下载的（本篇的 13.9MB 就是这么来的）。
07 篇与该声明已加更正说明，并提炼成案例库共识：「**证据不足时先审自己的取证路径，
再归因于上游。**」

案例库另有 5 条新共识（15–18），Halogen 又补两条（19–20）：把效果状态做成可读的、"全关装饰"是合法配置、
间距要么小于融合距离要么大于它别停中间、以及上面那条取证纪律。README 索引补 11 号一行。

### 补收 Halogen 案例（v0.0.139 追加）

研究过程中 subagent 又沿链发现一篇与 `shell_mode` 直接相关的素材：sasha birukoff
（[@sashabirukoff](https://x.com/sashabirukoff)）的 **Halogen** 常驻录屏小部件
（视频 11.5s / 1080×1080 / 30fps / 344 帧，素材在本机 `.scratch/ui-material/11-halogen.mp4` 不入库）。
新增 [`12-halogen-recorder-capsule-states.md`](docs/research/ui/12-halogen-recorder-capsule-states.md)。

- **它对我们最有价值的一条**：同一个常驻控件在 闭合 / 两行展开 / 录制单行 / 三行菜单 / 回落闭合
  之间切换，**全程共用同一个容器、同一套点阵图标、同一套细描边**——变的只有高度、行数与显隐。
  → 这正是 `shell_mode` 该有的口径：灵动岛与侧边栏是**一个容器的两种高度**，不是两套 UI。
- **第二条**：录制中把不可用的 `Screenshot` **移出场外而不是置灰**——常驻 UI 上长期放一个点不动的
  元素是负债。第三条：塌缩用**行淡出**（图标+文字一起变暗→消失，计时数字等塌缩完成后才出现），
  全程没有 scaleY 压缩；计时数字是白字，红色只属于 Record 图标。
- 与 [10 篇](docs/research/ui/10-swiftui-craft-invite-card-spring.md) 看着对立（那条讲切换要让尺寸
  参与、要夸张；这条讲塌缩要安静），合起来的判据是**按形态层级决定切换强度**：同层内换内容可以夸张，
  跨层增减内容要安静。
- 一手证据：全时间轴 6fps 采样 69 帧 + 展开/塌缩两段 5 帧裁剪放大读图标。
  已如实标注哪些是目视判定（点间距、是否 spring 及参数、计时出现的精确帧号）**未做像素级时序测量**。
- 修掉这篇自己的三处断链（`06-farhan-*` 误引、workbench 相对路径少一层），全库相对链接已脚本验通。

**本轮没做**：一行 Swift 都没改。两批素材（Plasma UI 与 Halogen）**共七个 demo/视频一个都没实跑**
（playground、workspace example、Halogen 源项目都未在浏览器里操作），一手证据止于 README 全文、
站点 HTML、推文元数据与抽帧。macOS 26 有系统 Liquid Glass 材质，**优先级高于把上游那套 WebGL
自绘搬过来**——可搬的是规则不是 shader。Swift 侧无改动，测试基数仍为 544 条。

## [0.0.138] - 2026-09-26

### 案例库从 4 篇扩到 10 篇：29 个外部 UI 素材逐个研究入库

用户一次给了 29 个链接（7 个素材站点 + 19 条动效推文 + liquid-taffy 与 morphicons 两个库）。
`docs/research/ui/` 原来只有 3 篇，本轮补齐并修正了两处自己写错的结论。

- **新增 7 篇指定素材**：
  - [`04-ui-resource-sites.md`](docs/research/ui/04-ui-resource-sites.md)（610 行）：六个站点横向对比。
    最关键的发现是 **Beautiful UI 与 BoardUI 两批互不相识的开发者收敛出了同一份 agent 界面词汇表**
    （Thinking / Approval Card / Tool Chips / Task Rows ↔ `agent-thinking` / `questionnaire` /
    `task-list` / `agent-progress`），其中 `questionnaire` 与 `04 Approval Card` 是同一个需求的两种实现。
    另抄到 beUI `Dynamic Island` 源码（外壳动真实宽高不动 transform、`RADIUS = 32` 常量永不做动画、
    出场比进场短一个量级且不加 blur）与 BoardUI `agent-log` 的五段时长（`height 0.38 <
    opacity/filter/y 0.42 < mask 0.44`，容器先让位文字后到位）。
  - [`05-morphicons-and-tools.md`](docs/research/ui/05-morphicons-and-tools.md)（342 行）：
    旋转是**解出来的**不是声明出来的——2D Procrustes 闭式解 `θ*=atan2(S_xy−S_yx, S_xx+S_yy)`，
    residual 驱动的分支（≈0 → 纯旋转）。附带一条对常驻 UI 重要的判断：它把图标 morph
    划为 reduce-motion 下"一般可接受的 micro-transition"于是默认播放，把决定权做成显式 prop。
  - [`06-liquid-taffy-goo-engine.md`](docs/research/ui/06-liquid-taffy-goo-engine.md)（703 行）：
    本批最硬的一篇。**gooey 边框为什么不膨胀**——轮廓是两条 iso-alpha contour 之间的缝，
    每个工作 blur 配自己的一对阈值（离线栅格化 32px 圆盘积分墨量解出），blur 与阈值同帧切换，
    且 rim 遮罩必须用同一张表。另录：弹簧不用缓动曲线（两条物理弹簧采成 GSAP CustomEase）、
    关节点光三条规则（一 body 一 lobe / 焊接是 latch 不是 test / lobe 大部分停在关节亮的半径外）、
    以及 `motion.ts` "prefers-reduced-motion, asked in one place" 的可达性形态。
  - [`07-ui-motion-tweet-sample.md`](docs/research/ui/07-ui-motion-tweet-sample.md)（246 行）：
    19 条推文当作一次抽样。10 张视频封面全部逐张视觉分析（OCR 取文字 + 语义读图取画面）。
    最硬的一条结论：**命名是作者的，特征是画面的**——`@arknow91` 自称 gooey toggle，
    三次定向读图都确认该帧只有一个连续形状、无 blur、无 neck、无 second blob；
    而 `@AlbiaHossain` 的 liquid glass 折射形变**确实在场**。20 张配图里最有参考价值的是
    **@Griveau 的 Linear 组件**：胶囊是唯一形状语言、彩色只编码状态（绿 `+232` / 红 `-17` /
    蓝紫对挑全部承载语义）、**单胶囊内用 1px 分隔线做多信息复合**。
  - 另三篇为研究过程中 subagent 引申发现的同源案例（编号 08–10，README 已注明哪些是指定素材）：
    [`08`](docs/research/ui/08-farhan-video-dashboard-info-partition.md) Farhan 视频工作台静态稿、
    [`09`](docs/research/ui/09-farhan-filenns-refining-details.md) Farhan 侧边栏 + 用量卡、
    [`10`](docs/research/ui/10-swiftui-craft-invite-card-spring.md) withAnimationUI 的 SwiftUI 邀请函。
    三篇素材与分析均为一手实样（图/视频在本机 `.scratch/ui-material/`，不入库）。

- **修正一处我上轮写错的结论（重要）**：v0.0.137 时为了回答"动画 WebP 无视 reduced-motion
  对我们会怎样"，我 grep 了 `reducedMotion` / `prefersReduced`，**零命中**，于是写进 README
  已知限制：「所有动效都不看 prefers-reduced-motion」。**那是错的**——这两个是 Web 侧拼法，
  SwiftUI 的键名是 `accessibilityReduceMotion`。按正确键名核实：`DockedSliver`（2 处）、
  `AgentRingView`（3 处）、`ActivityMatrixDots`（1 处）**已接**，其余 View 未接。
  README 该条已重写为「只覆盖 3 个文件，不是全部」，01 篇与 06 篇里引用旧结论的位置同步更正，
  并把"曾写错什么、为什么错"留在原地当反例。
  **教训写进了案例库 README：跨语言搬结论前，先确认那个词在目标语境里叫什么。**

- **收敛彩色配额的那个判断没有成真，如实改写**：07 篇从 Linear 那组推出"五态色应当是唯一
  允许出现的彩色"，但一查本仓就发现做不到位——五态色确实是 `Theme.statusWorking` /
  `statusIdle` / `statusOffline`（Theme.swift:171–173）这套语义名，但彩色不止五态在用：
  `Theme.sydedockCyan` 64 处（详情页数据色，如「工作总耗时」）、`actionBlue` 31 处、`focusBlue` 9 处。
  已从"照此宣布已达成"改成**待判问题**（数据色要不要让位给状态色）。

- **脱敏闸拦下一条**：09 篇原稿引用了设计稿里作者**公开**的邮箱，`scripts/scan-secrets.sh`
  新增 3 条未备案命中直接拒发。已脱敏——公开不等于该被本仓转载（转载会让下一次全文 grep 的
  人把它当联系方式）。这条也写进了案例库规矩。

- **并发 agent 的两处越界，已处理**：4 个 subagent 中两个自行引入了用户未给的素材
  （withAnimationUI 与 Farhan 两篇、另下载了一个未使用的 emilkowalski 视频）。
  素材与分析都是真实一手，经用户确认后**全部保留入库**，用 08–10 的编号与指定素材分开，
  README 注明区分规则。`docs/workbench/09-emilkowalski-*.md` 与本次任务无关，**未纳入本次提交**。

- **案例库 README 的规矩从 5 条加到 10 条**，新增的都是本轮真金白银换来的：
  写完自查引用（行号回头 grep）、跨语言搬结论前先确认关键词、别把作者命名当画面事实、
  未核实到的单列一节、素材不入库只放 `.scratch/`、外部读到的真实个人信息一律脱敏。

**本轮没做**：一行 Swift 都没改。案例库里已经有十几条指向具体控件的建议
（微细条呼吸灯改不对称速率、attention 态用 `Thinking...` 胶囊、Linear 三行结构重画、
`offline` 用更暗的灰、动效与声音成对设计），但它们全部**未排期**——按案例库的定位，
要做哪条就开 issue，不要把案例库读成待办板。九个 demo（morphicons playground、
liquid-taffy `npm run dev`、ThreeUI 各模板）**都没有实跑**，一手证据止于源码、页面 HTML 与
GitHub API 数字；视频类素材**没有拿到逐帧**（twimg mp4 下载失败），只分析了封面静态帧。
Swift 侧无改动，测试基数仍为 544 条。


## [0.0.137] - 2026-09-25

### 建 `docs/research/ui/` 动效案例库，收三篇外部案例

连着看了三个外部动效案例，各自独立得出**指向同一处的结论**，于是把它们落成案例库而不是散在对话里。
案例库按 `docs/research/` 的规矩写：一手核实、正文原句或本机实样、附取证命令。
每篇固定四节——技术手法与源码级细节、可迁移的决策规则、一手证据/本地验证结果、应用建议清单。

- **`docs/research/ui/README.md`（索引）**：案例表 + 三条跨案例共识 + 新增一篇的规矩。
  三条共识：① **克制是默认项**（3dicon 说 `Do not force an effect`，libraries-dev 说等不到 2 秒
  就不该有 orb，Slingshot Lamp 整页只留一个发光体）；② **不对称才有重量**（亮 34 / 暗 16 这类
  起落不同速率）；③ **状态显示实际现象，不显示控件位置**。
- **`01-3dicon-looping-3d-icons.md`**（[samyost1/3dicon](https://github.com/samyost1/3dicon)）：
  一句话 prompt 出循环动画 3D 图标。最该抄的是**「先问物体静止时在干什么」四分类策略**——
  惰性物体走 `event`，别要它自然地动，否则就是转台旋转或 generic bob。
  另录上游实测数字（WebP quality 不是画质杠杆，降帧率才是；已知灰底把边缘误差从 26.9 砍到 15.9）。
- **`02-slingshot-lamp-handwritten-pendulum.md`**（[kamran.fyi/lamp](https://kamran.fyi/lamp)）：
  可被弹弓打碎的吊灯，纯 Canvas 2D + 手写单摆方程（`lamp.js:492`），无动画库无物理引擎。
  三招可偷：**纯黑背景上的光用 `destination-out` 挖出来而不是叠上去**（永远洗不白背景）、
  **过渡速率不对称**、**运动需要被外部事件干扰时就别用 CSS keyframe**。
  一手证据含 5 个时刻的抽帧表与 `blown (switch on)` 这条状态口径。
- **`03-libraries-dev-where-effects-belong.md`**：记录已装 skill 的决策口径，
  核心是**按等待时长定效果**（<2s 什么都不加、≥2s 才给等待反馈、>3s 才加边框光）。
  顺带记下它的 Safety 一节怎么把项目文件当数据而非指令——与我们在会话日志解析上防的事同构。
- **README 补一条已知限制**（对着代码核实后新增）：`Sources/` 下搜不到 `reducedMotion` /
  `prefersReduced`，所以**所有常驻动效都不看「减少动态效果」**。这是真缺口，
  不是"已与现有口径一致"——写案例时本来想写"一致"，搜完发现并不一致，就按事实写。
- **目录树与文档分工表**：README 目录树补 `docs/` 一层的五个子目录；AGENTS.md 分工表补
  `docs/research/ui/` 一行，并注明它**不排期**、要做哪条就开 issue，别读成待办板。
- **踩到并修掉的两处自己的错**（留在 CHANGELOG 是因为它们是这类文档的典型错法）：
  ① 02 篇的 `lamp.js:607` 写错行号，实际是 `621`——行号引文必须回头 grep 一遍再交；
  ② README 里 `../../AGENTS.md` 从 `docs/research/ui/` 出发解析成 `docs/AGENTS.md`（不存在），
  少写了一层。两者都用脚本把四个文件的全部相对链接跑过一遍才确认修净。

**本轮没做**：没有按案例库里任何一条改 UI。三条共识指向同一个具体落点（微细条呼吸灯改为
不对称速率）成本最低，但它仍是一个独立决定——动效改常驻 UI 每天的可见频次很高，
值得单独一轮并单独验收。也没装 3dicon 的 skill（它需要 OpenRouter key 与 Python 环境，
且产物是 WebP，本项目用不上）；三个案例的网页与源码只是**读过**，没有 install 任何上游包。
Swift 侧无改动，测试基数仍为 544 条。

## [0.0.136] - 2026-09-25

### 给本项目装上 `libraries-dev` skill，并钉死它的安装方式（本回合唯一的改动）

起因是一条外部推文（Jakub Antalik 的 [libraries-dev skill](https://x.com/Jakubantalik/status/2103145432390262954)）：
它把「动效该加在界面的哪个位置、该用到什么程度」写成 agent 能读的知识。skill 本体是
[Libraries.dev](https://github.com/Jakubantalik/Libraries.dev)（7 个 React 视觉特效库）的配套，
规则全部面向 Web 界面——对 AgentIsland 这种 SwiftUI/macOS 项目**没有可直接调用的组件**，
它的价值在决策规则本身：按等待时长定效果（<2s 不加、≥2s 上 thinking 指示、>3s 才允许加边框光）、
同一屏不叠两种特效、效果必须匹配明暗主题。这套判断与我们自己写灵动岛等待态时的取舍是同源的。

- **装了什么**：`.agents/skills/libraries-dev/`（SKILL.md + `references/01..07` 七个库的参考文档，共约 1,900 行），
  `skills-lock.json` 记来源（`Jakubantalik/Libraries.dev`）与 hash。`.claude/skills/libraries-dev`
  是指向它的**相对软链**，Claude Code 直接读得到。
- **收敛成单份**：官方 CLI 的 `--agent '*'` 是 `--all` 的别名，会往项目里写 17 个 agent 目录
  （Antigravity、Cursor、Copilot、Goose、Junie、Qwen、ZCode…），每份都是同一批文件的**完整副本**——
  装了 10 个，共 1.2MB 重复。全部删掉，只留 `.agents/` 一份源 + `.claude/` 一条软链。
  安装命令连同这个坑写进了 `AGENTS.md` 的「Agent skills / Installed skills」小节，免得下次再铺一遍。
- **`skills-lock.json` 的 `skillPath` 已修正**：安装器写的是 `skills/libraries-dev/SKILL.md`
  （它自己创建的那一层），删掉副本后 lock 会指向一个不存在的路径。改成 `.agents/skills/libraries-dev/SKILL.md`，
  `npx skills list` 复验通过，认得这个项目级 skill。
- **`.gitignore` 未加豁免**：`.agents/` 不在任何忽略规则下，但 `*secrets*` 与 `*credentials*`
  这类模式将来有可能误伤 skill 文件，届时按需补 `!` 例外。
- **脱敏**：`scripts/scan-secrets.sh` 全绿，18 条命中全部在既有 baseline 内，无新增。

**本轮没做**：① 没有把它当代码库用——它不提供 SwiftUI 组件，本项目不会
`npm install thinking-orbs` 或 `border-beam`；② 没有顺势改造灵动岛的等待态动效（那要单独一轮，
且要先定「等待多长才值得加动效」的口径）；③ 没有装 Pro skill（`npx libraries-dev skill --pro`），
免费版对本项目已经够读；④ 没有改任何 Swift 源码，测试基数与 v0.0.135 相同（544 条）。

## [0.0.135] - 2026-09-25

### 🔌 CLI 终于读得到 App 里的自报：`GET /state` 与 `agentisland state`（11 号票）

v0.0.134 发版后真机验收撞出来的：App 侧 `POST /session` 回了 `bound:true`，而同一时刻
`agentisland status --json` 对同一个 Agent 给的是 `idle / inferred`。原因不浅——
**自报记录活在 App 那个进程的引擎里**，CLI 是另起进程做一次性采样，它的登记表永远是空的。
于是 04 号票给 JSON 加的 `provenance` / `conflictStatement` 两个键，在 CLI 侧
**结构上永远不会**出现 `selfReported` 与冲突句。脚本作者读到 `inferred` 就会得出
「这个 Agent 没自报过」——而真相是「这个进程看不到 App 里的自报」。
上一轮把这条写进了字段注释与 README（止血），本轮补的是读通路（治）。

- **`GET /state`**：与 `/session` 同一枚令牌，返回 `{generatedBy:"app", capturedAt, agents:[…]}`。
  每格只给聚合状态：`id / name / status / provenance / conflictStatement / processRunning /
  selfReportedState / selfReportExpiresInSeconds`。
  **刻意不给** `currentAction`、`detail`、`message`、会话 id——那些是从别家 Agent 的会话正文里来的
  （命令、路径、提示语）。有一条哨兵用例钉住这件事：往快照里塞三个哨兵串，读端点的正文里
  一个都不许出现。将来谁想「顺手多给一个字段」，红的是测试。
- **`agentisland state [--json] [--port <n>]`**：读到了就列（含「自报还剩 N 秒」与冲突双显）；
  读不到就**说清为什么读不到**——连不上 / 被拒 / 端点不存在（App 比 CLI 旧）/ 200 但解不出，
  四件事各有自己的一句话与自己的 `reason` 枚举，**不折成一个布尔**；
  失败一律退出码 1，不给调用方一个看起来正常的空表。
- **不与 `status` 合并**（这是本票最重要的一条取舍）：`status` 那一行是本 CLI 自己采的样，
  把 App 侧的 `selfReported` 拼进去，就会在同一行里出现「CLI 说 idle」与「App 说待确认」两个
  互相矛盾的结论而读者看不出来。两个来源就是两条命令，各自标清是谁给的。
- **顺手收掉一处重复**：`41999` 此前在 App（`LocalEventServer.defaultPort`）与 CLI
  （`NotifyCommand` 的默认值）各写一份，帮助文本里还有第三处。现在只有
  `SelfReportWire.defaultPort` 一处，帮助文本改成插值；结构断言扫 `Sources/` 下
  除 Core 那一行之外的所有字面量（m89 就是拿这个去验的）。
- **代价与被否决的方案**：
  ① 让 CLI 直接读 App 落在磁盘上的状态文件——被否：自报记录今天**不落盘**
  （02 号票的口径是内存 + 24 小时证据），为读它而新增一份磁盘副本，等于制造第二个会漂移的真值源；
  ② 把 `provenance` 合并进 `status` 的每一行——被否，理由见上；
  ③ 改走 MCP（07 号票）——被否为「本票的做法」而非被否定：MCP 只服务模型，
  而 `status`/`doctor`/报表这些既有消费者都在 shell 侧；读端点先做好，MCP 将来直接复用同一个；
  ④ 连不上时静默回落成独立采样、只打一行小字——被否，那正是本票要治的病。
- **一条诚实的边界**：`/state` 的令牌校验写在 executable target 里，runner 链接不到它，
  所以行为级测试碰不到，只能按**顺序**守（`authorizeSelfReport` 必须出现在给正文之前、
  拒答必须出现在给正文之前）。真机那条 curl 走的是实际链路，见本条目末尾的验收段（发版后当场补）。
- **自查补掉的两处**（都在 CLI 那一侧，同样链接不到测试，所以只能写在这里）：
  `state` 原来用 `(try? JSONEncoder().encode(...)) ?? Data()` 打印，编码失败会输出一个 `{}`
  并以 0 退出——那是把「我没答上」伪装成「这里没有东西」，正是本票要治的那类错；现在编码失败
  走 `malformed` 那句话并退出码 1。另一处是 `fetch` 里一个自用的时间戳变量（`_ = started`），
  是写的时候留下的死代码，删掉。
- **本轮没做**：`status` / `doctor` / 审计报表仍不读 `/state`（它们各自要先回答
  「两个来源不一致时听谁的」，那是口径决策不是接线）；04 号票欠的那张岛内视觉核对截图仍欠着。

- **测试与变异**：544 条全绿（新增 12 条，`AgentStateTests.swift`；v0.0.134 基线 532）。
  变异在仓库副本 `/tmp/AgentIsland-mut5` 跑，先量干净基线，逐条列出红在哪一个用例
  （脚本现在两样都报：锚点命中数、每条变异红的到底是哪一条）：
  | 变异（把被测逻辑改坏） | 红在哪（基线 `544 通过, 0 失败`） |
  | :--- | :--- |
  | m80 `/state` 放开方法白名单 | 1 条：只有 GET 进得来那条（期望 405，实际放行） |
  | m81 信封不再声明是谁给的（`app`→`cli`） | 1 条：`generatedBy` 那条 |
  | m82 拒答状态码从 403 漂成 200 | 1 条：没令牌读不到任何东西、回脸不含注册表内容 |
  | m83 读端点只看「带没带令牌」而不校验 | 1 条：结构断言「先验令牌，再给状态正文」 |
  | m84 把「被拒」折进「解不出」 | 1 条：classify 表格那条（denied→malformed） |
  | m85 连不上那句不再点名自报 | 1 条：五件事各一句那条（**第一版幸存**，见下） |
  | m86 状态词自己造一份（不复用 `ActivityLevel`） | 1 条：措辞复用那条 |
  | m87 来源完整说法里 `inferred` 变空 | 1 条：同一条用例里 inferred 那一断言 |
  | m88 剩余秒数不钳 0 | 1 条：钳 0 那条（印成 -70） |
  | m89 CLI 重新写死端口字面量 | 1 条：端口单一定义的结构扫描 |
  | m90 失败 reason 不再用枚举值 | 1 条：`--json` 失败格那条 |
  | m91 503 不再单列（折进「解不出」） | 1 条：`engineNotReady` 那条 |
  | m92 失败句重新承诺「下面这份」 | 1 条：反向断言（锚点命中 2 处，见下） |
  | m93 `--port` 缺值时静默用默认端口 | 1 条：缺值必须报错那条 |
  | m94 `engineNotReady` 的 reason 折成 `malformed` | 1 条：503 单列那条（reason 侧） |

  **15/15 全部变红，无幸存、无等价变异、无 flake。** m92 的锚点 `这次没有状态可读。` 在
  `AgentState.swift` 里命中 2 处（连不上与引擎未就绪两句共用这个结尾），脚本只替第一处；
  这一次替到的正是「连不上」那句，失败断言打印出来的正文就是被替后的句子，所以是有效的红——
  命中数照旧报出来，是为了让「只替到第一处」这种事下次不再靠事后回看发现。

  **m85 第一版幸存**：断言原本只要求输出里出现「自报」两个字，而 `--json` 的表头与冲突句里
  本来就有「自报」，于是把最关键的那半句话改掉也没人红。重写成断言第一行含具体短语
  「自报记录」，并加反向断言「下面这份」在任何一条失败句里都不许出现，m85 与 m92 才各自红在
  该红的那一条上。这已经是连续第三轮撞见同一个形状（**断言的关键词在别处也合法出现**），
  所以新增断言时先问一遍：这个词除了被测处，还有谁会打印它。

- **外部 review：Codex CLI（`codex review --uncommitted`），连续第 4 轮非退化路径。**
  报出 **3 条 P2、无 P0/P1/P3**，三条都在本轮修掉，而且全都落在我新写的那段代码里：
  ① `--port` 是最后一个参数时**静默改用默认端口**（用法错误被报成网络问题）——参数解析整体搬进
  Core 的 `parseArgs`，缺值/非数字/0 三种形状都出声；② 服务端明确的 **503 被折进 `malformed`**
  （「等一等就有」与「要升级」是两件事，折起来就会让用户去做不需要做的事）——新增第五种结论
  `engineNotReady`；③ 失败句写着「**下面这份**是本 CLI 独立采样的结果」，而 `state` 失败时
  什么都不打印、直接退出码 1——**承诺一份不存在的输出**，与「把没看到讲成没有」同类，
  句子改成陈述事实，并加一条反向断言：五种失败句里任何一条都不许出现「下面这份」。
  报告在 `docs/code-review/2026-09-25-1150-v0.0.135-codex.md`；它没有可用 SDK，所以是静态读码结论，
  门禁与变异由作者跑。落 issue 的仍是那条：`status`/`doctor`/报表要不要改用这个读端点，
  需要先回答「两个来源不一致时听谁的」。


## [0.0.134] - 2026-09-25

### 👁 自报开始参与显示：来源维度进快照，冲突时两句话都给（04 号票）

02 号票打通了 `/session`，03 号票把它的形状测到能改——但收进来的东西**至今没有一个界面读它**，
README 的已知限制里一直写着「自报的状态不参与显示」。这一轮把它接上：`AgentSnapshot` 多一维
`provenance`（spec 第 4 节的四种：`selfReported` / `observed` / `inferred` / `conflict`）。
加这一维的理由不是「信息更全」，是**同一张卡片上从此会混着两种可信度**：带令牌自报的
「我在等确认」与 CPU/写入兜底猜出来的「看起来在干活」，此前印成同一行字，用户没法分辨那句是谁给的。

- **采信规则三条，每条防一种症状**：① 没有可信自报 ⇒ 读到会话强语义是 `observed`，
  只有双信号兜底是 `inferred`；② 自报与**强语义观测**对不上 ⇒ `conflict`，状态仍按观测走
  （岛不该为一个自称在干活的会话挂着一张不存在的脸），但两句话都要显示；③ 自报只与**弱信号**
  对不上 ⇒ `selfReported` 并采信它的状态——带令牌的那句话比「CPU 有点高」更有资格决定卡片写什么，
  而 TTL 就是它的保质期。
- **「自报说 X，进程表说 Y」只在 Core 写一次**（`AgentSnapshot.conflictStatement`），
  短标签「 · 自报」也只在 Core 写一次（`AgentProvenance.badgeSuffix`）；卡片副标题、悬停、
  详情页、灵动岛货架、`status` 终端输出、`status --json` 六个出口全部问它们。
  结构断言钉住：那五个显示文件里不许出现自己的「自报」字面量，`Models.swift` 里那句原文只许出现一次。
- **自报的四种状态与卡片那四种逐字对齐**（`SelfReportState.label` 直接取 `level.label`）：
  双陈述那句里两边叫法不一样（「干活中」vs「工作中」）会让人以为说的是两件事，这条有用例钉。
- **一条边界写死在这里**：自报**只改显示**，不改事件与外发。`recordTaskCompleted`、待确认告警、
  资源激增告警、远程外发（ntfy/HTTP/SMTP）全部仍由观测那一侧驱动——`displayLevel` 是在这些判定
  **之后**才算出来的。理由是这条通道对本机任意进程开放（只要有令牌）：一个外部声明不该能
  推送到用户的手机上，这与 02 号票「无令牌 ⇒ 落回未采信」是同一种担心。
- **代价与被否决的写法**：
  ① 冲突时让自报顶掉观测——被否，理由就是上面那条②；
  ② 离线卡片也硬给一个 `observed`——被否：进程都不在时「它说了什么」这个问题不成立，
  那是给一次没发生的对撞盖章（与 `isHung` 的三态同一个理由，所以 `provenance` 是可选值，
  漏算的地方得到的是「没说」而不是一个假的「观测」）；
  ③ 冲突那一拍让自报的 `ask`/`detail` 去填动作行——被否：副标题与动作条是两处，
  两处各替用户挑一次，等于这一维白做；
  ④ TTL 过期的那条证据当冲突显示——被否：过期只是「不再采信」，不是「没说过」，
  永远挂着会把「它说过、后来断了」做成一条持续告警。02 号票留的 `expiredEvidence` 这一轮
  **仍然没有消费者**，落进 issue。
- **外部 review 抓到三条「我没对账的地方」**（Codex CLI，报告在
  `docs/code-review/2026-09-25-0938-v0.0.134-codex.md`：1 条 P1、2 条 P2，全部本轮修掉）：
  ① 采信自报改了对外状态，而**引擎级的 `anyWorking` 仍按观测算** —— 卡片说在干活、引擎按
  「没人干活」把文件扫描降频，同一拍里两个答案；② `AgentObservability` 的依据句会把自报来的状态
  写成「本轮读到了会话强语义」——**拿别人的证据给自己的结论背书**，而那句会印在 `doctor` 与报表里；
  ③ 灵动岛货架那行只处理了冲突，没处理「可信自报」，于是这一维只在展开列表里存在。
  三条修完之后我先跑了一次门禁：**529 全绿**——也就是说它们当时**没有任何断言覆盖**；
  补了用例（含这三处）再复跑才有脸说修好了。这条顺序记下来是因为它容易忘：
  「读了 review 改了代码」不等于「改了代码」。
- **本轮没做**：`AgentObservability` 补第六类结论（05 号票，本轮只把依据那句说真）、设置页的
  接入命令（06 号票）、以及 `provenance` 在报表/审计报告里的位置（那要等 05 的口径）。

- **测试与变异**：532 条全绿（新增 14 条，`SelfReportTests.swift` 的来源维度那一节；v0.0.133 基线 518）。
  变异在仓库副本 `/tmp/AgentIsland-mut4` 跑，先量干净基线（15 条那一批是 `531 通过, 0 失败`，
  之后补的顺序不变式那条是 `532 通过, 0 失败`），逐条列出红在哪一个用例：

  | 变异（把被测逻辑改坏） | 红在哪（基线 `531 通过, 0 失败`；m76 那一批是 532） |
  | :--- | :--- |
  | m60 去掉冲突判定（自报永远压过观测） | 3 条：两条冲突用例 + 对外 JSON 那条 |
  | m61 离线卡片也硬给一个来源 | 3 条：inferred/observed 那条、离线留 nil 那条、TTL 到期那条 |
  | m62 采信自报但不覆盖状态 | 2 条：「卡片按自报的状态写」与边界那条 |
  | m63 冲突时状态被自报顶掉 | 3 条——其中一条红成**「自报说工作中，进程表说工作中」**：一句自相矛盾的话，正是这条断言的价值 |
  | m64 自报那句 detail 完全不上卡片 | 1 条：副标题留空会被抓到 |
  | m65 可信窗口判定失效（过期也算自报） | 3 条：02 号票原有的两条 + 本轮的 TTL 那条 |
  | m66 观测/推断也挂上标签 | 1 条：「噪声当信息」那条断言 |
  | m67 冲突那句不看 provenance | 1 条：一致时不许造出冲突 |
  | m68 对外 JSON 丢掉 provenance | 1 条：契约那条 |
  | m69 读到强语义就一律算冲突 | **4 条**：一致与不一致两种形状都要红 |
  | m70 冲突那一拍也让自报顶掉动作行 | 1 条 — **第一版幸存**，见下 |
  | m71 引擎聚合位不跟着采信的状态走（外部 review 的 P1） | 1 条：`anyWorking` 那条新断言 |
  | m72 可观测性依据把自报说成会话强语义（P2） | 1 条：依据句那条新断言 |
  | m73 货架副标题不带来源标签（P2） | 1 条：结构断言 |
  | m74 审计报告那一格不带来源标签 | 2 条：Markdown 那一行的两条断言 |
  | m76 让自报也去驱动事件（顺序不变式） | 1 条：「不制造事件、不推送到手机」那条 |

  **m70 第一版幸存**：那条断言当时写在「观测给了动作」的冲突形状上，而 `action ?? 自报那句`
  在这种情况下本来就取前者——**改坏与不改坏输出一样**。真正能被观察到的形状是
  「冲突且观测没有动作」（进程不在那一侧），把用例挪到那里之后才红。这与上一轮 m47 的
  「锚点命中 2 处只替 1 处」是两类不同的假幸存，所以脚本现在两样都报：锚点命中数、
  以及每条变异红的到底是哪一条用例。

  **m76 是一条插入式变异**（不是改值）：把「让自报去调 `recordTaskCompleted`」这段错代码插回引擎，
  用来证明那条边界断言守的是**顺序**——`displayLevel` 必须在所有事件判定之后才算出来。
  这类不变式没有对应的「改坏某个值」的变异，只能把错的样子造出来测。

- **发版后真机验收（两条通过、一条撞出自己的边界、一条没做完）**：
  ① 带令牌 `POST /session?agent=dim` → `bound:true` + 自己的 `expiresAt` ✓；
  ② `agentisland status --json` 里 `provenance` 这个键**确实存在**并有真实取值（`inferred`），
  说明这一维在采样链路上真的算出来了 ✓；
  ③ **但 CLI 侧永远读不到自报**：App 里刚绑定成功（`bound:true`），另起进程的
  `agentisland status` 对同一个 Agent 给的是 `idle / inferred`——因为自报记录活在 App 那个进程的
  引擎里，而 `status` 是一次性独立采样、登记表恒为空。于是「脚本用户也要看得见冲突」这句话
  只对了一半：**代码接上了，数据没通**。这条已写进 `CLIModels.swift` 的字段注释、README 的已知限制，
  并单独落成 issue 11（读端点 or 走 MCP，两条设计摆在那里待选）；
  ④ **岛内视觉核对没做完**：截图时另一个会话正在同一台机器上操作 DimAgent，屏幕上挂着一个
  macOS 隐私授权对话框（不是本 App 的，也未替用户点）。所以「卡片挂出 · 自报 标签、
  货架出现双陈述」这两条今天只有单元/集成级证据（`来源:` 那一节 14 条 + m60-m76 变异），
  **缺一张人眼核对过的截图**——留给下一轮第一件事。
  另外记一笔自己的操作失误：为了唤起面板我跑过 `open -a AgentIsland` 与
  `./dist/agentisland open --help`（后者把参数当 id 拼进了深链，触发了一个无害的
  `agentisland://agent?id=...`）。在共享屏幕上这属于会打扰别人的动作，下次先用 `agentisland://expand`
  这类只读深链，不去抢前台。

- **外部 review 的结论一句话**：Codex CLI（连续第 3 轮非退化路径）报 **1 条 P1 + 2 条 P2、无 P0/P3**，
  三条全部本轮修掉并补了断言；没有落 issue 的分歧项，只有两条明确留给 05 号票
  （第六类可观测性结论、以及 `expiredEvidence` 到底要不要成为它的证据）。审查者自己没有可用 SDK，
  所以是**静态读码结论**，门禁由作者跑；`--uncommitted` 不接受自定义 PROMPT，重点清单没能交给它。

## [0.0.133] - 2026-09-25

### 🧪 本机 HTTP 入口的形状下沉进 Core 并补契约测试，顺带修掉「分块到达的 POST 被当畸形请求拒掉」

03 号票要的是 `/session` 的契约测试。它的难点不在断言，在**被测的东西链接不到**：
`LocalEventServer` 是 executable target 的一部分，测试 runner 进不去，所以「什么样的请求
会得到哪个状态码、哪句话」这一层此前只在 DTO 上断言过。本轮把切分、收完判定、路由、状态码、
原因短语、`/notify` 的 type 映射**整体搬进 `AgentIslandCore/LocalEventHTTP.swift`**，
服务端只剩传输胶水（收字节、判收完、把结论发出去）。搬的过程里撞出了三条真缺陷。

- **分块到达的合法请求会被拒**：上一版一次 `receive()` 就切一次 HTTP，切不出 `\r\n\r\n`
  就回 400 `Malformed HTTP request`。TCP 把一条稍大的 POST 拆成几段是常态（单次收的上限才 64KB），
  也就是说**这不是畸形而是正常网络**。现在按 `Content-Length` 判「收完没有」，没收完就继续收，
  只有连接真的断了才回畸形。
- **多字节字符被切开时整包解码会失败**：上一版一次 `receive()` 就把整包
  `String(data:encoding:.utf8)` 解一次，一个切在中间的汉字让它是 nil，于是合法的中文正文被拒成
  400 `Invalid encoding`。挡住这条的是**两步一起**：先按 `Content-Length` 判收完，再头/正文分开解。
  这条不是修辞——变异实验里「只把解码改回整包」是**等价变异**（不红，m42），
  而「只去掉收完判定」会红（m41），所以功劳不能记在其中一半头上，代码注释里也这么写。
- **原因短语那张表会把新加的码降级**：`default: "Bad Request"` 意味着任何新增一格状态码
  忘了跟着加就会拿到一句错话——本轮的 413 正好是这一格。表搬进 Core 逐码给，
  认不出的码按**类别**给（4xx ⇒ Client Error、5xx ⇒ Server Error），不假装知道具体含义。
- **超限有一条明确的话**：超过 64KB 的请求现在得到 413 与那句带上限的说明，
  而不是被静默截断或继续吃内存（缓冲跟着连接走，`dismiss`/答完/超时三条路径都会清）。
- **代价与被否决的写法**：① 支持 `Transfer-Encoding: chunked` 被否——这条回环链路上的调用方
  （curl、各家 hook、自带 CLI）都带 `Content-Length`，在没有需求时加分块解码等于留一条没人验的路径；
  ② 把 `LocalEventServer` 整个搬进 Core 被否——它要 `NWListener`/`NWConnection` 与一次
  `@MainActor` 引擎跳转，搬过去只是把不可测的东西换个地方；③ 用真 socket 测（起第二个端口）被否——
  41999 被正在运行的实例占着，测试里再起一个会跟用户的 App 抢端口。
- **一条诚实的边界**：服务端那一格胶水（`needMoreData` ⇒ 继续收、不许作答）仍然只能按
  **书写形态**守（在 `case .needMoreData:` 那一段里不许出现 `send(`），因为 executable target
  链接不到测试。行为级的证据来自本轮末尾的 `nc` 分块实测，不来自套件。
- **自己造出来又当场收掉的一条**：把处理改成异步分支之后，旧代码里「调用方在
  `processHTTPPayload` 之后统一 `dismiss`」那条兜底没了——`.reply`/超限这些终态会留着
  按连接的缓冲**和 `liveConnections` 的名额**。名额只有 16 个，跑满 16 条请求这个本机端点
  就会自己把门关了。修法是把回收挂到**唯一的写响应出口**（`send` 先 `dismiss` 再写），
  并加两条书写棘轮（`connection.send(content:` 全文件只许一处、`send` 那一段里必须有 `dismiss`）。
  外部 review 独立报了同一处（它看的是修复前的快照），两条路径指向同一个缺陷。
- **另一处重复判定**：`/notify` 的 `type` → 事件类型这张表**有两份**——HTTP 一份、
  深链 `agentisland://notify?type=` 一份，今天抄得一样，但下一格只改一边就是
  「同一个 `type=alert` 在深链里红着叫、在 curl 里绿着收」。统一到
  `AgentTaskEvent.externalType(from:)`，两边都问它，服务端与深链各加一条禁字面量断言。
- **本轮没做**：03 号票剩下的「只连不发的连接被 5s 超时回收」「并发上限 16」这两条今天仍只有
  注释与超时兜着；`/session` 的响应体逐字节契约（真 socket）也在那一票里。

- **测试与变异**：518 条全绿（新增 14 条，`LocalEventHTTPTests.swift`；v0.0.132 基线 504）。
  变异在仓库副本里跑（`rsync` 排除 `.build`/`dist`/`.git`，副本目录名含项目名——不然那条按 cwd
  判的断言会凭空红一条），先量一次干净基线，再逐条列出红在哪一个用例：

  | 变异（把被测逻辑改坏） | 红在哪一条用例（基线 `518 通过, 0 失败`） |
  | :--- | :--- |
  | m40 服务端在「没收完」时直接作答 | 2 条：`needMoreData` 那一段不许出现 `send(`、服务端不许自写 4xx —— **只能靠书写形态守**，那段胶水在 executable target 里 |
  | m41 `Content-Length` 判定失效（半条请求当收完） | 3 条：分块正文、半字符、超限 |
  | m42 只把解码改回「整包一次 UTF-8」 | **全绿——等价变异**，见下 |
  | m43 事件类型表漏掉 `alert` 那一格 | 1 条：`alert` 期望告警，实得 completed |
  | m49 事件类型表漏掉 `confirm` 那一格 | 1 条：同一条用例的另一格 |
  | m44 `/session` 方法白名单放开 | 1 条：`GET /session` 该回 405，实得 nil |
  | m45 服务端自己写一个 4xx 状态码 | 2 条：两条禁字面量棘轮 |
  | m46 原因短语表里没有 413 那一格 | 1 条：413 被 `default` 降级成 `Client Error` |
  | m47b 上限判定两处一起失效 | 1 条：越过上限必须报 tooLarge |
  | m48 未知路径从 404 降级成 400 | 1 条：`GET /nope` 期望 404 |

  **m42 是等价变异**，不是断言假：`headerData` 是整包的前缀，整包能解出 UTF-8 时头部也一定能解，
  所以「只把解码改回整包」在任何输入上都不可观察。真正挡住多字节被切开的是上面那道
  `Content-Length` 收完判定（m41 会红）。代码注释里把功劳记给两步一起，不偏到其中一半。

  **m47 的第一版「幸存」是脚本的问题**：同一句 `if data.count > maxRequestBytes` 在两个分支里各写一次，
  脚本只替换了第一处 ⇒ 有效的那处没被动过。这次脚本把「锚点命中 2 处」打了出来，
  按两处一起替换（m47b）就红了。教训：**变异脚本必须报锚点命中数**，命中多于 1 处的锚点
  要么全替、要么拆成两条独立变异——否则会把「我没改到」误报成「这条断言是假的」。

- **真机验收（发版后当场跑，套件做不到——被测的胶水在 executable target 里）**：
  对 `127.0.0.1:41999` 用 Python 直写 socket，五条：
  ① 请求**切在头部中间**分两块发 ⇒ `200 OK` + `Event accepted`（这一条在修之前是
  `400 Malformed HTTP request`）；② 切在**正文中间且落在一个汉字里** ⇒ 同样 `200`
  （修之前是 `400 Invalid encoding`）；③ 一次发完作对照 ⇒ `200`；
  ④ 70KB 的请求 ⇒ `HTTP/1.1 413 Payload Too Large` 与那句带上限的说明
  （原因短语这一格是新加的，`default` 会把它降级成 `Bad Request`）；
  ⑤ **连发 30 条 `/health` 全部 200** —— 这一条验的就是外部 review 报的那个名额泄漏：
  不回收 `liveConnections` 的话第 17 条开始就会被 `maxLiveConnections` 挡在门外。

- **外部 review：Codex CLI（`codex review --uncommitted`），连续第 2 轮非退化路径。**
  报出 **1 条 P2、无 P0/P1/P3**：终态响应不回收按连接存的缓冲与 `liveConnections` 名额
  （比内存更紧的后果是 16 条之后本机端点自己关门）。**本轮已修**，且是两条独立路径同时指向它——
  作者在收到报告前几分钟自查同一处已经改掉，外部 review 看的是修复前的快照。
  报告在 `docs/code-review/2026-09-25-0709-v0.0.133-codex.md`，里面另写清了三件事：
  它没有可用 SDK（**静态读码结论**，门禁由作者跑）、`--uncommitted` 不接受自定义 PROMPT
  （所以重点清单没交给它，其中两条由自查命中）、以及三条不采纳（chunked 编码、整体搬进 Core、
  真 socket 测试）的理由。落 issue 的仍是 03 号票剩下的那两条。


## [0.0.132] - 2026-09-25

### 🔕 Qoder 不再每续跑一轮就弹一次「任务完成」：完成事件改记在「哪一轮人类指令」上

用户报的现象：**岛对 Qoder 一路弹「任务完成」，可那个会话明明一直在跑。**
根因在 `detectQoder` 的完成指纹——它取的是**最后一条 assistant 消息的 id**，
而 Qoder 每次模型调用都换 id（本机实测 `chatcmpl-<hash>`，同一次调用的 `thinking` 行与
`tool_use` 行才共享同一个 id）。一个长任务里模型会 `end_turn` 很多次：等后台构建、
等并发会话回话、被任务通知唤醒后续跑——每 `end_turn` 一次就是一个新指纹，
引擎的「这条完成事件我已经发过了」判定就此失效，于是同一件活响一路的铃。
`end_turn` 真正的含义只是**模型这一轮说完了**，不等于这件事做完了。

- **区分「谁起的头」有现成的结构事实**：`promptId` 挂在每条 user 行上、**一整轮共享同一个值**
  （实测一个 promptId 覆盖 48~250 条 user 行），而一轮的**首行**额外带 `humanInput`（真人敲的）
  或 `isMeta`（系统注入：后台任务通知、并发会话消息）。本机一个会话实测 27 个 promptId：
  18 个 `humanInput` 起头、9 个 `isMeta` 起头，互斥且覆盖全部。取证脚本与结论落在
  `docs/research/qoder-monitoring.md`。于是完成事件的指纹改成
  **「最近一次 `humanInput` 那一轮的 promptId」**：真人派的活干完了响一声；
  系统自己续跑的续命轮沿用同一个指纹 ⇒ 引擎认得这是同一件事，不再重复弹。
- **一格备忘，因为尾窗只有 60 行**：首行只在轮次刚起头的那几拍可见，而完成事件发生在几十拍之后。
  `inspectQoderTranscript` 因此按会话根留**一格**「最近见过的人类轮次」（覆盖式，换会话文件即作废），
  递给纯函数 `detectQoder(lastHumanTurn:)`。判定本身仍然是可单测的纯函数，缓存只有那三行。
- **顺手修掉同族的另一条**：真人刚敲完指令、模型一个字都还没回时，尾窗里最新的消息行是
  带 `humanInput` 的 user 行，而最后一条 assistant 还是**上一轮**的 `end_turn`——旧判定就此
  报「任务完成」，报的还是上一件事。现在这条改报 active（「正在处理你的新指令」）。
- **代价与被否决的方案**：
  ① **只按 `promptId` 记**被否——续命轮自己也有新 promptId，等于什么都没修；
  ② **时间节流**（N 分钟内只弹一次）被否——它会把「用户真的又派了活」一起吞掉，
  而且是拿时钟当语义，本仓的口径是拿结构事实当语义；
  ③ **加宽尾窗到能看见首行**被否——单会话文件实测 10MB、每行还挂 `requestTokenAnchor` 的整包
  请求/响应，而 `LogTailReader` 的 262KB 字节上限会先于 60 行到点，加宽行数并不保证看得见；
  ④ **两个方向不确定的地方分别选了不同的保守侧**，各自有用例钉着：冷启动只见到续命轮、
  手里没有任何人类轮次可继承 ⇒ **宁可响一声**（把「认不出」做成永不完成，等于整条通知废掉）；
  而中途首行滑出尾窗、来源永久不可知 ⇒ **继承上一个人类轮**，也就是少响一次。
  后者选这一侧是因为续命轮同样是新 promptId，把它当新任务等于把这条修复抵消；
  这条取舍写进了代码注释与 README 的已知限制。
- **岛重启后也不重复弹**（外部 review 的 P2）：那格备忘是进程内的，重启即清零，
  而首行早就在尾窗之外——这时「认不出来源」若退到**当前轮次**，续命轮每轮都是新 promptId，
  等于把本轮要治的病原地复发。退路改成**会话文件的身份**（`sessionKey` = `<会话 uuid>`）：
  重启后第一次完成照响，之后的续跑共用一个指纹；真人敲新指令时备忘更新，指纹随之换新。
- **本轮没做**：其他方言（Claude / Codex / Antigravity / DSH / Cline）没动——它们的完成语义来自
  各自协议里的显式终态，不是 `end_turn` 计数；`isMeta` 是否覆盖所有注入来源（消息排队、
  `/compact` 之类）本轮样本没穷尽；**真机验收**（跑一个长后台任务，看它中途不再弹完成）
  留作下一轮第一件事。

- **测试与变异**：504 条全绿（新增 7 条，`QoderTrackingTests.swift` 6→13；v0.0.131 基线 497）。
  变异照上一轮的教训：**在仓库副本里跑**（`/tmp/AgentIsland-mut2`，副本目录名含项目名，
  否则那条按 cwd 判的断言会凭空红一条），并且**逐条列出红在哪一个用例**、先量一次干净基线。
  | 变异（把被测逻辑改坏） | 红在哪几条用例（基线 `504 通过, 0 失败`） |
  | :--- | :--- |
  | m30 完成指纹退回「最后一条 assistant 的 id」 | **6 条**：轮次身份、续命轮、冷启动退路、备忘继承、跨会话不串味、来源不可知时继承 |
  | m31 不继承「最近一次人类轮次」（去掉 `?? lastHumanTurn`） | 3 条：续命轮、备忘继承、不可知轮次 |
  | m32 忽略 `humanInput` 标记，按当前轮次算 | 4 条：续命轮、冷启动退路、跨会话、不可知轮次 |
  | m33 去掉「真人刚敲完、模型还没回话」那条 active | 1 条：那条用例直接报「用户刚敲完就报任务完成」 |
  | m34 把 `isMeta` 当成人类标记 | 4 条：两个标记读反是同源缺陷，四处都红 |
  | m35 那格备忘不按会话文件作废（跨会话串味） | 1 条：新会话不许继承旧会话那一格 |
  | m36 **不写**那格备忘（尾窗一走就失忆） | 1 条：备忘继承那条 — **第一版这条变异是全绿的**，见下 |
  | m37 认不出来源时不退到会话身份（外部 review 的那条 P2） | 2 条：冷启动退路、跨会话 |
  | m38 优先级写反（会话身份压过人类轮次） | 2 条：新任务必须换指纹、备忘继承 —— 治「永不弹」治过头的那种改法 |

  m36 第一版存活，原因不是断言假，是**用例写错了形状**：我把「首行已滑出尾窗」那一拍写成了
  还留着 `tool_result` 行的尾窗，而 `promptId` 挂在每条 user 行上——于是当前轮次身份仍然读得出来，
  那格备忘根本没被用到，删掉写入路径自然不红。改成**尾窗里只剩 assistant 行**（真实格式里
  assistant 行不带 `promptId`，而单行可以大到让 262KB 的字节上限先于 60 行到点）之后，
  备忘的写入路径成了唯一来源，m36 就红了。这条教训写进注释，是为了下次有人把用例写回
  「顺手能过」的形状时能看见：**测一条机制，就要把别的来源堵掉**。

- **外部 review：Codex CLI（`codex review --uncommitted`）——本轮结束连续 6 轮的退化路径。**
  报出 **1 条 P2、无 P0/P1**，本轮修完：那格备忘是进程内的，岛一重启就清零，此时「认不出来源」
  的退路按**当前轮次**给指纹，而续命轮每轮都是新 promptId ⇒ 用户报的症状在重启后的会话里原地复发。
  退路改成**会话文件的身份**（`sessionKey`），并补一条用例与一处变异（m37）钉住。
  报告在 `docs/code-review/2026-09-25-0514-v0.0.132-codex.md`，里面还写清了两件限制：
  它的沙箱没有可用 SDK，**是静态读码结论**（门禁由我另跑），
  且 `--uncommitted` 与自定义 PROMPT 互斥（第一次调用就报了这个），
  所以重点核查清单没能交给它——**「四个 CLI 本机都不存在」这句上一轮的记录也是以偏概全**：
  `codex` 不在 PATH，但随 ChatGPT 桌面应用装在 `/Applications/…/Resources/codex`。
  落 issue 的一条：真机验收（下一轮第一件事）。


## [0.0.131] - 2026-09-24

### 🔒 两轮 review 的收口：正文不再走标识符的归一化，以及这条修复自己带出来的「一行空白顶掉整条横幅」

0.0.130 发出去之后紧接着跑了第 2、第 3 轮独立 review
（`docs/code-review/2026-09-24-1955-v0.0.130-round2-fallback-subagent.md`、
`docs/code-review/2026-09-24-2103-v0.0.131-round3-fallback-subagent.md`）。
第 2 轮确认第 1 轮的 13 条修复与两条变异幸存**逐条兑现**，另报 2 条 P2、3 条 P3；我按它落地之后，
第 3 轮又在这批修复里报出 **2 条 P1**——其中一条是我修 P2 时自己造的。本轮把这些一起收尾。
先记一件流程上的事，免得读者对不上账：v0.0.130 的 tag 与 release 是**同仓库另一个会话**用
`scripts/release.sh` 推出去的，而那条脚本用 `git add -u` 暂存——于是当时还未提交的一部分第 2 轮修复
（`ensure()` 里那条不可观测的 `removeItem` 分支、悬空链接用例、m05 补的两条断言）跟着它自己的
Antigravity WAL 改动一起进了 v0.0.130。剩下的都在本条目里，代价是分两版发。

- **`detail`/`ask` 不再经过 `name()`**（第 2 轮 P2-1）：那个函数的语义是「标识符」——trim 首尾空白、
  纯空白按没给。它被复用在两栏用户可见的文本上，于是 `{"detail":"  在等确认  "}` 存成 `在等确认`
  （空格没了）。这不属于「拒绝与不拒绝」（形状不合格另有判据），是**静默改写内容**。
  新增只认字符串、不动内容的 `text()`；`agent`/`session` 与 URL 那侧的归一化一字未改。
  代价一是要订正 0.0.130 那句话：**「正文与 URL 同 trim 口径」只对标识符成立**；
  代价二是下面那条 P1。
- **P1：少了「全空白按没说」之后，未鉴权的一条申报可以把横幅主行与系统通知刷成空白**（第 3 轮 P1-1）。
  链路是 `message: ask ?? detail` 这条 **nil 合并**——改之前 `name()` 把 `""`/`"   "` 判成 nil，
  `??` 会自动退回 `detail`；改之后空白是**非 nil 的字符串**，于是它占住了 `message`，
  而有内容的 `detail` 被顶掉。下游三处都只区分「nil / 非 nil」：`AgentTaskEvent.summaryText`
  只挡 `isEmpty`（纯空白过闸，那一行就成了空白，默认文案全不会顶上）、
  系统通知是 `message ?? detail ?? 兜底句`（正文 = `"   "`，兜底句也被跳过）、
  「原因」折叠按钮看 `detail != nil`（长出一个展开后什么都没有的按钮）。
  **不需要令牌**就能触发：无凭证的申报照样会落回成一条事件。
  修法按 review 的建议放在**出口侧**而不是回填协议层：`SelfReportFallback.displayText(_:)`
  只判「有没有话要说」，非空白的内容一个字都不许多动——协议层保真与显示层判空是两件事，
  上一版把它们混在一个 `name()` 里，才两头都不对。
- **P1：我上一版写的「今天没有 UI 消费这两栏」是假陈述**（第 3 轮 P1-2），而它正是上面那条漏测的原因。
  真的说法是：`/session` 的**登记表**（`SelfReportRecord`）今天没有 UI 读者，
  但申报正文的两栏文字今天**会**进 UI——走的是无令牌降级的落回事件那条路。
  测试注释与 CHANGELOG 都改成了这个可核的口径，README 的「自报今天不参与显示」也收窄成
  「自报的**状态**今天不参与显示」。
- **「没令牌 ⇒ 200 + `noToken`」收成一处**（第 2 轮问答 2 + 第 3 轮 P2-1）：服务端原先在未鉴权的 POST
  分支自己写死 `status: 200, reason: .noToken`。第 2 轮我加了 `SelfReportWire.untrustedStatus`/
  `untrustedReason`，第 3 轮才发现这张表内部还留着第二份 200——`status(for:trusted:)` 的未鉴权分支
  写的是字面量，而它的生产调用点全都传 `trusted: true`（未鉴权的请求在上游就答完了），
  于是那个副本**生产不可达、唯一读者是测试**，而测试钉的是字面量 200 不是常量：
  两边各自漂移谁都不红。现在这个**生产不存在的分支整个删掉**了：`status(for:)` 不再收 `trusted` 参数，
  未鉴权那条只由 `untrustedStatus` 回答；同时新增 `rejectionStatus = 400`，把服务端绑定结果那处
  （`profileGone`）的字面量 400 也收了进来——「不采信 ⇒ 400」与「没令牌 ⇒ 200」现在各只有一处书写。
  顺带一条 review 说清了的事实：**把常量改回同值字面量这种事，行为测试原理上抓不到**（值一样），
  能抓的只有书写层面的计数/禁字面量断言；所以本轮的处置是**把这个形状改掉**，
  让第二出口无处存在，而不是再加一条容易绕的 grep。
- **棘轮改成计数式**（第 3 轮 P2-2）：第 2 轮那两条守卫是 `contains(...)` 与
  `expectFalse(contains("reason: .noToken"))`——前者只要文件里出现过一次就绿（DELETE 那侧退回
  字面量测不出来），后者只挡一种拼写（`SelfReportReason.noToken`、或本地 `let` 别名都能绕开）。
  现在改成 `untrustedStatus` 恰好出现 2 次、`untrustedReason` 恰好 2 次（两侧各一次），
  加两条禁字面量：剥完注释后服务端不许出现 `.noToken` 这个标识符的任何写法，也不许出现 `status: 400`。
  这套计数式断言改完第一次跑就抓到了 `contains` 挡不住的两种回退（DELETE 侧的 `status: 200`、
  与写成 `SelfReportReason.noToken` 的限定名），见下面的变异表 m22/m25/m27。
- **三条 P3 nit**：`untrustedStatus` 的注释原先只说「未鉴权 POST」，实际两条路径共用，会教下一个人
  再写一个字面量；`text()` 的注释补完句子并把它与 `clamped`/`clampedTTL` 之间的空行补上；
  重复的 `// MARK:` 删一行；`masked()` 上方补一句「今天还没有生产调用方，第一个是 06 号票」。
  另外顺手清了一处**已经发出去**的东西：v0.0.130 的一条注释里混进了一个俄文词（`通道在работает`），
  本机门禁与脱敏扫描都不会报它，是本轮读码时发现的。第 1 轮 review 报告里引用它的那句原文照原样留着
  ——那份文件是时间点证据，不改。
- **本轮没做**：接入命令文案（06 号票）、真 socket 的契约测试（03 号票）、
  研究清单 V1（`allowedEnvVars` 白名单外的变量会怎样）——都留在原票。

- **测试与变异**：497 条全绿（新增 1 条 `detail/ask` 用例 + 1 条落回层用例；v0.0.130 基线 495）。
  「版本单一来源」那条守卫在本轮自己红过一次：`AppVersion.string` 先改成 0.0.131 而 CHANGELOG 首条
  还是 0.0.130，门禁立刻报 `期望 [0.0.130] 实际 [0.0.131]`——这条断言是活的，不是摆设。
  **本轮的变异全部在仓库副本里跑**（`/tmp/aimut`，`rsync` 排除 `.build`/`dist`/`.git`）：
  这个工作树是共享的，另一个会话会在同一批文件上落它自己的改动，原地变异 + `restore()`
  等于拿我的快照盖它的未提交工作。副本的 `SourceTree.repoRoot` 由 `#filePath` 推出，
  所以读的还是副本自己的源码，实验成立。
  | 变异 | 红在哪一条用例（每行都是 `496 通过, 1 失败`，基线 `497 通过, 0 失败`） |
  | :--- | :--- |
  | m19 `detail` 走回 `name()` | 「自报: detail/ask 是要显示的句子…」— 期望 `"  在等确认  "`，实得 `"在等确认"` |
  | m20 `ask` 走回 `name()` | 同一条用例的 `ask` 那一格 |
  | m21 未鉴权 POST 自己写死 `200`/`.noToken` | 结构棘轮：`untrustedStatus` 出现次数 期望 2、实得 1 |
  | m22 未鉴权 DELETE 自己写死 `200`/`.noToken` | 同一条 — **这正是上一版 `contains` 挡不住的那种回退** |
  | m23 `displayText` 退化成恒等 | 新增的落回用例：全空白的 `ask` 顶掉了有内容的 `detail` |
  | m24 `message` 退回裸 `??` 合并 | 同一条落回用例 |
  | m25 服务端改写成限定名 `SelfReportReason.noToken` | 结构棘轮：`untrustedReason` 次数 期望 2、实得 1 — 上一版那条 `reason: .noToken` 对这种拼写是瞎的 |
  | m27 绑定结果那处 400 回到服务端字面量 | 结构棘轮：服务端不许出现 `status: 400` |
  | m28 `rejectionStatus` 从 400 漂到 403 | 协议用例：期望 400、实得 403 |
  | m29 `untrustedStatus` 从 200 漂到 204 | 协议用例：期望 200、实得 204 |

  一处改动只红一条用例，这是**修好之后的计数口径**：上一版那张表里有三行写着 2/3 条失败，
  那是把无关的偶发红点算到了变异头上（旧脚本只打印第一条失败明细 + 一行计数），第 3 轮 review
  按 `TestKit` 的累加口径把它证伪了，本轮连基线一起复跑订正。
  基线为什么必须干净：本轮在副本里跑，第一次的基线就带一条红
  （`ProcessInspector 读取当前进程工作目录` 断言 cwd 里含 `AgentIsland` 字样——它测的是**从哪个目录启动**，
  不是被测行为），把副本目录改名成含 `AgentIsland` 才拿到干净的 `497/0`。这条已落 issue（`audit-44`）。
  **一条幸存**：原本还要跑 m26（把 `status(for:trusted:)` 的未鉴权分支改回字面量 `200`），在共享树里
  实跑为**全绿**。不是断言假，是它**同值**——值一样的字面量与常量在任何行为测试里都无法区分。
  本轮的处置不是补一条更容易绕过的 grep，而是把那个生产不存在的分支删掉（见上面 P2-1 那段），
  这条变异就此无处可做；真正可测的漂移改由 m28/m29 两行守着。

- **外部 review（第 3 轮，退化路径）**：`dsh headless` 仍返回
  `NO_ADAPTER: no adapter registered for provider "dimagent-oauth"`，`claude`/`codex`/`opencode`/`gemini`
  四个 CLI 本机都不存在（连续第 6 轮），故由 Qoder general-purpose 子代理审 `git diff v0.0.130..工作树`。
  （**0.0.132 订正**：这句「`codex` 不存在」是只查了 `command -v` 的以偏概全——二进制随 ChatGPT 桌面应用
  装在 `/Applications/ChatGPT.app/Contents/Resources/codex`，下一轮就是用它做的 review。
  「没探到」不等于「没有」，这条口径本仓写进 README 的第一句，结果自己踩了。）
  它实跑了一次门禁（`496 通过, 0 失败`，那时还没落 P1 的用例），逐条核了第 2 轮的 5 项修复
  与信任边界（结论：无回归，`ingest` 唯一调用点仍在凭证之后，全仓 `SelfReportCredential()` 仍 1 处），
  然后报出 **2 条 P1、3 条 P2、3 条 P3**。两条 P1 与三条 P2 本轮全部修掉；
  P3 采纳三条（注释收窄、排版、`/tmp` 脚本不当证据——改成把复跑方法写在表头，脚本本身不入库）。
  **一条 review 的收获值得单独记**：它按 `TestKit` 的计数口径推断我上一版那张变异表里三行的失败数
  「不可能出现」（失败按**用例**累加，而读同一份服务端源码的用例只有一个），
  并给出旁证——m21/m22 的变异文本正是 v0.0.130 已提交、当时门禁全绿的文本。
  我复跑后**它对**：那几行真实的红是 1 条用例，多出来的计数是无关的偶发红点被归因到了变异上
  （旧脚本只打印第一条失败明细加一行计数，见下面订正）。


## [0.0.130] - 2026-09-24

### 🔑 自报通道打通：`/session` 的协议、令牌与 TTL（02 号票）

01 号票把「各家能不能自己上报生命周期」核完之后，P0 的下一步是让岛**收得到**。
本轮落地 spec 第 3 节：`LocalEventServer` 上加 `POST/DELETE /session`，带令牌、带 TTL、带 pid 绑定。
`/notify`、`/event`、深链的**处理逻辑**一字未改；但路由那一行现在会先剥掉 `?query`
（`/session` 的 `?agent=` 要靠它），所以 `POST /notify?x=1` 从 404 变成了进 `/notify`——
这两个入口本来就没有鉴权，扩的是「带查询串也认」而不是「谁能投」，写在这里是因为它确实改了行为。

- **令牌**：`~/Library/Application Support/AgentIsland/report.token`，首启生成、`0600`、目录 `0700`，
  比较走定长循环。`X-AgentIsland-Token` 的名字是 Core 里的一个常量——接入命令文案（06 号票）
  与服务端读取共用它，不然会有「一边发 X-Token、一边认 X-AgentIsland-Token」的那天。
- **两套载荷形状都收**（01 号票的直接结论）：既认我们自己定义的 `{agent, session, state}`，
  也认 hook 原样 POST 的 `{session_id, hook_event_name, cwd, …}`。
  真机接的时候发现第三条必需的路：**主会话的 hook 载荷里没有我们的档案 id**
  （`agent_id` 只在子代理场景出现），所以加了 `?agent=<id>` 这个 URL 兜底——
  没有它，「配一条 http hook 即接入」这句话就是空的。URL 只许带 `agent`，
  `state`/`detail` 一律不许从 URL 来：它决定岛显示什么，只许走正文的形状校验与截断。
- **事件映射按档案分家，而且是 allowlist**：`Notification` 在 Claude 家＝待确认，在 Trae 家**不许**映射
  （01 号票正文注明它同时覆盖「等确认」与「任务完成」且载荷没有区分字段）。
  写一张全局表就会把 Trae 的「任务完成」升格成一次假警报——宁缺毋滥，认不出就 400。
  所以现在只有 01 号票逐字核过的三家（claude / codex / qoder）共用那张表，
  其余档案一律不认事件名；`SubagentStop` 整个从表里删掉，它的 `session_id` 归属没有实样。
- **绑定顺序即口径：先令牌后 pid。** pid 匹配不能建立可信度——任何本机进程都能报一个
  真实存在的 pid，把 pid 检查放前面等于把安全边界换成一个可被猜中的整数。
  这条顺序有行为测试；而「服务端不许自己判定可信」现在由类型守着：令牌校验的产物是
  `SelfReportCredential`（`init` 是 internal），服务端**造不出一个真的凭证对象**，
  只能拿到 `authorize()` 给的那一个。被否掉的写法是把它做成 `Bool` 参数——
  那等于把「下一行就写成 `tokenAccepted: true`」的门留在原地。
- **pid 校验复用既有出口**：`ProcessTerminator.isAlive` 与 `ProcessMatcher.matchesProcessNames`
  （词边界前缀）。两个细节是刻意反着来的：
  ① `isAlive(expectedPath:)` 在读不到可执行路径时**当作还活着**（那边怕谎报清理成功），
  而绑定这边读不到就**不采信**（这边怕给错可信度）——同一个 libproc 出口，两种保守方向；
  ② 不用朴素 `hasPrefix`，否则一个叫 `claudex` 的进程能顶替 `claude` 拿到可信自报。
  ③ pid 只接受「正整数或正整数字符串」：`true`、`42.5`、越界值一律按没给（并在严格用例里拒），
  不做 `Int32(v)` 硬转——实测那种写法喂一个大数就 `Fatal error: Not enough bits…`，
  服务端会连进程一起挂掉。
- **未知档案一律 400 且不自动建档**；注册表 `AgentRegistry.builtin` 是 25 条档案
  （本机启用的只有 12 条——那是安装检测结果，不是注册表大小）。
- **TTL 到期只盖章，不清空、不消失**（spec 第 3 节的原话）：`SelfReportRecord.expiredAt`
  记一笔，`believable()` 从此不再采信它，而记录留着给 04 号票做「自报说 X / 进程表说 Y」的对撞。
  两个容易漏的点也补了：时钟回拨后 `expiresAt` 会落在未来，**申报就此永生**，所以照引擎给
  `workingSince` 做的那条防御重锚；保质期从「真正过期的那一刻」算而不是从「我们发现的那天」算。
  重锚的谓词带 `expiredAt == nil`：**只重锚还在窗口内的那些**。已盖章的那条一旦被改写了
  `receivedAt/expiresAt`，它的年龄就不再是自己真正的年龄，淘汰排序与 24 小时保质期
  都成了回拨幅度的函数——这条口径第一次变异实验没杀掉（见下面变异那节 m05）。
- **无令牌不是丢掉**：按 §3 落回既有的 `externallyDelivered` 通道——用户照样收到「它说它完成了」，
  但那条事件带未采信标记，远程外发闸门照旧拦得住。`working`/`idle` 不转事件
  （持续状态转成事件＝每续报一次响一次）。响应里的措辞按 `postEvent` 的**真实返回值**分叉：
  没投递成功却说「已按未采信事件投递」，就是拿一条谎报去补另一个谎报。
- **容量与清理**：`session id` 是外部随便起的，登记表总量封顶 64 条，**每档案再单独封顶 8 条**
  （实测灌 200 条 `qoder/flood*` 能把刚写进来的 `claude/real` 挤掉，卡片就此安静退回推断）；
  淘汰时先保可信窗口内的记录，再比 `receivedAt`。过期证据留 24 小时。
  回收只发生在**档案被删除**（`removeCustomProfile`）那一步：停用、进程被终止、快照里消失
  都不清——那些 reversible 的路径一旦顺手清掉，「它说过什么」就没了，而 04 号票要的对撞
  对象正是它。`resetTracking` 里不许出现 `selfReports`——用结构断言钉住。
- **代价与被否决的写法**：① 令牌**不进钥匙串**——接入命令要把它抄进第三方 Agent 的 settings，
  用户得看得见才能配；被否的写法是「服务端自己再开一份 store」，那会有第二个读令牌的地方，
  现在读取方只有引擎那一份。② `DELETE` 复用整份申报的解析被否决：真机接第一次就翻车，
  「撤销 live-1」回了 `unknownState`，于是拆出只认两个坐标的 `identity()`，POST/DELETE 共用它。
  ③ 不做通用表单解码（URL 只认 `agent`），④ 不在服务端判可信度（判定全在 Core，
  服务端只做 HTTP 形状，否则「什么算可信」会有两处）。
- **本轮没做**：`/session` 收到的东西今天**还没有任何 UI 消费**——`AgentSnapshot` 的
  `provenance` 维度与冲突双显是 04 号票，「复制接入命令」是 06 号票，契约测试补齐是 03 号票；
  卡片副标题写「自报」也在那两票里。研究清单的 **V1**（`allowedEnvVars` 白名单外的变量会怎样）
  本轮**仍未实测**——它需要一个真实的 Qoder hook 配置端，等 06 号票的接入命令做出来当天一起测。
- **真机验收**（ticket 的 Done-when，打包起 App 后逐条 curl 走过）：带令牌 + 真 pid →
  `bound:true` + 自己的 `expiresAt`；pid=1 → `pidMismatch`（原因里带着那个 pid）；不带令牌 +
  `completed` → 200 / `noToken` 且**真的**落回未采信事件；不带令牌 + `working` → 明说
  「持续状态、不转事件」；hook 原样正文配 `?agent=qoder` → `bound:true`；正文或 `?agent=`
  给未知档案 → 400 `unknownAgent` 且不建档；
  错令牌（48 位全零）→ `noToken`；`GET /session` → 405。撤销：已存在 → `revoked:true`，
  不存在 → `revoked:false` + `notFound` + 「可能从来没有，也可能已被容量/保质期收走」。
  令牌文件 `-rw-------` 48 字节、目录 `drwx------`。
  验收时撞出一条**没写在 spec 里的口径**：可声明的档案是**引擎当前启用的那 12 条**，
  不是注册表的 25 条——用内置但被停用的 `claude` 去报，回的是 `unknownAgent`。
  方向是对的（停用＝不采信，跟推断同一口径），但 06 号票的接入命令必须按启用档案生成，
  否则用户照着文案配好 hook 只会收到 400。已落进 issue。
- **自查后补的两处（真机接之前就自己撞上了）**：
  ① **响应不许拿错会话的到期时刻**——`/session` 的 `expiresAt` 原先取自「该 Agent 最晚过期的那条」，
  同一 Agent 开两个会话时会把**另一条**的时刻回给调用方。现在按 (agent, session) 精确回读。
  ② **令牌文件要先验明正身**：「文件里有串就用」在这台机器上不成立——
  `Application Support/AgentIsland/` 在首启之前归用户可写，任何本机进程都能抢先把 `report.token`
  写成它自己已知的那串。现在读它之前查：常规文件（非符号链接）、属主是本机 uid、
  权限不带组/其他位、内容是 ≥32 位十六进制，任何一条不合格就当没有令牌，`ensure()` 换掉它。
  诚实的边界写在代码里：**这验的是形态不是来源**，同一个用户同样 0600 的先写一串合法十六进制
  仍然挡不住；要不可伪造就得进钥匙串，而进钥匙串就放弃了「用户能把令牌抄进第三方配置」这条路。
  属主那条做成了注入值（`ownerUID`），否则它在测试里永远不会红。
  一处冗余防线也实测过并写在测试注释里：单删「非常规文件」那条判据不会让套件变红，
  因为 macOS 上符号链接自身的 mode 实测 `0755`（`lchmod` 不支持），会先被「权限不带组/其他位」
  那条挡住——**行为被两条判据同时守着**，不是无人守的空位，但也别误以为自己在审一条独立防线。

- **外部 review（第 1 轮，退化路径）**：`dsh headless` 仍报 `NO_ADAPTER`，`claude`/`codex`/
  `opencode`/`gemini` 四个 CLI 在本机都不存在（连续第 5 轮），所以由 Qoder general-purpose
  子代理独立审 `git diff v0.0.129..HEAD`——报告落在
  `docs/code-review/2026-09-24-1752-v0.0.130-fallback-subagent.md`，它是本轮为退化路径的证据。
  审出的 **P0 一条：令牌比较是 XOR 累加，不是定长比较**——`diff ^= a[i] ^ b[i]` 只区分
  「差异个数的奇偶」，实测把正确令牌的每个字节都翻转仍然判定通过，20000 个同长度随机串里
  157 个被接受。整条通道的安全边界就此失效（它挡的是「谁在说话」）。先在自己的
  `/tmp/p0chk` 里复现了因果再动代码，修法换成 `diff |= a[i] ^ b[i]`，并补一条
  「同长度、每个字节都不同」的用例——校验和写法在这条上必红。
  其余按「本轮必修」的口径逐条落地：未鉴权响应不许拿到注册表信息也不许替人说谎（响应形状
  收进 `SelfReportWire` 一张表）、时钟回拨不许让已过期申报复活、洪水不许挤掉别人的活记录、
  停用不许回收证据、「什么算可信」的第二出口用类型堵掉、事件映射改成 allowlist、
  `SubagentStop` 从表里删掉（`session_id` 归属没有实样）、pid 严格化、正文与 URL 同 trim 口径
  （**0.0.131 订正**：这条只该说标识符——`detail`/`ask` 跟着 trim 是被第 2 轮 review 报出来的缺陷，
  正文后来改走不动内容的 `text()`）、
  `identity()` 不再回传整份 JSON、档案消失报 `profileGone` 而不是甩锅 pid、20 条构建警告清零。
  **一条判断没采纳**：review 建议把 `SelfReportCredential` 推迟到 03 号票——它正是 P0 那条
  漏洞使「服务端拿到 Bool」变成可能的形状，所以本轮就做掉，代价是 03 号票的契约测试要重写签名。
  落 issue 的两条：真 socket 的契约测试归 03 号票；06 号票的「复制接入命令」文案要补
  `?agent=` 与 `allowedEnvVars`。

- **测试与变异**：495 条全绿（`SelfReportTests.swift` 新增 43 条；Antigravity WAL 回归新增 1 条；上一版基线 451）。
  本轮的变异实验是**在 review 修完之后**重跑的 16 处，逐条记下红在哪：

  | 变异 | 结果 |
  | :--- | :--- |
  | m01 定长比较换回 XOR 累加 | 红：「定长比较要挡住『同长度、每个字节都不同』的串」 |
  | m02 去掉每档案封顶 | 红：「一个说话者的洪水不能挤掉别人的活记录」 |
  | m03 淘汰不看可信窗口 | 红：「可信窗口内的记录优先于旧证据被保留」 |
  | m05 回拨重锚去掉 `expiredAt == nil` | **第一次全绿**→ 补两条断言（已盖章记录的 `receivedAt`/`expiresAt` 不许被改写）后重跑为红 |
  | m06 事件映射退回全局表 | 红：「事件映射是 allowlist，未核实的档案一律不收」 |
  | m07 `SubagentStop` 放回表里 | 红：同上那条（Trae/子代理两处一起管） |
  | m08 pid 越界直接 `Int32(v)` | 红在**崩溃**：`Fatal error: Not enough bits to represent the passed value`，测试器连摘要都没打出 |
  | m09 正文不 trim | 红：「同一个值在正文与 URL 里必须同口径」 |
  | m10 `authorize` 退化成「带没带令牌」 | 红：3 条（凭证能力 + 两条协议用例） |
  | m11 档案消失仍盲授 `.bound` | 红：「档案已经不在了 ⇒ 报 profileGone」 |
  | m12 删掉 `ensure` 里「先清非普通文件」 | **全绿**——见下面那条说明 |
  | m14 未鉴权也回载荷原因 | 红：「未鉴权的请求拿不到注册表的任何信息」 |
  | m15 没投递也说成已投递 | 红：同上那条（`message` 分叉） |
  | m16 服务端凭空构造凭证 | 编译失败（`init` 是 internal）——这条按类型级红记账，不当行为测试 |
  | m17 档案删除不回收自报 | 红：「档案**删除**才回收自报」 |
  | m18 撤销不存在的记录也说成功 | 红：「撤销要出示凭证，且没有的记录报 notFound」 |

  m12 的存活不是断言假，是**生产里那条分支永远不会被观察到**：删掉它之后建令牌、
  校验、读盘全都一样。用 `/tmp/symlinkprobe.swift`、`/tmp/danglingprobe.swift` 实测过原因——
  `FileManager.createFile` 会先把符号链接本身解掉、再建 0600 的普通文件（悬空链接也一样），
  它从不穿过链接写目标。代码里那句「createFile 会穿过符号链接」是错的，
  于是**删掉这条不可观测的分支**，把口径改写成两条真会红的平台契约测试：
  链接目标内容不许变（已有）＋ 悬空链接不许把整条通道永久钉死（新增）。
  哪天 Foundation 改成写穿，红的是测试，而不是一段没人能证明生效的 `removeItem`。
  上一版这条「24 处变异全部杀掉、无幸存」的写法本身是错的：那份清单里有几处并没有跑过，
  本轮改成上面这张逐条可复现的表（改法就是把每行写成「哪个函数改成什么」，跑法
  `swift build --build-tests && .build/debug/AgentIslandTestsRunner`，红绿以 `结果:` 行为准；
  当时另存过一份 `/tmp/mut2.py`，`/tmp` 会随重启消失，不作为证据）。
  **0.0.131 的订正**：这张表里凡按**条数**记账的行（如 m10「红：3 条」）当时由一个只打印
  「第一条失败明细 + 一行计数」的脚本产出，计数是那次运行的观察值，可能混入无关的偶发红点——
  第 3 轮 review 据此抓出了 0.0.131 上一版表格里的三行错计数（已按复跑订正）。
  从现在起本仓的变异表统一改成「红在哪一条用例」的写法，条数只在能逐条列出明细时才写。

### 🐛 Antigravity 空闲 WAL 不再触发“任务完成”

Antigravity 空闲时会周期性触碰 `~/.gemini/antigravity/conversations` 下的 SQLite `*.db-wal`，
旧规则把它当成任务写入证据，写入窗口结束后便可能补发“任务完成”。现在只在该目录过滤 WAL，
保留 `brain` transcript 的状态判断与其他 Agent 的 WAL 活动信号。

- **验证**：新增路径限定与文件树扫描回归用例；本轮完整测试 495 项通过。


## [0.0.129] - 2026-09-24

### 🔒 「待核实清单」的守卫扩到整个 `docs/research/`，七条「报绿而约定已破」一起补下界

上一轮那条断言只钉住 `agent-lifecycle-hooks.md` **一份**文件：换一份研究文档用散文写未完成项，
不会有任何东西变红——而「散文的未完成项」正是 v0.0.125/126 付过学费的坑（`.scratch/audit-42`，
上一轮自己落的 issue）。扩围之后本轮外部 review 又报出**七条「断言绿而约定已破」**，
其中三条一句话就能绕过。全部本轮修完。

- **覆盖面**：新增 `SourceTree.markdownTexts(under:atLeast:)`，遍历 `docs/research/` 下全部
  markdown；校验体从内联断言抽成 `checklistViolations(_:)`，一份文档同时错三处**一次报全**，
  而不是第一个 `expect` 就抛、剩下几份到底合规还是没看过读不出来。
  失败消息带「共 N 条／校验哪几份（报名字）／跳过哪几份」，且**按文档分组截断**——
  全局 `prefix(10)` 会被第一份文档的噪声挤满，第二份一条都读不到。
- **P1-1 声明块原先是一把不校验内容的万能钥匙。** 把任意一段带状态词与判定说法的正文包进
  `<!-- 状态词表 -->` 就全绿，块里空无一字也算声明。现在一块只管一类词（`状态词表` 管状态词、
  `禁词说明` 管引用禁词），且块本身要被审：必须逐字以 `` `词` `` 的形式给出定义、不许出现编号引用、
  ≤4/6 行、不许空块。**放行靠声明是对的，但声明必须真是那份定义。**
- **P1-2 「待核实」列只查列名存在，从不查格子里有没有内容。** 八行描述全清空而另三列合规，
  这张表照样能过——可它恰恰丢的就是「这条到底在问什么」。现在要求非空、非占位符、≥6 字。
- **P1-3 编号可重复。** `tableIDs` 是个 Set，复制一行会被静默折叠：9 行折叠成 8 个编号，
  两侧配对都看不出来，而**计数是这张表唯一的输出**。改成先数后判重。
- **P1-4 表头下一行是不是 `| :--- |` 从不验证。** 写死 `headerAt + 2` 时，对齐行被删或数据行
  顶上那一格，第一条数据行既不进逐行校验、也不进任何一侧编号集合——**一条清单项彻底隐形**；
  换真实文档的形状还会误诊成「编号没成对」。现在核对对齐行形态，不是就按第一条数据行处理并报出来。
- **P1-5 禁词的作用域从「行」→「段」→ 整篇。** 上一轮改成段，本轮实测「隔一个空行另起一段」
  仍然全绿。单位换成**文档**：判定类说法（对「我们核没核」下结论）整篇禁。
  代价是名单必须窄——「该页未列 hooks」这类对**别家正文**的陈述是证据不是定性，
  改成只在清单小节内禁（按 `## ` 标题切作用域），区外放行；上一版把两类词混在一张名单里，
  既冤枉了正当用法，又给真判定留了藏身处。
- **P1-6 触发条件原先只有一个出口 `contains("待核实")`，而那恰好是清单的列名**——把列名改成
  「未决项」整份文档静默进 skipped，状态格写什么都不看。改成四个出口的并集（触发词／四列表头／
  声明块标记／粗体编号引用），是原条件的严格超集，散文类守卫一条不减，也仍不逼任何文档建表。
  反向的冤枉也一并修：「本文档暂无待核实项」这种**否定陈述**不算欠项——判它红，最省事的修法
  恰恰是把触发词删掉，而那才是让守卫对该文档永久失效的动作。**激励方向必须对着判据。**
- **P1-7 注释写的失效面压根不发生。** 上一轮 review 声称「编号列前插一列会把硬索引 `cells[1]`
  指错列、误诊成编号没成对」——拿上一版原码跑同一变异实测：**它先红在定位条件**，报的是正确那句。
  根因是定位条件 `hasPrefix("| 编号 |")` 本身就把「编号必须是第一列」写进了约定，于是
  `cells[1]` 与 `idx("编号")` 恒等，注释里「一律从表头推」是空话。本轮把定位改成认列名
  （编号 + 状态两列即认定是清单表），**列序从此不再有含义**（插一列、四列换序都是绿的），
  那句注释第一次成真；缺列与没建表也分报两条，不再放大成每行两条噪声。
- **helper 的失败面（实测出来的）**：`FileManager.enumerator(at:)` 对**不存在的目录不返回 nil**，
  权限不足的目录也一样——只查数量下限会把「目录被改名」「读不了」都报成「只扫到 0 个」，
  所以先查存在、再查可读、再查下限。扩展名改成大小写不敏感并认 `.markdown`：
  实测 `B.MD` 与 `c.markdown` 会从清单上静默消失，而只要还剩两个 `.md` 数量下限照样过。
  表头/标记一律按 trim 后比对（缩进写的表格与声明块原先会被误判成「没建表」——那是相对上一版的回退）。
- **代价与被否决的写法**：① 声明块的豁免标签走**白名单**而不是识别任意 `<!-- … -->`，
  随手的注释不能开出免检区，代价是新增一类词要改断言；② 把触发条件做成「有表即校验」被否决是
  错的（上一版 CHANGELOG 写了个假二选一）——它其实是原条件的严格超集，本轮按超集实现；
  ③ 禁词名单不扩到「所有含『没/未』的说法」：可预期的误伤会把作者推向更含糊的措辞，那比第二出口更糟。
- **本轮没做**：今天的树上是**预防性扩围**——`docs/research/` 三份文档只有 1 份含触发词，
  校验集与上一版相同；另两份没有清单、也没被强制建。真实文档仍有 272/292 行删掉不响
  （守卫管的是表与引用，不是散文完整性）。02 号票（`/session` 协议）未开工，V1–V4 要在它打通当天测。
  外部审查者连续三轮拿不到 → `.scratch/audit-43-reviewer-downgrade.md`。
- **外部 review**：**退化路径**（`dsh` 报 `NO_ADAPTER`，`claude`/`codex`/`opencode`/`gemini` 均未安装），
  由非作者的独立 agent 审。结论：无 P0（它把校验器逐字搬进 /tmp 编译，7 000 份对抗输入 0 崩溃），
  7 条 P1 全部本轮修完、6 条 P2 采纳 5 条（另 1 条随 P1-7 一起解决）。
  它推翻了我上一版两处自述（`cells[1]` 的因果、被否决的「有表即校验」），两条都已按实测改写正文。
  报告见 `docs/code-review/2026-09-24-1335-v0.0.129-fallback-subagent.md`；
  上一轮那份外部巡检报告（`2026-09-24-1219.md`）随本轮一起入库。
- 测试：451 全绿（用例数不变）。34 处变异逐条实测，全部与预期一致：定义块夹带判定／空块／超长／
  缺一个词／未闭合／整块删掉／块里放编号引用、描述列清空与写「见正文」、同编号两行、删对齐行、
  对齐行换成隐形数据行、判定挪下一行／隔空行／写在别处／证据词写进清单小节、列名改名、
  另一份文档加清单与散文、`.MD` 扩展名、暂无待核实项不误伤、编号列前插一列与四列换序不误伤、
  缩进不误伤、缺列只报一条根因、删行／孤儿行／正文状态词／「未证实」／复核方式占位符／
  编号小写／全角数字编号／状态格清空与第四种词。其中「判定挪到下一行」与「编号列前插一列」
  两处拿上一版原码做过对照，前者由绿转红（真加强），后者由红转绿（列序不再有含义）。

## [0.0.128] - 2026-09-24

### 🧪 守约的断言补上结构下界，状态的第二出口真的拆掉

上一轮那条「未完成项必须编号+状态+复核方式」的断言，被独立 review 判为
**只闭了一半**：它自己带着三个静默失效面。本轮三条 P1 全修，P2 采纳五条。

- **P1-1 状态的第二出口没拆掉，只是换了说法。** 断言禁的是三个状态词的**字面量**，
  而正文把同一个判断留在括号外——「本轮没逐字复核」「未找到正文依据」「正文未出现 hooks」
  「没有逐条出处」「该页未列」。文档里新写的那句「正文只写编号、不带状态词」因此是假的。
  两侧一起修：正文 7 处改成「问哪件事 + 编号」，证据与状态一律进表；断言侧把禁令扩到
  同义 markers，且**只圈定含编号引用的行**，别处正常中文不误伤。
  教训：**改写不等于拆掉**——把同一个判断换个词再说一遍，第二个出口还在。
- **P1-2 编号识别的静默失效模式本身还在。** 上一版只把字符集从 `a/b` 放开到「任意一个字母」，
  但根因不是字符集窄，是**识别方式与被守约定之间没有下界**：写成 `v4`、裸文本 `V9`、`V5aa`
  仍然两侧同时隐身、全绿，而「从别的表复制一行、重排一次列宽」正好会掉样式。
  改成**结构下界**：清单每个数据行的编号格必须严格匹配 `**V<数字>[单个字母]**`，
  不匹配就指名行号与实际值。顺带治好了原先那句误导人的「每个编号恰好一行」消息。
- **P1-3 状态校验是行级 `contains`，同轮新增的列校验却是格级。** 状态格整列空白而状态词写在
  别的列时照样绿——「唯一出口」只剩形式。改成对状态格 `hasPrefix` 三分类之一。
- **P2 采纳**：列索引从**表头**推（插一列会让硬索引静默改指别的列）+ 每行列数必须等于表头列数；
  复核方式列加占位符黑名单（`-` / `见正文` / `同上` …）并要求含动作动词——「非空」不等于「写了」；
  状态词的合法区从「标题之后」改成**结构判定**（表格数据行 ∪ 显式声明块），因为原标题之后
  恰好还有归属段与复现命令两节，那是个合法的第二出口；词汇定义改成 `<!-- 状态词表 -->` 块，
  **放行靠声明不靠位置**（标记丢了会红）；收口动作原本不可执行（三个状态全是未完成态，
  「核完改状态」改不成任何合法值），改成「核完即删行 + 结论与日期写进票/CHANGELOG」。
- **推翻审查者的一条判断**：它说 `|**V4** |`（编号格少一个空格）会静默漏网。实测这条是**绿，
  而且应该绿**——那是合法 markdown，单元格内容仍是粗体 `V4`，新断言按 `|` 切格后验格内形状，
  不依赖那个空格。记在 review 报告的「不采纳及理由」里，不照它改。
- **P2-7 落 issue**：断言只钉住一份文档，`docs/research/` 另两份零守卫、新建文档也不会红
  → `.scratch/audit-42-research-doc-coverage.md`。没在本轮做是因为它要改 `SourceTree` 的
  helper 语义（列目录失败必须抛，不能返回空集当通过），跨文件且值得单独一轮。
- **P2-8 补出口**：review 报告的命名约定此前只活在 CHANGELOG 的一句自述里，
  `AGENTS.md` 的文档分工表没有它——现在补了 `docs/research/` 与 `docs/code-review/` 两行，
  本报告即按新名落盘。
- **代价**：断言从 24 行长到 90 行，报错更具体（一律带行号与实际值）。被否决的写法：
  ① 给「正文」边界加一条「清单小节必须在文末」的位置守卫——那只是把一种脆弱换成另一种脆弱；
  ② 对整篇文档做松散编号扫描（`V1–V7` 这类区间写法会全被判成违规），实测成本高于收益，不做；
  ③ 把状态词表定义挪到清单小节里以躲开断言——那是让内容迁就断言的视野，本轮反过来把放行做成声明。
- **外部 review**：本轮为**退化路径**——`dsh` 两种调用形式都稳定报 `NO_ADAPTER`（provider
  `dimagent-oauth` 未注册），`claude`/`codex`/`opencode`/`gemini` 均未安装，故由非作者的独立
  agent 审。结论：无 P0，3 条 P1 必修（全部本轮修完）、8 条 P2（采纳 6、落 issue 1、推翻 1）。
  报告见 `docs/code-review/2026-09-24-1211-v0.0.128-fallback-subagent.md`。
  **连续两轮拿不到外部 CLI** 这件事本身没有成文降级规则，已在报告里记为未闭环。
- 测试：451 条全绿（用例数不变，断言体重写）。十处变异逐条实测：小写编号、清单行不加粗、
  状态列前插列并清空复核方式、复核方式写 `-`、状态格清空、清单之后的小节陈述状态、
  删掉声明块标记、正文用同义改写、新编号只加一侧 —— 九处红；
  第十处（编号格少一个空格）红不了且**不该红**，见上。

## [0.0.127] - 2026-09-24

### 📋 研究文档的未完成项收进带编号的清单，Qoder http hook 的白名单补全

处置上一轮外部 review 的两条 P2（报告本身随本轮入库：
`docs/code-review/2026-09-24-0130.md`）。两条都指在 v0.0.125/126 新增的研究文档上，
而且第一条的成因**本轮已经付过一次学费**。

- **P2-2：Qoder 的 http hook 片段缺关键字段 ⇒「照抄即接入」不成立。** 重新逐项核正文：
  `url` 是**唯一必填**（表格原文 `| url | Yes | URL that receives the POST |`
  ⇒ **方法固定 POST，正文里没有 `method` 这个参数**）；`headers`、`timeout` 可选；
  `allowedEnvVars` 与 `type`/`url` **同层**、值是环境变量名数组、可省略，
  而省略的行为原文明确写着 **"all are allowed if omitted"**。
  - 由此得出一条安全约束并写进 02 号票：**「复制接入命令」必须显式写白名单**。
    默认「全允许」意味着 `headers` 里任何 `${VAR}` 都能把任意环境变量插值送出去，
    而这条配置的唯一作用就是往一个端口送令牌——少写这一个字段不是少一层保险，
    是把令牌面的最小权限直接关掉。
  - 顺带暴露一条正文没写的行为：**不在白名单里的变量会怎样**（丢 header / 置空 / 整条拒绝）
    文档没说 ⇒ 新增 **V1**，P0 打通当天实测，不许按「应该会拒绝」设计。
- **P2-1：未完成项没有可跟踪的收口。** 文末新增「待核实清单」**V1–V7**，每条带
  编号 / 当前状态 / 复核方式，正文对应处写 `⇒ **V#**`。状态从原来一个笼统的说法
  拆成三种，因为它们的出路完全不同：`未找到出处`（搜过、没有）· `待逐字复核`
  （有出处线索、本轮没打开原文）· `待真机实测`（文档说了但没逐事件/逐入口验证）。
  正文里那个笼统的词全部退役——它把「没查」「没看原文」「没跑过」混成一件事，
  而这三件事的成本和归属根本不同。
- **加了一条真不变式**（不只是改文档）：断言「正文引用的编号 ↔ 清单里的编号」一一对应、
  每个编号恰好一行、状态必须落在三分类内、正文不许再出现那个笼统说法。
  这条断言第一次跑就把自己写坏的地方抓了出来（正文还剩两处「未证实」）。
- **代价与被否决的写法**：被否决 ① 只在文末加一段提醒、不写断言——上一轮已经证明散文提醒
  挡不住重复劳动（v0.0.125 就是把有正文的四家误判成排除）；② 为了让断言放行「解释这个词
  为什么被废弃」的元提及而加例外——例外会让这个词继续留在正文，改成正文彻底不用它、
  解释只留在清单小节（断言范围之外）；③ 清单写进 `.scratch/`——那里不入库，等于没有。
- **没做与遗留**：V1–V7 本身仍未核（清单就是给它们的归属，V1–V4 归 02 号票打通当天）；
  review 报告的文件名没按新约定带版本号与审查者，那是别人生成的产物、不改写；
  02 号票（`/session` 协议 + 令牌 + TTL + pid 复核）仍未实现。
- 测试：+1（450 → 451）。六处变异逐条确认会红：
  删掉清单里的 V6 行、正文引用一个清单没有的编号、清单多一条从不引用的编号、
  某行状态改成笼统说法、正文重新出现那个词、同一编号在清单里出现两行。
  本轮无源码逻辑改动，故无源码级变异。
- 外部 review：本轮改动本身是**对 review 的处置**，未再推第二轮（约定上限 2 轮，
  且这两条 P2 不属于「必修」级），处置结果即本条。

## [0.0.126] - 2026-09-23

### 🩹 更正上一版的错误排除：Qoder / Cursor / Trae / Cline 都有公开文档的 hooks 面

v0.0.125 把四家记成「本轮排除，理由：拿不到可引用正文、也没有本机实样」。**那句理由是错的**，
而且错在一个很具体的地方：我把「本机 `~/.qoder/settings.json` 里只有 `enabledPlugins`」
当成了「Qoder 没有可配置的 hooks 面」。`本机没配` 与 `官方没有` 是两个命题，
我拿前者证了后者——四家里三家（Qoder / Cursor / Trae）其实有完整正文，Cline 有入口。

- **逐家抓正文复核后的结论**（出处都写进 `docs/research/agent-lifecycle-hooks.md`）：
  - **Qoder**：`docs.qoder.com/cli/hooks` 给了 schema、**23 个事件**、stdin JSON 字段清单
    （`session_id` / `transcript_path` / `cwd` / `hook_event_name` / `permission_mode` /
    `agent_id` / `agent_type`）与 env（`QODER_PROJECT_DIR` 等）。
    最有价值的一条是 **`type: "http"`**：hook 输入以 JSON POST 到 URL，`headers` 支持
    `${ENV_VAR}` 插值并受 `allowedEnvVars` 白名单约束 ⇒ **Qoder 不需要桥接脚本**，
    一条配置就能打进 §3 的 `/session`。P0 的顺序因此重排（原首选是 Claude Code）。
  - **Cursor**：`~/.cursor/hooks.json` + `"version": 1`，事件名是 **camelCase**
    （`sessionStart` / `stop` / `preToolUse`…），与 Claude/Codex/Qoder 的 PascalCase 不同构
    ⇒ 桥接层要单独一张映射表，不能复用。
  - **Trae**：官方正文原句 **"TraeCode supports reading hook configurations from Claude Code"**
    ⇒ 可蹭 Claude 那份配置，P3 铺开时几乎不是新增工作量。但它的 `Notification` 一个事件同时
    覆盖「工具等确认」与「任务完成」⇒ **定 state 必须读载荷**，只看事件名会误报。
  - **Cline**：只核到入口（`--hooks-dir`、`CLINE_HOOKS_DIR`、项目级 `.cline/hooks/` 的
    "Lifecycle hooks"）。事件名与 stdin/stdout 契约在别的页、本轮没逐字复核 ⇒ 记「有面、
    细节待核」，**不给接入片段**——这条是刻意留白的，宁可少给一家也不写没核到的键名。
  - **Roo Code**：上一版的排除**站住了**，而且这次是我自己复核的：抓 `docs.roocode.com/sitemap.xml`，
    **510 条 URL 里 "hook" 出现 0 次** ⇒ 这是全部家里唯一一条有依据的「按现行官方文档无此能力」。
- **对设计新增一条约束**：既然 Qoder 走 http hook 直连，`/session` 就必须能直接吃
  **hook 形状的事件 JSON**（字段名与 stdin 那套一致），否则「配置即接入」这条最省的路用不上。
  这条已写进 02 号票的接口约束。
- **文档开头加了核实纪律**：`本机没配 ≠ 官方没有`；两者都不许直接进实现，
  每条结论必须落到「抓到的正文原句」或「本机实样」，两者都没有的写「未证实」。
  文档末尾的「复现这些结论的命令」补了 Roo 那条的自证命令（两条 curl）。
- **代价**：一次纯更正的发布，代码零改动；spec 第 6 节的决策表重排（Qoder 升到 P0 最省、
  Cursor/Trae 进 P1、Cline 待核、Roo 不做）。被否决的写法：只在 `.scratch` 的票里改注释、
  让已发布的 `docs/research/` 继续错着——那份文档是入库、对外、会被实现引用的，
  错着比缺失更有害。
- **没做与遗留**：Cline 的事件名与 stdin 契约未核；`claude -p` / `codex exec` /
  Qoder headless 下 hooks 是否触发三条仍未证实；Cursor「cloud agents 用不了 `~/.cursor/` 那份」
  是二手转述、本轮正文只确认到「跑仓库里的 command hooks」。
- **验证方式（本轮无代码，故无变异测试）**：每条新结论都由我**自己**抓正文复核，
  不再采信子代理摘要——上一版的错正是信了自己没复核的东西；Roo 那条给了可原地重跑的命令。
  全量测试 450 条不变、0 失败；脱敏扫描 18 条命中全部已备案、无新增。

## [0.0.125] - 2026-09-23

### 🔧 各家 hook 面核实落进正文：Codex 的 notify 是 legacy，OpenCode 给的是 session.idle

本轮不改代码，做 `.scratch/agent-selfreport` 的 01 号研究票——它挡着后面 6 张票，
而 spec 第 6 节整节写着「待核实」。产出是入库的 `docs/research/agent-lifecycle-hooks.md`。

- **为什么要单独一轮**：`docs/adr/0006` 的教训是「没核实到正文就预置字段名」，症状是
  「界面显示已送达、其实没送」。自报通道正是同一类风险：照猜的键名写完，配上去不响，
  而岛只会显示「没配」——用户分不清「没配」和「配了但不生效」。
- **三家有结论**（每条都要求二选一：官方正文原句，或本机配置文件实样）：
  - **Claude Code**：`~/.claude/settings.json` 的 `hooks.<Event>[].hooks[] = {type, command}`，
    命令从 **stdin 收 JSON**（含 `session_id`、`cwd`），`CLAUDE_PROJECT_DIR` 可读。
    本机实样交叉验证过（第三方 codegraph 2026-08-28 写入的 `UserPromptSubmit` 条目形状一致）。
  - **Codex**：**spec 原先写错了载体**。`notify` 已是 legacy——载荷**追加为最后一个 argv**、
    **stdin 被显式置空**（`legacy_notify.rs` 注释与 `Stdio::null()`）；新一代是
    `~/.codex/hooks.json`，事件名 `HOOK_EVENT_NAMES` 12 个，**与 Claude Code 同一套词汇**
    ⇒ 桥接脚本两家共用，比原计划省一套接口。另记一条坑：`HOOK_EVENT_NAMES_WITH_MATCHERS`
    只含 9 个，`Stop` / `Interrupt` 的 matcher 被忽略，别拿 matcher 过滤终态。
  - **OpenCode**：不给 hooks 而给 plugin 的 `event({event})` 总线，事件名里有
    `session.idle` 与 `session.error`——判断 ①（生命周期不能交给模型自觉）要的东西
    在这里是现成的，运行时确定发出。接入形态与 03 号票的夹具都得为此分一支。
- **一处新风险，直接改测量口径**：Codex 管理员可在 `requirements.toml` 设
  `allow_managed_hooks_only = true`，忽略用户/项目/会话层 hooks ⇒ 企业机器上自报可能
  一条都不到。已写进票里：08 号对照测量必须把「配了自报但没收到」单独计一类，
  不能算成推断错误——否则会把托管环境读成「推断不准」。
- **排除的几家给的是排除结论，不是猜测**：Qoder（本机 `~/.qoder/settings.json` 只有
  `enabledPlugins`；本会话运行时自述有 hooks，但那是**运行环境自述、不是公开文档正文**）、
  Cursor / Trae / Cline / Roo（本机无实样、官方正文本轮未取到）、DimAgent / WorkBuddy
  （自有工具，属产品决策）。「拿不到出处」与「确认无此能力」在文档里分得很清——
  前者写排除，后者才写「无此能力」。
- **代价与被否决的写法**：被否决 ① 把常见的 hook 词汇表当结论写进去（省一轮抓取，
  但正是 ADR-0006 那个坑）；② 把详细证据留在 `.scratch/` 的 spec 里——`git check-ignore`
  确认 `.scratch/` 不入库，那样的「产出」发不出去也查不到，所以正文进 `docs/research/`、
  spec 只留决策表并指向它（同一份事实写两处必然漂移）；③ 为凑「本轮有测试」写一条
  断言文档字符串的假测试——本轮没有代码改动，没有可变异的东西，硬造断言只会多一条
  永远绿的噪声。
- **没做与遗留**：`claude -p` 与 `codex exec` 下 hooks 是否**逐事件**触发仍未证实，
  文档里标成「不许当已知」，P0 打通当天真机测；Cursor / Trae / Cline / Roo 的官方正文
  本轮没取到，P3 之前不动；02 号票（`/session` 协议 + 令牌 + TTL）仍未实现——它现在
  有了明确的接口形状，但那是另一轮的事。
- **验证方式（本轮无代码，故无变异测试）**：每条结论两个独立来源交叉——官方正文/已安装
  SDK 类型定义 **加** 本机配置文件实样；文档末尾附「复现这些结论的命令」，读者可原地重跑
  而不是信我。全量测试 450 条不变、0 失败；脱敏扫描工作区 18 条命中全部已备案、无新增
  （新文档不含个人路径与任何凭据）。

## [0.0.124] - 2026-09-23

### 🔒 成本的「什么算零」收成一个出口，`—` 从此只表示没查

`TokenUsage.cost()` 用空串表示「没有金额可显示」，于是每个调用点都得自己回答
「那零怎么写」——这一决定散在 **9 处、给了 3 种答案**（`$0.00` / `—` / 整条省略），
还长出一个**第二零阈值**：月末预测用 `<= 0.001`，其余用 `> 0`。

- **新增 `TokenUsage.costText(_:zero:)`**（两个重载）。`zero:` **故意不给默认值**：
  默认值会让某个出口悄悄把「没查」写成「没花钱」。零怎么写确实是各出口的排版差异（表里要占位、
  卡上要省略），但什么算零只能有一条线。
- **11 处调用点收敛**：`status` / `tokens`（4 处）/ `top` / 审计报表（2 处）/ 用量分析报表（3 处）/
  月末预测。文本版那个重载是给 `TokenCostEstimator.resolveCost` 的产物用的——它可能带估算标记
  （`~$0.42`），**必须原样透传**：按数字重排会把 `~` 弄丢，而 `~` 是「这数是估的」唯一的痕迹。
- **可见的口径变化两处**：
  1. 审计报表与用量分析报表里，**实测零成本**从 `—` 改成 `$0.00`。`—` 被「没查」独占之后，
     一个记号不能同时表示两件事；用量分析那一行右边本来就有「已连接/未发现」列负责表达
     查没查到，费用格不该再兼一次。
  2. 月末预测在 `$0.0005` 这种档位从 `$0.00` 变成 `<$0.01`——它本来就不是零，
     `<= 0.001` 那条线是第二套阈值，删掉。
- **棘轮第一轮就抓到一处漏网**：用量分析的 Markdown 写 `—`、同一文件的 CSV 写 `$0.00`,
  同一份导出的两个格式对同一个零各说各话。加了一条结构断言禁掉散写形状
  （`> 0 ? TokenUsage.cost(`、`<= 0.001`、`!….text.isEmpty`），并把这处补成语义断言
  （同一实测零在两份导出里同形）——只靠字符串形状守不住语义。
- **代价**：报表里 `$0.00` 变多，习惯扫「有数字的格子」的人会看到更多零成本行；
  这些行本来就在，只是从前伪装成「没查」。被否决的写法：① 给 `zero:` 默认值（见上）；
  ② 把 `resolveCost().text` 换成按 `res.cost` 重排（丢 `~`）；③ 强制所有出口都写 `$0.00`
  （详情卡上「没有成本就省略这一行」是排版自由，为口径牺牲它不值）。
- **没做与遗留**：`AgentHoverTooltip` 的 `if usage.cost24h > 0`（零就省略整行）保留——
  它不产出文本；审计报表头部那行「24h Token 消耗」仍只在成本非零时附成本，同理。
  `TokenCostEstimator.resolveCost` 的 `text` 仍用空串承载零，本轮把**读侧**收成一处，
  写侧要不要也换成 `Double?` 是另一件事。
- 测试：+3（`costText` 两个重载与 `~` 透传、跨出口一致性、结构棘轮），2 处既有断言按新口径改写
  （报表零成本 `—` → `$0.00`）。七处变异逐条确认会红：零占位被吞成空串、一律写零占位、
  重新长出第二个零阈值、`~` 被抹掉、调用点退回散写、分析报表的零退回 `—`、审计报表的零退回 `—`。
  全量 450 条（447 → 450）。

## [0.0.123] - 2026-09-23

### 🔒 审计报表不再把「没取到的用量」写成「没花钱」

`report` 导出的 Markdown / CSV 里，`tokenUsage == nil` 被折成 `0`——而同一个快照在
`status --json` 里是 `null`。本机实况：Qoder 的 token 字段拿不到（README 明写着「用量列显示 `—`，
它的 token 消耗监控不了」），岛内和 `status` 都照此显示 `—`，审计报告却给它印 `0`。
一份要拿去做运维评审的报告，把「这个工具我监控不了」写成了「这个工具没花钱」。

- **Markdown 的 Token 表**：没取到的**整行**四个格子都是 `—`，取到了才印数字（`0` 就是零用量）。
  原先是 `tok24 = u.map { … } ?? "0"` 而同一行的成本列却印 `—`——一行之内两个字段对同一次
  「没取到」给出相反结论，这正是 v0.0.120/121/122 一路在修的同一族错，只是这次在报表出口。
- **CSV**：四个用量列没取到时留空字段，与同一行已有的 PID（无则空）、CPU（v0.0.121 起无窗口则空）
  对齐。原先 `String(u?.tokens24h ?? 0)` 让电子表格把「监控不了」直接求和成「没花钱」。
- **合计要说清覆盖面**：头部「24h Token 消耗」下面加一行「其中 N 个智能体本轮没取到用量
  （不计入上面的合计）」——合计只累加取到的那些，这是求和的固有口径，但不写出来读者无从核对。
- **顺带修一处文档漂移**：三处代码注释引用「CONTEXT.md 的『源缺失不得当作零活动』」，
  而 CONTEXT.md 里从没写过这条——`**Token 数据覆盖**` 那一节原先只管分析页。现在把它升格成
  覆盖全部出口的口径条目，并把被引用的那句话作为条名放进去（引用不存在的规则比没有规则更糟：
  它会让下一轮自动化以为已经有人守着了）。
- **代价**：报表里 `—` 变多，习惯扫数字的人要先读标记说明。被否决的写法：
  ① 给没取到的行印 `<无源>` 之类第三种标记——同一份数据在 JSON 是 `null`、在岛内是 `—`，
  报表再造一种记号就是第四份口径；② 干脆不列没取到的行——「监控不了」恰恰是运维要看见的事实，
  隐行等于把它藏起来；③ 在导出层调 `refreshTokenUsageSync()` 补取一次——`report` 命令本来就已经
  取了（`refreshUsage: true`），取不到是源的问题，不是没取的问题，补取只是把谎报延后。
- **没做与遗留**：成本列的 `—` 仍然同时表示「测到是零」和「没取到」——整行 `—` 才是没取到，
  单看成本格分辨不出。要分开就得引入 `$0.00` 这一支，而 `> 0 ? cost : "$0.00"` 这个形状
  在 CLI/UI 里已经散落 8 处，先收敛成单一 helper 再谈（独立一项）。
  `AgentSnapshot.tokenUsage` 的 nil 语义没变，本轮只对齐出口。
- 测试：+1 条（报表用量列的三态：没取到 / 测到的零 / 实测非零，Markdown 与 CSV 各钉一侧，
  并钉住 24h 与累计两列不得对调、合计说明里的个数不得用总行数）。九处变异逐条确认会红：
  没取到的行印 0、两列接错、CSV 折成 0、合计不再说明、说明里的个数用总行数、非零成本被吞、
  成本两列对调、CSV 成本折成 0、测到的零成本改印空。全量 447 条（446 → 447）。

## [0.0.122] - 2026-09-23

### 🔒 一次读不到的目录不再让「刚刚还在写」和「0 个会话」同时成立

`FileMonitor.runScan` 里同一个目录的两次失败判定走岔了：活动日期走「宽限保留」，
活跃会话数走「无条件取 `r.activeSessions`」——而扫描失败时那个值恒 0。

- **症状**：会话目录一次 EACCES / 枚举器建不起来，同一份快照给出两个相反的结论——
  「最近活动：刚刚」与「会话：0 个」。下游跟着遭殃：`AgentObservability` 正是拿
  `activeSessions == 0` 判「无本地明细」（`noLocalData`），`status --json` / `doctor --json`
  印 `"activeSessions": 0`，岛上的「会话」行消失。这是 v0.0.120/121 那条纪律的漏网字段：
  把「没看到」讲成「没有」。
- **修法**：让计数跟着活动日期走**同一条分支**。没看完 → 保留上一份；扫完了 → 照实取。
  条件写成 `unseen = r.newest == nil && (rootDate == nil || r.scanFailed)`，与日期那三支
  一一对应，不留「一半保留一半清零」的缝。
- **一个容易搞错的细节**：「扫完但没有信号文件」**不等于** 0 个会话。一个空着的会话目录
  仍然算一个活跃会话（计数按顶层目录的活跃度算，不看文件），所以那一支必须继续取
  `r.activeSessions`。本轮第一版就是在这里改错的（把清零扩到了这一支），
  被既有的「窗口修改后下一次扫描采用新口径」用例当场拦下——那条测试因此是这处口径的守门人。
- **保留必须有终点**：连续 3 趟仍读不到会走终态清零（R33/F3 既有语义），但那条收尾原先只清
  活动/文件/快跳过键，不清会话数；而会话数是整表替换写回的（`sessionCounts = freshCounts.filter…`）。
  于是两件事必须同时成立：终态里加上清计数，且**排在整表替换之后**——先清就会被原样写回去。
  没有这条终点，「没看到」攒够 3 趟就冻成了「永远有一个会话」。
- **代价**：一次真失败的扫描会多保留一份旧计数（最长 3 趟）。与活动日期同一条权衡，
  且方向保守——宁可多留一会「可能还在干活」，不把活着的 Agent 判成空闲。
  被否决的写法：给 `activeSessions` 也做三态（`Int?` 一路传到 `AgentSnapshot.activeSessions`
  与全部展示点）——本轮的错不在缺一个 nil，而在两个字段口径不一致；先把一致性钉住，
  要不要显式「未测」是另一个决定，且它会让 `sessionCount(for:)` 的多目录求和变成可空求和。
- **没做与遗留**：`ProcessSnapshot.Entry.cpuPercent` 仍是非可空（新进程第一拍算 0%，一拍后自愈），
  与上一版留的同一条；`AgentSnapshot.activeSessions` 也仍是非可空 `Int`，本轮只保证它
  「跟活动日期同口径」，没保证它区分「从未扫到」与「扫到 0 个」。
- 测试：+1 条（连续缺失到终态时计数跟着收尾，含宽限期内不许提前清零），
  既有那条「读不到保留活动」加了会话数断言（同一份快照两个字段必须同口径）。
  六处变异逐条确认会红：回到无条件取计数、没看完直接清零、终态不清会话数、
  终态收尾排在整表替换之前、扫完无信号就写 0、终态阈值改成永不触发。
  全量 446 条（445 → 446）。

## [0.0.121] - 2026-09-23

### 🔒 CPU 也要有窗口才谈得上「测到」：一次性入口不再印 0.0%

上一版修的是死锁那一维，收尾时点名了同一家族的另一处：`status` 只采一拍，而 CPU% 是
**差分量**——没有前一拍就没有测量值，可它一直在印 `0.0%`。一台正在烧 CPU 的机器，
在 `status` 与审计报告里读起来是「空闲」。

- **`AgentSnapshot.cpuPercent` 改成 `Double?`**，`nil` = 本拍没有差分窗口。判定放在引擎里
  一处：本拍之前有没有过一拍（`lastSampleAt`）、以及有没有真正匹配到的进程条目。
  内部的状态判定照旧用 0 参与计算（`hasHighCpu` 那类近似值不改口径），**只有对外公布的那一份变可空**——
  谎报发生在展示与契约上，判定近似是另一回事，两件事不混在一个字段里。
- **占位条目不再算成实测**。`ProcessMatcher` 在「bundle 命中但进程名没匹配」时返回
  `pid = -1` 的占位条目，它的 CPU 恒 0；这条 0 的含义是「没看着这个进程」，不是「它不占 CPU」。
  现在这类档案公布 `nil`。原有那条断言「CPU 合计为 0」的回归测试跟着改成断言「没测」，
  并补采第二拍排除「首拍」这一层——否则它测的其实是窗口而不是占位。
- **健康评分把 CPU 并入「未评估维度」**：与死锁同一套处理——不凭空扣分（没测不等于有病），
  但作废「健康」评级（全清结论不能包含没查的维度），且已有内存/CPU 扣分时不降级（严重度不能被盖掉）。
  两维都没测时把两维都点出来（`死锁/僵卡 与 CPU 持续负荷本轮未评估`）。
  措辞仍然不带阈值数字，那段时长只有 `AnomalyScanGates.hungNotEvaluatedNote` 一个来源。
- **各展示点改口**：`status` 表 CPU 列印 `—`、`status --json` / `report --format json` 给 `null`、
  审计报告 Markdown 印 `—（本拍无差分窗口）`、CSV 留空、`--probe` 双采有值则印真值。
  `doctor` 走双采，CPU 是实测（它的「观测不全」只剩死锁那一维，那是结构性的）。
- **代价**：`status` 的 CPU 列对很多人来说从「一直 0.0%」变成「一直 —」。这是要的——
  前者会让人以为机器闲着。要数值有 `top`（持续）与 `doctor`（双采，多等 1.5s）两个入口，
  README 的限制条目里写明了。被否决的写法：给 `status` 也加双采（每次调用多付 1.5s，
  而它是被 Raycast / 脚本按次调的）；以及给 `LiveSampler` 预热 provider 基线（只花 0.35s 就能出数，
  但那会把 0.35s 的瞬时尖峰喂进 `hasHighCpu` 状态判定，等于用改变状态语义换一列好看）。
- **没做与遗留**：`ProcessSnapshot.Entry.cpuPercent` 仍是非可空的 `Double`——真正区分「首次见到这个 pid」
  与「确实是 0」的逻辑在 `CpuCache.update` 里，本轮只在引擎层收口，所以**一个新启动的进程在它的
  第二拍仍会被算作 0%**（一拍后自愈）。跨睡眠断点后的第一拍 CPU 是按整段睡眠窗口折算的均值，
  本轮没动（它是有窗口的，只是窗口很长）。
- 测试：+3（引擎差分窗口、健康分对 CPU 未评估的处理、JSON 的 null 口径）+1 条结构棘轮
  （展示点不许把快照的 `cpuPercent` 直接喂给格式化函数，`?? 0` 那种接回去的写法也拦），
  1 条既有断言按新口径改写。七处变异逐条确认会红：首拍公布利用率、占位条目算实测、整体不判窗口、
  没测被当成满载扣分、CPU 维度不进未评估名单、实测高 CPU 不扣分、JSON 把没测洗成 0。
  全量 445 条（441 → 445）。

## [0.0.120] - 2026-09-23

### 🔒 死锁判定改成三态：「本轮没测」不再被算成「不卡死」

上一版收尾时点名的那一处遗留：异常扫描的闸门收敛了，健康评分这条链路还在把「没看到」讲成「没有」。

- **`isHung` 从 `Bool` 改成三态**（`true` 卡死 / `false` 判过且清白 / `nil` 本轮判不出）。
  判据本身是时间性的——「CPU 连续超阈值达 N 分钟」，所以**有没有资格判**取决于对这个进程
  连续观测了多久。这份资格现在由引擎按 profile 记着并写进快照：进程一退出就作废，跨睡眠
  断点（墙钟在走而没采样）也作废。默认值是 `nil`——漏传只会得到「没测」，不会得到一个假的「没有」。
- **为什么不是让调用方声明**。v0.0.119 的 `sustainedObservation:` 是一个**可以说谎的参数**：
  新加的一次性入口传 `true` 照样编译得过、跑得起来继续谎报，而症状仍是静默的。本轮把它整个删掉，
  资格只能从数据里读出来，谎报这条路不存在了。棘轮换成一条结构断言：源码里再出现那个参数即失败。
- **健康评分多了一个评级「观测不全」**。死锁维度是 `nil` 时不许说「健康」——但**不凭空扣分**
  （没测不等于有病），也**不覆盖已有的严重度**（CPU/内存已经扣分时那两级正在喊话，
  换成「观测不全」等于把真问题盖掉）。被否决的写法：给未评估扣 50 分，那是凭空造病症。
- **对外契约跟着改**：`status --json` / `report --format json` 的 `isHung` 走键缺失（jq 读到
  `null`），与 `tokens24h` 的「nil 是没查、0 是查了确实为零」同一条口径；`doctor --json` 补
  `healthGrade`——只给一个 100 分，脚本分辨不出「清白」和「没查」。
- **测试逼出来的一个真 bug**：`resetAllTracking()`（睡眠/挂起断点）清了 `highCpuSince`
  却没清观测资格。后果是醒来的第一拍拿着**旧窗口给的资格** + **刚被清空的计数**判出一句
  「不卡死」——比原来的错更隐蔽。补的断言钉住两侧：断点后不许有结论，重新攒满窗口仍能判死锁。
- **顺带收一处措辞分叉**：审计报告曾自己 switch 出「需关注 / 预警」，而 `status --json` 与岛内
  详情卡说「需留意 / 异常」——同一个 grade 两份话，读两份输出的人没法核对。现在报告直接用
  `HealthGrade.rawValue`。
- **没做与遗留**：`status` 是单拍采样，CPU 是差分量所以首拍恒 0，表里一个正在跑的 Agent 会印
  `0.0%`——那是同一类「没测讲成 0」，本轮只动死锁维度，留作下一轮。岛内详情卡「观测不全」的
  配色没在真机走查到：`open agent <id>` 深链会把面板收掉，抓不到那张卡（列表、徽标、状态环都
  走查过，无异常）。
- 测试：+4（健康分三态、引擎观测资格含重启与断点、JSON 的 null 口径、评级措辞单一来源），
  结构断言改写为「观测资格不许由调用方自报」。八处变异逐条确认会红：资格阈值改 0、去掉降级、
  无条件降级（真清白被冤枉）、降级盖过严重评级、有快照就算有资格、断点漏清资格、JSON 把没测
  洗成 false、报告另译一份评级。全量 441 条（437 → 441）。

## [0.0.119] - 2026-09-23

### 🔒 异常扫描的两道闸门 UI 与 CLI 合成一份；发版链路上了一道脱敏门禁

两件独立的事，同一个来源：**该发生的步骤没发生**——扫描没做、闸门没传。

- **`check` 的「状态健康」以前是句空话**。异常扫描有两道前置闸门：死锁集（`hungAgentIDs`）
  与孤儿佐证集（`recentlyActiveProfileIDs`）。它们的算法只写在灵动岛工作台里，而
  `AgentCleaner.scanAnomalies` 把两个参数都给了默认空集，于是 CLI 的 `check` / `clean` /
  `top [c]` 三处一个都没传，症状是两条方向相反的错：死锁分支永不成立（列表里从来没有
  死锁行，末尾照印「未检测到任何死锁」），孤儿佐证永不生效（launchd 托管、仍在写会话的
  活进程被报成孤儿，还提示用户去 `clean` 它）。现在两道闸门收进 `AnomalyScanGates`，
  UI 与 CLI 同一份实现；**`scanAnomalies` 的默认值删掉了**——这类参数漏传的症状不是崩溃
  而是静默失效，那就得让它编译不过。另加一条结构断言：一次性入口不许自报「持续观测」，
  持续入口不许自废成一次性。
- **一次性扫描本来就测不出死锁，现在它自己会说**。`isHung` 的定义是「CPU 连续超过阈值
  达 5 分钟」，只有持续采样的引擎攒得出那段时长；`check` 哪怕双采也只有 1.5 秒。
  所以这种轮次显式作废死锁结论，印「本次未评估」并指出该去哪看，而不是报「没有」。
  阈值文案从 `EngineConfig` 读，不写死数字。`--json` 的 stdout 保持可被 jq 直接吃的数组，
  这句话走 stderr——空数组冒充「一切正常」而脚本读不出缺了一维，是同一类谎报。
- **CLI 冷启动第一拍的 CPU 恒 0** 也被顺手修了：CPU% 是两次快照的差分量，而死锁规则要求
  单条 `> 10%`，用没预热的 provider 去扫等于把死锁行全漏掉。工作台早就为此预热两拍，
  CLI 三个入口现在走同一个 `AgentCleaner.warmedForOneShot()`。
- **发版前置了一道密钥与个人信息扫描**（`scripts/scan-secrets.sh`，`release.sh` 与
  pre-commit 钩子都会跑）。规则做成**棘轮**而不是「零命中」：测试夹具与调研文档里合法地
  存在假口令、占位邮箱，硬要清零只会让人把规则调瞎，所以命中必须由 (规则, 文件, 行内容)
  三元组在 baseline 里备过案，新增即失败；`cred_prefix` / `private_key` 两类是绝对零命中，
  不许进 baseline。命中内容打码后才打印。扫描分两档：工作区 + 全部 git 对象（发版走这条），
  因为删掉文件并不会让已经 commit 的东西消失。
  **第一次跑就抓到一处真泄漏**：`docs/research/qoder-monitoring.md` 粘了本机实况，里面有
  `/Users/<开发者账号>/…`。当前版本已改占位；旧版本仍在历史里，改写公开历史要 force push
  已发布的全部 tag，代价大于收益，故记入豁免清单并写明理由。
- **README 归位成产品说明**。它在若干处滑成了更新记录（「已计入分工具占比」「不会再让 CLI
  崩掉」这类逐版话术），且「已知限制」三条已过期：多屏早就按光标选屏、四边都能贴；
  图标按钮已统一走 `hitTargetHeight()`（补的是命中高度，画出来的圆底仍是小的，这点如实留着）；
  token 明细索引早就按 70 天窗口折叠，不再是「只增不减」。同时加一条会红的断言守着文体：
  README 里出现版本号、`此前/原先/曾经/不再/新增了` 或带测量数字的解释即失败。
  被否决的写法：只把规矩写进 AGENTS.md——本项目里靠自觉的约定漂移过不止一次。
- **门禁脚本自己被 `.gitignore` 挡在库外**（v0.0.119 发完才发现）。那条 `*secrets*` 本意是
  拦住凭据文件，顺手把 `scripts/scan-secrets.sh` 与两份清单也拦了：`git add` 不报错，文件
  就是不在提交里。后果不只是少三个文件——门禁从此不随克隆传播，别人和下一轮自动化拿到的
  仓库没有这道闸。已补 `!scripts/scan-secrets.sh` 等三条例外。同一条规则还造成过一次更隐蔽的
  假绿：有一遍报「0 条命中」，而被扫的文件集里根本没有扫描器自己那批文件——**门禁扫不到自己，
  等于门禁没有**。
- **没做与遗留**：`status` / `doctor` 走一次性采样，`isHung` 同样恒 false，本轮只收敛了
  异常扫描链路，健康评分那一处仍会把「没测」算成「不卡死」，留作下一轮。另一件需要人拍板
  的：**v0.0.81..v0.0.118 共 38 个版本只推了 commit，tag 与 GitHub Release 都没建**
  （远端 release 停在 v0.0.80）。回填是对外可见动作，没有自动执行。
- 测试：+6（闸门作废语义、佐证窗口临界值、一次性端到端仍拦住活进程、措辞取自配置、
  调用点结构断言、README 文体）。六处变异逐条确认会红：忽略观测资格、删孤儿佐证 guard、
  去掉窗口上限、写死阈值数字、CLI 冒充持续观测、死锁 CPU 临界值。全量 437 条。

## [0.0.118] - 2026-09-22

### 📡 Xiaomi MiMo 接上真实数据，并补回 OpenCode 这一族从没拿到的结构化状态

上一版留的「等真实会话再核对」现在有数据了（本机 `mimocode.db`：2 会话 / 4 消息 / 10 片段）。
逐字段核对结论：`message.data` 与 OpenCode 同形——assistant 行带
`tokens{input,output,reasoning,cache}`、`cost`、`time{created,completed}`、`finish`。

- **用量明细接入**：`TokenUsageMonitor` 的 OpenCode 侧原先是**单点**（一个 `openCodeDB` 字段、
  一份戳、一份「源已消失」计数）。现在改成按注册表档案声明的**方言库列表**，每个源各自记戳、
  各自判缺失、各自用自己的档案 id 记账——否则第二个产品的量会在分工具占比里挂到
  `opencode` 名下。下钻查询（按模型、按会话）也不再按 id 硬列，改由档案解析。
  实测：`agentisland tokens` 出现 `mimocode 42.9k / 累计 42.9k`，正是真库里那条会话的 42,851。
- **这一族从没拿到的状态语义**：`inspectOpenCodeDatabase` 原先只把 `part.data` 丢给通用 JSONL
  检测器，而 part 是 `text` / `reasoning` / `step-start` / `step-finish` 这类**内容片段**，
  没有 message 信封——于是 OpenCode 与 MiMo 的「工作中 / 已完成」一直只是 mtime 与 CPU 近似。
  现在终态从 `message` 表读：assistant 行有 `time.completed` = 本轮封口（15 分钟内算已完成），
  只有 `time.created` = 正在生成（5 分钟保质期），最新一条是 user = 等模型开口。
  `part` 尾窗检测降为回落路径。崩溃留下的未完成行不会把岛永久钉在工作态（保质期就是为它设的）。
- **没做的部分说清楚**：这一族的「等你确认」在 `permission` 表里，本机 0 行、形状无从核对，
  所以不猜、不接。MiMo 目前跑的是 `step-5-preview`（provider `dimagent-stepfun`），
  库里 `cost` 为 0，费用列如实显示 `$0.00`。
- 测试：+3（方言状态纯函数、真表形状经 probe 走通、两个同构库各算各的且缺失判定独立）。
  五处变异逐条确认会红：不查 message 表、放宽完成保质期、放宽在途保质期、只遍历第一个源、
  以及把源循环截断。431 个用例全绿。

## [0.0.117] - 2026-09-22


### 📅 两处「同一个数两套算法」对齐：周/月柱子按日历日切桶，预算明确是滚动 24 小时

上一版留的两项待口径决策，这次一并定下来（口径与取舍记进
`docs/adr/0008-calendar-day-buckets-and-rolling-24h-budget.md`，术语进 `CONTEXT.md`）。

- **周/月两档的柱子标签是 `M/d`，分桶却是「此刻往回推」**：桶边界落在打开页面的钟点上，
  一根标着 9/18 的柱子实际覆盖 9/17 21:37 → 9/18 03:37，昨天深夜的用量被算进今天那一格。
  现在 `.week`/`.month` 的窗口起点对齐当地 00:00，`windowStart` / `bucketIndex` /
  `bucketStart` / `previousWindowStart`（环比分母）四处共用同一套算法，空态图表也走它。
  `.day` **保持滚动**——它的刻度带时分，本来没说谎，而「最近 24 小时」正是卡片那个数。
- **预算告警天天午夜重响**：`TokenBudgetTracker` 按 `component(.day)` 变化把告警级别归零，
  可它度量的 `used24h` 是滚动窗口——23:50 报过 90%，00:10 那段窗口几乎没滚动掉任何用量，
  于是同一个越线再报一次。删掉这条与口径不匹配的重置：重新武装只由回落给（<75% 滞回），
  告警次数与「越线次数」对齐。设置页标题与选项改成「滚动 24 小时 / tokens / 24h」，
  免得用户以为跨午夜会清零。
- **读取下界不再自己写第二套**：`timeline()` 的 SQL 下界原先是 `now - 2 × duration`，
  现在取 `previousWindowStart`——对齐后它比原来晚最多一天，即少读一段本来就用不上的数据。
- 明确接受的边界：日对齐后最后一格覆盖整天，右边缘会留一小段空柱（`bucketCount` 固定是
  窄卡片排版的前提）；DST 切换日某一格的墙钟跨度会是 5/7 小时（本机时区无夏令时，未实测）。
- 测试：三条新用例，逐条做过变异——把 `windowStart` 退回滚动，密度用例会报
  「week 应覆盖 7 个日历天，实得 8」，分桶用例会报「两笔挤进一根柱子」；把自然日重置加回去，
  预算用例会在午夜那一拍抓到重复告警。428 个用例全绿。

## [0.0.116] - 2026-09-22


### 🧹 批量清理的结论改由复核给：发了信号不等于杀掉了

- **一键清理此前当场宣布成功**。`cleanAnomalies` 只看「真正发出过信号的 pid 数」，就发
  「已安全清理 N 个异常进程 … 系统资源已就绪」的完成横幅（还配提示音）。而忽略 SIGTERM 的
  死锁进程收到信号也不会消失——同一次操作里，工作台自己 1.2s 后的复核会说「有 N 个进程未能
  终止」，两条结论互相矛盾，且更响的那条在说谎。现在信号阶段只记账，结论等
  `terminationProbe` 复核（0.8s，与单条终止同口径）：全部退出才配 `completed`，
  有残留发 `attention` 并点名 PID。
- **「回收内存」只统计确认退出的进程**。原先按候选内存求和，等于报一个必然偏大的数。
- **CLI 同口径**：`clean` 阻塞 0.8s 走同一个复核，输出改成「已确认退出 / 仍在运行 /
  回收内存（只算确认退出者）」；`clean --json` 的 `success` 与 `killedPids` 现在真的表示
  确认退出，新增 `unconfirmedPids` 表示发了信号仍在跑的。`top` 里的批量清理键同样改法。
- 复核窗口 0.8s 从引擎常量与 CLI 各写一遍，收敛成 `TerminationRecheck.delay` 单点。
- 顺带修正 `ActivityEngine` 里一句站不住脚的注释：它说「批量清理那条路早就这么做了」，
  事实是只有工作台的列表复核做了，引擎横幅没有——这正是本次要补的那条路。
- 测试：真实可终止进程用例改成「发信号时无结论 → 复核后才 completed」；新增部分失败用例
  （探针注入「一个杀不掉」，真进程造不出这一支）。三处变异逐条确认会红：提前发完成横幅、
  把仍在运行的内存算进回收、部分失败误发 completed。426 个用例全绿。

## [0.0.115] - 2026-09-22


### 📡 新增 Xiaomi MiMo（MiMo Code）识别，并把 OpenCode 方言改成按档案声明路由

- 注册表新增 `mimocode` 档案：bundle id `com.xiaomi.mimo.desktop`、进程名 `Xiaomi MiMo` 与
  `mimocode`（引擎进程，CPU 跨条目求和，漏掉会表现为「在跑任务却一路 0%」）、Electron 的
  GPU/渲染/网络 helper 与 crashpad 按路径排除，会话数据根只监控 `~/.local/share/mimocode`
  ——`Library/Application Support/Xiaomi MiMo` 是 Chromium 用户数据目录，空闲也在写，
  进监控就把「开着窗」报成「在干活」（Antigravity 的 R37 同一课）。
- **动作探测与实时流水不再按 agent id 硬列 `opencode`**：改为看档案声明的
  `sessionDatabase.schema == .openCode`。MiMo Code 的库与 OpenCode 同形
  （`session` / `message` / `part`，`part.data` 存 JSON part），复用即可；按 id 列的话
  接一个 fork 就会漏掉一两处，症状是「卡片在、当前动作空、流水一片空白」且没有任何报错。
  `fetchOpenCodeEvents` 随之参数化 `agentId`，事件归属不再写死。
- 本机实测：新卡片 📡 Xiaomi MiMo 命中主进程 pid、待机、CPU 0%；doctor 给的是
  「无本地明细：读不到会话与用量」——库里确实 0 条会话，不是探测瞎了。
- **用量明细刻意还没接**：Token 侧现在是一个 `openCodeDB` 单点，接第二个同方言的库要把它
  改成按档案列表；而 MiMo 的 `part.data` usage 字段形状在本机还没有一次真实编码会话可核对。
  先不猜（README 的 Token 口径一节明确写了「状态已接、用量未跑通」）。
- 测试：档案契约（bundle id、数据根、方言、库路径）、进程匹配（主进程 + 引擎进程进、四条
  helper 出）、以及「不许再按 id 硬分派」的结构哨兵。四处变异逐条确认新断言会红
  （删 `mimocode` 进程名、把方言换成 `statusIndex`、把流水路由改回按 id）。425 个用例全绿。

## [0.0.114] - 2026-09-22

### 🔬 测试可信度：把七处「永远绿的灯」换成真的会亮的

上一轮修的是注释与代码相反；这一轮修的是断言与代码相反——它们看着在守什么，实际什么都守不住。

**逐条改造**
- **Antigravity 统一入口分派**：旧写法拿用户机器上真实的 brain 目录，把同一个函数按 3s 定位
  缓存的前后两拍互相比——没装 Antigravity 就是两个 `.none` 走 `break`，零断言通过；装了也只
  证明「缓存与直读一致」，标题里的「防通用检测器误报」从未被执行。改为临时 brain 夹具（真实
  目录布局）：专有轨必须给出 `antigravity-step-*` 指纹与文案，同一份文件换成语义轨不得给出，
  超过 24h 的会话不得采信。
- **24h 采信上限要拿「未答复的提问」才测得出来**：解析器对 `ask_question` 本身不设年龄上限
  （等待确认可以持续很久），probe 那行 24h 是唯一天花板；换成别的内容，解析器自己的 15 分钟
  保质期会先把信号抹掉，测的就不是这行了。所以补一条 23h 的对照，证明那条 nil 来自时限。
- **「有信号才挂载在途上下文」**：旧断言是构造一个 `AgentSnapshot` 再读回它自己的字段——纯
  往返，永远不会失败。真正会坏的是引擎里 `sessionSignal == nil ? [] : ctx…` 那三行（解析器
  无条件交出上下文，信号过期后卡片会继续挂着上一轮的「1 子任务」）。改为驱动引擎两拍，
  保质期前后各断一次。
- **下钻 seam 记到实参**：测试替身以前只记方法名，`agentId` 被写死、时间范围被换成默认值都
  照样全绿；现在记 `timeline:week`、`sessions:dim/gpt-5`。
- **落盘键名与枚举 rawValue 钉成磁盘契约**：改掉三处 `suite.set(x)` 后 `suite.get == x` 的自证
  断言（那测的是 UserDefaults，生产代码一行都没参与）。这些字符串已经写在用户机器的
  `~/Library/Preferences` 里，改个名不会有任何编译错误，只会让老用户这项设置读不到然后静默
  回默认。清单与 `SettingKey` 的声明逐字比对——新增键不补进来就被点名。
- **结构断言不再静默跳过**：全仓 5 处「扫不到就 `return`/返回 `[]`」收进 `SourceTree`
  （`requireSourceTexts` 缺文件即抛），顺带删掉 3 份重复实现与 2 个死函数；版本哨兵不再因
  cwd 不同而 skip（发版闸口从别的目录跑就等于抬起）；`CardRoute` 与展开高度签名两条漂移
  哨兵在函数/枚举改名时报错而不是绿灯。
- **口径子集断言换成三档真集合**：只有一个元素时「可见集 ⊂ 总集」恒成立。现在总集 3、
  可见 2、看板 1，并要求两处都是**真**子集。
- **结构断言的匹配面剥掉整行注释**（`SourceTree.codeOnly`）：「不许再出现 X」会被一句解释 X
  的注释冤枉，「必须提供 X」会被一条写着 X 的注释蒙过——两种失败都不该发生。
- **时间轴空态钉住窗口起点与桶宽**（day 1h / week 6h / month 1d）：横轴按 points 画，起点或
  桶宽错了就是「报表少画半天」，而 `sources` 为空时这条以前一句断言都没有。

**验证**：4 处变异逐条确认新断言会红（去掉「无信号即丢弃上下文」、去掉 24h 上限、改一个
`SettingKey` 名、把空时间线起点挪半格）；另把 3 个 UI 源文件移出仓库，确认结构断言从
「静默跳过」变成 7 条齐声报错。用例数 421 → 422。

## [0.0.113] - 2026-09-22

### 🔬 三个层面各扫一遍：清理结论要复核、注释不许说假话、结构断言不许静默跳过

**行为层（真 bug）**
- **终止后不再当场宣布「系统资源已释放」**。单条清理路径发完信号就发布完成事件，而批量
  那条早就按 `ProcessTerminator.isAlive` 复核了——两条路口径不一致。现在结论由复核给：
  0.8s（SIGTERM 优雅期 + SIGKILL 兜底落地）后探活，退出才发「进程已终止」，仍在则发
  「收到终止信号后仍在运行」并明确这次没有宣告成功。异常列表变空**不算**复核。
- **僵尸进程不算存活**（新暴露的问题，来自上面这条的测试）：`kill(pid,0)` 对僵尸照样返回
  0，于是「父进程还没 wait 回收」会被复核判成杀不掉——僵尸既不占 CPU 也不占内存，那是
  一句永远不消失的假警报。实测 `proc_pidinfo(PROC_PIDTBSDINFO)` 对僵尸直接返回 0，
  只能走 `sysctl(KERN_PROC_PID)` 读 `p_stat == SZOMB`。
- 复核探针是注入点：「收到信号又杀不掉」这一支用真进程造不出来（SIGKILL 兜底一定带走）。
  僵尸那条用例则造真僵尸（`posix_spawn` 后不 waitpid），并先断言 `kill(0)` 仍成功——
  否则进程被回收的话，用例会在「根本没有僵尸」的前提下假通过。

**注释层（说假话的文档）**
逐条核到代码后修掉 12 处，其中 5 处是**与代码相反的断言**：
`LocalEventServer` 声称 accept 后按本地端点复核（没有，macOS 13 那条路端口仍局域网可达，
只 NSLog 一条警告）；`ActivityEngine`/ADR-0001 声称 FileMonitor 写回是「单调 merge、
只进不退」（实际扫描成功即替换，允许自然变旧——不退的话一次历史写入会永久判成工作中）；
`DiagnosticsSnapshot` 声称三处共用（深链那处已改成只导航，实为两处）；
`DimUsageSQL.netTokens` 声称汇总/模型/会话三处共用（实为两处）；
`Theme` 让人去 `updateChrome` 找阴影开关（在 `setupPanel`）。另有 3 处指向不存在的
用例名/ADR、2 处方言与结论枚举清单漏项、3 处 profile 数量写死过期。

**测试层（可信度）**
- 结构断言不再静默跳过：新增 `SourceTree`，扫不到源码树就**抛错**而不是 `return`
  （守卫关掉还不报警是本仓 #32 那轮的老账）。触觉那条用例顺带修掉一个真洞——它用
  `"Sources/AgentIsland/" + 文件名` 的**相对**路径读文件，cwd 不是仓库根时读不到就直接
  return，把整段断言跳过；现在从 /tmp 跑测试同样能变红（已验证）。
- 删掉 4 个**重复注册**的用例（RemoteNotifyTests 里四对逐字节相同的块）：它们让套件数
  虚高 4，且失败时看不出是哪一条。
- 两条恒真断言改成能失败的：`healthScore >= 0`（评分本就钳在 0...100）改成与原值比对；
  「空数组持久化」那条不只是恒真——它写的是 **`UserDefaults.standard` 并在 defer 里
  removeObject**，跑一次测试就把开发机上真实的启停集合清掉了，改成走 store + 独立 suite。

套件 425 → 421（去掉 4 条重复），新增 2 条真用例。剩余 7 处恒真/自我往返断言已带
file:line 记进遗留清单。

## [0.0.112] - 2026-09-22

### 🧪 审计第五批：采集根读不了时，不再报「已发现明细源」

`StructuredTokenUsageIndex` 只要根目录存在就把该工具记进 `availableToolIds`，而分析页拿它
区分「有明细源」与「没接入」。根在、整棵读不了（权限、卷没挂全）时，界面会显示
「有明细源 · 0 token」——把「没看到」讲成「真的没用量」，正是 `CONTEXT.md`
「Token 数据覆盖」这条口径反对的形态。

现在改为**真的看过目录**才算发现：挂 `errorHandler` 收集中断，只有「没出错」或
「确实看到了 jsonl 文件」才置真，否则落一条 AppLog 说明是哪个采集根读不了。
这里踩到的是与 v0.0.109 目录扫描同一个坑：`FileManager.enumerator` 对权限失败
**不返回 nil**，它照样给你一个枚举器，错误只在 `errorHandler` 里出现——所以只补
`guard let` 那一支等于补了条走不到的路。

用例真造一个 0100（能 stat 不能枚举）的采集根，验证「读不到 ⇒ 不算已发现」，
并验证恢复权限后重新算已发现（防止反向过度修正成「永远不可用」）。变异验证：
把条件改回无条件置真，用例立刻变红。套件 422 → 423。

## [0.0.111] - 2026-09-22

### 🔌 审计第四批：库被写锁挡住不再当成「没有确认请求」；不认识的通道名不再静默换轨

- **`sqlite3_step` 的错误码此前被当成「库里没行」**。三个会话查询点都只认 `SQLITE_ROW`，
  撞上 `SQLITE_BUSY`（对方**正在写库**的那一拍最容易撞）就正常收尾、返回「没有信号」。
  后果是正在等确认的卡片显示成待机——恰好在最需要提醒的那一刻失灵，且一条证据都不留。
  `prepareFailed` 抓的是「结构变了」，抓不到这一类，所以新增 `stepFailed`：
  只在「一行都没拿到且不是正常收尾」时报，已经拿到行就照常出信号（中断丢的是更旧的行）。
  用例用第二个连接持 `BEGIN EXCLUSIVE` 真造出 BUSY，并验证锁释放后自愈
  （故障证据不能永久挂着）。
- **存档里的通道名不认识时，回落不再是「当没发生过」**。版本回退、手改 plist、将来加新通道
  后降级都会留下不认识的 raw；静默回落成 ntfy 的真正后果不是显示错，而是**写错**：用户在
  界面上敲的每个字符都进 ntfy 的键，他原来的配置还躺在另一个键下，两份并存且无人说明。
  现在 `loadKindDetailed` 把原文一并回吐，设置页在警告条里明说「下面显示与写入的都是
  ntfy 的键，原通道配置没被动过，但也不会被读到」，并落一条 AppLog。

两条各做变异验证（去掉上报、把原文改成 nil）都立刻变红。套件 420 → 422。

## [0.0.110] - 2026-09-21

### 📋 审计第三批：坏掉的远程通知存档不再静默关掉外发，会话列表不再把「只看了前 200」讲成「一共 200」

- **远程通知的存档解码失败此前一声不响**：键在、解不开 ⇒ 走的是「回落默认值」，而策略的
  默认值就是总开关关。于是外发**静默停止**、零日志，设置页还显示成「你没配置」——用户
  以为功能坏了，其实是存档坏了。现在会落一条 AppLog，并把原字节备份到
  `<键>.corrupt`（只留最早那份），用户随手改一次设置也不会把最后一点现场冲掉。
  与 v0.0.108 的启停存档同一套做法。
- **会话下钻的标题与 LIMIT 各说各话**：查询带 `LIMIT 200`，标题写「\(N) 个会话」——
  取满 200 时那句「200 个会话」把「只看了前 200」讲成「一共就 200」。现在 LIMIT 提成
  `sessionDrilldownLimit` 常量，标题走同一个数（取满就说「最近 200 个会话（还有更多未列出）」），
  并有一条用例盯着「SQL 里不许再出现字面量 LIMIT 200」，防两边漂移。
- 两条新用例各做变异验证：把标题写死成「N 个会话」、把备份那几行删掉，都立刻变红。

**没做的两项，理由**：
- *预算预警跨日历日重置导致午夜重复告警*——「新的一天要不要再告一次」是产品选择，不是
  bug；现在的行为是同一段滚动窗口在午夜会被再判一次超额。要改得先定口径（按自然日统计，
  还是纯按升级跨越告警），记进遗留清单。
- *SQLite 步进错误（busy/corrupt）与「库里真的没行」同形*——属实，但要动三处查询与
  健康枚举的口径，值得单独一轮，不夹在这批里顺手改。

套件 418 → 420。

## [0.0.109] - 2026-09-21

### 🧭 审计第二批：读不到的目录不再把干活的人判成待机，报表与面板同一个口径

接 v0.0.108 那份审计里剩下的三项，逐条复核后动手。

- **目录读不了 ≠ 这个目录没有产物**。`FileActivityMonitor` 的终态语义本来写着
  「目录缺失/不可枚举 → 保留旧值 + 连续缺失计数」，但实现只认「根 stat 失败」那一半：
  根在、枚举器建不起来时走的是清零分支 ⇒ 一个正在写文件的 Agent 因为一次读不了目录被
  判成**待机**。而且 `FileManager.enumerator(...)` 对权限失败**不返回 nil**——它照样给你
  一个枚举器，只是第一个对象都拿不到，错误被咽进返回值里；所以第一版只补 `guard let`
  那一支等于补了条走不到的路，真正要挂的是 `errorHandler`。
- **审计报告的头条数字与面板汇总栏不同源**：报告把 `snapshots` 逐条相加，而那份列表
  不含「离线但仍有 24h 用量」的工具和被宿主合并的内嵌组件 ⇒ 报告说 200k、面板说 1.2M，
  用户拿报告对不上岛，怀疑的是岛。现在头条用跨源总量，并把逐条之和与差额来源一起写出来，
  不悄悄换一个数。
- **报表导出 `try?` 掉失败**：面板关掉、看起来像成功，文件其实不在——而用户是在要发给
  别人的时候才发现。现在写失败会留在工作台的红条上并落 AppLog。
- 用例的诚实性：「读不到的目录保留上一份活动」这条**连着两版都是永真的**——第二拍的
  根 mtime 没变、目录又不在 runningDirs 里，走的是「跳过深层递归」那条快路，缓存被原样
  返回，怎么改生产代码它都不会红。是变异验证（把调用方那半个条件删掉，仍然全绿）暴露的。
  现在测试显式让目录「新上线」来强制全量深搜，同样的变异会立刻变红。

套件 415 → 418。

## [0.0.108] - 2026-09-21

### 🔍 四路并行审计：设置管不到的震动、读不到却说「结论可信」、一次失败扫描摘掉档案

按四个互不重叠的视角各派一轮审计（数字口径 / 失败可见性 / 设置与持久化 / 常驻开销），
收回 25 项发现。逐条自己复核过才动手：修 6 项，否决 3 项（理由写在下面），其余记档待办。

**修掉的**
- **「触控板微触觉反馈」这个开关只管 8 个调用点里的 2 个**：其余 6 处直接调
  `NSHapticFeedbackManager`，关掉后展开、收起、停靠对齐、Esc 返回、工具箱动作照样震。
  现在收进 `HapticFeedback.perform` 一个出口，并加棘轮用例——它同时数「直调点必须为 1」
  和「那一个出口必须查 `isEnabled`」：只数第一条的话，把 `guard` 删掉仍然全绿
  （变异验证真的红了这一次，才补上第二条）。顺带把 `UserDefaults.standard` 直读基线
  从 37 降到 29（同一个键此前在 `SoundEffectsManager` 与 `IslandView` 用了两套容错）。
- **尾读把「读不到」和「真的没有」压成同一个空数组**：`LogTailReader` 现在把
  `unreadable` 分开回吐，通用尾窗 / Qoder / Antigravity 三条探测链路把它记进观测证据。
  此前的症状是最坏的组合：岛显示待机、doctor 说「结论可信」，而文件明明就在那里读不出来。
- **一次失败的 CLI 扫描会把宿主内嵌的档案从注册表摘掉**：启停口径是
  `hostInstalled && !selfInstalled ⇒ 摘掉`，所以「bundle 集还在、CLI 集被扫成空」等于
  把 Codex（ChatGPT 内嵌）这类档案删掉监控——卡片消失，没有任何地方说为什么。
  现在「整趟空了 + 上一份非空」沿用旧值并留日志；真实卸载仍然生效（只挡这一种形态）。
- **损坏的启停存档会被设置页第一次写回就地覆盖**：读取侧的「只读降级、不写回」保护不了
  `@State` 里那份降级后的默认集，用户拨任何一个开关就整体写回。现在覆盖前先把原字节
  备份到 `enabledAgents.corruptBackup`（只备份最早那一份），「保留可恢复」才真的可恢复。
- **月末预估费用是裸的 `cost24h × 天数`**：脏 cost 让它跑到 Inf，界面上出现
  「累计 $1e9、预估 $3.1e10」这种本该同量级却差 31 倍的自相矛盾。补 `costProduct` 饱和，
  方向与 `saturatingInt` 对齐（Inf→上限、NaN→0）——这条对齐是我自己写的新用例抓的：
  我先把实现写成了 Inf→0，测试期望才是原本的正确口径。
- **进程表 `sysctl` 三次 ENOMEM 后会带着全零缓冲区继续往下走**：那是「拿不到进程表」，
  却被处理成「机器上一个进程都没有」→ 所有档案判离线、岛清空、日志零字。改为早退并留证据。

**否决的（都试过、量过再决定）**
- *熄屏后冻结采样*（审计建议的省电做法）：会让「人不在机器前时任务完成」收不到通知，
  而那正是这台机器上最需要的场景；空闲降频本来就有（没有在跑的档案时已经 60s 一拍）。
- *把 Antigravity 会话树遍历移出主线程*：实测本机 0 个会话目录，而该遍历早就挂了
  「TTL + 根目录 mtime」双令牌缓存（真实机器量过 797 次 stat / 24ms）。不为不存在的对象做优化。
- *CLI 集之外也给 bundle 集加同样的空集保护*：空 bundle 集让 `hostInstalled` 为假，
  那条 guard 本来就保留档案——方向是安全的，加了等于加一条永不触发的分支。

套件 410 → 415；6 处变异逐一改坏，每处都有对应用例变红。

## [0.0.107] - 2026-09-21

### 🔧 保留窗口从 40 天改成 70 天：30 天那一档还要往前读「上一周期」

v0.0.106 把窗口定在 40 天，理由是「分析页最宽 30 天」。这个理由只覆盖了一半。

- 30 天那一档除了画图，还要往前读**同样长**的一段做对比
  （`TokenTimelineBuilder.previousStart = now - 2 × duration`），而 dim / opencode 两个
  SQLite 源本来就查得到 60 天前。JSONL 这边只留 40 天的话，
  **「较上一周期 ±N%」会被静默少算一块，而图表上完全看不出来**（图只画 30 天）——
  正是折入方案最该防的那类错：数字变了，界面上没有痕迹。
- 窗口改成 **70 天 = 2×最宽分析档 + 10 天余量**，并加一条用例把关系钉死：
  `detailRetention ≥ 2×最宽的 TokenTimeRange`，另附「55 天前的响应必须还在明细里」。
  以后改窗口或改分析档位，两边一动就红。
- 夹具年龄同步跨过新的折入线（45/50/60 天 → 75/80/90 天），真机再对拍一次：
  70 天窗口下 `agentisland tokens --json` 与改动前**逐字节相同**。

## [0.0.106] - 2026-09-21

### 📉 Token 明细不再随日志年龄无限增长，而累计一个数都没变

「明细索引只增不减」挂了很久：留存集合唯一的收缩条件是文件从磁盘消失。这次先测再改
（release 构建、合成语料、脏内存按 `phys_footprint` 差值取数）——
**每条留存明细约 780B 常驻、整趟重建 0.63ms/千条**，两者都只随磁盘语料总量线性上升；本机 24 天的日志已经是 103 份文件 / 200MB。
v0.0.9x 那道「戳备忘录」管不住它：备忘录只在整棵日志树没变时命中，**任何一次写入**
都要把整棵历史树重摊一遍。

- 做法是**折入而不是丢弃**：明细只留最近 40 天，更早的响应压成按工具的合计。
  累计口径照含这份合计，24h 与图表本来也看不到它们。另加单文件 20,000 条的硬上限，
  触顶折掉**最早**那一段。
  （**这一版的 40 天定窄了，v0.0.107 改成 70 天**：30 天那一档除了画图还要往前读同样长的
  「上一周期」做对比。理由见下面 0.0.107 与 `docs/adr/0007`。）
- 为什么不是「按窗口过期就好」：那正是最先写出来的一版，也是最坏的一版——掉出窗口的
  响应直接消失，**面板上的累计静默往下掉**，而没有任何地方说它变过。这违反本仓
  「没看到 ≠ 没有」的口径。不变式钉进了用例。
- 真机对拍：改动前后 `agentisland tokens --json` 输出**逐字节相同**（累计 260,163,366、
  24h 2,384,098）。这条钉的是「折入不改数字」。至于今天到底折没折：本机最老的日志文件
  是 24 天前，40 天的窗口一条都够不着——真机走的是「完全没折」这条分支，折入那条分支
  由注入时钟的用例覆盖（不拿真实个人日志跑测试）。
- 明确接受的不精确：**跨文件**重复且两份拷贝时间戳不同时，窗口外的那一份会各折一次、
  算两遍，多算的那一份落在明细里（所以它影响的不只是累计）。要修就得长期留一份已折入 id
  的台账，而它只要 producing 文件还在磁盘上就回收不掉——省下的存量有限，多出来的却是一个
  「集合缩小即整表重建」的状态机，它出错的方式又是一次数字错。本机 1,215 条响应里
  0 个重复 id，103 份文件也没有一份是另一份的前缀复制。
- 复核（差分跑改前/改后同一套写入与采样）抓到第一版一处真 bug：折入接在增量路径上时，
  去重看不见「先前已经折掉的那一段键」。后果是 **resume 一个掉出窗口的老会话**时同一响应
  被算第二遍，而且第二份带着新时间戳进明细——先污染的是岛上的 24h。修法是把规则收紧成
  「带过折入的文件不走增量」，让它整份重读、重新看得见全部已知事件；上限裁掉的那一侧
  同源，也补了断言。详见 `docs/adr/0007-token-detail-retention-folds-not-drops.md`。
- 顺手把解析里「先拷贝再找标记」倒过来（99% 以上的行是对话正文，命中才拷贝）。
  **这一条不是性能收益**：真机冷跑 `agentisland tokens` 改动前后都是 0.92s——`Data` 切片的
  `Data(line)` 本就是共享存储的写时复制，我原先「把整段 mmap 逐行搬上堆」的判断是错的。
  留着它只是少 165k 次临时对象。
- 8 条新用例，逐条做变异验证：把「累计不加折入」「追加时丢掉旧合计」「折入不做文件内
  去重」「折叠时不作废备忘录」「上限裁成最近的而非最早的」「只剩折入的工具不建桶」等
  8 处逐一改坏，每处都有且只有对应的用例变红。套件 400 → 408。

## [0.0.105] - 2026-09-21

### 👆 岛内小图标的命中区补到 24pt 高，且布局一点没动

v0.0.96 那轮可访问性审计记下「命中区扩到 24×24」，之后一直挂着没做，
原因是它必然改变布局而当时没人能目视核对。现在能自己截图了（`agentisland://settings?tab=`
与窗口级截图是 v0.0.104 加的），一次就做完并验完。

- **先按 24×24 做，实测不成立**：顶栏那一排（搜索 / 工具箱 / 历史 / 收起）每个 chip
  多占 6pt 宽，五个累计把整排往左推，**直接盖住了状态徽标「工作中」**（截图对比过）。
  岛内每一行右侧都排着内存/状态徽标，横向根本没有余量。
- 改成只补**纵向**命中区（`hitTargetHeight()`）：行高由标题文字决定，本来就 ≥24pt，
  所以补高度不动任何布局。12 处 hover 才显形的小图标全部覆盖，
  2D 版没人用就删掉，不留「备用 API」。
- 为什么是 24：macOS 没有 iOS 那条 44pt 规矩，但这些 chip 恰好都是
  **hover 才出现**的——鼠标要落进一个 18pt 的圆里才点得到，比常驻控件难受得多。
- 验证方式是截图对比，不是「看起来没问题」：改动前后各截一张展开态的岛，
  顶栏徽标、chip 位置、行内文本逐元素一致。

## [0.0.104] - 2026-09-21

### 🩹 还 v0.0.97 记档却没修的债：一个能崩的 CLI 参数、一处会误杀的匹配、一份会被覆写的存档

v0.0.97 那轮复核留了六项「确认为真但没修」的债。这一项一项过了一遍：
**四项已修，一项确认早已修掉，一项确认是设计取舍**。

- **`tokens --budget` 能让进程崩掉（实测复现）**：`agentisland tokens --budget 9000000000000000000`
  以 **SIGTRAP(133) 退出且零输出**。旧的入参检查只挡「> Double(Int.max)」，
  而这种值能过检查，再往下 `dailyBudget * 当月天数` 就 Int 溢出。
  现在预算有唯一读法 `DailyBudget.read()`（钳在 0…10 亿）与唯一解析入口
  `DailyBudget.parseArgument()`，三处裸 `integer(forKey:)` 全部收口；
  月度预估的两处乘法改 `SafeNumber.product`（饱和而不 trap）。
  顺手把 `1e999` 这类值在解码时丢掉对应字段——原先 `try` 会让**整条档案**解不开，
  一个坏数字放大成「这个 Agent 没了」。
- **`pathContains` 全路径子串匹配会误杀用户自己的程序**：profile 配 `trae` 时，
  用户在 `~/code/trae-sandbox/` 里跑的 `npx electron .` 命中 TRAE 档案，
  进列表并可被一键终止。改成按**路径段**判定：整段相等、整段等于 `<needle>.app`、
  或以 `<needle> ` 开头且以 `.app` 结尾（`TRAE SOLO CN.app` 这种带空格的应用包）；
  自带目录锚的 needle（`/applications/qoder.app`、`.workbuddy/`）保持子串。
  **识别率没有倒退是量出来的**：改前改后各跑一次 `status`，11 行条目完全一致，
  只有时间戳差几秒。
- **`customAgents` 损坏存档被静默覆写**：`enabledAgents` 早就有「损坏时只读降级、绝不写回」，
  这边没有。现在 `customArchiveState` 把「没这个键」与「有但读不懂」分开，
  写入统一走闸门：去空 id / 去重复 id / 超 64 条直接拒写并说明原因（静默截断等于丢档案），
  顶层损坏时在覆写前把原件挪到 `customAgents.corrupt-backup`——用户至少还有手工恢复的路。
  设置页改成**先落盘成功才改界面状态**，消掉「列表里有、重启就没」的假成功。
- **`check` 与 `clean` 看的不是同一份注册表**：check 列出某个 `cli-*`/自定义 Agent 的异常
  并提示「运行 agentisland clean 可一键终止」，clean 只扫内置集，回答「未发现异常」。
  用户按提示做的那一步必然失败。现在两者都走 `LiveSampler.context()`。
- **`clean` 的两个闸门问题**：① `clean` 以前一个键都不问就杀进程，而同仓 `top [c]`
  早就「先列目标再按 y」——两个入口一个问一个不问，用户记住的那个恰好是危险的；
  现在默认列出目标并等确认（`--force`/`--json` 视为显式非交互）。
  ② `--force` 原先的含义是「连孤儿一起杀」，而孤儿根本分不清是「用户刻意后台化的 agent」
  还是「launchd 托管的常驻服务」（ppid 都是 1）——一个批量开关不该跨过这道界线。
  现在孤儿要 `--include-orphans`，且**必须逐条人工确认**：非终端输入直接拒绝而不是静默放行，
  与 `--json` 同用也拒绝（脚本没有「逐条确认」这件事）。
- **`resolvedEnabled` 在 CLI 侧的写副作用**：读→算→整体写回没有版本号，
  写的是「此刻的注册表快照」。用户在应用里刚改的开关会被一次 `status` 覆盖，
  症状是「我明明关掉了它，跑了一次 status 又回来了」。现在 CLI 走 `readOnly: true`，
  这份配置只有设置页写。
- **确认早已修掉的一项**：`terminateAgent` 的「身份复核自己比自己」——现在拿
  `ProcessMatcher` 对**最新快照**重匹配该 pid 是否仍属于该 Agent，并把路径交给
  `ProcessTerminator.terminate(pid:expectedPath:)` 在发信号前再核一次。是独立校验，没动。
- **顺手补的入口**：`agentisland://settings[?tab=remote]`。齿轮按钮与深链统一走
  `SettingsOpener`（加结构断言后立刻发现 App 里还有一处裸 selector 调用，两处各写一遍
  系统改名字时只会坏一处）。加 `?tab=` 的直接原因是：这个 App 是 accessory 进程，
  Computer Use 看不见它的窗口，此前核对界面只能临时改默认页截图——改完还得撤。

### 🎨 自己开 App 看界面，抓到两处

`agentisland://settings?tab=remote` + 窗口级截图，620×480 实测：

- 通道单选用横向排布时，「自定义 HTTP（微信 Server酱 / PushPlus / 企微 / 钉钉…）」
  折成三行，把「邮箱（SMTP）」挤到中间高度，看起来像漏了一个控件 → 改纵向。
- 总开关关闭时整页 `opacity(0.55)`：连「启用外发」这颗开关自己都被压暗，
  看着像整页不可用，而且 0.55 压下去浅色模式文字对比度直接低于 AA
  （本仓刚把语义色全部拉到 4.5:1 以上，不该在新页面破例）→ 去掉压暗，
  禁用状态交给控件自身的视觉表达。

### 🧪 测试

本轮新增 11 个用例（套件现共 400）：预算解析/钳制与饱和乘法、
路径段匹配的正反两组形态（含 `trae-sandbox` 不误伤与三种真实包名要继续命中）、
自定义档案闸门（去重、空 id、超限拒写、损坏留证）、启停集只读求解不写回、
以及三条结构断言：check/clean 同注册表、预算不许裸读、设置窗口只有一处 selector 出口。
其中两条结构断言写出来的第一件事就是抓出现实问题——
一条发现 App 里真有第二处裸 selector 调用，一条被我自己写的注释文字绊住后改成跳过注释行。

## [0.0.103] - 2026-09-21

### 🩹 重试要分「暂时不行」与「对方明确拒绝」：v0.0.102 把两种失败混成了一种

v0.0.102 给自动外发加了「失败重试一次」，但对所有 `.failed` 一视同仁。于是一个填错的主题名
（ntfy 回 404）、一个写错的授权码（SMTP 回 535）会让每个事件都白发两次——而公开中转
（ntfy.sh）正是按条数限流的，用户在排查配置的路上先给自己撞一次限流。
更要紧的是界面上它显示成「重试 1 次仍未送达」：那读起来像链路在抖，而真相是配置就是错的，
排查方向被彻底带偏。

- `OutboundOutcome.failed` 加一位 `permanent`（默认 false，所以既有 `.failed(reason:)` 构造点
  一字不改）。判据放在**知道状态码的那一层**，不让调度器去猜字符串：
  HTTP 3xx 与 4xx（除 408 请求超时 / 425 过早 / 429 限流）算明确拒绝，5xx 与网络层错误算暂时；
  SMTP 直接用协议自己的分级——5xx 是明确拒绝（535 认证失败、550 拒绝中继），
  4xx 是暂时不行（421 服务器忙、450 稍后再试）；我们主动拒跟重定向那条也算明确拒绝
  （重试一次还是同样的决定）。
- 设置页据此分开说三种话：「已送达（重试 1 次后）」（通道在抖）/「重试 1 次仍未送达」（真不通）/
  「对端明确拒绝，重试无用」（去改配置）。
- 变异验证：让重试条件忽略 `permanent` → 1 条红（请求数从 1 变 2）；SMTP 的 4xx/5xx 分级由两条
  断言夹住（535 必须永久、421 必须可重试），改反即红。

测试 386 → 387。

## [0.0.102] - 2026-09-21

### ⚡️ 自动外发失败后重试一次：一次抖动不该丢掉唯一能叫醒用户的提醒

v0.0.101 解决的是「什么时候该发」，这条解决「没发出去怎么办」。原链路是 `transport.perform`
一次，失败只留一条统一日志和一行「最近外发」。人在机器前时这没关系——岛内本来就看得见；
而这个功能存在的唯一场景恰恰是人不在机器前，此时一次 502 / DNS 抖动就等于这次任务的
完成提醒永久丢失，而且用户完全无从知道。

- **只重试一次，且只重试真的失败**：只有 `.failed` 重试；被策略挡下（总开关 / 分类开关 /
  静默时段 / 在场判定 / 节流）与「未配置」都不重试——它们重试一万次也是同一个结果。
- **「发送测试」不重试**：这个按钮的语义是「现在立刻告诉我通不通」。多等 5 秒、并把一次
  真实失败藏进重试里，反而看不出通道到底是死是活。
- **重试期间不释放节流占位**：`claimThrottle` 的预登记要等最终结果出来才回滚。否则在第二次
  发送还在途时，同一 Agent 的同类事件能挤进来再发一份。
- **重试必须留痕**：`OutboundAttempt` 加 `tries`，设置页如实写「已送达（重试 1 次后）」或
  「失败：…；重试 1 次仍未送达」。通道在抖是用户该知道的事实，不该被一句「已送达」盖住——
  这与本仓「没做到就不印成做到了」是同一条线。
- 重试间隔由 `RemoteNotifier(retryDelay:)` 注入（生产 5 秒，测试归零），否则「结果记账」那条
  要跑 41 次投递的测试会从 1 秒变成 3 分钟。
- **变异验证**：把重试整段短路 → 3 条测试红；把「测试按钮不重试」这条边界去掉 → 1 条红。

测试 383 → 386。

## [0.0.101] - 2026-09-21

### 🩹 修一个「按用户的真实用法根本不成立」的在场判定：补上无输入时长

用户反馈：日常是 Windows 远程桌面连着 Mac，但**人去做别的事**时希望收到提醒。
v0.0.100 的「只在人不在机器前时发送」只认两条信号——屏幕锁定、显示器睡眠。
在远程桌面连着的状态下这两条永远不会成立：会话不锁、屏幕不熄，人早就走了。
也就是说那个开关一打开，通知就再也不发；而它看起来是个正常开关。

- **实测确认**（就在这台机器上）：`CGSessionCopyCurrentDictionary` 的字典里 11 个键、
  **无 `CGSSessionScreenIsLocked`**，`CGDisplayIsAsleep = false`，而键盘已 771 秒没有事件。
  三条里只有「无输入时长」能分辨这个状态。
- **加第三条判据**：`PresenceSignals.idleSeconds`，由 `CGEventSource
  .secondsSinceLastEventType(.combinedSessionState)` 取键盘/左右键/移动/滚轮里最近的一次。
  阈值 `awayIdleSeconds` 默认 120 秒、钳在 30–3600（0 秒会让任何时刻都算离开，等于开关失效）。
  判定 `RemoteNotifyPolicy.isAway(_:)` 是 Core 里的纯函数，信号与判定分层：
  取信号那层碰窗口服务器、测不了，所以不掺判断。
- **fail-open 的方向和节流相反**：取不到输入时长时按「已离开」照常发。
  这一条若 fail-closed，症状是「开关看着开了、其实永远不发」——本仓最难查的那类失效。
- **开关效果必须看得见**：设置页实时显示当前判定与三条信号
  （「当前判定：已离开（距上次输入 214 秒；锁屏 否、显示器睡眠 否）」），
  被挡下时「最近外发」写「未发：有人在机器前（距上次输入 8 秒，未达 120 秒）」；
  取不到时长时页面直接说「这台机器上这条判据没有数据，只认锁屏与显示器睡眠」。
- **诚实标注测不到的部分**：`ScreenPresence.idleSeconds` 本身在测试里造不出来（需要真实
  GUI 会话）。变异验证里「让它永远返回 nil」这一项 0 红——那是真盲区，不假装覆盖；
  改为靠页面上的实时判定行让人当场能核对。判定逻辑与调用点传参都有断言
  （8 项变异里 7 项被捕获，含「退回只认锁屏/熄屏」「阈值不参与判定」「fail-open 反了」
  「被挡下不给依据」「调用点漏传 presence」等）。
- 集成实测：真信号 + 真外发路径走一遍，阈值 30/120/300 秒判为离开会发，
  调到 3600 秒则 `suppressed(reason: "有人在机器前（距上次输入 771 秒，未达 3600 秒）")`。

测试 378 → 383。

## [0.0.100] - 2026-09-21

### ✨️ 新增：可选的远程通知通道（ntfy / 自定义 HTTP（微信中转）/ 邮箱 SMTP）

解决的问题：用 Windows 远程桌面连着 Mac 时，任务完成只落在 Mac 屏幕上，人收不到。
功能默认关闭，开关在「设置 → 远程通知」。两轮深度复核 + 一次本地真实链路实测，
共抓出 9 个「离线测试全绿但功能其实不成立」的缺陷（见下）。

- **三个通道，只有 ntfy 做成预设**。ntfy 的请求形状按官方文档核实过（`POST https://<服务器>/<主题>`、
  `X-Title`、`X-Priority` 告警 4 / 普通 3，公开服务器单条 4096 字节上限）。
  Server酱 / PushPlus / 企微 / 钉钉的文档站当时是 JS 渲染、404 或超时，
  **没核实到的字段名一律不预置**：微信走「自定义 HTTP 模板」，且**请求体模板必填**
  （早先版本在模板留空时自带 `title`/`body`/`content` 一套字段名，那正是本仓说不预置却自己编了的行为）。
  邮箱走 SMTP 465：`Network.framework` 没有「在已建立的 TCP 上原地升级 TLS」的能力，
  STARTTLS(25/587) 做不了，所以在配置层就拦住并说明原因；代码里也不留半条 STARTTLS 分支。
- **凭据只进钥匙串**，条目名由通道唯一决定（`remote.ntfy` / `remote.customHTTP` / `remote.smtpEmail`），
  不做成可配字段——「密钥存了但条目名对不上」是界面全绿、每次发送都失败的失效。
  写入改成「先加，遇重复才替换」：原先先删后加，而 ad-hoc 签名下「加」可能被拒，
  结果是一条没存上、旧凭据也没了。掩码覆盖 `?token=`/`?key=`/`?access_token=`、
  `{key}` 占位、**主机名首段就是密钥**（判据：20+ 位字母数字，与大小写无关）、
  以及**路径段里的长令牌**——帮助文本就叫用户「把控制台地址整段粘进来」，
  不遮路径段等于把 SendKey 印在设置页与预览上。另给两条独立警告：
  明文 `http://` 端点、以及地址里没写 `{key}` 却含 `key=`/`.send`/高熵片段（会明文落盘）。
- **默认外发内容**只有「哪个 Agent + 什么状态」；命令内容、路径、消息原文要显式勾选。
  深链 `/notify` 与 HTTP `/notify`、`/event` **一律不外送**。这条闸门差点是假的：
  `LocalEventServer` 构造事件时漏传 `externallyDelivered`，于是本机任意进程都能
  往用户手机/邮箱灌任意文字，而外发路径没有 `shouldPeek` 那类分级可挡。
- **实测（本地假接收端 + 自签证书 TLS 服务器）与复核抓出的其余缺陷**：
  1. `X-Title` 的中文被 URLSession **静默丢弃**（对端只收到 `Qoder · `），本地却报 `.delivered`
     → 头值走 `WireText.headerValue` 百分号编码，并把 Agent 名同时写进正文首行；
  2. 头值允许 `& # % +`，同一套编码被当成查询/表单编码器用 → 勾选「附带动作」后
     正文里的 `git commit -m a && curl …` 会在接收端被切成两个字段 → 拆成 `queryValue`/`formValue`
     （RFC 3986 unreserved）与头值两套规则；JSON 正文改为自己做字符串转义（可逆性有断言）；
  3. 对端不可达时 `connect()` **永久挂住**：`NWConnection` 对被拒绝的连接转入 `.waiting` 自行重试，
     状态永远走不到 `.ready/.failed` → 独立 `DispatchWorkItem` 兜底 + `ResumeOnce` 保证只恢复一次；
  4. 同样形态还留在读路径（`deadline` 只在 receive 回调里比较，「TLS 通了但不回行」永不超时）
     与写路径（完全没有期限）→ 读写各自加独立定时器；`SMTPClient.deliver` 补 `defer { io.close() }`，
     否则每条失败路径都留下一个活的 TLS 连接与已解密的授权码；
  5. SMTP 的多行回复（`250-` / `354-`）只读一行会让后续每次读取全部错位 → `expect` 读到续行结束；
  6. 节流键用显示名：显示名可被外部投递路径随便填，换个名字就绕过节流 → 改按 `agentId`，
     并把「查窗口 + 登记」合进一次加锁（原先两个锁区间之间可被并发穿透，等于两倍的窗口）；
  7. 用户填 `ntfy.mine.local/island`（少贴协议头）会被拼成 `https://ntfy.sh/ntfy.mine.local/island`
     ——内容跑到一台他没选过的公网服务器、主题名还是他的内网主机名 → 配置层拒绝；
  8. `deliver` 用 `Task { @MainActor in … }`：await 之后整条链留在主线程，且每个事件都在主线程
     解码三份 JSON、读一次钥匙串 → 总开关排到最前、配齐检查排在起 Task 之前、任务不标 `@MainActor`；
  9. 「真实 socket 连不上要如实返回」这一条只有假 IO 测过 → 新增走真 `NWConnection` 的用例；
  10. 为拦重定向给 `URLSession` 加了 delegate，却沿用 completionHandler 版 `dataTask(with:)`
      ——实测每次都是 `NSURLError -999` 且请求根本没出门，**而离线测试全绿**
      （它们注入的是假传输，从不经过 `HTTPTransport`）。改用 async 的 `data(for:)`，
      并加一条钉住这个组合的结构断言（变异验证：把 `dataTask(with:)` 加回去即变红）。
- **口径与可见性**：「发送测试」只绕过「什么时候打扰用户」三条（节流/静默/在场），
  总开关与事件类型开关照旧管用；`claim/release` 让失败不占用节流窗口；配置检查排在节流之前。
  预览按钮改名「生成预览（不送出）」，测试按钮写明「真的送出」，
  并说清「已送达＝对方接受了这条请求，不等于已推送到手机」（多数中转服务内部失败也回 200）。
  「最近外发」每 5 秒刷新，否则用户得动一下字段才看得到新记录。
- **可选的「只在人不在机器前时发送」**：信号取 `CGSessionCopyCurrentDictionary` 的
  `CGSSessionScreenIsLocked`（实测未锁屏时该键不存在）与 `CGDisplayIsAsleep`，默认关闭，
  被这条挡下时写「未发：有人在机器前」。**坑要说破**：远程桌面连着 Mac 时会话既未锁定也未熄屏，
  开着这条恰好会把自己要的通知挡掉。

### 🧪 测试

远程通知共 32 个用例（测试 346 → 378）：SMTP 状态机（465 命令序列、认证失败带回复码、
多行回复、无响应超时、dot-stuffing、RFC 2047 主题、RFC 822 日期、失败路径也关连接、
真 socket 的超时收敛）、ntfy 与自定义通道的请求形状与配置层拦截、三套线格式编码
（头值 / 查询串 / JSON）、策略矩阵（总开关 / 分类 / 节流按 id 分键 / 跨零点静默 / 在场判定 /
归一化）、预览与掩码四形态、落盘读盘与「加字段不清空旧配置」、
以及两条结构断言：外发前有闸门、外部入口造事件时当场打标记。
两轮变异验证合计 53 项，其中 5 项第一次跑「没让任何用例变红」——那是**测试真的没覆盖**，
补法分别是：多行回复、失败关连接、读盘端口收口、`/notify` 的构造侧标记、测试按钮的类型开关。
STARTTLS 的实现与用例一并删掉：那条路径在生产里永远走不到（非 465 在 socket 层就返回 false），
留一个能跑绿的测试只会让人以为这个客户端支持 587。
最后一轮真实链路实测（本地假接收端收三种通道的请求体并逐字段解析）确认：ntfy 的中文标题以
ASCII 百分号编码到达、表单里的 `&&` 与 `#` 没有多切出字段、JSON 正文可被对方 `json.loads` 解析。

## [0.0.99] - 2026-09-20

### 🖥️ 新增 Qoder 监控（状态 / 动作 / 等待确认），并实测确认它的用量无法监控

- **档案注册**：bundle id `com.qoder.app`、进程名 `qoder`（主进程与 `Qoder Helper (Renderer)`
  走前缀族规则一并命中）、会话根 `~/.qoder/projects`。`pathContains` **锚定到
  `/applications/qoder.app`** 而非裸子串 `qoder`——本仓刚把「过宽匹配可能误杀用户自己的
  Electron 程序」记为已知风险，新档案不再制造一例。
- **专用方言 `.qoderTranscript`**，而不是交给通用尾窗关键词扫描。Qoder 的逐行记录与
  Anthropic 同形（`message.role/content/stop_reason/usage`，工具名 `Bash`/`Edit`/
  `AskUserQuestion`/`Agent`），「等待用户」不是一个字段而是结构事实：
  **`AskUserQuestion` 这个 tool_use 还没有对应的 tool_result**。只看关键字会在
  「刚答完那一拍」反向误报（本仓在 Claude 上踩过同形），因此单独实现。
- **实测跑通**（Qoder 正在执行本会话时）：
  `🟢 工作中 ️ Qoder 514 0.0% 984M 3 — — 运行: cd /Users/…/workspace/Agent… 20s前`，
  `doctor` 给出「结论可信：本轮读到了会话强语义」。
- **Token 消耗：确认无法监控，并且没有假装能。** 本机 1,033 条 `usage` 记录求和：
  `input_tokens / output_tokens / cache_read_input_tokens / cache_creation_input_tokens`
  **全为 0**，唯一有值的是 `credits`（合计 303.37）与 `context_usage_ratio`；
  `requestTokenAnchor` 的两个字段是 64 字符不透明串，logs 与 `.models` 里也没有计数。
  试过接入采集：`agentisland tokens` 冷跑从 ~2.05s 涨到 ~2.9–4.0s（单个会话文件可达 10MB），
  换回 0 条数据 → **撤掉**，用量列对 Qoder 显示 `—`（没取到）而不是 `0`（没有），
  `doctor` 归入「未接入本地明细源」并说明「不代表它没在工作」。
- 新增 `docs/research/qoder-monitoring.md`：把「字段存在」与「数据存在」的区别、
  以及要做 credits 需要先定的三件事（单位、汇率来源、增量还是快照口径）写清楚。
- 新增 `docs/research/remote-notifications.md`：跨平台通知调研。核心结论——
  macOS 通知是本机另一个进程画的 UI，远程桌面只传像素，**在 Mac 侧无法穿透**，只能外发；
  通道按「要不要自己养服务器」分类对比（ntfy / 企业微信 / 钉钉 / 飞书 / Server酱 / 邮箱），
  并说明为什么邮箱排在后面（Foundation 无 SMTP 客户端，`sendmail` 无法判断是否真发出，
  违反本仓的诚实原则）。三条不可谈判的约束：凭据只进钥匙串（webhook URL 本身就是密钥）、
  默认只发「Agent 名 + 状态 + 时长」且发送前可预览、失败必须在岛内可见。

测试 340 → 346（Qoder 方言 6 条，其中「提问已回答不得再报等待」与「命令已返回不得仍显示运行中」
两条做过变异验证：让解析器忽略已回答集合，两条同时变红）。

## [0.0.98] - 2026-09-20

### 🧹 清理复核不再谎报「已处置」

v0.0.97 的 CHANGELOG 里留了一条「复核为真但未修」：**忽略 SIGTERM 的死锁进程会被报成清理完成**。
这一轮把它修掉，因为它骗的是用户做决策依据的那一行字。

- **成因**：工作台的清理复核拿的是「条目是否还在异常列表里」。而 `cleanAnomalies` 会
  `resetTracking(for:)` 清掉 hung 证据（`highCpuSince` 归零），1.2s 后重扫时该进程
  不再满足 hung 条件 → 条目消失 → 复核判定成功。**列表空了 ≠ 进程死了**，
  真死锁（仍在跑、拒绝 SIGTERM）恰好是最会命中这条的路径。单条清理与批量清理同一个口径，两处都改。
- **改法**：新增 `ProcessTerminator.isAlive(pid:expectedPath:)`，按 `kill(pid, 0)` 探活，
  并在给出预期路径时校验可执行文件名——pid 被系统复用给别的程序时不能算「目标仍存活」；
  探到活但取不到路径（权限/正在退出）时**保守当作存活**，宁可报失败也不谎报成功。
- 复核口径改为「原目标 pid 是否还活着」，失败提示也从「已保留在列表中」改成说明是探活失败、
  可能需要更高权限、可从活动监视器处理。
- 新增 5 条断言（自己的 pid 存活、pid 0/-1 不算目标、路径不符即身份不符、
  已 `waitUntilExit` 的子进程不得仍判存活）。把 `isAlive` 改成恒返回 false（即退回旧口径），
  第一条断言立刻变红。

### 仍未修的（复核为真，需要设计决定或你的目视确认）
- **杀进程目标过宽**：`pathContains` 是裸子串（`["trae"]`、`["zcode"]`、`["windsurf"]`…），
  进程名先命中 `Electron` 再路径含 `trae` 就会被认成 TRAE——例如用户在 `~/code/trae-sandbox`
  里跑 `npx electron .`。收紧匹配会改变识别率、且只能在你这台机器上验证，故未动；
  本轮的探活复核至少保证「误认的进程被点了清理后，结果如实回报」。
- `clean --force` 仍绕过「孤儿只准逐条手动确认」；`customAgents` 损坏存档会被下一次增删整档覆写；
  自定义档案数值/身份字段零校验；`dailyTokenBudget` 无区间；CLI `check` 与 `clean` 注册表不一致；
  子命令不识别 `--help` 与未知 flag。

## [0.0.97] - 2026-09-20

### 🚪 第六轮：改审四个从未被碰过的入口（深链 / 杀进程 / 设置持久化 / CLI）

前五轮都在引擎、解析、UI、测试上打转。这一轮换角度：把**外部输入面**与**破坏性操作面**
交给四个并行 agent。回报 24 条，复核确认为真并已修的 14 条如下——其中 6 条是
**用已发布二进制实测复现**的，不是静态推断。

**实测复现并修掉的 CLI 契约缺陷**
- `agentisland tokens --budget 1e308m` → **SIGTRAP(133)，无任何诊断**（`Int(无限)` 是运行时 trap）。
  改为可失败解析 + stderr + `exit 2`。实测修复后：`无效的预算值: 1e308m（需要非负数字，可带 k/m 后缀）`。
- `agentisland doctor --json | jq` → **jq 解析失败**：进度提示只看 `--quiet` 不看 `--json`，
  把 `⏳ 采集 CPU 基线…` 印进了 stdout。实测修复后 stdout 首行即 `[`。
- `agentisland report -o /不存在的目录/x.md` → 打印「导出报表失败」却 **exit 0**，CI 会认为报表已生成。
  新增 `CLIExit`（1=执行失败，2=用法错误），report / notify / open / clean 的失败分支一律非零退出。
- `agentisland notify` 缺消息 → 印红字后 exit 0；`--port abc` → 静默回落 41999；
  URL Scheme 回退路径 `try? proc.run()` 后**无条件印 success:true**（open 起不来也报已投递）。
  现在都按事实回报（含 `terminationStatus`）。
- `agentisland top` 每行印 `24h Tokens: 0`，而同一台机器 `report` 是 **1.61M**——这正是 v0.0.90
  在 `status` 修过的「把没取数印成 0」，`top` 自造一套引擎、从不 start 轮询。改为走 `LiveSampler`
  （含启停集与用量），nil 列印 `—`。实测修复后 `top` 显示 1.61M。
- `report --json` 实测输出的是 Markdown（help 里 `--json` 被宣称为全局写法）。现认成 `--format json`。
- `clean --json` 把**所有候选 pid** 报成已杀、`success` 恒 true，与 `terminatedCount` 无关；
  `CleanResult` 现在带 `terminatedPids`，一个都没杀掉时如实失败退出。

**深链（`agentisland://`）——任何本地进程都够得着的入口**
- 伪造「等待你确认」：`open "agentisland://notify?agent=claude&type=attention&message=..."`
  产生的横幅与系统通知，和引擎自己判定的真实待确认**逐字节相同**。npm postinstall / `.command` /
  cron 触发无需任何权限提示。现在事件带 `externallyDelivered` 来源标记，横幅角标与通知标题都会
  显示「外部投递 · 」，外部事件也无法冒充未知 Agent（投递目标必须解析到已知档案）。
- 循环发 `type=costspike` 的链接曾可**无限压制真实告警**：外部事件也登记 30s 保护期并抢占单槽横幅。
  现在外部投递不参与保护期。
- `agentisland://export` 会 `clearContents()` 静默销毁用户正准备粘贴的内容（密码/命令）。
  深链改为只导航到工作台；自动化请走用户显式执行的 `agentisland report --copy`。
- 审计报告 Markdown 表格只转义了 `summaryText` 的 `|`，`agentName` 与**换行**都不处理
  （`queryItems` 会把 `%0A` 解成真实换行）→ 攻击者可往用户粘进工单的报告里自造行。
  新增 `AuditReportExporter.cell()`，三处表格行统一转义。
- 解析本身此前**零覆盖**（它在 `@MainActor` 的 UI 目标里，runner 够不着）。抽出
  `AgentIslandCore.URLSchemeParser`，补 15 条解析断言（别名 / host 与 path 两形态 /
  查询键大小写 / 重复键 first-wins / percent 解码 / 非本 scheme 拒绝）。
- 文档纠正：`agentisland://clean` 从来只是跳到工作台，README 与代码注释却写着
  「一键静默清理孤儿与异常进程」。

**设置持久化**
- **预算预警对从未碰过设置页的用户永久失效**：`budgetAlertEnabled` 的 UI 默认是 true，
  引擎却用 `UserDefaults.bool(forKey:)` 读（缺键给 false）。新增 `SettingBool.read(_:default:)`
  唯一读法，并修掉同形的完成音开关（`SoundEffectsManager` 用 `?? true`、`IslandView` 用 `bool()`，
  同一个键两种真相）。
- 新增一条**不写死键名**的结构断言：凡 `@AppStorage` 默认 true 的 `SettingKey`，任何地方都不得
  再用 `bool(forKey:)` 裸读。把引擎改回旧写法，断言立刻指到 `ActivityEngine.swift:904`。

测试 338 → 339（新增深链解析、来源标记、报告转义、设置口径四组，全部做过变异验证）。

### 这一轮明确没做的（复核确认为真，但需要设计决定或目视确认）
- **杀进程目标过宽**：`pathContains` 用全路径子串匹配，用户在 `~/code/trae-sandbox` 里
  `npx electron .` 会被认成 TRAE；`terminateAgent` 的「身份复核」拿同一 pid 自己比自己，
  不构成独立校验。改法要重定匹配口径，会影响识别率，留待单独一轮。
- **清理复核恒报成功**：`cleanAnomalies` 先 `resetTracking` 把 hung 证据清零，1.2s 后重扫
  时条目消失即被判「已死」，真死锁（忽略 SIGTERM）也会被报成已处置。
- `clean --force` 仍绕过「孤儿只准逐条手动确认」的闸门；`top` 的 `[c]` 已改为先列目标再按 y 确认。
- `customAgents` 损坏存档会被下一次增删整档覆写（`enabledAgents` 有只读降级保护，这边没有）；
  自定义档案的数值/身份字段零校验（Infinity、空 id、重复 id、无条数上限）；
  `dailyTokenBudget` 是唯一没有区间的数值设置；`resolvedEnabled` 在 CLI 侧带写副作用（跨进程丢更新）。
- `check` 用完整注册表而 `clean` 只用内置表，于是 check 列出并承诺「clean 可一键终止」的目标，
  clean 看不见。

## [0.0.96] - 2026-09-20

### ♿ 第五轮：浅色可读性、VoiceOver 可操作性、视图重算，与一批「能失败的」新测试

三个并行 agent 分别做可访问性盘点、视图重算复核、测试盲区复核。这一轮的产出 mostly 不可见
——但岛内小字号在浅色下读不清、以及 VoiceOver 用户「听得到按钮却按不动」，都是真实缺陷。

**浅色对比度（实算 sRGB 相对亮度，非目测）**
- 离线态文字压在浅底上原本只有 **2.34:1**（连 3:1 的图形线都不到），待机 4.34、青色系强调字 3.51、
  琥珀强调字 4.25、`inkMuted48` 压在 chipFill 上 4.38 —— 全部低于 WCAG AA 正文 4.5:1，字号 8–10pt。
- 因为上一轮把色板收进了 `Theme.Ramp` 一处，这次是**改 6 行**而不是改 45 处：
  离线 `slate400→slate500` 且底色退浅一档 `slate100→slate50`（4.55）、待机 `slate500→slate600`（6.92）、
  青色浅半边 `0x0284c7→sky800`（6.42）、琥珀浅半边 `amber700→amber900`（7.73）、
  `inkMuted48`/`onDarkFaint` 浅半边压到 slate600（6.98）。深色外观一字未动。

**VoiceOver 可操作**
- 6 处 `.onTapGesture` + `.accessibilityAddTraits(.isButton)` 的元素**没有 AXPress**——
  `isButton` 只改语义不装动作，VO 用户听得到「按钮」却按不动。逐个补 `.accessibilityAction`：
  收起态细条（岛的第一入口）、列表行、模型行、会话行、流水条目、菜单栏快捷行。
  点按体抽成 `openDetail()` / `openSessionDirectory()` / `toggleExpanded(_:)`，
  手势与无障碍动作共用一处，不会两条入口各写一遍再漂移。
- 列表行/模型行/会话行加 `.accessibilityElement(children: .ignore)`：显式标签已把状态说全，
  不再让 VO 逐字播报 token/内存/点阵子元素。
- 4 个只有 `.help` 的图标按钮补 `.accessibilityLabel`（help 不是标签，VO 读不到）；
  3 个选中态只靠底色/字重表达的控件补 `.accessibilityValue("已选择"/"未选择")`。

**视图重算（复核后只改真正值得的）**
- `AgentRowView` / `AgentHoverTooltip` 把 `@ObservedObject var engine` 降为 `let`：
  两者 body 内**一处都不读** engine 的 @Published 状态（只调 terminateAgent）。
  复核同时纠正了审计的成本判断——父视图本就观察 engine 且无 `.equatable()`，
  所以这一改动省下的是「多一个订阅者」而非「每拍 N 次重渲染」，属卫生而非性能悬崖。
- 流水页：`filteredEvents` 计算属性在 body 里被读 4 次 → hoist 成一次；
  筛选条数从「每个 chip 各 filter+count 一遍（.errors 那支还会对每条事件跑模式分析）」
  改为刷新回调里一次算齐存 `countsByFilter`。
- 审计报的第三项（`expandedCard` 12 次 `visibleSnapshots`）经复核为微秒级，按本仓规矩不动。

**第三方 JSON 类型漂移容错**
- 新增 `SafeNumber.jsonInt`：`"step_index": "3"` 或 `3.0` 这类改版，`as? Int` 会静默给 nil，
  于是 `?? 0` 把 Antigravity 每行指纹压成 `step-0`（进度判定失效）。12 个站点统一改用它；
  越界仍走饱和钳制。
- 顺带把 ISO 时间戳插值也过一遍 `.escaped`（对它是恒等操作），换来一条**不需要例外清单**的
  结构规则：字符串插值进 SQL 引号必须过 `.escaped`。

**测试盲区（328 → 335，每条都做过变异验证）**
- `ReadonlyDB`「连接已缓存、文件随后被删 → 上报 `.missing`」此前零覆盖（只测了从未打开那一支）。
- SQL 转义此前只有纯函数用例：现在把含 `'` 的 modelId 灌进真实查询，去掉 `.escaped` 立刻变红。
- `Verdict.summary` 五条文案逐字锁定（对调两支返回串此前全绿）；`Code` 加 `CaseIterable`，
  新增码不补文案就会因数量断言失败。
- 两处恒过断言换成可失败：`usage["dim"] != nil || usage.isEmpty`（两支皆真）改为断言夹具期望值；
  可观测性夹具的 24h 与累计此前被构造成恒等，读错字段测不出——现在两者不同值并断言走 `tokensTotal`。
- 三条结构棘轮：`@ObservedObject var engine` 计数、流水页 `events.filter {` 计数、SQL 引号插值必须转义。
- **如实记录两处「没写测试」**：① 审计断言「parsedMeta 的 step_index 漂移会让已答复提问误报
  attention」，把该站点单独退回旧写法后整套测试仍全绿，因果不成立，故不为其写用例；
  ② `withDB` 重试「成功」分支需要「同 inode 且 prepare 瞬时失败」的夹具，构造不出来，
  仍属未覆盖路径（已覆盖的是两次都失败那一支）。

## [0.0.95] - 2026-09-20

### 🔍 第四轮复核抓出上一轮修复引入的性能悬崖，并把它变成可测语义

v0.0.94 的「增量解析只承认量到的那段字节」是对的方向，但收口收得太粗：**收敛检查**
（再派一个 agent 只审这一轮的 diff）指出，段尾若截在半行上，`endedWithNewline` 会变成
`false`，下一轮 `canAppend` 不成立 → `offset` 归零 → **整文件重解析**。对正在被追加的
日志这几乎每轮都发生——也就是说，为了修一个潜伏的重复计数，引入了一个更常触发的性能悬崖，
同时那半行还会被当成完整行解析出来（重复计数的另一种形态）。

- 被承认的段回退到**最后一个完整行**，回退上限与解析器「跳过 >1MB 巨行」的口径一致；
  `consumedThrough` 按回退后的位置写进戳。
- `data.subdata(in:)` 换回 `dropFirst/prefix` 切片：前者会把 mmap 段整体搬到堆上，
  正好抵消 `.mappedIfSafe` 的意图。
- 新增可测语义并做变异验证：半行不计入 → 补完整行恰好 +1 → 连续三轮采样数字稳定。
  把回退去掉跑测试，聚合立刻从 `n=1 / 50 tokens` 变成 `n=2 / 100 tokens`，正是那类重复计数。
- 顺带如实化两处注释：`openReadonly` 的代际校验只覆盖「打开期间发生 stop()」，
  完全发生在 stop 之后的查询仍会写回缓存（当前唯一 stop 点是应用退出，fd 随进程回收）；
  `LogTailReader` 开头「其他环节的毫秒级陈旧无影响」已被同文件改用 stat 的事实推翻。

测试 327 → 328。实机核对：`agentisland tokens` 与 v0.0.94 逐行一致。

### 关于收敛
四轮下来的信号很清楚：第一轮挖出仓库里的存量缺陷（崩溃、注入、锁竞争、死代码），
第二、三轮挖出的**几乎全是前一轮修复自身的问题**（`defer` 的注册位置、恒等的代际比较、
缓存负结果丢掉首会话、AppleScript 非法转义、这次的段尾截断）。存量面已接近见底，
剩下的都是有明确代价权衡的项（浅色对比度数值、可访问性命中区、索引按窗口裁剪、
notify 鉴权，均已连同实测数字写进 README「已知限制」），不再是「改一处少一处」的缺陷。

## [0.0.94] - 2026-09-20

### 🔧 第三轮：收口审计报回的 4 处潜伏缺陷，并修掉自己修复里的 2 处

第二轮复核派去审我自己的 diff，回报 5 条，其中 2 条是**这一轮修复本身**的缺陷——
`defer` 的位置决定了它到底覆盖不覆盖目标路径，这类细节最容易在「改了就是修了」的错觉里溜过去。

- **一次性 SQLite 连接的结算 `defer` 放晚了**：Swift 的 `defer` 只对注册点之后的控制流生效，
  我把它放在 `prepare` 之后，于是它恰好漏掉自己注释里点名要覆盖的「prepare 失败」那条 return。
  同时它的注册顺序晚于 `sqlite3_finalize`，逆序执行变成**先 close 后 finalize**——未 finalize 的
  连接上 `sqlite3_close` 只返回 `SQLITE_BUSY` 并把连接留下，而数组已清空，等于永不重试。
  上移到 `db` 定妥之后、`prepare` 之前，一并换 `sqlite3_close_v2` 兜底。
- **代际校验此前恒等成立**：`openReadonly` 在 `sqlite3_open_v2` **之后**才读 `dbGeneration`
  并与另一次读取比较，「stop() 发生在打开期间」这一种判不出来，迟到新建的连接会被写进缓存，
  而 `closeConnectionsAsync` 早已跑完，从此无人再关。改为打开前取基准。
- **增量解析只承认「量到的那段字节」**：`stat` 与 `mmap` 之间第三方仍可继续追加。此前按读到的
  实际长度解析、却把较小的旧 size 记进缓存，下一轮 offset 落在已解析过的字节上重复计数
  （无 `eventId` 的行其去重键含 offset，恰随之外移而失效）。修完膨胀后，第二轮复核指出**对称的
  漏算**仍在（原地截断时缓存 size 虚高）→ 改为回吐 `consumedThrough` 并写入戳，两侧一起收口。
  实机核对：改动前后 `agentisland tokens` 同一时刻逐行一致（`dim` 的 1.55M→1.26M 是 24h 窗口
  自然滑出，同一个旧二进制隔 20 分钟自己就变了，不是代码差异）。
- **`newestFile` 的 mtime 改 `stat(2)` 现取**：`URL.resourceValues` 有毫秒级陈旧窗口，而该函数的
  职责恰恰是「找出刚被写的那个文件」，缓存窗口会让它系统性漏掉最新一次写入。
- **预算耗尽不再静默，也不造成日志风暴**：条目数超预算时枚举顺序并非时间序，返回值只能算
  「部分枚举内最新」，现按目录去重告警一次（每拍对每个目录各调一次，不去重就是刷屏）。

测试 327 通过 / 0 失败，门禁 `GATE_EXIT=0`。

## [0.0.93] - 2026-09-20

### 🔒 五路并行深审两轮：修掉 2 处崩溃/UB、2 处命令注入、1 处局域网可伪造事件

五个审计 agent 并行覆盖并发、资源、测试有效性、外部输入、视图与可访问性；结论一律
**逐条复核后才动手**（本仓的审计有过假阳性，如「无锁字典」其实有 NSLock）。第二轮专门
派一个 agent 复核我自己这一轮的改动，它抓出 4 处我引入或改漏的问题（见末段）。

**崩溃与未定义行为**
- **`sqlite3_bind_text` 传了 `nil` 析构器（= SQLITE_STATIC）**：Swift `String` 桥出的 C 缓冲区
  只在 bind 那一行有效，而 `step` 在下一行——OpenCode 动作探测每 2s 在主线程读一次可能已回收
  的内存（轻则把别的字符串当 session_id 查错会话、动作文案串味，重则崩）。析构器常量收进
  `ReadonlyDB.transientDestructor` 单一来源（本仓另一处用法早就是对的，两处各写一份才漏了这一处）。
- **Cline 的 `Int64(ts)` 对外部 `Double` 直接转换**：`ui_messages.json` 里一条 `1e30` / `-9.2e18`
  哨兵就是运行时 trap，且发生在 `@MainActor` 采样拍上——岛直接消失，2s 后必复现。改走 `SafeNumber`
  饱和钳制。把修复改回去跑测试，整条 runner 当场 `Fatal error` 而死，这是最硬的一种验证。

**命令注入（两处，输入都是第三方库里的会话目录）**
- `RecentSessionNavigator` 的 `do script "cd …"` 与详情页「打开终端」的 `do shell script`
  都**只转义双引号**：`;` `|` `&&` `$()` 反引号照原样进 shell——点一次即在用户终端执行任意命令。
  新增 `ShellQuoting`（POSIX 单引号词 + AppleScript 字面量两层，顺序固定），并把 `cd` 加上 `--`
  终止选项。顺带修好一个真实故障：含空格的目录此前连 `cd` 都会失败。
- 新增结构断言：任何 `do script` / `do shell script` 出口未经 `ShellQuoting` 即测试失败。

**局域网可伪造岛上事件**
- `LocalEventServer` 注释写「仅绑定 127.0.0.1」，但 `NWParameters.tcp` 不设 `requiredLocalEndpoint`
  实际监听 `*:41999`（`lsof` 实测 IPv6 双栈通配），而 `/notify` 无鉴权——同局域网任意主机都能
  往岛上写「任务完成 / 需要你确认」。现在 macOS 14+ 限定回环监听，低版本显式告警而不是假装安全。
  改前 `lsof` 实测 `IPv6 TCP *:41999 (LISTEN)`，改后 `IPv4 TCP 127.0.0.1:41999 (LISTEN)`，且 `curl POST /notify` 仍返回 `{"success":true,…}`。
  （第一版把 `requiredLocalEndpoint` 与 `on: port` 同时传给 `NWListener`，实测构造直接失败、端点整个不监听——curl 空应答才发现，端口只能由 `requiredLocalEndpoint` 提供。）
- 同一处的三个资源缺陷：只连不发的客户端既不续收也不取消（连接与其完成块互相持有）、无并发上限、
  无空闲超时。改为活连接表（计数一旦漏减就会把好端端的 notify 关门）+ 5s 回收 + 上限 16。

**功能失效与主线程阻塞**
- **展开态合盖再开盖，面板 Token 数字永久停更**：`handleSystemSleep` 只 `pause()` 未复位
  `tokenPollingStarted`，唤醒路径的 `startTokenPollingIfNeeded()` 被 guard 挡回。复位后又引出
  第二个问题——`stop()` 的收尾判据失效，盒盖期间退出应用就没人关只读连接；改用
  「本次运行是否**曾经**启动过轮询」作判据，两条契约同时成立。
- **只读库的锁跨整段 SQL 持有，而实时流水早已在后台队列共用它**：一次 500 行扫描就能堵住主线程
  那一拍（类注释还写着「当前全部调用点在主线程」）。后台侧新增 `withDedicatedConnection`
  （不进缓存、不碰共享锁、`close_v2` 收尾），5 个流水源全部改道。
- **详情页每次渲染都付两次 `sysctl(KERN_PROC_ALL)`**：`performanceCard` 在 body 里调
  `inspectProcessTree`，约 600 条 `kinfo_proc` + 每 pid 一次 `proc_pid_rusage` 全落主线程，
  还会与采样拍互相消费 CPU 差分窗口把岛内占用数字带偏。改为复用上拍的进程表，零额外系统调用。

**两处「写着安全其实没守」的护栏**
- `InstalledAppsCache.refreshingThread` 全仓无人赋值，唯一的重入断言恒真；断言挪到会真死锁的
  `refresh()` 等待之前，线程登记补在 `performRefresh`。
- `locateCache` 把 `nil` 挡在命中条件外，等于「装了但闲置」的常态每拍重跑全树 stat；
  负结果改为按 TTL 命中，但**只在有失效令牌时**——第一版无差别缓存，把 cline（`rootDir: nil`）
  「用户刚开的第一个会话」的发现延迟从 ≤2s 拖到 10s，第二轮复核抓到后已收口。

**测试有效性（审计报回 5 处永真断言，全部改成可失败）**
- 「setEnabled 后走后台路径」三条断言恒真（`Int >= 0`、`while` 条件已假、`isEmpty` 由 setup 保证）
  ——把采样改回主线程同步它照样绿。改为计数断言：`setEnabled` 返回前全表扫描次数不得增加。
- `refreshTokenUsageOnce` 断言是 `expectTrue(true)`；清空方法体即红。
- 休眠唤醒联动六个调用零断言；改为断言 pause/start 次数（重复事件必须幂等）。
- 真实环境采样只断言 `dimSnap != nil`，而 profiles 就一个 dim——换成引擎契约（一档案一行快照、
  返回值与发布状态一致）。零断言的「真实 sessions 目录信息」用例删除（行为已由临时目录用例覆盖）。
- WorkBuddy bundle id 用例的 `data!` 会让整条 runner 崩溃而非失败；并补上不依赖本机的硬期望
  （国外版必须登记 `com.workbuddy.workbuddy-ai`，不得登记 Application Support 目录名）。
- CLI DTO 往返断言两边一起动，改 `CodingKeys` 键名永远绿——补字面 JSON 键名断言（下游 Raycast/CSV
  按键名取数）。给状态 DTO 注入一次改名，测试如期变红。
- 文件侧失明补 `.unreadableFile` / `.undecodableFile` 两个失败码并落证据：此前「会话文件读不出来」
  与「这个会话没有待确认事项」在岛与 `doctor` 上完全同形，正是 CONTEXT.md 禁止的两态合一。
- 新增 `HardeningTests` 共 10 例；结构类断言在扫不到源码时改为抛错（静默通过等于给自己发假绿证）。
  全部 11 次变异验证逐条做过。测试 317 → 327。

**第二轮复核抓出的一轮修复自身缺陷**
`appleScriptLiteral` 把 0x00–0x1F 之外的控制字符写成 `\u{1b}`——实测 AppleScript **不认这个转义**，
`NSAppleScript(source:)` 返回 nil，而两个调用点都静默跳过，症状恰是该函数声称要消灭的「点了没反应」。
改为控制字符原样透传，并补一条「拼出的脚本必须真能编译」的断言（含 ESC / 换页 / 中文 / emoji /
前导短横线路径）。同轮抓出：重入断言放错函数、`stop()` 收尾判据被自己的修复打断、负缓存丢首会话。

## [0.0.92] - 2026-09-20

### 🎨 语义色收到一处：硬编码色值 186 → 36，等级色阶 5 份副本并成 1 份

上一版把「Theme 外的硬编码色值」钉成 186 的棘轮基线，当时判断是大面积清扫、不划算。
这一版回头量了一遍，结论变了：**这些色值不是 186 个各不相同的颜色，而是少数几支被反复抄写**——
`slate-200` 一支描边色在 17 个文件里抄了 45 次。抄写的代价不是整洁，是漂移：改一次色板要改
几十处，漏掉的那几处只有眼睛能发现，而上一版刚刚证明过我的眼睛看不到全部界面。

- **`Theme.swift` 新增 `Ramp`（Tailwind 基色阶）**：18 支被复用 ≥2 次的基色在此唯一登记，
  整数与 `Color` 成对给出（`slate200Hex` / `slate200`）。`Theme` 自身的动态令牌浅色半边也改引
  这批整数，浅色色板与基色阶从此不可能各说各话。视图层 147 处字面量换成 `Ramp.xxx` 引用。
- **五份 `ActivityLevel` 色阶副本并成一份**：`AgentIslandApp` / `AgentRowView` / `AgentHoverTooltip`
  各有一份**逐字节相同**的浅色 text/fill/border 三段梯子（合计 45 处字面量），`IslandView` 还有
  第四份基础色梯子。现统一为 `Theme` 上的 `color` / `lightText` / `lightFill` / `lightBorder`；
  各调用点只保留自己的外观分支与染色系数（0.12 / 0.14 / 0.18 是组件自己的决定，不并入）。
- **顺带删掉一份真正的死副本**：`IslandView` 里的 `ActivityLevel.label` 与 `AgentIslandCore`
  中同名属性**五支文案完全一致**（Swift 允许 UI 模块的扩展遮蔽 Core 的实现，所以它一直静默存在）。
- **辉光与状态色解耦重复**：`0xffd60a` 原写在 3 处、`0xff3b30` 与 `0x30d158` 各 2 处；
  现收为 `Ramp.neonAmberHex` / `neonRedHex` / `neonGreenHex`，`DockedSliver` 的三处
  `NSColor(hex:)` 改用 `Theme.glowAlert` / `glowWorking` / `glowIdle`。
- **等价性用机器核对，不靠断言**：把新树里每个 `Ramp.xxx` 按其登记的整数还原回字面量，再与
  `HEAD` 逐行比对——剩余差异只有「改设计的那些」，无一处色值变化。唯一的写法变化是
  `Color(hex: X, alpha: A)` → `Ramp.x.opacity(A)`，对纯色而言同义。
- **新测试先抓到我自己的漏网**：`语义色单点` 首跑即报出 5 处漏改（`ModelDonutChartView` 的
  动态色对、`AgentRingView` 的 `Palette`——都是 `dynamicLight:` 形态，不在最初那轮替换的匹配里）；
  修完后又报出 `IslandView` 的第四份色阶。同时反向修正了测试自身的一处误报：事件类型
  （`EventType`，只有 `attention` 无 `working`）的梯子不该算重复。
- **棘轮同步收紧**：`债务棘轮` 色值基线 186 → 36（余下 36 处是一次性强调色与黑/白半透蒙层，
  收进 `Ramp` 只会多出十几支无人复用的色）；新增 `语义色单点` 测试——`Ramp` 基色整数只准出现在
  `Theme.swift`，且等级色阶只准定义一处（解析 `Ramp` 块自动跟随，日后加色无需改测试）。
  两条断言各自做过变异验证：抄一支基色、复制一份梯子，都能让测试红。

- **顺手清空最后的编译告警**：`NotifyCommand` 两处 `[#NoUsage]`（`if let url` 只做存在性判断、
  `let (data, response)` 的 `data` 无人用）——目标产物现在 Swift 告警为 0，下次真告警不会被埋。

## [0.0.91] - 2026-09-20

### 👁 首次目视核对岛内界面，就地修掉一处文案截断

- **目视核对发现了测试永远发现不了的问题**：工作台「监控可信度自查」卡的依据行原文有 40 字，
  在约 370pt 宽的面板里被中段截断成「会话库与…细是设计如此」——读不通，且 `12 项测试全绿` 对此
  完全无感。缩短为「该档案未登记本地明细源，不代表它没在工作」，完整解释留在悬停提示里。
  复验截图确认整行完整渲染。
- **核对方式与边界（如实记录）**：Computer Use 不枚举 accessory（`LSUIElement`）应用，
  按名称与 bundle id 都取不到岛的 AX 树，因此无法点击/键入面板；改用「深链导航 + 全屏截图」
  （终端命令，非 GUI 自动化）。已目视确认：工作台自查卡排版、异常空态、菜单栏微监控、
  岛内列表与后台任务胶囊归属。**未能目视确认**：Token 分析页的热力/环形图（面板在无指针悬停时
  会自动收起，多次抓图为空）、`j/k` 滚动与 `/` 聚焦（需要向面板键入，受同一限制）——
  这三项仍只有测试与源码级验证。
- **回归验证**：全量 316 项自建测试 100% 通过（0 失败）。

## [0.0.90] - 2026-09-20

### 🐛 修掉 CLI 的「用量恒为 0」：没取数不再印成 0，取数改为按需

- **真实缺陷**：常驻引擎的 token 轮询只在「呈现活跃」时开启，一次性 CLI 进程里没人开启它——
  于是 `agentisland status` 每个 Agent 的 24h 用量与费用都印 `0` / `$0.00`，而同一时刻
  `agentisland tokens` 报的是 24h 3.81M / 累计 257.76M。这正是 CONTEXT.md 明令禁止的
  「把没取到呈现成没有」，而且 `status --json` 是脚本与 Raycast 的数据源，错误会被下游当真。
- **修复**：`TokenUsagePolling` 新增 `refreshSync()`（默认转 `refreshAsync()`，测试替身无需改动），
  引擎转发 `refreshTokenUsageSync()`，`LiveSampler.makeEngine(refreshUsage:)` 在采样前同步取回用量。
- **但不把代价强加给所有人**：同步取数要解析全部会话索引，本机实测多花 3.5~5s，对脚本场景不划算。
  所以 `status` 默认**不取数**并把该列印 `—`（明说没查），`status --usage` 才付这笔钱；
  `doctor` / `report` 作为诊断与存档产物默认取数。实测：默认 `status` 0.69s 显示 `—`，
  `--usage` 4.1s 显示 DimAgent 2.85M / WorkBuddy 716.7k。
- **`CLIAgentStatusDTO.tokens24h` / `cost24h` 改为可空**：`nil` = 本轮没去取，`0` = 取到且确实是 0。
  消费方只有 CLI 自身与测试（已核实），JSON 契约因此变得更诚实。
- **顺带消掉一个上一版自己造的假警报**：`doctor` 的「无本地明细」依据原本写着「一次性采样不等待
  异步刷新，未必代表真的为零」——那是在用措辞掩盖取数缺失。真去取数之后，本机该类别从 1 项降到 **0 项**，
  依据文案也回落成一句事实陈述（「同步刷新用量源后仍无记录」）。
- **回归验证**：全量 316 项自建测试 100% 通过（0 失败）。

## [0.0.89] - 2026-09-20

### 🧷 债务棘轮：两处长期债务只准降不准升（统一优化方案 Wave 3）

- **不做大清扫，先止住增长**：架构审计量出 Theme 之外硬编码色值 **186 处**、`UserDefaults.standard` 直读 **37 处**（散在 7 个文件，`SettingsStore` 本该是唯一入口）。两者都没有缺陷史，186 处视觉清扫是拿回归风险换整洁，不划算；真正划算的是让债务不再长。
- **新增 `DebtRatchetTests`**：把当前数量钉成基线，新增一处就让测试失败，失败信息直接写清该用什么替代（Theme 的动态浅/深成对色、`SettingsStore`/`SettingKey` 读写口径）以及「收敛后请把基线数字改小」。基线只能往下改。
- **计数口径的坑顺手记进注释**：`grep -c` 数的是**行数**，一行两处会漏计——基线最初按它定成 165，实际是 186，第一次跑就被自己的测试抓到。
- **已做变异验证**：临时加一处 `Color(hex:` → 测试如期失败（186 → 187），还原后 316 项全绿。
- **回归验证**：全量 316 项自建测试 100% 通过（0 失败）。

## [0.0.88] - 2026-09-20

### 🖥 岛内监控可信度自查卡（统一优化方案 Wave 2）

- **`doctor` 的同一套结论搬进灵动岛**：维护工作台在「用量与运维审计」之下新增「监控可信度自查」卡，逐条列出结论不可信的 Agent（会话源读不到 / 无本地明细 / 未接入明细源），点行直达该 Agent 详情。
- **刻意不新增判断也不重新采样**：卡片只读引擎已经在算的快照、只调同一个 `AgentObservability`。两处各写一份判定的话，「终端说这个 Agent 的待机不可信、岛上却显示一切正常」这种分裂迟早会回来——而卡片存在的唯一理由就是让人相信它说的话。
- **空态如实**：没有存疑项时显示「已核对全部在跑与已装智能体：每条状态结论都有可读的会话或进程证据支撑」，而不是留一片空白让人以为没数据（对齐 CONTEXT.md 的诚实性规则）。
- **未安装不算问题**：`notInstalled` 不进入存疑列表——本来就不该期待它有状态。
- **回归验证**：全量 315 项自建测试 100% 通过（0 失败）。**视觉验证未完成**：Computer Use 需要系统授予「辅助功能 + 屏幕录制」权限，授权窗口尚未确认，因此这张卡的实际排版（截断、间距、对比度）尚未目视核对。

## [0.0.87] - 2026-09-20

### 🔍 自查结论补齐三类真相（统一优化方案 Wave 1）

- **「没接入明细源」不再冒充「读不到数据」**：doctor 此前一次都没引用 token 明细源可用性（实测 grep 0 处），于是档案压根没登记会话库/token 目录的 Agent，与「登记了却本轮读不到」共用同一条结论——正是 CONTEXT.md「Token 数据覆盖」禁止的混淆。新增 `AgentProfile.hasLocalDetailSource` 与结论 `sourceNotWired`，实机效果：原先笼统一片「无本地明细 4」，现在拆成「无本地明细 1（值得排查）」+「未接入明细源 3（属正常形态）」。
- **一次性采样不再把「没赶上」说成「没有」**：CLI 单拍/双拍都不等待 token 监控的异步刷新，`tokenUsage` 恒空——原依据「也没有可读到的用量账本」实际在测量采样器的急躁而不是机器的真相。措辞改为「本轮也没取到用量账本（一次性采样不等待异步刷新，未必代表真的为零）」。
- **只读库不再把陈旧句柄误诊成「对方改了表」**：`ReadonlyDB` 靠 (dev, inode) 判断库是否被替换，而 `VACUUM` / 截断式原地重写**不改 inode**，缓存的旧句柄会被继续复用，随后的 prepare 失败就被定性为「结构已变更」——一条根本不成立的结论。现在 `prepare` 失败先作废连接重试一次，两次都失败才对外上报；真改表仍落到 `prepareFailed`（既有测试守住），并新增 `ReadonlyDB.invalidate(_:)`。
- **孤儿入口变成可用命令**：`Selftest.run()` 与 `Probe.run()` 此前只能靠 `.app` 的隐藏参数 `--selftest` / `--probe` 触发，用户与脚本无从得知。新增 `agentisland selftest`（核心逻辑自检，与 `doctor` 的「这台机器可信吗」明确分工），`--help` 与联动示例同步补齐。
- **回归验证**：全量 315 项自建测试 100% 通过（0 失败）。

## [0.0.86] - 2026-09-20

### 🩺 新增 `agentisland doctor`：把「这个 Agent 是真闲着，还是我根本没看到它」变成一条可查的结论

- **`agentisland doctor`（含 `--json` / `--agent <id|名称>` / `--all` / `--quiet`）**：一次性实况自查，逐 Agent 给出「结论 + 依据」，四类结论计数相加等于总行数便于核对。支持双拍采样拿真实 CPU 利用率（脚本友好的 `status` / `report` 仍走单拍）。
- **`AgentObservability`：两套互不相识的「健康」合成一处**。此前 `AgentHealthEvaluator` 只看卡死/CPU/内存，对**根本没在被监控**的 Agent 直接给 100 分「健康」；`SessionProbeHealth` 说得出「会话库读不到」却只出现在详情页一个 9pt 标签和 Markdown 报告里。现在合并为 `observed / blindSessionSource / noLocalData / notInstalled` 四类，纯函数、可表驱动测试。其中一条规则是实测逼出来的：Antigravity 处于「待确认」却因活跃会话数为 0 被判成「无本地明细」——会话强语义本身就是「源是通的」的证据。
- **失明证据进入结构化输出**：`CLIAgentStatusDTO` 新增 `healthScore` / `healthGrade` / `observability` / `observabilityEvidence`，CSV 追加 `Observability` / `ObservationEvidence` 两列（既有列序不变）。此前「读不到」的证据只存在于 Markdown，脚本与 Raycast 读到的 JSON 会把「读不到」当成「闲着」。
- **一次性采样统一到 `LiveSampler`**：`Probe.run()` 里那套正确做法（热安装缓存 → 真实文件监控预热 → 双采拿 CPU 差分 → `fullRegistry` 含宿主内嵌过滤）此前只挂在 `.app --probe` 上，CLI 够不到，四处各自实现并已分叉。**修掉一个真实可见性缺陷**：`status` 只遍历 `AgentRegistry.builtin`，用户自定义与自动发现的 Agent 在 `status` / `status --json` 里根本不存在，而 `report` 能看到；现在 `status` / `report` / `check` / `doctor` 同口径，默认与灵动岛一致只看启用集（实测 21 项），`--all` 出全集。
- **三份「复制诊断快照」收成一个入口**：右键菜单、工作台卡片与深度链接 `agentisland://export` 各自拼一份写剪贴板，其中**右键菜单那一处漏传了 `history:`** —— 同一个用户动作在两个入口产出两份不同内容且无人报错。现统一走 `DiagnosticsSnapshot`。
- **探测故障时间线落日志**：快照里的探测健康只有 120s 保质期（回答「此刻可信吗」），新增 `ProbeFailureLog` 按 (Agent, 失败类型) 冷却 10 分钟记一条 `AppLog.error`，并在恢复时补一条、重新武装——坏源不会每 2s 刷满日志，但「从什么时候开始读不到」有迹可循。
- **回归验证**：全量 315 项自建测试 100% 通过（0 失败），新增 3 项（可观测性四类互斥表驱动、DTO/CSV 承载证据、故障日志冷却与恢复）。

## [0.0.85] - 2026-09-20

### 🔒 停止后的采样竞态、探测健康保质期与版本号单一来源

- **`stop()` 之后在飞的后台采样不再落地**：`sampleInBackground` 只在入口检查 `running`，保护不了「后台 libproc 遍历进行中、此时 stop()」这段异步间隙——落地的那一拍会发布快照、补发完成/告警事件，并经 `sampleCore → scheduleNext` 把刚被 `invalidate` 的定时器重建回来，等于引擎在停止后自己又活了。现在回到主线程时复检。
- **会话源「读不到」有了保质期**：v0.0.84 的探测健康是最近值，被有意保留以免一次跳拍就擦掉证据，但没有时间戳——源修好之后如果长时间没有新写入（探测被跳过、不再写新值），那条旧的「会话源不可读」会永远挂在卡片与审计报告上，反过来把「读不到」伪装成「坏了」；离线 Agent 此前也无条件参与报告。现在由采样时钟盖章、超过 120s 即回收、离线一律不报，而源重新坏掉时证据会重新落下。
- **版本号回到单一来源**：新增 `AppVersion`（Core）并被 CLI 横幅、导出的 Raycast 清单、设置页回落值共同读取——此前应用已发到 v0.0.84，而 CLI 仍打印 v0.0.80、清单也停在旧版（发版脚本只校验 CHANGELOG 与 README，管不到代码里的字面量）。现在 `scripts/build-app.sh` 在打包前校验 `AppVersion.string` 与 CHANGELOG 首条一致，另有一条测试从源码树读同一份 CHANGELOG 再守一遍，专门兜「绕过脚本手工构建」。
- **回归验证**：全量 312 项自建测试 100% 通过（0 失败）。本轮同时是一次收敛判定：对 v0.0.81–v0.0.84 做了跨版本回归复核（合并冲突处、按 Agent 隔离的告警保护、探测健康、目录剪枝等价性），未发现功能回归；剩余项已评估为收益递减（详见 README 之外的架构债盘点）。

## [0.0.84] - 2026-09-20

### ⚙️ 后台扫描与索引聚合提速，告警隔离与解析失败可见性

- **目录枚举在深度上限真正生效处剪枝**：`FileMonitor.scanTree` 的 `maxDepth` 守卫原先只丢弃 level+1 的条目，枚举器却仍为每个 `level == maxDepth` 的目录 `opendir/readdir` 一轮——纯支出、零收入。实测 `~/.gemini/antigravity/brain` 有 8,716 个 `steps/<n>` 目录正卡在这一层（其 `output.txt` 在 level 5，今天就读不到），趟数 **19,490 → 10,756**，单趟全量扫描 **218ms → 74ms**，`newest` / `activeSessions` / `newestFile` 逐项相等（夹具测试锁死）。刻意**没有**按目录名剪掉 `steps` 子树：那会连 level-4 目录自身的 mtime 一起丢掉，活跃会话判定不再等价。
- **结构化 Token 索引改分桶聚合**：按 (路径, mtime, 长度) 缓存每个文件的合计，未变文件不再参与本轮聚合——首趟 **1,988ms → 稳态 2.69ms**，`usage(now:)` 汇总 **1,002µs → 528µs**，逐工具用量签名与改造前完全一致；只读库探测改 `stat(2)`（26.2µs → 0.6µs/次，且避开 `resourceValues` 的毫秒级陈旧窗口）。
- **成本告警保护窗口按 Agent 隔离**：`alertProtectedUntil` 原是单个全局时间戳，而 `clearLatestEvent()` 会无条件清掉它——用户 dismiss 掉 B 的横幅，等于顺手解除了 A 正在生效的激增保护，A 可以立刻再告警。现改为按 agent id 记录，并接入 `resetTracking` / `resetAllTracking` / `retainTracking` 三个既有生命周期入口。
- **「解析器坏了」不再伪装成待机**：只读库打不开、`sqlite3_prepare` 失败（第三方应用改了 schema）、会话文件超上限这些路径原先一律返回 `nil`，UI 显示「待机」，用户没有任何线索。现在失败原因作为 `SessionProbeHealth` 随探测结果一起带出（沿用 v0.0.81 那条「不放进全局字典」的隔离原则），在诊断快照与详情页的既有状态位上如实呈现；「库里确实没数据」与「这个源读不到」重新分开，兑现 CONTEXT.md 的诚实性规则。
- **SQL 扫描合并**：DimAgent 的 24h 与累计两趟 `usage_ledger` 全表扫合并为单趟。
- **一处被证伪的优化没有做**：`workbuddy` 的 `ORDER BY updated_at DESC LIMIT 1` 不改用 `max(rowid)`——本机真实库即可反驳（rowid 5 比 rowid 1 早 10.6 天，因为行会被就地 UPDATE），且代价也没有可省的（`LIMIT 1` 让临时 B 树只存一行，实测 0.16µs，瓶颈是全表扫本身而 `updated_at` 无索引是第三方库结构决定）。证据写进注释，避免后人再试一遍。
- **回归验证**：全量 310 项自建测试 100% 通过（0 失败）。

## [0.0.83] - 2026-09-20

### 🩹 采样正确性、界面诚实性与键盘流补全（多智能体并行审计轮）

- **用户中断不再把 Agent 永久钉在 working**：Ctrl-C / 点停止不会补一条 `tool_result`，这条僵尸调用会让会话尾窗每拍谎报工作态，于是引擎的完成与待机分支永远走不到，2s 快采样与高频全树扫描一并被锁死（CPU 与耗电双输）。现在中断之前的在途调用一律撤销，中断之后重新发起的命令仍继续是在途判定。
- **时钟回拨后激增检测不再整段失明**：重锚 token 速率基线时只改了局部副本，窗口不足即 `continue`，导致未来数小时每一拍都重复走同一条重锚分支；改为立即写回（时区错 8 小时就是盲 8 小时）。
- **进程抖动不再重复弹通知**：libproc 抖一拍或 CLI 换 PID 重启到同一份会话，原先会抹掉通知去重指纹，让同一条「等待确认」重新弹窗加铃声；指纹的清理现在只发生在显式终止与档案移除。
- **采样缓存三处修正**：定位缓存的失效令牌改用 `stat()`（`URL.resourceValues` 有毫秒级陈旧窗口，同类问题曾在尾读合并上实测误命中）；TTL 由 3s 提到 10s 以避开与 2s 拍频打拍（Antigravity 一趟定位 24ms，原先约一半的拍仍在付费），命中时时间戳只向前修正以免 `fileAge` 因缓存虚高；尾读合并由单槽扩到四槽——两个以上 Agent 同时工作会互相踢出缓存，命中率归零。
- **专有方言不再每拍重复解析**：Antigravity / DSH / Cline 的动作文案与会话强语义同源（同一份 transcript / 投影缓存），直接复用本拍已探得的结果，省掉第二遍 262KB 尾读 + 120 行 JSON 解析。
- **界面诚实性**：热力图在无明细时不再合成一整排全零格子并显示「活跃 0/24h」，改为显式的「本时段未发现 Token 明细」（兑现 CONTEXT.md 里「未发现明细 ≠ 用量为 0」这条硬规则）；模型环形图 `prefix(4)` 静默丢弃的模型现在给出「其余 N 个 · xx%」与总计行，占比不再对不上。
- **破坏性动作与键盘流**：清空事件历史补二次确认（与删除档案、终止进程同一套模式）；`j/k` 焦点移动现在会把目标行滚动进可视区（超过 6 个 Agent 时此前键盘流是静默失效的）；`/` 现在真正聚焦搜索框，不必再用鼠标点一下。
- **搜索谓词单一来源**：列表过滤与键盘焦点集合此前各写一份谓词，可能互相不一致，现共用同一入口并有测试守护。
- **可访问性与设计令牌**：模型环形图、热力图、事件历史、进程树四张自绘卡片给 VoiceOver 各补一句可读摘要；纯图标按钮补标签；图表调色板回到 `Theme` 的动态浅/深成对色，低于项目自身 9pt 下限的字号全部抬回；`HH:mm:ss` 三处缺失 POSIX locale 的格式化器补齐（系统区域设置会改变定宽区里的数字与分隔符）。
- **档案数据补全**：`tokenRoots` 成为档案字段，Token 统计里最后几处 `~/...` 字面量（`~/.workbuddy*/projects`、`dimcode` 会话目录）回到注册表唯一声明，注册表自洽测试同时守住方言、库位置与 token 根目录三类漂移。
- **回归验证**：全量 301 项自建测试 100% 通过（0 失败），新增 15 项（含界面呈现层 12 项与采样正确性 3 项）。

## [0.0.82] - 2026-09-20

### 🏗 能力收进档案：路径漂移消除与通用会话解析再提速

- **`detect(lines:)` 由每行约 13 趟全树递归合并为单趟事实收集**：
  - 实测 96 行尾窗单拍 **7.61ms → 2.89ms**，其中 JSON 解析只占 0.27ms——即改造前 96% 的开销是把同一棵已解析好的树反复走完；
  - 新增 `LineFacts` 一次遍历产出请求标记、完成标记、工具调用与结果、`role/source/type/status/state` 集合与标识符，键值在每个节点只读一次并归一化；`identifier(in:)`（12 次键查找 + 下钻 `data`/`payload`）由每节点最多 3 次降为 1 次；
  - 判定顺序、DFS 首个命言语义与解除等待确认的条件均与原实现逐条对齐，287→286 项既有测试（该解析器是全仓测试最密处）零回归；
  - 删除被取代的 9 个辅助函数（`findRequest` / `findCompletion` / `resolvesAttention` / `isResolutionRecord` / `startsOrContinuesWork` / `structuralValues` / `identifiers` / `extractToolCalls` / `extractToolResolutions`）。
- **Agent 能力改为档案声明（见 ADR-0004）**：新增 `emoji`、`sessionDialect`、`sessionDatabase` 三个档案字段与 `AgentRegistry.profile(_:)` / `databasePath(for:)` 取值入口：
  - 同一批会话库路径此前在解析器、动作探测、日志流、Token 统计里各硬编码一份（`~/.dimcode/v2/dimcode.sqlite` 出现 4 次且完全不在注册表里），现由注册表唯一声明，日志流与 Token 统计一律向注册表取值；
  - 会话探测与库查询由 `switch profile.id` 改为按**方言/schema 穷尽分派**——格式数量远少于 Agent 数量（Cline 与 Roo Code 同源），新增复用既有格式的 Agent 只需登记档案；
  - 删除 CLI 状态表的 19 分支 id→emoji 梯子，其中 4 个 id（`dimagent` / `vibe` / `ima.copilot` / `egobrowser`）在档案改名后已不存在，只会静默退化成默认符号；
  - 新增「注册表自洽」测试：声明专有方言却没有对应会话目录的档案直接测失败（线上表现为该 Agent 永远只显示待机）；字段全部带默认值并走 `decodeIfPresent`，升级前存档的自定义 Agent 照旧可解。
- **日志流与会话探测共用注册表路径**：`AgentLogStreamer` 的 4 处库路径与 Antigravity `brain` 目录改为向注册表取值，`workbuddyDataDir` 这一中间层随之退役；`detectAntigravitySession` 的上下文出口由全局字典改为参数。
- **DimAgent CLI 名补登记**：`knownCLIs` 原本只认 `dim`，而其发行版可执行名同样有 `dimcode`——开启「自动发现」时 `dimcode` 会被当成一个陌生 CLI，与 DimAgent 重复成一行。
- **回归验证**：全量 286 项自建测试 100% 通过（0 失败）。

## [0.0.81] - 2026-09-20

### ⚡ 采样热路径主线程 I/O 治理与跨 Agent 上下文隔离

- **消除采样主线程的会话树递归遍历（实测单拍省约 84ms）**：
  - Antigravity 与 DSH 的专有会话探测此前每拍在 `@MainActor` 上重新枚举整棵会话树——真实机器实测 Antigravity 797 次 stat / 24ms、DSH 501 次 stat / 15ms，而「会话强语义」与「当前动作文案」两条链路各走一遍；
  - 新增**带失效令牌的会话定位缓存**（TTL 3s + 根目录 mtime 变化即刻重定位）：缓存的只是「选哪个文件」，尾读与解析每拍照常进行，因此新会话零延迟、状态转移无任何滞后；
  - 新增**尾读合并**：262KB 尾部读取（3.7ms）比 120 行 JSON 解析（2ms）更贵，同一文件在 mtime 与长度未变时复用上一次的行（有效期 0.5s，短于采样周期）；新鲜度取 `stat()` 而非 `URL.resourceValues`——后者有毫秒级缓存窗口，恰好会在这两条链路的微秒间隔内误命中；
  - 实测稳态单拍开销：Antigravity 4.6ms、DSH 0.2ms（改造前两条链路合计约 91ms）。
- **修复跨 Agent 上下文串味**：`AgentSessionSignal.backgroundTasks / subagents / tokenBreakdown` 长期硬编码读取按 `"antigravity"` 索引的全局上下文，导致 Claude、Codex 等**任意 Agent 的卡片与终端看板都会显示 Antigravity 的在途子任务、后台命令与 Token 细分**；上下文改为随 `AgentSessionProbe` 显式返回，按 agent id 索引的全局字典（`clearActiveContext` 零调用点、残留永不失效）整体删除。
- **大会话文件读取护栏**：Cline `ui_messages.json` 与 DSH 投影缓存的整块 `Data(contentsOf:)` 改为内存映射并加 32MB 上限，超限放弃本轮解析、降级到双信号判定，不再让无上限增长的日志拖垮采样。
- **状态重置收敛**：10 个按 agent id 索引的滞回/去重/告警基准集合的清空逻辑原先散落 5 处，且 `terminateAgent` 与 `cleanAnomalies` 确实漏清了 `tokenRateBaseline`（杀掉进程后首个结算窗口把离线全程计入分母，速率被摊薄、激增告警被推迟）；收敛为 `resetTracking(for:)` / `resetAllTracking()` / `retainTracking(for:)` 三个入口。
- **死配置清理**：删除 `predictiveAnalyticsEnabled`、`localEventServerEnabled`、`localEventServerPort` 三个声明后从未被读写的设置键。
- **高覆盖率回归验证**：新增 3 项测试（跨 Agent 上下文隔离、TTL 内新会话立刻重定位、等长改写立刻读到新内容），全量 286 项自建测试 100% 通过（0 失败）。

## [0.0.80] - 2026-09-19

### 🚀 多 Agent 在途命令感知、Antigravity 子任务树与 Token 细分、悬浮胶囊与菜单增强

- **多 Agent 在途命令（Bash / exec_command / say command）全生命周期闭环感知**：
  - **Claude Code 终端命令感知**：深度解析 `tool_use`（如 `Bash`）生命周期，在命令未交付 `tool_result` 前严格拦截 `completed` 终态，并实时抽取展示所执行的命令；
  - **Codex 函数调用感知**：精准感知 `exec_command` 及参数，在终端长命令执行中保持活跃工作态；
  - **Cline & Roo Code 任务流解析**：解析 `ui_messages.json`，将 `ask: command` 识别为等待批准操作（`.attention`），`say: command` 识别为活跃执行（`.active`），并在完成消息交付后准确恢复为 `.completed`。
- **Antigravity 专有深度分析能力升级**：
  - **子智能体树（Subagent Tree）全链路追踪**：提取 `invoke_subagent` 调用及其角色、模型与状态，建立子智能体生命周期映射；
  - **Token 深度细分指标拆解（TokenBreakdown）**：实时抽取 Prompt、Completion、Cached Read、Cache Write 与 Thoughts/Reasoning Token 细分数据，并在详情卡片直观呈现；
  - **任务取消与完成安全识别**：联动 `manage_task` 取消/终止指令与子智能体应答完成标记，杜绝死锁与虚假活跃。
- **灵动岛 UI 与上下文交互全面增强**：
  - **行内活跃胶囊（Capsules）与悬停 Tooltip**：智能体行内动态呈现 `⚡ 后台N` 与 `🤖 N子任务` 微胶囊，悬停展示在途任务命令详情；
  - **右键上下文菜单（Context Menu）扩展**：支持快捷吸附切边（上/右/下/左）、切换深浅外观、通知分级模式、静音/开启完成提示音与复制当前诊断快照；
  - **CLI 终端看板（`status` / `top`）同步升级**：直观高亮展示后台任务与子智能体状态标识。
- **高覆盖率回归验证**：
  - 新增 `MultiAgentAdvancedTests` 专有测试套件，全量 283 项自建测试 100% 通过（0 失败）。

## [0.0.79] - 2026-09-19

### 🎯 深度优化 Google Antigravity 运行感知与后台任务生命周期跟踪

- **彻底消除后台任务执行中“已完成”误判与弹窗/铃声轰炸**：
  - 深度剖析 Antigravity 异步任务机制（`run_command` 终端构建/长命令、`schedule` 定时器、`invoke_subagent` 子任务等）；
  - 当模型启动后台任务并输出中间等待消息时，严格拦截 `PLANNER_RESPONSE` 默认转换为 `completed` 的行为，精准保持 `working` 活跃工作状态；
  - 只有当所有已发起的后台任务完全交付（或被明确取消/终止）、且智能体产出最终用户响应时，才触发 `.completed`。
- **实时任务元数据提取与友好动态文案展示**：
  - 自动解析正在运行的后台命令（如 `swift build`、`pytest`）、定时等待时长（如 `后台定时中 (10秒)`）及子任务角色；
  - 清洗并过滤 `SDKROOT=...`、`SKIP_TESTS=...` 等过长环境变量前缀与 `arch -arm64 ` 指令包装，灵动岛与菜单栏文案更清爽直观；
  - 结合任务执行日志（`.system_generated/tasks/*.log`）写入更新时间戳，保障长时间无主转录写入时的感知保鲜。
- **回归测试套件全面覆盖**：
  - 新增 5 项专用测试用例覆盖单个/多个后台任务拦截、生命周期交付、任务取消与动作文案清洗，全量 274 项测试保持 100% 通过。

## [0.0.78] - 2026-09-19

### 🖥️ 终端交互动态看板、热门 Agent 生态扩充、快捷键速查 HUD 与 Raycast 导出

- **CLI 类似 htop 交互式动态监控看板（`agentisland top` / `status -w` · 方案 A）**：
  - 基于 ANSI 光标与清屏转义序列构建轻量全屏看板，支持自定义刷新周期（`-i <sec>`，默认 1.5s）；
  - 动态展示监控智能体总数、工作态占比、总 CPU% 与 24h Token/费用走势，高频刷新各智能体明细表格；
  - 终端 Raw 模式单字符交互：支持 `q` 退出并恢复终端光标、`r` 立即重采刷新、`c` 快速清理死锁与孤儿后台。
- **新兴 AI 编码智能体深度生态扩展（方案 B）**：
  - 内置增加 **Cline**（开源自治 Agent，监控 `saoudrizwan.claude-dev/tasks` 会话目录与进程）；
  - 内置增加 **Roo Code**（高频迭代分支 Agent，监控 `rooveterinaryinc.roo-cline/tasks` 与 `roo` 进程）；
  - 内置增加 **Continue.dev** 增强进程识别（支持 `continue-core`）；
  - 内置增加 **Goose**（Block 开源自治 CLI 智能体，监控 `~/.config/goose/sessions` 与 `goose` 进程）。
- **展开态快捷键速查指南 HUD（方案 C）**：
  - 展开卡片下按下 `?` 键，平滑呼出精美磨砂玻璃质感快捷键速查面板（`ShortcutHUDView`）；
  - 完整呈现 `1~3` 切页、`j/k` 焦点导航、`Enter` 下钻、`/` 搜索、`Esc` 逐级退出与 `⌘R` 刷新；
  - 再次按 `?`、`Esc` 或点击任意外部空白区域即刻淡出，无门槛驾驭全键盘流。
- **多格式数据报表与 Raycast 扩展清单导出（`agentisland report` · 方案 D）**：
  - `agentisland report` 扩展支持 `--format <markdown|csv|json|raycast>`（简写 `-f`）；
  - `--format csv`：生成标准逗号分隔数据表，包含时间戳、ID、名称、状态、PID、CPU、内存、健康评分与 24h/累计 Token 费用；
  - `--format raycast`（或直接运行 `agentisland raycast`）：导出适配 Raycast 扩展的命令清单配置，一键映射灵动岛协议联动。

## [0.0.77] - 2026-09-19

### 🚀 多屏热插拔自愈、本地 Webhook 外部事件接收器、Token 预算进度管控与全键盘流

- **多显示器协同与屏幕热插拔自适应（方案 A）**：
  - `IslandPanelPositioning` 屏幕有效性断连自愈：新增 `isValidScreen` 校验，当外接显示器断开拔出时，自动平滑回退至有效屏幕，杜绝窗口悬空或坐标越界；
  - 统一 `snapToDockEdge` 与 `placeWindow` 的目标屏幕计算口径，多屏跟随模式（`followMouse` / `mainScreen` / `builtInScreen` / `externalScreen`）全面提升健壮性。
- **本地零依赖 Webhook / IPC 外部事件接收器（`LocalEventServer` · 方案 B）**：
  - 基于 Apple 原生 `Network.framework`（`NWListener`）构建轻量本地 HTTP 监听服务，仅绑定 `127.0.0.1:41999`；
  - 支持通过 `POST /notify` 接收外部 JSON 载荷（`agent`、`type`、`message`、`detail`），毫秒级触发灵动岛微窥横幅、提示音与系统通知；
  - 原生 CLI 子命令 `agentisland notify`：支持命令行 `-a <agent> -t <type> -m <message>`，优先 HTTP 推送，服务未就绪时自动回退至 URL Scheme 分发。
- **智能体用量预算与成本警戒系统（方案 C）**：
  - 终端 CLI 工具 `agentisland tokens` 支持 `--budget <num>`（如 `--budget 1m` / `-b 500k`）或自动读取系统设置中的预算配置；
  - 终端字符级进度条呈现：`[██████████░░░░░░░░░░] 52% (520k / 1.0M)`，根据用量比例自适应切换色彩（正常绿/警告黄/超额红）；
  - `--json` 输出结构中增加 `dailyBudget`、`budgetRatio` 与 `budgetStatus` 字段。
- **展开态全键盘流操作体验（方案 D）**：
  - 数字快捷键 `1` ~ `3` 快速切页（`1`: 主列表，`2`: Token 用量分析，`3`: 维护工具箱）；
  - 支持 Vim 风格键位 `j`（下移）/ `k`（上移）聚焦选择智能体，配合 `Enter` 快速下钻详情；
  - 保留 `/` 搜索呼出与 `Esc` 逐级返回或收起面板。

## [0.0.76] - 2026-09-19

### 💻 原生终端命令行工具（agentisland-cli）与终端运维生态

- **轻量原生 CLI 二进制（`agentisland`）**：
  - 零额外三方依赖，直接基于 `AgentIslandCore` 构建编译，发布随附 `dist/agentisland` 及 `.app/Contents/MacOS/agentisland`；
  - 自动感知终端环境（TTY vs Pipe/Redirect），支持高对比度 ANSI 彩色排版与管道纯文本无缝回退。
- **快照总览与结构化输出（`agentisland status`）**：
  - 终端精美表格列出所有 Agent 状态（工作态 🟢 / 待机 🟡 / 离线 ⚪️）、PID、CPU%、内存、会话数、24h 用量及最近活动；
  - 支持 `--json` 格式化导出全量结构化 DTO，便于与 Raycast / Alfred 脚本无缝集成；支持 `--all` 包含未运行离线项。
- **Token 消耗分析与月末走势（`agentisland tokens`）**：
  - 汇总打印最近 24h 与历史总计 Token 及支出费用，展示各大智能体消耗排行；
  - 集成 `TokenForecastEvaluator`，一秒推算当月末消耗预期与预算耗尽风险。
- **死锁与异常进程诊断排查（`agentisland check`）**：
  - 自动探测长时间死锁卡顿、终端断开孤儿后台（ppid=1）与高内存泄漏（>2GB）目标，清晰列出诊断原因与处置安全性。
- **智能体一键清理释放（`agentisland clean`）**：
  - 支持 `--dry-run` 预览拟终止目标与预计释放内存；默认安全批处理模式，支持 `--force` 强行彻底清理。
- **深度链接呼出与交互（`agentisland open`）**：
  - 终端一条命令触发桌面灵动岛展开/收起、直达 Token 分析（`analytics`）、工作台（`toolbox`）或指定智能体卡片（`agent <id>`）。
- **Markdown 运维审计报告生成（`agentisland report`）**：
  - 快速生成格式化 Markdown 审计报告，支持 `--copy` 直接入系统剪贴板，支持 `--output` 保存至本地文件。

---

## [0.0.75] - 2026-09-19

### 🔗 URL Scheme 深度链接、能耗自适应、日志智能特征识别、月末成本预测与异常自愈守护

- **URL Scheme 深度链接路由（URLSchemeRouter）**：
  - 注册并解析 `agentisland://` 系统协议，支持与 Raycast、Alfred、macOS 快捷指令及脚本终端无缝联动；
  - 覆盖展开/收起切换（`toggle` / `expand` / `collapse`）、直达智能体详情（`agent?id=<id>`）、打开用量图表（`analytics`）、打开工作台（`toolbox`）、静默清理（`clean`）与导出审计报告（`export`）。
- **硬件电池与电源节能自适应（PowerSourceMonitor）**：
  - 实时监听 IOKit.ps 供电状态与系统低电量模式（Low Power Mode）；
  - 笔记本处于电池供电时自动平滑降频后台采样（工作态保底 3s、闲置 10s、离线 120s），接通电源后满血恢复，显著降低差旅与移动办公耗电。
- **实时流水错误智能特征识别与高亮（LogPatternAnalyzer）**：
  - 流水解析引擎自动模式识别 API 429 限流、编译构建失败、Git 分支冲突、鉴权认证失效及环境配置缺失；
  - 在事件行直观呈现红色错误徽标，支持分类筛选「异常/报错」及一键复制提取到的核心报错摘要与错误代码。
- **Token 走势与月末预算预测（TokenForecastEvaluator）**：
  - 基于最近 24 小时真实消耗速率与当月剩余自然日，推导月末预估总 Token 与费用支出；
  - 联动每日预算上限，在预算超额时主动推导预算枯竭天数并标记红色/橙色预警。
- **死锁与异常驻留自愈守护（AgentResilienceGuard）**：
  - 持续追踪智能体异常长死锁（≥3分钟）或内存剧增泄露（≥5分钟），结合冷却防抖机制主动发出带终止逃生通道的自愈处置横幅，防患于未然。

---

## [0.0.74] - 2026-09-19

### 🔊 交互音效与触觉、进程树全景、会话速览、多屏自适应与审计报告导出

- **原生交互音效与触觉反馈（SoundEffectsManager）**：
  - 支持任务完成提示音自定义（Glass 水滴 / Pop 微泡 / Ping 叮咚 / Blow 柔和 / 静音），提供设置页即时试听；
  - 熔断告警音效升级（Sosumi 敲击 / Basso 低沉 / Funk 强烈 / 静音）；
  - 接入 macOS 触控板微触觉反馈（NSHapticFeedbackManager），任务完成、熔断告警及贴边吸附时提供细腻物理感知。
- **派生工具与子进程全景树（ProcessTreeInspector）**：
  - 基于 libproc 进程表递归探查 Agent 派生的下游子进程树（如 node、git、python3、ripgrep、cargo 等）；
  - 在 Agent 详情页展示子进程层级、PID、分项与整树聚合 CPU / 内存占用，快速识别究竟是 Agent 自身还是下游子任务在吃系统资源。
- **会话速览与快捷接续（RecentSessionNavigator）**：
  - 会话列表中支持一键复制 Session ID，以及「在终端中接续」快速定位至该会话工程目录；
  - 提升跨项目、跨会话恢复上下文的切换效率。
- **多显示器自适应停靠策略（Multi-Monitor Follow Mode）**：
  - 新增屏幕停靠策略配置（跟随鼠标所在屏幕 / 固定主屏幕 / 优先外接显示器 / 优先内置屏幕）；
  - 彻底解决多屏与外接 4K 拓展坞场景下灵动岛无法随工作焦点自适应吸附的痛点。
- **运维与用量审计报告导出（AuditReportExporter）**：
  - 在工作台一键生成 Markdown / CSV 格式的完整审计报告；
  - 涵盖所有智能体在线状态、PID、CPU/内存、健康评分诊断及 24h/累计 Token 计费总览，方便向团队汇报与量化归档。

---

## [0.0.73] - 2026-09-18

### 🛡️ 智能体健康诊断评分、IDE 一键直达、模型环形图、历史时间线与菜单栏微监控

- **智能体健康度与稳定性诊断评估器（AgentHealthEvaluator）**：
  - 100 分制结构化评估运行中 Agent 的稳定度，综合死锁卡死倾向、CPU 高负载与物理内存 RSS 增长斜率；
  - 详情页性能卡片直观呈现「健康分 / 评级徽章 / 自愈建议」，帮助开发者及时发现隐性卡死与长会话内存泄露。
- **常用 IDE/代码编辑器一键直达（Editor Jumper）**：
  - 在当前工作区（CWD）卡片中自动探测本机已安装的 VS Code、Cursor、Windsurf、Xcode 等开发工具；
  - 一键在目标编辑器中快速打开当前工程目录，彻底免去手动切终端与切项目的繁琐操作。
- **模型 Token 消耗环形占比图（ModelDonutChartView）**：
  - 在 Agent 详情页按模型拆分模块新增现代化环形图（Donut Chart），直观呈现多模型配比与份额（如 Sonnet vs Haiku vs GPT-4o）；
  - 悬停扇区联动高亮各模型百分比与消耗量。
- **任务与告警事件历史时间线（EventHistoryPopoverView）**：
  - 引擎内置有界事件队列（保留最近 25 条历史任务），并支持一键清空；
  - 展开卡片顶栏新增时钟历史图标，点击秒级弹出历史时间线，随时回溯今日跑完了哪些任务、单次耗时及历史警报。
- **菜单栏系统顶栏动态微监控（MenuBarBadgeMode）**：
  - 设置页支持自定义系统菜单栏图标附加显示模式（仅图标 / 活跃任务数 / 今日 Token 消耗）；
  - 抬眼即可在 macOS 顶栏一瞥全局并发与开销，无需每次展开灵动岛。

---

## [0.0.72] - 2026-09-18

### 🌟 Token 预算预警、全局热键、活动热力图、紧凑排版与工作区直达

- **Token 消费预算预警与超额封顶（TokenBudgetTracker）**：
  - 支持在偏好设置中设定每日 Token 消费限额（100k ~ 10M / 天），引擎逐拍智能评估用量；
  - 消耗达 80% 时触发黄色预警，达 100% 触发红色超额告警横幅与呼吸光效，防止后台 Agent 死循环烧钱；
  - Token 分析页增加生动的「今日预算进度条」，直观展示用量百分比与剩余额度。
- **系统级全局热键秒级呼出/收起（GlobalHotKeyManager）**：
  - 基于 macOS Carbon 原生内核实现全局热键（默认 `⌥ A`），免辅助功能权限且跨全屏应用秒级响应；
  - 按下热键时自动识别当前光标所在的活动屏幕，置顶展开灵动岛或平滑收起。
- **24 小时协同活动节律热力图（ActivityHeatmapView）**：
  - 在 Token 用量分析页中新增 24 小时结对编程活跃矩阵（类似 GitHub Commit Heatmap）；
  - 24 格色块根据每小时真实 Token 消耗梯度渐变渲染，悬浮秒显具体时段与用量数据。
- **高密度紧凑视图排版模式（Compact Density View）**：
  - 设置页提供「紧凑排版密度」开关，针对 13 寸小屏 MacBook 或多 Agent 并发场景微调行高、字号与指示环尺寸；
  - 减少滚动操作，单屏容纳 Agent 数量显著提升。
- **工作区与终端一键直达（Quick Workspace & Terminal Jumper）**：
  - 使用 macOS 内核 `proc_pidinfo(PROC_PIDVNODEPATHINFO)` 直读运行中 Agent 的真实工作目录（CWD），微秒级响应且零子进程开销；
  - 在 Agent 详情页清晰呈现「当前工作区」，并提供「打开终端」与「在访达中显示」一键直达按钮。

---

## [0.0.71] - 2026-09-18

### ⚡️ 任务效能洞察、列表即时搜索、报表导出、外接多屏与极简纯净模式

- **任务耗时与效率统计洞察（TaskDurationTracker）**：
  - 新增任务执行效能分析引擎，记录各 Agent 任务时长、完成次数与历史极值；
  - Agent 详情页集成「任务耗时与效率」卡片，直观展现 24h 工作总时长、单任务平均用时及单次最长用时。
- **主列表即时搜索与快速过滤（Quick Search & Filter）**：
  - 展开卡片按快捷键 `/` 或点击顶栏放大镜秒级呼出行内搜索输入栏，按 Agent 名称或 CLI 命令快速过滤；
  - 过滤状态下无缝支持键盘方向键移动与 `Enter` 直达，按 `Esc` 智能退出搜索。
- **Token 消费与用量账单报表导出（TokenReportExporter）**：
  - 在 Token 用量分析页一键导出当前统计周期（今日/本周/本月）的精美 Markdown 表格或标准 CSV 账单；
  - 支持直接一键复制到剪贴板并带有「已复制」微动画反馈，方便开发者团队报销与成本归档。
- **多显示器智能跟随与热拔插重排（Multi-Screen Smart Placement）**：
  - 优化屏幕自适应定位机制，外接显示器断开拔掉时智能回退到主屏或包含光标的有效可用屏幕，防止灵动岛悬空在无效坐标；
  - 响应屏幕参数改变通知自动重新校准吸附。
- **极简纯净模式（Hide Docked Sliver）**：
  - 设置页新增「极简纯净模式」开关，常态收起时可完全隐去屏幕边缘 6pt 微细条，满足极简桌面需求；展开或有重要提醒横幅时正常平滑呈现。

---

## [0.0.70] - 2026-09-18

### 🚀 智能计费估算、全键盘穿梭、绿色节能感知与性能预设

- **智能 Token 计费估算引擎（TokenCostEstimator）**：
  - 为 DimAgent、Codex 及大量本地未上报消费金额的日志和会话，基于官方最新费率表（Claude 3.5/3.7 Sonnet、Haiku、Opus，OpenAI GPT-4o、o1、o3-mini，DeepSeek V3/R1，Gemini 2.0/2.5 Flash 与 Pro 等）提供加权混合参考估算；
  - 在 Agent 详情页概览卡片、模型列表行与 Token 分析页中智能呈现带 `~` 标识的估算费用与徽标，底层真实 DB 零污染。
- **全键盘极速交互与快捷键导航（Full Keyboard Navigation）**：
  - 展开卡片时支持键盘极速穿梭：`Esc` 智能逐级返回（二级页面返回主列表，主列表返回收起）；
  - `↑` / `↓` 方向键在主列表中平滑移动焦点并渲染青色微光外框；
  - `Enter` / `Return` 直达选中的 Agent 详情页；
  - 全局支持 `⌘R` 立即采样刷新、`⌘,` 快捷呼出偏好设置面板。
- **MacBook 电池与绿色节能感知自适应调度（Eco Battery Awareness）**：
  - 深度集成 macOS 原生低电量模式（`ProcessInfo.isLowPowerModeEnabled`）与 `NSProcessInfoPowerStateDidChange` 通知监听；
  - 开启低电量模式时，引擎自动将闲置采样间隔延展至 10s、全离线至 120s，在保证灵敏度的同时大幅降低唤醒与电池消耗。
- **设置页性能预设档位与体验重构（Performance Presets）**：
  - 引擎与性能设置增加「⚡️ 极速灵敏 / ⚖️ 平衡标准 / 🍃 极致省电」一键档位切换；
  - 增加一键「恢复默认」快捷重置功能；
  - 实时联动展示「系统低电量模式已开启」绿色节能指示。

---

## [0.0.69] - 2026-09-17

### 💎 优雅、高效、精简的架构去重与单一职责重构

- **视图层组件抽象与统一黑曜石卡片修饰符（IslandComponents）**：
  - 提取统一的 `.obsidianCardStyle(cornerRadius:fill:)` 与 `.subtleCardStyle(cornerRadius:fill:)` 视图修饰符，封装标准高光渐变边框、多阶阴影与深浅色黑曜石底色；
  - 彻底重构 `DetailViews`、`TokenAnalyticsView`、`ToolboxView` 中 8 处重复手写的 RoundedRectangle/strokeBorder/shadow 样板代码，消除大量样板并保证全应用设计系统 100% 一致。
- **通用分类筛选胶囊组件（FilterCapsuleBar）**：
  - 提取通用的 `FilterCapsuleBar<Item: Identifiable & Equatable>`，接管分类筛选横向滚动、弹簧触控动画、高亮选框与计数徽标；
  - 替换 `LiveLogStreamView` 与 `ToolboxView` 中完全一致的胶囊筛选栏，实现关注点收敛与组件复用。
- **控制器单一职责拆分与解耦（IslandPanelController Modularization）**：
  - `IslandPanelRouting.swift`：承载 `CardRoute` 路由导航与页面回溯栈（`openAgentDetail`, `closeAgentDetail`, `openLiveStream`, `closeLiveStream`，以及快捷导航 API）；
  - `IslandPanelPositioning.swift`：聚焦窗口几何计算、吸附动画、屏幕可用区域约束与边缘热区几何（`placeWindow`, `snapToDockEdge`, `dockTargetFrame`, `sliverRect` 等）；
  - `IslandPanel.swift` 主控制器大幅瘦身近 250 行，专注保留窗口生命周期管理、代理回调与核心交互状态派发。

---

## [0.0.68] - 2026-09-17

### 🌟 深度多视角多维度优化：前沿生态扩充、趋势图交互游标与主卡原生右键菜单

- **前沿主流 AI 编程智能体原生识别（AgentRegistry）**：
  - 内置支持 **Windsurf**（Codeium 旗下新一代 Agentic IDE，`com.exafunction.windsurf`，完整映射其工作区与进程家族）；
  - 内置支持 **Aider**（最流行终端 CLI 智能体，`aider`，支持会话跟踪与即装即用）。
- **Token 趋势图交互式游标与精确时段浮标（TokenAnalyticsView）**：
  - 在趋势折线图接入连续悬停（`.onContinuousHover`）与拖动手势（`DragGesture`）；
  - 动态渲染贯穿参考线与焦点光圈，毫秒级浮动展示时段起止与用量微徽标（例如 `15:00–16:00 · 45.2k`），让数据洞察精准到刻度。
- **macOS 原生 ContextMenu 右键菜单生态（AgentRowView）**：
  - 主卡列表行接入原生右键上下文菜单：
    - 🚀 **直达窗口 / 终端**：快速置顶激活运行中的应用或控制台；
    - 📊 **查看模型与 Token 详情**：一键深入多模型与消耗拆解；
    - 📜 **查看实时流水抽屉**：直达实时工具调用与输出事件；
    - 📁 **在访达中显示会话数据**：若存在 session 存储目录直达定位文件；
    - 📋 **复制进程 PID 与 Agent 名称**：极大方便开发者排查终端与脚本任务；
    - 🛑 **强制终止此进程 (逃生舱)**：高负荷卡死时直观退出。

---

## [0.0.67] - 2026-09-17

### 🚀 深入多角度优化：系统休眠节能与热唤醒、日志分类与自动跟踪、工具箱维度筛选

- **系统休眠节能与即时热唤醒（Sleep & Wake Lifecycle）**：
  - 接入 `NSWorkspace.willSleepNotification`：当 Mac 盒盖或休眠时彻底冻结采样定时器与令牌轮询，防止后台无谓唤醒与电池消耗；
  - 接入 `didWakeNotification` 与 `screensDidWakeNotification`：从休眠或锁屏唤醒时毫秒级恢复定时器并立即在后台派发热同步，瞬间对齐最新 Agent 状态。
- **实时日志分类筛选与自动跟踪（LiveLogStreamView）**：
  - 新增分类过滤胶囊栏（全部 / 工具与执行 / 文件编辑 / 思考与消息 / 系统），支持快速按事件类型精准筛选并展示实时命中计数；
  - 自动滚动追踪：自动刷新开启时，新事件到达自动平滑滚动定位到最新项，彻底告别手动翻找。
- **CPU 动态负荷色彩预警与微交互优化（AgentDetailView）**：
  - 性能与健康卡片中 CPU 占用根据实时负荷动态着色（>80% 红色警示、>30% 橙色注意、常规柔和银白）；
  - 内存与 PID 条目增加悬停 Tooltip 精准展示原始字节数与完整说明。
- **工具箱异常诊断多维度筛选（ToolboxView）**：
  - 新增异常类别过滤条（全部 / 死锁 / 孤儿 / 内存超限），支持分类直观审视与按分类一键批量清理。

---

## [0.0.66] - 2026-09-17

### ⚡️ 深入体验优化：即时热唤醒、触感反馈、Esc键导航与工程零警告守护

- **展开即时热唤醒（Adaptive Hot-Wakeup）**：
  - 展开灵动岛面板（点击或 Hover 悬停触发）时，立即通过后台派发采样（`sampleInBackground()`），在动画展开的数百毫秒内就刷新进程与用量快照，杜绝展开时视觉内容停留在旧周期的卡顿滞后感。
- **macOS 原生触觉反馈（Haptic Feedback）**：
  - 引入 `HapticFeedback`（基于 `NSHapticFeedbackManager`）；
  - 在灵动岛展开、收起、工具箱单项清理与批量清理异常进程等核心交互动作中触发对齐与确认级细微震颤反馈，极大提升操作质感。
- **全局 Esc 快捷退出与导航（.onExitCommand）**：
  - 在子页面（Agent 详情、Token 统计、会话列表、实时日志、工具箱、设置）按下 `Esc` 键平滑返回主卡；在主卡展开态按下 `Esc` 键平滑收起至状态条。
- **工具箱异常诊断体验增强（ToolboxView）**：
  - 异常进程诊断项中补充显示进程可执行文件名（`commandBasename`），方便用户更清晰地辨识卡死或泄漏的底层程序；清理动作配合触感反馈。
- **代码库健康与零编译器警告守护**：
  - 修复多处闭包中隐式强引用 `[weak self]`；
  - 清理所有测试与实现文件中的无用局部绑定变量，达成编译构建 100% 零警告。

---

## [0.0.65] - 2026-09-17

### 🎨 UI 与动效全面升级：流体弹性层级转场与触感反馈

- **灵动岛流体层级推入/返回转场（Route Transitions）**：
  - 主卡与二级/三级详情页（Agent 详情、Token 统计、会话列表、实时日志抽屉、快捷工具箱）全面接入 `ZStack` + `.transition(.asymmetric(...))` 深度视差动效；
  - 进入子页带有微妙水平微位移与缩放弹性推入（`offset x: 12, scale: 0.98`），返回主卡平滑淡出，彻底消除原本页面生硬跳切的问题；
  - `CardRoute` 升级遵循 `Hashable`，配合视图 `.id(controller.route)` 触发高保真原生弹簧物理流体转场。
- **Agent 列表行交互触感增强（AgentRowView）**：
  - 列表行增加轻微悬浮微缩放（`scale: 1.004`）与 `.spring(response: 0.22, dampingFraction: 0.75)` 触觉级弹性阻尼；
  - 点击行进入 Agent 详情页时联动显式弹簧转场动画，大幅强化按压沉浸感。
- **状态指示与顶栏标题平滑过渡**：
  - `AdaptiveHeaderText` 的主标题、副标题与徽标接入 `.contentTransition(.numericText())`，多 Agent 状态在“工作中/待确认/已完成/待机”切换时数字与文字平滑滚动过度，杜绝突兀闪烁；
  - `statusDot` 状态指示灯增加色彩插值平滑过渡动画（`easeInOut 0.3s`）。
- **极光流光弧与双层心跳呼吸环（AgentRingView）**：
  - `SpinningActivityArc` 重构为 `AngularGradient` 极光流光拖尾微弧，旋转时呈现平滑渐变流光轨迹；
  - `PulsingAttentionArc` 升级为外层光晕微扩散与内层核心呼吸双层脉冲环，待确认与警告状态更具灵动生命力。
- **全链路返回与汇总栏微动效统一**：
  - `DetailHeader` 返回按键增加悬停微弹性放大（`scale: 1.08`）与显式弹簧返回；
  - 底部 `TokenSummaryBar` 增加悬停轻浮升与点击弹簧转场。

---

## [0.0.64] - 2026-09-17

### 🐛 彻底根治 Antigravity 僵尸“等待确认”误报（专有探测路由优先）

- **双重根本原因**：
  1. **路由旁路失效**：`AgentSessionInspector.inspect(...)` 在最外层无条件对 `activityFiles` 遍历执行通用的 `detect(lines:)`，只有未命中时才回退至数据库/专有探测器。导致 Antigravity（以及 DSH）写在 `inspectAntigravitySession` 中的专有高保真逻辑被完全绕过，transcript.jsonl 被当作通用未知日志解析。
  2. **通用检测器 attention 无法解除**：通用的 `detect(lines:)` 遇到历史命令参数或已回答提问中的 `ask_question` 后，无法识别 Antigravity 的 `source: USER_EXPLICIT` / `type: GENERIC` 答复；且后续即使出现模型工作、新工具调用，也没有清除 attention 状态的机制，导致状态永远钉死在“等待确认”。
- **系统级修复**：
  - **专有协议优先分发**：在 `AgentSessionInspector.inspect` 入口置顶专有派发，Antigravity 与 DSH 100% 走其专有高保真会话探测器。
  - **通用状态机解挂**：通用 `resolvesAttention` 增加对 `source: "user" / "userexplicit"` 识别；并在通用 `detect` 中引入 `startsOrContinuesWork` 时自动解除旧 attention 信号。
- **验证**：真实环境现场探针验证 `inspect` 与 `inspectAntigravitySession` 输出完全对齐，回归测试 244 项全数通过。

---

## [0.0.63] - 2026-09-17

### 🐛 修复 Antigravity 确认通知误报

- **根本原因**：`detectAntigravitySession` 在对 transcript.jsonl 末尾行做 `reversed()` 扫描时，遇到含 `ask_question` 的 PLANNER_RESPONSE 会立即返回 `.attention`，但**未检查该 step 之后是否已存在 GENERIC / USER_INPUT**（即用户早已答复），导致历史上已答复的弹窗被反复误判为"等待确认"。
- **修复**：在正式扫描前，先对 tail-48 行做一次轻量元数据预解析，收集各行的 `step_index` 与 `type`。扫描到 `ask_question` 时，检查元数据中是否存在 `step_index` 更大的 GENERIC 或 USER_INPUT 行；若存在则视为已答复，继续向前扫描而非触发 attention。
- **新增回归测试**：
  - `ask_question → GENERIC → run_command` 序列 → 应为 `.active`，不得触发 `.attention`
  - `ask_question → GENERIC → 最终完成` 序列 → 应为 `.completed`

---

## [0.0.62] - 2026-09-16

### 🚀 性能调优、状态跟踪修复与 Token 动效流畅化

- **消除 120Hz 布局风暴（Docked CPU 降至 0%）**：将贴边微细条呼吸光晕重构为 CoreAnimation 独立硬件图层（`CoreAnimationBreathingGlow`），彻底消除 SwiftUI 120Hz 递归布局重排，贴边常态 CPU 占用由 20%~53% 压降至 **0.0% ~ 0.1%**。
- **全局鼠标移动近邻预过滤**：在 `MouseMoveThrottle` 引入 `proximityRect`，收起态仅当光标进入贴边条周围 40pt 感知区时才调度事件，99.9% 屏幕中央移动直接纳秒级丢弃，零 Task 分配与零主线程开销。
- **系统调用与内存堆复用**：`PathCache` 缓存存活进程可执行路径与小写字符串，复用 4KB CChar 缓冲区，避免每 2 秒循环创建大量小写副本与 `proc_pidpath` 调用。
- **离线 Agent 免深搜**：离线目录若根目录 mtime 未改变直接复用缓存，跳过递归树遍历；在线时自动恢复深搜。
- **通知中心与横幅联动自动消除**：通知在系统中心被用户关闭/点掉时自动清除灵动岛事件横幅；Agent 退出等待确认状态时自动解除待确认横幅，重新开始工作时自动清除历史完成横幅。
- **状态跟踪准确度修复**：
  - 精准区分 Antigravity 待机与工作中状态，消除空闲时的状态跳变；
  - 修复 DeepSeek Harness 运行中被提前误判完成的问题；
  - 针对 WorkBuddy 专家团引入专属 Token 激增安全下限，兼容多专家团高用量场景。
- **Token 统计页 24h / 7天 / 30天 切换动效流畅化**：
  - 范围选择胶囊增加 `@Namespace` 与 `.matchedGeometryEffect`，实现平滑弹性横向滑移；
  - 增加多时间范围内存缓存与后台静默预加载，除首次冷启动外彻底根除骨架屏闪烁；
  - 用量数值应用 `.contentTransition(.numericText())`，折线趋势图增加淡入过渡，进度条弹性伸缩。
- **自动化交付约定强化**：在 `AGENTS.md`、`CLAUDE.md` 与 `GEMINI.md` 中增加规范：改造完成后只要测试通过，自动提交至远端并执行发版与文档更新。
- **验证**：新增状态追踪、性能优化与横幅自动消除回归测试；runner **241/0 全绿**。

---

## [0.0.61] - 2026-09-15

### 🔎 Token 工具下钻修复（R44）

- **Codex 用量行可正常响应**：可读的 Codex 本地会话用量不再要求同时存在独立实时监控快照；被 ChatGPT 桌面端承载而有意去重的 Codex 也能点击进入详情。
- **详情语义准确**：没有独立运行项时，详情页展示 24h / 累计 Token 摘要，并说明数据来自本地会话记录，避免误报为“已不在监控列表”。
- **验证**：新增内嵌 Codex 可下钻回归；runner **222/0 全绿**。

---

## [0.0.60] - 2026-09-15

### 🧲 四边贴靠与自动收起修复（R43）

- **自动收起恢复**：状态栏宿主窗口不再被当作岛的交互浮层；其宽泛 frame 不会再错误阻止收起。popover / 菜单栏浮层实际被悬停时仍保持展开。
- **Vokie 式四边停靠**：展开卡片可自由拖拽，松手按面板到可用屏幕上、右、下、左四边的最近距离，吸附到对应边缘；水平边复用 X 锚点、垂直边复用 Y 锚点，跨重启保持位置。
- **四向完整交互**：顶部/底部使用 140×6pt 横向微细条，左侧/右侧使用 6×120pt 纵向微细条；反向倒角、悬停热区、自动展开、收起箭头、提示气泡箭头与辅助功能标签均按方向镜像。
- **打包阻塞修正**：补齐 Token 分析页指标组件的调用签名，避免此前已完成的分析界面在全量 Swift 构建中报错。
- **验证**：新增浮层命中与四边吸附回归；测试 runner **221/0 全绿**。

---

## [0.0.59] - 2026-09-15

### 📊 跨工具 Token 总体与明细（R42）

- **真正的总体用量**：汇总不再只覆盖 DimAgent / OpenCode；新增 Codex、Claude、WorkBuddy、WorkBuddy AI 本地结构化日志适配，24h、累计与时间趋势统一跨工具求和
- **按工具统计**：时间分析页明确分为“总体用量”和“按工具用量”，逐项展示 Token、占比与可用成本；无记录、未发现本地明细分开表达，不再用缺席造成统计完整的错觉
- **统一净消耗口径**：Codex / Claude / WorkBuddy 使用 `(input-cache.read)+output`，Codex reasoning 已包含在 output 时不重复计算；同一 response id 跨恢复/分叉日志去重
- **低开销结构化索引**：JSONL 只保留时间与计数，按 inode/mtime/size 缓存；活跃文件增长时只解析追加段，避免 60s 轮询反复扫描数十 MB 会话正文
- **下钻边界修正**：历史工具未进入当前监控列表时保留统计行，但不再伪造可点击箭头进入空详情页
- **验证**：新增工具覆盖状态、Codex/WorkBuddy 汇总、缓存扣除与去重回归，runner **219/0 全绿**

## [0.0.58] - 2026-09-15

### 📈 Token 时间分析（R41）

- **完整时间视图**：点击主卡底部 Token 汇总条即可进入独立分析页，支持 24h / 7天 / 30天切换；默认主卡仍保持一行摘要，不增加日常信息噪声
- **趋势与环比**：24h 按小时、7 天按 6 小时、30 天按天生成固定密度趋势图，同时显示范围总量、成本、累计量、上一等长周期及变化百分比
- **来源构成可下钻**：展示 DimAgent / OpenCode 在当前范围内的 Token 数与占比，点击来源继续进入原有模型、会话明细
- **统一净消耗口径**：DimAgent 使用 `(prompt-cache.read)+completion`，OpenCode 使用 `input+output+reasoning`；未来记录和两周期之前记录不进入图表，数据库保持只读且只在打开分析页或切换范围时查询
- **窄卡片与辅助功能**：图表固定 24–30 个点，适配 330pt 面板；空数据、加载态、范围选择和趋势摘要均提供明确视觉/VoiceOver 语义
- **验证**：新增时间边界、固定桶数、上一周期和双 SQLite 数据源集成回归，runner **217/0 全绿**

## [0.0.57] - 2026-09-15

### 📰 长标题可读性与信息层级（R40）

- **顶部标题不再消失**：原先 Agent 名、实时动作、在线计数和三个固定按钮挤在同一行，长内容会把标题压到近乎零宽；现在改为“稳定主标题 + 动态动作副标题”双层结构，并为主标题保留 96pt 最低可读区
- **次要信息主动让位**：在线计数在空间不足时从 `4/10 在线` 自动退化为 `4`，完整含义继续通过悬停提示和 VoiceOver 提供，不再与主标题争抢空间
- **截断仍可辨认**：文件名、模型名、动作与事件标题统一采用中间省略，同时保留语义前缀和文件/模型后缀；悬停显示全文，辅助功能始终朗读全文
- **同类界面统一**：主顶栏、Agent 行、事件横幅、详情标题、模型/会话行、工作台、实时流水与菜单栏概览共用可读性契约
- **布局闭环**：顶栏几何从单行 23pt 校准为双层 34pt，列表在 460pt 总高内自适应让位；新增 2 个标题契约回归，runner **214/0 全绿**，真实 SwiftUI 布局门禁通过

## [0.0.56] - 2026-09-15

### 👁️ 在线 Agent 可见口径

- 主列表、菜单摘要、顶部活动环统一改为**仅显示当前进程仍在的 Agent**；离线项即使 24h 内有活动或 Token 记录也不再形成幽灵条目
- 在线的待机、运行中、待确认与刚完成状态继续正常显示；设置页的安装与启停管理仍保留全部档案
- 菜单与卡片计数文案由“可见”改为“在线”，回归覆盖离线近期活动、离线有量、在线待机和在线工作四种边界

## [0.0.55] - 2026-09-15

### 🔔 状态语义与待确认通知（状态准确性优化 · R38）

- **状态从三态扩展为五态**：在 `offline / idle / working` 之外新增 `completed / attention`。CPU 与会话文件仍负责活动近似，Codex、Claude、DimAgent、ZCode、WorkBuddy、OpenCode 的结构化会话事件负责“本轮已完成”和“正在等待用户”的强语义，避免低 CPU 的确认等待被错标为待机、完成事件又被写入窗口拖成工作中
- **待确认通知可直达**：每个未处理的确认/权限请求只通知一次；通知携带 Agent ID，点击后优先激活对应 GUI，CLI Agent 则沿 PID 父进程链唤起 Terminal / iTerm2 / VS Code / Ghostty / Warp 等宿主。目标已退出时回退到 AgentIsland 对应详情页
- **多 Agent 同拍不丢通知**：系统通知改走逐事件流，多个 Agent 同一采样周期同时等待确认时逐条投递；岛内横幅仍保持单槽与严重告警保护
- **确认等待不会伪造完成**：进入 `attention` 会切断旧工作区间，用户处理后回到真实的 working/idle，不补一条虚假的“任务完成”；冷启动读到旧完成标记只展示状态，不追发陈年通知
- **隐私与开销边界**：只读取本轮扫描命中的最新会话文件，尾读最多 96 行 / 256 KiB；结构化解析仅消费 type/name/status/role/id 等协议字段，通知不带问题或回答正文；未知格式继续走原有双信号降级路径
- **验证**：新增待确认解析、解除、完成终态、事件去重、多 Agent 并发投递、通知路由、文件命中隔离等回归用例；自建 runner **212/0 全绿**

## [0.0.54] - 2026-09-12

### 🎯 状态真实性：打开应用没动不再误报「任务完成」与响铃（v4 战役 · R37）

用户报告「有些软件打开了没动也算一次完成，会响铃通知」。真机取证确认两条独立成因，均已修复：

- **完成事件需要写入证据**：CPU 高只说明进程在烧 CPU——实测 ChatGPT 静置时反复冲到 20.6%/43.4%，每次尖峰都会走完「working → idle」并补发完成事件（复现日志：`ChatGPT 任务完成 (10秒)` + 提示音）。**纯 CPU 区间收尾改为静默**：仍照常显示工作中（双信号判定不变），但不再产生完成事件、不响铃、不弹 Peek。有写入证据的区间一切照旧
- **文件噪声剥离（全部来自实测）**：
  - SQLite `-shm` 空转触碰：Antigravity 空闲时 10 个会话库的 `-shm` 每 200s 被同步刷一次、size 恒为 32768 字节，而**主库 mtime 停在 22 小时前**——「刚刚写入」的假活动足以把空闲应用顶成 working。`-shm` 是连接共享的 mmap 索引页，任务数据落在主库或 `-wal`（保留 `-wal` 判定，实测 600s 窗口零变化），过滤后不丢信号
  - 浏览器内核（Chromium/Electron）用户数据子树：Cache / Code Cache / GPUCache / Dawn 系列 / Session Storage / Local Storage / IndexedDB / Crashpad / blob_storage / DIPS / Trust Tokens / Singleton* 等 45 项整棵剪枝，同时消除虚报的「活跃会话」计数
  - 内核状态文件与应用账号文件：Network Persistent State、DevToolsActivePort、Preferences、Cookies、First Run、`BrowserMetrics-*.pma`、`oauth_credentials.json`、`app_storage.json`、Sparkle `*appcast*` 等
  - Antigravity 档案不再监控 `Library/Application Support/Antigravity`（内核用户数据目录：实测仅打开应用就有 36 次写入/20 分钟全部落在此），只保留 `conversations` + `brain` 两个真正的会话数据目录
- **判别性验证**：新增 6 个用例（含「CPU 高但无写入 → working」契约保留、有写入证据照常报完成、写入证据不跨区间泄漏）；变异验证 196/6 红（临时还原过滤与准入条件），恢复后 202/0 全绿
- **活体对照**：假 Agent 进程「CPU 尖峰 22s → 静置」形态零事件（修复前同形态补发完成事件）；触碰 `*.db-shm` 无任何状态变化、触碰 `*.db-wal` 立刻判 working；真实写入（vibe-usage）仍正常报完成并响铃


## [0.0.53] - 2026-09-12

### 🏁 v4 战役收官（R32–R36）

状态机与调度深度侦察（10 主发现 + 4 加固）驱动五轮闭环：

- **状态机时序批**（R32/0.0.49）：面板态处理代际守卫（sink Task 乱序覆盖新态）、唤醒后激增告警重新武装、拖拽后 grace 复位、收起延迟实时校验、断点阈值钳 180s（变异验证闭环）
- **文件监控生命周期批**（R33/0.0.50）：噪声文件经父目录 mtime 的传播链阻断、监控目录永久删除的幽灵活动终态清零、配置变更后首扫绕过节流（新目录信号不再空白到下一拍）
- **数据面批**（R34/0.0.51）：配置变更重采样移出主线程、菜单栏 popover 按需刷新 token（不再显示冻结值）、停止后连接重建代际治理、刷新请求合并
- **能效与护栏批**（R35/0.0.52）：边缘检测 Timer 自适应降档（活跃 0.12s / 静止 0.5s）、高度签名漂移哨兵（变异验证）
- 收官核验：测试 196/0 ×3、布局哨兵 PASS、selftest 全过、A/B 对照无回退（基线 v0.0.48 同条件 CPU 相当）、代码卫生终扫（print/try!/as! 零残留）


## [0.0.52] - 2026-09-12

### 🔋 能效与护栏批（v4 战役 · R35）

- **边缘检测 Timer 自适应降档**：活跃 0.12s / 静止 3 秒后降档 0.5s——hover 展开的主路径是 0.05s 节流的鼠标事件（不受降档影响），Timer 只是「光标停在热区内、事件被节流丢失」的兜底；静止时（远离热区且未展开）高频唤醒纯属浪费。光标进入热区、面板展开、拖拽/按压任一发生时立即恢复快档
- **高度签名漂移哨兵**：R08 的「展开态高度签名去重」依赖签名组成与 `expandedHeight` 输入严格对应，但此前只有注释约束——新增源级清单断言（六项输入逐一核对 + 渲染实参同组），布局改动漏改签名会直接测试红（变异验证：删签名项即 195/1）
- 测试 196/0；A/B 实测（同条件基线 v0.0.48 vs 当前）：CPU 相当（5–8%，WorkBuddy 活跃期同条件），无性能回退；收起态静态约 0–1%


## [0.0.51] - 2026-09-12

### 🧵 数据面批（v4 战役 · R34）

- **配置变更重采样移出主线程**：设置页启停开关、新增/删除自定义档案、终止与清理后的重采样此前在主线程同步执行（进程全表快照 + 全部档案匹配 + 动作探测 ~10ms 级），开关可感知卡顿——统一改走后台采样路径（同等 running/在飞防护）
- **菜单栏 popover 不再显示冻结的 token**：popover 是 token 数据的第三消费方但不参与「呈现活跃」生命周期（收起态轮询已暂停）——打开时按需单次刷新；此前从未展开过面板的话，Token 概览甚至永远空白
- **停止后的连接重建不再泄漏**：`stop()` 与在飞刷新竞态时，迟到的查询会重建连接并写回缓存（此后无人再关，违背「彻底清理」契约）——连接代际判定后一次性连接用毕即关
- **刷新请求合并**：首刷与 60 秒轮询相邻触发时，第二个请求此前白等锁——在飞去重后直接返回
- 测试 192 → **195**


## [0.0.50] - 2026-09-12

### 📁 文件监控生命周期批（v4 战役 · R33）

- **噪声写入不再经父目录传播成工作信号**：`.lock`/心跳类噪声文件写入会刷新父目录 mtime——目录计入 newest 会把已过滤的噪声反向传播回工作信号（working 误报 + peek 弹出），且每次噪声写入都改变根 mtime 使快跳过永久失效（大目录每拍全量枚举）。现在 newest 仅由信号文件聚合，目录条目只参与活跃会话计数
- **监控目录永久删除后的幽灵活动清零**：用户删除 `~/.claude/sessions` 等目录后，缓存残留删除前的最后写入时间（「抖动保留旧值」分支吞掉了终态），卡片 24h 内仍按幽灵活动显示。连续 ≥3 趟缺失即终态清零（阈值吸收原子替换/迁移的瞬时空窗），目录重建后自动恢复
- **配置变更后的首扫不再被节流吃掉**：切换启停集后新目录的文件信号此前会空白到引擎下一拍（idle 节律最长 60 秒）——完成扫描带代际标记，配置变更后队列上的下一趟扫描绕过节流立即落地
- 测试 188 → **192**（噪声传播/幽灵终态/节流豁免三组回归 + 既有用例适配新语义）；布局哨兵 PASS；selftest 全过


## [0.0.49] - 2026-09-12

### ⏭️ 状态机时序批（v4 战役 · R32）

- **面板状态处理加代际守卫**：`$displayState`/`$latestEvent` 两个 sink 经 Task @MainActor 跳跃且 MainActor 无 FIFO 保证——乱序执行时旧态处理覆盖新态（token 轮询在 docked 态空转开启、动画按错误几何计算、旧事件的音效/横幅语义覆盖新事件）。现在捕获值与当前值不符即过期跳过
- **唤醒后激增告警重新武装**：采样断点（睡眠/挂起）重置集此前漏 `tokenSpikeAlerted`——睡前已告警、醒后持续高速率的 Agent 会被残留去重标记静默压制
- **拖拽后 grace 复位**：peek 进行中开始拖拽，残留的 open-grace 会把拖拽结束后的自动收起压制最长 6 秒
- **收起延迟实时校验**：设置页挂起期改「自动收起延迟」后，在飞收起按新值校验到期（≤5s 窗口）
- **断点阈值钳 180s 上界**：idle 滑杆上限 60s 时 3 倍阈值恰为 180，越界脏配置（idle 最高 600）会让阈值放大到 1800。测试以有判别力构造锁定（变异验证：旧代码 187/1 红 / 新代码 188/0 绿）；120–180s App Nap 场景根治需事件驱动锚点，已落档为后续规格决策
- tokenSpikeAlerted 放宽 internal（@testable 断言用）；测试 186 → **188**


## [0.0.48] - 2026-09-12

### 🔖 WorkBuddy AI 安装标记修复（follow-up）

- **真实 bundle id 对齐**：`WorkBuddy AI.app` 的 Info.plist 实测 `com.workbuddy.workbuddy-ai`——档案此前照抄 Application Support 数据目录名（`com.workbuddy.workbuddy`）漏了 `-ai`，「已安装」标记永久假阴性（面板 INST 列显示 no）。已修正并新增哨兵测试：档案 bundleIDs 与真实 Info.plist 必须对齐（本机未装该变体时跳过）
- 测试 185 → **186**


## [0.0.47] - 2026-09-12

### 🧳 WorkBuddy 国内版 / 国外版区分（用户实测驱动）

本机同时装有 `WorkBuddy.app`（com.tencent.workbuddy.mac，腾讯系=国内版）与 `WorkBuddy AI.app`（com.workbuddy.workbuddy=国外版）。旧档案混合误配：bundle 只认国内版、数据只读国内版目录 `~/.workbuddy`，而宽口径 pathContains "workbuddy" 又会同时命中两个变体的 Electron 进程——两个版本并存使用时无法区分（用户实测两变体昨天到今天均在活跃使用）。

- **拆分为两条独立档案**：`workbuddy`（国内，~/.workbuddy）与新增 `workbuddy-ai`（国外，~/.workbuddy-ai，图标 globe）
- **pathContains 精确隔离**：两变体进程 basename 同为 Electron 且路径都含 "workbuddy"，宽口径会双份计数——国内版钳到 `/Applications/WorkBuddy.app` 与 `.workbuddy/`，国外版精确到 `WorkBuddy AI.app` 与 `.workbuddy-ai`
- 动作探测与实时流水按变体分发数据目录（inspectWorkBuddyAction/fetchWorkBuddyEvents 增加变体参数）
- 新增双向隔离测试（国内版不得吸走国外版路径、反之亦然）；probe 实证双行各自独立显示状态与最近活动（用户实测：国内版昨晚 23:41 仍有写入、国外版今天持续活跃）；测试 185/0


## [0.0.46] - 2026-09-12

### 🏁 v3 增量战役收官（R21–R30）

两路增量侦察（UI 次要文件 7+4 条 / Core 次要文件 5+3 条）+ v2 遗留 6 项，十轮闭环（小轮自验 + 单测/哨兵锁定）：

- **用户数据防丢失**（R21/0.0.38）：自定义档案逐元素容错（坏元素不再摧毁全部）、启停集损坏只读降级（绝不写回）、knownAgents union 口径
- **口径对齐**（R22/0.0.39）：前缀族冲突共享判定（UI 校验与匹配器单一事实源）、激增阈值非档位吸附、bundle 扫描并入 ~/Applications
- **面板几何**（R23/0.0.40）：菜单栏内展开振荡上界钳制；空态高度实测校准 87 → 121
- **字号统一**（R24/0.0.41）：badgeFont 令牌，14 处 <9pt 文本提升
- **性能**（R25/0.0.42）：进程快照 syscall 合并（基准 4.1ms 均值）；isFresh 稳态语义修正
- **Core 健壮化**（R28/0.0.45）：warmUp 已热仍回调（启用集重放契约）、refresh 重入运行期断言、Selftest 强解包修复
- 收官核验：测试 184/0 ×3、布局哨兵 PASS、selftest 全过、README 现势性复核（无 v2 Q7 类漂移残留）

## [0.0.45] - 2026-09-12

### 🧱 Core 健壮化批（v3 战役 · R28）

- **warmUp 已热仍回调**：引擎 init 依赖它做「冷启动完成后重放启用集」——此前以已热缓存构造引擎（Probe/测试/未来第二组合根）时重放被静默跳过。现在已热无在途扫描时 completion 仍在主线程立即回调（幂等）
- **refresh 重入断言**：扫描器经注入闭包递归调 `refresh()` 会自等待广播永久阻塞——`refreshingThread` + `performRefresh` 断言把注释约束变成运行期检查
- **Selftest 强解包修复**：临时目录在两拍间被删时 `before!` 直接 crash；内置表 id 变更时 5 处 `first{...}!` 全部 crash——均改为记 failure 继续（诊断工具保持「报告失败项」价值）
- `profile(id:)` 文档升级为显式防呆警告（永远查不到 cli-* 自动发现条目）；测试 184/0


## [0.0.44] - 2026-09-12

### ♿ 无障碍第二批（v3 战役 · R27）

- **菜单栏 popover Agent 快捷行补交互语义**：`isButton` trait + 「名称，状态，点按查看详情」label（与主卡 Agent 行同口径；此前 `onTapGesture` 行对 VoiceOver 无交互语义）
- **设置/退出 icon-only 按钮补 label**：此前只有 `.help`，VoiceOver 播成「…帮助」而非按钮名
- axdump 实证（System Events entire contents）：AX 树可达、面板节点存在；完整 VoiceOver 流程仍待真实辅助技术客户端人工确认；测试 184/0


## [0.0.43] - 2026-09-12

### 🧩 UI 细节批（v3 战役 · R26）

- **会话行「目录未找到」反馈**：会话目录被删除/改名/外置盘卸载后点击行静默无效（v2「直达失败反馈」修复在本页的漏网点）——消费 `NSWorkspace.open` 返回值，失败 1.5 秒行内提示
- **菜单栏文案对齐实体**：「收起/展开侧边栏」→「收起/展开灵动岛」（岛可贴顶，不再是侧边栏）
- **详情页占位说明**：无按模型拆分数据的源（claude/codex 等）此前无声缺席，现显示「该数据源暂不支持按模型拆分」
- TokenSummaryBar 累计花费补 help 与 layoutPriority（与 24h 侧对称，双 cost 防截断失察）
- 两处 DateFormatter 固定 `en_US_POSIX`（12 小时制用户的行内时间防 locale 改写变形）
- 恒真 `#available(macOS 13)` 死代码删除；设置页补「活动间隔不可大于闲置间隔」联动说明；测试 184/0


## [0.0.42] - 2026-09-12

### ⚡ 快照 syscall 合并与戳比对语义修正（v3 战役 · R25）

- **进程快照 syscall 合并**：单次 `sysctl(KERN_PROC_ALL)` 同时给出全部 pid 与父进程 ppid，替代 `proc_listpids` + 每条目一次 `proc_pidinfo(PROC_PIDTBSDINFO)`——实测单次 2.4µs × 500+ 进程 ≈ 1.2ms/拍直接消除（proc_pidpath 与 rusage 按需保留）。进程表在采样间增长时缓冲重试一次
- **token 戳比对在稳态生效**：`isFresh` 阈值恰等于轮询间隔（60s）时，下一拍 age 恒 ≥ 60 → 条件永不成立，「戳未变跳过重查」的短路设计永久空转（每拍 4 次 stat）。加 5 秒宽限吸收定时器抖动，稳态（文件戳未变）跳过全表聚合
- 基准实测：snapshot 均值 4.1ms；测试 184/0 ×3；probe 状态判定形态一致

## [0.0.41] - 2026-09-12

### 🔤 字号统一：徽标字号令牌（v3 战役 · R24）

- **徽标字号令牌 `badgeFont`**：monospaced 9pt 统一入口——此前同一「Token 徽标」语义散落 8/9/10pt 三种尺寸，且 <9pt 低于 HIG 最小可读字号、高分屏缩放下可读性差
- 14 处 <9pt 文本全部提升：Agent 行 Token 徽标、工作台异常标签、实时流水徽标、悬停卡状态、环看板副标题、事件横幅 chevron（图标 7 → 8）
- 行高无回归：徽标不是行内主导高度元素（名称 12pt 主导），IslandMetrics 校准常量与布局哨兵全部通过；测试 184/0

## [0.0.40] - 2026-09-12

### 📐 面板几何：菜单栏振荡修复与空态高度校准（v3 战役 · R23）

- **菜单栏内不再触发展开振荡**：top 贴边的展开热区此前只有下界（y ≥ max-18）无上界——菜单栏内整条 x 跨度都在热区里，而菜单栏在面板 frame 之外：光标停菜单栏触发展开 → 「不在面板内」0.5 秒后收起 → 热区仍命中再展开……无限振荡。热区补上界钳制（y ≤ max），菜单栏区域由 R11 的点击穿透正确处理
- **空态高度常量实测校准**：`emptyStateHeight` 87 → 121。依据：真实 SwiftUI 空态（图 + 文案 + 「打开偏好设置」按钮）理想高 185pt − chrome 64pt；R14 加按钮后未回校，一直靠 fittingSize 兜底不裁切，但常量失真让列表/空态的静态推导不可信
- 测试 184/0（空态高度断言同步校准值）

## [0.0.39] - 2026-09-12

### 🧭 口径与校验对齐（v3 战役 · R22）

- **自定义 Agent 前缀族冲突双向拦**：新增校验此前只做精确比对——内置 `codex` 已启用时添加自定义 `codex-helper` 会放行，而匹配器是「相等或词边界前缀族」口径，同一进程会被两个 profile 同时命中（监控数字翻倍、CPU 双报），表单错误文案承诺「会重复计数」却拦不住。抽共享判定 `hasPrefixFamilyConflict`（相等或互为 name+分隔符前缀，大小写归一），UI 校验与匹配器单一事实源
- **激增阈值非档位值吸附**：设置页 Picker 固定五档、自愈口径是连续区间——外部 `defaults write` 或旧版遗留的非档位值会让选项空白而引擎仍按未知阈值告警。打开设置页时吸附到最近档位
- **bundle 扫描并入 ~/Applications**：宿主 GUI 按用户安装在此处时，「已安装」标记与宿主内嵌组件过滤（避免同一程序显示两条）双双失效
- 测试 183 → **184**

## [0.0.38] - 2026-09-12

### 🛡️ 用户数据防丢失（v3 战役 · R21）

- **自定义档案不再「整批消失」**：存档是整条 JSON 数组、解码全有或全无——单个元素损坏（缺键 / 类型漂移）会让全部自定义 Agent 凭空消失，且此后任一次添加/删除会以空基线覆写坏数据，造成**静默永久丢失**。现在逐元素容错：坏元素丢弃并记日志（AppLog），其余原样救回
- **启停集损坏只读降级**：`enabledAgents` 存档损坏此前被并入「无记录」，会被首次安装分支覆写——用户全部启停选择一次性静默丢失。新增三态读取（无记录 / 正常 / 损坏），损坏时按默认启用集运行但**绝不写回**，损坏存档保留可恢复原状（修复数据后重启自愈）
- **knownAgents 口径统一**：用户「全部关闭」时不再丢弃 registry 外的历史已知项（与正常分支同口径）
- 测试 180 → **183**（坏元素抢救 / 损坏存档防覆写 / 历史 known 保留）

## [0.0.37] - 2026-09-12

### 🏁 20 轮优化收官：文档全量校对与终验（20 轮优化 · R20）

v0.0.18–v0.0.37 共 20 轮（R01–R20），由四路独立侦察（性能 / 健壮性 / UI 交互 / 质量工具链，约 56 条发现）驱动，每轮走「开发 → 修复 → 测试全绿 → 独立验收 → 提交 → 发布 → 验证记录」闭环，每轮独立验收（两处验收 FAIL 打回重做：R09 浮层类名、R11 穿透谓词，均以活体实证定位）。

#### 本轮（R20）内容

- **README 全量校对**：9 处历史事实漂移修正——CPU 判定阈值（1% → 6% 含桌面类下限）、内置档案数（12 → 17）、扫描节流（15s → 3s）、闲置降频（15s → 5s，两处）、卡宽（280 → 330）、用例数（163 → 179）、target 数（3 → 4）、进程枚举方式（ps → libproc）、阴影描述（已停用）
- **终验**：全量测试 180/0 连跑 3 次；--selftest 全过；--probe 功能等价；收起态 CPU 0.2%（全离线空载）；红线核查——采样节律零改动（scheduleNext/间隔全 diff 定性）、零第三方依赖、提交信息与代码注释无环境指纹

#### 战役总账（0.0.17 → 0.0.37）

- **崩溃与误杀**：3 个 P0 崩溃路径（越界值/开库泄漏/事件 id）在 0.0.17 已修；本轮再修 PID 复用误杀整棵进程树、collapseDelay 脏值「收起必崩」、孤儿误杀 launchd 服务、时钟回拨卡死 working、浮层按钮不可用等 **5 类可真实触发的高危缺陷**
- **测试**：159 → 180（+21），且获得：登记制测试卫生（退出后零 plist 残留自守护）、版本哨兵（三处版本漂移即红）、变异验证过判别力的用例
- **性能**：opencode 探测 SQL 8.1k part 会话 1.3–2.6ms → 0.002ms；采样热路径去重复计算（匹配/用量快照/扫描判定）；展开态每拍整卡重绘消除；永续 Timer 降频
- **架构**：SQLite 只读层统一（连接缓存）、IslandView 1168 → 513 行、吸附计算单实现、注册表分发
- **工程**：发布脚本版本自动化 + 测试门禁 + 版本哨兵、统一日志通道（发布版可观测）、验证记录入库
- **已知限制 / 下一战役候选**：菜单栏内细条上方光标致 expand/collapse 振荡（既有几何）；<9pt 字号统一（需可视化验收配合）；VoiceOver 全流程人工验收；空态高度常量已低估（兜底机制保证不裁切）；TokenUsageMonitor isFresh 阈值语义（S9，低值）

---
## [0.0.36] - 2026-09-12

### 🧰 工具链与发布强化（20 轮优化 · R19）

- **打包脚本版本自动化**：版本号此前在 build-app.sh 两处手工硬编码（历史上 README 硬编码 1.7.9 导致关于页显示错误的教训）——现在从 CHANGELOG 首条版本自动抽取（`./scripts/build-app.sh 1.2.3` 可显式覆盖），并在打包前校验 README 功能版本一致，漂移即拒绝
- **测试门禁**：打包前强制全量测试（176+ 用例），`SKIP_TESTS=1` 可跳过供快速冒烟——「未测试即发布」不再可能
- **布局哨兵脚本加固**：test-token-layout.py 失败时打印编译器 stderr（此前被吞只剩退出码）；链接前先 `swift build` 确保产物新鲜（此前会把过期对象当验收依据）
- **ADR-0003 补状态注记**：决策 4 的 ShadowHostView 投影已被实现取代（阴影停用），1-3 仍有效——不再让过期 ADR 误导后来者
- **验证记录入库**：docs/validation/ 从 .gitignore 移出——CHANGELOG 的性能数据从此在仓库内有出处（R01–R19 每轮验证记录随本版入库）

## [0.0.35] - 2026-09-12

### 📡 统一日志通道与调试指南落地（20 轮优化 · R18）

- **新增统一日志出口 `AppLog`**：发布构建里数据源异常此前散落在 print/debugPrint（全部不可见）——SafeNumber 钳制告警、SQLite prepare/step/open 失败、自启动注册失败现在统一走 os.Logger（`log show --predicate 'subsystem == "com.agentisland.app"'` 可查）；Probe/Selftest 的表格输出是 CLI 交互目的本身，不经过此通道
- **调试指南真实可用**：README 承诺的 `AGENTISLAND_DEBUG=1` + `/tmp/agentisland.log` 此前指向不存在的功能（代码无任何读取/写入）——现在开启开关即镜像全部诊断日志到该文件，并补充系统日志替代命令
- 镜像落盘有守护测试（环境开关 + 文件内容断言）；测试 179 → **180**

## [0.0.34] - 2026-09-12

### 📊 数据源消失终态与版本哨兵（20 轮优化 · R17）

- **数据源已消失不再显示陈旧统计**：dim/opencode 库文件被删除后，面板此前会永久显示最后一次成功值且无任何迹象（「查询失败保留旧值」契约只覆盖瞬时失败）。现在主库文件连续 3 拍缺失（约 3 分钟）即置空该源；单拍缺失（原子替换的短暂空窗）仍保留旧值——「数据库暂时缺失保留旧统计」契约不变。实现首版的字符串匹配会误判（组合戳里 `-wal:missing` 是合法形态），已改为主库文件存在性判定
- **版本哨兵测试**：CHANGELOG 首条版本、README 功能版本、build-app.sh 的 Info.plist 版本三处一致性入测试（历史上三处漂移导致关于页/打包/文档各说各话）；验收以变异实验证实精确报红
- **解码容错测试**：自定义档案存储损坏（坏 JSON / 空数据）回落空集
- postEvent 保留 public（LayoutRegression 目标普通 import 会破坏，偏离已落档）；测试 176 → **179**

## [0.0.33] - 2026-09-12

### 🧪 测试卫生：plist 泄漏根治（20 轮优化 · R16）

- **测试套件 plist 泄漏根治**：12 处测试散点创建 UserDefaults 套件并各自清理——`removePersistentDomain` 只解除注册，cfprefs 在进程退出 flush 时把已删域重建为 plist 文件，`~/Library/Preferences` 逐次累积（实测 2400+ 个）。改为 TestDefaults 登记制：统一清理（移除持久域 + 删文件双保险）+ 泄漏自守护断言 + 退出后独立进程兜底清扫（应对 cfprefsd flush 竞态，3 秒重试）——连跑 3 轮零残留，历史残留一次清零
- **注册表持久化可注入**：`loadCustomProfiles` / `saveCustomProfiles` / `fullRegistry` 增加 `defaults` 参数（默认 standard，向后兼容），自定义档案测试不再经 standard domain 污染后续用例；顺带修复 RegistryTests 中 `suite.description` 笔误（恒 no-op 的清理调用）
- **环境依赖用例加固**：真实引擎采样在进程表不可读（沙箱/CI）时显式跳过；清理用例的进程启动失败不再被 `try?` 静默吞掉
- 测试 176/0 连跑多次；独立验收 pass

## [0.0.32] - 2026-09-12

### 🏗️ 结构拆分：IslandView 按类型归位与吸附计算单实现（20 轮优化 · R15）

- **IslandView.swift 1168 → 513 行**：AgentRowView / TokenSummaryBar / DockedSliver（含脉冲动画）/ EventBannerView 纯搬移拆出（字节级一致、零逻辑改动）；卡内导航枚举留在原位（测试漂移哨兵按文件路径解析）
- **吸附计算双份收敛**：placeWindow 与拖拽吸附各自维护的「clamp 锚点 + 目标 origin」几乎逐行相同——收敛为 `dockTargetFrame` 单实现（行为逐行等价核对），改贴边行为不再需要同步两处
- **阴影宿主清理**：停用的 ShadowHostView no-op 类删除，注释与文档去引用（ADR-0003 的过时表述在 R19 统一补注记）
- 布局回归哨兵（test-token-layout.py）PASS；测试 176/0；独立验收含字节级纯搬移证明

## [0.0.31] - 2026-09-12

### ♿ 无障碍与键盘可达（20 轮优化 · R14）

- **Agent 行与流水行的 VoiceOver 语义**：行 label 补状态与动作提示（「Claude，工作中，点按查看详情」），内嵌终止/流水/直达图标按钮补显式 label（此前 VoiceOver 读的是 SF Symbol 默认名）；流水事件行补 isButton trait 与「展开/收起事件详情」hint，且不设显式 label 让合并语义保留徽标/时间/正文
- **菜单栏 popover 图标菜单补 label**：外观主题与通知策略两个 icon-only 菜单此前读出的是符号名，现在播报「外观主题，当前 深色」式现值
- **空态加「打开偏好设置」入口**：覆盖「全部禁用」与「全部离线超 24h」两种成因，不再只有一句「没有活跃的 Agent」
- **Esc 收起弱回退**：面板（或本 App 其他窗口）持有键盘焦点时按 Esc 收起展开卡；不吞事件，点击外部/光标离开仍是主路径
- 已知限制：实际 AX 树物化需辅助技术客户端，VoiceOver 全流程人工验收待补；测试 176/0

## [0.0.30] - 2026-09-12

### 📝 文案口径统一（20 轮优化 · R13）

- **时长文案三处收敛**：引擎完成事件 / summaryText 兜底 / 系统通知各自维护「N分M秒」实现且舍入口径不一（截断 vs 四舍五入，同一事件可能显示 59秒 或 1分0秒）——收敛为 `AgentTaskEvent.durationText` 唯一实现（四舍五入 + 负值钳 0），口径断言入测试
- **内存文案第 4 份漏网实现收编**：工作台「可回收」格的私有实现把 <1MB 显示成「0M」（0.0.17 收敛时的漏网点），改走统一的 `MemoryFormat.text`
- **口径对齐**：菜单栏 popover「N 在线」改为「N 可见」——可见口径 = 在线或 24h 内有活动，对离线但有近期活动的 Agent「在线」是错误陈述；流水页副标题中英混排改「实时流水 · N 条事件」并补加载态；提示音描述补全「完全静默模式下不发声」的行为承诺
- 测试 176/0；独立验收 pass（MemoryFormat.text(0) 占位符语义核实、加载态首帧时序核实）

## [0.0.29] - 2026-09-12

### 🎨 主题可读性与死代码清理（20 轮优化 · R12）

- **流水徽标双值动态色**：`thinking` 紫（固定系统紫，浅色白玻璃 ≈3.2:1）与 `toolCall` 蓝（深色玻璃 ≈2.6:1）改为双值动态——实算对比 thinking 浅色 5.79:1、toolCall 深色 4.54:1，均达 WCAG AA
- **熔断红环语义补全**：`isHung`（≥70% CPU 持续 5 分钟，与「疑似卡死」徽标/死锁扫描同源）的 Agent 水位环呈极光红——此前四级色标的红色从未接线，熔断中的 Agent 环上看不出严重度
- **死代码批删 18 个符号**：TooltipTail Shape（从未接线的几何小尾巴）、数值徽标参数、未使用的几何常量与 Binding、公开 API 僵尸（ProcessMatcher.cpuPercent）、Theme 13 个零引用令牌——全部逐一 grep 零引用后删除
- 顺带清零 R09 引入的两条编译警告；全仓重编译警告保持 0
- 声明偏离：<9pt 字号统一延后（影响 IslandMetrics 实测校准常量，需可视化验收配合）；测试 176/0

## [0.0.28] - 2026-09-12

### 🎛️ 面板状态机：连发事件 peek 重排、点击穿透与唤醒帧重同步（20 轮优化 · R11）

- **连发事件 peek 重排**：peek 进行中来了新事件时取消旧展示按新事件时长重排——此前单任务槽会把 costSpike（6 秒）的展示时长吞成前一 completed（3.5 秒）的剩余时间，最严重的告警可能只 peek 不到 1 秒就缩回
- **拖拽中高度自适应让位**：拖拽由 WindowServer 驱动，采样导致的高度自适应动画不再与之争夺窗口 frame；松手吸附时兜底终态
- **收起态幽灵命中区修复**：透明面板的透明区域仍参与鼠标命中——top 贴边时收起窗口自菜单栏向下延伸整卡高度、right 贴边有一列竖向盲柱（活体实测：细条外 213pt 处点击被面板吞掉）。现在光标在细条可视矩形 ±8pt 内才接收事件，其余全部点击穿透（hover 展开走全局光标判定不受影响；零窗口几何改动，动画零回归）
- **睡眠/唤醒帧重同步**：显示器睡眠期间 CA 动画挂起可能让窗口 frame 停在动画起点（与状态机脱钩）；睡眠期直落终态帧，唤醒（含熄屏唤醒，经 NSWorkspace center）后自动重放正确帧——活体 E2E 实证人为破坏 frame 后唤醒自动恢复
- 独立验收两轮（首轮活体测试打回两项实现缺陷，复验含 E2E 全过）；测试 176/0

## [0.0.27] - 2026-09-12

### 🧰 工作台与导航正确性（20 轮优化 · R10）

- **清理复核不再劫持导航**：「一键清理」的结果复核晚 1.2 秒到达，期间用户若已进入详情/流水页，旧逻辑会把页面强行拽回主列表。现在复核回调校验当前路由——不在工作台只旁观；按 id 差集归因「未能终止」，复核窗口内新升温的异常条目不再记到本次清理头上
- **工作台扫描与引擎彻底隔离**：此前共用同一进程快照源，工作台扫描会消费引擎下一拍的 CPU 差分窗口（实测可把全部进程 CPU 读数归零，仅靠 CPU 信号维持 working 的 Agent 会瞬时抖动）。现在工作台用独立快照源（静态长驻 + 后台预热两拍建立基线，冷启动不再漏报死锁行）
- **快照差分窗口原子化**：`snapshot()` 全程持锁——并发快照（引擎采样 / 工作台扫描 / 终止前身份复核）互不消费对方的差分窗口；新增 6 线程并发竞态冒烟测试
- **直达按钮失败有反馈**：ssh/tmux 启动的 CLI 没有可激活窗口，此前点击「直达」完全无反应（activate 返回 false 被忽略）。三处入口（Agent 行 / 事件横幅 / 悬停卡）失败时按钮切换「未找到窗口」1.5 秒
- 测试 175 → **176**；独立验收 pass（含并发锁死锁论证与真实 probe 对照）

## [0.0.26] - 2026-09-12

### 🖱️ 交互冲突修复：hover 浮层与菜单栏 popover 不再被收起打断（20 轮优化 · R09）

两个由活体实证（lldb 附着 + 辅助功能开窗 + 进程内逐字谓词执行）定位的 P1：

- **tooltip 弹窗里的按钮实际点不到**：悬停卡（SwiftUI popover）渲染在面板 frame 之外，鼠标移入即被判定「离开面板」，0.5 秒后自动收起并关闭弹窗——终止两段式确认与直达按钮只有半秒可用窗口。现在光标在面板**或其派生浮层**内都视为「在面板内」
- **菜单栏 popover 与点击收起互相打架**：展开态下点击菜单栏 popover 的「收起侧边栏」，点击监听先行收起、按钮 action 再翻转——按钮点成展开；popover 内点 Agent 行下钻的详情页在保护期过后一闪而过。本地点击监听改为延迟一拍复核（守卫含 popover 导航 3s 保护期与浮层判定）
- 浮层窗口按活体实证的类名稳定片段识别（`_NSPopoverWindow` / `MenuBarExtraWindow` / `NSStatusBarWindow`），Apple 改名时行为退化为修复前；peek 结束守卫同步收口
- 验收官端到端实证：保护期恒过期的 hover 展开 + 光标驻留菜单栏 popover 90 秒恒保持展开（旧版首个检测拍即计划收起）；光标移开 3 秒内正常收起（无粘滞）
- 已知边界：MenuBarExtra 无公开 dismiss API，「下钻后主动关弹窗」以保护期替代；鼠标级端到端待屏幕可用后人工复认

## [0.0.25] - 2026-09-12

### 🖥️ UI 渲染开销：展开态高度签名去重与设置页缓存（20 轮优化 · R08）

- **展开态不再每拍全卡重绘 + 全量排版**：引擎每拍发布（快照的 CPU/活动时间/动作逐拍变化），面板此前对每拍无条件执行 `needsDisplay` 整卡重绘 + `fittingSize` 全量 SwiftUI 排版（估 1–3ms/拍）。现在按「高度影响签名」（route / 可见数 / 汇总栏 / 环架 / 事件 id / 横幅展开）去重——签名不变时两者都跳过；SwiftUI 由 `@ObservedObject` 自行失效，逐拍数据刷新不受影响。收起态细条的显式失效保留（防回退 0.0.17 前的「细条不刷新」）
- **订阅去 Task 跳跃**：引擎 @MainActor、发布在主线程，每拍一次 Task 分配 + 调度不再需要
- **边缘检测兜底 Timer 0.06s → 0.12s**：hover 展开的主路径是 0.05s 节流的鼠标事件通道，Timer 只是兜底；16.7Hz 的永续唤醒阻止主 runloop 深度 idle（能效影响大于 CPU% 影响）
- **设置页档案列表缓存**：设置页开着时引擎每拍触发 body 重算，内置列表（fullRegistry 的 loadCustomProfiles：UserDefaults 读 + JSONDecoder 解码）与自动发现列表（PATH / Applications 扫描）此前逐拍重跑——按安装扫描版本缓存（引用语义缓存盒）
- 测试 175/0；独立验收 pass（签名与 expandedHeight 六输入逐一完备对照 + docked 呼吸灯机制论证）；`--selftest` 全过

## [0.0.24] - 2026-09-12

### ⚡ 采样热路径性能：去重复计算与防退化护栏（20 轮优化 · R07）

- **进程匹配热路径去分配**：`matchesProcessNames` 位于「条目数 × 档案数」≈ 8.7k 次/拍的调用路径上，此前每条目重复 `lowercased()` 分配副本——Entry 构造时已恒小写，契约写入注释后直接比对（估 0.3–0.8ms/拍）
- **dsh 命令行探测 10s TTL 缓存**：node 进程多的开发机此前每拍对每个 node 候选做一次 KERN_PROCARGS2 sysctl（args+env 整块拷贝解析），命中后 inspectDSHAction 还会对同一 pid 重复一次。现在匹配器与动作探测共用 TTL 缓存（pid 消失清残留、512 条容量上界、负结果不缓存）
- **opencode 探测 SQL 两步化**：原单条 SQL 的相关子查询对 part 按 time_updated 排序（无索引 → 临时 B 树，随活跃会话 part 数线性退化，万级 part 实测量级 1.3–2.6ms/拍全在主线程）。改为 session 最新行（百行级）+ part 按 rowid DESC 定位尾行；真库 EXPLAIN 验证临时 B 树消失，最大会话（8,108 part）实测 0.002ms
- **token 用量快照每拍一次**：引擎每拍 17 profile × 2 处共 34 次「锁 + 全字典拷贝」收敛为 1 次，告警链路复用同一快照
- **扫描热路径去分配**：忽略判定从「切分整条 pathComponents + 逐组件小写化」改为条目 basename 单次小写化（被忽略子树在其目录条目处已被剪枝，语义等价）；心跳判定先判扩展名再计算 stem。全量扫描为 working 态每 5–6s 一次的最重 I/O 路径
- 测试 174 → **175**：node_modules 深层「未来 mtime」判别用例锁死剪枝等价性；独立验收含真库 EXPLAIN 对照与结果逐字节等价核对

## [0.0.23] - 2026-09-12

### 🗄️ SQLite 只读访问层统一：连接缓存与样板收敛（20 轮优化 · R06）

- **新增 `ReadonlyDB` 共享层**：此前 5 个探测器 + 5 个 DB 流水源各自手写 open/prepare/finalize/close 样板，且每拍对大库（数百 MB）现开现关——重复支付 open 成本（~50–150µs/次）并抖动文件缓存。现在统一收口：连接按路径缓存长驻复用（页缓存温热），(设备号, inode) 校验外部替换，open 失败关句柄（0.0.17 泄漏契约延续），缺失文件走 stat 快路径
- **样板收敛**：10 处 open/defer-close 收口后全仓净减 ~400 行重复；标题截断 4 连抄收敛 `clipTitle`；`inspectAction` 的 9 分支 if-else 链改 switch 分发；`AgentLogStreamer.openReadonly` 删除
- **并行执行**：本轮由两个独立执行者分别完成 Inspector 与 Streamer 文件（文件独占无写冲突），主会话建共享层并统一构建；独立验收做了 dim SQL 字面量逐字节核对、真实库行为对照（探测器结果与 sqlite3 CLI 直查一致）
- 泄漏回归测试改测 ReadonlyDB：chmod 000 文件命中 open 失败分支（实测 rc=14、handle 非 NULL），2 万次失败开库内存零增长
- 测试 174/0 连跑 3 次；`--selftest`、`--probe` 全过

## [0.0.22] - 2026-09-12

### 🔍 动作探测门控与会话树枚举加固（20 轮优化 · R05）

- **idle 状态不再做主线程全树枚举**：动作透传（「正在执行 xxx」）此前对每个 running Agent 每 2s 调用一次，`idle` 拍同样枚举 `~/.claude/projects` 等会话目录树（重度用户数千~数万文件，实测量级 5–20ms/次），结果因等级判定在下才被整体丢弃——先算后丢，纯浪费的主线程 I/O。现在探测移到等级判定之后、仅 `working`（含滞回期）执行；working 拍代码路径零变化，idle/offline 拍严格减少工作
- **会话树枚举防失控**：`findNewestFile` 两份逐字相同的私有实现收敛为 `LogTailReader.newestFile` 单实现，新增两项防护——符号链接目录跳过整个子树（防循环），20000 条目预算（防失控枚举长时间占用主线程；超预算优雅退化为「无动作」/「空流水」而非卡死）
- **claude 流水事件 id 稳定化**：事件时间戳此前取 `Date()`，派生 id 拼接时间戳后每次刷新全部改变，稳定 id 的目的（详情展开态保持、增量刷新）对 claude 完全落空。现在优先解析行内 JSON 的 `timestamp` 字段，缺失回落文件修改时间
- 修复过程中发现并记录一个 `NSDirectoryEnumerator` 陷阱：对文件符号链接调用 `skipDescendants()` 会破坏枚举器状态、丢弃后续所有条目——`skipDescendants` 只能对目录调用，与 `FileMonitor.scanTree` 的既有写法一致
- 测试 171 → **174**：idle 不探测（注入计数 hook）、循环符号链接树秒级返回、maxAge/预算边界；独立验收 pass（性能量级声明核对、行为等价逐 case 论证）

## [0.0.21] - 2026-09-12

### 📜 实时流水：事件 id 去重、超长 detail 截断与解析开销（20 轮优化 · R04）

- **稳定事件 id 可碰撞**：同一消息内的多条 part（如连续两次同名 `tool_use`）共享同一 `createdAt`，派生 id「agentId-毫秒-标题」完全相同——该 id 同时是流水列表 ForEach 身份与展开态键，重复时触发 SwiftUI 未定义行为（行互相顶替、展开态错乱）。现在批次内去重：第 2+ 次出现追加序号（取「下一个可用序号」，标题含 `#` 的再碰撞形态也防住），首次出现不变，出现次序由数据行序决定、跨刷新稳定
- **64KB 原始 JSONL 不再整段渲染**：Claude 流水把 `readLastLines` 的最长 64KB 单行原样塞进 detail，dim 的整段 parts JSON、完整消息文本同样无上界——超大 `Text` 在 330pt 卡内一次性排版造成可感知卡顿，且原始 JSON 对用户无诊断价值。现在在唯一构造点统一截断（4096 字符 + 「…[已截断]」标记），完整内容本就无法在卡内滚动查看，截断无损
- **ISO8601DateFormatter 静态化**：初始化 ~0.5ms/个，流水页每 2s 刷新一次，antigravity 每次甚至新建 2 个（0.0.17 已在 TokenUsageMonitor 静态化同类对象，此处为漏网点）
- 测试 169 → **171**：去重语义矩阵（首现不变 / 序号后缀 / 异名不受影响 / 重复调用稳定 / 基础 id 自带 `#` 的再碰撞）、截断边界（含显式 id 路径无旁路）；独立验收 pass（dim 行级截断的整组进出边界、detail 消费点全链路核对）

---

## [0.0.20] - 2026-09-12

### 🧯 脏持久化防护：配置值全字段自愈，收起延迟崩溃循环修复（20 轮优化 · R03）

- **收起延迟脏值导致「收起必崩」循环**：`collapseDelay` 的 init 读取路径没有钳制，`defaults write` 写入负数或超大值（或 plist 损坏）后，`UInt64(delay * 1e9)` 转换对负值/溢出直接运行时 trap——鼠标每次离开展开卡都会触发收起调度，应用表现为每次收起即崩溃。现在 init 读取即钳入 `0.2…5s`（与设置页滑杆同口径），NaN 归位默认值
- **EngineConfig.normalized() 补齐全部字段**：此前只钳 3 个字段，「工作写入窗口」「活跃会话窗口」的脏负值会让文件信号通道与活跃会话计数整体静默失效（监控半残且无提示）。新增 6 个区间常量与设置页滑杆对齐，作为脏值自愈唯一口径；`minWorkingHold` / 死循环阈值 / 告警阈值虽不暴露也一并设防
- **NaN 防御**：`min/max` 对 NaN 透传（比较恒 false），先归位默认值再钳制，覆盖全部 8 个 Double 配置字段
- **停靠锚点 isFinite 防御**：损坏的 `dockAnchorX/Y`（NaN/Inf）会让窗口落在可见区域外且无法拖回，读取时拒绝并回落屏幕中心
- `EngineConfig.load` 注释声明三个非持久化字段的守则，防「设置实效」类缺陷复发（0.0.17 修过的 A1 同型埋雷）
- 测试 166 → **169**：补齐钳制矩阵、NaN 自愈、收起延迟区间（UI 侧接线由独立验收代码评审守护）；`--selftest` 全过；独立验收 pass

---

## [0.0.19] - 2026-09-12

### ⏱️ 引擎时间正确性：时钟回拨防御 + 离线速率基线清理（20 轮优化 · R02）

系统时钟被回拨（NTP 阶跃校正 / 手动调整 / 虚拟机恢复快照）后，引擎全部墙钟锚点的差值都会失真。本轮以「回拨安全的墙钟」替代单调时钟全量迁移（理由：`sample(now:)` 测试 seam 全仓 49 处、ADR-0001 守护采样节律内聚不动；每拍 O(1) 重锚达成同等防御，时钟大幅前跳由既有 resumeGap 断点检测兜底）。

- **回拨后 Agent 永远卡在「工作中」**：`now - lastSignal` 恒为负 → 恒小于滞回时长 `minWorkingHold` → working 判定永不回落，面板/peek 持续显示工作中直到真实时间追平回拨量。现在每拍检测「锚点晚于本拍」即重锚，滞回与任务时长在当前时钟下重新计起（`workingSince` / `lastSignalAt` / `highCpuSince` / `lastRunawayAlertedAt` 四锚点 + token 速率基线时间戳）
- **未来 mtime 负值钳制**：回拨后旧会话文件的 mtime 落在「未来」，负的经过时长不再透传给 `lastActivityAgo` 消费方（UI 文案与下游算术），钳为 0（视作刚写入——回拨前的真实写入确实发生在不久前）
- **offline 清除 token 速率基线**：与 resumeGap 断点口径对齐。此前进程退出后基线时间戳冻结，重启后首个结算窗口把离线全程计入分母，token 激增告警的速率被摊薄
- 测试 163 → **166**：回拨重锚后滞回正常过期（变异验证：删掉防御代码该用例即红）、未来 mtime 钳 0 且随真实时间正常过期、offline 清基线不变量；TestKit 新增 `FakeTokenUsageProvider`
- 全量 166/0 连跑多次无 flaky；`--selftest` 全部通过；独立验收 pass（含变异实验判别力验证）

---

## [0.0.18] - 2026-09-12

### 🛡️ 进程终止安全：身份复核防误杀 + 孤儿进程佐证门槛（20 轮优化 · R01）

本轮起按「20 轮计划」逐轮深度优化（性能 / 健壮性 / UI 交互 / 质量工具链四路独立侦察 → 定案 → 实施 → 独立验收 → 发布）。R01 聚焦全仓唯一会终止用户进程的两条路径：熔断逃生舱与工作台清理。

#### 身份复核：PID 被回收复用后不再可能误杀无关进程

- **熔断横幅的陈旧 PID 陷阱**：告警事件携带的 PID 来自事件产生时刻，用户可能数分钟后才点击「熔断」；期间目标进程若已退出，macOS 可能把同一 PID 分配给完全无关的程序——旧实现直接向该 PID 及其整棵进程树发送 SIGTERM（300ms 后仍存活补 SIGKILL），后果是把无关应用连同其全部子进程整体终止
- 三层防线（各有独立回归测试，删除任一层测试即失败）：
  1. 引擎 `terminateAgent` 终止前取**最新进程快照**，核对 PID 仍匹配该 Agent 的进程；不匹配按「进程已退出或已变更」拒绝，不发送任何信号
  2. `ProcessTerminator` 增加 `expectedPath` 复核：kill 前用 `proc_pidpath` 比对 basename（brew 升级导致的路径整体变化视为同一程序），不一致立即放弃——闭合「快照到 kill 之间」的最后竞态窗口
  3. 新增 `TerminationOutcome` 区分「已终止 / 身份不符 / 发送失败」，失败事件如实归因，延续 0.0.17 的「不谎报成功」口径
- 工作台清理同样携带扫描时记录的 `commandPath` 复核：工具箱列表可能已陈旧，同一防线覆盖单条与批量清理

#### 孤儿进程：不再批量误杀 LaunchAgent 托管的常驻服务

- **问题**：孤儿判定为「PPID==1 且非标准 App 主进程」，但 macOS 上 launchd（PID 1）同时也是一切 LaunchAgent / 登录项的父进程——用户刻意后台化的智能体（`launchctl` 托管的服务型 Agent）每次扫描都会被列成「孤儿进程」并进入一键批量清理，杀掉即任务静默丢失
- **修复**：① 近期仍有会话写入的 Agent（10 分钟窗口，复用引擎活动数据）不再报孤儿——它正在产出工作，不可能被遗弃；② 其余孤儿仍会列出（原因文案如实说明），但只允许**逐条确认清理**：一键批量按钮按可批量条目（死锁 / 内存超限）计数与禁用，复核反馈不再把孤儿计入「未能终止」
- 策略取向「宁可漏杀」：死锁与内存超限证据充分，不受活动佐证豁免

#### 测试

- 用例 **159 → 163**：身份不匹配时拒绝且目标进程存活、同名异径（brew 升级形态）放行、消失 PID 返回 failed；孤儿佐证矩阵（有活动不报 / 无活动报但不进批量 / 死锁不受豁免）
- 引擎成功路径夹具同时独立验证引擎侧成员核对与终止器侧路径复核两层防线
- 全量 163/0 连跑 3 次无 flaky；`--selftest` 全部通过

---

## [0.0.17] - 2026-09-11

### 🩺 全面深度优化：修复 3 个可触发崩溃、状态误判与性能瓶颈

本轮由多轮实测审查驱动，覆盖健壮性、状态判定、设置实效、性能、交互安全与测试。

#### 崩溃与资源泄漏（均可被真实数据触发）

- **越界 Token 值导致整个应用崩溃**：`Int(row[0]) ?? Double(row[0]).map { Int($0) } ?? 0` 的兜底分支自身是陷阱——`Int(Double)` 对越界值 / `Infinity` / `NaN` **直接 fatalError**。数据源一旦出现 `1e19`、`1e999`，或两行 `1e308` 相加使 `SUM` 溢出为 `Inf`，展开面板时进程即 trap（实测退出码 133，用户表现为「侧边栏一打开就消失」）
- **实时流水页事件 id 同一问题**：`Int(timestamp * 1000)` 遇库字段中的 Int64 上限哨兵值同样崩溃
- 新增 `SafeNumber` 饱和解析：越界钳制到量级上限并输出告警，`Inf`/`NaN` 归零，替换全部裸转换
- **SQLite 句柄泄漏**：`sqlite3_open_v2` 失败时仍会分配 handle（实测约 1.5KB/次），但 `defer { close }` 写在 `guard` 之后到不了。这些探测在主线程按采样节律反复执行，库缺失/不可读时约 **25–61MB/天** 常驻内存增长。10 处失败路径补齐关闭

#### 状态判定与设置实效

- **「CPU 判定阈值」此前对 13/17 个内置 Agent 完全无效**：引擎按 id 硬编码 20%/35%，只有 4 个纯 CLI 读用户设置，滑块形同虚设。阈值改为随档案下沉（下限语义，取 `max(下限, 用户设置)`），既让设置对所有 Agent 生效，又保留原硬编码防住的「桌面类空闲抖动误判」
- **睡眠/挂起后误报「任务完成 (480分0秒)」**：完成事件判定只有 3.5 秒下限、没有上限，合盖唤醒后的第一拍会补发完成事件并弹系统通知。新增采样断点检测（`max(120s, 3×闲置间隔)`），断点后跳过完成事件并重置工作区间
- **关闭「死循环告警」会连带关闭卡死检测**：高负载采集原先写在开关分支内，关闭后 `isHung` 恒 false，行内「疑似卡死」徽标、详情页状态与工作台死锁扫描同时静默失效。采集移出开关、无条件执行
- **严重告警被普通事件挤掉**：`latestEvent` 是单槽位，实测 Token 激增横幅几秒内就被其他 Agent 的「任务完成」顶掉，用户来不及处置。新增统一发布入口，告警后 30 秒内普通事件不覆盖
- **清理失败却谎报成功**：「已安全清理 0 个异常进程…系统资源已就绪」在一个都没杀掉时照样弹出。现在按结果区分文案，失败发「未能终止」提示
- 采样间隔钳入 `0.5–600s`：脏持久化值写成 0 时实测每秒采样 **2895 次**（忙循环）

#### 性能（实测数据）

| 指标 | 优化前 | 优化后 |
|---|---|---|
| 收起态 CPU（均值 / 峰值） | 2.55% / 12.8% | **1.91% / 7.8%** |
| 会话目录全量扫描 | 101 ms | **51 ms** |
| 主线程采样耗时 | 15.3 ms | **10.8 ms** |
| 实时流水页查询 | 161 ms | **2.67 ms** |

- 移除两个伪会话目录（opencode 配置目录含 `node_modules`、openviking 的 uv venv），二者合计占全量扫描 **57%** 的时间却与任务无关；忽略集补充 `node_modules`/`site-packages`/`.venv`/`__pycache__`/`.git`/`DerivedData`/`Caches`
- 进程匹配预计算小写路径：19 个 profile × 整张进程表此前每拍重复 `lowercased()`
- 告警链路复用主循环已算出的 CPU/PID，不再二次全表匹配

#### 交互安全

- **告警横幅「熔断」按钮单击即终止整棵进程树**，无二次确认（同一动作在列表行与工作台都有确认）。改为两段式确认，已端到端实测：首击后目标进程仍存活，二击才终止
- **系统通知绕过应用自己的通知策略**：「完全静默」下仍弹通知并响铃、「专注免打扰」下每次完成都响、标准模式下与岛内提示音叠加成双重音。现在按策略裁决投递，声音统一由岛内 `NSSound` 承担（不依赖通知权限）
- **浅色主题下多处内容不可见**：环形水位与 8pt 副标题在白底对比度仅 1.03:1，现改双值动态色（3.10:1，白底 5.08:1）；流水页与悬停卡的硬编码黑白改动态色
- 鼠标事件节流此前是死代码（每个 `mouseMoved` 都全量重算边缘判定并新建 Task），接入后全局监听的 Task 创建由约 60/s 降至 17/s
- 拖拽统一走原生 `performDrag`：删除闭包版入口与其失效的坐标钳制逻辑

#### 测试

- 用例 **102 → 159**，新增窗口高度全组合、清理规则矩阵、可见口径边界、配置归一化、内存文案一致性等
- 新增 `IslandMetricsKit` target 以符号链接纳入 UI 层纯几何文件，使「窗口高度 vs 子项之和」这组历史高频回归可被断言；配漂移哨兵，路由变化时用例会失败提示同步
- 全仓编译警告 1 → **0**

---

## [0.0.16] - 2026-09-10

### 🔔 完善任务通知、状态识别与告警展示

- **系统通知覆盖完整事件链**：任务完成、等待确认、Token 激增、持续高 CPU/疑似死循环和终止失败等事件均投递到 macOS 通知中心，并显式使用默认提示音与主动通知级别；事件 UUID 保证不重复发送。
- **DeepSeek Harness 状态及时跟随**：深层会话目录强制重扫周期收紧至 5 秒，避免常驻 UI 因目录缓存长时间停留在待机状态。
- **DimAgent 状态误报修复**：排除 `file-history` 与 `blobs` 编辑历史/附件缓存子树，避免没有实际任务时被后台同步写入误判为工作中。
- **告警文字可读**：展开页顶部直接显示告警摘要，展开原因可查看完整排查建议；Agent 名称和动作过长时单行省略，悬停显示完整内容。
- **完成通知声音修复**：通知内容显式配置系统默认声音，并修正通知正文插值。

## [0.0.15] - 2026-09-09

### 🐛 修复状态跟踪误报、Token 口径虚高、底部汇总栏裁切与贴边阴影

- **进程关闭不再误报「任务已完成」**：
  - 此前 `offline` 分支复用完成事件路径，把「进程被用户关闭」等同于「任务执行完毕」，手动退出 ChatGPT 也会弹出完成横幅
  - 现在进程消失只静默转 `offline`；完成事件仅由「进程仍在但工作信号消失」产生
- **Token 净消耗口径统一（dim 源）**：
  - `usage_ledger.usage.promptTokens` 含缓存命中部分，此前直接与 completion 相加导致缓存重复计入，累计用量被大幅虚高（数十倍量级）
  - 汇总、模型拆分、会话列表三处统一改为 `(prompt − cacheRead) + completion` 并逐行钳制非负，与 opencode 侧口径对齐
- **激增告警改为速率制 + 连续确认**：
  - 此前「300 秒内增量 ≥ 阈值」把长任务结束时一次性落盘的巨额 ledger 记录当成瞬时激增
  - 现在按每分钟净消耗速率判定，且需连续 3 个周期超阈值才告警；正常绘画/长任务不再误报
- **修复 peek 微弹窗导致侧边条错位**：
  - 此前 peek 只拉伸窗口 frame、`displayState` 仍为 `docked`，SwiftUI 只渲染 6pt 细条，于是细条被拉到展开位置且卡片内容缺失
  - 现在 peek 走真实 `displayState` 切换，与窗口尺寸同源，展开内容正常呈现
- **修复展开卡底部 Token 汇总栏被裁切**：
  - `IslandMetrics.expandedHeight` 漏算顶部「实时活动环微看板」（40pt 内容 + 1pt 分割线），渲染内容比窗口高出约 56pt，超出 460pt 上限的部分从底部裁掉，汇总栏只露出半行
  - 现在展开高度计入看板（新增 `chromeHeight`/`listHeight` 纯函数统一口径），触顶时压缩可滚动的 Agent 列表（保底一行）而非裁切汇总栏
  - 二次修复（首次展开正常、挪动后又被裁）：实测 `NSHostingView.fittingSize` 比常量推导值高 5.5pt——顶栏内容行高按 17pt 估算偏小（圆形图标按钮实际约 23pt），且汇总栏自身那 1pt 分割线未计入 `chromeHeight`。现按实测校准常量，并让窗口高度取「常量推导」与「内容理想高度」的较大值（`resolvedExpandedHeight`），`placeWindow`/`snapToDockEdge`/`syncExpandedHeight` 三处同源，常量再漏算也不会把汇总栏挤出窗口
- **移除面板阴影内渗，消除贴边侧上下暗带**：
  - 面板窗口与玻璃卡尺寸完全相同（330×447），AppKit 阴影没有卡片之外的落地空间，只会沿轮廓边缘向卡内渗入约一个模糊半径，在贴屏幕一侧的上下直角区域形成暗块（用户反馈的「两侧直角矩形的上下阴影」）
  - 现已停用面板阴影；一体化贴边的观感由玻璃卡自身的 1px 高光边缘与反向倒角承担
- **修复 ChatGPT 与 Codex 重复显示为两个 Agent**：
  - ChatGPT 桌面版把 Codex 打包进 `/Applications/ChatGPT.app/Contents/Resources/codex`，其 basename 与独立 `codex` CLI 相同且共用 `~/.codex` 会话目录，此前被数成两个 Agent
  - `AgentProfile` 新增 `pathExcludes`（路径排除）与 `hostBundleIDs`（宿主识别）：宿主已安装且该组件无独立安装时不再单独成条目；独立安装 codex CLI 的用户仍照常监控
  - 手写 `Codable` 解码（`decodeIfPresent` + 默认值），保证升级前保存的自定义 Agent 不因新增字段而整条失效
- **修复展开卡内容超高时底部汇总栏被压掉**：
  - 事件提醒栏出现后内容超过 460pt 上限，此前由 `VStack` 自行分配压缩，末尾的 Token 汇总栏成了牺牲品
  - 现给列表 `layoutPriority(-1)`、汇总栏 `layoutPriority(1)`：空间不足时只压可滚动的列表，汇总栏保持完整
- **性能：消除每 2 秒的主线程阻塞与 CPU 尖峰**（实测发现）：
  - `AgentActionInspector.activeChildCommand` 原先 fork `/usr/bin/pgrep` + `/bin/ps` 并 `waitUntilExit()`，单次 67ms，17 个 Agent 一轮 762ms 全部落在主线程；改为 sysctl `KERN_PROCARGS2` 直读命令行 + 复用采样快照做内存 BFS，`inspectDimAction` 单次由 170–220ms 降至 **0.9ms**
  - `inspectDimAction` 的 `ORDER BY createdAt DESC LIMIT 1` 在数万行 / 数百 MB 的 `messages` 表上退化为全表扫描 + 临时 B 树排序（220ms/次）；改用 `rowid = (SELECT max(rowid) …)` 走主键查找
  - `inspectOpenCodeAction` 的 `session LEFT JOIN part` 全表排序实测 330–964ms；改为「先取最新会话，再取该会话最新 part」，降至 8ms
  - `ProcessTerminator.getProcessTree` 与 `AppActivator` 的父进程追溯同样去掉逐节点 fork，改用一次快照内存遍历
  - 实测收起态 CPU 由均值 8.0% / 峰值 39.2% 降至 **1.2% / 2.8%**
- **修复状态误判：Agent 恒显「工作中」**：
  - `inspectAction` 返回的「最近动作」被当作核心工作信号，而各探测源在 Agent 空闲挂起时仍可能命中旧记录（dim 分支甚至无视注入的 fake 直读真实 SQLite），导致 `working` 永不消退、完成事件永不产生。现在工作状态只由「文件写入 + CPU」决定，动作仅作展示字段
  - 长驻子进程（MCP server、language server、`server.js`、`--liftoff-only` 索引进程）不再被判为「正在执行的任务」
  - `KERN_PROCARGS2` 解析按 `argc` 截断，避免把 `PATH=…` 等环境变量当成用户命令
  - 修复滞回锚点：原用「首次进入 working 的时刻」判断，任何超过 `minWorkingHold` 的任务滞回完全失效；改用每拍刷新的 `lastSignalAt`
  - 修复后 3 个长期失败的环境依赖用例全部转绿，测试 **84 通过 / 0 失败**
- **修复危险操作的可信度**：
  - 终止按钮在 `pid == nil`（GUI bundle 命中但进程名未匹配）时此前不发信号却宣告「进程已终止」——假成功；现在如实提示「无法终止：未定位到进程」并返回 `false`
  - 终止成功由 `attention` 改为 `completed`，收起态细条不再误报红色告警
  - 工具箱单条清理补二次确认（杀的是整棵进程树）；`overweight`/`hung` 不再把 `/Applications/*.app/Contents/MacOS` 主进程列为可清理项（开着大项目的 Electron IDE 占 2.5GB 属正常）
  - `ProcessTerminator.terminate` 返回真实信号发送结果，清理横幅不再谎报「已释放 N 个进程」
  - 工具箱扫描的 `NSWorkspace` 调用移回主线程（`ProcessProviding` 线程契约）
- **修复交互与显示缺陷**：
  - hover tooltip 内的按钮永远点不到：popover 由 26×26 环的 `onHover` 驱动，鼠标移向 popover 时立即触发关闭；改为 400ms 延迟关闭 + popover 内 hover 取消
  - 终止确认态不再于 Agent 转 idle 后残留（避免误杀已空闲进程），并用可取消 `Task` 替代 `DispatchQueue` 定时器
  - 顶栏优先展示「带动作」的 working Agent 并显示 `+N` 并行数；可见计数降为可压缩，长名称/动作不再被挤断
  - 事件横幅的按钮/背景改用动态色，浅色主题下不再白底白字不可见；关闭按钮热区 15→21pt
  - 「直达」在无对应 Agent 时置灰并说明，不再点击无反馈；熔断按钮在无 PID 时给出去向指引
  - 工作态 Agent 也显示 token 徽标（正在消耗的最需要关注）；活动环副标题与行内口径统一
  - 实时流水页返回按钮回到进入前的层级；事件 id 改为确定性生成，展开的详情不再每 2 秒被强制折叠
  - 脉冲与呼吸动画尊重系统「减弱动态效果」；版本号从 bundle 读取而非硬编码

---

## [0.0.14] - 2026-09-09

### 🩺 修复 Antigravity 监控识别与偏好设置自动自愈迁移

- **启用集自愈与向前兼容算法 (`EnabledAgentStore.resolvedEnabled`)**：
  - 彻底解决旧版本持久化 `enabledAgents` 导致新增内置智能体（Antigravity、ZCode、DSH、ChatGPT 等）被静默过滤的问题
  - 引入 `knownAgents` 持久化机制与 `legacyKnownAgentIDs` 基线迁移，存量用户升级时自动将新加入且默认启用的内置智能体合入启用集
  - 严格保持用户主动全关（`[]`）与单项显式关闭偏好，不发生意外覆写
- **用户环境自动修复与直达**：
  - 启动阶段与偏好设置面板同步自动自愈，Antigravity 无需用户手动翻找开关即可立即呈现在灵动岛监控与菜单栏中
  - 新增 4 组单元测试（首次安装/主动全关/历史存量迁移/显式关闭记忆），全量测试 64/64 保持全绿

---

## [0.0.13] - 2026-09-08

### ⚡ 智能体实时事件流与日志流水抽屉 (Live Log Stream)

- **智能体事件流与日志流水提取引擎 (`AgentLogStreamer`)**：
  - 支持多源并发智能日志采集：零侵入解析 Antigravity (`transcript.jsonl`)、Codex (`rollout-*.jsonl`)、DimAgent (`dimcode.sqlite`)、Claude Code、OpenCode、ZCode、WorkBuddy、Hermes 等
  - 提取高价值时序结构化数据：精准识别终端命令执行 (`EXEC`)、工具/MCP调用 (`TOOL`)、代码文件编辑 (`EDIT`)、深度推理思考 (`THINK`) 与模型对话流 (`MSG`)
- **原生极客暗黑风实时流水视图 (`LiveLogStreamView`)**：
  - 灵动岛主卡行与二级详情页增设终端图标（`terminal`）直达流水抽屉
  - 极客暗黑终端配色、彩色事件类型徽标、时间戳微调与参数展开查看
  - 支持实时静默自动跟随刷新（每 2 秒）与随时暂停切换
  - 提供一键复制全部诊断日志流水至剪贴板功能，便于排查与分析
- **窗口几何与导航联动**：
  - 深度集成 `IslandMetrics` 与 `IslandPanel` 自适应高度体系，保障展开卡片内无缝平滑滚动

---

## [0.0.12] - 2026-09-08

### 🛠️ 智能体维护工作台与进程清理系统 (Agent Workbench Cleaner)

- **原生工作台维护视图 (`ToolboxView`)**：
  - 展开卡顶栏增设快捷工具箱按钮（`wrench.and.screwdriver`），一键进入系统级 Agent 维护工作台
  - 展示待维护异常项、预估可回收内存与智能体整体健康度评分
- **三维智能体异常诊断 (Anomalies Detection)**：
  - **孤儿进程检测 (Orphaned Processes)**：自动捕捉主控终端被关闭后、父进程转为 `launchd` (PPID=1) 且脱离控制台的遗留 Agent 进程
  - **疑似死锁/假死检测 (Deadlocked / Hung)**：检测持续异常高载且失去会话响应的卡死任务
  - **内存超限检测 (Overweight Leaks)**：标识单进程驻留集物理内存超过 2.0GB 的潜在堆内存泄漏
- **一键安全清理与资源回收**：
  - 支持多选单项清理与一键安全全量清理，先尝试优雅信号通知保存状态，超时未退出则强制杀死并级联释放子进程树
  - 清理完毕后动态展示释放进程数与物理内存回收横幅，并触发自动刷新与状态自愈

---

## [0.0.11] - 2026-09-08

### 🩺 Agent 性能健康仪表盘与线程死锁检测

- **物理内存微秒级精准读取 (RSS Footprint)**：
  - 基于 Darwin 原生 `proc_pid_rusage` 读取物理驻留集 `ri_resident_size`，零外部命令开销，毫秒级反映各 Agent 真实物理内存占用
  - 在 `AgentRowView` 列表中增设内存紧凑徽标（如 `280M`、`1.2G`），卡片悬停 Tooltip 及详情页均可实时透视各 Agent 内存与 PID
- **死循环/高负载死锁异常检测 (Deadlock / Hung Detection)**：
  - 关联采样时间序列，当智能体发生非预期的高 CPU 持续占用（超出熔断阈值）或无会话响应时，自动在列表与详情页中标记「疑似卡死」健康警示
  - 支持一键安全终止与逃生舱清理，保障系统资源与开发机温度
- **二级详情页性能与健康矩阵**：
  - `AgentDetailView` 嵌入 CPU、物理内存与 PID 概览小方块，与 Token 用量协同构成完整的 Agent 资源透视图

---

## [0.0.10] - 2026-09-08

### 💎 灵动岛收起态边缘微胶囊视觉动效强化 (DockedSliverCapsule)

- **多状态一眼感知**：
  - **工作中 (Working)**：翡翠绿微光呼吸光晕与中心状态点呼吸流动，多 Agent 并行或后台生成时屏幕边缘清晰可感
  - **严重告警/待关注 (Alert)**：微红/琥珀金微光呼吸警示（针对熔断、死循环高负荷及进程异常），无需展开面板即可在屏幕边缘获知关键事件
  - **空闲待机 (Idle)**：晶莹半透明微晶胶囊，极致低调不扰工作
- **零额外能耗保证**：
  - 采用平滑缓和的 2.0s/1.2s 周期呼吸动画，仅在有明确状态（工作或告警）时开启；空闲待机时完全复位休眠
  - 完善 `IslandPanel` 对事件变更的观察管道，收起态状态无缝同步
- **边缘贴合自适应**：
  - 完美适配顶部吸附（横向微胶囊 140x6pt）与右侧吸附（纵向微胶囊 6x120pt）两种形态

---

## [0.0.9] - 2026-09-08

### 🎯 状态判断精准化与 Agent 工具生态全覆盖

- **消除 Electron 后台待机误报 WORKING**：
  - 将 `cpuThreshold` 默认阈值微调为 `6.0%`，彻底避开 Electron/Chromium UI 渲染器固有空闲抖动（1%~5%），真正在进行代码编译、大文件检索或模型计算时才触发 CPU 状态跃迁
  - 修正 WorkBuddy `sessionDirs` 为实际数据目录（`~/.workbuddy/sessions`, `~/.workbuddy/tasks`, `~/.workbuddy/memory`）
- **WorkBuddy 动作与状态时效性强校验**：
  - 加入 5 分钟更新新鲜度校验，超过 5 分钟未更新的会话标记为「待机」，绝不因陈旧未归档记录误报「正在处理」
- **补全本机 AI Agent 工具全生态覆盖**：
  - **Antigravity Studio**：支持 `com.yuzhiqiang.antigravity.studio` 独立识别并归并至 Antigravity 生态
  - **Ego Browser (Ego Lite)**：支持 Agent 专用隔离浏览器（`com.citrolabs.ego.lite` / `ego-browser`）
  - **Vibe Usage**：支持智能体 Token 用量聚合看板（`ai.vibecafe.vibe-usage` / `vibe-usage`）
  - **OpenViking**：支持本地 AI 知识库与智能体执行器（`openviking`, `openviking-server`, `ov`, `vikingbot`）
  - **扩充 CLI 检测池**：覆盖 `bsk` (Browser Skill)、`cua-driver` (Computer Use Driver) 等

---

## [0.0.8] - 2026-09-08

### ⚡️ 扩展主流 Agent 深度动作透传与会话解析

- **WorkBuddy 实时任务解析**：从 `workbuddy.db` 深度解析当前活跃会话标题、状态（`正在: ...` 或 `任务: ...`），精准透传工作上下文
- **OpenCode 深度解析**：支持从 `opencode.db` 的 `session` 与 `part` 提取实时思考规划（`思考规划中`）、工具调用（`正在调用: ...`）或当前会话主题
- **DSH (DeepSeek Harness) 模式感知**：结合进程命令行参数与会话状态，透传 Web 协作模式（`Web 协作服务运行中`）或任务执行详情
- **Hermes 会话与动作透传**：从 `state.db` 提取最近会话与活动描述
- **ZCode 任务检查器增强**：加入 `deleted = 0` 软删除过滤并放宽时间窗口，使进行中的任务持久准确透传

---

## [0.0.7] - 2026-09-08

### 🔔 告警与通知卡片交互升级与全量排查信息展示

- **事件卡片富文本展开（EventBannerView）**：告警卡片支持「精简两行」与「完整展开」双态切换，告警发生原因、阈值说明与排查建议一览无余，彻底杜绝文本截断 `...`
- **可解释性告警详细排查说明**：
  - **Token 激增告警**：显示具体监控时间跨度、增量绝对值、报警阈值与多 Agent 并发/Prompt 死循环排查指引
  - **死循环与高 CPU 告警**：展示持续分钟数、当前 CPU 百分比、进程 PID 以及熔断逃生舱操作建议
  - **任务完成通知**：展示实际耗时与状态转空闲说明
  - **终止逃生舱通知**：记录释放的目标 PID 与信号处理结果
- **一键复制诊断信息**：展开态下提供「复制诊断」按钮，可一键将 Agent ID、PID、发生时间、摘要与完整排查建议复制至剪贴板
- **窗口高度动态弹性自适应**：展开/折叠事件卡片时，面板高度自动平滑扩展（从 66pt 增至 142pt），保证下方的 Agent 列表不被遮挡或挤出窗口
- **严重告警自动展开**：对 Token 激增与死循环等熔断级警告，提醒时默认展开排查建议，协助用户迅速决策

---

## [0.0.6] - 2026-09-08

### 🔧 Agent 识别与状态检测全面修复

- **修复 ChatGPT/Codex BundleID 冲突**：`com.openai.codex` 实为 ChatGPT 桌面版，已独立为 ChatGPT profile，Codex 改为纯 CLI 检测
- **新增 ChatGPT 内置 profile**：`com.openai.codex` bundleID，独立监控 ChatGPT 桌面版运行状态
- **新增 DSH (DeepSeek Harness) 内置 profile**：进程名 `dsh` + pathContains `deepseek-harness`，会话目录 `~/.dsh/sessions` & `~/.dsh/storages`
- **修复 OpenCode.app 不被识别**：添加 `ai.opencode.desktop` bundleID，GUI 与 CLI 双路径均可检测
- **修复 Hermes sessionDirs 路径错误**：`~/.local/share/hermes`（不存在）→ `~/.hermes/sessions` + `~/.hermes/logs`（实际数据位置）
- **Probe CPU 双拍差分**：`--probe` 改为两次采样（1.5s 间隔），输出真实 CPU% 窗口值（修复永远 0.0 的问题）
- **闲置态采样提速 3×**：`idleSampleInterval` 从 15s 降至 5s，Agent 开始工作后最迟 5s 即被感知
- **InstalledAppsCache 同步**：`knownBundleIDs` 与 `knownCLIs` 与注册表完全同步

---

## [0.0.5] - 2026-09-07

### 💎 深度设计重构（CodeNotch 灵感：一体化反向倒角、微仪表环与悬停透视卡片）
- **反向倒角一体化贴边造型（SideNotchShape / Bezel Flares）**:
  - 彻底去除普通矩形切边的生硬感，借鉴 CodeNotch 与苹果硬件刘海的数学级 Bézier 曲线反向倒角（Flare）；
  - 小岛在屏幕右侧或顶部停靠时，贴边边缘自然平滑向屏幕边框弯曲延伸，如同从屏幕边框一体化生长出来；
  - 毛玻璃拟态、双色渐变蒙层、晶莹描边与窗口投射高斯阴影全链路对齐反向倒角曲线。
- **环形微仪表盘与双层动态活动弧（AgentRingView & ActivityArc）**:
  - 为所有 Agent 列表项与快速微看板引入 4 级彩色水位环（荧光绿、琥珀黄、预警橙、极光红）；
  - 居中渲染智能体高辨识度专属 Glyph；
  - **双层动效**：Agent 工作时内圈展开 0.25 长度的极细旋转微弧（1.2s 周期平滑旋转）；等待确认时呈现琥珀色呼吸警戒环；
  - 展开卡片顶部新增活跃智能体微看板（Quick Rings Shelf），一眼看清全局负载。
- **精准悬停透视卡片与指向小尾巴（AgentHoverTooltip & TooltipTail）**:
  - 鼠标悬停在任意 Agent 环上时，滑出带有指向尖角的悬浮透视浮层；
  - 零点击直达实时事实：当前执行的具体命令/正在修改的文件、进程 PID、24h/累计 Token 消耗与花费；
  - 卡片内一键直达终端/IDE 窗口或触发终止逃生舱。

---

## [0.0.4] - 2026-09-07

### 🔕 核心新特性：免打扰与通知分级 (Focus Mode & Notification Filtering)
- **三大通知策略模式（NotificationPolicy）**:
  - **专注免打扰（Focus Mode，推荐并默认）**：普通任务执行完毕静默更新（不弹窗微窥、不响提示音），仅在微细条或手动展开卡片内呼吸展示，彻底解决频繁编码被打扰的痛点；当发生**成本暴涨突增、死循环熔断告警（costSpike）**等紧急危险事件时，依然立即滑出 6 秒微弹窗并播放告警音。
  - **标准模式（Standard Mode）**：所有任务完成、等待确认、异常告警均滑出 3.5s 微弹窗并播放轻脆提示音（适合挂机等待智能体交付）。
  - **完全静默（Silent Mode）**：绝不滑出任何微弹窗，绝不播放任何提示音，纯后台静默记录与展示。
- **彻底根治无谓弹窗遮挡**:
  - 移除此前智能体一启动工作（`working`）就触发弹窗的过度打扰行为，还用户沉浸式专注编码体验。
- **全入口一键切换**:
  - 顶部菜单栏 Popover 底部操作栏新增通知模式切换快捷菜单（带对号与动态高亮）；
  - 灵动岛右键上下文菜单提供「通知模式」子菜单；
  - 设置面板「通用与外观」新增精致「通知与免打扰模式」卡片，并配有直观规则说明。

---

## [0.0.3] - 2026-09-07

### ✨ 核心新特性 (Appearance & Native Integrations)
- **深浅外观模式与跟随系统 (Appearance Modes)**:
  - 全局支持「跟随系统」、「浅色模式」、「深色模式」三态实时无缝热切换；
  - 灵动岛展开态顶栏右侧新增快捷主题切换图标按钮；
  - 菜单栏状态项 Popover、分栏设置面板、灵动岛右键上下文菜单均支持一键切换；
  - 玻璃拟态与单向圆角阴影（深色浓郁高斯阴影 vs 浅色晶莹投影）随主题自适应。
- **原生支持 Google Antigravity 智能体**:
  - 原生识别 Antigravity 进程与 CLI 会话目录；
  - 实时解析智能体执行轨迹（trajectory logs）与 tool_use 动作（如 `正在修改: ...`、`正在运行命令: ...`）；
  - 任务完成主动提醒并支持终端/IDE 窗口深度直达。
- **原生支持 ZCode 智能体**:
  - 会话日志感知、实时动作透视与一键直达。

### 🐞 关键修复与交互打磨 (Stability & Interaction Fixes)
- **灵动岛折叠与展开防抖重构 (Collapse & Gesture Refactor)**:
  - 修复边缘微细条几何判定误判（彻底移除卡片滑动过渡态对光标的误判），光标移出卡片后 0.5 秒平滑稳定收回贴边，绝不回弹；
  - 鼠标移入卡片立即解除手动展开保护期，无需等待固定延迟；
  - **点击外部区域自动收起（Click-outside to dismiss）**：在展开卡片外部任意桌面或窗口点击即刻平滑折叠；
  - **顶栏新增一键显式收起按钮**（右侧边栏为 `chevron.right`，顶部灵动岛为 `chevron.up`），右键上下文菜单同步提供「收起灵动岛」；
  - 重构顶栏拖拽把手为背景层，彻底解决拖拽手势遮挡外观切换菜单与收起按钮的问题。
- **死循环告警判定与横幅排版调优**:
  - 重构高负载检测算法，引入基准采样比对，避免短时编译与常规编码误报；
  - 告警横幅重构为双行自适应卡片，告警描述完整展示支持 Tooltip，操作按钮独立成行。
- **进程路径误报防御**:
  - 强化 `pathContains` 约束，避免同名 Electron 进程导致未安装智能体误报。

### ⚡️ 质量与用例
- 测试套件扩充至 56 个全自动化单元测试用例，全绿通过（`56 通过, 0 失败`）。

---

## [1.5.0] - 2026-09-07

### ✨ 核心新特性 (Practicality & Command Center)
- **任务完成主动提醒与智能微窥 (Event Peek & Sound)**:
  - 智能体经历持续工作（≥3.5秒）后转为空闲时，自动播放 macOS 原生轻脆 `Glass` 提示音；
  - 处于 6pt 贴边收起态时，自动平滑滑出 3.5 秒 Peek 微弹窗；若光标移入则自动升级为常驻展开态，移开后平滑收回；
- **终端与 IDE 窗口一键深度直达 (Window Deep-Linking)**:
  - 毫秒级递归追溯进程树父节点，精准定位承载 CLI 智能体（Claude Code、Codex、Dim 等）的 GUI 终端窗口（Terminal、iTerm2、VS Code、Cursor、Ghostty、Warp 等），一键拉至最前聚焦；
  - GUI 智能体（Cursor、Trae、DimAgent 等）直接通过 BundleID / PID 唤醒；
  - 任务完成横幅与 Agent 列表行均提供「直达」快捷按钮；
- **实时操作与工具调用透视 (Real-time Action Context)**:
  - 自动提取子进程实时执行的系统命令（如 `swift test`、`git diff`、`npm run build` 等），内置智能格式清洗器剥离 shell 包裹层；
  - 解析 DimAgent / Claude / Codex 运行时会话日志与 tool_use 元数据，顶部卡片与列表行实时呈现终端样式徽标（如 `> 正在修改: IslandView.swift`）；
  - CLI `--probe` 终端诊断命令新增 `ACTION` 实时动作列；
- **成本与异常死循环熔断保护 (Runaway Loop & Cost Circuit Breaker)**:
  - **Token 暴涨告警**：基于滑动窗口差分监测单分钟 Token 增量，超过阈值时触发红色/琥珀色高亮告警并播放低沉警示音；
  - **长耗时死循环告警**：高负荷工作超 3 分钟未释放时自动预警；
  - **一键 Kill 逃生舱**：告警横幅提供红色「熔断」按钮；列表行提供红色停止按钮，并配有 3 秒自动取消的「终止?」二次防误触确认，安全杀死整棵子进程树；
- **设置面板扩展 (Circuit Breaker Settings)**:
  - 「引擎与性能」Tab 新增「成本与异常熔断保护」配置卡片，支持 30k/50k/100k/200k tokens 阈值调节及各项开关。

### ⚡️ 质量与稳定性 (Quality & Testing)
- 测试套件扩充至 53 个全自动化测试用例，全绿通过；
- `--selftest` 进程内自检断言全部通过。

---

## [0.0.2] - 2026-09-07

### ✨ 交互与界面革新
- **自由移动与智能贴边吸附**: 支持按住顶栏全屏幕任意拖拽，松手根据物理距离智能吸附到屏幕顶部或右侧，并持久化锚点坐标；
- **6pt 晶莹微细条与弹性弹出**: 收起时保留 6pt 半透明微细条（含呼吸绿灯），光标碰触以流体弹簧动效自动弹出完整卡片；
- **现代分栏设置窗口**: NavigationSplitView 四大分类架构，支持贴边重置与外观切换；
- **顶部菜单栏 Compact Popover**: 现代原生浮窗浮动展示活跃 Agent 概览。
