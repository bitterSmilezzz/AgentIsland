# NOT-05 复审整改报告（第二轮）

日期：2026-10-17
范围：`NOT-05 策略、升级与交付状态` — 独立复审（周衡）**不能定案**的整改闭环
基线产物：`build_id=343dc5090bae` / `src=969eda8b9bfd`（整改前）
本轮产物：见文末「产物」小节

复审结论「不能定案」的 6 项发现（A1–A6 + B1/B2/D），本轮逐条闭环。**每条都给出可复现的验证命令**，
不是「已修复」这种自述。

---

## A1 + D：交付状态文案只有源码断言，界面渲染没被验证

**发现**：文档称 `DELIVERY_LABEL` 的键来自契约夹具，但浏览器测试里硬编码了一份列表；
夹具恒返回空数组，**任何一条记录都没真的进过 DOM**——「字段被渲染」只在源码层面成立
（读了不用、塞进隐藏元素都测不出）。

**整改**：
1. 夹具真正注入数据：`scripts/test-notify-latency-ui.mjs` 把受理记录与升级通道写进
   `sessionStorage`（`reload()` 会清空 `window` 变量、但保留 `sessionStorage`，这是关键）。
2. 断言改成**真实 DOM 断言**：等 `[data-remote-history]` 挂载后，用
   `waitForFunction` 读该容器的 `textContent`，必须出现「已受理，未确认送达」。
3. 遍历契约夹具的 `delivery_states`（4 态），**反向断言界面不再自建映射**。

**踩到的坑（值得记）**：`body.innerText` 在该页面读不到这些文字——`innerText` 只返回
**可见**文本，而设置页分区当前不可见。调试时抓到 `historyBox.innerHTML` 明明有内容、
`bodyHas: false`。改用 `textContent` + `locator.waitFor({state:'attached'})`
（默认 `waitFor` 等**可见**，会 TimeoutError）。

**验证**：
```sh
node scripts/automation/browser-check.mjs test-notify-latency-ui.mjs
# PASS: notification latency UI — unknown/19 samples/boundary/slow/incomplete/read failure; refresh is read-only
```
该 check（`not01.latencyui`）**在 release profile 内**，所以下面那 70/70 已包含这条 DOM 断言。

---

## A2：文档称界面显示通道故障告警，但 `escalated_channels()` 零消费者

**发现**：`escalated_channels()` 在**全仓库范围内没有任何调用方**——「升级态可读」是纯自述。

**整改**（补上整条链路）：
1. Rust：`main.rs` 新增 Tauri 命令 `remote_escalations` → 返回 `escalated_channels()`，
   并登记进 `invoke_handler` 列表（`main.rs:4781`）。
2. 界面：`views.js::hydrateRemoteHistory` 改成
   `Promise.all([invoke('remote_recent'), invoke('remote_escalations')])`，
   渲染 `class="sb-warn" data-remote-escalated` 故障条——**指名是哪个通道**、
   说明「已停止自动重试」、提示「恢复一次即复位」。

**验证**：浏览器测试注入 `escalationsFixture = ['ntfy']`，断言 DOM 里出现
`[data-remote-escalated]` 且文本含 `ntfy` 与「停止自动重试」。

---

## A3：`Throttled`/`permanent` 与 `Suppressed`/`NotConfigured` 的「不计入」是两种意思

**发现**：前者走 `_ =>` **清零连击**，后者早退**保留连击**——同一个「不计入」在文档里
被写成一句话，读的人会以为是一种行为。

**整改**：把两种行为在代码注释与文档里**分开写清**，并加交错序列测试钉住：

- `permanent`（配置错）与 `Throttled`（限流）：**连击清零**。它们不是链路抖动，
  不清零会把「配置错一次 + 抖动两次」误判成通道故障。
- `Suppressed`/`NotConfigured`：走闸门早退、**一次都没试网络**，连击**保留不动**。

**验证**：新增 `interleaved_other_outcomes_reset_or_hold_the_streak_as_documented`，
覆盖「抖动、抖动、配置错、抖动、抖动 → 不升级」与「抖动、抖动、被挡、抖动 → 升级」两条序列。

---

## A4：`DeliveryState::label()` 是死代码，文案有两处口径

**发现**：Rust 侧有 `label()`，界面侧另有一份 `DELIVERY_LABEL` JS 映射。
两份口径必然漂移，漂移后果是「受理成功被显示成未确认」这种**静默降级**。

**整改**：文案收敛到**唯一一处**（Rust 侧 `label()`）：

```
DeliveryState::label()  →  Recent.delivery_label  →  界面直接显示 entry.deliveryLabel
```

删掉 JS 的 `DELIVERY_LABEL` 映射；浏览器测试反向断言 `views.js` 里**不得**再出现
`accepted: '…'` 这类映射（出现了就报错，防止回潮）。

**验证**：`cargo test notifier::` 里三处断言
（受理 `"已受理，未确认送达"` / 且 `assert_ne!` 于 `"已送达"`；失败与限流 `"未确认"`）。

---

## A5：registry 的 `coverage_note` 仍称「缺」，与 `docs/需求测试矩阵.md` 矛盾

**发现**：`scripts/automation/registry.json` 里 NOT-05 的 `coverage_note` 写着
「缺失败升级、受理≠收到≠已读」，而矩阵与代码都已经有了。

**整改**：
1. `coverage_note` 改为与矩阵一致的事实陈述（已有失败升级与交付三段……）。
2. NOT-05 的 `checks` **补挂 `not01.latencyui`**：界面证据（文案 + 故障条的真实 DOM 断言）
   此前没挂在任何 requirement 上，等于跑了但不算数。

> 注：registry 有一条漂移校验禁止 check 侧维护 `maps_to_requirement` 副本
> （`test-test-registry.py:151`），权威映射就是 `requirements[].checks`。
> 一个 check 服务多个 requirement 是被明确支持的（注释里点了
> `task01.detailnav` 与 `ops01.measuremem` 两个先例）。

**验证**：`python3.14 scripts/test-test-registry.py` → `Ran 21 tests … OK`。

---

## A6：测试注释的因果讲错了

**发现**：原注释说「事件之间要跨过节流窗口，否则第 2、3 次会被节流挡下」——
失败路径**本来就会撤掉节流占位**（`if !outcome.is_accepted()` 里的 release），
所以在这个**失败**用例里那个间距是冗余的，注释把不存在的因果当成理由。

**整改**：注释改为如实说明——间距在失败用例里冗余，但升级用例脚本里混有成功态，
保持同一间距可让序列语义不依赖「哪一步是否撤了占位」这个实现细节。

---

## B1：变异表未独立复现 → 本轮**真的跑了两组变异**

「测试能抓住变异」这类声称，只有真跑过才算证据。本轮实测两组：

| 变异 | 注入点 | 结果 |
|---|---|---|
| 阈值 `3 → 2` | `ESCALATE_AFTER_FAILURES` | 交错测试 + 三次连续测试 **双双 FAILED** |
| 限流/永久失败「不清零」 | `_ =>` 分支删掉 `remove` | 交错测试 **FAILED** |

**⚠️ 第一组变异暴露了一个真实的弱断言**：`three_consecutive_transient_failures_…`
原来只断言「跑满 3 次后升级」，**没断言「第 1、2 次还不该升级」**——
阈值错改成 2 时它照样通过，是交错测试替它挡下来的。
已补边界断言（`index < 2` 时 `escalated_channels()` 必须为空），补后该变异被两个测试同时抓住。

这条正是复审 B1 的意义所在：**没有变异复现，「断言存在」和「断言有效」分不开。**

---

## B2：`Recent.delivered` 名字指代错误

**发现**：字段名说「送达」，实际语义是「**受理**」——NOT-05 的核心论点就是受理 ≠ 送达。

**整改**：改名为 `accepted`（配置的 serde 名随之变），并在字段注释里写明
「不叫 `delivered`：受理 ≠ 送达 ≠ 已读，三段见 `delivery`」。
全仓库 5 处消费点（`notifier.rs` 4 处断言 + `engine/state_tests.rs` 1 处）同步。

**验证**：`rg '\.delivered' app/src-tauri/src/` → 仅剩文档注释里的说明文字，无字段引用。

---

## 本轮验收数据

| 项目 | 结果 |
|---|---|
| Rust 全量测试 | `1098 passed; 0 failed; 14 ignored` |
| 门禁 `verify-framework.sh` | `ALL VERIFIED`（`fw rc=0`；4 profiles 因外部前置 blocked/unverified，非失败） |
| ci profile | **66 / 66 passed**（含 `automation.matrix`）|
| `test-automation-adapters` | **passed** —— 上一轮该项失败确系**并发共用 target dir** 所致（AGENTS.md 要求的隔离被违反），无并发复跑即通过 |
| release profile | **70 / 70 passed**（含 `not01.latencyui` 的 DOM 断言） |
| 脱敏检查 | `scan-secrets.sh` → 0 条新增命中，全部已备案 |
| 浏览器 DOM 断言 | PASS（受理文案 + 通道故障条 + 四态遍历） |
| 变异复现 | 2 组，均被抓住；并**因此发现并补上 1 处弱断言** |

## 仍**未**验证的部分（如实列出，不算已交付）

- 真实渠道（ntfy.sh 等）限流实测：`Retry-After` 的读取与文案由本地 TCP 测试证明，
  真实服务商行为未测。
- `received`/`read` 两段交付状态**永远填不上**：当前渠道没有回执能力，
  结构性留白而非待办（`delivery_states` 的四态是枚举完整，不是都能取到）。
- 升级态不持久化：重启从 0 起算，跨重启的升级行为未测。
- 岛内/窄条**不显示**通道故障（设计取舍，见文档）；窄条场景无自动化断言。
- `soak.resource`、`native.vm`、`rust.manual`、`task04.claudeplanclient` 四项
  受外部前置阻塞（VM / 人工 / 12h soak），未跑。

## 连带发现：改 registry 的 `requirements[].checks` 必须重生成矩阵

**这是本轮唯一一次门禁变红，值得单独记。**

把 `not01.latencyui` 挂到 NOT-05 之后，`verify-framework.sh` 的 ci profile 报
`automation.matrix` 失败（65/66）——矩阵文档是**从 registry 生成**的，
改了映射不重生成，`render-matrix.py --check` 就会判漂移：

```sh
python3.14 scripts/automation/render-matrix.py --check   # → matrix mapping drifted
python3.14 scripts/automation/render-matrix.py --write   # 重生成 docs/需求测试矩阵.md
```

重生成后 ci 回到 **66/66**。注意生成目标是 `docs/需求测试矩阵.md`
（不是 `docs/测试映射盘点.md`——后者是历轮手工盘点记录，不该被覆盖）。

**教训**：`requirements[].checks` 是权威映射，但它有**生成物消费者**。
改数据源之后要跑生成器，否则门禁会以「文档漂移」的形式报错，
而错误信息里的 check 名（`automation.matrix`）不会告诉你「你刚改了 registry」。

## 产物

- 二进制：`dist/agentisland`、`dist/AgentIsland.app`
- `build_id`：`c945abbd92ee…` / `source_manifest_sha256`：`fda36bcf0cb6…` / version `0.0.323`
- receipt 核验：`python3.14 scripts/automation/artifact.py verify --root .` → `code: ok`
- 运行中进程（pid 2274）加载的正是该产物，`Contents/MacOS/agentisland` 的 sha256
  与 receipt 记录一致

> 附注（两个指纹回答不同问题，不要互相校验）：本轮最后一次源码改动是在
> `#[cfg(test)]` 内补边界断言。重建后 `source_manifest_sha256` 变了
> （测试源码在清单覆盖范围内），而 release 二进制 sha256 **与上一轮产物逐字节相同**
> （`6fcab2671ac30da…`）——`cfg(test)` 之外的代码没动，编译器自然产出同样字节。
> 这正好互为交叉验证：清单指纹对测试敏感、二进制指纹对测试不敏感。

## 复现

```sh
cargo test --locked --manifest-path app/src-tauri/Cargo.toml
node scripts/automation/browser-check.mjs test-notify-latency-ui.mjs
python3.14 scripts/test-test-registry.py
bash scripts/verify-framework.sh
python3.14 scripts/automation/pipeline.py --profile release
bash scripts/scan-secrets.sh
python3.14 scripts/automation/artifact.py verify --root .
```

---

## 追加一：「渠道独立」是镜面声称——矩阵明列，但测试只跑 ntfy

整改 A1–A6 之后逐条核对矩阵场景时发现：NOT-05 的验收场景里写着**渠道独立**，
但 `notifier.rs` 的测试辅助函数 `run()` **硬编码 `Channel::Ntfy`**——
所有 19 个测试都只走一个通道。于是「渠道独立」从来没有被断言过。

**这是与 A1/A2 同一类的问题**：矩阵声称了、代码也按通道分键了（`channel.as_str()`），
但**没有断言把两者钉在一起**，所以任何一天有人把键改成全局，没有任何东西会响。

新增 `escalation_is_tracked_per_channel_not_globally`：ntfy 连打三次临时失败后
只有 ntfy 升级、customHTTP 不受牵连；且 customHTTP 成功**不复位** ntfy 的升级态。

### 我(agent)自己在这个测试里犯的两个错，都是变异测试逼出来的

**错 1：跨通道那一段用了另一个 notifier 实例。** 原来写成 `scripted(...)` 新建 `other` 再发——
那测的只是「新实例没有旧状态」（恒真），真正的缺陷形态（状态做成全局键）根本测不出来。
改成在**同一实例**上换通道。

**错 2：`Accepted` 来自兜底默认值，不是脚本设计。** `ScriptedTransport::perform` 在脚本用光后
`unwrap_or_else(|| Outcome::Accepted)`。原来只给 3 个元素却在第 4 次断言 `Accepted`——
那只是「兜底值恰好是 Accepted」的巧合。改成**显式**给第 4 个元素。

### 变异验证（含一次「没抓住」，如实记录）

| 变异 | 结果 |
|---|---|
| 升级/连击键换成全局常量 `"GLOBAL"` | **FAILED**（`left: ["GLOBAL"]`）——抓住了 |
| 把「已升级 → 停止重试」的条件换成全局升级闸门 | **没抓住** |

第二个变异没被抓住**是我的测试盲区，值得记**：升级态**只阻止「重试」**，
而重试闸门带 `self.retry_attempts > 0` 前置条件；我的测试用 `retry_attempts = 0`，
那条路径根本没走到。为补上这个盲区，新增独立测试
`escalation_stops_retrying_but_never_blocks_the_send_itself`（`retry_attempts = 1`，
断言升级后同一次失败**只发 1 次而非 2 次**，且**仍如实返回失败**）。
补后该变异被两个测试同时抓住。

**顺带纠正一处措辞**：我最初把这个测试的断言写成「换通道不该被 ntfy 的**故障状态**挡住」，
读起来像「升级会阻止发送」。实际语义是**升级只阻止重试、不阻止发送本身**
（文档 `docs/外发交付与故障升级交付验收.md` 写的是「停止**重试**」，与实现一致）。
断言与注释已改为如实陈述。

---

## 追加二：一处**没有**改的不一致（记录判断，不是遗漏）

矩阵给 NOT-05 标的层是 `L1/2/3/6/7`（含 **L2 格式与协议**——`Retry-After` 是 HTTP 头解析，
归 L2 合规）；而 registry 的 `requirements["NOT-05"].gates` 是 `["L1","L3","L7"]`（缺 L2）。

我(agent)查过：`requirements[].gates` **全仓库无人消费**（`evidence.py` 的覆盖度只从
`requirements[].checks` 构建），各需求的该字段与矩阵「层」列**26/36 不一致**，
说明两者不是同一件事（矩阵是**设计意图**，registry 那个字段另有语义）。

**因此我(agent)先改了又回退**——在没搞清字段语义前不该动它。
这里的结论只是「存在一处未经解释的不一致」，**不作为已修复**。
如需统一，应先把 `requirements[].gates` 的语义写进文档，再决定以哪边为准。

---

## 追加三：门禁里一条**既有 flake** 的根因被定位并修掉（不在 NOT-05 范围，但拦了门禁）

跑最终门禁时 `verify-framework.sh` 报 `rc=1`，失败项是 `test-automation-break`：

```
AssertionError: Items in the first set but not the second: 'automation-break-9x4gm2ms'
  scripts/test-automation-break.py:417  test_isolated_source_marker_is_detected_and_leaves_other_files_intact
```

**这不是我(agent)的改动引入的**，证据链：

1. `scripts/test-automation-break.py:22-30` 的注释自己写着「这套件已经这样失败过两次，
   之后 13 次运行都无法复现」——**测试自带 flake 诊断机制**。
2. `.scratch/automation-break-failures.jsonl` 里有 4 条历史记录，本次是第 4 条；
   前 3 条是别的测试名。文件由 `_RecordingResult` 写入（见该文件末尾）。
3. 单独跑该套件连续 3 次全 OK；失败前后 `/tmp`（实际是 `$TMPDIR`）无残留。

### 根因（可复现的机制解释，不是猜测）

- `temp_entries()` 断言的是**全局**临时目录里没有 `automation-break-*`
  （`Path(tempfile.gettempdir()).glob("automation-break-*")`）。
- 而 **ci profile 里有两个别的检查项也创建同一前缀**：
  `automation.break-artifact` 与 `automation.break-snapshot`，
  都调 `scripts/automation/break_check.py --isolated`，
  而该脚本用 `TemporaryDirectory(prefix="automation-break-")`。
- 只要这两者的清理与本测试的断言有**一瞬重叠**，断言就会把**别人的**目录
  读成「本测试泄漏」，报出一个属于别处的目录名——这正是日志里那个
  `automation-break-9x4gm2ms`。

**同一个文件里其它隔离测试都用「前后差集」写法**（`leftovers = self.temp_entries()`
→ 断言 `self.temp_entries() == leftovers`，见第 300/320/327/336/343 行），
**只有出问题的这一处写成了绝对空集**——写法不一致本身就是线索。

### 修法

把该断言改成与其它测试一致的前后差集：测「**本测试**有没有泄漏」，
而不是「整个 temp 目录此刻是否干净」——后者本来就不是它能保证的事。

### 变异验证（证明修完仍然有效）

把 `break_check.py` 的 `temporary.cleanup()` 改成空操作（模拟真实泄漏）→
`test_isolated_source_marker_is_detected_and_leaves_other_files_intact`
**仍在失败列表里**，与另外 5 个隔离测试一起红。
说明差集断言没有把灵敏度换掉，只是去掉了对无关套件的耦合。

> **边界说明**：这条 flake 与 NOT-05 无关，是既有资产的缺陷。
> 我(agent)修它是因为它**拦住了本次门禁**，且根因已被证据锁定（不是我(agent)猜的）。
> 修改仅限这一条断言 + 注释，未动 `break_check.py`（变异后已恢复，`diff` 为空）。

---

## 第二轮独立复审（2026-10-17 下午）结论与闭环

复审者在**隔离副本**（APFS clone）里独立验证了第一轮整改，结论：**6 条闭环
（A2/A3/A4/A5/A6/B2）、B1 部分闭环、A1 残留 1 处**，并抓出 **1 个新缺陷（N1，中）
与 2 个低级缺口（N2/N3）**。逐条核实后全部修掉，每条都有变异验证。

### N1（中）— 故障告警条渲染在默认收起的 `<details>` 里，用户看不到

复审者用真实布局量测证明：`detailsOpen=false`、`barInViewport=false`、
`closedAncestor="sb-group remote-advanced"`——「现在就不工作」的信号被折叠在
「高级设置」里，与注释自称的「比单条历史记录更急」正好相反。
而 `textContent` 断言对此完全无感（实测整块 `display:none` 照样 PASS）。

**修法**：故障条移出 `<details data-remote-advanced>`，放进**同级**的独立
`[data-remote-escalation-slot]` 容器（与「本地事件入队延迟」同级，
默认 `hidden`、有故障时显示，`aria-live="assertive"`）。

**断言**：浏览器回归新增**可见性断言**——`checkVisibility()` 不得为 false、
不得在 `details:not([open])` 内。
**变异验证**：只把 slot 容器移回 details 内（渲染逻辑一字不动）→ 精确报出
「故障条不得渲染在默认收起的 <details> 里（现在被折叠在 data-remote-advanced 内）」。

### N2（低）— A1 的「夹具共读」声称不成立：四态文案两侧各自硬编码

复审者实测：改 Rust 的 `label()` 文案（key 不变）→ cargo 只挂手动同步的那行，
**浏览器测试照旧 PASS**。夹具 `notify-keys.json` 只有 serde 名、没有文案，
所以「改文案 → 两侧都红」的机制是假的。

**修法**：契约夹具新增 `delivery_labels` 对象（四态文案的唯一来源）：
- Rust 侧（`models.rs`）：`DeliveryState::label()` 必须逐字等于夹具文案，
  逐态断言；
- 浏览器侧：渲染文案读 `contract.delivery_labels.accepted`，不再硬编码。

**变异验证**：改 `label()` 为「平台已收到请求」→ Rust 精确报出
「accepted 的界面文案与契约夹具不一致——界面会显示成另一句话」。

### N3（低）— `Recent.delivery_label` 的对外键名无守护

复审者实测：把 serde 名改成 `deliveryText`（JS 仍读 `deliveryLabel`）→
cargo 全量绿 + 浏览器绿，而真机上 `escapeHtml(undefined)` 变空串、
交付文案整段静默消失。根因：`notify-keys.json` 未收录 `Recent` 字段。

**修法**：夹具新增 `recent_fields`（`atMs/title/text/accepted/delivery/deliveryLabel`）；
Rust 侧序列化一个 `Recent` 样本、逐字段断言在 JSON 里存在。

**变异验证**：注入 `#[serde(rename = "deliveryText")]` → 精确报出
「Recent 必须以 deliveryLabel 暴露；界面按这个键名读取，缺了即静默显示空白」。

### 复审对 B1 边界断言的独立确认与 1 处遗留

复审者在**同一份 3→2 变异下删掉**我补的边界断言 → 该测试变绿，
独立证实「边界断言承重」这一自述为真。

复审者自己注入的一个变异**没被抓住**：让 `NotConfigured` 闸门早退也调
`note_channel_result`（把「保留」改成「清零」）→ 交错测试只用
`Suppressed`（`send_attention: false`）验证「保留」，没有 `NotConfigured` 的交错用例。
**已修**（当轮）：新增 `not_configured_keeps_the_streak_unlike_a_real_outcome`——
抖动、抖动、配置缺（空白主题命中 `missing_field`）、抖动 → 应升级；
并断言配置缺失那次不进传输层（`seen` 恰好 3 次）。
**变异验证**：在 `NotConfigured` 早退处注入 `note_channel_result` 调用
（清零连击）→ 只有这条新测试 FAILED，恢复后 diff 为空。

### 复审期间的并发告警（已理解、无实质影响）

复审者如实记录「另一个 agent 会话（即本 agent）正在实时编辑」，
并因此用哈希钉住每条判定。这不影响其结论的有效性——它所有变异都在隔离副本。
其中一处判定基于旧版文件（浏览器守卫加入前的 `ed4698f7`），
复审者自己用新版复测确认守卫有效。

---

## 追加四：补上**外发链路的真实二进制端到端验收**（本轮最重要的新增交付）

复查「端到端验收」时发现一个结构性缺口：**入站账本有真实进程验收，外发链路没有。**

- 入站：`not01.ledgerrestart` → `scripts/test-notify-ledger-restart.py --binary dist/agentisland`
  → 三次真实进程、真实退出与恢复；
- 外发：**只有 `#[cfg(test)]` 单测**，且用的是注入的 `ScriptedTransport`（假传输）。
  也就是说「**发布产物里的 `HttpTransport` 真的连出去、真的对端回 429/500/200、
  真的读到 `Retry-After` 头**」这条路径，此前在发布物上**没有任何验收**。

这正是目标里「端到端验收」缺的那一块，所以补上（不是可选项）：

### 新增
1. **探针**：`app/src-tauri/src/remote_delivery_probe.rs`
   - 与 `notify_ledger_probe` 同一纪律：显式 argv（`--remote-delivery-probe`）、
     headless、**从不读用户配置/钥匙串**、只对**探针自己起的 loopback 对端**说话、
     `ACTIVE` 标记短路日志；
   - 用**生产 `Notifier::new()`**（真实 `HttpTransport`，`retry_attempts = 1`），
     不是假传输。
2. **验收脚本**：`scripts/test-remote-delivery-probe.py --binary dist/agentisland`
   （断言全部落在**回执 JSON**上，不解析日志）。
3. **注册**：`not05.remotedelivery`，`depends_on: [package.artifact]`，
   profiles = `package` + `release`（与 `not01.ledgerrestart` 完全对齐；
   **不能进 ci**——`package.artifact` 需要真构建产物，registry 校验会拒绝）。

### 它验证什么（三条 NOT-05 核心语义，都在真实 HTTP 上）

| 场景 | 真实对端行为 | 断言 |
|---|---|---|
| 限流 | 回 **429 + `Retry-After: 12`** | 判为 `Throttled`；文案含「约 12 秒后可重试」；交付态 `未确认`（不冒充任何一段）；对端只收到 **1** 次请求（不立刻重试） |
| 失败升级 | 连续回 **500** | 三次后升级 `customHTTP`；请求数 = **6**（3 × 初次+重试）；升级后再发一次 → **只 +1**（不循环）且仍如实返回失败 |
| 受理 | 回 **200** | `accepted=true`、交付段 `accepted`、文案「已受理，未确认送达」且**不得**是「已送达」 |

### 探针第一版就被真实链路纠正了一个**我(agent)的错误假设**

第一版用 **503** 触发升级 → 升级列表为空。查 `transport.rs:193`：
`status == 429 || status == 503` 都判 `Throttled`，而 `Throttled` 按 NOT-05 **不计入升级**。
**503 是「服务端说了现在不行」，不是链路抖动**；能累计连击的是 500/网络层错误。
改用 500 后通过。这条如果只写单测、不进真实链路，很可能一直错下去——
**这正是端到端验收的价值**。

### 变异验证（证明验收真的有效）

| 变异 | 结果 |
|---|---|
| 受理文案改成「已送达」 | 探针 exit 2，精确报「受理不得说成已送达，实际文案：已送达」 |
| 升级后仍重试（`!is_escalated` 恒真） | 探针 exit 2，精确报「升级后必须只发一次不重试，实际新增 2 次请求」 |

两次变异均已恢复（`diff` 为空）。

### 边界（如实）

- 探针只连 loopback 明文 HTTP（`customHTTP` 通道），**不测真实第三方渠道**
  （ntfy.sh 等的限流行为仍属未验证）；TLS 路径由 `transport.rs` 的
  `https_really_connects_end_to_end_against_a_local_certificate` 单测覆盖。
- 升级态不跨重启（进程内存），探针不涉及重启语义——那是入站账本的领域。

### 探针自身的一个缺陷（我(agent)引入、自查发现、已修）

新探针的 `ACTIVE` 标记**没有接到日志路径上**：`main.rs::log_line` 只检查了
`notify_ledger_probe::ACTIVE`，所以我(agent)的探针每次运行都会往
**用户的真实日志文件**（`$TMPDIR/agentisland-tauri.log`，实测已累积 1.3 MB）
追加 `[run]` 行。

这违反了探针的契约（「只读用户状态、只在临时/loopback 里动作」），
也让「探针没碰任何东西」这句话不可证——而入站那个探针正是靠这个短路做到的。

**修法**：`log_line` 同时检查 `remote_delivery_probe::ACTIVE`。
**验证**（字节数比对，不是看输出）：
```
探针前: 1312637
探针后: 1312637   ← ✓ 字节数不变，探针不再写用户日志
```

---

## 第三轮独立复审（定案轮）：结论「能定案」

复审者在隔离副本里独立复现了 5 条整改，**全部判定闭环且断言承重**，
并纠正了我(agent)几处过期数据。逐条：

| 项 | 复审者独立结论 | 其变异证据 |
|---|---|---|
| N1 故障条移出 details | 闭环，两条断言**独立承重** | ① slot 移回 details → 精确报「被折叠在 data-remote-advanced 内」；② bar 加 `display:none`（slot 仍在 details 外）→ 精确报「checkVisibility 不得为 false」——证明可见性断言不是被「不在 details 内」顺带覆盖；③ else 不 hidden → 报「陈旧故障」 |
| N2 四态文案共读夹具 | 闭环 | 改 `label()` → `models.rs:675` 精确报两侧差异；并确认浏览器测试里**零**文案字面量 |
| N3 `Recent` 字段名契约 | 闭环 | `#[serde(rename="deliveryText")]` → `models.rs:699` 报「缺了即静默显示空白」 |
| B1 遗留 NotConfigured | 闭环 | 早退处注入 `note_channel_result` → **只有**新测试挂（21 passed/1 failed），旧交错测试单独跑仍 ok |
| 注册守卫 | 闭环 | 删 `remote_escalations,` → 精确报未注册命令 |

**复审者纠正的三处过期数据**（都对，我(agent)认账）：

1. release 现在是 **71** 项而非 70——因为我(agent)在报告「追加四」里新增了
   `not05.remotedelivery`（`package`+`release`）。**旧快照已作废**，正确数字是 71。
2. **`source.integrity` 红是并发编辑所致，不是缺陷**：流水线指纹覆盖全部跟踪
   + 未忽略未跟踪内容，而本工作区一直有其它会话在写（复审者现场抓到两个并发
   `pipeline.py`、`main.rs` 被改、`dist/` 被重建）。**结论：活仓库上无法取得
   「运行期间零变动」的一次全绿**——这是结构性限制，不是失败。
3. receipt/pid 值随时间作废（至少三次重建）；唯一有意义的判据是
   `artifact.py verify` 的 `code=ok` 与**当前** `build_id`。

**复审者的方法学提醒（我(agent)记下）**：用 `cp -c`（clonefile）恢复被变异文件时
**mtime 被保留**，cargo 判定无需重编 → 「恢复后重跑」会给出**假阴性**。
必须 `touch` 强制重编，或核对 `Compiling` 行。这条对任何后续变异验证都适用。

### 复审者新发现 P1（低）— 已修

**问题**：新加的外发端到端验收链路**又抄了第三份文案字面量**——
`scripts/test-remote-delivery-probe.py` 与 `remote_delivery_probe.rs` 里各写了一遍
「已受理，未确认送达」，正是 N2 要消灭的形态。

**复审者的严重度判断是准确的**（低，不是中）：与 N2 的原缺陷有本质差别——
N2 是**静默分歧**（两侧一致改文案，真机显示没人验证过的那份）；
这里是**响亮分歧**（改夹具+`label()` 而漏改探针 → 探针 exit 2 明确报错），
不会让假绿通过，只是把「唯一来源」从 1 处变成 3 处。

**修法**：探针与脚本都读契约夹具 `delivery_labels`（`remote_delivery_probe.rs`
新增 `contract_label()`，用 `include_str!` 读**同一份**文件；脚本读同一个 JSON）。

**变异验证（改夹具文案，三方都要红）**：
```
夹具 accepted → 「已受理，未确认送达（改）」
  探针：exit 2 —— 受理不得说成已送达，实际文案：已受理，未确认送达
  Rust：FAILED —— accepted 的界面文案与契约夹具不一致——界面会显示成另一句话
恢复后夹具 diff 为空
```
探针报错里「实际文案」是 `label()` 的旧值、夹具已是新值——**恰好证明探针确实
在拿夹具与实现比对**，而不是自己跟自己比。

---

## 最终交付状态（NOT-05，2026-10-17 收尾）

### 交付物
- `dist/agentisland`、`dist/AgentIsland.app`（v0.0.323）
- receipt：`python3.14 scripts/automation/artifact.py verify --root .` → **`code: ok`**
- `build_id=565d213384c5` / `source_manifest_sha256=f47441344f87`
- 运行中应用：pid 2373，加载的正是该产物

### 验收证据（全部为收尾时的实测，不是历史快照）
| 项 | 实测结果 |
|---|---|
| cargo 全量 | **1101 passed / 0 failed / 14 ignored** |
| 统一门禁 `verify-framework.sh` | **ALL VERIFIED**（rc=0；4 profiles 因外部前置 blocked/unverified） |
| release profile | **71 / 71 passed**（含新增 `not05.remotedelivery`） |
| 外发端到端（真实二进制） | **PASS** — 429+Retry-After / 三次 500 升级且不循环 / 200 只算受理 |
| 入站端到端（真实二进制，对照） | **PASS** — 三进程幂等/恢复/ack 不复活/0600 无隐私 |
| 浏览器回归（真实 DOM） | **PASS** — 含交付文案、故障条**可见性**与**清除后消失**断言 |
| registry 自校验 | `Ran 21 tests … OK` |
| 矩阵一致性 | `matrix mapping matches registry` |
| 脱敏扫描 | **0 条新增命中**，全部已备案 |

### 三轮独立复审的最终结论
- 第一轮：**不能定案**（A1–A6 / B1 / B2 / D）→ 全部整改
- 第二轮：**不能定案**（N1 中 / N2 / N3 / B1 遗留）→ 全部整改
- 第三轮：**能定案**（5 条整改独立复现为闭环且断言承重）；其新发现的 P1（低）已修

### 方法学教训（写给下一个做这件事的人）
1. **「断言存在」≠「断言有效」**：本轮每一次修复都配了变异注入，并因此发现
   **3 处真实的弱断言**（阈值边界未断言、跨通道用了新实例、`Accepted` 来自兜底默认值）。
2. **`cp -c` 恢复文件会保留 mtime**，cargo 会跳过重编 → 变异恢复后重跑得到**假阴性**。
   必须 `touch` 强制重编，或核对 `Compiling` 行。
3. **活仓库上跑全量门禁时，`source.integrity` 必然红**（指纹覆盖全部未忽略内容）。
   要一份可定案的全绿，必须在编辑停止后跑——这是结构性限制，不是缺陷。
4. **文案/键名的「唯一来源」要连**：夹具是唯一来源，但探针/脚本很容易再抄一份。
   本轮 N2 → P1 就是这个形态重复了两次。

### 如实列出：仍未验证（不是待办，是边界）
- `received` / `read` 两段交付态**结构性填不上**：当前渠道没有回执能力，
  枚举完整但不代表都能取到（`DeliveryState` 四态是契约完整，非能力完整）。
- **真实第三方渠道**（ntfy.sh 等）的限流行为未实测：`Retry-After` 的读取与文案
  已由本地真实 HTTP 证明，但服务商的实际限流策略取决于对方。
- **升级态不跨重启**（进程内存），跨重启的升级行为未验。
- 探针只连 **loopback 明文 HTTP**（`customHTTP` 通道）；TLS 路径由
  `transport.rs` 的 `https_really_connects_end_to_end_against_a_local_certificate` 单测覆盖。
- 4 个 profile 被外部前置阻塞：`soak.resource`（12h）、`native.vm`（VM）、
  `rust.manual`（人工）、`task04.claudeplanclient`。
- **P1 的修复由本 agent 自证**（变异：改夹具 → 探针与 Rust 断言双双报错 + 全链复跑），
  未经第四轮独立复核——第三轮复审判定 P1「不阻塞定案」。

### 未做（按纪律）
未 `git add` / `commit` / `push` / `tag` / release。工作区保留全部既有未提交改动
（含其它会话的），本任务只动与 NOT-05 相关的文件。
