# 0014. 持锁时不调回主线程的 API；探针只读

- 状态：已接受
- 日期：2026-09-29
- 相关：[ADR 0010 Swift 冻结与 Rust 前置门](0010-swift-freeze-and-rust-prerequisites.md)、
  [ADR 0012 侧边栏与灵动岛并存](0012-sidebar-alongside-island.md)、
  取证：[界面空白：启动阶段 AB-BA 死锁](../research/2026-09-29-blank-ui-deadlock.md)

## 背景

Rust 端交付之后，界面**一直**是空白的。拖了十几个版本才定位到真因：

```text
引擎线程：持引擎锁 ──等主线程──▶ （写托盘徽标）
主线程  ：要引擎锁 ──等引擎线程──▶ （放锁）
```

macOS 的托盘是 AppKit 的 `NSStatusItem`，Tauri 写它必须回到主线程，而且是
**同步等结果**的（实测栈：`mpsc::recv` → `Condvar::wait` → `Thread::park` →
`_dispatch_semaphore_wait_slow`）。于是 `setup` 永不返回，Tauri 不进事件循环，
WKWebView 从不发起导航——窗口在、尺寸对、资源嵌好了，`on_page_load` 一次不响。

值得注意的是：**这个坑命令侧一直没踩。**
`set_shell_mode` / `set_sidebar_width` 等都写成

```rust
{
    let mut e = state.lock().unwrap();
    …
}                                   // ← 锁在这里就放了
match mode { … sidebar.hide() … }    // ← 之后才碰窗口
```

规矩一直存在、也一直被遵守，**唯独引擎循环漏了**。所以这不是「没人知道规矩」，
而是「规矩没有守到点子上」。

## 决定

**一、持锁期间不得调用任何会同步派发到主线程的 Tauri API。**

具体到本项目就是托盘（`tray_by_id` / `set_title`）与一切窗口操作
（`set_size` / `set_position` / `show` / `hide` / `set_focus` / `eval`）。

- 临界区里**只做纯计算**。需要用到主线程的值，先在锁内算好，出了锁再写出去。
- 引擎循环现在的形状是：`锁内算出 badge 文本` → `出锁` → `apply_tray_badge(&app, badge)`。
- 这条不靠自觉：守护 `main_thread_dispatch_sentinel` 直接扫源码，
  断言 `engine_loop` 里 `shared.lock()` 与 `apply_tray_badge(` 之间
  **不出现** `set_title` / `tray_by_id`。**变异验证过**——把写入塞回临界区，
  守护精确变红。

**二、诊断用的探针必须只读。**

这条不是推论，是同一场排查里我自己连踩两次：

① 为「保险」在 +2.6s 补发一次 `tray://toggle`——此刻 `state.expanded` 已是 true，
toggle 语义于是把刚展开的岛**收了回去**，日志明明打了「已展开」而测到 0 个可点元素；
② 在探针里顺手调一次 `place_island(330, 120)`「验一下它通不通」——
它把岛窗口改了尺寸，在测量途中改掉了被测状态。

**诊断动作只要动了一点状态，它测出来的就不是原来那个东西了。**
「探针没测到」不能推出「那里没有」——要先问「探针有没有自己动过手脚」。

需要驱动状态变化时，走**产品自己的入口**（启动参数、真实事件），
别另开旁门：另开旁门验的就不是用户在用的那条路。

## 派生出的第二条规矩：等状态要等**具体标志物**

同一场排查里还栽了一次：UI 冒烟的 `settle()` 判「root 非空」就算稳，
而收起态的窄条本来就让 root 非空——于是这个等待等于没等，
驱动在卡片出现前就去查选择器了。改成等具体的标志物（本例是 `.card`）
并**记录它出现所需时间**，才把「一直是窄条」与「卡片闪过又被收回」分开。

## 什么时候改

规则本身不会过期。若将来 Tauri 提供了**异步**的主线程派发（不等的），
`apply_tray_badge` 可以改用它——但**判据仍是同一条**：调用点不得在持锁期间发生，
因为「同步变异步」是实现细节，「不持锁」才是规则。
