# R17 验证记录：数据源终态 + 版本哨兵（v0.0.34）

日期：2026-09-12 · 执行：主 agent + 独立验收官（轻量，含变异实验）

## 改动
- TokenUsageMonitor：主库连续 3 拍缺失 → 该源置空（streak 计数区分瞬时/永久；首轮字符串匹配误判 -wal:missing 已修正为文件存在性）
- 版本哨兵测试（CHANGELOG/README/build-app.sh 三处一致，验收变异实验证实报红）
- Registry 解码容错测试；测试 176 → 179

## 声明偏离（已落档 issue）
- postEvent 保留 public（LayoutRegression 普通 import 会破坏）
- 用例数目标 190 裁剪为 179（缺口已前轮落地）

## 运行证据
- 179/0 ×3；--selftest 全过；打包重启（0.0.34）
