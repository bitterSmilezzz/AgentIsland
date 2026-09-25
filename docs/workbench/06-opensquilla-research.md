# OpenSquilla — 调研与可借鉴的经验

> 一手核实记录。来源：GitHub [TokenRhythm/opensquilla](https://github.com/TokenRhythm/opensquilla)
> （1,654 commits、573 forks）、[opensquilla.cn](https://opensquilla.cn/) 与其 `docs/` 目录下的
> 文档（`diagnostics-and-replay.md` 126 行、`approvals-and-permissions.md` 169 行，均为全文抓取）。
> 本文只记「它的公开文档里有哪些经验值得一个跨 Agent 管理工具借」，不记版本历史。

## 0. 定位澄清（重要）

**我们不做 Agent，做的是跨 Agent 的管理工具。** 所以 OpenSquilla 的价值不在它的 agent 实现，
而在它作为一个已经跑了一阵的开源项目，**在文档里把哪些坑写明白了**。
本文记的是"经验"，不是"功能"。

它的自我定义：*"OpenSquilla is TokenRhythm's open-source microkernel AI Agent framework,
built for Token efficiency."*（开源微内核 AI Agent 框架）

## 1. 值得抄的四条经验

### 1.1 Diagnostics / Replay：把"刚才发生了什么"变成可回放的决策日志

它把诊断做成了 gateway 上的显式开关，而不是常开日志：

```
opensquilla diagnostics status [--json]
opensquilla diagnostics on [--raw]      # --raw 抓原始 turn-call，仅在维护者索要证据时开
opensquilla diagnostics off
opensquilla replay --session <key> --turn <id>
```

`replay` **只读、不重跑工具**，把某轮执行打成人类可读的 transcript。
管道步骤显示 `OK` / `SKIPPED` / `FAIL`。

**对我们最有价值的一条**——它处理了"后来隐私过滤把异常文本删掉"的情况：

> *"Their outcome is retained even when privacy filtering removes exception text."*
> （即使隐私过滤移除了异常文本，步骤的成败状态仍然保留。）

而且它给了第三种状态：**旧日志信息不足、无法区分"失败"与"跳过"时显示 `UNKNOWN`**，
而不是硬塞一个 `FAIL`。

**这正是本项目 `isHung` 三态（卡死/不卡死/本轮判不出）与可观测性五类结论在解决的问题**，
OpenSquilla 用同一思路处理了"证据被脱敏后"的退化：**结论可以退化，但退化本身要说出来**。

另外它明确给了**安全共享清单**：分享 diagnostics / replay / 导出 session 之前，
必须移除 provider key 与 bearer token、私有本地路径、私有 channel 标识、
客户/项目/账户名、含机密内容的原始 prompt 与工具输出。
并且警告：*"Avoid leaving raw diagnostics on longer than needed."*

### 1.2 Approvals & Permissions：四档权限 + 工作区围栏 + 远端 Guest 降权

四档权限，且给了选择原则——**"prefer the narrowest profile that can complete the task"**
（用能完成任务的最窄档）：

| Profile | 语义 |
| :--- | :--- |
| `restricted` / `off` | 保持保守，不做提权执行 |
| `on` | 允许宿主执行，但审批仍然有效 |
| `bypass` | 信任任务到自动授予审批，**但仍保留敏感路径检查** |
| `full` | 完全信任，慎用 |

配套工作区围栏三件套：`--workspace` + `--workspace-strict`（禁越界）、
`--workspace-lockdown` + `--scratch-dir`（写只进工作区/暂存区）。
无人在场的自动化明确推荐 `--workspace-lockdown`。

**最值得借鉴的一条：远端 Guest 的对称降权。**

> 无 token / 畸形 token / 错误 token 的远端 Web 连接，拿到的是**同一套 Guest Safe 权限**。
> 它能读普通宿主文件，**不能读内建凭据路径，也不能读 authority 数据**，
> 只能写在服务端指定的默认工作区里（服务端选，客户端不能替换或另建）。
> 边界对 file / Shell / Python / Node / Git Bash / 子进程**一致生效**。
> Guest **不进全局审批队列**——所以一个待审批项不会把 Guest 变成 owner、
> 不会授予宿主级写权限、也不会影响别的 session。
> 需要审批的高危动作在调用方认证之前一律保持阻止。

三点可直接搬：
1. **认证失败与未认证给同一套权限**（不给"差一点的凭据"留半开的门）
2. **Guest 不进审批队列**（待审批项不能成为提权通道）
3. **边界在所有执行面一致**（不是只在文件工具上挡）

这与本项目 `LocalEventServer` 的已知短板直接相关——`/notify` 无鉴权，
且 `NWParameters` 不设 `requiredLocalEndpoint` 时实际监听通配（局域网可伪造事件）。
见 `Sources/AgentIsland/LocalEventServer.swift:10-15`。

### 1.3 Safe / Full Access 两种产品模式 + 兼容垫片

> *"The supported product modes are **Safe** and **Full Access**. Older CLI mode names remain
> accepted only as an upgrade compatibility shim and are not shown in the current UI."*

（受支持的产品模式只有 Safe 与 Full Access。旧 CLI 模式名仍被接受，但仅作为升级兼容垫片，
不在当前界面显示。）

**这条对我们直接适用**：`shell_mode` 即将引入 `island` / `sidebar` 两种形态。
它给出的做法是——**对外只呈现当前这一代模式名，旧名字降级为后台兼容，不许出现在 UI 里**。
避免界面上同时挂着三代叫法，用户无从判断哪个有效。

### 1.4 明确的"分享前脱敏"清单

见 1.1 末尾。它的价值在于把脱敏从"凭自觉"变成**发布物前的固定步骤**。
本项目已有 `scripts/scan-secrets.sh` 与 `docs/agent/desensitization.md` 做同一件事，
可借鉴的是它列出的**具体条目**（channel 标识、客户/项目/账户名）——
我们的规则目前覆盖密钥/路径/邮箱/手机号，**不含客户与项目名这类上下文敏感信息**。

## 2. 不建议借鉴的部分

| 部分 | 原因 |
| :--- | :--- |
| gateway 及其协议翻译 | 与 Magpie 调研结论相同：改变产品性质、与低占用目标冲突。见 [05-magpie-research.md](05-magpie-research.md) |
| SquillaRouter / LightGBM 路由 | 为"切哪个模型"引入 ML 依赖，收益/复杂度比不成立 |
| agent 执行内核（plan-mode / goal-mode / tools / sandbox 实现） | 我们是管理工具，不当 agent |
| electron / webui / tui-host | 我们用 Tauri + 系统 webview 正是为避开 Electron |
| LLM-as-judge 打分 | 需要再调一次模型才能评价第一次，成本与不确定性都不该进监控工具 |

## 3. 与我们已有口径的对应

| OpenSquilla 的做法 | 我们已有的对应 |
| :--- | :--- |
| replay 步骤 `OK`/`SKIPPED`/`FAIL`，脱敏后保状态，信息不足给 `UNKNOWN` | `isHung` 三态、可观测性五类结论、「读不到不是没有」 |
| 「用能完成任务的最窄权限档」 | 凭据只进钥匙串、UserDefaults 零密钥 |
| Guest 降权 + 不进审批队列 | `/notify` 无鉴权的已知短板（尚未补） |
| 旧模式名只作兼容垫片、不进 UI | `shell_mode` 可沿用同一原则 |
| 分享前按清单脱敏 | `scripts/scan-secrets.sh` + `docs/agent/desensitization.md` |
| 省一次按下游计价 | `CONTEXT.md`「省一次要按下游计价」 |

**结论：它不是功能来源，是一面镜子——验证了我们几条口径是对的，并指出了两处我们能补强。**

## 4. 可落地的三条

1. **诊断/回放做成显式开关 + 可安全分享**：给本项目补一条"导出诊断前按清单脱敏"的固定步骤，
   清单里加上客户名/项目名/channel 标识这类上下文敏感项。
2. **Guest 降权三原则补进 `LocalEventServer` 的改进方向**：认证失败与未认证同权、
   远端不进审批队列、边界在所有执行面一致。这是本项目已知的安全短板。
3. **`shell_mode` 只用一代模式名**：旧名（若将来有）降为后台兼容，不进 UI。
   写进 [03-approach.md](03-approach.md) 的 §2.2 作为实现要求。

以上三条只有第 3 条与当前 Phase 绑定；前两条列入 [04-plan.md](04-plan.md) 待办，
不阻塞 Phase 1。
