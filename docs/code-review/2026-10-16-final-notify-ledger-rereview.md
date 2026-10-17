# NOT-01 通知账本：后加内容的独立复审（2026-10-16）

审查者：周衡（作者≠审查者）。被审对象：工作树未提交状态，HEAD `61d3fa3`。

## 为什么另开一份，而不改 21:10 那份

[2026-10-16-2110-notify-ledger.md](2026-10-16-2110-notify-ledger.md) 定格于 21:10。之后的改动
（21:49–21:50：共享契约夹具、`notify_unacked`/`notify_restored` 值断言、`models.rs` 同批变更）
**不在它的覆盖范围内**。按 AGENTS.md「绝不事后改写已提交的报告（它是时间点证据）」，
那份保持原样，本次针对**新增内容**另开报告。

## 复审范围（四项）

| 项 | 位置 | 判别力 | 依据 |
| --- | --- | --- | --- |
| 共享契约夹具 | `app/src-tauri/tests/fixtures/contracts/notify-keys.json` | **能拒错** | Rust 侧 `models.rs` 正向查键 + 反向查 camelCase 变体；JS 侧 `scripts/test-notify-latency-ui.mjs` 反向查「views.js 引用了契约外的 `notify_*` 键」。浏览器回归实跑 PASS |
| `notify_unacked` / `notify_restored` 值断言 | `app/src-tauri/src/notify_ledger_tests.rs` | **能拒错** | 断言恢复条数是**启动快照**（重放后不变）与**含队首**且 ack 后减一；若 `pending_count()` 退化为 `pending_events.len()`，断言立刻变红 |
| CLI `notify` 从 `kind` 改 `event` | `app/src-tauri/src/cli.rs` | **能拒错** | 把 CLI 真实请求体喂进生产 `notify_route`，断言解析出的 `event_type` 与声明一致；改回 `kind` 即失败 |
| `restart-app.sh` 优雅退出 | `scripts/restart-app.sh` | 见下 | 原先**没有任何门禁覆盖**，本次补上 |

## restart-app.sh：本轮发现的真实缺陷与修复

审查中发现两处，均已修并加测试：

1. **`osascript` 调用自身没有超时（真实缺陷）**。脚本的 8 秒等待是在 `osascript` **返回之后**
   才开始计时的。应用无响应时 Apple Event 会一直挂着，脚本就卡在这一行，等待循环一次都轮不到
   ——而那正是「优雅退出」最需要它的场景。修复：给 `osascript` 加 10 秒上限
   （`timeout` 若存在则用，否则用系统自带 `perl` 的 `alarm`，不依赖 GNU 工具）；
   超时后仍走信号兜底并在 stderr **明说**本次写盘收尾可能未执行。
2. **该路径无任何门禁覆盖，且「已实测生效」无回执**。新增 `scripts/test-restart-app.sh`
   （登记为 `auto01.restartapp`，进 `fast`/`ci`/`release`），在临时沙箱用假命令驱动**真实脚本**，
   不启动应用：

   | 断言 | 结果 |
   | --- | --- |
   | `osascript` 挂住时被自身超时打断，且仍以信号兜底 | 通过（耗时 21s 内，远小于无限挂住） |
   | 明说「本次写盘收尾可能未执行」 | 通过 |
   | 优雅退出生效时**不**告警（不静默降级成信号终止） | 通过 |
   | 等待循环真的跑过（`pgrep` 调用次数可观测） | 通过 |
   | 旧实例杀不掉时**非零退出**并指出重启未发生 | 通过 |
   | 脚本语法 | 通过 |

   为使该路径可测，脚本的 `OSASCRIPT` 改为可覆盖（`${OSASCRIPT:-/usr/bin/osascript}`），
   与仓库既有的 `SDKROOT` / `WINDOWS_TYPECHECK_CACHE` 覆盖点同风格。

## 交付文档中已更正的取证错误

**用「二进制内含 `notify_evicted` 等字符串」证明 UI 改动入包**——那四个串是 **Rust 侧字段名**，
对 UI 没有判别力。结论碰巧正确，但**取证方法错**。正确证据是收据的源码清单直接绑定
`app/ui/js/views.js`（`sha256=648d36c79b9ad5e0…`、`size=149564`，与当前文件逐字节一致，
`app/ui` 共 80 个文件在清单内）。交付文档已改为该证据并标注原方法的错误。
（Tauri 前端以压缩形式内嵌，明文搜索本不可靠。）

## 未复验 / 边界

- 桌面与原生四档（`native`/`manual`/`client`/`soak`）仍 blocked，需真实隔离用户会话或 VM。
- 真实 GUI 常驻进程内的 `notify_evicted`/`notify_restored` 目视值：本仓库无不干扰的读取通道
  （HTTP 只有入站、`agentisland state` 看不到进程内自报、隔离实例按设计关闭持久化）。
- 断电/强杀、磁盘满、Windows 实机未验。
- 本次未复现「从 `REQUIRED` 移除两个锁文件后 `test_all_build_input_categories_change_digest`
  立刻变红」这一次具体变异（约束禁止改文件），但该机制由测试自身逐项断言，且
  `REQUIRED` 确含这两个文件。

## 结论

四项后加内容的**判别力均已核实**；其中 `restart-app.sh` 暴露并修掉一处真实缺陷
（`osascript` 无超时会使 8 秒等待失效），并补上此前缺失的门禁。交付文档的取证错误已更正。
本报告不复核 21:10 那份已覆盖的内容，也不改写它。
