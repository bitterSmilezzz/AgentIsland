import Foundation
@testable import AgentIslandCore

// MARK: - 展示格式化纯函数（紧凑 token / 金额 / 时间相对量 / SQL 转义 / 日志尾读）

@MainActor
enum FormatTests {

    static func register() {

        TestKit.test("格式化: TokenUsage.compact 分档与边界（含临界进位）") {
            try expectEqual(TokenUsage.compact(0), "0", "零")
            try expectEqual(TokenUsage.compact(-5), "-5", "负数原样输出（无符号不臆造）")
            try expectEqual(TokenUsage.compact(9_999), "9999", "万以下原样")
            try expectEqual(TokenUsage.compact(10_000), "10.0k", "万起进 k 档")
            try expectEqual(TokenUsage.compact(45_600), "45.6k", "k 档一位小数")
            try expectEqual(TokenUsage.compact(999_999), "1000.0k",
                            "999999 仍属 k 档，%.1f 进位成 1000.0k（不越档改写）")
            try expectEqual(TokenUsage.compact(1_000_000), "1.00M", "百万起进 M 档")
            try expectEqual(TokenUsage.compact(1_234_567), "1.23M", "M 档两位小数")
            try expectEqual(TokenUsage.compact(999_999_999), "1000.00M",
                            "999999999 仍属 M 档，进位成 1000.00M")
            try expectEqual(TokenUsage.compact(1_000_000_000), "1.00B", "十亿起进 B 档")
            try expectEqual(TokenUsage.compact(1_647_916_622), "1.65B", "B 档两位小数")
        }

        TestKit.test("格式化: TokenUsage.cost 零/负数返回空串，<$0.01 与两位小数") {
            try expectEqual(TokenUsage.cost(0), "", "0 无金额可显示 → 空串")
            try expectEqual(TokenUsage.cost(-1), "", "负数 → 空串")
            try expectEqual(TokenUsage.cost(0.001), "<$0.01", "不足一分")
            try expectEqual(TokenUsage.cost(0.0099), "<$0.01", "不足一分（分界前）")
            try expectEqual(TokenUsage.cost(0.01), "$0.01", "恰好一分")
            try expectEqual(TokenUsage.cost(1.234), "$1.23", "两位小数截断")
            try expectEqual(TokenUsage.cost(12.3456), "$12.35", "两位小数四舍五入")
            try expectEqual(TokenUsage.cost(1_000), "$1000.00", "大额不加分隔符")
        }

        TestKit.test("格式化: formatAgo 相对时间分档（4.4/5/59.6/60/3599/3600/负/nil）") {
            try expectEqual(ActivityEngine.formatAgo(nil), "—", "从未活跃")
            try expectEqual(ActivityEngine.formatAgo(-10), "刚刚", "负数（时钟回拨）也按刚刚处理，不出现负值")
            try expectEqual(ActivityEngine.formatAgo(0), "刚刚", "0 秒")
            try expectEqual(ActivityEngine.formatAgo(4.4), "刚刚", "4.4 秒四舍五入为 4 → 刚刚")
            try expectEqual(ActivityEngine.formatAgo(4.6), "5s 前", "4.6 秒四舍五入为 5 → 进入秒档")
            try expectEqual(ActivityEngine.formatAgo(5), "5s 前", "5 秒整")
            try expectEqual(ActivityEngine.formatAgo(59), "59s 前", "59 秒")
            try expectEqual(ActivityEngine.formatAgo(59.6), "1m 前",
                            "59.6 秒进位到 60 → 分钟档（显示 1m 前 而非 60s 前）")
            try expectEqual(ActivityEngine.formatAgo(60), "1m 前", "60 秒进分钟档")
            try expectEqual(ActivityEngine.formatAgo(3_599), "59m 前", "3599 秒不足一小时")
            try expectEqual(ActivityEngine.formatAgo(3_600), "1h 前", "3600 秒进小时档")
            try expectEqual(ActivityEngine.formatAgo(86_399), "23h 前", "24h 内")
            try expectEqual(ActivityEngine.formatAgo(172_800), "48h 前", "超过一天仍以小时累计，不切换到「天」")
        }

        TestKit.test("格式化: String.escaped SQL 转义（单引号翻倍）") {
            try expectEqual("".escaped, "", "空串")
            try expectEqual("plain".escaped, "plain", "无引号不变")
            try expectEqual("it's".escaped, "it''s", "单个引号翻倍")
            try expectEqual("a'b'c".escaped, "a''b''c", "多个引号各自翻倍")
            try expectEqual("' OR 1=1 --".escaped, "'' OR 1=1 --", "注入式输入被转义")
        }

        TestKit.test("格式化: LogTailReader 空文件/无换行/部分行/超长行") {
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: dir) }
            let file = dir.appendingPathComponent("tail.log")

            // 文件不存在 / 空文件
            try expectTrue(LogTailReader.read(from: file, maxLines: 10, maxBytes: 100).isEmpty, "文件不存在 → 空")
            try Data().write(to: file)
            try expectTrue(LogTailReader.read(from: file, maxLines: 10, maxBytes: 100).isEmpty, "空文件 → 空")

            // 非法预算
            try Data("OK\n".utf8).write(to: file)
            try expectTrue(LogTailReader.read(from: file, maxLines: 0, maxBytes: 100).isEmpty, "maxLines 0 → 空")
            try expectTrue(LogTailReader.read(from: file, maxLines: -1, maxBytes: 100).isEmpty, "maxLines 负 → 空")
            try expectTrue(LogTailReader.read(from: file, maxLines: 10, maxBytes: 0).isEmpty, "maxBytes 0 → 空")

            // 无换行的末条（正在写入的整行）
            try Data("最后一行没有换行".utf8).write(to: file)
            try expectEqual(LogTailReader.read(from: file, maxLines: 10, maxBytes: 1_000),
                            ["最后一行没有换行"], "无换行末条也算一条")

            // 部分行（尾读窗口落在行中间且窗口内无完整行）→ 丢弃
            try Data("abc\ndef".utf8).write(to: file)
            try expectTrue(LogTailReader.read(from: file, maxLines: 10, maxBytes: 2).isEmpty,
                           "窗口只覆盖 'ef'（无行边界）→ 不得伪造半条记录")

            // 超长行 + 后续完整行：只保留完整行
            let longLine = String(repeating: "x", count: 200)
            try Data((longLine + "\nOK\n").utf8).write(to: file)
            try expectEqual(LogTailReader.read(from: file, maxLines: 10, maxBytes: 10), ["OK"],
                            "超长行被截断后丢弃，保留其后的完整行")

            // maxLines 预算：只取最后 N 条
            try Data("1\n2\n3\n4\n".utf8).write(to: file)
            try expectEqual(LogTailReader.read(from: file, maxLines: 2, maxBytes: 1_000), ["3", "4"], "只取末尾 2 条")

            // CRLF 与空行
            try Data("甲\r\n\r\n乙\r\n".utf8).write(to: file)
            try expectEqual(LogTailReader.read(from: file, maxLines: 10, maxBytes: 1_000), ["甲", "乙"],
                            "CRLF 与空行都不产生空条目")

            // 末行不完整 UTF-8 不得遮蔽前面的完整行
            try (Data("OK\n".utf8) + Data([0xe4, 0xb8])).write(to: file)
            try expectEqual(LogTailReader.read(from: file, maxLines: 10, maxBytes: 1_000), ["OK"],
                            "损坏的末行被丢弃，完整行保留")
        }
    }
}
