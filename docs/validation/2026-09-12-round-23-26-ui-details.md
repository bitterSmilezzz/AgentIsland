# R23–R26 验证记录：几何校准 / 字号 / 性能 / 细节批（v0.0.40–v0.0.43）

日期：2026-09-12 · 执行：主 agent（小轮自验 + 单测/哨兵锁定）

## R23（0.0.40）
- B1 菜单栏振荡：top 热区补上界钳制（无上界 → 菜单栏内整条 x 跨度触发展开→收起循环）
- B4 空态高度实测校准 87 → 121（校准脚本：真实视图强制空态 + GeometryReader 量测 = 185pt − chrome 64）
- 测试 184/0；空态断言同步

## R24（0.0.41）
- badgeFont 令牌（mono 9pt）；14 处 <9pt 文本提升；chevron 7→8
- 行高无回归（徽标非主导高度元素）；IslandMetricsTests 全过；布局哨兵 PASS

## R25（0.0.42）
- B6：sysctl(KERN_PROC_ALL) 合并 pid+ppid（删 ~500 次 proc_pidinfo/拍；单次实测 2.4µs ≈ 1.2ms/拍）；缓冲重试
- B5/S9：isFresh 宽限 5s（阈值=间隔的永久短路修正）
- 基准：snapshot 均值 4.1ms（50 次基准，/tmp/bench-work 可复跑）；测试 184/0 ×3

## R26（0.0.43）
- 会话行「目录未找到」反馈（SessionRowView 自持 @State）；菜单栏文案→灵动岛；模型拆分占位；SummaryBar 累计 help+layoutPriority；两处 formatter en_US_POSIX；恒真 #available 删除；采样联动说明
- 测试 184/0；selftest 全过

## 版本管理教训
R25 已占用 0.0.42（release 提交先于 CHANGELOG 检查），R26 编号顺延 0.0.43。版本哨兵（CHANGELOG==README==build-app.sh 抽取）本应拦住——发布命令里 README 的 perl bump 用了旧目标串导致漏改。教训：release 命令必须先 grep 全仓版本再提交。
