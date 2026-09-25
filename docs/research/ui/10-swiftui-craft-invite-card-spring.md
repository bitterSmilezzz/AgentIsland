# 「craft is still the moat」：一条 SwiftUI 邀请函动效（Spring + 匹配几何）

> 目的：账号 [withAnimationUI](https://x.com/withAnimationUI)（[@withAnimationUI](https://x.com/withAnimationUI)，
> 站 withanimation.com，自称 "learn SwiftUI from absolute zero and build delightful interactions"）
> 发的 SwiftUI 动效 demo。演示总长 9.6 秒、960×960、58fps，内容是一个
> "邀请码 / 邀请就绪" 胶囊卡片在两态之间的反复切换。
> 推文正文只有一句 **"craft is still the moat"**——本文件的价值恰恰在于把那句话
> 落回到可核对的细节上：这条 demo 到底在哪几处做了工。

素材文件：`.scratch/ui-material/04-withanimation-craft.mp4`（不入库，`artifacts/` 与 `.scratch/` 均在 .gitignore）。

## 一句话结论

**同一个控件在两个数据态之间切换，靠的不是"淡入淡出"，而是三条同时开的弹簧：
卡片整体 spring 换位、卡片高度 squash & stretch、内部虚线描边本身也是一条 spring。**
它把一个会让人烦的"状态切换"做得值得看第二遍，靠的是**节奏的开与合**，不是加特效。

## 它是什么

一个浅灰背景上的白色胶囊卡片，同一张卡承载两个数据态：

| 元素 | 态 A（就绪） | 态 B（密码） |
|---|---|---|
| 左侧徽标 | 实心黄圆 + 白色对勾 | 无（或隔离的图标位） |
| 主文案 | `Your Invite Is Ready` | `LoveSwiftUI`（黑体粗字） |
| 副标签 | 无 | 上方小字 `Your Invite Code`（灰） |
| 右侧按钮 | 无 | 黄底 `COPY` |
| 卡片描边 | 实线 | **虚线** |

整段 demo 是这两态之间的来回切换（约每 2–3 秒一次，共 4–5 个来回），
**每个来回都让卡片经过一段横向位移 + 高度压缩回弹**。卡片下方还有一条细长圆角
"进度杆"（浅灰底 + 黑色填充），作为演示用的驱动控件。
最后两个来回之间有一段约 1 秒的纯黑空场，随后进入下一个 cycle。

## 手法一：卡片整体是 spring 换位，不是线性移动

从 58fps 逐帧核对（构造命令见文末），卡片在两个左右位置之间移动时：

- 位移**不是匀速**：前段快、接近目标时减速并有一次小过冲再回位——典型的
  阻尼弹簧特征，而不是 `linear` 或 `easeInOut` 的对称加减速。
- 位移过程中**圆角半径没有跟着变**——卡片是"平移"，不是"缩放"，所以
  圆角在整段移动里保持视觉一致的曲率。

这条对 SwiftUI 而言几乎是免费的（`.animation(.spring(...), value:)`），
但它演示的关键是**换位和换态用的是同一条时间线**：卡片一边走一边改里面的内容，
新旧内容之间不是 cross-fade，而是由卡片自己的位置变化"带"过去。

> 对我们有直接意义：灵动岛的 `working → completed` 状态变化现在是即时切换。
> 这条 demo 给的做法是**让位置/尺寸参与状态切换**，而不是只让颜色或文字变。

## 手法二：squash & stretch 压缩与回弹

对卡片上半部分裁剪放大后逐帧比对（10 帧 / 约 0.07s 间隔），高度变化序列是：

| 帧 | 高度变化 |
|---|---|
| 1–3 | 基准高度（卡片稳定） |
| 4–6 | **明显变矮、更扁平** |
| 7 | 开始回升 |
| 8–9 | 基本回到基准 |
| 10 | **略高于基准**（回弹拉伸） |

即一次完整的 **压缩 → 恢复 → 轻微过冲**。这是经典动画十二条里的 squash & stretch，
在卡片控件上的正确用法：**压缩发生在"即将离开"的一瞬，回弹发生在"已经落位"之后**。
注意它压的是**纵向**（卡片是横向胶囊，纵向压扁视觉上像"被按了一下"），
而不是各向同等比缩——后者会让卡片整体变小，看起来像浏览器缩放。

实现上这对应 SwiftUI 的 `matchedGeometryEffect` + 一个额外的 scale 弹簧，
或者同一个 `.spring` 上叠加 `scaleEffect(y:)` 的关键帧。

## 手法三：虚线描边本身也是会动的

态 B 时卡片边框是**虚线**。逐帧看，虚线在位移动画开始/结束时
长度与间隔不是突变——它在切换中平滑过渡（demo 中表现为
从实线/无边框到虚线的渐变，而非硬切）。

SwiftUI 里 `StrokeStyle(dash: [a, b])` 默认是可动画的，但**很多手写实现会漏掉**：
只给容器加 `.animation` 而忘记 `Shape` 的 `dash` 不在隐式动画范围内，
结果是"边框闪一下变成虚线"。

**这条是"craft"最便宜也最难被注意到的一处**：用户不一定说得出哪里不同，
但硬切的那版会让人觉得"卡"。

## 手法四：节奏——切换之间有停顿，不是连续动

4–5 个来回之间都有明显静止帧（卡片完全停稳、内容静止），
且最后有一段纯黑空场（约 1 秒）再进入下一轮。

这是**为什么这条 demo 不烦人**的核心：所有夸张手法都被停顿框住。
对照本案例库已收录的三篇（3dicon 的 `Do not force an effect`、
libraries-dev 的 "<2s 什么都不加"、Slingshot Lamp 只留一个发光体），
这是第四条独立指向同一结论的证据：**动效的耐看程度由静止帧决定，不由运动帧决定。**

## 一手证据：抽帧与可复现命令

视频本身在推文里（`https://x.com/withAnimationUI/status/2103462558816727430/video/1`），
本次抓取与核对全部在本机完成：

```sh
# 1. 正文与媒体地址（fxtwitter 镜像 API；x.com 直连在本机超时）
curl -sS 'https://api.fxtwitter.com/withAnimationUI/status/2103462558816727430'
#   → text: "craft is still the moat ⌒   ➜°"
#   → url:  https://video.twimg.com/amplify_video/2103461987971977216/vid/avc1/960x960/b6XZG6iRIK6_1FmD.mp4?tag=29

# 2. 视频本体（video.twimg.com 必须走代理）
scutil --proxy
curl -x http://127.0.0.1:10808 -o 04-withanimation-craft.mp4 '<上面那条 url>'

# 3. 基本信息
ffprobe -v error -show_entries stream=codec_name,width,height,r_frame_rate,nb_frames \
        -show_entries format=duration -of default=noprint_wrappers=1 04-withanimation-craft.mp4
#   → h264 / 960x960 / 58fps / 550 帧 / 9.638s

# 4. 整段时间轴量化
for t in 0 0.5 1 1.5 2 2.5 3 3.5 4 4.5 5 5.5 6 6.5 7 7.5 8 8.5 9 9.5; do
  ffmpeg -ss $t -i 04-withanimation-craft.mp4 -frames:v 1 -vf scale=480:480 -y "h_${t}.png"
done

# 5. squash & stretch 逐帧（卡片上半区域裁剪 → 放大比对高度）
ffmpeg -i 04-withanimation-craft.mp4 -vf \
  "select='eq(n\,96)+eq(n\,100)+eq(n\,104)+eq(n\,108)+eq(n\,112)+eq(n\,116)+eq(n\,120)+eq(n\,124)+eq(n\,128)+eq(n\,132)',\
   crop=500:200:230:150,scale=400:160,tile=3x4" -frames:v 1 squash2.png -y
```

逐帧读到的量化事实：

| 项 | 实测值 |
|---|---|
| 分辨率 / 帧率 / 时长 | 960×960 / 58fps / 9.64s（550 帧） |
| 状态切换循环 | 4–5 个来回，每来回约 2–3s |
| 压缩持续帧数 | 约 3 帧（≈0.07s）明显压扁，随后 3–4 帧回弹 |
| 过冲 | 末帧高度略高于基准（约 +2%~5%，目视） |
| 静止段 | 每个来回之间有完全静止帧；末段约 1s 纯黑空场 |
| 视频体积 | 767 KB（h264，9.6s——可见画面变化量很小，这也是"克制"的间接证据） |

> 注：本文对"虚线在切换中平滑过渡"与"位移是阻尼弹簧过冲"两处是**目视判定**，
> 已用抽帧比对支撑但未做像素级时序测量；SwiftUI 具体的
> `stiffness/damping` 参数在推文中未给出（只发了视频，没有源码），
> 本文未推测具体数值。

## 可迁移的决策规则

1. **状态切换让尺寸参与，不要只让颜色/文字变。** 卡片在两个态之间既有位移又有高度变化，
   于是"换了内容"这件事被身体记住了。
2. **squash & stretch 只压一个轴，且压的方向要符合视觉隐喻。** 横向胶囊压纵向，
   像"被按一下"；等比缩小像"浏览器缩放"。
3. **压缩在离开的一瞬，回弹在落位之后。** 两条各被一个停顿框住，
   否则两个弹簧叠一起只会显得乱。
4. **写 `.animation` 时清点一遍"哪些属性其实没被动画覆盖"**：`dash`、
   `cornerRadius`、阴影偏移这类容易漏。漏掉的属性是"卡顿感"最常见的来源。
5. **动效的耐看程度由静止帧决定。** demo 每次切换之间都完全停稳，还有纯黑空场。
6. **一条 9.6s 的演示视频只花了 767 KB。** 画面变化越少，编码后越小——
   这可以反过来当自检：如果一个常驻动效塞满了每帧都在变的像素，它八成太吵了。

## 应用建议清单（与实现解耦，尚未排期）

1. **`working → completed` 让卡片尺寸参与**：现在是状态图标 + 文案切换。
   可以让微细条或状态胶囊在完成瞬间做一次纵向 squash（约 3 帧压扁 + 回弹），
   极低成本，把"这件事完成了"变成身体记忆。注意竖屏/横屏形态分别验。
2. **清点灵动岛现有动效里未被动画覆盖的属性**：据
   [01](01-3dicon-looping-3d-icons.md)–[03](03-libraries-dev-where-effects-belong.md)
   的口径，应该做的是"少加"，不是"多加"；但凡是已经在动的，
   要确认它没有硬切。具体清单未核实，做前先读代码。
3. **壳的展开/收起可考虑 spring 而不是现在的曲线**：若现状是 `easeInOut` 一类，
   可评估换成阻尼弹簧并检查过冲量（过冲超过 5% 在常驻 UI 上会显得廉价）。
4. **不动效的自检**：常驻动画如果录 10s 视频体积与一条静态 UI 无异，
   说明它其实没怎么动——这是好事，不是缺陷。

## 与前三篇的关系

| 篇 | 它给的东西 | 本篇给的东西 |
|---|---|---|
| [01 3dicon](01-3dicon-looping-3d-icons.md) | 先问"物体静止时在干什么" | 静止帧是耐看的前提（同一结论第四例） |
| [02 Slingshot Lamp](02-slingshot-lamp-handwritten-pendulum.md) | 不对称过渡速率 | 压缩/回弹也是不对称：去得快、回来慢 |
| [03 libraries-dev](03-libraries-dev-where-effects-belong.md) | 按等待时长定效果 | 换位 + 换态用同一条时间线，而不是两个动画叠 |
