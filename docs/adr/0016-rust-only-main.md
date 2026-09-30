# ADR 0016：Rust/Tauri 是唯一开发主线

状态：接受。日期：2026-09-30。

用户决定将 SwiftUI 端归档，后续开发只在 Rust/Tauri 基础上推进。归档分支 `codex/archive-swiftui` 固定在 v0.0.273，包含完整旧源码、测试、构建脚本及旧 WPF 实现，并已推送远端。查询历史使用该分支，不向其提交后续功能。

main 保留 Rust 核心、Tauri 桌面窗口、静态 Web UI 与 Rust CLI。删除 SwiftPM 清单、Swift 源码、Swift 测试、旧 WPF 端以及专属构建工具；历史 CHANGELOG、ADR 与审查记录作为时点证据保留。

发布包中的 CLI 从同一 Rust release 二进制复制，CLI 分派先于 GUI 初始化；不依赖 Swift 工具链或归档分支。Cargo.toml 为版本源，Cargo.lock、Tauri 配置和交付文档由同一检查器核对。

费率与等待词表的跨端对照转为独立 JSON 契约夹具；已归档的 CSV 安全与异常判据跨端源码检查退出，Rust 的 CSV 安全、GUI 排除与实际状态机用例继续执行。归档不表示 Windows 或 Linux 的移植已完成，平台限制仍按代码与 CI 结果说明。
