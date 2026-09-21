# 更新日志 (CHANGELOG)

所有关于 AgentIsland 的重要版本演进与功能更新均记录在此。

历史发布按时间统一编号为 0.0.1–0.0.53；对应关系见 [版本映射](docs/version-mapping.md)。

## [0.0.101] - 2026-09-21

### 🩹 修一个「按用户的真实用法根本不成立」的在场判定：补上无输入时长

用户反馈：日常是 Windows 远程桌面连着 Mac，但**人去做别的事**时希望收到提醒。
v0.0.100 的「只在人不在机器前时发送」只认两条信号——屏幕锁定、显示器睡眠。
在远程桌面连着的状态下这两条永远不会成立：会话不锁、屏幕不熄，人早就走了。
也就是说那个开关一打开，通知就再也不发；而它看起来是个正常开关。

- **实测确认**（就在这台机器上）：`CGSessionCopyCurrentDictionary` 的字典里 11 个键、
  **无 `CGSSessionScreenIsLocked`**，`CGDisplayIsAsleep = false`，而键盘已 771 秒没有事件。
  三条里只有「无输入时长」能分辨这个状态。
- **加第三条判据**：`PresenceSignals.idleSeconds`，由 `CGEventSource
  .secondsSinceLastEventType(.combinedSessionState)` 取键盘/左右键/移动/滚轮里最近的一次。
  阈值 `awayIdleSeconds` 默认 120 秒、钳在 30–3600（0 秒会让任何时刻都算离开，等于开关失效）。
  判定 `RemoteNotifyPolicy.isAway(_:)` 是 Core 里的纯函数，信号与判定分层：
  取信号那层碰窗口服务器、测不了，所以不掺判断。
- **fail-open 的方向和节流相反**：取不到输入时长时按「已离开」照常发。
  这一条若 fail-closed，症状是「开关看着开了、其实永远不发」——本仓最难查的那类失效。
- **开关效果必须看得见**：设置页实时显示当前判定与三条信号
  （「当前判定：已离开（距上次输入 214 秒；锁屏 否、显示器睡眠 否）」），
  被挡下时「最近外发」写「未发：有人在机器前（距上次输入 8 秒，未达 120 秒）」；
  取不到时长时页面直接说「这台机器上这条判据没有数据，只认锁屏与显示器睡眠」。
- **诚实标注测不到的部分**：`ScreenPresence.idleSeconds` 本身在测试里造不出来（需要真实
  GUI 会话）。变异验证里「让它永远返回 nil」这一项 0 红——那是真盲区，不假装覆盖；
  改为靠页面上的实时判定行让人当场能核对。判定逻辑与调用点传参都有断言
  （8 项变异里 7 项被捕获，含「退回只认锁屏/熄屏」「阈值不参与判定」「fail-open 反了」
  「被挡下不给依据」「调用点漏传 presence」等）。
- 集成实测：真信号 + 真外发路径走一遍，阈值 30/120/300 秒判为离开会发，
  调到 3600 秒则 `suppressed(reason: "有人在机器前（距上次输入 771 秒，未达 3600 秒）")`。

测试 378 → 383。

## [0.0.100] - 2026-09-21

### ✨️ 新增：可选的远程通知通道（ntfy / 自定义 HTTP（微信中转）/ 邮箱 SMTP）

解决的问题：用 Windows 远程桌面连着 Mac 时，任务完成只落在 Mac 屏幕上，人收不到。
功能默认关闭，开关在「设置 → 远程通知」。两轮深度复核 + 一次本地真实链路实测，
共抓出 9 个「离线测试全绿但功能其实不成立」的缺陷（见下）。

- **三个通道，只有 ntfy 做成预设**。ntfy 的请求形状按官方文档核实过（`POST https://<服务器>/<主题>`、
  `X-Title`、`X-Priority` 告警 4 / 普通 3，公开服务器单条 4096 字节上限）。
  Server酱 / PushPlus / 企微 / 钉钉的文档站当时是 JS 渲染、404 或超时，
  **没核实到的字段名一律不预置**：微信走「自定义 HTTP 模板」，且**请求体模板必填**
  （早先版本在模板留空时自带 `title`/`body`/`content` 一套字段名，那正是本仓说不预置却自己编了的行为）。
  邮箱走 SMTP 465：`Network.framework` 没有「在已建立的 TCP 上原地升级 TLS」的能力，
  STARTTLS(25/587) 做不了，所以在配置层就拦住并说明原因；代码里也不留半条 STARTTLS 分支。
- **凭据只进钥匙串**，条目名由通道唯一决定（`remote.ntfy` / `remote.customHTTP` / `remote.smtpEmail`），
  不做成可配字段——「密钥存了但条目名对不上」是界面全绿、每次发送都失败的失效。
  写入改成「先加，遇重复才替换」：原先先删后加，而 ad-hoc 签名下「加」可能被拒，
  结果是一条没存上、旧凭据也没了。掩码覆盖 `?token=`/`?key=`/`?access_token=`、
  `{key}` 占位、**主机名首段就是密钥**（判据：20+ 位字母数字，与大小写无关）、
  以及**路径段里的长令牌**——帮助文本就叫用户「把控制台地址整段粘进来」，
  不遮路径段等于把 SendKey 印在设置页与预览上。另给两条独立警告：
  明文 `http://` 端点、以及地址里没写 `{key}` 却含 `key=`/`.send`/高熵片段（会明文落盘）。
- **默认外发内容**只有「哪个 Agent + 什么状态」；命令内容、路径、消息原文要显式勾选。
  深链 `/notify` 与 HTTP `/notify`、`/event` **一律不外送**。这条闸门差点是假的：
  `LocalEventServer` 构造事件时漏传 `externallyDelivered`，于是本机任意进程都能
  往用户手机/邮箱灌任意文字，而外发路径没有 `shouldPeek` 那类分级可挡。
- **实测（本地假接收端 + 自签证书 TLS 服务器）与复核抓出的其余缺陷**：
  1. `X-Title` 的中文被 URLSession **静默丢弃**（对端只收到 `Qoder · `），本地却报 `.delivered`
     → 头值走 `WireText.headerValue` 百分号编码，并把 Agent 名同时写进正文首行；
  2. 头值允许 `& # % +`，同一套编码被当成查询/表单编码器用 → 勾选「附带动作」后
     正文里的 `git commit -m a && curl …` 会在接收端被切成两个字段 → 拆成 `queryValue`/`formValue`
     （RFC 3986 unreserved）与头值两套规则；JSON 正文改为自己做字符串转义（可逆性有断言）；
  3. 对端不可达时 `connect()` **永久挂住**：`NWConnection` 对被拒绝的连接转入 `.waiting` 自行重试，
     状态永远走不到 `.ready/.failed` → 独立 `DispatchWorkItem` 兜底 + `ResumeOnce` 保证只恢复一次；
  4. 同样形态还留在读路径（`deadline` 只在 receive 回调里比较，「TLS 通了但不回行」永不超时）
     与写路径（完全没有期限）→ 读写各自加独立定时器；`SMTPClient.deliver` 补 `defer { io.close() }`，
     否则每条失败路径都留下一个活的 TLS 连接与已解密的授权码；
  5. SMTP 的多行回复（`250-` / `354-`）只读一行会让后续每次读取全部错位 → `expect` 读到续行结束；
  6. 节流键用显示名：显示名可被外部投递路径随便填，换个名字就绕过节流 → 改按 `agentId`，
     并把「查窗口 + 登记」合进一次加锁（原先两个锁区间之间可被并发穿透，等于两倍的窗口）；
  7. 用户填 `ntfy.mine.local/island`（少贴协议头）会被拼成 `https://ntfy.sh/ntfy.mine.local/island`
     ——内容跑到一台他没选过的公网服务器、主题名还是他的内网主机名 → 配置层拒绝；
  8. `deliver` 用 `Task { @MainActor in … }`：await 之后整条链留在主线程，且每个事件都在主线程
     解码三份 JSON、读一次钥匙串 → 总开关排到最前、配齐检查排在起 Task 之前、任务不标 `@MainActor`；
  9. 「真实 socket 连不上要如实返回」这一条只有假 IO 测过 → 新增走真 `NWConnection` 的用例；
  10. 为拦重定向给 `URLSession` 加了 delegate，却沿用 completionHandler 版 `dataTask(with:)`
      ——实测每次都是 `NSURLError -999` 且请求根本没出门，**而离线测试全绿**
      （它们注入的是假传输，从不经过 `HTTPTransport`）。改用 async 的 `data(for:)`，
      并加一条钉住这个组合的结构断言（变异验证：把 `dataTask(with:)` 加回去即变红）。
- **口径与可见性**：「发送测试」只绕过「什么时候打扰用户」三条（节流/静默/在场），
  总开关与事件类型开关照旧管用；`claim/release` 让失败不占用节流窗口；配置检查排在节流之前。
  预览按钮改名「生成预览（不送出）」，测试按钮写明「真的送出」，
  并说清「已送达＝对方接受了这条请求，不等于已推送到手机」（多数中转服务内部失败也回 200）。
  「最近外发」每 5 秒刷新，否则用户得动一下字段才看得到新记录。
- **可选的「只在人不在机器前时发送」**：信号取 `CGSessionCopyCurrentDictionary` 的
  `CGSSessionScreenIsLocked`（实测未锁屏时该键不存在）与 `CGDisplayIsAsleep`，默认关闭，
  被这条挡下时写「未发：有人在机器前」。**坑要说破**：远程桌面连着 Mac 时会话既未锁定也未熄屏，
  开着这条恰好会把自己要的通知挡掉。

### 🧪 测试

远程通知共 32 个用例（测试 346 → 378）：SMTP 状态机（465 命令序列、认证失败带回复码、
多行回复、无响应超时、dot-stuffing、RFC 2047 主题、RFC 822 日期、失败路径也关连接、
真 socket 的超时收敛）、ntfy 与自定义通道的请求形状与配置层拦截、三套线格式编码
（头值 / 查询串 / JSON）、策略矩阵（总开关 / 分类 / 节流按 id 分键 / 跨零点静默 / 在场判定 /
归一化）、预览与掩码四形态、落盘读盘与「加字段不清空旧配置」、
以及两条结构断言：外发前有闸门、外部入口造事件时当场打标记。
两轮变异验证合计 53 项，其中 5 项第一次跑「没让任何用例变红」——那是**测试真的没覆盖**，
补法分别是：多行回复、失败关连接、读盘端口收口、`/notify` 的构造侧标记、测试按钮的类型开关。
STARTTLS 的实现与用例一并删掉：那条路径在生产里永远走不到（非 465 在 socket 层就返回 false），
留一个能跑绿的测试只会让人以为这个客户端支持 587。
最后一轮真实链路实测（本地假接收端收三种通道的请求体并逐字段解析）确认：ntfy 的中文标题以
ASCII 百分号编码到达、表单里的 `&&` 与 `#` 没有多切出字段、JSON 正文可被对方 `json.loads` 解析。

## [0.0.99] - 2026-09-20

### 🖥️ 新增 Qoder 监控（状态 / 动作 / 等待确认），并实测确认它的用量无法监控

- **档案注册**：bundle id `com.qoder.app`、进程名 `qoder`（主进程与 `Qoder Helper (Renderer)`
  走前缀族规则一并命中）、会话根 `~/.qoder/projects`。`pathContains` **锚定到
  `/applications/qoder.app`** 而非裸子串 `qoder`——本仓刚把「过宽匹配可能误杀用户自己的
  Electron 程序」记为已知风险，新档案不再制造一例。
- **专用方言 `.qoderTranscript`**，而不是交给通用尾窗关键词扫描。Qoder 的逐行记录与
  Anthropic 同形（`message.role/content/stop_reason/usage`，工具名 `Bash`/`Edit`/
  `AskUserQuestion`/`Agent`），「等待用户」不是一个字段而是结构事实：
  **`AskUserQuestion` 这个 tool_use 还没有对应的 tool_result**。只看关键字会在
  「刚答完那一拍」反向误报（本仓在 Claude 上踩过同形），因此单独实现。
- **实测跑通**（Qoder 正在执行本会话时）：
  `🟢 工作中 ️ Qoder 514 0.0% 984M 3 — — 运行: cd /Users/…/workspace/Agent… 20s前`，
  `doctor` 给出「结论可信：本轮读到了会话强语义」。
- **Token 消耗：确认无法监控，并且没有假装能。** 本机 1,033 条 `usage` 记录求和：
  `input_tokens / output_tokens / cache_read_input_tokens / cache_creation_input_tokens`
  **全为 0**，唯一有值的是 `credits`（合计 303.37）与 `context_usage_ratio`；
  `requestTokenAnchor` 的两个字段是 64 字符不透明串，logs 与 `.models` 里也没有计数。
  试过接入采集：`agentisland tokens` 冷跑从 ~2.05s 涨到 ~2.9–4.0s（单个会话文件可达 10MB），
  换回 0 条数据 → **撤掉**，用量列对 Qoder 显示 `—`（没取到）而不是 `0`（没有），
  `doctor` 归入「未接入本地明细源」并说明「不代表它没在工作」。
- 新增 `docs/research/qoder-monitoring.md`：把「字段存在」与「数据存在」的区别、
  以及要做 credits 需要先定的三件事（单位、汇率来源、增量还是快照口径）写清楚。
- 新增 `docs/research/remote-notifications.md`：跨平台通知调研。核心结论——
  macOS 通知是本机另一个进程画的 UI，远程桌面只传像素，**在 Mac 侧无法穿透**，只能外发；
  通道按「要不要自己养服务器」分类对比（ntfy / 企业微信 / 钉钉 / 飞书 / Server酱 / 邮箱），
  并说明为什么邮箱排在后面（Foundation 无 SMTP 客户端，`sendmail` 无法判断是否真发出，
  违反本仓的诚实原则）。三条不可谈判的约束：凭据只进钥匙串（webhook URL 本身就是密钥）、
  默认只发「Agent 名 + 状态 + 时长」且发送前可预览、失败必须在岛内可见。

测试 340 → 346（Qoder 方言 6 条，其中「提问已回答不得再报等待」与「命令已返回不得仍显示运行中」
两条做过变异验证：让解析器忽略已回答集合，两条同时变红）。

## [0.0.98] - 2026-09-20

### 🧹 清理复核不再谎报「已处置」

v0.0.97 的 CHANGELOG 里留了一条「复核为真但未修」：**忽略 SIGTERM 的死锁进程会被报成清理完成**。
这一轮把它修掉，因为它骗的是用户做决策依据的那一行字。

- **成因**：工作台的清理复核拿的是「条目是否还在异常列表里」。而 `cleanAnomalies` 会
  `resetTracking(for:)` 清掉 hung 证据（`highCpuSince` 归零），1.2s 后重扫时该进程
  不再满足 hung 条件 → 条目消失 → 复核判定成功。**列表空了 ≠ 进程死了**，
  真死锁（仍在跑、拒绝 SIGTERM）恰好是最会命中这条的路径。单条清理与批量清理同一个口径，两处都改。
- **改法**：新增 `ProcessTerminator.isAlive(pid:expectedPath:)`，按 `kill(pid, 0)` 探活，
  并在给出预期路径时校验可执行文件名——pid 被系统复用给别的程序时不能算「目标仍存活」；
  探到活但取不到路径（权限/正在退出）时**保守当作存活**，宁可报失败也不谎报成功。
- 复核口径改为「原目标 pid 是否还活着」，失败提示也从「已保留在列表中」改成说明是探活失败、
  可能需要更高权限、可从活动监视器处理。
- 新增 5 条断言（自己的 pid 存活、pid 0/-1 不算目标、路径不符即身份不符、
  已 `waitUntilExit` 的子进程不得仍判存活）。把 `isAlive` 改成恒返回 false（即退回旧口径），
  第一条断言立刻变红。

### 仍未修的（复核为真，需要设计决定或你的目视确认）
- **杀进程目标过宽**：`pathContains` 是裸子串（`["trae"]`、`["zcode"]`、`["windsurf"]`…），
  进程名先命中 `Electron` 再路径含 `trae` 就会被认成 TRAE——例如用户在 `~/code/trae-sandbox`
  里跑 `npx electron .`。收紧匹配会改变识别率、且只能在你这台机器上验证，故未动；
  本轮的探活复核至少保证「误认的进程被点了清理后，结果如实回报」。
- `clean --force` 仍绕过「孤儿只准逐条手动确认」；`customAgents` 损坏存档会被下一次增删整档覆写；
  自定义档案数值/身份字段零校验；`dailyTokenBudget` 无区间；CLI `check` 与 `clean` 注册表不一致；
  子命令不识别 `--help` 与未知 flag。

## [0.0.97] - 2026-09-20

### 🚪 第六轮：改审四个从未被碰过的入口（深链 / 杀进程 / 设置持久化 / CLI）

前五轮都在引擎、解析、UI、测试上打转。这一轮换角度：把**外部输入面**与**破坏性操作面**
交给四个并行 agent。回报 24 条，复核确认为真并已修的 14 条如下——其中 6 条是
**用已发布二进制实测复现**的，不是静态推断。

**实测复现并修掉的 CLI 契约缺陷**
- `agentisland tokens --budget 1e308m` → **SIGTRAP(133)，无任何诊断**（`Int(无限)` 是运行时 trap）。
  改为可失败解析 + stderr + `exit 2`。实测修复后：`无效的预算值: 1e308m（需要非负数字，可带 k/m 后缀）`。
- `agentisland doctor --json | jq` → **jq 解析失败**：进度提示只看 `--quiet` 不看 `--json`，
  把 `⏳ 采集 CPU 基线…` 印进了 stdout。实测修复后 stdout 首行即 `[`。
- `agentisland report -o /不存在的目录/x.md` → 打印「导出报表失败」却 **exit 0**，CI 会认为报表已生成。
  新增 `CLIExit`（1=执行失败，2=用法错误），report / notify / open / clean 的失败分支一律非零退出。
- `agentisland notify` 缺消息 → 印红字后 exit 0；`--port abc` → 静默回落 41999；
  URL Scheme 回退路径 `try? proc.run()` 后**无条件印 success:true**（open 起不来也报已投递）。
  现在都按事实回报（含 `terminationStatus`）。
- `agentisland top` 每行印 `24h Tokens: 0`，而同一台机器 `report` 是 **1.61M**——这正是 v0.0.90
  在 `status` 修过的「把没取数印成 0」，`top` 自造一套引擎、从不 start 轮询。改为走 `LiveSampler`
  （含启停集与用量），nil 列印 `—`。实测修复后 `top` 显示 1.61M。
- `report --json` 实测输出的是 Markdown（help 里 `--json` 被宣称为全局写法）。现认成 `--format json`。
- `clean --json` 把**所有候选 pid** 报成已杀、`success` 恒 true，与 `terminatedCount` 无关；
  `CleanResult` 现在带 `terminatedPids`，一个都没杀掉时如实失败退出。

**深链（`agentisland://`）——任何本地进程都够得着的入口**
- 伪造「等待你确认」：`open "agentisland://notify?agent=claude&type=attention&message=..."`
  产生的横幅与系统通知，和引擎自己判定的真实待确认**逐字节相同**。npm postinstall / `.command` /
  cron 触发无需任何权限提示。现在事件带 `externallyDelivered` 来源标记，横幅角标与通知标题都会
  显示「外部投递 · 」，外部事件也无法冒充未知 Agent（投递目标必须解析到已知档案）。
- 循环发 `type=costspike` 的链接曾可**无限压制真实告警**：外部事件也登记 30s 保护期并抢占单槽横幅。
  现在外部投递不参与保护期。
- `agentisland://export` 会 `clearContents()` 静默销毁用户正准备粘贴的内容（密码/命令）。
  深链改为只导航到工作台；自动化请走用户显式执行的 `agentisland report --copy`。
- 审计报告 Markdown 表格只转义了 `summaryText` 的 `|`，`agentName` 与**换行**都不处理
  （`queryItems` 会把 `%0A` 解成真实换行）→ 攻击者可往用户粘进工单的报告里自造行。
  新增 `AuditReportExporter.cell()`，三处表格行统一转义。
- 解析本身此前**零覆盖**（它在 `@MainActor` 的 UI 目标里，runner 够不着）。抽出
  `AgentIslandCore.URLSchemeParser`，补 15 条解析断言（别名 / host 与 path 两形态 /
  查询键大小写 / 重复键 first-wins / percent 解码 / 非本 scheme 拒绝）。
- 文档纠正：`agentisland://clean` 从来只是跳到工作台，README 与代码注释却写着
  「一键静默清理孤儿与异常进程」。

**设置持久化**
- **预算预警对从未碰过设置页的用户永久失效**：`budgetAlertEnabled` 的 UI 默认是 true，
  引擎却用 `UserDefaults.bool(forKey:)` 读（缺键给 false）。新增 `SettingBool.read(_:default:)`
  唯一读法，并修掉同形的完成音开关（`SoundEffectsManager` 用 `?? true`、`IslandView` 用 `bool()`，
  同一个键两种真相）。
- 新增一条**不写死键名**的结构断言：凡 `@AppStorage` 默认 true 的 `SettingKey`，任何地方都不得
  再用 `bool(forKey:)` 裸读。把引擎改回旧写法，断言立刻指到 `ActivityEngine.swift:904`。

测试 338 → 339（新增深链解析、来源标记、报告转义、设置口径四组，全部做过变异验证）。

### 这一轮明确没做的（复核确认为真，但需要设计决定或目视确认）
- **杀进程目标过宽**：`pathContains` 用全路径子串匹配，用户在 `~/code/trae-sandbox` 里
  `npx electron .` 会被认成 TRAE；`terminateAgent` 的「身份复核」拿同一 pid 自己比自己，
  不构成独立校验。改法要重定匹配口径，会影响识别率，留待单独一轮。
- **清理复核恒报成功**：`cleanAnomalies` 先 `resetTracking` 把 hung 证据清零，1.2s 后重扫
  时条目消失即被判「已死」，真死锁（忽略 SIGTERM）也会被报成已处置。
- `clean --force` 仍绕过「孤儿只准逐条手动确认」的闸门；`top` 的 `[c]` 已改为先列目标再按 y 确认。
- `customAgents` 损坏存档会被下一次增删整档覆写（`enabledAgents` 有只读降级保护，这边没有）；
  自定义档案的数值/身份字段零校验（Infinity、空 id、重复 id、无条数上限）；
  `dailyTokenBudget` 是唯一没有区间的数值设置；`resolvedEnabled` 在 CLI 侧带写副作用（跨进程丢更新）。
- `check` 用完整注册表而 `clean` 只用内置表，于是 check 列出并承诺「clean 可一键终止」的目标，
  clean 看不见。

## [0.0.96] - 2026-09-20

### ♿ 第五轮：浅色可读性、VoiceOver 可操作性、视图重算，与一批「能失败的」新测试

三个并行 agent 分别做可访问性盘点、视图重算复核、测试盲区复核。这一轮的产出 mostly 不可见
——但岛内小字号在浅色下读不清、以及 VoiceOver 用户「听得到按钮却按不动」，都是真实缺陷。

**浅色对比度（实算 sRGB 相对亮度，非目测）**
- 离线态文字压在浅底上原本只有 **2.34:1**（连 3:1 的图形线都不到），待机 4.34、青色系强调字 3.51、
  琥珀强调字 4.25、`inkMuted48` 压在 chipFill 上 4.38 —— 全部低于 WCAG AA 正文 4.5:1，字号 8–10pt。
- 因为上一轮把色板收进了 `Theme.Ramp` 一处，这次是**改 6 行**而不是改 45 处：
  离线 `slate400→slate500` 且底色退浅一档 `slate100→slate50`（4.55）、待机 `slate500→slate600`（6.92）、
  青色浅半边 `0x0284c7→sky800`（6.42）、琥珀浅半边 `amber700→amber900`（7.73）、
  `inkMuted48`/`onDarkFaint` 浅半边压到 slate600（6.98）。深色外观一字未动。

**VoiceOver 可操作**
- 6 处 `.onTapGesture` + `.accessibilityAddTraits(.isButton)` 的元素**没有 AXPress**——
  `isButton` 只改语义不装动作，VO 用户听得到「按钮」却按不动。逐个补 `.accessibilityAction`：
  收起态细条（岛的第一入口）、列表行、模型行、会话行、流水条目、菜单栏快捷行。
  点按体抽成 `openDetail()` / `openSessionDirectory()` / `toggleExpanded(_:)`，
  手势与无障碍动作共用一处，不会两条入口各写一遍再漂移。
- 列表行/模型行/会话行加 `.accessibilityElement(children: .ignore)`：显式标签已把状态说全，
  不再让 VO 逐字播报 token/内存/点阵子元素。
- 4 个只有 `.help` 的图标按钮补 `.accessibilityLabel`（help 不是标签，VO 读不到）；
  3 个选中态只靠底色/字重表达的控件补 `.accessibilityValue("已选择"/"未选择")`。

**视图重算（复核后只改真正值得的）**
- `AgentRowView` / `AgentHoverTooltip` 把 `@ObservedObject var engine` 降为 `let`：
  两者 body 内**一处都不读** engine 的 @Published 状态（只调 terminateAgent）。
  复核同时纠正了审计的成本判断——父视图本就观察 engine 且无 `.equatable()`，
  所以这一改动省下的是「多一个订阅者」而非「每拍 N 次重渲染」，属卫生而非性能悬崖。
- 流水页：`filteredEvents` 计算属性在 body 里被读 4 次 → hoist 成一次；
  筛选条数从「每个 chip 各 filter+count 一遍（.errors 那支还会对每条事件跑模式分析）」
  改为刷新回调里一次算齐存 `countsByFilter`。
- 审计报的第三项（`expandedCard` 12 次 `visibleSnapshots`）经复核为微秒级，按本仓规矩不动。

**第三方 JSON 类型漂移容错**
- 新增 `SafeNumber.jsonInt`：`"step_index": "3"` 或 `3.0` 这类改版，`as? Int` 会静默给 nil，
  于是 `?? 0` 把 Antigravity 每行指纹压成 `step-0`（进度判定失效）。12 个站点统一改用它；
  越界仍走饱和钳制。
- 顺带把 ISO 时间戳插值也过一遍 `.escaped`（对它是恒等操作），换来一条**不需要例外清单**的
  结构规则：字符串插值进 SQL 引号必须过 `.escaped`。

**测试盲区（328 → 335，每条都做过变异验证）**
- `ReadonlyDB`「连接已缓存、文件随后被删 → 上报 `.missing`」此前零覆盖（只测了从未打开那一支）。
- SQL 转义此前只有纯函数用例：现在把含 `'` 的 modelId 灌进真实查询，去掉 `.escaped` 立刻变红。
- `Verdict.summary` 五条文案逐字锁定（对调两支返回串此前全绿）；`Code` 加 `CaseIterable`，
  新增码不补文案就会因数量断言失败。
- 两处恒过断言换成可失败：`usage["dim"] != nil || usage.isEmpty`（两支皆真）改为断言夹具期望值；
  可观测性夹具的 24h 与累计此前被构造成恒等，读错字段测不出——现在两者不同值并断言走 `tokensTotal`。
- 三条结构棘轮：`@ObservedObject var engine` 计数、流水页 `events.filter {` 计数、SQL 引号插值必须转义。
- **如实记录两处「没写测试」**：① 审计断言「parsedMeta 的 step_index 漂移会让已答复提问误报
  attention」，把该站点单独退回旧写法后整套测试仍全绿，因果不成立，故不为其写用例；
  ② `withDB` 重试「成功」分支需要「同 inode 且 prepare 瞬时失败」的夹具，构造不出来，
  仍属未覆盖路径（已覆盖的是两次都失败那一支）。

## [0.0.95] - 2026-09-20

### 🔍 第四轮复核抓出上一轮修复引入的性能悬崖，并把它变成可测语义

v0.0.94 的「增量解析只承认量到的那段字节」是对的方向，但收口收得太粗：**收敛检查**
（再派一个 agent 只审这一轮的 diff）指出，段尾若截在半行上，`endedWithNewline` 会变成
`false`，下一轮 `canAppend` 不成立 → `offset` 归零 → **整文件重解析**。对正在被追加的
日志这几乎每轮都发生——也就是说，为了修一个潜伏的重复计数，引入了一个更常触发的性能悬崖，
同时那半行还会被当成完整行解析出来（重复计数的另一种形态）。

- 被承认的段回退到**最后一个完整行**，回退上限与解析器「跳过 >1MB 巨行」的口径一致；
  `consumedThrough` 按回退后的位置写进戳。
- `data.subdata(in:)` 换回 `dropFirst/prefix` 切片：前者会把 mmap 段整体搬到堆上，
  正好抵消 `.mappedIfSafe` 的意图。
- 新增可测语义并做变异验证：半行不计入 → 补完整行恰好 +1 → 连续三轮采样数字稳定。
  把回退去掉跑测试，聚合立刻从 `n=1 / 50 tokens` 变成 `n=2 / 100 tokens`，正是那类重复计数。
- 顺带如实化两处注释：`openReadonly` 的代际校验只覆盖「打开期间发生 stop()」，
  完全发生在 stop 之后的查询仍会写回缓存（当前唯一 stop 点是应用退出，fd 随进程回收）；
  `LogTailReader` 开头「其他环节的毫秒级陈旧无影响」已被同文件改用 stat 的事实推翻。

测试 327 → 328。实机核对：`agentisland tokens` 与 v0.0.94 逐行一致。

### 关于收敛
四轮下来的信号很清楚：第一轮挖出仓库里的存量缺陷（崩溃、注入、锁竞争、死代码），
第二、三轮挖出的**几乎全是前一轮修复自身的问题**（`defer` 的注册位置、恒等的代际比较、
缓存负结果丢掉首会话、AppleScript 非法转义、这次的段尾截断）。存量面已接近见底，
剩下的都是有明确代价权衡的项（浅色对比度数值、可访问性命中区、索引按窗口裁剪、
notify 鉴权，均已连同实测数字写进 README「已知限制」），不再是「改一处少一处」的缺陷。

## [0.0.94] - 2026-09-20

### 🔧 第三轮：收口审计报回的 4 处潜伏缺陷，并修掉自己修复里的 2 处

第二轮复核派去审我自己的 diff，回报 5 条，其中 2 条是**这一轮修复本身**的缺陷——
`defer` 的位置决定了它到底覆盖不覆盖目标路径，这类细节最容易在「改了就是修了」的错觉里溜过去。

- **一次性 SQLite 连接的结算 `defer` 放晚了**：Swift 的 `defer` 只对注册点之后的控制流生效，
  我把它放在 `prepare` 之后，于是它恰好漏掉自己注释里点名要覆盖的「prepare 失败」那条 return。
  同时它的注册顺序晚于 `sqlite3_finalize`，逆序执行变成**先 close 后 finalize**——未 finalize 的
  连接上 `sqlite3_close` 只返回 `SQLITE_BUSY` 并把连接留下，而数组已清空，等于永不重试。
  上移到 `db` 定妥之后、`prepare` 之前，一并换 `sqlite3_close_v2` 兜底。
- **代际校验此前恒等成立**：`openReadonly` 在 `sqlite3_open_v2` **之后**才读 `dbGeneration`
  并与另一次读取比较，「stop() 发生在打开期间」这一种判不出来，迟到新建的连接会被写进缓存，
  而 `closeConnectionsAsync` 早已跑完，从此无人再关。改为打开前取基准。
- **增量解析只承认「量到的那段字节」**：`stat` 与 `mmap` 之间第三方仍可继续追加。此前按读到的
  实际长度解析、却把较小的旧 size 记进缓存，下一轮 offset 落在已解析过的字节上重复计数
  （无 `eventId` 的行其去重键含 offset，恰随之外移而失效）。修完膨胀后，第二轮复核指出**对称的
  漏算**仍在（原地截断时缓存 size 虚高）→ 改为回吐 `consumedThrough` 并写入戳，两侧一起收口。
  实机核对：改动前后 `agentisland tokens` 同一时刻逐行一致（`dim` 的 1.55M→1.26M 是 24h 窗口
  自然滑出，同一个旧二进制隔 20 分钟自己就变了，不是代码差异）。
- **`newestFile` 的 mtime 改 `stat(2)` 现取**：`URL.resourceValues` 有毫秒级陈旧窗口，而该函数的
  职责恰恰是「找出刚被写的那个文件」，缓存窗口会让它系统性漏掉最新一次写入。
- **预算耗尽不再静默，也不造成日志风暴**：条目数超预算时枚举顺序并非时间序，返回值只能算
  「部分枚举内最新」，现按目录去重告警一次（每拍对每个目录各调一次，不去重就是刷屏）。

测试 327 通过 / 0 失败，门禁 `GATE_EXIT=0`。

## [0.0.93] - 2026-09-20

### 🔒 五路并行深审两轮：修掉 2 处崩溃/UB、2 处命令注入、1 处局域网可伪造事件

五个审计 agent 并行覆盖并发、资源、测试有效性、外部输入、视图与可访问性；结论一律
**逐条复核后才动手**（本仓的审计有过假阳性，如「无锁字典」其实有 NSLock）。第二轮专门
派一个 agent 复核我自己这一轮的改动，它抓出 4 处我引入或改漏的问题（见末段）。

**崩溃与未定义行为**
- **`sqlite3_bind_text` 传了 `nil` 析构器（= SQLITE_STATIC）**：Swift `String` 桥出的 C 缓冲区
  只在 bind 那一行有效，而 `step` 在下一行——OpenCode 动作探测每 2s 在主线程读一次可能已回收
  的内存（轻则把别的字符串当 session_id 查错会话、动作文案串味，重则崩）。析构器常量收进
  `ReadonlyDB.transientDestructor` 单一来源（本仓另一处用法早就是对的，两处各写一份才漏了这一处）。
- **Cline 的 `Int64(ts)` 对外部 `Double` 直接转换**：`ui_messages.json` 里一条 `1e30` / `-9.2e18`
  哨兵就是运行时 trap，且发生在 `@MainActor` 采样拍上——岛直接消失，2s 后必复现。改走 `SafeNumber`
  饱和钳制。把修复改回去跑测试，整条 runner 当场 `Fatal error` 而死，这是最硬的一种验证。

**命令注入（两处，输入都是第三方库里的会话目录）**
- `RecentSessionNavigator` 的 `do script "cd …"` 与详情页「打开终端」的 `do shell script`
  都**只转义双引号**：`;` `|` `&&` `$()` 反引号照原样进 shell——点一次即在用户终端执行任意命令。
  新增 `ShellQuoting`（POSIX 单引号词 + AppleScript 字面量两层，顺序固定），并把 `cd` 加上 `--`
  终止选项。顺带修好一个真实故障：含空格的目录此前连 `cd` 都会失败。
- 新增结构断言：任何 `do script` / `do shell script` 出口未经 `ShellQuoting` 即测试失败。

**局域网可伪造岛上事件**
- `LocalEventServer` 注释写「仅绑定 127.0.0.1」，但 `NWParameters.tcp` 不设 `requiredLocalEndpoint`
  实际监听 `*:41999`（`lsof` 实测 IPv6 双栈通配），而 `/notify` 无鉴权——同局域网任意主机都能
  往岛上写「任务完成 / 需要你确认」。现在 macOS 14+ 限定回环监听，低版本显式告警而不是假装安全。
  改前 `lsof` 实测 `IPv6 TCP *:41999 (LISTEN)`，改后 `IPv4 TCP 127.0.0.1:41999 (LISTEN)`，且 `curl POST /notify` 仍返回 `{"success":true,…}`。
  （第一版把 `requiredLocalEndpoint` 与 `on: port` 同时传给 `NWListener`，实测构造直接失败、端点整个不监听——curl 空应答才发现，端口只能由 `requiredLocalEndpoint` 提供。）
- 同一处的三个资源缺陷：只连不发的客户端既不续收也不取消（连接与其完成块互相持有）、无并发上限、
  无空闲超时。改为活连接表（计数一旦漏减就会把好端端的 notify 关门）+ 5s 回收 + 上限 16。

**功能失效与主线程阻塞**
- **展开态合盖再开盖，面板 Token 数字永久停更**：`handleSystemSleep` 只 `pause()` 未复位
  `tokenPollingStarted`，唤醒路径的 `startTokenPollingIfNeeded()` 被 guard 挡回。复位后又引出
  第二个问题——`stop()` 的收尾判据失效，盒盖期间退出应用就没人关只读连接；改用
  「本次运行是否**曾经**启动过轮询」作判据，两条契约同时成立。
- **只读库的锁跨整段 SQL 持有，而实时流水早已在后台队列共用它**：一次 500 行扫描就能堵住主线程
  那一拍（类注释还写着「当前全部调用点在主线程」）。后台侧新增 `withDedicatedConnection`
  （不进缓存、不碰共享锁、`close_v2` 收尾），5 个流水源全部改道。
- **详情页每次渲染都付两次 `sysctl(KERN_PROC_ALL)`**：`performanceCard` 在 body 里调
  `inspectProcessTree`，约 600 条 `kinfo_proc` + 每 pid 一次 `proc_pid_rusage` 全落主线程，
  还会与采样拍互相消费 CPU 差分窗口把岛内占用数字带偏。改为复用上拍的进程表，零额外系统调用。

**两处「写着安全其实没守」的护栏**
- `InstalledAppsCache.refreshingThread` 全仓无人赋值，唯一的重入断言恒真；断言挪到会真死锁的
  `refresh()` 等待之前，线程登记补在 `performRefresh`。
- `locateCache` 把 `nil` 挡在命中条件外，等于「装了但闲置」的常态每拍重跑全树 stat；
  负结果改为按 TTL 命中，但**只在有失效令牌时**——第一版无差别缓存，把 cline（`rootDir: nil`）
  「用户刚开的第一个会话」的发现延迟从 ≤2s 拖到 10s，第二轮复核抓到后已收口。

**测试有效性（审计报回 5 处永真断言，全部改成可失败）**
- 「setEnabled 后走后台路径」三条断言恒真（`Int >= 0`、`while` 条件已假、`isEmpty` 由 setup 保证）
  ——把采样改回主线程同步它照样绿。改为计数断言：`setEnabled` 返回前全表扫描次数不得增加。
- `refreshTokenUsageOnce` 断言是 `expectTrue(true)`；清空方法体即红。
- 休眠唤醒联动六个调用零断言；改为断言 pause/start 次数（重复事件必须幂等）。
- 真实环境采样只断言 `dimSnap != nil`，而 profiles 就一个 dim——换成引擎契约（一档案一行快照、
  返回值与发布状态一致）。零断言的「真实 sessions 目录信息」用例删除（行为已由临时目录用例覆盖）。
- WorkBuddy bundle id 用例的 `data!` 会让整条 runner 崩溃而非失败；并补上不依赖本机的硬期望
  （国外版必须登记 `com.workbuddy.workbuddy-ai`，不得登记 Application Support 目录名）。
- CLI DTO 往返断言两边一起动，改 `CodingKeys` 键名永远绿——补字面 JSON 键名断言（下游 Raycast/CSV
  按键名取数）。给状态 DTO 注入一次改名，测试如期变红。
- 文件侧失明补 `.unreadableFile` / `.undecodableFile` 两个失败码并落证据：此前「会话文件读不出来」
  与「这个会话没有待确认事项」在岛与 `doctor` 上完全同形，正是 CONTEXT.md 禁止的两态合一。
- 新增 `HardeningTests` 共 10 例；结构类断言在扫不到源码时改为抛错（静默通过等于给自己发假绿证）。
  全部 11 次变异验证逐条做过。测试 317 → 327。

**第二轮复核抓出的一轮修复自身缺陷**
`appleScriptLiteral` 把 0x00–0x1F 之外的控制字符写成 `\u{1b}`——实测 AppleScript **不认这个转义**，
`NSAppleScript(source:)` 返回 nil，而两个调用点都静默跳过，症状恰是该函数声称要消灭的「点了没反应」。
改为控制字符原样透传，并补一条「拼出的脚本必须真能编译」的断言（含 ESC / 换页 / 中文 / emoji /
前导短横线路径）。同轮抓出：重入断言放错函数、`stop()` 收尾判据被自己的修复打断、负缓存丢首会话。

## [0.0.92] - 2026-09-20

### 🎨 语义色收到一处：硬编码色值 186 → 36，等级色阶 5 份副本并成 1 份

上一版把「Theme 外的硬编码色值」钉成 186 的棘轮基线，当时判断是大面积清扫、不划算。
这一版回头量了一遍，结论变了：**这些色值不是 186 个各不相同的颜色，而是少数几支被反复抄写**——
`slate-200` 一支描边色在 17 个文件里抄了 45 次。抄写的代价不是整洁，是漂移：改一次色板要改
几十处，漏掉的那几处只有眼睛能发现，而上一版刚刚证明过我的眼睛看不到全部界面。

- **`Theme.swift` 新增 `Ramp`（Tailwind 基色阶）**：18 支被复用 ≥2 次的基色在此唯一登记，
  整数与 `Color` 成对给出（`slate200Hex` / `slate200`）。`Theme` 自身的动态令牌浅色半边也改引
  这批整数，浅色色板与基色阶从此不可能各说各话。视图层 147 处字面量换成 `Ramp.xxx` 引用。
- **五份 `ActivityLevel` 色阶副本并成一份**：`AgentIslandApp` / `AgentRowView` / `AgentHoverTooltip`
  各有一份**逐字节相同**的浅色 text/fill/border 三段梯子（合计 45 处字面量），`IslandView` 还有
  第四份基础色梯子。现统一为 `Theme` 上的 `color` / `lightText` / `lightFill` / `lightBorder`；
  各调用点只保留自己的外观分支与染色系数（0.12 / 0.14 / 0.18 是组件自己的决定，不并入）。
- **顺带删掉一份真正的死副本**：`IslandView` 里的 `ActivityLevel.label` 与 `AgentIslandCore`
  中同名属性**五支文案完全一致**（Swift 允许 UI 模块的扩展遮蔽 Core 的实现，所以它一直静默存在）。
- **辉光与状态色解耦重复**：`0xffd60a` 原写在 3 处、`0xff3b30` 与 `0x30d158` 各 2 处；
  现收为 `Ramp.neonAmberHex` / `neonRedHex` / `neonGreenHex`，`DockedSliver` 的三处
  `NSColor(hex:)` 改用 `Theme.glowAlert` / `glowWorking` / `glowIdle`。
- **等价性用机器核对，不靠断言**：把新树里每个 `Ramp.xxx` 按其登记的整数还原回字面量，再与
  `HEAD` 逐行比对——剩余差异只有「改设计的那些」，无一处色值变化。唯一的写法变化是
  `Color(hex: X, alpha: A)` → `Ramp.x.opacity(A)`，对纯色而言同义。
- **新测试先抓到我自己的漏网**：`语义色单点` 首跑即报出 5 处漏改（`ModelDonutChartView` 的
  动态色对、`AgentRingView` 的 `Palette`——都是 `dynamicLight:` 形态，不在最初那轮替换的匹配里）；
  修完后又报出 `IslandView` 的第四份色阶。同时反向修正了测试自身的一处误报：事件类型
  （`EventType`，只有 `attention` 无 `working`）的梯子不该算重复。
- **棘轮同步收紧**：`债务棘轮` 色值基线 186 → 36（余下 36 处是一次性强调色与黑/白半透蒙层，
  收进 `Ramp` 只会多出十几支无人复用的色）；新增 `语义色单点` 测试——`Ramp` 基色整数只准出现在
  `Theme.swift`，且等级色阶只准定义一处（解析 `Ramp` 块自动跟随，日后加色无需改测试）。
  两条断言各自做过变异验证：抄一支基色、复制一份梯子，都能让测试红。

- **顺手清空最后的编译告警**：`NotifyCommand` 两处 `[#NoUsage]`（`if let url` 只做存在性判断、
  `let (data, response)` 的 `data` 无人用）——目标产物现在 Swift 告警为 0，下次真告警不会被埋。

## [0.0.91] - 2026-09-20

### 👁 首次目视核对岛内界面，就地修掉一处文案截断

- **目视核对发现了测试永远发现不了的问题**：工作台「监控可信度自查」卡的依据行原文有 40 字，
  在约 370pt 宽的面板里被中段截断成「会话库与…细是设计如此」——读不通，且 `12 项测试全绿` 对此
  完全无感。缩短为「该档案未登记本地明细源，不代表它没在工作」，完整解释留在悬停提示里。
  复验截图确认整行完整渲染。
- **核对方式与边界（如实记录）**：Computer Use 不枚举 accessory（`LSUIElement`）应用，
  按名称与 bundle id 都取不到岛的 AX 树，因此无法点击/键入面板；改用「深链导航 + 全屏截图」
  （终端命令，非 GUI 自动化）。已目视确认：工作台自查卡排版、异常空态、菜单栏微监控、
  岛内列表与后台任务胶囊归属。**未能目视确认**：Token 分析页的热力/环形图（面板在无指针悬停时
  会自动收起，多次抓图为空）、`j/k` 滚动与 `/` 聚焦（需要向面板键入，受同一限制）——
  这三项仍只有测试与源码级验证。
- **回归验证**：全量 316 项自建测试 100% 通过（0 失败）。

## [0.0.90] - 2026-09-20

### 🐛 修掉 CLI 的「用量恒为 0」：没取数不再印成 0，取数改为按需

- **真实缺陷**：常驻引擎的 token 轮询只在「呈现活跃」时开启，一次性 CLI 进程里没人开启它——
  于是 `agentisland status` 每个 Agent 的 24h 用量与费用都印 `0` / `$0.00`，而同一时刻
  `agentisland tokens` 报的是 24h 3.81M / 累计 257.76M。这正是 CONTEXT.md 明令禁止的
  「把没取到呈现成没有」，而且 `status --json` 是脚本与 Raycast 的数据源，错误会被下游当真。
- **修复**：`TokenUsagePolling` 新增 `refreshSync()`（默认转 `refreshAsync()`，测试替身无需改动），
  引擎转发 `refreshTokenUsageSync()`，`LiveSampler.makeEngine(refreshUsage:)` 在采样前同步取回用量。
- **但不把代价强加给所有人**：同步取数要解析全部会话索引，本机实测多花 3.5~5s，对脚本场景不划算。
  所以 `status` 默认**不取数**并把该列印 `—`（明说没查），`status --usage` 才付这笔钱；
  `doctor` / `report` 作为诊断与存档产物默认取数。实测：默认 `status` 0.69s 显示 `—`，
  `--usage` 4.1s 显示 DimAgent 2.85M / WorkBuddy 716.7k。
- **`CLIAgentStatusDTO.tokens24h` / `cost24h` 改为可空**：`nil` = 本轮没去取，`0` = 取到且确实是 0。
  消费方只有 CLI 自身与测试（已核实），JSON 契约因此变得更诚实。
- **顺带消掉一个上一版自己造的假警报**：`doctor` 的「无本地明细」依据原本写着「一次性采样不等待
  异步刷新，未必代表真的为零」——那是在用措辞掩盖取数缺失。真去取数之后，本机该类别从 1 项降到 **0 项**，
  依据文案也回落成一句事实陈述（「同步刷新用量源后仍无记录」）。
- **回归验证**：全量 316 项自建测试 100% 通过（0 失败）。

## [0.0.89] - 2026-09-20

### 🧷 债务棘轮：两处长期债务只准降不准升（统一优化方案 Wave 3）

- **不做大清扫，先止住增长**：架构审计量出 Theme 之外硬编码色值 **186 处**、`UserDefaults.standard` 直读 **37 处**（散在 7 个文件，`SettingsStore` 本该是唯一入口）。两者都没有缺陷史，186 处视觉清扫是拿回归风险换整洁，不划算；真正划算的是让债务不再长。
- **新增 `DebtRatchetTests`**：把当前数量钉成基线，新增一处就让测试失败，失败信息直接写清该用什么替代（Theme 的动态浅/深成对色、`SettingsStore`/`SettingKey` 读写口径）以及「收敛后请把基线数字改小」。基线只能往下改。
- **计数口径的坑顺手记进注释**：`grep -c` 数的是**行数**，一行两处会漏计——基线最初按它定成 165，实际是 186，第一次跑就被自己的测试抓到。
- **已做变异验证**：临时加一处 `Color(hex:` → 测试如期失败（186 → 187），还原后 316 项全绿。
- **回归验证**：全量 316 项自建测试 100% 通过（0 失败）。

## [0.0.88] - 2026-09-20

### 🖥 岛内监控可信度自查卡（统一优化方案 Wave 2）

- **`doctor` 的同一套结论搬进灵动岛**：维护工作台在「用量与运维审计」之下新增「监控可信度自查」卡，逐条列出结论不可信的 Agent（会话源读不到 / 无本地明细 / 未接入明细源），点行直达该 Agent 详情。
- **刻意不新增判断也不重新采样**：卡片只读引擎已经在算的快照、只调同一个 `AgentObservability`。两处各写一份判定的话，「终端说这个 Agent 的待机不可信、岛上却显示一切正常」这种分裂迟早会回来——而卡片存在的唯一理由就是让人相信它说的话。
- **空态如实**：没有存疑项时显示「已核对全部在跑与已装智能体：每条状态结论都有可读的会话或进程证据支撑」，而不是留一片空白让人以为没数据（对齐 CONTEXT.md 的诚实性规则）。
- **未安装不算问题**：`notInstalled` 不进入存疑列表——本来就不该期待它有状态。
- **回归验证**：全量 315 项自建测试 100% 通过（0 失败）。**视觉验证未完成**：Computer Use 需要系统授予「辅助功能 + 屏幕录制」权限，授权窗口尚未确认，因此这张卡的实际排版（截断、间距、对比度）尚未目视核对。

## [0.0.87] - 2026-09-20

### 🔍 自查结论补齐三类真相（统一优化方案 Wave 1）

- **「没接入明细源」不再冒充「读不到数据」**：doctor 此前一次都没引用 token 明细源可用性（实测 grep 0 处），于是档案压根没登记会话库/token 目录的 Agent，与「登记了却本轮读不到」共用同一条结论——正是 CONTEXT.md「Token 数据覆盖」禁止的混淆。新增 `AgentProfile.hasLocalDetailSource` 与结论 `sourceNotWired`，实机效果：原先笼统一片「无本地明细 4」，现在拆成「无本地明细 1（值得排查）」+「未接入明细源 3（属正常形态）」。
- **一次性采样不再把「没赶上」说成「没有」**：CLI 单拍/双拍都不等待 token 监控的异步刷新，`tokenUsage` 恒空——原依据「也没有可读到的用量账本」实际在测量采样器的急躁而不是机器的真相。措辞改为「本轮也没取到用量账本（一次性采样不等待异步刷新，未必代表真的为零）」。
- **只读库不再把陈旧句柄误诊成「对方改了表」**：`ReadonlyDB` 靠 (dev, inode) 判断库是否被替换，而 `VACUUM` / 截断式原地重写**不改 inode**，缓存的旧句柄会被继续复用，随后的 prepare 失败就被定性为「结构已变更」——一条根本不成立的结论。现在 `prepare` 失败先作废连接重试一次，两次都失败才对外上报；真改表仍落到 `prepareFailed`（既有测试守住），并新增 `ReadonlyDB.invalidate(_:)`。
- **孤儿入口变成可用命令**：`Selftest.run()` 与 `Probe.run()` 此前只能靠 `.app` 的隐藏参数 `--selftest` / `--probe` 触发，用户与脚本无从得知。新增 `agentisland selftest`（核心逻辑自检，与 `doctor` 的「这台机器可信吗」明确分工），`--help` 与联动示例同步补齐。
- **回归验证**：全量 315 项自建测试 100% 通过（0 失败）。

## [0.0.86] - 2026-09-20

### 🩺 新增 `agentisland doctor`：把「这个 Agent 是真闲着，还是我根本没看到它」变成一条可查的结论

- **`agentisland doctor`（含 `--json` / `--agent <id|名称>` / `--all` / `--quiet`）**：一次性实况自查，逐 Agent 给出「结论 + 依据」，四类结论计数相加等于总行数便于核对。支持双拍采样拿真实 CPU 利用率（脚本友好的 `status` / `report` 仍走单拍）。
- **`AgentObservability`：两套互不相识的「健康」合成一处**。此前 `AgentHealthEvaluator` 只看卡死/CPU/内存，对**根本没在被监控**的 Agent 直接给 100 分「健康」；`SessionProbeHealth` 说得出「会话库读不到」却只出现在详情页一个 9pt 标签和 Markdown 报告里。现在合并为 `observed / blindSessionSource / noLocalData / notInstalled` 四类，纯函数、可表驱动测试。其中一条规则是实测逼出来的：Antigravity 处于「待确认」却因活跃会话数为 0 被判成「无本地明细」——会话强语义本身就是「源是通的」的证据。
- **失明证据进入结构化输出**：`CLIAgentStatusDTO` 新增 `healthScore` / `healthGrade` / `observability` / `observabilityEvidence`，CSV 追加 `Observability` / `ObservationEvidence` 两列（既有列序不变）。此前「读不到」的证据只存在于 Markdown，脚本与 Raycast 读到的 JSON 会把「读不到」当成「闲着」。
- **一次性采样统一到 `LiveSampler`**：`Probe.run()` 里那套正确做法（热安装缓存 → 真实文件监控预热 → 双采拿 CPU 差分 → `fullRegistry` 含宿主内嵌过滤）此前只挂在 `.app --probe` 上，CLI 够不到，四处各自实现并已分叉。**修掉一个真实可见性缺陷**：`status` 只遍历 `AgentRegistry.builtin`，用户自定义与自动发现的 Agent 在 `status` / `status --json` 里根本不存在，而 `report` 能看到；现在 `status` / `report` / `check` / `doctor` 同口径，默认与灵动岛一致只看启用集（实测 21 项），`--all` 出全集。
- **三份「复制诊断快照」收成一个入口**：右键菜单、工作台卡片与深度链接 `agentisland://export` 各自拼一份写剪贴板，其中**右键菜单那一处漏传了 `history:`** —— 同一个用户动作在两个入口产出两份不同内容且无人报错。现统一走 `DiagnosticsSnapshot`。
- **探测故障时间线落日志**：快照里的探测健康只有 120s 保质期（回答「此刻可信吗」），新增 `ProbeFailureLog` 按 (Agent, 失败类型) 冷却 10 分钟记一条 `AppLog.error`，并在恢复时补一条、重新武装——坏源不会每 2s 刷满日志，但「从什么时候开始读不到」有迹可循。
- **回归验证**：全量 315 项自建测试 100% 通过（0 失败），新增 3 项（可观测性四类互斥表驱动、DTO/CSV 承载证据、故障日志冷却与恢复）。

## [0.0.85] - 2026-09-20

### 🔒 停止后的采样竞态、探测健康保质期与版本号单一来源

- **`stop()` 之后在飞的后台采样不再落地**：`sampleInBackground` 只在入口检查 `running`，保护不了「后台 libproc 遍历进行中、此时 stop()」这段异步间隙——落地的那一拍会发布快照、补发完成/告警事件，并经 `sampleCore → scheduleNext` 把刚被 `invalidate` 的定时器重建回来，等于引擎在停止后自己又活了。现在回到主线程时复检。
- **会话源「读不到」有了保质期**：v0.0.84 的探测健康是最近值，被有意保留以免一次跳拍就擦掉证据，但没有时间戳——源修好之后如果长时间没有新写入（探测被跳过、不再写新值），那条旧的「会话源不可读」会永远挂在卡片与审计报告上，反过来把「读不到」伪装成「坏了」；离线 Agent 此前也无条件参与报告。现在由采样时钟盖章、超过 120s 即回收、离线一律不报，而源重新坏掉时证据会重新落下。
- **版本号回到单一来源**：新增 `AppVersion`（Core）并被 CLI 横幅、导出的 Raycast 清单、设置页回落值共同读取——此前应用已发到 v0.0.84，而 CLI 仍打印 v0.0.80、清单也停在旧版（发版脚本只校验 CHANGELOG 与 README，管不到代码里的字面量）。现在 `scripts/build-app.sh` 在打包前校验 `AppVersion.string` 与 CHANGELOG 首条一致，另有一条测试从源码树读同一份 CHANGELOG 再守一遍，专门兜「绕过脚本手工构建」。
- **回归验证**：全量 312 项自建测试 100% 通过（0 失败）。本轮同时是一次收敛判定：对 v0.0.81–v0.0.84 做了跨版本回归复核（合并冲突处、按 Agent 隔离的告警保护、探测健康、目录剪枝等价性），未发现功能回归；剩余项已评估为收益递减（详见 README 之外的架构债盘点）。

## [0.0.84] - 2026-09-20

### ⚙️ 后台扫描与索引聚合提速，告警隔离与解析失败可见性

- **目录枚举在深度上限真正生效处剪枝**：`FileMonitor.scanTree` 的 `maxDepth` 守卫原先只丢弃 level+1 的条目，枚举器却仍为每个 `level == maxDepth` 的目录 `opendir/readdir` 一轮——纯支出、零收入。实测 `~/.gemini/antigravity/brain` 有 8,716 个 `steps/<n>` 目录正卡在这一层（其 `output.txt` 在 level 5，今天就读不到），趟数 **19,490 → 10,756**，单趟全量扫描 **218ms → 74ms**，`newest` / `activeSessions` / `newestFile` 逐项相等（夹具测试锁死）。刻意**没有**按目录名剪掉 `steps` 子树：那会连 level-4 目录自身的 mtime 一起丢掉，活跃会话判定不再等价。
- **结构化 Token 索引改分桶聚合**：按 (路径, mtime, 长度) 缓存每个文件的合计，未变文件不再参与本轮聚合——首趟 **1,988ms → 稳态 2.69ms**，`usage(now:)` 汇总 **1,002µs → 528µs**，逐工具用量签名与改造前完全一致；只读库探测改 `stat(2)`（26.2µs → 0.6µs/次，且避开 `resourceValues` 的毫秒级陈旧窗口）。
- **成本告警保护窗口按 Agent 隔离**：`alertProtectedUntil` 原是单个全局时间戳，而 `clearLatestEvent()` 会无条件清掉它——用户 dismiss 掉 B 的横幅，等于顺手解除了 A 正在生效的激增保护，A 可以立刻再告警。现改为按 agent id 记录，并接入 `resetTracking` / `resetAllTracking` / `retainTracking` 三个既有生命周期入口。
- **「解析器坏了」不再伪装成待机**：只读库打不开、`sqlite3_prepare` 失败（第三方应用改了 schema）、会话文件超上限这些路径原先一律返回 `nil`，UI 显示「待机」，用户没有任何线索。现在失败原因作为 `SessionProbeHealth` 随探测结果一起带出（沿用 v0.0.81 那条「不放进全局字典」的隔离原则），在诊断快照与详情页的既有状态位上如实呈现；「库里确实没数据」与「这个源读不到」重新分开，兑现 CONTEXT.md 的诚实性规则。
- **SQL 扫描合并**：DimAgent 的 24h 与累计两趟 `usage_ledger` 全表扫合并为单趟。
- **一处被证伪的优化没有做**：`workbuddy` 的 `ORDER BY updated_at DESC LIMIT 1` 不改用 `max(rowid)`——本机真实库即可反驳（rowid 5 比 rowid 1 早 10.6 天，因为行会被就地 UPDATE），且代价也没有可省的（`LIMIT 1` 让临时 B 树只存一行，实测 0.16µs，瓶颈是全表扫本身而 `updated_at` 无索引是第三方库结构决定）。证据写进注释，避免后人再试一遍。
- **回归验证**：全量 310 项自建测试 100% 通过（0 失败）。

## [0.0.83] - 2026-09-20

### 🩹 采样正确性、界面诚实性与键盘流补全（多智能体并行审计轮）

- **用户中断不再把 Agent 永久钉在 working**：Ctrl-C / 点停止不会补一条 `tool_result`，这条僵尸调用会让会话尾窗每拍谎报工作态，于是引擎的完成与待机分支永远走不到，2s 快采样与高频全树扫描一并被锁死（CPU 与耗电双输）。现在中断之前的在途调用一律撤销，中断之后重新发起的命令仍继续是在途判定。
- **时钟回拨后激增检测不再整段失明**：重锚 token 速率基线时只改了局部副本，窗口不足即 `continue`，导致未来数小时每一拍都重复走同一条重锚分支；改为立即写回（时区错 8 小时就是盲 8 小时）。
- **进程抖动不再重复弹通知**：libproc 抖一拍或 CLI 换 PID 重启到同一份会话，原先会抹掉通知去重指纹，让同一条「等待确认」重新弹窗加铃声；指纹的清理现在只发生在显式终止与档案移除。
- **采样缓存三处修正**：定位缓存的失效令牌改用 `stat()`（`URL.resourceValues` 有毫秒级陈旧窗口，同类问题曾在尾读合并上实测误命中）；TTL 由 3s 提到 10s 以避开与 2s 拍频打拍（Antigravity 一趟定位 24ms，原先约一半的拍仍在付费），命中时时间戳只向前修正以免 `fileAge` 因缓存虚高；尾读合并由单槽扩到四槽——两个以上 Agent 同时工作会互相踢出缓存，命中率归零。
- **专有方言不再每拍重复解析**：Antigravity / DSH / Cline 的动作文案与会话强语义同源（同一份 transcript / 投影缓存），直接复用本拍已探得的结果，省掉第二遍 262KB 尾读 + 120 行 JSON 解析。
- **界面诚实性**：热力图在无明细时不再合成一整排全零格子并显示「活跃 0/24h」，改为显式的「本时段未发现 Token 明细」（兑现 CONTEXT.md 里「未发现明细 ≠ 用量为 0」这条硬规则）；模型环形图 `prefix(4)` 静默丢弃的模型现在给出「其余 N 个 · xx%」与总计行，占比不再对不上。
- **破坏性动作与键盘流**：清空事件历史补二次确认（与删除档案、终止进程同一套模式）；`j/k` 焦点移动现在会把目标行滚动进可视区（超过 6 个 Agent 时此前键盘流是静默失效的）；`/` 现在真正聚焦搜索框，不必再用鼠标点一下。
- **搜索谓词单一来源**：列表过滤与键盘焦点集合此前各写一份谓词，可能互相不一致，现共用同一入口并有测试守护。
- **可访问性与设计令牌**：模型环形图、热力图、事件历史、进程树四张自绘卡片给 VoiceOver 各补一句可读摘要；纯图标按钮补标签；图表调色板回到 `Theme` 的动态浅/深成对色，低于项目自身 9pt 下限的字号全部抬回；`HH:mm:ss` 三处缺失 POSIX locale 的格式化器补齐（系统区域设置会改变定宽区里的数字与分隔符）。
- **档案数据补全**：`tokenRoots` 成为档案字段，Token 统计里最后几处 `~/...` 字面量（`~/.workbuddy*/projects`、`dimcode` 会话目录）回到注册表唯一声明，注册表自洽测试同时守住方言、库位置与 token 根目录三类漂移。
- **回归验证**：全量 301 项自建测试 100% 通过（0 失败），新增 15 项（含界面呈现层 12 项与采样正确性 3 项）。

## [0.0.82] - 2026-09-20

### 🏗 能力收进档案：路径漂移消除与通用会话解析再提速

- **`detect(lines:)` 由每行约 13 趟全树递归合并为单趟事实收集**：
  - 实测 96 行尾窗单拍 **7.61ms → 2.89ms**，其中 JSON 解析只占 0.27ms——即改造前 96% 的开销是把同一棵已解析好的树反复走完；
  - 新增 `LineFacts` 一次遍历产出请求标记、完成标记、工具调用与结果、`role/source/type/status/state` 集合与标识符，键值在每个节点只读一次并归一化；`identifier(in:)`（12 次键查找 + 下钻 `data`/`payload`）由每节点最多 3 次降为 1 次；
  - 判定顺序、DFS 首个命言语义与解除等待确认的条件均与原实现逐条对齐，287→286 项既有测试（该解析器是全仓测试最密处）零回归；
  - 删除被取代的 9 个辅助函数（`findRequest` / `findCompletion` / `resolvesAttention` / `isResolutionRecord` / `startsOrContinuesWork` / `structuralValues` / `identifiers` / `extractToolCalls` / `extractToolResolutions`）。
- **Agent 能力改为档案声明（见 ADR-0004）**：新增 `emoji`、`sessionDialect`、`sessionDatabase` 三个档案字段与 `AgentRegistry.profile(_:)` / `databasePath(for:)` 取值入口：
  - 同一批会话库路径此前在解析器、动作探测、日志流、Token 统计里各硬编码一份（`~/.dimcode/v2/dimcode.sqlite` 出现 4 次且完全不在注册表里），现由注册表唯一声明，日志流与 Token 统计一律向注册表取值；
  - 会话探测与库查询由 `switch profile.id` 改为按**方言/schema 穷尽分派**——格式数量远少于 Agent 数量（Cline 与 Roo Code 同源），新增复用既有格式的 Agent 只需登记档案；
  - 删除 CLI 状态表的 19 分支 id→emoji 梯子，其中 4 个 id（`dimagent` / `vibe` / `ima.copilot` / `egobrowser`）在档案改名后已不存在，只会静默退化成默认符号；
  - 新增「注册表自洽」测试：声明专有方言却没有对应会话目录的档案直接测失败（线上表现为该 Agent 永远只显示待机）；字段全部带默认值并走 `decodeIfPresent`，升级前存档的自定义 Agent 照旧可解。
- **日志流与会话探测共用注册表路径**：`AgentLogStreamer` 的 4 处库路径与 Antigravity `brain` 目录改为向注册表取值，`workbuddyDataDir` 这一中间层随之退役；`detectAntigravitySession` 的上下文出口由全局字典改为参数。
- **DimAgent CLI 名补登记**：`knownCLIs` 原本只认 `dim`，而其发行版可执行名同样有 `dimcode`——开启「自动发现」时 `dimcode` 会被当成一个陌生 CLI，与 DimAgent 重复成一行。
- **回归验证**：全量 286 项自建测试 100% 通过（0 失败）。

## [0.0.81] - 2026-09-20

### ⚡ 采样热路径主线程 I/O 治理与跨 Agent 上下文隔离

- **消除采样主线程的会话树递归遍历（实测单拍省约 84ms）**：
  - Antigravity 与 DSH 的专有会话探测此前每拍在 `@MainActor` 上重新枚举整棵会话树——真实机器实测 Antigravity 797 次 stat / 24ms、DSH 501 次 stat / 15ms，而「会话强语义」与「当前动作文案」两条链路各走一遍；
  - 新增**带失效令牌的会话定位缓存**（TTL 3s + 根目录 mtime 变化即刻重定位）：缓存的只是「选哪个文件」，尾读与解析每拍照常进行，因此新会话零延迟、状态转移无任何滞后；
  - 新增**尾读合并**：262KB 尾部读取（3.7ms）比 120 行 JSON 解析（2ms）更贵，同一文件在 mtime 与长度未变时复用上一次的行（有效期 0.5s，短于采样周期）；新鲜度取 `stat()` 而非 `URL.resourceValues`——后者有毫秒级缓存窗口，恰好会在这两条链路的微秒间隔内误命中；
  - 实测稳态单拍开销：Antigravity 4.6ms、DSH 0.2ms（改造前两条链路合计约 91ms）。
- **修复跨 Agent 上下文串味**：`AgentSessionSignal.backgroundTasks / subagents / tokenBreakdown` 长期硬编码读取按 `"antigravity"` 索引的全局上下文，导致 Claude、Codex 等**任意 Agent 的卡片与终端看板都会显示 Antigravity 的在途子任务、后台命令与 Token 细分**；上下文改为随 `AgentSessionProbe` 显式返回，按 agent id 索引的全局字典（`clearActiveContext` 零调用点、残留永不失效）整体删除。
- **大会话文件读取护栏**：Cline `ui_messages.json` 与 DSH 投影缓存的整块 `Data(contentsOf:)` 改为内存映射并加 32MB 上限，超限放弃本轮解析、降级到双信号判定，不再让无上限增长的日志拖垮采样。
- **状态重置收敛**：10 个按 agent id 索引的滞回/去重/告警基准集合的清空逻辑原先散落 5 处，且 `terminateAgent` 与 `cleanAnomalies` 确实漏清了 `tokenRateBaseline`（杀掉进程后首个结算窗口把离线全程计入分母，速率被摊薄、激增告警被推迟）；收敛为 `resetTracking(for:)` / `resetAllTracking()` / `retainTracking(for:)` 三个入口。
- **死配置清理**：删除 `predictiveAnalyticsEnabled`、`localEventServerEnabled`、`localEventServerPort` 三个声明后从未被读写的设置键。
- **高覆盖率回归验证**：新增 3 项测试（跨 Agent 上下文隔离、TTL 内新会话立刻重定位、等长改写立刻读到新内容），全量 286 项自建测试 100% 通过（0 失败）。

## [0.0.80] - 2026-09-19

### 🚀 多 Agent 在途命令感知、Antigravity 子任务树与 Token 细分、悬浮胶囊与菜单增强

- **多 Agent 在途命令（Bash / exec_command / say command）全生命周期闭环感知**：
  - **Claude Code 终端命令感知**：深度解析 `tool_use`（如 `Bash`）生命周期，在命令未交付 `tool_result` 前严格拦截 `completed` 终态，并实时抽取展示所执行的命令；
  - **Codex 函数调用感知**：精准感知 `exec_command` 及参数，在终端长命令执行中保持活跃工作态；
  - **Cline & Roo Code 任务流解析**：解析 `ui_messages.json`，将 `ask: command` 识别为等待批准操作（`.attention`），`say: command` 识别为活跃执行（`.active`），并在完成消息交付后准确恢复为 `.completed`。
- **Antigravity 专有深度分析能力升级**：
  - **子智能体树（Subagent Tree）全链路追踪**：提取 `invoke_subagent` 调用及其角色、模型与状态，建立子智能体生命周期映射；
  - **Token 深度细分指标拆解（TokenBreakdown）**：实时抽取 Prompt、Completion、Cached Read、Cache Write 与 Thoughts/Reasoning Token 细分数据，并在详情卡片直观呈现；
  - **任务取消与完成安全识别**：联动 `manage_task` 取消/终止指令与子智能体应答完成标记，杜绝死锁与虚假活跃。
- **灵动岛 UI 与上下文交互全面增强**：
  - **行内活跃胶囊（Capsules）与悬停 Tooltip**：智能体行内动态呈现 `⚡ 后台N` 与 `🤖 N子任务` 微胶囊，悬停展示在途任务命令详情；
  - **右键上下文菜单（Context Menu）扩展**：支持快捷吸附切边（上/右/下/左）、切换深浅外观、通知分级模式、静音/开启完成提示音与复制当前诊断快照；
  - **CLI 终端看板（`status` / `top`）同步升级**：直观高亮展示后台任务与子智能体状态标识。
- **高覆盖率回归验证**：
  - 新增 `MultiAgentAdvancedTests` 专有测试套件，全量 283 项自建测试 100% 通过（0 失败）。

## [0.0.79] - 2026-09-19

### 🎯 深度优化 Google Antigravity 运行感知与后台任务生命周期跟踪

- **彻底消除后台任务执行中“已完成”误判与弹窗/铃声轰炸**：
  - 深度剖析 Antigravity 异步任务机制（`run_command` 终端构建/长命令、`schedule` 定时器、`invoke_subagent` 子任务等）；
  - 当模型启动后台任务并输出中间等待消息时，严格拦截 `PLANNER_RESPONSE` 默认转换为 `completed` 的行为，精准保持 `working` 活跃工作状态；
  - 只有当所有已发起的后台任务完全交付（或被明确取消/终止）、且智能体产出最终用户响应时，才触发 `.completed`。
- **实时任务元数据提取与友好动态文案展示**：
  - 自动解析正在运行的后台命令（如 `swift build`、`pytest`）、定时等待时长（如 `后台定时中 (10秒)`）及子任务角色；
  - 清洗并过滤 `SDKROOT=...`、`SKIP_TESTS=...` 等过长环境变量前缀与 `arch -arm64 ` 指令包装，灵动岛与菜单栏文案更清爽直观；
  - 结合任务执行日志（`.system_generated/tasks/*.log`）写入更新时间戳，保障长时间无主转录写入时的感知保鲜。
- **回归测试套件全面覆盖**：
  - 新增 5 项专用测试用例覆盖单个/多个后台任务拦截、生命周期交付、任务取消与动作文案清洗，全量 274 项测试保持 100% 通过。

## [0.0.78] - 2026-09-19

### 🖥️ 终端交互动态看板、热门 Agent 生态扩充、快捷键速查 HUD 与 Raycast 导出

- **CLI 类似 htop 交互式动态监控看板（`agentisland top` / `status -w` · 方案 A）**：
  - 基于 ANSI 光标与清屏转义序列构建轻量全屏看板，支持自定义刷新周期（`-i <sec>`，默认 1.5s）；
  - 动态展示监控智能体总数、工作态占比、总 CPU% 与 24h Token/费用走势，高频刷新各智能体明细表格；
  - 终端 Raw 模式单字符交互：支持 `q` 退出并恢复终端光标、`r` 立即重采刷新、`c` 快速清理死锁与孤儿后台。
- **新兴 AI 编码智能体深度生态扩展（方案 B）**：
  - 内置增加 **Cline**（开源自治 Agent，监控 `saoudrizwan.claude-dev/tasks` 会话目录与进程）；
  - 内置增加 **Roo Code**（高频迭代分支 Agent，监控 `rooveterinaryinc.roo-cline/tasks` 与 `roo` 进程）；
  - 内置增加 **Continue.dev** 增强进程识别（支持 `continue-core`）；
  - 内置增加 **Goose**（Block 开源自治 CLI 智能体，监控 `~/.config/goose/sessions` 与 `goose` 进程）。
- **展开态快捷键速查指南 HUD（方案 C）**：
  - 展开卡片下按下 `?` 键，平滑呼出精美磨砂玻璃质感快捷键速查面板（`ShortcutHUDView`）；
  - 完整呈现 `1~3` 切页、`j/k` 焦点导航、`Enter` 下钻、`/` 搜索、`Esc` 逐级退出与 `⌘R` 刷新；
  - 再次按 `?`、`Esc` 或点击任意外部空白区域即刻淡出，无门槛驾驭全键盘流。
- **多格式数据报表与 Raycast 扩展清单导出（`agentisland report` · 方案 D）**：
  - `agentisland report` 扩展支持 `--format <markdown|csv|json|raycast>`（简写 `-f`）；
  - `--format csv`：生成标准逗号分隔数据表，包含时间戳、ID、名称、状态、PID、CPU、内存、健康评分与 24h/累计 Token 费用；
  - `--format raycast`（或直接运行 `agentisland raycast`）：导出适配 Raycast 扩展的命令清单配置，一键映射灵动岛协议联动。

## [0.0.77] - 2026-09-19

### 🚀 多屏热插拔自愈、本地 Webhook 外部事件接收器、Token 预算进度管控与全键盘流

- **多显示器协同与屏幕热插拔自适应（方案 A）**：
  - `IslandPanelPositioning` 屏幕有效性断连自愈：新增 `isValidScreen` 校验，当外接显示器断开拔出时，自动平滑回退至有效屏幕，杜绝窗口悬空或坐标越界；
  - 统一 `snapToDockEdge` 与 `placeWindow` 的目标屏幕计算口径，多屏跟随模式（`followMouse` / `mainScreen` / `builtInScreen` / `externalScreen`）全面提升健壮性。
- **本地零依赖 Webhook / IPC 外部事件接收器（`LocalEventServer` · 方案 B）**：
  - 基于 Apple 原生 `Network.framework`（`NWListener`）构建轻量本地 HTTP 监听服务，仅绑定 `127.0.0.1:41999`；
  - 支持通过 `POST /notify` 接收外部 JSON 载荷（`agent`、`type`、`message`、`detail`），毫秒级触发灵动岛微窥横幅、提示音与系统通知；
  - 原生 CLI 子命令 `agentisland notify`：支持命令行 `-a <agent> -t <type> -m <message>`，优先 HTTP 推送，服务未就绪时自动回退至 URL Scheme 分发。
- **智能体用量预算与成本警戒系统（方案 C）**：
  - 终端 CLI 工具 `agentisland tokens` 支持 `--budget <num>`（如 `--budget 1m` / `-b 500k`）或自动读取系统设置中的预算配置；
  - 终端字符级进度条呈现：`[██████████░░░░░░░░░░] 52% (520k / 1.0M)`，根据用量比例自适应切换色彩（正常绿/警告黄/超额红）；
  - `--json` 输出结构中增加 `dailyBudget`、`budgetRatio` 与 `budgetStatus` 字段。
- **展开态全键盘流操作体验（方案 D）**：
  - 数字快捷键 `1` ~ `3` 快速切页（`1`: 主列表，`2`: Token 用量分析，`3`: 维护工具箱）；
  - 支持 Vim 风格键位 `j`（下移）/ `k`（上移）聚焦选择智能体，配合 `Enter` 快速下钻详情；
  - 保留 `/` 搜索呼出与 `Esc` 逐级返回或收起面板。

## [0.0.76] - 2026-09-19

### 💻 原生终端命令行工具（agentisland-cli）与终端运维生态

- **轻量原生 CLI 二进制（`agentisland`）**：
  - 零额外三方依赖，直接基于 `AgentIslandCore` 构建编译，发布随附 `dist/agentisland` 及 `.app/Contents/MacOS/agentisland`；
  - 自动感知终端环境（TTY vs Pipe/Redirect），支持高对比度 ANSI 彩色排版与管道纯文本无缝回退。
- **快照总览与结构化输出（`agentisland status`）**：
  - 终端精美表格列出所有 Agent 状态（工作态 🟢 / 待机 🟡 / 离线 ⚪️）、PID、CPU%、内存、会话数、24h 用量及最近活动；
  - 支持 `--json` 格式化导出全量结构化 DTO，便于与 Raycast / Alfred 脚本无缝集成；支持 `--all` 包含未运行离线项。
- **Token 消耗分析与月末走势（`agentisland tokens`）**：
  - 汇总打印最近 24h 与历史总计 Token 及支出费用，展示各大智能体消耗排行；
  - 集成 `TokenForecastEvaluator`，一秒推算当月末消耗预期与预算耗尽风险。
- **死锁与异常进程诊断排查（`agentisland check`）**：
  - 自动探测长时间死锁卡顿、终端断开孤儿后台（ppid=1）与高内存泄漏（>2GB）目标，清晰列出诊断原因与处置安全性。
- **智能体一键清理释放（`agentisland clean`）**：
  - 支持 `--dry-run` 预览拟终止目标与预计释放内存；默认安全批处理模式，支持 `--force` 强行彻底清理。
- **深度链接呼出与交互（`agentisland open`）**：
  - 终端一条命令触发桌面灵动岛展开/收起、直达 Token 分析（`analytics`）、工作台（`toolbox`）或指定智能体卡片（`agent <id>`）。
- **Markdown 运维审计报告生成（`agentisland report`）**：
  - 快速生成格式化 Markdown 审计报告，支持 `--copy` 直接入系统剪贴板，支持 `--output` 保存至本地文件。

---

## [0.0.75] - 2026-09-19

### 🔗 URL Scheme 深度链接、能耗自适应、日志智能特征识别、月末成本预测与异常自愈守护

- **URL Scheme 深度链接路由（URLSchemeRouter）**：
  - 注册并解析 `agentisland://` 系统协议，支持与 Raycast、Alfred、macOS 快捷指令及脚本终端无缝联动；
  - 覆盖展开/收起切换（`toggle` / `expand` / `collapse`）、直达智能体详情（`agent?id=<id>`）、打开用量图表（`analytics`）、打开工作台（`toolbox`）、静默清理（`clean`）与导出审计报告（`export`）。
- **硬件电池与电源节能自适应（PowerSourceMonitor）**：
  - 实时监听 IOKit.ps 供电状态与系统低电量模式（Low Power Mode）；
  - 笔记本处于电池供电时自动平滑降频后台采样（工作态保底 3s、闲置 10s、离线 120s），接通电源后满血恢复，显著降低差旅与移动办公耗电。
- **实时流水错误智能特征识别与高亮（LogPatternAnalyzer）**：
  - 流水解析引擎自动模式识别 API 429 限流、编译构建失败、Git 分支冲突、鉴权认证失效及环境配置缺失；
  - 在事件行直观呈现红色错误徽标，支持分类筛选「异常/报错」及一键复制提取到的核心报错摘要与错误代码。
- **Token 走势与月末预算预测（TokenForecastEvaluator）**：
  - 基于最近 24 小时真实消耗速率与当月剩余自然日，推导月末预估总 Token 与费用支出；
  - 联动每日预算上限，在预算超额时主动推导预算枯竭天数并标记红色/橙色预警。
- **死锁与异常驻留自愈守护（AgentResilienceGuard）**：
  - 持续追踪智能体异常长死锁（≥3分钟）或内存剧增泄露（≥5分钟），结合冷却防抖机制主动发出带终止逃生通道的自愈处置横幅，防患于未然。

---

## [0.0.74] - 2026-09-19

### 🔊 交互音效与触觉、进程树全景、会话速览、多屏自适应与审计报告导出

- **原生交互音效与触觉反馈（SoundEffectsManager）**：
  - 支持任务完成提示音自定义（Glass 水滴 / Pop 微泡 / Ping 叮咚 / Blow 柔和 / 静音），提供设置页即时试听；
  - 熔断告警音效升级（Sosumi 敲击 / Basso 低沉 / Funk 强烈 / 静音）；
  - 接入 macOS 触控板微触觉反馈（NSHapticFeedbackManager），任务完成、熔断告警及贴边吸附时提供细腻物理感知。
- **派生工具与子进程全景树（ProcessTreeInspector）**：
  - 基于 libproc 进程表递归探查 Agent 派生的下游子进程树（如 node、git、python3、ripgrep、cargo 等）；
  - 在 Agent 详情页展示子进程层级、PID、分项与整树聚合 CPU / 内存占用，快速识别究竟是 Agent 自身还是下游子任务在吃系统资源。
- **会话速览与快捷接续（RecentSessionNavigator）**：
  - 会话列表中支持一键复制 Session ID，以及「在终端中接续」快速定位至该会话工程目录；
  - 提升跨项目、跨会话恢复上下文的切换效率。
- **多显示器自适应停靠策略（Multi-Monitor Follow Mode）**：
  - 新增屏幕停靠策略配置（跟随鼠标所在屏幕 / 固定主屏幕 / 优先外接显示器 / 优先内置屏幕）；
  - 彻底解决多屏与外接 4K 拓展坞场景下灵动岛无法随工作焦点自适应吸附的痛点。
- **运维与用量审计报告导出（AuditReportExporter）**：
  - 在工作台一键生成 Markdown / CSV 格式的完整审计报告；
  - 涵盖所有智能体在线状态、PID、CPU/内存、健康评分诊断及 24h/累计 Token 计费总览，方便向团队汇报与量化归档。

---

## [0.0.73] - 2026-09-18

### 🛡️ 智能体健康诊断评分、IDE 一键直达、模型环形图、历史时间线与菜单栏微监控

- **智能体健康度与稳定性诊断评估器（AgentHealthEvaluator）**：
  - 100 分制结构化评估运行中 Agent 的稳定度，综合死锁卡死倾向、CPU 高负载与物理内存 RSS 增长斜率；
  - 详情页性能卡片直观呈现「健康分 / 评级徽章 / 自愈建议」，帮助开发者及时发现隐性卡死与长会话内存泄露。
- **常用 IDE/代码编辑器一键直达（Editor Jumper）**：
  - 在当前工作区（CWD）卡片中自动探测本机已安装的 VS Code、Cursor、Windsurf、Xcode 等开发工具；
  - 一键在目标编辑器中快速打开当前工程目录，彻底免去手动切终端与切项目的繁琐操作。
- **模型 Token 消耗环形占比图（ModelDonutChartView）**：
  - 在 Agent 详情页按模型拆分模块新增现代化环形图（Donut Chart），直观呈现多模型配比与份额（如 Sonnet vs Haiku vs GPT-4o）；
  - 悬停扇区联动高亮各模型百分比与消耗量。
- **任务与告警事件历史时间线（EventHistoryPopoverView）**：
  - 引擎内置有界事件队列（保留最近 25 条历史任务），并支持一键清空；
  - 展开卡片顶栏新增时钟历史图标，点击秒级弹出历史时间线，随时回溯今日跑完了哪些任务、单次耗时及历史警报。
- **菜单栏系统顶栏动态微监控（MenuBarBadgeMode）**：
  - 设置页支持自定义系统菜单栏图标附加显示模式（仅图标 / 活跃任务数 / 今日 Token 消耗）；
  - 抬眼即可在 macOS 顶栏一瞥全局并发与开销，无需每次展开灵动岛。

---

## [0.0.72] - 2026-09-18

### 🌟 Token 预算预警、全局热键、活动热力图、紧凑排版与工作区直达

- **Token 消费预算预警与超额封顶（TokenBudgetTracker）**：
  - 支持在偏好设置中设定每日 Token 消费限额（100k ~ 10M / 天），引擎逐拍智能评估用量；
  - 消耗达 80% 时触发黄色预警，达 100% 触发红色超额告警横幅与呼吸光效，防止后台 Agent 死循环烧钱；
  - Token 分析页增加生动的「今日预算进度条」，直观展示用量百分比与剩余额度。
- **系统级全局热键秒级呼出/收起（GlobalHotKeyManager）**：
  - 基于 macOS Carbon 原生内核实现全局热键（默认 `⌥ A`），免辅助功能权限且跨全屏应用秒级响应；
  - 按下热键时自动识别当前光标所在的活动屏幕，置顶展开灵动岛或平滑收起。
- **24 小时协同活动节律热力图（ActivityHeatmapView）**：
  - 在 Token 用量分析页中新增 24 小时结对编程活跃矩阵（类似 GitHub Commit Heatmap）；
  - 24 格色块根据每小时真实 Token 消耗梯度渐变渲染，悬浮秒显具体时段与用量数据。
- **高密度紧凑视图排版模式（Compact Density View）**：
  - 设置页提供「紧凑排版密度」开关，针对 13 寸小屏 MacBook 或多 Agent 并发场景微调行高、字号与指示环尺寸；
  - 减少滚动操作，单屏容纳 Agent 数量显著提升。
- **工作区与终端一键直达（Quick Workspace & Terminal Jumper）**：
  - 使用 macOS 内核 `proc_pidinfo(PROC_PIDVNODEPATHINFO)` 直读运行中 Agent 的真实工作目录（CWD），微秒级响应且零子进程开销；
  - 在 Agent 详情页清晰呈现「当前工作区」，并提供「打开终端」与「在访达中显示」一键直达按钮。

---

## [0.0.71] - 2026-09-18

### ⚡️ 任务效能洞察、列表即时搜索、报表导出、外接多屏与极简纯净模式

- **任务耗时与效率统计洞察（TaskDurationTracker）**：
  - 新增任务执行效能分析引擎，记录各 Agent 任务时长、完成次数与历史极值；
  - Agent 详情页集成「任务耗时与效率」卡片，直观展现 24h 工作总时长、单任务平均用时及单次最长用时。
- **主列表即时搜索与快速过滤（Quick Search & Filter）**：
  - 展开卡片按快捷键 `/` 或点击顶栏放大镜秒级呼出行内搜索输入栏，按 Agent 名称或 CLI 命令快速过滤；
  - 过滤状态下无缝支持键盘方向键移动与 `Enter` 直达，按 `Esc` 智能退出搜索。
- **Token 消费与用量账单报表导出（TokenReportExporter）**：
  - 在 Token 用量分析页一键导出当前统计周期（今日/本周/本月）的精美 Markdown 表格或标准 CSV 账单；
  - 支持直接一键复制到剪贴板并带有「已复制」微动画反馈，方便开发者团队报销与成本归档。
- **多显示器智能跟随与热拔插重排（Multi-Screen Smart Placement）**：
  - 优化屏幕自适应定位机制，外接显示器断开拔掉时智能回退到主屏或包含光标的有效可用屏幕，防止灵动岛悬空在无效坐标；
  - 响应屏幕参数改变通知自动重新校准吸附。
- **极简纯净模式（Hide Docked Sliver）**：
  - 设置页新增「极简纯净模式」开关，常态收起时可完全隐去屏幕边缘 6pt 微细条，满足极简桌面需求；展开或有重要提醒横幅时正常平滑呈现。

---

## [0.0.70] - 2026-09-18

### 🚀 智能计费估算、全键盘穿梭、绿色节能感知与性能预设

- **智能 Token 计费估算引擎（TokenCostEstimator）**：
  - 为 DimAgent、Codex 及大量本地未上报消费金额的日志和会话，基于官方最新费率表（Claude 3.5/3.7 Sonnet、Haiku、Opus，OpenAI GPT-4o、o1、o3-mini，DeepSeek V3/R1，Gemini 2.0/2.5 Flash 与 Pro 等）提供加权混合参考估算；
  - 在 Agent 详情页概览卡片、模型列表行与 Token 分析页中智能呈现带 `~` 标识的估算费用与徽标，底层真实 DB 零污染。
- **全键盘极速交互与快捷键导航（Full Keyboard Navigation）**：
  - 展开卡片时支持键盘极速穿梭：`Esc` 智能逐级返回（二级页面返回主列表，主列表返回收起）；
  - `↑` / `↓` 方向键在主列表中平滑移动焦点并渲染青色微光外框；
  - `Enter` / `Return` 直达选中的 Agent 详情页；
  - 全局支持 `⌘R` 立即采样刷新、`⌘,` 快捷呼出偏好设置面板。
- **MacBook 电池与绿色节能感知自适应调度（Eco Battery Awareness）**：
  - 深度集成 macOS 原生低电量模式（`ProcessInfo.isLowPowerModeEnabled`）与 `NSProcessInfoPowerStateDidChange` 通知监听；
  - 开启低电量模式时，引擎自动将闲置采样间隔延展至 10s、全离线至 120s，在保证灵敏度的同时大幅降低唤醒与电池消耗。
- **设置页性能预设档位与体验重构（Performance Presets）**：
  - 引擎与性能设置增加「⚡️ 极速灵敏 / ⚖️ 平衡标准 / 🍃 极致省电」一键档位切换；
  - 增加一键「恢复默认」快捷重置功能；
  - 实时联动展示「系统低电量模式已开启」绿色节能指示。

---

## [0.0.69] - 2026-09-17

### 💎 优雅、高效、精简的架构去重与单一职责重构

- **视图层组件抽象与统一黑曜石卡片修饰符（IslandComponents）**：
  - 提取统一的 `.obsidianCardStyle(cornerRadius:fill:)` 与 `.subtleCardStyle(cornerRadius:fill:)` 视图修饰符，封装标准高光渐变边框、多阶阴影与深浅色黑曜石底色；
  - 彻底重构 `DetailViews`、`TokenAnalyticsView`、`ToolboxView` 中 8 处重复手写的 RoundedRectangle/strokeBorder/shadow 样板代码，消除大量样板并保证全应用设计系统 100% 一致。
- **通用分类筛选胶囊组件（FilterCapsuleBar）**：
  - 提取通用的 `FilterCapsuleBar<Item: Identifiable & Equatable>`，接管分类筛选横向滚动、弹簧触控动画、高亮选框与计数徽标；
  - 替换 `LiveLogStreamView` 与 `ToolboxView` 中完全一致的胶囊筛选栏，实现关注点收敛与组件复用。
- **控制器单一职责拆分与解耦（IslandPanelController Modularization）**：
  - `IslandPanelRouting.swift`：承载 `CardRoute` 路由导航与页面回溯栈（`openAgentDetail`, `closeAgentDetail`, `openLiveStream`, `closeLiveStream`，以及快捷导航 API）；
  - `IslandPanelPositioning.swift`：聚焦窗口几何计算、吸附动画、屏幕可用区域约束与边缘热区几何（`placeWindow`, `snapToDockEdge`, `dockTargetFrame`, `sliverRect` 等）；
  - `IslandPanel.swift` 主控制器大幅瘦身近 250 行，专注保留窗口生命周期管理、代理回调与核心交互状态派发。

---

## [0.0.68] - 2026-09-17

### 🌟 深度多视角多维度优化：前沿生态扩充、趋势图交互游标与主卡原生右键菜单

- **前沿主流 AI 编程智能体原生识别（AgentRegistry）**：
  - 内置支持 **Windsurf**（Codeium 旗下新一代 Agentic IDE，`com.exafunction.windsurf`，完整映射其工作区与进程家族）；
  - 内置支持 **Aider**（最流行终端 CLI 智能体，`aider`，支持会话跟踪与即装即用）。
- **Token 趋势图交互式游标与精确时段浮标（TokenAnalyticsView）**：
  - 在趋势折线图接入连续悬停（`.onContinuousHover`）与拖动手势（`DragGesture`）；
  - 动态渲染贯穿参考线与焦点光圈，毫秒级浮动展示时段起止与用量微徽标（例如 `15:00–16:00 · 45.2k`），让数据洞察精准到刻度。
- **macOS 原生 ContextMenu 右键菜单生态（AgentRowView）**：
  - 主卡列表行接入原生右键上下文菜单：
    - 🚀 **直达窗口 / 终端**：快速置顶激活运行中的应用或控制台；
    - 📊 **查看模型与 Token 详情**：一键深入多模型与消耗拆解；
    - 📜 **查看实时流水抽屉**：直达实时工具调用与输出事件；
    - 📁 **在访达中显示会话数据**：若存在 session 存储目录直达定位文件；
    - 📋 **复制进程 PID 与 Agent 名称**：极大方便开发者排查终端与脚本任务；
    - 🛑 **强制终止此进程 (逃生舱)**：高负荷卡死时直观退出。

---

## [0.0.67] - 2026-09-17

### 🚀 深入多角度优化：系统休眠节能与热唤醒、日志分类与自动跟踪、工具箱维度筛选

- **系统休眠节能与即时热唤醒（Sleep & Wake Lifecycle）**：
  - 接入 `NSWorkspace.willSleepNotification`：当 Mac 盒盖或休眠时彻底冻结采样定时器与令牌轮询，防止后台无谓唤醒与电池消耗；
  - 接入 `didWakeNotification` 与 `screensDidWakeNotification`：从休眠或锁屏唤醒时毫秒级恢复定时器并立即在后台派发热同步，瞬间对齐最新 Agent 状态。
- **实时日志分类筛选与自动跟踪（LiveLogStreamView）**：
  - 新增分类过滤胶囊栏（全部 / 工具与执行 / 文件编辑 / 思考与消息 / 系统），支持快速按事件类型精准筛选并展示实时命中计数；
  - 自动滚动追踪：自动刷新开启时，新事件到达自动平滑滚动定位到最新项，彻底告别手动翻找。
- **CPU 动态负荷色彩预警与微交互优化（AgentDetailView）**：
  - 性能与健康卡片中 CPU 占用根据实时负荷动态着色（>80% 红色警示、>30% 橙色注意、常规柔和银白）；
  - 内存与 PID 条目增加悬停 Tooltip 精准展示原始字节数与完整说明。
- **工具箱异常诊断多维度筛选（ToolboxView）**：
  - 新增异常类别过滤条（全部 / 死锁 / 孤儿 / 内存超限），支持分类直观审视与按分类一键批量清理。

---

## [0.0.66] - 2026-09-17

### ⚡️ 深入体验优化：即时热唤醒、触感反馈、Esc键导航与工程零警告守护

- **展开即时热唤醒（Adaptive Hot-Wakeup）**：
  - 展开灵动岛面板（点击或 Hover 悬停触发）时，立即通过后台派发采样（`sampleInBackground()`），在动画展开的数百毫秒内就刷新进程与用量快照，杜绝展开时视觉内容停留在旧周期的卡顿滞后感。
- **macOS 原生触觉反馈（Haptic Feedback）**：
  - 引入 `HapticFeedback`（基于 `NSHapticFeedbackManager`）；
  - 在灵动岛展开、收起、工具箱单项清理与批量清理异常进程等核心交互动作中触发对齐与确认级细微震颤反馈，极大提升操作质感。
- **全局 Esc 快捷退出与导航（.onExitCommand）**：
  - 在子页面（Agent 详情、Token 统计、会话列表、实时日志、工具箱、设置）按下 `Esc` 键平滑返回主卡；在主卡展开态按下 `Esc` 键平滑收起至状态条。
- **工具箱异常诊断体验增强（ToolboxView）**：
  - 异常进程诊断项中补充显示进程可执行文件名（`commandBasename`），方便用户更清晰地辨识卡死或泄漏的底层程序；清理动作配合触感反馈。
- **代码库健康与零编译器警告守护**：
  - 修复多处闭包中隐式强引用 `[weak self]`；
  - 清理所有测试与实现文件中的无用局部绑定变量，达成编译构建 100% 零警告。

---

## [0.0.65] - 2026-09-17

### 🎨 UI 与动效全面升级：流体弹性层级转场与触感反馈

- **灵动岛流体层级推入/返回转场（Route Transitions）**：
  - 主卡与二级/三级详情页（Agent 详情、Token 统计、会话列表、实时日志抽屉、快捷工具箱）全面接入 `ZStack` + `.transition(.asymmetric(...))` 深度视差动效；
  - 进入子页带有微妙水平微位移与缩放弹性推入（`offset x: 12, scale: 0.98`），返回主卡平滑淡出，彻底消除原本页面生硬跳切的问题；
  - `CardRoute` 升级遵循 `Hashable`，配合视图 `.id(controller.route)` 触发高保真原生弹簧物理流体转场。
- **Agent 列表行交互触感增强（AgentRowView）**：
  - 列表行增加轻微悬浮微缩放（`scale: 1.004`）与 `.spring(response: 0.22, dampingFraction: 0.75)` 触觉级弹性阻尼；
  - 点击行进入 Agent 详情页时联动显式弹簧转场动画，大幅强化按压沉浸感。
- **状态指示与顶栏标题平滑过渡**：
  - `AdaptiveHeaderText` 的主标题、副标题与徽标接入 `.contentTransition(.numericText())`，多 Agent 状态在“工作中/待确认/已完成/待机”切换时数字与文字平滑滚动过度，杜绝突兀闪烁；
  - `statusDot` 状态指示灯增加色彩插值平滑过渡动画（`easeInOut 0.3s`）。
- **极光流光弧与双层心跳呼吸环（AgentRingView）**：
  - `SpinningActivityArc` 重构为 `AngularGradient` 极光流光拖尾微弧，旋转时呈现平滑渐变流光轨迹；
  - `PulsingAttentionArc` 升级为外层光晕微扩散与内层核心呼吸双层脉冲环，待确认与警告状态更具灵动生命力。
- **全链路返回与汇总栏微动效统一**：
  - `DetailHeader` 返回按键增加悬停微弹性放大（`scale: 1.08`）与显式弹簧返回；
  - 底部 `TokenSummaryBar` 增加悬停轻浮升与点击弹簧转场。

---

## [0.0.64] - 2026-09-17

### 🐛 彻底根治 Antigravity 僵尸“等待确认”误报（专有探测路由优先）

- **双重根本原因**：
  1. **路由旁路失效**：`AgentSessionInspector.inspect(...)` 在最外层无条件对 `activityFiles` 遍历执行通用的 `detect(lines:)`，只有未命中时才回退至数据库/专有探测器。导致 Antigravity（以及 DSH）写在 `inspectAntigravitySession` 中的专有高保真逻辑被完全绕过，transcript.jsonl 被当作通用未知日志解析。
  2. **通用检测器 attention 无法解除**：通用的 `detect(lines:)` 遇到历史命令参数或已回答提问中的 `ask_question` 后，无法识别 Antigravity 的 `source: USER_EXPLICIT` / `type: GENERIC` 答复；且后续即使出现模型工作、新工具调用，也没有清除 attention 状态的机制，导致状态永远钉死在“等待确认”。
- **系统级修复**：
  - **专有协议优先分发**：在 `AgentSessionInspector.inspect` 入口置顶专有派发，Antigravity 与 DSH 100% 走其专有高保真会话探测器。
  - **通用状态机解挂**：通用 `resolvesAttention` 增加对 `source: "user" / "userexplicit"` 识别；并在通用 `detect` 中引入 `startsOrContinuesWork` 时自动解除旧 attention 信号。
- **验证**：真实环境现场探针验证 `inspect` 与 `inspectAntigravitySession` 输出完全对齐，回归测试 244 项全数通过。

---

## [0.0.63] - 2026-09-17

### 🐛 修复 Antigravity 确认通知误报

- **根本原因**：`detectAntigravitySession` 在对 transcript.jsonl 末尾行做 `reversed()` 扫描时，遇到含 `ask_question` 的 PLANNER_RESPONSE 会立即返回 `.attention`，但**未检查该 step 之后是否已存在 GENERIC / USER_INPUT**（即用户早已答复），导致历史上已答复的弹窗被反复误判为"等待确认"。
- **修复**：在正式扫描前，先对 tail-48 行做一次轻量元数据预解析，收集各行的 `step_index` 与 `type`。扫描到 `ask_question` 时，检查元数据中是否存在 `step_index` 更大的 GENERIC 或 USER_INPUT 行；若存在则视为已答复，继续向前扫描而非触发 attention。
- **新增回归测试**：
  - `ask_question → GENERIC → run_command` 序列 → 应为 `.active`，不得触发 `.attention`
  - `ask_question → GENERIC → 最终完成` 序列 → 应为 `.completed`

---

## [0.0.62] - 2026-09-16

### 🚀 性能调优、状态跟踪修复与 Token 动效流畅化

- **消除 120Hz 布局风暴（Docked CPU 降至 0%）**：将贴边微细条呼吸光晕重构为 CoreAnimation 独立硬件图层（`CoreAnimationBreathingGlow`），彻底消除 SwiftUI 120Hz 递归布局重排，贴边常态 CPU 占用由 20%~53% 压降至 **0.0% ~ 0.1%**。
- **全局鼠标移动近邻预过滤**：在 `MouseMoveThrottle` 引入 `proximityRect`，收起态仅当光标进入贴边条周围 40pt 感知区时才调度事件，99.9% 屏幕中央移动直接纳秒级丢弃，零 Task 分配与零主线程开销。
- **系统调用与内存堆复用**：`PathCache` 缓存存活进程可执行路径与小写字符串，复用 4KB CChar 缓冲区，避免每 2 秒循环创建大量小写副本与 `proc_pidpath` 调用。
- **离线 Agent 免深搜**：离线目录若根目录 mtime 未改变直接复用缓存，跳过递归树遍历；在线时自动恢复深搜。
- **通知中心与横幅联动自动消除**：通知在系统中心被用户关闭/点掉时自动清除灵动岛事件横幅；Agent 退出等待确认状态时自动解除待确认横幅，重新开始工作时自动清除历史完成横幅。
- **状态跟踪准确度修复**：
  - 精准区分 Antigravity 待机与工作中状态，消除空闲时的状态跳变；
  - 修复 DeepSeek Harness 运行中被提前误判完成的问题；
  - 针对 WorkBuddy 专家团引入专属 Token 激增安全下限，兼容多专家团高用量场景。
- **Token 统计页 24h / 7天 / 30天 切换动效流畅化**：
  - 范围选择胶囊增加 `@Namespace` 与 `.matchedGeometryEffect`，实现平滑弹性横向滑移；
  - 增加多时间范围内存缓存与后台静默预加载，除首次冷启动外彻底根除骨架屏闪烁；
  - 用量数值应用 `.contentTransition(.numericText())`，折线趋势图增加淡入过渡，进度条弹性伸缩。
- **自动化交付约定强化**：在 `AGENTS.md`、`CLAUDE.md` 与 `GEMINI.md` 中增加规范：改造完成后只要测试通过，自动提交至远端并执行发版与文档更新。
- **验证**：新增状态追踪、性能优化与横幅自动消除回归测试；runner **241/0 全绿**。

---

## [0.0.61] - 2026-09-15

### 🔎 Token 工具下钻修复（R44）

- **Codex 用量行可正常响应**：可读的 Codex 本地会话用量不再要求同时存在独立实时监控快照；被 ChatGPT 桌面端承载而有意去重的 Codex 也能点击进入详情。
- **详情语义准确**：没有独立运行项时，详情页展示 24h / 累计 Token 摘要，并说明数据来自本地会话记录，避免误报为“已不在监控列表”。
- **验证**：新增内嵌 Codex 可下钻回归；runner **222/0 全绿**。

---

## [0.0.60] - 2026-09-15

### 🧲 四边贴靠与自动收起修复（R43）

- **自动收起恢复**：状态栏宿主窗口不再被当作岛的交互浮层；其宽泛 frame 不会再错误阻止收起。popover / 菜单栏浮层实际被悬停时仍保持展开。
- **Vokie 式四边停靠**：展开卡片可自由拖拽，松手按面板到可用屏幕上、右、下、左四边的最近距离，吸附到对应边缘；水平边复用 X 锚点、垂直边复用 Y 锚点，跨重启保持位置。
- **四向完整交互**：顶部/底部使用 140×6pt 横向微细条，左侧/右侧使用 6×120pt 纵向微细条；反向倒角、悬停热区、自动展开、收起箭头、提示气泡箭头与辅助功能标签均按方向镜像。
- **打包阻塞修正**：补齐 Token 分析页指标组件的调用签名，避免此前已完成的分析界面在全量 Swift 构建中报错。
- **验证**：新增浮层命中与四边吸附回归；测试 runner **221/0 全绿**。

---

## [0.0.59] - 2026-09-15

### 📊 跨工具 Token 总体与明细（R42）

- **真正的总体用量**：汇总不再只覆盖 DimAgent / OpenCode；新增 Codex、Claude、WorkBuddy、WorkBuddy AI 本地结构化日志适配，24h、累计与时间趋势统一跨工具求和
- **按工具统计**：时间分析页明确分为“总体用量”和“按工具用量”，逐项展示 Token、占比与可用成本；无记录、未发现本地明细分开表达，不再用缺席造成统计完整的错觉
- **统一净消耗口径**：Codex / Claude / WorkBuddy 使用 `(input-cache.read)+output`，Codex reasoning 已包含在 output 时不重复计算；同一 response id 跨恢复/分叉日志去重
- **低开销结构化索引**：JSONL 只保留时间与计数，按 inode/mtime/size 缓存；活跃文件增长时只解析追加段，避免 60s 轮询反复扫描数十 MB 会话正文
- **下钻边界修正**：历史工具未进入当前监控列表时保留统计行，但不再伪造可点击箭头进入空详情页
- **验证**：新增工具覆盖状态、Codex/WorkBuddy 汇总、缓存扣除与去重回归，runner **219/0 全绿**

## [0.0.58] - 2026-09-15

### 📈 Token 时间分析（R41）

- **完整时间视图**：点击主卡底部 Token 汇总条即可进入独立分析页，支持 24h / 7天 / 30天切换；默认主卡仍保持一行摘要，不增加日常信息噪声
- **趋势与环比**：24h 按小时、7 天按 6 小时、30 天按天生成固定密度趋势图，同时显示范围总量、成本、累计量、上一等长周期及变化百分比
- **来源构成可下钻**：展示 DimAgent / OpenCode 在当前范围内的 Token 数与占比，点击来源继续进入原有模型、会话明细
- **统一净消耗口径**：DimAgent 使用 `(prompt-cache.read)+completion`，OpenCode 使用 `input+output+reasoning`；未来记录和两周期之前记录不进入图表，数据库保持只读且只在打开分析页或切换范围时查询
- **窄卡片与辅助功能**：图表固定 24–30 个点，适配 330pt 面板；空数据、加载态、范围选择和趋势摘要均提供明确视觉/VoiceOver 语义
- **验证**：新增时间边界、固定桶数、上一周期和双 SQLite 数据源集成回归，runner **217/0 全绿**

## [0.0.57] - 2026-09-15

### 📰 长标题可读性与信息层级（R40）

- **顶部标题不再消失**：原先 Agent 名、实时动作、在线计数和三个固定按钮挤在同一行，长内容会把标题压到近乎零宽；现在改为“稳定主标题 + 动态动作副标题”双层结构，并为主标题保留 96pt 最低可读区
- **次要信息主动让位**：在线计数在空间不足时从 `4/10 在线` 自动退化为 `4`，完整含义继续通过悬停提示和 VoiceOver 提供，不再与主标题争抢空间
- **截断仍可辨认**：文件名、模型名、动作与事件标题统一采用中间省略，同时保留语义前缀和文件/模型后缀；悬停显示全文，辅助功能始终朗读全文
- **同类界面统一**：主顶栏、Agent 行、事件横幅、详情标题、模型/会话行、工作台、实时流水与菜单栏概览共用可读性契约
- **布局闭环**：顶栏几何从单行 23pt 校准为双层 34pt，列表在 460pt 总高内自适应让位；新增 2 个标题契约回归，runner **214/0 全绿**，真实 SwiftUI 布局门禁通过

## [0.0.56] - 2026-09-15

### 👁️ 在线 Agent 可见口径

- 主列表、菜单摘要、顶部活动环统一改为**仅显示当前进程仍在的 Agent**；离线项即使 24h 内有活动或 Token 记录也不再形成幽灵条目
- 在线的待机、运行中、待确认与刚完成状态继续正常显示；设置页的安装与启停管理仍保留全部档案
- 菜单与卡片计数文案由“可见”改为“在线”，回归覆盖离线近期活动、离线有量、在线待机和在线工作四种边界

## [0.0.55] - 2026-09-15

### 🔔 状态语义与待确认通知（状态准确性优化 · R38）

- **状态从三态扩展为五态**：在 `offline / idle / working` 之外新增 `completed / attention`。CPU 与会话文件仍负责活动近似，Codex、Claude、DimAgent、ZCode、WorkBuddy、OpenCode 的结构化会话事件负责“本轮已完成”和“正在等待用户”的强语义，避免低 CPU 的确认等待被错标为待机、完成事件又被写入窗口拖成工作中
- **待确认通知可直达**：每个未处理的确认/权限请求只通知一次；通知携带 Agent ID，点击后优先激活对应 GUI，CLI Agent 则沿 PID 父进程链唤起 Terminal / iTerm2 / VS Code / Ghostty / Warp 等宿主。目标已退出时回退到 AgentIsland 对应详情页
- **多 Agent 同拍不丢通知**：系统通知改走逐事件流，多个 Agent 同一采样周期同时等待确认时逐条投递；岛内横幅仍保持单槽与严重告警保护
- **确认等待不会伪造完成**：进入 `attention` 会切断旧工作区间，用户处理后回到真实的 working/idle，不补一条虚假的“任务完成”；冷启动读到旧完成标记只展示状态，不追发陈年通知
- **隐私与开销边界**：只读取本轮扫描命中的最新会话文件，尾读最多 96 行 / 256 KiB；结构化解析仅消费 type/name/status/role/id 等协议字段，通知不带问题或回答正文；未知格式继续走原有双信号降级路径
- **验证**：新增待确认解析、解除、完成终态、事件去重、多 Agent 并发投递、通知路由、文件命中隔离等回归用例；自建 runner **212/0 全绿**

## [0.0.54] - 2026-09-12

### 🎯 状态真实性：打开应用没动不再误报「任务完成」与响铃（v4 战役 · R37）

用户报告「有些软件打开了没动也算一次完成，会响铃通知」。真机取证确认两条独立成因，均已修复：

- **完成事件需要写入证据**：CPU 高只说明进程在烧 CPU——实测 ChatGPT 静置时反复冲到 20.6%/43.4%，每次尖峰都会走完「working → idle」并补发完成事件（复现日志：`ChatGPT 任务完成 (10秒)` + 提示音）。**纯 CPU 区间收尾改为静默**：仍照常显示工作中（双信号判定不变），但不再产生完成事件、不响铃、不弹 Peek。有写入证据的区间一切照旧
- **文件噪声剥离（全部来自实测）**：
  - SQLite `-shm` 空转触碰：Antigravity 空闲时 10 个会话库的 `-shm` 每 200s 被同步刷一次、size 恒为 32768 字节，而**主库 mtime 停在 22 小时前**——「刚刚写入」的假活动足以把空闲应用顶成 working。`-shm` 是连接共享的 mmap 索引页，任务数据落在主库或 `-wal`（保留 `-wal` 判定，实测 600s 窗口零变化），过滤后不丢信号
  - 浏览器内核（Chromium/Electron）用户数据子树：Cache / Code Cache / GPUCache / Dawn 系列 / Session Storage / Local Storage / IndexedDB / Crashpad / blob_storage / DIPS / Trust Tokens / Singleton* 等 45 项整棵剪枝，同时消除虚报的「活跃会话」计数
  - 内核状态文件与应用账号文件：Network Persistent State、DevToolsActivePort、Preferences、Cookies、First Run、`BrowserMetrics-*.pma`、`oauth_credentials.json`、`app_storage.json`、Sparkle `*appcast*` 等
  - Antigravity 档案不再监控 `Library/Application Support/Antigravity`（内核用户数据目录：实测仅打开应用就有 36 次写入/20 分钟全部落在此），只保留 `conversations` + `brain` 两个真正的会话数据目录
- **判别性验证**：新增 6 个用例（含「CPU 高但无写入 → working」契约保留、有写入证据照常报完成、写入证据不跨区间泄漏）；变异验证 196/6 红（临时还原过滤与准入条件），恢复后 202/0 全绿
- **活体对照**：假 Agent 进程「CPU 尖峰 22s → 静置」形态零事件（修复前同形态补发完成事件）；触碰 `*.db-shm` 无任何状态变化、触碰 `*.db-wal` 立刻判 working；真实写入（vibe-usage）仍正常报完成并响铃


## [0.0.53] - 2026-09-12

### 🏁 v4 战役收官（R32–R36）

状态机与调度深度侦察（10 主发现 + 4 加固）驱动五轮闭环：

- **状态机时序批**（R32/0.0.49）：面板态处理代际守卫（sink Task 乱序覆盖新态）、唤醒后激增告警重新武装、拖拽后 grace 复位、收起延迟实时校验、断点阈值钳 180s（变异验证闭环）
- **文件监控生命周期批**（R33/0.0.50）：噪声文件经父目录 mtime 的传播链阻断、监控目录永久删除的幽灵活动终态清零、配置变更后首扫绕过节流（新目录信号不再空白到下一拍）
- **数据面批**（R34/0.0.51）：配置变更重采样移出主线程、菜单栏 popover 按需刷新 token（不再显示冻结值）、停止后连接重建代际治理、刷新请求合并
- **能效与护栏批**（R35/0.0.52）：边缘检测 Timer 自适应降档（活跃 0.12s / 静止 0.5s）、高度签名漂移哨兵（变异验证）
- 收官核验：测试 196/0 ×3、布局哨兵 PASS、selftest 全过、A/B 对照无回退（基线 v0.0.48 同条件 CPU 相当）、代码卫生终扫（print/try!/as! 零残留）


## [0.0.52] - 2026-09-12

### 🔋 能效与护栏批（v4 战役 · R35）

- **边缘检测 Timer 自适应降档**：活跃 0.12s / 静止 3 秒后降档 0.5s——hover 展开的主路径是 0.05s 节流的鼠标事件（不受降档影响），Timer 只是「光标停在热区内、事件被节流丢失」的兜底；静止时（远离热区且未展开）高频唤醒纯属浪费。光标进入热区、面板展开、拖拽/按压任一发生时立即恢复快档
- **高度签名漂移哨兵**：R08 的「展开态高度签名去重」依赖签名组成与 `expandedHeight` 输入严格对应，但此前只有注释约束——新增源级清单断言（六项输入逐一核对 + 渲染实参同组），布局改动漏改签名会直接测试红（变异验证：删签名项即 195/1）
- 测试 196/0；A/B 实测（同条件基线 v0.0.48 vs 当前）：CPU 相当（5–8%，WorkBuddy 活跃期同条件），无性能回退；收起态静态约 0–1%


## [0.0.51] - 2026-09-12

### 🧵 数据面批（v4 战役 · R34）

- **配置变更重采样移出主线程**：设置页启停开关、新增/删除自定义档案、终止与清理后的重采样此前在主线程同步执行（进程全表快照 + 全部档案匹配 + 动作探测 ~10ms 级），开关可感知卡顿——统一改走后台采样路径（同等 running/在飞防护）
- **菜单栏 popover 不再显示冻结的 token**：popover 是 token 数据的第三消费方但不参与「呈现活跃」生命周期（收起态轮询已暂停）——打开时按需单次刷新；此前从未展开过面板的话，Token 概览甚至永远空白
- **停止后的连接重建不再泄漏**：`stop()` 与在飞刷新竞态时，迟到的查询会重建连接并写回缓存（此后无人再关，违背「彻底清理」契约）——连接代际判定后一次性连接用毕即关
- **刷新请求合并**：首刷与 60 秒轮询相邻触发时，第二个请求此前白等锁——在飞去重后直接返回
- 测试 192 → **195**


## [0.0.50] - 2026-09-12

### 📁 文件监控生命周期批（v4 战役 · R33）

- **噪声写入不再经父目录传播成工作信号**：`.lock`/心跳类噪声文件写入会刷新父目录 mtime——目录计入 newest 会把已过滤的噪声反向传播回工作信号（working 误报 + peek 弹出），且每次噪声写入都改变根 mtime 使快跳过永久失效（大目录每拍全量枚举）。现在 newest 仅由信号文件聚合，目录条目只参与活跃会话计数
- **监控目录永久删除后的幽灵活动清零**：用户删除 `~/.claude/sessions` 等目录后，缓存残留删除前的最后写入时间（「抖动保留旧值」分支吞掉了终态），卡片 24h 内仍按幽灵活动显示。连续 ≥3 趟缺失即终态清零（阈值吸收原子替换/迁移的瞬时空窗），目录重建后自动恢复
- **配置变更后的首扫不再被节流吃掉**：切换启停集后新目录的文件信号此前会空白到引擎下一拍（idle 节律最长 60 秒）——完成扫描带代际标记，配置变更后队列上的下一趟扫描绕过节流立即落地
- 测试 188 → **192**（噪声传播/幽灵终态/节流豁免三组回归 + 既有用例适配新语义）；布局哨兵 PASS；selftest 全过


## [0.0.49] - 2026-09-12

### ⏭️ 状态机时序批（v4 战役 · R32）

- **面板状态处理加代际守卫**：`$displayState`/`$latestEvent` 两个 sink 经 Task @MainActor 跳跃且 MainActor 无 FIFO 保证——乱序执行时旧态处理覆盖新态（token 轮询在 docked 态空转开启、动画按错误几何计算、旧事件的音效/横幅语义覆盖新事件）。现在捕获值与当前值不符即过期跳过
- **唤醒后激增告警重新武装**：采样断点（睡眠/挂起）重置集此前漏 `tokenSpikeAlerted`——睡前已告警、醒后持续高速率的 Agent 会被残留去重标记静默压制
- **拖拽后 grace 复位**：peek 进行中开始拖拽，残留的 open-grace 会把拖拽结束后的自动收起压制最长 6 秒
- **收起延迟实时校验**：设置页挂起期改「自动收起延迟」后，在飞收起按新值校验到期（≤5s 窗口）
- **断点阈值钳 180s 上界**：idle 滑杆上限 60s 时 3 倍阈值恰为 180，越界脏配置（idle 最高 600）会让阈值放大到 1800。测试以有判别力构造锁定（变异验证：旧代码 187/1 红 / 新代码 188/0 绿）；120–180s App Nap 场景根治需事件驱动锚点，已落档为后续规格决策
- tokenSpikeAlerted 放宽 internal（@testable 断言用）；测试 186 → **188**


## [0.0.48] - 2026-09-12

### 🔖 WorkBuddy AI 安装标记修复（follow-up）

- **真实 bundle id 对齐**：`WorkBuddy AI.app` 的 Info.plist 实测 `com.workbuddy.workbuddy-ai`——档案此前照抄 Application Support 数据目录名（`com.workbuddy.workbuddy`）漏了 `-ai`，「已安装」标记永久假阴性（面板 INST 列显示 no）。已修正并新增哨兵测试：档案 bundleIDs 与真实 Info.plist 必须对齐（本机未装该变体时跳过）
- 测试 185 → **186**


## [0.0.47] - 2026-09-12

### 🧳 WorkBuddy 国内版 / 国外版区分（用户实测驱动）

本机同时装有 `WorkBuddy.app`（com.tencent.workbuddy.mac，腾讯系=国内版）与 `WorkBuddy AI.app`（com.workbuddy.workbuddy=国外版）。旧档案混合误配：bundle 只认国内版、数据只读国内版目录 `~/.workbuddy`，而宽口径 pathContains "workbuddy" 又会同时命中两个变体的 Electron 进程——两个版本并存使用时无法区分（用户实测两变体昨天到今天均在活跃使用）。

- **拆分为两条独立档案**：`workbuddy`（国内，~/.workbuddy）与新增 `workbuddy-ai`（国外，~/.workbuddy-ai，图标 globe）
- **pathContains 精确隔离**：两变体进程 basename 同为 Electron 且路径都含 "workbuddy"，宽口径会双份计数——国内版钳到 `/Applications/WorkBuddy.app` 与 `.workbuddy/`，国外版精确到 `WorkBuddy AI.app` 与 `.workbuddy-ai`
- 动作探测与实时流水按变体分发数据目录（inspectWorkBuddyAction/fetchWorkBuddyEvents 增加变体参数）
- 新增双向隔离测试（国内版不得吸走国外版路径、反之亦然）；probe 实证双行各自独立显示状态与最近活动（用户实测：国内版昨晚 23:41 仍有写入、国外版今天持续活跃）；测试 185/0


## [0.0.46] - 2026-09-12

### 🏁 v3 增量战役收官（R21–R30）

两路增量侦察（UI 次要文件 7+4 条 / Core 次要文件 5+3 条）+ v2 遗留 6 项，十轮闭环（小轮自验 + 单测/哨兵锁定）：

- **用户数据防丢失**（R21/0.0.38）：自定义档案逐元素容错（坏元素不再摧毁全部）、启停集损坏只读降级（绝不写回）、knownAgents union 口径
- **口径对齐**（R22/0.0.39）：前缀族冲突共享判定（UI 校验与匹配器单一事实源）、激增阈值非档位吸附、bundle 扫描并入 ~/Applications
- **面板几何**（R23/0.0.40）：菜单栏内展开振荡上界钳制；空态高度实测校准 87 → 121
- **字号统一**（R24/0.0.41）：badgeFont 令牌，14 处 <9pt 文本提升
- **性能**（R25/0.0.42）：进程快照 syscall 合并（基准 4.1ms 均值）；isFresh 稳态语义修正
- **Core 健壮化**（R28/0.0.45）：warmUp 已热仍回调（启用集重放契约）、refresh 重入运行期断言、Selftest 强解包修复
- 收官核验：测试 184/0 ×3、布局哨兵 PASS、selftest 全过、README 现势性复核（无 v2 Q7 类漂移残留）

## [0.0.45] - 2026-09-12

### 🧱 Core 健壮化批（v3 战役 · R28）

- **warmUp 已热仍回调**：引擎 init 依赖它做「冷启动完成后重放启用集」——此前以已热缓存构造引擎（Probe/测试/未来第二组合根）时重放被静默跳过。现在已热无在途扫描时 completion 仍在主线程立即回调（幂等）
- **refresh 重入断言**：扫描器经注入闭包递归调 `refresh()` 会自等待广播永久阻塞——`refreshingThread` + `performRefresh` 断言把注释约束变成运行期检查
- **Selftest 强解包修复**：临时目录在两拍间被删时 `before!` 直接 crash；内置表 id 变更时 5 处 `first{...}!` 全部 crash——均改为记 failure 继续（诊断工具保持「报告失败项」价值）
- `profile(id:)` 文档升级为显式防呆警告（永远查不到 cli-* 自动发现条目）；测试 184/0


## [0.0.44] - 2026-09-12

### ♿ 无障碍第二批（v3 战役 · R27）

- **菜单栏 popover Agent 快捷行补交互语义**：`isButton` trait + 「名称，状态，点按查看详情」label（与主卡 Agent 行同口径；此前 `onTapGesture` 行对 VoiceOver 无交互语义）
- **设置/退出 icon-only 按钮补 label**：此前只有 `.help`，VoiceOver 播成「…帮助」而非按钮名
- axdump 实证（System Events entire contents）：AX 树可达、面板节点存在；完整 VoiceOver 流程仍待真实辅助技术客户端人工确认；测试 184/0


## [0.0.43] - 2026-09-12

### 🧩 UI 细节批（v3 战役 · R26）

- **会话行「目录未找到」反馈**：会话目录被删除/改名/外置盘卸载后点击行静默无效（v2「直达失败反馈」修复在本页的漏网点）——消费 `NSWorkspace.open` 返回值，失败 1.5 秒行内提示
- **菜单栏文案对齐实体**：「收起/展开侧边栏」→「收起/展开灵动岛」（岛可贴顶，不再是侧边栏）
- **详情页占位说明**：无按模型拆分数据的源（claude/codex 等）此前无声缺席，现显示「该数据源暂不支持按模型拆分」
- TokenSummaryBar 累计花费补 help 与 layoutPriority（与 24h 侧对称，双 cost 防截断失察）
- 两处 DateFormatter 固定 `en_US_POSIX`（12 小时制用户的行内时间防 locale 改写变形）
- 恒真 `#available(macOS 13)` 死代码删除；设置页补「活动间隔不可大于闲置间隔」联动说明；测试 184/0


## [0.0.42] - 2026-09-12

### ⚡ 快照 syscall 合并与戳比对语义修正（v3 战役 · R25）

- **进程快照 syscall 合并**：单次 `sysctl(KERN_PROC_ALL)` 同时给出全部 pid 与父进程 ppid，替代 `proc_listpids` + 每条目一次 `proc_pidinfo(PROC_PIDTBSDINFO)`——实测单次 2.4µs × 500+ 进程 ≈ 1.2ms/拍直接消除（proc_pidpath 与 rusage 按需保留）。进程表在采样间增长时缓冲重试一次
- **token 戳比对在稳态生效**：`isFresh` 阈值恰等于轮询间隔（60s）时，下一拍 age 恒 ≥ 60 → 条件永不成立，「戳未变跳过重查」的短路设计永久空转（每拍 4 次 stat）。加 5 秒宽限吸收定时器抖动，稳态（文件戳未变）跳过全表聚合
- 基准实测：snapshot 均值 4.1ms；测试 184/0 ×3；probe 状态判定形态一致

## [0.0.41] - 2026-09-12

### 🔤 字号统一：徽标字号令牌（v3 战役 · R24）

- **徽标字号令牌 `badgeFont`**：monospaced 9pt 统一入口——此前同一「Token 徽标」语义散落 8/9/10pt 三种尺寸，且 <9pt 低于 HIG 最小可读字号、高分屏缩放下可读性差
- 14 处 <9pt 文本全部提升：Agent 行 Token 徽标、工作台异常标签、实时流水徽标、悬停卡状态、环看板副标题、事件横幅 chevron（图标 7 → 8）
- 行高无回归：徽标不是行内主导高度元素（名称 12pt 主导），IslandMetrics 校准常量与布局哨兵全部通过；测试 184/0

## [0.0.40] - 2026-09-12

### 📐 面板几何：菜单栏振荡修复与空态高度校准（v3 战役 · R23）

- **菜单栏内不再触发展开振荡**：top 贴边的展开热区此前只有下界（y ≥ max-18）无上界——菜单栏内整条 x 跨度都在热区里，而菜单栏在面板 frame 之外：光标停菜单栏触发展开 → 「不在面板内」0.5 秒后收起 → 热区仍命中再展开……无限振荡。热区补上界钳制（y ≤ max），菜单栏区域由 R11 的点击穿透正确处理
- **空态高度常量实测校准**：`emptyStateHeight` 87 → 121。依据：真实 SwiftUI 空态（图 + 文案 + 「打开偏好设置」按钮）理想高 185pt − chrome 64pt；R14 加按钮后未回校，一直靠 fittingSize 兜底不裁切，但常量失真让列表/空态的静态推导不可信
- 测试 184/0（空态高度断言同步校准值）

## [0.0.39] - 2026-09-12

### 🧭 口径与校验对齐（v3 战役 · R22）

- **自定义 Agent 前缀族冲突双向拦**：新增校验此前只做精确比对——内置 `codex` 已启用时添加自定义 `codex-helper` 会放行，而匹配器是「相等或词边界前缀族」口径，同一进程会被两个 profile 同时命中（监控数字翻倍、CPU 双报），表单错误文案承诺「会重复计数」却拦不住。抽共享判定 `hasPrefixFamilyConflict`（相等或互为 name+分隔符前缀，大小写归一），UI 校验与匹配器单一事实源
- **激增阈值非档位值吸附**：设置页 Picker 固定五档、自愈口径是连续区间——外部 `defaults write` 或旧版遗留的非档位值会让选项空白而引擎仍按未知阈值告警。打开设置页时吸附到最近档位
- **bundle 扫描并入 ~/Applications**：宿主 GUI 按用户安装在此处时，「已安装」标记与宿主内嵌组件过滤（避免同一程序显示两条）双双失效
- 测试 183 → **184**

## [0.0.38] - 2026-09-12

### 🛡️ 用户数据防丢失（v3 战役 · R21）

- **自定义档案不再「整批消失」**：存档是整条 JSON 数组、解码全有或全无——单个元素损坏（缺键 / 类型漂移）会让全部自定义 Agent 凭空消失，且此后任一次添加/删除会以空基线覆写坏数据，造成**静默永久丢失**。现在逐元素容错：坏元素丢弃并记日志（AppLog），其余原样救回
- **启停集损坏只读降级**：`enabledAgents` 存档损坏此前被并入「无记录」，会被首次安装分支覆写——用户全部启停选择一次性静默丢失。新增三态读取（无记录 / 正常 / 损坏），损坏时按默认启用集运行但**绝不写回**，损坏存档保留可恢复原状（修复数据后重启自愈）
- **knownAgents 口径统一**：用户「全部关闭」时不再丢弃 registry 外的历史已知项（与正常分支同口径）
- 测试 180 → **183**（坏元素抢救 / 损坏存档防覆写 / 历史 known 保留）

## [0.0.37] - 2026-09-12

### 🏁 20 轮优化收官：文档全量校对与终验（20 轮优化 · R20）

v0.0.18–v0.0.37 共 20 轮（R01–R20），由四路独立侦察（性能 / 健壮性 / UI 交互 / 质量工具链，约 56 条发现）驱动，每轮走「开发 → 修复 → 测试全绿 → 独立验收 → 提交 → 发布 → 验证记录」闭环，每轮独立验收（两处验收 FAIL 打回重做：R09 浮层类名、R11 穿透谓词，均以活体实证定位）。

#### 本轮（R20）内容

- **README 全量校对**：9 处历史事实漂移修正——CPU 判定阈值（1% → 6% 含桌面类下限）、内置档案数（12 → 17）、扫描节流（15s → 3s）、闲置降频（15s → 5s，两处）、卡宽（280 → 330）、用例数（163 → 179）、target 数（3 → 4）、进程枚举方式（ps → libproc）、阴影描述（已停用）
- **终验**：全量测试 180/0 连跑 3 次；--selftest 全过；--probe 功能等价；收起态 CPU 0.2%（全离线空载）；红线核查——采样节律零改动（scheduleNext/间隔全 diff 定性）、零第三方依赖、提交信息与代码注释无环境指纹

#### 战役总账（0.0.17 → 0.0.37）

- **崩溃与误杀**：3 个 P0 崩溃路径（越界值/开库泄漏/事件 id）在 0.0.17 已修；本轮再修 PID 复用误杀整棵进程树、collapseDelay 脏值「收起必崩」、孤儿误杀 launchd 服务、时钟回拨卡死 working、浮层按钮不可用等 **5 类可真实触发的高危缺陷**
- **测试**：159 → 180（+21），且获得：登记制测试卫生（退出后零 plist 残留自守护）、版本哨兵（三处版本漂移即红）、变异验证过判别力的用例
- **性能**：opencode 探测 SQL 8.1k part 会话 1.3–2.6ms → 0.002ms；采样热路径去重复计算（匹配/用量快照/扫描判定）；展开态每拍整卡重绘消除；永续 Timer 降频
- **架构**：SQLite 只读层统一（连接缓存）、IslandView 1168 → 513 行、吸附计算单实现、注册表分发
- **工程**：发布脚本版本自动化 + 测试门禁 + 版本哨兵、统一日志通道（发布版可观测）、验证记录入库
- **已知限制 / 下一战役候选**：菜单栏内细条上方光标致 expand/collapse 振荡（既有几何）；<9pt 字号统一（需可视化验收配合）；VoiceOver 全流程人工验收；空态高度常量已低估（兜底机制保证不裁切）；TokenUsageMonitor isFresh 阈值语义（S9，低值）

---
## [0.0.36] - 2026-09-12

### 🧰 工具链与发布强化（20 轮优化 · R19）

- **打包脚本版本自动化**：版本号此前在 build-app.sh 两处手工硬编码（历史上 README 硬编码 1.7.9 导致关于页显示错误的教训）——现在从 CHANGELOG 首条版本自动抽取（`./scripts/build-app.sh 1.2.3` 可显式覆盖），并在打包前校验 README 功能版本一致，漂移即拒绝
- **测试门禁**：打包前强制全量测试（176+ 用例），`SKIP_TESTS=1` 可跳过供快速冒烟——「未测试即发布」不再可能
- **布局哨兵脚本加固**：test-token-layout.py 失败时打印编译器 stderr（此前被吞只剩退出码）；链接前先 `swift build` 确保产物新鲜（此前会把过期对象当验收依据）
- **ADR-0003 补状态注记**：决策 4 的 ShadowHostView 投影已被实现取代（阴影停用），1-3 仍有效——不再让过期 ADR 误导后来者
- **验证记录入库**：docs/validation/ 从 .gitignore 移出——CHANGELOG 的性能数据从此在仓库内有出处（R01–R19 每轮验证记录随本版入库）

## [0.0.35] - 2026-09-12

### 📡 统一日志通道与调试指南落地（20 轮优化 · R18）

- **新增统一日志出口 `AppLog`**：发布构建里数据源异常此前散落在 print/debugPrint（全部不可见）——SafeNumber 钳制告警、SQLite prepare/step/open 失败、自启动注册失败现在统一走 os.Logger（`log show --predicate 'subsystem == "com.agentisland.app"'` 可查）；Probe/Selftest 的表格输出是 CLI 交互目的本身，不经过此通道
- **调试指南真实可用**：README 承诺的 `AGENTISLAND_DEBUG=1` + `/tmp/agentisland.log` 此前指向不存在的功能（代码无任何读取/写入）——现在开启开关即镜像全部诊断日志到该文件，并补充系统日志替代命令
- 镜像落盘有守护测试（环境开关 + 文件内容断言）；测试 179 → **180**

## [0.0.34] - 2026-09-12

### 📊 数据源消失终态与版本哨兵（20 轮优化 · R17）

- **数据源已消失不再显示陈旧统计**：dim/opencode 库文件被删除后，面板此前会永久显示最后一次成功值且无任何迹象（「查询失败保留旧值」契约只覆盖瞬时失败）。现在主库文件连续 3 拍缺失（约 3 分钟）即置空该源；单拍缺失（原子替换的短暂空窗）仍保留旧值——「数据库暂时缺失保留旧统计」契约不变。实现首版的字符串匹配会误判（组合戳里 `-wal:missing` 是合法形态），已改为主库文件存在性判定
- **版本哨兵测试**：CHANGELOG 首条版本、README 功能版本、build-app.sh 的 Info.plist 版本三处一致性入测试（历史上三处漂移导致关于页/打包/文档各说各话）；验收以变异实验证实精确报红
- **解码容错测试**：自定义档案存储损坏（坏 JSON / 空数据）回落空集
- postEvent 保留 public（LayoutRegression 目标普通 import 会破坏，偏离已落档）；测试 176 → **179**

## [0.0.33] - 2026-09-12

### 🧪 测试卫生：plist 泄漏根治（20 轮优化 · R16）

- **测试套件 plist 泄漏根治**：12 处测试散点创建 UserDefaults 套件并各自清理——`removePersistentDomain` 只解除注册，cfprefs 在进程退出 flush 时把已删域重建为 plist 文件，`~/Library/Preferences` 逐次累积（实测 2400+ 个）。改为 TestDefaults 登记制：统一清理（移除持久域 + 删文件双保险）+ 泄漏自守护断言 + 退出后独立进程兜底清扫（应对 cfprefsd flush 竞态，3 秒重试）——连跑 3 轮零残留，历史残留一次清零
- **注册表持久化可注入**：`loadCustomProfiles` / `saveCustomProfiles` / `fullRegistry` 增加 `defaults` 参数（默认 standard，向后兼容），自定义档案测试不再经 standard domain 污染后续用例；顺带修复 RegistryTests 中 `suite.description` 笔误（恒 no-op 的清理调用）
- **环境依赖用例加固**：真实引擎采样在进程表不可读（沙箱/CI）时显式跳过；清理用例的进程启动失败不再被 `try?` 静默吞掉
- 测试 176/0 连跑多次；独立验收 pass

## [0.0.32] - 2026-09-12

### 🏗️ 结构拆分：IslandView 按类型归位与吸附计算单实现（20 轮优化 · R15）

- **IslandView.swift 1168 → 513 行**：AgentRowView / TokenSummaryBar / DockedSliver（含脉冲动画）/ EventBannerView 纯搬移拆出（字节级一致、零逻辑改动）；卡内导航枚举留在原位（测试漂移哨兵按文件路径解析）
- **吸附计算双份收敛**：placeWindow 与拖拽吸附各自维护的「clamp 锚点 + 目标 origin」几乎逐行相同——收敛为 `dockTargetFrame` 单实现（行为逐行等价核对），改贴边行为不再需要同步两处
- **阴影宿主清理**：停用的 ShadowHostView no-op 类删除，注释与文档去引用（ADR-0003 的过时表述在 R19 统一补注记）
- 布局回归哨兵（test-token-layout.py）PASS；测试 176/0；独立验收含字节级纯搬移证明

## [0.0.31] - 2026-09-12

### ♿ 无障碍与键盘可达（20 轮优化 · R14）

- **Agent 行与流水行的 VoiceOver 语义**：行 label 补状态与动作提示（「Claude，工作中，点按查看详情」），内嵌终止/流水/直达图标按钮补显式 label（此前 VoiceOver 读的是 SF Symbol 默认名）；流水事件行补 isButton trait 与「展开/收起事件详情」hint，且不设显式 label 让合并语义保留徽标/时间/正文
- **菜单栏 popover 图标菜单补 label**：外观主题与通知策略两个 icon-only 菜单此前读出的是符号名，现在播报「外观主题，当前 深色」式现值
- **空态加「打开偏好设置」入口**：覆盖「全部禁用」与「全部离线超 24h」两种成因，不再只有一句「没有活跃的 Agent」
- **Esc 收起弱回退**：面板（或本 App 其他窗口）持有键盘焦点时按 Esc 收起展开卡；不吞事件，点击外部/光标离开仍是主路径
- 已知限制：实际 AX 树物化需辅助技术客户端，VoiceOver 全流程人工验收待补；测试 176/0

## [0.0.30] - 2026-09-12

### 📝 文案口径统一（20 轮优化 · R13）

- **时长文案三处收敛**：引擎完成事件 / summaryText 兜底 / 系统通知各自维护「N分M秒」实现且舍入口径不一（截断 vs 四舍五入，同一事件可能显示 59秒 或 1分0秒）——收敛为 `AgentTaskEvent.durationText` 唯一实现（四舍五入 + 负值钳 0），口径断言入测试
- **内存文案第 4 份漏网实现收编**：工作台「可回收」格的私有实现把 <1MB 显示成「0M」（0.0.17 收敛时的漏网点），改走统一的 `MemoryFormat.text`
- **口径对齐**：菜单栏 popover「N 在线」改为「N 可见」——可见口径 = 在线或 24h 内有活动，对离线但有近期活动的 Agent「在线」是错误陈述；流水页副标题中英混排改「实时流水 · N 条事件」并补加载态；提示音描述补全「完全静默模式下不发声」的行为承诺
- 测试 176/0；独立验收 pass（MemoryFormat.text(0) 占位符语义核实、加载态首帧时序核实）

## [0.0.29] - 2026-09-12

### 🎨 主题可读性与死代码清理（20 轮优化 · R12）

- **流水徽标双值动态色**：`thinking` 紫（固定系统紫，浅色白玻璃 ≈3.2:1）与 `toolCall` 蓝（深色玻璃 ≈2.6:1）改为双值动态——实算对比 thinking 浅色 5.79:1、toolCall 深色 4.54:1，均达 WCAG AA
- **熔断红环语义补全**：`isHung`（≥70% CPU 持续 5 分钟，与「疑似卡死」徽标/死锁扫描同源）的 Agent 水位环呈极光红——此前四级色标的红色从未接线，熔断中的 Agent 环上看不出严重度
- **死代码批删 18 个符号**：TooltipTail Shape（从未接线的几何小尾巴）、数值徽标参数、未使用的几何常量与 Binding、公开 API 僵尸（ProcessMatcher.cpuPercent）、Theme 13 个零引用令牌——全部逐一 grep 零引用后删除
- 顺带清零 R09 引入的两条编译警告；全仓重编译警告保持 0
- 声明偏离：<9pt 字号统一延后（影响 IslandMetrics 实测校准常量，需可视化验收配合）；测试 176/0

## [0.0.28] - 2026-09-12

### 🎛️ 面板状态机：连发事件 peek 重排、点击穿透与唤醒帧重同步（20 轮优化 · R11）

- **连发事件 peek 重排**：peek 进行中来了新事件时取消旧展示按新事件时长重排——此前单任务槽会把 costSpike（6 秒）的展示时长吞成前一 completed（3.5 秒）的剩余时间，最严重的告警可能只 peek 不到 1 秒就缩回
- **拖拽中高度自适应让位**：拖拽由 WindowServer 驱动，采样导致的高度自适应动画不再与之争夺窗口 frame；松手吸附时兜底终态
- **收起态幽灵命中区修复**：透明面板的透明区域仍参与鼠标命中——top 贴边时收起窗口自菜单栏向下延伸整卡高度、right 贴边有一列竖向盲柱（活体实测：细条外 213pt 处点击被面板吞掉）。现在光标在细条可视矩形 ±8pt 内才接收事件，其余全部点击穿透（hover 展开走全局光标判定不受影响；零窗口几何改动，动画零回归）
- **睡眠/唤醒帧重同步**：显示器睡眠期间 CA 动画挂起可能让窗口 frame 停在动画起点（与状态机脱钩）；睡眠期直落终态帧，唤醒（含熄屏唤醒，经 NSWorkspace center）后自动重放正确帧——活体 E2E 实证人为破坏 frame 后唤醒自动恢复
- 独立验收两轮（首轮活体测试打回两项实现缺陷，复验含 E2E 全过）；测试 176/0

## [0.0.27] - 2026-09-12

### 🧰 工作台与导航正确性（20 轮优化 · R10）

- **清理复核不再劫持导航**：「一键清理」的结果复核晚 1.2 秒到达，期间用户若已进入详情/流水页，旧逻辑会把页面强行拽回主列表。现在复核回调校验当前路由——不在工作台只旁观；按 id 差集归因「未能终止」，复核窗口内新升温的异常条目不再记到本次清理头上
- **工作台扫描与引擎彻底隔离**：此前共用同一进程快照源，工作台扫描会消费引擎下一拍的 CPU 差分窗口（实测可把全部进程 CPU 读数归零，仅靠 CPU 信号维持 working 的 Agent 会瞬时抖动）。现在工作台用独立快照源（静态长驻 + 后台预热两拍建立基线，冷启动不再漏报死锁行）
- **快照差分窗口原子化**：`snapshot()` 全程持锁——并发快照（引擎采样 / 工作台扫描 / 终止前身份复核）互不消费对方的差分窗口；新增 6 线程并发竞态冒烟测试
- **直达按钮失败有反馈**：ssh/tmux 启动的 CLI 没有可激活窗口，此前点击「直达」完全无反应（activate 返回 false 被忽略）。三处入口（Agent 行 / 事件横幅 / 悬停卡）失败时按钮切换「未找到窗口」1.5 秒
- 测试 175 → **176**；独立验收 pass（含并发锁死锁论证与真实 probe 对照）

## [0.0.26] - 2026-09-12

### 🖱️ 交互冲突修复：hover 浮层与菜单栏 popover 不再被收起打断（20 轮优化 · R09）

两个由活体实证（lldb 附着 + 辅助功能开窗 + 进程内逐字谓词执行）定位的 P1：

- **tooltip 弹窗里的按钮实际点不到**：悬停卡（SwiftUI popover）渲染在面板 frame 之外，鼠标移入即被判定「离开面板」，0.5 秒后自动收起并关闭弹窗——终止两段式确认与直达按钮只有半秒可用窗口。现在光标在面板**或其派生浮层**内都视为「在面板内」
- **菜单栏 popover 与点击收起互相打架**：展开态下点击菜单栏 popover 的「收起侧边栏」，点击监听先行收起、按钮 action 再翻转——按钮点成展开；popover 内点 Agent 行下钻的详情页在保护期过后一闪而过。本地点击监听改为延迟一拍复核（守卫含 popover 导航 3s 保护期与浮层判定）
- 浮层窗口按活体实证的类名稳定片段识别（`_NSPopoverWindow` / `MenuBarExtraWindow` / `NSStatusBarWindow`），Apple 改名时行为退化为修复前；peek 结束守卫同步收口
- 验收官端到端实证：保护期恒过期的 hover 展开 + 光标驻留菜单栏 popover 90 秒恒保持展开（旧版首个检测拍即计划收起）；光标移开 3 秒内正常收起（无粘滞）
- 已知边界：MenuBarExtra 无公开 dismiss API，「下钻后主动关弹窗」以保护期替代；鼠标级端到端待屏幕可用后人工复认

## [0.0.25] - 2026-09-12

### 🖥️ UI 渲染开销：展开态高度签名去重与设置页缓存（20 轮优化 · R08）

- **展开态不再每拍全卡重绘 + 全量排版**：引擎每拍发布（快照的 CPU/活动时间/动作逐拍变化），面板此前对每拍无条件执行 `needsDisplay` 整卡重绘 + `fittingSize` 全量 SwiftUI 排版（估 1–3ms/拍）。现在按「高度影响签名」（route / 可见数 / 汇总栏 / 环架 / 事件 id / 横幅展开）去重——签名不变时两者都跳过；SwiftUI 由 `@ObservedObject` 自行失效，逐拍数据刷新不受影响。收起态细条的显式失效保留（防回退 0.0.17 前的「细条不刷新」）
- **订阅去 Task 跳跃**：引擎 @MainActor、发布在主线程，每拍一次 Task 分配 + 调度不再需要
- **边缘检测兜底 Timer 0.06s → 0.12s**：hover 展开的主路径是 0.05s 节流的鼠标事件通道，Timer 只是兜底；16.7Hz 的永续唤醒阻止主 runloop 深度 idle（能效影响大于 CPU% 影响）
- **设置页档案列表缓存**：设置页开着时引擎每拍触发 body 重算，内置列表（fullRegistry 的 loadCustomProfiles：UserDefaults 读 + JSONDecoder 解码）与自动发现列表（PATH / Applications 扫描）此前逐拍重跑——按安装扫描版本缓存（引用语义缓存盒）
- 测试 175/0；独立验收 pass（签名与 expandedHeight 六输入逐一完备对照 + docked 呼吸灯机制论证）；`--selftest` 全过

## [0.0.24] - 2026-09-12

### ⚡ 采样热路径性能：去重复计算与防退化护栏（20 轮优化 · R07）

- **进程匹配热路径去分配**：`matchesProcessNames` 位于「条目数 × 档案数」≈ 8.7k 次/拍的调用路径上，此前每条目重复 `lowercased()` 分配副本——Entry 构造时已恒小写，契约写入注释后直接比对（估 0.3–0.8ms/拍）
- **dsh 命令行探测 10s TTL 缓存**：node 进程多的开发机此前每拍对每个 node 候选做一次 KERN_PROCARGS2 sysctl（args+env 整块拷贝解析），命中后 inspectDSHAction 还会对同一 pid 重复一次。现在匹配器与动作探测共用 TTL 缓存（pid 消失清残留、512 条容量上界、负结果不缓存）
- **opencode 探测 SQL 两步化**：原单条 SQL 的相关子查询对 part 按 time_updated 排序（无索引 → 临时 B 树，随活跃会话 part 数线性退化，万级 part 实测量级 1.3–2.6ms/拍全在主线程）。改为 session 最新行（百行级）+ part 按 rowid DESC 定位尾行；真库 EXPLAIN 验证临时 B 树消失，最大会话（8,108 part）实测 0.002ms
- **token 用量快照每拍一次**：引擎每拍 17 profile × 2 处共 34 次「锁 + 全字典拷贝」收敛为 1 次，告警链路复用同一快照
- **扫描热路径去分配**：忽略判定从「切分整条 pathComponents + 逐组件小写化」改为条目 basename 单次小写化（被忽略子树在其目录条目处已被剪枝，语义等价）；心跳判定先判扩展名再计算 stem。全量扫描为 working 态每 5–6s 一次的最重 I/O 路径
- 测试 174 → **175**：node_modules 深层「未来 mtime」判别用例锁死剪枝等价性；独立验收含真库 EXPLAIN 对照与结果逐字节等价核对

## [0.0.23] - 2026-09-12

### 🗄️ SQLite 只读访问层统一：连接缓存与样板收敛（20 轮优化 · R06）

- **新增 `ReadonlyDB` 共享层**：此前 5 个探测器 + 5 个 DB 流水源各自手写 open/prepare/finalize/close 样板，且每拍对大库（数百 MB）现开现关——重复支付 open 成本（~50–150µs/次）并抖动文件缓存。现在统一收口：连接按路径缓存长驻复用（页缓存温热），(设备号, inode) 校验外部替换，open 失败关句柄（0.0.17 泄漏契约延续），缺失文件走 stat 快路径
- **样板收敛**：10 处 open/defer-close 收口后全仓净减 ~400 行重复；标题截断 4 连抄收敛 `clipTitle`；`inspectAction` 的 9 分支 if-else 链改 switch 分发；`AgentLogStreamer.openReadonly` 删除
- **并行执行**：本轮由两个独立执行者分别完成 Inspector 与 Streamer 文件（文件独占无写冲突），主会话建共享层并统一构建；独立验收做了 dim SQL 字面量逐字节核对、真实库行为对照（探测器结果与 sqlite3 CLI 直查一致）
- 泄漏回归测试改测 ReadonlyDB：chmod 000 文件命中 open 失败分支（实测 rc=14、handle 非 NULL），2 万次失败开库内存零增长
- 测试 174/0 连跑 3 次；`--selftest`、`--probe` 全过

## [0.0.22] - 2026-09-12

### 🔍 动作探测门控与会话树枚举加固（20 轮优化 · R05）

- **idle 状态不再做主线程全树枚举**：动作透传（「正在执行 xxx」）此前对每个 running Agent 每 2s 调用一次，`idle` 拍同样枚举 `~/.claude/projects` 等会话目录树（重度用户数千~数万文件，实测量级 5–20ms/次），结果因等级判定在下才被整体丢弃——先算后丢，纯浪费的主线程 I/O。现在探测移到等级判定之后、仅 `working`（含滞回期）执行；working 拍代码路径零变化，idle/offline 拍严格减少工作
- **会话树枚举防失控**：`findNewestFile` 两份逐字相同的私有实现收敛为 `LogTailReader.newestFile` 单实现，新增两项防护——符号链接目录跳过整个子树（防循环），20000 条目预算（防失控枚举长时间占用主线程；超预算优雅退化为「无动作」/「空流水」而非卡死）
- **claude 流水事件 id 稳定化**：事件时间戳此前取 `Date()`，派生 id 拼接时间戳后每次刷新全部改变，稳定 id 的目的（详情展开态保持、增量刷新）对 claude 完全落空。现在优先解析行内 JSON 的 `timestamp` 字段，缺失回落文件修改时间
- 修复过程中发现并记录一个 `NSDirectoryEnumerator` 陷阱：对文件符号链接调用 `skipDescendants()` 会破坏枚举器状态、丢弃后续所有条目——`skipDescendants` 只能对目录调用，与 `FileMonitor.scanTree` 的既有写法一致
- 测试 171 → **174**：idle 不探测（注入计数 hook）、循环符号链接树秒级返回、maxAge/预算边界；独立验收 pass（性能量级声明核对、行为等价逐 case 论证）

## [0.0.21] - 2026-09-12

### 📜 实时流水：事件 id 去重、超长 detail 截断与解析开销（20 轮优化 · R04）

- **稳定事件 id 可碰撞**：同一消息内的多条 part（如连续两次同名 `tool_use`）共享同一 `createdAt`，派生 id「agentId-毫秒-标题」完全相同——该 id 同时是流水列表 ForEach 身份与展开态键，重复时触发 SwiftUI 未定义行为（行互相顶替、展开态错乱）。现在批次内去重：第 2+ 次出现追加序号（取「下一个可用序号」，标题含 `#` 的再碰撞形态也防住），首次出现不变，出现次序由数据行序决定、跨刷新稳定
- **64KB 原始 JSONL 不再整段渲染**：Claude 流水把 `readLastLines` 的最长 64KB 单行原样塞进 detail，dim 的整段 parts JSON、完整消息文本同样无上界——超大 `Text` 在 330pt 卡内一次性排版造成可感知卡顿，且原始 JSON 对用户无诊断价值。现在在唯一构造点统一截断（4096 字符 + 「…[已截断]」标记），完整内容本就无法在卡内滚动查看，截断无损
- **ISO8601DateFormatter 静态化**：初始化 ~0.5ms/个，流水页每 2s 刷新一次，antigravity 每次甚至新建 2 个（0.0.17 已在 TokenUsageMonitor 静态化同类对象，此处为漏网点）
- 测试 169 → **171**：去重语义矩阵（首现不变 / 序号后缀 / 异名不受影响 / 重复调用稳定 / 基础 id 自带 `#` 的再碰撞）、截断边界（含显式 id 路径无旁路）；独立验收 pass（dim 行级截断的整组进出边界、detail 消费点全链路核对）

---

## [0.0.20] - 2026-09-12

### 🧯 脏持久化防护：配置值全字段自愈，收起延迟崩溃循环修复（20 轮优化 · R03）

- **收起延迟脏值导致「收起必崩」循环**：`collapseDelay` 的 init 读取路径没有钳制，`defaults write` 写入负数或超大值（或 plist 损坏）后，`UInt64(delay * 1e9)` 转换对负值/溢出直接运行时 trap——鼠标每次离开展开卡都会触发收起调度，应用表现为每次收起即崩溃。现在 init 读取即钳入 `0.2…5s`（与设置页滑杆同口径），NaN 归位默认值
- **EngineConfig.normalized() 补齐全部字段**：此前只钳 3 个字段，「工作写入窗口」「活跃会话窗口」的脏负值会让文件信号通道与活跃会话计数整体静默失效（监控半残且无提示）。新增 6 个区间常量与设置页滑杆对齐，作为脏值自愈唯一口径；`minWorkingHold` / 死循环阈值 / 告警阈值虽不暴露也一并设防
- **NaN 防御**：`min/max` 对 NaN 透传（比较恒 false），先归位默认值再钳制，覆盖全部 8 个 Double 配置字段
- **停靠锚点 isFinite 防御**：损坏的 `dockAnchorX/Y`（NaN/Inf）会让窗口落在可见区域外且无法拖回，读取时拒绝并回落屏幕中心
- `EngineConfig.load` 注释声明三个非持久化字段的守则，防「设置实效」类缺陷复发（0.0.17 修过的 A1 同型埋雷）
- 测试 166 → **169**：补齐钳制矩阵、NaN 自愈、收起延迟区间（UI 侧接线由独立验收代码评审守护）；`--selftest` 全过；独立验收 pass

---

## [0.0.19] - 2026-09-12

### ⏱️ 引擎时间正确性：时钟回拨防御 + 离线速率基线清理（20 轮优化 · R02）

系统时钟被回拨（NTP 阶跃校正 / 手动调整 / 虚拟机恢复快照）后，引擎全部墙钟锚点的差值都会失真。本轮以「回拨安全的墙钟」替代单调时钟全量迁移（理由：`sample(now:)` 测试 seam 全仓 49 处、ADR-0001 守护采样节律内聚不动；每拍 O(1) 重锚达成同等防御，时钟大幅前跳由既有 resumeGap 断点检测兜底）。

- **回拨后 Agent 永远卡在「工作中」**：`now - lastSignal` 恒为负 → 恒小于滞回时长 `minWorkingHold` → working 判定永不回落，面板/peek 持续显示工作中直到真实时间追平回拨量。现在每拍检测「锚点晚于本拍」即重锚，滞回与任务时长在当前时钟下重新计起（`workingSince` / `lastSignalAt` / `highCpuSince` / `lastRunawayAlertedAt` 四锚点 + token 速率基线时间戳）
- **未来 mtime 负值钳制**：回拨后旧会话文件的 mtime 落在「未来」，负的经过时长不再透传给 `lastActivityAgo` 消费方（UI 文案与下游算术），钳为 0（视作刚写入——回拨前的真实写入确实发生在不久前）
- **offline 清除 token 速率基线**：与 resumeGap 断点口径对齐。此前进程退出后基线时间戳冻结，重启后首个结算窗口把离线全程计入分母，token 激增告警的速率被摊薄
- 测试 163 → **166**：回拨重锚后滞回正常过期（变异验证：删掉防御代码该用例即红）、未来 mtime 钳 0 且随真实时间正常过期、offline 清基线不变量；TestKit 新增 `FakeTokenUsageProvider`
- 全量 166/0 连跑多次无 flaky；`--selftest` 全部通过；独立验收 pass（含变异实验判别力验证）

---

## [0.0.18] - 2026-09-12

### 🛡️ 进程终止安全：身份复核防误杀 + 孤儿进程佐证门槛（20 轮优化 · R01）

本轮起按「20 轮计划」逐轮深度优化（性能 / 健壮性 / UI 交互 / 质量工具链四路独立侦察 → 定案 → 实施 → 独立验收 → 发布）。R01 聚焦全仓唯一会终止用户进程的两条路径：熔断逃生舱与工作台清理。

#### 身份复核：PID 被回收复用后不再可能误杀无关进程

- **熔断横幅的陈旧 PID 陷阱**：告警事件携带的 PID 来自事件产生时刻，用户可能数分钟后才点击「熔断」；期间目标进程若已退出，macOS 可能把同一 PID 分配给完全无关的程序——旧实现直接向该 PID 及其整棵进程树发送 SIGTERM（300ms 后仍存活补 SIGKILL），后果是把无关应用连同其全部子进程整体终止
- 三层防线（各有独立回归测试，删除任一层测试即失败）：
  1. 引擎 `terminateAgent` 终止前取**最新进程快照**，核对 PID 仍匹配该 Agent 的进程；不匹配按「进程已退出或已变更」拒绝，不发送任何信号
  2. `ProcessTerminator` 增加 `expectedPath` 复核：kill 前用 `proc_pidpath` 比对 basename（brew 升级导致的路径整体变化视为同一程序），不一致立即放弃——闭合「快照到 kill 之间」的最后竞态窗口
  3. 新增 `TerminationOutcome` 区分「已终止 / 身份不符 / 发送失败」，失败事件如实归因，延续 0.0.17 的「不谎报成功」口径
- 工作台清理同样携带扫描时记录的 `commandPath` 复核：工具箱列表可能已陈旧，同一防线覆盖单条与批量清理

#### 孤儿进程：不再批量误杀 LaunchAgent 托管的常驻服务

- **问题**：孤儿判定为「PPID==1 且非标准 App 主进程」，但 macOS 上 launchd（PID 1）同时也是一切 LaunchAgent / 登录项的父进程——用户刻意后台化的智能体（`launchctl` 托管的服务型 Agent）每次扫描都会被列成「孤儿进程」并进入一键批量清理，杀掉即任务静默丢失
- **修复**：① 近期仍有会话写入的 Agent（10 分钟窗口，复用引擎活动数据）不再报孤儿——它正在产出工作，不可能被遗弃；② 其余孤儿仍会列出（原因文案如实说明），但只允许**逐条确认清理**：一键批量按钮按可批量条目（死锁 / 内存超限）计数与禁用，复核反馈不再把孤儿计入「未能终止」
- 策略取向「宁可漏杀」：死锁与内存超限证据充分，不受活动佐证豁免

#### 测试

- 用例 **159 → 163**：身份不匹配时拒绝且目标进程存活、同名异径（brew 升级形态）放行、消失 PID 返回 failed；孤儿佐证矩阵（有活动不报 / 无活动报但不进批量 / 死锁不受豁免）
- 引擎成功路径夹具同时独立验证引擎侧成员核对与终止器侧路径复核两层防线
- 全量 163/0 连跑 3 次无 flaky；`--selftest` 全部通过

---

## [0.0.17] - 2026-09-11

### 🩺 全面深度优化：修复 3 个可触发崩溃、状态误判与性能瓶颈

本轮由多轮实测审查驱动，覆盖健壮性、状态判定、设置实效、性能、交互安全与测试。

#### 崩溃与资源泄漏（均可被真实数据触发）

- **越界 Token 值导致整个应用崩溃**：`Int(row[0]) ?? Double(row[0]).map { Int($0) } ?? 0` 的兜底分支自身是陷阱——`Int(Double)` 对越界值 / `Infinity` / `NaN` **直接 fatalError**。数据源一旦出现 `1e19`、`1e999`，或两行 `1e308` 相加使 `SUM` 溢出为 `Inf`，展开面板时进程即 trap（实测退出码 133，用户表现为「侧边栏一打开就消失」）
- **实时流水页事件 id 同一问题**：`Int(timestamp * 1000)` 遇库字段中的 Int64 上限哨兵值同样崩溃
- 新增 `SafeNumber` 饱和解析：越界钳制到量级上限并输出告警，`Inf`/`NaN` 归零，替换全部裸转换
- **SQLite 句柄泄漏**：`sqlite3_open_v2` 失败时仍会分配 handle（实测约 1.5KB/次），但 `defer { close }` 写在 `guard` 之后到不了。这些探测在主线程按采样节律反复执行，库缺失/不可读时约 **25–61MB/天** 常驻内存增长。10 处失败路径补齐关闭

#### 状态判定与设置实效

- **「CPU 判定阈值」此前对 13/17 个内置 Agent 完全无效**：引擎按 id 硬编码 20%/35%，只有 4 个纯 CLI 读用户设置，滑块形同虚设。阈值改为随档案下沉（下限语义，取 `max(下限, 用户设置)`），既让设置对所有 Agent 生效，又保留原硬编码防住的「桌面类空闲抖动误判」
- **睡眠/挂起后误报「任务完成 (480分0秒)」**：完成事件判定只有 3.5 秒下限、没有上限，合盖唤醒后的第一拍会补发完成事件并弹系统通知。新增采样断点检测（`max(120s, 3×闲置间隔)`），断点后跳过完成事件并重置工作区间
- **关闭「死循环告警」会连带关闭卡死检测**：高负载采集原先写在开关分支内，关闭后 `isHung` 恒 false，行内「疑似卡死」徽标、详情页状态与工作台死锁扫描同时静默失效。采集移出开关、无条件执行
- **严重告警被普通事件挤掉**：`latestEvent` 是单槽位，实测 Token 激增横幅几秒内就被其他 Agent 的「任务完成」顶掉，用户来不及处置。新增统一发布入口，告警后 30 秒内普通事件不覆盖
- **清理失败却谎报成功**：「已安全清理 0 个异常进程…系统资源已就绪」在一个都没杀掉时照样弹出。现在按结果区分文案，失败发「未能终止」提示
- 采样间隔钳入 `0.5–600s`：脏持久化值写成 0 时实测每秒采样 **2895 次**（忙循环）

#### 性能（实测数据）

| 指标 | 优化前 | 优化后 |
|---|---|---|
| 收起态 CPU（均值 / 峰值） | 2.55% / 12.8% | **1.91% / 7.8%** |
| 会话目录全量扫描 | 101 ms | **51 ms** |
| 主线程采样耗时 | 15.3 ms | **10.8 ms** |
| 实时流水页查询 | 161 ms | **2.67 ms** |

- 移除两个伪会话目录（opencode 配置目录含 `node_modules`、openviking 的 uv venv），二者合计占全量扫描 **57%** 的时间却与任务无关；忽略集补充 `node_modules`/`site-packages`/`.venv`/`__pycache__`/`.git`/`DerivedData`/`Caches`
- 进程匹配预计算小写路径：19 个 profile × 整张进程表此前每拍重复 `lowercased()`
- 告警链路复用主循环已算出的 CPU/PID，不再二次全表匹配

#### 交互安全

- **告警横幅「熔断」按钮单击即终止整棵进程树**，无二次确认（同一动作在列表行与工作台都有确认）。改为两段式确认，已端到端实测：首击后目标进程仍存活，二击才终止
- **系统通知绕过应用自己的通知策略**：「完全静默」下仍弹通知并响铃、「专注免打扰」下每次完成都响、标准模式下与岛内提示音叠加成双重音。现在按策略裁决投递，声音统一由岛内 `NSSound` 承担（不依赖通知权限）
- **浅色主题下多处内容不可见**：环形水位与 8pt 副标题在白底对比度仅 1.03:1，现改双值动态色（3.10:1，白底 5.08:1）；流水页与悬停卡的硬编码黑白改动态色
- 鼠标事件节流此前是死代码（每个 `mouseMoved` 都全量重算边缘判定并新建 Task），接入后全局监听的 Task 创建由约 60/s 降至 17/s
- 拖拽统一走原生 `performDrag`：删除闭包版入口与其失效的坐标钳制逻辑

#### 测试

- 用例 **102 → 159**，新增窗口高度全组合、清理规则矩阵、可见口径边界、配置归一化、内存文案一致性等
- 新增 `IslandMetricsKit` target 以符号链接纳入 UI 层纯几何文件，使「窗口高度 vs 子项之和」这组历史高频回归可被断言；配漂移哨兵，路由变化时用例会失败提示同步
- 全仓编译警告 1 → **0**

---

## [0.0.16] - 2026-09-10

### 🔔 完善任务通知、状态识别与告警展示

- **系统通知覆盖完整事件链**：任务完成、等待确认、Token 激增、持续高 CPU/疑似死循环和终止失败等事件均投递到 macOS 通知中心，并显式使用默认提示音与主动通知级别；事件 UUID 保证不重复发送。
- **DeepSeek Harness 状态及时跟随**：深层会话目录强制重扫周期收紧至 5 秒，避免常驻 UI 因目录缓存长时间停留在待机状态。
- **DimAgent 状态误报修复**：排除 `file-history` 与 `blobs` 编辑历史/附件缓存子树，避免没有实际任务时被后台同步写入误判为工作中。
- **告警文字可读**：展开页顶部直接显示告警摘要，展开原因可查看完整排查建议；Agent 名称和动作过长时单行省略，悬停显示完整内容。
- **完成通知声音修复**：通知内容显式配置系统默认声音，并修正通知正文插值。

## [0.0.15] - 2026-09-09

### 🐛 修复状态跟踪误报、Token 口径虚高、底部汇总栏裁切与贴边阴影

- **进程关闭不再误报「任务已完成」**：
  - 此前 `offline` 分支复用完成事件路径，把「进程被用户关闭」等同于「任务执行完毕」，手动退出 ChatGPT 也会弹出完成横幅
  - 现在进程消失只静默转 `offline`；完成事件仅由「进程仍在但工作信号消失」产生
- **Token 净消耗口径统一（dim 源）**：
  - `usage_ledger.usage.promptTokens` 含缓存命中部分，此前直接与 completion 相加导致缓存重复计入，累计用量被大幅虚高（数十倍量级）
  - 汇总、模型拆分、会话列表三处统一改为 `(prompt − cacheRead) + completion` 并逐行钳制非负，与 opencode 侧口径对齐
- **激增告警改为速率制 + 连续确认**：
  - 此前「300 秒内增量 ≥ 阈值」把长任务结束时一次性落盘的巨额 ledger 记录当成瞬时激增
  - 现在按每分钟净消耗速率判定，且需连续 3 个周期超阈值才告警；正常绘画/长任务不再误报
- **修复 peek 微弹窗导致侧边条错位**：
  - 此前 peek 只拉伸窗口 frame、`displayState` 仍为 `docked`，SwiftUI 只渲染 6pt 细条，于是细条被拉到展开位置且卡片内容缺失
  - 现在 peek 走真实 `displayState` 切换，与窗口尺寸同源，展开内容正常呈现
- **修复展开卡底部 Token 汇总栏被裁切**：
  - `IslandMetrics.expandedHeight` 漏算顶部「实时活动环微看板」（40pt 内容 + 1pt 分割线），渲染内容比窗口高出约 56pt，超出 460pt 上限的部分从底部裁掉，汇总栏只露出半行
  - 现在展开高度计入看板（新增 `chromeHeight`/`listHeight` 纯函数统一口径），触顶时压缩可滚动的 Agent 列表（保底一行）而非裁切汇总栏
  - 二次修复（首次展开正常、挪动后又被裁）：实测 `NSHostingView.fittingSize` 比常量推导值高 5.5pt——顶栏内容行高按 17pt 估算偏小（圆形图标按钮实际约 23pt），且汇总栏自身那 1pt 分割线未计入 `chromeHeight`。现按实测校准常量，并让窗口高度取「常量推导」与「内容理想高度」的较大值（`resolvedExpandedHeight`），`placeWindow`/`snapToDockEdge`/`syncExpandedHeight` 三处同源，常量再漏算也不会把汇总栏挤出窗口
- **移除面板阴影内渗，消除贴边侧上下暗带**：
  - 面板窗口与玻璃卡尺寸完全相同（330×447），AppKit 阴影没有卡片之外的落地空间，只会沿轮廓边缘向卡内渗入约一个模糊半径，在贴屏幕一侧的上下直角区域形成暗块（用户反馈的「两侧直角矩形的上下阴影」）
  - 现已停用面板阴影；一体化贴边的观感由玻璃卡自身的 1px 高光边缘与反向倒角承担
- **修复 ChatGPT 与 Codex 重复显示为两个 Agent**：
  - ChatGPT 桌面版把 Codex 打包进 `/Applications/ChatGPT.app/Contents/Resources/codex`，其 basename 与独立 `codex` CLI 相同且共用 `~/.codex` 会话目录，此前被数成两个 Agent
  - `AgentProfile` 新增 `pathExcludes`（路径排除）与 `hostBundleIDs`（宿主识别）：宿主已安装且该组件无独立安装时不再单独成条目；独立安装 codex CLI 的用户仍照常监控
  - 手写 `Codable` 解码（`decodeIfPresent` + 默认值），保证升级前保存的自定义 Agent 不因新增字段而整条失效
- **修复展开卡内容超高时底部汇总栏被压掉**：
  - 事件提醒栏出现后内容超过 460pt 上限，此前由 `VStack` 自行分配压缩，末尾的 Token 汇总栏成了牺牲品
  - 现给列表 `layoutPriority(-1)`、汇总栏 `layoutPriority(1)`：空间不足时只压可滚动的列表，汇总栏保持完整
- **性能：消除每 2 秒的主线程阻塞与 CPU 尖峰**（实测发现）：
  - `AgentActionInspector.activeChildCommand` 原先 fork `/usr/bin/pgrep` + `/bin/ps` 并 `waitUntilExit()`，单次 67ms，17 个 Agent 一轮 762ms 全部落在主线程；改为 sysctl `KERN_PROCARGS2` 直读命令行 + 复用采样快照做内存 BFS，`inspectDimAction` 单次由 170–220ms 降至 **0.9ms**
  - `inspectDimAction` 的 `ORDER BY createdAt DESC LIMIT 1` 在数万行 / 数百 MB 的 `messages` 表上退化为全表扫描 + 临时 B 树排序（220ms/次）；改用 `rowid = (SELECT max(rowid) …)` 走主键查找
  - `inspectOpenCodeAction` 的 `session LEFT JOIN part` 全表排序实测 330–964ms；改为「先取最新会话，再取该会话最新 part」，降至 8ms
  - `ProcessTerminator.getProcessTree` 与 `AppActivator` 的父进程追溯同样去掉逐节点 fork，改用一次快照内存遍历
  - 实测收起态 CPU 由均值 8.0% / 峰值 39.2% 降至 **1.2% / 2.8%**
- **修复状态误判：Agent 恒显「工作中」**：
  - `inspectAction` 返回的「最近动作」被当作核心工作信号，而各探测源在 Agent 空闲挂起时仍可能命中旧记录（dim 分支甚至无视注入的 fake 直读真实 SQLite），导致 `working` 永不消退、完成事件永不产生。现在工作状态只由「文件写入 + CPU」决定，动作仅作展示字段
  - 长驻子进程（MCP server、language server、`server.js`、`--liftoff-only` 索引进程）不再被判为「正在执行的任务」
  - `KERN_PROCARGS2` 解析按 `argc` 截断，避免把 `PATH=…` 等环境变量当成用户命令
  - 修复滞回锚点：原用「首次进入 working 的时刻」判断，任何超过 `minWorkingHold` 的任务滞回完全失效；改用每拍刷新的 `lastSignalAt`
  - 修复后 3 个长期失败的环境依赖用例全部转绿，测试 **84 通过 / 0 失败**
- **修复危险操作的可信度**：
  - 终止按钮在 `pid == nil`（GUI bundle 命中但进程名未匹配）时此前不发信号却宣告「进程已终止」——假成功；现在如实提示「无法终止：未定位到进程」并返回 `false`
  - 终止成功由 `attention` 改为 `completed`，收起态细条不再误报红色告警
  - 工具箱单条清理补二次确认（杀的是整棵进程树）；`overweight`/`hung` 不再把 `/Applications/*.app/Contents/MacOS` 主进程列为可清理项（开着大项目的 Electron IDE 占 2.5GB 属正常）
  - `ProcessTerminator.terminate` 返回真实信号发送结果，清理横幅不再谎报「已释放 N 个进程」
  - 工具箱扫描的 `NSWorkspace` 调用移回主线程（`ProcessProviding` 线程契约）
- **修复交互与显示缺陷**：
  - hover tooltip 内的按钮永远点不到：popover 由 26×26 环的 `onHover` 驱动，鼠标移向 popover 时立即触发关闭；改为 400ms 延迟关闭 + popover 内 hover 取消
  - 终止确认态不再于 Agent 转 idle 后残留（避免误杀已空闲进程），并用可取消 `Task` 替代 `DispatchQueue` 定时器
  - 顶栏优先展示「带动作」的 working Agent 并显示 `+N` 并行数；可见计数降为可压缩，长名称/动作不再被挤断
  - 事件横幅的按钮/背景改用动态色，浅色主题下不再白底白字不可见；关闭按钮热区 15→21pt
  - 「直达」在无对应 Agent 时置灰并说明，不再点击无反馈；熔断按钮在无 PID 时给出去向指引
  - 工作态 Agent 也显示 token 徽标（正在消耗的最需要关注）；活动环副标题与行内口径统一
  - 实时流水页返回按钮回到进入前的层级；事件 id 改为确定性生成，展开的详情不再每 2 秒被强制折叠
  - 脉冲与呼吸动画尊重系统「减弱动态效果」；版本号从 bundle 读取而非硬编码

---

## [0.0.14] - 2026-09-09

### 🩺 修复 Antigravity 监控识别与偏好设置自动自愈迁移

- **启用集自愈与向前兼容算法 (`EnabledAgentStore.resolvedEnabled`)**：
  - 彻底解决旧版本持久化 `enabledAgents` 导致新增内置智能体（Antigravity、ZCode、DSH、ChatGPT 等）被静默过滤的问题
  - 引入 `knownAgents` 持久化机制与 `legacyKnownAgentIDs` 基线迁移，存量用户升级时自动将新加入且默认启用的内置智能体合入启用集
  - 严格保持用户主动全关（`[]`）与单项显式关闭偏好，不发生意外覆写
- **用户环境自动修复与直达**：
  - 启动阶段与偏好设置面板同步自动自愈，Antigravity 无需用户手动翻找开关即可立即呈现在灵动岛监控与菜单栏中
  - 新增 4 组单元测试（首次安装/主动全关/历史存量迁移/显式关闭记忆），全量测试 64/64 保持全绿

---

## [0.0.13] - 2026-09-08

### ⚡ 智能体实时事件流与日志流水抽屉 (Live Log Stream)

- **智能体事件流与日志流水提取引擎 (`AgentLogStreamer`)**：
  - 支持多源并发智能日志采集：零侵入解析 Antigravity (`transcript.jsonl`)、Codex (`rollout-*.jsonl`)、DimAgent (`dimcode.sqlite`)、Claude Code、OpenCode、ZCode、WorkBuddy、Hermes 等
  - 提取高价值时序结构化数据：精准识别终端命令执行 (`EXEC`)、工具/MCP调用 (`TOOL`)、代码文件编辑 (`EDIT`)、深度推理思考 (`THINK`) 与模型对话流 (`MSG`)
- **原生极客暗黑风实时流水视图 (`LiveLogStreamView`)**：
  - 灵动岛主卡行与二级详情页增设终端图标（`terminal`）直达流水抽屉
  - 极客暗黑终端配色、彩色事件类型徽标、时间戳微调与参数展开查看
  - 支持实时静默自动跟随刷新（每 2 秒）与随时暂停切换
  - 提供一键复制全部诊断日志流水至剪贴板功能，便于排查与分析
- **窗口几何与导航联动**：
  - 深度集成 `IslandMetrics` 与 `IslandPanel` 自适应高度体系，保障展开卡片内无缝平滑滚动

---

## [0.0.12] - 2026-09-08

### 🛠️ 智能体维护工作台与进程清理系统 (Agent Workbench Cleaner)

- **原生工作台维护视图 (`ToolboxView`)**：
  - 展开卡顶栏增设快捷工具箱按钮（`wrench.and.screwdriver`），一键进入系统级 Agent 维护工作台
  - 展示待维护异常项、预估可回收内存与智能体整体健康度评分
- **三维智能体异常诊断 (Anomalies Detection)**：
  - **孤儿进程检测 (Orphaned Processes)**：自动捕捉主控终端被关闭后、父进程转为 `launchd` (PPID=1) 且脱离控制台的遗留 Agent 进程
  - **疑似死锁/假死检测 (Deadlocked / Hung)**：检测持续异常高载且失去会话响应的卡死任务
  - **内存超限检测 (Overweight Leaks)**：标识单进程驻留集物理内存超过 2.0GB 的潜在堆内存泄漏
- **一键安全清理与资源回收**：
  - 支持多选单项清理与一键安全全量清理，先尝试优雅信号通知保存状态，超时未退出则强制杀死并级联释放子进程树
  - 清理完毕后动态展示释放进程数与物理内存回收横幅，并触发自动刷新与状态自愈

---

## [0.0.11] - 2026-09-08

### 🩺 Agent 性能健康仪表盘与线程死锁检测

- **物理内存微秒级精准读取 (RSS Footprint)**：
  - 基于 Darwin 原生 `proc_pid_rusage` 读取物理驻留集 `ri_resident_size`，零外部命令开销，毫秒级反映各 Agent 真实物理内存占用
  - 在 `AgentRowView` 列表中增设内存紧凑徽标（如 `280M`、`1.2G`），卡片悬停 Tooltip 及详情页均可实时透视各 Agent 内存与 PID
- **死循环/高负载死锁异常检测 (Deadlock / Hung Detection)**：
  - 关联采样时间序列，当智能体发生非预期的高 CPU 持续占用（超出熔断阈值）或无会话响应时，自动在列表与详情页中标记「疑似卡死」健康警示
  - 支持一键安全终止与逃生舱清理，保障系统资源与开发机温度
- **二级详情页性能与健康矩阵**：
  - `AgentDetailView` 嵌入 CPU、物理内存与 PID 概览小方块，与 Token 用量协同构成完整的 Agent 资源透视图

---

## [0.0.10] - 2026-09-08

### 💎 灵动岛收起态边缘微胶囊视觉动效强化 (DockedSliverCapsule)

- **多状态一眼感知**：
  - **工作中 (Working)**：翡翠绿微光呼吸光晕与中心状态点呼吸流动，多 Agent 并行或后台生成时屏幕边缘清晰可感
  - **严重告警/待关注 (Alert)**：微红/琥珀金微光呼吸警示（针对熔断、死循环高负荷及进程异常），无需展开面板即可在屏幕边缘获知关键事件
  - **空闲待机 (Idle)**：晶莹半透明微晶胶囊，极致低调不扰工作
- **零额外能耗保证**：
  - 采用平滑缓和的 2.0s/1.2s 周期呼吸动画，仅在有明确状态（工作或告警）时开启；空闲待机时完全复位休眠
  - 完善 `IslandPanel` 对事件变更的观察管道，收起态状态无缝同步
- **边缘贴合自适应**：
  - 完美适配顶部吸附（横向微胶囊 140x6pt）与右侧吸附（纵向微胶囊 6x120pt）两种形态

---

## [0.0.9] - 2026-09-08

### 🎯 状态判断精准化与 Agent 工具生态全覆盖

- **消除 Electron 后台待机误报 WORKING**：
  - 将 `cpuThreshold` 默认阈值微调为 `6.0%`，彻底避开 Electron/Chromium UI 渲染器固有空闲抖动（1%~5%），真正在进行代码编译、大文件检索或模型计算时才触发 CPU 状态跃迁
  - 修正 WorkBuddy `sessionDirs` 为实际数据目录（`~/.workbuddy/sessions`, `~/.workbuddy/tasks`, `~/.workbuddy/memory`）
- **WorkBuddy 动作与状态时效性强校验**：
  - 加入 5 分钟更新新鲜度校验，超过 5 分钟未更新的会话标记为「待机」，绝不因陈旧未归档记录误报「正在处理」
- **补全本机 AI Agent 工具全生态覆盖**：
  - **Antigravity Studio**：支持 `com.yuzhiqiang.antigravity.studio` 独立识别并归并至 Antigravity 生态
  - **Ego Browser (Ego Lite)**：支持 Agent 专用隔离浏览器（`com.citrolabs.ego.lite` / `ego-browser`）
  - **Vibe Usage**：支持智能体 Token 用量聚合看板（`ai.vibecafe.vibe-usage` / `vibe-usage`）
  - **OpenViking**：支持本地 AI 知识库与智能体执行器（`openviking`, `openviking-server`, `ov`, `vikingbot`）
  - **扩充 CLI 检测池**：覆盖 `bsk` (Browser Skill)、`cua-driver` (Computer Use Driver) 等

---

## [0.0.8] - 2026-09-08

### ⚡️ 扩展主流 Agent 深度动作透传与会话解析

- **WorkBuddy 实时任务解析**：从 `workbuddy.db` 深度解析当前活跃会话标题、状态（`正在: ...` 或 `任务: ...`），精准透传工作上下文
- **OpenCode 深度解析**：支持从 `opencode.db` 的 `session` 与 `part` 提取实时思考规划（`思考规划中`）、工具调用（`正在调用: ...`）或当前会话主题
- **DSH (DeepSeek Harness) 模式感知**：结合进程命令行参数与会话状态，透传 Web 协作模式（`Web 协作服务运行中`）或任务执行详情
- **Hermes 会话与动作透传**：从 `state.db` 提取最近会话与活动描述
- **ZCode 任务检查器增强**：加入 `deleted = 0` 软删除过滤并放宽时间窗口，使进行中的任务持久准确透传

---

## [0.0.7] - 2026-09-08

### 🔔 告警与通知卡片交互升级与全量排查信息展示

- **事件卡片富文本展开（EventBannerView）**：告警卡片支持「精简两行」与「完整展开」双态切换，告警发生原因、阈值说明与排查建议一览无余，彻底杜绝文本截断 `...`
- **可解释性告警详细排查说明**：
  - **Token 激增告警**：显示具体监控时间跨度、增量绝对值、报警阈值与多 Agent 并发/Prompt 死循环排查指引
  - **死循环与高 CPU 告警**：展示持续分钟数、当前 CPU 百分比、进程 PID 以及熔断逃生舱操作建议
  - **任务完成通知**：展示实际耗时与状态转空闲说明
  - **终止逃生舱通知**：记录释放的目标 PID 与信号处理结果
- **一键复制诊断信息**：展开态下提供「复制诊断」按钮，可一键将 Agent ID、PID、发生时间、摘要与完整排查建议复制至剪贴板
- **窗口高度动态弹性自适应**：展开/折叠事件卡片时，面板高度自动平滑扩展（从 66pt 增至 142pt），保证下方的 Agent 列表不被遮挡或挤出窗口
- **严重告警自动展开**：对 Token 激增与死循环等熔断级警告，提醒时默认展开排查建议，协助用户迅速决策

---

## [0.0.6] - 2026-09-08

### 🔧 Agent 识别与状态检测全面修复

- **修复 ChatGPT/Codex BundleID 冲突**：`com.openai.codex` 实为 ChatGPT 桌面版，已独立为 ChatGPT profile，Codex 改为纯 CLI 检测
- **新增 ChatGPT 内置 profile**：`com.openai.codex` bundleID，独立监控 ChatGPT 桌面版运行状态
- **新增 DSH (DeepSeek Harness) 内置 profile**：进程名 `dsh` + pathContains `deepseek-harness`，会话目录 `~/.dsh/sessions` & `~/.dsh/storages`
- **修复 OpenCode.app 不被识别**：添加 `ai.opencode.desktop` bundleID，GUI 与 CLI 双路径均可检测
- **修复 Hermes sessionDirs 路径错误**：`~/.local/share/hermes`（不存在）→ `~/.hermes/sessions` + `~/.hermes/logs`（实际数据位置）
- **Probe CPU 双拍差分**：`--probe` 改为两次采样（1.5s 间隔），输出真实 CPU% 窗口值（修复永远 0.0 的问题）
- **闲置态采样提速 3×**：`idleSampleInterval` 从 15s 降至 5s，Agent 开始工作后最迟 5s 即被感知
- **InstalledAppsCache 同步**：`knownBundleIDs` 与 `knownCLIs` 与注册表完全同步

---

## [0.0.5] - 2026-09-07

### 💎 深度设计重构（CodeNotch 灵感：一体化反向倒角、微仪表环与悬停透视卡片）
- **反向倒角一体化贴边造型（SideNotchShape / Bezel Flares）**:
  - 彻底去除普通矩形切边的生硬感，借鉴 CodeNotch 与苹果硬件刘海的数学级 Bézier 曲线反向倒角（Flare）；
  - 小岛在屏幕右侧或顶部停靠时，贴边边缘自然平滑向屏幕边框弯曲延伸，如同从屏幕边框一体化生长出来；
  - 毛玻璃拟态、双色渐变蒙层、晶莹描边与窗口投射高斯阴影全链路对齐反向倒角曲线。
- **环形微仪表盘与双层动态活动弧（AgentRingView & ActivityArc）**:
  - 为所有 Agent 列表项与快速微看板引入 4 级彩色水位环（荧光绿、琥珀黄、预警橙、极光红）；
  - 居中渲染智能体高辨识度专属 Glyph；
  - **双层动效**：Agent 工作时内圈展开 0.25 长度的极细旋转微弧（1.2s 周期平滑旋转）；等待确认时呈现琥珀色呼吸警戒环；
  - 展开卡片顶部新增活跃智能体微看板（Quick Rings Shelf），一眼看清全局负载。
- **精准悬停透视卡片与指向小尾巴（AgentHoverTooltip & TooltipTail）**:
  - 鼠标悬停在任意 Agent 环上时，滑出带有指向尖角的悬浮透视浮层；
  - 零点击直达实时事实：当前执行的具体命令/正在修改的文件、进程 PID、24h/累计 Token 消耗与花费；
  - 卡片内一键直达终端/IDE 窗口或触发终止逃生舱。

---

## [0.0.4] - 2026-09-07

### 🔕 核心新特性：免打扰与通知分级 (Focus Mode & Notification Filtering)
- **三大通知策略模式（NotificationPolicy）**:
  - **专注免打扰（Focus Mode，推荐并默认）**：普通任务执行完毕静默更新（不弹窗微窥、不响提示音），仅在微细条或手动展开卡片内呼吸展示，彻底解决频繁编码被打扰的痛点；当发生**成本暴涨突增、死循环熔断告警（costSpike）**等紧急危险事件时，依然立即滑出 6 秒微弹窗并播放告警音。
  - **标准模式（Standard Mode）**：所有任务完成、等待确认、异常告警均滑出 3.5s 微弹窗并播放轻脆提示音（适合挂机等待智能体交付）。
  - **完全静默（Silent Mode）**：绝不滑出任何微弹窗，绝不播放任何提示音，纯后台静默记录与展示。
- **彻底根治无谓弹窗遮挡**:
  - 移除此前智能体一启动工作（`working`）就触发弹窗的过度打扰行为，还用户沉浸式专注编码体验。
- **全入口一键切换**:
  - 顶部菜单栏 Popover 底部操作栏新增通知模式切换快捷菜单（带对号与动态高亮）；
  - 灵动岛右键上下文菜单提供「通知模式」子菜单；
  - 设置面板「通用与外观」新增精致「通知与免打扰模式」卡片，并配有直观规则说明。

---

## [0.0.3] - 2026-09-07

### ✨ 核心新特性 (Appearance & Native Integrations)
- **深浅外观模式与跟随系统 (Appearance Modes)**:
  - 全局支持「跟随系统」、「浅色模式」、「深色模式」三态实时无缝热切换；
  - 灵动岛展开态顶栏右侧新增快捷主题切换图标按钮；
  - 菜单栏状态项 Popover、分栏设置面板、灵动岛右键上下文菜单均支持一键切换；
  - 玻璃拟态与单向圆角阴影（深色浓郁高斯阴影 vs 浅色晶莹投影）随主题自适应。
- **原生支持 Google Antigravity 智能体**:
  - 原生识别 Antigravity 进程与 CLI 会话目录；
  - 实时解析智能体执行轨迹（trajectory logs）与 tool_use 动作（如 `正在修改: ...`、`正在运行命令: ...`）；
  - 任务完成主动提醒并支持终端/IDE 窗口深度直达。
- **原生支持 ZCode 智能体**:
  - 会话日志感知、实时动作透视与一键直达。

### 🐞 关键修复与交互打磨 (Stability & Interaction Fixes)
- **灵动岛折叠与展开防抖重构 (Collapse & Gesture Refactor)**:
  - 修复边缘微细条几何判定误判（彻底移除卡片滑动过渡态对光标的误判），光标移出卡片后 0.5 秒平滑稳定收回贴边，绝不回弹；
  - 鼠标移入卡片立即解除手动展开保护期，无需等待固定延迟；
  - **点击外部区域自动收起（Click-outside to dismiss）**：在展开卡片外部任意桌面或窗口点击即刻平滑折叠；
  - **顶栏新增一键显式收起按钮**（右侧边栏为 `chevron.right`，顶部灵动岛为 `chevron.up`），右键上下文菜单同步提供「收起灵动岛」；
  - 重构顶栏拖拽把手为背景层，彻底解决拖拽手势遮挡外观切换菜单与收起按钮的问题。
- **死循环告警判定与横幅排版调优**:
  - 重构高负载检测算法，引入基准采样比对，避免短时编译与常规编码误报；
  - 告警横幅重构为双行自适应卡片，告警描述完整展示支持 Tooltip，操作按钮独立成行。
- **进程路径误报防御**:
  - 强化 `pathContains` 约束，避免同名 Electron 进程导致未安装智能体误报。

### ⚡️ 质量与用例
- 测试套件扩充至 56 个全自动化单元测试用例，全绿通过（`56 通过, 0 失败`）。

---

## [1.5.0] - 2026-09-07

### ✨ 核心新特性 (Practicality & Command Center)
- **任务完成主动提醒与智能微窥 (Event Peek & Sound)**:
  - 智能体经历持续工作（≥3.5秒）后转为空闲时，自动播放 macOS 原生轻脆 `Glass` 提示音；
  - 处于 6pt 贴边收起态时，自动平滑滑出 3.5 秒 Peek 微弹窗；若光标移入则自动升级为常驻展开态，移开后平滑收回；
- **终端与 IDE 窗口一键深度直达 (Window Deep-Linking)**:
  - 毫秒级递归追溯进程树父节点，精准定位承载 CLI 智能体（Claude Code、Codex、Dim 等）的 GUI 终端窗口（Terminal、iTerm2、VS Code、Cursor、Ghostty、Warp 等），一键拉至最前聚焦；
  - GUI 智能体（Cursor、Trae、DimAgent 等）直接通过 BundleID / PID 唤醒；
  - 任务完成横幅与 Agent 列表行均提供「直达」快捷按钮；
- **实时操作与工具调用透视 (Real-time Action Context)**:
  - 自动提取子进程实时执行的系统命令（如 `swift test`、`git diff`、`npm run build` 等），内置智能格式清洗器剥离 shell 包裹层；
  - 解析 DimAgent / Claude / Codex 运行时会话日志与 tool_use 元数据，顶部卡片与列表行实时呈现终端样式徽标（如 `> 正在修改: IslandView.swift`）；
  - CLI `--probe` 终端诊断命令新增 `ACTION` 实时动作列；
- **成本与异常死循环熔断保护 (Runaway Loop & Cost Circuit Breaker)**:
  - **Token 暴涨告警**：基于滑动窗口差分监测单分钟 Token 增量，超过阈值时触发红色/琥珀色高亮告警并播放低沉警示音；
  - **长耗时死循环告警**：高负荷工作超 3 分钟未释放时自动预警；
  - **一键 Kill 逃生舱**：告警横幅提供红色「熔断」按钮；列表行提供红色停止按钮，并配有 3 秒自动取消的「终止?」二次防误触确认，安全杀死整棵子进程树；
- **设置面板扩展 (Circuit Breaker Settings)**:
  - 「引擎与性能」Tab 新增「成本与异常熔断保护」配置卡片，支持 30k/50k/100k/200k tokens 阈值调节及各项开关。

### ⚡️ 质量与稳定性 (Quality & Testing)
- 测试套件扩充至 53 个全自动化测试用例，全绿通过；
- `--selftest` 进程内自检断言全部通过。

---

## [0.0.2] - 2026-09-07

### ✨ 交互与界面革新
- **自由移动与智能贴边吸附**: 支持按住顶栏全屏幕任意拖拽，松手根据物理距离智能吸附到屏幕顶部或右侧，并持久化锚点坐标；
- **6pt 晶莹微细条与弹性弹出**: 收起时保留 6pt 半透明微细条（含呼吸绿灯），光标碰触以流体弹簧动效自动弹出完整卡片；
- **现代分栏设置窗口**: NavigationSplitView 四大分类架构，支持贴边重置与外观切换；
- **顶部菜单栏 Compact Popover**: 现代原生浮窗浮动展示活跃 Agent 概览。
