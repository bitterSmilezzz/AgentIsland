# R20 验证记录：文档全量校对 + 战役终验（v0.0.37）

日期：2026-09-12 · 执行：主 agent（终审 agent 超配额，由主 agent 按同一清单执行）

## 改动
- README 9 处历史漂移全量校对（CPU 阈值/内置条数/节流/闲置/卡宽/用例数/target/进程枚举/阴影）
- CHANGELOG 0.0.37 收官条目（战役总账 + 已知限制）

## 终验运行证据
- 全量测试 180/0 连跑 3 次（rc=0 ×3）；--selftest 全过；--probe 功能等价
- 收起态 CPU 0.2%（全离线空载，对照 0.0.17 基线的 1.91% 系不同负载下测得，不做直接对比；逐轮微基准见各 validation）
- scripts/test-token-layout.py PASS

## 红线核查（git diff v0.0.17..HEAD）
- ActivityEngine 39 commits 全量 diff：scheduleNext 仅作为上下文行出现，sampleInterval/idleSampleInterval 零改动——ADR-0001 采样节律零触碰 ✓
- Package.swift 零 diff（零第三方依赖）✓
- 39 commits 全部 conventional 中文风格，无环境指纹 ✓
- CHANGELOG（0.0.18–0.0.36）↔ validation 记录 ↔ git log 三方逐轮对应（20 轮闭环无虚报）✓

## 遗留盘点（下一战役候选，按优先级）
1. 菜单栏内细条上方光标致 expand/collapse 振荡（expand 热区 y 无上界，既有几何）
2. <9pt 字号统一（依赖可视化验收）
3. VoiceOver 全流程人工验收（AX 树物化需辅助技术客户端）
4. 空态高度常量低估 ~30pt（resolvedExpandedHeight 兜底保证不裁切，但常量不再镜像真实布局）
5. TokenUsageMonitor isFresh 阈值语义（S9，本机数据量下无感）
6. ADR-0001 讨论过的 sysctl(KERN_PROC_ALL) 单次快照优化（R07 余力未做）
