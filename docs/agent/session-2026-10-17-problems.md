# 2026-10-17 会话问题、踩坑与分诊

用户要求：把当天上下文中遇到的问题与踩过的坑落档记录。

记录范围：NOT-05 外发交付状态与故障升级的交付与三轮独立复审、外发端到端验收探针的补建、
v0.0.324 发版（含脱敏门禁拦截一次）。本篇按现有会话上下文整理，**不是**原始会话逐字重放。
凡本会话内可复现的，标注复现入口；无法核实的引述或次数，撤回或标为待核实。

**发布基线**：`405d515` / `v0.0.324`（2026-10-17 已推送并建 Release）。
**取材**：本会话的工具输出（命令与回显）、[整改报告](../code-review/2026-10-17-not05-review-remediation.md)、
[交付验收](../外发交付与故障升级交付验收.md)、`/tmp/release-324.log`、三轮复审 agent 的返回。

---

## 1. 影响事实准确性的问题（最高优先）

| ID | 记录 | 可确认边界 | 分诊 |
| --- | --- | --- | --- |
| A-1 | **断言存在 ≠ 断言有效**：三处「已通过」的断言实测抓不住对应变异 | ①升级阈值测试只断言「跑满 3 次后升级」，**没断言第 1、2 次不该升级**——阈值 `3→2` 变异下它照样绿；②「渠道独立」测试的跨通道那一段用了**另一个 notifier 实例**，测的只是「新实例无旧状态」（恒真）；③该段断言的 `Accepted` 来自 `ScriptedTransport` **脚本耗尽后的兜底默认值**，不是脚本设计 | 已有「拒错负例」规则的**强实例**：每次修复必须配变异注入。本轮 3 处弱断言全部由变异逼出 |
| A-2 | 用 `cp -c`（clonefile）恢复被变异文件后重跑，得到**假阴性** | clonefile 保留源 mtime ⇒ cargo 判定无需重编 ⇒ 跑的还是变异版。改用 `touch` 强制重编后拿到真结果 | 独立复审者踩到、我(agent)也踩到。**变异验证流程必须核对 `Compiling` 行或强制 touch** |
| A-3 | 把「另一通道不该被节流挡住」当成事实写进注释——因果是错的 | 节流键是 `agentId\|kind`（CONTEXT.md 明写，**不含通道**），跨通道共享；换通道能发出去的真实原因是失败路径 `release` 了占位 + 间隔超出窗口。**节流不参与「渠道独立」，这是有意设计** | 与 A-9 同族：写注释前先查实现。已在代码里写明「若把节流误当 per-channel 来读这个测试，会得出错误结论」 |
| A-4 | 首次跑 `verify-framework.sh` 用管道 `tail` 读到 `SOMETHING FAILED`，误判为门禁失败 | 实为并发写导致 `source.integrity` 红。写文件再读即得 `rc=0 / ALL VERIFIED` | 测量口径：**不要在管道里读退出码**，`rc=${PIPESTATUS[0]}` 或先落文件 |
| A-5 | 把 release 的 `source_drift` 读成「产物失效」 | 实际是**读得太早**——构建仍在 `cargo test` 阶段、`dist/agentisland` 还是旧件。构建完成后 receipt 为 `ok` | 先确认进程/产物 mtime 再解释 receipt 状态 |
| A-6 | 探针自身违反自己的契约：`ACTIVE` 没接到日志路径上 | `main.rs::log_line` 只查了 `notify_ledger_probe::ACTIVE`，新探针每次运行都往**用户真实日志**（`$TMPDIR/agentisland-tauri.log`，实测已 1.3 MB）追加 `[run]` 行 | 已修（字节数前后比对验证）。**新探针必须复刻既有探针的全部短路点**，不只 argv 解析 |
| A-7 | 自造探测数据的教训在本轮**升级为硬规则**并生效 | 本轮全程未向真实常驻应用投递任何事件；真实账本只读。发版后重启属常规操作 | 承接 10-16 的 C-15。本轮无新污染 |

---

## 2. 产品与工程缺陷记录

以下「已修」表示本会话内有实现与验证记录，**不表示**原生化之外的完整需求已闭环。

| ID | 问题 / 本轮处置 | 值得沉淀的位置与理由 |
| --- | --- | --- |
| B-1 | 交付状态文案在 Rust `label()` 与 JS `DELIVERY_LABEL` **两处各存一份**；改一侧另一侧不动会静默降级 | 踩坑史：**文案/键名的「唯一来源」必须连成一条**，否则每加一条链路就多抄一份 |
| B-2 | 界面故障告警条渲染在**默认收起的 `<details data-remote-advanced>`** 里；`textContent` 断言对「不可见」完全无感（实测整块 `display:none` 照样 PASS） | 踩坑史：**渲染断言要带可见性判据**（`checkVisibility()` + 不在 `closed details` 内） |
| B-3 | 只断言「故障条出现」，未断言「故障清除后消失」——else 分支写错会让陈旧故障永久挂在页面 | 踩坑史：**状态机两侧都要断言**；陈旧故障比不显示更坏（用户去排查已好的通道） |
| B-4 | `Recent.delivered` 名字指代「送达」、实为「受理」——而受理≠送达正是本需求核心论点 | 已改名 `accepted`。领域设计：**字段名不能与它表达的口径相反** |
| B-5 | `Recent` 的对外键名不在契约夹具里：改 serde 名 → cargo 全量绿 + 浏览器绿，真机上 `escapeHtml(undefined)` 变空串、交付文案整段消失 | 已把 `recent_fields` 纳入夹具。测试文档：**序列化契约要覆盖新结构**，不只覆盖既有摘要 |
| B-6 | 探针/验收脚本**又抄了第三份**文案字面量（N2 的同一形态复发） | 已让探针与脚本都读夹具 `delivery_labels`。**同族缺陷在一条链路上重复出现**，说明「唯一来源」需要机制而非自律 |
| B-7 | 外发链路**没有真实二进制端到端验收**：入站有 `not01.ledgerrestart`，外发只有注入假传输的单测 | 已建 `not05.remotedelivery`。验收文档：**发布物上要有一条真链路证据**，单测不算 |
| B-8 | 探针第一版用 **503** 触发升级，升级列表为空 | `transport.rs:193` 把 429 **与 503 都判 `Throttled`**，而 `Throttled` 按设计不计入升级。改用 **500**。领域设计：**503 是「服务端说现在不行」，不是链路抖动**——这条只写单测发现不了 |
| B-9 | `test-automation-break` 偶发：断言 glob **全局**临时目录里没有 `automation-break-*`，而 ci profile 里另有 `break-artifact`/`break-snapshot` 创建**同前缀**目录 | 已改为与同文件其它测试一致的**前后差集**。测试文档：跨套件共享全局资源时，断言必须限定到「本测试产生的那一份」 |
| B-10 | 豁免清单的注释**自己命中扫描规则**（注释里写了形如 `?key=…` 的字面量） | 已在注释里标注「本文件自身也过同一套规则，不写出该地址的字面形态」。**自指文件要按被扫描对象来写** |
| B-11 | 改 registry 的 `requirements[].checks` 后 `automation.matrix` 报漂移 | 矩阵由 registry 生成；改数据源要跑生成器。**错误信息里的 check 名不会提示「你刚改了 registry」** |
| B-12 | 新增 check 误进 `ci` profile：`depends_on: [package.artifact]` 而 `package.artifact` 不在 ci ⇒ 校验拒绝 | 与 `not01.ledgerrestart` 对齐为 `package` + `release`。**依赖项必须与被依赖者在同一 profile** |
| B-13 | `requirements[].gates` 与矩阵「层」列 26/36 不一致 | 查证后**先改又回退**：该字段全仓无人消费，语义未明。**记为一处未经解释的不一致，不作为已修复** |

---

## 3. 流程与协作

| ID | 问题 | 分诊 |
| --- | --- | --- |
| C-1 | 复审者报告「树在我评审期间被改」（记录哈希 `7d287d71`→`7ad49b25` 等） | 实际是**我(agent)本会话在编辑**。复审者的处置正确（隔离副本 + 哈希钉住每条判定）；我(agent)的处置也正确（不等不猜）。**并行只读审查 + 记录内容哈希**可继续 |
| C-2 | 我(agent)在 release 运行期间编辑了报告文档 ⇒ `source.integrity` 指纹前后不一致 ⇒ 流水线正确拒绝认证 | **运行期间不碰任何文件**。这是结构性限制：活仓库上无法在编辑同时取得一次干净全绿 |
| C-3 | 发版清单用 `unicode_escape` 还原中文路径失败，`release.sh` 报「文件不存在且未被跟踪：docs/å¥éå»¶…」 | 改用 `git status --porcelain -z`（NUL 分隔，无转义）。**处理非 ASCII 路径不要靠转义还原** |
| C-4 | `AskUserQuestion` 弹窗超时（5 分钟无回答） | 按规则**不发布**，先做无副作用准备。随后用户消息与我的推荐假设一致才推进。**超时等于未授权**，发布类动作尤其 |
| C-5 | `tart run` 用 shell `&` 后台化被工具拒绝（「remove shell background markers」） | 改用 `runInBackground: true`。macOS 无 `timeout` 命令，ssh 用自带 `-o ConnectTimeout` |
| C-6 | 首次发版在「提交后复扫」被拦：`render.rs` 1 个未核准 `url_token_param` blob | **闸门按设计工作**——拦下时提交已建、**未推送**，可修。核实为该内容在上一版已存在、仅因文件改动换了 blob 号；按既有格式备案。**这条闸门必须留在 push 之前** |
| C-7 | 因复扫失败，`release.sh` 无法从头重跑（会在「提交」步报「没有可提交的改动」） | 手工接续剩余步骤（打包 → tag → push → release），顺序与脚本定义一致。**脚本不支持断点续跑**，是可改进点 |
| C-8 | 我(agent)此前在报告里写「70/70」，实际已是 **71**（新增 `not05.remotedelivery`） | 复审者纠正。**发布数字随时间失效**，报告应绑定哈希或改为引用命令 |

---

## 4. 未验与遗留

| ID | 项目 | 当前口径 |
| --- | --- | --- |
| D-1 | `received` / `read` 两段交付态 | **结构性填不上**：当前渠道无回执能力。四态枚举完整 ≠ 能力完整 |
| D-2 | 真实第三方渠道（ntfy.sh 等）限流 | 未实测。`Retry-After` 读取与文案由本地真实 HTTP 证明；服务商行为取决于对方 |
| D-3 | 升级态跨重启 | 未验（升级态在进程内存） |
| D-4 | 探针 TLS 路径 | 探针只连 loopback 明文 HTTP；TLS 由 `transport.rs` 单测覆盖 |
| D-5 | 原生动效逐边取证、官方 Claude 客户端取证 | 发版时**登记为未验项**（需显式 opt-in 与外部资源），未冒充通过 |
| D-6 | `soak.resource` / `native.vm` / `rust.manual` / `task04.claudeplanclient` | 外部前置阻塞；`native.vm` 在本次发版中**已通过**（见 §5） |
| D-7 | `requirements[].gates` 与矩阵层列不一致 | 见 B-13，未修、未解释 |
| D-8 | 我(agent)未独立复跑的项 | 复审者列出的「未跑」项（如 `transport::` 的 Retry-After 单测计数）本会话未逐项复跑 |

---

## 5. 本轮正向证据（不只是坑）

- **隔离 VM 原生验收真实通过**：`acceptance-408c116c29674cce80c58fcc6abfe2cd`，
  `guest_verified=true`、`console_unlocked=true`、`os_build=26A434`、`exit_code=0`、
  `All native gates passed against transferred artifact hashes`。
- **三轮独立复审形成有效对抗**：第一轮「不能定案」（A1–A6/B1/B2/D）→ 第二轮「不能定案」
  （N1/N2/N3 + B1 遗留）→ 第三轮「**能定案**」并另发现 P1（低）已修。
  复审者纠正了我(agent)三处过期数据，也确认了我(agent)自述的「边界断言承重」为真。
- **脱敏门禁真实拦住一次**（C-6），且拦在 push 之前。
- 收尾验收：cargo **1101 passed / 0 failed / 14 ignored**、门禁 `ALL VERIFIED`（rc=0）、
  release **71/71**、外发与入站两条真实二进制端到端 **PASS**、脱敏 0 新增。

---

## 6. 更正台账

| 初稿/过程中的说法 | 更正 |
| --- | --- |
| release 是 70/70 | 已增至 **71**（本轮新增 `not05.remotedelivery`）；70 是旧快照 |
| `source.integrity` 红 = 缺陷 | 并发编辑所致；活仓库上无法避免。**不是缺陷** |
| `receipt: source_drift` = 产物失效 | 读得太早；构建完成后为 `ok` |
| 「节流是 per-channel」（我(agent)写的注释） | 错。节流键 `agentId\|kind` 跨通道共享；换通道能发出去的真实原因是失败路径 release 占位 |
| 「升级会阻止发送」 | 错。**升级只阻止重试、不阻止发送本身**；文档写的是「停止重试」，与实现一致 |
| 503 会触发升级 | 错。429 与 503 都判 `Throttled`，不计入升级；用 500 |
| 把 `NotConfigured` 缺口记为「改动面大于收益，留白」 | 复审证据改变了判断，**本轮已修**（新增交错用例 + 变异验证） |
| 复审者报告的「树被另一会话改动」 | 实为本会话我(agent)在编辑；不影响其结论有效性 |
| `verify-framework.sh` 第一次跑失败 | 管道里误读；写文件重读为 `rc=0 / ALL VERIFIED` |

以上更正保留错误来源与实际边界，避免复盘自身再成为错误事实的依据。

---

## 7. 分诊结论

- **已有规则，补执行证据**：变异验证（A-1）、测量口径（A-4/A-5）、隔离测试（A-7）、
  拒错负例。重复扩写规则不会解决未执行问题。
- **候选升级为硬规则**（按价值排序）：
  1. **每次修复必须配变异注入，且核对 `Compiling` 行**（A-1/A-2）——本轮 3 处弱断言全靠它发现。
  2. **文案/键名的唯一来源要连成一条链路**，新增链路时从夹具读，不抄字面量（B-1/B-5/B-6）。
  3. **渲染断言要带可见性判据**（B-2）。
  4. **状态机两侧都要断言**（B-3）。
  5. **跨套件共享全局资源时，断言限定到本测试产生的那一份**（B-9）。
  6. **发布类动作以弹窗超时为未授权**（C-4）。
  7. **流水线运行期间不碰文件**（C-2）。
- **领域/验收文档**：B-4/B-7/B-8 以 [外发交付与故障升级交付验收](../外发交付与故障升级交付验收.md)
  为权威；D 表继续以该文档与 [需求测试矩阵](../需求测试矩阵.md) 为准，此复盘不关闭未验项。
- **本次实际落地**：NOT-05 交付并三轮复审定案、外发端到端探针建成、v0.0.324 发布。
  未新增 AGENTS.md 规则；本篇作为候选经验的证据附录。

---

## 8. 复现入口

```sh
# 单测（含本轮新增断言）
cargo test --locked --manifest-path app/src-tauri/Cargo.toml

# 外发真实二进制端到端（NOT-05）
python3.14 scripts/test-remote-delivery-probe.py --binary dist/agentisland

# 入站真实进程端到端（对照）
python3.14 scripts/test-notify-ledger-restart.py --binary dist/agentisland

# 浏览器回归（含故障条可见性与清除后消失断言）
node scripts/automation/browser-check.mjs test-notify-latency-ui.mjs

# registry / 矩阵一致性
python3.14 scripts/test-test-registry.py
python3.14 scripts/automation/render-matrix.py --check

# 全量门禁
bash scripts/verify-framework.sh
python3.14 scripts/automation/pipeline.py --profile release

# 脱敏（工作区 / 含 git 历史）
bash scripts/scan-secrets.sh
bash scripts/scan-secrets.sh --history
```

发版（需要隔离 VM 在运行 + 来宾账号）：

```sh
RELEASE_FILES_FILE=<清单> NATIVE_TEST_VM=agentisland-iso NATIVE_TEST_USER=admin \
  bash scripts/release.sh <X.Y.Z> "<标题>"
```

> **注意**：`release.sh` 不支持断点续跑。若在「提交后复扫」失败，需手工接续
> 打包 → tag → push → release（顺序见脚本），或先 amend 修复后重跑全流程。
