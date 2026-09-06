# 01 — InstalledAppsCache 注入化 + Registry 去全局化

**What to build:** 安装缓存成为注入实例：InstalledAppsCache（扫描器闭包注入 + isInstalled + refreshIfNeeded(maxAge:completion:) 调度即标记）；组合根创建注入引擎与设置页；引擎 init 首刷 + 完成后重放 enabledIDs、300s 周期走 refreshIfNeeded；AgentRegistry 的静态锁/缓存/扫描函数整体迁出（discoverCLIProfiles/fullRegistry/profile 参数化、conflict 便捷入口带 cache）；markInstalledRefreshed 与 AppContext/设置页两处「刷新+打标」舞步删除；测试全部改注入 canned 扫描器。

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] AgentRegistry 零全局安装状态（installedLock/cached*/refreshInstalledCache/scanInstalled* 迁出）
- [ ] markInstalledRefreshed 删除；AppContext 首刷舞步整段删除（组合根只 new cache 并注入）
- [ ] 首刷完成后自动发现项恢复监控（引擎重放 enabledIDs，测试覆盖）
- [ ] 测试零真实文件系统扫描（canned 注入；refreshIfNeeded 跳过语义有用例）
- [ ] `swift build` + runner 全绿 + `--probe` 冒烟

## Comments

- 2026-09-06 发布。单票理由：Registry 签名变化扇出全部调用方，expand–contract 拆票无法单独保绿。
