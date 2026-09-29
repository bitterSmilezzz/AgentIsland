# 界面空白：两个构建配置的坑（v0.0.235 / v0.0.236）

> ⚠️ **本文的两个「根因」都已被推翻**（v0.0.242）。真正的原因是启动阶段的
> AB-BA 死锁：引擎线程持锁写托盘 ↔ 主线程要锁。完整取证见
> [`2026-09-29-blank-ui-deadlock.md`](./2026-09-29-blank-ui-deadlock.md)。
>
> 本文的**配置建议依然有效**（显式 CSP、别把 SDKROOT 钉死），
> 但它们**不是**空白的原因；正文里「修完 `[page]` 从 0 变成 6」与实测冲突。
> 正文按当时的样子保留，作为时间点证据。

> 这份记录写给**下一次换机器、或在别人机器上构建**的人。
> 两个坑都不报任何错，症状一律是「界面空白」——看起来像前端代码写错了，
> 而实际上一个在 `build-app.sh` 里，一个在 `tauri.conf.json` 里。

## 症状长什么样

| 现象 | 值 |
| :--- | :--- |
| 窗口建出来了吗 | ✅ 三个窗口都在 |
| 尺寸位置对吗 | ✅ `island` 372×520、`visible=true` |
| 前端资源嵌进去了吗 | ✅ `index.html=1840B`、`text/javascript` MIME 正确 |
| 命令注册了吗 | ✅ `invoke_handler` 里 45 个 |
| WebView 内容进程起来了吗 | ✅ 3 个 `WebContent` 进程 |
| **画面** | ❌ **空白** |

**每一条「看起来正常」都成立**，唯一失效的是最后一行。

## 坑一：`SDKROOT` 钉死在旧版本（v0.0.235 修）

`build-app.sh` 原来写死：

```sh
export SDKROOT="/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk"
```

而这台机器跑 **macOS 27.0**。拿 26.5 的 SDK 去链 WKWebView、跑在 27.0 上，
结果是 **webview 根本不发起导航**——`on_page_load` 一次都不触发。

改成按「机器上最新的那个」挑之后，**同一份代码**，`[page]` 从 0 变成 6。

### 为什么它这么难查

唯一可观测的征兆是「`on_page_load` 不触发」，而这个征兆**只说明页面没加载**，
不说明为什么。当时依次排除过：资源没嵌入、URL 格式、查询串、资产协议、
模块循环依赖、Tauri 全机注入时序、bundle id 冲突——**全都不是**。

一个更省事的判别法（当时没想到）：
**用纯 WKWebView 测同一台机器**。若纯 WKWebView 也坏 ⇒ 系统级；
若它好而 Tauri 坏 ⇒ 问题在 Tauri 这条链上。写成十几行 Swift 就行。

## 坑二：`security.csp` 写成 `null`（v0.0.236 修）

```json
"security": { "csp": null }
```

这样写时，Tauri/wry 会施加一条**会拦掉本项目脚本**的策略。
页面照常加载（`[page]` 正常），但 **JS 一行都不执行**（`[webview]` 永远 0 行）。

改成显式给出策略之后：

```
[webview] Tauri API 就绪（等了 0ms）    ← 此前永远是 0 行
```

三个窗口各一行。界面从这一刻起才是活的。

```json
"security": {
  "csp": "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self' ipc: http://ipc.localhost http://127.0.0.1:42000 ws://127.0.0.1:42000"
}
```

这条策略比 `csp: null` **更安全**，不是权宜之计：它写清了这份界面到底允许什么。
`connect-src` 里的回环地址是本机 webhook（ADR 0013），`ipc:` 是 Tauri 的 IPC。

## 怎么证明「页面加载了但 JS 没跑」

`[webview]` 是 **JS 自己写的**日志——JS 不跑它就必然是空的，
所以**用它证明「JS 跑过」是循环论证**。

要一个**独立于 Tauri IPC** 的观测点。做法：探针页只含内联 JS（不依赖任何模块），
`fetch()` 一下本机 HTTP 端口，而 Rust 侧每收到一个请求就记一行 `[http]`。

结果：探针页的 `[http]` 是 0 行，而 `[page]` 是 6
⇒ 与模块图无关、与 `invoke` 无关，**webview 里的 JavaScript 根本没执行**。

## 守护

`app/src-tauri/src/main.rs` 的 `build_env_sentinel` 两条用例守着这两点：

- `the_csp_is_explicit_rather_than_null`
- `the_packaging_script_never_pins_an_sdk_version`

两条都做过**变异验证**：把 `csp` 改回 `null` / 把 `export SDKROOT=` 写死回去，
各自精确变红，恢复后转绿。

## 顺手记一条：窗口「不在屏上」的读法

`CGWindowListCopyWindowInfo` 的窗口 **bounds 不需要录屏权限**（窗口是否存在、标题、
在不在屏上都读得到），但**别的进程的窗口尺寸在没有录屏权限时会返回 0**。

第一版把 `bounds=(0,0) 0x0` 读成了「窗口没尺寸」，据此得出「窗口没摆好」——
那是权限缺口的产物，不是缺陷。真实尺寸要用 Tauri 自己的 `outer_size()` 读。
