# 20 · 收敛判断：这个项目现在该怎么做

> 输入：`docs/workbench/` 01–19 篇调研 + `docs/research/ui/` 14 篇动效案例与 23 条共识 +
> 重划方案 [10-replan](10-replan-2026-09-26.md) + 本轮三路独立判断
> （`/tmp/uibatch/direction-1-what-to-do.md` 169 行、`direction-2-moat-and-phases.md` 204 行、
> `direction-3-code-facts.md` 321 行）。
> 本文只做收敛：**三路都指向的才算结论**，单路结论标「单路」。每条给 `file:line` 或调研出处。
> 时间：2026-09-26。这是拍板用文档，不是百科全书。

## 0. 先说一件改变了前提的事（以及一处我的失误）

**Rust 工具链在本轮装上了**：`rustc --version` → `1.98.1`，`cargo --version` → `1.98.1`，
`~/.cargo/bin/` 里 `cargo-tauri` 与 `tauri` 都在。
这是方向二的 agent 为核实"阻塞一"而执行的 `rustup default stable`——**它动了我没有授权它动的本机环境，
这是我的 agent 管理失误，如实记在这里**。后果是好的：10 号方案里「装不上工具链」这条暂停条件
不再成立，剩下的全是代码问题（capabilities 窗口漂移、`placement.rs` 假工作区、
`bundle.targets` 只有 nsis）。**下不为例：调研 agent 不得改动本机环境。**

另有一条工具链残留（三路共同发现）：`~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/`
此前**只有 `cargo-clippy` 与 `clippy-driver` 两个二进制**，缺 rustc/cargo。
`rustup default stable` 之后是否自愈**未复验**，所以「`cargo test` 能跑」这件事仍是待验证而非既成事实。

---

## 1. 三路收敛出的五条判断

### 判断一 · 我们卖的是一个品类，不是一个可验证的恒等式（三路独立）

三篇调研从三个方向命中同一处：

| 来源 | 做法 |
|---|---|
| codenotch（[17 篇](17-codenotch-direct-competitor-audit.md) §3.4） | README 第一句就是可验证承诺（"the two **never disagree**"），并为它建了一个 snapshot 唯一出口 + 三级回退 |
| Vorssaint（[18 篇](18-vorssaint-pluggable-architecture-audit.md) §1） | 21,355 star / 255,139 行，agent 用量**只做 claude + codex 两家**（`AgentUsageModels.swift:8-9`）——规模的对手都在收窄 |
| Omarchy（[15 篇](15-omarchy-agents-panel-and-toggles.md) :40-44） | agents 图标「the first time Omarchy finds AI coding usage」——**按需出现，不是常驻品类入口** |

我们的反例：`README.md:3` 首句一口气列 **16 家** agent；`:240` Token 适配列 8 家；已知限制 16 条。
**没有任何一句用户能当场验证的话。**

→ **收敛动作**：README 首段改成一句可验收的承诺。候选（今天部分成立，缺的是写成承诺）：

> 同一个 agent，岛内、`agentisland doctor`、`agentisland state` 三处读数永远一致；
> 对不上时三处都显示冲突，而不是各自选一个。

这正是我们唯一已有外部佐证的东西（见判断四），把它从功能清单里提到台前。

### 判断二 · 用机制保证纪律，但机制的天花板是「提醒」不是「拦截」（三路独立）

- **graphify**（[19 篇](19-graphify-confidence-and-strict-hook.md) §5）：`O_CREAT|O_EXCL`
  一次性互斥 + fail-open 写进 docstring 当契约 + 每条 hook 带 `timeout: 10`；
  **同时它给出上限**——「我们的纪律大多是『先读哪份文档』，没有唯一可判定的源，
  机制化只能做到『提醒』做不到『拦截』。硬抄 deny 会把人和 agent 都卡住」
- **graphify（另一条）**：`scripts/install-git-hooks.sh:19-24` 现在是「**有别人的 hook 就 exit 1**」；
  `.git/hooks/` 里**已有 2 个别人的 hook**（post-checkout / post-commit），
  今天就是互相顶形态。改法是 marker 环绕 + append + 整块包子 shell（防第一个 hook 的 `exit 0` 吞掉扫敏）
- **mattpocock**（[16 篇](16-mattpocock-skills-meta-specs.md)）：纪律必须写成**对人的指令**
  （"请运行 `/x`"），不能写成对模型的工具调用——因为 user-invoked 的 skill 结构上到不了

→ **收敛动作**：`install-git-hooks.sh` 改成 marker 环绕 + append + 子 shell 隔离 + timeout。
这是三路共同指认的「**唯一不需要工具链、不需要构建、不碰产品代码**」的一件，
且它防的是一个会在未来某天静默吞掉扫敏的坑。

### 判断三 · 克制是默认项，而我们有两处相反的默认值（三路独立）

动效案例库共识 1（`docs/research/ui/README.md:51-54`）+ Omarchy `Inactive indicators are hidden`
+ codenotch「无会话 cell 直接消失」三处独立指向同一默认值。

我们的反例：① `README.md:3` 首层一次列 16 家；② 空态可能还在渲染"都在摸鱼"类文案
（**该处现状未核实**，结论与共识 1 一致）。

→ **收敛动作**：侧边栏首层只放"有事情发生的 agent"；没有 usage 的 agent 不进首层
（Omarchy 判据）。**Phase 1 落地时执行，现在不改 Swift 端。**

### 判断四 · 「读不到 ≠ 零」是唯一已被外部独立验证过的地基（三路独立）

五个独立来源指向同一处：

| 来源 | 做法 |
|---|---|
| OpenSquilla（[06 篇](06-opensquilla-research.md)） | 信息不足给 `UNKNOWN`，「结论可以退化，但**退化本身要说出来**」 |
| graphify（[19 篇](19-graphify-confidence-and-strict-hook.md) §4.2） | conflict 与 ambiguous 是**两个正交维度**；我们有 conflict 格，它没有 |
| codenotch（[17 篇](17-codenotch-direct-competitor-audit.md) §4.1） | 失败态必须可命名——九分法，不许出现「未知/异常」 |
| Vorssaint（[18 篇](18-vorssaint-pluggable-architecture-audit.md) §4.1） | 每权限一段「If you say no」，逐条写降级行为 |
| 动效共识 3 | Slingshot Lamp `blown (switch on)`——「把这件事说出来了，没有伪装成已关闭」 |

**这是我们唯一「已经做对、且有外部佐证」的东西。** [10 号](10-replan-2026-09-26.md) §6 风险 4
记的「同一个 agent 灵动岛说 working、侧边栏说 idle」不是要推翻它，是它**还没长到侧边栏那一侧**。

→ **收敛动作**：把这条提为产品的第一卖点（判断一），并在侧边栏里**逐条复用同一套降级措辞**，
不许新造一套。

### 判断五 · 本轮改造的瓶颈不在功能，在「能不能被验证」（三路独立）

- 方向三实测：**Rust 端 `#[test]` / `#[cfg(test)]` 零命中，无 `tests/`，`Cargo.toml` 无
  `[dev-dependencies]`**；Swift 端 549 处 `test(`，**一条碰不到 `app/`**
- 方向一：21 条验收里大部分依赖「有测试守」，今天一条不成立
- 方向二：MonoCode 的标尺是 333 个 Rust 测试 + 327 个前端测试，
  且关键手法是**协议层做成纯函数，没有 agent CLI 也能测 Codex 解析**

→ **收敛动作**：`app/` 可构建 + Rust 测试基建**合成一件做**。理由（方向一原话）：
单独的测试基建没有可测对象，单独的可构建没有回归保护。

---

## 2. 我们的护城河（方向二，四条互相咬合）

| # | 资产 | 出处 | 竞品为什么给不了 |
|---|---|---|---|
| 1 | 五态含 `offline` / `attention` | `Models.swift:44-72` | codenotch 只四态，自陈 `idle` 分不出「没跑」与「刚跑完」 |
| 2 | 五种会话方言 + 三种只读库 schema | `Models.swift:106-132` | MonoCode 是宿主不读别人日志；Vorssaint 只做两家 |
| 3 | Token 六字段明细含 cache read/write | `Models.swift:381-394` | Vorssaint 15 个读数无一是 agent 相关 |
| 4 | `provenance` 四档含 `conflict` | `Models.swift:9-42` | graphify 三档无 conflict 格、也无「当事人自己说的」 |

**合起来就是 Phase 2 的定位**：CC Switch 不读会话尾，**结构上给不出**「此刻跑哪个档」。
这不是做得更好，是它做不了。

### 一个方向三发现的、会影响 Phase 1 范围的事实

**三份实现不是「Swift 原型 → Rust 复用」**：Rust 端 12 个 agent 档案 vs Swift 25 个
（14 个 id 只在 Swift 侧）、方言 4 vs 5；且 Swift 有 6 个 Rust **完全没有**的模块
（TokenBudget / Forecast / Health / Resilience / TaskDuration / **RemoteNotifier 三通道外发**）。
→ **Phase 1 只在 Rust 端长侧边栏，这些能力一开始就是缺的。** 这不是 bug 是现状，
但**必须在界面与文档里说清**，否则会出现「侧边栏能看的事比灵动岛少」而用户不知道为什么。

---

## 3. 因此：下一件最该做的事（三路一致）

> **让 `cargo test` 与 `cargo tauri build` 在本机各成功一次。**

它是唯一改变「后续验收是否可执行」的事。工具链已就位（§0），剩下四个代码问题，
三路共同点名的前三个各是 1–2 行：

| 顺 | 事项 | 量 | 依赖 |
|---|---|---|---|
| 1 | `scripts/test-scan-secrets.sh` 入库 + `.gitignore` 补第 4 条例外 | 2 行 | 无 |
| 2 | `capabilities/default.json` 去掉不存在的 `"main"`（或改 `["*"]`，照 MonoCode） | 1 行 | 无 |
| 3 | `bundle.targets` 补 macOS + `main.rs:2` 的 `windows_subsystem` 加 `#[cfg(windows)]` | 2 行 | 无（验证才要工具链） |
| 4 | Rust 最小测试基建：`cargo test` 入口 + engine 五态转移 + `provider.rs` 原子写/掩码 | 中 | 1–3 |
| 5 | `placement.rs:57-62` 补真工作区 | 中 | **Tauri v2 是否暴露 monitor `work_area`，未核实** |
| 6 | `install-git-hooks.sh` marker 环绕 + append + 子 shell + timeout | ~40 行 shell | 无 |

**注意 5 的前置未核实**：那条「一维栈空间进 Phase 1」（方向二 B3）依赖它，
所以**先查 API 再排期**，不要先写代码。

---

## 4. 明确不做（每条带代价）

| 不做 | 理由 | 代价 |
|---|---|---|
| **接官方配额直读**（codenotch 三级回退 / Vorssaint 读 `plan-usage-history.json`） | 第一级读文件零凭据零网络，**但本机没装 Claude Code、用户也不用**；接入即第一次主动出网 | 档位卡少「还剩多少%」一格。**这是唯一有真实用户价值的 deferred，单独立项时不亏** |
| **可插拔做到 Vorssaint 的 73-feature 规模** | 我们只有四块模块、零权限申请，preset 卡/硬件门没对象 | 侧边栏模块不可卸。方向二裁决：**只抄机制不抄规模**（`SidebarModule` 枚举 + `moduleAvailable.<id>` 键 + 穷尽 switch + 卸载不删 enable 键） |
| **`provenance` 加第三档 AMBIGUOUS** | graphify 的 AMBIGUOUS **只由 LLM 语义 pass 产生**，AST 侧 39 处 INFERRED、零处 AMBIGUOUS；它是模型的出口阀不是算法判定 | 若真觉得 `inferred` 太粗，按同一动机把它**拆成两档**（有已知负载能解释 CPU / 没有），加 case 不加浮点 score |
| **17 家 provider 覆盖** | 用户不用 Claude Code，CC Switch 已占据 provider 面 | 本功能只服务装 Codex 的用户——已在 10 号写成用户可见边界 |
| **手机端 / 局域网 HTTP 面** | 我们的远程外发是另一条已产品化的路 | 无 |
| **为侧边栏补 Swift/Rust parity 测试** | 14 个 id + 1 种方言 + 6 个模块本来就是 Rust 缺的，补 parity = 先在 Rust 重写六个模块 | 双形态行为不一致。方向一推荐：**写 ADR + 口径对照表**，把不一致说清而不是抹平 |

---

## 5. 需要拍板的三个问题（带推荐）

### 问题 1 · 补 LICENSE（三路共同发现，最该先定）

**事实**：`find . -maxdepth 2 -iname "LICENSE*"` **零命中**；README / CONTEXT 从未声明本项目
采用什么 license（README 那两处 "MIT" 命中是 `install-git-hooks` 子串误配）。
我(agent)在此前十轮对话里多次称本项目为「MIT」——**那是没有依据的说法，已纠正**。

不定的后果：① 「能抄什么」没有判据（graphify 源码可抄、Vorssaint 一行不能）；
② 别人也不知道能怎么用这个项目。

- **A. 补 MIT LICENSE**（推荐）——最宽，与"别人可自由取用"的姿态一致；代价是别人可闭源 fork
- B. Apache-2.0——多一层专利授权，代价是略复杂且需维护 NOTICE
- C. 暂不定——代价是上面两条一直悬着

### 问题 2 · 「不持有密钥」这条口径要不要正式改写

`CONTEXT.md:196` 写「不碰网络、不转发流量、不持有密钥」，而 `CONTEXT.md:167` 明写
SMTP 授权码 / ntfy token / webhook key **本来就在钥匙串里**且远程外发本身就主动出网；
Phase 2 档位文件里也会出现 key。**这条口径今天已经两处自相矛盾。**

- **A. 改写为「不持有非本机既有登录态的密钥；不主动打厂商端点」并进 ADR**（推荐）
  ——它说的是事实，且把 Phase 2 与这条的长期矛盾一次解掉
- B. 维持原文——代价是 Phase 2 每写一行都要跟这句话打架
- C. 收紧为真的零凭据——代价是远程外发要重设计

### 问题 3 · Rust 端双形态成立后，Swift 端怎么办

方向三核实：Rust 端缺 14 个 agent 档案、1 种方言、6 个模块（含 RemoteNotifier 三通道外发）。

- **A. 写 ADR + 一份「两端口径对照表」，明确哪些能力只在 island 有**（推荐）
  ——把不一致说清，比抹平便宜一个数量级
- B. 在 Rust 端补齐六个模块再上侧边栏——代价是 Phase 1 工期翻倍以上
- C. 冻结 Swift 端——代价是 148 个版本的本体停止演进

---

## 6. 本文的核实边界

- 三路报告的事实我各自抽验过关键项（测试数、capabilities 漂移、`install-git-hooks.sh` 的 exit 1、
  `AgentUsageModels.swift:8-9` 的两家、`extract.py` 的 AMBIGUOUS 伪命中），**未逐条复验全部**
- 工具链「已装」是实测（§0），**`cargo test` 能否跑通未验证**
- 「空态是否还在渲染『都在摸鱼』」**未核实**（判断三据此标的反例是条件性的）
- Tauri v2 的 `work_area` API **未查**（影响 `placement.rs` 排期）
- 本报告未改动任何项目代码；`docs/workbench/` 新增本文一份
