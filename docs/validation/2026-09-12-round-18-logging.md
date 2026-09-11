# R18 验证记录：统一日志通道（v0.0.35）

日期：2026-09-12 · 执行：主 agent

## 改动
- 新增 AppLog（os.Logger public，AGENTISLAND_DEBUG=1 镜像 /tmp/agentisland.log）
- 接入 5 个诊断点（SafeNumber/prepare/step/open/SMAppService）；Probe/Selftest CLI 输出不经过
- README 调试指南改写为真实机制；镜像落盘守护测试
- 测试 179 → 180

## 调试排查记录
AppLog 首版为 internal：runner（@testable）编译通过掩盖了跨 target 不可见问题，app target 编译才暴露——改 public 后全绿。另：新文件加入后 SwiftPM 增量构建未自动纳入（touch Package.swift/源文件强制）。

## 运行证据
- 180/0；--selftest 全过；AGENTISLAND_DEBUG=1 启动镜像文件生效（nm 符号 10 处）
