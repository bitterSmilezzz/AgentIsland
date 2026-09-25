# codenotch 一手调研

> 一手核实记录。来源：GitHub API 元数据与 git tree（2026-09-26）、
> [vinzdg/codenotch](https://github.com/vinzdg/codenotch) 默认分支 `main` 的
> `README.md` 全文（23946 字符 / 433 行）与 26 个源码/文档/CI 文件的逐行原文。
> GitHub API 元数据（2026-09-26）：**2493 star / 383 fork / 12 watchers / 53 open issues /
> MIT / Swift / 创建于 2026-09-05 14:55:35Z / 最后 push 2026-09-24 / repo size 161555 KB**。
> 本文只记录「它是什么」「它怎么做到的」「和我们已有能力的关系」「我们要不要抄」。
> 引用按 `path:line` 给出，读不到的写「未获取」。**全文不含任何真实 token / key 值。**
>
> 本地素材：`/tmp/uibatch/codenotch/`（`README.md`、`repo.html`、`tree.json`、`meta.json`、
> `commits.json`、`rel.json`，以及 `dl_*.swift` / `dl_*.md` 共 26 个抓取件）。

---

## 1. 一句话结论

**codenotch 是本文档写到这里为止最直接的竞品：同样 Swift、同样 macOS、同样把「屏幕边缘一个小条」
做成常驻 UI。但它把「边缘小条」做成的是「配额表」，我们做成的是「agent 状态台」——
两者共用一个形态，回答的不是同一个问题。** 它真正赢我们的地方不在形态也不在功能面，
而在两件工程纪律：**一个可验证的数字恒等式**（「它的环与 Claude Code 自己的 /usage 永不一致」被当作
硬约束实现并逐级回退）与**每 commit 出预览构建 + 18 天 16 个正式 release 的发布节奏**。
它真正该让我们警惕的只有一条：**它证明了「厂商侧配额」有干净的本地读取路径**
（Claude Desktop 的 HTTP 缓存是一个零网络、零凭据、纯读文件的入口），
而这条路径与我们「不碰网络、不持有密钥」的口径**只在后两级冲突，第一级不冲突**——
拆开看，可拍板的粒度比「要不要接配额」细得多。详见 §5。

---

## 2. A. 产品层

### 2.1 形态：一个 26pt×210pt 的黑 pill，不是「小条」

README 第一句：「A macOS app that pins a small black notch to a screen edge」（`README.md:3-5`）。
设计 spec 写死了几何（`docs/specs/2026-08-28-usage-notch-design.md`）：

- **静止态**：`NotchLayout.pillWidth = Design.px(26)`、`pillHeight = Design.px(210)`
  （`Sources/Notch/NotchLayout.swift:46-48`）——竖贴在右边，1–5 个 provider cell 竖排。
- **每个 cell**：一个 44pt 圆 + 下方百分比标签，圆被进度弧包住，弧长 = 已用百分比
  （spec 的 Collapsed state 图；`NotchLayout.ringDiameter = Design.px(117)` 即 44pt，
  `NotchLayout.swift:53`）。
- **环色四档**：0–49% 绿 / 50–79% 黄 / 80–99% 橙 / 100% 橙满环 + 暗淡 glyph（spec 的 Ring colour states 表）。
- **hover 卡**：`NotchLayout.cardWidth = Design.px(600)`、`cardCorner = 49.5`、
  `tailLength = 75` / `tailHeight = 87`（`NotchLayout.swift:255-259`），带指向被 hover cell 的尖尾，
  每个 limit window 一块（label + 重置文案 + 4pt 圆角轨条 + 「N% Used」）。
- **四边可贴**：`NotchEdge` 有 right/left/top/bottom 四态，上下边会把 stack 转成横向
  （`Sources/Notch/NotchEdge.swift:9-14`）；⌥+拖到边上滑动，每条边记住位置
  （`README.md:268-279`）；有硬件刘海时 top 贴会「做成刘海的形状，让两者读成一个东西」
  （`README.md:273-274`）。
- **折叠/展开**：静止是 pill，指针到达展开；可配「总是显示」或「完全隐藏」
  （`README.md:284-285`）。设置入口是 notch 下方一个 orb（静置是弧、hover 是齿轮）（`README.md:286`）。

**尺寸上它的 hover 卡比我们的卡片还宽**（600 设计像素 vs 我们 Swift 端 `cardWidth = 330`、
Tauri 端 `tauri.conf.json:15-16` 的 `372×520`）。「小」只是它的静止态——
这一点对 §4.2 的形态哲学对比很关键。

### 2.2 一维栈空间 + 唯一映射点（最值得抄的一处结构）

`NotchPlacement`（`Sources/Notch/NotchPlacement.swift`）的注释把设计说死了：
所有布局只在 **stack space** 里算，只有两个坐标——`along`（沿 provider 栈）与
`across`（**从 bezel 向内量**，0 永远是屏幕边），而 `NotchPlacement` 是
「唯一一个知道哪个轴是哪个的地方」。`rect(along:across:length:depth:)` 一律
**向内** span 而非对称 span，「一个锚在 bezel 上的命中区不得挂到 bezel 的远侧，
那里没有屏幕可待」（`NotchPlacement.swift:19-24`）。

对比我们：灵动岛的 `.edge-top` / `.sliver.vertical` 分支散在 `app/ui/css/island.css`
与 `IslandPanelPositioning.swift`，是**按边分支**而不是「一维算 + 一处映射」。

### 2.3 三态（实为四态）怎么判定

**数据模型只有四态**：`AgentSession.State = busy | waiting | success | idle`
（`Sources/Sessions/AgentSession.swift:14-19`）。归约逻辑在 `ActivitySummary`：

- 无会话 → `init?` 返回 nil，**cell 直接消失而不是留一句「什么都没发生」**
  （`Sources/Sessions/ActivitySummary.swift:25`）。
- 优先级：`waiting` > `busy` > `success` > `idle`，注释给了理由——
  **「任何 blocked on you 都压过任何 merely busy，因为那是唯一一个 notch 在向你要东西的状态」**
  （`ActivitySummary.swift:28-31`）。
- 颜色只编码状态，且 working 一律取 `textPrimary`（白），注释解释了为什么不用状态色：
  **「指示器待在一个环里，环的颜色已经意味着『你的额度用掉了多少』，
  中性色调不可能被误读成那个标尺的一部分；waiting 拿琥珀色，因为那是唯一想要你干点事的状态」**
  （`ActivitySummary.swift:62-74`）。
- 多一个 `queued: Int`——本地运行时的请求排队数，「对每个云端 agent 都是 0，
  它们没有这么一条队可报」（`ActivitySummary.swift:14-15`）。

**判定来源按 agent 分三路**（以 Claude 为例）：

1. **会话登记目录**：盯 `~/.claude/sessions`（或 profile 目录），
   `DispatchSource` 文件事件 + 2 秒 timer 双轨，timer 覆盖「文件事件不会报的两件事：
   没碰目录就死掉的进程，以及桌面 app 托管的会话在干什么——那根本不在目录里」
   （`Sources/Sessions/ClaudeSessionMonitor.swift:8-15`、`:72`）。
2. **进程活性不信文件**：`ProcessLiveness.isAlive` 先查 pid 存在，再比对
   `sysctl(KERN_PROC_PID)` 的 `p_starttime`，「一个崩溃的会话会把文件留在那里永远说 busy，
   所以 notch 必须查而不是信」；pid 复用容忍窗 5 分钟（`Sources/Sessions/ProcessLiveness.swift`）。
3. **桌面会话读 transcript**：`ClaudeSessionMonitor.state(of:)` 只对
   `!record.reportsStatus` 的记录读 transcript，`turn == .inFlight ? .busy : .idle`，
   注释强调**「terminal 会话永不走这条路，这保住了它们的 `waiting` 状态——
   那是只有终端界面知道的一件事」**（`ClaudeSessionMonitor.swift:219-226`）。

**「什么时候该响」是一个独立的纯函数**（`Sources/Sessions/SessionCompletionWatcher.swift`）：
只认 **离开 busy** 这一次 crossings（`reason(from:to:)` 的 `guard was == .busy else { return nil }`，
`:70-71`）；「一个问题被回答不是一件工作结束」（`waiting → idle` 不报）；
**会话文件消失整条丢弃不播报**，「a chime for a window that is already gone points at nothing」
（`:54-59`）；**首次读取什么都不播**——每个已存在的会话无历史，
「把启动当成一次 transition，会让每次启动、包括 Sparkle 半夜更新后的重启，
为每个开着的窗口响一次」（`:37`）。

**它自己的诚实边界**（`README.md:232-239`）：会话只发布 pid，
所以「点击把那个应用带到前台」是**向上走进程树找启动它的 app**；
「在 app 内选 tab 需要终端自己的 scripting 字典，而没有一个通用的：
Terminal.app 与 iTerm2 能按 tty 匹配 tab，Warp 与 Ghostty 根本不发布 scripting dictionary。
所以 app 被抬起，tooltip 里点名叫会话，这留给用户最后一次按键」。
**这段话就是我们 `SessionFocus` / `TerminalTabFocus` 那一类问题的答案：它选了「只抬 app + tooltip 点名」。**

### 2.4 「the two never disagree」是怎么做到的（它最强的卖点）

`ClaudeOAuthProvider` 的文档注释即设计声明（`Sources/Providers/ClaudeOAuthProvider.swift:1-13`）：
三个源，按序，**任一失败降级而不是猜**。顺序与判据：

| 序 | 源 | 成本 | 失效条件 |
| :--- | :--- | :--- | :--- |
| 1 | **Claude Desktop 自己的 HTTP 缓存** | 零子进程、零 keychain、零网络 | Desktop 没开 / 快照 > 30 分钟（`desktopFreshness: TimeInterval = 30 * 60`，`:124`） |
| 2 | **`claude "/usage"`**（起子进程） | 一个子进程，5 分钟缓存一次（`cliRefreshInterval = 5 * 60`，`:122`） | Claude Code 没装 / 非零退出（读作「它没登录」，抛 `needsAuth`） |
| 3 | **keychain OAuth token + 端点** | keychain 读 + 打 `https://api.anthropic.com/api/oauth/usage?cedar_ember=1`（`:34`） | 无 token / token 过期 / 429 |

三条让它「不会不一致」的机制：

1. **一个出口造 snapshot**。`snapshot(windows:plan:resetCredits:)`（`:263-278`）是
   Claude snapshot 的唯一构造点，注释原话：「窗口顺序或 headline 在 endpoint 上改了，
   不能偷偷地和 CLI 的或 Desktop 的不一样——这三者是同一个读数取自三个地方」。
   CLI 侧还把窗口 label 统一成 endpoint 的词汇（`UsageResponse.label(forKind:)`），
   「一个源归档的读数在另一个源接手时仍然对得上」（`Sources/Providers/ClaudeUsageCLI.swift:297-301`）。
2. **Desktop 缓存只读一个组织的那一条 URL**。`ClaudeDesktopUsageCache` 的文件注释
   （`Sources/Providers/ClaudeDesktopUsageCache.swift:14-24`）：Claude Desktop 是 Chromium，
   它自己的 usage 面板画的那次 `GET /api/organizations/<id>/usage` 落在磁盘的 Simple Cache 里；
   「**不涉及 token、cookie、keychain、对 Anthropic 的请求、子进程，也没有任何写**」；
   「 Finding the entry means looking at its neighbours, and the *amount* looked at is the point:
   那个目录里其他每个文件都只读前几个字节——刚够拿到缓存的 URL，不再多。
   只有 URL 是这个账号 usage 端点的条目才解压 body」。
   body 是 `content-encoding: zstd`，macOS 无解码器，于是**vendored 了一份 decode-only 的 zstd**
   （`README.md:389-390`）。
3. **CLI 那一跳的工程细节比想象的脏**。`ClaudeUsageCLI.arguments` =
   `["--print", "--no-session-persistence", "--strict-mcp-config", "/usage"]`
   （`:54`），注释逐条解释：`--print` 避开 workspace-trust 对话框；
   `--no-session-persistence` 只存在于 print 模式、跳过 transcript；
   `--strict-mcp-config` 且不给值 = 「没有任何 MCP 服务器…否则每轮轮询都启动用户
   `~/.claude.json` 里配置的全部，繁忙机器上是一打 Node 进程连 GitHub、Cloudflare，
   没有一个 `/usage` 用得上」（`:41-49`）；**telemetry 故意不关**，因为
   per-model weekly 那行（`Current week (Fable)`）藏在 feature-gate 后面，
   关掉开关 `/usage` 就不打它了（`:50-52`）。
   还有两个自伤防护：跑在**一个固定 scratch 目录**（否则每轮在
   `<config>/projects/` 留一个再也不会访问的文件夹，一小时十二个）（`:57-63`）；
   自己 spawn 的 `/usage` 进程会被 `ClaudeSessionMonitor.ignoredPIDs` 与
   `ignoredWorkingDirectories` 双网拦掉，否则「它跑 busy、然后消失，
   completion watcher 会在每次轮询时把 usage-scratch-e1 finished 报成一个横幅」
   （`ClaudeUsageCLI.swift:93-99`、`ClaudeSessionMonitor.swift:33-44`）。

**它自己承认的边界**（`README.md:365-372`）：「没有任何厂商为这些工具发布一个干净的
『你的会话限额用了 N%』API。每个 adapter 读的是拥有它的 app 自己读的东西——
一个内部端点、一个本地数据库、一个语言服务器自己的 RPC——而这些可以不经通知就变。
每个 adapter 的响应形状被测试钉住，每个失败降级成一个可见状态（stale、needsAuth、error）
而不是一个编出来的数字。」

### 2.5 17 家 provider 的三档判据

`UsageProvider` 协议要求每个 adapter 声名自己的 **Fidelity**（`.official` / `.derived` / `.manual`），
「于是 UI 从不把一个猜测打扮成厂商发布的东西」（`README.md:348-350`）。
`UsageProvider` 的方法面很窄（`Sources/Providers/UsageProvider.swift`）：
`fetchSnapshot()` / `account()` / `signInRoute` / `signOut()` / `presentSignIn()` /
`presentAccountSwitch()` / `forgetCachedCredential()` / `isVisibleWhenAbsent`。
其中三条被刻意做成**协议要求而不是 extension 成员**，注释给了真实理由：
「一个只存在于 protocol extension 的方法是**静态分发**的，通过 `any UsageProvider`
调用它永远落在默认实现上、永不落在 override 上——这正是发生过的事，
它安静地把每个账号报成不存在」（`:18-24`）。

错误类型被拆得比我们细（`UsageProvider.swift:75-117`）：`needsAuth` / `accessDenied` /
`credentialExpired` / **`signedOutByOwner`** / `timedOut` / `badResponse(status:)` /
`apiError(String)` / `rateLimited(retryAfter:)` / **`nothingMetered(String)`**。
两条特别值得注意：

- `accessDenied` ≠ `needsAuth`：「凭据在，而 macOS 拒绝交出来——keychain 提示被点了 Deny。
  跟登出不是一回事：告诉一个人去重新登录，而他登着录只是按了 Deny，
  是让他去修一个没坏的东西。」
- `nothingMetered`：「账号可读，但真的没有配额在被计——Cursor 的免费方案报 included limit 为 0。
  这不是错误，也绝不能显示成错误。」

17 家的档位原文见 `README.md:74-140`。判读规则（README 自述）：

- **多数 provider「从机器上已有的工具借一个凭据或会话」**（`README.md:142`）——
  表里逐行数得清的**12 家**明确借（Claude Code / Cursor / Codex / GLM / Grok / OpenCode /
  Command Code / GitHub Copilot / Kimi / Kiro / Amp / Antigravity），这是它不需要用户重新登录的原因；
  另 3 个是显式登录例外（见下），2 个是本地运行时（`README.md:106-112`）。
- **三个显式登录例外**（`README.md:143-148`）：DeepSeek / MiniMax 是「在 Codenotch 自己的
  WKWebView 里显式登录」或「在 Settings 里贴一个 key」；QianwenAI「不发布 usage API
  也没有 key 可贴，所以那个 WKWebView 会话是唯一入口」。**「没有一个会打开浏览器的 cookie store。」**
- **本地运行时两档**（Ollama / LM Studio）走 `local runtime`，且被手机端显式排除
  （`PhoneLinkSnapshotBuilder` 的 `if snap.kind == .localRuntime { continue }`，`:41`）。
- **关掉一个 provider「停掉它的 usage 轮询并忘掉它的读数；借来的账号保持登着录，
  归拥有它们的工具管」**（`README.md:150-152`）。

### 2.6 多账号 / 多 profile 的发现机制

**Claude 侧**：`~/.claude-<slug>` 在启动时被扫出来，默认 `~/.claude` 永远第一、其余按字母序，
「于是两个环永不换位置」（`README.md:196-201`）。 **Codex 侧同理**：
`~/.codex` 是 Codex 环，每个用过的 `~/.codex-<slug>` 加一个环（`README.md:203-214`）。
**每个环的重启、排序、开关在 Settings 里独立**；「关掉一个只忘记 Codenotch 的读数，
Codex 登录态完好」（`README.md:215-217`）。它读每个 profile 的 `auth.json`，
「keychain-only 或 API-key-only 登录给不出这些 ChatGPT 账号限额」；
「Directories outside the `~/.codex-<slug>` convention are not discovered automatically」
（`README.md:219-224`）。

**这里有一个它自己踩到并显式处理的坑**：`claude /usage` 在 print 模式下
**给整机一个答案，不管 `CLAUDE_CONFIG_DIR` 指哪**（原注释：「verified: identical output,
requests and sessions included, for `~/.claude` and a second config directory」），
所以多登录时它会把两个环涂成同一个数字。它的判据是
`cliEstimateApplies(slug:loginCount:)` —— **只有默认登录且只有它一个时才用 CLI**
（`ClaudeOAuthProvider.swift:200-216`）。我们 Phase 2 只做 Codex 一家，撞不到这个；
但它示范了「借用一个全局 CLI 的输出」这件事在多账号下会悄悄说谎。

### 2.7 手机端：局域网 QR 配对，wire 级协议，但主仓当前关闭

README 的口径（`README.md:47-66`）：iOS + Android app 显示「同样的百分比、重置时间与会话状态」，
**「只读 notch 已经显示的东西——从不读 token、凭据或原始 API 响应」**；
QR + 五分钟倒计时，Mac 与手机必须同一 Wi-Fi，「server 只答本地网址、
拒绝任何走互联网路由的」；每个 code 单次可用、五分钟过期；移除设备「凭据立刻删除」。

wire 级细节在 `docs/phone-link-protocol.md`（7947 字符，protocol **v2**）：

- 配对链接 `codenotch://pair?v=2&h=<host1>,<host2>&p=<port>&c=<code>&n=<mac name>`
  （§1）：host 最多 4 个，**明确排除 utun/bridge/awdl/llw/loopback 地址**；
  port 默认 8788；code 是 16 随机字节 32 位 hex，**只活在内存里，从不落盘也不进日志**（§1 Code lifecycle）。
- 每个已认证请求：`X-CN-Timestamp` / `X-CN-Nonce` /
  `X-CN-Signature = hex(HMAC-SHA256(key, ts + "." + nonce + "." + METHOD + "." + path + "." + hex(SHA256(body))))`
  （§2），服务端检查顺序「私网 gate → 限流 → 头在 → |now−ts| ≤ 120s → nonce 未见 → 签名（常量时间比较）」。
- `deviceSecret = HMAC-SHA256(key=UTF8(code), "codenotch-device-v2:" + deviceId)`，
  **「device secret 从不传输，手机从它扫到的 code 推出同一个值」**（§3）。
- 错误语义齐全：`code-expired` / `bad-code` / `unknown-device` / `bad-signature` /
  `clock-skew` / `replayed-nonce` / `local-network-only`（§4）。
- 快照 JSON 只含 `usedFraction` / `label` / `resetsAt` / `plan` / 会话四态映射
  （busy→busy、waiting→waiting、success→idle、idle→idle）（§5）；端口 8788。

**v2 已经在仓里被自己的复盘推翻，v3 规格是「contract for implementation」**
（`PHONE-LINK-V3.md`，174 字符内读到三条自陈缺陷）：

1. 「listener binds `0.0.0.0` 且 `POST /api/v2/pair` 永久armed」（:11-12）；
2. 「Bodies cross the LAN in plaintext, and **responses are not authenticated at all** —
   anyone on the network can spoof a snapshot」（:13-14）；
3. 「Device secrets sit in `devices.json` in the clear」（:15）。

v3 的修法（`PHONE-LINK-V3.md`）：**AES-256-GCM + HKDF-SHA256 派生四把单用途密钥**
（`K_sig` / `K_enc` / `K_pair_sig` / `K_pair_enc`，「一把钥匙一个用途，
不要用同一把钥匙又签名又加密」，:45-68）；envelope = `base64(nonce12 || ct || tag16)`；
**AAD 把每个响应对到唯一一个请求**，`v3|res|<ts>|<nonce>|<METHOD>|<path>|<deviceId>|<status>`，
「这样捕获的响应不能被重放或拼到另一个上」（:101-116）；
encrypt-then-MAC 且常量时间比较（:134-152）；**配对端点只在人看着 QR 时可达**，
窗口外返回 `403 pairing-closed`，窗口在成功配对 / 用户关闭 / 5 分钟到点时关（:183-195）；
**bind 只绑 `PhoneLinkNetwork.getHosts()` 返回的私网 IPv4 加 `127.0.0.1`，
没有私网地址就不 bind**（:196-212）；设备密钥从 `devices.json` 移到 keychain
（service `com.codenotch.phonelink.device`，account = deviceId），
「`remove(deviceId:)` 必须也删 keychain 项。解除配对销毁密钥，不让它变成孤儿」（:224-240）。

**一个必须记下的现状事实**：`Sources/PhoneLink/PhoneLinkServer.swift:8-14` 有一个硬开关
`enum PhoneLink { static let isAvailable = false }`，注释原话：
「Off until a phone app people can actually install exists: without one the Phone pane and
"Connect Phone…" lead nowhere. While off, the server never listens, even for someone who
switched it on in a development build.」
→ **README 宣传的手机端，在主仓当前 main 上是关着的**（server、配对窗口、快照全部不监听）。
Preferences 里 `phoneLinkEnabled` 默认 `false`、`phoneLinkPort` 默认 8788（`Sources/Settings/Preferences.swift:46-51`、`:484-485`、`:756-757`）。

### 2.8 Windows 端：Rust/Tauri 2，与我们的 `app/` 同构

`windows/README.md`（15294 字符）：Rust + Tauri 2 / WebView2 重写，
「Same design language as the macOS original (inverse-rounded pill, colour-graded rings,
hover card with per-window bars)…**No code is copied from the Swift app**; the providers are
reimplemented from their documented behaviour and the wire formats」（:7-9）。
规模实测（git tree）：**Rust 31 个文件 519806 字节**（`main.rs` 92798、`codex.rs` 49191、
`usage.rs` 38542、`antigravity.rs` 35380、`activity.rs` 26897），
UI 4 个文件 236476 字节（`notch.html` 105802、`settings.html` 125469、`dropzones.html` 2927）。
额外一个 `windows/codenotch-hook/src/main.rs`（4439 字节）——「tiny helper Claude Code calls
to report session events」（`windows/README.md:219-222`），即 **Windows 侧用 Claude Code hooks
上报会话事件**（`hooks_install.rs` 4213 字节对应安装）。端口不在 `windows/` 的同构上，
但结构上它就是「Rust/Tauri 壳 + 巨型 HTML UI + per-provider Rust 适配器」——
**与我们的 `app/`（Tauri v2 + `ui/*.html` + `src/*.rs`）是同一个形状**，
而我们的 `app/` 至今构建不通（`docs/workbench/10-replan-2026-09-26.md` §2 阻塞一）。

两处它做到了我们计划里还没做的：Codex 配额恢复走**原生 `codex.exe` 的 app-server
`account/rateLimits/read`**，「owned process is hidden, limited to 20 seconds, and
terminated/reaped after the read; no inference or login command is sent」
（`windows/README.md:24-40`）；Codex 侧还「reads the latest eight non-archived paths from
`state_5.sqlite` using a read-only, WAL-aware connection (50 ms busy timeout). This finds
resumed threads without scanning every session file」（:44-46）——
**它对"怎么在几千个文件里低成本找到已 resume 的会话"给了具体做法。**

---

## 3. B. 正面对比（本文重点）

### 3.1 逐条对比

| 维度 | codenotch | AgentIsland | 它强在哪 | 我们强在哪 | 该不该跟进 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **形态** | 屏幕边缘黑 pill，26×210pt 静止态，四边可贴、⌥+拖、可折叠；**hover 卡 600 设计 px**，比我们的卡还宽 | 灵动岛微细条 6×140pt + 可展开卡（Swift `cardWidth` 330 / Tauri `372×520`），三级导航 | **静止态克制 + 展开态不省**是同一套几何滑出来的；`NotchPlacement` 的一维栈空间让四边共一份布局代码 | 三级导航、菜单栏 compact popover、侧边栏主形态规划 | **抄结构不抄尺寸**：一维 stack space + 唯一映射点照搬；尺寸不动（我们的微细条 6pt 宽是刻意选择，不在 12 篇案例库的 8–12pt 区间内） |
| **覆盖的 agent** | **17 家**：10 家纯 `official` + 3 家混合标注（Antigravity / MiniMax / Amp 都含「otherwise / derived」）+ 2 家 `derived` + 2 家 `local runtime`（README 表逐行统计）；其中 12 家明确「借机器上已有工具的登录态」 | 26 个 agent id（Swift registry），无 usage 接入 | 面宽一个数量级 | **我们不依赖 agent 配合**：进程表 + 会话日志尾，装没装 CLI 都有状态 | **不跟进面宽**。理由同 workbench 已定项（CC Switch 10 家已占据 provider 面）；我们连 Claude Code 都不做（用户不用，Phase 2 只做 Codex） |
| **状态语义** | **四态** busy/waiting/success/idle（`AgentSession.swift:14-19`），加一个 `queued` 排队数 | **五态** offline/idle/completed/working/attention（`Sources/AgentIslandCore/Models.swift:45-49`） | 它的 `queued` 我们没有；「cell 无会话就消失」比我们的空态更干净 | **we have `offline` 与 `attention(AgentAttentionRequest)`**：进程级生死 + 等待的具体内容；它 `idle` 无法区分「没跑」与「刚跑完」 | **跟进两条小的**：`queued`（本地运行时排队）与「无会话即隐藏」；`offline` 不动（那是我们的结构性优势） |
| **判定精度** | 会话登记目录 + transcript + `sysctl` pid 起时比对；`SessionFocus` 走进程树抬 app；**「terminal 会话保留自己的 waiting，桌面会话才读 transcript」** | 同类（进程表 + 日志尾 + CPU 差分 + 五类可观测信号） | Claude 侧这套（尤其 `ignoredPIDs` / `ignoredWorkingDirectories` 拦自己 spawn 的 `/usage` 进程、`deduplicated` 处理 resume 双记录）**比我们的会话尾读更细** | 不依赖 agent 主动 emit；多 agent 并行差分 | **跟进一条**：`ProcessLiveness` 的 pid 起时比对（我们是否只靠 pid 存在判定，**未核实**）；**「抬 app 而非猜 tab」的取舍直接采纳** |
| **配额 / token** | **只做配额**（各家的 limit window）。token 明细**只覆盖本地运行时**：`LocalTokenLedger` 从 Ollama/LM Studio 的日志加总 requests / in / out / reasoning / draft / accepted（`Sources/Model/LocalTokenLedger.swift`），「counts only. No prompt, reasoning or reply text ever enters this type」（:3-7） | **TokenUsageMonitor 读本机会话日志**算净消耗、模型分布、时间线、预测 | 配额维度它做得深（429 退避 60s→翻倍→15 分钟封顶且 deadline 持久化，`README.md:415-418`） | **token 明细是我们的主菜**；本地三档口径在 `StructuredTokenUsageIndex` | **不合并口径**。配额 ≠ token；我们是「花了多少」，它是「还剩多少」。要不要接配额见 §5（**这是唯一一个需要用户拍板的跟进项**） |
| **配置管理** | 只有「关掉一个 provider」与「拖拽排序」，「关掉只忘记读数，借来的登录态完好」（`README.md:150-152`、`:215-217`） | **CC Switch 式档位切换**（Phase 2，只做 Codex 一家） | 无 | **决定性差距**：它结构上给不出「档位卡片显示这个 agent 此刻跑哪个档、跑着什么会话、要不要重启才生效」 | **不跟进也无需防守**。这是 `10-replan` Phase 2 定位改写后我们唯一的空地 |
| **待办** | **无**（README 全文无 todo/task/backlog 概念） | ToDos（Phase 3） | 无 | 我们有 | **不跟进** |
| **手机端** | 局域网 QR + HMAC/AES-GCM wire 协议（v2 已实现、v3 已 spec）；**但 `PhoneLink.isAvailable = false`，主仓当前不监听** | SMTP / Webhook 互联网外发 | wire 级协议完整（含 v2→v3 的自我否证与测试向量） | 外发通道已被我们产品化（`RemoteNotifier` / `SMTPSocket` / `app/src-tauri/src/webhook.rs`） | **不跟进手机端**（我们的远程外发是另一条已走通的路）；**但 v3 的两条设计要吸收进我们未来的任何本地 HTTP 面**：①「没有私网地址就不 bind」；②「配对端点只在人看着 QR 时可达」 |
| **跨平台** | Swift/macOS 15+ universal binary + Rust/Tauri 2 Windows 端口（`windows/`）+ 手机双端 | Swift/macOS 本体 + Rust/Tauri `app/`（构建不通） | **Rust 端与 Tauri 壳是能跑的形状**（31 个 rs 文件 + 4 个 HTML） | Swift 本体成熟（148 版 / 548 个测试调用） | **值得看的是它的 `windows/` 目录组织**（provider 一文件、`activity.rs` 独立、hooks helper 独立 crate），不是它的技术选型 |
| **工程纪律** | **99 个测试文件 / Sources 与 Tests 字节比约 1:0.76**（Sources 199 swift 2020464 B vs Tests 99 swift 1542482 B）；CI 在 macos-26 跑 `make test-ci`；每 commit 出 dmg；18 天 16 个正式 release | 548 个 `test(...)` 调用（自建 runner，无 XCTest）；Rust 端 **0** 测试；148 个 release | **测试与实现在字节上同级**；「新行为有测试」写进 CONTRIBUTING；`make test-ci` 不签名、夹具不读真 keychain | release 数量与自动化提交/发版脚本 | **跟进一条**：把「新行为必须有测试」从口头变成 CI 门（我们的 runner 已有，缺 CI）；**Rust 测试基建仍是 `10-replan` 阻塞二，本文不改判** |
| **设计校准** | 每个常量从一张 Figma 帧反推：`Design.scale = 44/117`，`NotchLayout` 全部 `Design.px(n)`（`Sources/Notch/NotchLayout.swift:9-11`）；`NotchLayout holds every measurement, quoted from docs/design/frame-124-hover-tooltip.png`（`README.md:358-360`） | 无等价机制（设计规范散在 `docs/workbench/` 的提炼里，未校准到常量） | **「布局可对着设计帧直接核对」** | 我们有 reduce-motion 真源与色彩存量处置的显式裁决（它在 README 里零命中 reduce-motion） | **跟进一条**：侧边栏新建时给 `tokens.css` 的间距/圆角建同一类「锚点 + 比例」常量表 |

### 3.2 问题一：它的 notch 是否比我们的微细条更聚焦因此更好用？

**是，但它换来的是"只回答一个问题"。**

- 它静止态只答一件事：**每个 provider 还剩多少额度**。44pt 环 + 一个百分比 + 四档颜色，
  零学习成本。它甚至刻意让颜色只表达这一件事（`ActivitySummary` 的 working 取白色就是
  为了「不被误读成额度标尺」，`ActivitySummary.swift:62-70`）。
- 我们的微细条答的是另一件事：**agent 此刻在干什么**（呼吸灯 + 五态 + 活跃数）。
- **两者不构成优劣，构成的是"问题不同"**。真正该问的是：我们的微细条有没有把"agent 在干什么"
  这件事做到像它答额度那样**绝对可信、零歧义**？
  它有三级回退 + 一个 snapshot 出口 + `signedOutByOwner`/`nothingMetered` 这类精确失败态来保证
  「不许出现一个编出来的数字」。我们这边的对应物是五态状态机与 CPU 差分——
  **我们的"不可信"不在读数在判定**（`10-replan` 风险 4 记的正是这个：
  「同一个 agent，灵动岛说 working、侧边栏说 idle」）。
- **一个它做得比我们干净的具体点**：`ActivitySummary.init?` 在无会话时返回 nil，
  cell 直接消失（`ActivitySummary.swift:25`）。我们的空态如果还在渲染"都在摸鱼"之类的文案，
  那是同一条路上更差的一档。

**结论**：不搬家。但把「无会话即隐藏」和「失败态必须可命名」两条借过来。

### 3.3 问题二：372×520 可展开卡片 vs small black notch——两种形态哲学的取舍

**"小"不是它的哲学，"静态一瞥 / 按需展开"才是。**

- 它的 hover 卡 600 设计 px 宽、带指向被 hover cell 的尖尾、每个 limit window 一块进度条
  （`NotchLayout.swift:255-259` + spec 的 Hover state 图）——**比我们 330/372 的卡片更宽**。
- 它还有一整套「常驻可选」：Dock 图标 / 菜单栏项 / 两者皆无三选一，
  菜单栏项可显示「72% · 2h 18m | 41% · 4h 05m」这种压缩读数，full readings 永远在菜单里
  （`README.md:305-311`）。spec 的 v1 非目标明写「No menu bar item — the notch *is* the UI」，
  **它后来自己加了**（`Sources/App/StatusItemController.swift` 15210 字节）。
- 规格里连「hover 出、250ms 宽限再收」都写了，理由是「指针必须跨过卡与 cell 之间的空隙」。

**这反而是对我们双形态方案的正面支持**：同一个信息面，静态一瞥 / 悬停展开 / 菜单栏常驻
是**三种密度而不是三种产品**。我们 `shell_mode` 双形态并存的取舍，和它从 notch-only
走到「notch + 菜单栏 + 设置面板」是同一条路。
**差别在我们的侧边栏是键盘可达的主形态（`focus: true`、接键盘），而它的 notch 永远不接焦点**
（`NotchPanel` 的 `canBecomeKey = false`、`becomesKeyOnlyIfNeeded = true`，
`Sources/Notch/NotchPanel.swift:69-87`）。

**一个直接可抄的实现点**：`NotchPanel` 用 `NSPanel` 子类 + `sendEvent` 覆写来接右键菜单与点击，
注释解释了为什么不在 content view 上做——「`NSWindow.sendEvent` 先看到每个事件；
hosting view 的 hit test 会落到一个 SwiftUI 子 subview 上，那没有自己的菜单、
并且可能在事件到我们之前就消费掉它」（`NotchPanel.swift:8-13`）。
⌥+拖是 `trackOptionDrag()` 里 `nextEvent(matching:)` 的阻塞循环，
「从不 fall through 到 `onClick`：⌥+拖是一个独立的Gesture，不是一个长成了拖拽的点击」
（`:52-62`）。**对照我们的 `IslandPanelInteraction.swift`：如果我们把点击/拖拽
做在 SwiftUI 的 `onTapGesture` 上，就吃不到这套优先级。**

### 3.4 问题三：它 3 周 2493 star，我们 148 个版本——它做对了什么我们没做的

按证据排（不按重要性猜）：

1. **它卖的是一个恒等式，不是一个功能集。**「Claude's ring shows the same current session
   window Claude Code's own /usage leads with, so the two **never disagree**」
   （`README.md:7-9`）。这是一句可验证的承诺，且它真的为它建了三级回退 + 一个 snapshot 出口。
   我们的 README 卖的是「跨 agent 本机管理工作台」——**一个品类，不是一个承诺**。
2. **设计帧反推常量，而不是"看着调"。** `Design.scale = 44/117` 一个锚点定全局面
   （`Sources/DesignSystem/Design.swift:9-14`）。我们侧边栏准备直接写 `tokens.css` 字面量。
3. **每 commit 出可装预览构建**（`.github/workflows/package.yml` 的 `make dmg-ci` + preview release）
   + **18 天 16 个正式 release**（`v1.4.0` 2026-09-06T05:36Z → `v1.18.0` 2026-09-24T10:33Z，API 实测 16 个正式 tag + 1 个 `preview` 滚动 tag）。
   我们有 `release.sh` 一条命令，**但版本号是"下一号"而不是"按交付节奏"**。
4. **README 把"诚实的部分"写成一等章节**：`## The honest caveat`（`README.md:365`）。
   它主动说「没有任何厂商发布干净的配额 API」「每个 adapter 读的东西可以不经通知就变」。
   我们的 README 已知限制是被 AGENTS.md 要求「逐条对着代码核实」的债。
5. **单人作者的限流，公开写**（CONTRIBUTING.md，同 MonoCode）。
6. **148 个版本不自动等于发布纪律。** 我们的 `release.sh` 已把顺序钉死（扫描→commit→tag→release），
   这条我们比他强；**弱的是"每个 commit 有一个可装产物"**——他的 `package` workflow 在每次 push 重建 dmg。

---

## 4. C. 必须跟进的三件 / 明确不跟的两件

### 4.1 跟进

1. **`ProcessLiveness` 的 pid 起时比对**（`Sources/Sessions/ProcessLiveness.swift`）。
   最小改动、直击「进程崩了会话文件还在说 busy」这一类假阳性。
   **前置核实**：我们当前的会话活性判定是否只用 pid 存在（`Sources/AgentIslandCore/ProcessMonitor.swift`，
   本次未逐行读，见 §7）。若已是起时/启动时间比对，则本条降级为「补一条测试」。

2. **一维 stack space + 唯一映射点**（`Sources/Notch/NotchPlacement.swift`）。
   这是侧边栏壳最有价值的一处结构借鉴：所有布局在两坐标系里算，
   只有一处知道「哪条边」。对我们的直接价值是 `placement.rs` 与 `IslandPanelPositioning`
   不必再各写一遍四边分支，且侧边栏贴左/右可以复用同一套数学。**成本低，Phase 1 顺手做。**

3. **「无会话即隐藏 + 失败态必须可命名」两条 UI 纪律**（`ActivitySummary.swift:25`、
   `UsageProviderError` 的九分法，`UsageProvider.swift:75-117`）。
   前者一行；后者提醒我们：侧边栏若出现「未知/异常」这种笼统文案，
   应至少拆成 未安装 / 未登录 / 已登出被清 / 无配额可计 / 被限流 五类。

### 4.2 明确不跟

1. **不跟进 17 家 provider 的覆盖。** 理由与 `10-replan` §3.3 非目标一致（CC Switch 已占据
   provider 面），且用户不用 Claude Code。**它证明了"有路径"，不证明"我们该接"。**

2. **不跟进手机端 / 局域网 HTTP 面。** 我们的远程外发（SMTP/Webhook）是另一条已产品化的路，
   且当前需求里没有手机端。它的 v3 协议是好的，但为一个尚不存在的形态建一套 wire 协议是负收益。
   **只吸收两条设计前提到未来的任何本地 HTTP 面上**：无私网地址就不 bind；
   配对端点只在人看着 QR 时可达。

---

## 5. D. 需用户拍板的问题

### 5.1 唯一的实质冲突：官方直读配额路径

任务书要求显式提出。**把它拆成三级之后，冲突的范围比"要不要接"小得多：**

| 级 | 路径 | 与 `CONTEXT.md:196`「不碰网络、不转发流量、不持有密钥」的关系 |
| :--- | :--- | :--- |
| 1 | **读 Claude Desktop 的 HTTP 缓存文件**（`~/Library/Application Support/Claude` 下那条 `/api/organizations/<id>/usage`） | **不冲突**。README 与源码注释三处独立确认：无 token、无 cookie、无 keychain、无请求到 Anthropic、无子进程、无写（`README.md:380-384`、`ClaudeDesktopUsageCache.swift:14-24`） |
| 2 | **起 `claude --print --no-session-persistence --strict-mcp-config /usage`** | **冲突的一半**。我们不转发流量成立，但这是一个我们 spawn 的第三方进程在联网（README 实测只打 `api.anthropic.com` 与 feature-gate host，`ClaudeUsageCLI.swift:41-49`），且它会临时在 `~/.claude/sessions` 里落一个记录——它自己都要用 `ignoredPIDs` 双网拦掉 |
| 3 | **keychain 读 OAuth token，自己打 `https://api.anthropic.com/api/oauth/usage?cedar_ember=1`** | **正面冲突**。我们自己持有凭据并主动打厂商端点 |

**而对"只做 Codex 一家"的方案，真正相关的是第三级在 Codex 侧的对应物**：
它的 Windows 端读 `~/.codex/auth.json` 打 ChatGPT 端点，或起原生 `codex.exe`
走 app-server `account/rateLimits/read`（`windows/README.md:14-22`、`:24-46`），
「Auth is read only, never refreshed」（:26）。**这正是我们 Phase 2 档位卡最自然的那一格
（"这个账号 5 小时窗口还剩多少"），也正是与 `CONTEXT.md` 口径正面相撞的那一格。**

**附带一条必须一起摆上桌的事实，且它不止与 Phase 2 有关**：本仓的「不持有密钥」
这条口径**今天就已经有两处自相矛盾**（本次逐条核实）：

1. **远程外发本身已在持有凭据并在主动出网**。`CONTEXT.md:167` 明写「密钥（SendKey / token /
   webhook 里的 key / SMTP 授权码）只进 macOS 钥匙串……UserDefaults 里零密钥」——
   也就是说 SMTP 授权码、ntfy token、自定义 webhook key **本来就在钥匙串里**，
   远程外发本身就是主动打外网。（区别在：那三个凭据是**用户为这个功能显式填的**，
   不是从别的 CLI 借来的登录态。）
2. **Phase 2 的档位文件里会出现 API key**。replan §7 问题 3 已定「元数据明文 + 凭据进钥匙串」，
   B-10 已承认「档位文件里会出现 key」。

所以问题不是「要不要为接配额而破例」，而是**「凭据在本机的边界到底画在哪」**——
这条边界今天已经模糊，接配额只是把它推到台前。三个候选：

| | 边界 | 今天谁在这么做 |
| :--- | :--- | :--- |
| (a) | 只读已存在的登录态用于展示、绝不自己持钥打端点 | 无人 |
| (b) | 允许**借**机器上已有 CLI 的登录态（读 `auth.json` / `CLAUDE_CONFIG_DIR` / `CODEX_HOME`，只读） | 无人 |
| (c) | 允许自己持钥打厂商端点 | 远程外发（凭据是用户显式填的） |

codenotch 是 **(b) 为主、(c) 为兜底、完全不做 (a) 之外的写**。
本仓的现状是「用户显式填的凭据进钥匙串」+「Phase 2 档位凭据也进钥匙串」，
**本来就接近 (b)+(c) 的混合**，只是从未有一条口径把这个混合说清。

### 5.2 要问的四个问题（建议一次弹窗）

1. **Codex 配额接不接？** （推荐：**不接**——它不改变五态、不改变档位切换，
   而读 `auth.json` 打端点是本产品第一次主动出网，收益是一格百分比。）
2. 若接，走哪条？（推荐：**借 CLI 路径优先、端点兜底**，即 (b)；
   不推荐 (c)，那是产品性质变化。）
3. 「不持有密钥」这条口径要不要正式改写为「不持有**非本机既有登录态**的密钥」？
   （推荐：**改写并写进 ADR**，否则 Phase 2 与这条口径长期互相矛盾。）
4. 侧边栏要不要建「配额」这一格但**标为未接入 / 显示本地计算值**？
   （推荐：**不建空壳**，`10-replan` 一致——不给一个与 Swift 不同口径的数字。）

---

## 6. 未核实清单

1. **`TASKS.md`（102841 字节）未获取**（curl 超时两次）。它是它的 implementation history，
   可能有「哪一版为什么改」的证据；本文的版本节奏结论只来自 GitHub releases API（16 个正式 tag + 1 个 `preview` 滚动 tag 与时间）。
2. **`docs/phone-link-protocol.md` 是 v2 规格，仓里同时有 `PHONE-LINK-V3.md`（已读到全文）**。
   v3 何时落到 `docs/` 下、v2 与 v3 哪个是当前生效实现，**未核实**（`PhoneLink.isAvailable = false` 让这件事暂时不影响结论）。
3. **`PhoneLinkRequestHandler.swift`（17727 字节）未读**：v2/v3 的**实际**实现状态（是否仍是 v2、是否仍是 `devices.json` 明文）未逐行核对。本文对手机端的结论以规格文件 + `isAvailable=false` 为据。
4. **`Sources/Sessions/*ActivityMonitor.swift` 共 12 家只读了 Claude 一家**（`ClaudeSessionMonitor` 逐行）。Cursor / Codex / Grok / Kimi / Antigravity / LMStudio / Ollama 的 monitor 判定细节未读。
5. **我们的 `ProcessMonitor.swift` 是否已做 pid 起时比对**未核实（本次任务范围是 codenotch，且不改项目文件）。这决定 §4.1 第 1 条是「新增」还是「补测试」。
6. **我们的 `IslandPanelInteraction.swift` 是否已用 `sendEvent` 接管点击/拖拽优先级**未核实（同上）。
7. **codenotch 的 macOS 26 / Liquid Glass 依赖**：CI 注释说「the Liquid Glass APIs the settings panel uses」（`.github/workflows/ci.yml`），但具体用了哪些 API、是否用了 SwiftUI 新容器，未读 `SettingsView.swift`（116824 字节）。
8. **`Sources/Providers/Providers.swift`（catalog/registry 装配处）未读**：17 家如何注册、`Fidelity` 在哪里逐家声明，未逐行核对（只在 README 与 `UsageProvider` 注释见到三档口径）。
9. **`NotchViewModel.swift`（61788 字节）与 `NotchRootView.swift`（29847）未读**：动效与动画参数（是否有 spring 集中定义、是否读 reduce-motion）**未核实**。因此本文未对它的动效纪律下任何结论。
10. **`site/` 目录未读**（它有对外主页，与我们的 `site/` 同类）。
11. **`Makefile`（17733 字节）未读**：`make test-ci` / `make dmg-ci` 的具体命令未逐行核对，本文对 CI 的描述只到 workflow 的 `run:` 一行。
12. **open issues 标题与内容未逐条读**（只从 API 拿到计数 53）。它的已知限制清单因此**只有 README 自陈那几条**（Desktop 缓存格式可能变、Chromium cache private、`/usage` 的每周行藏在 telemetry 门后、keychain 每次轮转重建项），未含用户侧 bug。
