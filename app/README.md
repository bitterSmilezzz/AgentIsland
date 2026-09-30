# AgentIsland — Rust/Tauri 桌面端

Rust 核心与静态 Web UI，提供灵动岛、侧边栏与工作台三个窗口。三个窗口共用监控状态、用量口径与主题；侧边栏提供监控、用量、Codex 档位、待办、设置、远程通知与 Agent 启停，工作台提供概览与独立功能页，Mac 使用原生窗口控制、系统材质和系统字体。导航底部展示本机状态摘要；概览聚焦核心用量，完整分析和配置使用独立页面。

主发布包是 `dist/AgentIsland.app` 和 Rust CLI `dist/agentisland`；main 只维护 Rust/Tauri，旧端见[归档决策](../docs/adr/0016-rust-only-main.md)。

## 构建与运行

macOS 需要 Rust stable、Command Line Tools 与 cargo-tauri；静态前端不需要 Node/npm。

```sh
cargo test --locked --manifest-path app/src-tauri/Cargo.toml
cd app/src-tauri
cargo tauri build --bundles app,dmg
```

完整验证、打包与发布由仓库根目录 `scripts/release.sh` 执行。

启动参数：`--demo` 演示数据、`--expand` 展开灵动岛、`--route=tokenAnalytics` / `--route=agentDetail:<id>` 直达内容；`--shell=sidebar` / `--shell=workbench` 选择桌面形态。重复启动显示已有工作台，CLI 子命令可以独立运行。

## 前端目录

- `ui/index.html`：首次绘制前设置形态类。
- `ui/css/tokens.css`：三种形态共用的深浅主题变量。
- `ui/css/island.css`：灵动岛几何与内容。
- `ui/css/sidebar.css`：侧栏导航与布局。
- `ui/css/panels.css`：侧栏与工作台共用的档位、待办与表单控件。
- `ui/css/workbench.css`：工作台导航、概览与独立内容页的响应式布局。
- `ui/js/views.js`：页面内容及交互；`main.js`：状态、启动与采样订阅。

## 平台边界

Windows 整体应用仍有 POSIX 进程、主机名、文件权限及条件编译缺口，不能把构建工具安装说明当作可用保证。本地日历接口使用 Windows CRT 安全接口；其独立编译检查不代表整体应用已经可构建。POSIX 用法由 `posix_port_ratchet` 限制增长。

Linux 打包与真机交互尚未验证。跨屏与混合 DPI 需要各平台真实设备验收。系统钥匙串实现限 macOS，其他平台返回不支持；远程通知的锁屏与显示器睡眠在场信号尚未接入。
