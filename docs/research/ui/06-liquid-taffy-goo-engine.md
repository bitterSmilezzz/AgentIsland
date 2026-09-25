# liquid-taffy：一个 gooey 交互的**边框为什么不膨胀**

> 目的：本项目迟早要给灵动岛做"融合 / 拉伸"一类的粘滞动效（`libraries-dev` 案例库里 `Liquid`
> 那一支就指到这里）。[arknow91/liquid-taffy](https://github.com/arknow91/liquid-taffy) 不是包，
> 作者明说是**参考实现**，因此它是本案例库里最值得逐行读的一份源码。
> 引文出处：2026-09-26 抓取的 README 全文（上游 main 分支，14,970 字符）与 `git clone` 后的
> 源码（commit `4bcf400`，2026-08-20）；star / license / 时间线取自 GitHub API（同日，161 star / MIT）。
> 下游仓库摘要 `liquid-taffy.md` 与上游 `README.md` 只差 Markdown 排版，正文与措辞一致。

## 一句话结论

**它把"blur + alpha-threshold 的 metaball 果冻效果"里最脏的一环——边框——解掉了。**
果冻的轮廓不是描边，而是**同一条模糊 alpha 的两条等值线之间的那条缝**；缝宽取决于 blur σ，
所以每个工作 blur 都要配自己的一对阈值，并且 blur 与阈值**同帧切换**。
作者由此解释了业界通病：

> That is why gooey buttons in the wild appear to inflate the moment they start moving.
> （README，"The border that never swells"）

外加三条同样可迁移的工程纪律：**弹簧不用缓动曲线**（物理弹簧采样成 GSAP CustomEase）、
**关节点光是 latch 不是 test**、**`prefers-reduced-motion` 只在一个地方问**。

## 它是什么

作者给自己的定位（README 开头）：

> This is a reference implementation, not a package. **It exists so you can read the source and
> see exactly how a production-quality gooey interaction is put together**: the SVG filter, the
> border trick, the spring choreography, and the shared grab gesture.
>
> Everything here is built around one question — not "how do surfaces melt into each other" but
> **"how does one surface feel like it has a body"**.

三个交互共用一套引擎（一张表 + 一句引擎共享的原话）：

| 交互 | 按钮做什么 |
|---|---|
| **Anchored dropdown** | Stays put and pours a dropdown out of itself, hanging **14px** above — the drop leaps upward on a pop spring, rows condense bottom-up. |
| **Morphing dropdown** | *Becomes* the dropdown. The circle stretches upward, its mass drawn into the pour; closing gathers the panel, necks it, and dives it back into the circle — splat included. |
| **Speed dial** | Breaks into three droplets that ooze out of the button, launch past full size, and ring themselves still. |

> All three share one trigger, one visual language, and — literally — one gesture engine.

亮/暗两帧是同一套东西的两半，不是两套配色（README "The two frames"）。
包栈：**GSAP + React 19 + Vite**，无测试框架（`package.json` 只有 dev/build/preview）。
`npm install && npm run dev` 即可跑。

## 手法一：三层叠加，任意时刻只有一层可见

README "Three layers, one picture at a time"：

> Each component draws its scene twice, stacked:
>
> **Crisp bodies** — plain CSS circles and an SVG squircle with a real 1px border and a real
> shadow. **This is the resting picture.**
>
> **The goo** — the same shapes cloned into one `<svg>`, run through a **blur → alpha-threshold
> filter** (the classic metaball trick), which draws its own rim and casts one shadow for the
> whole mass.
>
> **Icons and hit areas** — transparent buttons riding the same tweens, so glyphs stay crisp
> above the liquid and accessibility lives in real DOM buttons.
>
> **Exactly one of the first two layers is visible at any moment.** At rest you are looking at
> real CSS borders; the instant anything moves, the goo takes over the whole picture; when the
> motion settles, it hands back. **No half-blended states — even a sub-pixel of overlap reads as
> a doubled border.**

源码里的同一句话在 `LiquidMenu.tsx:501-504`：

```ts
/* Exactly ONE of the two pictures exists at a time: at rest the crisp CSS
   circles; in motion the goo (which draws its own rim and shadow). Never
   both — even a sub-pixel of the other layer reads as a second border. */
```

**第三条"图标与命中区骑同一套 tween"这一招是直达可达性的**：真 `<button>` 永远在 DOM 里、
永远可聚焦，粘滞层只是画在上面的画。`aria-hidden` 加在装饰层上
（`LiquidAdd.tsx:848` 给 bodies 整层、`:873` 给 goo SVG）。

胶水层与真 layer 的所有权切得很干净（`LiquidAdd.tsx:451-464`）：

```ts
handoff: (tl, at) => {
  tl.set(bodiesRef.current, { autoAlpha: 1 }, at);
  ...
  tl.to(gooRef.current, { autoAlpha: 0, duration: 0.15, ease: "power1.out" }, at);
  ...
  tl.call(() => rootRef.current?.removeAttribute("data-liquid"), undefined, at + ?);
},
```

同时作者对**阴影也坚持"任意时刻只有一个"**（`LiquidAdd.tsx:218-224`）：

> Exactly ONE shadow at any moment — the speed dial's rule. The goo casts its own while it owns
> the picture; the crisp bodies carry theirs on their container. At a handoff the bodies snap
> visible UNDER the still-opaque goo, so for the goo's fade both would cast — **a doubled shadow
> that pulses dark.** The goo's shadow switches off in the SAME frame the bodies snap on.

## 手法二：边框不膨胀——本篇最重头的一条

### 原理：轮廓是两条等值线之间的缝

README "The border that never swells" 原文：

> The goo's outline is **not a stroke**. It is the sliver between two iso-alpha contours of the same
> blurred alpha — an outer threshold that lands on the border's outer edge and an inner one a hair
> inside it. **How far apart those contours land in pixels depends on the blur σ**, so a threshold
> pair that draws a 1px rim at σ=1 draws a fat, misplaced one at σ=7. That is why gooey buttons in
> the wild appear to inflate the moment they start moving.
>
> Here **every working blur carries its own threshold pair, solved offline by rasterizing the exact
> filter over a 32px disc and integrating the rim's ink until its outer edge sat on the CSS
> border's outer edge and its weight equalled 1px.** Blur and thresholds are switched together, in
> the same frame, always (`src/components/liquid/goo.ts`). The result: one border, whatever the
> liquid does.

### 源码级实现：一张解出来的表 + 三个一起改

`src/components/liquid/goo.ts`（全文 58 行，注释即文档）：

```ts
/* Solved [outer, inner] threshold offsets per working σ.
   σ4 is the exception: it is the morph's opening blur, kept at the family's
   older fixed pair so the morph looks exactly as it always did — it is the
   one blur here that has not been through the solver. */
export const GOO_RIM_THRESHOLDS: Record<number, readonly [outer: number, inner: number]> = {
  1: [-14.5146, -24.6721],
  4: [-12.25, -14.25],
  5: [-12.7296, -15.063],
  7: [-11.6925, -13.245],
};

/* Alpha-only threshold: the RGB rows stay identity so the interior keeps the
   blurred source colors (two fills blend into a gradient at a neck). */
export function gooThreshold(offset: number) {
  return `1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 30 ${offset}`;
}
```

**注意那 6 位小数**：这不是手调的，是**离线栅格化解出来的**（注释：solved by rasterizing the
exact filter over a 32px disc and integrating the rim's ink，使外缘落在 r=16 的 CSS 边框外缘、
墨量等于 1px）。作者还说明求解是在**亮帧的 `#dadada`** 上做的，之所以能同时适用暗帧，因为
两者的 rim 是同一个 alpha（`goo.ts:10-12`）。

**blur 与阈值必须同帧改**，写成一个函数而不是三处调用（`goo.ts:40-57`）：

```ts
/* The blur and its two thresholds are ONE setting — a blur changed without
   its matching thresholds is a different border. Always set them together,
   in the same frame: on a timeline when the switch belongs to a
   choreography, straight through gsap when it happens on a pointer event. */
export function setGooBlur(els: GooFilterEls, blur: number, tl?: gsap.core.Timeline, at = 0) {
  const [outer, inner] = GOO_RIM_THRESHOLDS[blur];
  const steps: [Element | null, gsap.TweenVars][] = [
    [els.blur,  { attr: { stdDeviation: blur } }],
    [els.rim,   { attr: { values: gooThreshold(outer) } }],
    [els.inner, { attr: { values: gooThreshold(inner) } }],
  ];
  ...
}
```

三个组件的**工作 blur** 各不相同，因此都从这一张表取（各自常量）：

| 组件 | ACTIVE | REST | GRAB |
|---|---|---|---|
| `LiquidMenu`（speed dial） | 7 | 1 | 5 |
| `LiquidAdd`（anchored dropdown） | 5 | 1 | 5 |
| `LiquidMorph`（morphing dropdown） | 4 | 1 | 5 |

**σ=7 那一行是实打实被解过的**——正是 README 里"σ=1 的阈值对拿到 σ=7 会画出一条又胖又偏的边"
的那个场景。表里唯一没进过求解器的 σ=4（morph 的开场 blur）被作者显式标注为"保留了家族的旧固定对"，
**没解过就说没解过**。

### 一条容易漏的推论：rim 遮罩也要用同一套 blur + 阈值

`LiquidAdd.tsx:197-203` 的注释解释了为什么这个文件里 `setGooBlur` 被调用**两次**：

```ts
/* Blur + rim thresholds are ONE setting — see liquid/goo.ts. The mask
   carves its rim out of the same blur with the same pair of thresholds — a
   mask one setting behind would let the colour slip off the border it is
   supposed to be painted on. */
```

第二条 filter（`rimOnlyId`，`LiquidAdd.tsx:940-969`）把同一条链重跑一遍，`feComposite` 用
`outer ∖ inner` 得到**只有那条缝的 mask**，再用 `<use href>` 引用同一组 blob——
"so the mask cannot drift from the picture: every tween that moves a drop moves its mask with it."
即：**同一个物理量在两处被消费时，要么共用同一份来源（`<use>`），要么就一定会漂。**

> **给我们的判据**：macOS 上我们没有 SVG filter，但"同一个光学参数驱动两个视觉输出"的结构遍地都是
> （玻璃材质的模糊半径与描边宽度、光晕半径与阴影扩散）。**把它们收进一个函数、一个表，
> 不要在两处各写一次。**

## 手法三：拉伸引擎——四珠链

README "The stretch is one engine"：

> The grab gesture — press, drag, snap back — is a **single implementation shared verbatim** by all
> three interactions (`src/components/liquid/stretch.ts`):
>
> A chain of four beads is drawn out of the rim, shaped like a real liquid finger: **thick root,
> thin neck, a modest bulb at the head.** Each bead follows a different fraction of the pull with a
> different lag, which is what keeps the goo bridge unbroken and gives the stretch its taper.
>
> The pull is clamped — past **44px** the sponge stops giving — and the grabbed body leans after
> your cursor, stretching slightly along the pull direction.
>
> Release whips the chain home head-first on a spring, and the grabbed body shakes itself out with
> a squash-and-stretch splat.

源码 `stretch.ts:28-43` 把"粗根/细颈/适度球头"落成了表（**实际是 6 颗珠**，README 说 four beads，
以源码为准）：

```ts
/* Four beads shaped like a real liquid finger: THICK at the root so it melts
   into the body it is pulled from, THINNEST through the middle (the neck),
   and a modest bulb at the head. `thin` is how much each bead narrows at full
   tension — the neck necks down as the finger extends, the head keeps its
   mass. `lag` grows down the chain so the beads trail the head and keep the
   goo bridge unbroken all the way out. */
export const GRAB_MAX = 44;

export const GRAB_CHAIN = [
  { follow: 1,    size: 0.85, thin: 0.06, lag: 0.16 },
  { follow: 0.84, size: 0.72, thin: 0.16, lag: 0.18 },
  { follow: 0.68, size: 0.65, thin: 0.19, lag: 0.2  },
  { follow: 0.52, size: 0.64, thin: 0.19, lag: 0.22 },
  { follow: 0.36, size: 0.7,  thin: 0.14, lag: 0.24 },
  { follow: 0.2,  size: 0.8,  thin: 0.08, lag: 0.26 },
] as const;
```

四个维度各管一件事：`follow`（跟多少 → 桥不断）、`size`（多粗 → 锥度）、
`thin`（绷紧时收多少 → 颈收缩）、`lag`（迟多少 → 拖尾）。
pointerMove 里一共只有几行（`stretch.ts:269-299`）：

```ts
const reach = Math.max(0, dist - 6);
const pull  = Math.min(reach * 0.7, GRAB_MAX);
const tension = pull / GRAB_MAX;
...
gsap.to(host.chain()[index], {
  x: grabBase.x + ux * pull * link.follow,
  y: grabBase.y + uy * pull * link.follow,
  scale: link.size * (1 - tension * link.thin),
  duration: link.lag,
  ease: "power3.out",
  overwrite: "auto",
});
```

### 引擎与宿主的边界：引擎决定 WHEN，宿主知道 HOW

`stretch.ts:1-13` 的注释把这条契约写得比任何架构图都清楚：

> The engine owns the gesture: the math, the tweens, the release choreography and the press/click
> bookkeeping. Each component remains the owner of its own goo — blur, thresholds, alphas, the
> crisp-picture handoff — and hands those over as StretchHost callbacks. **The engine decides WHEN;
> the host knows HOW.**

宿主还提供了一个**专为"藏起来的东西"准备的钩子**（`stretch.ts:79-84`）：

> Show the goo at the grab blur: alphas re-armed, thresholds matched, data-liquid set. Also the
> host's chance to **HIDE any blob parked inside the trigger** (a shrunk dropdown panel) — a hidden
> mass that stays put while the grabbed circle leans away pokes out of the silhouette as a hump,
> which is exactly the deformation this engine exists to avoid.

对应 README 的同一段（"One subtlety worth stealing"）：

> while a dropdown is closed, its panel hides inside the button, scaled to **~0.1**. During a grab
> that hidden mass must sit the gesture out — it can't ride the button's lean, so left in the goo it
> pokes out of the moving silhouette as a hump. The engine's host contract has a hook for exactly
> this.

（源码常量：`PANEL_REST_SCALE = 0.11`，`LiquidMorph.tsx:51`。）

### 一条关于"阴影不得跳"的细节

`stretch.ts:186-198`：

> Rotation rides the SAME spring home as the position. The circle itself hides its rotation, but its
> **box-shadow does not** — the shadow's offset lives in the element's local frame, so a held
> rotation makes it point sideways, and **zeroing it with a `set()` after the handoff made the shadow
> visibly JUMP as the crisp body took over. Springing it home lands it (shadow and all) before
> anyone can see a seam.

**教训**：交接之后用 `set()` 归零，等于把跳变藏在"用户看不见"的假设里。同一个物理量要走同一条曲线。

## 手法四：弹簧，不是缓动曲线

README "Springs, not durations"：

> **Nothing here eases with a stock curve.** Two physical springs are sampled into GSAP CustomEase
> polylines (`src/components/liquid/springs.ts`):
>
> **House spring** (ζ=0.434, ω=22.46 — 22% overshoot): everything that pops in or springs back.
>
> **Pop spring** (ζ=0.479, ω=18.09 — 18% overshoot): the louder curve that carries each drop's
> whole leap out of the button.
>
> **Entrances overshoot and ring; exits are authored, not reversed** — big surfaces wind up ~10% the
> wrong way (anticipate) before collapsing, and whatever lands on the button is absorbed with an
> impact squash. **Colors never spring.**

源码把曲线实体化了（`springs.ts`）：两条各 35 个点的 polyline，由
"sampled by the motion kit's spring.mjs" 得到，再由 `springEase(name, points)` 铸成 CustomEase。
顶部注释是一条**防漂移规则**：

> ONE copy: a component that needs its own uniquely-named ease instance calls `springEase` with its
> own name, **but the curve itself can never drift**.

**"退场是编排的，不是反向的"** 在 `LiquidMorph.tsx:36-37` 有实体：

```ts
/* Anticipate — exit: wind up ~10% the wrong way, then collapse. */
const ANTICIPATE = CustomEase.create("liquidMorphAnticipate", "0.36,0,0.66,-0.56");
```

退场三连（`LiquidMorph.tsx:600-625`）：gather（scaleX 0.32 / scaleY 0.36 走 ANTICIPATE，
两轴错开 0.045s 制造 jelly phase-lag）→ dive → splat（`scaleX 1.2 / scaleY 0.82` 起手，
`0.94 / 1.07` 回一下，最后 `SPRING` 归位）。
**入场用 overshoot，退场用 anticipate + impact**——一条曲线的反向播放给不了这个。

"Colors never spring" 是真纪律：颜色只在 `styles/tokens.css` 里换帧，任何 hue 都不进 tween
（`hues.ts:24-26` 的 `motifHue` 在亮帧直接返回 null）。

## 手法五：关节点光——三条能直接偷的规则

README "The joint light"：

> Drag one drop toward another and, **a few pixels before they touch**, the border between them
> starts telling you about it (`src/components/liquid/seam.ts`). Nothing is painted on top of the
> picture: the rim keeps being the rim, it just runs colour through the joint and back out again a
> little way along each drop.
>
> The engine reports where two rims meet, how strongly, and whether they have **genuinely crossed
> rather than merely leaned**. Three things in it are worth stealing:

### 规则①：一个 body 一个 lobe，不是一对邻居一个

源码 `seam.ts:162-172`（**比 README 更狠，README 只说了"两 washes 会相加"**）：

> One lobe per body, not per pair. A body in two live joints at once — the drop a finger is welded
> into while its own rim still leans on a third — used to be painted by both, and **two washes on
> one border sum: the rim shimmered and dragged the neighbour's hue across it.** And a pair-shaped
> lobe could only ever say something about two bodies, so in a three-body cluster the odd one out
> either went unpainted or wore a neighbour's colour. Per body: **every drop in the picture shows
> ITS OWN hue on ITS OWN side, exactly once, and the hues meet in the seam between them.**

实现是一句 `spoken` set（`seam.ts:637-674`）：

```ts
/* ONE LOBE PER BODY. Candidates arrive strongest first, so a body always
   takes its side from the joint it is most involved in and is spoken for
   after that: no rim is ever washed twice, and no body is left showing a
   neighbour's colour. A three-body cluster therefore lights all three,
   each in its own hue, from the two joints that make it. */
```

**可迁移的规则**：当一个视觉量可能被多个来源同时叠加时，**按归属体归一，不按相邻关系归一**。

### 规则②：焊接是 latch，不是 test

源码 `seam.ts:86-99` 完整解释了为什么单阈值一定闪：

> A weld is a **LATCH, not a test.** Two thresholds, not one: it takes hold when the rims have
> genuinely crossed (WELD_ON) and only lets go once they have visibly come apart again (WELD_OFF) —
> **the gap has to travel five pixels to change the answer.**
>
> A single threshold sits exactly where the finger's beads breathe: their radii pulse with tension,
> their positions lag on tweens, and rolling a finger from one neighbour toward the next walks the
> gap back and forth across that one line — so the flag flipped several times per gesture and every
> glyph wired to it **BLINKED**, the X worst of all, because it is in every joint.
>
> The clock hold below is the second line of defence, for chatter fast enough to cross both
> thresholds.

数字（`seam.ts:98-102`）：`WELD_ON = -3`、`WELD_OFF = 2`、`WELD_HOLD = 0.18`。
README 的同一段写成 "the gap has to travel five pixels to change the answer"。

### 规则③：lobe 大部分停在关节亮的半径之外

`seam.ts:78-84`：

> How far off the joint a body's lobe is parked, as a share of the lit radius. **Most of it, not
> half**: a joint lights weakest — and so at its SMALLEST radius — exactly when it first forms, and
> at half the lobes sat barely a few pixels apart, so **each body's border opened wearing a mix of
> both hues and only resolved as the light grew.** This far out, a border is its own colour from the
> first lit frame and the blend stays in the seam.

常量：`LOBE_OFFSET = 0.85`。

### 还有一组"提前亮"的时间常数

`seam.ts` 顶部把"触碰前几像素就开始亮"拆成了可命名的量：
`REACH = 14`（接触前就开始互相拉，比 grab blur 下的颈长略宽，永远不用在接触那一瞬凭空出现）、
`RIM_MIN = 3` / `RIM_FADE = 4`（不要按阈值开关，用一段过渡带）、
`NECK_HOLD = 0.85`（颈变宽到小球的 0.85 倍才开始灭）、
`SEAM_RISE = 0.055` / `SEAM_FALL = 0.14`（起快落慢）、
`SEAM_CHASE = 0.06`（追上关节）、`SEAM_MIN = 13` / `SEAM_MAX = 24`（亮的弧长）。

**这些常量本身就是文档**。把"什么时候亮、亮多宽、起多快、怎么让位"全部命名成常量并写清为什么，
是这份源码最好读的原因。

## 手法六：声音是一台机器塑十种形

README "The voice"：

> Every sound is synthesized on the spot (`src/components/liquid/sfx.ts`), no samples, so a repeated
> gesture never sounds like one file played twice. **It is one little machine shaped ten ways:**
>
> a **body** — an oscillator sliding between two pitches; **the slide is the viscosity**
> a **throat** — a resonant lowpass sweeping with it: the hollow "bloop" of a bubble
> a **wobble** — a slow LFO bent into the pitch, the ear's version of the springs
> a **smack** — a whisper of filtered noise at the attack, the contact itself
> an **envelope** — attack and decay, and these are the character controls: everything else is a
> shape, **the envelope is how hard and how long you press it**
>
> A frame is described as **multipliers on that shape, never a second set of numbers**, so the two
> rooms cannot drift apart. Each of the dial's drops speaks in its own pitch, so a contact tells you
> which contact it is without looking.

源码 `sfx.ts:1-24` 把这些话写在函数上方，并补了两句 README 没有的动机：

```ts
   No samples. Every hit is built on the spot, so nothing ever repeats
   exactly: a drop landing twice sounds like the same drop landing twice,
   not like a file played twice. **That is the whole reason UI audio made of
   samples feels dead — the ear catches the loop long before the eye does.**
```

`sfx.ts:22-24` 给出两帧"房间"的差异，只用乘数描述：

> And two CHARACTERS, one per frame, because the two stages are different rooms. Light is a shallow
> dish of water: higher, brighter, drier, quick to die. Dark is a deep vessel of syrup: lower,
> darker, wetter, resonant, slow to let go. **Nothing else changes — same machine, same gestures,
> the room around it is what differs.**

**对我们**：`SoundEffectsManager.swift:18` 用的是 `NSSound(named:)`——**预置音频文件**。
这篇给的替代形态不是"别用系统音"，而是：**如果要自制，就让差异落在乘数上而不是新素材上**，
且"每种接触有自己的音高"这条（`Each of the drop's speaks in its own pitch`）是零成本的辨识度。

## 手法七：One palette, one word

README 用四段讲"曾经是四份拷贝，现在是一份"：

> Light and dark used to be four things: a "light" | "dark" alias re-declared in every component,
> a prop threaded down through two stages, a ref copied into each surface so long-lived callbacks
> could ask which frame they were speaking in, and a palette hand-copied into four stylesheets.
> **Copies drift — two of those stylesheets had already landed a percent apart on the same hover
> tint.**

收敛后的三条硬规则（README "One palette, one word" + 源码顶部注释）：

1. **颜色只在 `src/styles/tokens.css`**（`tokens.css:1-11`：`ONE palette for the whole project —
   both frames, in one file`），亮在 `:root`、暗在 `[data-theme="dark"]`，
   `A component stylesheet READS these, it never declares a colour of its own`。
   每个 liquid 表面**在自己的 root 上重复 `data-theme`**，这样组件被单独拎出来用仍知道自己在哪一帧
   （`tokens.css:4-6` / `LiquidAdd.tsx:844` 的 `data-theme={theme}`）。
2. **帧是一个值**：`theme.ts` 一个 `LiquidTheme` type + 一个 context。`App` 提供一次，
   需要帧的 JS（motif hues、voice 的两个房间、burst）都问 `useLiquidTheme()`，
   `Nothing is threaded through props, so nothing can forget to pass it on`。
   引擎类还有 `useLiquidThemeRef`（`theme.ts:36-43`），因为引擎活在 ref 里、
   首次渲染后拿到的东西会过期。
3. **CSS 读不到的地方才进 TS**（README 原文）：`What stays in TypeScript rather than CSS is what
   more than CSS reads. liquid/hues.ts holds the motif — one hue per glyph per frame — because the
   joint's gradient, a glyph's glow and the burst all need those values in JS`，
   并且用一条规则让亮帧保持单色：`there motifHue returns null, and every consumer falls back to
   its own ink. One function, so the two frames can never drift.`

源码 `theme.ts:1-14` 把"四份拷贝"的账记得很具体：

> Light and dark were four things before this file: a type alias re-declared in every component, a
> prop threaded down through two stages, a `themeRef` copied into each surface so long-lived
> callbacks could ask which frame they were speaking in, and a palette hand-copied into four
> stylesheets. They are one thing now. **The value travels by CONTEXT — the frame is a property of
> the stage, not of any one drop standing on it — and the colours travel by custom property.**
> A component reads both; it owns neither.

顺带一条"重构不减文件数"的观察（README）：两个下拉去掉各自声明的颜色之后"they were the same file
twice over"，于是共享面抽成 `dropdown.module.css`，每个变体只加"面板挂在哪"这一件事
（14px 上方 或 正贴按钮）。

## 手法八：可达性——本项目最该直接借鉴的一节

README "Accessibility" 原文（四条加一句）：

> `prefers-reduced-motion` **collapses every animation to a static state change**, including the
> press squash.
>
> The trigger is a real `<button>` with `aria-expanded` / `aria-haspopup` / `aria-controls`; the
> dial's items are `role="menuitem"` and the dropdowns' multi-select rows are
> `role="menuitemcheckbox"` with `aria-checked`, inside a `role="menu"`; **closed menus are inert**.
>
> **Colour is never the only carrier**: on the dark frame a selected row is a tick and a wash, a
> merge is a glyph change and a light, and the light frame carries the whole thing with no colour
> at all.
>
> **Escape closes and returns focus to the trigger; a drag-release is distinguished from a click**,
> so stretching something never accidentally activates it.

### `motion.ts`：prefers-reduced-motion 只在一个地方问

`src/components/liquid/motion.ts` 全文 10 行，注释比函数长：

```ts
/* The one motion question every component in here asks: may I move at all?

   ONE implementation — it was four, copied byte for byte into the switch, the
   checkbox, the radio and the pill row, while the liquid surfaces asked the
   grab engine for it. A component reaching into the grab engine for a media
   query was the tell that this belonged somewhere of its own. */

export function prefersReducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
```

**"asked in one place" 是机械事实**：`grep -rn prefersReducedMotion src` 得到 20 处调用、
分布在 9 个文件，但它们**全部 import 这一个函数**。调用点多不是问题，
**实现只有一份、所以口径不可能分叉**才是要点。

而且它是**每个动画入口的第一行**，不是渲染完再补的判断（`stretch.ts:228` / `:270`、
`LiquidAdd.tsx:512` / `:526` / `:560` / `:648`、`LiquidMorph.tsx:422` / `:436` / `:470` / `:575`、
`IconMorph.tsx:98`、`RowHover.tsx:62`、`SelectionBurst.tsx:74`、`PillTabs.tsx:82`、
`ThemeStage.tsx:58`、`LiquidMenu.tsx:562` / `:657` / `:795` / `:820`）：

```ts
const beginGrab = (...) => {
  if (prefersReducedMotion() || event.button !== 0) { return; }
  ...
```

静态退化的落点也是显式的（`LiquidAdd.tsx:560-562`）：

```ts
if (prefersReducedMotion()) {
  setStaticState(true);   // 秒开/秒关：开=终态，关=收态，全部跳过 tween
  return;
}
```

即**"塌成静态状态变化"不是把动画时长改短，而是直接给出终态**。
README 那句 "including the press squash" 对应源码里 `pressPanel()` / `releasePanelPress()`
第一行的同一个 early return——连"按下去扁一下"这种 2% 的形变也照塌。

### `consumeClick`：拖动松手与点击必须分开

`stretch.ts:349-356`：

```ts
const consumeClick = () => {
  pressed = false;
  if (suppressClick) {
    suppressClick = true→false;
    return true;      // 这次 click 是一次拉伸的尾巴，必须被吞掉
  }
  return false;
};
```

上游判定"拉伸超过 12px 就算拉伸过"（`const wasStretched = stretchDist > 12;`，`stretch.ts:148`），
并让拖动的宿主在 `onClick` 里先问它（`LiquidAdd.tsx:720-729` / `:776-785`）。
**这条对触控/拖拽并存的 UI 是刚需**，且实现只有 8 行。

### 真 button + `inert`

`LiquidAdd.tsx:1080-1102`：

```tsx
<div id={menuId} ... role="menu" aria-label="Shapes menu" inert={!isOpen}>
  ...
  <button type="button" role="menuitemcheckbox" aria-checked={selected[index]} ...>
```

触发器（`LiquidAdd.tsx:1130-1138`）带齐 `aria-expanded` / `aria-haspopup="menu"` /
`aria-controls={menuId}`。Escape 关 + 还焦点（`LiquidAdd.tsx:816-819`）：

```ts
if (event.key === "Escape") {
  closeMenu(false);
  triggerRef.current?.focus();
}
```

## 对本项目的具体关联

### 1. reduce-motion：README 的限制条目曾按错键名 grep，已按代码重写

README.md:246 曾写"**所有动效都不看 `prefers-reduced-motion`**：`Sources/` 下没有
`reducedMotion` 处理"。**那句话与代码不符**——它按 `reducedMotion` 这个 Web 侧拼法 grep，
而 SwiftUI 的键名是 `accessibilityReduceMotion`。实测（2026-09-26，本目录 grep）：

| 文件 | `accessibilityReduceMotion` | 动画 API |
|---|---|---|
| `Sources/AgentIsland/DockedSliver.swift` | 2 | 3 |
| `Sources/AgentIsland/AgentRingView.swift` | 3 | 2 |
| `Sources/AgentIsland/ActivityMatrixDots.swift` | 1 | 2 |
| `Sources/AgentIsland/IslandView.swift` | **0** | 3 |
| `Sources/AgentIsland/IslandComponents.swift` | **0** | 1 |
| `Sources/AgentIsland/IslandPanelPositioning.swift` | **0** | 2 |
| `Sources/AgentIsland/EventBannerView.swift` | **0** | 2 |
| `Sources/AgentIsland/AgentRowView.swift` / `DetailViews.swift` / `LiveLogStreamView.swift` / `TokenAnalyticsView.swift` / `TokenSummaryBar.swift` / `ProcessTreeView.swift` | **0** | 1–4 |

**正确表述**是"呼吸微光、环形进度、点阵脉冲三处已接；岛体展开/收缩、贴边弹出、横幅、
列表与图表转场没有接"。v0.0.138 已按此重写 README.md:246 那条，
并把"曾写错过什么、为什么错"留在 [01 篇](01-3dicon-looping-3d-icons.md) 与本篇。

### 2. 最该抄的形态：`motion.ts` 的 "asked in one place"

本项目今天**每个接到开关的 View 各自 `@Environment(\.accessibilityReduceMotion)`**
（`DockedSliver.swift:110` 与 `:201` 同一个文件里读了两遍，`AgentRingView.swift` 读了三遍），
这正是 liquid-taffy 在注释里点名的坏味道：

> A component reaching into the grab engine for a media query was the tell that this belonged
> somewhere of its own.

可搬的形态是：**Core 侧放一个唯一真源（例如 `AgentIslandCore` 里一个 `MotionPolicy` /
`ReducedMotionProvider`），所有 View 从同一处问；并且每个动画入口的第一行就问
（而不是在 body 里给某个具体动画加 `if`）**。这样将来补"岛体展开也塌成静态"时只加一处实现、
调用点照抄，不会出现"有的地方塌了有的没塌"。

### 3. "塌成静态状态变化"这个定义可以直接用

上游不是"把时长改短"，是给终态。对照我们今天的 `reduceMotion` 用法
（`DockedSliver.swift:73` 的 `if reduceMotion { removeAllAnimations() ... }`、
`AgentRingView.swift:170` / `:215` 的 `guard !reduceMotion else { return }`、
`ActivityMatrixDots.swift:78-90` 的条件表达式）——**方向已经对了**（同样是"不播"），
缺的是：**没接的那几处需要同类处理 + 一处统一入口**。

### 4. 颜色不是唯一载体

上游"the light frame carries the whole thing with no colour at all"是本仓「五态各有独立语义与颜色」
的一个补充约束：**状态不能只靠颜色**。可核对的是岛内五态是否每态都有非颜色的第二个载体
（形状、文案、`aria`）。

### 5. 声音上的替代形态

`SoundEffectsManager.swift:18/28/35` 走 `NSSound(named:)`。上游的 `sfx.ts` 给出的不是
"换成合成器"，而是三条规则：**差异用乘数描述**（两帧不会漂）、
**每种接触一个音高**（辨识度免费）、**同一形状从不逐字节重复**（耳朵先发现循环）。

## 应用建议清单（与实现解耦，尚未排期）

1. **建一个唯一的 reduce-motion 真源**（Core 侧一问一答），把今天三处 View 的
   `@Environment(\.accessibilityReduceMotion)` 改为向它提问；新动效一律**在入口第一行**问。
   这是本篇最可直接落地的一条。
2. **给"塌成静态"立定义**：`reduce-motion` 开时不播动画但**状态必须到终态**
   （`DockedSliver.swift:73` 现在的去动画相当于"停在当前帧"，需核对是否等价）。
3. **README「已知限制」的 reduce-motion 条目按代码重写**（区分已接/未接的文件），
   本篇提供核实过的表格。
4. **任何"同一光学参数驱动两个输出"的地方收成一个函数 + 一张表**：我们是玻璃模糊半径 /
   描边宽度 / 光晕半径，没有 SVG filter，但 `goo.ts` 的"一张解出来的表 + 一起改"
   结构照样适用。这是 goo 边框不膨胀的可迁移部分。
5. **做粘滞/融合前先定"谁在什么时候独占画面"**：上游"任意时刻只有一层可见、连阴影也是"
   的规矩值得先写进口径，否则半叠状态一定出现双线。
6. **拖拽与点击分开**：若要给岛内加任何可拖元素，先实现 `consumeClick` 那 8 行
   （超过阈值就吞掉随之而来的 click），别让"拖一下"变成"点了"。
7. **五态的第二个非颜色载体**核对一遍（形状/文案/`aria`），把"颜色不是唯一载体"落到具体控件。
8. **曲线纪律**：本仓的 `.spring(response:dampingFraction:)` 与上游"house spring / pop spring
   两条曲线全家族共用"同构；可核对是否已有集中的曲线定义，避免各处手写参数。

## 取证命令

```sh
# 上游 README（本文绝大多数引文出处）与源码
curl -sSL https://raw.githubusercontent.com/arknow91/liquid-taffy/main/README.md -o liquid-taffy.md
git clone --depth 1 https://github.com/arknow91/liquid-taffy.git liquid-taffy-src

# star / license / 时间线（2026-09-26 实测 161 star, MIT, created 2026-08-17, pushed 2026-08-20）
curl -sS https://api.github.com/repos/arknow91/liquid-taffy

# 边框那张表：每个 blur 一对阈值
grep -n -A8 "GOO_RIM_THRESHOLDS" liquid-taffy-src/src/components/liquid/goo.ts

# "同帧切换"那条纪律
grep -n -A6 "ONE setting" liquid-taffy-src/src/components/liquid/goo.ts

# reduce-motion：一处实现，20 个调用点全 import 它
cat   liquid-taffy-src/src/components/liquid/motion.ts
grep -rn "prefersReducedMotion" liquid-taffy-src/src | wc -l

# 弹簧：两条曲线 + "曲线本身不能漂"
grep -n -B4 "HOUSE_SPRING_POINTS" liquid-taffy-src/src/components/liquid/springs.ts

# 关节点三条规则
grep -n -A8 "ONE LOBE PER BODY"  liquid-taffy-src/src/components/liquid/seam.ts
grep -n -A12 "LATCH, not a test" liquid-taffy-src/src/components/liquid/seam.ts
grep -n -B4 -A6 "LOBE_OFFSET"    liquid-taffy-src/src/components/liquid/seam.ts

# 引擎/宿主契约与视觉不能跳的阴影
grep -n -B4 -A6 "The engine decides WHEN" liquid-taffy-src/src/components/liquid/stretch.ts
grep -n -B4 -A8 "shadow's offset"        liquid-taffy-src/src/components/liquid/stretch.ts

# 本仓现状（reduce-motion 的键名是 accessibilityReduceMotion，不是 reducedMotion）
grep -rn "accessibilityReduceMotion" Sources
grep -c "withAnimation" Sources/AgentIsland/IslandView.swift
```

> 版本口径：上游 `pushed_at` 为 2026-08-20，本文引用的 `goo.ts` / `seam.ts` / `stretch.ts` 行号
> 对应 `commit 4bcf400`。上游没有版本 tag，改版后行号会漂。
