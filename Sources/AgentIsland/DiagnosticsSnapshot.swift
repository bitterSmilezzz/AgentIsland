import AppKit
import AgentIslandCore

/// 「复制诊断快照」的唯一入口。
///
/// 此前有三处各自拼装并写剪贴板：右键菜单（`IslandView`）、工作台卡片（`ToolboxView`）、
/// 深度链接 `agentisland://export`（`URLSchemeRouter`）。它们调用同一个
/// `generateMarkdown`，但右键菜单那一处漏传了 `history:` —— 同一个用户动作在两个入口
/// 产出两份不同内容，且没有任何一处会报错。现在三处走同一份实现。
@MainActor
enum DiagnosticsSnapshot {

    static func markdown(from engine: ActivityEngine, now: Date = Date()) -> String {
        AuditReportExporter.generateMarkdown(snapshots: engine.snapshots,
                                             history: engine.eventHistory,
                                             now: now)
    }

    /// 生成并写入系统剪贴板；返回文本供调用方做反馈态（如工作台的「已复制」高亮）
    @discardableResult
    static func copyToPasteboard(from engine: ActivityEngine, now: Date = Date()) -> String {
        let text = markdown(from: engine, now: now)
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        return text
    }
}
