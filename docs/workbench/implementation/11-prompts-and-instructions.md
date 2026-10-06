# 11 · 内建提示词与用户指令

属于整体方案 D2 / NEW-06，保留 KEEP-04 并满足 UX-02。提示词管理由 AgentIsland 实现，不依赖 CC Switch、Magpie 或外部服务。

## 范围与流程

模型与连接 → 提示词。新增、编辑、移除只改本地提示词库；预览选定的已保存正文，选择 Codex 或 Claude Code，确认后替换对应完整用户级指令。Codex 为 AGENTS.md，Claude Code 为默认 ~/.claude/CLAUDE.md。CODEX_HOME 沿用已有后端解析，不接受前端文件路径。预览展示当前/目标正文，先备份、复核版本、原子替换，再读回。还原也先预览并备份当前正文。取消不写入；失败保留草稿，切换模型子页保留编辑内容；确认时保护子页导航、键盘焦点与 Escape。

## 契约与保护

prompts_list / prompt_save / prompt_remove 管本地库；prompt_preview / prompt_apply 管应用；prompt_backups / prompt_preview_restore / prompt_restore 管还原。库版本、提示词身份、当前指令、覆盖文件状态和目标正文以及目标文件身份绑定 plan_id；还原另绑定备份正文，变化拒绝旧预览。Store 锁串行本应用操作；不是跨进程事务。

严格 schema/version、UUID 与唯一名称；100 条、库 512 KiB、单提示词 32 KiB；当前指令/备份读上限 64 KiB，备份目录最多枚举 200 项。普通文件、UTF-8、有界读取；损坏、未来格式、链接和已识别凭据形态拒绝，不清空原件、不自动删历史。备份采用 ADR 0033 的不覆盖发布，Unix 文件权限 0600。检测不是任意秘密的通用识别器。

存在非空 AGENTS.override.md 时拒绝操作，因为它优先于 AGENTS.md。项目级规则不改动且可能覆盖全局指令；新会话加载后须在 Codex 内核对，写入读回成功不代表客户端已加载。还原是正文还原：原文件缺失的备份以空正文保存，还原会写入空 AGENTS.md，不恢复文件不存在状态。没有合并段落、未核实工具写入、同步、执行工具或自动回答。

依据 [官方 AGENTS.md 规则](https://learn.chatgpt.com/docs/agent-configuration/agents-md)。用户级规则加项目链默认共用 32 KiB 的文档读取上限，因此本应用的单正文上限不能保证所有项目文档都被读取。

## 当前证据

6 项 Rust 行为回归覆盖 CRUD/坏文件、精确应用还原、来源/覆盖变化、已识别私有内容、链接及库/备份变更；JS 列表转义及注册/解析回归通过。全套 737 项通过、7 项忽略。纯模拟浏览器验证草稿失败/刷新保留、取消零写入、旧预览拒绝、备份还原和键盘焦点；未读写真实用户指令。原生客户端加载和整体品质验收继续按 07 门禁执行。

## Claude Code 适配

共享提示词库，前端仅传 codex/claude 枚举，不接受路径。保留已有 Codex 备份目录，Claude 备份单独存于其 claude 子目录；目标文件身份参与预览哈希，跨工具旧计划拒绝。应用、还原都绑定目标工具，切换只更新备份清单，保留草稿，确认和处理中冻结选择。

仅支持默认 ~/.claude/CLAUDE.md，若应用继承 CLAUDE_CONFIG_DIR 则拒绝操作；无法观察另一工具独立启动环境，选择项和预览明确标注默认目录。其他用户规则、组织规则、项目规则及导入可能共同加载，不声称替换文件等于完整有效提示词。依据 [官方指令作用域](https://code.claude.com/docs/en/memory) 与 [配置目录变量](https://code.claude.com/docs/en/env-vars)。用新会话 /context 核对客户端加载；本实现不展开导入或读取会话私有数据。

新增沙箱回归核实共享库、目标哈希隔离、跨工具备份不能还原、外部修改拒绝旧还原、Claude 不误用 Codex 覆盖协议。
