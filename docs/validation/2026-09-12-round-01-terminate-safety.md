# R01 验证记录：进程终止安全（v0.0.18）

日期：2026-09-12 · 执行：主 agent + 独立验收官（agent）

## 改动
- ProcessMonitor.swift：`TerminationOutcome` 枚举 + `terminate(pid:expectedPath:)` proc_pidpath basename 复核 + 残余 TOCTOU 注释
- ActivityEngine.swift：terminateAgent 快照成员核对（防陈旧 pid），拒绝路径事件文案区分
- AgentCleaner.swift：AgentAnomaly.batchCleanable + 孤儿活动佐证（recentlyActiveProfileIDs）+ clean 带 commandPath
- ToolboxView.swift：扫描传 10 分钟活跃佐证集；cleanAll 过滤批量目标、反馈口径修正；批量按钮计数/禁用
- 测试：159 → 163（终止器 3 例、孤儿佐证矩阵、成功路径双层防线夹具）

## 运行证据
- swift build --build-tests：Build complete，0 警告
- AgentIslandTestsRunner：163 通过 / 0 失败（连跑 3 次一致，无 flaky）
- --selftest：20 项全部通过
- build-app.sh 0.0.18 打包成功，应用已重启
- 独立验收：pass（10 项验证矩阵全过；红线零触碰）

## 已知残留（记录不阻塞）
1. 进程树采集后子进程 pid 在 300ms 补发窗口内被复用的 TOCTOU——已注释声明接受
2. cleanAll 复核窗口内新出现的异常会被计入「未能终止」（归因失真，旧代码继承）→ 挪入 R10
