# AgentIsland — Agent 工作约定

macOS 灵动岛应用：监控本机 AI 编码智能体的运行状态与 token 消耗。SwiftPM 构建，`swift build` / 自建测试 runner（无 XCTest）。

## 交付约定

### 自动重启应用（无感）

每次改动完成、构建出新的 `dist/AgentIsland.app` 后，**直接重启应用**，不必询问也不必先检查它是否在运行：

```sh
pkill -x AgentIsland; sleep 0.6; open dist/AgentIsland.app
```

- 旧实例存在就先杀掉（`pkill -x` 按进程名精确匹配，不会误伤 `AgentIslandTestsRunner`）
- 旧实例不存在时 `pkill` 返回非零，忽略即可，继续 `open`
- 重启后简短汇报，不要写成长篇操作说明

### 自动化提交与发版（每轮改造的收尾）

一轮改造 = 优化实现 → 测试全绿 → 脱敏扫描 → 提交 → tag → GitHub Release → 文档同步，**不必逐步征求同意**，用一条命令收尾：

```sh
scripts/release.sh <X.Y.Z> "<CHANGELOG 首条那句话>"
```

- 顺序是钉死的：**扫描先于 commit**（`release.sh` 内置），因为一旦提交，泄漏的内容就进了 git 对象，删掉文件也拿不回来
- 发版前三处版本必须一致：CHANGELOG 首条、`AppVersion.string`、README 的版本行；漂移即拒绝（脚本与 `build-app.sh` 都查）
- tag 与 release 是**必须发生的步骤**。历史上出现过 38 个版本只推了 commit、没建 tag 也没发 release（v0.0.81..v0.0.118），所以这条不再靠记忆
- 版本号取 `git tag --sort=-v:refname | head -1` 的下一号；CHANGELOG 条目按既有风格写「改了什么 + 为什么 + 没做什么」

### 文档分工（README 不是更新日志）

| 文件 | 只写 | 绝不写 |
| :--- | :--- | :--- |
| `README.md` | 这个工具是什么、有什么功能、怎么装怎么用、哪些限制今天还成立 | 「新增/不再/此前/上一版/实测数字」这类逐版叙事 |
| `CHANGELOG.md` | 每个版本改了什么、为什么、代价与被否决的方案 | 功能全景（会长成第二份 README） |
| `CONTEXT.md` / `docs/adr/` | 口径、术语、长期决策 | 版本历史 |
| `docs/research/` | 一手核实记录：正文原句或本机实样 + 取证命令 | 没出处的推断（写成推断要标出来） |
| `docs/research/ui/` | 外部动效案例库：手法 + 可迁移规则 + 一手证据 + 应用建议，四节写全 | 待办清单（那是 issue tracker）、已定案的长期口径（该进 CONTEXT 或 ADR） |
| `docs/code-review/` | 每轮改动的独立 review：`YYYY-MM-DD-HHMM-v<版本>-<审查者>.md`，随该轮一起入库 | 事后改写已提交的报告（它是时间点证据） |
| `site/` | 对外主页：功能导览 + 截图（GitHub Pages，Actions 从 `site/` 部署） | 逐版流水（那是 CHANGELOG）、没逐张核对过内容的屏幕截图 |
| `docs/workbench/` | 一次大改造的现状/目标/方案/计划四件套，随改造推进更新 | 逐版流水（那是 CHANGELOG）、已定案的长期口径（该进 CONTEXT 或 ADR） |

README 里出现「此前」「不再」「v0.0.x」「本机实测 N 条」即为跑题；写功能，不写流水。已知限制条目必须**逐条对着代码核实**再留着——过期的限制比没有限制更误导人。

`docs/workbench/` 是**进行中的大改造**的作战文档，四件套分工：`01` 现状（全部核实过）、`02` 目标与非目标、`03` 技术方案、`04` 分 Phase 计划。改造定案的结论要往 `CONTEXT.md` / `docs/adr/` 沉淀，不要让 workbench 长成第二份长期口径。

`docs/research/ui/` 是**外部动效案例库**（索引见其 README）：每篇固定四节——技术手法与源码级细节、可迁移的决策规则、一手证据/本地验证结果、应用建议清单。它记录"看见了什么、能搬什么"，**不排期**；要做哪条就开 issue，别把案例库读成待办板。

正在进行的改造（跨平台工作台：双形态 + CC Switch + ToDos）见 [docs/workbench/README.md](docs/workbench/README.md)。

### 数据脱敏红线（提交与发版前必查）

本项目对外发布，任何 commit 之前都要过门禁：

```sh
scripts/scan-secrets.sh            # 工作区（git 认得的全部文件）
scripts/scan-secrets.sh --release  # 工作区 + 全部 git 对象（release.sh 自动跑这条）
scripts/install-git-hooks.sh       # 新克隆/新工作区先装 pre-commit
```

- 规则是**棘轮**：命中的 (规则, 文件, 行) 要在 `scripts/secrets-baseline.txt` 里备过案，新增即失败。假凭据、假邮箱可以备案；`cred_prefix`/`private_key` 两类**绝对零命中**，不许进 baseline
- 豁免用行内 `nosec: <理由>`，理由必须写在被豁免的那一行上；全局禁用开关是 `SKIP_SCAN=1`，只用于本地试跑
- 本项目**不存储任何凭据**（源码零硬编码 secret）；如需要凭据一律环境变量或钥匙串注入，禁止写进任何文件
- 密钥值永不出现在输出里：扫描器自己就做打码，agent 也不 `cat`/`echo` 凭据文件
- `.scratch/`、`artifacts/`、`dist/`、`.build/`、根目录 `*.zip` 均不入库、不交付（屏幕截图可能含个人会话内容）
- 交付第三方收集数据的工具前，先删上述本地目录再跑 `scripts/scan-secrets.sh --release`；细节见 `docs/agent/desensitization.md`

## Agent skills

### Installed skills

本项目用 `npx skills`（skills.sh）管理 agent skills。

**两类来源要分清，因为纪律不同：**

**① 本仓自有的（进 git，克隆即得）**——单份源放 `.agents/skills/<name>/`，`.claude/skills/<name>` 是指向它的相对软链，`skills-lock.json` 记录来源与 hash。

- `libraries-dev`（来自 `Jakubantalik/Libraries.dev`）：教你 agent 在**哪里**、**用什么程度**给界面加动效。领域是 React/Web 界面，对本项目（SwiftUI/macOS）没有可套用的组件，价值在于它的**决策规则**（按等待时长定效果、不叠特效、匹配明暗主题）——改界面时可以参考这套思路，但不要尝试 `npm install`。
- 安装/更新/卸载：`npx skills add Jakubantalik/Libraries.dev --skill '*' --copy -y`。**不要**用它默认的 `--agent '*'`：那是 `--all` 的别名，会往项目里写 17 个 agent 目录（Antigravity、Cursor、Copilot、Goose、Junie…），每个都是同一份内容的副本。

**② 全局装的（只在本机，不进 git）**——来自 [mattpocock/skills](https://github.com/mattpocock/skills)（269,700 star，MIT），装在家目录 `~/.agents/skills/`，当前有 24 个生效。它们**受上游 `.agents/invocation.md` 等元规范约束**，不是本仓定的规矩。装法 `npx skills@latest add mattpocock/skills --skill=<name>`，更新 `npx skills@latest update <name>`。

上游规范里有两条对我们有约束力，值得记住：

- **user-invoked 的 skill 不能被另一个 skill 调用**（连指名工具也不行）。所以"跟我说一声就跑 X"这种指令，若 X 是 user-invoked，必须写成**对人的指令**（"请运行 `/x`"），不能写成对模型的工具调用。我们本机的 `grill-me` / `grilling` / `grill-with-docs` / `triage` / `to-spec` / `implement` 等都属于 user-invoked。
- **skill 之间的依赖要写成"调用 Skill 工具并指名"**，不是 `../other-skill/FILE.md` 跨目录相对链接。（文档之间给人看的路径链接仍可用相对路径——那条不受此限。）

⚠️ **两条安装路径互斥**：Claude Code 的 plugin 路（`claude plugins install mattpocock-skills`，官方 marketplace，只读托管、自动更新）与 skills.sh 路（可编辑副本、自己 update）**都装会得两份**。本机走的是 skills.sh 路。

三个 grilling skill 的分工与"哪条链用哪个"**目前没有一张表**，已知是缺口；上游用"链上哪一环"说清角色，可照做。

### Issue tracker

单人开发：issues 走本地 markdown（`.scratch/<feature>/`，spec.md + issues/NN-*.md）。See `docs/agents/issue-tracker.md`.

### Triage labels

五个默认 triage 角色，标签名与角色同名。See `docs/agents/triage-labels.md`.

### Domain docs

单上下文：根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
