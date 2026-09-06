# 01 — 立 seam：token 轮询/查询双协议 + 引擎注入 + Core fake

**What to build:** token 子系统获得测试 seam——现有 token 实现同时满足「轮询面」（启停/暂停恢复/缓存读取/刷新回调）与「查询面」（两个下钻查询）两个小协议；引擎构造新增 token 侧注入参数（默认现网实现，所有现有调用点零改动）；Core 提供与既有进程/文件 fake 并列的 token fake；引擎测试首次可用 fake 经公开 interface 驱动 token 面。本票是纯 expand：现网行为零变化，UI 穿透点暂不动。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [ ] 引擎以默认参数构造时行为与主线完全一致（现有测试全绿）
- [ ] 引擎测试注入 fake 后，可经公开 interface 断言 start/stop 对轮询面的驱动
- [ ] fake 不触真实 SQLite/文件系统，无后台线程等待
- [ ] `swift build` + 测试 runner 全绿

## Comments

- 2026-09-06 发布（由 spec 拆票，expand–contract 第一步）。
- 2026-09-06 完成（7519358 配置 / ba577b0 代码）。36/36 绿；审查双轴通过；60s 默认节律收敛为 TokenUsagePollingDefaults.interval。
