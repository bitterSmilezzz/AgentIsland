# 消息关闭动画

关闭事件提醒时，旧实现立即从 SwiftUI 树中移除横幅，AppKit 窗口才开始缩短，用户会看到内容硬切和底部 Token 条突然跳动。

现在关闭按钮使用 240ms easeInOut 事务，横幅采用淡出加轻微向顶部收缩的 removal transition；外层列表与窗口高度使用同一时长。这样消息先平滑收束，Token 汇总栏随卡片一起落下，避免视觉上的硬截断。

验证：`swift build --disable-sandbox`、99/99 自建测试、`--selftest`、`git diff --check` 和 Release 签名校验均通过。新版应用已重启，进程正常运行。
