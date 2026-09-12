# R34 验证记录：数据面批（v0.0.51）

日期：2026-09-12 · 执行：主 agent（小轮自验 + 单测）

## 改动
- G1：配置变更/终止/清理的重采样改 sampleInBackground（主线程快照+匹配+探测移出）
- F6：refreshTokenUsageOnce + popover onAppear 按需刷新（协议补 refreshAsync，两处 fake 同步）
- F9：dbGeneration 连接代际（stop 后迟到重建不写回缓存，transientHandles 用毕即关）
- G4：refreshQueued 在飞去重
- 测试 192 → 195

## 运行证据
- 195/0 ×3；selftest 全过；打包重启（0.0.51）；收起态 CPU 1.2%（采样点）

## 说明
- refreshAsync 去重测试为「不崩溃 + 有结果语义」冒烟（去重的精确线程时序不可确定性观测）；
  popover 刷新出口为入口可用性测试，实际 popover 显示需人工确认（打开菜单栏即可看到 token 有值）
