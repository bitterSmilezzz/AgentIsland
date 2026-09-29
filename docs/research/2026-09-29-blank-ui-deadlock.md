# 界面空白：启动阶段 AB-BA 死锁（v0.0.242 修）

> 这份记录补上 [`2026-09-29-blank-ui-two-build-config-causes.md`](./2026-09-29-blank-ui-two-build-config-causes.md)
> 没能定位的那一层。前一份把 `SDKROOT` 与 CSP 当成根因，**两个都不成立**（见文末「被推翻的说法」）。

> **长期口径已沉淀**：[ADR 0014 持锁时不调回主线程的 API；探针只读](../adr/0014-no-main-thread-dispatch-while-locked.md)。

## 一句话

**引擎线程持有引擎锁去写托盘，而写托盘必须回到主线程同步执行；
主线程正在等这把锁。于是双方互等，应用的 `setup` 永不返回，
事件循环从不启动，WKWebView 从不导航——窗口是空的。**

## 症状：每一条「看起来正常」都成立

`$TMPDIR/agentisland-tauri.log`，同一次运行：

| 现象 | 值 |
| :--- | :--- |
| 窗口建出来了吗 | ✅ `["workbench", "sidebar", "island"]` |
| URL 对吗 | ✅ `tauri://localhost/index.html?shell=island` |
| 尺寸位置对吗 | ✅ island 744×1040 物理像素（= 372×520 @2x）、`visible=true` |
| 前端资源嵌进去了吗 | ✅ `index.html=1826B`、`js/views.js=96040B`、MIME 正确 |
| WebView 内容进程起来了吗 | ✅ |
| `on_page_load` | ❌ **一次都不触发** |
| `[webview]` 日志 | ❌ **零行** |
| **画面** | ❌ **空白** |

## 为什么前面十几轮全都跑偏

**因为可观测的面全在「死掉的那一侧」。**

`on_page_load` 与 `[webview]` 日志都属于**事件循环**。`setup` 挂住 ⇒ 事件循环不转 ⇒
这两样必然是空的。于是从外面看，「空白」同时符合下面每一个假设：

- 前端模块图没求值起来
- 资源没嵌进二进制
- URL / 查询串 / 资产协议不对
- CSP 把脚本拦了
- Tauri 全局 API 没注入
- WebView 进程没起来

前几轮就是在这份清单里逐条排除的——**每一条都排除对了，但清单本身是错的。**
它们的真实状态是「都对，只是没人跑到那一步」。

## 怎么把它挖出来的

关键是**换一个不会被死锁波及的观测面**：从 **Rust 侧**问 webview「你现在是什么状态」。

`WebviewWindow::eval_with_callback` 在真实 webview 里执行 JS 并把**返回值**交回 Rust。
在 `setup` 里分三个时刻（启动即刻 / +1.5s / +5s）对三个窗口各发一次：

```rust
w.eval(WEBVIEW_TRAP_JS);                    // 先装 error / unhandledrejection 陷阱
w.eval_with_callback(WEBVIEW_PROBE_JS, |r| log_line(&format!("[eval] {tag} {r}")));
```

脚本内含一条**不走 Tauri IPC 的兜底通道**（`fetch('http://127.0.0.1:42000/__probe__/…')`），
因为 `invoke` 正是可能坏掉的那一环，用它证明「JS 跑过」是循环论证。

**修复前的结果**：`eval` 与 `eval_with_callback` 都返回 `Ok`（没打出「装不上 / 取不到」），
但回调**一行都没触发**，兜底请求也**一行都没到** ⇒ 推断 webview 里 JS 引擎压根没在跑。

这条推断对了一半：**不是 JS 引擎没跑，是没人让它跑。**

统一日志（`com.apple.WebContent`）同样一条没有。`eval` 又只回 `Ok` 不带原因。
于是上 `sample`——直接看线程在干什么。

## 证据：两条线程的栈（`sample <pid> 2`）

进程状态 `STAT S`、`%CPU 0.0`：睡着，不是空转。
主线程 **1633 个采样点全在同一处**。

**主线程**（栈底 = 当前所在）：

```text
__psynch_mutexwait
  ← std::sync::Mutex::lock              mutex.rs:492
  ← engine::ActivityEngine::lock
  ← main.rs:1476                        ← setup 闭包里读设置那行 shared.lock()
  ← tauri::setup                        app.rs:2698
  ← … ← [NSApplication run] ← AEProcessAppleEvent
      ← _handleAEOpenEvent ← _sendFinishLaunchingNotification
```

**引擎线程**（`Thread_…513`）：

```text
agentisland::engine_loop                main.rs:1018
  ← tauri::tray::TrayIcon::set_title
    ← std::sync::mpsc::Receiver::recv
      ← Condvar::wait ← Thread::park
        ← _dispatch_semaphore_wait_slow
          ← semaphore_wait_trap          ← 等主线程执行完
```

**这就是全部。** macOS 的托盘是 AppKit 的 `NSStatusItem`，Tauri 写它必须回主线程，
而且是**同步等结果**的（`mpsc::recv` → `park` → dispatch 信号量）。

```text
引擎线程：持引擎锁 ──等主线程──▶ （托盘写入）
主线程  ：要引擎锁 ──等引擎线程──▶ （放锁）
```

**每次启动必然发生**，因为徽标每个 tick 都要写，而 `setup` 紧接着就要读设置。
这解释了它 100% 复现，也解释了为什么改 URL / 改 CSP / 换 SDK / 换 data URL 全部无效——
**那些手段一条都不在这条链上。**

`setup` 不返回 ⇒ Tauri 不进事件循环 ⇒ WKWebView 从不发起导航 ⇒ 窗口在、页面不在 ⇒ 空白。

## 修法

**在临界区里只做纯计算，把会回到主线程的写入挪到锁外。**

```rust
let (state, interval, badge) = {           // 临界区：算，不碰 AppKit
    let mut e = shared.lock().unwrap();
    …
    let badge = tray_badge_text(&badge_mode, working, s.grand_total.tokens24h);
    …
    (s, interval, badge)
};
apply_tray_badge(&app, badge);             // 锁外：写托盘
let _ = app.emit("engine://tick", &state);
```

关键是这个项目里**本来就有这条规矩**，而且命令侧一直是对的：

```rust
let (edge, width);
{
    let mut e = state.lock().unwrap();      // set_shell_mode / set_sidebar_width / …
    …
}
match mode { … sidebar.hide() … }           // 锁早就放了
```

**唯独引擎循环漏了。** 修的就是这一处，其余不动。

## 守护

`main_thread_dispatch_sentinel`（两条）：

1. `tray_writes_happen_outside_the_engine_lock` — 引擎循环源码里，
   `shared.lock()` 与 `apply_tray_badge(` 之间**不许**出现 `set_title` / `tray_by_id`。
2. `the_badge_is_still_written_every_tick` — 徽标不能为了躲死锁被静默丢掉。

**变异验证过**：把 `set_title` 塞回临界区，第一条会 FAILED（不是靠注释通过）。

## 修复后的对照

同一份二进制、同一条命令，只是锁的顺序变了：

| 证据 | 修复前 | 修复后 |
| :--- | :--- | :--- |
| `[page]` | 0 行 | 3 个窗口各一行 |
| `[webview] Tauri API 就绪` | 0 行 | 3 行（等了 0ms） |
| `[webview] shell=…` | 0 行 | 3 行 |
| `[eval]` 快照 | 回调从不触发 | `ready=complete, tauri=true, invoke=true, errs=[]` |
| root DOM 长度 | — | workbench 8013B / sidebar 992B / island 101B（收起的窄条） |

## 两条方法论

1. **观测面要和故障面解耦。** `on_page_load` 与 `[webview]` 日志都长在事件循环上，
   事件循环死了它们当然沉默——**用它们判断「前端坏没坏」是循环论证**。
   要问 webview 内部，就从 Rust 侧问回去。
2. **「没日志」有两种含义**：没发生，和发生不了。分不开时不是继续加日志，
   而是换一个不会同时被它影响的面（线程栈、进程列表、文件落盘）。

## 被推翻的说法

- `SDKROOT` 钉死在 26.5 SDK（v0.0.235 声称的根因）——**不成立**。改成选最新 SDK 之后，
  `[page]` 在发布版里**一次都没响过**，直到本次修掉死锁才第一次出现 3 行。
- `security.csp` 为 `null`（v0.0.236 声称的根因）——**不成立**，理由同上。
  两条配置本身该修（显式 CSP、别钉 SDK 都是对的），但它们**不是**空白的原因。

⚠️ 上一份文档里「改成显式 CSP 后 `[webview]` 从 0 行变成 3 行」这个观察，
与本次实测（修复前 `[webview]` 恒为 0 行）冲突。**推断**：那是把
**累积日志**当成了当次运行的证据——同一个坑在 `main.rs` 的 `[run]` 标记注释里记过一次。
这是推断，没有独立证据。

## 取证命令

```sh
L="${TMPDIR}agentisland-tauri.log"; : > "$L"
./target/debug/agentisland >/dev/null 2>&1 & PID=$!
sleep 8
sample $PID 2 -file /tmp/sample.txt
kill $PID

# 线程栈归属（别只看主线程——死锁的两边都在）
awk '/^ +[0-9]+ Thread_/{th=$0} /ActivityEngine|engine_loop/{print NR": ["th"] "$0}' /tmp/sample.txt
```
