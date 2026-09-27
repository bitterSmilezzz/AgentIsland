# 一条中文推文的五个 UI 站：agent 取件的三种接口形态，与一套按用途命名的动效 token

> 目的：一条中文推文把"把 UI 组件当积木丢给 AI Agent"当成偷懒招数，列了 5 个站点。
> 本仓关心的不是这 5 个站好不好看，而是**它教的动作在哪些站上真的成立**——
> "丢一个链接让 agent 自己扒"依赖站点是否给了机器可读的接口。
> 来源：[@x5cnhp](https://x.com/x5cnhp)（mantin，加密/web3，7,419 followers，verified）
> 2026-09-27 00:02:36 UTC 发的 note tweet，抓取时 176 赞 / 30 转 / 7 回 / 240 收藏 / 11,922 浏览（2026-09-27）。
> 唯一配图 1896×949，**已下载并逐字核对，确认是 ui.shadcn.com 首页被浏览器机翻成中文后的截图**。
> 分析时间 2026-09-27。

## 一句话结论

**这条推文教的"把链接丢给 agent，让它自己翻、自己挑、自己塞进项目"，5 个站上都能走通一部分，但走法不一样**：
3 个给了显式接口（`llms.txt` / MCP / agent skill），另 2 个只给注册表（shadcn 格式的索引 + 逐项 JSON），
而 **Beautiful UI 是唯一没有 `llms.txt`、也没有 MCP / skill 的那个**——它只有注册表，
所以 agent 得**先自己猜到 `/r/registry.json` 这个约定路径**才发现得了（本篇因此更正了 04 篇的一条旧结论，见文末）。
而 5 个站里最值得本仓搬的不是任何组件，是 transitions.dev 那套
**「按用途命名、按用途匹配」的动效 token 表**：7 个时长、6 条缓动、5 档位移、4 档缩放、3 档模糊，
每一档的注释写的是**谁在用它**（`--duration-quick: 150ms /* modal/dropdown close, text swap, tooltip appear */`），
并且它明说判据是用途而不是数字：*"a 300ms modal close still maps to `--duration-quick` (150ms)"*。

## 它是什么

一条**中文资源清单帖**，正文原句（照抄，含 emoji 与链接）：

> 写代码这关，程序员大多都能趟过去，真正把人卡住的，难道是审美？界面丑到连自己都不想点开，这谁扛得住？
> 我常用的偷懒招数：把看着顺眼的 UI 组件当积木，一股脑丢给 AI Agent，让它自己翻、自己挑、自己塞进项目里。
> 下面这 5 个地方收好，够你使了👇 ……
> 链接直接扔给 Claude Code、Codex 这类 Agent，让它自己扒组件、抠代码、调样式、怼进项目里。
> 你不用懂设计，只要清楚好看的玩意儿上哪儿淘就够了。🎯

五个站与它们的**自称**（全部照抄页面原文）：

| 站点 | 自称（原句） | 规模 | 给 agent 的接口（2026-09-27 实测） |
|---|---|---|---|
| [Beautiful UI](https://www.beautifului.dev) | "Crafted primitives for **AI-native** interfaces"（04 篇记录） | 21 个组件；注册表 **27** 项 | **只有注册表**：`/r/registry.json` **200 / 3,006 B**（`$schema: ui.shadcn.com/schema/registry.json`，27 项）+ `/r/<name>.json` 逐项（如 `/r/task-rows.json` 200 / 11,330 B）；`/llms.txt` **404**、`/mcp` **404**、`/skills` **404** |
| [beUI](https://beui.dev) | "Animated components for React and Next.js" | `registry.json` 实测 **125** 项 | `/llms.txt` **200 / 18,964 B**；`/registry.json` 200；MCP `https://mcp.beui.dev/mcp`（04 篇；本篇实探返回 **406** 的 `{"jsonrpc":"2.0","error":…"Client must accept text/event-stream"}`，**确认服务在线**） |
| [Rare UI](https://rareui.com) | "a free, open-source collection of rare animated React components … install any component with the shadcn CLI" | 首页写 20；仓库 `registry.json` **23** 项 | shadcn CLI 命名空间：`npx shadcn@latest add swamimalode07/rare-ui/<name>`；站点自身接口**未核实**（站点拒绝 curl，见文末） |
| [Transitions](https://transitions.dev) | "Collection of the most essential UI transitions for web apps. Copy and paste them, or use them with your coding agent via the skill." | 目录页写 **43**（Free 32 / Pro 11）；skill 里 32 条 | `npx skills add Jakubantalik/transitions.dev`；`npx transitions-dev add --free`；Refine 工具 `npx transitions-refine live`。`/llms.txt` → **404** |
| [shadcn/ui](https://ui.shadcn.com) | "The Foundation for your Design System" | GitHub **124,674** stars（2026-09-27 API） | `/llms.txt` **200 / 12,260 B**；MCP `npx shadcn@latest mcp init --client codex`（另有 claude / cursor / vscode / opencode）；skill `npx skills add shadcn/ui`；注册表系统 |

**注意表里的差别不是"有 / 没有"，是"agent 要不要猜"**：
Beautiful UI 的注册表**没有** `llms.txt` 去指路，`/r/registry.json` 这个路径来自 shadcn 生态的**约定**
（它自己的 `$schema` 就写着 `https://ui.shadcn.com/schema/registry.json`）——
一个不知道这个约定的 agent，只能去读渲染后的首页再猜有没有接口。
推文把 5 个站并列成"同一类东西"，但**对 agent 而言它们不是同一类**——这一条是本篇最该记住的。

## 1. 技术手法与源码级细节

### 1.1 推文唯一配图：一张被机翻的 shadcn 首页截图

配图内容与 ui.shadcn.com 今日首页 HTML **逐字对得上**（不是"意思接近"，是字符串级命中）：

| 图上的中文 | 站点原文 | 出处 |
|---|---|---|
| `设计系统的基础` | `shadcn/ui - The Foundation for your Design System` | `<title>` |
| `可组合、易于使用的组件，并提供贴心的默认设置。构建您自己的组件库，代码可自定义、扩展并实现您的个性化需求` | `Composable, accessible components with thoughtful defaults. Build your own component library with code you can customize, extend, and make your own.` | `<meta name="description">` |
| 导航 `家 / 文档 / 成分 / 积木 / 图表 / 目录 / 排版 / 创造` | `Home / Docs / Components / Blocks / Charts / Directory / Typeset / Create` | 文档站 header |
| `12.5万` | `125k` | header 的 star 计数（GitHub API 同日 124,674） |
| `开始使用` / `视图组件` | `Get Started` / `View Components` | 首页 hero 两个按钮 |
| `贡献历史`、`设定新的里程碑`、`目标名称`、`15,000`、`Dec 2025`、`1,211.29美元`、`扫描以连接您的移动设备` + `Ledger 移动应用`、`分发轨道` + `Spotify、Apple Music`、`分析 41.82万访客 +10%`、`新聊天`、`按钮/次要/大纲`、`警告对话框`、`按钮组` | `Contribution`、`Goal Name`、`15,000`、`Dec 2025`、`1,211.29`、`Scan to`…`Ledger`、`distribution`、`Spotify`、`Apple Music`、`418.2K Visitors`、`New Chat`、`Button/Secondary/Outline`、`Alert Dialog`、`Button Group` | 首页 HTML |

两个可迁移的观察：

1. **机翻把 `accessible` 译成"易于使用"、把 `Components` 译成"成分"、把 `Typeset` 译成"排版"。**
   引用外文界面时，**中文截图里的词不是作者的词**，也不是站点的词——它是翻译层的词。
   这与 07 篇「命名是作者的，特征是画面的」同一条纪律，只是这次错位发生在**界面文案**上。
2. 这张图**没有展示任何一个组件在动**。它是静态首页，用来当"这站长这样"的封面，
   信息量仅等于一个链接预览。**一张封面图能证明的上限就是这么低**（07 篇已立此规矩）。

### 1.2 transitions.dev 的 token 表：名字里写"谁在用"

`skills/transitions-dev/_root.css:4-38`（仓库原文，2026-09-27 抓取）把整套动效收成一份语义尺度，
**每一条注释都是一个用途清单**，不是参数说明：

```css
  /* Durations */
  --duration-stagger: 40ms;  /* per-item stagger offset */
  --duration-micro: 80ms;  /* tooltip/path delay, shake segment, large stagger */
  --duration-quick: 150ms;  /* modal/dropdown close, text swap, tooltip appear */
  --duration-fast: 250ms;  /* icon swap, dropdown/modal open, tabs sliding, page slide */
  --duration-medium: 350ms;  /* panel close, toast close */
  --duration-slow: 400ms;  /* panel open, skeleton content reveal, input clear */
  --duration-very-slow: 500ms;  /* emphasis moments, badge appear, text reveal, success check */
  /* Easings */
  --ease-smooth-out: cubic-bezier(0.22, 1, 0.36, 1);  /* modal/dropdown/panel open + close, page slide, resize, position change */
  --ease-in-out: ease-in-out;  /* icon swap, text swap, text reveal, skeleton reveal */
  --ease-linear: linear;  /* shimmer, skeleton pulse, spinner */
  --ease-bounce: cubic-bezier(0.34, 1.36, 0.64, 1);  /* badge pop open */
  --ease-bounce-strong: cubic-bezier(0.34, 3.85, 0.64, 1);  /* bouncy hover-out (avatar return) */
```

**"关"比"开"短，是写进 token 名的通则**，不是个别手艺：
`--duration-slow`（panel **open** 400ms）对 `--duration-medium`（panel **close** 350ms）；
badge 的 `--badge-pop-dur 500ms` / `--badge-fade-dur 400ms` 对
`--badge-pop-close-dur 180ms` / `--badge-fade-close-dur 180ms`；
tooltip 是 `--tt-in-dur 150ms` + `--tt-delay 80ms` 对 `--tt-out-dur 50ms`
（三个默认值见 `skills/transitions-dev/17-tooltip.md` 的变量表）。
口径统一成一句：**进场要交代，出场只需收尾。**

### 1.3 同族四个动效的"反直觉"细节（源码/文档原句）

- **Thinking states**（`28-thinking-states.md`）：状态行在**驻留时**走 shimmer，在**切换时**走文字交换，
  出与进**同时**跑——原句 *"Outgoing and incoming lines animate at the same time, so a swap costs one
  `--think-swap`, not two."* 默认 `--think-hold: 2000ms` / `--think-swap: 150ms` / `--think-gap: 50ms`。
  另外它放了一个隐藏的 sizer 装着**最长的那条状态文案**，原句：
  *"lines are absolutely positioned across that width, so every state centres in a box that never resizes mid-swap."*
  → **换文案时盒子宽度不该跟着变**，这条对本仓常驻条尤其重要。
- **Tooltip**（`17-tooltip.md`）：同一个组里**只有一个气泡**，指针移到相邻触发点时它自己 tween x 与 width
  过去（`--tt-move-dur: 160ms`），而不是弹第二个。→ 与 12/13 篇「一个控件的多个状态」同源。
- **Skeleton**（`14-skeleton-reveal.md`）：占位与真内容**共用同一个 flex 槽位**，
  原句 *"the skeleton stays in the same slot as the content so the swap is layout-free"*；
  `--reveal-dur 400ms` / `--reveal-blur 2px`。→ **reveal 不引起布局变化**是它能安静的前提。
- **Notification badge**（`03-notification-badge.md`）：原句 *"Only the badge slides + pops — the trigger
  itself stays put."* → 挂在图标上的计数，动的是数字，不是图标。

### 1.4 SKILL.md 的「Common mistakes to avoid」：这是本批站点的最高密度段落

`skills/transitions-dev/SKILL.md:198-211` 逐条抄（每条都是一个可在别处复现的坑）：

- 不加 `.is-closing` 的清理超时 → 下次打开从"正在关闭的 scale"起跳；
- 文本交换/数字 pop-in/success check 重播/错误抖动**必须先 `void el.offsetWidth` 强制回流**，否则不重播；
- **不要动画外层容器**——徽标要动那个点，page slide 要动两个 page 而不是 container；
- **禁止 `transition: all`**：*"every snippet enumerates exact properties on purpose so unrelated style changes don't ride in for free."*；
- success check 的 `stroke-dasharray` 不能用固定值，要用 `path.getTotalLength()` 向上取整 +1；
- avatar group hover 的 `transition-timing-function` **必须在 JS 里内联写**，否则回弹缓动会跑到 hover 进场上；
- 错误态把 `.is-error` 与 `.is-shaking` **拆成两个正交类**，抖动才能"移除→回流→再加"地重播而不闪整块；
- 手风琴的 padding 必须放在内层（`.t-acc-panel-inner`），放在 `0fr` 的 track 上会**永远留一条高度缝**；
- 手风琴 chevron **不要 morph `d` 路径**（CSS `d:` 插值只有 Chromium 支持，移动端 Safari/Firefox 不动），
  改用 `transform: scaleY(-1)` 垂直翻转——*"it passes through a flat line at the midpoint just like the path morph and works everywhere."*

最后一条与 05 篇 morphicons「先解最优相似变换再插值」形成对照：
**跨渲染引擎的兼容性，是判据的一部分，不是实现细节。**

### 1.5 Rare UI：一次勾选被拆成五个时长，且"搬走"要等动画播完

`components/ui/task-list.tsx`（仓库 `main`，2026-09-27 抓取）第 15-21 行：

```tsx
const FILL: Transition   = { duration: 0.24, ease: EASE_OUT };
const POP: Transition    = { duration: 0.34, ease: EASE_OUT, times: [0, 0.4, 1] };
const TICK: Transition   = { duration: 0.22, ease: EASE_OUT, delay: 0.06 };
const STRIKE: Transition = { duration: 0.38, ease: EASE_IN_OUT };
const NUDGE: Transition  = { duration: 0.3,  ease: EASE_OUT, times: FLICK_TIMES };
const REORDER: Transition = { type: "spring", stiffness: 320, damping: 30 };
const INSTANT: Transition = { duration: 0 };
```

三条设计判断，原文注释（同文件）：

1. `// dashes divide the circumference, so the ring closes without a seam`（环的虚线参数由周长算出）；
2. `// the strike rides on the text itself, so a label that wraps gets a line per row`（删除线跟着文字走，换行时逐行有线）；
3. `// ticking runs tick to strike to nudge, unticking runs the same road backwards`（**取消走同一条路反向回放**）。

最关键的是它**不立刻把完成项搬到列表底部**：勾选后动画先跑完，由 `onSettled` 把它"停"进 `parked`，
下一次渲染才出现在末尾；而如果外部状态又说它不是完成态，`onReverted` 把它收回
（`task-list.tsx:327` 注释：`// a parked row that is no longer done, or gone, was changed from outside`）。
搬运本身是 `layout` + `spring(320, 30)`；同时用 `aria-live` 播报 `"${label} completed"` / `"reopened"`。
**视觉先落地、布局后改变、外部状态可撤回**——三件事被拆成三个机制，没有糊在一起。

### 1.6 Rare UI：循环动画在状态切换时"重定目标"，而不是"重启"

`components/ui/matrix-orb.tsx` 是一个 dot-matrix 球，只有 `idle / listening / thinking` 三态。
它的 `useEffect` 依赖是 `[size, color, dots, dpr]`——**状态不在依赖里**，第 225 行注释：

```
// state stays out of the deps on purpose: the loop retargets, it never restarts
```

状态切换时它做的是**把权重朝新状态混合**，并且**从屏幕上现有的值出发**（第 210-217 行）：

```ts
// per-state weights, so interrupting a change blends from what is on screen
const step = 1 - Math.pow(1 - BLEND, dt * 60)
for (const s of STATES) weights[s] += ((s === current ? 1 : 0) - weights[s]) * step
// 缩放走弹簧，且起落速率不同
const rate = target > amplitude ? ATTACK : RELEASE
```

同文件还有几条与"点阵能不能看成点"有关的细节：
`// no Math.abs here, its corners read as a snap at every trough`、
`// 1.12, not the square's 1.41 corner, is what makes the outline round`、
`// anything under half a device pixel renders as haze, not a dot`、
`// a non-finite level would stick in the smoother forever`。

**这三态恰好是 AgentIsland 的 `working / idle / attention` 在别人家的写法**：
本仓若给微细条做"状态过渡"，该学的是**重定目标 + 从屏幕上现有值混合**，
而不是"切状态就重建一个动画"——后者在常驻 UI 上每天要重播几百次。

### 1.7 Rare UI 的许可不是 MIT

仓库 LICENSE 首行原文：`MIT + Commons Clause License Condition v1.0 + Attribution`，
版权人 `Copyright (c) 2026 Swami Malode`。首页则自称 *"free, open-source"*。
两句话不矛盾，但**不等价**：Commons Clause 限制"把它的功能本身拿去卖"。
GitHub API 的 license 字段返回 `NOASSERTION`（因为不是标准 SPDX 文本）。
→ 引用第三方组件前，"free/open-source" 是营销词，**LICENSE 文件才是事实**。

## 2. 可迁移的决策规则（原话，不是转述）

### 规则一：动效参数按用途命名，且按用途匹配

`SKILL.md:126` 原句：

> `transitions refine` maps each existing value to a usage below, then suggests the token to reference.
> Match on **usage**, not on the raw number — a 300ms modal close still maps to `--duration-quick` (150ms).

以及 `SKILL.md:118`：

> **The key decision point is usage, not the raw number.** … If a value's usage matches **no** token's usage,
> list it as `no matching token usage` and leave it untouched — never force a swap just because a number is close.

> 对我们：这条与共识 11「动效时长按读起来多快定，不按名义多长定」是同一条纪律的**工具化版本**。
> 本仓今天的时长散在 SwiftUI 各处（`withAnimation(.spring(...))` 满天飞），
> 若要收敛，该收的不是"把 0.3 改成 0.25"，而是**先给每个动效写出用途名**，再让相同用途共用一档。

### 规则二："给 agent 的接口"是可以逐项验证的四件套

不用看任何宣传语，四个 URL 就能定一个站的 agent 友好度：
`/llms.txt`（发现索引）、注册表 JSON 或 shadcn 命名空间（可编程取件）、MCP（能写文件）、agent skill（带判据）。
2026-09-27 实测：shadcn **四件都有**（`llms.txt` / 注册表系统 / 官方 MCP 命令 / 官方 skill）、
beUI 占 3 件（索引 / 注册表 / MCP，skill 由第三方 `starc007/ui-components` 提供）、
Transitions 走 skill + 自建 CLI（无注册表索引）、Rare UI 走 shadcn 命名空间、
**Beautiful UI 只占 1 件（注册表索引 + 逐项 JSON），四件里缺的那三件——尤其是指路的 `llms.txt`——正是"丢链接"要靠人脑补的部分**。

> 对我们：本仓反过来是"给 agent 的**写**接口"（`POST /session`），04 篇已记这条不对称。
> 这条规则的价值是把它变成一个**可执行的检查表**：将来若在 README 里写"我们支持 agent 接入"，
> 应先能回答"发现索引在哪、取件接口在哪、是否需要 agent 猜"。

### 规则三：每一次状态切换，先问"它是不是同一个东西在变形"

- tooltip：一个气泡在多个触发点之间**移动**（`17-tooltip.md`）；
- matrix-orb：状态切换是**权重混合**，循环不重启（`matrix-orb.tsx:225`）；
- task-list：行搬家由 spring 接管，但**搬运的时机**等动画落定（`task-list.tsx:356`）；
- transitions.dev 的 `Card resize`：容器改宽高由 tween 接管，而不是换一个组件。

> 对我们：与共识 21「这是几个控件，还是一个控件的几个状态」同一条，样本数再 +4。

### 规则四：状态文案的容器宽度要脱离文案

`28-thinking-states.md` 的 sizer 手法（隐藏一层装着最长文案，可见层绝对定位居中）。
理由原文：*"every state centres in a box that never resizes mid-swap."*

> 对我们：灵动岛收起态宽度是按硬件/内容算死的，**中间那行状态文案一换长度就可能抖**。
> 若确实抖，正确解法不是改文案，而是**给这个位置定宽**（或按最长文案定宽）。

### 规则五：进出不对称，且给出具体档差

token 表本身就是这条规则的证据（open 400 / close 350；badge 500/400 对 180/180；tooltip 150+80 延迟对 50）。

> 对我们：共识 2「不对称才有重量」现在有三批独立样本（Slingshot Lamp 34/16、3dicon 分级、
> transitions.dev 的成对 token），可以当收敛结论用了。

### 规则六：reduced-motion 是每个动效的出厂件，不是选项

目录页原句：*"43 production-ready UI transitions. Every one ships namespaced CSS with motion tokens and a
reduced-motion guard."*；`SKILL.md:193` 把"保留 `prefers-reduced-motion` 块"写成了交付步骤第 4 条，
并给出理由：*"Removing it makes the component fail accessibility audits."*
Rare UI 的 task-list 用另一种写法达成同一目的：所有过渡都过一层 `timing()`，
reduce 时返回 `INSTANT = { duration: 0 }`（`task-list.tsx:83-86`）——
**动画不是被删掉，而是变成零时长**，因此状态机与回调路径完全一致。

> 对我们：共识 5 说本仓缺的不是"要不要尊重"而是"按分层把剩下的逐个判"。
> 这两家示范了两种可复制的做法：**要么每个动效自带 guard，要么所有过渡统一过一层 timing 包装**。

## 3. 一手证据 / 本地验证结果

素材文件：`.scratch/ui-material/16/`（**不入库**，`.scratch/` 已在 `.gitignore`）。
本篇的每个数字都能用文末命令复跑；关键原始件：

| 文件 | 内容 |
|---|---|
| `tweet-fxtwitter.json` | 推文元数据（正文、作者、时间、互动数、媒体清单） |
| `16-tweet-image.jpg` | 唯一配图 1896×949（`pbs.twimg.com/media/HTLnqiTaEAAAxnw.jpg?name=orig`） |
| `shadcn.home.html` / `shadcn.llms.txt` / `shadcn.skills.html` / `shadcn.mcp.html` / `shadcn.github.json` | shadcn 的首页、`llms.txt`、Skills 页、MCP 页、GitHub API |
| `transitions.home.html` / `transitions.catalog.html` / `transitions.skill.html` / `transitions.pro.html` / `transitions.refine.html` | 站点侧原文 |
| `transitions.skill.SKILL.md` / `transitions.skill.root.css` / `transitions.skill.*.md` | 仓库里 skill 的判据、token 表与单条配方（**一手源码**） |
| `transitions.github.json` / `transitions.tree.json` | 仓库元数据与文件树（293 个路径） |
| `rareui.wayback.html` | Rare UI 首页 **2026-09-20 的 Wayback 快照**（站点本体拒绝 curl） |
| `rareui.github.json` / `rareui.repo.registry.json` / `rareui.repo.LICENSE` | 仓库元数据、`registry.json`（23 项）、LICENSE 原文 |
| `rareui.src.{task-list,matrix-orb,animated-counter,step-player}.tsx` | 四条组件的源码（引文行号即出自这些文件） |
| `beui.llms.txt` / `beui.registry.json` / `beui.mcp.html` | beUI 的 agent 接口复核（注册表 125 项） |
| `beautifului.registry.json` / `beautifului.registry.item.json` / `beui.mcp.host.html` | **更正 04 篇的关键件**：Beautiful UI 的 `/r/registry.json`（27 项）与 `/r/task-rows.json`；beUI 的 MCP 端点实探响应 |

实测数字一览（均为 2026-09-27 本机）：

- `curl -o /dev/null -w '%{http_code}'`：`beui.dev/llms.txt` **200**、`ui.shadcn.com/llms.txt` **200**、
  `www.beautifului.dev/llms.txt` **404**、`transitions.dev/llms.txt` **404**、`rareui.com/llms.txt` **402**；
- **Beautiful UI 的注册表（本篇更正 04 篇的那条）**：`/r/registry.json` **200 / 3,006 B**，`$schema` 指向
  `https://ui.shadcn.com/schema/registry.json`，`name: "beautifui"`，**27 项**——
  `foundation / button / glide-menu / entity-chip / value-pill / shimmer / stream-text / loading-state /
  thinking-state / streaming-text / approval-card / tool-chips / task-rows / chat-composer / prompt-bar /
  recommendation-card / context-cards / diff-table / records-table / filter-table / sidebar-nav / search /
  flowchart / insight-cards / code-block / fine-tune-card / selection-actions`；
  逐项 JSON `/r/<name>.json` 同样可取（`/r/task-rows.json` **200 / 11,330 B**）。
  而 `/registry.json`、`/mcp`、`/docs/mcp`、`/docs/ai-agents`、`/skills`、`/llms-full.txt` 全部 **404**；
- beUI 的 MCP 端点实探：`https://mcp.beui.dev/mcp` → **406**，正文
  `{"jsonrpc":"2.0","error":{"code":-32000,"message":"Not Acceptable: Client must accept text/event-stream"},"id":null}`
  ——**这是服务真在线的证据（它要求 SSE），比 04 篇引它自己的文档更硬**；
- GitHub API：`shadcn-ui/ui` **124,674** stars / MIT / pushed 2026-09-24；
  `Jakubantalik/transitions.dev` **4,379** stars / **license: None** / pushed 2026-09-21；
  `swamimalode07/rare-ui` **1,491** stars / `NOASSERTION` / pushed 2026-09-26；
- 目录页原句：Transitions "**43** production-ready UI transitions"（Free 32 / Pro 11），而 skill 里是 **32** 条——两个口径，未对账；
- Rare UI：首页（快照）写 "**20** … components"，仓库 `registry.json` 是 **23** 项（含 `utils`）——两个口径，未对账。

**本篇对 04 篇的更正（时间点证据，不改 04 原文）**：04 篇写
*"Beautiful UI 没有 `llms.txt` … 也没有暴露 registry JSON。它的 agent 友好度低于 beUI 与 BoardUI"*。
前半句今天仍成立（`/llms.txt` 404），**后半句被证伪**：它暴露了 shadcn 格式的完整注册表索引与逐项 JSON。
更正命令：`curl -sS https://www.beautifului.dev/r/registry.json`（200 / 3,006 B）。
04 篇抓取于 2026-09-26、本篇 2026-09-27，**中间只隔一天，所以更可能是 04 篇当时没试这个路径**，
而不是站点在这一天里新增了它——**两个可能都无法从今天回证**，此处只记"今天的实测是什么"。

**环境事实（和 07 篇的旧账不同，这次要分情况说）**：
`pbs.twimg.com` 那张配图**直连即得**（146,552 B，无需代理）；
`web.archive.org` 直连 60s 超时，**走代理 `127.0.0.1:10808` 才拿到**；
`rareui.com` 对 curl 返回 **403（直连）/ 429（走代理）**，页面标题是 `Vercel Security Checkpoint`，
firecrawl 与 modsearch 的 local 引擎都拿到 **402 Payment required**——
**这不是"被墙"，是本机没有浏览器指纹就过不去**，所以本篇没有把任何站点级事实写成"今天就是这样"。

## 4. 应用建议清单（与实现解耦，尚未排期）

1. **状态文案定宽**（来自 `28-thinking-states.md` 的 sizer）：若微细条/侧边栏的状态行在换文案时宽度抖，
   按**最长文案**定宽，而不是缩短文案。**现状未核实**——先量一次宽度再说。
2. **状态过渡学 matrix-orb 的"重定目标"**：若给状态色/呼吸灯做过渡，写成对目标值的逐帧混合，
   不要在状态切换时取消并重建动画（常驻 UI 上重播成本最高）。**现状未核实**。
3. **一次动效拆成多个时长要看它有几个可读阶段**（task-list 的做法：填充/弹出/打勾/划线/轻推/重排各一条）。
   本仓的 `completed` 态切换天然有"停下 → 变色 → 归位"三段，值得先清点**当前是不是只挂了一条曲线**。
4. **完成的项不要立刻搬家**：若面板里会为了"已完成"把条目移到底部，让它在动画落定后再移
   （`onSettled` 模式），并保留"外部状态又说没完成就撤回"的路径（`onReverted`）。
5. **徽标只动数字不动图标**（`03-notification-badge.md`）：本仓若有计数徽标，动的应是计数，不是底座。
6. **时长 token 化先写用途名**：把散在 SwiftUI 里的时长收敛时，先写"这个时长在交代什么"，
   再决定分几档；direct 参照 transitions.dev 的 7 档（40/80/150/250/350/400/500ms）——
   **不要照抄数字，照抄"用途→档位"的映射方式**。
7. **duration 让位给 reduced-motion 的方式选一种**：要么每个动效自带 guard，
   要么所有过渡统一过一层 `timing()` 包装并在 reduce 时返回零时长（Rare UI 的做法）。
   两种都比"在某处删掉动画"安全，因为后者会改变状态机的回调时序。
8. **数字滚动有了第二个可实现样本**：`animated-counter.tsx` 用预渲染的 21 个 span + 尾随 0 保证
   `9 → 0` 落在同一张面上（`// the trailing 0 makes the wrap from 9 back to 0 land on an identical face`），
   并用 `// eased rather than a straight ramp; a linear fade of the same width reads as a hard edge` 解释
   遮罩必须缓动。→ 与共识 22「数字会变时让它数上去」合起来，本仓 token 计数若做滚动，实现细节有处可抄。
9. **别把"淘组件"当主线**：5 个站里 4 个是 React 组件库（beUI / Beautiful UI / Rare UI / shadcn），
   1 个是纯 CSS 配方（Transitions），**没有一个能搬进 SwiftUI**；
   可搬的是 token 表、判据清单与"反直觉细节"。这条也是给"看到推文就想装库"的刹车。
10. **注册表要能被发现，才算真的给了接口**：Beautiful UI 已有 `/r/registry.json`，
    但既没有 `llms.txt` 指路、也没有在自己的文档里写这个路径——**它的可发现性等于"读者是否知道 shadcn 的约定"**。
    本仓若将来暴露任何给 agent 的读接口，应同时给一个 `llms.txt` 式的索引，
    而不是只把 JSON 放在一个聪明人猜得到的路径上。

## 与已收录案例的关系

| 篇 | 关系 |
|---|---|
| [03 libraries-dev](03-libraries-dev-where-effects-belong.md) | **同一作者**：transitions.dev 页脚原文 `Created by Jakub Antalik`，与 03 篇的 `Jakubantalik/Libraries.dev` 同一人。03 讲"动效该加在哪"（决策规则），本篇讲"动效的参数怎么命名与匹配"（token 化）——同一思路的下一层 |
| [04 ui-resource-sites](04-ui-resource-sites.md) | **同一批站点的补集 + 一处更正**：04 覆盖 beUI / Beautiful UI（+BoardUI/ThreeUI/Inspora/CollectUI），本篇补 Rare UI / Transitions / shadcn/ui；`/llms.txt` 状态与 04 一致（beUI 200、Beautiful UI 404），但 **04 篇"Beautiful UI 没有暴露 registry JSON"被本篇证伪**（`/r/registry.json` 200 / 27 项，见第 3 节） |
| [12 halogen](12-halogen-recorder-capsule-states.md) / [13 morphing dropdown](13-kopp-morphing-dropdown.md) | 同向：tooltip 的一个气泡在多个触发点间移动 = "一个控件多个状态"的第三个样本 |
| [14 progressive payment](14-mide-progressive-payment-reveal.md) | 共识 22「数字数上去」的**可实现版本**：`animated-counter.tsx` 的 odometer 轮盘 |
| [06 liquid taffy](06-liquid-taffy-goo-engine.md) / [11 plasma-ui](11-plasma-ui-liquid-glass-panels.md) | 「同一参数的多个消费者共用一份来源」→ token 表是这条纪律的工程化形态：**参数只写一次，名字说明谁在用** |
| [05 morphicons](05-morphicons-and-tools.md) | 对照：05 是"把形变解成数学"，本篇 SKILL.md 是"**别用只有 Chromium 支持的 `d:` 插值**，改 `scaleY(-1)`"——一个讲最优解，一个讲可用解 |
| [07 ui-motion-tweet-sample](07-ui-motion-tweet-sample.md) | 同一条门禁的两个新样本：本篇配图是**机翻截图**（词不是作者的），Rare UI 是**自称 free/open-source 但 LICENSE 是 MIT+Commons Clause**（词不是事实） |

## 没能核实的（不写成事实）

1. **rareui.com 站点本体完全没拿到。** 直连 403 / 代理 429 / firecrawl 402 / local 402，
   标题为 `Vercel Security Checkpoint`；本机没有浏览器自动化，所以本篇关于 Rare UI 的**页面级**
   事实一律标注为来自 **2026-09-20 的 Wayback 快照**，组件事实来自 **GitHub 仓库**（pushed 2026-09-26）。
   **它今天的 `/llms.txt` 是否存在、是否有 MCP，本篇不能说"没有"**——只能说 curl 过不去。
2. **推文配图的卡片内文字是"读图"而非"OCR 逐字"。** 表格里的映射项都是"图上看到的词 → 站点 HTML 里的字符串"，
   命中是字符串级的；但图上是否还有我漏读的文案**没有逐像素核对**。
3. **两个规模口径未对账**：Transitions 目录页 **43**（Free 32 / Pro 11）对 skill 里 **32** 条；
   Rare UI 首页 **20** 对仓库 registry **23** 项。差异原因（是否含 Pro、是否含 utils/hook）未查。
4. **Transitions 的 Pro 那 11 条没拿到**（Pro 内容要登录），只有价格页文字：Solo `$9/mo`、
   "36+ production-ready transitions"、"Commercial license, unlimited projects"；Annual / Lifetime / Team 的价格未读。
5. **transitions.dev 的仓库 license 是 None**（GitHub API 返回 `null`，仓库根目录文件树里也没有 LICENSE）。
   所以"免费 32 条能否商用"**未核实**——页脚联系方式经 Cloudflare 邮件混淆，**按本仓脱敏红线未解码、未转载**。
6. **`npx skills add …` 没有真的跑过。** 只核对了站点与仓库里的命令原文；
   "装完 agent 就能正确用"这件事本机未验证（也没有可用的目标项目）。
7. **Rare UI 的中文推荐语是它自己首页的 testimonial**（`@LgyLight`）：
   `每一个组件都很精致且独特，用 shadcn CLI 一行命令就能装 … 比让 AI 发挥稳定多了`。
   这是**营销位上的引用**，本篇只把它当"有人这么说过"，不当任何结论的证据。
8. **transitions.dev 的 `AI Agents` 分类页签成员未逐个核实**：页签确实存在（`All / Essential / AI Agents / Effects / Texts / Pro`），
   但成员由客户端渲染过滤；从名字看至少 `Thinking states` / `Reasoning stream` / `Streaming text` / `Matrix dot loader`
   四条属于它——**这是推测，不是核实**。
9. **推文作者与这 5 个站是否有利益关系未核实**：作者 bio 是"加密web3、日常分享"，
   正文没有披露赞助/返佣，本篇也不替它判定；**当资源清单读，不当背书读**。
10. **Rare UI 的 4 条组件源码只读了 4 个文件**（task-list / matrix-orb / animated-counter / step-player），
    其余 19 项未读；本篇引用它们只用于说明手法，不代表 Rare UI 的普遍水准。
11. **"04 篇当时没试这个路径"是推测，不是核实。** 04 篇（2026-09-26）与本篇（2026-09-27）只隔一天，
    所以"04 漏了"比"站点当天新增"更可能；但**两种可能今天都无法回证**，本篇只记今天的实测结果。
12. **Beautiful UI 的注册表"能不能装"没验证过。** 只确认了索引与逐项 JSON 可取、`$schema` 是 shadcn 格式，
    没有真的跑过 `npx shadcn@latest add https://www.beautifului.dev/r/…`；它是否登记在 shadcn 的 Directory 里也未查。
13. **五个站的接口清单是当天的快照。** `/llms.txt`、`/mcp` 这类路径加上或撤掉都只改一行，本篇的数字**只对 2026-09-27 有效**，
    这也是第 2 节把"逐 URL 核实"写成检查表而不是结论的原因。

## 取证命令

```sh
# ── 0. 推文元数据与配图（pbs 直连即可）────────────────────
curl -sS 'https://api.fxtwitter.com/x5cnhp/status/2103998703908643272' -o tweet-fxtwitter.json
python3 - <<'PY'
import json; d=json.load(open('tweet-fxtwitter.json'))['tweet']
print(d['created_at'], d['author']['screen_name'], d['likes'], d['bookmarks'], d['views'])
print([ (m['type'], m['url'], m['width'], m['height']) for m in d['media']['all'] ])
PY
curl -sS -o 16-tweet-image.jpg 'https://pbs.twimg.com/media/HTLnqiTaEAAAxnw.jpg?name=orig'

# ── 1. 配图身份：图上中文逐条对回 ui.shadcn.com 首页 ────────
curl -sSL -A "$UA" https://ui.shadcn.com -o shadcn.home.html
grep -oE '<title>[^<]*|<meta name="description" content="[^"]*' shadcn.home.html
for s in '1,211.29' '418.2K Visitors' 'Contribution' 'Goal Name' '15,000' 'Dec 2025' \
         'New Chat' 'Scan to' 'Ledger' 'Spotify' 'Apple Music' 'distribution'; do
  printf '%-18s %s\n' "$s" "$(grep -o -i -m1 -- "$s" shadcn.home.html)"
done

# ── 2. 五个站的 agent 接口逐项探测（本篇第 2 节规则二的检查表）──
for u in https://www.beautifului.dev/llms.txt https://beui.dev/llms.txt \
         https://rareui.com/llms.txt https://transitions.dev/llms.txt \
         https://ui.shadcn.com/llms.txt; do
  printf '%-42s %s\n' "$u" "$(curl -sS -A "$UA" -o /tmp/p.out --max-time 20 -w '%{http_code} %{size_download}' "$u")"
done
curl -sS -A "$UA" https://beui.dev/registry.json \
  | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["items"]))'          # 125

# ── 2b. 更正 04 篇：Beautiful UI 其实有 shadcn 注册表 ─────────
curl -sS -A "$UA" https://www.beautifului.dev/r/registry.json -o beautifului.registry.json   # 200 / 3006B
python3 - <<'PY'
import json; d=json.load(open('beautifului.registry.json'))
print(d['$schema'], len(d['items'])); print([i['name'] for i in d['items']])
PY
curl -sS -A "$UA" https://www.beautifului.dev/r/task-rows.json -o beautifului.registry.item.json  # 200 / 11330B
for p in /llms.txt /mcp /docs/mcp /docs/ai-agents /skills /llms-full.txt /registry.json; do
  printf '%-18s %s\n' "$p" "$(curl -sS -A "$UA" -o /dev/null --max-time 15 -w '%{http_code}' "https://www.beautifului.dev$p")"
done      # 全 404

# ── 2c. beUI 的 MCP 端点：406 就是"服务在线"的证据 ───────────
curl -sS -A "$UA" https://mcp.beui.dev/mcp | head -c 200   # jsonrpc error: Client must accept text/event-stream

# ── 3. shadcn 的 MCP 与 skill（agent 接口的两个形态）────────
curl -sSL -A "$UA" https://ui.shadcn.com/docs/mcp    -o shadcn.mcp.html
curl -sSL -A "$UA" https://ui.shadcn.com/docs/skills -o shadcn.skills.html
grep -oE 'npx shadcn@latest mcp init --client [a-z]+' shadcn.mcp.html | sort -u
grep -oE 'npx skills add [a-z/]+' shadcn.skills.html | sort -u
curl -sS -A "$UA" https://api.github.com/repos/shadcn-ui/ui \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);print(d["stargazers_count"],(d.get("license") or {}).get("spdx_id"))'

# ── 4. Transitions：token 表与判据都在公开仓库里 ────────────
base=https://raw.githubusercontent.com/Jakubantalik/transitions.dev/main
curl -sSL -A "$UA" "$base/skills/transitions-dev/_root.css"  -o transitions.skill.root.css
curl -sSL -A "$UA" "$base/skills/transitions-dev/SKILL.md"   -o transitions.skill.SKILL.md
curl -sSL -A "$UA" "$base/skills/transitions-dev/17-tooltip.md" -o transitions.skill.17-tooltip.md
sed -n '9,38p' transitions.skill.root.css        # 7 档时长 + 6 条缓动的用途注释
sed -n '198,211p' transitions.skill.SKILL.md     # Common mistakes to avoid
curl -sSL -A "$UA" https://transitions.dev/transitions/ | grep -oE '[0-9]+ production-ready UI transitions'   # 43
curl -sS  -A "$UA" https://api.github.com/repos/Jakubantalik/transitions.dev \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);print(d["stargazers_count"],(d.get("license") or {}).get("spdx_id"))'  # 4379 None

# ── 5. Rare UI：站点过不去，走仓库 + 快照（并写明这条限制）──
curl -sS -A "$UA" -o rareui.home.html -w '%{http_code}\n'   https://rareui.com   # 403 Vercel Security Checkpoint
curl -sSL -x http://127.0.0.1:10808 --compressed \
  'https://web.archive.org/web/20260920151943id_/https://www.rareui.com/' -o rareui.wayback.html
curl -sS -A "$UA" https://api.github.com/repos/swamimalode07/rare-ui -o rareui.github.json
curl -sSL -A "$UA" "$base2/registry.json" -o rareui.repo.registry.json   # base2=.../swamimalode07/rare-ui/main
python3 -c 'import json;print(len(json.load(open("rareui.repo.registry.json"))["items"]))'   # 23
head -1 rareui.repo.LICENSE        # MIT + Commons Clause License Condition v1.0 + Attribution
for n in task-list matrix-orb animated-counter step-player; do
  curl -sSL -A "$UA" "https://raw.githubusercontent.com/swamimalode07/rare-ui/main/components/ui/$n.tsx" -o "rareui.src.$n.tsx"
done
grep -nE 'const (FILL|POP|TICK|STRIKE|NUDGE|REORDER|INSTANT)' rareui.src.task-list.tsx   # 15-21
grep -n 'retargets, it never restarts' rareui.src.matrix-orb.tsx                          # 225
```

> 环境事实：本篇配图（`pbs.twimg.com`）**直连即得**；`web.archive.org` 直连 60s 超时、
> **必须走代理 `127.0.0.1:10808`**（`scutil --proxy` 可见）；`rareui.com` 对 curl **直连 403 / 代理 429**，
> 无浏览器指纹过不去 Vercel Security Checkpoint——**这三条不能互相套用**，07 篇的旧账（"直连不通就怀疑没走代理"）
> 在本篇只对 Wayback 成立。
