# UI 素材/灵感站点集群：agent 界面正在形成自己的组件词汇表

> 目的：本文是**横向对比一批站点的单篇案例**，不是逐个抄目录。同一批抓取里六个站点
> 分成三类（组件库 / 灵感档案 / 3D），其中两个站点——Beautiful UI 与 BoardUI——明确把
> 自己定位成 "AI-native / agentic interface" 的设计系统，它们列的组件清单与 AgentIsland
> 已有的五态、货架、通知策略高度重合。这个重合是本文最该看的部分。
> 全部一手事实来自 `/tmp/uibatch/` 已抓 HTML 与补充 curl，**抓取时间 2026-09-26**。
> 未核实到的内容在文末「没能核实的」里逐条列出，没有写成事实。

## 一句话结论

**这不是六个可以彼此替代的站点，是一条从素材到词汇表的演化链。** 一端是 CollectUI / Inspora
这种「看别人做了什么」的灵感档案，另一端是 Beautiful UI 与 BoardUI 这种「agent 界面该有哪些
零件都给你列好了」的组件系统。真正值得本仓记录的不是任何一家的动效参数，而是一个判断：
**Thinking / Approval Card / Tool Chips / Task Rows / Agent Screen 正在从各家的私货变成公共名词。**
两批互不相识的开发者分别列出的清单高度收敛，说明 agent 界面的基础词汇已经浮出来了——
而 AgentIsland 的五态正是这条词汇表在 macOS 常驻 UI 上的一个变体。

---

## 1. 技术手法与源码级细节

### 1.1 六个站点各自解决什么问题

| 站点 | 自称 / 定位 | 分发形态 | 一手出处（2026-09-26 抓取） |
|---|---|---|---|
| [beUI](https://beui.dev) | "Animated Components for React and Next.js"，124 个组件，Tailwind 4 + React 19 | shadcn CLI + `llms.txt` + JSON registry；MIT | `site-beui_dev.html` meta description；首页 "124 components · Tailwind 4 + React 19"；`/llms.txt`、`/registry.json` |
| [Beautiful UI](https://www.beautifului.dev) | "Crafted primitives for **AI-native** interfaces"；21 个组件，演示内容全是一个虚构冰淇淋店的 agent | 单页锚点全展示，MIT；作者 Shane Levine | `site-www_beautifului_dev.html` title 与 meta description；`href="#..."` 21 个；`/license` 页 |
| [BoardUI](https://www.boardui.com) | "React Design System for **Agentic Interface**"；72 组件 + 17 图表卡 + 8 Pro 模板 + 400+ 设计 token，Figma 先画 | 自建 `boardui` CLI + MCP + shadcn schema registry；Free → Pro 付费 | `site-www_boardui_com.html` 正文；`/llms.txt` 全文 |
| [ThreeUI](https://threeui.com) | "Three.js components, templates and interactive shaders" | npm 包 `@designcodeio/threeui` + 目录式分类页；Pro $199 终身 / $99 一年 | `site-threeui_com.html`；`/pricing`；`/installation` |
| [Inspora](https://www.inspora.design) | "A curated archive of recent visual design and creative work"，**Updated hourly** | 精选档案，无代码 | `site-www_inspora_design.html` meta description |
| [CollectUI](https://collectui.com) | "Daily User Interface Inspiration / Curated by Design Magicians" | 每日聚合，无代码 | `site-collectui_com.html` meta description + og:description |

三类的分工很干净，不重叠：

- **组件库**（beUI / Beautiful UI / BoardUI）：给的是能装进项目的源码。
- **3D 场景库**（ThreeUI）：给的是完整页面级 WebGL 场景与变体，不是控件。
- **灵感档案**（Inspora / CollectUI）：给的是"别人做了什么"，没有源码、没有设计说明。

### 1.2 Beautiful UI 的 21 个组件：一整套 agent 词汇表

这是整批站点里对本仓参考密度最高的一份清单。首页按 `01`–`21` 编号逐个展示，
每个都有一句自己的定位原话（以下全部照抄自抓取 HTML）：

| # | 组件 | 原文一句话 | 演示里的具体内容 |
|---|---|---|---|
| 01 | Loading State | "Pixel-grid loader with shimmer and elapsed time." | 五个变体名：Churning / Drive / Dots / Orbit / Surfer，带 `0.0s` 计时 |
| 02 | Thinking | "Expandable traces — steps, reasoning, search, coding." | 四个 tab：Thinking / Steps / Reasoning / Search / Coding |
| 03 | Streaming Text | "Streamed answer with inline sources, actions, and follow-ups." | `10 sources` + 三个带域名来源 + "Follow-ups" 两条追问 |
| 04 | Approval Card | "**Human-in-the-loop questions the agent asks before acting.**" | "How many flavors should we launch?" 三个选项 + `1 / 3` 进度 + Skip / Continue |
| 05 | Tool Chips | "Code edits and tool calls as compact chips." | `4 tool calls, 2 messages` |
| 06 | Task Rows | "Live agent task status — running, failed, completed." | 分三组编号（组末是 `2` / `3`），组内每条带计数与百分比：`12 suppliers`、`12/12`、`0`、`7 SKUs`、`68%`、`2 messages`；状态含 Completed / draft |
| 07 | Chat | "Tabbed chat panel with reasoning replies and a composer." | 两个 tab（Flavors / Suppliers）+ 每条回复各带 "Flavor Data **for 4s**" / "Trend Detection **for 2s**" |
| 08 | Prompt Bar | "Composer with @ sources, / commands, model picker, and dictation." | `@` / `/` 触发 + 模型选择 + Rounded / Pill 两种形态 |
| 09 | Recommendation Card | "Agent suggestion with a **confidence meter** and actions." | "Want me to place this restock order?" + 三个带置信度标签的选项：`No signal` / `High confidence` / `Needs review` |
| 10 | Context Cards | "Retrieved knowledge chunks with their sources." | `All chunks 32`，每条带字符数（`290 characters`、`1,250 characters`）与 PDF / CSV 来源文件 |
| 11 | Diff Table | "AI-proposed edits sweeping through tabular data." | "Proposed menu cleanup" 表格 |
| 12 | Records Table | "CRM-style grid with tags, sorting, and relationship status." | 含 `Connection strength`（Very strong / Weak / Very weak）与 `No communication` |
| 13 | Filter Table | "Status chips that reorganize live data." | All 5 / To do 2 / In Progress 2 / Completed 1 |
| 14 | Sidebar Nav | "Collapsible workspace and chat navigation with gliding hover states." | 含 `3/10`（已用 3 / 配额 10）这种额度表达 |
| 15 | Search | "Command search with live filtering and an empty state." | 明确把 empty state 算作组件的一部分 |
| 16 | Flowchart | "Workflow trigger and condition steps on a dotted canvas." | Trigger / If-Else 节点 |
| 17 | Insight Cards | "Paged agent insights with scrub-ready live charts." | `Insights 3`，含 "The worst performer ... is Rocky Road — down -6% or -$2,453.44" |
| 18 | Code Block | "A line-numbered listing and a unified diff." | Code / Diff 双 tab |
| 19 | Fine-tune Card | "The agent adjusts design properties in an inspector." | Layout W / H / Radius / Opacity |
| 20 | Selection Actions | "Highlight a passage and hand it to the agent to rewrite." | Explain / Improve / Shorten / Tone / Grammar |
| 21 | Agent Screen | "**Watch an agent's screen — open, teach a task, record.**" | Open / Working 两态 |

注意 09 和 21 这两条：一个把**置信度**做成了一等公民（选项上直接挂 `No signal` /
`High confidence` / `Needs review`），一个把**看 agent 的屏幕**当成了一个有名字的组件。
这两件都不是"更漂亮的按钮"，是 agent 时代才出现的交互需求。

### 1.3 Beautiful UI 的演示数据集：一个虚构冰淇淋店贯穿全部 21 个组件

这是它比 beUI 强的一个手法，值得单独记：整个 demo 用同一套虚构业务数据
（`Alpine Churn — Zürich`、`Kumo Creamery — Tokyo`、`aurora-scoops`、
`Dairy Onboarding SOP.pdf`、`Sales Velocity Export.csv`，金额字符是 `$2,453.44` / `$617.22`），
21 个组件共享同一个故事。它不是 21 个孤立 demo，是一个产品的前 21 帧。
另外 `/harness` 页把这套组件拼成一个完整 app（标题 "Ice Cream Harness"，
侧栏列出 "Parking ticket appeal / Supplier records / Urgent to-dos this morning" 等会话）。

### 1.4 BoardUI 的 agent 组件：同一份词汇表的另一种切法

BoardUI 的 registry 分三桶（`/r/registry.json` 实测 63 个免费项：foundations 10 / base 37 /
application 16；Pro 另 36 项全在 `application`，见 `/r/registry-pro.json`）。
与"等待态"直接相关的免费项：

| 组件 | 原话 |
|---|---|
| `agent-thinking` | "Agent thinking indicator for chat composers — dot wave, dot spin, stars, and infinity variants with a shimmering label and **elapsed timer**." |
| `agent-log` | "Shared streaming-log machinery: the reveal ticker, the blur-in with its soft clipping edge, and the curved tree guide that draws itself. **Behind Task List and Web Search.**" |
| `agent-chat` | "A working chat app: app sidebar, streaming replies, thinking indicator, a composer pill with stop control …" |
| `agent-runtime` | "Streaming chat endpoint for agent templates: one AI_API_KEY from OpenAI, Anthropic, Google, OpenRouter, Groq, xAI …" |
| `composer-loader` | "Loading state that wraps a chat composer — an **iridescent light band orbiting the rim** with a soft inward bloom, fading in while the agent works." |

Pro 项（`registry-pro.json`）把词汇表补齐了：

- `task-list`：`Streaming agent task log: tasks reveal step by step with soft height, blur, and a shimmering running title.`
- `web-search`：`Streaming research trail: the queries an agent ran and the sources it opened, with real site marks.`
- `agent-progress`：`Collapsible multi-step AI task progress with animated active, pending, and completed states.`
- `agent-limits-card`：`Context window usage bar with an expandable token breakdown … and plan usage limits with reset times.`
- `questionnaire`：`Plan-mode questions as a chat card: one question per step with checkbox or numbered rows, a free-text Other row, step pills and Previous / Next, sliding between questions as the card animates to each one's height.`
- `composer` / `composer-panel` / `composer-attachments`：composer 的三层拆分（含 `File tiles landing one by one above the prompt, each with its upload ring.`）

**`questionnaire` 与 Beautiful UI 的 `04 Approval Card` 是同一个需求的两个实现**：agent 要问人一件事、
需要分步答、要能跳步。Beautiful UI 给了一个 `1 / 3` 进度条 + Skip / Continue，
BoardUI 给了一个 step pills + Previous / Next。两家独立做出了同一个组件。

### 1.5 BoardUI 的 motion 源码：为什么"流式出现"要拆成五个时长

`agent-log.md`（`Accept: text/markdown` 拿到的纯 markdown，含内联源码）是这批站点里
唯一把 motion 推理写进注释的地方，值得逐条抄：

```tsx
export const SOFT_EASE = [0.22, 1, 0.36, 1] as const;

const UNIT_TRANSITION = {
  height:  { duration: 0.38, ease: SOFT_EASE },
  opacity: { duration: 0.42, ease: SOFT_EASE },
  filter:  { duration: 0.42, ease: SOFT_EASE },
  y:       { duration: 0.42, ease: SOFT_EASE },
  [REVEAL_FADE_VAR]: { duration: 0.44, ease: SOFT_EASE },
};
```

三条源码级判断（全部照抄注释）：

1. **容器先让位，文字后到位。** 注释原话：`The height runs a touch shorter on the soft
   curve so the row has finished making space slightly before the text finishes sharpening —
   the container settles first, then the words arrive, which is what reads as smooth rather
   than as a jump.` 所以 height 0.38 < opacity/filter/y 0.42。
2. **`overflow-hidden` 会切出一根硬线，用渐变遮罩软化，动画结束就把遮罩卸掉。**
   注释原话：`--bui-reveal-fade softens that edge … Once the reveal finishes, drop the mask
   entirely: no compositing layer at rest, and the guide meets pixel-exactly at every row boundary.`
   即：遮罩只在动的时候存在。
3. **引导线必须 `linear`，且用 `px / 速度` 反推时长。** 注释原话：`Timings are lengths
   divided by one pen speed, ~160px/s, so the stroke never changes pace as it hands off.
   linear matters here: an eased draw would slow at every junction and give the handoff away.`
   它还解释了为什么 42px 的线只给 0.26s：`The chain is sized against how fast the label reads,
   not against its nominal duration.`（文字 0.42s 但前陡，实际约 0.2s 就读完了）

### 1.6 beUI 的 Dynamic Island：一个与本仓同名的组件

beUI 免费组件里有一个 `Dynamic Island`（安装名 `@beui/dynamic-island`；
`/r/dynamic-island` 实测该 item 的源文件落在 `components/motion/dynamic-island.tsx`，
另带 `lib/ease.ts`、`lib/utils.ts`、`lib/hooks/use-hover-capable.ts`
与 button / number-ticker 作为 `util` 依赖；MIT，publishedAt 2026-06-10 / updatedAt 2026-09-22）。
它的注册表原话是
`iOS-style island pill that morphs between live activity views with bouncy shell resize
and blur crossfades.`。直接读 `/r/dynamic-island/raw` 拿到源码，几个决策：

```tsx
// Shell physics in Apple's duration/bounce form one long perceptual glide with
// barely-there bounce, identical in both directions. The shell animates real
// width/height (not transforms), so slots are never scale-distorted.
const SHELL_SPRING   = { type: "spring", duration: 0.8, bounce: 0.2  };
const CONTENT_SPRING = { type: "spring", duration: 0.8, bounce: 0.35 };

// Constant radius — never animated. The browser clamps it to half the shell
// height, so the pill-to-rounded-rect morph falls out of the resize for free
// with zero chance of corner glitches.
const RADIUS = 32;

const PILL_WIDTH  = 126;   // iPhone pill proportions
const PILL_HEIGHT = 37;
```

值得搬的四条：

1. **外壳动真实宽高，不动 transform**——注释给的理由是"slots are never scale-distorted"。
2. **圆角半径是常量，永不做动画**——浏览器自己会把 radius 钳到半个高度，
   胶囊→圆角矩形就"免费"得到，且 `zero chance of corner glitches`。
3. **外壳 bounce 0.2 < 内容 bounce 0.35**：注释原话 `Content gets a touch more life than the shell.`
4. **进场 blur 5px、出场 `y: -6` 且 0.08s，blur 归零**——注释原话
   `Exit gets sucked up into the pill — fast, blur-free, before the shrinking shell can clip it.`
   即：出场比进场短一个量级，且**出场不加模糊**（加了会被缩小中的外壳切出毛边）。
   两条路径都由 `useReducedMotion()` 分流，reduce 时只剩 opacity 与 `duration: 0`。

它的 `EqBars`（音乐视图的均衡器）用了 `const BAR_DELAYS = [0, 0.18, 0.09, 0.27]`
配 `scaleY: [0.4, 1, 0.55, 0.9, 0.4]`——**非均等的相位差，且关键帧本身不是正弦**
（0.4→1→0.55→0.9→0.4）。这与 01 号案例 3dicon 的"同步是最响的破绽"是同一条。

### 1.7 分发模式：shadcn copy-paste 与"给 agent 用的接口"是同一件事

这批站点在分发上有一个共同演化：**先给人一个安装命令，再给 agent 一套接口。**

beUI 的 `llms.txt` 与 `docs/ai-agents.md` 明说了它同时提供三条通道
（`/llms.txt` 是 markdown 发现索引，`/r` 是 JSON 目录，`/registry.json` 是 shadcn 目录格式）：

```
- Registry index (JSON): https://beui.dev/r
- Component detail (JSON): https://beui.dev/r/{slug}
- Raw source (text/plain): https://beui.dev/r/{slug}/raw
- Component Markdown: https://beui.dev/components/{category}/{slug}.md
```

并且它把"agent 该怎么用"写在第五步 `Usage for agents`：抓索引 → 抓单项 JSON →
把文件写到声明路径 → 装 `dependencies`。它还给了一个 MCP（`https://mcp.beui.dev/mcp`，
四个工具 `list_components` / `search_components` / `get_component` / `get_install_command`）
和一个 agent skill（`npx skills add starc007/ui-components --skill beui`）。

BoardUI 更进一步，把它写成了**优先级顺序**（`/llms.txt` 原文）：

> 1. **MCP over stdio** — `npx -y boardui@latest mcp`。Full capability: discovery, source,
>    *and* writing files, project init, agent rules, and Pro license activation.
> 2. **MCP over HTTP** — `https://www.boardui.com/mcp` … Read-only tools …
> 3. **Plain HTTP** — `GET /r/registry.json` …

它还有一句直接对 agent 说的话，值得本仓记：

> **You are a coding agent with filesystem access.** Use the MCP server and install real
> source rather than reproducing components from a screenshot or from memory.

ThreeUI 的 `/mcp` 页最短，一句话：`Connect the authenticated ThreeUI MCP to access every
template, component, prompt, and source file with a verified Pro membership.`——
它的 MCP 是 Pro 的付费闸门，不是免费获客手段。

### 1.8 灵感档案站的做法（以及为什么它不值得多抄）

Inspora：`Updated hourly`，分类 All / Web / Branding / Product / Motion / Illustration /
3D / Print；每条是视频或图，字段含 `creator`（如 `@ShohanUIX`）、`createdAt`、
`mediaCount`、图片带 `variants`（640 / 960 两档 webp 与原始 1091×1200）。
首页 flight data 里实测 16 条，标题如 `Chat component interaction`、`Session progress and
recovery timeline`、`macOS Folder Icon Timeline`、`Liquid metal`。
**没有任何文字说明它为什么好**——这是它最大的信息损失。

CollectUI：`Daily Design Inspiration / Curated by Design Magicians`，
承诺 `Subscribe to receive hand-picked websites and interfaces curated by top creatives
and delivered daily.`；有 Home / Designers / Categories / Trending / Submit Design 五个入口，
Trending 页有 `Top Designers by post count` 与 `Top Designs by Clicks`，时间窗
All Time / This Week / This Month。它的实际内容靠客户端渲染，
`curl` 拿到的 `/categories`、`/trending` 页正文都停在骨架与 `Loading...`
（详见文末「没能核实的」）。它自己的赞助区倒是把社群体量写明了：
`Trusted by Framer, Adobe, Mobbin & 100+ design tools`、`1M+ monthly impressions`、`DA 60+`。

ThreeUI 的规模（用首页 href 计数，2026-09-26 实测）：八个分类共 **457 个条目链**，
按分类是 three-js 119 / backgrounds 104 / buttons 62 / landing-pages 51 /
ui-elements 38 / hero 33 / motion-design 24 / text-animation 15 / css 11。
它的条目描述粒度值得学：不说"一个好看的 hero"，而说
`the complete Meridian revenue-platform page, preserved unchanged with the whole Three.js
r155 module inlined into the document, so the page renders its Earth-from-orbit scene
without making a single network request.`——**它把"零网络请求"当成卖点写进标题描述**。
它还保留原始作者的修订要求当作文档（如 `One owner-requested optics revision sits over the
supplied export: the barrel stops reading as see-through …`）。

---

## 2. 可迁移的决策规则（原话，不是转述）

### 规则一：先把"给谁看"分层，再谈组件

BoardUI 的 `llms.txt` 把同一套东西写给三种读者（人、agent、Figma），且**明确列出不该用它的情况**：

> Do not reach for BoardUI when:
> - You want headless, unstyled behavior only. …
> - You want a component library you can upgrade with `npm update`. BoardUI hands over source
>   you own; there is no runtime package to bump.
> - You want a hosted UI service. Nothing here is called at runtime.

> 对我们：这是"已知限制"的正确写法。README 的已知限制今天写得很好，但这条规则更普适——
> **写清"不解决什么"，比写清"解决什么"更能防误用**。本仓的「不碰网络、不转发流量、不持有密钥」
> 属于同一条，只是散在 CONTEXT 与 README 两处。

### 规则二：agent 界面需要哪些零件，已经可以列成清单

这是本文的核心规则。两家独立收敛出的公共词汇（左边 Beautiful UI，右边 BoardUI）：

| 需求 | Beautiful UI | BoardUI |
|---|---|---|
| 等待态 | `01 Loading State`（Churning / Drive / Dots / Orbit / Surfer，带 elapsed time） | `agent-thinking`（dot wave / dot spin / stars / infinity + elapsed timer）、`composer-loader`（iridescent light orbiting the rim） |
| 我在想什么 / 做过什么 | `02 Thinking`（Steps / Reasoning / Search / Coding） | `agent-log`（reveal ticker + blur-in + tree guide）、`web-search` |
| 多步任务的进度 | `06 Task Rows`（running / failed / completed） | `agent-progress`、`task-list` |
| 要人拍板 | `04 Approval Card`（`1 / 3` + Skip） | `questionnaire`（step pills + Previous/Next + free-text Other） |
| 工具调用的可见性 | `05 Tool Chips`（`4 tool calls, 2 messages`） | `tool-result`（beUI 侧）、`web-search` |
| agent 提的建议 + 不确定度 | `09 Recommendation Card`（confidence meter） | 无直接对应 |
| 看 agent 的屏幕 | `21 Agent Screen`（"open, teach a task, record"） | 无直接对应 |
| 用量/额度 | `14 Sidebar Nav` 里的 `3/10` | `agent-limits-card`（context window + plan limits + reset times） |

> 对我们：这不是"去抄两个 React 库"（本项目 SwiftUI/macOS，抄不到），而是一个**外部佐证**——
> 别人在把 agent 的状态分成"在等 / 在想 / 在做 / 要你批 / 已做完"五类，并各自给了名字。
> AgentIsland 的五态（`working` / `attention` / `completed` / `idle` / `offline`）是同一张表
> 在常驻监控场景的投影，方向一致；差在我们没有给"在想什么"留位置（详见第 4 节）。

### 规则三：把等待时长与计时器显式做进组件

三家都自发做了同一件事，且措辞几乎一样：**等待态必须带一个走过的时间。**

- Beautiful UI `01 Loading State`：`Pixel-grid loader with shimmer and elapsed time.`（demo 里是 `0.0s`）
- Beautiful UI `07 Chat`：每个步骤标 `for 4s` / `for 2s`
- BoardUI `agent-thinking`：`a shimmering label and **elapsed timer**`

这与 03 号案例 libraries-dev 的"按等待时长定效果"是同一件事的两半：
libraries-dev 说 **≥2s 才该给反馈**，这批站点说 **给了反馈就要显示已经等了多久**。
两者合起来才是一条完整规则。

> 对我们：灵动岛的 `working` 态已经有 `workingSince` 一类的时间量（面板与详情在用），
> 但**收起态的微细条上没有一个计时读数**。外部三个站点一致认为"计时器"是等待态的一部分，
> 不是可选装饰。

### 规则四：置信度与失败要显示成现象，不是藏起来

Beautiful UI 的 `09 Recommendation Card` 把三个选项分别标成 `No signal`、`High confidence`、`Needs review`；
`12 Records Table` 的关系强度分档 `Very strong / Weak / Very weak / No communication`。
BoardUI 的 `agent-progress` 明说 `animated active, pending, and **completed** states`，
`task-list` 的任务有 `draft` 这种未完成标记。

> 对我们：与 README 已知限制里「读不到 ≠ 闲着」、CONTEXT 的 `provenance`
> （`observed` / `inferred` / 缺省）完全同向。外部两个库把 confidence 做成了**挂在选项上的一等标签**，
> 我们的 `自报说 X，进程表说 Y` 也已经是这个形态——这条是跨案例共识的第四条，
> 三篇各自独立得出同一结论。

### 规则五：给 agent 的接口优先级是"能写文件 > 只能读"

BoardUI 写成了显式优先级（stdio MCP 能写文件 > HTTP MCP 只读 > 纯 HTTP JSON），
beUI 的 `Usage for agents` 也是同一个顺序（抓 JSON → 写文件 → 装依赖）。

> 对我们：本项目今天有一条 `POST /session` 给 agent 自己上报状态、带令牌鉴权
> （README 有载）。这条规则的反向提示是：**给 agent 的读接口，优先级应该低于写接口**——
> 或者说，只读接口会诱使 agent 去猜。本仓已有一个反面教材写在 README 里：
> `pid 只能否掉一条申报，永远不能建立可信度`。将来若开放任何读接口，
> 应同时回答"那 agent 会不会用读数替代申报"。

### 规则六：组件命名要能当名词用

这批站点的命名密度很高，而且**名字本身就在描述职责**：
`Approval Card` / `Tool Chips` / `Task Rows` / `Prompt Bar` / `Recommendation Card` /
`Context Cards` / `Diff Table` / `Insight Cards` / `Agent Screen`；
BoardUI 是 `agent-thinking` / `agent-log` / `agent-progress` / `agent-limits-card` /
`web-search` / `task-list` / `questionnaire`。两家都不约而同用**复数**表示"一类内容"
（Cards / Chips / Rows / Actions），用**单数**表示"一个交互面"（Card / Bar / Screen）。

> 对我们：灵动岛面板的"列表 / 详情 / 会话"三级导航在 README 有载，
> 但缺一套对外说得出口的词。若做侧边栏主形态，先有名词再有界面会比先有界面再补名词便宜。

### 规则七：把"没有内容"当组件设计，不是异常

Beautiful UI 的 `15 Search` 原话把空状态算进组件定义：`Command search with live
filtering and an empty state.`。BoardUI 的 `agent-chat` 有一条 `a setup notice when no
provider key is set`，`agent-runtime` 有 `a config probe for unconfigured deploys`——
**未配置状态是组件的一个受设计状态，不是错误分支**。

> 对我们：五态里的 `offline` 与 `idle` 就是"没有内容"的两个状态。这一点与 01 号案例
> （Slingshot Lamp 的 `blown (switch on)`）与 CONTEXT 的「确认请求解除后不补完成事件」
> 是同一条。**空态要有自己的样子，不是把内容区留白。**

### 规则八：motion 的时长按"读起来多快"定，不按"名义多长"定

BoardUI `agent-log` 的注释：`The chain is sized against how fast the label reads, not
against its nominal duration. The blur-in runs 0.42s but on a steeply front-loaded curve,
so the words have settled by roughly 0.2s; sizing the guide to the full 0.42s left it
visibly trailing text that was already done.`

> 对我们：这与 01 号案例 3dicon 的"降帧率毁掉缓慢平滑运动"、02 号 Slingshot Lamp 的
> 不对称起落是同一族手艺：**动多久要由感知决定，不由参数表决定**。灵动岛的
> 玻璃卡片过渡、微细条弹出都是候选。若只搬一条：先看有效曲线，再看 duration。

### 规则九：动效是分级的，reduced-motion 是"设计过的状态"

beUI 的 Motion Guides 写得很直接（`docs/motion-patterns.md` 原文）：

> 1. **Check frequency.** Repeated actions should feel nearly instant. Save expressive motion for rare moments.
> 2. **Name the purpose.** Motion should explain space, confirm input, show state, or soften a change.
> 3. **Choose the physics.** Use ease-out for entrances, ease-in-out for movement, linear motion for progress, and springs for gestures.
> 4. **Design the fallback.** Reduced motion should keep useful opacity and color feedback while removing travel, scale, parallax, and overshoot.

时长表（原表照抄）：

| Interaction | Range | Desired feel |
|---|---|---|
| Press feedback | 100–160ms | Immediate and physical |
| Tooltip or popover | 125–200ms | Quick and origin-aware |
| Dropdown or select | 150–250ms | Responsive, with no waiting |
| Modal or drawer | 200–500ms | Enough time to explain space |
| Marketing demo | Flexible | Clarity matters more than speed |

> Under 300ms is the default for interface motion. Longer motion belongs to explanatory
> demos, deliberate gestures, and large spatial changes.

> 对我们：这是给"动效准入门槛"（03 号案例的建议 1）补上的一套具体数字。第 1 条尤其值得记：
> **高频动作要近乎瞬时**——灵动岛是常驻 UI，任何每天看几百次的东西都落在"repeated"这一档。
> 第 4 条则与 README 已知限制里那条"动效减少只覆盖 3 个文件，不是全部"直接对上：
> 外部独立来源也认为 reduce 时**只保留 opacity 与颜色**，删掉 travel / scale / parallax / overshoot。

---

## 3. 一手证据 / 本地验证结果

全部来自 2026-09-26 抓取。`/tmp/uibatch/` 为既存快照，curl 为当日补抓（collectui 直连可达，
其余多数走 `-x http://127.0.0.1:10808`）。

### 3.1 可数的事实

| 站点 | 数字 | 取证方式 |
|---|---|---|
| beUI | 首页横幅 "124 components · Tailwind 4 + React 19" | `site-beui_dev.html` 正文 |
| beUI | `/registry.json` 实测 **124 个 item**（motion 98 / agents 19 / charts 7） | `curl ... /registry.json \| python3 -c 'import json,sys;print(len(json.load(sys.stdin)["items"]))'` |
| beUI | `/r` 实测 **89 个**（motion 42 / blocks 23 / agents 17 / charts 7）——与 124 不一致，见「没能核实的」 | 同上换 `/r` |
| beUI | 页脚分类计数：Components `View all (42)`、Blocks `View all (23)`、AI Agents `View all (17)` | `site-beui_dev.html` 页脚 |
| beUI | 免费 vs Pro：Pro 声称 `220+ premium blocks`、`Agent Skill, MCP access, and one-command installs` | 首页 "Free and Pro" 段 |
| beUI | MIT（`LICENSE` 指向 `github.com/starc007/ui-components/blob/main/LICENSE`）；GitHub 实测 **1684 star / 85 fork / MIT**（2026-09-26） | GitHub API |
| Beautiful UI | **21** 个组件，编号 `01`–`21`，21 个 `href="#..."` 锚点一一对应 | 记数组 `01..21` + `grep -oE 'href="#[a-z-]+"' \| sort -u \| wc -l` → 21 |
| Beautiful UI | MIT（`/license`：`Copyright (c) 2026 Shane Levine`）；页脚 `Ice Cream Harness` | `/license` 页正文 |
| Beautiful UI | 演示内容全部挂在同一套虚构冰淇淋店数据上（`aurora-scoops` / `Dairy Onboarding SOP.pdf` / `Sales Velocity Export.csv` / `$2,453.44`） | 首页 flight data 与正文 |
| BoardUI | `72 unique components, 17 data charts, 8 templates and 400+ design tokens`；`React v19.2` / `Tailwind CSS v4` / `React Aria v1.17` / `TanStack Table v8.21` | 首页正文 |
| BoardUI | `/r/registry.json` 实测 **63** 项（foundations 10 / base 37 / application 16）；`/r/registry-pro.json` 实测 **36** 项（全 application） | 两处 JSON 计数 |
| BoardUI | 价目：`€179 / €149`（1 年更新）、`€249 / €199`（终身，含 `€49 launch discount`）、`€349`（最多 5 人）；3,000+ 图标库 | 首页 Pricing 段 |
| BoardUI | 免费档含 `MCP support for AI agents`、`Design tokens based on Tailwind CSS`、`Dark mode variables`、`Discord access` | 同上 |
| ThreeUI | 八个分类 **457** 个条目链 / 180 个 distinct 页面 | `grep -oE 'href="(/[a-z0-9-]+/[a-z0-9-]+...)"' \| 分类计数` |
| ThreeUI | Pro `$199 lifetime or $99/year`；npm 包 `@designcodeio/threeui` | `/pricing`、`/installation` meta description |
| ThreeUI | og:image:alt 自称 `Open Source. 6.2k GitHub Stars.`——**这是站点自述，未独立核实** | og:image:alt |
| Inspora | `Updated hourly`；7 个分类；首页首屏实测 **16** 条（字段含 `slug` / `title` / `creator` / `mediaCount` / `media[].variants`） | flight data `slug/title/creator` 解析 |
| CollectUI | 5 个入口（Home / Designers / Categories / Trending / Submit Design）；Trending 有两个榜 × 三个时间窗 | `/` 与 `/trending` 正文 |
| CollectUI | 赞助区自述 `Trusted by Framer, Adobe, Mobbin & 100+ design tools`、`1M+ monthly impressions`、`DA 60+`、`48h listing approval` | 首页赞助段 |

### 3.2 语言与措辞的收敛（本文最关键的证据）

两个站点、两批作者、都没有提及对方，却用了同一定语：

| | Beautiful UI | BoardUI |
|---|---|---|
| 站点级自述 | title：`Crafted primitives for **AI-native** interfaces` | hero：`React Design System for **Agentic Interface**`（`sr-only` 的可访问名，视觉上由 CSS 重复排成多行） |
| meta description | `A small library of extremely crafted, copy-paste components for chat agents, thinking states, human-in-the-loop approvals, and everything agents need to talk to humans beautifully.` | `Build **agentic products** and data-rich dashboards with 72 React components and blocks, 17 of them chart cards …` |
| llms.txt 首句 | 无 llms.txt（`/llms.txt` 返回 404） | `... a design system for dashboards and **agentic interfaces** ...` |
| 用例措辞 | `human-in-the-loop approvals` | `plan-mode questionnaires` |

BoardUI 的 `/llms.txt` 甚至把"该不该用我"的判断标准写成了 `When to use BoardUI` /
`Do not reach for BoardUI when` 两段；beUI 的 FAQ 也是同一结构（`Is beUI free for
commercial projects?` / `Do I need Pro to use the free components?`）。
**三个组件库都在首页回答"什么时候别用我"**——这是组件库成熟的标志，不是营销话术。

### 3.3 本机环境事实

- `curl https://collectui.com/` **直连 200**（29369 字节），走代理同样 200，字节数一致。
  也就是说 collectui 抓不到内容**不是被墙**，是它客户端渲染（见下）。
- 其余站点（beui.dev / boardui.com / threeui.com / beautifului.dev / inspora.design）
  当日补抓均需 `-x http://127.0.0.1:10808`；`beautifului.dev` 裸域名返回 `308` 重定向，
  需要 `-L` 跟到 `https://www.beautifului.dev`。
- `grep -c 'self.__next_f.push' site-www_inspora_design.html` = 1：Inspora 的首屏内容
  全在 Next.js flight data 里，**必须解析 `self.__next_f` 才能拿到条目**，
  只去 `<script>` 再抽正文会得到 13 行壳。

### 3.4 与已有三篇的口径交叉

- 03 号（libraries-dev）说 `< 2 秒什么都不加`；Beautiful UI 与 BoardUI 两家都在等待态上
  挂了 elapsed time。**一条规则的两半在两个完全不相干的来源上对上了。**
- 01 号（Slingshot Lamp）说状态要显示实际现象；`09 Recommendation Card` 的
  `No signal` / `Needs review` 与 BoardUI 的 `pending` 是同一手法。
- 02 号（3dicon）说同步是最响的破绽；beUI `EqBars` 的 `[0, 0.18, 0.09, 0.27]` 相位差
  + 非正弦关键帧是同一手法在 React 侧的实现。
- 01 号案例里核实到的「本项目 reduce-motion 只覆盖 3 个 View，不是全部」这条仍然成立
  （README「已知限制」已按代码重写；键名是 `@Environment(\.accessibilityReduceMotion)`
  而非 Web 侧拼法 `prefers-reduced-motion`，已接的是 `DockedSliver` 2 处、
  `AgentRingView` 3 处、`ActivityMatrixDots` 1 处）。
  beUI 的 Motion Guides 第 4 条给了一份可照抄的 fallback 定义
  （保留 opacity 与颜色，去掉 travel / scale / parallax / overshoot）——
  它正好可以作为"剩下的 View 逐个判"时的判据。

---

## 4. 应用建议清单（与实现解耦，尚未排期）

要做哪条就开 issue，不直接改。

1. **给"在想什么"留一个位置。** 两家的词汇表里都有 Thinking / Reasoning / Search / Coding
   这一层（Beautiful UI 的 `02 Thinking` 四个 tab；BoardUI 的 `agent-log` + `web-search`），
   而 AgentIsland 五态里 `working` 是一个无内部结构的点。侧边栏主形态里，
   `working` 展开后是否有"在想/在搜/在跑"的三分，是第一个可问的问题。
   *注意：这一条需要先确认能拿到什么——今天只从会话文件提取 `attention` / `completed`
   两类强语义（README 判定优先级有载），中间过程拿不到就不能假装有。*
2. **`attention` 的呈现直接参考 Approval Card / Questionnaire 的共用形态。**
   两家的共识是：一个问题、可多选或单选、有进度（`1 / 3` 或 step pills）、
   有一句 "agent 在动手前问人" 的说明（Beautiful UI 原话
   `Human-in-the-loop questions the agent asks before acting.`）。
   本仓 `attention` 的定义是「存在未处理的用户确认或授权请求」，语义完全重合。
   今天它在岛内只有一个态点；若要展开，进度与跳过都是必要件，不是加分项。
3. **等待态显示计时读数。** 三家独立做了 elapsed time。本仓 `working` 态已有时间量，
   建议评估的是**收起态/微细条**上要不要一个最小读数（哪怕只 `12s`），
   以及它是否违反 03 号案例的「<2s 不动」——注意不违反：计时只在 ≥2s 后出现。
4. **`completed` 的呈现参考 Task Rows 的分组计数。** Beautiful UI 的 `06 Task Rows`
   每组带一个数（`12/12`、`0 2`、`7 SKUs`、`68%`），读起来是"这一组完成了多少"。
   本仓已完成事件带 token 用量与模型名（详情页有载），货架那一行的密度可以参考这种
   "一組一个数"的组织方式，而不是堆更多文字。
5. **任务货架（shelf）的副标题不要塞来源标签。** `code-review/2026-09-25-0938-v0.0.134-codex.md`
   已经判定货架副标题要处理「可信自报」冲突。Beautiful UI 的做法是把置信度
   做成**挂在选项上的短标签**（`No signal` / `High confidence`），不长篇解释。
   若货架要表达冲突，一个短标签比一句说明更接近外部惯例。
6. **给侧边栏先立名词，再画界面。** 参考第 2 节那条公共词汇表，挑本仓真正要用的
   十几个词写进 `CONTEXT.md` 术语（如「等待态 / 问询 / 任务行 / 建议卡」），
   避免"列表/详情/会话"之外全靠指代。
7. **若做任何动效，按 beUI 的四问与时长表校准**：高频（微细条、呼吸灯、卡片过渡）
   落在 `repeated → 近乎瞬时` 这一档；`attention` 是唯一天然 ≥2s 的状态，
   可以按 libraries-dev 的口径给足反馈但**仍不超过 300ms 的界面动效上限**。
8. **reduced-motion 的 fallback 直接抄 beUI 第 4 条**（保留 opacity 与颜色，
   去掉 travel / scale / parallax / overshoot）。这能把 README 已知限制里那条
   「动效减少只覆盖 3 个文件，不是全部」从缺口变成有定义的工作项。
9. **不要试图 `npm install` 这批库。** 本项目 SwiftUI/macOS，`beui` / `boardui` / `threeui`
   的产物对无可调用组件（同 03 号案例的结论）。但 beUI 的 `Dynamic Island` 源码
   （外壳动真实宽高、圆角常量、出场比进场短且不加 blur）**可以直接当设计参考读**，
   不需要装任何东西。
10. **给 agent 的读写接口，想清楚谁优先。** BoardUI 的优先级（stdio 可写 > HTTP 只读 >
    纯 HTTP）反过来提醒：只读接口会诱使 agent 用读数替代申报。本仓已有
    `pid 只能否掉一条申报，永远不能建立可信度` 的口径，将来加读接口时要先回答这一条。

---

## 与本仓的关系

- 本篇**不引入任何依赖**，不 `npm install`，不改任何 Swift 代码。
- 交叉印证的对象：README 的五态定义与判定优先级（`attention` / `completed` / `working` /
  `idle` / `offline`）、`Sources/AgentIslandCore/SettingsStore.swift:414` 的
  `NotificationPolicy`（standard / focus / silent 三档）、
  `CONTEXT.md` 的 `provenance` 与「读不到 ≠ 闲着」。
- 它是本篇唯一没有"外部动效案例"性质的一篇：**动效参考集中在 1.5 / 1.6 / 2.8 / 2.9 四节**，
  其余是组件词汇与分发模式的观察。

---

## 取证命令

```sh
# ── 0. 环境 ──────────────────────────────────────────────
# 本机 *.twimg.com 直连不通需 127.0.0.1:10808；collectui 例外，直连即可
P='-x http://127.0.0.1:10808'

# ── 1. beUI：组件数、agent 通道、motion 规则 ─────────────
curl -sSL https://beui.dev/llms.txt                       # 发现索引 + "Usage for agents" 五步
curl -sSL https://beui.dev/docs/ai-agents.md              # skill / MCP 四工具 / 六个 endpoints
curl -sSL https://beui.dev/docs/motion-patterns.md        # 四问决策框架 + 时长表 + reduced-motion
curl -sSL https://beui.dev/registry.json \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);print(len(d["items"]))'   # 124
curl -sSL https://beui.dev/components/blocks/dynamic-island.md                   # 带 front-matter 的 md
curl -sSL https://beui.dev/r/dynamic-island/raw           # 真源码：SHELL_SPRING/RADIUS/PILL_*
curl -sS https://api.github.com/repos/starc007/ui-components \
  | grep -E '"(stargazers|forks)_count"|spdx_id'          # 1684 / 85 / MIT（2026-09-26）

# ── 2. Beautiful UI：21 个组件清单与措辞 ─────────────────
curl -sSL https://www.beautifului.dev/ -o beautifului.html   # 注意裸域 308，要 -L
grep -oE 'href="#[a-z-]+"' beautifului.html | sort -u | wc -l   # 21
grep -oE '<meta name="description" content="[^"]*"' beautifului.html
grep -oE '<title>[^<]*' beautifului.html                     # "AI-native interfaces"
curl -sSL https://www.beautifului.dev/license | grep -i 'copyright'   # Shane Levine, MIT
curl -sSL https://www.beautifului.dev/harness                # "Ice Cream Harness" 完整 app

# ── 3. BoardUI：agentic 词汇表 + runtime 规模 ────────────
curl -sSL https://www.boardui.com/llms.txt                   # "When to use / Do not reach for"
curl -sSL https://www.boardui.com/r/registry.json \
  | python3 -c 'import json,sys,collections;d=json.load(sys.stdin);print(collections.Counter(i.get("boarduiType") for i in d["items"]))'   # 63
curl -sSL https://www.boardui.com/r/registry-pro.json \
  | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["items"]))'          # 36
curl -sSL -H 'Accept: text/markdown' https://www.boardui.com/components/agent-log \
  | sed -n '1,120p'                                          # UNIT_TRANSITION / pen-speed 注释
grep -c 'Agentic' /tmp/uibatch/site-www_boardui_com.html                 # 1 次（计入正文那次）
grep -o 'sr-only[^>]*>Agentic' /tmp/uibatch/site-www_boardui_com.html | head -1

# ── 4. ThreeUI：规模与分类 ───────────────────────────────
grep -oE 'href="(/[a-z0-9-]+/[a-z0-9-]+(?:/[a-z0-9-]+)*)"' /tmp/uibatch/site-threeui_com.html \
  | cut -d/ -f2 | sort | uniq -c | sort -rn                  # 457 条 / 8 分类
curl -sSL https://threeui.com/pricing                        # $199 lifetime / $99 per year
curl -sSL https://threeui.com/mcp                            # "verified Pro membership"

# ── 5. Inspora：解析 flight data（去 script 再抽正文会只剩壳）──
python3 - <<'PY'
import re
s=open('/tmp/uibatch/site-www_inspora_design.html',encoding='utf-8',errors='replace').read()
u=s.replace('\\"','"')
for m in re.finditer(r'"slug":"([a-z0-9-]{3,60})","title":"([^"]{2,80})","creator":\{"name":"([^"]+)"',u):
    print(m.group(3), '|', m.group(2), '|', m.group(1))
PY                                                          # 16 条（2026-09-26）

# ── 6. CollectUI：直连即可，但内容是客户端渲染 ─────────────
curl -sSL https://collectui.com/ | grep -oE '<title>[^<]*'
curl -sSL https://collectui.com/trending | grep -oE 'Top (Designers|Designs)[^<]*'
curl -sS  https://collectui.com/categories/__data.json       # {"nodes":[null,null]} → 客户端取数
```

---

## 没能核实的（不写成事实）

1. **CollectUI 的条目内容完全没拿到。** `/`、`/categories`、`/trending` 三个页面 curl
   都返回 200，但正文停在骨架与 `Loading...`——内容由 SvelteKit 客户端取数。
   `/categories/__data.json` 返回 `{"type":"data","nodes":[null,null]}`，
   `/__data.json` 同。本机没有浏览器自动化去跑它，**所以本文没有收集到 CollectUI 的
   任何一条具体设计、分类名或设计师名**，只用了它自己写明的入口名与赞助区自述数字。
   （直连与走代理字节数一致，所以不是"被墙"，已排除这一个解释。）
2. **beUI 的组件数口径不一**：首页横幅写 `124 components`、`/registry.json` 实测 124，
   但 `/r` 索引只返回 89、页脚三个分类计数相加也是 89（42+23+17）。差异原因未查
   （可能是 free/Pro 或索引是否含 charts 变体）。**本文写"124"时只用于首页横幅与
   registry.json 口径，并在表中并列记下 89 这个数。**
3. **ThreeUI 的 6.2k GitHub Stars 未独立核实。** 只有它自己 og:image:alt 的
   `Open Source. 6.2k GitHub Stars.`。抓取 HTML 里没有任何 `github.com/...` 链接，
   `api.github.com/repos/designcodeio/threeui` 返回 404（可能改名或非该组织）。
4. **ThreeUI 的 MCP 具体工具清单没拿到。** `/mcp` 页正文只有一句话
   （`Connect the authenticated ThreeUI MCP … verified Pro membership.`），
   没有列工具名；beUI 与 BoardUI 都列了。
5. **BoardUI 首页 "Agentic Interface" 在 HTML 里是 `sr-only aria-live` 的可访问名**，
   视觉上由 CSS 复制成多行装饰；直接 `grep 'Agentic'` 会在多处命中，
   不宜当作多个独立声明引用。本文只把它当 hero 主标语用了一次。
6. **Beautiful UI 没有 `llms.txt`**（`/llms.txt` 返回 404，首页也没有 `llms.txt` 链接），
   也没有暴露 registry JSON。它的 agent 友好度**低于** beUI 与 BoardUI——
   本文没有推测它"一定会加"，只记今天没有。
7. **Beautiful UI / BoardUI 的作者身份除 BoardUI 外未核实。** Beautiful UI 的
   `/license` 确认 `Copyright (c) 2026 Shane Levine` 且由 cal.com 链接（`shane-levine-7bnfdw`）
   指向同一人；BoardUI 页脚署名 `Mertcan Esmergül`（`x.com/sitenley`），
   其余站点未署名。
8. **Beautiful UI 的 21 条 demo 文案**（`$2,453.44`、`68%`、`12 suppliers` 等）是
   它虚构演示数据的一部分，本文只用来证明"组件带了什么信息"，
   **不作为任何真实业务数字引用**。
9. **Inspora 的分类条目数未逐项核对**：本文只解析出首页首屏 16 条
   （`mediaCount` 字段说明还有更多），没有翻页或按 category 过滤抓取。
