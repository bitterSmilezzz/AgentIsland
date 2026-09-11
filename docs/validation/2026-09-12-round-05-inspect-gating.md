# R05 验证记录：动作探测门控与会话树枚举加固（v0.0.22）

日期：2026-09-12 · 执行：主 agent + 独立验收官

## 改动
- ActivityEngine：inspectAction 移到等级判定后、仅 working（含滞回）调用；inspectActionHook 测试注入点（默认 nil）
- LogTailReader.newestFile：双份 findNewestFile 收敛 + 符号链接目录跳子树 + 20000 条目预算
- AgentLogStreamer.fetchClaudeEvents：timestamp 行内 JSON → 文件 mtime 回落（id 稳定化）
- 测试 171 → 174

## 运行证据
- 174/0（修复后连跑）；--selftest 全过；打包重启（0.0.22）
- 验收 pass：行为等价逐 case 论证、性能量级声明核对合理（idle 拍省 5-20ms/次枚举）、hook 生产无赋值

## 跳过项（验收官认定成立）
- 根 mtime 缓存：门控已消除 idle 调用点，working 态树频繁变化缓存收益低（任务书任务 3 为「或」可选分支）

## 过程事故记录（重要教训）
1. 对文件符号链接调用 NSDirectoryEnumerator.skipDescendants() 会破坏枚举器状态、丢弃后续所有条目——守卫必须带 isDirectory 条件（ddaa30d 修复，实测复现）。
2. 测试命令 `runner | tail` 的管道会掩盖 runner 退出码（tail 恒 0），导致带失败用例的提交进入历史（0f16132 + 修复 ddaa30d）。后续轮次验证一律重定向到文件后看文件 + 显式取 rc。
