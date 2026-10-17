# 事件类型覆盖与 CLI 参数修复：独立复审（2026-10-17）

审查者：周衡（作者≠审查者）。被审对象：NOT-01 事件类型从 3 类扩展到 8 类
（`question`/`failure`/`review`/`schedule`/`chief` 新增）及附带的 CLI 参数修复。
被审快照为复审启动时的未提交工作树；本报告记录**复审结论与逐项修复状态**，
不改写更早的两份报告（[账本复审](2026-10-16-2110-notify-ledger.md)、
[后加内容复审](2026-10-16-final-notify-ledger-rereview.md)）。

## 复审结论（原文要点）

复审判定「**不能定案**」，给出 7 项必修。其独立核实确认的**正面结论**先记在这里：

- `normalized_kind` 的值域被 `_ => "completed"` 限死在 8 项内，而 `decode()` 白名单与它
  **逐项相等**——因此「webhook 接受、decode 判无效 → 账本永久停写」这条组合路径
  在当前代码下**结构性闭合**，不是巧合。
- `EventKind::ALL ↔ parse ↔ as_str ↔ serde` 互查、逐类开关穷尽 match、逐类 snake_case
  别名、同 id 换类型各占一条（真实 TCP + 生产 consumer）四项判别力属实。
- 既有三类型行为未变；dedup / 节流键 / 通知路由无回归。

## 7 项必修与修复状态

| # | 复审发现 | 修复 |
| --- | --- | --- |
| 1 | `audit.rs::event_type_text` 与 `models.rs::summary` 只认旧三类 → 导出报告把 5 类印成「任务完成」；`views.js` 的「与 summary 同口径」注释已成假话 | 两处均补 8 类文案，与界面同口径（`audit.rs:114`、`models.rs:418`） |
| 2 | **深链 `notify` 是第三套语义**，把 question 等静默折叠成 completed，且零测试 | 抽出 `deeplink_event_type()` 并接 `EventKind::parse`，保留深链历史别名（`confirm`/`wait`/`cost` 等），未知值才回落；补 2 条测试（每类不被折叠 + 别名 + 未知回落）。该路径此前**零覆盖** |
| 3 | **界面关不掉新 5 类外发**：默认全开、前端不读开关，只能手改配置文件 | 通知页补 5 个开关；夹具 `event_kind_switches` 改为 UI 实际使用的 snake_case；Rust 断言「关掉它只影响这一类」+ 浏览器断言「开关真的渲染」。此前该键是死数据 |
| 4 | `/notify` 的 400 文案手写字面量，会随新增类型漂移 | 改为从 `EventKind::ALL` 派生（`invalid_event_kind_body`），加断言：含全部 8 类 + 分隔符个数与清单一致 |
| 5 | CLI 只拒了空格形式；**单横线 `-kind` 与拼错前缀 `--kindd=x` 仍静默 fallback** | 参数校验改白名单：只接受三个 `--名=值`，其余（裸标志、单横线、未知标志）一律拒绝并逐条打印 |
| 6 | `cli.rs` 有一处**恒真断言**（自称「与实现同一套规则」，实为对字面量求值） | 换成真正穿过实现的断言（等号形式不被参数检查拦住） |
| 7 | 交付文档 1077 / 1085 并存，未注明来源 | 已改：旧值标注「该版源码当时读数」，并统一以「三·D / 最终产物」为准 |

## 复审未列、后续补修的两处

| 发现 | 处置 |
| --- | --- |
| 收起态摘要与窄条提醒**只认 `attention`/`costSpike`**——卡片能看到 question，收起岛后完全无感 | 集中为 `views.js::ALERT_EVENT_TYPES` + `isAlertEvent()`，卡片 / 收起态 / 窄条三处共用；浏览器断言清单含全部提醒类、且调用点 ≥3 处 |
| `task-attention.js::taskCoversEvent` 仍只认 `attention` | **刻意不扩**：该函数回答「任务关口的待处理项是否就是当前事件」，而任务关口只承载确认；放宽会让一条 `failure` 被误当成关口确认项。边界已写入设计文档 |

## 变异验证（新增断言的判别力）

| 变异 | 结果 |
| --- | --- |
| 界面删掉 `send_question` 开关 | 浏览器测试拒绝：「views.js 没有渲染 question 的外发开关」 |
| 深链把 question 折叠成 completed | Rust 测试拒绝（`deeplink_event_type` 两条用例红） |
| `EventKind::ALL` 去掉 `chief` | 编译错（清单一致性断言的长度校验） |
| `parse` 去掉 `question` | Rust 测试拒绝 |
| `wire_value` 去掉 `sendChief`/`sendSchedule` 别名 | 逐类 snake_case 测试拒绝（**camelCase 那侧直接命中，只有 snake_case 能暴露**） |
| 界面删 `failure` 分支 / 两类文案改成相同 | 浏览器测试拒绝 |

## 本轮门禁与产物

- `cargo test`：**1089 passed / 0 failed / 14 ignored**
- 通知页浏览器回归：PASS
- `verify-framework.sh`：rc=0、`ALL VERIFIED`（4 档外部环境 blocked）
- `release` 档：**70 passed / 0 failed / 0 blocked**
- 脱敏扫描（工作区 + 全 git 对象）：通过
- 最终产物：`build_id=0a1968222e4f…`，源码摘要 `d910a001a511…`，
  收据 `status ok`、与当前源码一致、产物哈希绑定
- `not01.ledgerrestart`（三真实进程）：PASS

## 未验边界（继承交付文档，本报告不关闭它们）

- 真实 GUI 界面内 `notify_evicted`/`notify_restored` 的目视值：本仓库无不干扰的读取通道。
- 桌面/原生四档、断电/强杀、磁盘满、Windows 实机：未验。
- 「question 投递到真实常驻进程 → 落盘保留」的证据来自上一版产物（`ac0de371…`）；
  其所依赖的 `webhook.rs`/`notify_ledger.rs` 此后未改动，故由**源码清单 hash** 而非再次
  投递来证明等价——按 AGENTS.md「用户真实状态只读」，不再向运行中的应用投递探测事件。

## 关联：本轮另有一次数据事故

见[会话复盘](../agent/session-2026-10-16-problems.md) 第 8 节：为验证类型覆盖曾向真实
应用投递探测事件，清理时用过宽的时间窗判据误删真实数据，已从备份恢复并把规则写入
`AGENTS.md`「用户真实状态只读」。本报告的验收结论**不受该事故影响**（账本内容与
通知功能无关），但事故本身必须随这轮一起记档。
