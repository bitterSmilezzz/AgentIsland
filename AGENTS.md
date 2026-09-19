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

### 自动化提交与发版

- 改造完成后，只要测试通过，就自动提交到远端，然后执行发版并更新文档

### 数据脱敏红线（交付给第三方工具前必读）

本项目对外交付前必须保证工作区不含个人数据。约定见 `docs/agent/desensitization.md`：
- 本项目**不存储任何凭据**（源码零硬编码 secret）；如未来需要凭据，一律走环境变量注入，禁止写进任何文件
- `.scratch/`、`artifacts/`、`dist/`、`.build/`、根目录 `*.zip` 均不入库、不交付（屏幕截图可能含个人会话内容）
- 交付第三方收集数据的工具前，先删上述本地目录再用 `docs/agent/desensitization.md` 的复扫清单过一遍

## Agent skills

### Issue tracker

单人开发：issues 走本地 markdown（`.scratch/<feature>/`，spec.md + issues/NN-*.md）。See `docs/agents/issue-tracker.md`.

### Triage labels

五个默认 triage 角色，标签名与角色同名。See `docs/agents/triage-labels.md`.

### Domain docs

单上下文：根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
