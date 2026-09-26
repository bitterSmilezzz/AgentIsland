# AgentIsland — Agent 会话灵动岛监控器

监控本机所有 Agent 软件（DimAgent / Claude Code / Codex / Cursor / Trae / Google Antigravity / ZCode / WorkBuddy / Copilot / OpenCode / Xiaomi MiMo / Qoder / Cline / Roo Code / Continue / Goose 等）的运行状态：谁在跑、正在做什么、需不需要你回去确认、这一轮花了多少。以 macOS 灵动岛风格呈现，可选地把通知送出本机到手机或邮箱。

> 本文档描述 **v0.0.151** 的行为；每个版本改了什么见 [CHANGELOG.md](CHANGELOG.md)。
>
> 项目主页（截图与功能导览）：<https://bitterSmilezzz.github.io/AgentIsland/>，源码在 `site/`。

## 能看见什么

- **五态**：`working` 运行中 / `attention` 等待你确认 / `completed` 已完成 / `idle` 待机 / `offline` 离线，各有独立语义与颜色；异常高负载再叠一层橙/红告警。
- **正在做什么**：子进程命令实时提取（`git diff`、`swift build`、`npm run build`…）与会话日志里的动作上下文（`正在修改: IslandView.swift`、`正在执行: pytest`）。后台任务与子智能体以 `⚡ 后台N`、`🤖 N子任务` 微胶囊呈现。
- **等待确认**：识别 Agent 的结构化确认、权限申请与用户输入请求。多个 Agent 同时在等也不会漏报，每个请求只提醒一次。
- **成本**：24h 与累计 Token、按模型拆分、花费，以及月末消耗与预算枯竭预测。预算与 24h 用量同为滚动 24 小时口径（不按自然日清零），周/月趋势图的每根柱子对应一个日历天。异常大的数值会被饱和到上限并说明，不会把报表算成 `inf`。
- **异常**：Token 暴涨、长耗时死循环、孤儿后台与内存激增；实时流水里的 429 限流、编译报错、Git 冲突与鉴权失败会被特征分析出来并给一键复制摘要。
- **监控可信度自查**：岛内与维护工作台都列得清「这个 Agent 是真闲着，还是我根本没看到它」——会话源读不到 / 本地无明细 / 未接入明细源分别是三种不同结论。判定与 `doctor` 共用同一份实现，不会出现「终端说不可信、岛上说正常」。

## 灵动岛界面

- **四边贴边与 6pt 微细条**：收起时在屏幕上、右、下、左任一边缘留一条 6pt 半透明细条（含工作状态呼吸灯），光标碰触即弹性弹出完整卡片。展开态顶栏按住可自由拖拽，松手按最近距离吸附并持久化该边锚点。
- **反向倒角一体化**：基于三次 Bézier 的 `SideNotchShape`，边缘向屏幕物理边框平滑过渡，而不是悬空矩形；玻璃卡片与倒角边缘共用同一套几何路径。
- **环形微仪表盘**：5 态水位环、工作态旋转微弧（`0.25` 弧长、1.2s 无级旋转）、告警态呼吸警戒环，顶部并列当前活跃 Agent 的迷你环，点击秒切。
- **悬停透视卡**：光标停在环或列表项上即显示实时命令、正在修改的文件、PID、24h / 累计 Token 与花费，带几何指向小尾巴，并内嵌「直达窗口」与二次确认的「终止进程」逃生舱。
- **卡内三级导航**：主列表 → Agent 详情（双口径总览 + 按模型拆分）→ 模型会话列表（时间 / 消息数 / token / 花费，可跳 Finder 目录）。
- **窗口直达**：GUI Agent 按 BundleID/PID 置顶聚焦；CLI Agent 递归追溯进程树，定位 Terminal、iTerm2、VS Code、Ghostty、Warp 等宿主终端；目标已退出时回退到岛内详情。
- **键盘流**：`1~3` 切视图、`j/k` 移动焦点（自动滚进可视区）、`Enter` 下钻、`/` 聚焦搜索、`Esc` 逐级返回、`?` 快捷键速查浮层。破坏性动作一律两段确认。
- **收起行为**：光标离开自动折叠（防抖，移出后稳定贴边不回弹）、点击岛外任意位置收起、顶栏一键收起，右键菜单同步。
- **菜单栏 Popover**：原生 `.window` 风格浮窗，显示呼吸状态灯、活跃 Agent 概览、Token 统计与操作直达。
- **设置**：分栏窗口，五个分类是「通用与外观 / Agent 监控 / 远程通知 / 引擎与性能 / 关于」。启停开关、熔断阈值、采样间隔、收起延迟、贴边位置与外观都实时生效，不需重启。
- **外观**：浅色 / 深色 / 跟随系统，顶栏图标、菜单栏浮窗、设置面板与右键菜单全入口热切换；浅色小字号对比度按 WCAG AA 正文档校准。
- **长文本**：主顶栏把 Agent/状态与实时动作分层显示，始终保留标题最低可读宽度；文件、模型与动作文本中间省略，悬停与 VoiceOver 可读全文。

## 状态判定与误报防护

- **判定优先级**：结构化会话事件优先识别 `attention` 与 `completed`；无强语义时按双信号降级——`working` = 进程在 且（60s 内有文件写入 **或** CPU ≥ 6%，桌面类档案用 20%/35% 下限），`idle` = 进程在但两者皆不满足，`offline` = 进程不在。
- **只在「有写入证据」时宣布完成**：纯 CPU 高负载（桌面应用空闲抖动、启动尖峰）、只打开应用不做任何操作、浏览器内核缓存与账号状态写入、SQLite `-shm` 空转触碰、睡眠/合盖唤醒均不算任务完成。Ctrl-C 留下的僵尸 `tool_use` 会立即撤销在途命令，Agent 不会被钉在工作态不放。
- **在途命令全周期拦截**：Claude Code（`Bash`）、Codex（`exec_command`）、Cline / Roo Code（`ask/say command`）、Antigravity 异步任务与定时器期间，严格保持工作态，不放完成横幅与提示音。
- **可见口径**：主列表、菜单摘要与顶部环只显示进程仍在的 Agent；离线项即使有近期活动或 Token 记录也隐藏，避免「昨天的事」冒充「现在的状态」。
- **误报防护**：按完整路径匹配（非 basename），排除系统目录前缀（`/System/`、`/usr/libexec` 等）与已知噪声（`CursorUIViewService`、`ssh-agent` 等）；扫描跳过依赖树与配置目录。
- **注册表 + 自动发现 + 自定义**：启动时扫描 `/Applications` 与 PATH 自动补充 CLI；设置里可添加自定义 Agent（进程名 + 会话目录）。会话方言与库路径由注册表唯一声明，新增复用既有格式的 Agent 只改一处；声明专有方言却没给目录的档案会被测试直接判失败。
- **只读**：仅尾读本轮命中的会话文件并解析结构化字段，不展示确认问题与回答正文，不修改 Agent 数据，也不需要辅助功能 / 完全磁盘访问权限。
- **运行环境**：全屏 Space 跟随（`fullScreenAuxiliary`）、多屏跟随鼠标所在屏、外接屏拔出平滑重定位、开机自启可选（`SMAppService`，默认关）。

## 提醒分级

- **三档策略**：标准模式 / 专注免打扰（默认推荐：普通完成静默更新，待确认与严重告警仍即时通知并出声）/ 完全静默。
- **Peek 微弹窗**：收起态下按策略滑出约 3.5 秒短提示，光标移入转为常驻展开。
- **严重告警保护**：30 秒内不会被其他 Agent 的完成事件顶掉，留出处置时间；熔断横幅可展开详情与排查说明，收起某个 Agent 的横幅只收起它自己。
- **声音**：完成用 Glass，成本激增与死循环用警示音；由应用内发声，不依赖通知权限。
- **一键终止带身份复核**：横幅与列表行的「终止」都要两段确认（首击进入确认态、3 秒自动复位）；终止前按 pid + 可执行路径复核身份，pid 已被系统回收复用给别的程序时拒绝执行。

## 远程通知（可选，默认关闭）

解决的问题：用 Windows 远程桌面连着 Mac 时，通知只画在 Mac 屏幕上——远程协议传的是像素不是事件，人不在 Mac 前就收不到。唯一可靠的办法是让通知走网络出去。开关在 **设置 → 远程通知**。

- **三个通道**：ntfy 推送（订阅一个主题名）、自定义 HTTP 模板（微信 Server酱 / PushPlus、企微、钉钉、飞书都走这条——各家字段不预置，把你那家控制台给出的地址整段粘进来，密钥写 `{key}`、标题 `{title}`、正文 `{body}`）、邮箱 SMTP。邮箱只支持 465（隐式 TLS）：`Network.framework` 没有「在已建立的 TCP 上原地升级 TLS」的能力，25/587 会在配置层被拦住并说明原因，而不是让你配完发现每次都不通。
- **凭据只进 macOS 钥匙串**，由你自己录入；界面与日志只显示掩码，UserDefaults 里零密钥。写入失败会把原因显示出来——本 App 是 ad-hoc 签名，每次重新出包代码标识都变，可能需要在弹窗点「始终允许」。
- **默认只送「哪个 Agent + 什么状态」**。命令内容、文件路径、消息原文必须显式勾选「附带最后一条动作」才会送出；「发送预览」显示实际会离开本机的原文（密钥处为掩码）。
- **深链、`/notify` 与没令牌而降级的 `/session` 产生的事件一律不外送**：那是本机任意进程都能写的入口，岛内能看出「外部投递」，手机上看不出。
- **什么时候发**：按事件类型分别开关（完成 / 等待确认 / 消耗告警）、同一 Agent 同类事件在 N 秒内只发一次（默认 90 秒）、可设静默时段（支持跨零点）。
- **「只在人不在机器前时发送」**（默认关）。三条信号任一成立即算离开：屏幕锁定、显示器睡眠、**键鼠无输入超过阈值**（默认 120 秒，可调 30–3600）。远程桌面连着 Mac 时前两条永远不会成立——会话不锁、屏幕不熄，**只有无输入这条管用**。设置页实时显示当前判定与三条信号取值，被挡下时「最近外发」写明依据（「有人在机器前（距上次输入 8 秒，未达 120 秒）」）。取不到输入时长时按「已离开」照常发（fail-open），并把「这台机器上这条判据没有数据」写在页面上。
- **暂时性失败隔 5 秒重试一次**；对端明确拒绝（主题不存在 404、授权码错 SMTP 535、拒绝中继 550）不重试——再要一次也是同一个结果，而公开中转按条数限流。「最近外发」会分开写「已送达（重试 1 次后）」「重试 1 次仍未送达」「对端明确拒绝，重试无用」。
- **「发送测试」会真的送出一条**，并绕过节流、静默时段与在场判定，但总开关仍然管用。注意「已送达」只表示对方服务器接受了这次请求，**不等于已推送到你手机**——多数中转服务内部失败也回 200。

## 命令行与自动化

`agentisland`（构建产物在 `dist/`，也随 `.app` 装在 `Contents/Helpers/`）：

| 命令 | 用途 |
|---|---|
| `status` | 所有 Agent 的运行态快照，`-w` 动态监控，`--json` 供脚本调用 |
| `state` | 读灵动岛**进程里**的实时状态：谁在跑、这一拍的状态是谁说的（观测 / 推断 / 自报 / 冲突）、自报还剩多久；连不上、被拒、端点不存在都会明说并以退出码 1 结束 |
| `top` | 类 htop 的全屏看板，`q` 退出、`r` 刷新、`c` 一键清理 |
| `tokens` | 24h 用量明细、成本与月末预测；`--budget` 打印终端进度条 |
| `doctor` | 一次性自查：这个 Agent 是真闲着，还是我根本没看到它（结论带依据，支持 `--json`、`--quiet`，`--agent` 可写 id 或名称） |
| `check` / `clean` | 排查孤儿与内存异常 / 一键释放，`-n` 只预览不终止、`-f` 连孤儿后台一起清 |
| `selftest` | 用假数据断言核心判定逻辑，验证构建本身而非本机状态 |
| `open` | 从终端控制灵动岛展开、折叠、直达指定 Agent 或看板 |
| `notify` | 主动投递完成 / 待确认 / 告警事件（岛内会标为「外部投递」） |
| `report` | 生成 Markdown / CSV / JSON 运维报告，`-c` 复制剪贴板、`-o` 写文件 |
| `raycast` | 导出 Raycast 命令清单 |

- **「没取到」不印成 `0`**：`status` 默认用量列是 `—`，`--usage` 才同步取数（约 5s）；JSON 里 `tokens24h` / `cost24h` 可空，`nil` 是没查、`0` 是查了确实为零。审计报表同一口径：Markdown 里没查到的**整行**都是 `—`、CSV 里是空字段（与 PID/CPU 列一致），合计那一行会写出有几个源没计入。`—` 只表示「没查」；查了、确实是零，成本格写 `$0.00`。
- **CPU 要有窗口才谈得上测到**：利用率是两次采样之间的进程时间差分量，所以只采一拍的入口（`status` / `report`）根本没有窗口，那一列印 `—`、JSON 里是 `null`，不是 `0.0%`。要真实 CPU 用 `doctor`（双采）或 `top`（持续观测）。只按 bundle 命中的档案同样算「没测」——占位条目的 0 是「没看着这个进程」。
- **退出码**：1 = 失败，2 = 用法错。写盘失败不会静默 exit 0。
- **深链**：`agentisland://` 支持 `toggle`、`expand`、`collapse`、`agent?id=<id>`、`analytics`、`toolbox`、`clean`、`export`、`notify`。投递目标必须解析到已知档案，否则拒绝。
- **本地 Webhook**：`127.0.0.1:41999`。
  - `POST /notify`、`/event`：供 CI 或脚本毫秒级直推事件。**无鉴权**，只靠「仅监听回环」限制来源，岛内标为「外部投递」，不外送。
  - `POST /session`、`DELETE /session`：给智能体自己上报生命周期（working / idle / attention / completed）并带 TTL。要出示令牌 `X-AgentIsland-Token`，值在 `~/Library/Application Support/AgentIsland/report.token`（`0600`，同机用户可读得懂的文件形态都会被打回）。TTL 钳在 15–600 秒（默认 90），到期只盖章不删除——之后那个会话的状态重新由进程表推断。没有令牌的申报不丢：落回上面那条无鉴权通道，带「未采信」标记。可声明的状态只有四种，认不出就 400，绝不猜。
  - `GET /state`：读 App 进程里的实时状态，与 `/session` 用同一枚令牌。只给聚合状态（谁在跑、状态是谁说的、自报还剩多久、冲突那句原文）；**命令正文、文件路径、会话 id 一律不给**。
  - 自报记录只在 App 那个进程里：`status` 是另起进程独立采样，它看不到这些记录，要读自报请用 `agentisland state`。
  - 请求形状：单条请求上限 64KB，越过回 413 并说明上限；分块到达的正文按 `Content-Length` 收齐再判，没收齐不作答（连接断了才回 400）。`type` 的取值深链与 HTTP 共用同一张表：`attention`/`confirm`/`wait` 是待确认，`costspike`/`cost`/`budget`/`alert` 是告警，其余按完成。
- **死锁判定是三态，不是两态**：`isHung` 有「卡死 / 不卡死 / 本轮判不出」三种。判据是「CPU 连续 70% 以上达 5 分钟」，所以资格取决于**对这个进程连续观测了多久**——由引擎写在快照里，不给调用方自报的余地。`check` / `clean` / `status` / `report` / `doctor` 这些一次性入口因此一律是「判不出」：明说「本次未评估」，评级给「观测不全」，JSON 里 `isHung` 为 `null`（不是 `false`）。要死锁结论只能用持续观测的入口（灵动岛工作台、`top`）。
- **孤儿判定要证据**：`ppid == 1` 分不清「终端关掉的遗孤」和「launchd 刻意托管的常驻服务」，两者进程表长得一样。规则是会话目录 10 分钟内仍有写入就算活着、不报孤儿；宁漏不错杀。这条与上一条在 UI 与 CLI 之间共用同一份实现。

## Token 用量口径

- **DimAgent**：`~/.dimcode/v2/dimcode.sqlite` 的 `usage_ledger`（精确）
- **OpenCode**：`opencode.db` 消息表（token + 花费 $）
- **Codex / Claude / WorkBuddy / WorkBuddy AI**：只读解析本机会话 JSONL 的结构化 usage 字段，不保留对话正文
- **Antigravity**：Prompt / Completion / Cache / Thoughts 细分
- **Xiaomi MiMo（MiMo Code）**：与 OpenCode 同一套 `session` / `message` / `part` 表，用量、状态、当前动作、实时流水都按 OpenCode 方言读 `~/.local/share/mimocode/mimocode.db`
- **Qoder：用量列显示 `—`，它的 token 消耗监控不了**——会话库里四个 token 字段恒为 0，真值只有 `credits`；接进去会让用量页明显变慢却仍然拿不到 token，所以宁可显示「没取到」，不拿 credits 折算成一个看起来像真的数字。
- 行内徽标显示 24h 用量；卡片底部汇总栏双口径（跨工具 24h / 累计 + 花费）；点击汇总条进入 24h / 7 天 / 30 天分析（趋势、环比、按工具占比与成本）。内嵌的 Codex 即使不单独显示为运行项，也不会失去用量入口。
- 分工具列表始终说明数据覆盖状态：「未发现本地明细」与「真实零用量」不混为一谈；无明细的图表区间显式提示，而不是画一整排全零格子；环形图静默丢弃的模型改为可见的「其余 N 个 · xx%」。
- 60s 后台轮询；SQLite 只读打开，JSONL 按文件指纹缓存并增量解析，不锁库、不碰凭证。

## 性能与能耗

- **贴边收起近零开销**：呼吸光晕由 CoreAnimation 独立硬件图层驱动，消除高刷屏上的递归重排，Docked 态 CPU 稳定在 0.0%–0.1%；鼠标移动做邻近预过滤，屏幕中央滑动不派发任务。
- **采样节奏**：有活动 2s，全闲置降频 5s；电池供电时后台扫描平滑降频，插电即刻恢复满速。
- **热路径 I/O**：进程快照一次 libproc 遍历（`proc_listpids`/`proc_pidpath`）复用全部档案，CPU 两次采样差分；文件扫描后台递归 + 节流 + 快跳过缓存；会话尾读带失效令牌的定位缓存；超长日志内存映射并设 32MB 上限。
- **常驻有界**：节流表、外发历史、解析缓存与告警基准集合都有上界与明确的收敛入口；停止引擎后在飞的后台遍历会复检再落地。

## 安装

从 [Releases](https://github.com/bitterSmilezzz/AgentIsland/releases) 下载最新的 `AgentIsland-*.zip`，解压后拖入「应用程序」或直接运行。

> 未公证（ad-hoc 签名），首次打开需右键 → 打开。

## 构建与测试

无需 Xcode，SwiftPM + CommandLineTools 构建，手工组装 `.app`：

```bash
# 若系统装了 beta SDK，先钉住正式版（否则 SwiftUI 宏插件找不到，构建失败）
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk

swift build && swift build --build-tests
.build/debug/AgentIslandTestsRunner     # 自建测试套件（零依赖，CLT 可用）
.build/debug/AgentIsland --selftest     # 进程内自检
.build/debug/AgentIsland --probe        # 真实环境状态表

./scripts/test-scan-secrets.sh          # 脱敏闸门自身的行为测试（7 条，专盯「假绿」）

./scripts/build-app.sh                  # 打包：测试门禁 + 图标 + ad-hoc 签名
open dist/AgentIsland.app

AGENTISLAND_DEBUG=1 open dist/AgentIsland.app && tail -f /tmp/agentisland.log
# 或不设开关直接看系统日志：
# log show --predicate 'subsystem == "com.agentisland.app"' --last 10m
```

交付链路（新克隆先装钩子）：

```bash
./scripts/install-git-hooks.sh              # pre-commit 挂上脱敏扫描
./scripts/scan-secrets.sh --release         # 手动扫一遍：工作区 + 全部 git 对象
./scripts/release.sh 0.0.119 "这一版的一句话"  # 扫描 → 打包 → 提交 → tag → 推送 → GitHub Release
```

发版要求三处版本一致（CHANGELOG 首条、`AppVersion.string`、本文件的版本行），任一处漂移 `release.sh` 与 `build-app.sh` 都会拒绝。**本文件只讲这个工具是什么、能做什么；逐版改了什么一律写进 [CHANGELOG.md](CHANGELOG.md)。**

协同开发的 agent 可以装第三方 skill（管理与写法见 [AGENTS.md](AGENTS.md)）：skill 放 `.agents/skills/<名称>/`，`.claude/skills/<名称>` 指过去的软链供 Claude Code 读取。当前装有 `libraries-dev`（Libraries.dev 的配套 skill，含 7 个 React 视觉特效库的取舍规则），它对 SwiftUI/macOS 没有可调用组件，本项目不依赖它构建。

套件覆盖五态状态机、会话语义、通知路由、标题可读性、事件唤醒、进程树熔断、外观主题、命令清洗、token 时间统计、深链与 CLI、远程外发协议与策略。多数断言做过**变异验证**（把被测逻辑改坏、确认对应测试变红），抓不到的缺口在 CHANGELOG 里如实列出。另有**债务棘轮**：Theme 外硬编码色值与 `UserDefaults.standard` 直读处数钉成基线，新增即测试失败并说明该用什么替代。开发约定见 [AGENTS.md](AGENTS.md)，设计依据见 `docs/research/`（含远程通知与 Qoder 监控两份调研）。

## 目录结构

只列常改的文件，完整清单看 `Sources/`。

```
AgentIsland/
├── Package.swift                     # 5 target：Core 库 + App + CLI + IslandMetricsKit + 测试 runner
├── .agents/skills/                   # agent skills 单份源（.claude/skills/<name> 是指向它的软链）
├── site/                             # GitHub Pages 主页（纯静态，Actions 从 site/ 部署）
│   ├── index.html                    # 功能导览与截图（深浅两套外观）
│   └── assets/screens/               # 逐张核对过不含个人路径与会话正文的截图
├── docs/
│   ├── adr/                          # 长期决策（编号 0001…）
│   ├── research/                     # 一手核实记录（正文原句/本机实样 + 取证命令）
│   │   └── ui/                       # 外部动效案例库：手法 + 可迁移规则 + 应用建议
│   ├── code-review/                  # 每轮改动的独立 review
│   └── workbench/                    # 进行中的大改造：现状/目标/方案/计划四件套
├── scripts/
│   ├── build-app.sh                  # .app 打包（无 Xcode 环境）
│   ├── scan-secrets.sh               # 提交/发版前的密钥与个人信息门禁（棘轮式 baseline）
│   ├── release.sh                    # 扫描 → 打包 → 提交 → tag → 推送 → GitHub Release
│   └── install-git-hooks.sh          # 把扫描挂到 pre-commit
├── Sources/
│   ├── AgentIslandCore/              # 核心库（可被测试 import，不依赖 UI）
│   │   ├── Models.swift              # ActivityLevel / AgentProfile / AgentSnapshot / EngineConfig
│   │   ├── AgentRegistry.swift       # 内置集 + 自动发现 + 自定义存储（无全局状态）
│   │   ├── ProcessMonitor.swift      # libproc 快照（路径 + CPU 差分）/ 过滤 / 黑名单
│   │   ├── FileMonitor.swift         # 后台递归 + 节流 + 单调 merge 缓存（主线程只读）
│   │   ├── AgentSessionInspector.swift # 会话尾部强语义解析（待确认/已完成）+ SQLite 适配
│   │   ├── TokenUsageMonitor.swift   # 跨工具用量与时间线（SQLite + JSONL 只读）
│   │   ├── ActivityEngine.swift      # 五态状态机 + 双信号降级 + 逐事件通知流
│   │   ├── AgentCleaner.swift        # 异常排查与清理复核（按 pid 探活，不谎报已处置）
│   │   ├── RemoteNotification.swift  # 远程外发的消息模型、策略与钥匙串
│   │   ├── RemoteNotifier.swift      # 策略 → 渲染 → 传输 → 结果记账
│   │   ├── SMTPSocket.swift          # SMTP 465（Network.framework 真 socket）
│   │   └── URLSchemeParser.swift     # 深链解析（从 UI 抽出来，可测）
│   ├── AgentIslandCLI/               # agentisland 终端工具
│   └── AgentIsland/                  # UI（SwiftUI + AppKit）
│       ├── AgentIslandApp.swift      # @main + MenuBarExtra + AppContext 组合根
│       ├── IslandPanel.swift         # NSPanel 控制器（拖拽/贴边/细条/peek/事件路由/外发触发）
│       ├── IslandView.swift          # 玻璃卡片 + 贴边微细条 + 三级导航
│       ├── ScreenPresence.swift      # 在场信号采集（锁屏 / 显示器睡眠 / 无输入时长）
│       ├── RemoteNotifySettingsView.swift # 远程通知设置页
│       ├── AgentRingView.swift       # 环形微仪表盘
│       ├── IslandMetrics.swift       # 窗口几何唯一事实来源
│       ├── Theme.swift               # 设计令牌与语义色（唯一来源）
│       └── SettingsView.swift        # 分栏设置
└── Tests/AgentIslandTestsRunner/     # 自建测试框架
```

## 监控原理

```
┌────────────────┐   ┌─────────────────────┐   ┌──────────────────┐
│ ProcessMatcher  │   │ FileActivityMonitor │   │ TokenUsageMonitor │
│ · 进程表快照     │   │ · 后台批量枚举+缓存  │   │ · SQLite 只读     │
│ · 路径白名单     │   │ · 节流 + merge 缓存  │   │ · SQLite + JSONL  │
│ · 黑名单排除     │   └─────────┬───────────┘   └────────┬─────────┘
└──────┬─────────┘              │                        │
       └──────────┬─────────────┴────────────────────────┘
                  ▼
        ActivityEngine（活动 2s / 闲置 5s 采样）
        · 未解决的确认/授权请求             → attention
        · 本轮明确结束（有写入证据）        → completed
        · 进程在 + 写入 60s 内 或 CPU≥6%   → working
        · 进程在但静默                      → idle
        · 进程不在                          → offline
                  ▼
        IslandPanel（NSPanel，非激活置顶，多屏跟随）
        · docked：任一边贴边留 6pt 微细条（触碰即弹性弹出）
        · expanded：玻璃卡片（列表 / 详情 / 会话三级导航）
                  ▼
        三条互不干扰的出口
        · 岛内横幅与 peek   ── 看通知策略分级
        · 系统通知与提示音  ── 同上
        · 远程外发（默认关）── 只看自己的开关与在场判定；
                              深链与 /notify 来的事件一律不走这条路
```

## 已知限制

- 已适配结构化确认/完成事件的 Agent 能区分待确认与已完成；未知或改版后的日志格式会安全降级到「进程 + 文件写入/CPU」三态近似。
- Token 统计当前适配 DimAgent、OpenCode、Xiaomi MiMo、Codex、Claude、WorkBuddy、WorkBuddy AI 与 Antigravity；不提供稳定本地 usage 明细的工具（如 Qoder 只有 `credits`）会在分析页明确标为未接入而不估算
- **OpenCode 的适配只对过源码，没对过真库**：`~/.local/share/opencode/opencode.db` 在本机不存在（未装 CLI），字段口径是从上游 `packages/schema/src/v1/session.ts:315-322` 核出来的。token 与状态两类字段名与我们的 SQL 逐字一致；「当前动作」一处的字段名（`type=="tool"` + `tool`）已按上游修正并有测试守着，但**装上 opencode 跑一个会话仍是未做的验收**。opencode 另有一个 Electron Desktop App（BETA），我们只匹配了进程名 `opencode`，其 bundle id `ai.opencode.desktop` 与 helper 进程未逐个数过
- 闲置降频 5s 时，Agent 开始工作的检测最多延迟一个采样周期（可调「闲置降频间隔」）。
- 多显示器跟随光标所在的那块屏（`NSScreen.screens` 按鼠标位置选），贴哪条边由吸附设置决定，四边都行。
- **浅色小字号按 AA 正文档校准**：8–10pt 文字已 ≥4.5:1，但热力图 / 环图一类**纯图形**仍按 3:1 的图形线取值，不追求正文档。
- **命中区补的是高度不是画出来的尺寸**：顶栏与详情页的图标按钮统一走 `hitTargetHeight()`（`minHeight: 24` + `contentShape`），按得到也点得到；视觉圆底仍是 19×19 一类的小图形，横向命中区跟着图形走。扩成 24×24 要改圆底与行高，属可见布局改动。
- **在场判定的「无输入时长」在测试里构造不出来**（需要真实 GUI 会话）。判定逻辑与调用点传参都有断言，但取信号那一层只能靠设置页的实时判定行当场核对。
- **动效减少只覆盖了 3 个文件，不是全部**：`accessibilityReduceMotion` 在 `DockedSliver`（2 处）、`AgentRingView`（3 处）、`ActivityMatrixDots`（1 处）**已接**；`IslandView` / `IslandComponents` / `EventBannerView` 与各列表图表 View **未接**。所以设了「减少动态效果」的用户会得到一部分静态替代、另一部分照常动，两种行为并存。补齐是逐个 View 的活，不是改一处常量。
- Qoder 的「任务完成」按**哪一轮人类指令**记：同一轮里模型多次收尾（等后台构建、被通知唤醒后续跑）只响一次。若某一轮的首行在两次采样之间就滑出了尾窗（大 payload 时字节上限会先于行数到点），那一轮是谁起的头就认不出来：见过人类轮次时挂在最近那一轮上，一次都没见过时挂在**这份会话**的身份上——两种都是少响一次，不是不响。
- Token 明细只留近 70 天（分析页最宽 30 天，另需同长的上一周期做对比）；更早的折成按工具的合计，累计口径不变但按天铺不开。索引仍按磁盘上存在的 jsonl 数量线性重建（本机 5,778 条时稳态 2.7ms），全库未变化时走戳备忘录直接复用。
- `/notify` 无鉴权，仅靠「只监听回环」限制来源；macOS 13 无法限定，会在启动时告警。
- `/session` 的可信度**只到令牌为止**：读那个文件之前会查它是常规文件、属主是本机用户、
  权限不带组/其他位、内容是够长的十六进制，任何一条不合格就当没有令牌并换一把新的。
  这验的是**形态不是来源**——同一个用户先写一串形态合格的十六进制仍然挡不住，
  要不可伪造就得进钥匙串，代价是用户看不见令牌、也就没法把它抄进第三方 Agent 的配置。
  pid 只能否掉一条申报，永远不能建立可信度。
- 自报记录只活在 **App 那个进程**里：`agentisland status` / `doctor` 是另起的一次性采样，
  它的登记表永远是空的。所以 CLI 与报表里的 `provenance` 只会出现 `observed`/`inferred`/缺省，
  **读到这些值不等于「这个 Agent 没自报过」**；带令牌的声明只在岛内（卡片、货架、详情）参与显示。
  打通这条读通路是另一件事（要嘛给 loopback 加读端点，要嘛走 MCP）。
- 可信自报会**压过兜底推断**：带令牌、TTL 内的申报直接决定卡片写什么（副标题挂一格「 · 自报」），
  但它不改变进程表与 CPU 的采集。与**会话日志强语义**对不上时两条都给——「自报说 X，进程表说 Y」，
  而状态仍按观测那一侧走；TTL 一到期就回到推断口径，不会留下一个持续挂着的冲突标记。
- 没令牌而降级的申报走本机无鉴权入口：正文会进岛内横幅与系统通知（带「外部投递」标记），
  但它不建立任何可信状态，也不参与上面这一维。
