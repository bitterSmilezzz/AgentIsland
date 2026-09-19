import Foundation
@testable import AgentIslandCore

// MARK: - 多 Agent 进阶生命周期、未决工具拦截、Cline/Roo 解析与 Antigravity 深度指标测试

@MainActor
enum MultiAgentAdvancedTests {
    static func register() {
        TestKit.test("Claude Code: 未决 Bash 工具调用必须拦截 completed 并保持 active") {
            let line1 = #"{"type":"user","message":{"content":[{"type":"text","text":"compile code"}]}}"#
            let line2 = #"{"type":"assistant","message":{"content":[{"type":"text","text":"Starting build..."},{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"swift build"}}]}}"#

            let signal = AgentSessionInspector.detect(lines: [line1, line2])
            guard let sig = signal, sig.isActive else {
                throw TestError(message: "在途 Bash 命令必须保持 active 态")
            }
            try expectEqual(sig.actionText, "执行: swift build")
        }

        TestKit.test("Claude Code: 工具调用结果返回后，模型完成回复正常转为 completed") {
            let line1 = #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"swift build"}}]}}"#
            let line2 = #"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"Build complete"}]}}"#
            let line3 = #"{"type":"task_complete","message":{"content":[{"type":"text","text":"Build succeeded!"}]}}"#

            let signal = AgentSessionInspector.detect(lines: [line1, line2, line3])
            guard case .completed? = signal else {
                throw TestError(message: "工具结果交付且模型结束本轮后应正常转为 completed")
            }
        }

        TestKit.test("Codex: 未决 exec_command 保持 active 并展示命令") {
            let line1 = #"{"type":"response_item","item":{"type":"function_call","call_id":"call_codex_1","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}"}}"#

            let signal = AgentSessionInspector.detect(lines: [line1])
            guard let sig = signal, sig.isActive else {
                throw TestError(message: "未决 exec_command 必须保持 active 态")
            }
            try expectEqual(sig.actionText, "执行: cargo test")
        }

        TestKit.test("Cline: ask command 识别为 attention 等待批准命令") {
            let messages: [[String: Any]] = [
                ["ts": 1700000001000.0, "type": "say", "say": "task", "text": "Run build"],
                ["ts": 1700000002000.0, "type": "ask", "ask": "command", "text": "npm run build"]
            ]
            let signal = AgentSessionInspector.detectClineOrRoo(messages: messages, fileAge: 10)
            guard case let .attention(req)? = signal else {
                throw TestError(message: "Cline ask command 必须解析为 attention")
            }
            try expectTrue(req.message.contains("等待你批准命令"), "文案应包含等待批准命令")
            try expectTrue(req.message.contains("npm run build"), "文案应包含具体命令")
        }

        TestKit.test("Cline: say command 识别为 active 执行命令中") {
            let messages: [[String: Any]] = [
                ["ts": 1700000001000.0, "type": "say", "say": "task", "text": "Run tests"],
                ["ts": 1700000002000.0, "type": "say", "say": "command", "text": "pytest -v"]
            ]
            let signal = AgentSessionInspector.detectClineOrRoo(messages: messages, fileAge: 10)
            guard let sig = signal, sig.isActive else {
                throw TestError(message: "Cline say command 必须解析为 active")
            }
            try expectEqual(sig.actionText, "执行命令: pytest -v")
        }

        TestKit.test("Cline: say completion_result 识别为 completed") {
            let messages: [[String: Any]] = [
                ["ts": 1700000001000.0, "type": "say", "say": "completion_result", "text": "All tasks completed successfully"]
            ]
            let signal = AgentSessionInspector.detectClineOrRoo(messages: messages, fileAge: 10)
            guard case .completed? = signal else {
                throw TestError(message: "Cline completion_result 必须解析为 completed")
            }
        }

        TestKit.test("Antigravity: 子智能体调用树跟踪与生命周期提取") {
            let line1 = #"{"step_index":200,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"invoke_subagent","args":{"Subagents":[{"TypeName":"research","Role":"Codebase Researcher","Model":"pro"}]}}]}"#
            let line2 = #"{"step_index":201,"source":"TOOL","type":"GENERIC","status":"DONE","content":"Created the following subagents: conv-sub-888"}"#
            let line3 = #"{"step_index":202,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"Waiting for subagent to finish analysis..."}"#

            var ctx = SessionActiveContext()
            let signal = AgentSessionInspector.detectAntigravitySession(lines: [line1, line2, line3], fileAge: 10) { ctx = $0 }
            guard let sig = signal, sig.isActive else {
                throw TestError(message: "子智能体在途中必须返回 active 态")
            }
            try expectEqual(ctx.subagents.count, 1, "应准确识别出 1 个在途子智能体")
            let sub = ctx.subagents.first!
            try expectEqual(sub.conversationId, "conv-sub-888")
            try expectEqual(sub.role, "Codebase Researcher")
            try expectEqual(sub.model, "pro")
        }

        TestKit.test("Antigravity: Token 深度指标拆解（prompt, completion, cache, thoughts）") {
            let line1 = #"{"step_index":300,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"Thinking deeply...","usageMetadata":{"promptTokenCount":1500,"candidatesTokenCount":320,"cachedContentTokenCount":800,"thoughtsTokenCount":250,"totalTokenCount":2870}}"#

            var ctx = SessionActiveContext()
            let signal = AgentSessionInspector.detectAntigravitySession(lines: [line1], fileAge: 10) { ctx = $0 }
            guard let sig = signal, sig.isActive, let tb = ctx.tokenBreakdown else {
                throw TestError(message: "必须提取出 TokenBreakdown 细分指标")
            }
            try expectEqual(tb.promptTokens, 1500)
            try expectEqual(tb.completionTokens, 320)
            try expectEqual(tb.cacheReadTokens, 800)
            try expectEqual(tb.reasoningTokens, 250)
            try expectEqual(tb.totalTokens, 2870)
        }

        TestKit.test("AgentSnapshot: 挂载后台任务与子智能体数据结构") {
            let profile = AgentRegistry.builtin.first { $0.id == "antigravity" }!
            let bgTask = AgentBackgroundTask(id: "task-1", action: "swift build")
            let subagent = AgentSubagentInfo(conversationId: "conv-1", role: "Search Agent", model: "flash")
            let tb = AgentTokenBreakdown(promptTokens: 100, completionTokens: 50, totalTokens: 150)

            let snap = AgentSnapshot(
                profile: profile,
                level: .working,
                processRunning: true,
                cpuPercent: 5.0,
                installed: true,
                activeSessions: 1,
                lastActivityAgo: 2,
                lastActivityText: "刚刚",
                backgroundTasks: [bgTask],
                subagents: [subagent],
                tokenBreakdown: tb
            )

            try expectEqual(snap.backgroundTasks.count, 1)
            try expectEqual(snap.backgroundTasks.first?.action, "swift build")
            try expectEqual(snap.subagents.count, 1)
            try expectEqual(snap.subagents.first?.role, "Search Agent")
            try expectEqual(snap.tokenBreakdown?.totalTokens, 150)
        }

        TestKit.test("会话上下文隔离: 其他 Agent 的探测结果不得携带 Antigravity 的在途上下文") {
            // 前置：Antigravity 解析器产出一份非空的在途上下文
            let line1 = #"{"step_index":200,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"invoke_subagent","args":{"Subagents":[{"TypeName":"research","Role":"Codebase Researcher","Model":"pro"}]}}]}"#
            let line2 = #"{"step_index":201,"source":"TOOL","type":"GENERIC","status":"DONE","content":"Created the following subagents: conv-sub-888"}"#
            let line3 = #"{"step_index":202,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"Waiting for subagent to finish analysis..."}"#
            var agContext = SessionActiveContext()
            _ = AgentSessionInspector.detectAntigravitySession(lines: [line1, line2, line3], fileAge: 10) { agContext = $0 }
            try expectFalse(agContext.subagents.isEmpty, "前置条件：Antigravity 侧必须解析出在途子智能体")

            // 同一时刻探测 Claude：上下文必须为空。上下文一旦存放在按 agent id 索引的
            // 全局字典里，这里就会串到上面那份 Antigravity 子任务（悬浮胶囊误报 🤖1子任务）
            let tempDir = NSTemporaryDirectory() + "ctx-isolation-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: tempDir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(atPath: tempDir) }
            let transcript = URL(fileURLWithPath: tempDir).appendingPathComponent("session.jsonl")
            try #"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pytest"}}]}}"#
                .data(using: .utf8)!
                .write(to: transcript)

            let claude = AgentRegistry.builtin.first { $0.id == "claude" }!
            let probe = AgentSessionInspector.probe(profile: claude, activityFiles: [transcript], now: Date())
            try expectTrue(probe.signal?.isActive == true, "在途 Bash 未交付结果，Claude 应为 active")
            try expectTrue(probe.context.subagents.isEmpty, "Claude 不得继承其他 Agent 的子智能体")
            try expectTrue(probe.context.backgroundTasks.isEmpty, "Claude 不得继承其他 Agent 的后台任务")
            try expectNil(probe.context.tokenBreakdown, "Claude 不得继承其他 Agent 的 Token 细分")
        }
    }
}
