# mattpocock/skills：269,700 star 的 skill 集，以及我们本机 24 个 skill 的上游规范

> 目的：[mattpocock/skills](https://github.com/mattpocock/skills)（GitHub API 2026-09-26：
> **269,700 star / 22,724 fork / Shell / MIT / 528 open issues** / homepage aihero.dev/skills）。
> 自述 "Skills for Real Engineers. Straight from my `.agents` directory."
> 它不是竞品也不是被监控对象——**它是我们本机已装的 24 个 agent skill 的上游**。
> 本文读了 README 全文（12059 字符）与 `.agents/` 下三份元规范
> （`invocation.md` / `writing-docs.md` / `install-block.md`），未读各 skill 的 SKILL.md 本体。

## 一句话结论

**它的价值不在那些 skill，在 `.agents/` 下三份元规范。** 其中 `invocation.md` 定义的分界规则
（user-invoked 不能调用另一个 user-invoked；依赖要写成"调用 Skill 工具并指名"而不是
跨目录相对链接）**直接命中我们 AGENTS.md 里一处写错的做法**；而本机已装它的 24 个 skill、
AGENTS.md 却只记了 `libraries-dev` 一个——这是一条实的文档缺口。

## 它是什么

Matt Pocock（TypeScript 教育者）把他每天在用的 agent skill 开源出来。自述动机是修四类常见失败模式：

| # | 失败模式 | 他的解法 |
| :--- | :--- | :--- |
| 1 | **The Agent Didn't Do What I Want**（对齐失败） | grill：让 agent 反过来盘问你 |
| 2 | **The Agent Is Way Too Verbose**（术语不通） | 共享语言的文档，帮 agent 解码项目黑话 |
| 3 | **The Code Doesn't Work**（缺少反馈环） | 静态类型 + 浏览器访问 + 自动化测试 |
| 4 | **We Built A Ball Of Mud**（复杂度加速膨胀） | "caring about the design of the code"——每层 skill 都内建这条 |

设计原则（自述）：**small, easy to adapt, composable；不拥有你的流程**。原话点名批评
GSD / BMAD / Spec-Kit："they take away your control and make bugs in the process hard to resolve"。

Skill 分两桶：`engineering/`（代码工作）与 `productivity/`（通用工作流），
另有 `misc/` `in-progress/` `deprecated/` 不对外发布文档页。

## 元规范一：invocation（对我们最有用）

`.agents/invocation.md` 全文 3848 字符，定义了**唯一的分界轴：谁能调用它**。

- **User-invoked**（只能人打出名字）：frontmatter 设 `disable-model-invocation: true`（Claude Code）
  与 `policy.allow_implicit_invocation: false`（Codex 的 `agents/openai.yaml`）。
  **`description` 是给人看的**——一行摘要，**要去掉 trigger 列表**（"Use when the user says…"）
- **Model-invoked**（模型或人都能）：默认。`description` 面向模型，保留丰富触发措辞
  （"Use when the user wants…, mentions…, asks for…"）让自动触发能命中。
  判据一句话：_could the model usefully reach for this autonomously？_
  （**"复用"是抽 skill 的理由，不是判据**）

两条不变量值得逐字记：

> Each harness excludes a user-invoked skill from the model's reach in its own way, so **nothing but
> the human can fire it: no other skill can**. A user-invoked skill may invoke model-invoked skills,
> but it can **never** reach another user-invoked skill.

> Dependencies are expressed as an explicit instruction to **call the Skill tool** with the named skill
> (`Call the Skill tool with "grilling"`), **not deep `../other-skill/FILE.md` cross-references**,
> and not a bare `/skill`-style mention left for the model to interpret.

**这条直接命中我们 AGENTS.md 的写法**：[AGENTS.md](../../AGENTS.md) 的 Installed skills 一节写的是
「`.claude/skills/<name>` 是指向它的相对软链」——那是**安装事实**没错，
但我们自己的 skill（`docs/research/ui/` 那 14 篇案例与 `docs/workbench/` 的调研）在互相引用时
用的正是 `../workbench/xxx.md` 这种跨目录相对路径。
上游规范说的是**skill 之间**的依赖要用工具点名；文档之间的引用用相对路径没问题（我们也确实更愿用相对路径），
但**这条界线值得写明**：引用给人看的文档可以用相对链接，**给 agent 下的操作指令必须点名工具**。
（本仓自己在 CHANGELOG 与案例库里已经在用"先读 X 那份"这种指路式写法，方向一致。）

### 一个我们能用的自检问题

上游给了 user/model 分界的判据「模型能不能自己有用的伸手去拿」。
拿来问我们本机的 skill 列表：

- `grill-me` / `grilling` / `grill-with-docs` **三个都在**（我的可用列表里确实有），
  且都属 user-invoked。它们的分工（纯盘问 / 带文档的盘问 / 与我无关的路由）**没有一张表说清**。
  它们的上游 README 用"哪条链的一环"来说清角色；我们本机缺这张表。
- `implement` / `to-spec` / `to-tickets` 是一条链（SPEC → 票 → 实现），
  上游说 user-invoked 之间**不能互调**，所以这条链必须靠人在节点上推进，而不是一个 skill 自动串下一个。
  → 我们跑 workbench 那轮改造时正是这么做的（先 grill 再 to-spec 再汇总），**当时是自发的，没有规范写明**。

## 元规范二：install-block（一条我们踩过的坑，它写明了）

`.agents/install-block.md` 的核心是**两条安装路径互斥**：

> The plugin is a managed, read-only bundle you **subscribe** to. skills.sh writes files **you own
> and edit**. Installing both leaves the user with every skill twice: **always say "pick one"**.

对照我们：本机这 24 个 skill 装在 `~/.agents/skills/`（全局）——**与 `libraries-dev` 装在
本仓 `.agents/skills/` 是两套**。前者可 `npx skills update` 更新，后者是本仓自有副本。
AGENTS.md 现在只写后者，所以**新克隆的人读 AGENTS.md 会以为本仓只有一个 skill**，
而实际跑起来时我的可用列表里有 24 个来自 mattpocock 的 skill 都在生效。
**这是文档与实况的偏差，比"少记一行"严重**：它让"哪些纪律是本仓的、哪些是上游的"分不清。

另：它的 canonical 命令是 `npx skills@latest add mattpocock/skills`（带 `@latest` 钉版本），
单 skill 形式 `--skill=<name>`，更新 `npx skills update <name>`。
我们 AGENTS.md 写的 `npx skills add Jakubantalik/Libraries.dev --skill '*' --copy -y` 是 `skills.sh`
那条路（可编辑副本），**与它的 plugin 路互斥**这件事我们没记。

## 元规范三：writing-docs（一套可借的文档纪律）

`.agents/writing-docs.md`（12688 字符）规定每个 promoted skill 都要有一份人读的文档页，
固定三段式 `## What it does` / `## When to reach for it` / `## Where it fits`，
加可选的 `## Prerequisites`、自由中段、`## Common questions`、`## It's working if`。

几条可直接借的判据：

1. **「defining constraint」必须用一句平实陈述写出**，且"never a labelled aside like
   'The defining constraint:' or 'The key thing:'"——那种带标签的插入语读起来像填充物。
   → 这与我们案例库 README 的写法同向：结论先行，不写"关键是"。
2. **`## Common questions` 只能收真实观察到的问题**，"A question filed twice is a question the
   page owes an answer to"，且**"the count stays honest to the evidence"**——
   薄技能配一两条，不许为了对齐丰技能而编。**这条我们已经在用了**
   （案例库每篇的「没能核实的」、CHANGELOG 的诚实边界），但它给出了更硬的说法。
3. **`## It's working if` 的门槛**：读者**不打开 SKILL.md 就能验证**。
   原话点名一种伪信号："the library section is byte-identical to `template.sh`" 是
   "a compliance check on the skill's internals wearing this section's name"。
   → 这条可搬到我们的验收标准：`risk-and-acceptance.md` 的 B 系列里若有"文件内容正确"这类，
   要问一句它是不是在测实现细节而不是测用户能看到的结果。
4. **文档页不带安装命令**（站点自己渲染安装控件），理由是"两份拷贝会漂移"。
   → 与我们「README 不是更新日志」、`install-block.md` 自己是唯一安装口径同源：**一件事只说一次**。

## 应用建议清单（与实现解耦，尚未排期）

1. **AGENTS.md 的 Installed skills 要分两类记**：本仓自有的（`libraries-dev`）
   与从 mattpocock/skills 全局装的 24 个（含它们受上游规范约束这件事）。
   并且写明两条安装路径互斥。
2. **给本机三个 grilling skill 补一张分工表**（谁调谁、什么时候用哪个）——
   上游用"链上哪一环"说清角色，我们没有。
3. **把「user-invoked 不能互调」写成显式纪律**：任何"跟我说一声就跑 X"的指令，
   若 X 是 user-invoked，必须写成对人的指令（"让用户运行 `/x`"），不能写成对模型的工具调用。
4. **文档引用纪律分清两半**：给人看的路径链接可用相对路径；**给 agent 的操作指令必须点名工具**。

## 没能核实的

- **三份元规范读全了，但 24 个 skill 的 `SKILL.md` 本体一个都没读**。
  所以"它们在我们项目里实际效果如何"未评估——本仓跑过的 grill-me / to-spec / implement 等
  是**用过但没做过效果评估**
- **`skills.sh` 的实现未读**（那只在 npm 包 `skills` 里，本仓用过它的 CLI）
- 269,700 star 是 GitHub 数字，**它对本仓的实际影响面没量化**（只知道 24 个在生效）
- 上游 README 说 ~60,000 订阅 newsletter，未核实
- `agents/openai.yaml` 这套 Codex 侧元数据我们本机没有对应物（本仓只跑 Claude Code / skills.sh 路径）
- 它与我们 `npx skills` 装的 `libraries-dev` 是否共享同一份 skills.sh 规范——**未核实**
  （从命令形态看是同一套，但没读 `skills` 包的源码）

## 取证命令

```sh
# 元数据（269700 star / Shell / 528 open issues）
curl -sSL https://api.github.com/repos/mattpocock/skills | python3 -c "
import sys,json; d=json.load(sys.stdin)
print({k:d.get(k) for k in ['stargazers_count','forks_count','language','open_issues_count']})"

# 三份元规范（本文引文出处）
for f in invocation writing-docs install-block; do
  curl -sSL "https://raw.githubusercontent.com/mattpocock/skills/main/.agents/$f.md" -o "meta-$f.md"
done

# README 全文（自述动机与 skill 清单）
curl -sSL https://raw.githubusercontent.com/mattpocock/skills/main/README.md

# 本机实际装的它的 skill（预期 24 个）
ls ~/.agents/skills/ | grep -E 'grill|to-spec|to-tickets|implement|code-review|domain-modeling|codebase-design|prototype|diagnosing|research|tdd|resolving-merge|wizard|wayfinder|triage|ask-matt|teach|handoff|wait-what|to-questionnaire|claude-handoff'

# 本机直连不通时走系统代理
scutil --proxy   # 127.0.0.1:10808
```
