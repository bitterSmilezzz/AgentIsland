# Spec: C1 — token 子系统收进引擎单一 seam

> 产出：/improve-codebase-architecture → grilling（五问全按推荐）→ /to-spec，2026-09-06。
> GitHub 存档：issue #1（已切换本地 tracker，此处为工作版）。

## Problem Statement

token 数据链路不可测试也不可替换：UI 三处视图直接穿透引擎公开子对象读取 token 数据与轮询控制；轮询生命周期有两个 owner（引擎 start/stop 与面板 docked/expanded 各管一半），「暂停不关连接、stop 前先清回调」等顺序约束只活在注释里；4 个测试用例绑定本机真实 SQLite 数据，换机器必挂。修复散落全链路、反复回归（15 轮审查中 token 链路占 5 轮）。

## Solution

为 token 子系统立一个测试 seam：轮询面与查询面拆为两个小协议，现有实现一个类同时满足；轮询生命周期收归引擎单一 owner（懒启动，仅「呈现活跃」时运行）；UI 全部改为只依赖引擎 interface；环境绑定测试改为临时 fixture SQLite。

## User Stories

1. As a 面板窗口控制器, I want 用一个引擎方法表达「呈现活跃/不活跃」, so that 我不再需要知道 token 轮询的暂停/恢复语义与调用顺序
2. As a 引擎, I want 独占 token 轮询的启停时机, so that 停止/暂停语义的回归集中在一个 module 里
3. As a 引擎测试, I want 注入假的 token 轮询 adapter, so that 不需要真实 SQLite 与后台线程即可断言生命周期语义
4. As a 岛卡片视图, I want 经引擎读取 token 汇总, so that 依赖面收敛为引擎一个 interface
5. As a 详情页, I want 经引擎转发发起按模型/按会话下钻查询, so that 未来换数据源不影响调用方
6. As a 查询层测试, I want 用临时目录自建 fixture SQLite, so that SQL 口径获得环境无关守护
7. As a 用户, I want 面板展开即刷新并展示最新 token 用量（行为保留）
8. As a 用户, I want 面板收起时无 token 查询开销（行为保留，与现网净行为等价）
9. As a 维护者, I want 引擎不再公开 token 子对象, so that 「谁在什么时候驱动 token」在类型层面可见
10. As a 干净机器上的贡献者, I want 测试套件全绿, so that CI/本地验证可信
11. As a 后续 agent, I want 引擎 fake 用法与既有 fake 一致, so that 学习成本为零
12. As a 维护者, I want 本轮不动 SQL 数据源内部实现, so that 改动面可控

## Implementation Decisions

- 两个小协议：**TokenUsagePolling**（生命周期 + 缓存读取 + 刷新回调，引擎消费）与 **TokenUsageQuerying**（两个下钻查询，引擎转发时消费）；现有实现类同时满足，内部锁、增量缓存、SQL 保持不变
- 引擎新增「呈现活跃」方法（setPresentationActive）：激活→启动轮询并立即刷新；失活→暂停（保连接）；stop→全停；幂等
- **懒启动**：轮询仅呈现活跃时运行（菜单不展示 token，与现网净行为等价）
- 引擎 token 子对象降为私有实现细节；9 处 UI 穿透全部改走引擎 interface
- 测试 fake 放 Core，与既有进程/文件 fake 并列
- 两套内联 SQL 本轮不动（one adapter = hypothetical seam）
- 术语遵循 CONTEXT.md：呈现活跃（presentation active）、Token 用量（净消耗口径）、采样 vs 轮询

## Testing Decisions

- 只测外部行为：经引擎公开 interface 断言生命周期语义，不翻内部字段
- 引擎侧：fake 轮询协议记录调用序列；沿用 EngineTests 风格与 TestKit runner
- 查询层：fixture SQLite（临时目录建表插行）替换 4 个「本机真实数据」用例；token 路径改构造注入（默认现网路径）
- 回归验证：`swift build && .build/debug/AgentIslandTestsRunner` 全绿 + `--probe` 冒烟

## Out of Scope

- SQL per-source adapter 化；TokenUsageMonitor 内部并发结构（锁/缓存/WAL 戳）调整
- UI target 可测化（Package target 依赖不变）
- 用户可见行为变化（除懒启动等价简化）
- C2–C6 各候选
