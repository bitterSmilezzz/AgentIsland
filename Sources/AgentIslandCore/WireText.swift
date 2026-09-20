import Foundation

// MARK: - 上线前的文本整形（HTTP 头 / SMTP 行）

/// 网络协议里「看着对但到不了对方」的一类错：HTTP 头值必须是 ASCII，
/// URLSession 会**静默丢弃**非 ASCII 字符（实测中文 `X-Title: Qoder · 等待你确认`
/// 到达对端只剩 `Qoder · `——通知最要紧的那几个字没了，且本地看不出来）。
/// SMTP 更严：一行里不许出现裸换行，否则一行变成两条命令。
enum WireText {
    /// 允许出现在 HTTP 头值里的字符（RFC 7230 ttext 的可打印子集，去掉空格与分隔符）
    private static let headerAllowed = CharacterSet(
        charactersIn: "!#$%&'*+-.^_`|~0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
    )

    /// 百分号编码成纯 ASCII 头值；ntfy 等接收端按文档会对头值做 URL 解码。
    /// 空格编成 `%20`（而不是 `+`），非 ASCII 走 UTF-8 字节序列。
    static func headerValue(_ text: String) -> String {
        var out = ""
        out.reserveCapacity(text.count)
        for scalar in text.unicodeScalars {
            if headerAllowed.contains(scalar) {
                out.unicodeScalars.append(scalar)
            } else if scalar.value == 0x20 {
                out += "%20"
            } else {
                for byte in Data(String(scalar).utf8) {
                    out += String(format: "%%%02X", byte)
                }
            }
        }
        return out
    }

    /// 查询串/表单里的值。与头值同一套安全字符集，但空格按 `application/x-www-form-urlencoded`
    /// 编成 `+`。刻意不用 `addingPercentEncoding(withAllowedCharacters: .alphanumerics)`：
    /// 那会把 `-_.~` 与全部子规则以外的 ASCII 也编成 `%XX`，一条 6 字的中文标题
    /// 从 54 字节涨到更大——各家中转服务对 URL 长度都有上限（实测 Server酱 免费版最紧），
    /// 白付一倍以上的长度不值。
    static func formValue(_ text: String) -> String {
        queryValue(text).replacingOccurrences(of: "%20", with: "+")
    }

    /// RFC 3986 unreserved 集，其余一律百分号编码。
    /// 不能复用 `headerValue`：头值允许 `& # % +`，把它们原样放进查询串就是参数注入——
    /// 勾选「附带最后一条动作」后正文里是真命令，`git commit -m a && curl …` 这类
    /// 文本会在接收端被切成两个字段
    static func queryValue(_ text: String) -> String {
        var out = ""
        out.reserveCapacity(text.count)
        for scalar in text.unicodeScalars {
            if queryAllowed.contains(scalar) {
                out.unicodeScalars.append(scalar)
            } else if scalar.value == 0x20 {
                out += "%20"
            } else {
                for byte in Data(String(scalar).utf8) {
                    out += String(format: "%%%02X", byte)
                }
            }
        }
        return out
    }

    private static let queryAllowed = CharacterSet(
        charactersIn: "-._~0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
    )

    /// JSON 字符串字面量（带引号）。自己写转义而不是拼字符串：
    /// 正文里一个裸引号或换行就能让整条 JSON 非法，而接收端只会回一个看不懂的 4xx
    static func jsonString(_ text: String) -> String {
        var out = "\""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            default:
                if scalar.value < 0x20 {
                    out += String(format: "\\u%04X", scalar.value)
                } else {
                    out.unicodeScalars.append(scalar)
                }
            }
        }
        return out + "\""
    }

    /// SMTP 的一行：把裸 CR/LF 折成空格，避免消息里夹一个换行就在协议里多写一条命令
    static func smtpLine(_ text: String) -> String {
        var out = text
        // 先折叠 CRLF 再折叠单字符，否则 "\r\n" 会变成两个空格
        for bad in ["\r\n", "\n", "\r"] {
            out = out.replacingOccurrences(of: bad, with: " ")
        }
        return out
    }
}
