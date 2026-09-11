# R13 验证记录：文案口径统一（v0.0.30）

日期：2026-09-12 · 执行：主 agent + 独立验收官（轻量）

## 改动
- durationText 唯一实现（Models）+ 三处调用收敛 + 3 条口径断言（59.6/59.4/负值）
- ToolboxView 可回收格走 MemoryFormat.text（text(0)="—"，0M 特判保留）
- 口径对齐：popover「N 可见」+help、流水页「实时流水 · N 条事件」+加载态、声音描述补静默行为
- 测试 176/0

## 声明偏离
R9（数据源缺失提示）未搭车——涉及「查询失败保留上次成功值」契约语义变更与 UI，挪 R17。

## 验收证据
- 全仓「N分M秒」仅剩 Models 一处；MemoryFormat.text(0) 占位符核实；加载态首帧时序核实（onAppear 同步置位无闪烁）；主卡/popover 同口径
