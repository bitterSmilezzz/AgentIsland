# 具体实现文档

依据 [26 · 整体方案](../26-product-master-plan.md)拆分。目标契约与当前实施状态分开标注；规格中的目标不能单独证明 API 已可用。当前证据取自 main 工作区，各批验收以源码、回归与原生记录为准。

先读 [08 · 开发前方向与门槛](08-direction-and-readiness.md)，并用 [09 · 功能保留与追溯](09-baseline-and-traceability.md)检查每个批次的范围。

## 阅读与实施顺序

| 文档 | 解决的问题 | 前置 |
| --- | --- | --- |
| [01 · 契约与模块边界](01-contracts-and-modules.md) | 稳定身份、能力、状态、错误和命令分层 | 无 |
| [02 · UI 与导航动效](02-ui-and-navigation.md) | 三形态、组件、草稿、可中断过渡与原生几何 | 01 |
| [03 · 任务与人工关口](03-tasks-and-attention.md) | 普通待办兼容、任务/运行/产出、来源绑定 | 01；显示依赖 02 |
| [13 · Claude 方案只读采集](13-claude-plan-capture.md) | opt-in 采集、事件版本、短期正文、配置安装与鉴权边界 | 03、02 |
| [04 · macOS 窗口布局](04-macos-window-layouts.md) | 枚举、预览、权限、应用/部分失败/撤销 | 01；显示依赖 02 |
| [05 · 模型与连接](05-models-and-connections.md) | 现有档位归位、外部只读连接、配置适配 | 01、02 |
| [15 · 本机工具预算](15-local-tool-budgets.md) | 按工具预算、来源状态、受控保存、草稿和预警 | 02、05 |
| [06 · 平台适配](06-platform-adaptation.md) | Windows、Linux 交付项与 Web/移动边界 | macOS 完整验收后进入实现 |
| [14 · 隔离原生验收](14-native-vm-acceptance.md) | VM 身份、产物一致性、UI/Dock/单实例/CLI门禁及证据边界 | 07、08 |
| [07 · 实施与验收](07-delivery-and-acceptance.md) | 小批次交付、退出条件与验证命令 | 贯穿全部 |
| [11 · 提示词与用户指令](11-prompts-and-instructions.md) | 本地提示词库、Codex/Claude Code 用户指令预览/备份/还原与生效边界 | 02、05 |
| [12 · 本地 Skills 安装与恢复](12-local-skill-packages.md) | macOS 本地目录预览、发布/更新、持久恢复、记录移出与完整目标边界 | 02、05 |
| [10 · 工作空间](10-workspaces.md) | 项目/工具/档位/布局引用组合、核对、分项执行收据与恢复 | 02–05 已接入的内建模块 |

依赖允许先分别调研，不表示并行开发后续平台。macOS 的任务和窗口模块可各自推进，但合入前统一契约与页面生命周期；当前不要求多智能体并行工作。

## 范围约定

已有能力全部保留，详细清单见整体方案 §2。本轮实现规划聚焦体验收敛、本地任务、窗口布局、配置与可选服务。首个任务版本只组织和导航，不接管 Agent 执行或自动批准；内建能力融合优先，外部服务互操作只读，不接管请求，也不能替代功能融合验收（ADR 0023）。

文档描述的是实施规格，不是 issue tracker。开始具体代码批次时按 `docs/agents/issue-tracker.md` 在 `.scratch/<feature>/spec.md` 与独立 `issues/NN-*.md` 建执行记录；这里保留跨批次契约及验收，不记逐版流水。早期 03/04 文档是演进记录，本组与 26/25 是当前实现依据。
