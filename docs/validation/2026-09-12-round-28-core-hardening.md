# R27/R28 验证记录：无障碍第二批 + Core 健壮化（v0.0.44 / v0.0.45）

日期：2026-09-12 · 执行：主 agent

## R27（0.0.44）
- popover Agent 快捷行 isButton + 状态 label；设置/退出 icon 按钮 label
- axdump 实证（System Events entire contents）：AX 树可达、面板节点存在
- **版本哨兵实战首秀**：README 版本漏改被拦截，精确报错修复后打包

## R28（0.0.45）
- warmUp 已热仍回调（Task 主线程幂等）——启用集重放契约修复
- refreshingThread + performRefresh 断言——递归死锁从注释约束变运行期检查
- Selftest：before/after 双 nil 保护 + builtinOr 安全查找（5 处强解包）
- profile(id:) 显式防呆警告文档
- 测试 184/0；selftest 全过；打包重启（0.0.45）
