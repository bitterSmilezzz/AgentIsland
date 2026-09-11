# R08 验证记录：UI 渲染开销（v0.0.25）

日期：2026-09-12 · 执行：主 agent + 独立验收官

## 改动
- IslandPanel：observe() 高度签名去重（route/可见数/汇总/环架/事件 id/横幅六元组）+ docked needsDisplay 保留 + sink 去 Task 跳跃；边缘 Timer 0.06→0.12s
- SettingsView：ProfilesCache 引用盒按 installedScanVersion 缓存 builtin/discovered
- 测试 175/0（用例数不变）

## 运行证据
- 175/0 ×2；--selftest 全过；app 运行稳定（0.1% CPU、无新崩溃日志）；打包重启（0.0.25）
- 验收 pass：签名与 expandedHeight 六输入完备对照；docked 呼吸灯 withAnimation 自驱动 + needsDisplay 保留论证；$updatedAt 末位发布时序安全

## 限制声明
- 屏幕熄屏，实机截图视觉验收未做；三态渲染由代码层论证（SwiftUI @ObservedObject 独立失效通道 + docked 显式失效保留）。待用户日常使用确认：展开态逐拍文本刷新、事件横幅展开/收起、细条呼吸灯
