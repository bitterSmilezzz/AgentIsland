import Foundation
import Darwin

// MARK: - ANSI 终端色彩支持

public enum CLIColor {
    public static var isTTY: Bool = isatty(STDOUT_FILENO) != 0

    public static func wrap(_ text: String, code: String) -> String {
        guard isTTY else { return text }
        return "\u{001B}[\(code)m\(text)\u{001B}[0m"
    }

    public static func bold(_ text: String) -> String { wrap(text, code: "1") }
    public static func dim(_ text: String) -> String { wrap(text, code: "2") }
    public static func red(_ text: String) -> String { wrap(text, code: "31") }
    public static func green(_ text: String) -> String { wrap(text, code: "32") }
    public static func yellow(_ text: String) -> String { wrap(text, code: "33") }
    public static func blue(_ text: String) -> String { wrap(text, code: "34") }
    public static func magenta(_ text: String) -> String { wrap(text, code: "35") }
    public static func cyan(_ text: String) -> String { wrap(text, code: "36") }
    public static func white(_ text: String) -> String { wrap(text, code: "37") }
}

// MARK: - 终端表格排版

public struct CLITable {
    public struct Column {
        public let title: String
        public let minWidth: Int
        public let alignRight: Bool

        public init(_ title: String, minWidth: Int = 4, alignRight: Bool = false) {
            self.title = title
            self.minWidth = minWidth
            self.alignRight = alignRight
        }
    }

    public let columns: [Column]
    public var rows: [[String]] = []

    public init(columns: [Column]) {
        self.columns = columns
    }

    public mutating func addRow(_ row: [String]) {
        rows.append(row)
    }

    public func render() -> String {
        guard !columns.isEmpty else { return "" }

        // 计算每列最大宽度
        var widths = columns.map { max($0.minWidth, stringDisplayWidth($0.title)) }
        for row in rows {
            for (idx, cell) in row.enumerated() where idx < widths.count {
                widths[idx] = max(widths[idx], stringDisplayWidth(stripANSI(cell)))
            }
        }

        var lines: [String] = []

        // 表头
        var headerCells: [String] = []
        for (i, col) in columns.enumerated() {
            headerCells.append(pad(col.title, width: widths[i], right: col.alignRight))
        }
        lines.append(CLIColor.bold(headerCells.joined(separator: "  ")))

        // 分割线
        let separator = widths.map { String(repeating: "─", count: $0) }.joined(separator: "  ")
        lines.append(CLIColor.dim(separator))

        // 数据行
        for row in rows {
            var rowCells: [String] = []
            for (i, col) in columns.enumerated() {
                let cell = i < row.count ? row[i] : ""
                let rawLen = stringDisplayWidth(stripANSI(cell))
                let padCount = max(0, widths[i] - rawLen)
                let padding = String(repeating: " ", count: padCount)
                if col.alignRight {
                    rowCells.append(padding + cell)
                } else {
                    rowCells.append(cell + padding)
                }
            }
            lines.append(rowCells.joined(separator: "  "))
        }

        return lines.joined(separator: "\n")
    }

    private func pad(_ text: String, width: Int, right: Bool) -> String {
        let len = stringDisplayWidth(stripANSI(text))
        let padCount = max(0, width - len)
        let space = String(repeating: " ", count: padCount)
        return right ? (space + text) : (text + space)
    }

    private func stripANSI(_ text: String) -> String {
        text.replacingOccurrences(of: #"\u{001B}\[[0-9;]*m"#, with: "", options: .regularExpression)
    }

    private func stringDisplayWidth(_ text: String) -> Int {
        var width = 0
        for scalar in text.unicodeScalars {
            // 变体选择符不占显示宽度
            if scalar.value == 0xFE0E || scalar.value == 0xFE0F {
                continue
            }
            // 表情符号及杂项符号（通常占用 2 个终端单元格）
            if (scalar.value >= 0x1F300 && scalar.value <= 0x1FAFF) ||
               (scalar.value >= 0x2600 && scalar.value <= 0x27BF) {
                width += 2
            }
            // CJK 统一表意文字与标点、全角字符
            else if (scalar.value >= 0x4E00 && scalar.value <= 0x9FFF) ||
                    (scalar.value >= 0x3400 && scalar.value <= 0x4DBF) ||
                    (scalar.value >= 0x20000 && scalar.value <= 0x2A6DF) ||
                    (scalar.value >= 0x3000 && scalar.value <= 0x303F) ||
                    (scalar.value >= 0xFF01 && scalar.value <= 0xFF60) {
                width += 2
            } else {
                width += 1
            }
        }
        return width
    }
}
