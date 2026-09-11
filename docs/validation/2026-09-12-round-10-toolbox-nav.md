# R10 验证记录：工作台与导航正确性（v0.0.27）

日期：2026-09-12 · 执行：主 agent + 独立验收官

## 改动
- ToolboxView：复核回调 route 守卫 + id 差集归因；独立 scannerProvider（静态长驻 + 后台预热两拍）
- ProcessMonitor：snapshot() 全程 snapshotLock（差分窗口原子）
- IslandView / AgentHoverTooltip：直达按钮消费 activate() 返回值，失败 1.5s 反馈（三处入口）
- 测试 175 → 176（6 线程并发快照竞态冒烟）

## 运行证据
- 176/0 ×2；--selftest、--probe 全过；打包重启（0.0.27）
- 验收 pass：锁序单向无死锁论证、hung 行不漏报论证（引擎 hungAgentIDs + 真实死循环进程任意窗口均值 >10%）、离开页面不给反馈的合理性论证

## 遗留
- activateFailed 复位任务不取消前驱（连点下纯外观级，可接受）
- 长窗口均值显示：久置后首次重扫的行内 CPU 为窗口均值（展示层，下扫自愈）
