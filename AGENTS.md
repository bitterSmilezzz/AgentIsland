# AgentIsland — Agent 工作约定

macOS 灵动岛应用：监控本机 AI 编码智能体的运行状态与 token 消耗。SwiftPM 构建，`swift build` / 自建测试 runner（无 XCTest）。

## 交付约定

### 自动重启应用（无感）

每次改动完成、构建出新的 `dist/AgentIsland.app` 后，**直接重启应用**，不必询问也不必先检查它是否在运行：

```sh
pkill -x AgentIsland; sleep 0.6; open dist/AgentIsland.app
```

- 旧实例存在就先杀掉（`pkill -x` 按进程名精确匹配，不会误伤 `AgentIslandTestsRunner`）
- 旧实例不存在时 `pkill` 返回非零，忽略即可，继续 `open`
- 重启后简短汇报，不要写成长篇操作说明

## Agent skills

### Issue tracker

单人开发：issues 走本地 markdown（`.scratch/<feature>/`，spec.md + issues/NN-*.md）。See `docs/agents/issue-tracker.md`.

### Triage labels

五个默认 triage 角色，标签名与角色同名。See `docs/agents/triage-labels.md`.

### Domain docs

单上下文：根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
