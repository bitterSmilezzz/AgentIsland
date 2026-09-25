# Emil Kowalski「让 AI 尝试打破你造的东西」— 一手调研

> 素材用途：**跨平台工作台改造「实现思想」的一种** —— 一种把 AI 用在
> 验收而不是实现环节的工作方式，用来校准我们的测试与验收口径。

## 出处（原始记录）

| 项 | 值 |
| :--- | :--- |
| 推文 | <https://x.com/emilkowalski/status/2103516287452483885> |
| 作者 | Emil Kowalski [@emilkowalski](https://x.com/emilkowalski)（animations.dev / [Animations](https://animations.dev) 作者，Linear 动效改进者） |
| 发布时间 | 2026-09-26 00:05 (CST) —— 由 Snowflake ID `2103516287452483885` 解析得出 |
| 抓取时间 | 2026-09-26 |
| 抓取方式 | `curl -x http://127.0.0.1:10808 'https://x.com/emilkowalski/status/2103516287452483885'`，读 `og:description`；fxtwitter 镜像 API 对本 ID 返回 404（镜像未收录），`x.com` 直连本机超时 |

## 原文（og:description 全文照录）

> One of my recent favorite AI use cases: asking it to try and break the stuff I built.
> Adding lots of data, long names, unusual emails, labels, etc. Coming up with worst case
> scenario basically.
>
> This is a vibe-coded demo, but you get the idea.

（中文大意：我最近最喜欢的 AI 用例之一，是让它**尝试搞坏我做的东西**。往里面塞大量数据、
超长名字、奇怪的邮箱、各种标签——基本上就是**构想最坏情况**。这是个 vibe-code 出来的 demo，
但你懂我意思。）

随文视频 8.28 秒（1172×1080、120fps、260 帧），内容是同一个「Members」列表在
两组数据之间来回切换，下方有一个 `Demo data / Worst case` 切换器。

## 视频里实际演了什么（逐帧核实）

本机下载视频并逐帧读屏，两侧数据是完全写死的两组：

| | Demo data | Worst case |
|---|---|---|
| 标题计数 | `5 members` | **`1,284 members`** |
| 姓名 | Sarah Chen / Marco Ruiz / Ada Owens / Tom Weber / Lena Park | **Aleksandra Wiśniewska-Kowalczyk**（折行）、**Christopher Alexander Montgomery III**、`Jo` |
| 邮箱/标识 | 无 | **`bartholomew.fitzgerald@northwind-industries-holdings.example.com`**（一行的主文本，超长折行） |
| 职位 | Design / Engineering / Product / Marketing | **Senior Product Designer, Platform Infrastructure**（折行占多行）、`—`（空） |
| 状态 | Active ×4、Invited ×1 | Active、Invited、**`Invitation expired 12 days ago`** |
| 底部 | `Showing all 5` | **`Showing 40 of 1,284`** |

读到的关键事实：**切到 Worst case 时界面没有报错、没有红框、没有 "Invalid email"，
但布局明显被撑坏**——长姓名与长邮箱折行、行高变大、一屏能显示的成员变少、
列表区出现滚动条。也就是说：**他把"最坏情况"的衡量标准定在"布局退化成什么样",
而不是"有没有报错"。**

## 它主张的方法

1. **让 AI 当破坏者，不当实现者。** 用法不是"帮我写"，而是"**想办法搞坏它**"。
2. **破坏手段是数据，不是代码。** 塞大量数据、超长名字、异常邮箱、各种标签。
3. **产出的东西是最坏情况剧本**（worst case scenario），而不是测试用例清单。
4. **他自己说这是 vibe-coded demo**——demo 本身不重要，**这个方法**才是他要传达的。

## 与 AgentIsland 工作台的对照

我们恰好有一个**天然的 worst case 输入源**，而且比 Emil 手动塞的更真实：

| Emil 手动塞的 | 我们这儿真的会有 |
| :--- | :--- |
| 1284 条数据 | 一台机器上同时跑 5–10 个 agent，每个 agent 几天几百次 token 统计 |
| 超长名字 | **agent 会话标题可以任意长**（用户项目名、分支名、prompt 摘要） |
| 异常邮箱 | **模型名 / provider 档位名来自本机配置文件，用户可写任意字符串**（`provider.rs` 将来要读的） |
| 各种标签 / 空字段 | 五态状态机里的 `inferred` / 读不到明细源 |
| 过期邀请 | **进程在但会话日志读不到**（本仓已有口径「读不到 ≠ 闲着」） |

也就是说：**这条推文对我们不是"要不要做"的问题，是"我们手里的极端输入比他的 demo 真实得多，
而我们今天没有系统性地拿它们试过"。**

当前测试的实际情况（据 `01-current-state.md` 与测试目录已有内容）：
我们有 7 条守卫测试与自建 runner，覆盖的是**逻辑分支**；
没有一轮是专门拿「超长项目名 + 1284 个文件 + 空字段」这类组合去压侧边栏与灵动岛的。

## 可以借的思想（一句话版）

> AI 最被我低估的用法，是让它**专门负责找最坏情况**——验收环节的对抗性输入，
> 由它来生成，而不是由实现它的人凭想象力列。

## 明确不适用 / 未采用

- **不引入他的 demo 代码**（推文自述 vibe-coded，无源码可核）。
- **不改成"用 AI 生成全部测试"**：那是把判断外包出去。他的做法恰恰相反——
  是他自己盯着 demo 看布局退化成什么样，AI 只负责**制造情况**。
- **不作为 Phase 1/2/3 的阻塞项**：这是验收方法，属 Phase 4 及以后的纪律。

## 与已归档素材的分工

| 素材 | 它的位置 |
| :--- | :--- |
| [08-chendahuang-cloudflare-research.md](08-chendahuang-cloudflare-research.md) | 容量/成本/是否自建轮子的选型思想 |
| 本篇（09） | **验收方法**：谁负责制造最坏情况 |
