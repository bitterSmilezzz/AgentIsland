# 01 — ProcessMonitor 吸收 + collapseDelay 迁出 EngineConfig + 残留文件删除

**What to build:** 引擎直持 ProcessProviding（ProcessMonitor struct 删除，线程契约注释上协议）；EngineConfig 回归纯采样参数（collapseDelay 字段/load 读取删除），IslandPanel 持有 collapseDelay 并经 applyCollapseDelay 直调（键沿用 SettingKey.collapseDelay，设置页滑块松手直调）；MenuBarView.swift 删除。

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] ProcessMonitor struct 不存在；引擎全部调用点经 ProcessProviding（Fake/Provider 直传）
- [ ] EngineConfig 无 collapseDelay；设置页滑块调整仍实时生效（持久化 + 面板行为）
- [ ] MenuBarView.swift 删除，构建不受影响
- [ ] `swift build` + runner 全绿 + `--probe` 冒烟

## Comments

- 2026-09-06 发布（弹窗未答，按推荐执行）。
