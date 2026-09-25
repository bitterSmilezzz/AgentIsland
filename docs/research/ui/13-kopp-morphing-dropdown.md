# Morphing Dropdown：同一个面板容器在四种内容之间变形

> 目的：在给灵动岛做任何"展开面板"之前，看一个把 morphing 做到极简的样本怎么做。
> Kopp（[@koppkev](https://x.com/koppkev)，瑞士，"Obsessed with the details"，在 build
> [@details_so](https://x.com/details_so)）2026-09-25 发的 "morphing dropdown ✨ available in the vault."，
> 846 赞 / 39,659 浏览 / **975 收藏**（收藏数高于点赞数——这是"想照着做"的信号，比点赞更值钱）。
> 素材：9.17 秒视频，1152×720@60fps（原始 1920×1200），**已完整下载并逐帧分析**。
> 分析时间 2026-09-26。

## 一句话结论

**它不是四个下拉菜单，是一个下拉面板在四种完全不同的内容之间反复变形**——
四种导航项、四种内容形态（图片四格 / 洲际列表 / 两栏图文 / 四格图文）、四种高度，
**但容器始终是同一个**，导航项的下划线指示器跟着内容一起换。
这是本案例库继 [12 篇](12-halogen-recorder-capsule-states.md)（一个控件的四种高度）之后
第二个"形态列表而非多个控件"的独立样本，而它比 12 篇更激进：**内容形态也换了。**

## 它是什么

视频录的是 [details.so](https://www.details.so/inspo) 的 **Vault** 页面（推文里说的 "the vault"）：
顶部一条深色工具条（`Vault` / `MCP ▸NEW` 标签、搜索框显示 `Navigation Dropdown`、
`Get Code` 按钮、`Invite & earn`），下面是被演示的组件本身——一张全屏山景 hero +
悬浮在上的导航栏 `Explore ▾ / Experiences ▾ / Destinations ▾ / About / Contact`，
hero 中央有 `( scroll down )`。

**Details.so 本身值得记一笔**（站点描述原文）：

> Curated web design inspiration from real websites: hero sections, footers, preloaders,
> page transitions and animations — updated weekly.

Vault 页的自我描述更直接（meta description 原文）：

> Copy-paste animations and interaction code, crafted by hand for production.
> Page transitions, text reveals, scroll effects — **the details AI can't replicate.**

它的定位是把动效做成**可 copy 的手写片段**，站内导航是 `Inspo / Vault / MCP NEW`。
（本篇只分析了推文视频，**没有注册账号，Vault 里的源码未获取**——见文末。）

## 一手证据：四种内容逐个核对

视频 9.17 秒内无硬切（`ffmpeg select='gt(scene,0.02)'` 零命中），是一段连续操作。
按内容把四种形态与出现时刻对起来（全部 OCR 逐字确认）：

| 时刻 | 展开的导航项 | 面板内容（OCR 原文） | 形态 |
|---|---|---|---|
| ~1.0s | **Explore** | `Guided Tours` / `Private Expeditions` + `Expert-led journeys` / `Adventures tailored to your interests` | **两栏图文**（每条：小标题 + 描述） |
| ~1.5s | **Experiences** | `North America` / `South America` / `Africa` / `Asia Pacific` | **纯文字列表**（四项，无图无描述） |
| ~2.5s | **Experiences** | `Guided Tours` / `Private Expeditions` + 两段描述 | 两栏图文（回到 Experiences 自己的内容） |
| ~3.5–4.5s | **Explore** | `Canyons` / `Forests` / `Mountains` / `Coastlines`，各配一句 `Dramatic sandstone landscapes shaped by time.` / `Ancient woodlands filled with wildlife and trails.` / `From alpine peaks to highland adventures.` / `Rugged shores, cliffs, and endless horizons.` | **四格图文**（配风景图，读取到的左侧首字母残缺为 `yons`/`ests`/`untains`/`istlines`，是图片压字所致） |
| ~5.0–6.2s | **Experiences** | 两栏图文 + `Asia Pacific` 混现 | 过渡 |
| ~6.5s | — | `North America` / `South America` / `Africa` / `Asia Pacific` | 纯文字列表 |
| ~7.0s | — | 只剩 `Tours` / `Private Expeditions` 残影 | **过渡中**（内容被裁/正在换） |
| ~7.5–8.5s | **Explore** | 四格图文完整重现 | 四格图文 |
| ~8.8s | 全部收起 | 面板消失，hero 与导航恢复初始态 | **收起** |

### 关键判定一：四种导航项 → 至少四种内容，且互不相同

- `Explore` 至少有两种面貌：**两栏图文**（Guided Tours / Private Expeditions）
  与**四格图文**（Canyons / Forests / Mountains / Coastlines）
- `Experiences` 也有两种：**纯文字洲际列表**（North America / South America / Africa / Asia Pacific）
  与**两栏图文**
- 也就是同一个 `Explore` 触发词在不同时刻给出不同内容，同一个 `Experiences` 也是
  → **这不是"每个导航项一个菜单"，是"一个面板按状态换内容"**。

### 关键判定二：面板外框始终是同一个

各帧里 hero 上浮出的面板**位置、宽度、圆角看起来一致**，变的只有高度与内容。
（这条是目视判定——视频没有可测的边框刻度，未做像素测量，标为推断。）

### 关键判定三：导航项的下划线/高亮跟着内容走

`Explore` 展开时该项被标记（OCR 在该项后读出 `^` 残影，如 f3.5 / f4 / d3.8 的 `Explore^`），
`Experiences` 展开时同样（f5.5 / f6 / d6.2）。指示器与面板内容同步切换。

## 可迁移的决策规则

1. **先问"这是几个菜单还是一个菜单的几个状态"。** 四种导航项、四种内容形态，
   如果做成四个独立下拉，就有四套开合状态、四份定位逻辑、四倍测试面；
   做成一个面板换内容，则只有一份。**判据是内容形态是否共享同一个容器几何。**
2. **纯文字列表与图文网格是同一面板的两种内容密度。** 洲际列表四项无图，
   四格图文带图带一句描述。用同一个容器承载两种密度，比给"轻内容"另做一个简版菜单更省。
3. **导航指示器必须跟内容同步。** 面板换了而高亮没换，用户会以为菜单没响应——
   这是这类 morph 最容易出的 bug，且无法靠截图发现。
4. **收起态要真的回到初始态。** d8.8 帧：面板消失、hero 与导航完全恢复首帧构图，
   没有残留半透明层、没有残留高度。**常驻 UI 上"看起来关了其实没关"是负债。**

## 应用建议清单（与实现解耦，尚未排期）

1. **灵动岛展开态可参考"一个面板换内容"而不是"每态一个视图"。** 五态若都做展开面板，
   按本篇应共享容器几何与转场，只换内容层。这与 [12 篇](12-halogen-recorder-capsule-states.md)
   "一个控件的四种高度"是同一取向的第二个独立佐证。
2. **Agent 列表/详情/会话三级导航可考虑 morph 而非替换。** README 载明现在是"列表 / 详情 / 会话"
   三级导航；若级与级之间是硬切换，可评估共用容器 + 内容淡换。**未核实现状实现方式，做前先读
   `IslandView.swift` / `IslandPanelRouting.swift`。**
3. **轻内容不要另做简版。** 例如只有一个 agent 在跑时，不需要一个"单行简版面板"，
   同一个面板内容少一点即可。
4. **指示器同步要有一条测试。** 面板内容与导航高亮必须同源；这类 bug 视觉上极难截到。

## 与已收录案例的关系

| 篇 | 关系 |
|---|---|
| [12 Halogen](12-halogen-recorder-capsule-states.md) | **同一取向的第二个独立样本**：那条讲一个控件的四种高度、内容只是增减行；这条讲同一个面板换完全不同形态的内容（图文 ↔ 纯列表）。合起来说明"形态列表"不是个人风格而是收敛中的范式 |
| [10 withAnimationUI](10-swiftui-craft-invite-card-spring.md) | 那条讲切换让尺寸参与（squash & stretch）；本篇视频里高度变化是可见的但**未见夸张过冲**，二者可按形态层级取舍 |
| [06 liquid-taffy](06-liquid-taffy-goo-engine.md) | 那条的 morphing dropdown 是"按钮本身变成面板"（圆形拉伸、mass 被吸入、splat 收尾）；本条是"面板内容换形、容器不动"。**两种 morph 层级不同，不要混引** |
| [05 morphicons](05-morphicons-and-tools.md) | 无关（那条是图形插值的数学，本条是内容层转场） |

## 没能核实的

- **Vault 里的源码没拿到**：需要账号（站点有 `Login` / `Join for free` / `Pricing` / `Upgrade`），
  本篇**没有注册**。所以"同一个容器"是从画面判定的，不是从代码确认的
- **高度变化的具体数值与曲线**：未做像素测量，也未逐帧比对高度变化时序
- **是否用了 spring 及其参数**：视频无源码可核，未推测
- **面板外框几何一致**：目视判定（标为推断），视频没有可测边界
- **内容为何在同一导航项下变化**（如 Explore 为何一时给两栏一时给四格）：
  **没有读到触发条件**，可能是依次演示不同预设、可能是 hover 不同区域，未确认
- 站点 Vault 页可见的 snippet 标题经抓取只有 8 条且**不含** morphing dropdown
  （Parallax Images / Reveal Navigation 2.0 / Circular Scroll Gallery / Morphing Carousel /
  Reveal Navigation / Stacked Scroll Panel 3 / ASCII Cursor / Parallax Carousel）——
  说明该 snippet 在需登录的部分或未列入首屏
- 视频里左上角深色工具条的 `Navigation Dropdown` 是 Vault 的**分类筛选框**（当前筛选中），
  不是被演示组件的一部分

## 取证命令

```sh
# 推文元数据（fxtwitter，无需代理）
curl -sS 'https://api.fxtwitter.com/koppkev/status/2103378630797595109'

# 视频（走系统代理）
scutil --proxy
curl -x http://127.0.0.1:10808 -o clip.mp4 \
  'https://video.twimg.com/ext_tw_video/2103378586799394816/pu/vid/avc1/1152x720/jMCAN23R2vK8LEfw.mp4?tag=12'

# 元数据（1152x720 / 60fps / 9.216s）
ffprobe -v error -show_entries format=duration:stream=width,height,r_frame_rate -of default=nw=1 clip.mp4

# 确认无硬切（一段连续操作）
ffmpeg -i clip.mp4 -vf "select='gt(scene,0.02)'" -vsync vfr -f null - 2>&1 | grep pts_time

# 抽帧（本文时刻表即由这些帧的 OCR 得到）
for t in 0.5 1 1.5 2 2.5 3 3.5 4 4.5 5 5.5 6 6.5 7 7.5 8 8.5; do
  ffmpeg -ss $t -i clip.mp4 -frames:v 1 -q:v 4 -y "f$t.jpg"
  dim ocr recognize "f$t.jpg" --json
done

# 过渡帧（0.2s 左右加密）
for t in 1.2 1.8 2.2 2.8 3.8 6.2 8.8; do
  ffmpeg -ss $t -i clip.mp4 -frames:v 1 -q:v 4 -y "d$t.jpg"
done

# Details.so 两个页面（Inspo 与 Vault）
curl -sSL -x http://127.0.0.1:10808 'https://www.details.so/inspo' -o inspo.html
curl -sSL -x http://127.0.0.1:10808 'https://www.details.so/vault' -o vault.html
python3 -c "
import re
s=open('vault.html',encoding='utf-8',errors='ignore').read()
print(re.findall(r'\"title\":\"([^\"]{3,90})\"',s))"
```

> 环境事实：本机 `*.twimg.com` / `pbs.twimg.com` 直连超时，必须走系统代理
> `127.0.0.1:10808`（`scutil --proxy` 可见）。视频 mp4 走该代理可完整下载。
> `details.so` 除 collectui 外均需代理（本站直连亦不通，实测）。
