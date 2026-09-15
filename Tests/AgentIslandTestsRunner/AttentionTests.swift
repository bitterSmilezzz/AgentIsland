import Foundation
import Combine
@testable import AgentIslandCore

// MARK: - 等待用户确认状态与通知路由回归

@MainActor
enum AttentionTests {
    static let dim = AgentRegistry.builtin.first { $0.id == "dim" }!

    static func register() {
        TestKit.test("等待确认解析: Codex request_user_input 仅在未响应时成立") {
            let request = #"{"type":"response_item","payload":{"type":"custom_tool_call","name":"request_user_input","call_id":"call-1","status":"completed"}}"#
            let unrelated = #"{"type":"event_msg","payload":{"type":"token_count"}}"#
            let result = #"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-1","output":"<REDACTED>"}}"#

            let pending = AgentSessionInspector.detect(lines: [request, unrelated])?.attentionRequest
            try expectEqual(pending?.fingerprint, "call-1", "未响应请求应保持等待确认")
            try expectEqual(pending?.message, "等待你选择或确认", "通知文案不泄露原始消息")
            try expectNil(AgentSessionInspector.detect(lines: [request, result]),
                          "同一 call_id 已有结果后必须解除等待确认")
        }

        TestKit.test("等待确认解析: Claude AskUserQuestion 被用户回复解除") {
            let request = #"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-7","name":"AskUserQuestion","input":{"question":"<REDACTED>"}}]}}"#
            let answer = #"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-7","content":"<REDACTED>"}]}}"#
            try expectEqual(AgentSessionInspector.detect(lines: [request])?.attentionRequest?.fingerprint, "tool-7")
            try expectNil(AgentSessionInspector.detect(lines: [request, answer]),
                          "用户回复/tool_result 后不得继续误报等待")
        }

        TestKit.test("等待确认解析: 尾窗只有 AskUserQuestion 结果时不得反向误报") {
            let resultOnly = #"{"role":"tool_result","toolMetadata":{"toolName":"AskUserQuestion","status":"success","toolCallId":"tool-7"},"parts":[{"type":"tool_result","tool_use_id":"tool-7"}]}"#
            try expectNil(AgentSessionInspector.detect(lines: [resultOnly]),
                          "工具结果中的 toolName 只是关联信息，不是新的确认请求")
        }

        TestKit.test("等待确认解析: 普通问句与运行中工具不得误报") {
            let assistantQuestion = #"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"是否继续？"}]}}"#
            let runningTool = #"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","call_id":"call-2","status":"completed"}}"#
            try expectNil(AgentSessionInspector.detect(lines: [assistantQuestion]), "普通文本问句不是结构化确认请求")
            try expectNil(AgentSessionInspector.detect(lines: [runningTool]), "普通工具调用不是确认请求")
        }

        TestKit.test("完成解析: task_complete 立即成立，新一轮推理使旧完成失效") {
            let completed = #"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-9"}}"#
            let accounting = #"{"type":"event_msg","payload":{"type":"token_count"}}"#
            let reasoning = #"{"type":"response_item","payload":{"type":"reasoning","id":"reason-10"}}"#
            try expectEqual(AgentSessionInspector.detect(lines: [completed, accounting]),
                            .completed(fingerprint: "turn-9"), "收尾记账不应冲掉完成态")
            try expectNil(AgentSessionInspector.detect(lines: [completed, reasoning]),
                          "新一轮推理开始后旧完成态必须失效")
        }

        TestKit.test("引擎: 等待确认覆盖工作/待机且每个请求只通知一次") {
            let start = Date()
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.dimcode/v2/data/sessions"
            let files = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: [dim],
                config: EngineConfig(workingWindow: 5, minWorkingHold: 2),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: files,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            var signal: AgentSessionSignal? = .attention(AgentAttentionRequest(
                fingerprint: "approval-1", message: "等待你批准操作"))
            engine.inspectSessionHook = { _, _, _ in signal }

            let waiting = engine.sample(now: start)
            try expectEqual(waiting.first?.level, .attention, "确认请求优先于仍在窗口内的写入信号")
            try expectEqual(waiting.first?.currentAction, "等待你批准操作")
            try expectFalse(engine.anyWorking, "等用户时 Agent 没在工作")
            try expectEqual(engine.latestEvent?.eventType, .attention, "首次进入应通知")

            engine.clearLatestEvent()
            _ = engine.sample(now: start.addingTimeInterval(2))
            try expectNil(engine.latestEvent, "同一待确认请求不得每拍重复通知")

            signal = nil
            files.writes = [dir: start.addingTimeInterval(-600)]
            let idle = engine.sample(now: start.addingTimeInterval(8))
            try expectEqual(idle.first?.level, .idle, "用户处理后应回到真实待机态")
            try expectNil(engine.latestEvent, "等待确认解除不是任务完成，不得响完成铃")

            signal = .attention(AgentAttentionRequest(fingerprint: "approval-2", message: "等待你选择或确认"))
            _ = engine.sample(now: start.addingTimeInterval(10))
            try expectEqual(engine.latestEvent?.eventType, .attention, "新的确认请求应重新通知")
        }

        TestKit.test("引擎: 显式任务完成立即结束 working 且不重复通知") {
            let start = Date()
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.dimcode/v2/data/sessions"
            let files = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: [dim],
                config: EngineConfig(workingWindow: 60, minWorkingHold: 10),
                processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                fileMonitor: files,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            var signal: AgentSessionSignal?
            engine.inspectSessionHook = { _, _, _ in signal }

            try expectEqual(engine.sample(now: start).first?.level, .working)
            signal = .completed(fingerprint: "turn-9")
            let completed = engine.sample(now: start.addingTimeInterval(5))
            try expectEqual(completed.first?.level, .completed,
                            "有 task_complete 时不应继续被 60 秒写入窗口卡在工作中")
            try expectEqual(engine.latestEvent?.eventType, .completed, "显式完成应立即通知")

            engine.clearLatestEvent()
            _ = engine.sample(now: start.addingTimeInterval(7))
            try expectNil(engine.latestEvent, "同一完成标记不得重复通知")
        }

        TestKit.test("通知路由: userInfo 可无损定位对应 Agent") {
            let info = AgentNotificationRoute.userInfo(agentId: "codex")
            try expectEqual(AgentNotificationRoute.agentId(from: info), "codex")
            try expectNil(AgentNotificationRoute.agentId(from: [:]), "无目标的普通通知不可误跳转")
        }

        TestKit.test("通知路由: 系统通知写入 agentId 且点击回调激活目标窗口") {
            let root = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .deletingLastPathComponent()
            let notificationFile = root.appendingPathComponent("Sources/AgentIsland/CompletionNotification.swift")
            let appFile = root.appendingPathComponent("Sources/AgentIsland/AgentIslandApp.swift")
            guard let notificationSource = try? String(contentsOf: notificationFile, encoding: .utf8),
                  let appSource = try? String(contentsOf: appFile, encoding: .utf8) else { return }
            try expectTrue(notificationSource.contains("content.userInfo = AgentNotificationRoute.userInfo"),
                           "通知必须携带 Agent 路由")
            try expectTrue(appSource.contains("UNUserNotificationCenter.current().delegate = self"),
                           "启动时必须注册通知点击代理")
            try expectTrue(appSource.contains("didReceive response: UNNotificationResponse"),
                           "必须实现通知点击回调")
            try expectTrue(appSource.contains("AppActivator.activate(pid: snapshot?.pid"),
                           "点击后必须尝试激活对应 GUI 或 CLI 宿主终端")
        }


        TestKit.test("事件流: 多个 Agent 同拍等待确认时逐条投递") {
            let profiles = AgentRegistry.builtin.filter { $0.id == "dim" || $0.id == "claude" }
            let provider = FakeProcessProvider(processNames: ["DimAgent", "claude"], bundleIDs: [])
            let engine = ActivityEngine(
                profiles: profiles,
                processMonitor: provider,
                fileMonitor: FakeFileActivityProvider(writes: [:]),
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            engine.inspectSessionHook = { profile, _, _ in
                .attention(AgentAttentionRequest(
                    fingerprint: "request-\(profile.id)", message: "等待你选择或确认"))
            }
            var delivered: [String] = []
            let subscription = engine.taskEvents.sink { delivered.append($0.agentId) }
            defer { subscription.cancel() }

            _ = engine.sample(now: Date())
            try expectEqual(Set(delivered), Set(["dim", "claude"]),
                            "latestEvent 虽只有一个槽位，系统通知事件流也不能丢另一个 Agent")
        }
    }
}
