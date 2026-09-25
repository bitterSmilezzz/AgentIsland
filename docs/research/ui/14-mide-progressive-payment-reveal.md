# Progressive Payment Reveal：进度数字是数出来的，不是跳出来的

> 目的：在给灵动岛做任何"进度/用量"显示之前，看一个把 **progressive reveal** 用在支付进度上的样本。
> Mídé（[@mide_ajibade](https://x.com/mide_ajibade)，8454 followers，"Building products people use
> and pay for"）2026-09-25 发的 "🛳️ —• Progressive Payment Reveal Interaction."，
> 238 赞 / 4629 浏览 / 113 收藏。视频 26.97 秒、1080×1080@30fps（原始 2160×2160 方形），
> **已完整下载并逐帧分析**。分析时间 2026-09-26。

## 一句话结论

**它的"渐进"不在视觉炫技，而在数字本身：`0% paid` 用约半秒数到 `47%` 再落到 `50%`**
（d1.5 → d2 → d2.5 三帧 OCR 逐字确认），进度条同步从 0 长到一半。
同一个数字如果用 CSS transition 只会"跳一下"，而它是**可读地数上去的**——
用户在半秒里看清了"正在从 0 变成 50"，而不是看到一个突然的 50。
这是本案例库里第二个"数字本身就是动效"的样本（第一个是 [04 篇](04-ui-resource-sites.md)
记录的三家站点一致做 elapsed time）。

## 它是什么

一个**分期付款管理界面**，浅色主题、垂直居中卡片布局，三个分段控件 tab：

| tab | 内容 |
|---|---|
| **Installment** | `Full Payment - UG` / `Plan 1 • 2 installments • ₱ 6,000 total`；`0% paid` + 进度条；两行分期明细；`Autopay from wallet` 开关 |
| **Progression** | `Curriculum Progression`；`CGPA: 1.22`；四色图例 `Earned Credits 3.05`（绿）/ `Remaining Credits 3.05`（红）/ `Overall Credits 12.22`（蓝）/ `Reg. Credits 9.16`（紫）+ 网格状进度图 |
| **Courses** | 可展开课程卡：`CS101` / `Chemistry for Engineers`，展开后六个字段（`Course Type: Lecture and Laboratory`、`Units: 5`、`Section: CPMTMLS-SEC A`、`Semester: Y1S1`、`Teacher: Semilla`、`Approval Status: Present`） |

分期明细的两行（OCR 逐字确认）：

- `Installment 1` / `₱ 3,000` / `Paid 2 Feb 2026 • GCash`（GCash 是菲律宾电子钱包，`₱` 是比索）
- `Installment 2` / `₱ 3,000` / `Due 26 Feb 2027 • in 164 days`

顶部有**明暗主题切换器**（太阳/月亮图标胶囊），视频里确实演示了从亮到暗的切换（见一手证据）。

## 一手证据：逐帧时序

`ffmpeg select='gt(scene,0.02)'` 对全片**零命中**——27 秒是一条连续操作录像，不是拼接。
抽 21 帧，内容由 OCR + 语义读图逐字读出。

### 核心时序：进度数字是数出来的

| 帧 | 时刻 | OCR 读到的进度 |
|---|---|---|
| d1.5 | 1.5s | `0% paid` |
| d2 | 2.0s | **`47% paid`** |
| d2.5 | 2.5s | `50% paid` |
| d3 / d3.5 / d4 / d4.5 / d5 | 3–5s | `50% paid`（稳定） |

半秒内 `0 → 47 → 50`，中间值是**可读的**——不是 0.3s 的 CSS opacity 淡切能解释的
（那会只有起点与终点两态）。说明数字是逐帧或分步滚上去的。

进度条形态（d2 帧语义读图确认）：**一条**水平胶囊进度条，蓝色填充在**左**、浅灰未填充在右，
填充比例与数字一致（47% / 50%）。

### 三个 tab 依次演示

- **Installment**：1–5s，进度从 0 数到 50
- **Courses**：8s 与 12s —— f8 帧课程卡**折叠**（只 `CS101` / `Chemistry for Engineers` 两行 + 右侧展开箭头），
  f12 帧**展开**出六个字段（箭头变为向上）。f9/f13/f17 的 OCR 为空，是因为抽在滚动/展开过渡中，
  由语义读图补齐（语义读图确认了展开前后字段数：2 → 6）
- **Progression**：f6 帧 —— `Curriculum Progression` + `CGPA: 1.22` + 四色图例 + 网格状进度图，
  图上有蓝色与紫色高亮块（对应 Overall Credits 与 Reg. Credits）

### 主题切换被真实演示

x14 / x15 两帧语义读图确认**月亮图标被激活、界面为暗色主题**——也就是视频中途真的切了明暗，
不是只放了一个开关。这比"有个开关但没切"的demo 更可信。

## 可迁移的决策规则

1. **数字会变时，让它数上去，不要跳。** `0 → 47 → 50` 与 `0 → 50` 给用户的信息量不同：
   前者说明"正在变化"，后者说明"已经变了"。判据很简单：**这个变化用户需要知道过程吗？**
   token 消耗、耗时、进度——都需要。
2. **进度条与数字必须同源。** 这一条在本片成立：填充比例与数字在 d2 / d5.5 帧一致。
   两者不同步是这类控件最常见的 bug（数字 50% 而条才 30%），而且**截图看不出来**，
   只在动的时候暴露。
3. **轻内容折叠、重内容展开，但别用两套组件。** Courses 卡的折叠态两行、展开态六个字段，
   同一张卡同一个箭头。与 [13 篇](13-kopp-morphing-dropdown.md)「一个面板的几个状态」同向。
4. **分段控件承载的是"同类数据的三种切法"，不是三个页面。** 这里三个 tab 全是同一个学生的
   付款/学业进度，不是三个功能模块。判据：切 tab 时**外壳（卡片、标题栏、主题）不动**。
5. **图例用色点 + 文字，不用纯色块。** Progression 的四项 `Earned / Remaining / Overall / Reg.`
   各配一个色点 + 名称 + 数值——**颜色在这里不是唯一载体**，色盲用户读文字照样懂。
6. **相对时间要说"还有多久"。** `Due 26 Feb 2027 • in 164 days` 把绝对日期与相对天数并排。
   绝对日期用于存档，相对天数用于决策——两个都要，别只给一个。

## 应用建议清单（与实现解耦，尚未排期）

1. **Token 用量与耗时改成"数上去"**：这是本篇对本仓最直接的一条。分析页的 24h / 累计 token、
   月末预测若现在是直接赋值，可评估改成分步滚动。**注意必须先确认它的取值频率**——
   如果是 1 秒一次的轮询，每秒都数一次反而比跳变更吵；应当**只在值真正变化时数**。
2. **进度条与数字同源要有一条断言。** 若做任何"百分比 + 条"的组合控件，让条的长度由同一个值算出，
   不要两份状态。这与 [11 篇](11-plasma-ui-liquid-glass-panels.md)「同一光学参数的两个消费者
   共用一份来源」是同一条纪律在数据层的版本。
3. **"in 164 days" 这种相对时间可借鉴到 `attention` 态**：等待确认的请求挂了多久，
   相对时长（"已等 4 分钟"）比绝对时间戳更有行动指导性。**未核实现状措辞，做前先读代码。**
4. **`offline` 与"未接入明细源"的呈现参考四色图例法**：色点 + 名称 + 数值，
   让颜色不当唯一载体。与本仓已知限制里"三种不同结论"的表达需求吻合。
5. **分段控件外壳不动**：若做 sidebar 主形态的 tab 切换，先确认外壳（含玻璃背景）不重建——
   重建会让玻璃/发光每切一次重新长出来。**现状未核实。**

## 与已收录案例的关系

| 篇 | 关系 |
|---|---|
| [04 ui-resource-sites](04-ui-resource-sites.md) | **同一条规则的两半**：那篇三家站点一致做 elapsed time（"等了多久"），本篇做"进度数上去"（"变成多少"）。合起来：**给反馈 + 让过程可读** |
| [13 morphing dropdown](13-kopp-morphing-dropdown.md) | 同向：tab 外壳不动、内容层切换；Courses 卡折叠/展开也是一个控件的两种形态（[12](12-halogen-recorder-capsule-states.md) 的第三种样本） |
| [11 plasma-ui](11-plasma-ui-liquid-glass-panels.md) | 同一条纪律的数据层版本：计数可读 + 同源 |
| [05 morphicons](05-morphicons-and-tools.md) | 无关（图形插值数学） |

## 没能核实的

- **源码没拿到**：这是作者自己做的设计稿/demo，**未找到公开仓库或站点**（推文只给了视频）。
  所有实现层面的结论都是从画面推断，没有代码可核
- **数字滚动的具体曲线与步长**：只从 0.5s 间隔的帧读到 `0 / 47 / 50` 三个值，
  **没有逐帧追**它是线性滚、分步跳还是 spring 到 50；中间是否有 20% / 35% 等值未验证
- **进度条是否与数字严格同步**：d2 与 d5.5 两帧看着一致，但**没有做像素测量**比对填充比例与数字
- **`0% → 50%` 是用户操作触发的还是自动演示**：视频里看不到触发那一刻的输入（无光标点击证据），
  推测是自动播放的 demo 序列，未确认
- **Courses 卡展开/折叠的时长与曲线**：f8 折叠、f12 展开，中间 4 秒差里没有逐帧，曲线未验证
- **暗色主题的完整配色**：x14/x15 确认切到了暗色且月亮激活，但**没有逐字段读暗色下的颜色值**
- **`CGPA: 1.22` 与学分四个数字的关系**：图上是网格状进度图，但它如何编码这四个值，未读出来
- 界面语言是英文，货币是菲律宾比索（`₱`）、支付渠道是 GCash——这是菲律宾学生场景，
  **数字本身没有可迁移性**，可迁移的是呈现方式

## 取证命令

```sh
# 推文元数据（fxtwitter，无需代理）
curl -sS 'https://api.fxtwitter.com/mide_ajibade/status/2103405709539057899'

# 视频（走系统代理）
scutil --proxy
curl -x http://127.0.0.1:10808 -o clip.mp4 \
  'https://video.twimg.com/amplify_video/2103405614198145024/vid/avc1/1080x1080/LAVk9Qi7hngi7GkJ.mp4?tag=29'

# 元数据（1080x1080 / 30fps / 26.967s；原始 2160x2160）
ffprobe -v error -show_entries format=duration:stream=width,height,r_frame_rate -of default=nw=1 clip.mp4

# 确认无硬切（一条连续录像）
ffmpeg -i clip.mp4 -vf "select='gt(scene,0.02)'" -vsync vfr -f null - 2>&1 | grep pts_time

# 关键时序帧（本文 0→47→50 即由这三帧 OCR 得到）
for t in 1.5 2 2.5 3 3.5 4.5 5 5.5 6; do
  ffmpeg -ss $t -i clip.mp4 -frames:v 1 -q:v 4 -y "d$t.jpg"
  dim ocr recognize "d$t.jpg" --json
done

# 三个 tab（语义读图补 OCR 读不到的过渡帧）
for t in 1 4 8 10 12 14 16 18 20 22 24 26; do
  ffmpeg -ss $t -i clip.mp4 -frames:v 1 -q:v 4 -y "f$t.jpg"
  dim image read "f$t.jpg" --json --prompt '这是哪个 tab？列出所有文字字段与值'
done
```

> 环境事实：本机 `*.twimg.com` / `pbs.twimg.com` 直连超时，必须走系统代理
> `127.0.0.1:10808`（`scutil --proxy` 可见）。视频 mp4 走该代理可完整下载。
