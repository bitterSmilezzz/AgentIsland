# UI 动效案例库

> 收集外部可复现的动效/交互案例，按 `docs/research/` 的规矩写：**一手核实，正文原句或本机实样 + 取证命令**。
> 没有出处的推断不允许进来；写成推断必须标注出来。

## 这个目录是干什么的

AgentIsland 是常驻 UI，动效的取舍比一次性落地页更重：常驻意味着任何动画每天要看几百次，
也就意味着**不耐看的东西代价翻倍**。这个目录用来在动手之前先看别人怎么做、做到什么程度、代价是什么。

它**不是**待办清单（那是 issue tracker）、**不是**设计规范（那是 `CONTEXT.md` / `docs/adr/`）、
**不是**逐版记录（那是 `CHANGELOG.md`）。每篇只回答：这个案例是什么、技术上强在哪、哪条规则能搬、
搬到我们这里具体落在哪。

## 每篇的结构

固定四节，缺一节说明还没核实完：

1. **技术手法与源码级细节**——带 `file:line` 或确切出处，可复现
2. **可迁移的决策规则**——原作者的原话与判断表，不是我的转述
3. **一手证据 / 本地验证结果**——抽帧、实测数字、对比 demo
4. **应用建议清单**——与实现解耦，尚未排期；要做时开 issue 而不是直接改

## 案例

| # | 案例 | 一句话 | 最该抄的一条 |
|---|---|---|---|
| [01](01-3dicon-looping-3d-icons.md) | [samyost1/3dicon](https://github.com/samyost1/3dicon) | 一句话 prompt 进，循环动画 3D 图标出（带真 alpha） | **先问「物体静止时在干什么」再选策略**；惰性物体走 `event`，别要它自然地动 |
| [02](02-slingshot-lamp-handwritten-pendulum.md) | [kamran.fyi/lamp](https://kamran.fyi/lamp) | 可被弹弓打碎的吊灯，Canvas 2D 手写物理 | **纯黑背景上的光要"挖"出来，不是叠上去**（`destination-out`） |
| [03](03-libraries-dev-where-effects-belong.md) | [Jakubantalik/Libraries.dev](https://github.com/Jakubantalik/Libraries.dev) | 把"动效该加在哪"写成 agent skill | **按等待时长定效果：<2s 什么都不加** |
| [04](04-ui-resource-sites.md) | UI 素材站点集群（beUI / Beautiful UI / BoardUI / ThreeUI / Inspora / CollectUI） | 六个站点各自解决什么、定位差异 | **agent 界面正在形成自己的组件词汇表**（Thinking / Approval Card / Tool Chips / Task Rows） |
| [05](05-morphicons-and-tools.md) | [guillermolg00/morphicons](https://github.com/guillermolg00/morphicons) | 任意两个 stroke 图标互变，旋转是 Procrustes 闭式解出来的 | **先解最优相似变换再插值**；纯度量分不清时加一个眼睛看得见的罚项（`res + λ·|θ|/π`） |
| [06](06-liquid-taffy-goo-engine.md) | [arknow91/liquid-taffy](https://github.com/arknow91/liquid-taffy) | gooey 三件套的参考实现：边框为什么不会膨胀 | **同一个光学参数的两个消费者必须共用同一份来源**（`<use>` / 同一张解出来的表） |
| [07](07-ui-motion-tweet-sample.md) | 一周 19 条 UI 动效推文抽样（8/18–8/24） | 从一张静态封面能证明什么、不能证明什么 | **命名是作者的，特征是画面的**——自称 gooey 的那帧，三次读图都不见一个 metaball 特征 |
| [08](08-farhan-video-dashboard-info-partition.md) | Farhan「Video Editing Dashboard」静态稿 | 一个深色大面板如何同时容纳四类信息而不显得吵 | **进度分三档**：徽标 / 行级 / 汇总，三者回答不同问题，不要用一个组件的三种尺寸糊过去 |
| [09](09-farhan-filenns-refining-details.md) | Farhan「Refining the details」侧边栏 + 用量卡 | 侧边栏主导航与用量卡怎么做才不土 | **彩虹渐变只给唯一的行动召唤**——全图唯一的彩色渐变成为唯一 CTA |
| [10](10-swiftui-craft-invite-card-spring.md) | [@withAnimationUI](https://x.com/withAnimationUI)「craft is still the moat」 | SwiftUI 邀请码胶囊卡片在两态间切换 | **状态切换让尺寸参与**（spring 换位 + 纵向 squash & stretch，而非只改颜色/文字） |
| [11](11-plasma-ui-liquid-glass-panels.md) | [CruxGarden/plasma-ui](https://github.com/CruxGarden/plasma-ui) | 液态玻璃面板：融合、折射、SDF 材质 | **把效果状态做成可读计数器**（`5 panels, 4 joined`），于是熔断-重连能被 0.2s 帧抓住；**且"全关装饰只留透镜"是一个合法配置而不是 fork** |
| [12](12-halogen-recorder-capsule-states.md) | [@sashabirukoff](https://x.com/sashabirukoff)「Halogen case study otw」 | 常驻录屏小部件的四种形态：闭合 / 展开 / 录制单行 / 回落 | **它不是四个控件，是一个控件的四种高度**；不可用的选项让它在场外而不是置灰 |
| [13](13-kopp-morphing-dropdown.md) | [@koppkev](https://x.com/koppkev)「morphing dropdown」 | Details.so Vault 的导航下拉在四种内容形态间反复变形 | **先问「这是几个菜单，还是一个菜单的几个状态」**——四种内容共享同一个容器几何，就只有一份开合逻辑 |

> 编号说明：01–07、11、12、13 由使用者指定链接研究而来；08–10 是研究过程中 subagent 引申发现的
> 同源案例（素材与分析均为一手实样，见各篇「素材文件」与「取证命令」）。
> 分开编号是为了让人一眼看出哪些是指定素材、哪些是沿链发现的。

## 当前已经跨案例共识的结论

这些是各篇独立得出、指向同一处的判断，比任何单篇的细节更值得先读：

1. **克制是默认项。** 3dicon 说 `Do not force an effect`（拿不准就别加）；libraries-dev 说
   等不到 2 秒就不该有 orb；Slingshot Lamp 干脆把整页只留一个发光体；
   withAnimationUI 那条 9.6s 的 SwiftUI demo 在**每个来回之间都完全停稳**，最后还有 1s 纯黑空场。
   → 灵动岛常驻UI，默认应该更保守而不是更热闹。
2. **不对称才有重量。** Slingshot Lamp 的灯丝亮 34 / 暗 16（`lamp.js:621`）、3dicon 的
   energy 分级，本质都是同一个手法：起与落用不同速率。
   → 微细条呼吸灯现在是线性脉动，这是成本最低的一处改进。
3. **状态显示实际现象，不显示控件位置。** Slingshot Lamp 碎了灯泡会写
   `blown (switch on)`——开关还开着但没有光，**它把这件事说出来了，没有伪装成已关闭**。
   → 与本仓「读不到 ≠ 闲着」同一口径，可作外部佐证。
4. **"卡顿感"多数来自没被动画覆盖的属性。** [10](10-swiftui-craft-invite-card-spring.md) 那条 demo 把虚线描边、
   位移与高度压扁全部挂上了同一条弹簧；单改内容而漏掉描边/圆角，就是硬切。
   → 凡是已经在动的效果，先清点它还有哪些属性没被动画覆盖，比再加新动效优先级高。
5. **让图标变形"默认照常播"，但把决定权交出来。** [05](05-morphicons-and-tools.md) 的 morphicons
   把图标 morph 判定为 reduce-motion 下一般可接受的 micro-transition，于是默认播放、
   用 `reducedMotion="user"` 显式 opt-in 尊重系统设置
   → 它给的是**分层判据**：micro-transition（图标形变、勾选）可以照常动，
   large motion（整屏移动、parallax）该塌。本仓不是"全都没接"——
   [06 篇](06-liquid-taffy-goo-engine.md) 第 590 行实测 3 个 View 已接、其余未接——
   所以缺的不是"要不要尊重"，而是**按这个分层把剩下的逐个判**。
6. **同一个光学参数的两个消费者必须共用一份来源。** [06](06-liquid-taffy-goo-engine.md) 的 goo
   轮廓用"blur + 一对阈值"在 filter 里画边框、又在 mask 里画光晕——作者两次都从同一张表取、
   用 `<use>` 引用同一组 blob，注释原话 "a mask one setting behind would let the colour slip off
   the border it is supposed to be painted on"
   → 灵动岛的玻璃模糊 / 描边 / 光晕若是各写各的参数，就是同一类分叉。
7. **一秒的动效，全程都该被同一条曲线覆盖。** [05](05-morphicons-and-tools.md) 的中途飞行用
   2 位小数折线、落地那一帧**吸回规范值**；[06](06-liquid-taffy-goo-engine.md) 连阴影旋转都走
   同一条弹簧回家，注释原话 "zeroing it with a `set()` after the handoff made the shadow
   visibly JUMP"
   → 交接之后用 `set()` 归零 = 把跳变藏在"用户看不见"的假设里。
8. **命名是作者的，特征是画面的。** [07](07-ui-motion-tweet-sample.md) 里自称 gooey toggle 的那一帧，
   三次定向读图都确认"只有一个连续形状、无 blur、无 neck、无 second blob"；
   反过来自称 liquid glass 的那一帧折射形变**确实在场**。引用别人的动效时，
   能引用的只有画面能证的那部分——这一条对本案例库本身就是门禁。
9. **等待态必须带"已经等了多久"。** [04](04-ui-resource-sites.md) 里 Beautiful UI 与 BoardUI
   两家独立做了 elapsed time；[03](03-libraries-dev-where-effects-belong.md) 说 ≥2s 才该给反馈。
   一条规则的两半：**先决定要不要反馈，再决定反馈里显不显时间**。
10. **先把"该不该用我"写清，再谈做好。** [04](04-ui-resource-sites.md) 的三个组件库首页都有
   "Do not reach for me when..."；这与本仓「写清不解决什么比写清解决什么更能防误用」同向，
   也是 README 已知限制该有的写法。
11. **动效时长按"读起来多快"定，不按"名义多长"定。** [04](04-ui-resource-sites.md) 的
   `agent-log` 注释原话：blur-in 跑 0.42s 但前陡，实际约 0.2s 字就落定了，
   照 0.42s 定引导线长度会"visibly trailing text that was already done"
   → 只看 duration 会判错，要看有效曲线。
12. **引用外部事实前，先按正确的关键词 grep。** v0.0.137 时照 Web 侧拼法 `reducedMotion`
   grep，得出"本项目没有 reduce-motion 处理"并写进 README 与 01 篇；正确键名是
   `accessibilityReduceMotion`，实测 3 个文件 6 处已接。**跨语言搬结论时，先确认那个词在目标语境里叫什么。**
13. **彩色只编码状态，选中用明度。** [07](07-ui-motion-tweet-sample.md) 的 Linear 那组里绿/红/蓝紫
   三个颜色**全部**承载语义，而选中态靠"比容器亮一档 + 文字提亮"。
   → **本仓今天没有做到这条**：五态色另有一套语义名（`Theme.statusWorking` / `statusIdle` /
   `statusOffline`，Theme.swift:171–173），但彩色不止五态在用——
   `Theme.sydedockCyan` 被 64 处引用，用在详情页数据单元格（如「工作总耗时」）当数据色，
   `actionBlue` 31 处、`focusBlue` 9 处。所以这条不是"照此宣布已达成"，
   而是一个**待判的问题：数据色要不要让位给状态色**。若让，详情页要另找区分手段（明度/字重）。
14. **胶囊是微型信息容器，不只是圆角按钮。** [07](07-ui-motion-tweet-sample.md) 的
   `Send to agent` 胶囊内用一条 1px 分隔线重新分层（图标 / 文字 / 次级图标）。
   → 灵动岛宽度极窄，只有"一胶囊多层信息"能塞进足够上下文。
15. **把效果的状态做成可读的。** [11](11-plasma-ui-liquid-glass-panels.md) 的 playground 常驻显示
   `N panels, M joined` 与 `24px grid`——那不是给用户的开关，是**让效果可被验证**。
   → 与 `doctor` / 可信度自查同源：把"我看到的"和"实际是什么"分开显示。
16. **"全关装饰"必须是一个合法配置。** [11](11-plasma-ui-liquid-glass-panels.md) 的 `Aqua` preset
   把 rim / highlight / shimmer / glow / wash / grain 六个 sheen 全关，只留折射与色散，
   上游原话 "the plasma is a plain lens"。playground 里直接写着 "Turn all four off... and the
   plasma is a plain lens - the Aqua tab above."
   → 任何材质系统都该能表达"只要功能不要装饰"，否则克制只是口号。
17. **间距要么小于融合距离、要么大于它，别停在中间。** [11](11-plasma-ui-liquid-glass-panels.md)
   上游原话：`Smaller gaps render as liquid bridging them.` 中间态是 bug 不是特性。
   → 微细条与卡片、卡片与货架行之间的距离，将来若做任何光桥/融合效果，先按这个分档定。
18. **视频拿不到，先怀疑自己没走代理。** [07](07-ui-motion-tweet-sample.md) 写"twimg mp4 下载失败，
   拿不到逐帧"——那是**当时没走系统代理**。本机 `127.0.0.1:10808` 能完整下载 13.9MB / 36.2s 视频
   （11 篇的 72 帧逐帧分析就是这么来的）。**证据不足时先审自己的取证路径，再归因于上游。**
19. **同一个常驻控件应该是"一种形态列表"，不是多个控件。** [12](12-halogen-recorder-capsule-states.md)
   的 Halogen 把 idle / 可选 / 录制中 / 收起做成同一条胶囊的四种高度，全程共用同一份点阵图标与同一套描边；
   且录制中把不可用的 `Screenshot` **移出场外而不是置灰**。
   → 这正是本仓 `shell_mode` 该有的实现口径：灵动岛与侧边栏是一个容器的两种高度，不是两套 UI。
20. **塌缩用行淡出，不用整体缩放。** [12](12-halogen-recorder-capsule-states.md) 从三行菜单塌到单行胶囊时，
   多余的行走"图标+文字一起变暗 → 消失"，计时数字**等塌缩完成后才出现**；全程没有 scaleY 压缩。
   → 与 [10](10-swiftui-craft-invite-card-spring.md)「切换让尺寸参与」看着对立，判据其实是
   **按形态层级决定切换强度**：同层内换内容可以让尺寸夸张，跨层增减内容要安静。
21. **「形态列表」已是两个独立样本，可以当收敛结论用。** [12](12-halogen-recorder-capsule-states.md)
   是一个控件的四种高度（内容只增减行）；[13](13-kopp-morphing-dropdown.md) 更进一步：
   同一个面板在**图文四格 / 纯文字列表 / 两栏图文**之间整体换形，容器几何不动。
   → 动手前先问一句：**这是几个控件，还是一个控件的几个状态？** 内容形态共享同一个容器几何时，
   答案永远是后者——四套开合状态、四份定位逻辑、四倍测试面，都是自找的。

## 新增一篇要做什么

1. 文件命名 `NN-<英文slug>.md`，按加入顺序编号，不要用日期（日期会诱导人按时间读而不是按主题读）
2. 四节写全，尤其第 3 节：没有一手证据的案例不收
3. 若涉及本机抓取，写明走代理等环境事实（本机 `*.twimg.com` 直连不通，需 `127.0.0.1:10808`）
4. 更新本文件的案例表与「共识」两节
5. 若是第三方 skill，同时记一笔到 [AGENTS.md](../../../AGENTS.md) 的 Installed skills
6. **写完自查引用**：行号引文回头 grep 一遍（v0.0.137 时写错过 `lamp.js:607`，实际 621）；
   跨语言搬结论前先确认那个词在目标语境里叫什么（v0.0.137 照 Web 侧 `reducedMotion` grep
   SwiftUI 代码，得出过错误结论）。本文库里**每一个版本号、每一个行号、每一句"本项目现状"**都要能指回具体文件
7. **不要把"作者的命名"当"画面能证的事实"**（07 篇的 gooey toggle 教训）：引用前先问这个特征在本帧里真的在吗
8. **未核实到的单列一节**，不掺进事实陈述。04、05 篇都靠这一节保住了可信度
9. 素材（图/视频）放 `.scratch/ui-material/`，**不入库**（`.scratch/` 已在 .gitignore）；
   篇内以 `素材文件：.scratch/ui-material/<name>` 指向它，并在文末说明它是本机实样
10. **从外部素材读到的真实个人信息一律脱敏**，哪怕它本来就公开。09 篇原稿引用了设计稿里
    作者公开的邮箱，被 `scripts/scan-secrets.sh` 拦下——公开不等于该被本仓转载：
    转载会让下一次全文 grep 的人把它当联系方式。原文可到出处核对，不需要搬进来。
    （假凭据、假邮箱可以备案；真实的不行。）
