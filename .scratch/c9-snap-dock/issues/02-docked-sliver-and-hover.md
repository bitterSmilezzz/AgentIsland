# 02 — 顶部与右侧微细条（Sliver）UI 及悬停唤出逻辑

**What to build:**
- 在 `IslandView` 增加 `dockedSliver` 视图：
  - 顶部贴边时：横向圆角胶囊（长 140pt，露 6pt，居中或拖动 X 轴），带微弱工作呼吸绿点。
  - 右侧贴边时：纵向圆角胶囊（高 120pt，露 6pt，居中或拖动 Y 轴），带微弱工作呼吸绿点。
  - 晶莹质感：毛玻璃 + 1px 细微高光反光边框。
- 窗口在 docked 状态下尺寸与位置：
  - 露出 6pt 微细条，其余内容隐于屏外。
- 光标悬停（Hover）与热区侦测：
  - 当光标碰触微细条区域时，自动触发 `.expanded`。
  - 当光标离开面板后，延迟 `collapseDelay` 自动收回为微细条。

**Blocked by:** Issue 01.

**Status:** resolved

- [x] `dockedSliver` 视图实现与深浅色适配
- [x] 顶部与右侧热区 / 细条 hover 判定
- [x] 离开后收回到微细条

## Comments
已完成并验证。
