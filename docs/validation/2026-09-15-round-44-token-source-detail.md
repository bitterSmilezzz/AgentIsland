# R44 验证记录：Codex 用量下钻（v0.0.61）

## 问题与根因

“按工具用量”中的 Codex 有本地 JSONL 用量，却在点击后无响应。Token 来源和实时
Agent 快照被错误地绑定：ChatGPT 桌面端承载 Codex 时，为避免重复显示，Codex 不会有
独立快照；但 `~/.codex/sessions` 仍可提供可读 Token 明细。

## 修复契约

- 导航资格改为“本地来源可读 + 已注册的详情能力”，不再要求独立实时快照。
- 详情页从引擎读取该来源的 24h / 累计聚合；无独立运行项时明确说明数据来自本地会话记录。
- 主列表的在线可见性与 ChatGPT/Codex 去重规则保持不变，不会因历史用量重新制造离线或重复 Agent 行。

## 验证

- 回归先以“Codex 可读、仅 ChatGPT 在实时快照中”复现失败：目标路由为 `nil`。
- 修复后同一用例通过；`SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk .build/debug/AgentIslandTestsRunner` 为 **222/0**。
- `scripts/build-app.sh` 完成测试门禁、release 构建与 ad-hoc 签名；`dist/AgentIsland.app` 为 **0.0.61**，已重启。
