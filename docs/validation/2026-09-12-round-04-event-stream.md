# R04 验证记录：事件流水正确性（v0.0.21）

日期：2026-09-12 · 执行：主 agent + 独立验收官（轻量）

## 改动
- AgentLogStreamer.deduplicateIds：批次内 id 去重（下一个可用序号，防标题含 # 再碰撞）；fetchRecentEvents 统一出口
- AgentLogEvent.init：detail 截断 4096 字符 + 标记（唯一构造点，覆盖 9 个 fetch 路径）
- ISO8601DateFormatter 静态化 ×2（isoFractional/isoPlain）
- 测试 169 → 171

## 运行证据
- 171/0；--selftest 全过；打包重启（0.0.21）
- 验收 pass：dim 行级截断整组进出边界核实、detail 消费点（LiveLogStreamView 渲染/copyAllLogs）全链路核对、formatter 格式选项逐一比对等价

## 遗留（记入 R05）
- claude 流水 timestamp: Date() → 事件 id 对 claude 每次刷新全变（稳定 id 目的落空），R05 一并修
