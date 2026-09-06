# AgentIsland — Agent 工作约定

macOS 灵动岛应用：监控本机 AI 编码智能体的运行状态与 token 消耗。SwiftPM 构建，`swift build` / 自建测试 runner（无 XCTest）。

## Agent skills

### Issue tracker

单人开发：issues 走本地 markdown（`.scratch/<feature>/`，spec.md + issues/NN-*.md）。See `docs/agents/issue-tracker.md`.

### Triage labels

五个默认 triage 角色，标签名与角色同名。See `docs/agents/triage-labels.md`.

### Domain docs

单上下文：根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
