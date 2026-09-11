# R06 验证记录：SQLite 只读层统一（v0.0.23）

日期：2026-09-12 · 执行：主 agent（ReadonlyDB + 测试）+ 双执行者并行（A: AgentActionInspector / B: AgentLogStreamer，文件独占）+ 独立验收官

## 改动
- 新增 ReadonlyDB.swift：连接缓存（按路径长驻）+ (dev, inode) 失效 + open 失败关句柄 + 缺失 stat 快路径；body 持锁、不可重入契约
- 10 处样板收口；openReadonly 删除；clipTitle 收敛；inspectAction switch 分发
- 泄漏测试改测 ReadonlyDB（chmod 000 rc=14 形态）
- 测试 174/0（用例数不变，改造 1 例）

## 运行证据
- 174/0 ×3；--selftest、--probe 全过；打包重启（0.0.23）
- 验收 pass：dim SQL 字面量逐字节核对、真实库对照（inspectZCodeAction 与 sqlite3 CLI 一致）、withDB shim 类型推断问题独立复现属实

## 过程记录
- 双执行者并行未发生写冲突；执行者 A 发现 Swift 泛型求解器 T? 恒等钉死问题，加私有 withDB shim（验收官认定等价透传）
- 主 agent 失误一次：管道掩盖构建失败 + 旧二进制假绿灯导致一次带编译错误的提交，已 amend 修复（9d5f6d3）。教训：build rc 与 test rc 必须显式检查后再提交
- 验收建议已落：不可重入 doc 警示、设备号比对（st_dev）
