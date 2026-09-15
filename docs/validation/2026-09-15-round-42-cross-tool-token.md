# R42 验证记录：跨工具 Token 总体与分工具用量（v0.0.59）

## 覆盖范围

- 总体：DimAgent、OpenCode、Codex、Claude、WorkBuddy、WorkBuddy AI 的可读本地数据合并。
- 明细：当前范围 Token、占比、可用成本，以及「0 用量」和「未发现本地明细」的状态区分。
- 安全：缓存读取不重复计入；Codex response id 全局去重；JSONL 不保留对话正文。

## 自动验证

- `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk swift build --build-tests`
- `.build/debug/AgentIslandTestsRunner`：**219 通过，0 失败**。
- `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk python3 scripts/test-token-layout.py`：通过；Token 汇总文本在有/无事件横幅时均未侵入反向倒角。
- `git diff --check`：通过。

## 实机验证

- Release v0.0.59 已打包、ad-hoc 签名并重启 `dist/AgentIsland.app`。
- 面板辅助功能树确认 Token 汇总入口可用；本机实际显示 **24h 3.13M、累计 229.94M**，高于旧双 SQLite 统计，证明总体已纳入结构化工具日志。
- 主面板只显示在线 Agent；统计页可保留历史工具用量。未在当前监控列表中的历史工具不再显示详情箭头，避免进入空白详情。

## 工具链说明

- 默认 macOS 27 SDK 缺少 `SwiftUIMacros` 插件；验证与打包固定使用 macOS 26.5 SDK。该问题与本次功能无关。
