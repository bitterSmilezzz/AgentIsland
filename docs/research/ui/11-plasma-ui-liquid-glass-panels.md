# Plasma UI：液态玻璃面板，以及一次"计数器可读"的熔断-重连实录

> 目的：在考虑给灵动岛加"液态玻璃"质感之前，把一个把这件事做成产品的库摸到底。
> [CruxGarden/plasma-ui](https://github.com/CruxGarden/plasma-ui)（npm `@cruxgarden/plasma-ui`，
> MIT，v0.3.0，2026-09-17 建库），作者从 Crux Garden 项目里抽出来的。
> 素材：36.2 秒 demo 视频（1652×1080，60fps，2173 帧）**已完整下载并逐帧分析**——
> 这是本案例库第一次拿到视频而不是封面，所以本篇的一手证据比 07 篇厚一档。
> 分析时间 2026-09-26；上游 README 全文 21.8KB 已落盘；docs 站点 264KB 已抓取。

## 一句话结论

**它是本案例库里唯一一个"液态玻璃"的完整工业实现，而它的 playground 把融合状态做成了可读计数器
（`5 panels, 4 joined`）——于是我能用 0.2 秒间隔的帧抓到一次完整的熔断-重连**
（`4 joined → 0 joined`，发生在 16.8s 与 17.0s 之间，< 200ms）。
同一条证据链还显示 **blend distance 从 40px 调到 56px 时，joined 上限从 3 升到 4**。
但必须如实说：**README 宣称的"像 Slinky 一样起伏的拖尾"，这 36.2 秒里没有任何一帧能证明。**

## 它是什么

上游 README 的原话（可引用）：

> Liquid panels for React, rendered in WebGL on canvas, inspired by Apple's Liquid Glass design.
> The `<Plasma>` panel looks and behaves like liquid, with surface tension that fuses on contact
> with other panels. Anything visible behind the panel is refracted. And for layout convenience,
> the panels ultimately snap to a grid layout.

`npm install @cruxgarden/plasma-ui`，**零依赖（除 React）**。一个 provider + 一个组件：

```tsx
<PlasmaProvider mood="tidal">
  <Plasma as="header" lean={false}>My App</Plasma>
  <Plasma draggable><h3>Inbox</h3></Plasma>
</PlasmaProvider>
```

上游自己划的适用范围（这条比参数更重要）：**容器类组件**——panels、docks、cards、dialogs，
业务组件嵌在里面。并且明确说**桌面优先**："the effect is GPU-heavy and does not run well on mobile"，
每个 pass 都是全视口的，成本随画布尺寸走。GitHub topics 自报：
`animation / glassmorphism / liquid-glass / metaballs / shaders / webgl`。

## 一手证据：视频三段结构（scene-change 独立测出）

用 `ffmpeg select='gt(scene,0.02)'` 扫全片找到两个硬切点，与逐帧 OCR 的页面内容变化一致：

| 时间 | 内容 | 判定依据 |
|---|---|---|
| 0–21.6s | **Playground 调参**（21.6s，占 60%） | tab URL 为 `cruxgarden.github.io/plasma-ui/` |
| ~22.75–22.79s | → 官网 landing | scene score 命中 + 内容突变 |
| 21.7–27.6s | **官网 landing** | 标题 "Plasma UI" + "Open the playground" / "Send a pulse" 两枚胶囊按钮 |
| ~27.65–27.82s | → GitHub 仓库页 | scene score 连续 4 帧命中 + URL 突变 |
| 27.7–36.2s | **GitHub 仓库页**（占全片 23%） | 标准浅色 GitHub UI，**无任何 plasma 动效** |

**第三段对本项目参考价值极低**——它渲染的是 GitHub 自己的 UI。但它提供了一个意外佐证（见下"提交信息"）。

Playground 的说明文案（f7 帧 OCR 读出，可直接引用）：

> Playground — Drag panels; they fuse on contact and snap to the grid on release.
> Throw one to stretch the plasma. Click empty space to send a pulse.
> The controls change the whole page.

## 最硬的一条证据：熔断-重连，计数器可读

Playground 界面把融合状态显示成 `N panels, M joined`。用 0.2 秒间隔抽 16.0–18.2s：

| 帧 | 时间 | 读数 |
|---|---|---|
| t16.5 | 16.5s | **4 joined** |
| f16 | 16.0s | 3 joined |
| d16.6 / d16.8 | 16.6 / 16.8s | 2 joined |
| **d17.0 / d17.2 / d17.4 / d17.6** | 17.0–17.6s | **0 joined** |
| d17.8 / d18.0 / d18.2 | 17.8s 起 | 2 joined |
| t18.5 | 18.5s | 2 joined |

**熔断在 16.8s 与 17.0s 之间完成（< 200ms，真实速度可能更快），重连在 17.6s 与 17.8s 之间。**

同一段里 blend distance 也在变：40px（f10/s12/s14/s15）→ 56px（f16/f19/t16.5/s17/t18.5），
而 joined 上限随之从 3 升到 4。**参数与结果共变可读**——不过要诚实：这是时间上的共变，
没有同帧证明"调 blend 的那一刹那 joined 数跳了"，因果是推断、共变是事实。

### 融合有两种形态，不能互换引用

| 形态 | 出现帧 | 特征 |
|---|---|---|
| **细颈（metaball neck）** | t16.5（4 joined）、f7、d16.6/d16.8 | 颈宽约为面板宽的 1/5–1/4（40–60px vs 面板 180–220px），中点收细、S 形曲线 |
| **宽桥/团块** | d17.8、f19 | Files 旋转后右上角嵌进 Notes 左下角，大块融合区 |
| **完全分离** | d17.0–d17.6、f10、f4 | 界面计数器 `0 joined`，视觉确认有空隙 |

**一个值得单独记的细节**：即使在 4 joined 的 t16.5，每个面板的轮廓在融合点仍能被单独描出，
"三颗珠子串在绳上"，**没有溶解成一块无形团**。这与它用 SDF + smooth minimum 的画法方向一致。

### 意外佐证：上游自己的提交信息

GitHub 页那段（虽然对动效参考价值低）里有一条提交：

> `Show a longer demo: panels tearing away and fusi…`

从字面就能佐证 d 系列那个 0 joined 实验是**库自己做的演示**（tearing away = 拉开熔断，
fusing = 重新熔接）。另三条提交正好对应 playground 顶部那六个预置 tab：
`Crystal, modelled` / `Six materials on one engine` / `Mercury is marched as a solid`
（PrettierIgnore 那行也是提交信息：`Document shimmer speed in the playground API`）。

## 技术手法：它用了什么，写在注释里

站点 HTML 内嵌了 shader 源码，注释就是作者的技术说明。上游自己列了谱系（README "Acknowledgements"）：

| 技术 | 来源 |
|---|---|
| Blobby surfaces / metaballs（融合行为） | Jim Blinn《A Generalization of Algebraic Surface Drawing》(1982) |
| Signed distance fields（SDF + smooth minimum） | Inigo Quilez 的 2D distance functions / smooth minimum / fBM |
| Spring integration（半隐式欧拉定步长） | Glenn Fiedler《Integration Basics》 |
| 光学处理（折射、色散、frost） | 上游自称是 Apple Liquid Glass 方向的 **original WebGL take** |

smooth minimum 是 IQ 的标准形，源码里两次出现（同一实现）：

```glsl
float smin(float a, float b, float k){ float h=max(k-abs(a-b),0.)/k; return min(a,b)-h*h*k*.25; }
```

shader 注释里能读到的判据（都是上游原话，值得当设计规则读）：

- `Blend only where surfaces face different ways (corners, gaps, steps);` —— **只在朝向不同的地方融合**（转角、缝隙、台阶），不是处处糊
- `A 3D normal from the 2D height gradient: flat in the middle, turning…` —— 法线由 2D 高度梯度求出，中间平、边缘转
- `Extrude, with the rim rolled over so the slab has a shoulder rather than…` —— 挤出并把 rim 翻卷，让板子有"肩"而不是薄片
- `Marded, not painted. The ray enters the slab, and every normal…` —— 金属/云是**行进**出来的不是画出来的：光线进入实体内部
- `Beer-Lambert, scattering by Henyey-Greenstein.` —— 云用 Beer-Lambert 加 HG 相函数
- `Grain lives in page coordinates, so a panel that resizes keeps its…` —— 颗粒活在页面坐标里，所以面板缩放时颗粒不动
- `Palette phases run on viewport position, like the grain:` —— 调色板相位跟着视口位置走

### 上游亲手调好的 preset 表（从 docs 站点提取）

这比 README 的参数逐个解释有用——是可直接对照的配置组合：

| 预设 | 上游原话 | 关键参数（从站点源码提取） |
|---|---|---|
| `Lumen`（默认） | Clear plasma, iridescent rim. The default look. | viscosity 默认 0.5, stretch 1, blend 40, rimHex `#9ff3e4` |
| `Studio` | Frosted workspace. Calm motion, quiet edges. | tint `#000000`, opacity .55, frost .85, **viscosity .7** |
| `Slate` | Opaque panels, no shine. Reads as a plain app. | opacity **1**, refraction **0**, dispersion 0, rim .5, highlight 0 |
| `Aqua` | Clear as water. Every sheen off, only the lens remains. | rim 0, highlight 0, shimmer 0, glow 0, wash 0, grain 0, **edgeLine .35**, refraction **1.5**, dispersion **1.6**, elevation **0** |
| `Neon` | Dark tinted panels, colored rims, fast and springy. | tint `#160b1e`, opacity .75, rim `#ff2d95` strength 1.6, **viscosity .15**, stretch 1.2, dispersion **2** |
| `Entropy` | Every motion field at maximum. Wobbly, dreamy, never still. | rim 1.3, highlight 1, shimmer 1.6, glow 1.4, **viscosity 0**, stretch **2.5**, flow **2**, blend **56**, dispersion 2.2 |

四档粘度预设（同上来源）：`Water` viscosity .1 / stretch 1.3 / flow .6；
`Gel` .5 / 1 / 0；`Honey` .85 / 1.8 / .3；`Solid` .6 / **stretch 0** / 0。

 playground 自己的说明（s11 帧 OCR 读出，对应 README "Clear as water" 节）：

> Four separate looks the material adds on its own. Turn all four off, with Rim and
> Highlight, and the plasma is a plain lens - the Aqua tab above.

**这句话就是它的设计哲学**：六种 sheen 各自是独立开关，所以"Plain glass, nothing but the lens"
是一个配置项，不是一个 fork。上游把这条写成了可抄的原则：**让"减去所有装饰"成为一个合法状态**。

## 可迁移的决策规则

1. **容器与内容分开，别把效果做在按钮上。** 上游明确 `Use Plasma for container components:
   panels, docks, cards, dialogs. Components should be nested inside.` 效果只给容器，交互件保持朴素。
2. **面板要么贴得比 blend 近，要么离得比 blend 远。** 上游原话：`Place surfaces together or
   further apart than the Blend distance. Smaller gaps render as liquid bridging them.`
   **中间态是 bug 不是特性**——这条对我们排布微细条与卡片间距有直接意义。
3. **把状态做成可读的。** `N panels, M joined`、`24px grid` 一直显示在界面上。
   这不是给用户看的开关，是**让效果可被验证**。对照本仓 `doctor` 与可信度自查——同一个取向：
   把"我看到的"和"实际是什么"分开显示。
4. **六个 sheen 各自独立，且"全关"是一个合法配置。** 见上。这条比任何参数值都值钱。
5. **性能预算是显式参数，不是隐藏成本。** `maxSurfaces`（默认 16）**编译进 shader**，
   改它要重编译；超了会 console warn 并开始丢面板（offscreen 先丢）。
   上游明说 `Two render passes loop over every slot per pixel, so set this value only as high as you need.`
6. **`flow` 会把轮廓揉出波纹，所以需要平直边缘时要显式关掉。** 上游原话：
   `NOTE: flow ripples the outline, so leave it at 0 whenever flush edges should stay perfectly straight.`
7. **降级路径不是空白。** 不支持的浏览器里 `Plasma renders as a CSS frosted panel and all layout,
   drag, and snap behavior still works`。SSR 时 surfaces 输出 CSS fallback，hydration 后 canvas 接管。
8. **`prefers-reduced-motion` 关掉的是"位移类"而不是"全部"。** 上游原话：它 disables
   `Lean, Pulse, the pointer Drop, and Spring - and the form-in`。**材质本身不动**——
   玻璃、rim、折射仍在。这与 beUI 的 fallback 定义（保留 opacity 与颜色，删 travel/scale/parallax）
   几乎一字不差，是第二个独立来源给同一判据。
9. **第二层 canvas 的代价要写在文档里。** 弹窗要浮在 scrim 之上时，需要第二个 provider
   （`ground="clear"`，把底层 canvas 当背景采样）。上游把代价写明了：`Each overlay is a full
   render pass while it is open, at the same size as the ground, plus one texture upload of the
   ground canvas per frame.` 并给出纪律：**随弹窗挂载、随弹窗卸载，没有弹窗开着时什么都不跑。**

## 应用建议清单（与实现解耦，尚未排期）

1. **`N panels, M joined` 这个"计数可读"可以直接搬。** 本仓已有"后台 N"、"🤖 N 子任务"胶囊，
   但那是计数不是状态。若做任何多 agent 聚合视图，把"几个在跑 / 几个在等"做成常驻读数，
   比让用户自己数更符合本条。
2. **要么贴得近要么离得远，别留半截间距。** 微细条、卡片、货架行之间的间距，若将来做融合/光桥效果，
   先按 blend 距离定两档；否则宁可按现有中距排，不要制造"看起来要连却没连"的中间态。
3. **考虑给玻璃卡片加一条"纯透镜"配置**（对应 `Aqua` preset：rim/highlight/shimmer/glow/wash
   全关，只留 refraction 1.5 + dispersion 1.6 + edgeLine .35 + elevation 0）。
   `Theme.swift` 今天有没有能力表达"只留折射不留彩边"，做前先读代码。
4. **macOS 26 有系统 Liquid Glass 材质，优先级高于自绘 shader。** 上游整套是 WebGL 自绘
   （SDF + smin + Beer-Lambert），SwiftUI 侧没有对应物也不必造。可搬的是上面 9 条规则，
   不是 shader。
5. **性能预算要显式。** 若做任何 WebGL/Metal 效果，把"最大表面数"做成编译期或显式参数并
   console warn，别静默丢。这与本仓"读不到就说读不到"同源。
6. **降级路径先设计。** 任何新材质都要先回答"不支持时是什么"——上游的答案是 CSS frosted panel，
   布局/拖拽/吸附全部照常。

## 没能核实的（本节与事实陈述分开）

**视频里的（72 帧 + scene-change 检测后仍未验证）：**

- **拖尾与粘度差异的运动形态：全片无证据。** 这是与 README 声明**最大的落差**——
  README 说 `Each panel undulates like a Slinky when moving`、viscosity 0 是水 1 是糖浆，
  但 d17.0/d17.4 两帧视觉读图明确写"无泪滴/彗尾/拉长，无运动模糊，看起来静止"，
  f10 写 "no motion trails, ghosting, or smear"。可能录制时面板运动幅度小、弹簧已 settle，
  或拖尾只在真实拖动帧出现。**本项目若要写"拖尾可调"，须另找素材或标为未验证。**
- Shimmer 在表面流动：需连续帧比对，未做
- dispersion 的彩色分裂边缘：rim 的虹彩 **≠** dispersion 色散，后者无帧证据
- 网格吸附与"推开其他面板"：界面一直显示 `24px grid`，但没有一帧能确认吸附或推开
- Mood `Aurora` / `Ember` 生效的样子：按钮在，没有一帧确认它们被选中并改变了背景色调
  （唯一变深的是 s15，但那是 Tint `#160b1e` 不是 mood）
- 六种材质（plasma/crystal/metal/wood/stone/cloud）：六个 tab 在，但没一帧能读全对应关系
- `4 joined` 只在 t16.5 一帧读到（3/2 joined 有多帧）——引用这个数字时注意是单帧
- 熔断速度 `< 200ms` 由 0.2s 帧间隔推得，真实可能更快

**README/站点层面的：**

- 六个 shader demo **一个都没实跑**（playground 与 workspace example 未在浏览器里操作过），
  一手证据止于 README 全文、站点 HTML 与视频逐帧
- **star 数有两个口径**：视频里 GitHub 页读到 **119**（f16/f31/f34 OCR 一致 + f28 视觉读图双证），
  GitHub API 与仓库页读到的描述是 **125**。不同期，两个都记，不合并
- `74 Commits`（x32.0 与 f28 双证；`4 Commits` 是截断误识）
- 提交信息 `Show a longer demo: panels tearing away and fusi…` 与 d 系列 0 joined 的关联是**字面推测**，
  没有证据证明录的就是那个 demo 页面

**明确标为推断而非事实的：**

- "Joined" 标签的语义（f1/s2 五面板分离却全标 Joined → 推断它表示"属于同一 plasma 实例"，
  与 README 的 `onJoinChange` 不是一回事）**未在代码中核实**
- 预置 tab 选中态（f13 判定 Entropy、t4.5 判定 Tidal+Gel）依据视觉读图的"高亮条"描述，
  **OCR 不返回高亮信息**
- blend 40→56 导致 joined 3→4 是**共变非因果**（见上）
- 颈宽 1/5–1/4 面板宽是视觉模型像素估算，非测量值

## 取证命令

```sh
# 上游 README 全文（本文所有引文出处）
curl -sSL https://raw.githubusercontent.com/CruxGarden/plasma-ui/main/README.md

# docs/playground 站点（preset 参数表与 shader 注释出处）
curl -sSL https://cruxgarden.github.io/plasma-ui/ -o site.html
python3 -c "
import re
s=open('site.html',encoding='utf-8',errors='ignore').read()
print(re.findall(r'name:\"([A-Z][a-z]+)\",blurb:\"([^\"]{10,120})\"',s))
print(re.findall(r'float\s+smin[^}]{0,200}\}',s)[0])"

# 视频（走系统代理；twimg 直连不通）
scutil --proxy
curl -x http://127.0.0.1:10808 -o clip.mp4 \
  'https://video.twimg.com/amplify_video/2103497560392622080/vid/avc1/1652x1080/qdwX9zlrvzY5THOw.mp4?tag=29'

# 视频元数据（1652x1080 / 60fps / 2173 帧 / 36.217s）
ffprobe -v error -show_entries format=duration:stream=width,height,nb_frames,r_frame_rate \
  -of default=nw=1 clip.mp4

# 定位硬切点（本文两处 segment 边界即由此测出）
ffmpeg -i clip.mp4 -vf "select='gt(scene,0.02)'" -vsync vfr -f null - 2>&1 | grep pts_time

# 密集抽帧（joined 计数器序列即由此读出）
for t in 16.0 16.5 16.6 16.8 17.0 17.2 17.4 17.6 17.8 18.0 18.2; do
  ffmpeg -ss $t -i clip.mp4 -frames:v 1 -q:v 4 -y "d$t.jpg"
  dim ocr recognize "d$t.jpg" --json
done

# 仓库元数据
curl -sSL https://api.github.com/repos/CruxGarden/plasma-ui | grep -E '"(stargazers|forks)_count"|"license"'
```

> 环境事实：本机 `*.twimg.com` 与 `pbs.twimg.com` 直连超时，必须走系统代理
> （`scutil --proxy` 显示 HTTP/HTTPS/SOCKS 均在 `127.0.0.1:10808`）。
> 视频 mp4 走该代理**可以**完整下载（13.9MB / 36.2s / 1652×1080）；
> 上轮 07 篇拿不到视频是因为当时没有走系统代理。
