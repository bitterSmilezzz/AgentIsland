# 项目优化与验证（2026-09-09）

Mode: Full Sweep（风险筛查与局部修复）
Scope: 项目结构、核心监控链路与测试入口；深入检查 TokenUsageMonitor、FileMonitor、ActivityEngine、日志尾部读取与安装缓存。不是全仓逐行审计，也不代表所有缺陷已排除。

## Findings 与修复

### R6 / Warning：24h 缓存没有时间失效条件

Symptom: 仅用数据库及 WAL 文件戳判断缓存有效性；文件静止后，24h 数据一直不变。
Source: Domain-Driven Design — Domain Model：滚动时间窗口依赖时间，不能只依赖数据写入。
Consequence: 已超过 24h 的消耗仍留在当日统计。
Remedy: 每个源独立记录成功刷新时间，缓存最长保留一个默认轮询周期（60s）；文件变化立即失效。一次刷新共享同一个截止时间，毫秒边界在乘法后转整数。

### R4 / Warning：打开预检不能代表查询成功

Symptom: 打开 SQLite 成功后，聚合 SQL 失败仍返回零值、更新文件戳；缺失源直接从汇总移除。
Source: Code Complete — Defensive Programming；A Philosophy of Software Design — Strategic vs. Tactical Programming。
Consequence: 临时故障表现为用量突然下降，失败结果还可能被缓存。
Remedy: 聚合结果使用可选值区分失败与成功的零值；失败保留旧统计和旧戳并继续重试，健康源独立更新。移除每轮额外打开/关闭连接的预检，整次刷新串行化，避免查询与发布交错。不存在未安装 Agent 的数据库属于正常情况，不输出重复缺失日志。

### R3 / Warning：详情页与汇总的 NULL 口径不同

Symptom: 汇总将缺失字段视为零，模型及会话详情直接将多个 SUM 相加。
Source: The Pragmatic Programmer — DRY：同一个净消耗规则在不同查询中发生漂移。
Consequence: 一组记录都缺少 reasoning 或 completionTokens 时，详情中的整组 token 变成零，与汇总不符。
Remedy: 双源的模型、会话查询均对每项 SUM 使用 COALESCE(..., 0)。

### T5 / Warning：缓存失效与错误恢复缺少回归保护

Symptom: 原有 SQLite fixture 测试主要覆盖常规汇总和下钻。
Source: How Google Tests Software — Change Coverage；The Art of Unit Testing — Test Completeness。
Consequence: 文件静止、源故障及可选字段缺失时的错误未被检出。
Remedy: 新增五个真实临时 SQLite 用例：时间推进、SQL 失败与恢复、数据库暂缺、成功空库、双源缺字段的汇总/模型/会话一致性。时间推进通过内部时间参数完成，无需真实等待一天。

## Dimension Summary

| 维度 | 结果 |
| --- | --- |
| Review | 修复上述三个统计问题，修改仅涉及 Token 子系统 |
| Test | 新增五个行为回归用例，沿用现有 runner 和临时 SQLite fixture |
| Debt | 删除冗余结果副本、打开预检；详情查询统一 NULL 处理 |
| Audit | SwiftPM 的 App → Core、Tests → Core 无 target 循环；保留 ADR 0001 的采样节律归属 |

## 后续验证范围

- FileMonitor 的快跳过会复用活跃会话数，修改 activeSessionWindow 后没有立即使缓存失效；需补充扫描与设置变更并发时的回归测试，再修正失效策略。
- 两处日志尾读从固定字节偏移解码 UTF-8；偏移切在中文多字节字符内部时整段解码可能失败。需要增加跨字节边界的文件 fixture 并统一尾读工具。
- 现有部分测试触及真实进程和本机会话目录。沙箱内出现 `sysmond service not found` / `Cannot get process list`，这些测试通过不等于真实监控已完成验证。
- 未进行桌面交互验收、真实 CPU/内存基准、发布打包或线上运行验证。本次不声称 CPU 降低百分比。
- 查询失败后显示的是上次成功数据；持续失败时可能陈旧，当前模型没有“数据过期”标记。

## 验证记录

原始基线：69 通过，0 失败。修复后：74 通过，0 失败；程序 `--selftest` 全部通过；`git diff --check` 通过。

构建环境默认 Clang 缓存不可写，使用临时缓存可成功构建：

```sh
CLANG_MODULE_CACHE_PATH=/private/tmp/agentisland-clang-cache \
SWIFTPM_MODULECACHE_OVERRIDE=/private/tmp/agentisland-swift-cache \
swift build --disable-sandbox
.build/debug/AgentIslandTestsRunner
.build/debug/AgentIsland --selftest
```

没有修改系统工具链或缓存权限。

对照验证：在独立临时副本中恢复 HEAD 的 TokenUsageMonitor，仅接入内部 `now` 参数以运行相同时间用例。相同 74 个测试得到 70 通过、4 失败，分别为时间过期、SQL 失败保留、数据库暂缺、详情缺字段。成功空库用例在新旧实现均通过。工作区修复后的最终复跑为 74/74，自检通过。

迭代：完成统计修复后复查缓存的读写保护、双源独立更新、空值处理及消费方协议；未改变公开方法签名。停止于本次局部修复验证完成，其余条目保留供后续专项处理，不宣称全仓清零。
