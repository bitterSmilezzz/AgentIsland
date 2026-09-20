import AppKit
import Foundation
import AgentIslandCore

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
        // 目录取自第三方会话库：先做成一个 shell 词，再按 AppleScript 字面量转义。
        // 只转义双引号是不够的——`;` `$()` 会照原样进 shell（见 ShellQuoting）。
        // `--` 终止选项：单引号只保证是一个词，不阻止 cd 把以 `-` 开头的路径当选项
        let shellCommand = "cd -- " + ShellQuoting.shellWord(directory)
        let script = """
        tell application "Terminal"
            activate
            do script "\(ShellQuoting.appleScriptLiteral(shellCommand))"
        end tell
        """
        if let appleScript = NSAppleScript(source: script) {
            var error: NSDictionary?
            appleScript.executeAndReturnError(&error)
        }
    }
}
