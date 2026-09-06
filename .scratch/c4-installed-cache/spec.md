# Spec: C4 — 安装缓存注入化，根除启动时序补丁

> 产出：/improve-codebase-architecture 候选 C4 → grilling 四问全按推荐（引擎唯一驱动；扫描器注入；Registry 去全局化；SettingsView 显式注入），2026-09-06。

## Problem Statement

注册表安装缓存是全局静态可变状态（static NSLock + 双缓存）：interface 藏着「初始空集、必须等首次后台刷新」的时序约定，引擎被迫暴露 markInstalledRefreshed() 补丁方法让外部告知「缓存已热」，AppContext 与设置页各自编排「刷新 + 打标记」舞步——「启动双扫」在批次 16/17/18 修了三轮。测试只能打真实文件系统与全局状态。

## Solution

InstalledAppsCache 实例化（扫描器闭包注入：生产真实扫描 / 测试 canned；refreshIfNeeded(maxAge:) 调度即标记，时序约定内聚），组合根创建并注入引擎与设置页；引擎成为唯一驱动（init 首刷、300s 周期、首刷后重放 enabledIDs 恢复自动发现监控）；AgentRegistry 全局安装状态整体迁出（discoverCLIProfiles/fullRegistry/profile 参数化，回归无状态）；markInstalledRefreshed 与两处舞步删除。

## User Stories

1. As a 引擎, I want 独占安装缓存的刷新时机, so that 「双扫/漏扫」类回归只有一个家
2. As a 测试, I want 注入 canned 扫描器, so that 安装判定零真实文件系统
3. As a 组合根, I want 创建缓存并注入, so that 时序编排知识不再靠注释传递
4. As a 维护者, I want markInstalledRefreshed 补丁方法删除, so that 「缓存已热」由缓存自身状态表达
5. As a 用户, I want 首次扫描完成后自动发现项恢复监控（行为保留，机制从 AppContext 舞步变为引擎重放）
6. As a 设置页, I want 打开即触发重扫并拿到完成回调, so that 自动发现列表刷新机制不变（installedScanVersion 保留）

## Implementation Decisions

- InstalledAppsCache：clis/bundles 存储 + 扫描闭包注入（默认真实 PATH/补扫目录 + /Applications plist）+ isInstalled(profile) + refreshIfNeeded(maxAge:completion:)（调度即标记；completion 主线程回调）+ refresh()
- 引擎：注入 `installedApps`；init 调 refreshIfNeeded(maxAge: 0)（首刷）并在完成回调重放 lastEnabledIDs（setEnabled 记录）；sampleCore 300s 周期改 refreshIfNeeded(maxAge: 300)；installedCLIs/installedBundles/refreshInstalled/refreshInstalledInBackground/lastInstalledRefresh/markInstalledRefreshed 六成员全删
- AgentRegistry：scanInstalled*/knownCLIs/knownBundleIDs 迁入缓存类；discoverCLIProfiles(installedCLIs:)、fullRegistry(installedCLIs:)、profile(id:installedCLIs:) 参数化（无默认值）；conflictingProcessNames 便捷入口改带 cache 参数；全局锁与缓存删除
- AppContext：创建 `installedApps` 常量并注入 engine/SettingsView；背景首刷舞步整段删除；enabledOrDefault 保留
- SettingsView：显式注入 installedApps；onAppear 重扫改 refreshIfNeeded(maxAge: 0)；discoverCLIProfiles 经注入缓存取数；installedScanVersion 机制不变

## Testing Decisions

- 缓存：canned 扫描器下 refresh 后 isInstalled 判定；refreshIfNeeded 首次必触发、maxAge 内跳过（scanner 调用计数）；completion 回调送达
- 引擎：canned cache 注入下快照 installed 标记；首刷完成后自动发现重放（引擎测试）
- RegistryTests：discover/fullRegistry 用例传 canned installed 集，不再依赖全局缓存
- `swift build` + runner 全绿 + `--probe` 冒烟

## Out of Scope

- 引擎 300s 周期数值调整；设置页 installedScanVersion 机制重设计
- C5（Sampler）的采样节律重组

## Tickets

- 01：InstalledAppsCache + Registry 去全局化 + 全调用方接线（单垂直切片：Registry 签名变化扇出全部调用方，拆票无法单独保绿）
