# 04 — 玻璃拟态 1px 微反光边缘与全局视觉打磨

**What to build:**
- `GlassCardBackground` 增加 1px 内描边（Specular Stroke）：深色模式微白微光（white 0.12~0.15），浅色模式微暗勾边（black 0.08），给毛玻璃边缘增加类似 Apple 灵动岛与 Control Center 的高级精致折射质感。
- 优化 `AgentRowView`、`TokenSummaryBar` 的 hover 动画过渡时间与反馈态，消除生硬跳变。
- 优化呼吸光环 `PulseAnimation`：使用平滑的双环呼吸微动效与柔和羽化。
- 更新 `CONTEXT.md` 补充新架构交互词汇。

**Blocked by:** Issues 01, 02, 03.

**Status:** resolved

- [x] 玻璃拟态 1px 内描边
- [x] 呼吸灯平滑度与羽化打磨
- [x] CONTEXT.md 词汇同步
- [x] `swift build` 与全部 48 个 runner 测试验证

## Comments

- 2026-09-06 完成：Theme 补充 glassSpecularBorder 微反光描边，GlassCardBackground 绘制晶莹内边，HoverRowBackground 添加淡入淡出动画，CONTEXT.md 同步领域词汇。全部测试与打包顺利通过。
