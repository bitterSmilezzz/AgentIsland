import AppKit
import Foundation

// MARK: - 会话速览与快捷接续 (v0.0.74)

public enum RecentSessionNavigator {

    /// 拷贝会话 ID 至系统剪贴板
    @MainActor
    public static func copySessionId(_ id: String) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(id, forType: .string)
    }

    /// 在终端中快速打开目录或接续会话
    @MainActor
    public static func resumeInTerminal(directory: String?) {
        guard let directory, !directory.isEmpty else { return }
        let script = """
        tell application "Terminal"
            activate
            do script "cd \(directory.replacingOccurrences(of: "\"", with: "\\\""))"
        end tell
        """
        if let appleScript = NSAppleScript(source: script) {
            var error: NSDictionary?
            appleScript.executeAndReturnError(&error)
        }
    }
}
