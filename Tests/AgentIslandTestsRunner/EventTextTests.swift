import Foundation
@testable import AgentIslandCore

// MARK: - 事件文案（summaryText / copyableDiagnosticText）
//
// 事件栏与「复制诊断信息」共用这两处文案：文案错 = 用户看到的事件内容错。

@MainActor
enum EventTextTests {

    private static func makeEvent(_ type: AgentTaskEvent.EventType, duration: TimeInterval = 0,
                                  pid: Int32? = nil, message: String? = nil,
                                  detail: String? = nil) -> AgentTaskEvent {
        AgentTaskEvent(agentId: "fixture-agent", agentName: "Fixture",
                       eventType: type, duration: duration,
                       timestamp: Date(timeIntervalSince1970: 1_700_000_000),
                       pid: pid, message: message, detail: detail)
    }

    static func register() {

        TestKit.test("事件文案: completed 时长文案 N秒 / N分M秒 边界") {
            try expectEqual(makeEvent(.completed, duration: 0).summaryText, "Fixture 任务完成 (0秒)", "0 秒")
            try expectEqual(makeEvent(.completed, duration: 59).summaryText, "Fixture 任务完成 (59秒)", "59 秒不分钟")
            try expectEqual(makeEvent(.completed, duration: 60).summaryText, "Fixture 任务完成 (1分0秒)", "60 秒整分钟")
            try expectEqual(makeEvent(.completed, duration: 61).summaryText, "Fixture 任务完成 (1分1秒)", "61 秒")
            try expectEqual(makeEvent(.completed, duration: 3_599).summaryText, "Fixture 任务完成 (59分59秒)", "59分59秒")
            try expectEqual(makeEvent(.completed, duration: 3_600).summaryText, "Fixture 任务完成 (60分0秒)", "整小时不进位到时分")
        }

        TestKit.test("事件文案: attention / costSpike 固定文案") {
            try expectEqual(makeEvent(.attention).summaryText, "Fixture 等待确认操作", "等待确认")
            try expectEqual(makeEvent(.costSpike).summaryText, "⚠️ Fixture 资源/Token 消耗突增", "消耗突增")
        }

        TestKit.test("事件文案: message 优先；nil 与空串都回落到按类型生成") {
            try expectEqual(makeEvent(.completed, duration: 10, message: "自定义摘要").summaryText,
                            "自定义摘要", "有 message 时优先展示")
            try expectEqual(makeEvent(.completed, duration: 10, message: nil).summaryText,
                            "Fixture 任务完成 (10秒)", "nil 回落")
            try expectEqual(makeEvent(.attention, message: "").summaryText,
                            "Fixture 等待确认操作", "空串视同缺失，回落到类型文案")
            try expectEqual(makeEvent(.costSpike, message: "   ").summaryText,
                            "   ", "纯空白不算空串（保持原样透传）")
        }

        TestKit.test("事件文案: 复制诊断信息含类型/时间/摘要，PID 与详情行按存在与否出现") {
            let bare = makeEvent(.completed, duration: 5)
            let bareLines = bare.copyableDiagnosticText.split(separator: "\n").map(String.init)
            try expectEqual(bareLines.count, 3, "无 pid 无 detail 时应为 3 行（实际 \(bareLines)）")
            try expectEqual(bareLines[0], "[COMPLETED] Fixture (ID: fixture-agent)", "首行含类型与 ID")
            try expectTrue(bareLines[1].hasPrefix("时间: "), "第二行为时间")
            try expectEqual(bareLines[2], "摘要: Fixture 任务完成 (5秒)", "第三行为摘要")
            try expectFalse(bare.copyableDiagnosticText.contains("PID: "), "无 pid 不得出现 PID 行")
            try expectFalse(bare.copyableDiagnosticText.contains("详情: "), "无 detail 不得出现详情行")

            let full = makeEvent(.attention, pid: 4_242, message: "需要确认",
                                 detail: "等待用户批准命令")
            let fullLines = full.copyableDiagnosticText.split(separator: "\n").map(String.init)
            try expectEqual(fullLines.count, 5, "pid + detail 时应为 5 行（实际 \(fullLines)）")
            try expectEqual(fullLines[0], "[ATTENTION] Fixture (ID: fixture-agent)", "类型大写")
            try expectEqual(fullLines[2], "摘要: 需要确认", "摘要行")
            try expectEqual(fullLines[3], "PID: 4242", "PID 行")
            try expectEqual(fullLines[4], "详情: 等待用户批准命令", "详情行")

            let emptyDetail = makeEvent(.costSpike, pid: 7, detail: "")
            try expectTrue(emptyDetail.copyableDiagnosticText.contains("PID: 7"), "有 pid 时出 PID 行")
            try expectFalse(emptyDetail.copyableDiagnosticText.contains("详情: "),
                            "空 detail 视同缺失，不出详情行")
        }
    }
}
