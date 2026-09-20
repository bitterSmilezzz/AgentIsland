import Foundation

// MARK: - 送进 shell / AppleScript 的转义（不可信输入的唯一出口）

/// 会话目录、会话 ID 来自第三方 Agent 的 SQLite 库与日志，属于**不可信输入**。
/// 它们此前被直接拼进 `do script "cd …"` 与 `do shell script "…"`，且只转义了双引号——
/// `;` `|` `&&` `$()` 反引号会照原样进 shell：点一次岛上的「打开终端」即可在用户机器上
/// 执行任意命令（未安装 Agent 的库改版、被截写的日志都足以触发）。
/// 顺带地，含空格的目录在修复前连 `cd` 都会失败。
public enum ShellQuoting {
    /// POSIX shell 单引号词：单引号内除单引号外一切字符都失去语义，结果永远是**一个**词。
    /// 内部单引号按 `'\''`（关引号 → 转义单引号 → 重开）处理，这是唯一不需要知道上下文的写法。
    public static func shellWord(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    /// AppleScript 字符串字面量**内容**转义：反斜杠与双引号加倍，控制字符写成转义序列
    /// （裸换行会让整段脚本报语法错，症状是「点了没反应」而不是注入）。
    public static func appleScriptLiteral(_ value: String) -> String {
        var out = ""
        for scalar in value.unicodeScalars {
            switch scalar {
            case "\\": out += "\\\\"
            case "\"": out += "\\\""
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            default:
                // 其余控制字符（ESC / 换页 / NUL）原样透传：AppleScript 字符串字面量
                // 只在 `"`、`\` 与行终止符上有语法含义。实测（NSAppleScript(source:) 返回 nil
                // 即编译失败）：写成 `\u{1b}` 反而**编译不过**，整条脚本被静默丢弃——
                // 症状正是本函数要消灭的「点了没反应」。
                out.unicodeScalars.append(scalar)
            }
        }
        return out
    }
}
