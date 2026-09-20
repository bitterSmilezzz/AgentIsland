import Foundation
@testable import AgentIslandCore

// MARK: - Qoder 会话方言（~/.qoder/projects/<slug>/<uuid>.jsonl）
//
// 逐行是 Anthropic 兼容的对话记录。这里最要紧的一条不是「能认出工作中」，
// 而是**刚回答完的那一拍不能再报成等待回答**——那会把用户叫回一个已经没事的窗口。

enum QoderTrackingTests {
    private static func line(_ role: String, _ blocks: [String], stop: String = "tool_use", id: String = "msg-1") -> String {
        "{\"type\":\"\(role)\",\"uuid\":\"\(id)\",\"timestamp\":\"2026-09-20T10:00:00.000Z\","
        + "\"message\":{\"role\":\"\(role)\",\"id\":\"\(id)\",\"stop_reason\":\"\(stop)\","
        + "\"content\":[\(blocks.joined(separator: ","))]}}"
    }
    private static func use(_ name: String, _ id: String, input: String = "{}") -> String {
        "{\"type\":\"tool_use\",\"id\":\"\(id)\",\"name\":\"\(name)\",\"input\":\(input)}"
    }
    private static func result(_ id: String) -> String {
        "{\"type\":\"tool_result\",\"tool_use_id\":\"\(id)\",\"content\":\"ok\"}"
    }

    @MainActor
    static func register() {
        TestKit.test("Qoder: AskUserQuestion 未回答 → 等待你回答，指纹跟着那次调用走") {
            let ask = line("assistant", [use("AskUserQuestion", "tu-9", input: #"{"questions":"[]"}"#)])
            guard case let .attention(req)? = AgentSessionInspector.detectQoder(lines: [ask], fileAge: 5) else {
                throw TestError(message: "未回答的提问必须报 attention")
            }
            try expectEqual(req.fingerprint, "qoder-tu-9")
        }

        TestKit.test("Qoder: 提问已被回答 → 绝不再报 attention（反向误报是本方言存在的理由）") {
            let ask = line("assistant", [use("AskUserQuestion", "tu-9")])
            let answered = line("user", [result("tu-9")], stop: "", id: "msg-2")
            let signal = AgentSessionInspector.detectQoder(lines: [ask, answered], fileAge: 5)
            if case .attention = signal {
                throw TestError(message: "用户已答完，仍在要求确认——通用关键字扫描正是这样误报的")
            }
            try expectTrue(signal?.isActive == true, "答完后应回到「继续处理中」而不是凭空无信号")
        }

        TestKit.test("Qoder: 未返回的 Bash → active 并带命令文案；已返回则不再冒充在跑") {
            let running = line("assistant", [use("Bash", "tu-1", input: #"{"command":"swift build"}"#)])
            guard case let .active(fp, action)? = AgentSessionInspector.detectQoder(lines: [running], fileAge: 3) else {
                throw TestError(message: "未返回结果的 Bash 应报 active")
            }
            try expectEqual(fp, "qoder-tu-1")
            try expectEqual(action, "运行: swift build")

            let done = line("user", [result("tu-1")], stop: "", id: "msg-2")
            let after = AgentSessionInspector.detectQoder(lines: [running, done], fileAge: 3)
            let afterAction: String?
            if case let .active(_, action)? = after { afterAction = action } else { afterAction = nil }
            try expectFalse(afterAction?.contains("swift build") == true,
                            "命令已有结果，不该还显示「运行: swift build」，实得 \(afterAction ?? "nil")")
        }

        TestKit.test("Qoder: end_turn 完成态只在 15 分钟内有效，过期自然回到待机") {
            let turn = line("assistant", ["{\"type\":\"text\",\"text\":\"done\"}"], stop: "end_turn", id: "msg-7")
            if case .completed(let fp)? = AgentSessionInspector.detectQoder(lines: [turn], fileAge: 60) {
                try expectEqual(fp, "qoder-msg-7")
            } else {
                throw TestError(message: "刚 end_turn 应报 completed")
            }
            try expectNil(AgentSessionInspector.detectQoder(lines: [turn], fileAge: 16 * 60),
                          "完成态过期后必须让位给「待机」，否则岛上一整天挂着假完成")
        }

        TestKit.test("Qoder: 畸形行与类型漂移不得崩，也不得编造状态") {
            let lines = ["{不是 JSON",
                         line("assistant", ["{\"type\":\"text\",\"text\":\"x\"}"], stop: "end_turn", id: "m0"),
                         line("assistant", [use("Edit", "tu-2", input: #"{"file_path":"/a/b.swift"}"#), ], id: "msg-3")]
            let signal = AgentSessionInspector.detectQoder(lines: lines, fileAge: 2)
            guard case let .active(_, action)? = signal else {
                throw TestError(message: "尾窗里最后一条有效 assistant 仍应识别为 active")
            }
            try expectEqual(action, "修改: /a/b.swift")
        }

        TestKit.test("Qoder: 档案声明自洽（安装路径锚定、会话根指向 projects、暂不采 token）") {
            guard let qoder = AgentRegistry.builtin.first(where: { $0.id == "qoder" }) else {
                throw TestError(message: "Qoder 档案必须内置")
            }
            try expectEqual(qoder.sessionDialect, .qoderTranscript)
            try expectTrue(qoder.bundleIDs.contains("com.qoder.app"), "bundle id 取自 Info.plist 实测值")
            try expectTrue(qoder.pathContains.allSatisfy { $0.hasPrefix("/") },
                           "pathContains 必须锚定安装路径，否则 ~/code/qoder-playground 里的 Electron 会被认成本 Agent；实得 \(qoder.pathContains)")
            try expectTrue(qoder.sessionDirs.contains { $0.hasSuffix("/.qoder/projects") }, "会话根必须指向 projects")
            try expectTrue(qoder.tokenRoots.isEmpty,
                           "Qoder 暂不登记 token 采集根（本机实测 usage 的四个 token 字段恒为 0）")
        }
    }
}
