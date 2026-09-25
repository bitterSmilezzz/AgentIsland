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

## 三条当前已经跨案例共识的结论

这些是三篇各自独立得出、指向同一处的判断，比任何单篇的细节更值得先读：

1. **克制是默认项。** 3dicon 说 `Do not force an effect`（拿不准就别加）；libraries-dev 说
   等不到 2 秒就不该有 orb；Slingshot Lamp 干脆把整页只留一个发光体。
   → 灵动岛常驻UI，默认应该更保守而不是更热闹。
2. **不对称才有重量。** Slingshot Lamp 的灯丝亮 34 / 暗 16（`lamp.js:621`）、3dicon 的
   energy 分级，本质都是同一个手法：起与落用不同速率。
   → 微细条呼吸灯现在是线性脉动，这是成本最低的一处改进。
3. **状态显示实际现象，不显示控件位置。** Slingshot Lamp 碎了灯泡会写
   `blown (switch on)`——开关还开着但没有光，**它把这件事说出来了，没有伪装成已关闭**。
   → 与本仓「读不到 ≠ 闲着」同一口径，可作外部佐证。

## 新增一篇要做什么

1. 文件命名 `NN-<英文slug>.md`，按加入顺序编号，不要用日期（日期会诱导人按时间读而不是按主题读）
2. 四节写全，尤其第 3 节：没有一手证据的案例不收
3. 若涉及本机抓取，写明走代理等环境事实（本机 `*.twimg.com` 直连不通，需 `127.0.0.1:10808`）
4. 更新本文件的案例表与「共识」两节
5. 若是第三方 skill，同时记一笔到 [AGENTS.md](../../../AGENTS.md) 的 Installed skills
