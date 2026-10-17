# AgentIsland — 工作约定（Agent 无关）

> **这份约定对所有 AI 编码 Agent 一视同仁。** 它不假设你(agent)是哪一个产品、跑在哪个 harness 上、
> 用哪种模型。任何 Agent 克隆本仓后，读这一份就能接手。
> `CLAUDE.md` 与 `GEMINI.md` 都是指向本文件的软链——**只有这一份源**，不要各写一份。
>
> **接手入口（新 Agent 从这里开始）**：
> 1. 读本文件（工作约定）与 [`CONTEXT.md`](CONTEXT.md)（领域口径与术语）。
> 2. 读 [`docs/需求.md`](docs/需求.md)（需求基线）与 [`docs/需求测试矩阵.md`](docs/需求测试矩阵.md)（验收场景与层）。
> 3. 查 [`docs/需求状态.md`](docs/需求状态.md)：**哪些已完成、哪些在做、有哪些未修 bug**。
>    若是从上一个 Agent 手里接过来的，先读 [`docs/agent/HANDOFF.md`](docs/agent/HANDOFF.md)：
>    那里有当前状态、环境前置、本轮踩过的坑与硬边界。
> 4. 动手前跑 `bash scripts/verify-framework.sh` 确认基线是绿的；不绿先查状态文档里的已知问题。
> 5. 改动收尾按「[自动化提交与发版](#自动化提交与发版每轮改造的收尾)」一条命令走完。

跨平台 Rust/Tauri 工作台：监控本机 AI 编码智能体的运行状态与 token 消耗。main 是唯一开发主线；SwiftUI 与旧 WPF 源码保存在 `codex/archive-swiftui`，需要查历史实现时读取该分支。测试用 `cargo test --locked --manifest-path app/src-tauri/Cargo.toml`，打包用 `scripts/build-app.sh`。

**本文件里不写「某个 Agent 专属」的做法**：凡是「用 X 工具」「在 Y 面板里点」这类指令，
都必须能改写成命令行或文件路径。发现违反的地方，改成 Agent 无关的表述——
后续接手的人可能是另一个 Agent，甚至是人。

发布门禁里的浏览器 UI 回归（`test-session-refresh-focus` / `test-task-detail-navigation` / `test-task-reader-leave`）需要真实浏览器运行时：`npm i playwright`，或用 `TEST_BROWSER_MODULE` 指向 `playwright/index.mjs` 的绝对路径。缺运行时 `release.sh` 会**明确失败**而不是跳过——这三条此前不在任何门禁里，v0.0.321 起 committed 测试对 committed 产品变红也没人发现（窄窗口隐藏列表让旧断言超时）。

## 交付约定

### 自动重启应用（无感）

每次改动完成、构建出新的 `dist/AgentIsland.app` 后，**直接重启应用**，不必询问：

```sh
scripts/restart-app.sh
```

- 脚本会杀掉旧实例、打开新构建，并**校验两步都真的发生了**（任一步不符就非零退出）
- ⚠️ **不要自己写 `pkill -x AgentIsland`**——那是条**从来没生效过**的命令。
  `pkill -x` 匹配的是**进程名**，而 bundle 里的二进制叫 **`agentisland`（小写）**，
  不是 `AgentIsland`。实测 `pkill -x AgentIsland` 退出码 = 1（一个都没打中），
  `pgrep -x agentisland` 才命中。所以应用过去每次都**没有真的重启**，
  看起来更新只是因为 `open` 撞上被改过的 bundle、Launch Services 自己重开的
- 也**不要用 `pkill -x agentisland`**：用户自己跑的 CLI（`dist/agentisland`）同名，会被一起杀掉。
  按应用内的完整路径匹配才只打应用
- 重启后简短汇报，不要写成长篇操作说明

### 自动化提交与发版（每轮改造的收尾）

一轮改造 = 优化实现 → 测试全绿 → 脱敏扫描 → 提交 → tag → GitHub Release → 文档同步，**不必逐步征求同意**，用一条命令收尾：

```sh
scripts/release.sh <X.Y.Z> "<CHANGELOG 首条那句话>"
```

- 顺序是钉死的：**扫描先于 commit**（`release.sh` 内置），因为一旦提交，泄漏的内容就进了 git 对象，删掉文件也拿不回来
- 发版前版本必须一致：Rust Cargo / lock / Tauri 配置、CHANGELOG 首条、README 的版本行；漂移即拒绝（脚本与 `build-app.sh` 都查）
- tag 与 release 是**必须发生的步骤**。历史上出现过 38 个版本只推了 commit、没建 tag 也没发 release（v0.0.81..v0.0.118），所以这条不再靠记忆
- 版本号取 `git tag --sort=-v:refname | head -1` 的下一号；CHANGELOG 条目按既有风格写「改了什么 + 为什么 + 没做什么」

### 需求状态与 bug 登记（**改了需求就同步**）

`docs/需求状态.md` 是**接手者第一眼要看的文件**：哪些需求已完成、哪些在做、有哪些未修 bug。
它由 `scripts/automation/registry.json` **生成**，不手写：

```sh
python3 scripts/automation/render-status.py          # 校验（门禁跑这条）
python3 scripts/automation/render-status.py --write  # 改完 registry 后重新生成
```

- **功能完成后**：把 `registry.json` 里该需求的 `status` 改成 `done`，
  `coverage_note` 改成以 `done — ` 开头（`partial — ` 表示还有缺口）。
  生成器会把它渲染成「已完成」，并计入状态文档的合计行。
- **发现 bug 时**：写进 `registry.json` 的 `open_bugs` 数组
  （`id` / `title` / `status` / `evidence` 必填，`requirement` 可选），
  生成器渲染成「未修 bug 登记」表。**不要只写在聊天里或某个 review 里**——
  那些不是接手者会读的地方。
- 状态机（生成器实现并在门禁里校验，组合不可能成立即失败）：
  `checks` 空 = 未开始；有 `checks` 且 note 以 `partial — ` 开头 = 部分完成；
  以 `done — ` 开头 = 已完成。
- ⚠️ **改了 `requirements[].checks` 必须同时跑两个生成器**：
  `render-matrix.py --write`（需求→检查项映射）与 `render-status.py --write`（状态与 bug）。
  漏跑任一个，`automation.matrix` / `automation.status` 会在门禁里报漂移。

### 文档分工（README 不是更新日志）

文档地图见 [`docs/README.md`](docs/README.md)——**先读它**再找具体文档。
`docs/` 按用途分三类子目录：`spec/`（要做什么）、`testing/`（怎么验）、`ops/`（怎么发）。

> 根目录 `docs/*.md` 里那些同名软链是**兼容层**（门禁脚本硬编码了旧路径），
> 新增文档直接放进子目录，不要往根下放。

| 文件 | 只写 | 绝不写 |
| :--- | :--- | :--- |
| `README.md` | 这个工具是什么、有什么功能、怎么装怎么用、哪些限制今天还成立 | 「新增/不再/此前/上一版/实测数字」这类逐版叙事 |
| `CHANGELOG.md` | 每个版本改了什么、为什么、代价与被否决的方案 | 功能全景（会长成第二份 README） |
| `CONTEXT.md` / `docs/adr/` | 口径、术语、长期决策 | 版本历史 |
| [ui-research](../../03-行业调研/ui-research/) | 外部一手调研库（独立仓库）：动效案例四节写全 + 各家 agent 调研 + 故障排查 | 待办清单（那是 issue tracker）、已定案的长期口径（该进 CONTEXT 或 ADR） |
| `docs/code-review/` | 每轮改动的独立 review：`YYYY-MM-DD-HHMM-v<版本>-<审查者>.md`，随该轮一起入库 | 事后改写已提交的报告（它是时间点证据） |
| `site/` | 对外主页：功能导览 + 截图（GitHub Pages，Actions 从 `site/` 部署） | 逐版流水（那是 CHANGELOG）、没逐张核对过内容的屏幕截图 |
| `docs/workbench/` | 一次大改造的现状/目标/方案/计划四件套，随改造推进更新 | 逐版流水（那是 CHANGELOG）、已定案的长期口径（该进 CONTEXT 或 ADR） |

README 里出现「此前」「不再」「v0.0.x」「本机实测 N 条」即为跑题；写功能，不写流水。已知限制条目必须**逐条对着代码核实**再留着——过期的限制比没有限制更误导人。

`docs/workbench/` 是**进行中的大改造**的作战文档，四件套分工：`01` 现状（全部核实过）、`02` 目标与非目标、`03` 技术方案、`04` 分 Phase 计划。改造定案的结论要往 `CONTEXT.md` / `docs/adr/` 沉淀，不要让 workbench 长成第二份长期口径。

[ui-research](../../03-行业调研/ui-research/) 是**外部一手调研库**（独立仓库；索引见 [ui/README.md](../../03-行业调研/ui-research/ui/README.md)）：每篇固定四节——技术手法与源码级细节、可迁移的决策规则、一手证据/本地验证结果、应用建议清单。它记录"看见了什么、能搬什么"，**不排期**；要做哪条就开 issue，别把案例库读成待办板。待核实清单由该仓自己的 [check-checklist.sh](../../03-行业调研/ui-research/scripts/check-checklist.sh) + pre-commit 守着。工作区重构前的 `../ui-research/` 路径已失效；白屏、几何与会话等迁出取证文件须核实新位置后更新当前文档链接，历史审查报告保留原样。

新增功能、排开发任务或补测试前先读当前 [需求基线](docs/需求.md)和 [需求测试矩阵](docs/需求测试矩阵.md)，旧“不做网关/任务执行”范围已由 [ADR 0040](docs/adr/0040-supervisor-full-integration-requirements.md)取代；新能力需按子系统契约实施，不能据需求目标绕过现行安全控制。方案与历史入口见 [docs/workbench/README.md](docs/workbench/README.md)。读旧方案、迁移表或冲突决策前先看 [文档梳理与冲突登记](docs/agent/project-document-audit-2026-10-14.md)；定位归档 Swift/WPF 或迁出取证文件时查 [历史路径映射](docs/agent/history-file-map.md)。

### 数据脱敏红线（提交与发版前必查）

本项目对外发布，任何 commit 之前都要过门禁：

```sh
scripts/scan-secrets.sh            # 工作区（git 认得的全部文件）
scripts/scan-secrets.sh --release  # 工作区 + 全部 git 对象（release.sh 自动跑这条）
scripts/install-git-hooks.sh       # 新克隆/新工作区先装 pre-commit
```

- **CI 的 `$USER` 是通用账号名**（`runner`/`runneradmin`）：身份类规则（`username`、`local_user_path`）在通用名下会**显式停用**并说明原因；要保留 CI 的判别力，在仓库变量 `AGENTISLAND_SCAN_USER` 填开发者本机用户名（扫描器读 `SCAN_USER`）。本机 pre-commit 不受影响
- 规则是**棘轮**：命中的 (规则, 文件, 行) 要在 `scripts/secrets-baseline.txt` 里备过案，新增即失败。假凭据、假邮箱可以备案；`cred_prefix`/`private_key` 两类**绝对零命中**，不许进 baseline
- 豁免用行内 `nosec: <理由>`，理由必须写在被豁免的那一行上；全局禁用开关是 `SKIP_SCAN=1`，只用于本地试跑
- 本项目**不存储任何凭据**（源码零硬编码 secret）；如需要凭据一律环境变量或钥匙串注入，禁止写进任何文件
- 密钥值永不出现在输出里：扫描器自己就做打码，agent 也不 `cat`/`echo` 凭据文件
- `.scratch/`、`artifacts/`、`dist/`、`.build/`、根目录 `*.zip` 均不入库、不交付（屏幕截图可能含个人会话内容）
- 交付第三方收集数据的工具前，先删上述本地目录再跑 `scripts/scan-secrets.sh --release`；细节见 `docs/agent/desensitization.md`

## 工程纪律（从历史事故提炼）

以下是跨轮次必须执行的规则。排查白屏/资源异常、修改解析与缓存、添加门禁或重做原生验收前，按主题读 [改造踩坑史](docs/agent/pitfalls-history.md)；其中 Swift 案例是历史证据，当前领域口径以 `CONTEXT.md` 和相应 ADR 为准。

### 诊断与验证

- **隔离当次运行**：日志、截图、回执绑定运行身份与被测构建；历史日志、截断输出和不同轮次产物不能拼成当前结论。
- **校准测量方法**：窗口几何交叉核对原生读数、viewport 与截图；资源测量固定目标身份、负载及稳态窗口。排除启动尖峰和生命周期均值造成的误判。
- **探针只读**：状态驱动走产品入口；观测链受故障影响时改用独立证据。异步操作等具体完成标志，等待有界。
- **核对当前产物**：前端/编译期配置改动走 `scripts/build-app.sh` 并验证实际加载内容，避免普通 `cargo build` 复用旧嵌入资源。
- **门禁证明能拒错**：新增或修复门禁要有负例或可编译变异，指出失败断言；结构扫描同时检查扫描范围。变异在隔离副本执行，测试默认用合成/脱敏夹具。
- **检查执行链**：显式核对构建、测试与扫描退出码；管道使用 `pipefail` 或保留原命令状态。测试进入对应门禁，必需依赖缺失即失败；外部环境测试单列范围与未验项。

### 数据、并发与写入

- **未知单独表达**：源缺失、读失败、未观测与实际零值分开；相关派生字段同进退。CPU/卡死等连续观测资格由引擎维护，单次读数不能冒充持续结论。
- **事实只声明一次**：Agent 能力放注册表，解析按方言/schema 分派；修改时检查真实生产路由及全部输出，不在测试里复制一套分派冒充覆盖。
- **守住事件身份**：区分会话、运行、事件版本和通知去重；使用已核实的消息 ID、轮次或事件自身记录的时间戳，成对验证「同事件不重报、新事件不漏报」，不以采样时钟代替事件身份。
- **锁内只计算**：持锁时不得调用同步派发到主线程的 Tauri API。窗口/托盘写入及事件发送在释放锁后执行，见 [ADR 0014](docs/adr/0014-no-main-thread-dispatch-while-locked.md)。
- **异步结果绑定身份与代次**：返回前复核目标和 revision；失效结果丢弃，普通读取失败与目标失效分别处理。后台刷新保留草稿、焦点及滚动，正文离页/过期及时清除。
- **缓存回收保住结果**：优化前后对拍相同输入；当前文件汇总完成后才回收。交代漏一次状态转移的代价，缓存计费、RSS、footprint 与整应用预算分别报告。
- **持久化保原件**：写入前校验并原子发布；损坏/未来格式保留原字节、明确拒写或按模块契约留档。备份以独占创建防同名覆盖，不能靠 `exists` + `rename` 声称无竞态，见 [ADR 0033](docs/adr/0033-configuration-backup-publication.md)。

### 边界与交付证据

- **按已核实契约接入**：字段、schema、URL 与能力不能猜；配置存在/保存成功不证明客户端加载，配置目标、运行模型与账单分别取证。现有通知和手动模型读取按 [ADR 0009](docs/adr/0009-credential-boundary-borrow-dont-hold.md) / [0038](docs/adr/0038-manual-interface-model-read.md)保持原安全控制；新网关/订阅/执行目标按 [0040](docs/adr/0040-supervisor-full-integration-requirements.md)另定子系统授权与验证，不能直接复用通知凭据或放宽旧路径。
- **门禁故障明确失败**：空扫描与零命中分开，拒绝必须传播到顶层；钩子在临时仓库验证真能阻止提交。共享工作树的实验与发布使用隔离环境及显式文件清单，保护无关改动。
- **证据按层报告**：静态审查、单元/浏览器、官方客户端受控端点、签名包原生和平台整体验收分别记录；明示执行者、结果及未验范围，清理与业务结果分别核实。
- **保留失败和更正**：重跑通过只说明本次通过；保留原失败及不完整样本，不放宽判据凑通过。误诊追加撤回说明，历史 review 不改写；当前入口和长期文档同步修正。
- **用户真实状态只读**：不要把探测/合成事件投进用户正在运行的应用。验证走隔离实例
  （`--background-test`/`--ui-smoke` 用独立 bundle id，不启用账本持久化）或临时 fixture 探针。
  确需写入真实状态时：停应用 → 备份整份文件 → 按**能逐条指向「这条是我造的」的精确 id 集合**删除
  → 重启 → 逐条 diff 复验。**禁止用时间窗口当判据**（会随后续投递不断变宽，误删真实数据）；
  并注意有界容量的连带淘汰（`pending` 上限 64，注入 N 条会静默挤掉最多 N 条最早的记录）。
  目标状态是「与写入前逐条一致」，不是「看起来干净了」。

## Agent skills（**可选**，非接手前置）

> **这一节不构成接手前置。** skills 是本机可选的便利设施，与产品代码、测试门禁、
> 发版流程都无关——换一台机器、换一个 Agent，不做这一节也能完整接手本仓。
> 之所以留着，是因为踩过的坑值得记（尤其「两条安装路径互斥」）。

本项目用 `npx skills`（skills.sh）管理 agent skills。

**两类来源要分清，因为纪律不同：**

**① 本仓自有的（进 git，克隆即得）**——单份源放 `.agents/skills/<name>/`；
`.claude/skills/<name>` 只是指向它的相对软链（为了兼容读 `.claude/` 的 Agent，
**不是**「Claude 专用目录」），`skills-lock.json` 记录来源与 hash。

- `libraries-dev`（来自 `Jakubantalik/Libraries.dev`）：教你 agent 在**哪里**、**用什么程度**给界面加动效。领域是 React/Web 界面，本项目使用静态 Web UI，不能直接套用 React 组件，价值在于它的**决策规则**（按等待时长定效果、不叠特效、匹配明暗主题）——改界面时可以参考这套思路，但不要尝试 `npm install`。
- 安装/更新/卸载：`npx skills add Jakubantalik/Libraries.dev --skill '*' --copy -y`。**不要**用它默认的 `--agent '*'`：那是 `--all` 的别名，会往项目里写 17 个 agent 目录（Antigravity、Cursor、Copilot、Goose、Junie…），每个都是同一份内容的副本。

**② 全局装的（只在本机，不进 git）**——来自 [mattpocock/skills](https://github.com/mattpocock/skills)（MIT），装在家目录 `~/.agents/skills/`。它们**受上游 `.agents/invocation.md` 等元规范约束**，不是本仓定的规矩。装法 `npx skills@latest add mattpocock/skills --skill=<name>`，更新 `npx skills@latest update <name>`。

上游规范里有两条对我们有约束力，值得记住（**它们是「人机交互约定」，不是某个 Agent 的特性**）：

- **user-invoked 的 skill 不能被另一个 skill 调用**（连指名工具也不行）。所以"跟我说一声就跑 X"这种指令，若 X 是 user-invoked，必须写成**对人的指令**（"请运行 `/x`"），不能写成对模型的工具调用。本机的 `grill-me` / `grilling` / `grill-with-docs` / `triage` / `to-spec` / `implement` 等都属于 user-invoked。
- **skill 之间的依赖要写成"调用 Skill 工具并指名"**，不是 `../other-skill/FILE.md` 跨目录相对链接。（文档之间给人看的路径链接仍可用相对路径——那条不受此限。）

⚠️ **两条安装路径互斥**：plugin 路（只读托管、自动更新）与 skills.sh 路（可编辑副本、自己 update）**都装会得两份**。本机走的是 skills.sh 路。

三个 grilling skill 的分工与"哪条链用哪个"**目前没有一张表**，已知是缺口；上游用"链上哪一环"说清角色，可照做。

### Issue tracker

单人开发：issues 走本地 markdown（`.scratch/<feature>/`，spec.md + issues/NN-*.md）。See `docs/agents/issue-tracker.md`.

### Triage labels

五个默认 triage 角色，标签名与角色同名。See `docs/agents/triage-labels.md`.

### Domain docs

单上下文：根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
