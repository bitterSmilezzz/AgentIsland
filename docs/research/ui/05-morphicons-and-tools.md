# morphicons：旋转是**解出来的**，不是声明出来的

> 目的：在给灵动岛做任何「两种形态之间变形」的动效之前，先看一个把这件事做到数学闭合的库怎么做。
> [guillermolg00/morphicons](https://github.com/guillermolg00/morphicons)（作者 Guillermo López，
> 站点 [morphicons.com](https://www.morphicons.com/)），MIT，2026-08-01 建库。
> 引文出处：2026-09-26 抓取的站点 HTML（`site-www_morphicons_com.html`，303 KB）与作者随站发布的
> 机器可读文档 `llms.txt` / `llms-full.txt`；star / license 数字取自 GitHub API（同日）。
> Github 描述与 `llms.txt` 的 gzip 口径与站点不一致，本文分别标注出处，不合并成一个数字。

## 一句话结论

**它不是"把 A 的点搬到 B 去"，而是先闭式求出两个图形之间的最优相似变换（2D Procrustes），
再把运动分解到「旋转 / 缩放 / 残余形变」各自的自然空间里插值。**
于是 `arrow-right → arrow-down` 自己转 90°，**没有任何人声明过旋转组**。
附带一个对常驻 UI 很重要的结论：它把图标 morph 判定为 reduce-motion 指引下"一般可接受"的
micro-transition，因此**默认照常播放**，把决定权做成显式 prop——这条判断比它的数学更值得先读。

## 它是什么

一行装、一个组件、零运行时依赖：

```tsx
import { MorphIcon } from "morphicons/react";
import { Menu, X } from "lucide"; // icon data, not components

<button onClick={() => setOpen(o => !o)} aria-expanded={open}>
  <MorphIcon icon={open ? X : Menu} />
</button>
```

站点口号原文：

> Morph any SVG icon into any other. Animate Lucide, Tabler, Heroicons or any stroke icon set.
> **Optimal rotation solved in closed form, spring physics, zero dependencies.**

作者自己给这一节的标题是 **"Six kilobytes of math."**（站点 HTML）。

站点展示的数字（`llms-full.txt` 的 CI size 表口径略有不同，见「一手证据」）：

| 站点标称 | 值 |
|---|---|
| core, gzipped, everything included | **6.5 KB** |
| runtime dependencies | **0** |
| to plan any morph pair | **<1 ms** |
| shared rAF for every icon on screen | **1** |

支持 React / Vue / Svelte / React Native / Astro / vanilla JS；vanilla 版一行：

```ts
const m = createMorph(pathEl, Menu);
m.morphTo(X, "snappy"); // interruptible spring; re-plans mid-flight
```

作者对 API 表面积的总结（站点 HTML）：**"No wrappers, no keys, no from/to pairs, no configuration."**

## 手法一：naive 线性插值为什么一定塌

`llms-full.txt` 把业界默认做法的毛病写得很直白：

> The usual icon morphs either interpolate raw coordinates (shapes shrink and shear in transit)
> or require hand-declaring "rotation groups" per icon pair.

> Raw coordinate lerp **collapses rotations**: points take the chord, the shape shrinks and shears
> in transit.

也就是说：直接对两组点做 lerp，每个点走**弦**而不是走**弧**。`X → 菜单` 这种小图形看起来只是"缩一下"，
但 `arrow-right → arrow-down` 会被走成弦，形状中途塌扁、剪切——**该转 90° 的动作变成了一次错误的形变**。
传统解法是人工声明"这两个图标互为旋转"，于是每加一对图标就要多一条手写规则。

## 手法二：2D Procrustes 闭式解（本篇核心）

`llms-full.txt` §4 原文：

> Before interpolating, the **optimal similarity** (rotation θ, scale σ, translation) mapping A onto B
> is computed by minimizing `Σ|σ·R(θ)·(aᵢ−c_A) − (bᵢ−c_B)|²`. **In 2D it has a closed form, no SVD:**

```
S_xx = Σ aₓbₓ    S_xy = Σ aₓbᵧ    (centered clouds)
S_yx = Σ aᵧbₓ    S_yy = Σ aᵧbᵧ

θ* = atan2(S_xy − S_yx,  S_xx + S_yy)
σ* = [cos θ*·(S_xx+S_yy) + sin θ*·(S_xy−S_yx)] / Σ‖aᵢ‖²
```

**为什么 2D 能闭式**：两个去质心的点云求最优旋转，本质是最大化 `Σ aᵢ·R(θ)bᵢ`。
把 `a` 与 `b` 看成复数，这个和就是一对复共轭乘积之和的实部与虚部，其辐角即最优 θ——
所以 `atan2` 两行就够。3D 没有这个结构（SO(3) 不可交换），才需要 SVD。
（这一段是 agent 的补充说明；上游只写了 "In 2D it has a closed form, no SVD"。）

**关键在紧随其后的那个残差**——它才是让"旋转组"变成 emergent property 的东西：

> The normalized RMS **residual** after alignment (`res = √(Σ|σRa−b|² / Σ‖b‖²)`) is the shape metric:
> ≈ 0 means A and B are the same shape rotated/scaled. This turns rotation groups into an **emergent
> property**: arrow-right → arrow-down gives residual ~0 with θ = 90° and the system chooses pure
> rotation on its own.

于是运行时是一条**分支判断**，不是一张查表：

| residual | 这场 morph 是什么 |
|---|---|
| ≈ 0 | A 与 B 只差一个相似变换 → **纯旋转**（+缩放 +平移） |
| 明显 > 0 | 在**已对齐的坐标系**里做普通形变 |

**对齐还分两级**，这是它比教科书 Procrustes 多走的一步：

- **per subpath**：每个子路径绕**自己的质心**转。作者注释（`llms-full.txt` §4）：
  "this gives local, organic motion: a hamburger *folds* instead of spinning as a block."
- **global hybrid**：整个图标全等（global residual < 5e-3）时，所有子路径共享同一个 (θ, σ)。
  否则每个子路径各自 lerp 自己的质心，会把"弦病"带到上一层：外偏的部件会**朝中心漂**。
  作者给了实测数字：`An arrow's head sagged ~1 px toward its shaft at t = 0.5
  (r·(1 − cos(θ/2)) with r = 3.5, θ = 90°) while both parts rotated perfectly.`

## 手法三：polar interpolation——把每部分插在它自己的空间里

`llms-full.txt` §5 原文（公式照抄）：

```
P(t) = c(t) + σ*ᵗ · R(t·θ*) · [(1−t)·aᶜᵢ + t·b̃ᵢ]

  c(t)  = lerp(c_A, c_B, t)        linear translation (block transport under the global hybrid)
  t·θ*  = linear angle             (θ* is already in (−π, π], short path)
  σ*ᵗ   = exp(t·ln σ*)             log-linear scale (geodesic in ℝ⁺)
```

三条分解各有出处，值得逐条记：

| 分量 | 自然空间 | 为什么不是线性 |
|---|---|---|
| 角度 | 线性扫过 θ* | θ* 已归一到 `(−π, π]`，**天然走最短弧**，不存在绕远路 |
| 缩放 | `exp(t·ln σ*)` |  multiplicative 量的测地线；线性缩放会在视觉上"先猛后僵" |
| 残余 | 对齐后的坐标 lerp | 只有这部分真的是"形变" |

作者列的三个性质，第三个是白捡的：

> - If B is A rotated (residual 0), the bracket is constant and the motion is **pure rotation**.
> - If there is no rotation, it's a clean coordinate morph.
> - Mixed cases do both simultaneously and continuously.
> - With spring overshoot (t > 1) the formula **extrapolates** naturally: rotation and scale
>   overshoot slightly and come back — **free juice**.

**"free juice" 是这篇最该偷的一句话**：过冲不需要为它写第二套逻辑——
把弹簧的 t 直接代进同一个闭式，越界自动外推。

## 手法四：tie-break——数学看不见、眼睛看得见的那一项

这是全篇最细的一处。`llms-full.txt` §4 的 "Minimal-rotation tie-break"：

> For shapes symmetric under inversion (a straight line), both traversal orientations give
> residual 0 — indistinguishable to Procrustes — but produce different rotations. Naive
> tie-breaking can pick θ = 135° when the inverted orientation gives θ = −45° with the same
> end result. Each orientation is scored with:
>
> ```
> score = res + λ·|θ|/π      (λ = 0.05)
> ```
>
> Deformation is minimized first and, at comparable residuals, the shortest rotation wins.
> λ is small enough that a notably worse shape never wins just by rotating less.
> **This is the kind of detail the math doesn't see but the eye does.**

一条线段，正向走与反向走残差都是 0——**纯度量无法区分**，但一个是 135°、一个是 −45°，
眼睛立刻看出后者对。作者用一个 λ=0.05 的小罚项把"最短旋转"排进同一次评分，而不是 if-else。

## 手法五：静止时必须精确，运动中才允许近似

morphicons 把"什么时候可以近似"分得很清，两条都值得抄：

**(a) 采样时把角钉住**（§2）：

> Instead of fighting Bézier structures of different cardinality, every subpath is sampled at
> **N points equidistant by arc length** (N = 64). ... The refinement that drives quality:
> **corner detection** (tangent discontinuity above an angular threshold) with corners anchored as
> exact sample points.
>
> - At rest the shape is **exact** (a check's vertex is a sample, not an approximation).
> - In transit, source corners flatten smoothly and target corners sharpen — the desired behavior.
>   **Without anchoring, a morphing check visibly rounds its corners.**

"对勾在变形中圆角"是这类动效最常见的丑态，作者的解法不是加插值器，是**在采样阶段就把角点设为精确采样点**。

**(b) 落地时吸回规范 `d`**（§6）：

> On settle, the driver **snaps to the canonical `d`** of the target icon: exact fidelity at rest
> (real curves, not polylines), subpath count reset after duplications, and the DOM ends up
> identical to a static icon's. **The jump is < 0.02px — imperceptible.**

中途飞行时每帧只发 `M x y L x y …`（2 位小数），停下来那一帧换成目标图标**本来的** `d`。
即：**运动中可以凑合，静止时必须与一个静态图标逐字节一致。** 对常驻 UI 这条尤其重要——
稳态是每天看几百次的那一帧。

**顺带一个正确性细节**（跟我们测试断言直接相关）：

> The canonical `d` itself is emitted with 4 decimals, and that is a correctness choice, not a size
> one: arc→cubic conversion goes through trig whose last ulp differs across JS engines, and SSR
> hydration compares the server's bytes against the browser's — quantized emission keeps them
> identical, where full precision leaked each engine's ulp into the markup (a Next hydration
> mismatch on icons with non-cardinal arcs).

**浮点输出要量化**，否则同一份代码在两个实现上给出不同字节，比较就成了抛硬币。

## 手法六：中断为什么是免费的

`llms-full.txt` §7：

> **Interruptions**: when a `morphTo` arrives mid-flight, the plan is rebuilt from the current
> intermediate shape (the rendered buffers are already N points per subpath — they serve directly
> as source), `x` resets to 0 and **velocity is preserved** (clamped to ±14). Perceived motion is
> continuous; **tap spam feels alive, never jumps.**

脾气的来源在架构注释里：`plan()` accepts any list of sampled subpaths — **not just canonical
icons** — which is what makes interruptions free: the currently rendered buffers are a valid
morph source.

即"当前帧的中间态"本身就是一次合法输入，所以**不需要为中断保留任何特殊状态**。
弹簧与收尾条件：

```text
ẍ = k·(1 − x) − c·ẋ          # semi-implicit Euler, h = 1/240 s
settle when |1−x| < 0.001 ∧ |v| < 0.02
```

| preset | k | c | ζ = c/(2√k) | character |
|---|---|---|---|---|
| smooth | 170 | 26 | 1.00 | critically damped, no overshoot |
| snappy | 420 | 30 | 0.73 | fast, subtle overshoot |
| bouncy | 300 | 14 | 0.40 | playful |

站点 playground 正是这三个 spring 与 stroke 1 / 1.5 / 2 / 2.5 的组合（HTML 快照可见）。

## 手法七：那条"所有图标库共享 24×24 网格"的观察

站点 playground 的说明文字（HTML 快照原文）：

> Icons from different libraries morph into each other: **they all share the 24×24 grid**.

`llms-full.txt` 把它写成硬约束与逃生门：

> 3. **A shared coordinate space per pair.** Both endpoints of a morph must live on the same grid.
> Lucide, Tabler, Heroicons and Iconoir all draw on 24×24 — that's why cross-library morphs just
> work. For a pack on another canvas (Heroicons *solid* on 20, Carbon on 32, Teenyicons on 15),
> re-grid it once with `fitIcon`:
>
> It returns a plain `d`, so it goes anywhere an icon is accepted. Call it at module scope, not
> per render. **Skipping it doesn't throw** — Procrustes is similarity-invariant, so the grid
> mismatch never reads as false rotation; it lands in σ as an unwanted zoom, and the target draws
> outside the canvas.

**妙处在最后一句**：网格不匹配不会报错、也不会伪装成旋转——它会在 σ 上变成一个不想要的缩放。
这与本仓「读不到 ≠ 闲着，失败要显式」是同一条口径的另一个化身：静默降级比报错更难查。

同类要求还有两条（§Icon library compatibility）：

1. **Stroke-drawn icons.** The geometry must be the stroked centerline (`fill="none"`, color via
   `stroke`). ... filled or outlined-fill glyphs parse fine but **won't read correctly in transit**.
2. **Geometry available as data.** ... No `<g>` wrappers and no `transform` attributes — coordinates
   must be literal. Any other tag throws a clear error.

所以支持列表是 **Heroicons outline / Iconoir / Akar / Untitled UI / Hugeicons**，
而 Material Symbols、Phosphor fill、Heroicons solid 是 **parse 但不保证 transit 正确**。
限制被写清楚，而不是藏在实现里。

## 可迁移的决策规则

1. **先解最优相似变换，再谈插值。** 两个形态之间要变形时，第一件事是"它们是不是同一形状的不同朝向"，
   而不是"给哪些点配哪些点"。判据是一个可计算的残差，不是人的品味。
2. **把每部分插在它自己的自然空间里**：角度走最短弧、缩放走对数线性、残余才做坐标 lerp。
   混在一个空间里插值，动作一定塌。
3. **残差驱动的分支优于分类查表。** 声明式"旋转组"每加一对图标多一条规则；残差 ≈ 0 就纯旋转，
   规则自己长出。
4. **纯度量分不清时，加一个眼睛看得见的罚项。** `score = res + λ·|θ|/π`。不要 if-else 特判。
5. **越界是白捡的。** 弹簧过冲 t > 1 直接代进闭式，让旋转与缩放一起外推。
6. **运动中可以近似，静止时必须精确。** 中途用折线 2 位小数，落地那帧吸回规范 `d`。
7. **采样阶段就把结构钉住**（角点 = 精确采样点），不要指望插值器救圆角。
8. **中断的免费来自"当前帧即合法输入"**。想清楚哪份中间状态可以被当成下一次调用的原料，
   特殊状态就归零了。
9. **不支持的条件要抛错，别静默降级。** 网格不匹配落到"不想要的缩放"上——上游自己都说清了代价，
   但没有藏。

## 对本项目的具体关联

- **本项目已有 `.spring` 且可打断**：`IslandView.swift:238` / `IslandView.swift:430` /
  `IslandComponents.swift:367` 用的是 `withAnimation(.spring(response:dampingFraction:))`。
  SwiftUI 的 spring 由状态驱动、天然可中断，这与 morphicons 的"速度保留、重规划"同向；
  可核对的是它**没有显式的收尾阈值与速度钳制**（上游 settle 在 `|1−x|<0.001 ∧ |v|<0.02`、
  速度钳在 ±14），也没有"落地那帧吸回规范值"这一步。前者影响长尾抖动，后者影响稳态像素。
- **reduce-motion 的判断值得单独记**：上游把图标 morph 划为 "small, short, communicative
  micro-transitions: the kind of motion the reduce-motion guidance considers generally acceptable"，
  因此 **默认播放**，并给出 `reducedMotion: "never" | "user" | "always"` 三态显式策略。
  对照本仓 README「已知限制」那条 reduce-motion 条目（v0.0.138 已按代码重写）——真实情况更细：
  `Sources/AgentIsland/DockedSliver.swift:110`、`AgentRingView.swift:12/145/189`、
  `ActivityMatrixDots.swift:13` 都读了 `@Environment(\.accessibilityReduceMotion)`，
  而 `IslandView.swift` / `IslandComponents.swift` / `EventBannerView.swift` 一处都没读。
  **README 那条限制在 v0.0.138 已按代码重写**：它先前写的是"所有动效都不看
  `prefers-reduced-motion`"，而这句话本身是按 `reducedMotion` 这个名字 grep 出来的——
  SwiftUI 的键名是 `accessibilityReduceMotion`。按正确键名核实后才得到上面那张表。
  → 上游给的分层判据正好是继续补的依据：**micro-transition（图标形变、勾选）可以照常动；
  parallax / 整屏移动这类 large motion 该塌。**
- **浮点断言的量化**：上游为 SSR 字节一致把 `d` 量化到 4 位小数，理由是"不同实现的最后一个 ulp
  会漏进输出"。本仓自建测试 runner 里任何浮点/几何断言都应有显式容差，否则跨机器漂移。

## 应用建议清单（与实现解耦，尚未排期）

1. **给"两态之间变形"立一条数学前置**：动手前先问这两个态是不是同一形状的不同朝向
   （residual ≈ 0 → 纯变换；否则 → 形变）。`DockedSliver` 收起⇄展开 目前是整体 spring，
   没做这个区分。
2. **曲线过冲优先考虑"同一公式外推"**，而不是为过冲单独写一段关键帧。
3. **稳态帧给一次"吸回规范值"**：所有动画结束后把几何/颜色夹到设定值，替代"最后一帧看着差不多"。
4. **测试断言显式量化**：浮点比较给 epsilon，别直接 `==`。
5. **README「已知限制」的 reduce-motion 条目按代码重写**：区分"已接开关的三个 View"与
   "完全没接的岛体/横幅/组件"，并把 micro-transition / large-motion 的分层判据写进口径。
   （本篇只做记录，不改 README。）
6. **图标规格收敛**：若将来做图标级形变动效，先确认参与变形的图形是否同网格——
   不同网格就先归一化，别让不匹配变成一次不想要的缩放。

## 取证命令

```sh
# 站点 HTML（本文站点侧引文出处，2026-09-26 抓取，直连可用）
curl -sSL https://www.morphicons.com/ -o site-www_morphicons_com.html

# 作者随站发布的机器可读文档（本文全部数学引文出处）
curl -sSL https://www.morphicons.com/llms.txt      -o morphicons-llms.txt
curl -sSL https://www.morphicons.com/llms-full.txt -o morphicons-full.txt

# 站点口号 / 24×24 网格那句 / "Six kilobytes of math."
grep -n "Optimal rotation solved in closed form" site-www_morphicons_com.html
grep -n "they all share the 24×24 grid"        site-www_morphicons_com.html
grep -n "Six kilobytes of math"                 site-www_morphicons_com.html

# Procrustes 闭式解 / polar interpolation / tie-break / 中断
grep -n "closed form, no SVD"  morphicons-full.txt
grep -n "collapses rotations"  morphicons-full.txt
grep -n "λ·|θ|/π"              morphicons-full.txt
grep -n "velocity is preserved" morphicons-full.txt

# star / license / 时间线（2026-09-26 实测 2679 star）
curl -sS https://api.github.com/repos/guillermolg00/morphicons
```

> 版本口径提示：站点 JSON-LD 自称 `softwareVersion: 1.4.0`，而 `llms-full.txt` 的
> Reduced motion 一节说 "since 1.4.2 they play by default"——两份抓取时间相同但版本不一致，
> 说明站点的结构化数据没跟着更新。引用版本相关结论时以 `llms-full.txt` 为准。
