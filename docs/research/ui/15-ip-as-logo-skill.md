# IP as Logo skill：把"做过"的吉祥物规格写成可执行指令

> 目的：[s1dashu/ip-as-logo-skill](https://github.com/s1dashu/ip-as-logo-skill)（MIT；star 数随时间变动，
> 我方复核 2026-09-26 为 5556 / 273 fork，原文采集时 5555，两者都对，别把 star 当定值引用）
> 是一份**单文件 Agent Skill**（`SKILL.md` 17157 字节，仓库总共只有 `SKILL.md` + `README.md` + `LICENSE` + 一张壁纸资产，
> 没有任何脚本、样式表或生成依赖）。它教 agent 用顶级图像模型生成"极简可爱的公司吉祥物方图"。
> 本仓过去的研究对象都是**界面怎么动**；这一篇是**视觉资产怎么被规格化**——它把"为什么
> 这个 logo 看着像 logo"变成了一条条可执行约束，其中若干条与本仓的五态色、图标资产、
> `32×32` 可读性是同一类问题的答案。
>
> 一手素材：`/tmp/ip-as-logo-skill/`（浅克隆，含 `SKILL.md` / `README.md` / `LICENSE` / `assets/ip-as-logo-wall.webp`）。
> **核实日期 2026-09-26**，方法：GitHub API 取仓库元数据 + `git clone --depth 1` 本地通读全文 + `ffprobe` 量壁纸。
> 未核实到的内容集中在文末「没能核实的」，没有写成事实。

## 一句话结论

**这份 skill 的价值不在"怎么生成图片"，在它把"好看"拆成了可判定的硬约束：4–7 个大形状、
恰好三色、32×32 可读、85–95% 占幅、从下角探出。** 更重要的是它的两条**元约束**——
生成提示词里**绝不出现 logo / brand mark / app icon 这些词**，以及**每次生成只画一次，
不做合规审查、不自动重试、不后处理修补**。前者是一条关于"模型会照你说的做"的实证经验，
后者是一条关于"别把创作当验收测试"的纪律。这两条对本仓同样成立。

---

## 1. 技术手法与源码级细节

### 1.1 它到底约束了什么（全部照抄自 `SKILL.md`）

| 维度 | 约束（原文数字与措辞） | 出处行 |
|---|---|---|
| 复杂度 | 一条主轮廓由约 **`4–7`** 个大基本几何形构成；不承载识别/表情的形一律合并或删除 | `SKILL.md:36` |
| 识别特征 | **至多一个**物种定义特征（一个大喙 / 一对弯角 / 一个宽面罩） | `SKILL.md:37` |
| 内部色区 | 至多两块大色区；脸只许**两只眼**，表情需要时才加**一张小嘴**；眉、高光、鼻孔、纹理、描边、装饰一律不要 | `SKILL.md:38` |
| 小尺寸 | **必须有一版可读的黑色剪影，且 32×32 可辨**；某特征在该尺寸消失或变噪声 → 放大、合并或删除 | `SKILL.md:41` |
| 形与轮廓 | 粗、圆、有分量；**禁止**尖角、尖耳/尖喙、针状尾、细天线、细笑嘴、窄缝、尖火焰/羽尖；每个必须的尖端换成** visibly 钝的圆头** | `SKILL.md:45-46` |
| 成对特征 | 耳、角、翼、鳃、铃这类**成对特征必须两个都在画面内** | `SKILL.md:47,51` |
| 构图 | 角色**直立**、从指定的**左下或右下**探出，占画幅约 **85–95%**；**除用户明确要求，绝不居中或底中** | `SKILL.md:48,50` |
| 颜色 | 整图恰好**三个语义色**：两个 IP 基色 + 一个背景色；面部标记复用这两个 IP 色，不引入第三种语义色 | `SKILL.md:65,71` |
| 背景 | 无用户调色板时**把背景饱和度略微压低**，标准是"仍然明确有色、干净、有意图，而不是鲜艳/灰/脏"；**禁止**出现 `opaque` / `alpha` / `transparency` 这类图像模式词 | `SKILL.md:68,72` |
| 画布 | 直接出 **`1:1`** 直角方图；请求约 `1536×1536`，服务上限是 `1254×1254` 时**照收不重采样** | `SKILL.md:73` |
| 材质 | 只许一句"几乎察觉不到的新拟物纵深"，**不许展开成数值强度、渐变/高光/阴影指令** | `SKILL.md:60` |
| 风格 | 保持图形化简单，**不要**黏土 / 充气 / 塑料 / 毛绒 / 玩具感 / 写实渲染 | `SKILL.md:61` |
| 分层配色 | 两个 IP 色各是一个**语义族**，族内的 incidental tonal variation 不算违规；同一批候选之间要**刻意变化双色策略**，不要每次都来同一套"中性色为主" | `SKILL.md:70-71` |

### 1.2 编译流程：三方向 + 六张 + 左右均分

它的工作流是一条明确的漏斗（`SKILL.md:10-33`）：

1. 解析请求；**除非用户明确要求控制颜色，否则不问颜色模式**
2. 用户没指定主体且当前 workspace 是产品仓库 → **先只读一遍上下文**（README、产品文档、
   package/app 元数据、落地页文案、manifest、design token）；能推断出产品用途/受众/气质就算够
   （`SKILL.md:13`）
3. 上下文不够 → **只问一轮**背景题（做什么、服务谁、该什么感觉），**明确禁止第二轮问卷**
   （`SKILL.md:14`："Do not start a second background questionnaire"）
4. 上下文够了 → **必须先给三个方向**、明确提议一次生成六张；用户没说"直接出六张"就不生成
5. 三个方向的定法：
   - 用户已指定主体 → 主体不变，从**剪影处理 / 次色区 / 定义特征 / 性格侧重**出三种设计处理
     （`SKILL.md:20`）
   - 用户未指定 → 三个**真正不同**的主体或隐喻，**每个绑一个产品属性或品牌承诺**，
     "不要给三个没有理由的随机动物"（`SKILL.md:20`）
6. 六张的左右分配是钉死的：全盘接受时 `A1 B1 C1` 走左下、`A2 B2 C2` 走右下（**保证三左三右**）；
   只选一个方向时 `A1..A6` 奇偶分左右；批量数为偶数时均分，为奇数时**把多的那张刻意放到一侧、
   并把这个不平衡记下来**（`SKILL.md:19,21,23`）
7. 给每张候选一张**独立的整幅方图**，"绝不要让图像模型去拼 contact sheet / grid / 多图 sheet"，
   且测 prompt-only 可复现性时**不拿前一批候选当图像参考**（`SKILL.md:28`）
   ——若支持 subagent，六张按可用并发并行，不够就分波次，每个 subagent 拿同一份产品简报与
   同一套共享约束、只领一个方向或一个变体（`SKILL.md:29`）

方向的行文格式也规定死了：`<IP 主体> — <产品关联> — <定义剪影>` 一行说完，
"除非用户要求，不要把发现期变成一场 branding workshop"（`SKILL.md:32`）。

### 1.3 元约束一：提示词里不许出现用途词

这是全文最反直觉、也最该被记住的一条：

> Describe the requested visual as an image only. Never tell the image generator that the image is a
> `logo`, `brand mark`, `app icon`, `icon asset`, or intended for any of those uses. Do not prepend
> use-case or asset-type scaffolding that reveals such a use. This rule applies only to the
> generation prompt; the surrounding user conversation and Skill name may still describe the broader
> （`SKILL.md:81-82`）

注意它划线的那半句：**这条只管生成提示词**；对用户说话、skill 自己的名字，照常说 "logo" 没问题。
同理 `opaque` / `alpha` / `transparency` 也不许进提示词——理由是这些图像模式词会**把模型的注意力
从想要的视觉结果上引开**（`SKILL.md:72`）。它不要 negative prompt payload（对现代指令遵循模型），
只保留一行自然语言 `Constraints:`；只有在旧模型真有 `negative_prompt` 参数时，才把最小排除项
改走那个参数，**并且从主提示词里删掉 `Constraints:` 行，避免同一件事说两遍**（`SKILL.md:83-85`）。

### 1.4 元约束二：生成是抽签，不是验收测试

`SKILL.md:108-114` 的 Delivery behavior 整节都在说同一件事：

- "Treat generation as a stochastic draw, not a conformance test."
- 每张候选**只生成一次**，交付**原样返回的每一张**
- **不**检查或报告 alpha / 透明度 / 背景模式
- **不**拦交付、**不**给候选打合规/不合规、**不**标记推荐或不推荐、**不**因背景/配色/细节/构图/
  渐变/明暗/纵深自动重试
- **不**后处理让结果"显得更合规"；用户另要方向或替换时，**按那个明确请求再画一张新的**

也就是说：它把所有"这张图对不对"的判断从 generate 阶段完全拿掉，只留"画之前把规格写清"。
README 里对应的一句是 "The skill does not inspect transparency, block outputs, classify candidates
as compliant or non-compliant, or automatically retry results"。

### 1.5 上游示例 prompt 骨架的完整形状

`SKILL.md:96-105` 给了一段可直接抄的整段 prompt，**按 `Background:` / `Subject:` / `Complexity:` /
`Color behavior:` / `Composition:` / `Style:` / `Finish:` / `Constraints:` 分八段**，每段都是一个可判定的祈使句
（如 `use exactly three semantic colors in the complete image`）。它的 legacy negative payload 是：
`text, watermark, borders, frames, cards, presentation masks, extra subjects, scenery, thin fragile
lines, sharp tips, photorealistic materials, strong three-dimensional rendering, external cast
shadows`（`SKILL.md:88-92`）。**注意 `presentation masks` 这个词**——它专门防模型把图塞进一张
带遮罩的展示卡里。它还要把模型/供应商、检测到的约束投递方式（`main-prompt constraints` 或
`dedicated negative parameter`）与确切的约束文本**记进生成报告**（`SKILL.md:86`）。

---

## 2. 可迁移的决策规则

以下每条都是上游原话或判断，不是我的转述：

1. **"好看"要能被拆成可判定项，否则 agent 无从下手也无从复盘。** 4–7 个形状、恰好三色、
   32×32 可读、85–95% 占幅、左下或右下——五个数字就把"极简可爱"从形容词变成了 checklist。
2. **先定"该问什么"，再定"怎么问"。** 不问颜色模式；主体未指定时先**只读**产品上下文；
   不够才问，且**只问一轮**（`SKILL.md:13`："Do not start a second background questionnaire"）。
3. 模型会照你说的字面做，所以用途词不进提示词。**它的原话是规矩，不是效果承诺**：
   说它是 logo，它就给你一张"像 logo 的图"而不是"这张图"；`opaque/alpha/transparency` 会把它往
   图像模式的岔路上带。这条在本机**没有跑过任何生成实验**——它是上游的实证结论，本文照搬其规则、未验证其效果。
4. **同一件事不要在两条通道里各说一遍。** 有专用 negative 参数就删掉主提示词里的 `Constraints:` 行
   （`SKILL.md:83-85`）。
5. **每个候选必须是独立整图，永远不要多图拼版。** 拼一次就得裁六次，且裁出来的是同一张构图。
   它还额外要求：**测 prompt-only 可复现性时，不许拿前一批候选当图像参考**（`SKILL.md:28`）。
6. **生成是抽签。** 拦交付、打合规标签、自动重试、后处理修补，四件事一件都不做（`SKILL.md:110-114`）。
7. **为不可见的尺寸设计。** 32×32 是硬门槛：特征在那个尺寸是噪声，就该被合并掉（`SKILL.md:41`）。
8. **背景要"有色但不抢"。** 无调色板时略微降饱和，标准是"clearly chromatic and intentional
   rather than vivid, gray, or muddy"（`SKILL.md:68`）——这条比"降低饱和度"这种说法可执行得多。
9. **颜色是语义族，不是色值清单。** 两个 IP 色各是一个 family，族内的 incidental tonal
   variation 不算违规（`SKILL.md:71`）；同一批候选之间要**刻意变化双色策略**，
   不要每次都来同一套"中性色为主"（`SKILL.md:70`）。
10. **用户给的调色板只绑背景。** 用户给了背景色，就把它全留给背景，IP 两色另选；
    背景与 IP 之间**必须保持清晰分隔**，分隔不够时**先调 IP 色，不换用户指定的背景**
    （`SKILL.md:67,69`）。
11. **成对特征必须成对在场。** 只画一只耳朵在极简里看着像 bug。
12. **生产者要自带"模型要什么"的知识。** 它要求先探测可用图像模型与其真实 tool schema，
    "Do not guess a model or invent unsupported parameters"（`SKILL.md:79`）；
    没有顶级模型时**要用户开工具或给 key，绝不悄悄降级到 SVG**，也不许编造生成结果
    （`SKILL.md:25` 的 "Never fall back to SVG generation" 与 "Do not fabricate generated results"）。
13. **交付前要留痕。** 每一个标签、方向与理由、分配的角落、保存路径、prompt/配色映射、
    尺寸都要报（`SKILL.md:30`）；连用的是哪个模型、约束走了哪条通道、确切约束文本也要记
    （`SKILL.md:86`）。**报不全，用户就无法复核一次抽签抽到了什么。**

---

## 3. 一手证据 / 本地验证结果

### 3.1 仓库元数据（GitHub API，2026-09-26）

```sh
curl -s https://api.github.com/repos/s1dashu/ip-as-logo-skill | python3 -c \
  'import json,sys; d=json.load(sys.stdin); print(d["full_name"], d["stargazers_count"], d["forks_count"], d["license"]["spdx_id"], d["created_at"], d["pushed_at"])'
#   → s1dashu/ip-as-logo-skill 5555 273 MIT 2026-08-18T13:59:23Z 2026-08-22T16:48:59Z
```

- topics：`codex`、`codex-skill`、`image-generation`、`logo-design`、`mascot-design`
- `created_at` 2026-08-18 → `pushed_at` 2026-08-22：**四天完成主体后停更**，
  30 commits。`homepage` 指向 https://ipaslogo.com （README 自称 "a searchable library backed
  by Cloudflare R2 and Supabase"）。
- 文件清单（本地实样 `find`，与 `git ls-tree -r --name-only main` 一致）：

  ```sh
  cd /tmp/ip-as-logo-skill && find . -type f -not -path './.git/*' | sort
  #   → ./.gitignore  ./LICENSE  ./README.md  ./SKILL.md  ./assets/ip-as-logo-wall.webp
  wc -c SKILL.md README.md
  #   → 17157 SKILL.md    7963 README.md
  ```

  **四个文件、一个资产、零脚本零依赖**——`README.md` 的 Repository structure 一节也这么说：
  "The skill itself intentionally consists of a single instruction document. The repository also
  includes the showcase image above, but no scripts, style references, or generation dependencies."

### 3.2 唯一资产是什么

```sh
cd /tmp/ip-as-logo-skill && file assets/ip-as-logo-wall.webp
#   → RIFF (little-endian) data, Web/P image, VP8 encoding, 2560x2200, YUV color
```

2560×2200 的 WebP 展示墙，就是 README 顶部那张 `ip-as-logo-wall.webp`。
**它是展示墙，不是 skill 的规则依赖**（4 号仓库结构小节原话）。本文不做逐张视觉判读——
素材在本机 `/tmp/ip-as-logo-skill/assets/`（不入库，`/tmp` 会清）。

### 3.3 上游对 license 与分发形态的自述

- `LICENSE` 首行：`MIT License` / `Copyright (c) 2026 s1dashu`（本地实样 `head -3 LICENSE`）
- README 安装命令：`npx skills@latest add s1dashu/ip-as-logo-skill`，`--global` 供跨项目个人安装
- README 自列兼容 agent：**Codex, Coze, Doubao, YouMind, Manus, Gemini Apps, Replit Agent**；
  并声明"the agent must have a top-tier image model: preferably GPT Image 2, or Seedance 5.0 Pro,
  Nano Banana Pro (Gemini Image Pro), or Nano Banana 2 (Gemini Image Flash)"；
  "The skill never falls back to SVG; another image model may be used only with explicit user consent"
- 没有 codex 的 `.codex-plugin` / plugin manifest，也没有 GitHub Pages / wiki 内容——
  **它是纯 Agent Skills 格式的裸分发**（与本仓 `.agents/skills/` 同一格式）

### 3.4 本仓现状：拿它当镜子照的三处实测

这三条是对着我们代码读出来的，用来判断上面哪些规则对我们今天已经成立、哪些没有：

| 对我们的问题 | 本仓现状（实样） | 结论 |
|---|---|---|
| 小尺寸可读有没有被当硬门槛 | [scripts/make-icon.swift](../../../scripts/make-icon.swift) 画的是"深色圆角方块 + 居中拉长黑胶囊 + 左绿点 + 右白色文字条"，十档尺寸从 16px 起全部用**同一个 `drawIcon(size:)` 等比重绘**（`make-icon.swift:9-20,22,68-73`） | **做法与上游一致**（唯一源、各尺寸等比重画、不用缩放），但我们的尺寸表止于 1024，**没有 32×32 与 16×16 的剪影判定**——上游 `SKILL.md:41` 那条"消失在 32×32 就合并掉"，我们没有对应的自查 |
| 图标是不是"语义色恰好三色" | 图标全部色值写在 `make-icon.swift:32-35`（背景渐变 `#29292b→#121214`）、`:42`（纯黑胶囊）、`:46`（白 10% 描边）、`:53`（`#30d158` 绿点）、`:62`（白 75% 文字条）。**数一下：两个灰背景 + 黑 + 白 + 白10% + 绿 = 5~6 个色值**，且背景是渐变不是纯色 | **不满足"恰好三色"**。这不构成 bug（我们的图标是拟物玻璃风，不是极简吉祥物），但说明**上游的三色口径与 macOS App Icon 惯例不可兼得**——要搬就得明确放弃哪一头 |
| 颜色是否按语义分层 | [CONTEXT.md](../../../CONTEXT.md) 已有 `Theme.statusWorking/statusIdle/statusOffline` 等五态色（见案例库 README 共识 13） | 上层颜色纪律**已有口径**；上游补的是"**资产侧**（一张静态图）也该有色彩预算"这一层，本仓今天没有对应文档 |

---

## 4. 应用建议清单（与实现解耦，尚未排期）

1. **做任何应用图标 / 资产前，先写"复杂度预算"再画。** 上游那五个数字（4–7 形 / 三色 /
   32×32 / 85–95% / 下角）可以直接改造 Asset 生成脚本的自查项。**不要**现在就改 `make-icon.swift`——
   它服务的是"拟物玻璃"定位，与上游的"极简吉祥物"是两种审美，硬凑会两头不搭。
2. **给 `make-icon.swift` 补一张 16×16 / 32×32 的剪影判定**：在今天缩到最小尺寸时，
   那颗绿点与白色文字条是否还分得开？上游的判据是"某特征在该尺寸变噪声就合并它"。
   这是一次纯自查，不动产物。
3. **给静态资产的生成提示词立一条规矩：用途词不进 prompt。** 本仓若将来用图像模型生成
   图标、站点配图、`site/` 的截图占位，这条直接可用，且它是**免费的正确性**
   （不说 logo 反而得到更像成品的图）。同理 `opaque/alpha/transparency` 不进提示词。
4. **"生成即交付，不做合规审查"值得写进本仓自己的图像生成约定。** 它与本仓已有的一条纪律同向：
   读不到 ≠ 零（[CONTEXT.md](../../../CONTEXT.md)「Token 数据覆盖」）——**不要用后处理把
   "我看到的"伪装成"实际是什么"**。
5. **调研本机到底有没有顶级图像模型，再谈要不要真用它。** 上游自己就要求先探测模型与 tool schema、
   没有就让用户开工具或给 key、**并且明确不降级到 SVG**。本仓若哪天要生成资产，
   第一步是核实能力，不是先写 prompt。
6. **上游的 ipaslogo.com 可以当免费素材源看一眼**（README 自称每个 logo 免费商用）。
   但注意：**它的 logo 版权与许可条款本文没有核实过**，要用先自己读一遍它的许可页，
   别只凭 README 一句 "free for commercial use" 就用进交付物。

---

## 没能核实的

| # | 事项 | 为什么没核实 |
|---|---|---|
| 1 | `assets/ip-as-logo-wall.webp` 里每张示例图是否真的满足上游自己那套约束（三色 / 4–7 形 / 下角构图） | 本文**没有逐张读图**，只做了 `file` 与尺寸判定。上游 prompt 与产物之间的符合率是未知的 |
| 2 | ipaslogo.com 的 logo 到底在什么许可下可商用、要不要署名 | README 只有一句 "Every logo is free for commercial use"，**没有许可正文**；本文未抓该站 |
| 3 | 上游那份 prompt 骨架在 GPT Image 2 / Nano Banana Pro 上的实际成功率 | 上游没有给出任何统计；README 只说"模型是随机的，可能对单条约束有不同解释" |
| 4 | 本机是否装有可用的顶级图像模型或对应工具 | 未探测。上游把它当硬前置（`SKILL.md:56`），本仓要跟进此事之前必须先补这一项 |

> 与案例库纪律的关系：本文引用的每一条上游规则都带 `SKILL.md` 行号，可在
> `/tmp/ip-as-logo-skill/SKILL.md` 逐条对上；本仓现状三行均指到具体文件与行号。
> 上面四条没有一条被当成事实使用。
