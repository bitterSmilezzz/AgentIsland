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
    /// 带轮次身份的行：`promptId` 挂在每条 user 行上，而**一轮的首行**才额外带
    /// `humanInput`（真人敲的）或 `isMeta`（系统注入：后台任务通知、并发会话消息）
    private static func turn(_ role: String, _ blocks: [String], prompt: String,
                             human: Bool = false, meta: Bool = false,
                             stop: String = "tool_use", id: String = "msg-1") -> String {
        "{\"type\":\"\(role)\",\"uuid\":\"\(id)\",\"promptId\":\"\(prompt)\","
        + (human ? "\"humanInput\":{}," : "")
        + (meta ? "\"isMeta\":true," : "")
        + "\"timestamp\":\"2026-09-20T10:00:00.000Z\","
        + "\"message\":{\"role\":\"\(role)\",\"id\":\"\(id)\",\"stop_reason\":\"\(stop)\","
        + "\"content\":[\(blocks.joined(separator: ","))]}}"
    }
    private static func text(_ s: String) -> String {
        "{\"type\":\"text\",\"text\":\"\(s)\"}"
    }
    private static func completedFingerprint(_ signal: AgentSessionSignal?) -> String? {
        if case let .completed(fp)? = signal { return fp }
        return nil
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

        TestKit.test("Qoder: 完成事件按「哪一轮人类指令」记，不按每条 assistant 消息记") {
            // 用户报的现象：岛对 Qoder 一路弹「任务完成」，可那个会话一直在跑。
            // 根因是完成指纹取最后一条 assistant 的 id——Qoder 每次模型调用都换 id，
            // 而一个长任务里模型会 end_turn 很多次（等后台构建、被通知唤醒后续跑），
            // 于是同一件活每续跑一轮就再响一次铃、再弹一次横幅。
            let prompt = turn("user", [text("做一轮改造")], prompt: "A", human: true)
            let firstEnd = turn("assistant", [text("第 1 段收尾")], prompt: "A",
                                stop: "end_turn", id: "m1")
            guard let fp1 = completedFingerprint(
                AgentSessionInspector.detectQoder(lines: [prompt, firstEnd], fileAge: 5)) else {
                throw TestError(message: "真人那一轮收尾应报 completed")
            }
            try expectEqual(fp1, "qoder-A", "完成指纹要带的是这一轮人类指令的身份")
            // 任务跑到后半程：首行早已滑出尾窗，靠调用方递进来的「最近一次人类轮次」继承
            let later = [turn("user", [result("tu-1")], prompt: "A", stop: "", id: "u2"),
                         turn("assistant", [text("第 2 段收尾")], prompt: "A",
                              stop: "end_turn", id: "m2")]
            try expectEqual(completedFingerprint(
                AgentSessionInspector.detectQoder(lines: later, fileAge: 5, lastHumanTurn: "A")), fp1,
                "同一轮里的第二次 end_turn 不许换个指纹——那等于又弹一次「已完成」")
        }

        TestKit.test("Qoder: 系统注入的续命轮收尾，不再弹第二次「任务完成」") {
            // `isMeta` 起头的轮次是 Qoder 自己续跑的（后台任务通知、并发会话注入），
            // 用户没有派新活 ⇒ 完成指纹仍要落在**上一次真人那一轮**上，引擎才认得这是同一件事
            let metaTurn = [turn("user", [text("（后台任务已完成）")], prompt: "B", meta: true),
                            turn("assistant", [text("续跑后收尾")], prompt: "B",
                                 stop: "end_turn", id: "m3")]
            try expectEqual(completedFingerprint(
                AgentSessionInspector.detectQoder(lines: metaTurn, fileAge: 5, lastHumanTurn: "A")),
                "qoder-A", "续命轮不许生成新的完成指纹")
            // 反过来：冷启动只见到续命轮、没有可继承的人类轮次时**宁可响一声**——
            // 把「认不出来源」做成永不完成，等于把这条通知整个废掉（本仓犯过同类错）
            try expectEqual(completedFingerprint(
                AgentSessionInspector.detectQoder(lines: metaTurn, fileAge: 5)), "qoder-B",
                "认不出来源时按当前轮次报完成，别静默")
        }

        TestKit.test("Qoder: 岛刚重启（备忘是空的）时，续跑要沿用同一个指纹而不是各报一次") {
            // 这一条是外部 review 报出来的：备忘那格是**进程内**的，重启即清零。
            // 若「认不出来源」退到当前轮次 id，那么重启后的每一次续跑都会换个新指纹，
            // 用户报的「一路弹已完成」就原地复发。退到会话身份才是稳定的。
            let continuation = [turn("user", [text("（后台任务已完成）")], prompt: "B", meta: true),
                                turn("assistant", [text("续跑收尾")], prompt: "B",
                                     stop: "end_turn", id: "m3")]
            let next = [line("assistant", [text("又续了一次")], stop: "end_turn", id: "m4")]
            let fp1 = completedFingerprint(
                AgentSessionInspector.detectQoder(lines: continuation, fileAge: 5, sessionKey: "sess-1"))
            let fp2 = completedFingerprint(
                AgentSessionInspector.detectQoder(lines: next, fileAge: 5, sessionKey: "sess-1"))
            try expectEqual(fp1, "qoder-sess-1", "退路要落在会话身份上，不是这一轮的 promptId")
            try expectEqual(fp2, fp1, "同一份会话里的续跑共用一个指纹 ⇒ 引擎认得是同一件事")
            // 但别把「该响的」也吞掉：真人敲了新指令，首行进尾窗 ⇒ 指纹换成新轮次
            try expectEqual(completedFingerprint(AgentSessionInspector.detectQoder(
                lines: [turn("user", [text("新任务")], prompt: "C", human: true),
                        line("assistant", [text("干完了")], stop: "end_turn", id: "m5")],
                fileAge: 5, sessionKey: "sess-1")), "qoder-C",
                "新的人类轮次必须换指纹，否则这条通知就此永久静默")
        }

        TestKit.test("Qoder: 真人刚发出指令、模型还没回话 ⇒ 是「正在处理」，不是上一件事完成") {
            let lines = [turn("assistant", [text("上一轮收尾")], prompt: "A",
                              stop: "end_turn", id: "m1"),
                         turn("user", [text("再来一轮")], prompt: "C", human: true)]
            let signal = AgentSessionInspector.detectQoder(lines: lines, fileAge: 1, lastHumanTurn: "A")
            if case .completed = signal {
                throw TestError(message: "用户刚敲完就报「任务完成」，报的还是上一轮的收尾")
            }
            guard case let .active(_, action)? = signal else {
                throw TestError(message: "该报 active，实得 \(String(describing: signal))")
            }
            try expectEqual(action, "正在处理你的新指令")
            // 同一条分支还顺手接住了另一种卡死：提问没走 tool_result 而是被用户直接打字回答，
            // 旧判定会永远停在「等待你回答或选择」，因为那个 tu-9 再也不会有结果
            let answeredByTyping = [turn("assistant", [use("AskUserQuestion", "tu-9")], prompt: "A", id: "m0"),
                                    turn("user", [text("我选第二个")], prompt: "C", human: true)]
            let after = AgentSessionInspector.detectQoder(lines: answeredByTyping, fileAge: 1,
                                                          lastHumanTurn: "A")
            try expectTrue(after?.attentionRequest == nil,
                           "用户已经用打字回答了，不许还挂着「等待你回答」")
            try expectTrue(after?.isActive == true, "打字回答之后应回到「正在处理」")
        }

        TestKit.test("Qoder: 人类轮次的备忘按会话文件留一格，让后半程的完成事件认得来源") {
            // 覆盖 `inspectQoderTranscript` 的真实路径：判定要的那条信息（这一轮是真人起的头）
            // 只在首行还留在尾窗里的那几拍可见，而完成事件发生在几十拍之后
            let root = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-qoder-turn-\(UUID().uuidString)", isDirectory: true)
            let project = root.appendingPathComponent("proj", isDirectory: true)
            try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
            let file = project.appendingPathComponent("s1.jsonl")
            AgentSessionInspector.resetQoderHumanTurnCache()
            func write(_ lines: [String]) throws {
                try lines.joined(separator: "\n").write(to: file, atomically: true, encoding: .utf8)
            }
            try write([turn("user", [text("开工")], prompt: "A", human: true),
                       turn("assistant", [use("Bash", "tu-1")], prompt: "A", id: "m1")])
            let first = AgentSessionInspector.inspectQoderTranscript(dirs: [root.path], now: Date())
            try expectTrue(first?.isActive == true, "第 1 拍该是 active（工具还没回）")
            // 第 2 拍：尾窗里**只剩 assistant 行**（真实格式里 promptId 只挂在 user 行上，
            // 而单行可以大到让 262KB 的字节上限先于 60 行到点）。这时「这一轮是谁起的头」
            // 只能从那一格备忘里拿——所以这条用例必须写成这个形状，否则备忘的写入路径
            // 根本没人验（第一版就是写成了还留着 tool_result 行，变异掉写入仍然全绿）
            try write([line("assistant", [text("干完了")], stop: "end_turn", id: "m2")])
            try expectEqual(completedFingerprint(
                AgentSessionInspector.inspectQoderTranscript(dirs: [root.path], now: Date())), "qoder-A",
                "尾窗里一条 user 行都没有时，要靠那格备忘认得这是真人那一轮、并沿用同一个指纹")
            AgentSessionInspector.resetQoderHumanTurnCache()
            try FileManager.default.removeItem(at: root)
        }

        TestKit.test("Qoder: 那格备忘只服务它自己那个会话文件（换会话不得继承）") {
            // 继承是为了「同一轮的后半程还认得来源」，不是「上一个会话的人类轮次算数」。
            // 跨会话串味会让新会话的第一次完成挂着旧 promptId，引擎就此把它当重复事件吞掉——
            // 用户看到的症状是「这个 Agent 再也不报完成了」
            let root = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-qoder-two-\(UUID().uuidString)", isDirectory: true)
            let project = root.appendingPathComponent("proj", isDirectory: true)
            try FileManager.default.createDirectory(at: project, withIntermediateDirectories: true)
            AgentSessionInspector.resetQoderHumanTurnCache()
            let old = project.appendingPathComponent("old.jsonl")
            try [turn("user", [text("开工")], prompt: "A", human: true),
                turn("assistant", [text("收尾")], prompt: "A", stop: "end_turn", id: "m1")]
                .joined(separator: "\n").write(to: old, atomically: true, encoding: .utf8)
            _ = AgentSessionInspector.inspectQoderTranscript(dirs: [root.path], now: Date())
            // 换一个新会话文件，尾窗里只有它自己的续命轮（没有 humanInput 行）
            try FileManager.default.removeItem(at: old)
            let fresh = project.appendingPathComponent("fresh.jsonl")
            try [turn("user", [text("（系统注入）")], prompt: "B", meta: true),
                turn("assistant", [text("收尾")], prompt: "B", stop: "end_turn", id: "m2")]
                .joined(separator: "\n").write(to: fresh, atomically: true, encoding: .utf8)
            try expectEqual(completedFingerprint(
                AgentSessionInspector.inspectQoderTranscript(dirs: [root.path],
                                                             // 定位缓存 TTL 是 10s：不越过它就会命中刚被删掉的旧文件
                                                             now: Date().addingTimeInterval(60))),
                "qoder-\(fresh.deletingPathExtension().lastPathComponent)",
                "新会话不许继承旧会话那一格人类轮次（退路落在自己这份会话的身份上）")
            AgentSessionInspector.resetQoderHumanTurnCache()
            try FileManager.default.removeItem(at: root)
        }

        TestKit.test("Qoder: 来源不可知的那一轮继承上一个人类轮，宁可少响一次也不重复响") {
            // 这条钉的是**取舍**，不是巧合：首行若在两拍采样之间滑出尾窗，这一轮到底是谁起的头
            // 就永久不可知了。此时继承旧指纹 ⇒ 这一轮的完成不再响铃；反过来把它当新任务，
            // 续命轮也会当新任务——这条修复就被自己抵消了。用户报的是「弹太多」，方向照这个选。
            let tail = [turn("user", [result("tu-9")], prompt: "B", stop: "", id: "u9"),
                        turn("assistant", [text("收尾")], prompt: "B", stop: "end_turn", id: "m9")]
            try expectEqual(completedFingerprint(
                AgentSessionInspector.detectQoder(lines: tail, fileAge: 5, lastHumanTurn: "A")), "qoder-A",
                "不可知的轮次要继承旧指纹，别当成新任务再响一次")
        }

        TestKit.test("Qoder: 档案声明自洽（安装路径锚定、会话根指向 projects、暂不采 token）") {            guard let qoder = AgentRegistry.builtin.first(where: { $0.id == "qoder" }) else {
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
