# Omarchy：DHH 的 agentic Linux 发行版（43140 star）—— agents panel 与静默通知的另一半答案

> 目的：[omacom/omarchy](https://github.com/omacom/omarchy)（GitHub API 2026-09-26：43140 star /
> 5016 fork / **Shell** / MIT / 创建于 2025-06-01 / 4555 open issues / homepage omarchy.org）。
> 自我描述 "Beautiful, fun & **agentic** Linux distribution by DHH"。
> 它不是一个库或一个竞品，是**一套完整的桌面环境配置**（Hyprland + Quickshell）。
> 与我们的关系：**它的 `agents panel` 是我们产品的 Linux 侧对应物**，它的静默通知设计
> 解释了我们的 `attention` 策略为什么该那么写。readme 只有 1KB，实质全在 `manual/` 下 34 篇文档。
> 本文读了 `manual/17-ai.md`、`13-toggles-idle-screensaver.md`、`10-notices.md`、
> `09-reminders.md`、`05-the-top-bar.md` 五篇原文。
> **注意默认 branch 是 `quattro` 而不是 `main`**（`main` 上抓 `manual/*.md` 全部 404）。

## 一句话结论

**它把 13 家 coding agent CLI 做成一等公民：mise 懒加载 stub、`omarchy default agent` 挑默认、
顶栏一个 `agents` 图标（第一次发现 AI coding usage 才出现）、点开是 agents panel
（套餐、5 小时/每周限额已用百分比、按天按模型的 token）——这套东西在 Linux 桌面上
就是我们正在 macOS 上做的那件事。** 除 panel 之外，另有三条设计可直接搬：
静默通知**进历史而不是丢弃**、indicator **inactive 隐藏 / hover 淡入 / 点亮可反选**、
toggles 全部是"一个 flag 文件 + 一个热键 + 一个命令"同一开关。

## 它是什么

一个**开箱即用的发行版配置**：Hyprland 合成器 + 一个叫 Omarchy shell 的长驻 Quickshell 进程
（画顶栏、菜单、通知、OSD 弹窗、锁屏），外加 34 篇 manual 与大量 `omarchy <verb>` 命令。
`manual/05-the-top-bar.md` 原话：

> The strip along the top of your screen is the Omarchy bar. It's not a bolted-on status bar but part of
> the Omarchy shell, the single long-running Quickshell process that also draws the menu, the
> notifications, the OSD popups, and the lock screen. That's why it themes perfectly with everything
> else and why **a panel opens instantly instead of spawning a new app**.

所有 widget **几乎都绑了左/右/中键与滚轮**（manual 里的 Clicking around 表），
且每个 panel 都有自己的热键，"这样你永远不用去瞄一个 16 像素的 glyph"。

## 与我们最相关的一节：agents panel

`manual/17-ai.md` 原文：

> The top bar grows an agents icon **the first time Omarchy finds AI coding usage on the machine**
> (and stays out of the way until then). The panel behind it tracks every subscription in one place:
> **your plan, the percentage used of the 5-hour session and weekly limits (or the remaining prepaid
> balance), and token usage by day and by model.** Claude Code, Codex, and Fireworks are covered
> out of the box.

它预装了 13 家 agent 的 **mise 懒加载 stub**（`claude` / `codex` / `opencode` / `agy` / `copilot` /
`crush` / `grok` / `pi` / `omp` / `ori` / `hermes` / `muse` / `cursor-agent`）——
"The launchers are tiny mise-managed stubs in `~/.local/bin/`, so nothing is downloaded until the
first time you actually run one."

### 三条可直接搬的判据

1. **agents 图标按需出现**：没发现 AI coding usage 之前顶栏上根本没有它。
   → 对照我们：五态里的 `offline`（进程不在）今天是怎么显示的？**"没有 agent"这件事本身
   该不该有一个常驻图标**，Omarchy 给的答案是不该。这与我们"未接入明细源要说出来"并不冲突——
   前者是**图标该不该存在**，后者是**存在时必须说清它看得见什么**。
2. **token 按天 + 按模型双维度展示**，且**限额百分比与剩余预付余额并列**（两个口径都给）。
   → 我们的分析页已有按模型拆分与 24h/累计口径（README 有载），
   `TokenAnalyticsView.swift:418` 那个 `percent` 是**与上一周期比的增减百分比**，
   不是"配额已用百分之几"——两者回答的不是同一个问题。后者对"这个月还能用多少"更直接，今天没有。
3. **默认 agent 用 `omarchy default agent <name>`，未选时启动走 picker**。
   → 与我们的多 agent 列表同构；它多做一件事：`omarchy agent prompt "Review this project"`
   可带任务启动，且明说"以各家 don't-stop-to-ask 的模式跑，准备好它们真的会动手"。

## 第二条：静默通知——我们 `attention` 策略缺的那一半

`manual/13-toggles-idle-screensaver.md` 的 Do not disturb 段，原文：

> `Super + Ctrl + ,` silences notifications. No toasts pop up while it's on, and the crossed-out bell
> indicator sits in the bar **to remind you why the desktop has gone quiet**.
>
> **Nothing is lost, though. A silenced notification is written straight into your notification
> history**, which is exactly the record you want when you come back and wonder what you missed.

且只放行两类：Omarchy 自己的操作回执（"Theme changed"）与命令行发来的 critical alerts；
"把一切都标成 critical 来强行插队的聊天应用不算"。

**这对我们的价值极高**，因为它回答了一个我们README 只写了一半的问题。我们的远程外发策略分级
与岛内提醒分级都在，但**"静默期间的事件去哪了"这件事没有对应设计**。Omarchy 的答案是：
静默≠丢弃，进历史；且**要有一个可见的记号提醒你"现在是静默的"**（划线铃铛）。
→ 可搬：给任何"关掉提醒"的状态配一个常驻记号 + 一份可回看的历史。
   **已核实（2026-09-26，`grep -rn -i '勿扰|muteAll|silenceAll' Sources/`）：
   本仓今天没有任何勿扰/静默开关**——只有两处 `AgentCleaner.swift:164` / `TopCommand.swift:204`
   说"静默漏掉报警"的注释，与通知无关。所以这不是"补一半"，是整块缺失。

## 第三条：indicators 的可见性逻辑——与我们微细条呼吸灯同题

同一个文件的 indicators 段，原文：

> **Inactive indicators are hidden.** Hover the area around them and they fade in dimmed, so you can
> click one to turn it on **without knowing its hotkey**. Clicking an active one turns it back off.
> If you'd rather see all of them all the time, set `alwaysShow` to `true`.

indicator 集合：dictation、screen recording、pending reminders、night light、do not disturb、stay awake。
**"未激活的隐藏"这个默认值，与案例库里 14 篇动效案例收敛出的"克制是默认项"是同一件事**，
而且它给了一个我们没考虑过的中间档：**hover 淡入（可发现性）而非常驻（不打扰）**。
→ 微细条上的状态点若要做多颗，"默认隐藏 + hover 淡入"比"全常亮"更符合案例库口径；
  代价是多一个 hover 区域与一份 `alwaysShow` 设置。**未核实现状。**

## 第四条：toggles 是临时模式，不是设置

原文把这件事说得很准：

> A lot of what you change day to day isn't really a setting. It's **a mode you flip on for an hour
> and off again**: night light while you're working late, do not distribute while you're presenting,
> stay awake while you're watching something.

实现也统一得干净：**每个 toggle 就是一个 flag 文件**（`~/.local/state/omarchy/toggles/`），
热键、菜单项、`omarchy toggle <thing>` 三个入口打同一个开关；
且 flag **按"关态"命名**（`screensaver-off`、`bar-off`），存在即代表该功能关闭。
另外给了一个脚本接口 `omarchy-toggle-enabled <name>` 返回退出码，
"这样你不用自己去找那个文件"。

→ 对照我们：`shell_mode` 今天**一行实现都没有**（`grep -rn 'shell_mode|shellMode' Sources/ app/` = 0，
只活在 CONTEXT 与 CHANGELOG 里）。[10 号方案](10-replan-2026-09-26.md) 已把它列为 Phase 1 要落地的东西，
Omarchy 给了具体形态：一个 flag 文件 + 三个入口 + 一个可脚本查询的状态。
**它其实不是设置，是模式**——按 Omarchy 的判据，它该有"一个 flag + 三个入口 + 一个可脚本查询的状态"。
这条与 [10 号方案](10-replan-2026-09-26.md) 里「`shell_mode` 从文档名词变成可验证的设计约束」
是同一个方向，Omarchy 给了具体形态。

## 一条与 AGENTS.md 直接相关的：skill 的分发方式

`manual/17-ai.md` 的 Omarchy Skill 段，原文：

> It's **symlinked into the skill directories for Claude Code (`~/.claude/skills`), Codex
> (`~/.codex/skills`), Pi (`~/.pi/agent/skills`), Antigravity (`~/.gemini/config/skills`), Hermes
> (`~/.hermes/skills` and each `~/.hermes/profiles/*/skills`), and the generic `~/.agents/skills`
> location, so most harnesses pick it up automatically.**

它做了和我们 `npx skills` 一样的事（软链一份源到多家 CLI 的 skills 目录），
但**列出的目录清单比我们 AGENTS.md 里记的完整**（我们只记了 `.agents/` 与 `.claude/` 两条）。
对我们的直接用处：**将来若要支持更多 harness，这份目录映射是现成的一张表**。
它还诚实标注了这个 skill 是 experimental，并建议先用 plan mode、准备好
`omarchy reinstall configs` 回滚——**"给 agent 一个能改自己的 skill"这件事它把风险讲在了明处**。

## 应用建议清单（与实现解耦，尚未排期）

1. **静默期间的事件要被记下来**，且静默状态要有常驻记号。这是本轮最该做的一条，
   因为它补的是产品语义缺口，不是加功能。**先核实我们的勿扰/关提醒今天的行为。**
2. **`shell_mode` 按"模式"而非"设置"实现**：一个 flag 文件 + 三个入口（设置页 / 快捷键 /
   可脚本查询的状态命令）。Omarchy 的 flag 按关态命名也值得照做（存在即关）。
3. **多状态点的可见性**评估"inactive 隐藏 + hover 淡入"，并准备一个 `alwaysShow` 等价设置。
4. **限额百分比视图**：分析页若只有绝对值，补一个"已用 x% / 剩 y"的口径（与剩余预付余额并列）。
5. **agents 图标按需出现**：确认我们在"一台机器没有任何 agent"时的首屏不是一堆空壳。
6. **harness skills 目录映射表**补进 AGENTS.md（它的六条比我们现有两条全）。

## 没能核实的

- **只读了 manual 五篇 + README**，没读它的任何 shell 脚本实现（`omarchy` CLI 的源码未看）。
  "面板秒开""theme 同步到 agent"这些是文档说法，未验证
- **agents panel 的具体数据来源未核实**：它怎么读到 Claude Code / Codex 的 5 小时与每周限额？
  （我们是从会话日志反推，它可能有 provider API）**这是本条最值得追的问题**，
  若它有直接读配额的路径，我们的口径可能要改
- Hyprland/Quickshell 是 Linux/Wayland 专有，**与我们 macOS SwiftUI 无技术可复用性**——
  本文全部可迁移项都是设计判据，不是代码
- 4555 open issues 对一个 43k star 项目偏高，**未判断这是活跃度还是规模常态**
- `omarchy weather` 靠 IP 定位、`Super+Ctrl+Alt+T/W/B` 那批 notices，**与本项目无关，未展开**
- manual 里 34 篇只读了 5 篇；`06-themes.md`、`07-hotkeys.md`、`30-updates.md` 未读

## 取证命令

```sh
# ⚠️ 默认 branch 是 quattro，不是 main（main 上 manual/*.md 全 404）
curl -sSL https://api.github.com/repos/omacom/omarchy | python3 -c "import sys,json;print(json.load(sys.stdin)['default_branch'])"

# 本文引文出处
for f in 17-ai 13-toggles-idle-screensaver 10-notices 09-reminders 05-the-top-bar; do
  curl -sSL "https://raw.githubusercontent.com/omacom/omarchy/quattro/manual/$f.md" -o "$f.md"
done

# 元数据（43140 star / Shell / 4555 open issues）
curl -sSL https://api.github.com/repos/omacom/omarchy | python3 -c "
import sys,json; d=json.load(sys.stdin)
print({k:d.get(k) for k in ['stargazers_count','forks_count','language','open_issues_count']})"

# 本机直连不通时走系统代理
scutil --proxy   # 127.0.0.1:10808
```
