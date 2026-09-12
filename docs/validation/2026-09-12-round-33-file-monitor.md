# R33 验证记录：文件监控生命周期批（v0.0.50）

日期：2026-09-12 · 执行：主 agent（小轮自验 + 单测矩阵 + 布局哨兵）

## 改动
- F2：scanTree newest 仅由非忽略非噪声常规文件聚合（DirScanResult 补 signalFiles 区分「成功无信号」与「枚举失败」）
- F3：目录缺失连续 ≥3 趟终态清零（missingStreaks 实例级持久；runScan 单飞互斥下读写）
- F4：lastScanGeneration 完成代际——invalidateScan 后下一趟扫描绕过 3s 节流
- 适配 F2 语义的既有用例（空目录 nil 为正确行为；夹具补信号文件与目录 mtime 回拨）
- 测试 188 → 192

## 过程记录
- 中途踩坑 ×3：DirScanResult 重复声明（脚本重跑）、missing 声明为 runScan 局部变量（终态需跨扫描持久，提为实例级）、断言与夹具不匹配（dictionary 下标 nil 赋值=删键语义，改用 clearedDirs 显式集合）
- selftest「临时目录应可读 mtime」在 F2 语义下空目录返回 nil 是正确行为——改为「写入后可读」断言

## 运行证据
- 192/0 ×3；布局哨兵 PASS；selftest 全过；probe 双源正常；打包重启（0.0.50）
