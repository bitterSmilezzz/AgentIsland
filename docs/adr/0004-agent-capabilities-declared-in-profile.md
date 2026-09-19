# Agent 的能力以档案数据声明，不按 agent id 反查

2026-09-20 做采样热路径治理时发现：新增一个被监控 Agent 要改约 8 个文件。根因不是分支多，而是**同一事实被到处重述**——`AgentSessionInspector` 按 `profile.id` 选解析器、`inspectKnownDatabase` 按 id 给出 (库路径, SQL)、`AgentActionInspector` 与 `AgentLogStreamer` 各自再写一份库路径、CLI 状态表另有一张 19 分支的 id→emoji 表。注册表只声明目录不声明库文件，于是 `~/.dimcode/v2/dimcode.sqlite` 在四个文件里各写一遍；emoji 表里 4 个 id（`dimagent` / `vibe` / `ima.copilot` / `egobrowser`）在档案改名后已不存在，只会静默退化成默认符号——字符串梯子不会报错，只会悄悄失效。

决定：把 Agent 的**能力与展示事实**收进 `AgentProfile`，注册表成为唯一声明处，解析器改为按格式与 schema 穷尽分派。落地的三项：`emoji`、`sessionDialect`（`genericTail` / `antigravityBrain` / `dshProjection` / `clineTasks`）、`sessionDatabase`（路径 + `dimTasks` / `statusIndex` / `openCode` schema + 查询语句）。配套 `AgentRegistry.profile(_:)` / `databasePath(for:)` 作为取值入口，并由「注册表自洽」测试守住：声明了专有方言却没给出对应目录的档案直接测失败（那种组合的线上表现是 Agent 永远只显示待机）。字段全部带默认值且走 `decodeIfPresent`，升级前存档的自定义 Agent 照旧可解。

**为什么不一次做完**：动作文案与日志流两处仍是 id 梯子，但它们内部的**路径**已改为向注册表取值——漂移风险（改了档案忘了改这里）已经消除，剩下的只是分派写法，收益不再抵得上改动检测核心路径的风险。同理不把解析器函数指针放进档案：那会破坏 `AgentProfile` 的 `Codable`/`Equatable`，而穷尽的方言枚举已经能提供编译器帮忙查漏。

**取舍**：档案字段变多（13 → 16），读一个 Agent 的行为要看两处（档案声明 + 方言实现）。换来的是新增复用既有格式的 Agent 只改注册表一处，且路径漂移从「四处各写一遍」变为「不可能」。若将来某个 Agent 需要第三种格式，成本是加一个方言 case + 一个实现——这是**该付**的成本，不是回归。
