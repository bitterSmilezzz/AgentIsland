# Vibe Usage 本地计数对照

参考固定提交 `c9ca682ff783ebe66eba052c61c64b51aeba092c` 的 [vibe-cafe/vibe-usage](https://github.com/vibe-cafe/vibe-usage)。仅对照纯读取器，不执行 CLI 同步、登录或网络通知。

## 来源契约

| 来源 | 新鲜用量口径 | 身份与数据源 |
| --- | --- | --- |
| Claude | input + output + max(cache_creation 总项, TTL 分项和)，不扣 cache_read | message.id 优先，老记录回退 id/uuid；同一请求的内容块/流式修订取最完整记录 |
| Codex | inclusive input − cached input + inclusive output | 本轮保留 response_id 去重的 token_usage_record；不叠加 token_count |
| WorkBuddy | 显式 cache miss（正值）或 inclusive input − cache read，加 inclusive output | 已完成 assistant 消息和 function_call；providerData.usage/rawUsage；缓存明细可为数组 |
| ZCode | message tokens.input − tokens.cache.read + tokens.output；output 已含 reasoning | CLI db/db.sqlite 的 message 是 canonical；rollout 不与镜像叠加 |
| OpenCode | input + output + reasoning + cache.write | 动态解析 message/session_message 表名，assistant 记录 |
| MiniMax Code | input + output + reasoning + cache_write | local_runtime_token_usage；不叠加消息投影 |

一手证据：[Claude](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/claude-code.js)、[Codex](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/codex.js)、[WorkBuddy](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/workbuddy.js)、[ZCode](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/zcode.js)、[OpenCode](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/opencode.js)、[MiniMax](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/mcode.js)、[桶合并](https://github.com/vibe-cafe/vibe-usage/blob/c9ca682ff783ebe66eba052c61c64b51aeba092c/src/parsers/aggregate.js)。实现独立编写；未复制上游代码进入产品。

## 复现与验证

修复前的确定性 fixture：Claude 带缓存创建记录应为 5820，读成 320；同一请求两个内容块应为 130，读成 240；MiniMax 缓存创建被丢弃。修复后同一命令通过：`cargo test --locked --manifest-path app/src-tauri/Cargo.toml tokens::tests`。

本地隔离执行固定提交的 WorkBuddy parser + aggregate，只替换数据目录解析器以锁定同一默认目录；输出仅含桶计数和 Token 合计，不输出项目/模型/会话内容。AgentIsland 的 opt-in `local_vibe_reference_totals_match_runtime_reader` 对照该合计，以及按上游契约独立计算的 ZCode / MiniMax SQL 合计，三项一致。完整私人输入和具体用量不入库。

## 准确性边界

这是默认本地目录的净消耗对照，不是 Vibe Usage 云端账号或账单对照。其他 profile、重定位目录、不同时间范围、其他版本的数据格式不能据此宣称完全一致。Claude/WorkBuddy 改动文件会重建修订集合，不改动时仍按文件戳复用缓存；Token 明细折入保留累计与模型归属。

本机 Codex 独立 response 记录比 event/token_count 路径多约 3%，后者缺少部分 response 的对应事件。仅检查字段形状与相邻事件类型还不能确定应扣除哪一侧；不得为了对齐某个数盲目删记录。event-only 旧日志、跨文件 fork/replay 去重、来源读取错误的可见诊断、费用的分量计价仍需独立验证，本轮没有宣布所有 Agent 绝对准确。
