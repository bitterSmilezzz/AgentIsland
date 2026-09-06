# 03 — UI 去穿透 + 收缩公开面（contract）

**What to build:** UI 对 token 数据的全部直达引用改走引擎 interface：面板高度判断两处、岛视图汇总栏两处经引擎暴露的 grandTotal 只读面；详情页下钻两处经引擎的两个转发方法。迁移完成后引擎的 token 子对象降为私有实现细节——expand–contract 收缩步，编译器强制「UI 只能从引擎拿 token 数据」。

**Blocked by:** 02 — 呈现活跃单一 owner（懒启动）

**Status:** ready-for-agent

- [ ] UI target 内不再有任何引擎 token 子对象直达引用（编译器强制）
- [ ] 汇总栏出现/消失与展开高度计算行为不变
- [ ] 详情页下钻查询行为不变（含代际防串页语义）
- [ ] `swift build` + 测试 runner 全绿 + `--probe` 实机冒烟

## Comments

- 2026-09-06 发布。
