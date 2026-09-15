# R38 验证记录：状态强语义与待确认通知路由（v0.0.55）

日期：2026-09-15 · 执行：主 agent
触发：用户报告任务完成/未完成/待机跟踪不准确，并要求 Agent 需要用户确认时通知、点击直达确认界面。

## 根因

1. 活动等级只有 `offline / idle / working`，等待用户时 CPU 与文件写入都沉寂，因此必然被压成 idle；明确完成后会话文件仍很新，又会被 workingWindow 暂时顶成 working。
2. 引擎只消费 CPU 与文件 mtime，没有读取 Agent 已经写出的结构化确认/完成事件。
3. 通知未携带 Agent 标识，应用也没有注册通知响应代理，点击后无法定位应激活的 GUI 或终端宿主。
4. `latestEvent` 是岛内横幅的单槽状态，多个 Agent 同拍产生事件时，若系统通知也依赖它会丢失前面的事件。

## 修复

- `ActivityLevel` 增加 `attention / completed`；结构化会话信号优先，CPU/mtime 双信号保留为未知格式的降级路径。
- `FileActivityMonitor` 在原扫描中缓存每个监控目录最新信号文件；`AgentSessionInspector` 只对这些命中文件做最多 96 行 / 256 KiB 尾读，不另行遍历目录。
- 支持 Codex `request_user_input`、Claude/Dim `AskUserQuestion`、常见 approval/permission 状态和 task/turn/session/run complete 标记；Dim、ZCode、WorkBuddy、OpenCode 增加已知 SQLite schema 的只读末条适配。
- 确认请求以结构 ID 去重；关联 tool result / user response 解除。进入等待会切断旧工作区间，解除时不产生虚假完成通知。
- 新增逐条 `taskEvents` 流负责外部通知；`latestEvent` 继续只负责岛内横幅。通知写入 Agent id，点击时调用既有 `AppActivator` 激活 GUI 或 CLI 宿主，失败则展开 AgentIsland 对应详情。
- Focus 模式静默普通完成，但保留需要人介入的 `attention` 与严重告警；Silent 仍完全静默。

## 回归与判别性验证

- 首次加入测试时，编译因缺少 `AgentSessionInspector / AgentNotificationRoute` 等类型失败，证明测试确实覆盖新能力。
- 新增/扩展 10 个用例：Codex 与 Claude 请求/解除、result-only 反例、普通问句反例、完成终态失效、attention 覆盖写入、完成立即收尾、通知 userInfo 路由、多 Agent 同拍逐条投递、最新活动文件缓存与清理。
- 一次全量回归暴露“无采样文件仍旁路读取真实 Dim DB”的隔离缺陷，表现为 14 个既有状态机用例被误判 completed。修正为仅在本轮扫描实际命中文件时启用生产解析，测试 hook 保持显式可注入。
- 修正后自建 runner：**212 通过 / 0 失败**。

## 仍需实机确认

- 系统通知点击行为需在新打包的 `dist/AgentIsland.app` 中触发真实确认请求验证；如果通知权限被系统关闭，岛内 attention 状态与 Peek 仍可见，但通知中心不会展示横幅。
