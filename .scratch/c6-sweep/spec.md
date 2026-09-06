# Spec: C6 — 清扫包

> 产出：/improve-codebase-architecture 候选 C6 → grilling 三问（弹窗未答，按推荐执行——本会话既定模式），2026-09-06。
> 原清单「SettingsView 未用 controller 属性」已由 C3 顺带解决（外观直调使用中）。

## Problem Statement

一批低危但确定的杂散：自称「兼容封装」的 ProcessMonitor 透传壳（删除测试不过）；纯 UI 旋钮 collapseDelay 住在 Core 的 EngineConfig；卡片外壳/hover 行/拖动手势/分割线/loading 行各有 2-4 份复制品；少量 Theme 外硬编码色值；MenuBarView.swift 6 行注释残留文件。

## Solution

透传壳吸收（引擎直持 ProcessProviding，线程契约上协议注释）；collapseDelay 迁出 EngineConfig（面板持有 + 直调，模式同外观）；样式收敛为 CardShell/HoverRow/拖动 modifier/DarkDivider/LoadingRow 单一实现；Theme 补残留令牌；残留文件删除。

## User Stories

1. As a 维护者, I want 引擎只依赖 ProcessProviding 协议, so that 不再有通不过删除测试的中间层
2. As a 协议消费者, I want 线程契约写在 interface 注释里, so that 主线程约束显式可见
3. As a 维护者, I want EngineConfig 只含采样参数, so that Core config 不被 UI 旋钮污染
4. As a 后续 agent, I want 同一视觉语义只有一份实现, so that 改样式不漏改复制品（批次11「滚动回归」的复制品各自为政根源）
5. As a 用户, I want 界面逐像素不变（行为保留，纯结构收敛）

## Implementation Decisions

- 删除 ProcessMonitor struct；引擎 init 参数改 `processProvider: ProcessProviding`；matcher 装配内联；ProcessProviding 协议方法补线程契约注释（runningBundleIDs 须主线程 / snapshot 任意线程）
- EngineConfig 删 collapseDelay 字段与 load() 读取；IslandPanel 持有（init 读 SettingKey.collapseDelay，默认 0.5）+ applyCollapseDelay(_:) 直调；SettingsView collapseDelay 滑块松手回调直调面板
- IslandComponents.swift 新文件：CardShell（width/maxFrame/GlassCardBackground 三连）、HoverRow（hover 态圆角行）、DarkDivider、LoadingRow、cardDrag modifier
- Theme 补令牌：dockedSliver 填充/描边（0.55/0.34/0.85/0.22）、docked 蒙层 0.42、面板阴影参数（0.30/16/-6）
- queryToken 代际防串页模式不抽（逻辑非样式）；MenuBarView.swift 删除

## Testing Decisions

- UI target 不可测（C2 既定）：build + runner 全绿 + --probe 冒烟 + 三路由/详情页视觉验收
- EngineConfig 字段删减由现有 Core 测试守护（SettingsTests/EngineTests 全绿即等价）

## Out of Scope

- queryToken 代际模式抽取；线程编排彻底隐藏（C5）；已收敛的 280/176/300（C2）

## Tickets

- 01：ProcessMonitor 吸收 + collapseDelay 迁移 + MenuBarView 删除（无阻塞）
- 02：样式收敛 + Theme 令牌（无阻塞；与 01 无依赖，排后减小审查面）
