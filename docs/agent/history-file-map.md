# AgentIsland 历史资料与路径映射

2026-10-14 核对。只涉及 AgentIsland 的历史资料，不管理或修改其他项目。旧记录保留当时的路径、行号、结论与失败，当前入口通过本表解释位置。

## 1. 历史源码

当前 main 唯一实现为 Rust/Tauri；Swift/WPF 冻结在 `codex/archive-swiftui`，本地分支 tip 为 `ee0f075` / v273，见 [ADR 0016](../adr/0016-rust-only-main.md)。

| 旧 main 路径 | 查看方法 | 用途 |
| --- | --- | --- |
| `Sources/AgentIslandCore/` | `git show codex/archive-swiftui:Sources/AgentIslandCore/<文件>.swift` | 旧监控、解析、统计与存储 |
| `Sources/AgentIsland/` | `git show codex/archive-swiftui:Sources/AgentIsland/<文件>.swift` | 旧 SwiftUI/AppKit 应用 |
| `Sources/AgentIslandCLI/` | `git show codex/archive-swiftui:Sources/AgentIslandCLI/main.swift` | 旧 CLI |
| `Tests/` | `git ls-tree -r --name-only codex/archive-swiftui -- Tests` | 旧测试 runner 与夹具 |
| `windows/` | `git ls-tree -r --name-only codex/archive-swiftui -- windows` | 旧 WPF 对照端 |

[两端迁移表](../workbench/23-swift-rust-parity-matrix.md)中 43 个不同 `Sources/` 目标均在该分支找到。正文中的旧 Rust 行号也只是调查时点，不能直接用于当前定位。

需要精确对应某轮报告时，优先用该报告基线/tag：`git show <当时提交>:<仓库内路径>`。归档 tip 适合查最后冻结实现，不能保证与每轮旧行号一致。以上操作只读，无须切分支或覆盖当前工作树。

## 2. 已迁出的 AgentIsland 取证文件

早期 `docs/research/<文件>` 或同级 `../ui-research/<文件>` 的引用，现定位到工作区 `03-行业调研/ui-research/`。下列链接从本文件出发，已经逐项校验存在；只作为本项目证据引用，未修改该调研库。

- [2026-09-26-agent-session-paths.md](../../../../03-行业调研/ui-research/2026-09-26-agent-session-paths.md)
- [2026-09-27-codex-provider-config.md](../../../../03-行业调研/ui-research/2026-09-27-codex-provider-config.md)
- [2026-09-27-jsonl-net-token-formula.md](../../../../03-行业调研/ui-research/2026-09-27-jsonl-net-token-formula.md)
- [2026-09-27-macos-tls-for-rust-outbound.md](../../../../03-行业调研/ui-research/2026-09-27-macos-tls-for-rust-outbound.md)
- [2026-09-27-opencode-schema-and-real-db-check.md](../../../../03-行业调研/ui-research/2026-09-27-opencode-schema-and-real-db-check.md)
- [2026-09-27-session-probe-parity-check.md](../../../../03-行业调研/ui-research/2026-09-27-session-probe-parity-check.md)
- [2026-09-28-report-token-shape-check.md](../../../../03-行业调研/ui-research/2026-09-28-report-token-shape-check.md)
- [2026-09-28-statusindex-real-db-check.md](../../../../03-行业调研/ui-research/2026-09-28-statusindex-real-db-check.md)
- [2026-09-29-generic-detector-port-check.md](../../../../03-行业调研/ui-research/2026-09-29-generic-detector-port-check.md)

白屏与窗口测量原始记录亦已迁出：

- [白屏死锁](../../../../03-行业调研/ui-research/2026-09-29-blank-ui-deadlock.md)
- [曾被撤回的 SDK/CSP 因果](../../../../03-行业调研/ui-research/2026-09-29-blank-ui-two-build-config-causes.md)
- [窗口尺寸误诊记录](../../../../03-行业调研/ui-research/2026-09-29-island-window-never-resizes.md)

独立克隆本仓时外部工作区链接可能不可用；可先查本仓 CHANGELOG 对应版本与 Git 历史，不能因此宣称从未有取证。

## 3. 历史文档如何使用

| 集合 | 身份 | 使用边界 |
| --- | --- | --- |
| `CHANGELOG.md` / Git tags | 逐版变化与更正 | 连同后续撤回一起读；tag 不等于完整平台验收 |
| `docs/validation/` | 47 份 Swift 时期轮次报告 | 保留旧测量与遗留，不套为当前 Rust 状态 |
| `docs/code-review/` | 195 份时间点审查 | 保留原文和旧链接，查当时基线；本表定位迁出资料 |
| workbench 01/03/04/07/10/20/21/23 | 早期快照、计划与迁移对照 | 先看编写时间；旧缺口须重新查代码 |
| `docs/adr/` | 决策与修订依据 | 区分冻结技术、当前规则与后续窄例外；具体实现须查代码 |
| workbench 25/26 + implementation | 当前整体规划与分项实施记录 | 计划完成、源码存在、原生通过和整体交付分别核验 |
| workbench 27/28/29 | 新测试设计、范围收敛、并行竞品材料 | 27 未实现；28 以 ADR 0039 为准；29 与四态冲突须显式保留 |

没有移动、删除或重写历史审查报告，也没有处理根目录历史 zip、`.build/`、`dist/`、`artifacts/` 或 `.scratch/`。这些产物可能包含个人数据或被其他会话使用，不在本次文档梳理的清理范围。
