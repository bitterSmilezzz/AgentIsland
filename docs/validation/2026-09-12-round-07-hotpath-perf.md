# R07 验证记录：采样热路径性能（v0.0.24）

日期：2026-09-12 · 执行：主 agent + 独立验收官

## 改动
- ProcessMonitor：matchesProcessNames 契约收口（删重复 lowercased）；dsh 分支走 cachedCommandLine
- AgentActionInspector：cachedCommandLine 10s TTL 缓存（NSLock + 512 粗清 + 负结果不缓存）；opencode SQL 两步化
- ActivityEngine：usageSnapshot 每拍 1 次，checkCostSpikeAndRunaway 增 usage 参数
- FileMonitor：isIgnoredActivityPath basename 化 + 心跳先判扩展名
- 测试 174 → 175（node_modules 深层未来 mtime 判别用例 + 夹具 basename 小写契约修正）

## 运行证据
- 175/0 ×3；--selftest、--probe 全过；打包重启（0.0.24）
- 验收 pass：真库 EXPLAIN 对照（临时 B 树消失）、8.1k part 会话 1.3-2.6ms → 0.002ms、SQL 结果逐字节等价、probe 形态一致
- 性能量级声明全部读码评估合理（0.3-0.8ms/拍、2-5ms、TTL 复用）

## 遗留
- sampleInfo/results 合一（改进2）未做，低值；S9 isFresh 跳过（口径改动 > 收益）
- EngineTests 夹具 "Codex (Service)" → "codex (service)"（验收发现，已修）
