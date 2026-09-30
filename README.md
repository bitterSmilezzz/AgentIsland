# AgentIsland

本机 AI 编码智能体的轻量工作台，监控运行状态、当前动作与 token 用量。

**main 使用 Rust/Tauri 开发。** Rust 负责监控、会话解析、配置与 CLI；静态 Web UI 提供灵动岛、贴边侧栏及工作台，macOS 使用系统窗口控制和材质。SwiftUI 与旧 WPF 实现保存在 [归档分支](https://github.com/bitterSmilezzz/AgentIsland/tree/codex/archive-swiftui)，归档规则见 [ADR 0016](docs/adr/0016-rust-only-main.md)。

> 本文档描述 **v0.0.277** 的行为；版本改动见 [CHANGELOG.md](CHANGELOG.md)。

## 功能

- **灵动岛**：小型贴边把手、实时智能体列表、当前动作、用量摘要；支持搜索、详情与用量分析，离开后自动收起。
- **侧边栏**：持续显示监控、用量、档位、待办、设置、远程通知和智能体管理；左右贴边与宽度可配置。
- **工作台**：分组导航、概览与独立功能页；复杂表单进入独立页，采样不会重置正在输入的内容。
- **监控**：进程、文件与会话证据共同判定运行状态；会话源读不到、无本地明细和未接入明细源分别呈现，详情与 doctor 共用诊断口径。
- **用量**：读取本机 JSONL / SQLite 等记录，统计最近 24 小时与累计净消耗；缓存读取不重复计入，费用估算用 `~` 标记。
- **Codex 档位**：管理本机模型与 provider 配置、切换和备份；档位只保存环境变量名，密钥由环境或系统钥匙串提供。
- **待办与报告**：本机待办列表、Markdown / CSV 用量报告与复制；CLI 可导出文件。
- **通知**：确认、完成和成本事件提示；远程通知可选，默认关闭。

## 安装与使用

从 [Releases](https://github.com/bitterSmilezzz/AgentIsland/releases/latest) 下载发布包。macOS 包含 `AgentIsland.app` 和 Rust CLI `agentisland`；应用未公证，首次运行使用右键 → 打开。

托盘可打开工作台。灵动岛把手可通过悬停、点击或键盘 Enter / 空格展开，四个方向均可贴边；设置中的“收起时隐藏微细条”隐藏视觉元素并保留唤回热区。重复启动复用已有应用实例。

```sh
./agentisland --shell=workbench
./agentisland --shell=sidebar
./agentisland --expand
./agentisland --demo
./agentisland help
./agentisland status --json
./agentisland doctor
./agentisland report --format csv
```

CLI 命令的参数与边界以 `agentisland help` 为准。清理命令会终止进程，请先查看检查结果并核实目标；单次采样的死锁判定为“本次未评估”，持续监控才积累足够观测证据。

## 构建与验证

macOS 需要 Rust stable、Command Line Tools、Python 3 和 cargo-tauri；前端不需要 Node/npm 构建。Rust 测试中的 JS 守护需要 Node.js。

```sh
scripts/install-git-hooks.sh
cargo test --locked --manifest-path app/src-tauri/Cargo.toml
scripts/build-app.sh
scripts/restart-app.sh
```

构建产物为 `dist/AgentIsland.app` 和 `dist/agentisland`，二者都使用 Rust；发布流程由 `scripts/release.sh <X.Y.Z> "<标题>"` 执行扫描、回归、打包、UI 冒烟、单实例检查、提交、tag 和 Release。版本校验覆盖 Cargo、锁文件、Tauri 配置、CHANGELOG 和本 README。

## 工程结构

- `app/src-tauri/src/`：Rust 监控、业务、Tauri 命令与 CLI。
- `app/src-tauri/tests/fixtures/`：会话与稳定契约夹具。
- `app/ui/`：三种窗口共用的静态界面与主题。
- `scripts/`：构建、验证、脱敏与发布。
- `docs/adr/` 与 `CONTEXT.md`：长期决策及领域口径。
- `docs/workbench/`：跨平台工作台计划与迁移取证记录。
- `site/`：对外功能主页。

## 当前限制

- macOS 是当前完整验证的交付平台；Windows 仍有 POSIX 适配工作，CI 的 Windows 检查作为可见的非阻塞结果。
- 用量覆盖取决于工具是否提供可读取的本机会话记录；云端用量和缓存读取消耗不能凭本地净消耗推导。
- 档位切换只修改本机 Codex 配置，不能保证目标厂商、账号或模型可用；运行中的会话需要重启才能读取配置。
- 应用没有后台云端同步；远程通知须自行配置。Linux 未作完整交付验证。
